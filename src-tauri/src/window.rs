//! Showing, hiding and quitting.
//!
//! The rules background mode has to keep:
//!
//! * Closing the window with background mode on **hides** it. It is never
//!   closed and never recreated, so reopening is the same window with the same
//!   state, and there is never a second one.
//! * An explicit quit — the tray's own item, ⌘Q, logging out, shutting down —
//!   always quits. The only thing that delays it is files in motion: a move is
//!   stopped at its next safe point and the window says so, and a second quit
//!   does not wait at all. The hiding happens at the window's close, which is
//!   a different event, and that is precisely why hiding rather than closing
//!   is the right mechanism.
//! * With background mode off, or with no tray to return from, behaviour is
//!   exactly what it was before any of this existed.

use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

use crate::commands::AppState;

pub const MAIN: &str = "main";

/// An instruction for the window, for the few things the tray can ask of it.
///
/// A menu item cannot call a command, so it says what it wants and the
/// frontend — which owns navigation — decides what that means.
pub const INTENT: &str = "scuttle://intent";

pub fn main_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(MAIN)
}

/// Bring Scuttle back: the same window, raised, focused and on screen.
pub fn reveal<R: Runtime>(app: &AppHandle<R>) {
    // The dock icon comes back before the window does. Asking for a window in
    // front while the process is still an accessory is how you get a window
    // that is technically visible and behind everything else.
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);

    let Some(window) = main_window(app) else {
        tracing::warn!("asked to show a window that is not there");
        return;
    };

    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();

    if let Some(state) = app.try_state::<AppState>() {
        state.set_window_visible(true);
    }
    // Events are not replayed, so a window that has been away is told where
    // things stand rather than being left to assume.
    announce(app);
}

/// Tell the window where background mode stands.
fn announce<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<AppState>() {
        crate::commands::emit_background(app, &state);
    }
}

/// Put Scuttle away without quitting it.
pub fn conceal<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = main_window(app) {
        let _ = window.hide();
    }
    if let Some(state) = app.try_state::<AppState>() {
        state.set_window_visible(false);
    }

    // Leaving the dock is what makes this a menu bar application rather than
    // an application that happens to be hiding.
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

/// Whether closing the window should hide it rather than end the process.
///
/// Both halves matter. The setting is the user's intent; the tray being alive
/// is whether Scuttle can honour it. Without an icon to come back from,
/// hiding would be a trap, so it does not happen.
pub fn should_conceal<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    if state.quitting() {
        return false;
    }
    if !state.tray_alive() {
        return false;
    }
    state
        .store()
        .settings()
        .map(|s| s.background_mode)
        .unwrap_or(false)
}

/// Quit, properly.
///
/// Stops the scheduler first so that nothing is mid-scan when the process
/// goes. Quitting Scuttle stops Scuttle's background work: there is no helper,
/// no service and nothing left running.
///
/// If files are moving, quitting waits for them — briefly, and visibly. The
/// move is asked to stop at its next safe point (between files, or between
/// chunks of a copy), the window says so, and the process ends once the
/// Drawer's records are settled. Asking to quit a second time ends it at once:
/// the journal was written for exactly that, and the next launch settles
/// whatever was in flight.
pub fn quit<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        app.exit(0);
        return;
    };
    let already = state.quitting();
    state.begin_quitting();
    state.stop_scheduler();
    state.cancel_scan();

    if !already && finish_before_quitting(app, &state) {
        return;
    }
    app.exit(0);
}

/// Is something changing files that should be allowed to reach a safe point?
pub fn busy_with_files(state: &AppState) -> bool {
    use crate::commands::Operation;
    matches!(
        state.current_operation(),
        Some(Operation::Move | Operation::Restore | Operation::EmptyDrawer | Operation::RemoveItem)
    )
}

/// Wind down a running file operation, then exit. Returns false when there was
/// nothing to wait for.
pub fn finish_before_quitting<R: Runtime>(app: &AppHandle<R>, state: &AppState) -> bool {
    if !busy_with_files(state) {
        return false;
    }
    state.cancel_move();
    reveal(app);
    ask_frontend_to(app, "quitting");

    let app = app.clone();
    let state = state.clone_handle();
    let spawned = std::thread::Builder::new()
        .name("scuttle-quit".into())
        .spawn(move || {
            // A move stops within a chunk; a restore or an empty finishes the
            // item it is on. Two minutes is far past either, and past it the
            // journal is the safer bet than waiting on a stuck disk.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            while busy_with_files(&state) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            app.exit(0);
        });
    spawned.is_ok()
}

/// Ask the window to go somewhere. Ignored if nobody is listening yet.
pub fn ask_frontend_to<R: Runtime>(app: &AppHandle<R>, intent: &str) {
    let _ = app.emit(INTENT, intent.to_string());
}
