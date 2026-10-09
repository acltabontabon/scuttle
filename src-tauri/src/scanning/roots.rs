//! One root policy for scans, previews and the move gate.
use std::path::PathBuf;

use super::ScanOptions;
use crate::{platform::PlatformService, safety::paths, storage::Settings};

pub fn default_stale_days() -> u32 {
    14
}

pub fn common(platform: &dyn PlatformService) -> Vec<PathBuf> {
    let home = platform.home_dir();
    ["Code", "Projects", "Workspace", "Developer", "source/repos"]
        .iter()
        .map(|name| home.join(name))
        .filter(|p| {
            crate::quarantine::fsx::identity_of(p)
                .is_ok_and(|i| i.kind == crate::quarantine::fsx::EntryKind::Dir)
                && paths::first_link_below(p, &home).is_none()
        })
        .collect()
}

pub fn deduplicate(mut roots: Vec<PathBuf>) -> Vec<PathBuf> {
    roots = roots.into_iter().map(|p| paths::normalize(&p)).collect();
    roots.sort_by_key(|p| p.components().count());
    let mut out: Vec<PathBuf> = Vec::new();
    for root in roots {
        if !out.iter().any(|p| paths::is_within(&root, p)) {
            out.push(root);
        }
    }
    out
}

pub fn resolve(
    platform: &dyn PlatformService,
    settings: &Settings,
    explicit: Option<Vec<PathBuf>>,
    include: Option<bool>,
) -> ScanOptions {
    let enabled = include.unwrap_or(settings.include_developer_debris);
    let explicit = explicit.filter(|r| !r.is_empty());
    let default_locations = explicit.is_none() && settings.scan_roots.is_empty();
    let developer_roots = if enabled && explicit.is_none() {
        if settings.developer_roots.is_empty() {
            common(platform)
        } else {
            settings.developer_roots.clone()
        }
    } else {
        Vec::new()
    };
    let roots = explicit.unwrap_or_else(|| {
        if settings.scan_roots.is_empty() {
            platform
                .default_scan_roots()
                .into_iter()
                .map(|r| r.path)
                .collect()
        } else {
            settings.scan_roots.clone()
        }
    });
    let organization_roots = platform
        .organization_roots()
        .into_iter()
        .filter(|p| default_locations || roots.iter().any(|r| paths::is_within(p, r)))
        .collect();
    ScanOptions {
        organization_roots,
        roots: deduplicate(roots),
        developer_roots: deduplicate(developer_roots),
        include_developer_debris: enabled,
        developer_stale_days: match settings.developer_stale_days {
            14 | 30 | 60 => settings.developer_stale_days,
            _ => 14,
        },
        dependency_cache_locations: settings.dependency_caches.locations.clone(),
        heavy_threshold: settings.heavy_threshold,
        ..Default::default()
    }
}

impl ScanOptions {
    pub fn all_roots(&self) -> Vec<PathBuf> {
        deduplicate(
            self.roots
                .iter()
                .chain(
                    self.developer_roots
                        .iter()
                        .filter(|_| self.include_developer_debris),
                )
                .cloned()
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::testing::FixedPlatform;
    #[test]
    fn common_locations_are_opt_in_and_explicit_requests_stay_confined() {
        let temp = tempfile::tempdir().unwrap();
        for rel in ["Code", "Workspace", "source/repos"] {
            std::fs::create_dir_all(temp.path().join(rel)).unwrap();
        }
        let platform = FixedPlatform::new(temp.path());
        let mut settings = Settings::default();
        assert!(resolve(&platform, &settings, None, None)
            .developer_roots
            .is_empty());
        settings.include_developer_debris = true;
        let options = resolve(&platform, &settings, None, None);
        assert_eq!(options.developer_roots.len(), 3);
        assert_eq!(options.developer_stale_days, 14);
        let explicit = temp.path().join("OnlyHere");
        let options = resolve(&platform, &settings, Some(vec![explicit.clone()]), None);
        assert_eq!(options.all_roots(), vec![explicit]);
        assert!(options.developer_roots.is_empty());
    }
    #[test]
    fn custom_locations_replace_common_defaults_and_overlap_is_deduplicated() {
        let platform = FixedPlatform::new("/fixture/home");
        let settings = Settings {
            include_developer_debris: true,
            developer_stale_days: 60,
            developer_roots: vec!["/fixture/custom".into(), "/fixture/custom/nested".into()],
            ..Default::default()
        };
        let options = resolve(&platform, &settings, None, None);
        assert_eq!(
            options.developer_roots,
            vec![PathBuf::from("/fixture/custom")]
        );
        assert_eq!(options.developer_stale_days, 60);
    }
    #[test]
    fn legacy_preferences_keep_developer_mode_off_and_use_fourteen_days() {
        let settings: Settings = serde_json::from_str(r#"{"appearance":"dark"}"#).unwrap();
        assert!(!settings.include_developer_debris);
        assert_eq!(settings.developer_stale_days, 14);
        assert!(settings.developer_roots.is_empty());
    }
}
