//! The candidate profile import: `POST /profile/linkedin` (a LinkedIn data export) and
//! `POST /resumes/{id}/profile` (the `resume_profile` job), their dedupe and reuse rules,
//! bad input and the request guards. `claude` is `fake-claude.sh`.

mod common;

use std::io::{Cursor, Write};
use std::path::Path;
use std::time::Duration;

use rocket::http::{ContentType, Header, Method, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use zip::write::SimpleFileOptions;

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ZIP_BYTES: usize = 20 * 1024 * 1024;
const MAX_CSV_BYTES: usize = 5 * 1024 * 1024;

const POSITIONS: &str = "Company Name,Title,Description,Location,Started On,Finished On\n\
                         Acme,Staff Engineer,Led the platform team.,Berlin,Jan 2020,\n\
                         Hooli,Engineer,,,2016,2019\n\
                         ,No company,,,,\n";
const SKILLS: &str = "Name\nRust\nTypeScript\n";
const EDUCATION: &str =
    "School Name,Start Date,End Date,Notes,Degree Name,Activities\nMIT,2010,2014,,BSc,\n";

/// One stored memory, in the columns these tests check.
type Row = (
    String,
    String,
    String,
    Vec<String>,
    String,
    bool,
    String,
    Option<String>,
    Option<String>,
);

/// A bad LinkedIn upload: what it is, the query, the content type, the body, the expected
/// status and the expected `(field, message)`s.
type BadUpload = (
    &'static str,
    String,
    ContentType,
    Vec<u8>,
    Status,
    Vec<(&'static str, &'static str)>,
);

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

async fn client(pool: PgPool, mode: &str) -> (Client, common::Fake) {
    let fake = common::fake(mode, TIMEOUT);
    let rocket = common::app(pool, fake.runner.clone(), common::fast_config());
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

/// A zip of `(name, content)` entries, deflated.
fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .expect("start entry");
        writer.write_all(content).expect("write entry");
    }
    writer.finish().expect("finish zip").into_inner()
}

/// The three export files, nested as LinkedIn does, with an entry that is ignored.
fn export() -> Vec<u8> {
    archive(&[
        (
            "Basic_LinkedInDataExport/Positions.csv",
            POSITIONS.as_bytes(),
        ),
        ("Basic_LinkedInDataExport/SKILLS.CSV", SKILLS.as_bytes()),
        ("education.csv", EDUCATION.as_bytes()),
        ("Basic_LinkedInDataExport/Profile.csv", b"First Name\nAda\n"),
    ])
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Posts `body` to `/profile/linkedin?<query>` with `content_type`.
async fn post_zip<'c>(
    client: &'c Client,
    query: &str,
    content_type: ContentType,
    body: &[u8],
) -> LocalResponse<'c> {
    client
        .post(format!("/profile/linkedin?{query}"))
        .header(host())
        .header(content_type)
        .body(body)
        .dispatch()
        .await
}

/// Imports `body` as `export.zip` and returns the summary.
async fn imported(client: &Client, body: &[u8]) -> Value {
    let response = post_zip(client, "filename=export.zip", ContentType::ZIP, body).await;
    assert_eq!(response.status(), Status::Ok);
    response.into_json().await.expect("summary json")
}

fn summary(added: u32, already_known: u32, skipped: u32) -> Value {
    json!({"added": added, "already_known": already_known, "skipped": skipped})
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .expect("count")
}

async fn rows(pool: &PgPool, source_kind: &str) -> Vec<Row> {
    sqlx::query_as(
        "SELECT kind, topic, content, tags, status, confirmed_at IS NOT NULL, source_kind, \
         source_ref, source_label FROM memories WHERE source_kind = $1 ORDER BY id",
    )
    .bind(source_kind)
    .fetch_all(pool)
    .await
    .expect("memories")
}

fn resume_pdf() -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resumes/resume.pdf"))
        .expect("read fixture")
}

/// Uploads the fixture resume as `name` and returns the version's id.
async fn upload_resume(client: &Client, name: &str) -> i64 {
    let response = client
        .post(format!("/resumes?name={name}"))
        .header(host())
        .header(ContentType::PDF)
        .body(resume_pdf())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Created);
    let body: Value = response.into_json().await.expect("version json");
    assert_eq!(body["has_text"], true, "the fixture has text");
    body["id"].as_i64().expect("id")
}

async fn start_import<'c>(client: &'c Client, id: i64) -> LocalResponse<'c> {
    client
        .post(format!("/resumes/{id}/profile"))
        .header(host())
        .dispatch()
        .await
}

/// Starts a resume import, expecting a 202, and returns the body.
async fn queued(client: &Client, id: i64) -> Value {
    let response = start_import(client, id).await;
    assert_eq!(response.status(), Status::Accepted);
    response.into_json().await.expect("queued json")
}

/// Polls `GET /jobs/{id}` until `done` holds or the timeout passes.
async fn wait_for_job(client: &Client, id: i64, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let job: Value = client
            .get(format!("/jobs/{id}"))
            .header(host())
            .dispatch()
            .await
            .into_json()
            .await
            .expect("job json");
        if done(&job) || tokio::time::Instant::now() > deadline {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn errors(body: &Value) -> Vec<(String, String)> {
    body["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .map(|e| {
            (
                e["field"].as_str().unwrap().to_string(),
                e["message"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

#[sqlx::test]
async fn linkedin_import_stores_confirmed_facts(pool: PgPool) {
    let (client, _fake) = client(pool.clone(), "success").await;
    let body = export();
    assert_eq!(imported(&client, &body).await, summary(5, 0, 1));

    let reference = Some(sha256(&body));
    let label = Some("export.zip".to_string());
    let fact = |topic: &str, content: &str, tags: &[&str]| -> Row {
        (
            "fact".into(),
            topic.into(),
            content.into(),
            strings(tags),
            "confirmed".into(),
            true,
            "linkedin".into(),
            reference.clone(),
            label.clone(),
        )
    };
    assert_eq!(
        rows(&pool, "linkedin").await,
        vec![
            fact(
                "position",
                "Staff Engineer at Acme · Berlin\nJan 2020 – present\n\nLed the platform team.",
                &["acme"],
            ),
            fact("position", "Engineer at Hooli\n2016 – 2019", &["hooli"]),
            fact("skill", "Rust", &[]),
            fact("skill", "TypeScript", &[]),
            fact("education", "BSc at MIT\n2010 – 2014", &["mit"]),
        ]
    );

    // No filename: the label is null.
    let other = archive(&[("Skills.csv", b"Name\nGo\n")]);
    let response = post_zip(&client, "", ContentType::ZIP, &other).await;
    assert_eq!(response.status(), Status::Ok);
    let (source_ref, source_label): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT source_ref, source_label FROM memories WHERE content = 'Go'")
            .fetch_one(&pool)
            .await
            .expect("the Go skill");
    assert_eq!(source_ref, Some(sha256(&other)));
    assert_eq!(source_label, None);
}

#[sqlx::test]
async fn linkedin_reimport_is_idempotent(pool: PgPool) {
    let (client, _fake) = client(pool.clone(), "success").await;
    let body = export();
    assert_eq!(imported(&client, &body).await, summary(5, 0, 1));
    let total = count(&pool, "memories").await;

    let (id,): (i64,) = sqlx::query_as("SELECT id FROM memories WHERE content = 'Rust'")
        .fetch_one(&pool)
        .await
        .expect("the Rust skill");
    let edit = client
        .put(format!("/memories/{id}"))
        .header(host())
        .header(ContentType::JSON)
        .body(json!({"kind": "fact", "topic": "skill", "content": "Rust (async, tokio)", "tags": ["lang"]}).to_string())
        .dispatch()
        .await;
    assert_eq!(edit.status(), Status::Ok);

    assert_eq!(imported(&client, &body).await, summary(0, 5, 1));
    assert_eq!(count(&pool, "memories").await, total, "nothing is added");
    let (content, tags): (String, Vec<String>) =
        sqlx::query_as("SELECT content, tags FROM memories WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("the edited memory");
    assert_eq!(content, "Rust (async, tokio)", "the edit survives");
    assert_eq!(tags, strings(&["lang"]));

    let newer = archive(&[
        ("Positions.csv", POSITIONS.as_bytes()),
        ("Skills.csv", format!("{SKILLS}Kotlin\n").as_bytes()),
        ("Education.csv", EDUCATION.as_bytes()),
    ]);
    assert_eq!(imported(&client, &newer).await, summary(1, 5, 1));
    assert_eq!(count(&pool, "memories").await, total + 1);

    let duplicates = archive(&[("Skills.csv", b"Name\nGo\n  go \n")]);
    assert_eq!(imported(&client, &duplicates).await, summary(1, 1, 0));
    assert_eq!(count(&pool, "memories").await, total + 2);
}

#[sqlx::test]
async fn linkedin_invalid_archives_store_nothing(pool: PgPool) {
    let (client, _fake) = client(pool.clone(), "success").await;
    let skills = SKILLS.as_bytes();
    let inflates = vec![b'a'; MAX_CSV_BYTES + 1];
    let long_name = format!("filename={}.zip", "a".repeat(252));
    let cases: Vec<BadUpload> = vec![
        (
            "an empty body",
            String::new(),
            ContentType::ZIP,
            Vec::new(),
            Status::UnprocessableEntity,
            vec![("file", "Required")],
        ),
        (
            "a non-zip body",
            String::new(),
            ContentType::ZIP,
            SKILLS.as_bytes().to_vec(),
            Status::UnprocessableEntity,
            vec![("file", "Must be a LinkedIn data export (.zip)")],
        ),
        (
            "none of the CSVs",
            String::new(),
            ContentType::ZIP,
            archive(&[("Profile.csv", b"First Name\nAda\n")]),
            Status::UnprocessableEntity,
            vec![(
                "file",
                "No Positions, Skills or Education CSV found in the archive",
            )],
        ),
        (
            "a CSV that inflates past 5 MiB, beside a good one",
            String::new(),
            ContentType::ZIP,
            archive(&[("Skills.csv", skills), ("Positions.csv", &inflates)]),
            Status::UnprocessableEntity,
            vec![("file", "Positions.csv is too large")],
        ),
        (
            "a body over 20 MiB",
            String::new(),
            ContentType::ZIP,
            vec![0; MAX_ZIP_BYTES + 1],
            Status::PayloadTooLarge,
            vec![],
        ),
        (
            "a JSON body",
            String::new(),
            ContentType::JSON,
            export(),
            Status::UnsupportedMediaType,
            vec![],
        ),
        (
            "no content type",
            String::new(),
            ContentType::Any,
            export(),
            Status::UnsupportedMediaType,
            vec![],
        ),
        (
            "a filename over 255 characters",
            long_name,
            ContentType::ZIP,
            export(),
            Status::UnprocessableEntity,
            vec![("filename", "At most 255 characters")],
        ),
    ];
    for (what, query, content_type, body, status, want) in cases {
        let response = post_zip(&client, &query, content_type, &body).await;
        assert_eq!(response.status(), status, "{what}");
        if !want.is_empty() {
            let body: Value = response.into_json().await.expect("errors json");
            let want: Vec<(String, String)> = want
                .iter()
                .map(|(f, m)| (f.to_string(), m.to_string()))
                .collect();
            assert_eq!(errors(&body), want, "{what}");
        }
        assert_eq!(count(&pool, "memories").await, 0, "{what} stores nothing");
    }
}

#[sqlx::test]
async fn resume_import_proposes_facts(pool: PgPool) {
    let (client, fake) = client(pool.clone(), "profile_resume").await;
    let linkedin = archive(&[("Skills.csv", b"Name\nRust\n")]);
    assert_eq!(imported(&client, &linkedin).await, summary(1, 0, 0));
    let id = upload_resume(&client, "Backend%20CV").await;

    let body = queued(&client, id).await;
    let job_id = body["job_id"].as_i64().expect("job_id");
    assert_eq!(body, json!({"job_id": job_id, "reused": false}));
    assert_eq!(count(&pool, "jobs").await, 1);

    let job = wait_for_job(&client, job_id, |job| job["status"] == "succeeded").await;
    assert_eq!(job["status"], "succeeded", "{job}");
    assert_eq!(job["kind"], "resume_profile");
    // "rust" was imported from LinkedIn and "typescript" repeats "TypeScript".
    assert_eq!(job["result"], summary(5, 2, 0));

    let reference = Some(id.to_string());
    let label = Some("Backend CV".to_string());
    let fact = |topic: &str, content: &str, tags: &[&str]| -> Row {
        (
            "fact".into(),
            topic.into(),
            content.into(),
            strings(tags),
            "proposed".into(),
            false,
            "resume".into(),
            reference.clone(),
            label.clone(),
        )
    };
    assert_eq!(
        rows(&pool, "resume").await,
        vec![
            fact(
                "position",
                "Staff Engineer at Acme · Berlin\nJan 2020 – present\n\nLed the platform team.",
                &["acme"],
            ),
            fact("position", "Engineer at Hooli\n2016 – 2019", &["hooli"]),
            fact("skill", "TypeScript", &[]),
            fact("skill", "Postgres", &[]),
            fact(
                "education",
                "BSc Computer Science at MIT\n2010 – 2014",
                &["mit"]
            ),
        ]
    );
    assert_eq!(
        rows(&pool, "linkedin").await.len(),
        1,
        "the LinkedIn skill wins"
    );

    let (purpose, model): (String, Option<String>) =
        sqlx::query_as("SELECT purpose, model FROM claude_calls")
            .fetch_one(&pool)
            .await
            .expect("one call");
    assert_eq!(purpose, "profile.resume");
    assert!(model.is_some_and(|m| m.contains("sonnet")));
    let argv = std::fs::read_to_string(fake.record.join("argv")).expect("argv");
    let args: Vec<&str> = argv.lines().collect();
    let schema_at = args
        .iter()
        .position(|arg| *arg == "--json-schema")
        .expect("--json-schema");
    let schema: Value = serde_json::from_str(args[schema_at + 1]).expect("schema json");
    assert_eq!(schema, waypoint_api::profile::resume::schema());
    let stdin = std::fs::read_to_string(fake.record.join("stdin")).expect("stdin");
    assert!(stdin.contains("Waypoint Fixture Resume"), "{stdin}");
}

#[sqlx::test]
async fn resume_import_reuses_earlier_job(pool: PgPool) {
    let (client, _fake) = client(pool.clone(), "profile_resume").await;
    let first = upload_resume(&client, "CV").await;
    let same_pdf = upload_resume(&client, "CV%20again").await;

    let job_id = queued(&client, first).await["job_id"]
        .as_i64()
        .expect("job_id");
    let job = wait_for_job(&client, job_id, |job| job["status"] == "succeeded").await;
    assert_eq!(job["status"], "succeeded", "{job}");
    let memories = count(&pool, "memories").await;

    let reused = json!({"job_id": job_id, "reused": true});
    for (status, version) in [
        ("succeeded", first),
        ("succeeded", same_pdf),
        ("queued", first),
        ("running", same_pdf),
    ] {
        // A far `run_at` keeps the worker from claiming a requeued row.
        sqlx::query(
            "UPDATE jobs SET status = $2, run_at = now() + interval '1 hour' WHERE id = $1",
        )
        .bind(job_id)
        .bind(status)
        .execute(&pool)
        .await
        .expect("set status");
        assert_eq!(queued(&client, version).await, reused, "{status}");
        assert_eq!(count(&pool, "jobs").await, 1, "{status}: no new job");
    }
    assert_eq!(count(&pool, "claude_calls").await, 1);
    assert_eq!(count(&pool, "memories").await, memories);

    sqlx::query("UPDATE jobs SET status = 'failed' WHERE id = $1")
        .bind(job_id)
        .execute(&pool)
        .await
        .expect("fail the job");
    let fresh = queued(&client, same_pdf).await;
    assert_eq!(fresh["reused"], false);
    assert_ne!(fresh["job_id"], json!(job_id));
    assert_eq!(count(&pool, "jobs").await, 2);
}

#[sqlx::test]
async fn resume_import_refuses_missing_or_textless(pool: PgPool) {
    let (client, _fake) = client(pool.clone(), "profile_resume").await;
    assert_eq!(
        start_import(&client, 999_999).await.status(),
        Status::NotFound
    );

    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO resume_versions (name, filename, size_bytes, sha256, pdf, text) \
         VALUES ('Scan', 'scan.pdf', 5, $1, '\\x255044462d'::bytea, NULL) RETURNING id",
    )
    .bind("0".repeat(64))
    .fetch_one(&pool)
    .await
    .expect("a textless version");
    let response = start_import(&client, id).await;
    assert_eq!(response.status(), Status::UnprocessableEntity);
    let body: Value = response.into_json().await.expect("errors json");
    assert_eq!(
        errors(&body),
        vec![(
            "text".to_string(),
            "This resume has no text to read".to_string()
        )]
    );
    assert_eq!(count(&pool, "jobs").await, 0);
    assert_eq!(count(&pool, "memories").await, 0);
}

#[sqlx::test]
async fn resume_import_wrong_output_stores_nothing(pool: PgPool) {
    let (client, _fake) = client(pool.clone(), "structured_wrong").await;
    let id = upload_resume(&client, "CV").await;
    let job_id = queued(&client, id).await["job_id"]
        .as_i64()
        .expect("job_id");
    let job = wait_for_job(&client, job_id, |job| job["status"] == "failed").await;
    assert_eq!(job["status"], "failed", "{job}");
    assert_eq!(job["attempts"], 3, "retried up to max_attempts");
    assert!(
        job["last_error"]
            .as_str()
            .is_some_and(|e| e.contains("does not match the schema")),
        "{job}"
    );
    assert_eq!(job["result"], Value::Null);
    assert_eq!(count(&pool, "memories").await, 0);
}

#[sqlx::test]
async fn profile_guards_refuse_foreign_requests(pool: PgPool) {
    let (client, _fake) = client(pool.clone(), "profile_resume").await;
    let id = upload_resume(&client, "CV").await;
    let resume = format!("/resumes/{id}/profile");
    let zip = export();

    let routes: [(&str, ContentType, &[u8]); 2] = [
        ("/profile/linkedin?filename=x.zip", ContentType::ZIP, &zip),
        (&resume, ContentType::JSON, b""),
    ];
    for (path, content_type, body) in routes {
        let response = client
            .req(Method::Post, path)
            .header(host())
            .header(content_type)
            .header(Header::new("Sec-Fetch-Site", "cross-site"))
            .body(body)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden, "cross-site {path}");

        for form in [ContentType::Form, ContentType::FormData] {
            let response = client
                .req(Method::Post, path)
                .header(host())
                .header(form.clone())
                .body("filename=x.zip")
                .dispatch()
                .await;
            assert_eq!(response.status(), Status::Forbidden, "{form} {path}");
        }

        let response = client
            .req(Method::Post, path)
            .header(Header::new("Host", "evil.example"))
            .header(ContentType::ZIP)
            .body(body)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden, "evil host {path}");
    }
    assert_eq!(count(&pool, "memories").await, 0);
    assert_eq!(count(&pool, "jobs").await, 0);
}
