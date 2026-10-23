//! Goals and the `plan_goal` planner end to end: `POST /goals` queues a run, the worker plays
//! `fake-claude.sh`, and the questions, plans, review items and failures it stores; then
//! the answers, replan, edit and run routes and their guards.

mod common;

use std::time::Duration;

use rocket::http::{ContentType, Header, Method, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::goals::store::{self, PlanDraft};
use waypoint_api::goals::{
    PlanAuthor, PlanCompanyConfig, PlanConfig, PlanDropped, PlanStatus, PlanTrigger,
};
use waypoint_api::sourcing::Source;

const TIMEOUT: Duration = Duration::from_secs(20);
const GOAL: &str = "Rust engineering roles at Example Labs";

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

/// The API with the production runtime and a runner playing `mode`. Keep the `Fake` alive
/// as long as the client.
async fn app(pool: &PgPool, mode: &str) -> (Client, common::Fake) {
    let fake = common::fake(mode, Duration::from_secs(30));
    let rocket = common::app(pool.clone(), fake.runner.clone(), common::fast_config());
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

/// `(status, JSON body or null)` of a request with an optional JSON body.
async fn send(client: &Client, method: Method, path: &str, body: Option<Value>) -> (Status, Value) {
    let mut request = client.req(method, path.to_owned()).header(host());
    if let Some(body) = &body {
        request = request.header(ContentType::JSON).body(body.to_string());
    }
    let response = request.dispatch().await;
    let status = response.status();
    let body = response.into_json().await.unwrap_or(Value::Null);
    (status, body)
}

async fn get(client: &Client, path: &str) -> Value {
    let (status, body) = send(client, Method::Get, path, None).await;
    assert_eq!(status, Status::Ok, "GET {path}: {body}");
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

/// `(input, output)` of the `plan_goal` runs, oldest first.
async fn planner_runs(pool: &PgPool) -> Vec<(Value, Option<Value>)> {
    sqlx::query_as("SELECT input, output FROM agent_runs WHERE workflow = 'plan_goal' ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("read the runs")
}

/// The output of `plan_goal` run number `index`, once the executor stores it. The step
/// commits the goal's new status before the executor records the run's output.
async fn planner_output(pool: &PgPool, index: usize) -> Value {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        if let Some(Some(output)) = planner_runs(pool).await.get(index).map(|run| run.1.clone()) {
            return output;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "plan_goal run {index} never stored an output"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The prompt of the fake's latest call.
fn last_prompt(fake: &common::Fake) -> String {
    let stdin = std::fs::read_to_string(fake.record.join("stdin")).expect("stdin");
    let line: Value = serde_json::from_str(stdin.lines().next().expect("a line")).expect("json");
    line["message"]["content"][0]["text"]
        .as_str()
        .expect("a text prompt")
        .to_owned()
}

/// One live posting on greenhouse/example-labs.
async fn seed_posting(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO job_postings (source, board, external_id, title, company, url) \
         VALUES ('greenhouse', 'example-labs', '1', 'Senior Backend Engineer', 'Example Labs', \
         'https://example.com/job') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert a posting")
}

/// The config `plan_goal.jsonl` answers with.
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

/// A goal with `status` and a `ready` planner plan of [`config`]; returns `(goal, plan)`.
async fn seed_planned(pool: &PgPool, status: &str) -> (i64, i64) {
    let mut tx = pool.begin().await.expect("begin");
    let goal: i64 =
        sqlx::query_scalar("INSERT INTO goals (text, status) VALUES ($1, $2) RETURNING id")
            .bind(GOAL)
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

/// A `PUT /goals/{id}/plan` body for [`config`] with `base` and `min_score`.
fn edit(base: i64, min_score: i64) -> Value {
    json!({
        "base_plan_id": base,
        "companies": [{"name": "Example Labs", "source": "greenhouse", "board": "example-labs",
                       "reason": "Builds the platform the goal names"}],
        "title_keywords": ["Engineer"],
        "locations": [],
        "remote_only": false,
        "max_to_score": 10,
        "shortlist_size": 5,
        "min_score": min_score,
        "prep_notes": "Lead with Rust and Postgres work",
    })
}

#[sqlx::test]
async fn planner_asks_then_plans(pool: PgPool) {
    sqlx::query(
        "INSERT INTO memories (kind, topic, content, status, confirmed_at) VALUES \
         ('preference', 'target_role', 'Senior engineering roles', 'confirmed', now()), \
         ('fact', 'skill', 'Rust compiler work', 'proposed', NULL), \
         ('fact', 'skill', 'Shipped Rust services', 'confirmed', now())",
    )
    .execute(&pool)
    .await
    .expect("seed memories");
    seed_posting(&pool).await;
    let (client, fake) = app(&pool, "route_goals_planner").await;

    let (status, created) = send(
        &client,
        Method::Post,
        "/goals",
        Some(json!({"text": format!("  {GOAL}  ")})),
    )
    .await;
    assert_eq!(status, Status::Created, "{created}");
    assert_eq!(created["status"], "planning");
    assert_eq!(created["text"], GOAL);
    let id = created["id"].as_i64().expect("an id");
    let runs = planner_runs(&pool).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, json!({"goal_id": id, "trigger": "goal"}));

    // The first answer asks two questions.
    let asked = until(&client, id, "needs_answers").await;
    let questions: Vec<&str> = asked["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .map(|q| q["question"].as_str().expect("text"))
        .collect();
    assert_eq!(
        questions,
        [
            "Which kind of role are you after?",
            "Which cities, or remote?"
        ]
    );
    assert!(asked["plan"].is_null());
    let first = asked["questions"][0]["id"].as_i64().unwrap();
    let second = asked["questions"][1]["id"].as_i64().unwrap();
    assert_eq!(
        planner_output(&pool, 0).await,
        json!({"goal_id": id, "kind": "questions", "question_ids": [first, second]})
    );

    // Answering plans again with the answers in the prompt.
    let (status, answered) = send(
        &client,
        Method::Post,
        &format!("/goals/{id}/answers"),
        Some(
            json!({"answers": [{"question_id": first, "answer": " Backend "},
                                {"question_id": second}]}),
        ),
    )
    .await;
    assert_eq!(status, Status::Accepted, "{answered}");
    assert_eq!(answered["status"], "planning");
    assert_eq!(answered["questions"][0]["answer"], "Backend");
    assert!(answered["questions"][1]["answer"].is_null());
    assert!(answered["questions"][1]["answered_at"].is_string());
    let planned = until(&client, id, "planned").await;
    let runs = planner_runs(&pool).await;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[1].0, json!({"goal_id": id, "trigger": "goal"}));

    let prompt = last_prompt(&fake);
    for needle in [
        "<goal>\nRust engineering roles at Example Labs\n</goal>",
        "Q: Which kind of role are you after?\nA: Backend",
        "Q: Which cities, or remote?\nA: (skipped)",
        "- greenhouse/example-labs · Example Labs (1 live postings)",
        "[memory 1 · preference · target_role] Senior engineering roles",
        "[memory 3 · fact · skill] Shipped Rust services",
        "You may not ask questions now",
    ] {
        assert!(prompt.contains(needle), "{needle} in {prompt}");
    }
    assert!(!prompt.contains("Rust compiler work"), "{prompt}");
    let argv = std::fs::read_to_string(fake.record.join("argv")).expect("argv");
    let args: Vec<&str> = argv.lines().collect();
    let after = |flag: &str| -> Option<&str> {
        let at = args.iter().position(|arg| *arg == flag)?;
        args.get(at + 1).copied()
    };
    assert_eq!(after("--tools"), Some(""), "{args:?}");
    assert_eq!(after("--model"), Some("sonnet"), "{args:?}");
    assert!(args.contains(&"--json-schema"), "{args:?}");

    // Version 1: ready, by the planner, five pending steps in order.
    let plan = &planned["plan"];
    assert_eq!(plan["version"], 1);
    assert_eq!(plan["status"], "ready");
    assert_eq!(plan["author"], "planner");
    assert_eq!(plan["trigger"], "goal");
    assert_eq!(plan["min_score"], 60);
    assert_eq!(plan["companies"][0]["verified"], true);
    let steps: Vec<(&str, &str)> = plan["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .map(|s| (s["kind"].as_str().unwrap(), s["status"].as_str().unwrap()))
        .collect();
    assert_eq!(
        steps,
        [
            ("target_companies", "pending"),
            ("source", "pending"),
            ("score", "pending"),
            ("shortlist", "pending"),
            ("prep", "pending"),
        ]
    );
    let call: (i64, f64) =
        sqlx::query_as("SELECT id, cost_usd FROM claude_calls ORDER BY id DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("a call");
    assert_eq!(plan["claude_call_id"], call.0);
    assert_eq!(plan["cost_usd"], call.1);
    assert!(call.1 > 0.0);
    let plan_id = plan["id"].as_i64().unwrap();
    assert_eq!(
        planner_output(&pool, 1).await,
        json!({
            "goal_id": id, "kind": "plan", "plan_id": plan_id, "version": 1,
            "status": "ready", "review_item_id": null, "companies": 1, "unverified": 0,
            "dropped": {"companies": 0, "keywords": 0, "locations": 0},
        })
    );
    let second_run: i64 = sqlx::query_scalar(
        "SELECT id FROM agent_runs WHERE workflow = 'plan_goal' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("the run");
    let used: Vec<i64> =
        sqlx::query_scalar("SELECT memory_id FROM run_memories WHERE run_id = $1 ORDER BY id")
            .bind(second_run)
            .fetch_all(&pool)
            .await
            .expect("run_memories");
    assert_eq!(used, [1, 3]);
    assert_eq!(get(&client, "/goals").await[0]["id"], id);

    // A user replan with an instruction writes version 2.
    let (status, replanned) = send(
        &client,
        Method::Post,
        &format!("/goals/{id}/replan"),
        Some(json!({"instruction": " Focus on Series B "})),
    )
    .await;
    assert_eq!(status, Status::Accepted, "{replanned}");
    assert_eq!(replanned["status"], "planning");
    let runs = planner_runs(&pool).await;
    assert_eq!(
        runs[2].0,
        json!({"goal_id": id, "trigger": "user_replan", "plan_id": plan_id,
               "instruction": "Focus on Series B"})
    );
    let planned = until(&client, id, "planned").await;
    assert_eq!(planned["plan"]["version"], 2);
    assert_eq!(planned["plan"]["trigger"], "user_replan");
    assert_eq!(planned["versions"].as_array().map(Vec::len), Some(2));
    let prompt = last_prompt(&fake);
    assert!(
        prompt.contains("<instruction>\nFocus on Series B\n</instruction>"),
        "{prompt}"
    );
    assert!(prompt.contains("<plan>\n{"), "{prompt}");
}

#[sqlx::test]
async fn unsure_plan_goes_to_review(pool: PgPool) {
    seed_posting(&pool).await;
    // Every board fetch answers 404, so a run fails at `source` and replans.
    let server = wiremock::MockServer::start().await;
    let mut capture = waypoint_api::capture::Config::default();
    capture.greenhouse.api = server.uri();
    let fake = common::fake("plan_goal_unsure", Duration::from_secs(30));
    let rocket = common::app_with_capture(
        pool.clone(),
        fake.runner.clone(),
        common::fast_config(),
        capture,
    );
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let (status, created) =
        send(&client, Method::Post, "/goals", Some(json!({"text": GOAL}))).await;
    assert_eq!(status, Status::Created, "{created}");
    let id = created["id"].as_i64().unwrap();
    let planned = until(&client, id, "planned").await;
    let plan = &planned["plan"];
    assert_eq!(plan["status"], "in_review");
    let plan_id = plan["id"].as_i64().unwrap();
    let item = plan["review_item_id"].as_i64().expect("a review item");
    let items = get(&client, "/review-items").await;
    assert_eq!(items.as_array().map(Vec::len), Some(1));
    assert_eq!(items[0]["id"], item);
    assert_eq!(items[0]["kind"], "plan_goal.plan");
    assert_eq!(items[0]["confidence"], 0.6);
    let payload = &items[0]["payload"];
    assert_eq!(payload["goal_id"], id);
    assert_eq!(payload["plan_id"], plan_id);
    assert_eq!(payload["version"], 1);
    assert_eq!(payload["goal"], GOAL);
    assert_eq!(payload["summary"], "A guess at engineering roles");
    assert_eq!(payload["companies"][0]["board"], "example-labs");
    assert_eq!(payload["dropped"]["companies"][0]["board"], "bad slug!");
    assert_eq!(payload["dropped"]["companies"][0]["name"], "Bad Slug Co");

    // Run waits for the review.
    let run = format!("/goals/{id}/run");
    let (status, body) = send(&client, Method::Post, &run, Some(json!({}))).await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(body["message"], "This plan needs review before it can run");

    // Apply makes it ready, once.
    let accept = format!("/review-items/{item}/accept");
    let (status, _) = send(&client, Method::Post, &accept, Some(json!({}))).await;
    assert_eq!(status, Status::Ok);
    assert_eq!(
        get(&client, &format!("/goals/{id}")).await["plan"]["status"],
        "ready"
    );
    let (status, _) = send(&client, Method::Post, &accept, Some(json!({}))).await;
    assert_eq!(status, Status::Conflict);
    let (status, body) = send(&client, Method::Post, &run, Some(json!({}))).await;
    assert_eq!(status, Status::Accepted, "{body}");
    assert_eq!(body["status"], "running");

    // The source step fails, the replan is unsure again: version 2 waits for review.
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    let second = loop {
        let goal = get(&client, &format!("/goals/{id}")).await;
        if goal["status"] == "planned" && goal["plan"]["version"] == 2 {
            break goal;
        }
        assert!(tokio::time::Instant::now() < deadline, "{goal}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(second["plan"]["status"], "in_review");
    assert_eq!(second["plan"]["trigger"], "step_failed");
    assert_eq!(second["auto_replans"], 1);
    let second_item = second["plan"]["review_item_id"].as_i64().unwrap();
    let (status, _) = send(
        &client,
        Method::Post,
        &format!("/review-items/{second_item}/dismiss"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::Ok);
    let dismissed = get(&client, &format!("/goals/{id}")).await;
    assert_eq!(dismissed["plan"]["status"], "dismissed");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM memory_signals WHERE kind = 'dismissal' \
             AND subject = 'plan_goal.plan'"
        )
        .await,
        1
    );
    let (status, body) = send(&client, Method::Post, &run, Some(json!({}))).await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(
        body["message"],
        "You dismissed this plan. Edit it or replan"
    );
}

#[sqlx::test]
async fn invalid_plan_fails_the_goal(pool: PgPool) {
    let (client, _fake) = app(&pool, "route_goals_invalid").await;

    let (status, created) =
        send(&client, Method::Post, "/goals", Some(json!({"text": GOAL}))).await;
    assert_eq!(status, Status::Created, "{created}");
    let id = created["id"].as_i64().unwrap();
    let failed = until(&client, id, "failed").await;
    let error = failed["error"].as_str().expect("an error");
    assert!(error.starts_with("Planning failed: "), "{error}");
    assert!(error.contains("no valid company is left"), "{error}");
    assert!(failed["plan"].is_null());
    assert_eq!(failed["run"]["status"], "failed");
    assert_eq!(count(&pool, "SELECT count(*) FROM plans").await, 0);

    // A goal that already had its questions can't get more.
    let asked: i64 = sqlx::query_scalar(
        "INSERT INTO goals (text, status) VALUES ('find me a job', 'failed') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("a goal");
    sqlx::query(
        "INSERT INTO goal_questions (goal_id, ordinal, question, answer, answered_at) \
         VALUES ($1, 1, 'Which kind of role are you after?', 'Backend', now())",
    )
    .bind(asked)
    .execute(&pool)
    .await
    .expect("a question");
    let (status, body) = send(
        &client,
        Method::Post,
        &format!("/goals/{asked}/replan"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::Accepted, "{body}");
    assert_eq!(
        planner_runs(&pool).await.last().unwrap().0,
        json!({"goal_id": asked, "trigger": "goal"})
    );
    let failed = until(&client, asked, "failed").await;
    let error = failed["error"].as_str().expect("an error");
    assert!(error.contains("questions are not allowed"), "{error}");
    assert_eq!(
        failed["questions"].as_array().map(Vec::len),
        Some(1),
        "no new questions"
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM plans").await, 0);
}

#[sqlx::test]
async fn editing_the_plan_writes_a_version(pool: PgPool) {
    let (goal, v1) = seed_planned(&pool, "planned").await;
    let (client, _fake) = app(&pool, "exit1").await;
    let path = format!("/goals/{goal}/plan");

    let (status, edited) = send(&client, Method::Put, &path, Some(edit(v1, 40))).await;
    assert_eq!(status, Status::Ok, "{edited}");
    let plan = &edited["plan"];
    assert_eq!(plan["version"], 2);
    assert_eq!(plan["author"], "user");
    assert_eq!(plan["trigger"], "user_edit");
    assert_eq!(plan["status"], "ready");
    assert_eq!(plan["min_score"], 40);
    assert!(plan["confidence"].is_null());
    assert!(plan["run_id"].is_null());
    assert_eq!(plan["summary"], "Engineering roles at Example Labs");
    assert_eq!(edited["status"], "planned");
    assert_eq!(edited["versions"].as_array().map(Vec::len), Some(2));
    let v2 = plan["id"].as_i64().unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM claude_calls").await, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM agent_runs").await, 0);

    // The same plan again changes nothing.
    let (status, same) = send(&client, Method::Put, &path, Some(edit(v2, 40))).await;
    assert_eq!(status, Status::Ok, "{same}");
    assert_eq!(same["plan"]["id"], v2);
    assert_eq!(count(&pool, "SELECT count(*) FROM plans").await, 2);

    // A stale base is refused.
    let (status, body) = send(&client, Method::Put, &path, Some(edit(v1, 30))).await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(body["message"], "This plan changed since you opened it");
    assert_eq!(count(&pool, "SELECT count(*) FROM plans").await, 2);

    // Every failing field is named.
    let mut invalid = edit(v2, 101);
    invalid["companies"] = json!([
        {"name": "Example Labs", "source": "greenhouse", "board": "example-labs"},
        {"name": "Bad", "source": "lever", "board": "bad slug"},
        {"name": " ", "source": "workday", "board": "example-labs"},
        {"name": "Dup", "source": "greenhouse", "board": "example-labs"},
    ]);
    invalid["shortlist_size"] = json!(0);
    invalid["max_to_score"] = json!(51);
    invalid["title_keywords"] = json!(["Engineer", "engineer", ""]);
    invalid["prep_notes"] = json!("p".repeat(1001));
    let (status, body) = send(&client, Method::Put, &path, Some(invalid)).await;
    assert_eq!(status, Status::UnprocessableEntity, "{body}");
    let errors: Vec<(String, String)> = body["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .map(|e| {
            (
                e["field"].as_str().unwrap().into(),
                e["message"].as_str().unwrap().into(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = [
        (
            "companies[1].board",
            "Must be 1–100 letters, digits, hyphens or underscores",
        ),
        ("companies[2].name", "Required"),
        ("companies[2].source", "Must be greenhouse, lever or ashby"),
        ("companies[3].board", "Duplicate board"),
        ("title_keywords[1]", "Duplicate keyword"),
        ("title_keywords[2]", "Required"),
        ("max_to_score", "Must be 1 to 50"),
        ("shortlist_size", "Must be 1 to 20"),
        ("min_score", "Must be 0 to 100"),
        ("prep_notes", "At most 1000 characters"),
    ]
    .iter()
    .map(|(f, m)| (f.to_string(), m.to_string()))
    .collect();
    assert_eq!(errors, expected);
    let mut empty = edit(v2, 40);
    empty["companies"] = json!([]);
    empty["title_keywords"] = json!([]);
    empty.as_object_mut().unwrap().remove("base_plan_id");
    let (status, body) = send(&client, Method::Put, &path, Some(empty)).await;
    assert_eq!(status, Status::UnprocessableEntity, "{body}");
    let fields: Vec<&str> = body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, ["base_plan_id", "companies", "title_keywords"]);
    assert_eq!(count(&pool, "SELECT count(*) FROM plans").await, 2);

    // A completed goal becomes planned again.
    sqlx::query("UPDATE goals SET status = 'completed' WHERE id = $1")
        .bind(goal)
        .execute(&pool)
        .await
        .unwrap();
    let (status, body) = send(&client, Method::Put, &path, Some(edit(v2, 50))).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["status"], "planned");
    assert_eq!(body["plan"]["version"], 3);

    // Planning and waiting for answers refuse edits.
    for status in ["planning", "needs_answers"] {
        sqlx::query("UPDATE goals SET status = $2 WHERE id = $1")
            .bind(goal)
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
        let current = body["plan"]["id"].as_i64().unwrap();
        let (code, refused) = send(&client, Method::Put, &path, Some(edit(current, 45))).await;
        assert_eq!(code, Status::Conflict, "{status}");
        assert_eq!(
            refused["message"],
            "The plan can't be edited while Waypoint is planning"
        );
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM plans").await, 3);
    let (status, _) = send(&client, Method::Put, "/goals/999/plan", Some(edit(v2, 40))).await;
    assert_eq!(status, Status::NotFound);
}

#[sqlx::test]
async fn goal_routes_refuse_cross_site_and_forms(pool: PgPool) {
    let (goal, plan) = seed_planned(&pool, "planned").await;
    let (client, _fake) = app(&pool, "exit1").await;
    let counts = |pool: PgPool| async move {
        let mut counts = Vec::new();
        for table in [
            "goals",
            "goal_questions",
            "plans",
            "plan_steps",
            "agent_runs",
            "jobs",
        ] {
            counts.push(count(&pool, &format!("SELECT count(*) FROM {table}")).await);
        }
        let status: String = sqlx::query_scalar("SELECT status FROM goals")
            .fetch_one(&pool)
            .await
            .unwrap();
        (counts, status)
    };
    let before = counts(pool.clone()).await;

    let routes = [
        (Method::Post, "/goals".to_owned(), json!({"text": GOAL})),
        (
            Method::Post,
            format!("/goals/{goal}/answers"),
            json!({"answers": []}),
        ),
        (Method::Put, format!("/goals/{goal}/plan"), edit(plan, 40)),
        (Method::Post, format!("/goals/{goal}/run"), json!({})),
        (Method::Post, format!("/goals/{goal}/replan"), json!({})),
    ];
    for (method, path, body) in &routes {
        let response = client
            .req(*method, path.clone())
            .header(host())
            .header(ContentType::JSON)
            .header(Header::new("Sec-Fetch-Site", "cross-site"))
            .body(body.to_string())
            .dispatch()
            .await;
        assert_eq!(
            response.status(),
            Status::Forbidden,
            "cross-site {method} {path}"
        );
        for (form, text) in [
            (ContentType::Form, "a=b".to_owned()),
            (ContentType::FormData, "a=b".to_owned()),
            (ContentType::Plain, body.to_string()),
        ] {
            let response = client
                .req(*method, path.clone())
                .header(host())
                .header(form.clone())
                .body(text)
                .dispatch()
                .await;
            assert_eq!(
                response.status(),
                Status::Forbidden,
                "{form} {method} {path}"
            );
        }
    }
    // Rocket would turn this form POST into a PUT.
    let response = client
        .post(format!("/goals/{goal}/plan"))
        .header(host())
        .header(ContentType::Form)
        .body("_method=put&base_plan_id=1")
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Forbidden, "_method=put");

    // A non-local Host is refused on every goal route.
    let mut every: Vec<(Method, String)> = routes
        .iter()
        .map(|(method, path, _)| (*method, path.clone()))
        .collect();
    every.push((Method::Get, "/goals".into()));
    every.push((Method::Get, format!("/goals/{goal}")));
    for ((method, path), body) in every.iter().zip(
        routes
            .iter()
            .map(|(_, _, body)| body.clone())
            .chain([json!({}), json!({})]),
    ) {
        let response = client
            .req(*method, path.clone())
            .header(Header::new("Host", "evil.example"))
            .header(ContentType::JSON)
            .body(body.to_string())
            .dispatch()
            .await;
        assert_eq!(
            response.status(),
            Status::Forbidden,
            "evil host {method} {path}"
        );
    }
    assert_eq!(counts(pool.clone()).await, before);
}
