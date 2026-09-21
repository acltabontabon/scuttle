//! Scuttle's burrow: the little window that opens from the tray icon.
//!
//! A left click on the icon opens it beside the icon; a right click still
//! opens the plain native menu, which stays the dependable way in if a webview
//! ever misbehaves. The burrow shows where things stand, one short line from
//! Scuttle, and the obvious ways onward. It never starts anything by being
//! opened, never opens by itself, and only takes focus because someone just
//! clicked for it.
//!
//! It is one window, built the first time it is wanted and then shown and
//! hidden, placed from the icon's own rectangle and kept inside the work area
//! of whichever display that icon is on — at any scale, whichever edge the
//! taskbar or menu bar sits at.

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Position, Rect, Runtime, Size, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

use crate::commands::AppState;

pub const BURROW: &str = "burrow";

/// Sent to the burrow each time it opens, so it can refresh and react once.
pub const OPENED: &str = "scuttle://burrow-opened";

/// Logical size. Small enough to feel like part of the tray.
const WIDTH: f64 = 300.0;
const HEIGHT: f64 = 372.0;
/// Space between the icon and the burrow, and between the burrow and a screen
/// edge, in logical pixels.
const GAP: f64 = 6.0;
const MARGIN: f64 = 8.0;

/// Open the burrow beside the icon at `rect`, or put it away if it is open.
pub fn toggle<R: Runtime>(app: &AppHandle<R>, rect: Rect) {
    if let Some(window) = app.get_webview_window(BURROW) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            return;
        }
    }
    let window = match app.get_webview_window(BURROW) {
        Some(window) => window,
        None => match build(app) {
            Ok(window) => window,
            Err(err) => {
                // The native menu still works; nothing is stranded.
                tracing::warn!(error = %err, "the burrow could not be built");
                return;
            }
        },
    };
    place(app, &window, rect);
    let _ = window.show();
    let _ = window.set_focus();
    let _ = app.emit_to(BURROW, OPENED, ());
}

/// Put the burrow away, if it is out.
pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(BURROW) {
        let _ = window.hide();
    }
}

fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    let window = WebviewWindowBuilder::new(app, BURROW, WebviewUrl::App("burrow.html".into()))
        .title("Scuttle")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .shadow(true)
        .visible(false)
        .focused(false)
        .background_color(tauri::window::Color(0xF4, 0xEF, 0xE6, 0xFF))
        .build()?;

    // Clicking anywhere else puts it away, like every other tray popover.
    let handle = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            let _ = handle.hide();
        }
    });
    Ok(window)
}

/// Where the burrow goes, in physical pixels: centred on the icon, on the
/// side of it that faces the middle of the screen, and never past the edge of
/// the work area. `icon` is the icon's rectangle, `work` the display's work
/// area, `scale` its scale factor.
pub(crate) fn placement(
    icon: (f64, f64, f64, f64),
    work: (f64, f64, f64, f64),
    scale: f64,
) -> (f64, f64) {
    let (ix, iy, iw, ih) = icon;
    let (wx, wy, ww, wh) = work;
    let width = WIDTH * scale;
    let height = HEIGHT * scale;
    let gap = GAP * scale;
    let margin = MARGIN * scale;

    let clamp = |value: f64, low: f64, high: f64| {
        if high < low {
            low
        } else {
            value.clamp(low, high)
        }
    };

    let centre_y = iy + ih / 2.0;
    let centre_x = ix + iw / 2.0;
    let beside = iw > 0.0 && (ix + iw <= wx + margin || ix >= wx + ww - margin);

    let (x, y) = if beside {
        // A taskbar on the left or right edge: open sideways.
        let x = if ix >= wx + ww / 2.0 {
            ix - width - gap
        } else {
            ix + iw + gap
        };
        (x, centre_y - height / 2.0)
    } else if centre_y < wy + wh / 2.0 {
        // Menu bar or a top taskbar: open below.
        (centre_x - width / 2.0, iy + ih + gap)
    } else {
        // The usual Windows taskbar: open above.
        (centre_x - width / 2.0, iy - height - gap)
    };

    (
        clamp(x, wx + margin, wx + ww - width - margin),
        clamp(y, wy + margin, wy + wh - height - margin),
    )
}

fn place<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>, rect: Rect) {
    // The icon's rectangle can come as physical or logical pixels; a logical
    // one needs a scale, and the display under it is what supplies that.
    let guess = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let physical = |position: &Position, size: &Size, scale: f64| {
        let p = position.to_physical::<f64>(scale);
        let s = size.to_physical::<f64>(scale);
        (p.x, p.y, s.width, s.height)
    };
    let first = physical(&rect.position, &rect.size, guess);
    let monitor = app
        .monitor_from_point(first.0 + first.2 / 2.0, first.1 + first.3 / 2.0)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let scale = monitor.scale_factor();
    let icon = physical(&rect.position, &rect.size, scale);
    let area = monitor.work_area();
    let work = (
        area.position.x as f64,
        area.position.y as f64,
        area.size.width as f64,
        area.size.height as f64,
    );
    let (x, y) = placement(icon, work, scale);
    let _ = window.set_size(tauri::LogicalSize::new(WIDTH, HEIGHT));
    let _ = window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

/// Everything the burrow shows, as facts. The line Scuttle says is chosen by
/// the burrow from these; nothing here is phrased for effect.
#[derive(Debug, Clone, Serialize)]
pub struct BurrowStatus {
    pub now_unix: i64,
    pub has_looked: bool,
    pub last_look_unix: Option<i64>,
    /// Findings from the last look.
    pub found: u64,
    /// Of those, how many Scuttle suggests.
    pub suggested: u64,
    pub drawer_items: u64,
    pub drawer_bytes: u64,
    /// Held items an interruption left for a person to look at.
    pub needs_attention: u64,
    /// What Scuttle is busy with right now, in words, if anything.
    pub busy: Option<String>,
    pub personality: crate::storage::Personality,
    pub reduced_motion: Option<bool>,
    /// "system", "light" or "dark", so the burrow matches the window.
    pub appearance: String,
}

pub fn status(state: &AppState) -> crate::Result<BurrowStatus> {
    let store = state.store();
    let settings = store.settings()?;
    let latest = store.latest_scan()?;
    let (found, suggested) = match &latest {
        Some(scan) => {
            let candidates = store.candidates_for_scan(&scan.id).unwrap_or_default();
            let suggested = candidates
                .iter()
                .filter(|c| {
                    c.assessment.eligibility == crate::safety::assess::Eligibility::Suggested
                })
                .count() as u64;
            (candidates.len() as u64, suggested)
        }
        None => (0, 0),
    };
    let held = store.held_quarantine()?;
    Ok(BurrowStatus {
        now_unix: crate::commands::now_unix(),
        has_looked: latest.is_some(),
        last_look_unix: latest.as_ref().and_then(|s| s.finished_unix),
        found,
        suggested,
        drawer_items: held.len() as u64,
        drawer_bytes: held.iter().map(|r| r.size).sum(),
        needs_attention: held.iter().filter(|r| r.attention).count() as u64,
        busy: state.current_operation().map(|op| op.doing().to_string()),
        personality: settings.personality,
        reduced_motion: settings.reduced_motion,
        appearance: settings.appearance.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK_1080: (f64, f64, f64, f64) = (0.0, 0.0, 1920.0, 1040.0);

    #[test]
    fn a_bottom_taskbar_opens_it_above_the_icon() {
        let (x, y) = placement((1700.0, 1044.0, 24.0, 32.0), WORK_1080, 1.0);
        assert!(y + HEIGHT <= 1044.0, "above the icon: {y}");
        assert!(x + WIDTH <= 1920.0 - MARGIN + 0.5);
    }

    #[test]
    fn a_menu_bar_opens_it_below_the_icon() {
        let (_, y) = placement((1400.0, 0.0, 44.0, 48.0), (0.0, 50.0, 3024.0, 1914.0), 2.0);
        assert!(y >= 48.0, "{y}");
    }

    #[test]
    fn it_never_leaves_the_screen_at_the_corner() {
        let (x, y) = placement((1910.0, 1044.0, 24.0, 32.0), WORK_1080, 1.0);
        assert!(x >= 0.0 && x + WIDTH <= 1920.0);
        assert!(y >= 0.0 && y + HEIGHT <= 1040.0);
    }

    #[test]
    fn a_side_taskbar_opens_it_sideways() {
        // Taskbar on the left: work area starts at 48.
        let (x, _) = placement((4.0, 900.0, 40.0, 40.0), (48.0, 0.0, 1872.0, 1080.0), 1.0);
        assert!(x >= 48.0);
    }

    #[test]
    fn a_second_display_to_the_left_is_honoured() {
        // A display at negative coordinates, scaled 1.5x.
        let work = (-2560.0, 0.0, 2560.0, 1400.0);
        let (x, y) = placement((-300.0, 1404.0, 36.0, 36.0), work, 1.5);
        assert!(x >= -2560.0 && x + WIDTH * 1.5 <= 0.0, "{x}");
        assert!(y + HEIGHT * 1.5 <= 1404.0);
    }
}
