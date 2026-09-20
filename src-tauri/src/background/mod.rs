//! Staying out of the way.
//!
//! One thread, asleep almost all of the time, that wakes every few minutes to
//! ask [`schedule::decide`] whether this is a reasonable moment. Almost always
//! the answer is no, and the whole tick costs a handful of integer
//! comparisons. It waits on a condition variable rather than polling, so
//! turning the setting off, pausing, or quitting takes effect at once instead
//! of at the end of the interval.
//!
//! What this does *not* do is as much the design as what it does. It does not
//! watch the filesystem, keep a catch-up queue, retry a skipped check, wake
//! the machine, prevent it sleeping, or run anything the moment the app
//! starts. A check that does not happen is not a failure.

pub mod conditions;
pub mod notify;
pub mod schedule;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::commands::{AppState, Operation, Priority};
use crate::storage::{BackgroundState, ScanKind};

pub use schedule::{Decision, Moment, Reason};

/// Handle on the scheduler thread.
///
/// Deliberately holds only the control block and not an [`AppState`]: the
/// state holds one of these, and the thread holds a state, so a handle that
/// also held a state would close a reference cycle and keep the whole
/// application alive after it should have gone.
#[derive(Clone)]
pub struct Scheduler {
    shared: Arc<Shared>,
}

struct Shared {
    stop: AtomicBool,
    /// Woken when something the scheduler cares about changes, so a paused or
    /// switched-off scheduler reacts immediately rather than at the next tick.
    wake: Condvar,
    lock: Mutex<()>,
}

impl Shared {
    /// Sleep for `secs`, or until woken. Returns early on stop.
    fn rest(&self, secs: i64) {
        let guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _unused = self
            .wake
            .wait_timeout(guard, Duration::from_secs(secs.max(1) as u64))
            .unwrap_or_else(|e| e.into_inner());
    }

    fn nudge(&self) {
        self.wake.notify_all();
    }
}

impl Scheduler {
    /// Start ticking. The first tick happens one interval from now: launching
    /// Scuttle is never itself a reason to do anything.
    pub fn start(state: AppState, app: tauri::AppHandle, launched_unix: i64) -> Scheduler {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            wake: Condvar::new(),
            lock: Mutex::new(()),
        });

        let thread_shared = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("scuttle-background".into())
            .spawn(move || run(&thread_shared, &state, &app, launched_unix));

        if let Err(err) = spawned {
            // A scheduler that could not start means no ambient checks. The
            // tray, the window and every manual action keep working, so this
            // is worth a line in the log and nothing more dramatic.
            tracing::warn!(error = %err, "background checks could not be scheduled");
            shared.stop.store(true, Ordering::Relaxed);
        }

        Scheduler { shared }
    }

    /// Reconsider now rather than at the next tick.
    pub fn nudge(&self) {
        self.shared.nudge();
    }

    /// Ask the thread to finish. It checks between ticks and on waking, so a
    /// scheduler asleep in the middle of an interval stops at once rather than
    /// at the end of it.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.nudge();
    }

    pub fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
    }
}

fn run(shared: &Arc<Shared>, state: &AppState, app: &tauri::AppHandle, launched_unix: i64) {
    while !shared.stop.load(Ordering::Relaxed) {
        shared.rest(schedule::TICK);
        if shared.stop.load(Ordering::Relaxed) {
            return;
        }
        if let Err(err) = tick(state, app, launched_unix) {
            // A tick that fails is a tick that did nothing. The next one is in
            // five minutes; there is nothing to recover and nobody to tell.
            tracing::debug!(error = %err, "a background tick came to nothing");
        }
    }
}

fn tick(state: &AppState, app: &tauri::AppHandle, launched_unix: i64) -> crate::Result<()> {
    let now = crate::commands::now_unix();
    let settings = state.store().settings()?;
    let mut persisted = state.store().background_state()?;

    let moment = Moment {
        launched_unix,
        window_visible: state.window_visible(),
        busy: state.current_operation().is_some(),
        has_roots: !state.resolved_roots().is_empty(),
    };

    let decision = schedule::decide(now, &settings, &persisted, &moment, conditions::read);

    // Whatever was decided, record that the scheduler was awake — and, if the
    // gap says the machine slept, when it came back. Both are written before
    // any of the work that might take a while, so a crash mid-check cannot
    // leave the next tick thinking no time passed.
    if schedule::slept(now, &persisted) {
        persisted.woke_unix = now;
    }
    persisted.last_tick_unix = now;
    state.store().save_background_state(&persisted)?;

    // The drawer is swept whenever the scheduler is running, including when
    // ambient checks are switched off. Items still expire after exactly the
    // days they were given; an application that stays open simply no longer
    // has to wait for its next launch to honour that. This removes only what
    // the user themselves put in the drawer, under a date they were shown.
    if schedule::should_sweep(&moment) {
        sweep(state);
    }

    if let Decision::Skip(reason) = decision {
        tracing::trace!(reason = ?reason, "not looking around");
        return Ok(());
    }

    glance(state, app, now)
}

/// Expire drawer items whose time is up. Never touches anything flagged for
/// attention — an interrupted move must be understood, not swept.
fn sweep(state: &AppState) {
    let Ok(_guard) = state.begin_operation_as(Operation::Sweep, Priority::Background) else {
        return;
    };
    let Ok(quarantine) = state.quarantine() else {
        return;
    };
    match quarantine.sweep_expired(crate::commands::now_unix()) {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, "expired drawer items removed"),
        Err(err) => tracing::warn!(error = %err, "could not tidy the drawer"),
    }
    let cutoff = crate::commands::now_unix() - schedule::NOTICE_MEMORY;
    let _ = state.store().prune_notices(cutoff);
}

/// What a completed check concluded, once the database has been brought up
/// to date. Returned rather than acted on, so the whole of it can be tested
/// without a window.
#[derive(Debug, Clone, PartialEq)]
pub enum Conclusion {
    /// Cut short — by the budget, by a user action taking priority, or by a
    /// quit. What it found is saved, because it is real, but it does not
    /// count as the day's check and it says nothing. Presenting a partial
    /// look as a whole one is the failure this variant exists to prevent.
    CutShort,
    /// Finished, with nothing worth interrupting anybody about.
    Quiet { found: u64 },
    /// Finished, and there is a summary to send.
    Worth {
        found: u64,
        title: String,
        body: String,
    },
}

/// One background check, start to finish, with no dependency on a window.
///
/// Everything that decides *what happened* is here; the caller's only job is
/// to deliver the result. Split out for the same reason
/// [`AppState::run_scan_with`] is: it makes the whole path — run, save,
/// deduplicate, decide whether to speak — testable against a fixture
/// filesystem with no Tauri handle in sight.
pub fn check(state: &AppState, now: i64, budget_secs: u64) -> crate::Result<Conclusion> {
    let outcome = state.run_glance(budget_secs)?;

    let mut persisted = state.store().background_state()?;
    persisted.last_attempt_unix = now;

    if outcome.summary.cancelled {
        state.store().save_background_state(&persisted)?;
        return Ok(Conclusion::CutShort);
    }

    persisted.last_completed_unix = crate::commands::now_unix();
    let found = outcome.summary.candidates_found;

    let settings = state.store().settings()?;
    let notice = if settings.background_notify {
        worth_saying(state, &outcome, &persisted)?
    } else {
        None
    };

    // Something worth looking at is worth pointing at when the window next
    // opens, whether or not a summary was sent.
    if found > 0 {
        persisted.pending_review = Some(outcome.summary.scan_id.clone());
    }

    let conclusion = match notice {
        Some(notice) => {
            // The keys are burned and the cooldown started here, before the
            // summary is handed over. A notification the system then refuses
            // must not leave Scuttle believing it never mentioned these —
            // otherwise a platform that always refuses would mean a fresh
            // "new findings" summary queued up every single day.
            state
                .store()
                .remember_notified(&notice.keys, crate::commands::now_unix())?;
            persisted.last_notified_unix = crate::commands::now_unix();
            let (title, body) = notify::phrase(&notice);
            Conclusion::Worth { found, title, body }
        }
        None => Conclusion::Quiet { found },
    };

    state.store().save_background_state(&persisted)?;
    Ok(conclusion)
}

/// Whatever is new, ignored and not already in the drawer — if there is
/// enough of it, and enough time has passed since the last summary.
///
/// Findings the user has ignored never reach here: the scan itself filters
/// them. When there is something new but not enough of it to interrupt for,
/// the keys are still remembered by the caller, so a trickle of one new file
/// a day never accumulates into a sudden announcement later.
fn worth_saying(
    state: &AppState,
    outcome: &crate::scanning::ScanOutcome,
    persisted: &BackgroundState,
) -> crate::Result<Option<notify::Notice>> {
    let keys: Vec<String> = outcome.candidates.iter().map(notify::notice_key).collect();
    let unseen = state.store().unseen_notices(&keys)?;
    if unseen.is_empty() {
        return Ok(None);
    }

    let held = state.store().quarantined_paths().unwrap_or_default();
    let fresh: Vec<&crate::model::CleanupCandidate> = outcome
        .candidates
        .iter()
        .filter(|c| unseen.contains(&notify::notice_key(c)))
        // Something already sitting in the drawer has been decided about.
        .filter(|c| !held.iter().any(|p| p == &c.path))
        .collect();

    let notice = notify::summarise(&fresh);
    let now = crate::commands::now_unix();
    if notify::worth_interrupting(&notice, persisted, now) {
        return Ok(Some(notice));
    }

    // Not worth saying — but these have now been seen, so remember them
    // anyway and stay quiet.
    state.store().remember_notified(&notice.keys, now)?;
    Ok(None)
}

/// Run a check and deliver whatever it concluded.
fn glance(state: &AppState, app: &tauri::AppHandle, now: i64) -> crate::Result<()> {
    let conclusion = check(state, now, schedule::GLANCE_BUDGET_SECS)?;

    match &conclusion {
        Conclusion::CutShort => {
            tracing::debug!("a background check was cut short; saying nothing")
        }
        Conclusion::Quiet { found } => tracing::info!(found, "a background check finished"),
        Conclusion::Worth { found, title, body } => {
            tracing::info!(
                found,
                "a background check finished, and had something to say"
            );
            if let Err(err) = send(app, title, body) {
                // Notifications being refused or unavailable must not cost
                // the user the rest of the feature.
                tracing::debug!(error = %err, "could not send a summary");
            }
        }
    }

    crate::commands::emit_background(app, state);
    crate::tray::refresh(app);
    Ok(())
}

fn send(app: &tauri::AppHandle, title: &str, body: &str) -> crate::Result<()> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| crate::ScuttleError::Internal(format!("notification refused: {e}")))
}

/// The scan kind a background check runs as. Named here so the one place that
/// starts one and the one place that labels one cannot drift apart.
pub const GLANCE_KIND: ScanKind = ScanKind::Glance;
