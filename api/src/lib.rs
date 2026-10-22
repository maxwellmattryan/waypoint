//! Waypoint's HTTP API.

pub mod claude;
pub mod config;
#[cfg(debug_assertions)]
mod dev;
mod health;

use rocket::figment::Figment;
use rocket::{Build, Rocket};
use sqlx::PgPool;

use crate::claude::ClaudeRunner;

/// Migrations embedded from `api/migrations`, applied on startup.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

/// Builds the Rocket instance with the given configuration, database pool and Claude
/// runner. Debug builds also mount the `/dev` routes.
pub fn rocket(figment: Figment, pool: PgPool, claude: ClaudeRunner) -> Rocket<Build> {
    let rocket = rocket::custom(figment)
        .manage(pool)
        .manage(claude)
        .mount("/", rocket::routes![health::health]);
    #[cfg(debug_assertions)]
    let rocket = rocket.mount("/", dev::routes());
    rocket
}
