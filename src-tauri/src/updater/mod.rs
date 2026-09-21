//! Updating Scuttle from inside Scuttle.
//!
//! Three things are kept apart on purpose:
//!
//! * **What the updater plugin does** — fetch a manifest, download a package,
//!   verify its signature, replace the application — sits behind [`Backend`].
//!   Nothing else in Scuttle touches the plugin, and the webview is given no
//!   permission to.
//! * **Whether it is safe to replace the application right now** sits behind
//!   [`Host`], and is answered by the same operation gate that serialises every
//!   move, restore and delete (see [`crate::commands::AppState::begin_install`]).
//!   The updater does not keep an "idle" flag of its own to get out of date.
//! * **What state an update is in, and which transitions are allowed** is
//!   [`Updater`], a small state machine that is the only thing the interface
//!   talks to. It is Tauri-free, so all of it is tested here against a mock
//!   backend and a real operation gate.
//!
//! The states are: idle, checking, up to date, available, downloading, ready
//! to install, installing, and unavailable. A failure is not a state of its
//! own: it is a note ([`Failure`]) beside the state the updater fell back to,
//! so a failed download leaves the update still available and one click from
//! being tried again, and a failed check leaves whatever was known before.
//!
//! What this deliberately does not do: keep a downloaded package across a
//! restart. The verified bytes live in memory only. After a restart the update
//! is found again and downloaded again, which costs a few megabytes and means
//! "ready to install" is never shown for a file nothing re-verified.

pub mod channel;
mod tauri_backend;

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use semver::Version;
use serde::Serialize;

use crate::commands::{AppState, InstallLease, Operation};
use crate::{Result, ScuttleError};

pub use channel::{channel_for, is_offered, Channel};
pub use tauri_backend::{start, TauriBackend, TauriPublisher};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a release is, as far as the interface needs to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    /// The release notes, as written in the changelog.
    pub notes: Option<String>,
    pub date_unix: Option<i64>,
}

/// What the backend found.
#[derive(Debug, Clone)]
pub struct Remote {
    pub version: Version,
    pub notes: Option<String>,
    pub date_unix: Option<i64>,
}

impl Remote {
    fn info(&self) -> UpdateInfo {
        UpdateInfo {
            version: self.version.to_string(),
            notes: self.notes.clone().filter(|notes| !notes.trim().is_empty()),
            date_unix: self.date_unix,
        }
    }
}

/// Everything that can go wrong at the boundary, sorted by what a person can
/// do about it rather than by which library complained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// Could not reach the server, or the connection dropped.
    Offline,
    /// The server did not answer in time.
    Timeout,
    /// An answer that is not a manifest Scuttle understands.
    BadManifest,
    /// A manifest with nothing for this operating system and architecture.
    NoPlatform,
    /// The downloaded package did not verify against the public key.
    BadSignature,
    /// The download stopped before it finished.
    Interrupted,
    /// The application could not be replaced.
    InstallFailed(String),
    /// Updates cannot work for this installation at all: not an installed
    /// copy, no public key configured, no endpoint.
    Unavailable(String),
    /// Anything else, with the underlying words.
    Other(String),
}

impl UpdateError {
    /// What to tell the person. Each says what happened *and* what did not:
    /// nothing was installed, or Scuttle is still running.
    pub fn message(&self) -> String {
        match self {
            UpdateError::Offline => {
                "Scuttle could not reach the update server. Check the connection and try again."
                    .into()
            }
            UpdateError::Timeout => "The update server took too long to answer.".into(),
            UpdateError::BadManifest => {
                "The update information was not something Scuttle could read, so it did nothing."
                    .into()
            }
            UpdateError::NoPlatform => {
                "No update has been published for this kind of computer.".into()
            }
            UpdateError::BadSignature => {
                "The download did not pass Scuttle's signature check, so it was thrown away. \
                 Nothing was installed."
                    .into()
            }
            UpdateError::Interrupted => {
                "The download stopped partway. Nothing was installed, and it can be tried again."
                    .into()
            }
            UpdateError::InstallFailed(why) => {
                format!("The update could not be installed, and Scuttle is still running. {why}")
                    .trim()
                    .to_string()
            }
            UpdateError::Unavailable(why) | UpdateError::Other(why) => why.clone(),
        }
    }
}

/// The updater plugin, as far as the state machine cares.
///
/// Stateful on the far side of the boundary: the real one remembers the update
/// `check` found and the bytes `download` verified, so the state machine never
/// handles a package and cannot hand an unverified one to `install`.
pub trait Backend: Send + Sync {
    /// Look for something newer than what is running. `Ok(None)` is "nothing
    /// is offered", which is not an error.
    fn check(&self) -> BoxFuture<'_, std::result::Result<Option<Remote>, UpdateError>>;

    /// Download what `check` found and verify its signature. `progress` is
    /// told how many bytes arrived and, when the server said, how many to
    /// expect. Succeeding means the package is verified; nothing else does.
    fn download<'a>(
        &'a self,
        progress: &'a (dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> BoxFuture<'a, std::result::Result<(), UpdateError>>;

    /// Replace the application with the verified package. On Windows a
    /// successful install ends the process, so nothing after this may matter;
    /// on macOS it relaunches. Only ever called with the gate held.
    fn install(&self) -> BoxFuture<'_, std::result::Result<(), UpdateError>>;

    /// Forget a downloaded package.
    fn discard(&self);
}

/// The operation gate, from the updater's side.
pub trait Host: Send + Sync {
    /// Commit to installing, or say why not, in words for a person.
    fn begin_install(&self) -> std::result::Result<Box<dyn Lease>, String>;
}

/// The committed right to install. Dropped along with the process on a
/// successful install; handed back through [`Lease::abandon`] on a failed one.
pub trait Lease: Send {
    fn abandon(self: Box<Self>);
}

/// Where snapshots go: the `scuttle://update` event in the application, a
/// vector in tests.
pub trait Publish: Send + Sync {
    fn publish(&self, snapshot: &Snapshot);
}

/// Words for "the install is waiting on something".
pub fn blocked_message(holder: Operation) -> String {
    format!(
        "Scuttle can't restart while it's {}. Try again once that finishes.",
        holder.doing()
    )
}

/// Adapts [`AppState`]'s install commitment to the updater's [`Host`].
pub struct StateHost {
    pub state: AppState,
    /// `None` in tests, which have no window to hand the scheduler back to.
    pub app: Option<tauri::AppHandle>,
}

struct StateLease {
    inner: InstallLease,
    app: Option<tauri::AppHandle>,
}

impl Lease for StateLease {
    fn abandon(self: Box<Self>) {
        let StateLease { mut inner, app } = *self;
        match app {
            Some(app) => inner.abandon(&app),
            None => inner.release(),
        }
    }
}

impl Host for StateHost {
    fn begin_install(&self) -> std::result::Result<Box<dyn Lease>, String> {
        self.state
            .begin_install()
            .map(|inner| {
                Box::new(StateLease {
                    inner,
                    app: self.app.clone(),
                }) as Box<dyn Lease>
            })
            .map_err(blocked_message)
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    /// Nothing has been looked for yet.
    Idle,
    Checking,
    UpToDate {
        checked_unix: i64,
    },
    Available {
        info: UpdateInfo,
    },
    Downloading {
        info: UpdateInfo,
        received: u64,
        /// `None` when the server did not say how big the download is. The
        /// interface shows movement without a percentage rather than a bar
        /// that lies.
        total: Option<u64>,
    },
    /// Downloaded and verified. Held in memory only.
    Ready {
        info: UpdateInfo,
    },
    Installing {
        info: UpdateInfo,
    },
    /// Updates do not work here at all.
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Check,
    Download,
    Install,
}

/// The last thing that went wrong, kept beside the state rather than as one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub stage: Stage,
    pub message: String,
    /// Whether a person asked for the check that failed. A background check
    /// that fails is not worth an interruption; the interface only speaks up
    /// for a manual one. Downloads and installs are always manual.
    pub manual: bool,
}

/// Everything the interface needs, in one piece. Sent whole on every change
/// and returned whole from every command, so it never has to be pieced
/// together from events that may have been missed while the window was hidden.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot {
    pub current_version: String,
    pub channel: Channel,
    pub state: Phase,
    /// The person dismissed the notice for the version in `state` during this
    /// run. Still shown in Settings; not pushed at them again.
    pub dismissed: bool,
    /// Why installing is not possible right now, if it was just tried.
    pub blocked: Option<String>,
    pub error: Option<Failure>,
}

struct Machine {
    phase: Phase,
    dismissed_version: Option<String>,
    blocked: Option<String>,
    error: Option<Failure>,
    /// Kept for as long as the process lives once an install has begun, so the
    /// gate stays shut. A failed install takes it back out and abandons it.
    lease: Option<Box<dyn Lease>>,
}

struct Shared {
    backend: Arc<dyn Backend>,
    host: Arc<dyn Host>,
    publisher: Arc<dyn Publish>,
    current: Version,
    machine: Mutex<Machine>,
    /// The clock, so a test can say what time it is.
    now: fn() -> i64,
}

/// The updater. Cheap to clone; every clone is the same updater.
#[derive(Clone)]
pub struct Updater {
    shared: Arc<Shared>,
}

impl Updater {
    pub fn new(
        backend: Arc<dyn Backend>,
        host: Arc<dyn Host>,
        publisher: Arc<dyn Publish>,
        current: Version,
    ) -> Updater {
        Updater::with_clock(backend, host, publisher, current, || {
            chrono::Utc::now().timestamp()
        })
    }

    pub fn with_clock(
        backend: Arc<dyn Backend>,
        host: Arc<dyn Host>,
        publisher: Arc<dyn Publish>,
        current: Version,
        now: fn() -> i64,
    ) -> Updater {
        Updater {
            shared: Arc::new(Shared {
                backend,
                host,
                publisher,
                current,
                machine: Mutex::new(Machine {
                    phase: Phase::Idle,
                    dismissed_version: None,
                    blocked: None,
                    error: None,
                    lease: None,
                }),
                now,
            }),
        }
    }

    pub fn current(&self) -> &Version {
        &self.shared.current
    }

    fn machine(&self) -> MutexGuard<'_, Machine> {
        self.shared
            .machine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn snapshot_of(&self, machine: &Machine) -> Snapshot {
        let shown = match &machine.phase {
            Phase::Available { info }
            | Phase::Downloading { info, .. }
            | Phase::Ready { info }
            | Phase::Installing { info } => Some(info.version.as_str()),
            _ => None,
        };
        Snapshot {
            current_version: self.shared.current.to_string(),
            channel: channel_for(&self.shared.current),
            state: machine.phase.clone(),
            dismissed: shown.is_some() && machine.dismissed_version.as_deref() == shown,
            blocked: machine.blocked.clone(),
            error: machine.error.clone(),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let machine = self.machine();
        self.snapshot_of(&machine)
    }

    fn publish(&self, snapshot: &Snapshot) {
        self.shared.publisher.publish(snapshot);
    }

    /// Look for an update.
    ///
    /// A request that arrives while a check, download or install is already
    /// under way, or once an update is downloaded, changes nothing and returns
    /// the current snapshot: overlapping requests are absorbed, not queued.
    pub async fn check(&self, manual: bool) -> Snapshot {
        let before = {
            let mut machine = self.machine();
            match machine.phase {
                Phase::Checking
                | Phase::Downloading { .. }
                | Phase::Ready { .. }
                | Phase::Installing { .. }
                | Phase::Unavailable { .. } => return self.snapshot_of(&machine),
                Phase::Idle | Phase::UpToDate { .. } | Phase::Available { .. } => {}
            }
            let before = std::mem::replace(&mut machine.phase, Phase::Checking);
            machine.error = None;
            machine.blocked = None;
            if manual {
                // Asking again is asking to be told, whatever was dismissed.
                machine.dismissed_version = None;
            }
            let snapshot = self.snapshot_of(&machine);
            drop(machine);
            self.publish(&snapshot);
            before
        };

        let outcome = self.shared.backend.check().await;

        let snapshot = {
            let mut machine = self.machine();
            match outcome {
                // Applied here as well as inside the plugin, so what reaches
                // the interface never depends on the plugin having agreed.
                Ok(Some(remote)) if is_offered(&self.shared.current, &remote.version) => {
                    machine.phase = Phase::Available {
                        info: remote.info(),
                    };
                }
                Ok(_) => {
                    machine.phase = Phase::UpToDate {
                        checked_unix: (self.shared.now)(),
                    };
                }
                Err(UpdateError::Unavailable(reason)) => {
                    machine.phase = Phase::Unavailable { reason };
                }
                Err(error) => {
                    // Whatever was known before is still known.
                    machine.phase = before;
                    machine.error = Some(Failure {
                        stage: Stage::Check,
                        message: error.message(),
                        manual,
                    });
                }
            }
            self.snapshot_of(&machine)
        };
        self.publish(&snapshot);
        snapshot
    }

    /// Download the update that was found, if one was, and nobody is already
    /// doing so. Verified means the backend verified it; only then is it ready.
    pub async fn download(&self) -> Snapshot {
        let info = {
            let mut machine = self.machine();
            let Phase::Available { info } = &machine.phase else {
                return self.snapshot_of(&machine);
            };
            let info = info.clone();
            machine.phase = Phase::Downloading {
                info: info.clone(),
                received: 0,
                total: None,
            };
            machine.error = None;
            machine.blocked = None;
            let snapshot = self.snapshot_of(&machine);
            drop(machine);
            self.publish(&snapshot);
            info
        };

        let last_sent = Mutex::new(Instant::now() - Duration::from_secs(1));
        let updater = self.clone();
        let progress = move |chunk: u64, total: Option<u64>| {
            let mut machine = updater.machine();
            if let Phase::Downloading {
                received,
                total: known,
                ..
            } = &mut machine.phase
            {
                *received += chunk;
                // A length of zero is a server that did not know, not an
                // empty download.
                if let Some(total) = total.filter(|total| *total > 0) {
                    *known = Some(total);
                }
            }
            let snapshot = updater.snapshot_of(&machine);
            drop(machine);
            let mut last = last_sent.lock().unwrap_or_else(|e| e.into_inner());
            if last.elapsed() >= Duration::from_millis(100) {
                *last = Instant::now();
                drop(last);
                updater.publish(&snapshot);
            }
        };

        let outcome = self.shared.backend.download(&progress).await;

        let snapshot = {
            let mut machine = self.machine();
            match outcome {
                Ok(()) => machine.phase = Phase::Ready { info },
                Err(error) => {
                    // Back to available: the update is still there to be
                    // downloaded again, and nothing partial is kept.
                    self.shared.backend.discard();
                    machine.phase = Phase::Available { info };
                    machine.error = Some(Failure {
                        stage: Stage::Download,
                        message: error.message(),
                        manual: true,
                    });
                }
            }
            self.snapshot_of(&machine)
        };
        self.publish(&snapshot);
        snapshot
    }

    /// Install the downloaded update and restart into it.
    ///
    /// The order is the safety argument, and it does not vary:
    ///
    /// 1. Only a verified, downloaded update can be installed at all.
    /// 2. The gate is claimed. If a move, restore, delete or rummage holds it,
    ///    this returns [`ScuttleError::Busy`] with words for whoever is in the
    ///    way, the update stays ready, and nothing else has changed.
    /// 3. With the gate held, nothing that changes a file can start. Only now
    ///    is the state saved and the last snapshot sent — on Windows the
    ///    process may end inside the next step.
    /// 4. The application is replaced. If that fails, the gate and the
    ///    scheduler are given back and Scuttle carries on as it was.
    pub async fn install(&self) -> Result<Snapshot> {
        {
            let mut machine = self.machine();
            match machine.phase {
                Phase::Ready { .. } => {}
                Phase::Installing { .. } => return Ok(self.snapshot_of(&machine)),
                _ => {
                    return Err(ScuttleError::Refused(
                        "There is no downloaded update to install.".into(),
                    ))
                }
            }
            machine.blocked = None;
        }

        let lease = match self.shared.host.begin_install() {
            Ok(lease) => lease,
            Err(message) => {
                let snapshot = {
                    let mut machine = self.machine();
                    machine.blocked = Some(message.clone());
                    self.snapshot_of(&machine)
                };
                self.publish(&snapshot);
                return Err(ScuttleError::Busy(message));
            }
        };

        let info = {
            let mut machine = self.machine();
            // The state can only have moved if another install request got in
            // first, which the gate refused above; guard it anyway.
            let Phase::Ready { info } = machine.phase.clone() else {
                drop(machine);
                lease.abandon();
                return Err(ScuttleError::Refused(
                    "There is no downloaded update to install.".into(),
                ));
            };
            machine.phase = Phase::Installing { info: info.clone() };
            machine.error = None;
            machine.lease = Some(lease);
            let snapshot = self.snapshot_of(&machine);
            drop(machine);
            // The last word before the process may end.
            self.publish(&snapshot);
            info
        };

        match self.shared.backend.install().await {
            Ok(()) => {
                // The real backend does not return from a successful install.
                // If one does, the lease stays in the machine and the gate
                // stays shut: this process is on its way out.
                Ok(self.snapshot())
            }
            Err(error) => {
                self.shared.backend.discard();
                let (snapshot, lease) = {
                    let mut machine = self.machine();
                    machine.phase = Phase::Available { info };
                    machine.error = Some(Failure {
                        stage: Stage::Install,
                        message: error.message(),
                        manual: true,
                    });
                    (self.snapshot_of(&machine), machine.lease.take())
                };
                if let Some(lease) = lease {
                    lease.abandon();
                }
                self.publish(&snapshot);
                Ok(snapshot)
            }
        }
    }

    /// Stop showing the notice for the version currently offered, until a
    /// newer one turns up or the person asks again.
    pub fn dismiss(&self) -> Snapshot {
        let snapshot = {
            let mut machine = self.machine();
            let version = match &machine.phase {
                Phase::Available { info }
                | Phase::Downloading { info, .. }
                | Phase::Ready { info } => Some(info.version.clone()),
                _ => None,
            };
            if let Some(version) = version {
                machine.dismissed_version = Some(version);
            }
            self.snapshot_of(&machine)
        };
        self.publish(&snapshot);
        snapshot
    }

    /// Whether an install has begun and this process is on its way out.
    pub fn installing(&self) -> bool {
        matches!(self.machine().phase, Phase::Installing { .. })
    }
}

/// Whether an exit that has been requested should be put off.
///
/// While an install is replacing the application, a Quit — ⌘Q, the tray's Quit,
/// a shutdown request — is deferred rather than allowed to stop it halfway.
/// The install's *own* restart is not: on macOS Tauri delivers it through the
/// same exit request, tagged with [`tauri::RESTART_EXIT_CODE`], and stopping
/// that would leave Scuttle sitting in "installing" with its gate shut and
/// nothing left to finish it.
pub fn defers_exit(installing: bool, code: Option<i32>) -> bool {
    installing && code != Some(tauri::RESTART_EXIT_CODE)
}

/// A flag the auto-check loop can wait on, so turning the setting on checks
/// promptly instead of at the next daily wake.
#[derive(Default)]
pub struct AutoCheckSwitch {
    pub wake: tokio::sync::Notify,
    stopped: AtomicBool,
}

impl AutoCheckSwitch {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quit_during_an_install_is_deferred() {
        // ⌘Q and a shutdown carry no code; the tray's Quit says 0.
        assert!(defers_exit(true, None));
        assert!(defers_exit(true, Some(0)));
    }

    #[test]
    fn the_installs_own_restart_is_never_deferred() {
        assert!(!defers_exit(true, Some(tauri::RESTART_EXIT_CODE)));
    }

    #[test]
    fn nothing_is_deferred_when_no_install_is_under_way() {
        assert!(!defers_exit(false, None));
        assert!(!defers_exit(false, Some(0)));
        assert!(!defers_exit(false, Some(tauri::RESTART_EXIT_CODE)));
    }
}
