//! The gate every destructive action passes through.
//!
//! The frontend is not trusted to decide whether something may be removed. It
//! sends an id; this module re-derives everything from scratch against the
//! live filesystem and either produces an [`AuthorizedTarget`] or refuses.
//!
//! The ordering below is deliberate: cheap structural refusals come before
//! filesystem I/O, and identity checks come before anything is touched.

use std::path::{Path, PathBuf};

use super::paths::{self};
use super::protected::ProtectedPaths;
use crate::error::{Result, ScuttleError};
use crate::model::{CleanupCandidate, StateFingerprint, TargetKind};

/// A path that has survived every check, together with the freshly observed
/// state that justified it. Only the quarantine module can consume one.
#[derive(Debug, Clone)]
pub struct AuthorizedTarget {
    pub path: PathBuf,
    pub kind: TargetKind,
    pub observed: StateFingerprint,
}

/// Everything the gate needs to make a decision.
pub struct ActionContext<'a> {
    pub protected: &'a ProtectedPaths,
    /// The roots the user actually asked Scuttle to look at. Nothing outside
    /// them can be acted on, whatever a stored finding claims.
    pub allowed_roots: &'a [PathBuf],
}

/// Re-check a stored finding against the world as it is *now*.
pub fn authorize(
    candidate: &CleanupCandidate,
    ctx: &ActionContext<'_>,
) -> Result<AuthorizedTarget> {
    let path = paths::normalize(&candidate.path);

    // 1. Is this the kind of finding that may be acted on at all?
    if !candidate.is_actionable() {
        return Err(ScuttleError::Refused(format!(
            "Scuttle does not act on findings marked {}.",
            candidate.recommended_action_label()
        )));
    }

    authorize_path(&path, candidate.target_kind, ctx)?;

    // 6. Identity: is this still the same thing Scuttle looked at?
    let observed = observe(&path)?;
    check_unchanged(&candidate.fingerprint, &observed, &path)?;

    Ok(AuthorizedTarget {
        path,
        kind: candidate.target_kind,
        observed,
    })
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

impl CleanupCandidate {
    pub(crate) fn recommended_action_label(&self) -> &'static str {
        use crate::model::RecommendedAction::*;
        match self.recommended_action {
            Quarantine => "quarantine",
            Review => "review",
            InspectOnly => "inspect only",
        }
    }
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

    #[test]
    fn an_inspect_only_finding_cannot_be_acted_on_even_if_the_ui_asks() {
        let h = harness();
        let f = h.file("Downloads/big.zip", "bytes");
        let mut c = h.candidate_for(&f);
        c.recommended_action = RecommendedAction::InspectOnly;
        let err = authorize(&c, &h.ctx()).unwrap_err();
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
