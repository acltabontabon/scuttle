//! Application state: the one place the store, the platform and the running
//! scan are held together.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::model::CleanupCandidate;
use crate::platform::PlatformService;
use crate::quarantine::Quarantine;
use crate::safety::{ActionContext, Bidding, ProtectedPaths};
use crate::scanning::{
    self, CleanupObserver, Phase, Progress, ScanContext, ScanObserver, ScanOptions, ScanSummary,
};
use crate::space::SpaceOverview;
use crate::storage::{QuarantineRecord, Store};
use crate::{Result, ScuttleError};

/// What the safety gate needs, gathered once. Building the protected-path table
/// costs something, and a run over many findings should not rebuild it for each.
pub struct ActionScope {
    protected: ProtectedPaths,
    roots: Vec<PathBuf>,
}

impl ActionScope {
    pub fn ctx(&self, bidding: Bidding) -> ActionContext<'_> {
        ActionContext {
            protected: &self.protected,
            allowed_roots: &self.roots,
            bidding,
        }
    }
}

/// Shared, cheap to clone, safe to hand to a worker thread.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<Store>,
    platform: Arc<dyn PlatformService>,
    /// `Some` while a rummage is running.
    running: Mutex<Option<RunningScan>>,
    /// The roots of the most recent scan. Nothing outside them can be acted on.
    allowed_roots: Mutex<Vec<PathBuf>>,
    /// Who, if anyone, is changing files or the Drawer right now.
    gate: Mutex<Option<GateHold>>,
    /// The move in progress, and the last one's outcome.
    jobs: Mutex<super::moves::JobBook>,
}

struct RunningScan {
    id: String,
    cancel: Arc<AtomicBool>,
    /// Held for as long as the scan runs, so nothing that could invalidate its
    /// results starts underneath it.
    _guard: OperationGuard,
}

/// The things that change files, the Drawer, or the findings a selection was
/// made from. Scuttle does one of them at a time.
///
/// This is deliberately coarse for the alpha. A move and a restore touching
/// unrelated files would be safe in principle; proving that they are unrelated
/// — ownership of every record and path — is more machinery than the
/// guarantee is worth right now. Reads (the findings, the Drawer listing, the
/// space overview) are never gated, so the interface stays usable throughout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Scan,
    Move,
    Restore,
    EmptyDrawer,
    RemoveItem,
    Refresh,
    Sweep,
    /// Keeping or ignoring a finding: quick, but it changes what a selection
    /// was made from.
    Decide,
}

impl Operation {
    fn doing(&self) -> &'static str {
        match self {
            Operation::Scan => "looking around",
            Operation::Move => "moving files into the Drawer",
            Operation::Restore => "putting something back",
            Operation::EmptyDrawer => "emptying the Drawer",
            Operation::RemoveItem => "removing something from the Drawer",
            Operation::Refresh => "reviewing findings again",
            Operation::Sweep => "tidying the Drawer",
            Operation::Decide => "updating the findings",
        }
    }
}

struct GateHold {
    id: u64,
    operation: Operation,
}

/// Releases the operation gate when dropped — on success, on an error, and
/// when a worker unwinds from a panic.
pub struct OperationGuard {
    state: AppState,
    id: u64,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut gate = self.state.lock_gate();
        if gate.as_ref().is_some_and(|hold| hold.id == self.id) {
            *gate = None;
        }
    }
}

impl AppState {
    pub fn new(platform: Arc<dyn PlatformService>) -> Result<AppState> {
        let store = Arc::new(Store::open(&platform.data_dir().join("scuttle.db"))?);
        std::fs::create_dir_all(platform.quarantine_root())?;

        // Roots default to whatever the platform suggests, so a restart that
        // happens between a scan and an action does not strand the findings.
        let settings = store.settings()?;
        let roots = if settings.scan_roots.is_empty() {
            platform
                .default_scan_roots()
                .into_iter()
                .map(|k| k.path)
                .collect()
        } else {
            settings.scan_roots.clone()
        };

        Ok(AppState {
            inner: Arc::new(Inner {
                store,
                platform,
                running: Mutex::new(None),
                allowed_roots: Mutex::new(roots),
                gate: Mutex::new(None),
                jobs: Mutex::new(super::moves::JobBook::default()),
            }),
        })
    }

    pub fn clone_handle(&self) -> AppState {
        self.clone()
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn platform(&self) -> Arc<dyn PlatformService> {
        Arc::clone(&self.inner.platform)
    }

    pub fn quarantine(&self) -> Result<Quarantine> {
        let retention = self.store().settings()?.quarantine_retention_days;
        Ok(Quarantine::new(
            self.inner.platform.quarantine_root(),
            Arc::clone(&self.inner.store),
            retention,
        ))
    }

    /// Register a new scan, refusing if one is already running.
    pub fn start_scan(&self, options: &ScanOptions) -> Result<String> {
        let mut running = self.lock_running();
        if running.is_some() {
            return Err(ScuttleError::ScanBusy);
        }
        // A scan replaces the findings a selection was made from, so it cannot
        // start while something is moving them.
        let guard = self.begin_operation(Operation::Scan)?;
        let id = uuid::Uuid::new_v4().to_string();
        *running = Some(RunningScan {
            id: id.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            _guard: guard,
        });
        drop(running);

        *self.lock_roots() = options.roots.clone();
        self.store().begin_scan(&id, options, super::now_unix())?;
        Ok(id)
    }

    pub fn cancel_scan(&self) -> bool {
        match self.lock_running().as_ref() {
            Some(scan) => {
                scan.cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    fn finish_scan(&self) {
        // Dropping the scan releases the operation gate with it.
        let finished = self.lock_running().take();
        drop(finished);
    }

    /// Claim the right to change files or the Drawer, or be told what is in
    /// the way. The claim ends when the returned guard is dropped.
    pub fn begin_operation(&self, operation: Operation) -> Result<OperationGuard> {
        let mut gate = self.lock_gate();
        if let Some(hold) = gate.as_ref() {
            return Err(ScuttleError::Busy(format!(
                "Scuttle is busy {}. Try again when that finishes.",
                hold.operation.doing()
            )));
        }
        let id = self.next_gate_id();
        *gate = Some(GateHold { id, operation });
        Ok(OperationGuard {
            state: self.clone(),
            id,
        })
    }

    /// What holds the gate, if anything.
    pub fn current_operation(&self) -> Option<Operation> {
        self.lock_gate().as_ref().map(|hold| hold.operation)
    }

    fn next_gate_id(&self) -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    pub(super) fn jobs(&self) -> std::sync::MutexGuard<'_, super::moves::JobBook> {
        self.inner.jobs.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_gate(&self) -> std::sync::MutexGuard<'_, Option<GateHold>> {
        self.inner.gate.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run a scan to completion, emitting events as it goes.
    pub fn run_scan(
        &self,
        scan_id: &str,
        options: ScanOptions,
        app: &tauri::AppHandle,
    ) -> Result<ScanSummary> {
        let observer = EventObserver {
            app: app.clone(),
            scan_id: scan_id.to_string(),
        };
        let summary = self.run_scan_with(scan_id, options, &observer)?;
        super::emit_done(app, &summary);
        Ok(summary)
    }

    /// The scan itself, with no dependency on a window.
    ///
    /// Split out from [`AppState::run_scan`] so the whole command-layer path —
    /// start, scan, persist, read back — can be tested without a Tauri handle.
    pub fn run_scan_with(
        &self,
        scan_id: &str,
        options: ScanOptions,
        observer: &dyn ScanObserver,
    ) -> Result<ScanSummary> {
        let cancel = match self.lock_running().as_ref() {
            Some(scan) if scan.id == scan_id => Arc::clone(&scan.cancel),
            // Cancelled or superseded before the worker got going.
            _ => return Err(ScuttleError::ScanBusy),
        };

        let result = (|| {
            let ignores = self.store().ignore_set()?;
            let ctx = ScanContext::new(
                options.clone(),
                Arc::clone(&self.inner.platform),
                ignores,
                cancel,
            );
            let detectors = crate::detectors::default_set(&options);
            let outcome = scanning::run(scan_id, &ctx, detectors, observer);

            self.store().save_candidates(scan_id, &outcome.candidates)?;
            self.record_reviewed_sets(&outcome.candidates, &ctx);
            self.store()
                .finish_scan(&outcome.summary, super::now_unix())?;

            let mut settings = self.store().settings()?;
            if !settings.has_rummaged_before {
                settings.has_rummaged_before = true;
                let _ = self.store().save_settings(&settings);
            }

            Ok(outcome.summary)
        })();

        self.finish_scan();
        result
    }

    /// Record, for every shared folder that was found, the files it was
    /// reviewed as. A move later acts on that set and nothing else.
    ///
    /// A failure here costs a finding its ability to be moved file by file — it
    /// will ask to be reviewed again — and never fails the scan.
    fn record_reviewed_sets(&self, candidates: &[CleanupCandidate], ctx: &ScanContext) {
        let rules = self.inner.platform.cache_rules();
        for candidate in candidates.iter().filter(|c| c.is_shared_contents()) {
            if ctx.cancelled() {
                return;
            }
            let settle_secs = rules
                .iter()
                .find(|rule| {
                    crate::safety::paths::normalize(&rule.path)
                        == crate::safety::paths::normalize(&candidate.path)
                })
                .map(|rule| rule.settle_secs)
                .unwrap_or(0);
            let cancelled = || ctx.cancelled();
            let policy = scanning::snapshot::SnapshotPolicy {
                protected: &ctx.protected,
                settle_secs,
                now_unix: super::now_unix(),
                cancelled: &cancelled,
                limit: scanning::snapshot::MAX_ENTRIES,
            };
            if let Err(err) =
                scanning::snapshot::record(self.store(), &candidate.id, &candidate.path, &policy)
            {
                tracing::warn!(error = %err, "could not record the files of a shared folder");
            }
        }
    }

    /// Move a candidate into the drawer on Scuttle's own judgement, through
    /// the safety gate. Bound by the verdicts Scuttle computed, because in
    /// this path nobody looked at the item.
    pub fn hold(&self, candidate: &CleanupCandidate) -> Result<QuarantineRecord> {
        self.hold_with(candidate, Bidding::Scuttle)
    }

    /// Move a candidate the user picked out by hand. Same gate, same protected
    /// table, same containment and staleness checks — the only thing that
    /// changes is that Scuttle's own "not suggested" verdict stops being
    /// binding, because someone looked and decided. See [`Bidding`].
    pub fn hold_for_user(&self, candidate: &CleanupCandidate) -> Result<QuarantineRecord> {
        self.hold_with(candidate, Bidding::User)
    }

    fn hold_with(
        &self,
        candidate: &CleanupCandidate,
        bidding: Bidding,
    ) -> Result<QuarantineRecord> {
        let scope = self.action_scope();
        let ctx = scope.ctx(bidding);
        let quarantine = self.quarantine()?;

        if candidate.is_shared_contents() {
            // A shared folder is cleaned file by file from its reviewed set;
            // the folder itself is never what moves. The caller of this
            // blocking helper wants one record or one refusal, so a run that
            // moved nothing is reported as the first reason it did not.
            let outcome = quarantine.hold_contents(
                candidate,
                &ctx,
                super::now_unix(),
                false,
                &crate::quarantine::transfer::Ctl::none(),
                &crate::quarantine::contents::Unobserved,
            )?;
            return match outcome.record {
                Some(record) => Ok(record),
                None => Err(match outcome.issues.groups().first() {
                    Some(group) => crate::quarantine::transfer::FsFailure {
                        kind: group.kind,
                        phase: group.phase,
                        os_code: group.os_code,
                        item: group.samples.first().cloned().unwrap_or_default(),
                    }
                    .into(),
                    None => ScuttleError::Stale("There was nothing left in it to move.".into()),
                }),
            };
        }
        quarantine.hold(candidate, &ctx, super::now_unix())
    }

    /// The safety gate's inputs for the current scan roots.
    pub fn action_scope(&self) -> ActionScope {
        ActionScope {
            protected: self.protected_paths(),
            roots: self.lock_roots().clone(),
        }
    }

    /// Take a fresh reviewed set of the files each finding covers, and
    /// re-measure it, so it can be looked at and chosen again. Never moves
    /// anything, and never starts a move.
    ///
    /// For a shared folder that is a new set of its eligible files; for
    /// anything else it is a fresh fingerprint of what is there now. Either way
    /// the finding is returned as it now stands, and it is the person's call
    /// what to do with it.
    pub fn refresh_findings(&self, ids: &[String]) -> Result<Vec<CleanupCandidate>> {
        let scope = self.action_scope();
        let rules = self.inner.platform.cache_rules();
        let mut refreshed = Vec::new();
        for id in ids {
            let candidate = match self.store().candidate(id) {
                Ok(candidate) => candidate,
                Err(_) => continue,
            };
            let ctx = scope.ctx(Bidding::User);
            if candidate.is_shared_contents() {
                // Structural checks only: the reviewed set replaces the
                // freshness comparison.
                if crate::safety::authorize_contents(&candidate, &ctx).is_err() {
                    continue;
                }
                let settle_secs = rules
                    .iter()
                    .find(|rule| {
                        crate::safety::paths::normalize(&rule.path)
                            == crate::safety::paths::normalize(&candidate.path)
                    })
                    .map(|rule| rule.settle_secs)
                    .unwrap_or(0);
                let never = || false;
                let policy = scanning::snapshot::SnapshotPolicy {
                    protected: &scope.protected,
                    settle_secs,
                    now_unix: super::now_unix(),
                    cancelled: &never,
                    limit: scanning::snapshot::MAX_ENTRIES,
                };
                scanning::snapshot::record(self.store(), &candidate.id, &candidate.path, &policy)?;
            } else {
                if crate::safety::authorize_path(&candidate.path, candidate.target_kind, &ctx)
                    .is_err()
                {
                    continue;
                }
                let fingerprint = crate::safety::observe(&candidate.path)?;
                self.store()
                    .update_fingerprint(&candidate.id, &fingerprint)?;
            }
            if let Ok(updated) = self.store().candidate(id) {
                refreshed.push(updated);
            }
        }
        Ok(refreshed)
    }

    /// The id of the most recent completed scan, if there is one.
    pub fn latest_scan_id(&self) -> Option<String> {
        self.store()
            .latest_scan()
            .ok()
            .flatten()
            .map(|scan| scan.id)
    }

    /// Quarantine every member of a group finding except the one to keep.
    /// The Tauri command is a thin wrapper over this.
    pub fn quarantine_group(
        &self,
        id: &str,
        keep: super::KeepChoice,
    ) -> Result<super::GroupOutcome> {
        super::run_group_action(self, id, keep)
    }

    /// Quarantine every confident finding in one pile. The Tauri command is a
    /// thin wrapper over this.
    pub fn quarantine_confident(
        &self,
        category: crate::model::Category,
    ) -> Result<super::BulkOutcome> {
        super::run_bulk_quarantine(self, Some(category))
    }

    /// Quarantine every confident finding on the floor, across all piles.
    pub fn quarantine_all_confident(&self) -> Result<super::BulkOutcome> {
        super::run_bulk_quarantine(self, None)
    }

    /// Quarantine a hand-picked selection of findings.
    pub fn quarantine_many(&self, ids: &[String]) -> Result<super::BulkOutcome> {
        super::run_quarantine_many(self, ids)
    }

    /// Empty the drawer. The Tauri command is a thin wrapper over this.
    pub fn empty_drawer(&self) -> Result<crate::quarantine::PurgeOutcome> {
        self.quarantine()?.purge_all(crate::commands::now_unix())
    }

    pub fn space_overview(&self) -> Result<SpaceOverview> {
        let protected = self.protected_paths();
        crate::space::overview(
            self.inner.platform.as_ref(),
            &protected,
            self.store(),
            &|| false,
        )
    }

    /// Classify everything, change nothing.
    pub fn dry_run(&self, request: super::RummageRequest) -> Result<super::DryRunReport> {
        let settings = self.store().settings()?;
        let roots = request.roots.filter(|r| !r.is_empty()).unwrap_or_else(|| {
            if settings.scan_roots.is_empty() {
                self.inner
                    .platform
                    .default_scan_roots()
                    .into_iter()
                    .map(|k| k.path)
                    .collect()
            } else {
                settings.scan_roots.clone()
            }
        });

        let options = ScanOptions {
            roots,
            include_developer_debris: request
                .include_developer_debris
                .unwrap_or(settings.include_developer_debris),
            heavy_threshold: settings.heavy_threshold,
            ..Default::default()
        };

        let ctx = ScanContext::new(
            options.clone(),
            Arc::clone(&self.inner.platform),
            self.store().ignore_set()?,
            Arc::new(AtomicBool::new(false)),
        );
        let detectors = crate::detectors::default_set(&options);
        let outcome = scanning::run("dry-run", &ctx, detectors, &CleanupObserver);
        Ok(super::dry_run::build(&options, &outcome, &ctx))
    }

    pub fn protected_paths(&self) -> ProtectedPaths {
        let mut protected = ProtectedPaths::for_current_user();
        crate::platform::protect_own_files(&mut protected, self.inner.platform.as_ref());
        protected
    }

    fn lock_running(&self) -> std::sync::MutexGuard<'_, Option<RunningScan>> {
        self.inner.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_roots(&self) -> std::sync::MutexGuard<'_, Vec<PathBuf>> {
        self.inner
            .allowed_roots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
}

/// Forwards scan progress to the webview.
struct EventObserver {
    app: tauri::AppHandle,
    scan_id: String,
}

impl ScanObserver for EventObserver {
    fn phase(&self, phase: Phase) {
        super::emit_phase(&self.app, &self.scan_id, phase);
    }
    fn progress(&self, progress: &Progress) {
        super::emit_progress(&self.app, &self.scan_id, progress);
    }
    fn candidate(&self, candidate: &CleanupCandidate) {
        super::emit_found(&self.app, &self.scan_id, candidate);
    }
    fn finished(&self, _summary: &ScanSummary) {
        // `emit_done` is sent by the caller, after the results are saved, so
        // the frontend never asks for findings that are not written yet.
    }
}
