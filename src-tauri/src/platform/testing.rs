//! A platform that knows exactly what you tell it to know.
//!
//! This is part of the public API on purpose: writing a detector means writing
//! tests for it, and those tests must not depend on which machine the suite
//! runs on or what happens to be installed there. See
//! `docs/writing-a-detector.md`.
//!
//! ```no_run
//! use scuttle_core::platform::testing::FixedPlatform;
//!
//! let platform = FixedPlatform::new("/tmp/fake-home")
//!     .with_app("Figma", Some("com.figma.Desktop"))
//!     .with_process("slack");
//! ```

use std::path::{Path, PathBuf};

use super::apps::{AppSource, InstalledApp};
use super::caches::CacheRule;
use super::games::GameLibrary;
use super::{KnownLocation, LocationRole, PlatformService};

#[derive(Clone)]
pub struct FixedPlatform {
    pub home: PathBuf,
    pub apps: Vec<InstalledApp>,
    pub caches: Vec<CacheRule>,
    pub libraries: Vec<GameLibrary>,
    pub processes: Vec<String>,
    pub installer_extensions: &'static [&'static str],
}

impl FixedPlatform {
    pub fn new(home: impl Into<PathBuf>) -> FixedPlatform {
        FixedPlatform {
            home: home.into(),
            apps: Vec::new(),
            caches: Vec::new(),
            libraries: Vec::new(),
            processes: Vec::new(),
            // Both platforms' formats, so detector tests behave the same way
            // wherever the suite runs.
            installer_extensions: &["dmg", "pkg", "exe", "msi", "msix", "iso"],
        }
    }

    pub fn with_app(mut self, name: &str, bundle_id: Option<&str>) -> Self {
        self.apps.push(InstalledApp {
            name: name.to_string(),
            bundle_id: bundle_id.map(str::to_string),
            publisher: None,
            install_location: None,
            source: AppSource::Bundle,
        });
        self
    }

    pub fn with_library(mut self, library: GameLibrary) -> Self {
        self.libraries.push(library);
        self
    }

    pub fn with_cache_rule(mut self, rule: CacheRule) -> Self {
        self.caches.push(rule);
        self
    }

    /// Add a running process. Note that leaving the process list empty means
    /// "could not tell", not "nothing is running" — detectors are required to
    /// treat those differently.
    pub fn with_process(mut self, name: &str) -> Self {
        self.processes.push(name.to_lowercase());
        self
    }
}

impl PlatformService for FixedPlatform {
    fn name(&self) -> &'static str {
        "fixed"
    }

    fn default_scan_roots(&self) -> Vec<KnownLocation> {
        vec![KnownLocation {
            path: self.home.clone(),
            label: "Home".into(),
            role: LocationRole::Downloads,
        }]
    }

    fn screenshot_locations(&self) -> Vec<PathBuf> {
        vec![
            self.home.join("Desktop"),
            self.home.join("Pictures"),
            self.home.join("Pictures/Screenshots"),
        ]
    }

    fn application_data_roots(&self) -> Vec<PathBuf> {
        [
            self.home.join("AppData"),
            self.home.join("Library/Application Support"),
        ]
        .into_iter()
        .filter(|p| p.is_dir())
        .collect()
    }

    fn installed_apps(&self) -> Vec<InstalledApp> {
        self.apps.clone()
    }

    fn cache_rules(&self) -> Vec<CacheRule> {
        self.caches.clone()
    }

    fn game_libraries(&self) -> Vec<GameLibrary> {
        self.libraries.clone()
    }

    fn running_processes(&self) -> Vec<String> {
        self.processes.clone()
    }

    fn reveal(&self, _path: &Path) -> crate::Result<()> {
        Ok(())
    }

    fn quarantine_root(&self) -> PathBuf {
        self.home.join(".scuttle/quarantine")
    }

    fn data_dir(&self) -> PathBuf {
        self.home.join(".scuttle")
    }

    fn installer_extensions(&self) -> &'static [&'static str] {
        self.installer_extensions
    }
}
