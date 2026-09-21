//! The window's side of updating.
//!
//! Five commands and one event. Every command returns the whole
//! [`Snapshot`], and the same snapshot goes out as `scuttle://update` whenever
//! anything changes, so the interface holds no update state of its own to
//! disagree with this one.
//!
//! Checking and downloading are `async` and run on the runtime, not the main
//! thread: neither blocks the window, and neither is blocked by a file
//! operation. Installing goes through [`Updater::install`], which asks the
//! operation gate rather than anything the webview says about being idle.

use tauri::State;

use crate::updater::{Snapshot, Updater};
use crate::Result;

#[tauri::command]
pub fn update_status(updater: State<'_, Updater>) -> Snapshot {
    updater.snapshot()
}

/// Look now. `manual` is true when a person asked, which is what makes a
/// failure worth showing.
#[tauri::command]
pub async fn check_for_update(updater: State<'_, Updater>, manual: bool) -> Result<Snapshot> {
    Ok(updater.check(manual).await)
}

/// Download the update that was found. Returns when the download has finished
/// or failed; progress arrives as events meanwhile.
#[tauri::command]
pub async fn download_update(updater: State<'_, Updater>) -> Result<Snapshot> {
    Ok(updater.download().await)
}

/// Install the downloaded update and restart. Refused with `busy` while
/// anything is changing files.
#[tauri::command]
pub async fn install_update(updater: State<'_, Updater>) -> Result<Snapshot> {
    updater.install().await
}

#[tauri::command]
pub fn dismiss_update(updater: State<'_, Updater>) -> Snapshot {
    updater.dismiss()
}
