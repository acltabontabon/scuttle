//! macOS.

use std::path::{Path, PathBuf};

use super::apps::{AppSource, InstalledApp};
use super::caches::{self, CacheRule};
use super::games::{self, GameLibrary};
use super::util;
use super::{KnownLocation, LocationRole, PlatformService};
use crate::{Result, ScuttleError};

pub struct MacPlatformService {
    home: PathBuf,
}

impl MacPlatformService {
    pub fn new() -> Self {
        Self { home: util::home() }
    }

    /// Read `CFBundleIdentifier` and the display name out of an app bundle.
    fn read_bundle(path: &Path) -> Option<InstalledApp> {
        let info = path.join("Contents/Info.plist");
        let name_from_path = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let (bundle_id, display_name) = match plist::Value::from_file(&info) {
            Ok(plist::Value::Dictionary(dict)) => {
                let get = |key: &str| {
                    dict.get(key)
                        .and_then(|v| v.as_string())
                        .map(str::to_string)
                };
                (
                    get("CFBundleIdentifier"),
                    get("CFBundleDisplayName").or_else(|| get("CFBundleName")),
                )
            }
            // A bundle without a readable Info.plist is still an installed
            // application; we just know less about it.
            _ => (None, None),
        };

        Some(InstalledApp {
            name: display_name.unwrap_or(name_from_path),
            bundle_id,
            publisher: None,
            install_location: Some(path.to_path_buf()),
            source: AppSource::Bundle,
        })
    }

    fn collect_bundles(root: &Path, depth: usize, out: &mut Vec<InstalledApp>) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() || !file_type.is_dir() {
                continue;
            }
            if path.extension().is_some_and(|e| e == "app") {
                if let Some(app) = Self::read_bundle(&path) {
                    out.push(app);
                }
            } else if depth > 0 {
                // Applications are often one folder deep (`/Applications/Utilities`).
                Self::collect_bundles(&path, depth - 1, out);
            }
        }
    }

    fn steam_root(&self) -> PathBuf {
        self.home.join("Library/Application Support/Steam")
    }
}

impl PlatformService for MacPlatformService {
    fn name(&self) -> &'static str {
        "macos"
    }

    fn default_scan_roots(&self) -> Vec<KnownLocation> {
        let l = |rel: &str, label: &str, role: LocationRole| KnownLocation {
            path: self.home.join(rel),
            label: label.to_string(),
            role,
        };
        let mut roots = vec![
            l("Downloads", "Downloads", LocationRole::Downloads),
            l("Desktop", "Desktop", LocationRole::Desktop),
            l(
                "Library/Application Support",
                "Application Support",
                LocationRole::ApplicationSupport,
            ),
            l("Library/Caches", "Caches", LocationRole::Caches),
            l("Library/Logs", "Logs", LocationRole::Logs),
            l(
                "Library/Developer",
                "Developer",
                LocationRole::DeveloperCache,
            ),
            l(
                "Pictures/Screenshots",
                "Screenshots",
                LocationRole::Screenshots,
            ),
        ];
        let steam = self.steam_root();
        if steam.is_dir() {
            roots.push(KnownLocation {
                path: steam.join("steamapps"),
                label: "Steam library".into(),
                role: LocationRole::GameData,
            });
        }
        roots.retain(|r| r.path.is_dir());
        roots
    }

    fn screenshot_locations(&self) -> Vec<PathBuf> {
        let mut locations = vec![
            self.home.join("Desktop"),
            self.home.join("Pictures"),
            self.home.join("Pictures/Screenshots"),
            self.home.join("Downloads"),
        ];
        // The user may have moved the screenshot destination.
        let configured =
            util::capture("defaults", &["read", "com.apple.screencapture", "location"]);
        let configured = configured.trim();
        if !configured.is_empty() {
            let expanded = if let Some(rest) = configured.strip_prefix("~/") {
                self.home.join(rest)
            } else {
                PathBuf::from(configured)
            };
            locations.insert(0, expanded);
        }
        locations.retain(|p| p.is_dir());
        locations.dedup();
        locations
    }

    fn application_data_roots(&self) -> Vec<PathBuf> {
        [
            "Library/Application Support",
            "Library/Caches",
            "Library/Logs",
            "Library/Preferences",
            "Library/Saved Application State",
        ]
        .iter()
        .map(|rel| self.home.join(rel))
        .filter(|p| p.is_dir())
        .collect()
    }

    fn application_managed_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.application_data_roots();
        // Xcode owns all of this, including the simulators' stock photo
        // libraries — which are otherwise a fine source of "duplicates"
        // nobody can do anything about.
        roots.push(self.home.join("Library/Developer"));
        // Two copies of an asset inside two installed games are the games'
        // business, not the user's.
        for library in self.game_libraries() {
            roots.extend(library.install_roots);
        }
        roots.retain(|path| path.is_dir());
        roots
    }

    fn installed_apps(&self) -> Vec<InstalledApp> {
        let mut apps = Vec::new();
        for root in [
            PathBuf::from("/Applications"),
            self.home.join("Applications"),
            PathBuf::from("/System/Applications"),
        ] {
            Self::collect_bundles(&root, 1, &mut apps);
        }
        for library in self.game_libraries() {
            for game in library.installed {
                apps.push(InstalledApp {
                    name: game.name,
                    bundle_id: None,
                    publisher: Some(library.kind.label().to_string()),
                    install_location: game.install_dir,
                    source: AppSource::GameLibrary,
                });
            }
        }
        apps
    }

    fn cache_rules(&self) -> Vec<CacheRule> {
        caches::rules_for_home(&self.home)
    }

    fn game_libraries(&self) -> Vec<GameLibrary> {
        let mut libraries = Vec::new();
        if let Some(steam) = games::read_steam_library(&self.steam_root()) {
            libraries.push(steam);
        }
        let epic_root = self
            .home
            .join("Library/Application Support/Epic/EpicGamesLauncher");
        if let Some(epic) = games::read_epic_library(&epic_root.join("Data/Manifests"), &epic_root)
        {
            libraries.push(epic);
        }
        libraries
    }

    fn running_processes(&self) -> Vec<String> {
        util::capture("ps", &["-axco", "command"])
            .lines()
            .skip(1)
            .map(|l| l.trim().to_lowercase())
            .filter(|l| !l.is_empty())
            .collect()
    }

    fn reveal(&self, path: &Path) -> Result<()> {
        let status = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .status()
            .map_err(ScuttleError::Io)?;
        if status.success() {
            Ok(())
        } else {
            Err(ScuttleError::Internal(
                "Finder would not open that location.".into(),
            ))
        }
    }

    fn quarantine_root(&self) -> PathBuf {
        self.data_dir().join("Quarantine")
    }

    fn data_dir(&self) -> PathBuf {
        self.home.join("Library/Application Support/Scuttle")
    }

    fn installer_extensions(&self) -> &'static [&'static str] {
        &["dmg", "pkg", "mpkg", "iso"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_with_a_plist_is_read() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Thing.app");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>com.example.thing</string>
  <key>CFBundleName</key><string>Thing</string>
</dict></plist>"#,
        )
        .unwrap();

        let app = MacPlatformService::read_bundle(&bundle).unwrap();
        assert_eq!(app.name, "Thing");
        assert_eq!(app.bundle_id.as_deref(), Some("com.example.thing"));
    }

    #[test]
    fn a_bundle_without_a_plist_still_counts_as_installed() {
        // Getting this wrong would mean treating a real app's data as a ghost.
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Mystery.app");
        std::fs::create_dir_all(&bundle).unwrap();
        let app = MacPlatformService::read_bundle(&bundle).unwrap();
        assert_eq!(app.name, "Mystery");
        assert_eq!(app.bundle_id, None);
    }

    #[test]
    fn bundles_are_found_one_folder_deep() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("Utilities/Deep.app/Contents")).unwrap();
        std::fs::create_dir_all(tmp.path().join("Shallow.app/Contents")).unwrap();
        let mut out = Vec::new();
        MacPlatformService::collect_bundles(tmp.path(), 1, &mut out);
        let names: Vec<_> = out.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Deep"));
        assert!(names.contains(&"Shallow"));
    }

    #[test]
    fn the_real_machine_reports_some_applications() {
        // A smoke test against the host: if this returns nothing, ghost
        // detection would treat every leftover as orphaned.
        let service = MacPlatformService::new();
        assert!(
            !service.installed_apps().is_empty(),
            "no applications discovered on a real macOS machine"
        );
    }

    #[test]
    fn scan_roots_all_exist_and_are_absolute() {
        for root in MacPlatformService::new().default_scan_roots() {
            assert!(root.path.is_absolute());
            assert!(root.path.is_dir(), "{} does not exist", root.path.display());
            assert!(!root.label.is_empty());
        }
    }

    #[test]
    fn process_listing_returns_something_on_a_live_machine() {
        let processes = MacPlatformService::new().running_processes();
        assert!(!processes.is_empty());
        assert!(processes.iter().all(|p| p == &p.to_lowercase()));
    }
}
