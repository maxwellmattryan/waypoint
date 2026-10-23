//! The read-only `GET /postings` and `GET /postings/{id}` routes over `job_postings`,
//! seeded straight into the table.

mod common;

use std::time::Duration;

use rocket::http::{Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;

/// One row to insert; `None` columns stay NULL.
#[derive(Clone, Default)]
struct Seed {
    source: &'static str,
    company: &'static str,
    location_raw: Option<&'static str>,
    city: Option<&'static str>,
    region: Option<&'static str>,
    country: Option<&'static str>,
    remote: bool,
    removed: bool,
}

impl Seed {
    fn new(source: &'static str, company: &'static str) -> Self {
        Self {
            source,
            company,
            ..Default::default()
        }
    }
}

/// Inserts the rows in order and returns their ids.
async fn seed(pool: &PgPool, rows: &[Seed]) -> Vec<i64> {
    let mut ids = Vec::new();
    for (n, row) in rows.iter().enumerate() {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO job_postings (source, board, external_id, title, company, url, \
             location_raw, city, region, country, remote, description, removed_at) \
             VALUES ($1, 'example-labs', $2, $3, $4, 'https://boards.example/' || $2, \
             $5, $6, $7, $8, $9, 'About the role', CASE WHEN $10 THEN now() END) \
             RETURNING id",
        )
        .bind(row.source)
        .bind(format!("ext-{n}"))
        .bind(format!("Role {n}"))
        .bind(row.company)
        .bind(row.location_raw)
        .bind(row.city)
        .bind(row.region)
        .bind(row.country)
        .bind(row.remote)
        .bind(row.removed)
        .fetch_one(pool)
        .await
        .expect("insert a posting");
        ids.push(id);
    }
    ids
}

async fn client(pool: &PgPool) -> (Client, common::Fake) {
    let fake = common::fake("success", Duration::from_secs(10));
    let rocket = common::app(pool.clone(), fake.runner.clone(), common::fast_config());
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

/// `(status, JSON body or null)` of `GET path`.
async fn get(client: &Client, path: &str) -> (Status, Value) {
    let response = client
        .get(path.to_owned())
        .header(Header::new("Host", common::HOST))
        .dispatch()
        .await;
    let status = response.status();
    let body = response.into_json().await.unwrap_or(Value::Null);
    (status, body)
}

/// The listed ids of `GET path`, which must succeed.
async fn ids(client: &Client, path: &str) -> Vec<i64> {
    let (status, body) = get(client, path).await;
    assert_eq!(status, Status::Ok, "GET {path}: {body}");
    body.as_array()
        .expect("an array")
        .iter()
        .map(|posting| posting["id"].as_i64().expect("id"))
        .collect()
}

#[sqlx::test]
async fn lists_newest_first_and_pages(pool: PgPool) {
    let (client, _fake) = client(&pool).await;
    assert!(ids(&client, "/postings").await.is_empty());

    let mut removed = Seed::new("lever", "Gone Co");
    removed.removed = true;
    let rows = [
        Seed::new("greenhouse", "Acme"),
        Seed::new("lever", "Beta"),
        removed,
        Seed::new("ashby", "Gamma"),
        Seed::new("greenhouse", "Delta"),
    ];
    let seeded = seed(&pool, &rows).await;
    let live: Vec<i64> = [4, 3, 1, 0].iter().map(|&i| seeded[i]).collect();
    let all: Vec<i64> = seeded.iter().rev().copied().collect();

    assert_eq!(ids(&client, "/postings").await, live);
    assert_eq!(ids(&client, "/postings?include_removed=false").await, live);
    assert_eq!(ids(&client, "/postings?include_removed=").await, live);
    assert_eq!(ids(&client, "/postings?include_removed=true").await, all);

    // Pages of two, following `before`, cover every live posting once.
    let mut paged: Vec<i64> = Vec::new();
    let mut path = "/postings?limit=2".to_string();
    loop {
        let page = ids(&client, &path).await;
        assert!(page.len() <= 2, "{path}: {page:?}");
        let Some(&last) = page.last() else { break };
        paged.extend(&page);
        path = format!("/postings?limit=2&before={last}");
    }
    assert_eq!(paged, live);
    assert_eq!(ids(&client, "/postings?limit=0").await, live[..1]);
    assert_eq!(ids(&client, "/postings?limit=1000").await, live);

    let (_, body) = get(&client, "/postings").await;
    let first = body[0].as_object().expect("a posting");
    assert!(!first.contains_key("description"), "{first:?}");
    assert_eq!(first["source"], "greenhouse");
    assert_eq!(first["company"], "Delta");
    assert_eq!(first["removed_at"], Value::Null);
    assert_eq!(first["apply_url"], Value::Null);
}

#[sqlx::test]
async fn filters_postings(pool: PgPool) {
    let (client, _fake) = client(&pool).await;
    let row = |source, company, location_raw, city, region, country, remote| Seed {
        source,
        company,
        location_raw,
        city,
        region,
        country,
        remote,
        removed: false,
    };
    let seeded = seed(
        &pool,
        &[
            // 0: matched by its raw location only.
            row(
                "greenhouse",
                "Example Labs",
                Some("Lisbon office"),
                None,
                None,
                None,
                false,
            ),
            // 1: matched by its city only.
            row(
                "lever",
                "Acme Robotics",
                None,
                Some("Berlin"),
                None,
                None,
                false,
            ),
            // 2: matched by its region only.
            row(
                "ashby",
                "Example Cloud",
                None,
                None,
                Some("Ontario"),
                None,
                true,
            ),
            // 3: matched by its country only.
            row(
                "greenhouse",
                "100% Coffee",
                None,
                None,
                None,
                Some("Japan"),
                true,
            ),
            // 4: no location at all.
            row("lever", "1000 Coffee", None, None, None, None, false),
        ],
    )
    .await;
    let want = |indexes: &[usize]| -> Vec<i64> {
        let mut ids: Vec<i64> = indexes.iter().map(|&i| seeded[i]).collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids
    };

    let cases: &[(&str, &[usize])] = &[
        ("company=example", &[0, 2]),
        ("company=EXAMPLE%20LABS", &[0]),
        ("source=greenhouse", &[0, 3]),
        ("source=lever", &[1, 4]),
        ("source=ashby", &[2]),
        ("location=LISBON", &[0]),
        ("location=berl", &[1]),
        ("location=ontario", &[2]),
        ("location=japan", &[3]),
        ("remote=true", &[2, 3]),
        ("remote=false", &[0, 1, 4]),
        ("company=coffee&remote=false", &[4]),
        (
            "company=example&source=ashby&remote=true&location=ont",
            &[2],
        ),
        ("company=example&source=greenhouse&remote=true", &[]),
        ("company=", &[0, 1, 2, 3, 4]),
        ("company=%20%20&location=&source=&remote=", &[0, 1, 2, 3, 4]),
        ("company=100%25", &[3]),
        ("company=_", &[]),
    ];
    for (query, expected) in cases {
        assert_eq!(
            ids(&client, &format!("/postings?{query}")).await,
            want(expected),
            "{query}"
        );
    }
}

#[sqlx::test]
async fn rejects_bad_filters(pool: PgPool) {
    let (client, _fake) = client(&pool).await;
    let error = |field: &str, message: &str| json!({"field": field, "message": message});
    let cases = [
        ("source=workday", vec![error("source", "Unknown source")]),
        ("source=Greenhouse", vec![error("source", "Unknown source")]),
        ("remote=yes", vec![error("remote", "Must be true or false")]),
        (
            "include_removed=1",
            vec![error("include_removed", "Must be true or false")],
        ),
        (
            "source=x&remote=TRUE&include_removed=no&company=ok",
            vec![
                error("source", "Unknown source"),
                error("remote", "Must be true or false"),
                error("include_removed", "Must be true or false"),
            ],
        ),
    ];
    for (query, errors) in cases {
        let (status, body) = get(&client, &format!("/postings?{query}")).await;
        assert_eq!(status, Status::UnprocessableEntity, "{query}");
        assert_eq!(body, json!({ "errors": errors }), "{query}");
    }
}

#[sqlx::test]
async fn shows_one_posting(pool: PgPool) {
    let (client, _fake) = client(&pool).await;
    let mut removed = Seed::new("ashby", "Gone Co");
    removed.removed = true;
    let mut live = Seed::new("greenhouse", "Example Labs");
    live.location_raw = Some("Remote - US");
    live.country = Some("United States");
    live.remote = true;
    let seeded = seed(&pool, &[live, removed]).await;

    let (status, body) = get(&client, &format!("/postings/{}", seeded[0])).await;
    assert_eq!(status, Status::Ok, "{body}");
    let mut keys: Vec<&str> = body
        .as_object()
        .expect("a posting")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "apply_url",
            "board",
            "city",
            "company",
            "country",
            "created_at",
            "department",
            "description",
            "employment_type",
            "external_id",
            "first_seen_at",
            "id",
            "last_seen_at",
            "location_raw",
            "posted_at",
            "region",
            "remote",
            "removed_at",
            "source",
            "title",
            "updated_at",
            "url",
        ]
    );
    assert_eq!(body["id"], seeded[0]);
    assert_eq!(body["source"], "greenhouse");
    assert_eq!(body["board"], "example-labs");
    assert_eq!(body["external_id"], "ext-0");
    assert_eq!(body["title"], "Role 0");
    assert_eq!(body["company"], "Example Labs");
    assert_eq!(body["url"], "https://boards.example/ext-0");
    assert_eq!(body["location_raw"], "Remote - US");
    assert_eq!(body["country"], "United States");
    assert_eq!(body["city"], Value::Null);
    assert_eq!(body["remote"], true);
    assert_eq!(body["description"], "About the role");
    assert_eq!(body["removed_at"], Value::Null);

    let (status, body) = get(&client, &format!("/postings/{}", seeded[1])).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["company"], "Gone Co");
    assert!(body["removed_at"].is_string(), "{body}");
    assert_eq!(body["description"], "About the role");

    let unknown = seeded[1] + 1;
    let (status, _) = get(&client, &format!("/postings/{unknown}")).await;
    assert_eq!(status, Status::NotFound);
}
