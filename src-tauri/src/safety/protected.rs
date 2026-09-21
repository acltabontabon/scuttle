//! The protected-path table.
//!
//! A missed cleanup candidate costs a user some disk space. A destructive
//! false positive costs them their SSH keys, their password vault or their
//! thesis. The table below is written with that asymmetry in mind, and it is
//! consulted in three places: traversal (don't even look), classification
//! (refuse to recommend) and action (refuse to perform).

use std::path::{Path, PathBuf};

use super::paths::{self, is_within};

/// Why a path is off limits, in words a user can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub name: &'static str,
    pub scope: Scope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// This path and everything beneath it.
    Subtree(PathBuf),
    /// Any path having this exact component anywhere in it.
    AnyComponent(&'static str),
    /// A file whose name matches exactly.
    FileNameExact(&'static str),
    /// A file whose name ends with this (already lowercased).
    FileNameSuffix(&'static str),
}

/// The rule table, pre-folded for matching.
///
/// Rules are grouped by how they match, so a query touches only the
/// comparisons that can possibly apply, and every rule's own path is folded
/// once at construction rather than once per file examined. This is checked
/// for every entry in a scan; when it re-folded each rule's path per file it
/// was comfortably the hottest code in the program.
#[derive(Debug, Clone, Default)]
struct Compiled {
    /// (folded components of the root, index into `rules`)
    subtrees: Vec<(Vec<String>, usize)>,
    components: Vec<(String, usize)>,
    exact_names: Vec<(String, usize)>,
    suffixes: Vec<(String, usize)>,
}

impl Compiled {
    fn add(&mut self, rule: &Rule, index: usize) {
        match &rule.scope {
            Scope::Subtree(root) => self.subtrees.push((paths::folded_components(root), index)),
            Scope::AnyComponent(name) => self.components.push((name.to_lowercase(), index)),
            Scope::FileNameExact(name) => self.exact_names.push((name.to_lowercase(), index)),
            Scope::FileNameSuffix(suffix) => self.suffixes.push((suffix.to_lowercase(), index)),
        }
    }

    /// The index of the first matching rule, if any.
    fn first_match(&self, folded: &[String], name_lower: &str) -> Option<usize> {
        for (root, index) in &self.subtrees {
            if folded.len() >= root.len() && folded[..root.len()] == root[..] {
                return Some(*index);
            }
        }
        for (needle, index) in &self.components {
            if folded.iter().any(|component| component == needle) {
                return Some(*index);
            }
        }
        for (needle, index) in &self.exact_names {
            if name_lower == needle {
                return Some(*index);
            }
        }
        for (needle, index) in &self.suffixes {
            if name_lower.ends_with(needle) {
                return Some(*index);
            }
        }
        None
    }
}

/// Areas that are not forbidden, but where anything Scuttle finds is presumed
/// to be the user's own work until proven otherwise.
#[derive(Debug, Clone)]
pub struct SensitiveArea {
    pub reason: &'static str,
    pub root: PathBuf,
}

/// The resolved rule table for this machine.
#[derive(Debug, Clone)]
pub struct ProtectedPaths {
    rules: Vec<Rule>,
    compiled: Compiled,
    sensitive: Vec<SensitiveArea>,
    home: PathBuf,
}

impl ProtectedPaths {
    /// Build the table for a given home directory. Taking `home` as an
    /// argument rather than reading it from the environment is what makes
    /// this testable on any machine.
    pub fn for_home(home: impl Into<PathBuf>) -> ProtectedPaths {
        let home = home.into();
        let mut rules = Vec::new();
        let sub = |base: &Path, rel: &str| Scope::Subtree(base.join(rel));

        // ---- Credentials and keys, every platform -------------------------
        rules.push(Rule {
            name: "SSH keys",
            scope: sub(&home, ".ssh"),
        });
        rules.push(Rule {
            name: "GnuPG keyring",
            scope: sub(&home, ".gnupg"),
        });
        rules.push(Rule {
            name: "Cloud credentials",
            scope: sub(&home, ".aws"),
        });
        rules.push(Rule {
            name: "Cloud credentials",
            scope: sub(&home, ".azure"),
        });
        rules.push(Rule {
            name: "Cloud credentials",
            scope: sub(&home, ".config/gcloud"),
        });
        rules.push(Rule {
            name: "Kubernetes credentials",
            scope: sub(&home, ".kube"),
        });
        rules.push(Rule {
            name: "Container registry credentials",
            scope: sub(&home, ".docker"),
        });
        rules.push(Rule {
            name: "Password store",
            scope: sub(&home, ".password-store"),
        });
        rules.push(Rule {
            name: "Stored credentials",
            scope: Scope::FileNameExact(".netrc"),
        });
        rules.push(Rule {
            name: "Stored credentials",
            scope: Scope::FileNameExact("_netrc"),
        });
        rules.push(Rule {
            name: "Stored credentials",
            scope: Scope::FileNameExact(".git-credentials"),
        });
        rules.push(Rule {
            name: "Private key",
            scope: Scope::FileNameExact("id_rsa"),
        });
        rules.push(Rule {
            name: "Private key",
            scope: Scope::FileNameExact("id_ecdsa"),
        });
        rules.push(Rule {
            name: "Private key",
            scope: Scope::FileNameExact("id_ed25519"),
        });
        rules.push(Rule {
            name: "Private key",
            scope: Scope::FileNameSuffix(".pem"),
        });
        rules.push(Rule {
            name: "Private key",
            scope: Scope::FileNameSuffix(".key"),
        });
        rules.push(Rule {
            name: "Certificate bundle",
            scope: Scope::FileNameSuffix(".p12"),
        });
        rules.push(Rule {
            name: "Certificate bundle",
            scope: Scope::FileNameSuffix(".pfx"),
        });
        rules.push(Rule {
            name: "Password vault",
            scope: Scope::FileNameSuffix(".kdbx"),
        });
        rules.push(Rule {
            name: "Password vault",
            scope: Scope::FileNameSuffix(".kdb"),
        });
        rules.push(Rule {
            name: "Password vault",
            scope: Scope::FileNameSuffix(".opvault"),
        });
        rules.push(Rule {
            name: "Password vault",
            scope: Scope::FileNameSuffix(".agilekeychain"),
        });
        rules.push(Rule {
            name: "Password vault",
            scope: Scope::FileNameSuffix(".keychain"),
        });
        rules.push(Rule {
            name: "Password vault",
            scope: Scope::FileNameSuffix(".keychain-db"),
        });
        rules.push(Rule {
            name: "Encrypted volume",
            scope: Scope::FileNameSuffix(".sparsebundle"),
        });
        rules.push(Rule {
            name: "Wallet data",
            scope: sub(&home, ".electrum"),
        });
        rules.push(Rule {
            name: "Wallet data",
            scope: Scope::FileNameExact("wallet.dat"),
        });

        // ---- Version control internals ------------------------------------
        rules.push(Rule {
            name: "Repository internals",
            scope: Scope::AnyComponent(".git"),
        });
        rules.push(Rule {
            name: "Repository internals",
            scope: Scope::AnyComponent(".hg"),
        });
        rules.push(Rule {
            name: "Repository internals",
            scope: Scope::AnyComponent(".svn"),
        });
        rules.push(Rule {
            name: "Repository internals",
            scope: Scope::AnyComponent(".jj"),
        });

        // ---- Sync and backup infrastructure --------------------------------
        for dir in [
            "Dropbox",
            "OneDrive",
            "Google Drive",
            "Nextcloud",
            "Sync",
            "pCloud Drive",
        ] {
            rules.push(Rule {
                name: "Cloud sync folder",
                scope: Scope::Subtree(home.join(dir)),
            });
        }

        #[cfg(target_os = "macos")]
        {
            // OS-owned territory.
            for root in [
                "/System",
                "/bin",
                "/sbin",
                "/usr",
                "/private/etc",
                "/private/var/db",
                "/private/var/folders",
                "/Library/Apple",
                "/Library/Keychains",
                "/Library/LaunchDaemons",
                "/Library/LaunchAgents",
                "/Library/Extensions",
                "/cores",
                "/Network",
                "/.vol",
                "/dev",
            ] {
                rules.push(Rule {
                    name: "Part of macOS",
                    scope: Scope::Subtree(PathBuf::from(root)),
                });
            }
            // Installed software itself. Scuttle finds leftovers, it does not
            // uninstall applications.
            rules.push(Rule {
                name: "Installed applications",
                scope: Scope::Subtree(PathBuf::from("/Applications")),
            });
            rules.push(Rule {
                name: "Installed applications",
                scope: Scope::Subtree(home.join("Applications")),
            });
            rules.push(Rule {
                name: "Package manager",
                scope: Scope::Subtree(PathBuf::from("/opt/homebrew")),
            });
            rules.push(Rule {
                name: "Package manager",
                scope: Scope::Subtree(PathBuf::from("/nix")),
            });
            rules.push(Rule {
                name: "Other volumes",
                scope: Scope::Subtree(PathBuf::from("/Volumes")),
            });

            for rel in [
                "Library/Keychains",
                "Library/Cookies",
                "Library/Mail",
                "Library/Messages",
                "Library/Safari",
                "Library/Calendars",
                "Library/Reminders",
                "Library/Suggestions",
                "Library/Accounts",
                "Library/IdentityServices",
                "Library/Mobile Documents", // iCloud Drive
                "Library/CloudStorage",     // iCloud/Dropbox/OneDrive providers
                "Library/Photos",
                "Library/Application Support/AddressBook",
                "Library/Application Support/MobileSync", // iOS device backups
                "Library/Application Support/com.apple.TCC",
                "Library/Application Support/1Password",
                "Library/Application Support/Bitwarden",
                "Library/Application Support/KeePassXC",
                "Library/Group Containers",
                "Library/Containers",
            ] {
                rules.push(Rule {
                    name: system_rule_name(rel),
                    scope: sub(&home, rel),
                });
            }

            // Browser profiles hold sessions, passwords and history. Their
            // *caches* are handled by specific cache rules; the profiles are
            // not ours.
            for rel in [
                "Library/Application Support/Google/Chrome",
                "Library/Application Support/Firefox/Profiles",
                "Library/Application Support/BraveSoftware",
                "Library/Application Support/Microsoft Edge",
                "Library/Application Support/Arc",
            ] {
                rules.push(Rule {
                    name: "Browser profile",
                    scope: sub(&home, rel),
                });
            }
        }

        #[cfg(target_os = "windows")]
        {
            let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
            let d = |rel: &str| Scope::Subtree(PathBuf::from(format!("{system_drive}\\{rel}")));
            for rel in [
                "Windows",
                "Program Files",
                "Program Files (x86)",
                "ProgramData\\Microsoft",
                "$Recycle.Bin",
                "System Volume Information",
                "Recovery",
                "PerfLogs",
                "Boot",
            ] {
                rules.push(Rule {
                    name: "Part of Windows",
                    scope: d(rel),
                });
            }
            for name in [
                "pagefile.sys",
                "hiberfil.sys",
                "swapfile.sys",
                "ntuser.dat",
                "bootmgr",
            ] {
                rules.push(Rule {
                    name: "Part of Windows",
                    scope: Scope::FileNameExact(name),
                });
            }
            for rel in [
                "AppData\\Local\\Microsoft\\Credentials",
                "AppData\\Roaming\\Microsoft\\Credentials",
                "AppData\\Roaming\\Microsoft\\Crypto",
                "AppData\\Roaming\\Microsoft\\Protect",
                "AppData\\Roaming\\Microsoft\\SystemCertificates",
                "AppData\\Local\\Packages",
                "AppData\\Local\\Microsoft\\Windows\\SchCache",
            ] {
                rules.push(Rule {
                    name: "Windows credential storage",
                    scope: sub(&home, rel),
                });
            }
            for rel in [
                "AppData\\Local\\Google\\Chrome\\User Data",
                "AppData\\Roaming\\Mozilla\\Firefox\\Profiles",
                "AppData\\Local\\Microsoft\\Edge\\User Data",
                "AppData\\Local\\BraveSoftware",
            ] {
                rules.push(Rule {
                    name: "Browser profile",
                    scope: sub(&home, rel),
                });
            }
            rules.push(Rule {
                name: "Password vault",
                scope: sub(&home, "AppData\\Local\\1Password"),
            });
            rules.push(Rule {
                name: "Password vault",
                scope: sub(&home, "AppData\\Roaming\\Bitwarden"),
            });
        }

        // Areas that belong to the user rather than to their software.
        let sensitive = user_content_areas(&home);

        let mut compiled = Compiled::default();
        for (index, rule) in rules.iter().enumerate() {
            compiled.add(rule, index);
        }

        ProtectedPaths {
            rules,
            compiled,
            sensitive,
            home,
        }
    }

    /// Build the table for the current user.
    pub fn for_current_user() -> ProtectedPaths {
        ProtectedPaths::for_home(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
    }

    /// Add a rule at runtime. Used for Scuttle's own data directory, which
    /// lives inside an area Scuttle scans and must never appear in results.
    pub fn also_protect(&mut self, name: &'static str, path: impl Into<PathBuf>) {
        let rule = Rule {
            name,
            scope: Scope::Subtree(path.into()),
        };
        self.compiled.add(&rule, self.rules.len());
        self.rules.push(rule);
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// A protected place *inside* `dir`, if there is one that can be named
    /// without walking: a protected subtree, or the home folder itself.
    ///
    /// Moving a folder moves everything in it, so a folder that contains
    /// something protected is refused just as firmly as the protected thing.
    /// Rules that match by name anywhere (`.git`, `*.kdbx`) need a walk; see
    /// [`ProtectedPaths::first_protected_descendant`].
    pub fn protected_inside(&self, dir: &Path) -> Option<&'static str> {
        let dir = paths::normalize(dir);
        if paths::is_within(&self.home, &dir) {
            return Some("your home folder");
        }
        self.rules.iter().find_map(|rule| match &rule.scope {
            Scope::Subtree(root) if paths::is_strictly_within(root, &dir) => Some(rule.name),
            _ => None,
        })
    }

    /// Walk `dir` for anything the table protects. Links are not followed:
    /// a link inside a moved folder moves as a link, and what it points at
    /// stays where it is.
    ///
    /// Returns the rule's name and the offending path.
    pub fn first_protected_descendant(
        &self,
        dir: &Path,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<(&'static str, PathBuf)> {
        if let Some(name) = self.protected_inside(dir) {
            return Some((name, dir.to_path_buf()));
        }
        let walker = walkdir::WalkDir::new(dir).follow_links(false).into_iter();
        for entry in walker.flatten() {
            if entry.depth() == 0 {
                continue;
            }
            if cancelled() {
                return Some(("a check that was stopped", entry.path().to_path_buf()));
            }
            if let Some(rule) = self.rule_for(entry.path()) {
                return Some((rule.name, entry.path().to_path_buf()));
            }
        }
        None
    }

    /// The first rule that forbids this path, if any.
    ///
    /// Called for every entry in a scan, so the candidate path is folded once
    /// and compared against the pre-folded table.
    pub fn rule_for(&self, path: &Path) -> Option<&Rule> {
        let folded = paths::folded_components(path);
        let name_lower = folded.last().cloned().unwrap_or_default();
        self.compiled
            .first_match(&folded, &name_lower)
            .map(|index| &self.rules[index])
    }

    pub fn is_protected(&self, path: &Path) -> bool {
        self.rule_for(path).is_some()
    }

    /// Why this path counts as the user's own content, if it does.
    pub fn sensitivity(&self, path: &Path) -> Option<&'static str> {
        self.sensitive
            .iter()
            .find(|a| is_within(path, &a.root))
            .map(|a| a.reason)
    }

    /// Paths so close to a root that acting on them is never sensible,
    /// regardless of what any detector thinks.
    ///
    /// Removing `~/Library` or `C:\Users\me\AppData` would be catastrophic and
    /// no evidence could justify it.
    pub fn is_too_shallow(&self, path: &Path) -> bool {
        let normalized = paths::normalize(path);
        // Named folders only: a drive prefix (`C:`) and the root separator
        // are not depth. Counting them made `C:\Users` and `C:\Windows` three
        // components deep on Windows, one more than the limit below — so the
        // limit that holds `/Users` never held them.
        let depth = normalized
            .components()
            .filter(|c| matches!(c, std::path::Component::Normal(_)))
            .count();
        // A filesystem root or anything directly beneath one: `/`, `/Users`,
        // `C:\`, `C:\Users`, `D:\Games`.
        if depth <= 1 {
            return true;
        }
        // The root of a mounted drive is as structural as `C:\`.
        if depth == 2 {
            let first = normalized
                .components()
                .find_map(|c| match c {
                    std::path::Component::Normal(name) => {
                        Some(name.to_string_lossy().to_lowercase())
                    }
                    _ => None,
                })
                .unwrap_or_default();
            if matches!(first.as_str(), "volumes" | "mnt" | "media" | "run") {
                return true;
            }
        }
        match paths::depth_below(&normalized, &self.home) {
            // The home directory itself, or a direct child of it that is a
            // well-known top-level folder.
            Some(0) => true,
            Some(1) => TOP_LEVEL_HOME_FOLDERS
                .iter()
                .any(|f| paths::file_name_lower(&normalized) == *f),
            _ => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn rule_names(&self) -> Vec<&'static str> {
        self.rules.iter().map(|r| r.name).collect()
    }
}

/// Direct children of home that are structural, not disposable.
const TOP_LEVEL_HOME_FOLDERS: [&str; 12] = [
    "library",
    "documents",
    "desktop",
    "downloads",
    "pictures",
    "movies",
    "music",
    "videos",
    "public",
    "appdata",
    "applications",
    "sites",
];

fn user_content_areas(home: &Path) -> Vec<SensitiveArea> {
    let mut areas = vec![
        SensitiveArea {
            reason: "it is in Documents",
            root: home.join("Documents"),
        },
        SensitiveArea {
            reason: "it is on the Desktop",
            root: home.join("Desktop"),
        },
        SensitiveArea {
            reason: "it is in Pictures",
            root: home.join("Pictures"),
        },
        SensitiveArea {
            reason: "it is in Music",
            root: home.join("Music"),
        },
        SensitiveArea {
            reason: "it is in Movies",
            root: home.join("Movies"),
        },
        SensitiveArea {
            reason: "it is in Videos",
            root: home.join("Videos"),
        },
        SensitiveArea {
            reason: "it is in a projects folder",
            root: home.join("Projects"),
        },
        SensitiveArea {
            reason: "it is in a projects folder",
            root: home.join("Workspace"),
        },
        SensitiveArea {
            reason: "it is in a projects folder",
            root: home.join("Developer"),
        },
        SensitiveArea {
            reason: "it is in a projects folder",
            root: home.join("src"),
        },
        SensitiveArea {
            reason: "it is in a projects folder",
            root: home.join("code"),
        },
    ];
    areas.retain(|a| a.root != *home);
    areas
}

#[cfg(target_os = "macos")]
fn system_rule_name(rel: &str) -> &'static str {
    if rel.contains("1Password") || rel.contains("Bitwarden") || rel.contains("KeePass") {
        "Password vault"
    } else if rel.contains("MobileSync") {
        "Device backups"
    } else if rel.contains("Mobile Documents") || rel.contains("CloudStorage") {
        "Cloud sync folder"
    } else if rel.contains("Containers") {
        "Sandboxed application data"
    } else {
        "Personal data managed by macOS"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> ProtectedPaths {
        ProtectedPaths::for_home("/Users/testuser")
    }

    #[test]
    fn credentials_are_protected_on_every_platform() {
        let t = table();
        for p in [
            "/Users/testuser/.ssh/id_ed25519",
            "/Users/testuser/.ssh",
            "/Users/testuser/.gnupg/secring.gpg",
            "/Users/testuser/.aws/credentials",
            "/Users/testuser/.kube/config",
            "/Users/testuser/.password-store/site.gpg",
            "/Users/testuser/Downloads/vault.kdbx",
            "/Users/testuser/Downloads/server.pem",
            "/Users/testuser/Downloads/.netrc",
            "/Users/testuser/anywhere/wallet.dat",
        ] {
            assert!(t.is_protected(Path::new(p)), "{p} should be protected");
        }
    }

    #[test]
    fn repository_internals_are_protected_wherever_they_live() {
        let t = table();
        assert!(t.is_protected(Path::new("/Users/testuser/code/proj/.git/objects/ab/cd")));
        assert!(t.is_protected(Path::new("/tmp/whatever/.hg/store")));
        // ...but the working tree itself is not blanket-protected, or we could
        // never surface a stale build directory.
        assert!(!t.is_protected(Path::new("/Users/testuser/code/proj/target")));
    }

    #[test]
    fn cloud_sync_folders_are_protected() {
        let t = table();
        assert!(t.is_protected(Path::new("/Users/testuser/Dropbox/taxes/2019.pdf")));
        assert!(t.is_protected(Path::new("/Users/testuser/OneDrive/x")));
    }

    #[test]
    fn a_protected_rule_reports_why() {
        let t = table();
        let rule = t
            .rule_for(Path::new("/Users/testuser/.ssh/config"))
            .unwrap();
        assert_eq!(rule.name, "SSH keys");
    }

    #[test]
    fn ordinary_rummaging_ground_is_not_protected() {
        let t = table();
        for p in [
            "/Users/testuser/Downloads/Chrome.dmg",
            "/Users/testuser/Downloads/old-game-cache",
            "/Users/testuser/Library/Caches/com.example.app",
        ] {
            assert!(!t.is_protected(Path::new(p)), "{p} should be reachable");
        }
    }

    #[test]
    fn structural_folders_are_too_shallow_to_act_on() {
        let t = table();
        for p in [
            "/",
            "/Users",
            "/Users/testuser",
            "/Users/testuser/Library",
            "/Users/testuser/Downloads",
            "/Users/testuser/Documents",
        ] {
            assert!(t.is_too_shallow(Path::new(p)), "{p} must never be a target");
        }
        assert!(!t.is_too_shallow(Path::new("/Users/testuser/Downloads/thing.dmg")));
        assert!(!t.is_too_shallow(Path::new("/Users/testuser/Library/Caches/x")));
    }

    #[test]
    fn user_content_areas_are_flagged_but_reachable() {
        let t = table();
        assert_eq!(
            t.sensitivity(Path::new("/Users/testuser/Desktop/Screenshot.png")),
            Some("it is on the Desktop")
        );
        assert_eq!(
            t.sensitivity(Path::new("/Users/testuser/Documents/thesis.docx")),
            Some("it is in Documents")
        );
        assert_eq!(
            t.sensitivity(Path::new("/Users/testuser/Library/Caches/x")),
            None
        );
        // Flagged, not forbidden — the screenshot detector still needs to see it.
        assert!(!t.is_protected(Path::new("/Users/testuser/Desktop/Screenshot.png")));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_system_territory_is_protected() {
        let t = table();
        for p in [
            "/System/Library/CoreServices",
            "/usr/bin/env",
            "/Applications/Safari.app",
            "/Library/Keychains/System.keychain",
            "/Users/testuser/Library/Mobile Documents/com~apple~CloudDocs/x",
            "/Users/testuser/Library/Containers/com.apple.Notes",
            "/Users/testuser/Library/Application Support/MobileSync/Backup",
            "/Users/testuser/Library/Application Support/Google/Chrome/Default/Login Data",
            "/Volumes/Backup/anything",
        ] {
            assert!(t.is_protected(Path::new(p)), "{p} should be protected");
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn case_variations_do_not_defeat_the_table() {
        let t = table();
        assert!(t.is_protected(Path::new("/Users/TestUser/.SSH/ID_RSA")));
        assert!(t.is_protected(Path::new("/SYSTEM/Library")));
    }

    #[test]
    fn a_runtime_rule_protects_scuttles_own_drawer() {
        let mut t = table();
        let drawer = "/Users/testuser/Library/Application Support/Scuttle";
        assert!(!t.is_protected(Path::new(drawer)));
        t.also_protect("Scuttle's own files", drawer);
        assert!(t.is_protected(Path::new(drawer)));
        assert!(t.is_protected(Path::new(&format!("{drawer}/Quarantine/abc/thing.dmg"))));
    }

    #[test]
    fn no_rule_is_nameless() {
        assert!(table().rule_names().iter().all(|n| !n.is_empty()));
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_and_its_top_folders_are_structural_on_windows() {
        let table = ProtectedPaths::for_home(r"C:\Users\tester");
        for path in [r"C:\", r"C:\Users", r"C:\Windows", r"D:\Games", r"c:\users"] {
            assert!(table.is_too_shallow(Path::new(path)), "{path}");
        }
        assert!(!table.is_too_shallow(Path::new(r"D:\Games\Old")));
    }

    #[cfg(unix)]
    #[test]
    fn a_mounted_drive_root_is_structural() {
        let table = ProtectedPaths::for_home("/Users/tester");
        assert!(table.is_too_shallow(Path::new("/Volumes/Backup")));
        assert!(table.is_too_shallow(Path::new("/Users")));
        assert!(!table.is_too_shallow(Path::new("/Volumes/Backup/old")));
    }

    #[test]
    fn a_folder_holding_something_protected_is_known_to() {
        let table = ProtectedPaths::for_home("/home/tester");
        assert!(table.protected_inside(Path::new("/home")).is_some());
        assert!(table
            .protected_inside(Path::new("/home/tester/Downloads"))
            .is_none());
    }
}
