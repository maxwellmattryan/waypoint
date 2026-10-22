//! Waypoint's HTTP API.

pub mod applications;
pub mod claude;
pub mod config;
pub mod crypto;
#[cfg(debug_assertions)]
mod dev;
pub mod events;
pub mod google;
mod health;
pub mod host;
pub mod jobs;
pub mod openapi;
pub mod resumes;

use rocket::fairing::AdHoc;
use rocket::figment::Figment;
use rocket::{Build, Rocket};
use sqlx::PgPool;
use tokio::sync::watch;

use crate::claude::ClaudeRunner;
use crate::events::EventBus;
use crate::jobs::{Wake, Worker};

/// Migrations embedded from `api/migrations`, applied on startup.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

/// Builds the Rocket instance with the given configuration, database pool, Claude runner,
/// queue settings and Google OAuth settings. The job worker starts at liftoff and stops at shutdown. Debug
/// builds also mount the `/dev` routes. Every route takes [`host::LocalHost`].
pub fn rocket(
    figment: Figment,
    pool: PgPool,
    claude: ClaudeRunner,
    jobs: jobs::Config,
    google: google::Config,
) -> Rocket<Build> {
    let bus = EventBus::new();
    let wake = Wake::default();
    let (stop, stopped) = watch::channel(false);
    let worker = Worker {
        pool: pool.clone(),
        runner: claude.clone(),
        bus: bus.clone(),
        config: jobs,
        wake: wake.0.clone(),
    };

    let rocket = rocket::custom(figment)
        .manage(pool)
        .manage(claude)
        .manage(bus)
        .manage(wake)
        .manage(google::Google::new(google))
        .mount(
            "/",
            rocket::routes![
                health::health,
                events::events,
                applications::list,
                applications::create,
                applications::show,
                applications::update,
                applications::set_status,
                applications::set_resume,
                applications::delete,
                resumes::list,
                resumes::upload,
                resumes::show,
                resumes::pdf,
                resumes::delete,
                jobs::show,
                google::connect,
                google::callback,
                google::status,
                google::disconnect,
            ],
        )
        .attach(AdHoc::on_liftoff("job worker", move |_| {
            Box::pin(async move {
                worker.spawn(stopped);
            })
        }))
        .attach(AdHoc::on_shutdown("job worker", move |_| {
            Box::pin(async move {
                stop.send_replace(true);
            })
        }));
    #[cfg(debug_assertions)]
    let rocket = rocket.mount("/", dev::routes());
    rocket
}
