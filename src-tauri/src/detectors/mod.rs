//! Detectors.
//!
//! Each detector answers one question and produces evidence for its answer.
//! None of them delete anything, decide their own confidence, or walk the
//! filesystem on their own unless they genuinely need to look somewhere
//! specific.
//!
//! Adding one means implementing [`crate::scanning::Detector`] and adding it
//! to [`default_set`]. See `docs/writing-a-detector.md`.

pub mod caches;
pub mod devdebris;
pub mod duplicates;
pub mod ghosts;
pub mod heavy;
pub mod installers;
pub mod screenshots;

#[cfg(test)]
pub mod test_support;

use crate::scanning::{Detector, ScanOptions};

/// The detectors a normal rummage runs, in the order their notes appear.
pub fn default_set(options: &ScanOptions) -> Vec<Box<dyn Detector>> {
    let mut set: Vec<Box<dyn Detector>> = vec![
        Box::new(ghosts::GhostDetector::new()),
        Box::new(installers::InstallerDetector::new()),
        Box::new(screenshots::ScreenshotDetector::new()),
        Box::new(duplicates::DuplicateDetector::new()),
        Box::new(caches::CacheDetector::new()),
        Box::new(heavy::HeavyStrayDetector::new()),
    ];
    if options.include_developer_debris {
        set.push(Box::new(devdebris::DeveloperDebrisDetector::new()));
    }
    set
}

/// Naming helpers shared by several detectors.
pub mod naming {
    /// Reduce an installer or download file name to the product it installs.
    ///
    /// `Slack-4.35.126-macOS.dmg`, `Slack (2).dmg` and `slack_4.36.dmg` all
    /// collapse to `slack`, which is what lets Scuttle notice that you
    /// downloaded the same thing five times.
    pub fn product_key(stem: &str) -> String {
        const NOISE: [&str; 20] = [
            "setup",
            "installer",
            "install",
            "universal",
            "x64",
            "x86",
            "amd64",
            "arm64",
            "aarch64",
            "win",
            "win32",
            "win64",
            "windows",
            "mac",
            "macos",
            "osx",
            "darwin",
            "intel",
            "full",
            "offline",
        ];

        let mut cleaned = String::with_capacity(stem.len());
        let mut depth = 0i32;
        for ch in stem.chars() {
            // Drop parenthesised duplicate counters: "Chrome (1)".
            match ch {
                '(' | '[' => {
                    depth += 1;
                    continue;
                }
                ')' | ']' => {
                    depth = (depth - 1).max(0);
                    continue;
                }
                _ => {}
            }
            if depth > 0 {
                continue;
            }
            cleaned.push(ch);
        }

        let tokens: Vec<String> = cleaned
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase())
            .collect();

        let kept: Vec<String> = tokens
            .into_iter()
            .filter(|t| {
                // Version numbers and build identifiers.
                if t.chars().all(|c| c.is_ascii_digit()) {
                    return false;
                }
                if t.starts_with('v') && t[1..].chars().all(|c| c.is_ascii_digit()) && t.len() > 1 {
                    return false;
                }
                !NOISE.contains(&t.as_str())
            })
            .collect();

        if kept.is_empty() {
            // Everything was noise; fall back to the original so two unrelated
            // files never collapse into the same empty key.
            return stem.to_lowercase();
        }
        kept.join("")
    }

    /// A display name that reads like something a person would say.
    pub fn display_name(path: &std::path::Path) -> String {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::naming::*;

    #[test]
    fn versions_and_counters_collapse_to_one_product() {
        let key = product_key("Slack-4.35.126-macOS");
        for variant in ["Slack (1)", "slack_4.36.0", "Slack-4.40.0-macOS", "SLACK"] {
            assert_eq!(product_key(variant), key, "{variant} should match {key}");
        }
    }

    #[test]
    fn different_products_stay_distinct() {
        assert_ne!(product_key("Chrome"), product_key("Firefox"));
        assert_ne!(product_key("node-v20.10.0-x64"), product_key("npm"));
    }

    #[test]
    fn architecture_and_platform_noise_is_ignored() {
        assert_eq!(product_key("Docker-arm64"), product_key("Docker-x64"));
        assert_eq!(
            product_key("VSCodeSetup-x64-1.85.0"),
            product_key("VSCodeSetup")
        );
    }

    #[test]
    fn a_name_made_entirely_of_noise_keeps_its_identity() {
        // Otherwise "setup.exe" and "installer.exe" would look like copies of
        // each other and Scuttle would claim a duplicate that is not one.
        assert_ne!(product_key("setup"), product_key("installer"));
        assert_eq!(product_key("setup"), "setup");
    }

    #[test]
    fn version_tokens_with_a_v_prefix_are_dropped() {
        assert_eq!(product_key("node-v20.10.0"), "node");
        // ...but a real word starting with v is not.
        assert_eq!(product_key("vim-9.0"), "vim");
    }
}
