//! The gate every destructive action passes through.
//!
//! The frontend is not trusted to decide whether something may be removed. It
//! sends an id; this module re-derives everything from scratch against the
//! live filesystem and either produces an [`AuthorizedTarget`] or refuses.
//!
//! The ordering below is deliberate: cheap structural refusals come before
//! filesystem I/O, and identity checks come before anything is touched.

use std::path::{Path, PathBuf};

use super::assess::{self, Assessment, Caution, CautionKind, Eligibility};
use super::paths::{self};
use super::protected::ProtectedPaths;
use crate::error::{Result, ScuttleError};
use crate::model::{CleanupCandidate, Risk, StateFingerprint, TargetKind};
use crate::platform::installations::CONTAINED_DEPTH;
use crate::platform::InstallAreas;

/// A path that has survived every check, together with the freshly observed
/// state that justified it. Only the quarantine module can consume one.
#[derive(Debug, Clone)]
pub struct AuthorizedTarget {
    pub path: PathBuf,
    pub kind: TargetKind,
    pub observed: StateFingerprint,
    /// What the gate concluded about it, live. The Drawer records from this
    /// whether the item may ever expire on its own.
    pub assessment: Assessment,
}

/// Who asked for this, and how specifically.
///
/// The distinction decides which of an item's [eligibilities] a request may
/// act on. It never unlocks a hard protection: `Blocked` refuses everyone.
///
/// Treating "Scuttle would not suggest this" as "this must never move" once
/// made the largest piles most people have — screenshots, heavy strays —
/// impossible to act on by any route. Nothing about that was safe; it just
/// moved the mess somewhere the application could not reach. The opposite
/// mistake is the one the Discord incident made: a sweep that nobody looked at
/// item by item reaching an application. Both are ruled out here.
///
/// [eligibilities]: crate::safety::assess::Eligibility
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bidding {
    /// Scuttle acting on its own judgement: a sweep. Only what it suggests,
    /// with nothing needing acknowledgement, because nobody looked.
    Scuttle,
    /// A batch a person built, with every item's cautions in front of them.
    /// Anything eligible by choice, once its cautions are acknowledged —
    /// never an application folder.
    User,
    /// One item a person opened and chose on its own. The only request that
    /// may move an application folder, and still only past its cautions.
    UserSpecific,
}

/// Everything the gate needs to make a decision.
pub struct ActionContext<'a> {
    pub protected: &'a ProtectedPaths,
    /// The roots the user actually asked Scuttle to look at. Nothing outside
    /// them can be acted on, whatever a stored finding claims.
    pub allowed_roots: &'a [PathBuf],
    /// Whether Scuttle decided this or a person did, and how specifically.
    pub bidding: Bidding,
    /// Cautions the person accepted for this batch, by kind.
    pub acknowledged: &'a [CautionKind],
    /// Where applications live, to recognise one the stored finding did not.
    pub installs: &'a InstallAreas,
}

/// Re-check a stored finding against the world as it is *now*.
pub fn authorize(
    candidate: &CleanupCandidate,
    ctx: &ActionContext<'_>,
) -> Result<AuthorizedTarget> {
    let path = paths::normalize(&candidate.path);

    // 1. What may this request move? Recomputed here from the stored evidence
    //    and the live filesystem; the webview's copy is never consulted.
    let assessment = assess_live(candidate, ctx);
    permit(candidate, &assessment, ctx)?;

    authorize_path(&path, candidate.target_kind, ctx)?;

    // 5b. A folder moves with everything in it, so a folder holding something
    //     protected is as off limits as the protected thing.
    if candidate.target_kind == TargetKind::Directory {
        if let Some((rule, _)) = ctx.protected.first_protected_descendant(&path, &|| false) {
            return Err(ScuttleError::Refused(format!(
                "This folder contains something protected ({rule}). Moving the folder would \
                 move that too, so Scuttle will not."
            )));
        }
    }

    // 6. Identity: is this still the same thing Scuttle looked at?
    let observed = observe(&path)?;
    check_unchanged(&candidate.fingerprint, &observed, &path)?;

    Ok(AuthorizedTarget {
        path,
        kind: candidate.target_kind,
        observed,
        assessment,
    })
}

/// The assessment the gate acts on: the stored evidence, plus whatever the
/// live filesystem says about applications at or around the path.
pub fn assess_live(candidate: &CleanupCandidate, ctx: &ActionContext<'_>) -> Assessment {
    let path = paths::normalize(&candidate.path);
    let root = ctx
        .allowed_roots
        .iter()
        .filter(|root| paths::is_within(&path, root))
        .max_by_key(|root| root.components().count());
    let install = if candidate.is_shared_contents() {
        // A cache is judged as the folder it is; the gate for contents
        // checks it does not hold an application.
        None
    } else {
        ctx.installs
            .enclosing(&path, root.map(PathBuf::as_path))
            .or_else(|| {
                if candidate.target_kind == TargetKind::Directory {
                    ctx.installs.contained(&path, CONTAINED_DEPTH)
                } else {
                    None
                }
            })
    };
    assess::with_installation(
        assess::assess(candidate, now_unix()),
        install.as_ref(),
        candidate.target_kind,
    )
}

/// Does this request's bidding and acknowledgement cover this assessment?
fn permit(candidate: &CleanupCandidate, a: &Assessment, ctx: &ActionContext<'_>) -> Result<()> {
    if candidate.risk == Risk::Protected || a.eligibility == Eligibility::Blocked {
        return Err(ScuttleError::Refused(
            a.blocked
                .clone()
                .unwrap_or_else(|| "Scuttle will not act on this at all.".into()),
        ));
    }
    match (ctx.bidding, a.eligibility) {
        (Bidding::Scuttle, Eligibility::Suggested) => {}
        (Bidding::Scuttle, _) => {
            return Err(ScuttleError::Refused(
                "Scuttle only moves this if you choose it yourself.".into(),
            ));
        }
        (Bidding::User, Eligibility::ExplicitOnly) => {
            return Err(ScuttleError::Refused(
                "This is part of an application. Scuttle only moves an application folder \
                 when you open it and choose it on its own, never as part of a batch."
                    .into(),
            ));
        }
        _ => {}
    }
    let missing: Vec<&Caution> = a
        .cautions
        .iter()
        .filter(|c| !ctx.acknowledged.contains(&c.kind))
        .collect();
    if let Some(first) = missing.first() {
        return Err(ScuttleError::NeedsAcknowledgement(format!(
            "{} {}",
            first.kind.headline(),
            first.detail
        )));
    }
    Ok(())
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The gate for cleaning the *contents* of a shared folder.
///
/// It is [`authorize`] with exactly one check left out: comparing the folder's
/// own modification time and child count with the scan. Those change whenever
/// anything is created or deleted inside it, which for a temp or cache folder
/// is constantly, so with them in place such a finding could never pass. What
/// the reviewed set replaces them with is a per-file check, made immediately
/// before each file moves.
///
/// Everything structural still applies — protected table, containment, links
/// below the scanned root, the target being a real directory — and the folder
/// itself is never a candidate to move.
pub fn authorize_contents(
    candidate: &CleanupCandidate,
    ctx: &ActionContext<'_>,
) -> Result<PathBuf> {
    let path = paths::normalize(&candidate.path);

    // A shared folder's own application does not make its files the
    // application: this is the one kind of finding that lives inside
    // application territory by design. But the folder must not *be* an
    // application, or hold one.
    let install = crate::platform::installations::looks_installed(&path)
        .map(|_| ())
        .or_else(|| ctx.installs.contained(&path, CONTAINED_DEPTH).map(|_| ()));
    if install.is_some() {
        return Err(ScuttleError::Refused(
            "This folder holds an application, not a cache. Scuttle will not clean it.".into(),
        ));
    }
    permit(candidate, &assess::assess(candidate, now_unix()), ctx)?;
    if !candidate.is_shared_contents() {
        return Err(ScuttleError::Refused(
            "That is not a folder whose contents Scuttle cleans.".into(),
        ));
    }
    authorize_path(&path, TargetKind::Directory, ctx)?;
    Ok(path)
}

/// The path-shaped half of the gate, shared with restore (which has a
/// destination rather than a finding).
pub fn authorize_path(path: &Path, kind: TargetKind, ctx: &ActionContext<'_>) -> Result<()> {
    // 2. Structural refusals. No evidence can override these.
    if !path.is_absolute() {
        return Err(ScuttleError::Refused(
            "Scuttle only acts on absolute paths.".into(),
        ));
    }
    if ctx.protected.is_too_shallow(path) {
        return Err(ScuttleError::Refused(
            "That location is part of the structure of your home folder. Scuttle will not touch it."
                .into(),
        ));
    }

    // 3. The protected table.
    if let Some(rule) = ctx.protected.rule_for(path) {
        return Err(ScuttleError::Refused(format!(
            "Protected: {}. Scuttle will not remove this.",
            rule.name
        )));
    }

    // 4. Containment. A finding cannot escape the area that was scanned.
    let Some(root) = ctx
        .allowed_roots
        .iter()
        .find(|root| paths::is_strictly_within(path, root))
    else {
        return Err(ScuttleError::Refused(
            "That path is outside the area Scuttle was asked to look at.".into(),
        ));
    };

    // 5. Links *below the scanned root*. If an intermediate directory is a
    // symlink, junction or reparse point, the thing we validated and the thing
    // we would move may not be the same object. Scuttle declines rather than
    // resolving and hoping. Links above the root (macOS `/var`, Windows
    // `C:\Users\Public`) are the operating system's business, not ours.
    if paths::first_link_below(path, root).is_some() {
        return Err(ScuttleError::Refused(
            "This path passes through a link. Scuttle will not follow links when removing things."
                .into(),
        ));
    }

    // The target itself being a link is equally disqualifying: removing it
    // would remove a reference whose meaning Scuttle cannot vouch for.
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(ScuttleError::Refused(
                    "This is a link rather than real data. Scuttle leaves links alone.".into(),
                ));
            }
            let actually_dir = meta.is_dir();
            let expected_dir = kind == TargetKind::Directory;
            if actually_dir != expected_dir {
                return Err(ScuttleError::Stale(
                    "This is no longer the kind of thing it was when Scuttle found it. Rummage again."
                        .into(),
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ScuttleError::Stale(
                "This is already gone. Nothing to do.".into(),
            ));
        }
        Err(e) => return Err(ScuttleError::Io(e)),
    }

    Ok(())
}

/// Take a fresh fingerprint of whatever is at `path`.
pub fn observe(path: &Path) -> Result<StateFingerprint> {
    let meta = std::fs::symlink_metadata(path)?;
    let is_dir = meta.is_dir();
    let modified_unix = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    let (size, child_count) = if is_dir {
        // Directory size is expensive to recompute; the child count is the
        // cheap signal that catches "the user started using this again".
        let count = std::fs::read_dir(path)
            .map(|rd| rd.filter_map(|e| e.ok()).count() as u64)
            .unwrap_or(0);
        (0, Some(count))
    } else {
        (meta.len(), None)
    };

    Ok(StateFingerprint {
        size,
        modified_unix,
        is_dir,
        child_count,
    })
}

/// Compare the scan-time snapshot with the live one.
fn check_unchanged(at_scan: &StateFingerprint, now: &StateFingerprint, path: &Path) -> Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "this".into());

    if at_scan.is_dir != now.is_dir {
        return Err(ScuttleError::Stale(format!(
            "{name} changed between the rummage and now. Rummage again."
        )));
    }
    if !at_scan.is_dir && at_scan.size != now.size {
        return Err(ScuttleError::Stale(format!(
            "{name} has changed size since Scuttle found it. Rummage again."
        )));
    }
    if at_scan.modified_unix != now.modified_unix {
        return Err(ScuttleError::Stale(format!(
            "{name} has been modified since Scuttle found it. Rummage again."
        )));
    }
    if at_scan.is_dir && at_scan.child_count != now.child_count {
        return Err(ScuttleError::Stale(format!(
            "The contents of {name} changed since Scuttle looked. Rummage again."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{ev, EvidenceKind};
    use crate::model::*;
    use std::fs;

    struct Harness {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        roots: Vec<PathBuf>,
        protected: ProtectedPaths,
    }

    fn harness() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home").join("tester");
        fs::create_dir_all(home.join("Downloads")).unwrap();
        fs::create_dir_all(home.join(".ssh")).unwrap();
        fs::create_dir_all(home.join("Library/Caches")).unwrap();
        let protected = ProtectedPaths::for_home(&home);
        let roots = vec![home.clone()];
        Harness {
            _tmp: tmp,
            home,
            roots,
            protected,
        }
    }

    impl Harness {
        fn ctx(&self) -> ActionContext<'_> {
            ActionContext {
                protected: &self.protected,
                allowed_roots: &self.roots,
                // Path checks are what these tests are about, so every
                // caution is accepted; the bidding tests say otherwise.
                bidding: Bidding::User,
                acknowledged: &CautionKind::ALL,
                installs: &crate::platform::installations::NO_INSTALL_AREAS,
            }
        }
        fn file(&self, rel: &str, contents: &str) -> PathBuf {
            let p = self.home.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, contents).unwrap();
            p
        }
        fn candidate_for(&self, path: &Path) -> CleanupCandidate {
            let fingerprint = observe(path).unwrap();
            CleanupCandidate {
                id: "c1".into(),
                detector: "test".into(),
                category: Category::Installers,
                target_kind: if fingerprint.is_dir {
                    TargetKind::Directory
                } else {
                    TargetKind::File
                },
                path: path.to_path_buf(),
                display_name: paths::file_name_lower(path),
                associated_app: None,
                size: fingerprint.size,
                group_bytes: fingerprint.size,
                confidence: Confidence::High,
                risk: Risk::Low,
                recommended_action: RecommendedAction::Quarantine,
                evidence: vec![ev(EvidenceKind::InstallerFormat { ext: "dmg".into() })],
                remark: None,
                modified_unix: fingerprint.modified_unix,
                accessed_unix: None,
                created_unix: None,
                group: vec![],
                fingerprint,
                assessment: Default::default(),
            }
        }
    }

    #[test]
    fn a_clean_candidate_is_authorized() {
        let h = harness();
        let f = h.file("Downloads/Thing.dmg", "installer bytes");
        let c = h.candidate_for(&f);
        let target = authorize(&c, &h.ctx()).unwrap();
        assert_eq!(target.path, paths::normalize(&f));
    }

    #[test]
    fn a_file_modified_since_the_scan_is_refused_as_stale() {
        let h = harness();
        let f = h.file("Downloads/Thing.dmg", "installer bytes");
        let c = h.candidate_for(&f);
        // The world moves.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(&f, "different bytes now").unwrap();

        let err = authorize(&c, &h.ctx()).unwrap_err();
        assert_eq!(err.code(), "stale", "got: {err}");
    }

    #[test]
    fn a_directory_that_gained_children_is_refused_as_stale() {
        let h = harness();
        let dir = h.home.join("Library/Caches/com.dead.app");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.bin"), "x").unwrap();
        let mut c = h.candidate_for(&dir);
        c.target_kind = TargetKind::Directory;

        fs::write(dir.join("b.bin"), "y").unwrap();

        let err = authorize(&c, &h.ctx()).unwrap_err();
        assert_eq!(err.code(), "stale", "got: {err}");
    }

    #[test]
    fn a_vanished_file_is_stale_not_an_error() {
        let h = harness();
        let f = h.file("Downloads/Thing.dmg", "bytes");
        let c = h.candidate_for(&f);
        fs::remove_file(&f).unwrap();
        assert_eq!(authorize(&c, &h.ctx()).unwrap_err().code(), "stale");
    }

    #[test]
    fn a_protected_path_is_refused_even_with_a_pristine_fingerprint() {
        let h = harness();
        let f = h.file(".ssh/id_ed25519", "PRIVATE KEY");
        let c = h.candidate_for(&f);
        let err = authorize(&c, &h.ctx()).unwrap_err();
        assert_eq!(err.code(), "refused");
        assert!(err.to_string().contains("Protected"), "{err}");
    }

    #[test]
    fn a_path_outside_the_scanned_roots_is_refused() {
        let h = harness();
        let outside = h._tmp.path().join("elsewhere/thing.dmg");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, "x").unwrap();
        let c = h.candidate_for(&outside);
        let err = authorize(&c, &h.ctx()).unwrap_err();
        assert_eq!(err.code(), "refused");
        assert!(err.to_string().contains("outside"), "{err}");
    }

    #[test]
    fn a_symlink_target_is_refused() {
        let h = harness();
        let real = h.file("Downloads/real.dmg", "bytes");
        let link = h.home.join("Downloads/link.dmg");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&real, &link).unwrap();

        let mut c = h.candidate_for(&real);
        c.path = link;
        let err = authorize(&c, &h.ctx()).unwrap_err();
        assert_eq!(err.code(), "refused");
        assert!(err.to_string().contains("link"), "{err}");
    }

    #[test]
    fn a_path_reached_through_a_symlinked_directory_is_refused() {
        let h = harness();
        let real_dir = h.home.join("Downloads/real");
        fs::create_dir_all(&real_dir).unwrap();
        fs::write(real_dir.join("thing.dmg"), "bytes").unwrap();
        let link_dir = h.home.join("Downloads/shortcut");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real_dir, &link_dir).unwrap();

        let mut c = h.candidate_for(&real_dir.join("thing.dmg"));
        c.path = link_dir.join("thing.dmg");
        let err = authorize(&c, &h.ctx()).unwrap_err();
        assert_eq!(err.code(), "refused");
        assert!(err.to_string().contains("link"), "{err}");
    }

    impl Harness {
        fn ctx_as<'a>(
            &'a self,
            bidding: Bidding,
            acknowledged: &'a [CautionKind],
        ) -> ActionContext<'a> {
            ActionContext {
                bidding,
                acknowledged,
                ..self.ctx()
            }
        }

        /// A synthetic Squirrel install, shaped like Discord's.
        fn discord_like(&self, base: &str) -> PathBuf {
            let root = self.home.join(base).join("Discord");
            self.file(&format!("{base}/Discord/Update.exe"), "updater");
            self.file(
                &format!("{base}/Discord/app-1.0.9001/Discord.exe"),
                "program",
            );
            self.file(
                &format!("{base}/Discord/app-1.0.9001/ffmpeg.dll"),
                "library",
            );
            self.file(
                &format!("{base}/Discord/packages/DiscordSetup.exe"),
                "setup",
            );
            root
        }
    }

    #[test]
    fn a_sweep_moves_only_what_scuttle_suggests() {
        let h = harness();
        // Fresh: it changed today, so it carries a caution and is not
        // something Scuttle sweeps on its own.
        let f = h.file("Downloads/big.zip", "bytes");
        let c = h.candidate_for(&f);
        let err = authorize(&c, &h.ctx_as(Bidding::Scuttle, &[])).unwrap_err();
        assert_eq!(err.code(), "refused");
        // A person may still choose it, once they have seen why it is
        // flagged.
        assert!(authorize(
            &c,
            &h.ctx_as(Bidding::User, &[CautionKind::RecentlyChanged])
        )
        .is_ok());
    }

    #[test]
    fn a_caution_must_be_acknowledged_but_is_not_a_prohibition() {
        let h = harness();
        let f = h.file("Downloads/draft.docx", "words");
        let mut c = h.candidate_for(&f);
        c.recommended_action = RecommendedAction::InspectOnly;
        c.confidence = Confidence::Low;

        let err = authorize(&c, &h.ctx_as(Bidding::User, &[])).unwrap_err();
        assert_eq!(err.code(), "needs_acknowledgement");
        let err = authorize(
            &c,
            &h.ctx_as(Bidding::User, &[CautionKind::RecentlyChanged]),
        )
        .unwrap_err();
        assert_eq!(err.code(), "needs_acknowledgement", "uncertainty too");
        authorize(
            &c,
            &h.ctx_as(
                Bidding::User,
                &[CautionKind::RecentlyChanged, CautionKind::Uncertain],
            ),
        )
        .expect("a recently edited document of the person's own can be moved after a caution");
    }

    #[test]
    fn an_application_is_never_moved_by_a_sweep_or_a_batch() {
        // The Discord incident at the gate: a stored finding that calls an
        // application's program "an installer, confident, low risk" — as the
        // old detector did. The live filesystem says otherwise, and that wins.
        let h = harness();
        let root = h.discord_like("AppData/Local");
        let exe = root.join("app-1.0.9001/Discord.exe");
        let c = h.candidate_for(&exe);
        assert_eq!(c.recommended_action, RecommendedAction::Quarantine);

        let sweep = authorize(&c, &h.ctx_as(Bidding::Scuttle, &CautionKind::ALL)).unwrap_err();
        assert_eq!(sweep.code(), "refused");
        let batch = authorize(&c, &h.ctx_as(Bidding::User, &CautionKind::ALL)).unwrap_err();
        assert_eq!(batch.code(), "refused");
        assert!(batch.to_string().contains("application"), "{batch}");

        // Chosen on its own, it still has to be acknowledged as breaking it.
        let specific = authorize(
            &c,
            &h.ctx_as(Bidding::UserSpecific, &[CautionKind::RecentlyChanged]),
        )
        .unwrap_err();
        assert_eq!(specific.code(), "needs_acknowledgement");
        assert!(
            specific.to_string().contains("stop that application"),
            "{specific}"
        );
        assert!(authorize(&c, &h.ctx_as(Bidding::UserSpecific, &CautionKind::ALL)).is_ok());
        assert!(exe.exists(), "the gate never touches anything");
    }

    #[test]
    fn a_parent_holding_an_application_is_an_application_folder() {
        let h = harness();
        h.discord_like("Downloads/stuff");
        let parent = h.home.join("Downloads/stuff");
        let c = h.candidate_for(&parent);
        let err = authorize(&c, &h.ctx_as(Bidding::User, &CautionKind::ALL)).unwrap_err();
        assert_eq!(err.code(), "refused");
    }

    #[test]
    fn a_folder_containing_something_protected_is_refused_for_everyone() {
        let h = harness();
        h.file("Downloads/backup/.ssh/id_ed25519", "secret");
        h.file("Downloads/backup/notes.txt", "notes");
        let dir = h.home.join("Downloads/backup");
        let c = h.candidate_for(&dir);
        for bidding in [Bidding::Scuttle, Bidding::User, Bidding::UserSpecific] {
            let err = authorize(&c, &h.ctx_as(bidding, &CautionKind::ALL)).unwrap_err();
            assert_eq!(err.code(), "refused", "{bidding:?}");
        }
        assert!(dir.join(".ssh/id_ed25519").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_protected_place_is_refused() {
        let h = harness();
        let link = h.home.join("Downloads/keys");
        std::os::unix::fs::symlink(h.home.join(".ssh"), &link).unwrap();
        let fingerprint = StateFingerprint {
            is_dir: true,
            ..Default::default()
        };
        let mut c = h.candidate_for(&h.home.join("Downloads"));
        c.path = link.clone();
        c.fingerprint = fingerprint;
        c.target_kind = TargetKind::Directory;
        let err = authorize(&c, &h.ctx_as(Bidding::UserSpecific, &CautionKind::ALL)).unwrap_err();
        assert_eq!(err.code(), "refused");
        assert!(h.home.join(".ssh").exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_to_a_protected_place_is_refused() {
        let h = harness();
        let link = h.home.join("Downloads\\keys");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(h.home.join(".ssh"))
            .status()
            .unwrap();
        assert!(status.success());
        let mut c = h.candidate_for(&h.home.join("Downloads"));
        c.path = link;
        c.target_kind = TargetKind::Directory;
        c.fingerprint.is_dir = true;
        let err = authorize(&c, &h.ctx_as(Bidding::UserSpecific, &CautionKind::ALL)).unwrap_err();
        assert_eq!(err.code(), "refused");
    }

    #[test]
    fn protected_is_refused_even_for_a_specific_choice() {
        let h = harness();
        let f = h.file("Downloads/thing.dmg", "bytes");
        let mut c = h.candidate_for(&f);
        c.risk = Risk::Protected;
        let err = authorize(&c, &h.ctx_as(Bidding::UserSpecific, &CautionKind::ALL)).unwrap_err();
        assert_eq!(err.code(), "refused");
    }

    #[test]
    fn a_finding_that_became_a_directory_is_stale() {
        let h = harness();
        let f = h.file("Downloads/thing", "bytes");
        let c = h.candidate_for(&f);
        fs::remove_file(&f).unwrap();
        fs::create_dir(&f).unwrap();
        assert_eq!(authorize(&c, &h.ctx()).unwrap_err().code(), "stale");
    }

    #[test]
    fn structural_home_folders_are_refused() {
        let h = harness();
        let mut c = h.candidate_for(&h.home.join("Downloads"));
        c.target_kind = TargetKind::Directory;
        let err = authorize(&c, &h.ctx()).unwrap_err();
        assert_eq!(err.code(), "refused");
    }
}
