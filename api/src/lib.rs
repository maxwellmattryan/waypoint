//! Waypoint's HTTP API.

pub mod applications;
pub mod capture;
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
pub mod kill_switch;
pub mod mcp;
pub mod openapi;
pub mod resumes;
pub mod runs;
pub mod tools;

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
/// queue settings, Google OAuth settings and capture settings. The job worker starts at
/// liftoff and stops at shutdown. Debug builds also mount the `/dev` routes. Every route
/// takes [`host::LocalHost`].
pub fn rocket(
    figment: Figment,
    pool: PgPool,
    claude: ClaudeRunner,
    jobs: jobs::Config,
    google: google::Config,
    capture: capture::Config,
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
        // No workflows or tools are registered yet.
        runtime: runs::Runtime::default(),
    };

    let rocket = rocket::custom(figment)
        .manage(pool)
        .manage(claude)
        .manage(bus)
        .manage(wake)
        .manage(google::Google::new(google))
        .manage(capture::Capturer::new(capture))
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
                applications::stages::list,
                applications::stages::add,
                applications::stages::update,
                applications::stages::reorder,
                applications::stages::remove,
                applications::timeline::list_events,
                applications::timeline::add_entry,
                capture::capture,
                jobs::show,
                runs::list,
                runs::show,
                google::connect,
                google::callback,
                google::status,
                google::disconnect,
                kill_switch::show,
                kill_switch::update,
                kill_switch::delete_all,
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
