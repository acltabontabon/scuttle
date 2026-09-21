//! The real backend: `tauri-plugin-updater`, and the timer that calls it.
//!
//! Everything in here is glue, and none of it can be exercised without a
//! packaged application, a signed release and a network. The parts that
//! decide anything — channel policy, the state machine, the gate — are tested
//! elsewhere against mocks; the parts that only *do* — download, verify,
//! replace, relaunch — are the plugin's, and are covered by the end-to-end
//! procedure in `docs/updates.md` rather than by anything here.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use semver::Version;
use tauri::{Emitter, Manager, Url};
use tauri_plugin_updater::{Error as PluginError, Update, UpdaterExt};

use super::{
    channel_for, is_offered, AutoCheckSwitch, Backend, BoxFuture, Publish, Remote, Snapshot,
    StateHost, UpdateError, Updater,
};
use crate::commands::{events, AppState};

/// How long the very first check waits after launch. Long enough that starting
/// Scuttle is never itself made slower or noisier by it.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(15);
/// After that, about once a day for as long as Scuttle stays running.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// The manifest is a few hundred bytes. Waiting longer than this is not
/// patience, it is a hung connection.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// What `tauri.conf.json` holds until a real key is put there. A build with
/// this in it cannot verify anything, so it does not try: updates are reported
/// as unavailable rather than failing in a way that looks like an attack.
const UNSET_PREFIX: &str = "UNSET";

#[derive(Default)]
struct Staged {
    /// What the last check found. Kept across a failed download or install so
    /// trying again does not need another check.
    found: Option<Update>,
    /// The verified package. In memory only; never written anywhere by Scuttle.
    package: Option<Vec<u8>>,
}

pub struct TauriBackend {
    app: tauri::AppHandle,
    current: Version,
    staged: Mutex<Staged>,
}

impl TauriBackend {
    pub fn new(app: tauri::AppHandle, current: Version) -> TauriBackend {
        TauriBackend {
            app,
            current,
            staged: Mutex::new(Staged::default()),
        }
    }

    fn staged(&self) -> std::sync::MutexGuard<'_, Staged> {
        self.staged.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The plugin's updater for this installation's channel, or the reason
    /// there cannot be one.
    fn updater(&self) -> Result<tauri_plugin_updater::Updater, UpdateError> {
        let config = self.app.config().plugins.0.get("updater").ok_or_else(|| {
            UpdateError::Unavailable("Updates are not set up in this build.".into())
        })?;

        let pubkey = config
            .get("pubkey")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim();
        if pubkey.is_empty() || pubkey.starts_with(UNSET_PREFIX) {
            return Err(UpdateError::Unavailable(
                "This build has no update signing key, so it cannot check for updates.".into(),
            ));
        }

        let channel = channel_for(&self.current);
        let endpoints = endpoints(config, channel.slug())?;

        self.app
            .updater_builder()
            .endpoints(endpoints)
            .map_err(|error| classify(&error))?
            .timeout(CHECK_TIMEOUT)
            .version_comparator(|current, remote| is_offered(&current, &remote.version))
            .build()
            // Not an installed copy (a development run has no bundle to
            // replace), or an architecture nothing is published for.
            .map_err(|error| UpdateError::Unavailable(unavailable_reason(&error)))
    }
}

/// The endpoints from the plugin's own configuration, with this installation's
/// channel filled in. Read from the same place the plugin reads them, so a test
/// build can point at somewhere else with an ordinary config overlay.
fn endpoints(config: &serde_json::Value, channel: &str) -> Result<Vec<Url>, UpdateError> {
    let urls: Vec<Url> = config
        .get("endpoints")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(|template| template.replace("{{channel}}", channel))
        .filter_map(|text| Url::parse(&text).ok())
        .collect();
    if urls.is_empty() {
        return Err(UpdateError::Unavailable(
            "This build has nowhere to look for updates.".into(),
        ));
    }
    Ok(urls)
}

fn unavailable_reason(error: &PluginError) -> String {
    match error {
        PluginError::FailedToDetermineExtractPath => {
            "Updates are for an installed copy of Scuttle, and this one is not installed.".into()
        }
        PluginError::UnsupportedArch | PluginError::UnsupportedOs => {
            "Updates are not available for this kind of computer.".into()
        }
        other => format!("Updates are not available: {other}"),
    }
}

/// Sort the plugin's errors by what can be done about them.
fn classify(error: &PluginError) -> UpdateError {
    match error {
        PluginError::Reqwest(inner) if inner.is_timeout() => UpdateError::Timeout,
        PluginError::Reqwest(inner) if inner.is_connect() || inner.is_request() => {
            UpdateError::Offline
        }
        PluginError::Reqwest(inner) if inner.is_decode() => UpdateError::BadManifest,
        PluginError::Reqwest(_) | PluginError::Network(_) => UpdateError::Interrupted,
        PluginError::ReleaseNotFound => UpdateError::Other(
            "Scuttle could not find any update information. Nothing may have been published for \
             this kind of release yet."
                .into(),
        ),
        PluginError::Serialization(_) | PluginError::Semver(_) | PluginError::Http(_) => {
            UpdateError::BadManifest
        }
        PluginError::TargetNotFound(_)
        | PluginError::TargetsNotFound(_)
        | PluginError::UnsupportedArch
        | PluginError::UnsupportedOs => UpdateError::NoPlatform,
        // The signature is wrong, unreadable, absent, or was made for a
        // different version than the manifest claims. All of them mean the
        // package is not to be trusted.
        PluginError::Minisign(_)
        | PluginError::Base64(_)
        | PluginError::SignatureUtf8(_)
        | PluginError::SignedVersionMismatch { .. }
        | PluginError::MissingSignedVersion => UpdateError::BadSignature,
        other => UpdateError::Other(other.to_string()),
    }
}

/// Whether the folder holding the application can be written to by this user.
///
/// On macOS the plugin replaces the bundle in place, and when it cannot it
/// falls back to asking the system for administrator rights. Scuttle does not
/// go looking for elevation: an installation it cannot replace as an ordinary
/// user is one the person updates by hand.
#[cfg(target_os = "macos")]
fn can_replace_bundle() -> Result<(), UpdateError> {
    use std::os::unix::ffi::OsStrExt;

    let exe = std::env::current_exe().map_err(|e| UpdateError::InstallFailed(e.to_string()))?;
    // …/Scuttle.app/Contents/MacOS/scuttle
    let parent = exe.ancestors().nth(4);
    let Some(parent) = parent else {
        return Ok(());
    };
    let path = std::ffi::CString::new(parent.as_os_str().as_bytes())
        .map_err(|e| UpdateError::InstallFailed(e.to_string()))?;
    // SAFETY: `path` is a valid NUL-terminated string that outlives the call.
    if unsafe { libc::access(path.as_ptr(), libc::W_OK) } != 0 {
        return Err(UpdateError::InstallFailed(
            "It lives in a folder this account cannot change, and Scuttle does not ask for \
             administrator rights to update itself. Install the new version by hand instead."
                .into(),
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn can_replace_bundle() -> Result<(), UpdateError> {
    Ok(())
}

impl Backend for TauriBackend {
    fn check(&self) -> BoxFuture<'_, Result<Option<Remote>, UpdateError>> {
        Box::pin(async move {
            let updater = self.updater()?;
            let found = updater.check().await.map_err(|error| classify(&error))?;
            let mut staged = self.staged();
            // Whatever was staged belongs to the previous answer.
            staged.package = None;
            match found {
                Some(update) => {
                    let version =
                        Version::parse(&update.version).map_err(|_| UpdateError::BadManifest)?;
                    let remote = Remote {
                        version,
                        notes: update.body.clone(),
                        date_unix: update.date.map(|date| date.unix_timestamp()),
                    };
                    staged.found = Some(update);
                    Ok(Some(remote))
                }
                None => {
                    staged.found = None;
                    Ok(None)
                }
            }
        })
    }

    fn download<'a>(
        &'a self,
        progress: &'a (dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> BoxFuture<'a, Result<(), UpdateError>> {
        Box::pin(async move {
            let update = self
                .staged()
                .found
                .clone()
                .ok_or_else(|| UpdateError::Other("There is no update to download.".into()))?;
            self.staged().package = None;

            // The plugin checks the signature before it returns the bytes, and
            // returns an error rather than the bytes when it does not verify.
            // There is no path from here to `package` that skips it.
            let bytes = update
                .download(|chunk, total| progress(chunk as u64, total), || {})
                .await
                .map_err(|error| classify(&error))?;

            self.staged().package = Some(bytes);
            Ok(())
        })
    }

    fn install(&self) -> BoxFuture<'_, Result<(), UpdateError>> {
        Box::pin(async move {
            let (update, package) = {
                let mut staged = self.staged();
                (staged.found.clone(), staged.package.take())
            };
            let (Some(update), Some(package)) = (update, package) else {
                return Err(UpdateError::InstallFailed(
                    "There is no verified download to install.".into(),
                ));
            };
            can_replace_bundle()?;

            // On Windows the plugin starts the installer and then ends this
            // process, so a successful call never returns there. Everything
            // that had to happen first — the gate, the scheduler, the
            // database, the last snapshot — already has.
            let outcome =
                tauri::async_runtime::spawn_blocking(move || update.install(&package)).await;
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(UpdateError::InstallFailed(error.to_string())),
                Err(error) => return Err(UpdateError::InstallFailed(error.to_string())),
            }

            #[cfg(not(windows))]
            {
                // The relaunched process would otherwise find this one's
                // single-instance socket still open and take itself for a
                // second copy.
                tauri_plugin_single_instance::destroy(&self.app);
                self.app.restart()
            }
            #[cfg(windows)]
            {
                Ok(())
            }
        })
    }

    fn discard(&self) {
        self.staged().package = None;
    }
}

/// Sends every snapshot to the window as `scuttle://update`.
pub struct TauriPublisher(pub tauri::AppHandle);

impl Publish for TauriPublisher {
    fn publish(&self, snapshot: &Snapshot) {
        // The name of the state only: release notes and versions are not the
        // kind of thing a log needs, and paths never appear here at all.
        if let Ok(state) = serde_json::to_value(&snapshot.state) {
            let phase = state.get("phase").and_then(|p| p.as_str()).unwrap_or("?");
            let failed = snapshot.error.as_ref().map(|failure| failure.stage);
            tracing::debug!(
                "update: {phase} (channel {:?}, failure {failed:?})",
                snapshot.channel
            );
        }
        let _ = self.0.emit(events::UPDATE, snapshot);
    }
}

/// Build the updater, hand it to Tauri as managed state, and start the timer
/// that checks for updates in the background. Called once, from setup, and
/// returns immediately: nothing here waits on the network.
pub fn start(app: &tauri::AppHandle, state: &AppState) -> Updater {
    let current = Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo's version is semver");
    let updater = Updater::new(
        Arc::new(TauriBackend::new(app.clone(), current.clone())),
        Arc::new(StateHost {
            state: state.clone(),
            app: Some(app.clone()),
        }),
        Arc::new(TauriPublisher(app.clone())),
        current,
    );
    let switch = Arc::new(AutoCheckSwitch::default());

    app.manage(updater.clone());
    app.manage(Arc::clone(&switch));

    tauri::async_runtime::spawn(auto_checks(updater.clone(), state.clone(), switch));
    updater
}

/// One check shortly after launch and one a day after that, each only if the
/// setting is on when the time comes. Looking is all it does.
async fn auto_checks(updater: Updater, state: AppState, switch: Arc<AutoCheckSwitch>) {
    tokio::time::sleep(FIRST_CHECK_DELAY).await;
    loop {
        if switch.is_stopped() {
            return;
        }
        let wanted = state
            .store()
            .settings()
            .map(|settings| settings.auto_check_updates)
            .unwrap_or(true);
        // Not while an install is under way. A check needs no gate — it moves
        // nothing — so a move in progress is no reason to skip a day.
        if wanted && !state.installing() {
            updater.check(false).await;
        }
        // Until tomorrow, or until the setting is changed, whichever is first.
        let _ = tokio::time::timeout(CHECK_EVERY, switch.wake.notified()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_or_unreadable_signature_is_never_anything_but_a_bad_signature() {
        for error in [
            PluginError::SignatureUtf8("not base64".into()),
            PluginError::MissingSignedVersion,
            PluginError::SignedVersionMismatch {
                signed: "0.1.0".into(),
                announced: "9.9.9".into(),
            },
        ] {
            assert_eq!(classify(&error), UpdateError::BadSignature, "{error}");
        }
    }

    #[test]
    fn a_signature_failure_says_nothing_was_installed() {
        let message = classify(&PluginError::MissingSignedVersion).message();
        assert!(message.contains("signature"));
        assert!(message.contains("Nothing was installed"));
    }

    #[test]
    fn metadata_the_plugin_cannot_read_is_a_bad_manifest() {
        let broken = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        assert_eq!(
            classify(&PluginError::Serialization(broken)),
            UpdateError::BadManifest
        );
        let version = semver::Version::parse("not-a-version").unwrap_err();
        assert_eq!(
            classify(&PluginError::Semver(version)),
            UpdateError::BadManifest
        );
    }

    #[test]
    fn a_manifest_with_nothing_for_this_machine_says_so() {
        assert_eq!(
            classify(&PluginError::TargetNotFound("darwin-aarch64".into())),
            UpdateError::NoPlatform
        );
        assert_eq!(
            classify(&PluginError::TargetsNotFound(vec![])),
            UpdateError::NoPlatform
        );
    }

    #[test]
    fn a_dropped_download_is_interrupted_not_a_verification_failure() {
        assert_eq!(
            classify(&PluginError::Network("connection reset".into())),
            UpdateError::Interrupted
        );
    }

    #[test]
    fn the_channel_is_filled_into_every_endpoint() {
        let config = serde_json::json!({
            "endpoints": ["https://example.test/{{channel}}.json"],
        });
        let urls = endpoints(&config, "alpha").unwrap();
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].as_str(), "https://example.test/alpha.json");
        let urls = endpoints(&config, "stable").unwrap();
        assert_eq!(urls[0].as_str(), "https://example.test/stable.json");
    }

    #[test]
    fn no_endpoint_at_all_means_updates_are_unavailable() {
        for config in [
            serde_json::json!({}),
            serde_json::json!({ "endpoints": [] }),
            serde_json::json!({ "endpoints": ["not a url"] }),
        ] {
            assert!(matches!(
                endpoints(&config, "stable"),
                Err(UpdateError::Unavailable(_))
            ));
        }
    }
}
