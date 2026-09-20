//! Scanning: the rummage itself.
//!
//! Shape of a scan:
//!
//! ```text
//! prepare   read installed apps, game libraries, running processes once
//!    |
//! walk      one streaming pass per root, every entry offered to every
//!    |      stream detector (detectors never walk the tree themselves)
//!    |
//! probe     detectors that need targeted lookups do them here
//!    |
//! finish    deferred expensive work (hashing, grouping) and emission
//!    |
//! guard     every candidate re-checked against the safety layer before it
//!           is allowed to exist
//! ```
//!
//! Detectors never delete anything and never decide their own confidence.

pub mod contents;
pub mod walk;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::Hiccup;
use crate::evidence::{self, ev, Evidence, EvidenceKind};
use crate::model::{
    Category, CleanupCandidate, GroupMember, RecommendedAction, Risk, StateFingerprint, TargetKind,
};
use crate::platform::{AppIndex, GameLibrary, PlatformService};
use crate::safety::{paths, ProtectedPaths};

pub use contents::{ContentProfile, DirectoryIndex};
pub use walk::{FileEntry, Step, WalkOptions};

/// What the user asked for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanOptions {
    /// Absolute directories to rummage through.
    pub roots: Vec<PathBuf>,
    /// Build output and package caches are off by default: they belong to
    /// work in progress more often than not.
    pub include_developer_debris: bool,
    /// Files at or above this size are worth mentioning on their own.
    pub heavy_threshold: u64,
    pub max_depth: usize,
    /// Duplicate detection reads file contents; below this size the saving is
    /// not worth the I/O.
    pub duplicate_min_size: u64,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            roots: Vec::new(),
            include_developer_debris: false,
            heavy_threshold: 1024 * 1024 * 1024, // 1 GB
            max_depth: 12,
            duplicate_min_size: 1024 * 1024, // 1 MB
        }
    }
}

/// Decisions the user has already made, so Scuttle stops asking.
#[derive(Debug, Clone, Default)]
pub struct IgnoreSet {
    pub paths: Vec<PathBuf>,
    /// Lowercased application names.
    pub apps: Vec<String>,
    pub categories: Vec<Category>,
}

impl IgnoreSet {
    pub fn covers(&self, path: &std::path::Path, app: Option<&str>, category: Category) -> bool {
        if self.categories.contains(&category) {
            return true;
        }
        if self.paths.iter().any(|p| paths::is_within(path, p)) {
            return true;
        }
        match app {
            Some(app) => {
                let lowered = app.to_lowercase();
                self.apps.contains(&lowered)
            }
            None => false,
        }
    }
}

/// Everything a detector is allowed to know, gathered once.
pub struct ScanContext {
    pub options: ScanOptions,
    pub protected: Arc<ProtectedPaths>,
    pub platform: Arc<dyn PlatformService>,
    pub apps: AppIndex,
    pub libraries: Vec<GameLibrary>,
    /// Lowercased names of processes running when the scan started.
    pub processes: Vec<String>,
    pub ignores: IgnoreSet,
    /// Directories an application maintains on the user's behalf.
    application_areas: Vec<PathBuf>,
    /// Directories a cache rule claims, whether or not that rule is enabled
    /// for this scan.
    cache_areas: Vec<PathBuf>,
    pub now_unix: i64,
    cancel: Arc<AtomicBool>,
    /// Directory profiles gathered during the shared traversal. Set once, by
    /// the scanner, between the walk and the probe phase.
    directories: std::sync::OnceLock<DirectoryIndex>,
}

impl ScanContext {
    pub fn new(
        options: ScanOptions,
        platform: Arc<dyn PlatformService>,
        ignores: IgnoreSet,
        cancel: Arc<AtomicBool>,
    ) -> ScanContext {
        let mut protected_paths = ProtectedPaths::for_current_user();
        // Scuttle's own database and quarantine drawer live inside a
        // directory Scuttle scans. Finding its own held items and offering to
        // quarantine them again would be absurd.
        protected_paths.also_protect("Scuttle's own files", platform.data_dir());
        let protected = Arc::new(protected_paths);
        let apps = AppIndex::new(platform.installed_apps());
        let libraries = platform.game_libraries();
        let processes = platform.running_processes();
        let application_areas = platform.application_managed_roots();
        let cache_areas = platform
            .cache_rules()
            .into_iter()
            .map(|rule| rule.path)
            .collect();
        ScanContext {
            options,
            protected,
            platform,
            apps,
            libraries,
            processes,
            ignores,
            application_areas,
            cache_areas,
            now_unix: chrono::Utc::now().timestamp(),
            cancel,
            directories: std::sync::OnceLock::new(),
        }
    }

    /// Hand the scanner's directory index to the detectors. Called once,
    /// after the walk; later calls are ignored.
    pub(crate) fn publish_directories(&self, index: DirectoryIndex) {
        let _ = self.directories.set(index);
    }

    /// What a directory holds.
    ///
    /// Returns the profile built during the shared traversal when there is
    /// one, and walks the directory only when there is not — for paths
    /// outside the scanned roots, or deeper than the index goes. Detectors
    /// should always ask through here rather than walking themselves: on a
    /// real machine the difference was the whole scan again.
    pub fn profile_of(&self, path: &std::path::Path) -> ContentProfile {
        if let Some(index) = self.directories.get() {
            if let Some(profile) = index.get(path) {
                return profile.clone();
            }
        }
        contents::profile(path, &self.protected, &|| self.cancelled())
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Ask the scan using this context to stop at its next checkpoint.
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Is this path inside a directory an application maintains for itself?
    ///
    /// Two copies of the same JAR in two versions of an IDE's plugin folder
    /// are duplicates in the strict sense and useless as a finding: the user
    /// cannot remove either without breaking the thing that put them there.
    /// Detectors about *the user's own* files should skip these.
    pub fn is_application_managed(&self, path: &std::path::Path) -> bool {
        self.application_areas
            .iter()
            .any(|root| paths::is_strictly_within(path, root))
    }

    /// Does a cache rule claim this path?
    ///
    /// Checked regardless of whether the rule is enabled for this scan.
    /// Otherwise turning off developer debris would hide the Go build cache
    /// from the cache detector and let the ghost detector call it an orphaned
    /// application — a worse answer, delivered more confidently.
    pub fn cache_rule_owns(&self, path: &std::path::Path) -> bool {
        self.cache_areas
            .iter()
            .any(|root| paths::is_within(path, root))
    }

    /// Is a process by this name running? `None` means "could not tell",
    /// which detectors must treat as "possibly yes".
    pub fn process_running(&self, name: &str) -> Option<bool> {
        if self.processes.is_empty() {
            return None;
        }
        let needle = name.to_lowercase();
        Some(self.processes.iter().any(|p| p.contains(&needle)))
    }
}

/// What a detector produces. Note what is *absent*: confidence, risk level and
/// recommended action. Those are computed centrally from the evidence.
pub struct Finding {
    pub detector: &'static str,
    pub category: Category,
    pub target_kind: TargetKind,
    pub path: PathBuf,
    pub display_name: String,
    pub associated_app: Option<String>,
    pub size: u64,
    /// The detector's starting position on how much it would cost to be wrong.
    pub base_risk: Risk,
    pub evidence: Vec<Evidence>,
    /// One restrained line, when the detector has something worth saying.
    pub remark: Option<String>,
    pub group: Vec<GroupMember>,
    /// Set by detectors that positively identified what the file *is*. The
    /// safety net then skips adding generic "this might be yours" evidence,
    /// because the detector already knows, and more precisely.
    pub self_classified: bool,
    pub modified_unix: Option<i64>,
    pub accessed_unix: Option<i64>,
    pub created_unix: Option<i64>,
}

impl Finding {
    pub fn new(
        detector: &'static str,
        category: Category,
        entry: &FileEntry,
        display_name: impl Into<String>,
    ) -> Finding {
        Finding {
            detector,
            category,
            target_kind: if entry.is_dir {
                TargetKind::Directory
            } else {
                TargetKind::File
            },
            path: entry.path.clone(),
            display_name: display_name.into(),
            associated_app: None,
            size: entry.size,
            base_risk: Risk::Moderate,
            evidence: Vec::new(),
            remark: None,
            group: Vec::new(),
            self_classified: false,
            modified_unix: entry.modified_unix,
            accessed_unix: entry.accessed_unix,
            created_unix: entry.created_unix,
        }
    }

    pub fn with(mut self, kind: EvidenceKind) -> Finding {
        self.evidence.push(ev(kind));
        self
    }

    pub fn risk(mut self, risk: Risk) -> Finding {
        self.base_risk = risk;
        self
    }

    pub fn size(mut self, size: u64) -> Finding {
        self.size = size;
        self
    }

    pub fn app(mut self, app: impl Into<String>) -> Finding {
        self.associated_app = Some(app.into());
        self
    }

    pub fn saying(mut self, remark: impl Into<String>) -> Finding {
        self.remark = Some(remark.into());
        self
    }

    /// Declare that this detector knows what the file is, so the generic
    /// user-content signal is not added on top of its own classification.
    pub fn classified(mut self) -> Finding {
        self.self_classified = true;
        self
    }
}

/// Where detectors hand their findings.
pub trait CandidateSink {
    fn emit(&mut self, finding: Finding);
}

/// A detector observes; it never acts.
///
/// Most detectors only implement [`Detector::observe`] and
/// [`Detector::finish`], consuming the shared traversal rather than walking the
/// disk themselves. [`Detector::probe`] exists for the ones that need to look
/// somewhere specific (known cache locations, game libraries).
pub trait Detector: Send {
    fn id(&self) -> &'static str;
    fn category(&self) -> Category;

    /// What Scuttle says it is doing while this detector works. Shown to the
    /// user, so it must describe real work.
    fn rummaging_note(&self) -> &'static str;

    fn enabled(&self, _ctx: &ScanContext) -> bool {
        true
    }

    /// Called for every entry in the shared walk. Must be cheap: no I/O beyond
    /// the metadata already gathered.
    fn observe(&mut self, _entry: &FileEntry, _ctx: &ScanContext) {}

    /// Targeted lookups. Must check [`ScanContext::cancelled`] in any loop.
    fn probe(&mut self, _ctx: &ScanContext, _sink: &mut dyn CandidateSink) {}

    /// Deferred expensive work (hashing, grouping) and emission.
    fn finish(&mut self, _ctx: &ScanContext, _sink: &mut dyn CandidateSink) {}
}

/// How often progress is reported while walking.
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(120);

/// Where the scan currently is. The note attached to each phase describes work
/// that is genuinely happening — Scuttle does not narrate fiction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    Preparing,
    Rummaging { area: String },
    Examining { note: String },
    Considering { note: String },
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub files_seen: u64,
    pub bytes_seen: u64,
    pub candidates_found: u64,
    pub current_area: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSummary {
    pub scan_id: String,
    pub files_seen: u64,
    pub bytes_seen: u64,
    pub candidates_found: u64,
    pub reclaimable_bytes: u64,
    pub duration_ms: u64,
    /// Where the time went. Guessing at this is how you optimise the wrong
    /// thing, so the scan measures itself.
    pub walk_ms: u64,
    pub probe_ms: u64,
    pub finish_ms: u64,
    pub cancelled: bool,
    pub hiccups: HiccupSummary,
}

/// Counts rather than paths: logging every unreadable path would leak the
/// shape of someone's home directory into a log file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HiccupSummary {
    pub permission_denied: u64,
    pub vanished: u64,
    pub unreadable: u64,
    pub loops_avoided: u64,
}

impl HiccupSummary {
    pub fn record(&mut self, hiccup: &Hiccup) {
        use crate::error::HiccupKind::*;
        match hiccup.kind {
            PermissionDenied => self.permission_denied += 1,
            Vanished => self.vanished += 1,
            Unreadable => self.unreadable += 1,
            LoopAvoided => self.loops_avoided += 1,
            TooDeep => {}
        }
    }

    pub fn total(&self) -> u64 {
        self.permission_denied + self.vanished + self.unreadable + self.loops_avoided
    }
}

/// How the scan reports itself. The Tauri layer forwards these as events; the
/// tests collect them in a vector.
pub trait ScanObserver: Send + Sync {
    fn phase(&self, phase: Phase);
    fn progress(&self, progress: &Progress);
    fn candidate(&self, candidate: &CleanupCandidate);
    fn finished(&self, summary: &ScanSummary);
}

/// An observer that does nothing, for the dry-run report and tests.
pub struct SilentObserver;

/// The same thing under the name the dry run uses.
pub use SilentObserver as CleanupObserver;

impl ScanObserver for SilentObserver {
    fn phase(&self, _phase: Phase) {}
    fn progress(&self, _progress: &Progress) {}
    fn candidate(&self, _candidate: &CleanupCandidate) {}
    fn finished(&self, _summary: &ScanSummary) {}
}

/// The safety net between a detector and the outside world.
///
/// Even if a detector is wrong, buggy or freshly written by someone who has
/// not read the safety docs, nothing protected can get past this.
struct GuardedSink<'a> {
    ctx: &'a ScanContext,
    observer: &'a dyn ScanObserver,
    scan_id: &'a str,
    candidates: Vec<CleanupCandidate>,
    seen: HashMap<PathBuf, usize>,
    rejected_protected: u64,
}

impl<'a> GuardedSink<'a> {
    fn new(ctx: &'a ScanContext, observer: &'a dyn ScanObserver, scan_id: &'a str) -> Self {
        GuardedSink {
            ctx,
            observer,
            scan_id,
            candidates: Vec::new(),
            seen: HashMap::new(),
            rejected_protected: 0,
        }
    }
}

impl CandidateSink for GuardedSink<'_> {
    fn emit(&mut self, mut finding: Finding) {
        let path = paths::normalize(&finding.path);

        // 1. Nothing protected or structural ever becomes a finding, whatever
        //    the detector believes.
        if self.ctx.protected.is_protected(&path) || self.ctx.protected.is_too_shallow(&path) {
            self.rejected_protected += 1;
            return;
        }

        // 2. Decisions the user already made.
        if self
            .ctx
            .ignores
            .covers(&path, finding.associated_app.as_deref(), finding.category)
        {
            return;
        }

        // 3. Anything in the user's own territory carries that as evidence,
        //    which raises risk through the normal arithmetic rather than
        //    through a special case.
        if let Some(reason) = self.ctx.protected.sensitivity(&path) {
            let already = finding.self_classified
                || finding
                    .evidence
                    .iter()
                    .any(|e| matches!(e.kind, EvidenceKind::UserContentDetected { .. }));
            if !already {
                finding.evidence.push(ev(EvidenceKind::UserContentDetected {
                    reason: reason.into(),
                }));
            }
        }

        // 4. The arithmetic. Detectors do not get a vote here.
        let verdict = evidence::weigh(&finding.evidence, finding.base_risk);

        // 5. A finding with nothing to say is noise, not a finding.
        if finding.evidence.is_empty() {
            return;
        }

        let fingerprint = crate::safety::observe(&path).unwrap_or(StateFingerprint {
            size: finding.size,
            modified_unix: finding.modified_unix,
            is_dir: finding.target_kind == TargetKind::Directory,
            child_count: None,
        });

        let candidate = CleanupCandidate {
            id: format!("{}-{}", self.scan_id, self.candidates.len()),
            detector: finding.detector.to_string(),
            category: finding.category,
            target_kind: finding.target_kind,
            path,
            display_name: finding.display_name,
            associated_app: finding.associated_app,
            size: finding.size,
            confidence: verdict.confidence,
            risk: verdict.risk,
            recommended_action: verdict.action,
            evidence: finding.evidence,
            remark: finding.remark,
            modified_unix: finding.modified_unix,
            accessed_unix: finding.accessed_unix,
            created_unix: finding.created_unix,
            group_bytes: crate::model::group_footprint(&finding.group, finding.size),
            group: finding.group,
            fingerprint,
        };

        // 6. One object, one pile. When two detectors claim the same path the
        //    better-evidenced account wins, so a file is not shown twice.
        if let Some(&existing) = self.seen.get(&candidate.path) {
            let incumbent = &self.candidates[existing];
            let better = candidate.evidence.len() > incumbent.evidence.len()
                || (candidate.evidence.len() == incumbent.evidence.len()
                    && candidate.confidence > incumbent.confidence);
            if better {
                self.candidates[existing] = candidate;
            }
            return;
        }

        self.seen
            .insert(candidate.path.clone(), self.candidates.len());
        self.observer.candidate(&candidate);
        self.candidates.push(candidate);
    }
}

/// Everything a finished scan produced.
pub struct ScanOutcome {
    pub summary: ScanSummary,
    pub candidates: Vec<CleanupCandidate>,
}

/// Run a scan to completion, or until cancelled.
///
/// This is synchronous by design: it is called on a blocking worker, and the
/// observer is what makes results incremental. Making it `async` would add
/// ceremony without making any of the underlying filesystem calls yield.
pub fn run(
    scan_id: &str,
    ctx: &ScanContext,
    mut detectors: Vec<Box<dyn Detector>>,
    observer: &dyn ScanObserver,
) -> ScanOutcome {
    let started = std::time::Instant::now();
    let mut hiccups = HiccupSummary::default();
    let mut progress = Progress {
        files_seen: 0,
        bytes_seen: 0,
        candidates_found: 0,
        current_area: String::new(),
    };

    observer.phase(Phase::Preparing);
    detectors.retain(|d| d.enabled(ctx));

    let mut sink = GuardedSink::new(ctx, observer, scan_id);
    let mut cancelled = false;
    let mut directories = DirectoryIndex::with_walk_depth(ctx.options.max_depth);
    let walk_started = std::time::Instant::now();

    // ---- one streaming pass per root -------------------------------------
    'roots: for root in &ctx.options.roots {
        if ctx.cancelled() {
            cancelled = true;
            break;
        }
        let area = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        progress.current_area = area.clone();
        observer.phase(Phase::Rummaging { area: area.clone() });

        let mut last_report = std::time::Instant::now();
        walk::walk(
            root,
            &ctx.protected,
            &WalkOptions {
                max_depth: ctx.options.max_depth,
                report_dirs: true,
            },
            |entry| {
                progress.files_seen += 1;
                progress.bytes_seen += entry.size;

                directories.observe(&entry);
                for detector in detectors.iter_mut() {
                    detector.observe(&entry, ctx);
                }

                // Time-based rather than count-based: a progress line that
                // updates fifty times a second is not more informative than
                // one that updates eight times a second, and every update
                // re-renders the window.
                if last_report.elapsed() >= PROGRESS_INTERVAL {
                    last_report = std::time::Instant::now();
                    observer.progress(&progress);
                    if ctx.cancelled() {
                        return Step::Stop;
                    }
                }
                Step::Continue
            },
            |hiccup| hiccups.record(&hiccup),
        );

        observer.progress(&progress);
        if ctx.cancelled() {
            cancelled = true;
            break 'roots;
        }
    }

    let walk_ms = walk_started.elapsed().as_millis() as u64;

    // Everything the walk learned about directories, handed to the detectors
    // so none of them has to walk the same tree a second time.
    tracing::debug!(directories = directories.len(), "directory index built");
    ctx.publish_directories(directories);

    // ---- targeted probes --------------------------------------------------
    let probe_started = std::time::Instant::now();
    if !cancelled {
        for detector in detectors.iter_mut() {
            if ctx.cancelled() {
                cancelled = true;
                break;
            }
            observer.phase(Phase::Examining {
                note: detector.rummaging_note().to_string(),
            });
            detector.probe(ctx, &mut sink);
        }
    }

    let probe_ms = probe_started.elapsed().as_millis() as u64;

    // ---- deferred work and emission ---------------------------------------
    let finish_started = std::time::Instant::now();
    if !cancelled {
        for detector in detectors.iter_mut() {
            if ctx.cancelled() {
                cancelled = true;
                break;
            }
            observer.phase(Phase::Considering {
                note: detector.rummaging_note().to_string(),
            });
            detector.finish(ctx, &mut sink);
        }
    }

    let finish_ms = finish_started.elapsed().as_millis() as u64;
    let candidates = sink.candidates;
    progress.candidates_found = candidates.len() as u64;
    if sink.rejected_protected > 0 {
        tracing::debug!(
            rejected = sink.rejected_protected,
            "findings refused by the safety net"
        );
    }

    let reclaimable_bytes = candidates
        .iter()
        .filter(|c| c.recommended_action != RecommendedAction::InspectOnly)
        .map(|c| c.size)
        .sum();

    let summary = ScanSummary {
        scan_id: scan_id.to_string(),
        files_seen: progress.files_seen,
        bytes_seen: progress.bytes_seen,
        candidates_found: candidates.len() as u64,
        reclaimable_bytes,
        duration_ms: started.elapsed().as_millis() as u64,
        walk_ms,
        probe_ms,
        finish_ms,
        cancelled,
        hiccups,
    };

    observer.phase(Phase::Finished);
    observer.finished(&summary);

    ScanOutcome {
        summary,
        candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Confidence;
    use std::sync::Mutex;

    /// Collects everything a scan emitted.
    #[derive(Default)]
    pub struct Recorder {
        pub phases: Mutex<Vec<String>>,
        pub candidates: Mutex<Vec<CleanupCandidate>>,
        pub progress_reports: Mutex<u32>,
    }

    impl ScanObserver for Recorder {
        fn phase(&self, phase: Phase) {
            self.phases.lock().unwrap().push(format!("{phase:?}"));
        }
        fn progress(&self, _p: &Progress) {
            *self.progress_reports.lock().unwrap() += 1;
        }
        fn candidate(&self, c: &CleanupCandidate) {
            self.candidates.lock().unwrap().push(c.clone());
        }
        fn finished(&self, _s: &ScanSummary) {}
    }

    /// A detector that emits whatever it is told to, including things it
    /// should not be allowed to emit.
    struct Puppet {
        findings: Vec<Finding>,
    }

    impl Detector for Puppet {
        fn id(&self) -> &'static str {
            "puppet"
        }
        fn category(&self) -> Category {
            Category::Oddments
        }
        fn rummaging_note(&self) -> &'static str {
            "Doing as it is told"
        }
        fn finish(&mut self, _ctx: &ScanContext, sink: &mut dyn CandidateSink) {
            for finding in self.findings.drain(..) {
                sink.emit(finding);
            }
        }
    }

    fn entry(path: &std::path::Path, size: u64) -> FileEntry {
        FileEntry {
            name: crate::safety::paths::file_name_lower(path),
            ext: crate::safety::paths::extension(path).unwrap_or_default(),
            path: path.to_path_buf(),
            size,
            modified_unix: Some(1_700_000_000),
            accessed_unix: Some(1_700_000_000),
            created_unix: None,
            is_dir: false,
            depth: 2,
            opaque: false,
        }
    }

    struct Harness {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        ctx: ScanContext,
    }

    fn harness_with(ignores: IgnoreSet) -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();

        let platform = crate::platform::current();
        let mut ctx = ScanContext::new(
            ScanOptions {
                roots: vec![home.clone()],
                ..Default::default()
            },
            platform,
            ignores,
            Arc::new(AtomicBool::new(false)),
        );
        // Point the safety table at the fixture home rather than the real one.
        ctx.protected = Arc::new(ProtectedPaths::for_home(&home));
        Harness {
            _tmp: tmp,
            home,
            ctx,
        }
    }

    fn harness() -> Harness {
        harness_with(IgnoreSet::default())
    }

    fn run_puppet(h: &Harness, findings: Vec<Finding>) -> (Vec<CleanupCandidate>, Recorder) {
        let recorder = Recorder::default();
        let outcome = run(
            "scan1",
            &h.ctx,
            vec![Box::new(Puppet { findings })],
            &recorder,
        );
        (outcome.candidates, recorder)
    }

    #[test]
    fn the_safety_net_drops_a_protected_path_a_detector_tried_to_emit() {
        let h = harness();
        let key = h.home.join(".ssh/id_ed25519");
        std::fs::write(&key, "PRIVATE").unwrap();

        let finding = Finding::new(
            "puppet",
            Category::Oddments,
            &entry(&key, 100),
            "id_ed25519",
        )
        .with(EvidenceKind::UntouchedFor { days: 900 })
        .risk(Risk::Low);

        let (candidates, _) = run_puppet(&h, vec![finding]);
        assert!(
            candidates.is_empty(),
            "a buggy detector must not be able to surface a protected path"
        );
    }

    #[test]
    fn the_safety_net_drops_structural_folders() {
        let h = harness();
        let finding = Finding::new(
            "puppet",
            Category::Oddments,
            &entry(&h.home.join("Downloads"), 0),
            "Downloads",
        )
        .with(EvidenceKind::UntouchedFor { days: 900 });
        let (candidates, _) = run_puppet(&h, vec![finding]);
        assert!(candidates.is_empty());
    }

    #[test]
    fn detectors_cannot_choose_their_own_confidence_or_action() {
        let h = harness();
        let file = h.home.join("Downloads/thing.dmg");
        std::fs::write(&file, "x").unwrap();

        // Weak evidence, but the detector claims minimal risk.
        let finding = Finding::new(
            "puppet",
            Category::Installers,
            &entry(&file, 1),
            "thing.dmg",
        )
        .with(EvidenceKind::UntouchedFor { days: 10 })
        .risk(Risk::Low);

        let (candidates, _) = run_puppet(&h, vec![finding]);
        let c = &candidates[0];
        assert_eq!(c.confidence, Confidence::Low);
        assert_eq!(c.recommended_action, RecommendedAction::InspectOnly);
    }

    #[test]
    fn a_finding_with_no_evidence_is_not_a_finding() {
        let h = harness();
        let file = h.home.join("Downloads/thing.dmg");
        std::fs::write(&file, "x").unwrap();
        let finding = Finding::new("puppet", Category::Oddments, &entry(&file, 1), "thing.dmg");
        let (candidates, _) = run_puppet(&h, vec![finding]);
        assert!(candidates.is_empty());
    }

    #[test]
    fn ignored_paths_are_never_raised_again() {
        let h = harness_with(IgnoreSet {
            paths: vec![PathBuf::from("/")],
            ..Default::default()
        });
        let file = h.home.join("Downloads/thing.dmg");
        std::fs::write(&file, "x").unwrap();
        let finding = Finding::new(
            "puppet",
            Category::Installers,
            &entry(&file, 1),
            "thing.dmg",
        )
        .with(EvidenceKind::InstallerFormat { ext: "dmg".into() });
        let (candidates, _) = run_puppet(&h, vec![finding]);
        assert!(candidates.is_empty());
    }

    #[test]
    fn an_ignored_category_is_skipped_wholesale() {
        let h = harness_with(IgnoreSet {
            categories: vec![Category::Installers],
            ..Default::default()
        });
        let file = h.home.join("Downloads/thing.dmg");
        std::fs::write(&file, "x").unwrap();
        let finding = Finding::new(
            "puppet",
            Category::Installers,
            &entry(&file, 1),
            "thing.dmg",
        )
        .with(EvidenceKind::InstallerFormat { ext: "dmg".into() });
        let (candidates, _) = run_puppet(&h, vec![finding]);
        assert!(candidates.is_empty());
    }

    #[test]
    fn the_same_path_is_only_ever_in_one_pile() {
        let h = harness();
        let file = h.home.join("Downloads/thing.dmg");
        std::fs::write(&file, "x").unwrap();

        let thin = Finding::new(
            "puppet",
            Category::HeavyStrays,
            &entry(&file, 1),
            "thing.dmg",
        )
        .with(EvidenceKind::LargeSize { bytes: 1 });
        let rich = Finding::new(
            "puppet",
            Category::Installers,
            &entry(&file, 1),
            "thing.dmg",
        )
        .with(EvidenceKind::InstallerFormat { ext: "dmg".into() })
        .with(EvidenceKind::InstalledAppSupersedes {
            app: "Thing".into(),
        })
        .with(EvidenceKind::UntouchedFor { days: 200 });

        let (candidates, _) = run_puppet(&h, vec![thin, rich]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].category,
            Category::Installers,
            "the better-evidenced account should win"
        );
    }

    #[test]
    fn findings_in_user_territory_gain_that_as_evidence() {
        let h = harness();
        let desktop = h.home.join("Desktop");
        std::fs::create_dir_all(&desktop).unwrap();
        let file = desktop.join("Screenshot.png");
        std::fs::write(&file, "x").unwrap();

        let finding = Finding::new(
            "puppet",
            Category::Screenshots,
            &entry(&file, 1),
            "Screenshot.png",
        )
        .with(EvidenceKind::ScreenshotNamePattern {
            pattern: "Screenshot".into(),
        })
        .risk(Risk::Low);

        let (candidates, _) = run_puppet(&h, vec![finding]);
        let c = &candidates[0];
        assert!(
            c.evidence
                .iter()
                .any(|e| matches!(e.kind, EvidenceKind::UserContentDetected { .. })),
            "a file on the Desktop should carry that fact as evidence"
        );
        assert_eq!(c.risk, Risk::High);
        assert_eq!(c.recommended_action, RecommendedAction::InspectOnly);
    }

    #[test]
    fn a_cancelled_scan_says_so_and_stops() {
        let h = harness();
        h.ctx.request_cancel();
        let recorder = Recorder::default();
        let outcome = run("scan1", &h.ctx, vec![], &recorder);
        assert!(outcome.summary.cancelled);
        assert!(outcome.candidates.is_empty());
    }

    #[test]
    fn reclaimable_bytes_exclude_inspect_only_findings() {
        let h = harness();
        std::fs::write(h.home.join("Downloads/a.dmg"), "x").unwrap();
        std::fs::write(h.home.join("Downloads/b.zip"), "x").unwrap();

        let actionable = Finding::new(
            "puppet",
            Category::Installers,
            &entry(&h.home.join("Downloads/a.dmg"), 1000),
            "a.dmg",
        )
        .size(1000)
        .risk(Risk::Low)
        .with(EvidenceKind::InstallerFormat { ext: "dmg".into() })
        .with(EvidenceKind::KnownCachePath {
            owner: "Thing".into(),
        })
        .with(EvidenceKind::NoProcessUsingIt);

        let inspect_only = Finding::new(
            "puppet",
            Category::HeavyStrays,
            &entry(&h.home.join("Downloads/b.zip"), 9000),
            "b.zip",
        )
        .size(9000)
        .risk(Risk::High)
        .with(EvidenceKind::LargeSize { bytes: 9000 });

        let (candidates, _) = run_puppet(&h, vec![actionable, inspect_only]);
        assert_eq!(candidates.len(), 2);
        let outcome_bytes: u64 = candidates
            .iter()
            .filter(|c| c.recommended_action != RecommendedAction::InspectOnly)
            .map(|c| c.size)
            .sum();
        assert_eq!(outcome_bytes, 1000);
    }

    #[test]
    fn the_scan_reports_its_phases_in_order() {
        let h = harness();
        let (_, recorder) = run_puppet(&h, vec![]);
        let phases = recorder.phases.lock().unwrap().clone();
        assert!(phases.first().unwrap().contains("Preparing"));
        assert!(phases.last().unwrap().contains("Finished"));
        assert!(phases.iter().any(|p| p.contains("Rummaging")));
    }

    #[test]
    fn an_ignored_app_shields_all_of_its_findings() {
        let h = harness_with(IgnoreSet {
            apps: vec!["old game".into()],
            ..Default::default()
        });
        let file = h.home.join("Downloads/cache.bin");
        std::fs::write(&file, "x").unwrap();
        let finding = Finding::new("puppet", Category::Ghosts, &entry(&file, 1), "cache.bin")
            .app("Old Game")
            .with(EvidenceKind::ApplicationNotInstalled {
                app: "Old Game".into(),
            });
        let (candidates, _) = run_puppet(&h, vec![finding]);
        assert!(candidates.is_empty());
    }
}
