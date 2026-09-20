//! Known caches.
//!
//! Scuttle does not have a rule that says "delete anything in a folder called
//! cache". Every entry below names an owner, says what happens when the data
//! is gone, and states whether the owner needs to be closed first. If Scuttle
//! cannot say those things about a directory, it does not treat it as a cache.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheSafety {
    /// Regenerated automatically; clearing it costs time, not data.
    Regenerates,
    /// Regenerated, but the owning application should not be running.
    RegeneratesWhenClosed,
    /// Known to be a cache, but clearing it loses something the user might
    /// miss (offline media, signed-in sessions). Surfaced, never recommended.
    CostsSomething,
}

impl CacheSafety {
    pub fn explanation(&self) -> &'static str {
        match self {
            CacheSafety::Regenerates => "Rebuilt automatically the next time it is needed.",
            CacheSafety::RegeneratesWhenClosed => {
                "Rebuilt automatically, but the app should be closed first."
            }
            CacheSafety::CostsSomething => {
                "This is a cache, but clearing it loses something you may want back."
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheRule {
    /// The application or tool that owns this directory.
    pub owner: &'static str,
    /// What to call the pile in the interface.
    pub label: &'static str,
    pub path: PathBuf,
    pub safety: CacheSafety,
    /// Process name to check before suggesting anything (lowercased).
    pub owner_process: Option<&'static str>,
    /// True when this belongs to the developer-debris opt-in rather than the
    /// default rummage.
    pub developer_only: bool,
}

impl CacheRule {
    pub fn matches(&self, path: &Path) -> bool {
        crate::safety::paths::is_within(path, &self.path)
    }
}

struct Spec {
    owner: &'static str,
    label: &'static str,
    rel: &'static str,
    safety: CacheSafety,
    owner_process: Option<&'static str>,
    developer_only: bool,
}

const fn spec(
    owner: &'static str,
    label: &'static str,
    rel: &'static str,
    safety: CacheSafety,
    owner_process: Option<&'static str>,
    developer_only: bool,
) -> Spec {
    Spec {
        owner,
        label,
        rel,
        safety,
        owner_process,
        developer_only,
    }
}

// Deliberately absent: Docker. Its credentials live in `~/.docker` and its
// images live inside `~/Library/Containers`, both of which are protected — and
// the image store is user data, not a cache.
#[cfg(target_os = "macos")]
const SPECS: &[Spec] = &[
    spec(
        "Xcode",
        "Xcode derived data",
        "Library/Developer/Xcode/DerivedData",
        CacheSafety::RegeneratesWhenClosed,
        Some("xcode"),
        true,
    ),
    spec(
        "Xcode",
        "iOS device support",
        "Library/Developer/Xcode/iOS DeviceSupport",
        CacheSafety::Regenerates,
        Some("xcode"),
        true,
    ),
    spec(
        "Xcode",
        "Simulator caches",
        "Library/Developer/CoreSimulator/Caches",
        CacheSafety::RegeneratesWhenClosed,
        Some("simulator"),
        true,
    ),
    spec(
        "Homebrew",
        "Homebrew downloads",
        "Library/Caches/Homebrew",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "npm",
        "npm package cache",
        ".npm/_cacache",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "Yarn",
        "Yarn cache",
        "Library/Caches/Yarn",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "pnpm",
        "pnpm store",
        "Library/pnpm/store",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "pip",
        "pip wheel cache",
        "Library/Caches/pip",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "Cargo",
        "Cargo registry cache",
        ".cargo/registry/cache",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "Gradle",
        "Gradle build cache",
        ".gradle/caches/build-cache-1",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "Go",
        "Go build cache",
        "Library/Caches/go-build",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "Spotify",
        "Spotify offline cache",
        "Library/Caches/com.spotify.client",
        CacheSafety::CostsSomething,
        Some("spotify"),
        false,
    ),
    spec(
        "Slack",
        "Slack cache",
        "Library/Application Support/Slack/Cache",
        CacheSafety::RegeneratesWhenClosed,
        Some("slack"),
        false,
    ),
    spec(
        "Slack",
        "Slack service worker cache",
        "Library/Application Support/Slack/Service Worker/CacheStorage",
        CacheSafety::RegeneratesWhenClosed,
        Some("slack"),
        false,
    ),
    spec(
        "Google Chrome",
        "Chrome cache",
        "Library/Caches/Google/Chrome",
        CacheSafety::RegeneratesWhenClosed,
        Some("google chrome"),
        false,
    ),
    spec(
        "Firefox",
        "Firefox cache",
        "Library/Caches/Firefox",
        CacheSafety::RegeneratesWhenClosed,
        Some("firefox"),
        false,
    ),
    spec(
        "Microsoft Edge",
        "Edge cache",
        "Library/Caches/Microsoft Edge",
        CacheSafety::RegeneratesWhenClosed,
        Some("microsoft edge"),
        false,
    ),
    spec(
        "Adobe",
        "Adobe media cache",
        "Library/Application Support/Adobe/Common/Media Cache Files",
        CacheSafety::Regenerates,
        None,
        false,
    ),
    spec(
        "Unity",
        "Unity asset cache",
        "Library/Unity/cache",
        CacheSafety::Regenerates,
        Some("unity"),
        true,
    ),
    spec(
        "Steam",
        "Steam shader cache",
        "Library/Application Support/Steam/steamapps/shadercache",
        CacheSafety::Regenerates,
        Some("steam"),
        false,
    ),
    spec(
        "Steam",
        "Steam download staging",
        "Library/Application Support/Steam/steamapps/downloading",
        CacheSafety::Regenerates,
        Some("steam"),
        false,
    ),
    spec(
        "Discord",
        "Discord cache",
        "Library/Application Support/discord/Cache",
        CacheSafety::RegeneratesWhenClosed,
        Some("discord"),
        false,
    ),
];

#[cfg(target_os = "windows")]
const SPECS: &[Spec] = &[
    spec(
        "Windows",
        "Temporary files",
        "AppData\\Local\\Temp",
        CacheSafety::Regenerates,
        None,
        false,
    ),
    spec(
        "Windows",
        "Crash dumps",
        "AppData\\Local\\CrashDumps",
        CacheSafety::Regenerates,
        None,
        false,
    ),
    spec(
        "Windows",
        "Delivery optimisation",
        "AppData\\Local\\Microsoft\\Windows\\DeliveryOptimization",
        CacheSafety::Regenerates,
        None,
        false,
    ),
    spec(
        "DirectX",
        "Shader cache",
        "AppData\\Local\\D3DSCache",
        CacheSafety::Regenerates,
        None,
        false,
    ),
    spec(
        "NVIDIA",
        "DirectX shader cache",
        "AppData\\Local\\NVIDIA\\DXCache",
        CacheSafety::Regenerates,
        None,
        false,
    ),
    spec(
        "NVIDIA",
        "OpenGL shader cache",
        "AppData\\Local\\NVIDIA\\GLCache",
        CacheSafety::Regenerates,
        None,
        false,
    ),
    spec(
        "AMD",
        "Shader cache",
        "AppData\\Local\\AMD\\DxCache",
        CacheSafety::Regenerates,
        None,
        false,
    ),
    spec(
        "npm",
        "npm package cache",
        "AppData\\Local\\npm-cache",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "Yarn",
        "Yarn cache",
        "AppData\\Local\\Yarn\\Cache",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "pip",
        "pip wheel cache",
        "AppData\\Local\\pip\\Cache",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "Cargo",
        "Cargo registry cache",
        ".cargo\\registry\\cache",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "Gradle",
        "Gradle build cache",
        ".gradle\\caches\\build-cache-1",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "NuGet",
        "NuGet package cache",
        ".nuget\\packages",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "Go",
        "Go build cache",
        "AppData\\Local\\go-build",
        CacheSafety::Regenerates,
        None,
        true,
    ),
    spec(
        "Visual Studio",
        "Visual Studio cache",
        "AppData\\Local\\Microsoft\\VisualStudio",
        CacheSafety::RegeneratesWhenClosed,
        Some("devenv"),
        true,
    ),
    spec(
        "Unity",
        "Unity asset cache",
        "AppData\\Local\\Unity\\cache",
        CacheSafety::Regenerates,
        Some("unity"),
        true,
    ),
    spec(
        "Google Chrome",
        "Chrome cache",
        "AppData\\Local\\Google\\Chrome\\User Data\\Default\\Cache",
        CacheSafety::RegeneratesWhenClosed,
        Some("chrome"),
        false,
    ),
    spec(
        "Firefox",
        "Firefox cache",
        "AppData\\Local\\Mozilla\\Firefox\\Profiles",
        CacheSafety::RegeneratesWhenClosed,
        Some("firefox"),
        false,
    ),
    spec(
        "Microsoft Edge",
        "Edge cache",
        "AppData\\Local\\Microsoft\\Edge\\User Data\\Default\\Cache",
        CacheSafety::RegeneratesWhenClosed,
        Some("msedge"),
        false,
    ),
    spec(
        "Discord",
        "Discord cache",
        "AppData\\Roaming\\discord\\Cache",
        CacheSafety::RegeneratesWhenClosed,
        Some("discord"),
        false,
    ),
    spec(
        "Spotify",
        "Spotify offline cache",
        "AppData\\Local\\Spotify\\Storage",
        CacheSafety::CostsSomething,
        Some("spotify"),
        false,
    ),
    spec(
        "Steam",
        "Steam shader cache",
        "AppData\\Local\\Steam\\htmlcache",
        CacheSafety::Regenerates,
        Some("steam"),
        false,
    ),
];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SPECS: &[Spec] = &[];

/// Resolve the table against a home directory, keeping only the rules whose
/// directory actually exists.
pub fn rules_for_home(home: &Path) -> Vec<CacheRule> {
    SPECS
        .iter()
        .map(|s| CacheRule {
            owner: s.owner,
            label: s.label,
            path: home.join(s.rel),
            safety: s.safety,
            owner_process: s.owner_process,
            developer_only: s.developer_only,
        })
        .filter(|rule| rule.path.is_dir())
        .collect()
}

/// The full table without the existence filter. Used by the dry-run report
/// and by tests.
pub fn all_rules_for_home(home: &Path) -> Vec<CacheRule> {
    SPECS
        .iter()
        .map(|s| CacheRule {
            owner: s.owner,
            label: s.label,
            path: home.join(s.rel),
            safety: s.safety,
            owner_process: s.owner_process,
            developer_only: s.developer_only,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::ProtectedPaths;

    /// A home directory shaped like the platform the test is running on.
    ///
    /// `/Users/testuser` is not an absolute path on Windows — an absolute
    /// path there needs a drive letter or a UNC prefix — so a literal Unix
    /// home made these assertions fail for a reason that had nothing to do
    /// with what they were checking.
    fn test_home(user: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:\\Users\\{user}"))
        } else {
            PathBuf::from(format!("/Users/{user}"))
        }
    }

    #[test]
    fn every_rule_names_an_owner_and_a_label() {
        for rule in all_rules_for_home(&test_home("testuser")) {
            assert!(!rule.owner.is_empty());
            assert!(!rule.label.is_empty());
            assert!(rule.path.is_absolute());
        }
    }

    #[test]
    fn no_cache_rule_points_at_a_protected_path() {
        // A rule that collided with the protected table would be a rule that
        // could never fire — and a sign someone had written a dangerous one.
        let home = test_home("testuser");
        let protected = ProtectedPaths::for_home(&home);
        for rule in all_rules_for_home(&home) {
            assert!(
                !protected.is_protected(&rule.path),
                "cache rule for {} points into protected territory: {}",
                rule.owner,
                rule.path.display()
            );
        }
    }

    #[test]
    fn no_cache_rule_is_dangerously_shallow() {
        let home = test_home("testuser");
        let protected = ProtectedPaths::for_home(&home);
        for rule in all_rules_for_home(&home) {
            assert!(
                !protected.is_too_shallow(&rule.path),
                "{} is too close to a root",
                rule.path.display()
            );
        }
    }

    #[test]
    fn rules_are_scoped_to_the_home_they_were_built_for() {
        let home = test_home("alice");
        let rules = all_rules_for_home(&home);
        for rule in rules {
            assert!(
                rule.path.starts_with(&home),
                "{} escaped the home directory",
                rule.path.display()
            );
        }
    }

    #[test]
    fn matching_uses_component_containment() {
        let home = test_home("testuser");
        let rules = all_rules_for_home(&home);
        if let Some(rule) = rules.first() {
            assert!(rule.matches(&rule.path));
            assert!(rule.matches(&rule.path.join("deep/inside")));
            assert!(!rule.matches(&home.join("somewhere-else")));
        }
    }

    #[test]
    fn nonexistent_directories_are_filtered_out() {
        let rules = rules_for_home(&test_home("definitely-not-a-real-user"));
        assert!(rules.is_empty());
    }

    #[test]
    fn every_safety_level_explains_itself() {
        for safety in [
            CacheSafety::Regenerates,
            CacheSafety::RegeneratesWhenClosed,
            CacheSafety::CostsSomething,
        ] {
            assert!(!safety.explanation().is_empty());
        }
    }
}
