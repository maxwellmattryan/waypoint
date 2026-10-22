//! The resume version routes: upload, list, read, PDF download and delete, linking a
//! version to an application, and the guards on every changing route.

use std::path::Path;
use std::time::Duration;

use rocket::http::{ContentType, Header, Method, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use waypoint_api::claude::ClaudeRunner;

const PHRASE: &str = "Waypoint Fixture Resume";
const MAX_PDF_BYTES: usize = 10 * 1024 * 1024;

/// An invalid upload: the query string, the body and the expected `(field, message)`s.
type InvalidUpload = (String, Vec<u8>, Vec<(&'static str, &'static str)>);

fn host() -> Header<'static> {
    Header::new("Host", "127.0.0.1")
}

fn fixture() -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resumes/resume.pdf"))
        .expect("read fixture")
}

async fn client(pool: PgPool) -> Client {
    // No resume route calls the runner.
    let claude = ClaudeRunner::new("claude", Duration::from_secs(1));
    let rocket = waypoint_api::rocket(
        rocket::Config::figment(),
        pool,
        claude,
        waypoint_api::jobs::Config::default(),
        waypoint_api::google::Config::default(),
        waypoint_api::capture::Config::default(),
    );
    Client::tracked(rocket).await.expect("valid rocket")
}

async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM resume_versions")
        .fetch_one(pool)
        .await
        .expect("count resume versions")
}

/// Posts `body` to `/resumes?<query>` as `application/pdf`.
async fn upload<'c>(client: &'c Client, query: &str, body: &[u8]) -> LocalResponse<'c> {
    client
        .post(format!("/resumes?{query}"))
        .header(host())
        .header(ContentType::PDF)
        .body(body)
        .dispatch()
        .await
}

/// Uploads the fixture as `name` and returns the stored version.
async fn uploaded(client: &Client, name: &str) -> Value {
    let response = upload(client, &format!("name={name}"), &fixture()).await;
    assert_eq!(response.status(), Status::Created, "{name}");
    json_body(response).await
}

/// Sends `body` as JSON with `method` to `path`.
async fn send<'c>(
    client: &'c Client,
    method: Method,
    path: &str,
    body: &Value,
) -> LocalResponse<'c> {
    client
        .req(method, path.to_string())
        .header(host())
        .header(ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await
}

async fn get<'c>(client: &'c Client, path: &str) -> LocalResponse<'c> {
    client.get(path.to_string()).header(host()).dispatch().await
}

async fn json_body(response: LocalResponse<'_>) -> Value {
    response.into_json().await.expect("json body")
}

/// Creates an application and returns it.
async fn application(client: &Client) -> Value {
    let response = send(
        client,
        Method::Post,
        "/applications",
        &json!({"company": "Acme", "title": "Engineer"}),
    )
    .await;
    assert_eq!(response.status(), Status::Created);
    json_body(response).await
}

async fn link<'c>(client: &'c Client, application: i64, version: Value) -> LocalResponse<'c> {
    send(
        client,
        Method::Put,
        &format!("/applications/{application}/resume"),
        &json!({"resume_version_id": version}),
    )
    .await
}

fn id(value: &Value) -> i64 {
    value["id"].as_i64().expect("id")
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

#[sqlx::test]
async fn upload_then_read_round_trips(pool: PgPool) {
    let client = client(pool).await;
    let pdf = fixture();
    let response = upload(
        &client,
        "name=%20Backend%20%26%20Rust%20&filename=My%20%22CV%22.pdf",
        &pdf,
    )
    .await;
    assert_eq!(response.status(), Status::Created);
    let created = json_body(response).await;

    assert_eq!(created["name"], "Backend & Rust");
    assert_eq!(created["filename"], "My \"CV\".pdf");
    assert_eq!(created["size_bytes"], pdf.len());
    assert_eq!(created["sha256"], format!("{:x}", Sha256::digest(&pdf)));
    assert_eq!(created["has_text"], true);
    assert_eq!(created["application_count"], 0);
    assert!(created["created_at"].is_string());
    assert!(created.get("text").is_none(), "the upload omits the text");
    assert!(created.get("pdf").is_none(), "the upload omits the bytes");

    let response = get(&client, &format!("/resumes/{}", id(&created))).await;
    assert_eq!(response.status(), Status::Ok);
    let detail = json_body(response).await;
    let text = detail["text"].as_str().expect("text");
    assert!(text.contains(PHRASE), "{text:?}");
    assert_eq!(text, text.trim(), "the text is trimmed");
    let mut without_text = detail.clone();
    without_text.as_object_mut().unwrap().remove("text");
    assert_eq!(without_text, created);

    let response = get(&client, &format!("/resumes/{}/pdf", id(&created))).await;
    assert_eq!(response.status(), Status::Ok);
    let headers = response.headers();
    assert_eq!(headers.get_one("Content-Type"), Some("application/pdf"));
    assert_eq!(
        headers.get_one("Content-Disposition"),
        Some("inline; filename=\"My _CV_.pdf\"")
    );
    assert_eq!(headers.get_one("X-Content-Type-Options"), Some("nosniff"));
    assert_eq!(headers.get_one("Cache-Control"), Some("no-store"));
    assert_eq!(response.into_bytes().await.expect("bytes"), pdf);

    // A blank or missing filename is stored as `resume.pdf`.
    for query in ["name=CV", "name=CV&filename=%20%20"] {
        let response = upload(&client, query, &pdf).await;
        assert_eq!(response.status(), Status::Created, "{query}");
        assert_eq!(
            json_body(response).await["filename"],
            "resume.pdf",
            "{query}"
        );
    }

    for path in ["/resumes/999999", "/resumes/999999/pdf"] {
        assert_eq!(
            get(&client, path).await.status(),
            Status::NotFound,
            "{path}"
        );
    }
}

#[sqlx::test]
async fn invalid_uploads_store_nothing(pool: PgPool) {
    let client = client(pool.clone()).await;
    let pdf = fixture();
    let long = |n: usize| "x".repeat(n);
    let unprocessable: Vec<InvalidUpload> = vec![
        ("".into(), pdf.clone(), vec![("name", "Required")]),
        (
            "name=%20%09".into(),
            pdf.clone(),
            vec![("name", "Required")],
        ),
        (
            format!("name={}&filename={}", long(201), long(256)),
            pdf.clone(),
            vec![
                ("name", "At most 200 characters"),
                ("filename", "At most 255 characters"),
            ],
        ),
        ("name=CV".into(), Vec::new(), vec![("file", "Required")]),
        (
            "name=CV".into(),
            b"<html>not a pdf</html>".to_vec(),
            vec![("file", "Must be a PDF")],
        ),
        (
            format!("filename={}", long(256)),
            b"PDF-1.4".to_vec(),
            vec![
                ("name", "Required"),
                ("filename", "At most 255 characters"),
                ("file", "Must be a PDF"),
            ],
        ),
    ];
    for (query, body, want) in unprocessable {
        let response = upload(&client, &query, &body).await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{query}");
        let want: Vec<(String, String)> = want
            .into_iter()
            .map(|(f, m)| (f.to_string(), m.to_string()))
            .collect();
        assert_eq!(errors(&json_body(response).await), want, "{query}");
    }

    // Exactly at the limits (characters, not bytes) is fine.
    let response = upload(
        &client,
        &format!("name={}&filename={}", "%C3%A9".repeat(200), long(255)),
        &pdf,
    )
    .await;
    assert_eq!(response.status(), Status::Created);
    sqlx::query("DELETE FROM resume_versions")
        .execute(&pool)
        .await
        .unwrap();

    let mut too_large = pdf.clone();
    too_large.resize(MAX_PDF_BYTES + 1, b' ');
    let response = upload(&client, "name=CV", &too_large).await;
    assert_eq!(response.status(), Status::PayloadTooLarge);

    let mut at_limit = pdf.clone();
    at_limit.resize(MAX_PDF_BYTES, b' ');
    let response = upload(&client, "name=CV", &at_limit).await;
    assert_eq!(
        response.status(),
        Status::Created,
        "exactly 10 MiB is allowed"
    );
    sqlx::query("DELETE FROM resume_versions")
        .execute(&pool)
        .await
        .unwrap();

    let response = client
        .post("/resumes?name=CV")
        .header(host())
        .header(ContentType::Plain)
        .body(&pdf)
        .dispatch()
        .await;
    assert_eq!(
        response.status(),
        Status::UnsupportedMediaType,
        "text/plain"
    );
    let response = client
        .post("/resumes?name=CV")
        .header(host())
        .body(&pdf)
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::UnsupportedMediaType, "no type");

    assert_eq!(count(&pool).await, 0, "no invalid upload stored a row");
}

#[sqlx::test]
async fn unparseable_pdf_is_kept_without_text(pool: PgPool) {
    let client = client(pool.clone()).await;
    let pdf = fixture();
    for body in [
        b"%PDF-1.4\nthis is not really a pdf".to_vec(),
        pdf[..pdf.len() / 2].to_vec(),
    ] {
        let response = upload(&client, "name=Broken", &body).await;
        assert_eq!(response.status(), Status::Created);
        let created = json_body(response).await;
        assert_eq!(created["has_text"], false);
        assert_eq!(created["size_bytes"], body.len());

        let detail = json_body(get(&client, &format!("/resumes/{}", id(&created))).await).await;
        assert_eq!(detail["text"], Value::Null);
        let response = get(&client, &format!("/resumes/{}/pdf", id(&created))).await;
        assert_eq!(response.into_bytes().await.expect("bytes"), body);
    }

    // The server is still up.
    assert_eq!(get(&client, "/health").await.status(), Status::Ok);
    assert_eq!(count(&pool).await, 2);
}

#[sqlx::test]
async fn list_orders_newest_first_with_counts(pool: PgPool) {
    let client = client(pool.clone()).await;
    let list = || async {
        let response = get(&client, "/resumes").await;
        assert_eq!(response.status(), Status::Ok);
        json_body(response).await
    };
    assert_eq!(list().await, json!([]));

    let first = uploaded(&client, "First").await;
    let second = uploaded(&client, "Second").await;
    let third = uploaded(&client, "Third").await;
    sqlx::query("UPDATE resume_versions SET created_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    let ids = |list: &Value| -> Vec<i64> { list.as_array().unwrap().iter().map(id).collect() };
    assert_eq!(
        ids(&list().await),
        vec![id(&third), id(&second), id(&first)],
        "ties fall back to id descending"
    );

    sqlx::query("UPDATE resume_versions SET created_at = now() WHERE id = $1")
        .bind(id(&first))
        .execute(&pool)
        .await
        .unwrap();
    for _ in 0..2 {
        let app = application(&client).await;
        assert_eq!(
            link(&client, id(&app), json!(id(&second))).await.status(),
            Status::Ok
        );
    }
    let app = application(&client).await;
    assert_eq!(
        link(&client, id(&app), json!(id(&first))).await.status(),
        Status::Ok
    );

    let listed = list().await;
    assert_eq!(ids(&listed), vec![id(&first), id(&third), id(&second)]);
    let counts: Vec<i64> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["application_count"].as_i64().unwrap())
        .collect();
    assert_eq!(counts, vec![1, 0, 2]);
    for item in listed.as_array().unwrap() {
        assert!(item.get("text").is_none(), "the list omits the text");
        assert_eq!(item["has_text"], true);
    }
    let detail = json_body(get(&client, &format!("/resumes/{}", id(&second))).await).await;
    assert_eq!(detail["application_count"], 2);
}

#[sqlx::test]
async fn link_resume_to_application(pool: PgPool) {
    let client = client(pool.clone()).await;
    let version = uploaded(&client, "CV").await;
    let created = application(&client).await;
    assert_eq!(
        created["resume_version_id"],
        Value::Null,
        "new applications start unlinked"
    );
    let app = id(&created);
    let age = || async {
        sqlx::query(
            "UPDATE applications SET updated_at = now() - interval '1 minute' WHERE id = $1",
        )
        .bind(app)
        .execute(&pool)
        .await
        .unwrap();
        json_body(get(&client, &format!("/applications/{app}")).await).await
    };

    let before = age().await;
    let response = link(&client, app, json!(id(&version))).await;
    assert_eq!(response.status(), Status::Ok);
    let linked = json_body(response).await;
    assert_eq!(linked["resume_version_id"], id(&version));
    assert!(linked["updated_at"].as_str() > before["updated_at"].as_str());
    assert_eq!(linked["status_changed_at"], before["status_changed_at"]);

    let aged = age().await;
    let response = link(&client, app, json!(id(&version))).await;
    assert_eq!(
        json_body(response).await,
        aged,
        "the same link changes nothing"
    );

    // Every application route returns the link, and `PUT /applications/{id}` keeps it.
    let response = send(
        &client,
        Method::Put,
        &format!("/applications/{app}"),
        &json!({"company": "Acme", "title": "Staff Engineer", "resume_version_id": null}),
    )
    .await;
    assert_eq!(json_body(response).await["resume_version_id"], id(&version));
    let response = send(
        &client,
        Method::Put,
        &format!("/applications/{app}/status"),
        &json!({"status": "applied"}),
    )
    .await;
    assert_eq!(json_body(response).await["resume_version_id"], id(&version));
    let listed = json_body(get(&client, "/applications").await).await;
    assert_eq!(listed[0]["resume_version_id"], id(&version));
    let shown = json_body(get(&client, &format!("/applications/{app}")).await).await;
    assert_eq!(shown["resume_version_id"], id(&version));

    let response = link(&client, app, json!(999_999)).await;
    assert_eq!(response.status(), Status::UnprocessableEntity);
    assert_eq!(
        errors(&json_body(response).await),
        vec![(
            "resume_version_id".to_string(),
            "No resume version has this id".to_string()
        )]
    );
    let response = link(&client, 999_999, json!(id(&version))).await;
    assert_eq!(response.status(), Status::NotFound);
    for raw in ["{}", r#"{"resume_version_id": "1"}"#, "{not json"] {
        let response = client
            .put(format!("/applications/{app}/resume"))
            .header(host())
            .header(ContentType::JSON)
            .body(raw)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::BadRequest, "{raw}");
    }
    let response = client
        .put(format!("/applications/{app}/resume"))
        .header(host())
        .header(ContentType::Plain)
        .body(json!({"resume_version_id": null}).to_string())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::UnsupportedMediaType);
    let shown = json_body(get(&client, &format!("/applications/{app}")).await).await;
    assert_eq!(shown["resume_version_id"], id(&version), "still linked");

    let before = age().await;
    let response = link(&client, app, Value::Null).await;
    assert_eq!(response.status(), Status::Ok);
    let cleared = json_body(response).await;
    assert_eq!(cleared["resume_version_id"], Value::Null);
    assert!(cleared["updated_at"].as_str() > before["updated_at"].as_str());
}

#[sqlx::test]
async fn delete_refuses_linked_versions(pool: PgPool) {
    let client = client(pool.clone()).await;
    let unlinked = uploaded(&client, "Unlinked").await;
    let path = format!("/resumes/{}", id(&unlinked));
    let response = client.delete(path.clone()).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::NoContent);
    assert_eq!(get(&client, &path).await.status(), Status::NotFound);
    let response = client.delete(path).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::NotFound);

    let linked = uploaded(&client, "Linked").await;
    let app = id(&application(&client).await);
    assert_eq!(
        link(&client, app, json!(id(&linked))).await.status(),
        Status::Ok
    );
    let path = format!("/resumes/{}", id(&linked));
    let response = client.delete(path.clone()).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::Conflict);
    assert_eq!(get(&client, &path).await.status(), Status::Ok, "it stays");
    let shown = json_body(get(&client, &format!("/applications/{app}")).await).await;
    assert_eq!(shown["resume_version_id"], id(&linked));

    assert_eq!(link(&client, app, Value::Null).await.status(), Status::Ok);
    let response = client.delete(path.clone()).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::NoContent);
    assert_eq!(count(&pool).await, 0);
}

#[sqlx::test]
async fn guards_refuse_foreign_requests(pool: PgPool) {
    let client = client(pool.clone()).await;
    let pdf = fixture();
    let version = uploaded(&client, "CV").await;
    let created = application(&client).await;
    let one = format!("/resumes/{}", id(&version));
    let pdf_path = format!("{one}/pdf");
    let resume = format!("/applications/{}/resume", id(&created));
    let link_body = json!({"resume_version_id": id(&version)})
        .to_string()
        .into_bytes();
    let snapshot = || async {
        (
            count(&pool).await,
            json_body(get(&client, &format!("/applications/{}", id(&created))).await).await,
        )
    };
    let before = snapshot().await;

    let mutating: [(Method, &str, ContentType, &[u8]); 3] = [
        (Method::Post, "/resumes?name=Evil", ContentType::PDF, &pdf),
        (Method::Delete, &one, ContentType::JSON, b""),
        (Method::Put, &resume, ContentType::JSON, &link_body),
    ];
    for (method, path, content_type, body) in mutating {
        let response = client
            .req(method, path)
            .header(host())
            .header(content_type.clone())
            .header(Header::new("Sec-Fetch-Site", "cross-site"))
            .body(body)
            .dispatch()
            .await;
        assert_eq!(
            response.status(),
            Status::Forbidden,
            "cross-site {method} {path}"
        );

        for form in [ContentType::Form, ContentType::FormData] {
            let response = client
                .req(method, path)
                .header(host())
                .header(form.clone())
                .body("resume_version_id=1")
                .dispatch()
                .await;
            assert_eq!(
                response.status(),
                Status::Forbidden,
                "{form} {method} {path}"
            );
        }

        let response = client
            .req(method, path)
            .header(Header::new("Host", "evil.example"))
            .header(content_type)
            .body(body)
            .dispatch()
            .await;
        assert_eq!(
            response.status(),
            Status::Forbidden,
            "evil host {method} {path}"
        );
    }

    // What any web page can auto-submit: a form POST Rocket turns into DELETE or PUT.
    for (path, override_) in [(&one, "delete"), (&resume, "put")] {
        let response = client
            .post(path.clone())
            .header(host())
            .header(ContentType::Form)
            .body(format!("_method={override_}&resume_version_id=1"))
            .dispatch()
            .await;
        assert_eq!(
            response.status(),
            Status::Forbidden,
            "_method={override_} {path}"
        );
    }

    for path in ["/resumes", &one, &pdf_path] {
        let response = client
            .get(path.to_string())
            .header(Header::new("Host", "evil.example"))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden, "evil host GET {path}");
    }

    assert_eq!(
        snapshot().await,
        before,
        "nothing was stored, linked or deleted"
    );
    assert_eq!(get(&client, &one).await.status(), Status::Ok);
}
