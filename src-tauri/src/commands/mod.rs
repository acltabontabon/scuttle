//! The IPC surface.
//!
//! Everything the webview can ask for is here, and the rules are the same for
//! all of it:
//!
//! * The frontend addresses things by **id**, never by path. There is no
//!   command that takes an arbitrary path and does something to it.
//! * Every destructive command re-derives its decision from the live
//!   filesystem through [`crate::safety`]. A stored finding is a request.
//! * Scans are coarse-grained: one `rummage` call, then a stream of events.
//!   React never calls into Rust per file.

mod dry_run;
mod state;

pub use dry_run::{DryRunReport, DryRunRow};
pub use state::AppState;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

use crate::model::{human_bytes, Category, CleanupCandidate, RecommendedAction};
use crate::scanning::{HiccupSummary, Phase, Progress, ScanOptions, ScanSummary};
use crate::space::SpaceOverview;
use crate::storage::{HistoryEntry, QuarantineRecord, Settings};
use crate::{Result, ScuttleError};

/// Event names. Kept in one place so the TypeScript side can mirror them.
pub mod events {
    pub const PHASE: &str = "scuttle://phase";
    pub const PROGRESS: &str = "scuttle://progress";
    pub const FOUND: &str = "scuttle://found";
    pub const DONE: &str = "scuttle://done";
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// One pile on the floor.
#[derive(Debug, Clone, Serialize)]
pub struct Pile {
    pub category: Category,
    pub title: String,
    pub bytes: u64,
    pub count: u64,
    /// How many of these Scuttle would actually act on.
    pub actionable: u64,
    /// The pile's contents, capped. `count` is the real total.
    pub items: Vec<CleanupCandidate>,
}

/// Findings are never returned as one flat list: they arrive already grouped
/// the way they will be shown.
#[derive(Debug, Clone, Serialize)]
pub struct Findings {
    pub scan_id: Option<String>,
    pub finished_unix: Option<i64>,
    pub piles: Vec<Pile>,
    pub total_bytes: u64,
    pub reclaimable_bytes: u64,
    pub files_seen: u64,
    pub hiccups: HiccupSummary,
    /// Present only when a rummage has completed at least once.
    pub has_rummaged: bool,
}

/// Cap on how many items of a single pile cross the IPC boundary at once.
/// Nobody scrolls past two hundred screenshots, and the total is still exact.
const MAX_ITEMS_PER_PILE: usize = 200;

#[derive(Debug, Clone, Deserialize)]
pub struct RummageRequest {
    #[serde(default)]
    pub roots: Option<Vec<PathBuf>>,
    #[serde(default)]
    pub include_developer_debris: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RummageStarted {
    pub scan_id: String,
    pub roots: Vec<RootDescription>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RootDescription {
    pub label: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanEvent<T> {
    pub scan_id: String,
    #[serde(flatten)]
    pub payload: T,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreScope {
    Path,
    App,
    Category,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuarantineView {
    pub items: Vec<QuarantineRecord>,
    pub held_bytes: u64,
    pub retention_days: u32,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Start a rummage. Returns immediately; results arrive as events.
#[tauri::command]
pub fn rummage(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: RummageRequest,
) -> Result<RummageStarted> {
    let settings = state.store().settings()?;

    let roots: Vec<RootDescription> = match request.roots.filter(|r| !r.is_empty()) {
        Some(roots) => roots
            .into_iter()
            .map(|path| RootDescription {
                label: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
                path,
            })
            .collect(),
        None if !settings.scan_roots.is_empty() => settings
            .scan_roots
            .iter()
            .map(|path| RootDescription {
                label: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
                path: path.clone(),
            })
            .collect(),
        None => state
            .platform()
            .default_scan_roots()
            .into_iter()
            .map(|known| RootDescription {
                label: known.label,
                path: known.path,
            })
            .collect(),
    };

    if roots.is_empty() {
        return Err(ScuttleError::Refused(
            "There is nowhere to look. Choose at least one folder in Settings.".into(),
        ));
    }

    let options = ScanOptions {
        roots: roots.iter().map(|r| r.path.clone()).collect(),
        include_developer_debris: request
            .include_developer_debris
            .unwrap_or(settings.include_developer_debris),
        heavy_threshold: settings.heavy_threshold,
        ..Default::default()
    };

    let scan_id = state.start_scan(&options)?;
    let started = RummageStarted {
        scan_id: scan_id.clone(),
        roots,
    };

    // The scan itself runs on a worker so the window stays responsive and the
    // user can keep using their computer.
    let handle = app.clone();
    let state_for_worker = state.inner().clone_handle();
    let scan_id_for_worker = scan_id.clone();
    std::thread::Builder::new()
        .name("scuttle-rummage".into())
        .spawn(move || {
            if let Err(err) = state_for_worker.run_scan(&scan_id_for_worker, options, &handle) {
                tracing::warn!(error = %err, "rummage ended badly");
                let _ = handle.emit(
                    events::DONE,
                    ScanEvent {
                        scan_id: scan_id_for_worker,
                        payload: serde_json::json!({ "error": err }),
                    },
                );
            }
        })
        .map_err(|e| ScuttleError::Internal(format!("could not start rummaging: {e}")))?;

    Ok(started)
}

/// Ask the running scan to stop. Returns whether there was one.
#[tauri::command]
pub fn cancel_rummage(state: State<'_, AppState>) -> bool {
    state.cancel_scan()
}

/// Everything the last rummage turned up, already grouped into piles.
#[tauri::command]
pub fn findings(state: State<'_, AppState>) -> Result<Findings> {
    let store = state.store();
    let Some(scan) = store.latest_scan()? else {
        return Ok(Findings {
            scan_id: None,
            finished_unix: None,
            piles: Vec::new(),
            total_bytes: 0,
            reclaimable_bytes: 0,
            files_seen: 0,
            hiccups: HiccupSummary::default(),
            has_rummaged: false,
        });
    };

    let candidates = store.candidates_for_scan(&scan.id)?;
    let mut piles = Vec::new();

    for category in Category::ALL {
        let matching: Vec<&CleanupCandidate> = candidates
            .iter()
            .filter(|c| c.category == category)
            .collect();
        if matching.is_empty() {
            continue;
        }
        piles.push(Pile {
            category,
            title: category.title().to_string(),
            bytes: matching.iter().map(|c| c.size).sum(),
            count: matching.len() as u64,
            actionable: matching.iter().filter(|c| c.is_actionable()).count() as u64,
            items: matching
                .into_iter()
                .take(MAX_ITEMS_PER_PILE)
                .cloned()
                .collect(),
        });
    }
    piles.sort_by_key(|p| std::cmp::Reverse(p.bytes));

    Ok(Findings {
        scan_id: Some(scan.id),
        finished_unix: scan.finished_unix,
        total_bytes: candidates.iter().map(|c| c.size).sum(),
        reclaimable_bytes: candidates
            .iter()
            .filter(|c| c.recommended_action != RecommendedAction::InspectOnly)
            .map(|c| c.size)
            .sum(),
        files_seen: scan.files_seen,
        hiccups: HiccupSummary::default(),
        piles,
        has_rummaged: true,
    })
}

#[tauri::command]
pub fn finding(state: State<'_, AppState>, id: String) -> Result<CleanupCandidate> {
    state.store().candidate(&id)
}

/// Move a finding into the drawer.
#[tauri::command]
pub fn quarantine(state: State<'_, AppState>, id: String) -> Result<QuarantineRecord> {
    let candidate = state.store().candidate(&id)?;
    state.hold(&candidate)
}

/// Move one member of a group finding — a specific duplicate or screenshot —
/// rather than the one Scuttle proposed.
#[tauri::command]
pub fn quarantine_member(
    state: State<'_, AppState>,
    id: String,
    member_index: usize,
) -> Result<QuarantineRecord> {
    let candidate = state.store().candidate(&id)?;
    let derived = derive_member(&candidate, member_index)?;
    state.hold(&derived)
}

/// Which member of a group to hold on to.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepChoice {
    Newest,
    Oldest,
}

/// What happened to each member of a group.
#[derive(Debug, Clone, Serialize)]
pub struct GroupOutcome {
    pub held: Vec<QuarantineRecord>,
    pub kept: String,
    /// Members Scuttle would not move, and why. Partial success is the normal
    /// case here — one file of eighteen having changed since the scan is not a
    /// reason to abandon the other seventeen.
    pub refused: Vec<GroupRefusal>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupRefusal {
    pub display_name: String,
    pub reason: String,
    pub code: String,
}

/// Quarantine every member of a group except the one to keep.
///
/// The *policy* — which copy survives — is resolved here rather than in the
/// interface, from the group data the scan recorded. The frontend names an
/// intent ("keep the newest"); it does not choose paths.
#[tauri::command]
pub fn quarantine_group(
    state: State<'_, AppState>,
    id: String,
    keep: KeepChoice,
) -> Result<GroupOutcome> {
    run_group_action(state.inner(), &id, keep)
}

pub(crate) fn run_group_action(
    state: &AppState,
    id: &str,
    keep: KeepChoice,
) -> Result<GroupOutcome> {
    let candidate = state.store().candidate(id)?;
    if candidate.group.len() < 2 {
        return Err(ScuttleError::Refused(
            "That finding is a single thing, not a group.".into(),
        ));
    }
    if !candidate.is_actionable() {
        return Err(ScuttleError::Refused(format!(
            "Scuttle does not act on findings marked {}.",
            candidate.recommended_action_label()
        )));
    }

    // Members with no timestamp sort last, so they are never silently chosen
    // as the survivor.
    let keeper = match keep {
        KeepChoice::Newest => candidate
            .group
            .iter()
            .enumerate()
            .max_by_key(|(_, m)| m.modified_unix.unwrap_or(i64::MIN)),
        KeepChoice::Oldest => candidate
            .group
            .iter()
            .enumerate()
            .min_by_key(|(_, m)| m.modified_unix.unwrap_or(i64::MAX)),
    };
    let (keeper_index, keeper_member) =
        keeper.ok_or_else(|| ScuttleError::not_found("Anything to keep"))?;
    let kept = display_of(&keeper_member.path);

    let mut held = Vec::new();
    let mut refused = Vec::new();

    for index in 0..candidate.group.len() {
        if index == keeper_index {
            continue;
        }
        let derived = match derive_member(&candidate, index) {
            Ok(derived) => derived,
            Err(error) => {
                refused.push(GroupRefusal {
                    display_name: display_of(&candidate.group[index].path),
                    reason: error.to_string(),
                    code: error.code().to_string(),
                });
                continue;
            }
        };
        match state.hold(&derived) {
            Ok(record) => held.push(record),
            Err(error) => refused.push(GroupRefusal {
                display_name: derived.display_name.clone(),
                reason: error.to_string(),
                code: error.code().to_string(),
            }),
        }
    }

    Ok(GroupOutcome {
        held,
        kept,
        refused,
    })
}

/// A candidate pointing at one member of a group.
///
/// It carries the group's evidence and verdict but the member's own path and
/// scan-time state, so the staleness check still means something for the file
/// actually being moved.
fn derive_member(candidate: &CleanupCandidate, index: usize) -> Result<CleanupCandidate> {
    let member = candidate
        .group
        .get(index)
        .ok_or_else(|| ScuttleError::not_found("That copy"))?;

    let mut derived = candidate.clone();
    derived.path = member.path.clone();
    derived.display_name = display_of(&member.path);
    derived.size = member.size;
    derived.target_kind = crate::model::TargetKind::File;
    derived.group = Vec::new();
    derived.fingerprint = crate::model::StateFingerprint {
        size: member.size,
        modified_unix: member.modified_unix,
        is_dir: false,
        child_count: None,
    };
    Ok(derived)
}

fn display_of(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[tauri::command]
pub fn quarantine_list(state: State<'_, AppState>) -> Result<QuarantineView> {
    let store = state.store();
    Ok(QuarantineView {
        items: store.held_quarantine()?,
        held_bytes: state.quarantine()?.held_bytes()?,
        retention_days: store.settings()?.quarantine_retention_days,
    })
}

#[tauri::command]
pub fn restore(
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::quarantine::RestoreOutcome> {
    state.quarantine()?.restore(&id, now_unix())
}

/// The only command that destroys data. Named so nobody calls it by accident.
#[tauri::command]
pub fn remove_permanently(state: State<'_, AppState>, id: String) -> Result<()> {
    state.quarantine()?.purge(&id, now_unix())
}

/// "Keep" — drop the finding from the results without remembering anything.
#[tauri::command]
pub fn keep(state: State<'_, AppState>, id: String) -> Result<()> {
    state.store().forget_candidate(&id)
}

/// "Don't show me this again."
#[tauri::command]
pub fn ignore(state: State<'_, AppState>, id: String, scope: IgnoreScope) -> Result<()> {
    let store = state.store();
    let candidate = store.candidate(&id)?;
    let now = now_unix();
    match scope {
        IgnoreScope::Path => store.ignore_path(&candidate.path, now)?,
        IgnoreScope::App => {
            let app = candidate
                .associated_app
                .clone()
                .unwrap_or_else(|| candidate.display_name.clone());
            store.ignore_app(&app, now)?;
        }
        IgnoreScope::Category => store.ignore_category(candidate.category, now)?,
    }
    store.forget_candidate(&id)
}

#[tauri::command]
pub fn clear_ignores(state: State<'_, AppState>) -> Result<()> {
    state.store().clear_ignores()
}

/// Show a finding to the user in Finder or Explorer.
#[tauri::command]
pub fn reveal(state: State<'_, AppState>, id: String) -> Result<()> {
    let candidate = state.store().candidate(&id)?;
    state.platform().reveal(&candidate.path)
}

#[tauri::command]
pub fn reveal_quarantined(state: State<'_, AppState>, id: String) -> Result<()> {
    let record = state.store().quarantine_record(&id)?;
    state.platform().reveal(&record.stored_path)
}

#[tauri::command]
pub fn space(state: State<'_, AppState>) -> Result<SpaceOverview> {
    state.space_overview()
}

#[tauri::command]
pub fn settings(state: State<'_, AppState>) -> Result<Settings> {
    state.store().settings()
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<Settings> {
    let mut settings = settings;
    // Retention is a fixed set of choices, not free input.
    if ![7, 14, 30].contains(&settings.quarantine_retention_days) {
        settings.quarantine_retention_days = 14;
    }
    state.store().save_settings(&settings)?;
    Ok(settings)
}

/// The folders Scuttle would look at, so Settings can show them without
/// guessing at platform conventions.
#[tauri::command]
pub fn suggested_roots(state: State<'_, AppState>) -> Vec<RootDescription> {
    state
        .platform()
        .default_scan_roots()
        .into_iter()
        .map(|known| RootDescription {
            label: known.label,
            path: known.path,
        })
        .collect()
}

#[tauri::command]
pub fn history(state: State<'_, AppState>) -> Result<Vec<HistoryEntry>> {
    state.store().history(100)
}

/// Developer mode: classify everything and change nothing.
#[tauri::command]
pub fn dry_run(state: State<'_, AppState>, request: RummageRequest) -> Result<DryRunReport> {
    state.dry_run(request)
}

/// A one-line description of the machine, for the about panel and bug reports.
#[tauri::command]
pub fn about(state: State<'_, AppState>) -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "platform": state.platform().name(),
        "quarantine_root": state.platform().quarantine_root(),
    })
}

// ---------------------------------------------------------------------------
// Event emission
// ---------------------------------------------------------------------------

pub(crate) fn emit_phase(app: &tauri::AppHandle, scan_id: &str, phase: Phase) {
    let _ = app.emit(
        events::PHASE,
        ScanEvent {
            scan_id: scan_id.to_string(),
            payload: phase,
        },
    );
}

pub(crate) fn emit_progress(app: &tauri::AppHandle, scan_id: &str, progress: &Progress) {
    let _ = app.emit(
        events::PROGRESS,
        ScanEvent {
            scan_id: scan_id.to_string(),
            payload: progress.clone(),
        },
    );
}

pub(crate) fn emit_found(app: &tauri::AppHandle, scan_id: &str, candidate: &CleanupCandidate) {
    let _ = app.emit(
        events::FOUND,
        ScanEvent {
            scan_id: scan_id.to_string(),
            payload: candidate.clone(),
        },
    );
}

pub(crate) fn emit_done(app: &tauri::AppHandle, summary: &ScanSummary) {
    let _ = app.emit(
        events::DONE,
        ScanEvent {
            scan_id: summary.scan_id.clone(),
            payload: summary.clone(),
        },
    );
}

pub(crate) fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Bytes, phrased for people. Exposed so the frontend does not reimplement the
/// rounding rules and end up disagreeing with the backend.
#[tauri::command]
pub fn format_bytes(bytes: u64) -> String {
    human_bytes(bytes)
}

/// The full command list, in one place.
pub fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        rummage,
        cancel_rummage,
        findings,
        finding,
        quarantine,
        quarantine_member,
        quarantine_group,
        quarantine_list,
        restore,
        remove_permanently,
        keep,
        ignore,
        clear_ignores,
        reveal,
        reveal_quarantined,
        space,
        settings,
        save_settings,
        suggested_roots,
        history,
        dry_run,
        about,
        format_bytes,
    ]
}

/// Set up state once the app has a handle to its own data directory.
pub fn init(app: &tauri::App) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let platform = crate::platform::current();
    let state = AppState::new(platform)?;

    // Anything past its retention window goes on launch, quietly.
    if let Ok(quarantine) = state.quarantine() {
        match quarantine.sweep_expired(now_unix()) {
            Ok(0) => {}
            Ok(n) => tracing::info!(removed = n, "swept expired quarantine items"),
            Err(err) => tracing::warn!(error = %err, "quarantine sweep failed"),
        }
    }
    // Old scans are not an archive; keep a few for the space overview.
    let _ = state.store().prune_scans(5);

    app.manage(state);
    Ok(())
}
