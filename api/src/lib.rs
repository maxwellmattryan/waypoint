//! Waypoint's HTTP API.

pub mod applications;
pub mod calendar;
pub mod capture;
pub mod claude;
pub mod config;
pub mod crypto;
#[cfg(debug_assertions)]
mod dev;
pub mod email;
pub mod evals;
pub mod events;
pub mod gmail;
pub mod google;
mod health;
pub mod host;
pub mod jobs;
pub mod kill_switch;
pub mod mcp;
pub mod memories;
pub mod memory_signals;
pub mod openapi;
pub mod profile;
pub mod resumes;
pub mod retrieval;
pub mod runs;
pub mod tools;
pub mod workflows;

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
    let runtime = workflows::runtime();
    let google = google::Google::new(google);
    let worker = Worker {
        pool: pool.clone(),
        runner: claude.clone(),
        bus: bus.clone(),
        config: jobs,
        wake: wake.0.clone(),
        runtime: runtime.clone(),
        google: google.clone(),
    };

    let rocket = rocket::custom(figment)
        .manage(pool)
        .manage(claude)
        .manage(bus)
        .manage(wake)
        .manage(runtime)
        .manage(google)
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
                memories::list,
                memories::create,
                memories::show,
                memories::evidence,
                memories::update,
                memories::confirm,
                memories::delete,
                workflows::memory_writer::learn,
                memory_signals::list,
                profile::linkedin::import,
                profile::resume::start,
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
                runs::approvals::list,
                runs::approvals::approve_route,
                runs::approvals::reject_route,
                runs::actions::list,
                runs::actions::undo_route,
                runs::review::list,
                runs::review::dismiss_route,
                runs::review::accept_route,
                workflows::tidy::start_tidy,
                google::connect,
                google::callback,
                google::status,
                google::disconnect,
                gmail::status,
                gmail::sync_now,
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
