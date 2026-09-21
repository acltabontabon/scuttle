//! Updating, end to end — as far as it can go without a network, a signed
//! release and a packaged application.
//!
//! What is real here: the state machine, the channel policy, and the operation
//! gate, which is the same one every move, restore and delete goes through.
//! What is mocked: the updater plugin, behind [`Backend`]. So these tests show
//! that Scuttle decides correctly *given* what the plugin says; they cannot
//! show that the plugin downloads, verifies or replaces anything. That is what
//! the two-signed-builds procedure in `docs/updates.md` is for.
//!
//! Each test is named for the failure it prevents.

mod fixtures;

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use fixtures::World;
use scuttle_core::commands::{AppState, Operation, Priority};
use scuttle_core::platform::PlatformService;
use scuttle_core::storage::ScanKind;
use scuttle_core::updater::{
    Backend, BoxFuture, Channel, Failure, Phase, Publish, Remote, Snapshot, Stage, StateHost,
    UpdateError, Updater,
};
use scuttle_core::ScuttleError;
use semver::Version;

fn run<F: Future>(future: F) -> F::Output {
    tauri::async_runtime::block_on(future)
}

/// Something a test can hold a mock at, and let go of.
#[derive(Default)]
struct Latch {
    open: Mutex<bool>,
    changed: Condvar,
    reached: AtomicUsize,
}

impl Latch {
    /// Called by the mock: note that it got here, then wait to be released.
    fn pass(&self) {
        self.reached.fetch_add(1, Ordering::SeqCst);
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.changed.wait(open).unwrap();
        }
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }

    fn wait_until_reached(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.reached.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "the mock was never reached");
            thread::sleep(Duration::from_millis(2));
        }
    }
}

struct MockBackend {
    check_result: Mutex<Result<Option<Remote>, UpdateError>>,
    download_result: Mutex<Result<(), UpdateError>>,
    install_result: Mutex<Result<(), UpdateError>>,
    chunks: Mutex<Vec<(u64, Option<u64>)>>,
    check_latch: Option<Arc<Latch>>,
    download_latch: Option<Arc<Latch>>,
    install_latch: Option<Arc<Latch>>,
    checks: AtomicUsize,
    downloads: AtomicUsize,
    installs: AtomicUsize,
    discards: AtomicUsize,
    /// Whether a verified package is currently held, to prove that a failed
    /// signature check never leaves one behind for `install` to find.
    holds_package: Mutex<bool>,
    /// Called after every chunk, so a test can look at the machine mid-download.
    probe: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl MockBackend {
    fn new() -> MockBackend {
        MockBackend {
            check_result: Mutex::new(Ok(None)),
            download_result: Mutex::new(Ok(())),
            install_result: Mutex::new(Ok(())),
            chunks: Mutex::new(vec![(500, Some(1000)), (500, Some(1000))]),
            check_latch: None,
            download_latch: None,
            install_latch: None,
            checks: AtomicUsize::new(0),
            downloads: AtomicUsize::new(0),
            installs: AtomicUsize::new(0),
            discards: AtomicUsize::new(0),
            holds_package: Mutex::new(false),
            probe: Mutex::new(None),
        }
    }

    fn offering(self, version: &str) -> MockBackend {
        *self.check_result.lock().unwrap() = Ok(Some(remote(version)));
        self
    }

    fn set_check(&self, result: Result<Option<Remote>, UpdateError>) {
        *self.check_result.lock().unwrap() = result;
    }
    fn set_download(&self, result: Result<(), UpdateError>) {
        *self.download_result.lock().unwrap() = result;
    }
    fn set_install(&self, result: Result<(), UpdateError>) {
        *self.install_result.lock().unwrap() = result;
    }
    fn count(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::SeqCst)
    }
}

fn remote(version: &str) -> Remote {
    Remote {
        version: Version::parse(version).unwrap(),
        notes: Some("Fixed a thing.".into()),
        date_unix: Some(1_700_000_000),
    }
}

impl Backend for MockBackend {
    fn check(&self) -> BoxFuture<'_, Result<Option<Remote>, UpdateError>> {
        Box::pin(async move {
            self.checks.fetch_add(1, Ordering::SeqCst);
            if let Some(latch) = &self.check_latch {
                latch.pass();
            }
            self.check_result.lock().unwrap().clone()
        })
    }

    fn download<'a>(
        &'a self,
        progress: &'a (dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> BoxFuture<'a, Result<(), UpdateError>> {
        Box::pin(async move {
            self.downloads.fetch_add(1, Ordering::SeqCst);
            if let Some(latch) = &self.download_latch {
                latch.pass();
            }
            for (chunk, total) in self.chunks.lock().unwrap().clone() {
                progress(chunk, total);
                let probe = self.probe.lock().unwrap().clone();
                if let Some(probe) = probe {
                    probe();
                }
            }
            let result = self.download_result.lock().unwrap().clone();
            // Only a download that verified leaves a package behind.
            *self.holds_package.lock().unwrap() = result.is_ok();
            result
        })
    }

    fn install(&self) -> BoxFuture<'_, Result<(), UpdateError>> {
        Box::pin(async move {
            self.installs.fetch_add(1, Ordering::SeqCst);
            assert!(
                *self.holds_package.lock().unwrap(),
                "install was reached without a verified package"
            );
            if let Some(latch) = &self.install_latch {
                latch.pass();
            }
            *self.holds_package.lock().unwrap() = false;
            self.install_result.lock().unwrap().clone()
        })
    }

    fn discard(&self) {
        self.discards.fetch_add(1, Ordering::SeqCst);
        *self.holds_package.lock().unwrap() = false;
    }
}

#[derive(Default)]
struct Collector(Mutex<Vec<Snapshot>>);

impl Publish for Collector {
    fn publish(&self, snapshot: &Snapshot) {
        self.0.lock().unwrap().push(snapshot.clone());
    }
}

impl Collector {
    fn phases(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|snapshot| phase_name(&snapshot.state).to_string())
            .collect()
    }
}

fn phase_name(phase: &Phase) -> &'static str {
    match phase {
        Phase::Idle => "idle",
        Phase::Checking => "checking",
        Phase::UpToDate { .. } => "up_to_date",
        Phase::Available { .. } => "available",
        Phase::Downloading { .. } => "downloading",
        Phase::Ready { .. } => "ready",
        Phase::Installing { .. } => "installing",
        Phase::Unavailable { .. } => "unavailable",
    }
}

struct Rig {
    updater: Updater,
    backend: Arc<MockBackend>,
    state: AppState,
    events: Arc<Collector>,
    world: World,
}

fn rig_with(backend: MockBackend, current: &str) -> Rig {
    let world = World::new();
    let platform: Arc<dyn PlatformService> = Arc::new(world.platform.clone());
    let state = AppState::new(platform).expect("app state");
    let backend = Arc::new(backend);
    let events = Arc::new(Collector::default());
    let updater = Updater::with_clock(
        backend.clone(),
        Arc::new(StateHost {
            state: state.clone(),
            app: None,
        }),
        events.clone(),
        Version::parse(current).unwrap(),
        || 1_700_000_000,
    );
    Rig {
        updater,
        backend,
        state,
        events,
        world,
    }
}

fn rig(current: &str, offered: &str) -> Rig {
    rig_with(MockBackend::new().offering(offered), current)
}

/// A rig that has already checked, downloaded, and is ready to install.
fn ready_rig() -> Rig {
    let rig = rig("0.1.0-alpha.2", "0.1.0-alpha.3");
    run(rig.updater.check(true));
    run(rig.updater.download());
    assert!(matches!(rig.updater.snapshot().state, Phase::Ready { .. }));
    rig
}

fn phase(rig: &Rig) -> &'static str {
    phase_name(&rig.updater.snapshot().state)
}

// ---- finding an update --------------------------------------------------

#[test]
fn a_new_updater_has_looked_at_nothing_and_knows_its_channel() {
    let alpha = rig("0.1.0-alpha.2", "0.1.0-alpha.3");
    assert_eq!(phase(&alpha), "idle");
    assert_eq!(alpha.updater.snapshot().channel, Channel::Alpha);
    assert_eq!(alpha.updater.snapshot().current_version, "0.1.0-alpha.2");

    let stable = rig("0.1.0", "0.1.1");
    assert_eq!(stable.updater.snapshot().channel, Channel::Stable);
}

#[test]
fn a_newer_release_is_available_with_its_notes() {
    let rig = rig("0.1.0-alpha.2", "0.1.0-alpha.3");
    let snapshot = run(rig.updater.check(true));

    let Phase::Available { info } = snapshot.state else {
        panic!("expected available, got {:?}", snapshot.state);
    };
    assert_eq!(info.version, "0.1.0-alpha.3");
    assert_eq!(info.notes.as_deref(), Some("Fixed a thing."));
    assert!(snapshot.error.is_none());
    // Nothing is downloaded until somebody says so.
    assert_eq!(MockBackend::count(&rig.backend.downloads), 0);
}

#[test]
fn nothing_newer_is_up_to_date() {
    let rig = rig_with(MockBackend::new(), "0.1.0");
    let snapshot = run(rig.updater.check(true));
    assert!(matches!(snapshot.state, Phase::UpToDate { .. }));
}

#[test]
fn a_backend_offering_the_same_or_an_older_version_is_not_believed() {
    // Whatever the plugin returns, the policy is applied again on the result.
    for offered in ["0.1.0", "0.0.9", "0.1.0-alpha.9"] {
        let rig = rig("0.1.0", offered);
        let snapshot = run(rig.updater.check(true));
        assert!(
            matches!(snapshot.state, Phase::UpToDate { .. }),
            "{offered} should not have been offered to 0.1.0"
        );
    }
}

#[test]
fn a_stable_installation_is_never_shown_a_prerelease() {
    let rig = rig("0.1.0", "0.2.0-alpha.1");
    let snapshot = run(rig.updater.check(true));
    assert!(matches!(snapshot.state, Phase::UpToDate { .. }));
    assert_eq!(snapshot.channel, Channel::Stable);
}

#[test]
fn an_alpha_installation_is_offered_the_stable_release_it_led_up_to() {
    let rig = rig("0.1.0-alpha.9", "0.1.0");
    let snapshot = run(rig.updater.check(false));
    assert!(matches!(snapshot.state, Phase::Available { .. }));
}

// ---- failing to find one ------------------------------------------------

#[test]
fn a_failed_manual_check_says_so_and_a_retry_recovers() {
    let rig = rig("0.1.0", "0.1.1");
    rig.backend.set_check(Err(UpdateError::Offline));

    let failed = run(rig.updater.check(true));
    assert!(matches!(failed.state, Phase::Idle));
    let error = failed.error.expect("a manual check that fails is reported");
    assert_eq!(error.stage, Stage::Check);
    assert!(error.manual);
    assert!(error.message.contains("could not reach"));

    rig.backend.set_check(Ok(Some(remote("0.1.1"))));
    let retried = run(rig.updater.check(true));
    assert!(matches!(retried.state, Phase::Available { .. }));
    assert!(retried.error.is_none(), "the old failure is cleared");
}

#[test]
fn a_failed_background_check_is_marked_quiet() {
    let rig = rig("0.1.0", "0.1.1");
    rig.backend.set_check(Err(UpdateError::Timeout));
    let snapshot = run(rig.updater.check(false));
    let Failure { manual, .. } = snapshot.error.expect("recorded");
    assert!(!manual, "the interface keys off this to stay quiet");
}

#[test]
fn a_failed_check_does_not_forget_an_update_already_found() {
    let rig = rig("0.1.0", "0.1.1");
    run(rig.updater.check(false));
    rig.backend.set_check(Err(UpdateError::Offline));

    let snapshot = run(rig.updater.check(false));
    assert!(
        matches!(snapshot.state, Phase::Available { .. }),
        "still available; a dropped connection is not a withdrawn release"
    );
    assert!(snapshot.error.is_some());
}

#[test]
fn malformed_metadata_and_missing_platforms_never_become_an_update() {
    for (error, words) in [
        (UpdateError::BadManifest, "not something Scuttle could read"),
        (UpdateError::NoPlatform, "this kind of computer"),
    ] {
        let rig = rig("0.1.0", "0.1.1");
        rig.backend.set_check(Err(error));
        let snapshot = run(rig.updater.check(true));
        assert!(!matches!(snapshot.state, Phase::Available { .. }));
        assert!(snapshot.error.expect("reported").message.contains(words));
    }
}

#[test]
fn an_installation_that_cannot_update_says_why_once_and_stops_asking() {
    let rig = rig_with(MockBackend::new(), "0.1.0");
    rig.backend
        .set_check(Err(UpdateError::Unavailable("Not installed.".into())));

    let first = run(rig.updater.check(false));
    assert!(matches!(first.state, Phase::Unavailable { .. }));
    run(rig.updater.check(true));
    run(rig.updater.check(false));
    assert_eq!(MockBackend::count(&rig.backend.checks), 1);
}

// ---- overlapping requests -----------------------------------------------

#[test]
fn two_checks_at_once_make_one_request() {
    let latch = Arc::new(Latch::default());
    let mut backend = MockBackend::new().offering("0.1.1");
    backend.check_latch = Some(latch.clone());
    let rig = rig_with(backend, "0.1.0");

    let first = {
        let updater = rig.updater.clone();
        thread::spawn(move || run(updater.check(true)))
    };
    latch.wait_until_reached();
    assert_eq!(phase(&rig), "checking");

    // Arrives while the first is in flight: absorbed, not queued.
    let second = run(rig.updater.check(true));
    assert!(matches!(second.state, Phase::Checking));

    latch.release();
    let done = first.join().unwrap();
    assert!(matches!(done.state, Phase::Available { .. }));
    assert_eq!(MockBackend::count(&rig.backend.checks), 1);
}

#[test]
fn two_downloads_at_once_make_one_download() {
    let latch = Arc::new(Latch::default());
    let mut backend = MockBackend::new().offering("0.1.1");
    backend.download_latch = Some(latch.clone());
    let rig = rig_with(backend, "0.1.0");
    run(rig.updater.check(true));

    let first = {
        let updater = rig.updater.clone();
        thread::spawn(move || run(updater.download()))
    };
    latch.wait_until_reached();
    let second = run(rig.updater.download());
    assert!(matches!(second.state, Phase::Downloading { .. }));

    // And a check while downloading must not disturb it.
    let during = run(rig.updater.check(true));
    assert!(matches!(during.state, Phase::Downloading { .. }));

    latch.release();
    assert!(matches!(first.join().unwrap().state, Phase::Ready { .. }));
    assert_eq!(MockBackend::count(&rig.backend.downloads), 1);
    assert_eq!(MockBackend::count(&rig.backend.checks), 1);
}

#[test]
fn nothing_is_downloaded_unless_an_update_was_found() {
    let rig = rig("0.1.0", "0.1.1");
    let snapshot = run(rig.updater.download());
    assert!(matches!(snapshot.state, Phase::Idle));
    assert_eq!(MockBackend::count(&rig.backend.downloads), 0);
}

// ---- downloading --------------------------------------------------------

/// What the machine said after each chunk of a download.
fn downloading_as_seen(rig: &Rig) -> Vec<(u64, Option<u64>)> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let probe = {
        let updater = rig.updater.clone();
        let seen = seen.clone();
        Arc::new(move || {
            if let Phase::Downloading {
                received, total, ..
            } = updater.snapshot().state
            {
                seen.lock().unwrap().push((received, total));
            }
        })
    };
    *rig.backend.probe.lock().unwrap() = Some(probe);
    let done = run(rig.updater.download());
    assert!(matches!(done.state, Phase::Ready { .. }));
    let seen = seen.lock().unwrap().clone();
    seen
}

#[test]
fn progress_is_counted_and_a_known_size_is_kept() {
    let rig = rig("0.1.0", "0.1.1");
    run(rig.updater.check(true));
    assert_eq!(
        downloading_as_seen(&rig),
        vec![(500, Some(1000)), (1000, Some(1000))]
    );
    let phases = rig.events.phases();
    assert_eq!(phases.first().map(String::as_str), Some("checking"));
    assert_eq!(phases.last().map(String::as_str), Some("ready"));
}

#[test]
fn a_download_of_unknown_size_is_reported_without_inventing_one() {
    let backend = MockBackend::new().offering("0.1.1");
    // No length at all, and a length of zero, which a server sends when it
    // does not know rather than when the download is empty.
    *backend.chunks.lock().unwrap() = vec![(300, None), (300, Some(0)), (300, None)];
    let rig = rig_with(backend, "0.1.0");
    run(rig.updater.check(true));

    assert_eq!(
        downloading_as_seen(&rig),
        vec![(300, None), (600, None), (900, None)],
        "bytes add up, and no total is made up"
    );
}

#[test]
fn a_size_that_turns_up_partway_is_used_from_then_on() {
    let backend = MockBackend::new().offering("0.1.1");
    *backend.chunks.lock().unwrap() = vec![(100, None), (100, Some(400)), (100, None)];
    let rig = rig_with(backend, "0.1.0");
    run(rig.updater.check(true));

    assert_eq!(
        downloading_as_seen(&rig),
        vec![(100, None), (200, Some(400)), (300, Some(400))]
    );
}

#[test]
fn a_package_that_fails_its_signature_is_never_ready_and_never_installable() {
    let rig = rig("0.1.0", "0.1.1");
    run(rig.updater.check(true));
    rig.backend.set_download(Err(UpdateError::BadSignature));

    let snapshot = run(rig.updater.download());
    assert!(
        matches!(snapshot.state, Phase::Available { .. }),
        "back to available, not ready"
    );
    let error = snapshot.error.expect("reported");
    assert_eq!(error.stage, Stage::Download);
    assert!(error.message.contains("signature"));
    assert!(error.message.contains("Nothing was installed"));
    assert!(MockBackend::count(&rig.backend.discards) >= 1);

    // And there is no way to install it anyway.
    let refused = run(rig.updater.install());
    assert!(matches!(refused, Err(ScuttleError::Refused(_))));
    assert_eq!(MockBackend::count(&rig.backend.installs), 0);
    assert_eq!(rig.state.current_operation(), None);
}

#[test]
fn an_interrupted_download_can_be_tried_again() {
    let rig = rig("0.1.0", "0.1.1");
    run(rig.updater.check(true));
    rig.backend.set_download(Err(UpdateError::Interrupted));
    let failed = run(rig.updater.download());
    assert!(matches!(failed.state, Phase::Available { .. }));
    assert!(failed.error.unwrap().message.contains("stopped partway"));

    rig.backend.set_download(Ok(()));
    let retried = run(rig.updater.download());
    assert!(matches!(retried.state, Phase::Ready { .. }));
    assert!(retried.error.is_none());
}

// ---- installing: the gate -----------------------------------------------

#[test]
fn only_a_downloaded_update_can_be_installed() {
    let rig = rig("0.1.0", "0.1.1");
    assert!(matches!(
        run(rig.updater.install()),
        Err(ScuttleError::Refused(_))
    ));
    run(rig.updater.check(true));
    assert!(matches!(
        run(rig.updater.install()),
        Err(ScuttleError::Refused(_))
    ));
    assert_eq!(MockBackend::count(&rig.backend.installs), 0);
}

#[test]
fn installing_waits_for_every_kind_of_file_operation() {
    for operation in [
        Operation::Move,
        Operation::Restore,
        Operation::EmptyDrawer,
        Operation::RemoveItem,
        Operation::Refresh,
        Operation::Sweep,
        Operation::Decide,
    ] {
        let rig = ready_rig();
        let _held = rig.state.begin_operation(operation).expect("free gate");

        let refused = run(rig.updater.install());
        let Err(ScuttleError::Busy(message)) = refused else {
            panic!("{operation:?} should have blocked the install");
        };
        assert!(
            message.contains("Try again once that finishes"),
            "says what to do: {message}"
        );
        assert_eq!(
            MockBackend::count(&rig.backend.installs),
            0,
            "{operation:?} was running and the install went ahead"
        );

        let snapshot = rig.updater.snapshot();
        assert!(matches!(snapshot.state, Phase::Ready { .. }), "still ready");
        assert_eq!(snapshot.blocked.as_deref(), Some(message.as_str()));
        assert!(!rig.state.installing());
    }
}

#[test]
fn the_reason_names_what_is_in_the_way() {
    let rig = ready_rig();
    let _held = rig.state.begin_operation(Operation::Move).unwrap();
    let Err(ScuttleError::Busy(message)) = run(rig.updater.install()) else {
        panic!("blocked");
    };
    assert!(
        message.contains("moving files into the Drawer"),
        "{message}"
    );
}

#[test]
fn a_rummage_in_progress_is_not_thrown_away_for_an_update() {
    let rig = ready_rig();
    let options = rig.world.options.clone();
    rig.state.start_scan(&options).expect("a rummage");

    let Err(ScuttleError::Busy(message)) = run(rig.updater.install()) else {
        panic!("a rummage a person is waiting on blocks the install");
    };
    assert!(message.contains("looking around"), "{message}");
}

#[test]
fn a_background_check_stands_aside_for_an_install() {
    let rig = ready_rig();
    let options = rig.world.options.clone();
    rig.state
        .start_scan_as(&options, ScanKind::Glance, Priority::Background)
        .expect("a background check");
    assert!(rig.state.background_scan_running());

    let snapshot = run(rig.updater.install()).expect("only a check was running");
    assert!(matches!(snapshot.state, Phase::Installing { .. }));
    assert!(!rig.state.background_scan_running());
}

#[test]
fn installing_goes_ahead_once_the_work_has_finished() {
    let rig = ready_rig();
    let held = rig.state.begin_operation(Operation::Restore).unwrap();
    assert!(run(rig.updater.install()).is_err());

    drop(held);
    let snapshot = run(rig.updater.install()).expect("the gate is free now");
    assert!(matches!(snapshot.state, Phase::Installing { .. }));
    assert_eq!(MockBackend::count(&rig.backend.installs), 1);
    assert!(snapshot.blocked.is_none(), "the old refusal is cleared");
}

#[test]
fn finishing_an_operation_does_not_install_by_itself() {
    let rig = ready_rig();
    let held = rig.state.begin_operation(Operation::Move).unwrap();
    assert!(run(rig.updater.install()).is_err());
    drop(held);

    // Nothing restarts because the work ended; only being asked does.
    thread::sleep(Duration::from_millis(50));
    assert_eq!(MockBackend::count(&rig.backend.installs), 0);
    assert!(matches!(rig.updater.snapshot().state, Phase::Ready { .. }));
}

#[test]
fn once_installing_has_begun_no_file_operation_can_start() {
    let latch = Arc::new(Latch::default());
    let mut backend = MockBackend::new().offering("0.1.1");
    backend.install_latch = Some(latch.clone());
    let rig = rig_with(backend, "0.1.0");
    run(rig.updater.check(true));
    run(rig.updater.download());

    let installing = {
        let updater = rig.updater.clone();
        thread::spawn(move || run(updater.install()))
    };
    latch.wait_until_reached();

    // The install is committed and is in the middle of replacing the app.
    assert!(rig.state.installing());
    let options = rig.world.options.clone();
    for operation in [
        Operation::Scan,
        Operation::Move,
        Operation::Restore,
        Operation::EmptyDrawer,
        Operation::RemoveItem,
        Operation::Refresh,
        Operation::Sweep,
        Operation::Decide,
    ] {
        let refused = rig.state.begin_operation(operation);
        let Err(ScuttleError::Busy(message)) = refused else {
            panic!("{operation:?} started underneath an install");
        };
        assert!(message.contains("getting ready to update"), "{message}");
    }
    assert!(rig.state.start_scan(&options).is_err());
    // Not even the scheduler's own kind of request gets in.
    assert!(rig
        .state
        .begin_operation_as(Operation::Sweep, Priority::Background)
        .is_err());
    // A second install request during the first is absorbed.
    let again = run(rig.updater.install()).expect("absorbed");
    assert!(matches!(again.state, Phase::Installing { .. }));

    latch.release();
    installing.join().unwrap().expect("installed");
    assert_eq!(MockBackend::count(&rig.backend.installs), 1);

    // The backend returned (a real one would have replaced the process), and
    // the gate is still shut: this process is on its way out.
    assert_eq!(rig.state.current_operation(), Some(Operation::Update));
    assert!(rig.state.installing());
}

#[test]
fn a_failed_install_leaves_scuttle_usable_and_the_update_retryable() {
    let rig = ready_rig();
    rig.backend
        .set_install(Err(UpdateError::InstallFailed("Disk full.".into())));

    let snapshot = run(rig.updater.install()).expect("a failure is a snapshot");
    assert!(matches!(snapshot.state, Phase::Available { .. }));
    let error = snapshot.error.expect("reported");
    assert_eq!(error.stage, Stage::Install);
    assert!(error.message.contains("still running"));
    assert!(error.message.contains("Disk full."));

    // Usable again: the gate is free, nothing believes it is installing, and
    // file operations start.
    assert_eq!(rig.state.current_operation(), None);
    assert!(!rig.state.installing());
    rig.state
        .begin_operation(Operation::Move)
        .expect("a move can start after a failed install");
}

#[test]
fn a_failed_install_can_be_retried_end_to_end() {
    let rig = ready_rig();
    rig.backend
        .set_install(Err(UpdateError::InstallFailed("Busy volume.".into())));
    run(rig.updater.install()).unwrap();

    rig.backend.set_install(Ok(()));
    let downloaded = run(rig.updater.download());
    assert!(matches!(downloaded.state, Phase::Ready { .. }));
    let installed = run(rig.updater.install()).unwrap();
    assert!(matches!(installed.state, Phase::Installing { .. }));
}

// ---- the race -----------------------------------------------------------

/// The claim on the gate is the commitment, so a file operation and an
/// install can never both hold it. Hammer both and check nothing overlaps.
#[test]
fn a_file_operation_and_an_install_never_hold_the_gate_together() {
    let rig = rig("0.1.0", "0.1.1");
    let inside = Arc::new(AtomicUsize::new(0));
    let overlaps = Arc::new(AtomicUsize::new(0));
    let operations_won = Arc::new(AtomicUsize::new(0));
    let installs_won = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut threads = Vec::new();
    for _ in 0..4 {
        let state = rig.state.clone();
        let (inside, overlaps, won, stop) = (
            inside.clone(),
            overlaps.clone(),
            operations_won.clone(),
            stop.clone(),
        );
        threads.push(thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if let Ok(guard) = state.begin_operation(Operation::Restore) {
                    if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                        overlaps.fetch_add(1, Ordering::SeqCst);
                    }
                    won.fetch_add(1, Ordering::SeqCst);
                    thread::yield_now();
                    inside.fetch_sub(1, Ordering::SeqCst);
                    drop(guard);
                }
                thread::yield_now();
            }
        }));
    }
    for _ in 0..2 {
        let state = rig.state.clone();
        let (inside, overlaps, won, stop) = (
            inside.clone(),
            overlaps.clone(),
            installs_won.clone(),
            stop.clone(),
        );
        threads.push(thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if let Ok(mut lease) = state.begin_install() {
                    if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                        overlaps.fetch_add(1, Ordering::SeqCst);
                    }
                    assert!(state.installing(), "the flag follows the claim");
                    won.fetch_add(1, Ordering::SeqCst);
                    thread::yield_now();
                    inside.fetch_sub(1, Ordering::SeqCst);
                    lease.release();
                }
                thread::yield_now();
            }
        }));
    }

    // Until both sides have won the gate plenty of times, so the test is
    // about contention rather than about how the scheduler happened to fall.
    let deadline = Instant::now() + Duration::from_secs(20);
    while (operations_won.load(Ordering::SeqCst) < 200 || installs_won.load(Ordering::SeqCst) < 200)
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(5));
    }
    stop.store(true, Ordering::SeqCst);
    for thread in threads {
        thread.join().unwrap();
    }

    assert_eq!(overlaps.load(Ordering::SeqCst), 0, "the gate was shared");
    assert!(operations_won.load(Ordering::SeqCst) >= 200);
    assert!(installs_won.load(Ordering::SeqCst) >= 200);
    assert_eq!(rig.state.current_operation(), None);
    assert!(!rig.state.installing());
}

// ---- dismissing ---------------------------------------------------------

#[test]
fn a_dismissed_version_stays_dismissed_for_the_session() {
    let rig = rig("0.1.0", "0.1.1");
    run(rig.updater.check(false));
    assert!(!rig.updater.snapshot().dismissed);

    let dismissed = rig.updater.dismiss();
    assert!(dismissed.dismissed);

    // A background re-check that finds the same version must not undo it.
    let again = run(rig.updater.check(false));
    assert!(matches!(again.state, Phase::Available { .. }));
    assert!(
        again.dismissed,
        "no nagging about a version already waved off"
    );
}

#[test]
fn a_newer_version_is_announced_even_after_an_older_one_was_dismissed() {
    let rig = rig("0.1.0", "0.1.1");
    run(rig.updater.check(false));
    rig.updater.dismiss();

    rig.backend.set_check(Ok(Some(remote("0.1.2"))));
    let newer = run(rig.updater.check(false));
    let Phase::Available { info } = &newer.state else {
        panic!("available");
    };
    assert_eq!(info.version, "0.1.2");
    assert!(!newer.dismissed);
}

#[test]
fn asking_by_hand_shows_a_dismissed_update_again() {
    let rig = rig("0.1.0", "0.1.1");
    run(rig.updater.check(false));
    rig.updater.dismiss();
    let asked = run(rig.updater.check(true));
    assert!(!asked.dismissed);
}
