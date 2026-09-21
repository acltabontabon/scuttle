//! Moving findings into the Drawer, in the background.
//!
//! The window must stay alive from the instant someone clicks. So the command
//! that starts a move does almost nothing: it claims the operation gate, writes
//! down a job, starts a worker thread, and returns. Everything else — resolving
//! which findings, checking each one against the safety gate, moving files,
//! settling the Drawer's records — happens on the worker, and the interface is
//! told about it through one event channel.
//!
//! What the events promise:
//!
//! * Every snapshot carries a **job id** (rising with each job) and a **rev**
//!   (rising with every change to that job). A listener that ignores anything
//!   older than what it has already seen cannot be misled by a delayed or
//!   reordered event, and one that missed events can ask for the current
//!   snapshot and pick up from there.
//! * Progress is **counts, never lists**: a run over a hundred thousand files
//!   is a handful of small numbers, emitted at most every 100 ms.
//! * A snapshot is `completed`, `partial`, `failed` or `cancelled` only once the
//!   worker has stopped touching files and the Drawer's records are settled.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use super::state::{AppState, Operation};
use super::{derive_member, events, KeepChoice};
use crate::model::{Category, CleanupCandidate, TargetKind};
use crate::quarantine::contents::MoveObserver;
use crate::quarantine::transfer::{Ctl, FailureKind, IssueGroup, IssueLog, Tally};
use crate::safety::assess::{CautionKind, Eligibility};
use crate::safety::Bidding;
use crate::storage::SnapshotState;
use crate::{Result, ScuttleError};

/// The fewest milliseconds between two progress events for one job.
const THROTTLE: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------------
// What can be asked for
// ---------------------------------------------------------------------------

/// What to move. Findings are named by id, never by path, and what is eligible
/// is decided here, from the core's own verdicts.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MoveRequest {
    /// A selection a person built by hand, with each item's cautions in front
    /// of them. `retry` continues findings that already had a run, and only
    /// what did not move. Never moves an application folder.
    Selection {
        ids: Vec<String>,
        #[serde(default)]
        retry: bool,
        /// Cautions accepted for the whole batch, once each.
        #[serde(default)]
        acknowledged: Vec<CautionKind>,
    },
    /// One finding a person opened and chose on its own. The only request
    /// that can move an application folder — still past its cautions and every
    /// hard protection.
    Single {
        id: String,
        #[serde(default)]
        acknowledged: Vec<CautionKind>,
    },
    /// Everything Scuttle suggests, in one pile or on the whole floor. Only
    /// findings whose eligibility is `suggested`: nothing a person would need
    /// to acknowledge, and nothing belonging to an application.
    Confident {
        #[serde(default)]
        category: Option<Category>,
    },
    /// Every member of a group finding except the one to keep.
    Group {
        id: String,
        keep: KeepChoice,
        #[serde(default)]
        acknowledged: Vec<CautionKind>,
    },
    /// One member of a group finding.
    Member {
        id: String,
        member_index: usize,
        #[serde(default)]
        acknowledged: Vec<CautionKind>,
    },
}

// ---------------------------------------------------------------------------
// What is reported
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovePhase {
    /// Deciding what can move. The amount of work is not known yet.
    Checking,
    /// Moving files.
    Moving,
    /// Every file is dealt with; the Drawer's records are being settled.
    Finalizing,
    Completed,
    Partial,
    Failed,
    Cancelled,
}

impl MovePhase {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            MovePhase::Completed | MovePhase::Partial | MovePhase::Failed | MovePhase::Cancelled
        )
    }
}

/// One moment in a job. Small, and cheap to send often.
#[derive(Debug, Clone, Serialize)]
pub struct MoveSnapshot {
    pub job_id: u64,
    /// Rises with every change. The newest snapshot of a job has the largest.
    pub rev: u64,
    pub phase: MovePhase,
    /// A stop has been asked for; work is still winding down.
    pub cancelling: bool,
    /// Files (or whole items) to move. `None` until Scuttle knows, which is not
    /// while it is still checking.
    pub total: Option<u64>,
    pub processed: u64,
    pub moved: u64,
    pub skipped: u64,
    pub failed: u64,
    /// Bytes now in the Drawer.
    pub moved_bytes: u64,
    /// Bytes actually copied across a volume boundary. Zero for renames.
    pub copied_bytes: u64,
    /// The finding being worked on, by name. Never a path.
    pub label: String,
    /// Present on the last snapshot only.
    pub report: Option<MoveReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MoveReport {
    pub outcome: MovePhase,
    pub moved_files: u64,
    /// Bytes now in the Drawer. This is *moved*, not *freed*: nothing is
    /// deleted until the Drawer is emptied.
    pub moved_bytes: u64,
    pub skipped: u64,
    pub failed: u64,
    /// Planned work that was never attempted (a cancelled run).
    pub remaining: u64,
    pub cancelled: bool,
    /// For a group action, the copy that was kept.
    pub kept: Option<String>,
    pub findings: Vec<FindingResult>,
    /// Something a person should be told that is not about one finding.
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Moved,
    Partial,
    Skipped,
    Failed,
}

/// Scuttle's own gate said no, before or instead of any file operation.
#[derive(Debug, Clone, Serialize)]
pub struct Refusal {
    pub code: String,
    pub message: String,
}

/// What a finding's counts are counts *of*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// The files of a shared folder, moved one at a time.
    Files,
    /// A single file or a whole folder, moved as one thing.
    Item,
}

impl Unit {
    fn of(candidate: &CleanupCandidate) -> Unit {
        if candidate.is_shared_contents() {
            Unit::Files
        } else {
            Unit::Item
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FindingResult {
    pub finding_id: String,
    pub display_name: String,
    pub category: Option<Category>,
    pub unit: Unit,
    pub status: FindingStatus,
    pub moved: u64,
    pub skipped: u64,
    pub failed: u64,
    pub moved_bytes: u64,
    /// The Drawer record holding what moved.
    pub record_id: Option<String>,
    /// The finding is still there and its reviewed set no longer describes it.
    pub needs_refresh: bool,
    /// Whether "retry" could do something for this finding.
    pub retryable: bool,
    pub issues: Vec<IssueGroup>,
    pub refusal: Option<Refusal>,
}

impl MoveReport {
    fn empty(outcome: MovePhase) -> MoveReport {
        MoveReport {
            outcome,
            moved_files: 0,
            moved_bytes: 0,
            skipped: 0,
            failed: 0,
            remaining: 0,
            cancelled: false,
            kept: None,
            findings: Vec::new(),
            notice: None,
        }
    }
}

// ---------------------------------------------------------------------------
// The job book and the live job
// ---------------------------------------------------------------------------

/// Where events go. The desktop app sends them to the webview; tests collect
/// them.
pub trait MoveSink: Send + Sync {
    fn emit(&self, snapshot: &MoveSnapshot);
}

struct TauriSink(tauri::AppHandle);

impl MoveSink for TauriSink {
    fn emit(&self, snapshot: &MoveSnapshot) {
        let _ = self.0.emit(events::MOVE, snapshot);
    }
}

#[derive(Default)]
pub struct JobBook {
    next_id: u64,
    pub(super) active: Option<Arc<Job>>,
    /// The last job's final snapshot, until someone dismisses it. It is how an
    /// interface that reloaded, or missed the event, finds out what happened.
    last: Option<MoveSnapshot>,
}

pub(super) struct Job {
    id: u64,
    cancel: AtomicBool,
    snapshot: Mutex<MoveSnapshot>,
    last_emit: Mutex<Instant>,
    sink: Arc<dyn MoveSink>,
}

impl Job {
    /// Change the snapshot and, if enough time has passed or `force` is set,
    /// tell the listener. The lock is never held while emitting.
    fn update(&self, force: bool, change: impl FnOnce(&mut MoveSnapshot)) {
        let to_send = {
            let mut snapshot = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
            change(&mut snapshot);
            snapshot.rev += 1;
            let mut last = self.last_emit.lock().unwrap_or_else(|e| e.into_inner());
            if force || last.elapsed() >= THROTTLE {
                *last = Instant::now();
                Some(snapshot.clone())
            } else {
                None
            }
        };
        if let Some(snapshot) = to_send {
            // A listener that fails must not stop files being handled
            // correctly halfway through a move.
            let _ = std::panic::catch_unwind(AssertUnwindSafe(|| self.sink.emit(&snapshot)));
        }
    }

    fn current(&self) -> MoveSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Reports a contents run's tallies into the job, on top of what earlier
/// findings in the same job already did.
struct JobObserver<'a> {
    job: &'a Job,
    base: Tally,
}

impl MoveObserver for JobObserver<'_> {
    fn tally(&self, current: &Tally) {
        let base = self.base;
        self.job.update(false, |s| {
            s.processed = base.processed + current.processed;
            s.moved = base.moved + current.moved;
            s.skipped = base.skipped + current.skipped;
            s.failed = base.failed + current.failed;
            s.moved_bytes = base.moved_bytes + current.moved_bytes;
            s.copied_bytes = base.copied_bytes + current.copied_bytes;
        });
    }

    fn settling(&self) {
        self.job.update(true, |s| s.phase = MovePhase::Finalizing);
    }
}

impl AppState {
    /// Start a move in the background. Returns the job's id and the worker's
    /// handle (which the desktop app drops and tests wait on).
    ///
    /// Fails immediately with `busy` if anything else is changing files.
    pub fn spawn_move(
        &self,
        request: MoveRequest,
        sink: Arc<dyn MoveSink>,
    ) -> Result<(u64, std::thread::JoinHandle<()>)> {
        self.spawn_with(sink, move |state, job| execute(state, job, request))
    }

    /// The machinery of [`AppState::spawn_move`] around any piece of work that
    /// produces a report: the gate, the job book, the worker, the panic net and
    /// the ordering of the last events.
    pub(super) fn spawn_with<F>(
        &self,
        sink: Arc<dyn MoveSink>,
        work: F,
    ) -> Result<(u64, std::thread::JoinHandle<()>)>
    where
        F: FnOnce(&AppState, &Job) -> MoveReport + Send + 'static,
    {
        let guard = self.begin_operation(Operation::Move)?;

        let job = {
            let mut book = self.jobs();
            book.next_id += 1;
            let id = book.next_id;
            let job = Arc::new(Job {
                id,
                cancel: AtomicBool::new(false),
                snapshot: Mutex::new(MoveSnapshot {
                    job_id: id,
                    rev: 1,
                    phase: MovePhase::Checking,
                    cancelling: false,
                    total: None,
                    processed: 0,
                    moved: 0,
                    skipped: 0,
                    failed: 0,
                    moved_bytes: 0,
                    copied_bytes: 0,
                    label: String::new(),
                    report: None,
                }),
                last_emit: Mutex::new(Instant::now()),
                sink,
            });
            book.active = Some(Arc::clone(&job));
            book.last = None;
            job
        };

        let state = self.clone_handle();
        let worker_job = Arc::clone(&job);
        let spawned = std::thread::Builder::new()
            .name("scuttle-move".into())
            .spawn(move || run_job(state, worker_job, guard, work));
        match spawned {
            Ok(handle) => Ok((job.id, handle)),
            Err(e) => {
                let mut book = self.jobs();
                book.active = None;
                Err(ScuttleError::Internal(format!(
                    "could not start moving: {e}"
                )))
            }
        }
    }

    /// Ask the running move to stop. It stops between files, or between chunks
    /// of a copy; the snapshot says `cancelling` until the worker has actually
    /// finished. Returns whether there was a move to stop.
    pub fn cancel_move(&self) -> bool {
        let job = self.jobs().active.clone();
        match job {
            Some(job) => {
                job.cancel.store(true, Ordering::Relaxed);
                job.update(true, |s| s.cancelling = true);
                true
            }
            None => false,
        }
    }

    /// The running job's current snapshot, or the last finished job's.
    pub fn move_status(&self) -> Option<MoveSnapshot> {
        let book = self.jobs();
        match &book.active {
            Some(job) => Some(job.current()),
            None => book.last.clone(),
        }
    }

    /// Forget a finished job's outcome. Does nothing to a running one.
    pub fn dismiss_move(&self) {
        let mut book = self.jobs();
        if book.active.is_none() {
            book.last = None;
        }
    }
}

fn run_job<F>(state: AppState, job: Arc<Job>, guard: super::state::OperationGuard, work: F)
where
    F: FnOnce(&AppState, &Job) -> MoveReport,
{
    // Announce that work has begun. The interface already saw `checking` from
    // the start command's own snapshot; this is the first event.
    job.update(true, |_| {});

    // A panic on this thread must still release the gate and end the job with
    // a report. Release builds unwind for exactly this reason.
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| work(&state, &job)));
    let report = match outcome {
        Ok(report) => report,
        Err(_) => {
            tracing::error!("the move worker panicked");
            let mut report = MoveReport::empty(MovePhase::Failed);
            report.notice = Some(
                "Scuttle stopped unexpectedly while moving. Anything that already moved is in \
                 the Drawer; look there before trying again."
                    .into(),
            );
            report
        }
    };

    // The last snapshot: the worker has stopped, the records are settled.
    let terminal = report.outcome;
    job.update(true, |s| {
        s.phase = terminal;
        s.report = Some(report);
        s.processed = s.processed.min(s.total.unwrap_or(s.processed));
    });
    {
        let mut book = state.jobs();
        book.last = Some(job.current());
        book.active = None;
    }
    // Released only now, after the outcome is published, so a listener that
    // reacts to the last event by starting something never finds it busy.
    drop(guard);
}

// ---------------------------------------------------------------------------
// The work
// ---------------------------------------------------------------------------

struct Work {
    candidate: CleanupCandidate,
    bidding: Bidding,
    retry: bool,
    acknowledged: Vec<CautionKind>,
}

enum Planned {
    Contents,
    Whole,
}

fn refusal_of(error: &ScuttleError) -> Refusal {
    Refusal {
        code: error.code().to_string(),
        message: error.to_string(),
    }
}

fn result_for_error(
    candidate: Option<&CleanupCandidate>,
    id: &str,
    error: &ScuttleError,
) -> FindingResult {
    let mut issues = IssueLog::default();
    let mut refusal = None;
    let mut failed = 0;
    let mut skipped = 0;
    match error {
        ScuttleError::Transfer(failure) => {
            issues.add(failure);
            if failure.kind.is_change() || failure.kind.is_deliberate_skip() {
                skipped = 1;
            } else {
                failed = 1;
            }
        }
        ScuttleError::Storage(_) | ScuttleError::Io(_) | ScuttleError::Internal(_) => {
            refusal = Some(refusal_of(error));
            failed = 1;
        }
        _ => {
            refusal = Some(refusal_of(error));
            skipped = 1;
        }
    }
    FindingResult {
        finding_id: id.to_string(),
        display_name: candidate
            .map(|c| c.display_name.clone())
            .unwrap_or_else(|| id.to_string()),
        category: candidate.map(|c| c.category),
        unit: candidate.map(Unit::of).unwrap_or(Unit::Item),
        status: if failed > 0 {
            FindingStatus::Failed
        } else {
            FindingStatus::Skipped
        },
        moved: 0,
        skipped,
        failed,
        moved_bytes: 0,
        record_id: None,
        needs_refresh: matches!(error, ScuttleError::Stale(_)),
        retryable: issues.groups().iter().any(|g| g.kind.is_retryable()),
        issues: issues.into_groups(),
        refusal,
    }
}

/// Turn the request into concrete findings to move.
fn resolve(
    state: &AppState,
    request: MoveRequest,
    results: &mut Vec<FindingResult>,
    kept: &mut Option<String>,
) -> Vec<Work> {
    let mut work = Vec::new();
    match request {
        MoveRequest::Selection {
            ids,
            retry,
            acknowledged,
        } => {
            // The same finding twice is one decision, and one move.
            let mut unique: Vec<String> = Vec::with_capacity(ids.len());
            for id in ids {
                if !unique.contains(&id) {
                    unique.push(id);
                }
            }
            for id in unique {
                match state.store().candidate(&id) {
                    // Someone picked this exact item with its cautions in front
                    // of them, so Scuttle's own "not suggested" verdict stops
                    // binding. Every other check still applies.
                    Ok(candidate) => work.push(Work {
                        candidate,
                        bidding: Bidding::User,
                        retry,
                        acknowledged: acknowledged.clone(),
                    }),
                    Err(error) => results.push(result_for_error(None, &id, &error)),
                }
            }
        }
        MoveRequest::Single { id, acknowledged } => match state.store().candidate(&id) {
            Ok(candidate) => work.push(Work {
                candidate,
                bidding: Bidding::UserSpecific,
                retry: false,
                acknowledged,
            }),
            Err(error) => results.push(result_for_error(None, &id, &error)),
        },
        MoveRequest::Confident { category } => {
            let candidates = state
                .store()
                .latest_scan()
                .ok()
                .flatten()
                .and_then(|scan| state.store().candidates_for_scan(&scan.id).ok())
                .unwrap_or_default();
            for candidate in candidates {
                if category.is_none_or(|wanted| candidate.category == wanted)
                    && candidate.assessment.eligibility == Eligibility::Suggested
                {
                    work.push(Work {
                        candidate,
                        bidding: Bidding::Scuttle,
                        retry: false,
                        acknowledged: Vec::new(),
                    });
                }
            }
        }
        MoveRequest::Group {
            id,
            keep,
            acknowledged,
        } => match state.store().candidate(&id) {
            Err(error) => results.push(result_for_error(None, &id, &error)),
            Ok(candidate) => match super::plan_group(&candidate, keep) {
                Err(error) => results.push(result_for_error(Some(&candidate), &id, &error)),
                Ok(plan) => {
                    *kept = Some(plan.kept);
                    for member in plan.members {
                        match member {
                            // A person chose "keep the newest" for this group,
                            // so these are their moves — with the group's
                            // cautions acknowledged, not waived.
                            Ok(derived) => work.push(Work {
                                candidate: derived,
                                bidding: Bidding::User,
                                retry: false,
                                acknowledged: acknowledged.clone(),
                            }),
                            Err(refusal) => results.push(FindingResult {
                                finding_id: id.clone(),
                                display_name: refusal.display_name,
                                category: Some(candidate.category),
                                unit: Unit::Item,
                                status: FindingStatus::Skipped,
                                moved: 0,
                                skipped: 1,
                                failed: 0,
                                moved_bytes: 0,
                                record_id: None,
                                needs_refresh: false,
                                retryable: false,
                                issues: Vec::new(),
                                refusal: Some(Refusal {
                                    code: refusal.code,
                                    message: refusal.reason,
                                }),
                            }),
                        }
                    }
                }
            },
        },
        MoveRequest::Member {
            id,
            member_index,
            acknowledged,
        } => match state.store().candidate(&id) {
            Err(error) => results.push(result_for_error(None, &id, &error)),
            Ok(candidate) => match derive_member(&candidate, member_index) {
                Ok(derived) => work.push(Work {
                    candidate: derived,
                    bidding: Bidding::User,
                    retry: false,
                    acknowledged,
                }),
                Err(error) => results.push(result_for_error(Some(&candidate), &id, &error)),
            },
        },
    }
    // Parents before children, so a folder that moves whole is decided before
    // anything inside it — see `execute`.
    work.sort_by_key(|w| w.candidate.path.components().count());
    work
}

/// A finding inside a folder that is already moving as a whole: it goes with
/// its parent, once, rather than being moved (or failing to be moved) twice.
fn included_in(candidate: &CleanupCandidate, parent: &CleanupCandidate) -> FindingResult {
    FindingResult {
        finding_id: candidate.id.clone(),
        display_name: candidate.display_name.clone(),
        category: Some(candidate.category),
        unit: Unit::of(candidate),
        status: FindingStatus::Skipped,
        moved: 0,
        skipped: 0,
        failed: 0,
        moved_bytes: 0,
        record_id: None,
        needs_refresh: false,
        retryable: false,
        issues: Vec::new(),
        refusal: Some(Refusal {
            code: "included".into(),
            message: format!(
                "Inside {}, which was chosen to move as a whole, so it is handled with that folder.",
                parent.display_name
            ),
        }),
    }
}

fn execute(state: &AppState, job: &Job, request: MoveRequest) -> MoveReport {
    let quarantine = match state.quarantine() {
        Ok(quarantine) => quarantine,
        Err(error) => {
            let mut report = MoveReport::empty(MovePhase::Failed);
            report.notice = Some(error.to_string());
            return report;
        }
    };
    // Built once for the whole job, not once per file.
    let scope = state.action_scope();
    let now = super::now_unix();

    let mut results: Vec<FindingResult> = Vec::new();
    let mut kept = None;

    // ---- checking: what can move, and how much work that is ---------------
    let work = resolve(state, request, &mut results, &mut kept);
    let mut accepted: Vec<(Work, Planned, u64)> = Vec::new();
    for item in work {
        if job.cancelled() {
            break;
        }
        // Already inside a folder that is moving whole.
        if let Some((parent, _, _)) = accepted.iter().find(|(p, planned, _)| {
            matches!(planned, Planned::Whole)
                && p.candidate.target_kind == TargetKind::Directory
                && crate::safety::paths::is_strictly_within(&item.candidate.path, &p.candidate.path)
        }) {
            results.push(included_in(&item.candidate, &parent.candidate));
            continue;
        }
        job.update(false, |s| s.label = item.candidate.display_name.clone());
        let ctx = scope.ctx(item.bidding, &item.acknowledged);
        let planned = if item.candidate.is_shared_contents() {
            quarantine
                .plan_contents(&item.candidate, &ctx, item.retry)
                .map(|plan| (Planned::Contents, plan.files))
        } else {
            crate::safety::authorize(&item.candidate, &ctx).map(|_| (Planned::Whole, 1))
        };
        match planned {
            Ok((plan, units)) if units > 0 => accepted.push((item, plan, units)),
            Ok(_) => {
                // Nothing left in the reviewed set: not an error, just done.
            }
            Err(error) => {
                results.push(result_for_error(
                    Some(&item.candidate),
                    &item.candidate.id,
                    &error,
                ));
            }
        }
    }
    let total: u64 = accepted.iter().map(|(_, _, units)| *units).sum();
    job.update(true, |s| {
        s.phase = MovePhase::Moving;
        s.total = Some(total);
        s.label.clear();
    });

    // ---- moving -----------------------------------------------------------
    let ctl = Ctl {
        cancel: Some(&job.cancel),
        ..Ctl::default()
    };
    let mut done = Tally::default();
    let mut not_started = 0u64;
    let mut stopped_early = false;

    for (item, planned, units) in accepted {
        if job.cancelled() {
            stopped_early = true;
            not_started += units;
            results.push(unstarted(&item.candidate));
            continue;
        }
        job.update(false, |s| s.label = item.candidate.display_name.clone());
        let ctx = scope.ctx(item.bidding, &item.acknowledged);

        let result = match planned {
            Planned::Contents => {
                let observer = JobObserver { job, base: done };
                match quarantine.hold_contents(
                    &item.candidate,
                    &ctx,
                    now,
                    item.retry,
                    &ctl,
                    &observer,
                ) {
                    Ok(outcome) => {
                        done.processed += outcome.tally.processed;
                        done.moved += outcome.tally.moved;
                        done.skipped += outcome.tally.skipped;
                        done.failed += outcome.tally.failed;
                        done.moved_bytes += outcome.tally.moved_bytes;
                        done.copied_bytes += outcome.tally.copied_bytes;
                        if outcome.cancelled {
                            stopped_early = true;
                            not_started += outcome
                                .unmoved
                                .saturating_sub(outcome.tally.skipped + outcome.tally.failed);
                        }
                        contents_result(state, &item.candidate, &outcome)
                    }
                    Err(error) => {
                        done.processed += units;
                        let result =
                            result_for_error(Some(&item.candidate), &item.candidate.id, &error);
                        done.skipped += result.skipped * units;
                        done.failed += result.failed * units;
                        result
                    }
                }
            }
            Planned::Whole => match quarantine.hold_controlled(&item.candidate, &ctx, now, &ctl) {
                Ok(record) => {
                    done.processed += 1;
                    done.moved += 1;
                    done.moved_bytes += record.size;
                    FindingResult {
                        finding_id: item.candidate.id.clone(),
                        display_name: item.candidate.display_name.clone(),
                        category: Some(item.candidate.category),
                        unit: Unit::Item,
                        status: FindingStatus::Moved,
                        moved: 1,
                        skipped: 0,
                        failed: 0,
                        moved_bytes: record.size,
                        record_id: Some(record.id),
                        needs_refresh: false,
                        retryable: false,
                        issues: Vec::new(),
                        refusal: None,
                    }
                }
                Err(ScuttleError::Transfer(failure)) if failure.kind == FailureKind::Cancelled => {
                    stopped_early = true;
                    not_started += 1;
                    unstarted(&item.candidate)
                }
                Err(error) => {
                    done.processed += 1;
                    let result =
                        result_for_error(Some(&item.candidate), &item.candidate.id, &error);
                    done.skipped += result.skipped;
                    done.failed += result.failed;
                    result
                }
            },
        };
        results.push(result);
        job.update(false, |s| {
            s.processed = done.processed;
            s.moved = done.moved;
            s.skipped = done.skipped;
            s.failed = done.failed;
            s.moved_bytes = done.moved_bytes;
            s.copied_bytes = done.copied_bytes;
        });
    }

    // ---- finalizing -------------------------------------------------------
    // Each finding's records are settled as it finishes, so this is the last
    // accounting. It is a real phase only in that nothing is reported as done
    // until it has happened.
    job.update(true, |s| {
        s.phase = MovePhase::Finalizing;
        s.label.clear();
    });

    let cancelled = job.cancelled() && stopped_early;
    let moved_files: u64 = results.iter().map(|r| r.moved).sum();
    let moved_bytes: u64 = results.iter().map(|r| r.moved_bytes).sum();
    let skipped: u64 = results.iter().map(|r| r.skipped).sum();
    let failed: u64 = results.iter().map(|r| r.failed).sum();
    let all_moved = !results.is_empty()
        && results
            .iter()
            .all(|r| r.status == FindingStatus::Moved && r.issues.is_empty());

    let outcome = if cancelled {
        MovePhase::Cancelled
    } else if results.is_empty() || all_moved {
        MovePhase::Completed
    } else if moved_files == 0 {
        MovePhase::Failed
    } else {
        MovePhase::Partial
    };

    MoveReport {
        outcome,
        moved_files,
        moved_bytes,
        skipped,
        failed,
        remaining: not_started,
        cancelled,
        kept,
        findings: results,
        notice: None,
    }
}

fn unstarted(candidate: &CleanupCandidate) -> FindingResult {
    FindingResult {
        finding_id: candidate.id.clone(),
        display_name: candidate.display_name.clone(),
        category: Some(candidate.category),
        unit: Unit::of(candidate),
        status: FindingStatus::Skipped,
        moved: 0,
        skipped: 0,
        failed: 0,
        moved_bytes: 0,
        record_id: None,
        needs_refresh: false,
        retryable: true,
        issues: Vec::new(),
        refusal: None,
    }
}

fn contents_result(
    state: &AppState,
    candidate: &CleanupCandidate,
    outcome: &crate::quarantine::contents::ContentsOutcome,
) -> FindingResult {
    let needs_refresh = state
        .store()
        .snapshot_info(&candidate.id)
        .map(|info| info.state == SnapshotState::NeedsRefresh)
        .unwrap_or(false);
    let issues: Vec<IssueGroup> = outcome.issues.groups().to_vec();
    let retryable = issues.iter().any(|g| g.kind.is_retryable()) || outcome.cancelled;
    let tally = outcome.tally;
    let status = if tally.moved > 0 && tally.skipped == 0 && tally.failed == 0 && !outcome.cancelled
    {
        FindingStatus::Moved
    } else if tally.moved > 0 {
        FindingStatus::Partial
    } else if tally.failed > 0 {
        FindingStatus::Failed
    } else {
        FindingStatus::Skipped
    };
    FindingResult {
        finding_id: candidate.id.clone(),
        display_name: candidate.display_name.clone(),
        category: Some(candidate.category),
        unit: Unit::Files,
        status,
        moved: tally.moved,
        skipped: tally.skipped,
        failed: tally.failed,
        moved_bytes: tally.moved_bytes,
        record_id: outcome.record.as_ref().map(|r| r.id.clone()),
        needs_refresh,
        retryable,
        issues,
        refusal: None,
    }
}

// ---------------------------------------------------------------------------
// Review: what a move would do, before it does anything
// ---------------------------------------------------------------------------

/// What will happen to one finding if the reviewed move goes ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannedStatus {
    /// It will move, once any cautions are acknowledged.
    Ready,
    /// It cannot move by this request, and `note` says why.
    Refused,
    /// It is inside a folder in the same move and goes with it.
    Included,
}

/// What the move takes, in the terms a person would check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannedShape {
    /// One file.
    File,
    /// A whole folder, with everything in it.
    Folder,
    /// The files Scuttle reviewed inside a shared folder. The folder stays.
    FilesInside,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedItem {
    pub finding_id: String,
    pub display_name: String,
    /// The exact path that will move. Never a parent or a neighbour of what
    /// was shown.
    pub path: std::path::PathBuf,
    pub shape: PlannedShape,
    pub size: u64,
    /// Files and folders a folder move takes with it, counted just now.
    /// `None` for a single file, or when counting was stopped.
    pub contains: Option<u64>,
    pub confidence: crate::model::Confidence,
    /// The evidence, as sentences. Positive and negative alike.
    pub reasons: Vec<PlannedReason>,
    pub remark: Option<String>,
    pub impact: crate::safety::assess::Impact,
    pub eligibility: Eligibility,
    pub cautions: Vec<crate::safety::assess::Caution>,
    pub status: PlannedStatus,
    pub note: Option<String>,
    /// Kept in the Drawer until a person removes it, rather than expiring.
    pub kept_until_removed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedReason {
    pub summary: String,
    pub negative: bool,
}

/// One caution, to be acknowledged once for the whole move.
#[derive(Debug, Clone, Serialize)]
pub struct CautionGroup {
    pub kind: CautionKind,
    pub headline: String,
    /// How many of the ready items it applies to.
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MovePlan {
    pub items: Vec<PlannedItem>,
    pub cautions: Vec<CautionGroup>,
    pub ready: u64,
    pub ready_bytes: u64,
    /// Where things go.
    pub drawer: std::path::PathBuf,
    pub retention_days: u32,
}

/// Count what a folder holds, up to a limit. Links are counted, not followed.
fn count_inside(path: &std::path::Path) -> Option<u64> {
    const LIMIT: usize = 250_000;
    let mut count = 0u64;
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .take(LIMIT + 1)
    {
        if entry.is_ok() {
            count += 1;
        }
    }
    if count as usize > LIMIT {
        None
    } else {
        Some(count.saturating_sub(1))
    }
}

impl AppState {
    /// Work out, without touching anything, what a move request would do:
    /// exactly which paths, file or folder, why, what needs acknowledging and
    /// what will be refused. The same gate as the move itself, run with every
    /// caution provisionally accepted so that only the real refusals show.
    pub fn preview_move(&self, request: MoveRequest) -> Result<MovePlan> {
        let mut results = Vec::new();
        let mut kept = None;
        let work = resolve(self, request, &mut results, &mut kept);
        let scope = self.action_scope();
        let all = CautionKind::ALL;
        let mut items: Vec<PlannedItem> = Vec::new();
        let mut folders: Vec<(std::path::PathBuf, String)> = Vec::new();

        for refused in results {
            items.push(PlannedItem {
                finding_id: refused.finding_id.clone(),
                display_name: refused.display_name.clone(),
                path: std::path::PathBuf::new(),
                shape: PlannedShape::File,
                size: 0,
                contains: None,
                confidence: crate::model::Confidence::Low,
                reasons: Vec::new(),
                remark: None,
                impact: crate::safety::assess::Impact::PersonalFile,
                eligibility: Eligibility::Blocked,
                cautions: Vec::new(),
                status: PlannedStatus::Refused,
                note: refused.refusal.map(|r| r.message),
                kept_until_removed: true,
            });
        }

        for item in work {
            let c = &item.candidate;
            let ctx = scope.ctx(item.bidding, &all);
            let assessment = crate::safety::assess_live(c, &ctx);
            let shape = if c.is_shared_contents() {
                PlannedShape::FilesInside
            } else if c.target_kind == TargetKind::Directory {
                PlannedShape::Folder
            } else {
                PlannedShape::File
            };

            let parent = folders
                .iter()
                .find(|(dir, _)| crate::safety::paths::is_strictly_within(&c.path, dir));
            let (status, note) = if let Some((_, name)) = parent {
                (
                    PlannedStatus::Included,
                    Some(format!("Inside {name}, which moves as a whole.")),
                )
            } else {
                let checked = if c.is_shared_contents() {
                    crate::safety::authorize_contents(c, &ctx).map(|_| ())
                } else {
                    crate::safety::authorize(c, &ctx).map(|_| ())
                };
                match checked {
                    Ok(()) => (PlannedStatus::Ready, None),
                    Err(error) => (PlannedStatus::Refused, Some(error.to_string())),
                }
            };
            if status == PlannedStatus::Ready && shape == PlannedShape::Folder {
                folders.push((c.path.clone(), c.display_name.clone()));
            }
            let contains = if shape == PlannedShape::Folder && status == PlannedStatus::Ready {
                count_inside(&c.path)
            } else {
                None
            };
            items.push(PlannedItem {
                finding_id: c.id.clone(),
                display_name: c.display_name.clone(),
                path: c.path.clone(),
                shape,
                size: c.size,
                contains,
                confidence: c.confidence,
                reasons: c
                    .evidence
                    .iter()
                    .map(|e| PlannedReason {
                        summary: e.summary.clone(),
                        negative: e.negative,
                    })
                    .collect(),
                remark: c.remark.clone(),
                impact: assessment.impact,
                eligibility: assessment.eligibility,
                kept_until_removed: crate::quarantine::keeps(&assessment),
                cautions: assessment.cautions,
                status,
                note,
            });
        }

        let mut cautions: Vec<CautionGroup> = Vec::new();
        for item in items.iter().filter(|i| i.status == PlannedStatus::Ready) {
            for caution in &item.cautions {
                match cautions.iter_mut().find(|g| g.kind == caution.kind) {
                    Some(group) => group.count += 1,
                    None => cautions.push(CautionGroup {
                        kind: caution.kind,
                        headline: caution.kind.headline().to_string(),
                        count: 1,
                    }),
                }
            }
        }
        cautions.sort_by_key(|g| g.kind);
        let ready: Vec<&PlannedItem> = items
            .iter()
            .filter(|i| i.status == PlannedStatus::Ready)
            .collect();
        Ok(MovePlan {
            ready: ready.len() as u64,
            ready_bytes: ready.iter().map(|i| i.size).sum(),
            items,
            cautions,
            drawer: self.platform().quarantine_root(),
            retention_days: self.store().settings()?.quarantine_retention_days,
        })
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MoveStarted {
    pub job_id: u64,
}

/// Start moving findings into the Drawer. Returns at once; progress arrives on
/// the `scuttle://move` channel, and `move_status` tells a listener that
/// arrived late where things stand.
#[tauri::command]
pub fn start_move(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: MoveRequest,
) -> Result<MoveStarted> {
    let probe = std::env::var_os("SCUTTLE_PROBE").is_some();
    let (job_id, _worker) = state.spawn_move(request, Arc::new(TauriSink(app.clone())))?;
    if probe {
        spawn_main_thread_probe(app, state.clone_handle());
    }
    Ok(MoveStarted { job_id })
}

/// Measure how long the window's own thread takes to answer while a move runs.
///
/// Off unless `SCUTTLE_PROBE` is set. Every 20 ms it asks the main thread — the
/// one that draws the window and handles input — to run a trivial closure, and
/// times the round trip. If a move ever blocked that thread, the round trips
/// would stretch to match; the worst one is logged when the move ends. It sends
/// nothing anywhere and reads no files.
fn spawn_main_thread_probe(app: tauri::AppHandle, state: AppState) {
    let _ = std::thread::Builder::new()
        .name("scuttle-probe".into())
        .spawn(move || {
            let mut worst = Duration::ZERO;
            let mut total = Duration::ZERO;
            let mut samples = 0u32;
            while state.current_operation() == Some(Operation::Move) {
                let (tx, rx) = std::sync::mpsc::channel();
                let sent = Instant::now();
                if app
                    .run_on_main_thread(move || {
                        let _ = tx.send(());
                    })
                    .is_err()
                {
                    break;
                }
                if rx.recv_timeout(Duration::from_secs(30)).is_ok() {
                    let took = sent.elapsed();
                    worst = worst.max(took);
                    total += took;
                    samples += 1;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if samples > 0 {
                tracing::info!(
                    samples,
                    mean_ms = (total / samples).as_millis() as u64,
                    worst_ms = worst.as_millis() as u64,
                    "main-thread latency during a move"
                );
            }
        });
}

/// What a move would do, reviewed before anything moves. Reads only.
#[tauri::command]
pub async fn preview_move(state: State<'_, AppState>, request: MoveRequest) -> Result<MovePlan> {
    let handle = state.clone_handle();
    tauri::async_runtime::spawn_blocking(move || handle.preview_move(request))
        .await
        .map_err(|e| ScuttleError::Internal(format!("reviewing stopped unexpectedly: {e}")))?
}

/// Ask the running move to stop. Returns whether there was one.
#[tauri::command]
pub fn cancel_move(state: State<'_, AppState>) -> bool {
    state.cancel_move()
}

/// The running move's snapshot, or the last finished one's until dismissed.
#[tauri::command]
pub fn move_status(state: State<'_, AppState>) -> Option<MoveSnapshot> {
    state.move_status()
}

/// Let go of a finished move's outcome.
#[tauri::command]
pub fn dismiss_move(state: State<'_, AppState>) {
    state.dismiss_move();
}

/// Review findings again: take a fresh set of the files each one covers and
/// re-measure it, so it can be looked at and chosen anew. Moves nothing.
#[tauri::command]
pub async fn refresh_findings(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<CleanupCandidate>> {
    let guard = state.begin_operation(Operation::Refresh)?;
    let handle = state.clone_handle();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        handle.refresh_findings(&ids)
    })
    .await
    .map_err(|e| ScuttleError::Internal(format!("reviewing stopped unexpectedly: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::testing::FixedPlatform;

    struct Collect(Mutex<Vec<MoveSnapshot>>);
    impl MoveSink for Collect {
        fn emit(&self, s: &MoveSnapshot) {
            self.0.lock().unwrap().push(s.clone());
        }
    }

    fn state() -> (tempfile::TempDir, AppState) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let state = AppState::new(Arc::new(FixedPlatform::new(&home))).unwrap();
        (tmp, state)
    }

    #[test]
    fn a_panic_on_the_worker_ends_the_job_with_a_report_and_frees_the_gate() {
        let (_tmp, state) = state();
        let sink = Arc::new(Collect(Mutex::new(Vec::new())));
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let (_, worker) = state
            .spawn_with(sink.clone(), |_, _| panic!("something impossible"))
            .unwrap();
        worker.join().unwrap();
        std::panic::set_hook(hook);

        let last = sink.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.phase, MovePhase::Failed);
        let notice = last.report.unwrap().notice.unwrap();
        assert!(notice.contains("Drawer"), "it says where to look: {notice}");

        assert_eq!(state.current_operation(), None, "the gate is released");
        assert_eq!(state.move_status().unwrap().phase, MovePhase::Failed);
        state
            .begin_operation(Operation::Restore)
            .expect("nothing is stuck busy");
    }

    #[test]
    fn a_listener_that_panics_cannot_take_the_move_down_with_it() {
        struct Faulty;
        impl MoveSink for Faulty {
            fn emit(&self, _: &MoveSnapshot) {
                panic!("the webview went away");
            }
        }
        let (_tmp, state) = state();
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let (_, worker) = state
            .spawn_with(Arc::new(Faulty), |_, job| {
                job.update(true, |s| s.phase = MovePhase::Moving);
                MoveReport::empty(MovePhase::Completed)
            })
            .unwrap();
        worker.join().unwrap();
        std::panic::set_hook(hook);

        assert_eq!(state.move_status().unwrap().phase, MovePhase::Completed);
        assert_eq!(state.current_operation(), None);
    }

    #[test]
    fn an_error_from_the_work_still_releases_the_gate_and_publishes_an_outcome() {
        let (_tmp, state) = state();
        let (_, worker) = state
            .spawn_with(Arc::new(Collect(Mutex::new(Vec::new()))), |_, _| {
                let mut report = MoveReport::empty(MovePhase::Failed);
                report.notice = Some("nothing to do".into());
                report
            })
            .unwrap();
        worker.join().unwrap();
        assert_eq!(state.current_operation(), None);
        assert_eq!(state.move_status().unwrap().phase, MovePhase::Failed);
    }
}
