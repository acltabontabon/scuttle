//! Scuttle's core.
//!
//! Everything trust-sensitive lives here: traversal, detection, evidence,
//! safety, quarantine and persistence. The webview asks; this crate decides.

pub mod background;
pub mod cli;
pub mod commands;
pub mod detectors;
pub mod error;
pub mod evidence;
pub mod model;
pub mod platform;
pub mod quarantine;
pub mod safety;
pub mod scanning;
pub mod space;
pub mod storage;
pub mod tray;
pub mod window;

pub use error::{Result, ScuttleError};

use tauri::Manager;

/// Start the desktop application.
pub fn run() {
    // `--dry-run` and friends answer without ever opening a window.
    let Some(launch) = cli::handle_arguments() else {
        return;
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SCUTTLE_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("scuttle_core=info")),
        )
        // Paths are not written to logs at normal levels; see docs/privacy.md.
        .with_target(false)
        .init();

    tauri::Builder::default()
        // Registered first, as the plugin requires. One Scuttle per user:
        // two processes would share one database and one drawer while each
        // believed its own operation gate was the only one.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::reveal(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            // A per-user login item. No installer, no service, no elevation.
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            // Started by the system rather than by a person, so it should not
            // throw a window at whoever just logged in.
            Some(vec![cli::BACKGROUND_FLAG]),
        ))
        .setup(move |app| {
            commands::init(app)?;
            start_window(app, &launch);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                if window::should_conceal(app) {
                    // Hidden, not closed: the same window comes back, and
                    // because it still exists the application is never asked
                    // to exit behind our back.
                    api.prevent_close();
                    window::conceal(app);
                }
            }
        })
        .invoke_handler(commands::handlers())
        .build(tauri::generate_context!())
        .expect("Scuttle could not start")
        .run(|app, event| {
            // Nothing here ever calls `prevent_exit`. ⌘Q, the tray's Quit, a
            // logout and a shutdown all arrive through this, and all of them
            // mean it. What this does is make sure Scuttle's own background
            // work stops with it, rather than a scan outliving the decision
            // to quit.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app.try_state::<commands::AppState>() {
                    state.begin_quitting();
                    state.stop_scheduler();
                    state.cancel_scan();
                }
            }
        });
}

/// Show the window, install the tray, and start the scheduler if it is wanted.
///
/// Order matters. The tray is built first so that whether it exists is known
/// before anything decides to hide a window, and the window is shown unless
/// every condition for staying out of sight is true.
fn start_window(app: &tauri::App, launch: &cli::Launch) {
    let handle = app.handle().clone();
    let alive = tray::install(&handle);

    let Some(state) = app.try_state::<commands::AppState>() else {
        return;
    };
    state.set_tray_alive(alive);

    let wants_background = state
        .store()
        .settings()
        .map(|s| s.background_mode)
        .unwrap_or(false);

    // Start hidden only when the system started Scuttle, the user asked for
    // background mode, and there is an icon to get back from. Any doubt at all
    // and the window appears: an invisible application with no way in is the
    // one failure this whole feature must not have.
    let stay_hidden = launch.background && wants_background && alive;

    if stay_hidden {
        window::conceal(&handle);
    } else {
        window::reveal(&handle);
    }

    state.sync_scheduler(&handle);
    tray::refresh(&handle);
}
