//! What is installed on this machine, and how to recognise its leftovers.
//!
//! Ghost detection is an attribution problem: a directory called
//! `com.figma.Desktop` or `Cyberpunk 2077` is only interesting if we can say
//! with a straight face that the thing which owned it is gone. This module
//! does the matching and, crucially, is willing to answer "no idea".

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where the knowledge that an app exists came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppSource {
    /// A macOS `.app` bundle.
    Bundle,
    /// A Windows uninstall registry entry.
    Registry,
    /// Installed in a game library.
    GameLibrary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledApp {
    pub name: String,
    /// Reverse-DNS identifier, where the platform has one.
    pub bundle_id: Option<String>,
    pub publisher: Option<String>,
    pub install_location: Option<PathBuf>,
    pub source: AppSource,
}

/// Reduce a name to something comparable: lowercase, letters and digits only.
///
/// "Adobe Photoshop 2024.app", "adobe-photoshop-2024" and "AdobePhotoshop2024"
/// all collapse to the same token.
pub fn normalize_name(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        }
    }
    // Trailing platform noise that never distinguishes two applications.
    for suffix in ["app", "exe", "launcher", "helper"] {
        if out.len() > suffix.len() + 2 {
            if let Some(stripped) = out.strip_suffix(suffix) {
                return stripped.to_string();
            }
        }
    }
    out
}

/// Does this look like a reverse-DNS application identifier?
///
/// Deliberately strict. `com.figma.Desktop` yes; `my.notes` no; a folder
/// called `2024.backups` no.
pub fn looks_like_bundle_id(token: &str) -> bool {
    const KNOWN_PREFIXES: [&str; 8] = ["com", "org", "net", "io", "dev", "co", "app", "us"];
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 3 {
        return false;
    }
    if !KNOWN_PREFIXES.contains(&parts[0].to_lowercase().as_str()) {
        return false;
    }
    parts.iter().all(|p| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '~')
    })
}

/// The answer to "who did this belong to, and are they still here?"
#[derive(Debug, Clone, PartialEq)]
pub enum Attribution<'a> {
    /// The owner is installed. Leave its things alone.
    Installed(&'a InstalledApp),
    /// The token names something specific that is not installed.
    Missing {
        /// What to call it in the interface.
        display: String,
        /// The evidence-grade identifier, if the token was one.
        identifier: Option<String>,
    },
    /// Scuttle has no idea what this is. Oddments material.
    Unknown,
}

/// An index over the installed applications, built once per scan.
pub struct AppIndex {
    apps: Vec<InstalledApp>,
    by_bundle_id: HashMap<String, usize>,
    by_name: HashMap<String, usize>,
    /// Vendor segments of installed bundle ids ("figma" from
    /// "com.figma.Desktop"), used to match vendor-level data folders.
    vendors: HashMap<String, usize>,
}

impl AppIndex {
    pub fn new(apps: Vec<InstalledApp>) -> AppIndex {
        let mut by_bundle_id = HashMap::new();
        let mut by_name = HashMap::new();
        let mut vendors = HashMap::new();

        for (i, app) in apps.iter().enumerate() {
            if let Some(id) = &app.bundle_id {
                by_bundle_id.insert(id.to_lowercase(), i);
                if let Some(vendor) = bundle_vendor(id) {
                    vendors.entry(vendor).or_insert(i);
                }
            }
            by_name.entry(normalize_name(&app.name)).or_insert(i);
            if let Some(publisher) = &app.publisher {
                vendors.entry(normalize_name(publisher)).or_insert(i);
            }
        }

        AppIndex {
            apps,
            by_bundle_id,
            by_name,
            vendors,
        }
    }

    pub fn len(&self) -> usize {
        self.apps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    pub fn apps(&self) -> &[InstalledApp] {
        &self.apps
    }

    /// Attribute a directory or file name to an application.
    ///
    /// `token` is the raw name as it appears on disk — `com.figma.Desktop`,
    /// `Cyberpunk 2077`, `Slack`.
    pub fn attribute(&self, token: &str) -> Attribution<'_> {
        let lowered = token.to_lowercase();

        // 1. Exact bundle id.
        if let Some(&i) = self.by_bundle_id.get(&lowered) {
            return Attribution::Installed(&self.apps[i]);
        }

        // 2. Exact normalized name.
        let normalized = normalize_name(token);
        if !normalized.is_empty() {
            if let Some(&i) = self.by_name.get(&normalized) {
                return Attribution::Installed(&self.apps[i]);
            }
        }

        // 3. A bundle id whose vendor is installed. `com.figma.Agent` when
        //    Figma is present is still Figma's, even if the exact id differs.
        if looks_like_bundle_id(token) {
            if let Some(vendor) = bundle_vendor(token) {
                if let Some(&i) = self.vendors.get(&vendor) {
                    return Attribution::Installed(&self.apps[i]);
                }
            }
            // A well-formed identifier that matches nothing installed is the
            // strongest ghost signal there is.
            return Attribution::Missing {
                display: token.to_string(),
                identifier: Some(token.to_string()),
            };
        }

        // 4. A vendor-named folder ("Google", "JetBrains") whose vendor is here.
        if !normalized.is_empty() {
            if let Some(&i) = self.vendors.get(&normalized) {
                return Attribution::Installed(&self.apps[i]);
            }
        }

        // 5. A plain name that no installed app claims. Only treated as a
        //    named absence when it reads like a product name rather than a
        //    random folder — otherwise Scuttle says it does not know.
        if reads_like_a_product_name(token) {
            return Attribution::Missing {
                display: token.to_string(),
                identifier: None,
            };
        }

        Attribution::Unknown
    }
}

/// `com.figma.Desktop` -> `figma`.
fn bundle_vendor(id: &str) -> Option<String> {
    let parts: Vec<&str> = id.split('.').collect();
    parts.get(1).map(|v| normalize_name(v))
}

/// A heuristic with a deliberately narrow appetite. Folders that are clearly
/// user-shaped ("2019 taxes", "misc", "tmp") do not qualify, and neither do
/// single characters or pure numbers.
fn reads_like_a_product_name(token: &str) -> bool {
    const TOO_GENERIC: [&str; 14] = [
        "temp",
        "tmp",
        "misc",
        "stuff",
        "new folder",
        "untitled",
        "data",
        "backup",
        "backups",
        "old",
        "archive",
        "downloads",
        "documents",
        "test",
    ];
    let trimmed = token.trim();
    let lowered = trimmed.to_lowercase();
    if trimmed.len() < 3 || TOO_GENERIC.contains(&lowered.as_str()) {
        return false;
    }
    if trimmed.starts_with('.') {
        return false;
    }
    // Must contain at least one letter, and not be mostly punctuation.
    let letters = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 3 {
        return false;
    }
    // Capitalised or CamelCase names read like products; all-lowercase
    // single words usually do not carry enough signal on their own.
    let has_capital = trimmed.chars().any(|c| c.is_uppercase());
    let has_separator = trimmed.contains(' ') || trimmed.contains('-') || trimmed.contains('_');
    has_capital || has_separator
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, bundle: Option<&str>) -> InstalledApp {
        InstalledApp {
            name: name.into(),
            bundle_id: bundle.map(str::to_string),
            publisher: None,
            install_location: None,
            source: AppSource::Bundle,
        }
    }

    fn index() -> AppIndex {
        AppIndex::new(vec![
            app("Figma", Some("com.figma.Desktop")),
            app("Visual Studio Code", Some("com.microsoft.VSCode")),
            app("Steam", Some("com.valvesoftware.steam")),
        ])
    }

    #[test]
    fn normalization_collapses_cosmetic_differences() {
        assert_eq!(normalize_name("Visual Studio Code"), "visualstudiocode");
        assert_eq!(normalize_name("visual-studio-code"), "visualstudiocode");
        assert_eq!(normalize_name("VisualStudioCode.app"), "visualstudiocode");
        assert_eq!(normalize_name("Adobe Photoshop 2024"), "adobephotoshop2024");
    }

    #[test]
    fn normalization_does_not_eat_short_names() {
        // Stripping "app" from "App" would leave nothing useful.
        assert_eq!(normalize_name("App"), "app");
        assert_eq!(normalize_name("Exe"), "exe");
    }

    #[test]
    fn bundle_ids_are_recognised_conservatively() {
        assert!(looks_like_bundle_id("com.figma.Desktop"));
        assert!(looks_like_bundle_id("org.mozilla.firefox"));
        assert!(looks_like_bundle_id("com.valvesoftware.steam"));
        assert!(!looks_like_bundle_id("my.notes"));
        assert!(
            !looks_like_bundle_id("com~apple~CloudDocs"),
            "an iCloud container folder is not an application identifier"
        );
        assert!(
            !looks_like_bundle_id("2019.tax.returns"),
            "user folders are not bundle ids"
        );
        assert!(!looks_like_bundle_id("Cyberpunk 2077"));
    }

    #[test]
    fn an_installed_app_claims_its_own_identifier() {
        let idx = index();
        assert!(matches!(
            idx.attribute("com.figma.Desktop"),
            Attribution::Installed(a) if a.name == "Figma"
        ));
        assert!(matches!(
            idx.attribute("Visual Studio Code"),
            Attribution::Installed(a) if a.name == "Visual Studio Code"
        ));
    }

    #[test]
    fn a_vendor_still_present_protects_its_other_identifiers() {
        let idx = index();
        // Figma is installed; a helper id from the same vendor is not a ghost.
        assert!(matches!(
            idx.attribute("com.figma.Agent"),
            Attribution::Installed(a) if a.name == "Figma"
        ));
    }

    #[test]
    fn an_unmatched_bundle_id_is_a_named_absence() {
        let idx = index();
        match idx.attribute("com.sublimetext.4") {
            Attribution::Missing {
                display,
                identifier,
            } => {
                assert_eq!(display, "com.sublimetext.4");
                assert_eq!(identifier.as_deref(), Some("com.sublimetext.4"));
            }
            other => panic!("expected a named absence, got {other:?}"),
        }
    }

    #[test]
    fn a_product_shaped_folder_is_a_named_absence() {
        let idx = index();
        assert!(matches!(
            idx.attribute("Cyberpunk 2077"),
            Attribution::Missing { .. }
        ));
        assert!(matches!(
            idx.attribute("Unreal Engine"),
            Attribution::Missing { .. }
        ));
    }

    #[test]
    fn a_folder_scuttle_cannot_read_is_honestly_unknown() {
        let idx = index();
        // This is the important case: silence beats a confident guess.
        for token in ["tmp", "misc", "stuff", "data", "x", "2019", ".cache", "old"] {
            assert_eq!(
                idx.attribute(token),
                Attribution::Unknown,
                "{token} should not be attributed to anything"
            );
        }
    }

    #[test]
    fn an_empty_index_never_claims_something_is_installed() {
        let idx = AppIndex::new(vec![]);
        assert!(idx.is_empty());
        assert!(matches!(
            idx.attribute("com.figma.Desktop"),
            Attribution::Missing { .. }
        ));
    }

    #[test]
    fn publisher_names_shield_their_products() {
        let idx = AppIndex::new(vec![InstalledApp {
            name: "IntelliJ IDEA".into(),
            bundle_id: None,
            publisher: Some("JetBrains".into()),
            install_location: None,
            source: AppSource::Registry,
        }]);
        assert!(matches!(
            idx.attribute("JetBrains"),
            Attribution::Installed(_)
        ));
    }
}
