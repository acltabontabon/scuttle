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
mod moves;
mod state;
mod updates;

pub use dry_run::{DryRunReport, DryRunRow};
pub use moves::{
    CautionGroup, FindingResult, FindingStatus, MovePhase, MovePlan, MoveReport, MoveRequest,
    MoveSink, MoveSnapshot, PlannedItem, PlannedShape, PlannedStatus, Refusal, Unit,
};
pub use state::{AppState, InstallLease, Operation, OperationGuard, Priority};

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
    /// Progress of a move into the Drawer. One channel, for every job.
    pub const MOVE: &str = "scuttle://move";
    /// Where background mode stands. Only sent when something changes it.
    pub const BACKGROUND: &str = "scuttle://background";
    /// The whole state of updating, sent whole whenever any of it changes.
    pub const UPDATE: &str = "scuttle://update";
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
    /// How many Scuttle is confident enough about to sweep without being
    /// asked item by item, and how much that is worth. Counted over the whole
    /// pile rather than the capped `items`, because it is shown as a total.
    pub confident_count: u64,
    pub confident_bytes: u64,
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
    /// Whether these came from a rummage or from a background check.
    ///
    /// A background check reads no file contents, so it cannot have found
    /// duplicates or near-identical screenshots. The screen says so rather
    /// than letting their absence read as "there are none".
    pub kind: crate::storage::ScanKind,
    /// Whether the scan that produced these was stopped before it finished.
    pub cancelled: bool,
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
    let protected = state.protected_paths();
    for root in &roots {
        check_scan_root(&root.path, &protected)?;
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
            kind: crate::storage::ScanKind::Full,
            cancelled: false,
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
            confident_count: matching
                .iter()
                .filter(|c| c.recommended_action == RecommendedAction::Quarantine)
                .count() as u64,
            confident_bytes: matching
                .iter()
                .filter(|c| c.recommended_action == RecommendedAction::Quarantine)
                .map(|c| c.size)
                .sum(),
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
        // Reported from the scan that produced these findings. This was a
        // hardcoded default, so "N places Scuttle could not read" never
        // appeared and a partial scan looked exactly like a complete one.
        hiccups: scan.hiccups.clone(),
        piles,
        has_rummaged: true,
        kind: scan.kind,
        cancelled: scan.cancelled,
    })
}

#[tauri::command]
pub fn finding(state: State<'_, AppState>, id: String) -> Result<CleanupCandidate> {
    state.store().candidate(&id)
}

/// Move a finding into the drawer.
/// Move one member of a group finding — a specific duplicate or screenshot —
/// rather than the one Scuttle proposed.
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
/// Which members of a group finding will move, and which one is kept.
pub(super) struct GroupPlan {
    /// One entry per member that is not the keeper: the derived finding to
    /// move, or why that member cannot be.
    pub members: Vec<std::result::Result<CleanupCandidate, GroupRefusal>>,
    pub kept: String,
}

/// Resolve a group action's *policy* — which copy survives — from the group
/// data the scan recorded.
pub(super) fn plan_group(candidate: &CleanupCandidate, keep: KeepChoice) -> Result<GroupPlan> {
    if candidate.group.len() < 2 {
        return Err(ScuttleError::Refused(
            "That finding is a single thing, not a group.".into(),
        ));
    }
    // A group action is a person's choice about their own copies, so it is
    // held to what a person may choose — not to what Scuttle would sweep.
    // Cautions are checked per member at the gate.
    match candidate.assessment.eligibility {
        crate::safety::assess::Eligibility::Blocked => {
            return Err(ScuttleError::Refused(
                "Scuttle will not act on this at all.".into(),
            ))
        }
        crate::safety::assess::Eligibility::ExplicitOnly => {
            return Err(ScuttleError::Refused(
                "These belong to an application. Scuttle only moves application files one \
                 at a time, when you choose them."
                    .into(),
            ))
        }
        _ => {}
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

    let members = (0..candidate.group.len())
        .filter(|index| *index != keeper_index)
        .map(|index| {
            derive_member(candidate, index).map_err(|error| GroupRefusal {
                display_name: display_of(&candidate.group[index].path),
                reason: error.to_string(),
                code: error.code().to_string(),
            })
        })
        .collect();
    Ok(GroupPlan { members, kept })
}

pub(crate) fn run_group_action(
    state: &AppState,
    id: &str,
    keep: KeepChoice,
    acknowledged: &[crate::safety::assess::CautionKind],
) -> Result<GroupOutcome> {
    let candidate = state.store().candidate(id)?;
    let plan = plan_group(&candidate, keep)?;

    let mut held = Vec::new();
    let mut refused = Vec::new();
    for member in plan.members {
        let derived = match member {
            Ok(derived) => derived,
            Err(refusal) => {
                refused.push(refusal);
                continue;
            }
        };
        match state.hold_for_user(&derived, acknowledged) {
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
        kept: plan.kept,
        refused,
    })
}

/// What a bulk action did.
#[derive(Debug, Clone, Serialize)]
pub struct BulkOutcome {
    pub held: Vec<QuarantineRecord>,
    pub bytes: u64,
    /// Findings Scuttle would not move, and why. Partial success is normal:
    /// one file having changed since the scan is not a reason to abandon the
    /// rest.
    pub refused: Vec<GroupRefusal>,
}

/// Quarantine everything in one pile that Scuttle was already confident about.
///
/// The frontend sends a *category*, never a list of ids. Which findings are
/// eligible is decided here, from the recommended action the core computed —
/// so there is no request shape that can ask for a risky finding to be swept
/// up with the safe ones. Anything rated `Review` or `InspectOnly` has to be
/// opened and acted on individually, which is the whole point of those
/// ratings.
/// Sweep every pile at once.
///
/// The same rule as the per-pile sweep, applied across the floor: only
/// findings the core already rated `Quarantine` are eligible, so widening the
/// scope from one pile to all of them does not widen what may be touched. This
/// exists because the per-pile version made the ordinary case — "deal with all
/// of it" — into a tour of every category, and a cleanup tool that charges a
/// tour for the common path is not a cleanup tool.
/// `category: None` means every pile.
pub(crate) fn run_bulk_quarantine(
    state: &AppState,
    category: Option<Category>,
) -> Result<BulkOutcome> {
    let Some(scan) = state.store().latest_scan()? else {
        return Ok(BulkOutcome {
            held: Vec::new(),
            bytes: 0,
            refused: Vec::new(),
        });
    };

    let eligible: Vec<CleanupCandidate> = state
        .store()
        .candidates_for_scan(&scan.id)?
        .into_iter()
        .filter(|c| category.is_none_or(|wanted| c.category == wanted))
        .filter(|c| c.assessment.eligibility == crate::safety::assess::Eligibility::Suggested)
        .collect();

    let mut held = Vec::new();
    let mut refused = Vec::new();
    let mut bytes = 0u64;

    for candidate in eligible {
        // Each one still goes through the full gate: the stored finding is a
        // request, and being part of a batch does not make it an authorisation.
        match state.hold(&candidate) {
            Ok(record) => {
                bytes += record.size;
                held.push(record);
            }
            Err(error) => refused.push(GroupRefusal {
                display_name: candidate.display_name.clone(),
                reason: error.to_string(),
                code: error.code().to_string(),
            }),
        }
    }

    Ok(BulkOutcome {
        held,
        bytes,
        refused,
    })
}

/// Quarantine a list of findings the user picked out themselves.
///
/// This *does* take ids, which the per-pile sweep deliberately does not, and
/// the distinction is the whole design. A sweep decides on your behalf, so it
/// may only ever touch what the core already rated `Quarantine`. This acts on
/// a selection a person built by hand with the size and risk of every item in
/// front of them, so it will move `Review` and `InspectOnly` findings too.
/// Ticking twelve boxes is twelve decisions, not one.
///
/// Refusing `InspectOnly` here would confuse an opinion with a prohibition.
/// `InspectOnly` means *cleanup is not suggested* — a statement about what
/// Scuttle does unprompted. Enforced as a lock it makes the largest piles
/// most people have, screenshots and heavy strays, impossible to act on at
/// all, which is not caution; it is an application that cannot do its job.
///
/// [`Risk::Protected`] is the real prohibition, and it is enforced where it
/// belongs: inside [`safety::authorize`], against the live filesystem, for
/// every item individually. Nothing routed through here can get past it.
pub(crate) fn run_quarantine_many(
    state: &AppState,
    ids: &[String],
    acknowledged: &[crate::safety::assess::CautionKind],
) -> Result<BulkOutcome> {
    let mut held = Vec::new();
    let mut refused = Vec::new();
    let mut bytes = 0u64;

    for id in ids {
        let candidate = match state.store().candidate(id) {
            Ok(candidate) => candidate,
            Err(error) => {
                refused.push(GroupRefusal {
                    display_name: id.clone(),
                    reason: error.to_string(),
                    code: error.code().to_string(),
                });
                continue;
            }
        };

        // `hold_for_user`, not `hold`: the full safety gate still runs against
        // the live filesystem for every item — protected table, containment,
        // links, staleness — and only Scuttle's own "not suggested" verdict
        // steps aside, because someone looked at this one and chose it.
        match state.hold_for_user(&candidate, acknowledged) {
            Ok(record) => {
                bytes += record.size;
                held.push(record);
            }
            Err(error) => refused.push(GroupRefusal {
                display_name: candidate.display_name.clone(),
                reason: error.to_string(),
                code: error.code().to_string(),
            }),
        }
    }

    Ok(BulkOutcome {
        held,
        bytes,
        refused,
    })
}

/// A candidate pointing at one member of a group.
///
/// It carries the group's evidence and verdict but the member's own path and
/// scan-time state, so the staleness check still means something for the file
/// actually being moved.
pub(super) fn derive_member(
    candidate: &CleanupCandidate,
    index: usize,
) -> Result<CleanupCandidate> {
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

pub(super) fn display_of(path: &std::path::Path) -> String {
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

/// Run something that changes files or the Drawer on a blocking thread, while
/// holding the operation gate.
///
/// A synchronous `#[tauri::command]` runs on the main thread, the thread that
/// draws the window; restoring or emptying a Drawer of a large folder's files
/// can take a long time. The gate makes it one such operation at a time, so a
/// restore cannot run underneath a move, an empty, or a scan.
async fn gated<T, F>(state: &AppState, operation: Operation, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&AppState) -> Result<T> + Send + 'static,
{
    let guard = state.begin_operation(operation)?;
    let handle = state.clone_handle();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        work(&handle)
    })
    .await
    .map_err(|e| ScuttleError::Internal(format!("that stopped unexpectedly: {e}")))?
}

#[tauri::command]
pub async fn restore(
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::quarantine::RestoreOutcome> {
    gated(state.inner(), Operation::Restore, move |state| {
        state.quarantine()?.restore(&id, now_unix())
    })
    .await
}

/// The only command that destroys data. Named so nobody calls it by accident.
#[tauri::command]
pub async fn remove_permanently(state: State<'_, AppState>, id: String) -> Result<()> {
    gated(state.inner(), Operation::RemoveItem, move |state| {
        state.quarantine()?.purge(&id, now_unix())
    })
    .await
}

/// Empty the drawer. Destroys data, in bulk, and is the only thing in Scuttle
/// that gives the user disk space back — quarantine is a move, not a deletion,
/// so until this runs nothing has actually been freed.
///
/// It takes no arguments on purpose: the drawer is the set, and there is no
/// request shape here that can name a path.
#[tauri::command]
pub async fn empty_drawer(state: State<'_, AppState>) -> Result<crate::quarantine::PurgeOutcome> {
    gated(state.inner(), Operation::EmptyDrawer, |state| {
        state.quarantine()?.purge_all(now_unix())
    })
    .await
}

/// "Keep" — drop the finding from the results without remembering anything.
#[tauri::command]
pub fn keep(state: State<'_, AppState>, id: String) -> Result<()> {
    // Changing the findings while a move works from them would pull the rug
    // from under it.
    let _guard = state.begin_operation(Operation::Decide)?;
    state.store().forget_candidate(&id)
}

/// "Don't show me this again."
#[tauri::command]
pub fn ignore(state: State<'_, AppState>, id: String, scope: IgnoreScope) -> Result<()> {
    let _guard = state.begin_operation(Operation::Decide)?;
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

/// What Scuttle has been told to leave alone.
///
/// Flattened into one list because the settings screen shows them together;
/// the `kind` is what `stop_ignoring` needs to find the row again.
#[derive(Debug, Clone, Serialize)]
pub struct IgnoredEntry {
    pub kind: crate::scanning::IgnoreKind,
    /// The stored value: a path, an app name, or a category slug.
    pub value: String,
    /// What to show a person. For a category this is its title.
    pub label: String,
}

#[tauri::command]
pub fn ignored(state: State<'_, AppState>) -> Result<Vec<IgnoredEntry>> {
    let set = state.store().ignore_set()?;
    let mut out = Vec::new();
    for path in set.paths {
        out.push(IgnoredEntry {
            kind: crate::scanning::IgnoreKind::Path,
            label: path.display().to_string(),
            value: path.display().to_string(),
        });
    }
    for app in set.apps {
        out.push(IgnoredEntry {
            kind: crate::scanning::IgnoreKind::App,
            label: app.clone(),
            value: app,
        });
    }
    for category in set.categories {
        out.push(IgnoredEntry {
            kind: crate::scanning::IgnoreKind::Category,
            label: format!("Everything in {}", category.title()),
            value: category.slug().to_string(),
        });
    }
    Ok(out)
}

/// Let one ignored thing be mentioned again. Touches no files.
#[tauri::command]
pub fn stop_ignoring(
    state: State<'_, AppState>,
    kind: crate::scanning::IgnoreKind,
    value: String,
) -> Result<()> {
    state.store().unignore(kind, &value)
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

/// Open the drawer's own folder, rather than one item inside it. Settings
/// shows the location, so it needs a way to get there with nothing held.
#[tauri::command]
pub fn reveal_quarantine_root(state: State<'_, AppState>) -> Result<()> {
    let root = state.platform().quarantine_root();
    state.platform().reveal(&root)
}

/// Measure the volume.
///
/// `async`, and the measuring itself handed to a blocking thread, because a
/// synchronous `#[tauri::command]` runs on the main thread — the same thread
/// that draws the window. This walks real directories and takes seconds on a
/// full disk, so as a synchronous command it froze the entire interface for
/// the duration: no repaint, no animation, and no possibility of showing a
/// loading state, since the code that would draw one could not run either.
///
/// `rummage` has always known this and put its scan on a worker. This is the
/// same rule, applied to the other command that touches the whole disk.
#[tauri::command]
pub async fn space(state: State<'_, AppState>) -> Result<SpaceOverview> {
    let handle = state.clone_handle();
    tauri::async_runtime::spawn_blocking(move || handle.space_overview())
        .await
        .map_err(|e| ScuttleError::Internal(format!("measuring stopped unexpectedly: {e}")))?
}

#[tauri::command]
pub fn settings(state: State<'_, AppState>) -> Result<Settings> {
    state.store().settings()
}

#[tauri::command]
pub fn save_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings> {
    let mut settings = settings;
    // Retention is a fixed set of choices, not free input.
    if ![7, 14, 30].contains(&settings.quarantine_retention_days) {
        settings.quarantine_retention_days = 14;
    }

    // Each of these depends on the one above it, and the dependency is
    // enforced here rather than trusted to the interface. A check cannot
    // happen if closing the window quits, and there is nothing to notify
    // about if no checks happen — a toggle left on in either of those states
    // would be a promise Scuttle cannot keep.
    if !settings.background_mode {
        settings.background_checks = false;
    }
    if !settings.background_checks {
        settings.background_notify = false;
    }

    // A folder to look in is also, after a scan, the boundary of what may be
    // moved — so the same rules apply to it as to anything Scuttle moves.
    let protected = state.protected_paths();
    for root in &settings.scan_roots {
        check_scan_root(root, &protected)?;
    }

    let previous = state.store().settings()?;
    state.store().save_settings(&settings)?;

    // Turning update checks on checks now rather than at the next daily wake.
    if !previous.auto_check_updates && settings.auto_check_updates {
        if let Some(switch) = app.try_state::<std::sync::Arc<crate::updater::AutoCheckSwitch>>() {
            switch.wake.notify_one();
        }
    }

    // The scheduler's existence follows the setting rather than recording
    // what was true at launch.
    if previous.background_mode != settings.background_mode {
        state.sync_scheduler(&app);
    }
    state.nudge_scheduler();
    crate::tray::refresh(&app);
    emit_background(&app, state.inner());

    Ok(settings)
}

/// Refuse a folder that must never become the area Scuttle may act in: a
/// filesystem or drive root, a folder directly beneath one (`/Users`,
/// `C:\\Windows`), anything protected, or Scuttle's own storage. The home
/// folder itself is allowed — looking is harmless, and everything structural
/// inside it is still refused at the gate.
pub(crate) fn check_scan_root(
    root: &std::path::Path,
    protected: &crate::safety::ProtectedPaths,
) -> Result<()> {
    let normalized = crate::safety::paths::normalize(root);
    let named = normalized
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .count();
    if !normalized.is_absolute() || named <= 1 {
        return Err(ScuttleError::Refused(format!(
            "{} is too close to the root of the disk for Scuttle to look after. Choose a \
             folder inside it.",
            root.display()
        )));
    }
    if let Some(rule) = protected.rule_for(&normalized) {
        return Err(ScuttleError::Refused(format!(
            "{} is protected ({}). Scuttle does not look there.",
            root.display(),
            rule.name
        )));
    }
    Ok(())
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

/// Tell the window where background mode stands.
///
/// Emitted whenever something changes it — a check finishing, a pause, the
/// tray being used. Events are not replayed, so the window also asks on
/// becoming visible; this is the same arrangement moves already use.
pub(crate) fn emit_background<R: tauri::Runtime>(app: &tauri::AppHandle<R>, state: &AppState) {
    if let Ok(status) = background_status_of(state, app) {
        let _ = app.emit(events::BACKGROUND, status);
    }
}

pub(crate) fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---------------------------------------------------------------------------
// Background mode
// ---------------------------------------------------------------------------

/// Everything the interface needs to describe background mode honestly,
/// including the parts where the answer is "the system said no".
#[derive(Debug, Clone, Serialize)]
pub struct BackgroundStatus {
    /// Whether the tray icon actually exists. When this is false the setting
    /// may be on and yet closing the window still quits, and the interface
    /// says so rather than letting the setting read as a promise.
    pub tray_alive: bool,
    pub mode: bool,
    pub checks: bool,
    pub notify: bool,
    /// Whether the system is currently allowing notifications. `None` when it
    /// has not been asked.
    pub notifications_permitted: Option<bool>,
    /// What the operating system reports, not what was stored.
    pub launch_at_login: bool,
    /// Whether launch at login could be read or changed at all here.
    pub launch_at_login_available: bool,
    pub last_check_unix: i64,
    pub paused_until_unix: i64,
    /// The core's own clock, sent so the window never has to consult its own.
    ///
    /// Everything the interface says about background mode — whether a pause
    /// has expired, how long ago the last check was — is a comparison against
    /// a timestamp the core produced. Comparing it against a second clock
    /// would mean two answers that can disagree, and a render that is not a
    /// pure function of what it was given.
    pub now_unix: i64,
    /// A scan the user has not looked at yet.
    pub pending_review: Option<String>,
    /// Why nothing is happening at the moment, phrased for a person.
    pub waiting_because: String,
}

fn background_status_of<R: tauri::Runtime>(
    state: &AppState,
    app: &tauri::AppHandle<R>,
) -> Result<BackgroundStatus> {
    let settings = state.store().settings()?;
    let persisted = state.store().background_state()?;

    let (launch_at_login, launch_at_login_available) = match login_item_enabled(app) {
        Some(on) => (on, true),
        None => (false, false),
    };

    let moment = crate::background::Moment {
        launched_unix: state.launched_unix(),
        window_visible: state.window_visible(),
        busy: state.current_operation().is_some(),
        has_roots: !state.resolved_roots().is_empty(),
    };
    // Asking why, not deciding whether: the conditions are not read here,
    // because this runs whenever the window asks and must stay cheap.
    let waiting_because = match crate::background::schedule::decide(
        now_unix(),
        &settings,
        &persisted,
        &moment,
        Default::default,
    ) {
        crate::background::Decision::Skip(reason) => reason.say(),
        _ => "Ready to look when the machine is free.".into(),
    };

    Ok(BackgroundStatus {
        tray_alive: state.tray_alive(),
        mode: settings.background_mode,
        checks: settings.background_checks,
        notify: settings.background_notify,
        notifications_permitted: notifications_permitted(app),
        launch_at_login,
        launch_at_login_available,
        last_check_unix: persisted.last_completed_unix,
        paused_until_unix: persisted.paused_until_unix,
        now_unix: now_unix(),
        pending_review: persisted.pending_review,
        waiting_because,
    })
}

fn notifications_permitted<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<bool> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .permission_state()
        .ok()
        .map(|state| matches!(state, tauri_plugin_notification::PermissionState::Granted))
}

fn login_item_enabled<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<bool> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().ok()
}

/// Where background mode stands. Cheap enough to call on every window show.
#[tauri::command]
pub fn background_status(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<BackgroundStatus> {
    background_status_of(state.inner(), &app)
}

/// Stop checking until the start of tomorrow, or start again now.
///
/// The offset comes from the window because the core has no business
/// deciding what day it is where the user lives.
#[tauri::command]
pub fn pause_background(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    paused: bool,
    utc_offset_secs: i32,
) -> Result<BackgroundStatus> {
    let mut persisted = state.store().background_state()?;
    persisted.paused_until_unix = if paused {
        crate::background::schedule::tomorrow_unix(now_unix(), utc_offset_secs)
    } else {
        0
    };
    state.store().save_background_state(&persisted)?;
    state.nudge_scheduler();
    background_status_of(state.inner(), &app)
}

/// Turn the login item on or off, and report what the system actually did.
///
/// The returned value is read back from the platform rather than assumed: a
/// setting that says "on" when the operating system disagrees is worse than
/// one that admits it could not.
#[tauri::command]
pub fn set_launch_at_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<bool> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let attempt = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(err) = attempt {
        tracing::warn!(error = %err, "the system would not change the login item");
    }

    let actual = manager.is_enabled().unwrap_or(false);
    let mut settings = state.store().settings()?;
    settings.launch_at_login = actual;
    state.store().save_settings(&settings)?;
    Ok(actual)
}

/// Ask for notification permission, at the moment the user turns them on.
///
/// Returns whether they are allowed. A refusal is not an error: everything
/// else about background mode keeps working, and the interface says what it
/// can and cannot do.
#[tauri::command]
pub async fn request_notification_permission(app: tauri::AppHandle) -> bool {
    use tauri_plugin_notification::NotificationExt;
    let notifier = app.notification();
    if let Ok(tauri_plugin_notification::PermissionState::Granted) = notifier.permission_state() {
        return true;
    }
    matches!(
        notifier.request_permission(),
        Ok(tauri_plugin_notification::PermissionState::Granted)
    )
}

/// The user has seen what the last background check turned up.
#[tauri::command]
pub fn clear_pending_review(state: State<'_, AppState>) -> Result<()> {
    let mut persisted = state.store().background_state()?;
    persisted.pending_review = None;
    state.store().save_background_state(&persisted)?;
    Ok(())
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
        moves::start_move,
        moves::preview_move,
        moves::cancel_move,
        moves::move_status,
        moves::dismiss_move,
        moves::refresh_findings,
        quarantine_list,
        restore,
        remove_permanently,
        empty_drawer,
        keep,
        ignore,
        clear_ignores,
        ignored,
        stop_ignoring,
        reveal,
        reveal_quarantined,
        reveal_quarantine_root,
        space,
        settings,
        save_settings,
        suggested_roots,
        history,
        dry_run,
        about,
        format_bytes,
        background_status,
        pause_background,
        set_launch_at_login,
        request_notification_permission,
        clear_pending_review,
        updates::update_status,
        updates::check_for_update,
        updates::download_update,
        updates::install_update,
        updates::dismiss_update,
    ]
}

/// Set up state once the app has a handle to its own data directory.
///
/// Everything here belongs to the *process*, not to the window. With a tray,
/// a window can be hidden and shown many times in one run, and none of those
/// is a launch: reopening Scuttle must never be a fresh startup that quietly
/// expires part of someone's drawer.
pub fn init(app: &tauri::App) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let platform = crate::platform::current();
    let state = AppState::new(platform)?;

    if !state.claim_startup() {
        app.manage(state);
        return Ok(());
    }

    // Settle anything a crash left half-done *before* anything is allowed to
    // expire: an interrupted move must be understood, not swept.
    if let Ok(quarantine) = state.quarantine() {
        match quarantine.reconcile(now_unix()) {
            Ok(report) if report.settled > 0 || report.orphan_cells > 0 => tracing::info!(
                settled = report.settled,
                attention = report.attention,
                orphan_cells = report.orphan_cells,
                "settled an interrupted move"
            ),
            Ok(_) => {}
            Err(err) => tracing::warn!(error = %err, "could not settle interrupted moves"),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::check_scan_root;
    use crate::safety::ProtectedPaths;
    use std::path::Path;

    #[test]
    fn a_scan_root_must_be_somewhere_scuttle_can_look_after() {
        let protected = ProtectedPaths::for_home("/home/tester");
        assert!(check_scan_root(Path::new("/"), &protected).is_err());
        assert!(check_scan_root(Path::new("/home"), &protected).is_err());
        assert!(check_scan_root(Path::new("relative/path"), &protected).is_err());
        assert!(check_scan_root(Path::new("/home/tester/.ssh"), &protected).is_err());
        assert!(check_scan_root(Path::new("/home/tester"), &protected).is_ok());
        assert!(check_scan_root(Path::new("/home/tester/Downloads"), &protected).is_ok());
    }
}
