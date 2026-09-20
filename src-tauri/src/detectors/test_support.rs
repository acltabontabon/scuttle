//! A real filesystem, a real walk, a real safety net.
//!
//! Detector tests run through the whole scanning pipeline against a temporary
//! home directory rather than calling `observe` directly. That way a detector
//! test also proves the walker reached the file and the safety net let it
//! through — the three places a detector can be wrong.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::model::CleanupCandidate;
use crate::platform::apps::{AppSource, InstalledApp};
use crate::platform::caches::CacheRule;
use crate::platform::games::GameLibrary;
use crate::platform::testing::FixedPlatform;
use crate::safety::ProtectedPaths;
use crate::scanning::{Detector, IgnoreSet, ScanContext, ScanOptions, SilentObserver};

/// A file to conjure into existence before the scan.
pub struct FixtureFile {
    pub rel: String,
    pub days_old: i64,
    pub size: u64,
    pub contents: Option<Vec<u8>>,
}

/// A file of `size` bytes, last modified `days_old` days ago.
pub fn fixture_entry(rel: &str, days_old: i64, size: u64) -> FixtureFile {
    FixtureFile {
        rel: rel.to_string(),
        days_old,
        size,
        contents: None,
    }
}

/// A file with exact contents, for duplicate and image tests.
pub fn fixture_file(rel: &str, days_old: i64, contents: Vec<u8>) -> FixtureFile {
    FixtureFile {
        rel: rel.to_string(),
        days_old,
        size: contents.len() as u64,
        contents: Some(contents),
    }
}

pub struct Harness {
    tmp: tempfile::TempDir,
    pub home: PathBuf,
    pub apps: Vec<InstalledApp>,
    pub caches: Vec<CacheRule>,
    pub libraries: Vec<GameLibrary>,
    pub processes: Vec<String>,
    pub options: ScanOptions,
    pub ignores: IgnoreSet,
}

impl Harness {
    pub fn new() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        // A `home` component keeps the fixture clear of any rule that keys off
        // the real user's home directory.
        let home = tmp.path().join("home/rummager");
        std::fs::create_dir_all(&home).unwrap();
        Harness {
            tmp,
            home,
            apps: Vec::new(),
            caches: Vec::new(),
            libraries: Vec::new(),
            processes: Vec::new(),
            options: ScanOptions::default(),
            ignores: IgnoreSet::default(),
        }
    }

    /// `(display name, bundle id)` pairs for the applications this machine
    /// should appear to have.
    pub fn with_apps(apps: &[(&str, Option<&str>)]) -> Harness {
        let mut h = Harness::new();
        h.apps = apps
            .iter()
            .map(|(name, bundle)| InstalledApp {
                name: (*name).to_string(),
                bundle_id: bundle.map(str::to_string),
                publisher: None,
                install_location: None,
                source: AppSource::Bundle,
            })
            .collect();
        h
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.home.join(rel)
    }

    pub fn scratch(&self) -> &Path {
        self.tmp.path()
    }

    /// Create the fixture files on disk, with their stated ages.
    pub fn materialise(&self, files: &[FixtureFile]) {
        for file in files {
            let path = self.home.join(&file.rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            match &file.contents {
                Some(bytes) => std::fs::write(&path, bytes).unwrap(),
                None => std::fs::write(&path, vec![b'\0'; file.size as usize]).unwrap(),
            }
            set_age(&path, file.days_old);
        }
    }

    pub fn context(&self) -> ScanContext {
        let platform = Arc::new(FixedPlatform {
            home: self.home.clone(),
            apps: self.apps.clone(),
            caches: self.caches.clone(),
            libraries: self.libraries.clone(),
            processes: self.processes.clone(),
            ..FixedPlatform::new(self.home.clone())
        });
        let mut options = self.options.clone();
        if options.roots.is_empty() {
            options.roots = vec![self.home.clone()];
        }
        let mut ctx = ScanContext::new(
            options,
            platform,
            self.ignores.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        let mut protected = ProtectedPaths::for_home(&self.home);
        protected.also_protect("Scuttle's own files", self.home.join(".scuttle"));
        ctx.protected = Arc::new(protected);
        ctx
    }

    /// Materialise the files, run one detector through the full pipeline and
    /// return what survived.
    pub fn run<D: Detector + 'static>(
        &self,
        detector: D,
        files: Vec<FixtureFile>,
    ) -> Vec<CleanupCandidate> {
        self.materialise(&files);
        let ctx = self.context();
        crate::scanning::run("test", &ctx, vec![Box::new(detector)], &SilentObserver).candidates
    }

    /// Run without creating anything first, for probe-based detectors whose
    /// fixtures were built by hand.
    pub fn run_bare<D: Detector + 'static>(&self, detector: D) -> Vec<CleanupCandidate> {
        let ctx = self.context();
        crate::scanning::run("test", &ctx, vec![Box::new(detector)], &SilentObserver).candidates
    }
}

impl Default for Harness {
    fn default() -> Self {
        Harness::new()
    }
}

/// Backdate a file so age-based evidence has something to work with.
pub fn set_age(path: &Path, days: i64) {
    let when = std::time::SystemTime::now()
        - std::time::Duration::from_secs((days.max(0) * 86_400) as u64);
    let times = std::fs::FileTimes::new()
        .set_accessed(when)
        .set_modified(when);
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_times(times).unwrap();
}

/// Assert there is exactly one candidate in an owned result and hand it over.
pub fn only(candidates: Vec<CleanupCandidate>) -> CleanupCandidate {
    assert_eq!(
        candidates.len(),
        1,
        "expected exactly one finding, got {:#?}",
        candidates
            .iter()
            .map(|c| (&c.display_name, c.category))
            .collect::<Vec<_>>()
    );
    candidates.into_iter().next().unwrap()
}

/// Assert there is exactly one candidate and hand it over.
pub fn one(candidates: &[CleanupCandidate]) -> &CleanupCandidate {
    assert_eq!(
        candidates.len(),
        1,
        "expected exactly one finding, got {:#?}",
        candidates
            .iter()
            .map(|c| (&c.display_name, c.category))
            .collect::<Vec<_>>()
    );
    &candidates[0]
}

/// Find the one candidate whose display name matches.
pub fn named<'a>(candidates: &'a [CleanupCandidate], name: &str) -> &'a CleanupCandidate {
    candidates
        .iter()
        .find(|c| c.display_name == name)
        .unwrap_or_else(|| {
            panic!(
                "no finding named {name}; got {:?}",
                candidates
                    .iter()
                    .map(|c| &c.display_name)
                    .collect::<Vec<_>>()
            )
        })
}
