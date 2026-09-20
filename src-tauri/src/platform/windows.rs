//! Windows.

use std::path::{Path, PathBuf};

use winreg::enums::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
};
use winreg::RegKey;

use super::apps::{AppSource, InstalledApp};
use super::caches::{self, CacheRule};
use super::games::{self, GameLibrary};
use super::util;
use super::{KnownLocation, LocationRole, PlatformService};
use crate::{Result, ScuttleError};

const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

pub struct WindowsPlatformService {
    home: PathBuf,
}

impl WindowsPlatformService {
    pub fn new() -> Self {
        Self { home: util::home() }
    }

    /// Walk one uninstall registry hive into installed applications.
    ///
    /// Both the 32- and 64-bit views are read: an application registered only
    /// in the WOW6432Node view would otherwise look uninstalled, and Scuttle
    /// would call its data a ghost.
    fn read_uninstall_hive(root: RegKey, flags: u32, out: &mut Vec<InstalledApp>) {
        let Ok(uninstall) = root.open_subkey_with_flags(UNINSTALL_KEY, KEY_READ | flags) else {
            return;
        };
        for name in uninstall.enum_keys().flatten() {
            let Ok(entry) = uninstall.open_subkey_with_flags(&name, KEY_READ | flags) else {
                continue;
            };
            // Update patches and system components are not applications.
            let system_component: u32 = entry.get_value("SystemComponent").unwrap_or(0);
            if system_component == 1 {
                continue;
            }
            let Ok(display_name) = entry.get_value::<String, _>("DisplayName") else {
                continue;
            };
            if display_name.trim().is_empty() {
                continue;
            }
            out.push(InstalledApp {
                name: display_name,
                bundle_id: None,
                publisher: entry.get_value::<String, _>("Publisher").ok(),
                install_location: entry
                    .get_value::<String, _>("InstallLocation")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .map(PathBuf::from),
                source: AppSource::Registry,
            });
        }
    }

    fn steam_root(&self) -> Option<PathBuf> {
        // Steam records its own location; fall back to the usual spot.
        let from_registry = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Valve\Steam")
            .ok()
            .and_then(|k| k.get_value::<String, _>("SteamPath").ok())
            .map(PathBuf::from);

        from_registry
            .into_iter()
            .chain([
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"C:\Program Files\Steam"),
            ])
            .find(|p| p.join("steamapps").is_dir())
    }

    fn local_app_data(&self) -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join("AppData\\Local"))
    }

    fn roaming_app_data(&self) -> PathBuf {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join("AppData\\Roaming"))
    }
}

impl PlatformService for WindowsPlatformService {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn default_scan_roots(&self) -> Vec<KnownLocation> {
        let mut roots = vec![
            KnownLocation {
                path: self.home.join("Downloads"),
                label: "Downloads".into(),
                role: LocationRole::Downloads,
            },
            KnownLocation {
                path: self.home.join("Desktop"),
                label: "Desktop".into(),
                role: LocationRole::Desktop,
            },
            KnownLocation {
                path: self.local_app_data(),
                label: "Local app data".into(),
                role: LocationRole::ApplicationSupport,
            },
            KnownLocation {
                path: self.roaming_app_data(),
                label: "Roaming app data".into(),
                role: LocationRole::ApplicationSupport,
            },
            KnownLocation {
                path: self.home.join("AppData\\LocalLow"),
                label: "LocalLow app data".into(),
                role: LocationRole::ApplicationSupport,
            },
            KnownLocation {
                path: self.home.join("Pictures\\Screenshots"),
                label: "Screenshots".into(),
                role: LocationRole::Screenshots,
            },
            KnownLocation {
                path: self.home.join("Videos\\Captures"),
                label: "Captures".into(),
                role: LocationRole::Screenshots,
            },
        ];
        if let Some(steam) = self.steam_root() {
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
            self.home.join("Pictures\\Screenshots"),
            self.home.join("Pictures"),
            self.home.join("Videos\\Captures"),
            self.home.join("OneDrive\\Pictures\\Screenshots"),
            self.home.join("Desktop"),
        ];
        locations.retain(|p| p.is_dir());
        locations.dedup();
        locations
    }

    fn application_data_roots(&self) -> Vec<PathBuf> {
        [
            self.local_app_data(),
            self.roaming_app_data(),
            self.home.join("AppData\\LocalLow"),
        ]
        .into_iter()
        .filter(|p| p.is_dir())
        .collect()
    }

    fn application_managed_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.application_data_roots();
        for library in self.game_libraries() {
            roots.extend(library.install_roots);
        }
        roots.retain(|path| path.is_dir());
        roots
    }

    fn installed_apps(&self) -> Vec<InstalledApp> {
        let mut apps = Vec::new();
        Self::read_uninstall_hive(
            RegKey::predef(HKEY_LOCAL_MACHINE),
            KEY_WOW64_64KEY,
            &mut apps,
        );
        Self::read_uninstall_hive(
            RegKey::predef(HKEY_LOCAL_MACHINE),
            KEY_WOW64_32KEY,
            &mut apps,
        );
        Self::read_uninstall_hive(RegKey::predef(HKEY_CURRENT_USER), 0, &mut apps);

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
        if let Some(root) = self.steam_root() {
            if let Some(steam) = games::read_steam_library(&root) {
                libraries.push(steam);
            }
        }
        let epic_root = PathBuf::from(r"C:\ProgramData\Epic\EpicGamesLauncher");
        if let Some(epic) = games::read_epic_library(&epic_root.join("Data\\Manifests"), &epic_root)
        {
            libraries.push(epic);
        }
        libraries
    }

    fn running_processes(&self) -> Vec<String> {
        util::capture("tasklist", &["/fo", "csv", "/nh"])
            .lines()
            .filter_map(|line| line.split('"').nth(1))
            .map(|name| name.trim_end_matches(".exe").to_lowercase())
            .filter(|name| !name.is_empty())
            .collect()
    }

    fn reveal(&self, path: &Path) -> Result<()> {
        // explorer.exe returns a non-zero exit code even on success, so the
        // status is deliberately not checked.
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .map_err(ScuttleError::Io)?;
        Ok(())
    }

    fn quarantine_root(&self) -> PathBuf {
        self.data_dir().join("Quarantine")
    }

    fn data_dir(&self) -> PathBuf {
        self.local_app_data().join("Scuttle")
    }

    fn installer_extensions(&self) -> &'static [&'static str] {
        &["exe", "msi", "msix", "appx", "msp", "iso"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_reports_some_installed_applications() {
        let mut apps = Vec::new();
        WindowsPlatformService::read_uninstall_hive(
            RegKey::predef(HKEY_LOCAL_MACHINE),
            KEY_WOW64_64KEY,
            &mut apps,
        );
        assert!(
            !apps.is_empty(),
            "no applications found in the uninstall hive"
        );
        assert!(apps.iter().all(|a| !a.name.trim().is_empty()));
    }

    #[test]
    fn scan_roots_all_exist_and_are_absolute() {
        for root in WindowsPlatformService::new().default_scan_roots() {
            assert!(root.path.is_absolute());
            assert!(root.path.is_dir(), "{} does not exist", root.path.display());
        }
    }

    #[test]
    fn process_names_are_lowercased_without_the_extension() {
        let processes = WindowsPlatformService::new().running_processes();
        assert!(!processes.is_empty());
        assert!(processes.iter().all(|p| !p.ends_with(".exe")));
        assert!(processes.iter().all(|p| p == &p.to_lowercase()));
    }

    #[test]
    fn app_data_roots_are_discovered() {
        let service = WindowsPlatformService::new();
        assert!(!service.application_data_roots().is_empty());
    }
}
