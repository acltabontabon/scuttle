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
pub mod updater;
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
        // Driven from Rust only. The webview has no updater permission in its
        // capability, so it cannot check, download or install on its own; it
        // asks the commands in `commands::updates`, which go through the same
        // operation gate as everything else that changes files.
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            // After the window: checking for updates must never be the reason
            // Scuttle takes longer to appear. It returns at once and looks
            // later, on the async runtime.
            if let Some(state) = app.try_state::<commands::AppState>() {
                updater::start(app.handle(), state.inner());
            }
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
            // Nothing here calls `prevent_exit` except while an update is being
            // installed. ⌘Q, the tray's Quit, a logout and a shutdown all
            // arrive through this, and all of them mean it. What this does is
            // make sure Scuttle's own background work stops with it, rather
            // than a scan outliving the decision to quit.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if let Some(state) = app.try_state::<commands::AppState>() {
                    // The one exception to "nothing here prevents an exit": an
                    // install has committed and is replacing the application.
                    // A quit that arrived now would stop it halfway. The install
                    // ends this process itself, or fails and lets go of this —
                    // and its own restart arrives here too, which must pass.
                    if updater::defers_exit(state.installing(), code) {
                        api.prevent_exit();
                        return;
                    }
                    // Files in motion: stop them at a safe point first, and
                    // say so. `code` is `None` for a quit a person asked for
                    // (⌘Q, the menu); Scuttle's own exit after winding down
                    // arrives with a code and passes straight through, as does
                    // a second request.
                    if code.is_none() && !state.quitting() && window::busy_with_files(&state) {
                        state.begin_quitting();
                        state.stop_scheduler();
                        state.cancel_scan();
                        if window::finish_before_quitting(app, &state) {
                            api.prevent_exit();
                            return;
                        }
                    }
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
