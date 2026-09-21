//! Application state: the one place the store, the platform and the running
//! scan are held together.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::model::CleanupCandidate;
use crate::platform::PlatformService;
use crate::quarantine::Quarantine;
use crate::safety::assess::CautionKind;
use crate::safety::{ActionContext, Bidding, ProtectedPaths};
use crate::scanning::{
    self, CleanupObserver, Phase, Progress, ScanContext, ScanObserver, ScanOptions, ScanSummary,
};
use crate::space::SpaceOverview;
use crate::storage::{QuarantineRecord, ScanKind, Store};
use crate::{Result, ScuttleError};

/// What the safety gate needs, gathered once. Building the protected-path table
/// costs something, and a run over many findings should not rebuild it for each.
pub struct ActionScope {
    protected: ProtectedPaths,
    roots: Vec<PathBuf>,
    installs: crate::platform::InstallAreas,
}

impl ActionScope {
    pub fn ctx<'a>(
        &'a self,
        bidding: Bidding,
        acknowledged: &'a [CautionKind],
    ) -> ActionContext<'a> {
        ActionContext {
            protected: &self.protected,
            allowed_roots: &self.roots,
            bidding,
            acknowledged,
            installs: &self.installs,
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
    /// Whether a window is on screen. Kept here rather than asked of Tauri on
    /// demand so the background scheduler — which has no window handle and
    /// must not touch the main thread — can read it.
    window_visible: AtomicBool,
    /// Whether the tray icon was actually created. Hiding the window on close
    /// is conditional on this: a failed tray must never leave someone with a
    /// running application and no way back into it.
    tray_alive: AtomicBool,
    /// Set once the one-per-launch startup work has run, so that showing a
    /// hidden window is never mistaken for a fresh start.
    startup_done: AtomicBool,
    /// True from the moment an explicit quit begins, so the close handler
    /// stops hiding the window and lets it go.
    quitting: AtomicBool,
    /// True while an update install holds the gate. Read by the exit handler,
    /// which must not let a quit tear the application apart halfway through
    /// replacing it.
    installing: AtomicBool,
    /// The background scheduler, while one is running. Only the control block
    /// is held here — the thread keeps its own handle on the state, and
    /// storing a second one the other way would be a cycle.
    scheduler: Mutex<Option<crate::background::Scheduler>>,
    /// When this process started. Background checks stay quiet for a while
    /// after it, so that launching is never itself a reason to scan.
    launched_unix: i64,
}

struct RunningScan {
    id: String,
    kind: ScanKind,
    cancel: Arc<AtomicBool>,
    /// Held for as long as the scan runs, so nothing that could invalidate its
    /// results starts underneath it.
    _guard: OperationGuard,
}

/// Who wants the gate.
///
/// The rule is one sentence: a background check yields to a person. Putting it
/// here rather than at each of the eight call sites means there is one place
/// to read, and no command can forget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Someone clicked something.
    User,
    /// The scheduler.
    Background,
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
    /// An update is being installed. Held from the moment installing is
    /// committed until the process is replaced or the install fails, so that
    /// nothing which changes files can start underneath it.
    Update,
}

impl Operation {
    pub(crate) fn doing(&self) -> &'static str {
        match self {
            Operation::Scan => "looking around",
            Operation::Move => "moving files into the Drawer",
            Operation::Restore => "putting something back",
            Operation::EmptyDrawer => "emptying the Drawer",
            Operation::RemoveItem => "removing something from the Drawer",
            Operation::Refresh => "reviewing findings again",
            Operation::Sweep => "tidying the Drawer",
            Operation::Decide => "updating the findings",
            Operation::Update => "getting ready to update",
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

/// The right to replace the running application, held from the moment an
/// install is committed. A failed install calls [`InstallLease::abandon`],
/// which gives the gate back and restarts what was stopped; a process that is
/// about to be replaced never does, and takes the lease with it.
pub struct InstallLease {
    state: AppState,
    guard: Option<OperationGuard>,
}

impl Drop for InstallLease {
    fn drop(&mut self) {
        // A lease that is dropped for any reason — an install that failed, a
        // worker that unwound — must not leave the application believing it is
        // in the middle of one. Only if it still holds the gate, though: once
        // `release` has given the gate up, somebody else may already hold it
        // and have set the flag for themselves.
        if self.guard.is_some() {
            self.state.inner.installing.store(false, Ordering::SeqCst);
        }
    }
}

impl InstallLease {
    /// The install did not happen and the process is staying. Give the gate
    /// back and restart the background work that was stopped for it.
    pub fn abandon(mut self, app: &tauri::AppHandle) {
        self.release();
        self.state.sync_scheduler(app);
    }

    /// As [`InstallLease::abandon`], for callers with no window to restore.
    pub fn release(&mut self) {
        self.state.inner.installing.store(false, Ordering::SeqCst);
        drop(self.guard.take());
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
                window_visible: AtomicBool::new(false),
                tray_alive: AtomicBool::new(false),
                startup_done: AtomicBool::new(false),
                quitting: AtomicBool::new(false),
                installing: AtomicBool::new(false),
                scheduler: Mutex::new(None),
                launched_unix: super::now_unix(),
            }),
        })
    }

    pub fn launched_unix(&self) -> i64 {
        self.inner.launched_unix
    }

    /// Start the background scheduler if it is wanted and not already
    /// running, and stop it if it is not.
    ///
    /// Called at startup and whenever the settings change, so the thread's
    /// existence always matches the setting rather than the setting being a
    /// note about what happened at launch.
    pub fn sync_scheduler(&self, app: &tauri::AppHandle) {
        let wanted = self
            .store()
            .settings()
            .map(|s| s.background_mode)
            .unwrap_or(false);

        let mut slot = self
            .inner
            .scheduler
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        match (wanted, slot.as_ref()) {
            (true, None) => {
                *slot = Some(crate::background::Scheduler::start(
                    self.clone(),
                    app.clone(),
                    self.inner.launched_unix,
                ));
            }
            (false, Some(running)) => {
                running.stop();
                *slot = None;
            }
            // Already in the right state; a settings change that did not touch
            // background mode should not restart the thread.
            _ => {}
        }
    }

    /// Reconsider now rather than at the next tick.
    pub fn nudge_scheduler(&self) {
        if let Some(scheduler) = self
            .inner
            .scheduler
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            scheduler.nudge();
        }
    }

    /// Stop the scheduler for good. Quitting Scuttle stops its background
    /// work; there is no helper, service or daemon left behind.
    pub fn stop_scheduler(&self) {
        if let Some(scheduler) = self
            .inner
            .scheduler
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            scheduler.stop();
        }
    }

    // ---- what the background scheduler needs to know -------------------

    pub fn window_visible(&self) -> bool {
        self.inner.window_visible.load(Ordering::Relaxed)
    }

    pub fn set_window_visible(&self, visible: bool) {
        self.inner.window_visible.store(visible, Ordering::Relaxed);
    }

    pub fn tray_alive(&self) -> bool {
        self.inner.tray_alive.load(Ordering::Relaxed)
    }

    pub fn set_tray_alive(&self, alive: bool) {
        self.inner.tray_alive.store(alive, Ordering::Relaxed);
    }

    pub fn quitting(&self) -> bool {
        self.inner.quitting.load(Ordering::Relaxed)
    }

    pub fn begin_quitting(&self) {
        self.inner.quitting.store(true, Ordering::Relaxed);
    }

    /// True the first time it is called and false afterwards.
    ///
    /// Startup work — reconciling interrupted moves, expiring the drawer,
    /// pruning old scans — belongs to the process, not to the window. With a
    /// tray, a window can be shown and hidden many times in one run, and none
    /// of those is a launch.
    pub fn claim_startup(&self) -> bool {
        !self.inner.startup_done.swap(true, Ordering::Relaxed)
    }

    /// The roots a scan would actually use: what the user chose, or what the
    /// platform suggests when they have chosen nothing. Exactly the resolution
    /// [`super::rummage`] performs — a background check must not look anywhere
    /// a rummage would not.
    pub fn resolved_roots(&self) -> Vec<PathBuf> {
        let chosen = self
            .store()
            .settings()
            .map(|s| s.scan_roots)
            .unwrap_or_default();
        if !chosen.is_empty() {
            return chosen;
        }
        self.inner
            .platform
            .default_scan_roots()
            .into_iter()
            .map(|known| known.path)
            .collect()
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
        )
        .with_protected(self.protected_paths()))
    }

    /// Register a new scan, refusing if one is already running.
    pub fn start_scan(&self, options: &ScanOptions) -> Result<String> {
        self.start_scan_as(options, ScanKind::Full, Priority::User)
    }

    pub fn start_scan_as(
        &self,
        options: &ScanOptions,
        kind: ScanKind,
        priority: Priority,
    ) -> Result<String> {
        let mut running = self.lock_running();
        if running.is_some() && !self.displace_background_scan(&mut running, priority) {
            return Err(ScuttleError::ScanBusy);
        }
        // A scan replaces the findings a selection was made from, so it cannot
        // start while something is moving them. Any yielding a user's scan
        // needed has already happened above, before this lock was taken.
        let guard = self.claim_gate(Operation::Scan)?;
        let id = uuid::Uuid::new_v4().to_string();
        *running = Some(RunningScan {
            id: id.clone(),
            kind,
            cancel: Arc::new(AtomicBool::new(false)),
            _guard: guard,
        });
        drop(running);

        *self.lock_roots() = options.roots.clone();
        self.store()
            .begin_scan(&id, kind, options, super::now_unix())?;
        Ok(id)
    }

    /// Whether the scan currently running, if any, is a background check.
    pub fn background_scan_running(&self) -> bool {
        self.lock_running()
            .as_ref()
            .is_some_and(|scan| scan.kind == ScanKind::Glance)
    }

    /// Turf out a running background check on a user's behalf.
    ///
    /// Returns whether the slot is now free. Only a check is ever displaced,
    /// and only for a person: the courtesy is deliberately one-directional.
    ///
    /// It takes the slot rather than waiting for the check's worker to let go.
    /// Waiting was the first attempt and it was wrong — the worker releases the
    /// slot on its own schedule, so "a rummage is never refused because of a
    /// background check" held only if the worker happened to be quick, which is
    /// not a guarantee at all. The displaced worker discovers that the
    /// registered scan is no longer its own and stands down without saving
    /// anything; dropping its entry here releases the gate it was holding.
    fn displace_background_scan(
        &self,
        running: &mut Option<RunningScan>,
        priority: Priority,
    ) -> bool {
        let displaceable = priority == Priority::User
            && running
                .as_ref()
                .is_some_and(|scan| scan.kind == ScanKind::Glance);
        if !displaceable {
            return false;
        }
        if let Some(scan) = running.as_ref() {
            scan.cancel.store(true, Ordering::Relaxed);
        }
        // Dropping it releases the operation gate the check held with it.
        drop(running.take());
        true
    }

    /// Whether this scan is still the one the application is running.
    fn still_registered(&self, scan_id: &str) -> bool {
        self.lock_running()
            .as_ref()
            .is_some_and(|scan| scan.id == scan_id)
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

    /// Clear the running slot, but only if it still holds the scan that is
    /// finishing.
    ///
    /// The identity check matters now that a user action can displace a
    /// background check: the check is cancelled, the rummage takes the slot,
    /// and only then does the check's worker unwind and come here. Taking the
    /// slot unconditionally at that point would clear the rummage's entry —
    /// releasing its gate underneath it and leaving `cancel_rummage` with
    /// nothing to stop.
    fn finish_scan(&self, scan_id: &str) {
        let mut running = self.lock_running();
        if running.as_ref().is_some_and(|scan| scan.id == scan_id) {
            // Dropping the scan releases the operation gate with it.
            let finished = running.take();
            drop(running);
            drop(finished);
        }
    }

    /// Claim the right to change files or the Drawer, or be told what is in
    /// the way. The claim ends when the returned guard is dropped.
    pub fn begin_operation(&self, operation: Operation) -> Result<OperationGuard> {
        self.begin_operation_as(operation, Priority::User)
    }

    /// As [`AppState::begin_operation`], but saying who is asking.
    ///
    /// A user action finding the gate held by a background check asks the
    /// check to stop and waits briefly for it. Everything else behaves exactly
    /// as it did before: the gate is still one-at-a-time, and a busy Scuttle
    /// still refuses rather than queues.
    pub fn begin_operation_as(
        &self,
        operation: Operation,
        priority: Priority,
    ) -> Result<OperationGuard> {
        if priority == Priority::User {
            let mut running = self.lock_running();
            if running.is_some() {
                self.displace_background_scan(&mut running, priority);
            }
        }
        self.claim_gate(operation)
    }

    /// Take the gate, with no opinion about who is asking.
    ///
    /// Split from [`AppState::begin_operation_as`] because the yielding half
    /// inspects the running scan, and [`AppState::start_scan_as`] calls this
    /// while already holding that lock. Going through the yielding path from
    /// there would deadlock on a lock the same thread owns.
    fn claim_gate(&self, operation: Operation) -> Result<OperationGuard> {
        self.try_claim(operation).map_err(|holder| {
            ScuttleError::Busy(format!(
                "Scuttle is busy {}. Try again when that finishes.",
                holder.doing()
            ))
        })
    }

    /// The claim itself, saying *who* is in the way rather than how to phrase
    /// it. The one place the gate is taken, so every caller — a move, a
    /// restore, an install — is serialised by the same mutex and none can slip
    /// between another's check and its claim.
    fn try_claim(&self, operation: Operation) -> std::result::Result<OperationGuard, Operation> {
        let mut gate = self.lock_gate();
        if let Some(hold) = gate.as_ref() {
            return Err(hold.operation);
        }
        let id = self.next_gate_id();
        *gate = Some(GateHold { id, operation });
        Ok(OperationGuard {
            state: self.clone(),
            id,
        })
    }

    /// Commit to installing an update, or be told what is in the way.
    ///
    /// Claiming the operation gate *is* the commitment. There is no separate
    /// "is Scuttle idle?" question to ask first, because a question answered
    /// before the claim can be out of date by the time the claim is made: the
    /// answer and the claim are one step under one lock. Once this returns
    /// `Ok`, every command that would change a file is refused as busy, and it
    /// stays that way until the lease is dropped.
    ///
    /// Only a background check is ever asked to stand down for it, exactly as
    /// for any other thing a person asks for. Anything else — a move, a
    /// restore, a rummage the person is waiting on — is left to finish, and the
    /// install is the one refused.
    ///
    /// On success the background scheduler is stopped and the database is
    /// checkpointed, so what is on disk when the process is replaced is
    /// complete. If the install then fails, [`InstallLease::abandon`] puts all
    /// of it back.
    pub fn begin_install(&self) -> std::result::Result<InstallLease, Operation> {
        {
            let mut running = self.lock_running();
            if running.is_some() {
                self.displace_background_scan(&mut running, Priority::User);
            }
        }
        let guard = self.try_claim(Operation::Update)?;
        self.inner.installing.store(true, Ordering::SeqCst);
        self.stop_scheduler();
        // Everything committed is already in the database; this folds the
        // write-ahead log into it so the file alone is whole. A failure here
        // is not a reason to refuse: the log is just as durable.
        if let Err(error) = self.store().checkpoint() {
            tracing::warn!("could not checkpoint before installing: {error}");
        }
        Ok(InstallLease {
            state: self.clone(),
            guard: Some(guard),
        })
    }

    /// Whether an install has been committed and not yet abandoned.
    pub fn installing(&self) -> bool {
        self.inner.installing.load(Ordering::SeqCst)
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
        // The tray says when Scuttle last looked and what turned up, so it is
        // rewritten whenever that changes rather than on a timer.
        crate::tray::refresh(app);
        super::emit_background(app, self);
        Ok(summary)
    }

    /// One background check: metadata only, bounded in time, yielding to
    /// anything the user does.
    ///
    /// Everything that makes a rummage trustworthy applies unchanged — the
    /// same operation gate, the same ignore lists, the same safety guard on
    /// every candidate, the same roots. The differences are that it reads no
    /// file contents (see [`crate::detectors::glance_set`]) and that it gives
    /// up when asked. It moves nothing, deletes nothing and selects nothing.
    pub fn run_glance(&self, budget_secs: u64) -> Result<crate::scanning::ScanOutcome> {
        let settings = self.store().settings()?;
        let roots = self.resolved_roots();
        if roots.is_empty() {
            return Err(ScuttleError::Refused("There is nowhere to look.".into()));
        }

        let options = ScanOptions {
            roots,
            include_developer_debris: settings.include_developer_debris,
            heavy_threshold: settings.heavy_threshold,
            ..Default::default()
        };

        let scan_id = self.start_scan_as(&options, ScanKind::Glance, Priority::Background)?;

        // A watchdog rather than a timeout: it sets the same flag the Stop
        // button does, so the check unwinds through the ordinary cancellation
        // path and its partial results are saved and labelled as partial.
        let watchdog = self.spawn_budget(&scan_id, budget_secs);

        let outcome = self.run_scan_collecting(&scan_id, options);
        watchdog.store(true, Ordering::Relaxed);
        outcome
    }

    /// Stop the named scan once `budget_secs` have passed. Returns a flag the
    /// caller sets to stand the watchdog down.
    fn spawn_budget(&self, scan_id: &str, budget_secs: u64) -> Arc<AtomicBool> {
        let done = Arc::new(AtomicBool::new(false));
        let watch = Arc::clone(&done);
        let state = self.clone();
        let id = scan_id.to_string();
        // Sleeping in short steps so a finished scan does not keep a thread
        // parked for the whole budget.
        let _ = std::thread::Builder::new()
            .name("scuttle-glance-budget".into())
            .spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(budget_secs);
                while Instant::now() < deadline {
                    if watch.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                if watch.load(Ordering::Relaxed) {
                    return;
                }
                if state
                    .lock_running()
                    .as_ref()
                    .is_some_and(|scan| scan.id == id)
                {
                    tracing::debug!("a background check ran long and was asked to stop");
                    state.cancel_scan();
                }
            });
        done
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
        self.scan_to_completion(scan_id, options, observer)
            .map(|outcome| outcome.summary)
    }

    /// As [`AppState::run_scan_with`], but keeping what was found.
    ///
    /// A background check needs the candidates themselves to work out what is
    /// new, and reading them back out of the database afterwards would race
    /// with the next scan.
    fn run_scan_collecting(
        &self,
        scan_id: &str,
        options: ScanOptions,
    ) -> Result<crate::scanning::ScanOutcome> {
        self.scan_to_completion(scan_id, options, &crate::scanning::SilentObserver)
    }

    fn scan_to_completion(
        &self,
        scan_id: &str,
        options: ScanOptions,
        observer: &dyn ScanObserver,
    ) -> Result<crate::scanning::ScanOutcome> {
        let (cancel, kind) = match self.lock_running().as_ref() {
            Some(scan) if scan.id == scan_id => (Arc::clone(&scan.cancel), scan.kind),
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
            let detectors = match kind {
                ScanKind::Full => crate::detectors::default_set(&options),
                ScanKind::Glance => crate::detectors::glance_set(&options),
            };
            let outcome = scanning::run(scan_id, &ctx, detectors, observer);

            // Displaced part-way? Then these findings belong to a scan nobody
            // is waiting for, and saving them would overwrite the ones that
            // took its place.
            if !self.still_registered(scan_id) {
                return Err(ScuttleError::ScanBusy);
            }

            self.store().save_candidates(scan_id, &outcome.candidates)?;
            self.record_reviewed_sets(&outcome.candidates, &ctx);
            self.store()
                .finish_scan(&outcome.summary, super::now_unix())?;

            // Only a rummage someone asked for counts as having rummaged. A
            // background check must not be able to unlock behaviour — its own
            // included — that is meant to wait for a deliberate first run.
            if kind == ScanKind::Full {
                let mut settings = self.store().settings()?;
                if !settings.has_rummaged_before {
                    settings.has_rummaged_before = true;
                    let _ = self.store().save_settings(&settings);
                }
            }

            Ok(outcome)
        })();

        self.finish_scan(scan_id);
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
        self.hold_with(candidate, Bidding::Scuttle, &[])
    }

    /// Move a candidate the user picked out by hand. Same gate, same protected
    /// table, same containment and staleness checks — the only thing that
    /// changes is that Scuttle's own "not suggested" verdict stops being
    /// binding, because someone looked and decided. See [`Bidding`].
    pub fn hold_for_user(
        &self,
        candidate: &CleanupCandidate,
        acknowledged: &[CautionKind],
    ) -> Result<QuarantineRecord> {
        self.hold_with(candidate, Bidding::User, acknowledged)
    }

    fn hold_with(
        &self,
        candidate: &CleanupCandidate,
        bidding: Bidding,
        acknowledged: &[CautionKind],
    ) -> Result<QuarantineRecord> {
        let scope = self.action_scope();
        let ctx = scope.ctx(bidding, acknowledged);
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
            installs: self.inner.platform.install_areas(),
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
            let ctx = scope.ctx(Bidding::User, &[]);
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
        acknowledged: &[CautionKind],
    ) -> Result<super::GroupOutcome> {
        super::run_group_action(self, id, keep, acknowledged)
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
    pub fn quarantine_many(
        &self,
        ids: &[String],
        acknowledged: &[CautionKind],
    ) -> Result<super::BulkOutcome> {
        super::run_quarantine_many(self, ids, acknowledged)
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
