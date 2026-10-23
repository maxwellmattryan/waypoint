//! Running a goal's plan: `POST /goals/{id}/run` and the `plan_step` ticks that source the
//! boards (wiremock, `fixtures/sourcing/greenhouse_board.json`), score the matching postings
//! with `score_fit` (`fake-claude.sh`), shortlist them and prepare a packet for each with
//! `prep_packet`, ending with packets waiting for approval; the automatic replans after a
//! failed step; and resuming a running goal when the worker starts. Plans are seeded
//! through `goals::store::write_version`.

mod common;

use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::events::{Event, EventBus};
use waypoint_api::goals::store::{self, PlanDraft};
use waypoint_api::goals::{
    PlanAuthor, PlanCompanyConfig, PlanConfig, PlanDropped, PlanStatus, PlanTrigger,
};
use waypoint_api::sourcing::Source;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TIMEOUT: Duration = Duration::from_secs(30);
const GREENHOUSE: &str = include_str!("fixtures/sourcing/greenhouse_board.json");

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

/// Serves greenhouse/example-labs from the recorded board; without it, every fetch is 404.
async fn board_server(serve: bool) -> MockServer {
    let server = MockServer::start().await;
    if serve {
        Mock::given(method("GET"))
            .and(path("/v1/boards/example-labs/jobs"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(GREENHOUSE.as_bytes().to_vec(), "application/json"),
            )
            .mount(&server)
            .await;
    }
    server
}

/// The API with greenhouse at `server` and a runner playing `mode`.
async fn app(pool: &PgPool, server: &MockServer, mode: &str) -> (Client, common::Fake) {
    let fake = common::fake(mode, Duration::from_secs(30));
    let mut capture = waypoint_api::capture::Config::default();
    capture.greenhouse.api = server.uri();
    let rocket = common::app_with_capture(
        pool.clone(),
        fake.runner.clone(),
        common::fast_config(),
        capture,
    );
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

async fn get(client: &Client, path: &str) -> Value {
    let response = client.get(path.to_owned()).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::Ok, "GET {path}");
    response.into_json().await.expect("json")
}

async fn run(client: &Client, goal: i64) -> Value {
    let response = client
        .post(format!("/goals/{goal}/run"))
        .header(host())
        .header(ContentType::JSON)
        .body("{}")
        .dispatch()
        .await;
    let status = response.status();
    let body: Value = response.into_json().await.unwrap_or(Value::Null);
    assert_eq!(status, Status::Accepted, "{body}");
    body
}

/// Polls `GET /goals/{id}` until its status is `status`; returns the detail.
async fn until(client: &Client, id: i64, status: &str) -> Value {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let goal = get(client, &format!("/goals/{id}")).await;
        if goal["status"] == status {
            return goal;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "goal {id} is {}, never {status}: {goal}",
            goal["status"]
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("{sql}: {err}"))
}

/// Polls until `sql` counts at least one row.
async fn eventually(pool: &PgPool, sql: &str) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while count(pool, sql).await == 0 {
        assert!(tokio::time::Instant::now() < deadline, "never true: {sql}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// What `prep_packet_goal.jsonl` cites, after [`seed_memory`]: resume 1 "Backend",
/// confirmed answer memories 2 to 5 (one per answer-bank topic) and confirmed fact 6, which
/// matches the engineer posting.
async fn seed_prep(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO resume_versions (name, filename, size_bytes, sha256, pdf, text) \
         VALUES ('Backend', 'Backend.pdf', 9, md5('Backend'), $1, \
         'Backend engineer: Rust services and PostgreSQL')",
    )
    .bind(b"%PDF-test".as_slice())
    .execute(pool)
    .await
    .expect("seed a resume");
    sqlx::query(
        "INSERT INTO memories (kind, topic, content, status, confirmed_at) VALUES \
         ('answer', 'work_authorization', 'EU citizen, no sponsorship needed', 'confirmed', now()), \
         ('answer', 'notice_period', 'Four weeks notice', 'confirmed', now()), \
         ('answer', 'salary_expectation', 'Around 90,000 EUR a year', 'confirmed', now()), \
         ('answer', 'why_leaving', 'Wants to own a larger platform', 'confirmed', now()), \
         ('fact', 'skill', 'Shipped Rust services on Postgres', 'confirmed', now())",
    )
    .execute(pool)
    .await
    .expect("seed prep memories");
}

/// The one confirmed memory the fit fixtures cite (id 1 in a fresh database).
async fn seed_memory(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO memories (kind, topic, content, status, confirmed_at) VALUES \
         ('preference', 'target_role', 'Senior backend roles in Rust', 'confirmed', now())",
    )
    .execute(pool)
    .await
    .expect("seed a memory");
}

/// The config `plan_goal.jsonl` answers with: greenhouse/example-labs, keyword `Engineer`,
/// `min_score` 60.
fn config() -> PlanConfig {
    PlanConfig {
        companies: vec![PlanCompanyConfig {
            name: "Example Labs".into(),
            source: Source::Greenhouse,
            board: "example-labs".into(),
            reason: Some("Builds the platform the goal names".into()),
        }],
        title_keywords: vec!["Engineer".into()],
        locations: vec![],
        remote_only: false,
        max_to_score: 10,
        shortlist_size: 5,
        min_score: 60,
        prep_notes: "Lead with Rust and Postgres work".into(),
    }
}

/// A goal with `status` and a `ready` plan of [`config`]; returns `(goal, plan)`.
async fn seed_goal(pool: &PgPool, status: &str) -> (i64, i64) {
    let mut tx = pool.begin().await.expect("begin");
    let goal: i64 =
        sqlx::query_scalar("INSERT INTO goals (text, status) VALUES ($1, $2) RETURNING id")
            .bind("Engineering roles at Example Labs")
            .bind(status)
            .fetch_one(&mut *tx)
            .await
            .expect("insert a goal");
    let plan = store::write_version(
        &mut tx,
        goal,
        &PlanDraft {
            author: PlanAuthor::Planner,
            trigger: PlanTrigger::Goal,
            status: PlanStatus::Ready,
            summary: "Engineering roles at Example Labs".into(),
            confidence: Some(0.9),
            config: config(),
            dropped: PlanDropped::default(),
            run_id: None,
            review_item_id: None,
            claude_call_id: None,
            cost_usd: 0.0,
        },
    )
    .await
    .expect("write a plan");
    tx.commit().await.expect("commit");
    (goal, plan.id)
}

/// `step kind -> step` of a detail's plan.
fn step<'a>(goal: &'a Value, kind: &str) -> &'a Value {
    goal["plan"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|s| s["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} step"))
}

/// The id of the board's `Senior Backend Engineer` posting.
async fn engineer(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT id FROM job_postings WHERE title = 'Senior Backend Engineer'")
        .fetch_one(pool)
        .await
        .expect("the engineer posting")
}

/// `(id, run_id)` of the newest packet.
async fn newest_packet(pool: &PgPool) -> (i64, i64) {
    sqlx::query_as("SELECT id, run_id FROM application_packets ORDER BY id DESC LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("a packet")
}

#[sqlx::test]
async fn runs_a_goal_to_packets_awaiting_approval(pool: PgPool) {
    seed_memory(&pool).await;
    seed_prep(&pool).await;
    let (goal, _) = seed_goal(&pool, "planned").await;
    let server = board_server(true).await;
    let (client, _fake) = app(&pool, &server, "route_goal_runs_prep").await;
    let mut events = client
        .rocket()
        .state::<EventBus>()
        .expect("a bus")
        .subscribe();

    let started = run(&client, goal).await;
    assert_eq!(started["status"], "running");
    let done = until(&client, goal, "completed").await;
    assert!(done["error"].is_null());
    let posting = engineer(&pool).await;

    let source = step(&done, "source");
    assert_eq!(source["status"], "succeeded");
    assert_eq!(source["output"]["boards"], 1);
    assert_eq!(source["output"]["inserted"], 3);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM jobs WHERE kind = 'source_board' AND status = 'succeeded'"
        )
        .await,
        1
    );
    let score = step(&done, "score");
    assert_eq!(score["status"], "succeeded");
    assert_eq!(
        score["output"],
        json!({"posting_ids": [posting], "queued": 1, "reused": 0, "in_review": 0, "failed": 0})
    );
    let scored: Vec<Value> =
        sqlx::query_scalar("SELECT input FROM agent_runs WHERE workflow = 'score_fit'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(scored, [json!({"posting_id": posting})]);
    let shortlist = step(&done, "shortlist");
    assert_eq!(
        shortlist["output"],
        json!({"shortlisted": 1, "in_review": 0})
    );
    assert_eq!(
        done["plan"]["shortlist"],
        json!([{"rank": 1, "posting_id": posting, "company": "Example Labs Inc.",
                "title": "Senior Backend Engineer", "score": 82, "band": "strong"}])
    );
    let (packet, prep_run) = newest_packet(&pool).await;
    let prep = step(&done, "prep");
    assert_eq!(prep["status"], "succeeded");
    assert_eq!(
        prep["output"],
        json!({"packets": [{"posting_id": posting, "packet_id": packet, "run_id": prep_run,
                            "outcome": "prepared", "error": null}],
               "prepared": 1, "reused": 0, "failed": 0})
    );
    let prepped: Vec<Value> =
        sqlx::query_scalar("SELECT input FROM agent_runs WHERE workflow = 'prep_packet'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        prepped,
        [json!({"posting_id": posting, "notes": "Lead with Rust and Postgres work"})]
    );
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let shown = get(&client, &format!("/packets/{packet}")).await;
        assert_eq!(shown["status"], "ready");
        if shown["pending_action_id"].is_i64() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "packet {packet} never awaits approval: {shown}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    eventually(
        &pool,
        "SELECT count(*) FROM agent_runs \
         WHERE workflow = 'approve_packet' AND status = 'awaiting_approval'",
    )
    .await;
    let done = get(&client, &format!("/goals/{goal}")).await;
    assert_eq!(
        done["plan"]["packets"],
        json!([{"posting_id": posting, "company": "Example Labs Inc.",
                "title": "Senior Backend Engineer", "outcome": "prepared",
                "packet_id": packet, "status": "ready", "run_id": prep_run, "error": null}])
    );
    assert_eq!(
        step(&done, "target_companies")["output"],
        json!({"companies": 1, "unverified": 1})
    );
    let seen = common::events(&mut events, Duration::from_millis(200), |_| false).await;
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::GoalUpdated { goal: g } if g.id == goal)),
        "goal.updated"
    );

    // An equivalent plan reuses the applied score and the ready packet.
    let (again, _) = seed_goal(&pool, "planned").await;
    run(&client, again).await;
    let done = until(&client, again, "completed").await;
    assert_eq!(
        step(&done, "score")["output"],
        json!({"posting_ids": [posting], "queued": 0, "reused": 1, "in_review": 0, "failed": 0})
    );
    assert_eq!(done["plan"]["shortlist"][0]["posting_id"], posting);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'score_fit'"
        )
        .await,
        1
    );
    assert_eq!(
        step(&done, "prep")["output"],
        json!({"packets": [{"posting_id": posting, "packet_id": packet, "run_id": prep_run,
                            "outcome": "reused", "error": null}],
               "prepared": 0, "reused": 1, "failed": 0})
    );
    assert_eq!(done["plan"]["packets"][0]["outcome"], "reused");
    assert_eq!(done["plan"]["packets"][0]["status"], "ready");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'prep_packet'"
        )
        .await,
        1
    );
}

#[sqlx::test]
async fn failing_prep_replans_then_fails(pool: PgPool) {
    seed_memory(&pool).await;
    seed_prep(&pool).await;
    let (goal, _) = seed_goal(&pool, "planned").await;
    let server = board_server(true).await;
    let (client, _fake) = app(&pool, &server, "route_goal_runs_prep_fails").await;

    run(&client, goal).await;
    let failed = until(&client, goal, "failed").await;
    assert_eq!(failed["auto_replans"], 2);
    let error = failed["error"].as_str().expect("an error");
    assert!(
        error.starts_with("prep failed: Every packet failed: "),
        "{error}"
    );
    let prep = step(&failed, "prep");
    assert_eq!(prep["status"], "failed");
    assert!(
        prep["last_error"]
            .as_str()
            .is_some_and(|e| e.starts_with("Every packet failed: ")),
        "{prep}"
    );
    assert_eq!(prep["output"]["failed"], 1);
    assert_eq!(prep["output"]["packets"][0]["outcome"], "failed");
    assert_eq!(failed["plan"]["packets"][0]["outcome"], "failed");
    assert!(failed["plan"]["packets"][0]["status"].is_null());
    for kind in ["target_companies", "source", "score", "shortlist"] {
        assert_eq!(step(&failed, kind)["status"], "succeeded", "{kind}");
    }
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'plan_goal'"
        )
        .await,
        2
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'score_fit'"
        )
        .await,
        1
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM application_packets").await,
        0
    );
}

#[sqlx::test]
async fn failed_step_replans_and_keeps_finished_steps(pool: PgPool) {
    seed_memory(&pool).await;
    seed_prep(&pool).await;
    let (goal, v1) = seed_goal(&pool, "planned").await;
    let server = board_server(true).await;
    let (client, _fake) = app(&pool, &server, "route_goal_runs_replan").await;

    run(&client, goal).await;
    let done = until(&client, goal, "completed").await;
    assert_eq!(done["auto_replans"], 1);
    let plan = &done["plan"];
    assert_eq!(plan["version"], 2);
    assert_eq!(plan["trigger"], "step_failed");
    assert_eq!(plan["failed_step"], "shortlist");
    assert_eq!(plan["min_score"], 40);
    let planner: Vec<Value> =
        sqlx::query_scalar("SELECT input FROM agent_runs WHERE workflow = 'plan_goal'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        planner,
        [json!({"goal_id": goal, "trigger": "step_failed", "plan_id": v1, "step": "shortlist"})]
    );
    let old: Vec<(i64, String, Option<String>)> = sqlx::query_as(
        "SELECT id, status, last_error FROM plan_steps WHERE plan_id = $1 ORDER BY ordinal",
    )
    .bind(v1)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(old[3].1, "failed");
    assert_eq!(
        old[3].2.as_deref(),
        Some("No posting scored 60 or more (0 waiting in review)")
    );
    for (at, kind) in ["target_companies", "source", "score"].iter().enumerate() {
        let kept = step(&done, kind);
        assert_eq!(kept["kept_from"], old[at].0, "{kind}");
        assert_eq!(kept["kept_from_version"], 1, "{kind}");
        assert_eq!(kept["status"], "succeeded", "{kind}");
    }
    assert!(step(&done, "shortlist")["kept_from"].is_null());
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM jobs WHERE kind = 'source_board'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'score_fit'"
        )
        .await,
        1
    );
    let posting = engineer(&pool).await;
    assert_eq!(plan["shortlist"][0]["posting_id"], posting);
    assert_eq!(plan["shortlist"][0]["score"], 55);
    assert_eq!(plan["shortlist"][0]["band"], "partial");
    let prep = step(&done, "prep");
    assert_eq!(prep["status"], "succeeded");
    assert!(prep["kept_from"].is_null());
    assert_eq!(prep["output"]["prepared"], 1);
    assert_eq!(plan["packets"][0]["posting_id"], posting);
    assert_eq!(plan["packets"][0]["outcome"], "prepared");
}

#[sqlx::test]
async fn failing_board_stops_after_two_replans(pool: PgPool) {
    let (goal, _) = seed_goal(&pool, "planned").await;
    let server = board_server(false).await;
    let (client, _fake) = app(&pool, &server, "plan_goal").await;

    run(&client, goal).await;
    let failed = until(&client, goal, "failed").await;
    assert_eq!(failed["auto_replans"], 2);
    let error = failed["error"].as_str().expect("an error");
    assert!(
        error.starts_with("source failed: Couldn't source greenhouse/example-labs: "),
        "{error}"
    );
    assert_eq!(failed["plan"]["version"], 3);
    assert_eq!(step(&failed, "source")["status"], "failed");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'plan_goal'"
        )
        .await,
        2
    );
    // Nothing else is queued for the goal.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'plan_goal'"
        )
        .await,
        2
    );
    assert_eq!(
        get(&client, &format!("/goals/{goal}")).await["status"],
        "failed"
    );
}

#[sqlx::test]
async fn running_goal_resumes_on_start(pool: PgPool) {
    let (goal, plan) = seed_goal(&pool, "running").await;
    let posting: i64 = sqlx::query_scalar(
        "INSERT INTO job_postings (source, board, external_id, title, company, url) \
         VALUES ('greenhouse', 'example-labs', '1', 'Senior Backend Engineer', 'Example Labs', \
         'https://example.com/job') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("a posting");
    let run: i64 = sqlx::query_scalar(
        "INSERT INTO agent_runs (workflow, input, status) \
         VALUES ('score_fit', jsonb_build_object('posting_id', $1::bigint), 'succeeded') \
         RETURNING id",
    )
    .bind(posting)
    .fetch_one(&pool)
    .await
    .expect("a run");
    sqlx::query(
        "INSERT INTO posting_fit_scores (posting_id, run_id, status, score, band, confidence, \
         summary, reasons) VALUES ($1, $2, 'applied', 75, 'strong', 0.9, 'Earlier', '[]')",
    )
    .bind(posting)
    .bind(run)
    .execute(&pool)
    .await
    .expect("a score");
    // An approved packet from an earlier prep run.
    let prep_run: i64 = sqlx::query_scalar(
        "INSERT INTO agent_runs (workflow, input, status) \
         VALUES ('prep_packet', jsonb_build_object('posting_id', $1::bigint), 'succeeded') \
         RETURNING id",
    )
    .bind(posting)
    .fetch_one(&pool)
    .await
    .expect("a prep run");
    let packet: i64 = sqlx::query_scalar(
        "INSERT INTO application_packets (posting_id, run_id, status, resume_name, \
         resume_reasons, answers, confidence, approved_resume_name, approved_answers, \
         approved_at) VALUES ($1, $2, 'approved', 'Backend', '[]', '[]', 0.9, 'Backend', \
         '[]', now()) RETURNING id",
    )
    .bind(posting)
    .bind(prep_run)
    .fetch_one(&pool)
    .await
    .expect("a packet");
    for (ordinal, output) in [
        (0, json!({"companies": 1, "unverified": 0})),
        (1, json!({"boards": 1, "inserted": 1, "updated": 0})),
        (
            2,
            json!({"posting_ids": [posting], "queued": 0, "reused": 1, "in_review": 0, "failed": 0}),
        ),
    ] {
        sqlx::query(
            "UPDATE plan_steps SET status = 'succeeded', output = $3, started_at = now(), \
             finished_at = now() WHERE plan_id = $1 AND ordinal = $2",
        )
        .bind(plan)
        .bind(ordinal)
        .bind(output)
        .execute(&pool)
        .await
        .expect("finish a step");
    }

    let server = board_server(false).await;
    let (client, _fake) = app(&pool, &server, "exit1").await;
    let done = until(&client, goal, "completed").await;
    assert_eq!(
        done["plan"]["shortlist"],
        json!([{"rank": 1, "posting_id": posting, "company": "Example Labs",
                "title": "Senior Backend Engineer", "score": 75, "band": "strong"}])
    );
    let prep = step(&done, "prep");
    assert_eq!(prep["status"], "succeeded");
    assert_eq!(
        prep["output"],
        json!({"packets": [{"posting_id": posting, "packet_id": packet, "run_id": prep_run,
                            "outcome": "reused", "error": null}],
               "prepared": 0, "reused": 1, "failed": 0})
    );
    assert_eq!(done["plan"]["packets"][0]["status"], "approved");
    assert_eq!(count(&pool, "SELECT count(*) FROM claude_calls").await, 0);
}
