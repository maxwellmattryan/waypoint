use std::process::ExitCode;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use waypoint_api::claude::ClaudeRunner;
use waypoint_api::{MIGRATOR, capture, config, google, jobs};

#[rocket::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("waypoint-api: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    dotenvy::dotenv().ok();

    let port = config::parse_port(std::env::var("WAYPOINT_API_PORT").ok().as_deref())?;
    let claude = ClaudeRunner::from_env()?;
    let jobs = jobs::Config::from_env()?;
    let google = google::Config::from_env(port)?;
    let database_url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .ok_or("DATABASE_URL is not set (copy .env.example to .env)")?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(2))
        .connect(&database_url)
        .await
        .map_err(|err| format!("cannot connect to the database at DATABASE_URL: {err}"))?;

    MIGRATOR
        .run(&pool)
        .await
        .map_err(|err| format!("database migrations failed: {err}"))?;

    let figment = config::figment(rocket::Config::figment(), port);
    waypoint_api::rocket(
        figment,
        pool,
        claude,
        jobs,
        google,
        capture::Config::default(),
    )
    .launch()
    .await
    .map_err(|err| format!("server failed: {err}"))?;
    Ok(())
}
