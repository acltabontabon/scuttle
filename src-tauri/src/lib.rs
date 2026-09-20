//! Scuttle's core.
//!
//! Everything trust-sensitive lives here: traversal, detection, evidence,
//! safety, quarantine and persistence. The webview asks; this crate decides.

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

pub use error::{Result, ScuttleError};

/// Start the desktop application.
pub fn run() {
    // `--dry-run` and friends answer without ever opening a window.
    if cli::handle_arguments() {
        return;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SCUTTLE_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("scuttle_core=info")),
        )
        // Paths are not written to logs at normal levels; see docs/privacy.md.
        .with_target(false)
        .init();

    tauri::Builder::default()
        .setup(|app| {
            commands::init(app)?;
            Ok(())
        })
        .invoke_handler(commands::handlers())
        .run(tauri::generate_context!())
        .expect("Scuttle could not start");
}
