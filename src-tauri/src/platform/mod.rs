//! The platform layer.
//!
//! `cfg(target_os)` appears in this module and essentially nowhere else.
//! Detectors ask a [`PlatformService`] what is installed, where screenshots
//! land and which caches have known owners; they never ask which OS they are
//! running on.

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod apps;
pub mod caches;
pub mod games;
pub mod testing;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub use apps::{AppIndex, AppSource, InstalledApp};
pub use caches::{CacheRule, CacheSafety};
pub use games::{GameInstall, GameLibrary, LibraryKind};

/// A directory Scuttle knows something about before it looks inside.
#[derive(Debug, Clone)]
pub struct KnownLocation {
    pub path: PathBuf,
    /// What a person would call this place.
    pub label: String,
    pub role: LocationRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationRole {
    Downloads,
    Screenshots,
    ApplicationSupport,
    Caches,
    Logs,
    Documents,
    Desktop,
    GameData,
    DeveloperCache,
}

/// Everything platform-specific the rest of Scuttle is allowed to know.
pub trait PlatformService: Send + Sync {
    fn name(&self) -> &'static str;

    /// Where Scuttle looks when the user has not said otherwise. Chosen to be
    /// useful without being invasive: no blanket scan of the home directory.
    fn default_scan_roots(&self) -> Vec<KnownLocation>;

    /// Places the OS or its screenshot tool writes captures to.
    fn screenshot_locations(&self) -> Vec<PathBuf>;

    /// Directories where applications keep their own data.
    ///
    /// Each *immediate child* is expected to belong to one application, which
    /// is what makes them candidates for ghost detection.
    fn application_data_roots(&self) -> Vec<PathBuf>;

    /// Directories whose contents an application maintains for itself.
    ///
    /// A superset of [`PlatformService::application_data_roots`]. Some places
    /// are wholly owned by tooling without giving each application a folder of
    /// its own — `~/Library/Developer` is Xcode's, top to bottom — so they
    /// belong here but not above: their children are not ghost candidates,
    /// and nothing inside them is a duplicate the user can act on.
    fn application_managed_roots(&self) -> Vec<PathBuf> {
        self.application_data_roots()
    }

    /// Applications this machine actually has.
    fn installed_apps(&self) -> Vec<InstalledApp>;

    /// Caches Scuttle has a specific, owner-attributed rule for.
    fn cache_rules(&self) -> Vec<CacheRule>;

    /// Game libraries and the titles currently installed in them.
    fn game_libraries(&self) -> Vec<GameLibrary>;

    /// Lowercased names of currently running processes. Best effort: an empty
    /// list means "could not tell", never "nothing is running".
    fn running_processes(&self) -> Vec<String>;

    /// Show the file to the user in their file manager.
    fn reveal(&self, path: &Path) -> crate::Result<()>;

    /// Where quarantined items are held. Must be on the same volume as the
    /// user's home directory so that quarantining is usually a rename.
    fn quarantine_root(&self) -> PathBuf;

    /// Where Scuttle's own database lives.
    fn data_dir(&self) -> PathBuf;

    /// Installer file extensions that mean something on this platform.
    fn installer_extensions(&self) -> &'static [&'static str];
}

/// Build the service for whatever this is running on.
pub fn current() -> Arc<dyn PlatformService> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacPlatformService::new())
    }
    #[cfg(target_os = "windows")]
    {
        Arc::new(windows::WindowsPlatformService::new())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Arc::new(unsupported::UnsupportedPlatformService::new())
    }
}

/// Shared helpers the concrete services use. Keeping them here stops the two
/// implementations from drifting apart.
pub(crate) mod util {
    use std::path::PathBuf;
    use std::process::Command;

    pub fn home() -> PathBuf {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    }

    /// Run a command purely to read its stdout. Returns an empty string on any
    /// failure — callers treat "could not tell" as "no information", never as
    /// "definitely nothing".
    pub fn capture(program: &str, args: &[&str]) -> String {
        let mut command = Command::new(program);
        command.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        match command.output() {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
            _ => String::new(),
        }
    }

    #[allow(dead_code)]
    pub fn exists(path: &std::path::Path) -> bool {
        path.symlink_metadata().is_ok()
    }
}

/// A stub so the crate still compiles (and its tests still run) on platforms
/// Scuttle does not ship for yet. It reports no knowledge of anything, which
/// makes every detector conservative rather than wrong.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod unsupported {
    use super::*;

    pub struct UnsupportedPlatformService {
        home: PathBuf,
    }

    impl UnsupportedPlatformService {
        pub fn new() -> Self {
            Self { home: util::home() }
        }
    }

    impl PlatformService for UnsupportedPlatformService {
        fn name(&self) -> &'static str {
            "unsupported"
        }
        fn default_scan_roots(&self) -> Vec<KnownLocation> {
            vec![KnownLocation {
                path: self.home.join("Downloads"),
                label: "Downloads".into(),
                role: LocationRole::Downloads,
            }]
        }
        fn screenshot_locations(&self) -> Vec<PathBuf> {
            vec![self.home.join("Pictures")]
        }
        fn application_data_roots(&self) -> Vec<PathBuf> {
            vec![self.home.join(".local/share"), self.home.join(".cache")]
        }
        fn installed_apps(&self) -> Vec<InstalledApp> {
            Vec::new()
        }
        fn cache_rules(&self) -> Vec<CacheRule> {
            Vec::new()
        }
        fn game_libraries(&self) -> Vec<GameLibrary> {
            Vec::new()
        }
        fn running_processes(&self) -> Vec<String> {
            Vec::new()
        }
        fn reveal(&self, _path: &Path) -> crate::Result<()> {
            Err(crate::ScuttleError::Refused(
                "Revealing files is not supported on this platform yet.".into(),
            ))
        }
        fn quarantine_root(&self) -> PathBuf {
            self.home.join(".local/share/scuttle/quarantine")
        }
        fn data_dir(&self) -> PathBuf {
            self.home.join(".local/share/scuttle")
        }
        fn installer_extensions(&self) -> &'static [&'static str] {
            &["appimage", "deb", "rpm"]
        }
    }
}
