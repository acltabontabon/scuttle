//! The menu bar icon on macOS, the notification area icon on Windows.
//!
//! Small on purpose. A left click opens Scuttle's burrow (see
//! [`crate::burrow`]); a right click opens the plain native menu, which is the
//! dependable fallback and offers the same ways in. The icon itself never
//! animates, never wears a badge and never counts problems at anybody.
//!
//! If any of this fails to build, the application says so and carries on with
//! its ordinary close-quits-the-app behaviour. The one outcome that must never
//! happen is a running Scuttle that has hidden its window and left no icon to
//! bring it back.

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

use crate::commands::AppState;

pub const TRAY_ID: &str = "scuttle";

mod ids {
    pub const OPEN: &str = "open";
    pub const RUMMAGE: &str = "rummage";
    pub const DRAWER: &str = "drawer";
    pub const STATUS: &str = "status";
    pub const PAUSE: &str = "pause";
    pub const SETTINGS: &str = "settings";
    pub const QUIT: &str = "quit";
}

/// Build the tray icon. Returns whether it worked.
///
/// The caller uses the answer to decide whether hiding the window on close is
/// safe, so a failure here quietly turns background mode back into ordinary
/// behaviour rather than stranding anyone.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> bool {
    match build(app) {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(error = %err, "the tray icon could not be created");
            false
        }
    }
}

fn build<R: Runtime>(app: &AppHandle<R>) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let menu = menu(app, "Nothing to report yet.", false)?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon()?)
        // macOS recolours a template image for light and dark menu bars, and
        // for when the menu is open. Setting this is the difference between an
        // icon that belongs there and a small coloured sticker.
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Scuttle")
        .menu(&menu)
        // Left opens the burrow; the menu stays one right click away, the
        // way tray icons that open a popover behave on both systems.
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu)
        .on_tray_icon_event(on_icon)
        .build(app)?;

    Ok(())
}

fn icon() -> std::result::Result<tauri::image::Image<'static>, Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    let bytes: &[u8] = include_bytes!("../icons/tray/tray-template@2x.png");

    // Windows does not recolour tray icons, so Scuttle picks the one that will
    // be legible against the taskbar it is about to sit on. Read once, at
    // startup: this does not follow a theme change until the next launch,
    // which is a limitation rather than a decision.
    #[cfg(target_os = "windows")]
    let bytes: &[u8] = if windows_taskbar_is_light() {
        include_bytes!("../icons/tray/tray-dark@2x.png")
    } else {
        include_bytes!("../icons/tray/tray-light@2x.png")
    };

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let bytes: &[u8] = include_bytes!("../icons/tray/tray-dark@2x.png");

    // The PNG is decoded here rather than handed over raw: `tray-icon` wants
    // RGBA, and the `image` crate is already a dependency for the screenshot
    // detector.
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.to_rgba8();
    let (width, height) = decoded.dimensions();
    Ok(tauri::image::Image::new_owned(
        decoded.into_raw(),
        width,
        height,
    ))
}

#[cfg(target_os = "windows")]
fn windows_taskbar_is_light() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|key| key.get_value::<u32, _>("SystemUsesLightTheme"))
        .map(|value| value == 1)
        // Windows has defaulted to a dark taskbar since 10; assume that when
        // the key cannot be read.
        .unwrap_or(false)
}

/// The menu, rebuilt whenever the status line changes.
fn menu<R: Runtime>(app: &AppHandle<R>, status: &str, offer_pause: bool) -> tauri::Result<Menu<R>> {
    let open = MenuItem::with_id(app, ids::OPEN, "Open Scuttle", true, None::<&str>)?;
    let rummage = MenuItem::with_id(app, ids::RUMMAGE, "Rummage now", true, None::<&str>)?;
    let drawer = MenuItem::with_id(app, ids::DRAWER, "Open the Drawer", true, None::<&str>)?;
    // Not clickable: it is a sentence, not a control.
    let status_item = MenuItem::with_id(app, ids::STATUS, status, false, None::<&str>)?;
    let settings = MenuItem::with_id(app, ids::SETTINGS, "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ids::QUIT, "Quit Scuttle", true, None::<&str>)?;

    let menu = Menu::new(app)?;
    menu.append(&open)?;
    menu.append(&rummage)?;
    menu.append(&drawer)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&status_item)?;

    if offer_pause {
        let paused = is_paused(app);
        let label = if paused {
            "Resume background checks"
        } else {
            "Pause checks until tomorrow"
        };
        menu.append(&MenuItem::with_id(
            app,
            ids::PAUSE,
            label,
            true,
            None::<&str>,
        )?)?;
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&settings)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;
    Ok(menu)
}

fn is_paused<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<AppState>()
        .and_then(|state| state.store().background_state().ok())
        .is_some_and(|s| s.paused_until_unix > crate::commands::now_unix())
}

/// Rewrite the status line. Called when a scan, move or check finishes —
/// there is no reliable cross-platform hook for "the menu is about to open".
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    let offer_pause = state
        .store()
        .settings()
        .map(|s| s.background_checks)
        .unwrap_or(false);

    let status = status_line(&state);
    if let Ok(menu) = menu(app, &status, offer_pause) {
        let _ = tray.set_menu(Some(menu));
    }
}

/// One line about where things stand, or nothing much if there is nothing
/// much. Never a count of problems.
fn status_line(state: &AppState) -> String {
    let Ok(Some(scan)) = state.store().latest_scan() else {
        return "Nothing looked at yet.".into();
    };

    let when = scan
        .finished_unix
        .map(|at| whenish(crate::commands::now_unix() - at))
        .unwrap_or_else(|| "recently".into());

    let what = match scan.candidates_found {
        0 => "nothing worth a look".to_string(),
        1 => "1 thing worth a look".to_string(),
        n => format!("{n} things worth a look"),
    };

    format!("Last look {when} — {what}")
}

/// Rough, and deliberately so: a menu does not need the minute.
fn whenish(seconds_ago: i64) -> String {
    const HOUR: i64 = 3600;
    const DAY: i64 = 24 * HOUR;
    match seconds_ago {
        s if s < 2 * HOUR => "just now".into(),
        s if s < DAY => "today".into(),
        s if s < 2 * DAY => "yesterday".into(),
        s if s < 7 * DAY => format!("{} days ago", s / DAY),
        _ => "a while ago".into(),
    }
}

fn on_menu<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    match event.id().as_ref() {
        ids::OPEN => crate::window::reveal(app),
        ids::RUMMAGE => {
            // Shown first: a rummage nobody can see is the thing this whole
            // feature was careful not to become.
            crate::window::reveal(app);
            crate::window::ask_frontend_to(app, "rummage");
        }
        ids::SETTINGS => {
            crate::window::reveal(app);
            crate::window::ask_frontend_to(app, "settings");
        }
        ids::DRAWER => {
            crate::window::reveal(app);
            crate::window::ask_frontend_to(app, "drawer");
        }
        ids::PAUSE => {
            toggle_pause(app);
            refresh(app);
        }
        ids::QUIT => crate::window::quit(app),
        _ => {}
    }
}

fn toggle_pause<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Ok(mut persisted) = state.store().background_state() else {
        return;
    };
    let now = crate::commands::now_unix();
    persisted.paused_until_unix = if persisted.paused_until_unix > now {
        0
    } else {
        // The tray has no idea what time zone the user is in, so a pause
        // started here runs to the next UTC midnight. The window's own control
        // uses the local day; both are honest about what they did, and the
        // status line shows the result either way.
        crate::background::schedule::tomorrow_unix(now, 0)
    };
    let _ = state.store().save_background_state(&persisted);
    state.nudge_scheduler();
}

fn on_icon<R: Runtime>(tray: &tauri::tray::TrayIcon<R>, event: TrayIconEvent) {
    // A left click opens (or puts away) the burrow beside the icon. Nothing
    // else happens on a click: opening it starts no scan and moves nothing.
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        rect,
        ..
    } = event
    {
        crate::burrow::toggle(tray.app_handle(), rect);
    }
}
