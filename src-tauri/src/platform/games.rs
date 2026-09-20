//! Game libraries.
//!
//! "I uninstalled this six months ago, why is 8 GB still here?" is the
//! question Scuttle most wants to answer well. Answering it means reading the
//! launchers' own installation records rather than guessing from folder names.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryKind {
    Steam,
    Epic,
}

impl LibraryKind {
    pub fn label(&self) -> &'static str {
        match self {
            LibraryKind::Steam => "Steam",
            LibraryKind::Epic => "Epic Games",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GameInstall {
    pub name: String,
    /// Steam appid or Epic catalog item id.
    pub id: String,
    pub install_dir: Option<PathBuf>,
}

/// One launcher's view of the machine: where its games live and which of them
/// are currently installed.
#[derive(Debug, Clone)]
pub struct GameLibrary {
    pub kind: LibraryKind,
    /// The launcher's own root.
    pub root: PathBuf,
    /// Directories the launcher installs games into.
    pub install_roots: Vec<PathBuf>,
    /// Directories holding launcher-generated data (shader caches, depot
    /// downloads) that belong to titles rather than to the launcher itself.
    pub generated_roots: Vec<PathBuf>,
    pub installed: Vec<GameInstall>,
}

impl GameLibrary {
    /// Is a title with this folder name currently installed?
    pub fn has_install_dir(&self, folder_name: &str) -> bool {
        let needle = super::apps::normalize_name(folder_name);
        self.installed.iter().any(|g| {
            g.install_dir
                .as_ref()
                .and_then(|d| d.file_name())
                .map(|n| super::apps::normalize_name(&n.to_string_lossy()) == needle)
                .unwrap_or(false)
                || super::apps::normalize_name(&g.name) == needle
        })
    }

    pub fn has_app_id(&self, id: &str) -> bool {
        self.installed.iter().any(|g| g.id == id)
    }
}

// ---------------------------------------------------------------------------
// Steam
// ---------------------------------------------------------------------------

/// Read a Steam installation rooted at `steam_root`.
pub fn read_steam_library(steam_root: &Path) -> Option<GameLibrary> {
    let steamapps = steam_root.join("steamapps");
    if !steamapps.is_dir() {
        return None;
    }

    // Steam can spread games over several volumes; libraryfolders.vdf is the
    // index. If it is missing or unreadable we still know about the default.
    let mut library_paths = vec![steam_root.to_path_buf()];
    if let Ok(text) = std::fs::read_to_string(steamapps.join("libraryfolders.vdf")) {
        for (key, value) in vdf::pairs(&text) {
            if key == "path" {
                let p = PathBuf::from(value);
                if p.is_dir() && !library_paths.contains(&p) {
                    library_paths.push(p);
                }
            }
        }
    }

    let mut installed = Vec::new();
    let mut install_roots = Vec::new();
    let mut generated_roots = Vec::new();

    for library in &library_paths {
        let apps_dir = library.join("steamapps");
        install_roots.push(apps_dir.join("common"));
        for generated in ["shadercache", "downloading", "temp", "compatdata"] {
            generated_roots.push(apps_dir.join(generated));
        }

        let Ok(entries) = std::fs::read_dir(&apps_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(game) = parse_app_manifest(&text, &apps_dir.join("common")) {
                installed.push(game);
            }
        }
    }

    Some(GameLibrary {
        kind: LibraryKind::Steam,
        root: steam_root.to_path_buf(),
        install_roots,
        generated_roots,
        installed,
    })
}

/// Pull the fields Scuttle needs out of an `appmanifest_*.acf`.
pub fn parse_app_manifest(text: &str, common_dir: &Path) -> Option<GameInstall> {
    let fields: HashMap<String, String> = vdf::pairs(text)
        .into_iter()
        .map(|(k, v)| (k.to_lowercase(), v))
        .collect();

    let id = fields.get("appid")?.clone();
    let name = fields
        .get("name")
        .cloned()
        .or_else(|| fields.get("installdir").cloned())?;
    let install_dir = fields.get("installdir").map(|d| common_dir.join(d));

    Some(GameInstall {
        name,
        id,
        install_dir,
    })
}

/// A forgiving reader for Valve's key-value format.
///
/// Scuttle only ever needs flat `"key" "value"` pairs, so this deliberately
/// ignores nesting rather than building a tree it would not use.
pub mod vdf {
    /// Every quoted key/value pair in the document, in order.
    pub fn pairs(text: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for line in text.lines() {
            let mut strings = Vec::new();
            let mut chars = line.chars().peekable();
            let mut current: Option<String> = None;
            while let Some(c) = chars.next() {
                match c {
                    '"' => match current.take() {
                        Some(s) => strings.push(s),
                        None => current = Some(String::new()),
                    },
                    '\\' => {
                        // Valve escapes backslashes in Windows paths.
                        if let Some(next) = chars.next() {
                            if let Some(buf) = current.as_mut() {
                                match next {
                                    'n' => buf.push('\n'),
                                    't' => buf.push('\t'),
                                    other => buf.push(other),
                                }
                            }
                        }
                    }
                    other => {
                        if let Some(buf) = current.as_mut() {
                            buf.push(other);
                        }
                    }
                }
            }
            if strings.len() >= 2 {
                out.push((strings[0].clone(), strings[1].clone()));
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Epic Games
// ---------------------------------------------------------------------------

/// Epic records one JSON `.item` manifest per installed title.
pub fn read_epic_library(manifest_dir: &Path, root: &Path) -> Option<GameLibrary> {
    if !manifest_dir.is_dir() {
        return None;
    }
    let mut installed = Vec::new();
    let mut install_roots = Vec::new();

    for entry in std::fs::read_dir(manifest_dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "item").unwrap_or(true) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(game) = parse_epic_manifest(&text) {
            if let Some(dir) = &game.install_dir {
                if let Some(parent) = dir.parent() {
                    let parent = parent.to_path_buf();
                    if !install_roots.contains(&parent) {
                        install_roots.push(parent);
                    }
                }
            }
            installed.push(game);
        }
    }

    Some(GameLibrary {
        kind: LibraryKind::Epic,
        root: root.to_path_buf(),
        install_roots,
        generated_roots: Vec::new(),
        installed,
    })
}

pub fn parse_epic_manifest(text: &str) -> Option<GameInstall> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let name = value
        .get("DisplayName")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("AppName").and_then(|v| v.as_str()))?
        .to_string();
    let id = value
        .get("CatalogItemId")
        .or_else(|| value.get("AppName"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let install_dir = value
        .get("InstallLocation")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    Some(GameInstall {
        name,
        id,
        install_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
"AppState"
{
    "appid"        "1091500"
    "Universe"     "1"
    "name"         "Cyberpunk 2077"
    "StateFlags"   "4"
    "installdir"   "Cyberpunk 2077"
    "LastUpdated"  "1700000000"
    "SizeOnDisk"   "72000000000"
}
"#;

    #[test]
    fn an_app_manifest_yields_name_id_and_folder() {
        let game = parse_app_manifest(MANIFEST, Path::new("/steam/steamapps/common")).unwrap();
        assert_eq!(game.name, "Cyberpunk 2077");
        assert_eq!(game.id, "1091500");
        assert_eq!(
            game.install_dir.unwrap(),
            PathBuf::from("/steam/steamapps/common/Cyberpunk 2077")
        );
    }

    #[test]
    fn vdf_reads_escaped_windows_paths() {
        let text = r#"
"libraryfolders"
{
    "0"
    {
        "path"    "C:\\Program Files (x86)\\Steam"
    }
    "1"
    {
        "path"    "D:\\SteamLibrary"
    }
}
"#;
        let paths: Vec<String> = vdf::pairs(text)
            .into_iter()
            .filter(|(k, _)| k == "path")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(
            paths,
            vec![r"C:\Program Files (x86)\Steam", r"D:\SteamLibrary"]
        );
    }

    #[test]
    fn vdf_ignores_structure_it_does_not_need() {
        let pairs = vdf::pairs(MANIFEST);
        assert!(pairs
            .iter()
            .any(|(k, v)| k == "name" && v == "Cyberpunk 2077"));
        // Lines that only open a block produce nothing.
        assert!(!pairs.iter().any(|(k, _)| k == "AppState"));
    }

    #[test]
    fn a_truncated_manifest_is_skipped_rather_than_guessed_at() {
        assert!(
            parse_app_manifest("\"AppState\"\n{\n  \"appid\" \"1\"\n", Path::new("/x")).is_none()
        );
        assert!(parse_app_manifest("", Path::new("/x")).is_none());
    }

    #[test]
    fn an_epic_manifest_is_understood() {
        let text = r#"{
            "InstallLocation": "C:\\Program Files\\Epic Games\\Hades",
            "DisplayName": "Hades",
            "CatalogItemId": "abc123",
            "AppName": "Min"
        }"#;
        let game = parse_epic_manifest(text).unwrap();
        assert_eq!(game.name, "Hades");
        assert_eq!(game.id, "abc123");
        // The manifest records a Windows path; assert on what was stored
        // rather than on path semantics, which differ per host.
        assert_eq!(
            game.install_dir.unwrap().to_string_lossy(),
            r"C:\Program Files\Epic Games\Hades"
        );
    }

    #[test]
    fn malformed_epic_manifests_do_not_panic() {
        assert!(parse_epic_manifest("not json").is_none());
        assert!(parse_epic_manifest("{}").is_none());
    }

    #[test]
    fn a_library_recognises_its_own_installed_folders() {
        let library = GameLibrary {
            kind: LibraryKind::Steam,
            root: PathBuf::from("/steam"),
            install_roots: vec![],
            generated_roots: vec![],
            installed: vec![
                parse_app_manifest(MANIFEST, Path::new("/steam/steamapps/common")).unwrap(),
            ],
        };
        assert!(library.has_install_dir("Cyberpunk 2077"));
        assert!(library.has_install_dir("cyberpunk2077"));
        assert!(library.has_app_id("1091500"));
        assert!(!library.has_install_dir("Hades"));
    }

    #[test]
    fn a_steam_root_without_steamapps_is_not_a_library() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_steam_library(tmp.path()).is_none());
    }

    #[test]
    fn a_real_steam_tree_is_read_end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().join("steamapps");
        std::fs::create_dir_all(apps.join("common/Cyberpunk 2077")).unwrap();
        std::fs::write(apps.join("appmanifest_1091500.acf"), MANIFEST).unwrap();

        let library = read_steam_library(tmp.path()).unwrap();
        assert_eq!(library.installed.len(), 1);
        assert!(library.has_install_dir("Cyberpunk 2077"));
        assert!(library
            .generated_roots
            .iter()
            .any(|p| p.ends_with("shadercache")));
    }
}
