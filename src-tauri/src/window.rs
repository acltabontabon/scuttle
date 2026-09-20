//! Showing, hiding and quitting.
//!
//! The rules background mode has to keep:
//!
//! * Closing the window with background mode on **hides** it. It is never
//!   closed and never recreated, so reopening is the same window with the same
//!   state, and there is never a second one.
//! * An explicit quit — the tray's own item, ⌘Q, logging out, shutting down —
//!   always quits. Nothing here ever prevents an exit; the hiding happens at
//!   the window's close, which is a different event, and that is precisely why
//!   hiding rather than closing is the right mechanism.
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
pub fn quit<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<AppState>() {
        state.begin_quitting();
        state.stop_scheduler();
        state.cancel_scan();
    }
    app.exit(0);
}

/// Ask the window to go somewhere. Ignored if nobody is listening yet.
pub fn ask_frontend_to<R: Runtime>(app: &AppHandle<R>, intent: &str) {
    let _ = app.emit(INTENT, intent.to_string());
}
