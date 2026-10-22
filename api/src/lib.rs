//! Waypoint's HTTP API.

pub mod config;
mod health;

use rocket::figment::Figment;
use rocket::{Build, Rocket};
use sqlx::PgPool;

/// Migrations embedded from `api/migrations`, applied on startup.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

/// Builds the Rocket instance with the given configuration and database pool.
pub fn rocket(figment: Figment, pool: PgPool) -> Rocket<Build> {
    rocket::custom(figment)
        .manage(pool)
        .mount("/", rocket::routes![health::health])
}
