//! Positive references only. A missing match never proves an unused package.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use base64::Engine;
use quick_xml::{events::Event, Reader as XmlReader};
use serde::Serialize;

use super::{reader::Reader, CacheKind};

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub path: PathBuf,
    pub ecosystem: String,
    pub manifests: Vec<String>,
    /// Explicit even when all inspected manifests parsed successfully: these
    /// files do not cover every profile, plugin, branch or undiscovered repo.
    pub coverage_complete: bool,
    pub notes: Vec<String>,
    #[serde(skip)]
    references: Vec<(String, String)>,
    #[serde(skip)]
    integrities: Vec<(String, String, String)>,
}

pub fn discover(roots: &[PathBuf], reader: &mut Reader<'_>) -> Vec<Project> {
    let mut out = Vec::new();
    let mut pending: Vec<_> = crate::scanning::roots::deduplicate(roots.to_vec())
        .into_iter()
        .rev()
        .map(|p| (p, 0))
        .collect();
    while let Some((path, depth)) = pending.pop() {
        if !reader.available() {
            break;
        }
        let children = reader.children(&path);
        let mut is_project = false;
        for (ecosystem, markers) in [
            ("Maven", &["pom.xml"][..]),
            (
                "Gradle",
                &[
                    "build.gradle",
                    "build.gradle.kts",
                    "settings.gradle",
                    "settings.gradle.kts",
                ][..],
            ),
            ("Node", &["package.json"][..]),
        ] {
            let manifests: Vec<_> = markers
                .iter()
                .filter(|m| children.contains(&path.join(m)) && reader.file(&path.join(m)))
                .map(|m| (*m).to_string())
                .collect();
            if manifests.is_empty() {
                continue;
            }
            is_project = true;
            let mut project = Project {
                path: path.clone(),
                ecosystem: ecosystem.into(),
                manifests,
                coverage_complete: false,
                notes: vec![
                    "References are protective hints; dependency coverage is incomplete.".into(),
                ],
                references: Vec::new(),
                integrities: Vec::new(),
            };
            match ecosystem {
                "Maven" => {
                    if let Some(text) = reader.text(&path.join("pom.xml")) {
                        match maven_references(&text) {
                            Some(refs) => project.references = refs,
                            None => project.notes.push("The POM could not be parsed; no absence-of-reference claim is made.".into()),
                        }
                    }
                    project.notes.push("Effective POMs, inherited properties, transitive dependencies, plugins and profiles require Maven-aware resolution.".into());
                }
                "Gradle" => {
                    if let Some(text) = reader.text(&path.join("gradle.lockfile")) {
                        project.manifests.push("gradle.lockfile".into());
                        gradle_references(&text, &mut project.references);
                    }
                    for lock in reader.children(&path.join("gradle/dependency-locks")) {
                        if lock.extension().is_some_and(|e| e == "lockfile") {
                            if let Some(text) = reader.text(&lock) {
                                gradle_references(&text, &mut project.references);
                            }
                        }
                    }
                    if let Some(text) =
                        reader.text(&path.join("gradle/wrapper/gradle-wrapper.properties"))
                    {
                        project
                            .manifests
                            .push("gradle/wrapper/gradle-wrapper.properties".into());
                        for line in text.lines() {
                            if let Some(url) = line.trim().strip_prefix("distributionUrl=") {
                                let filename = url
                                    .split(['?', '#'])
                                    .next()
                                    .unwrap_or(url)
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or_default();
                                if let Some(dist) = filename.strip_suffix(".zip") {
                                    project
                                        .references
                                        .push(("Gradle distribution".into(), dist.into()));
                                }
                            }
                        }
                    }
                    project.notes.push("Build scripts are not executed; unlocked configurations and plugin dependencies remain unknown.".into());
                }
                _ => {
                    if let Some(text) = reader.text(&path.join("package.json")) {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            for section in [
                                "dependencies",
                                "devDependencies",
                                "optionalDependencies",
                                "peerDependencies",
                            ] {
                                if let Some(deps) = json.get(section).and_then(|v| v.as_object()) {
                                    for (name, spec) in deps {
                                        if let Some(version) =
                                            spec.as_str().filter(|v| exact_version(v))
                                        {
                                            project.references.push((name.clone(), version.into()));
                                        }
                                    }
                                }
                            }
                        } else {
                            project.notes.push(
                                "package.json is malformed; dependency coverage is unknown.".into(),
                            );
                        }
                    }
                    for manifest in ["package-lock.json", "npm-shrinkwrap.json"] {
                        if let Some(text) = reader.text(&path.join(manifest)) {
                            project.manifests.push(manifest.into());
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                npm_references(&json, &mut project.references);
                                npm_integrities(&json, &mut project.integrities);
                            } else {
                                project.notes.push(format!(
                                    "{manifest} is malformed; dependency coverage is unknown."
                                ));
                            }
                        }
                    }
                    for manifest in ["pnpm-lock.yaml", "yarn.lock"] {
                        if children.contains(&path.join(manifest)) {
                            project.manifests.push(manifest.into());
                            project.notes.push(format!("{manifest} is present; its references are not yet resolved by this preview."));
                        }
                    }
                    project.notes.push("Workspace links, version ranges and other branches are not fully resolved. node_modules is left alone.".into());
                }
            }
            project.references.sort();
            project.references.dedup();
            project.integrities.sort();
            project.integrities.dedup();
            out.push(project);
            if out.len() >= 500 {
                reader.incomplete("The project limit was reached.");
                return out;
            }
        }
        for child in children.into_iter().rev() {
            let name = child.file_name().unwrap_or_default().to_string_lossy();
            if (is_project
                && ["src", "test", "tests", "public", "resources", "gradle"]
                    .contains(&name.as_ref()))
                || name.starts_with('.')
                || ["node_modules", "target", "build", "dist", "vendor", "Pods"]
                    .contains(&name.as_ref())
            {
                continue;
            }
            if reader.directory(&child) {
                if depth < 6 {
                    pending.push((child, depth + 1));
                } else {
                    reader.incomplete("The project discovery depth limit was reached.");
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path).then(a.ecosystem.cmp(&b.ecosystem)));
    out
}

fn exact_version(v: &str) -> bool {
    !v.is_empty()
        && v.as_bytes()[0].is_ascii_digit()
        && !v.contains(['$', '^', '~', '*', ' ', '>', '<', '|'])
}

fn maven_references(text: &str) -> Option<Vec<(String, String)>> {
    let mut xml = XmlReader::from_str(text);
    let mut stack: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let mut coordinates: Vec<(usize, [String; 3])> = Vec::new();
    loop {
        match xml.read_event().ok()? {
            Event::Start(tag) => {
                let name = tag.local_name().as_ref().to_string();
                if ["dependency", "parent", "plugin", "extension"].contains(&name.as_str()) {
                    coordinates.push((stack.len() + 1, Default::default()));
                }
                stack.push(name);
            }
            Event::Text(value)
                if coordinates
                    .last()
                    .is_some_and(|(depth, _)| *depth + 1 == stack.len()) =>
            {
                let value = value.xml10_content().trim().to_string();
                let fields = &mut coordinates.last_mut()?.1;
                match stack.last()?.as_str() {
                    "groupId" => fields[0] = value,
                    "artifactId" => fields[1] = value,
                    "version" => fields[2] = value,
                    _ => {}
                }
            }
            Event::End(_) => {
                if coordinates
                    .last()
                    .is_some_and(|(depth, _)| *depth == stack.len())
                {
                    let (_, fields) = coordinates.pop()?;
                    if !fields[0].is_empty() && !fields[1].is_empty() && exact_version(&fields[2]) {
                        out.push((format!("{}:{}", fields[0], fields[1]), fields[2].clone()));
                    }
                }
                stack.pop()?;
            }
            Event::DocType(_) => return None,
            Event::Eof => return stack.is_empty().then_some(out),
            _ => {}
        }
    }
}
fn gradle_references(text: &str, out: &mut Vec<(String, String)>) {
    for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
        let coordinate = line.split('=').next().unwrap_or_default().trim();
        let parts: Vec<_> = coordinate.split(':').collect();
        if parts.len() == 3 && exact_version(parts[2]) {
            out.push((format!("{}:{}", parts[0], parts[1]), parts[2].into()));
        }
    }
}
fn npm_references(json: &serde_json::Value, out: &mut Vec<(String, String)>) {
    if let Some(packages) = json.get("packages").and_then(|v| v.as_object()) {
        for (path, package) in packages {
            if let Some(version) = package.get("version").and_then(|v| v.as_str()) {
                if let Some((_, name)) = path.rsplit_once("node_modules/") {
                    out.push((name.into(), version.into()));
                }
            }
        }
    }
    fn nested(json: &serde_json::Value, out: &mut Vec<(String, String)>, depth: usize) {
        if depth >= 64 {
            return;
        }
        if let Some(deps) = json.get("dependencies").and_then(|v| v.as_object()) {
            for (name, package) in deps {
                if let Some(version) = package.get("version").and_then(|v| v.as_str()) {
                    out.push((name.clone(), version.into()));
                }
                nested(package, out, depth + 1);
            }
        }
    }
    nested(json, out, 0);
}

fn npm_integrities(json: &serde_json::Value, out: &mut Vec<(String, String, String)>) {
    fn record(package: &serde_json::Value, name: &str, out: &mut Vec<(String, String, String)>) {
        let Some(version) = package.get("version").and_then(|v| v.as_str()) else {
            return;
        };
        let Some(integrity) = package.get("integrity").and_then(|v| v.as_str()) else {
            return;
        };
        for token in integrity.split_whitespace() {
            if let Some(encoded) = token.strip_prefix("sha512-") {
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                    if bytes.len() == 64 {
                        let hash: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                        out.push((hash, name.into(), version.into()));
                    }
                }
            }
        }
    }
    if let Some(packages) = json.get("packages").and_then(|v| v.as_object()) {
        for (path, package) in packages {
            if let Some((_, name)) = path.rsplit_once("node_modules/") {
                record(package, name, out);
            }
        }
    }
    fn nested(json: &serde_json::Value, out: &mut Vec<(String, String, String)>, depth: usize) {
        if depth >= 64 {
            return;
        }
        if let Some(deps) = json.get("dependencies").and_then(|v| v.as_object()) {
            for (name, package) in deps {
                record(package, name, out);
                nested(package, out, depth + 1);
            }
        }
    }
    nested(json, out, 0);
}

/// Build lookups once. Per-cache-row matching must not rescan every lockfile.
#[derive(Default)]
pub struct ReferenceIndex {
    coordinates: BTreeMap<(String, String, String), BTreeSet<PathBuf>>,
    integrities: BTreeMap<String, NpmIdentity>,
}
struct NpmIdentity {
    artifact: String,
    version: String,
    projects: BTreeSet<PathBuf>,
}
impl ReferenceIndex {
    pub fn new(projects: &[Project], reader: &mut Reader<'_>) -> Self {
        let mut index = Self::default();
        for project in projects {
            for (artifact, version) in &project.references {
                if !reader.available() {
                    return index;
                }
                index
                    .coordinates
                    .entry((project.ecosystem.clone(), artifact.clone(), version.clone()))
                    .or_default()
                    .insert(project.path.clone());
            }
            for (digest, artifact, version) in &project.integrities {
                if !reader.available() {
                    return index;
                }
                let identity =
                    index
                        .integrities
                        .entry(digest.clone())
                        .or_insert_with(|| NpmIdentity {
                            artifact: artifact.clone(),
                            version: version.clone(),
                            projects: BTreeSet::new(),
                        });
                identity.projects.insert(project.path.clone());
            }
        }
        index
    }
    pub fn references(
        &self,
        kind: CacheKind,
        artifact: &str,
        version: Option<&str>,
    ) -> Vec<PathBuf> {
        let Some(version) = version else {
            return Vec::new();
        };
        let ecosystem = match kind {
            CacheKind::Maven => "Maven",
            CacheKind::Gradle => "Gradle",
            _ => "Node",
        };
        self.coordinates
            .get(&(ecosystem.into(), artifact.into(), version.into()))
            .map(|paths| paths.iter().cloned().collect())
            .unwrap_or_default()
    }
    /// Lockfile integrity protects opaque npm blobs without reading their data.
    pub fn npm_identity(
        &self,
        path: &std::path::Path,
        repository: &std::path::Path,
    ) -> Option<(String, String, Vec<PathBuf>)> {
        let relative = path
            .strip_prefix(repository.join("content-v2/sha512"))
            .ok()?;
        let parts: Vec<_> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect();
        if parts.len() != 3 {
            return None;
        }
        self.integrities.get(&parts.join("")).map(|identity| {
            (
                identity.artifact.clone(),
                identity.version.clone(),
                identity.projects.iter().cloned().collect(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_or_external_xml_never_claims_coverage() {
        assert!(maven_references("<project><dependency>").is_none());
        assert!(maven_references("<!DOCTYPE project SYSTEM 'file:///secret'><project/>").is_none());
        let refs = maven_references("<project><dependencies><dependency><groupId>org.demo</groupId><artifactId>lib</artifactId><version>1.0</version></dependency></dependencies></project>").unwrap();
        assert_eq!(refs, vec![("org.demo:lib".into(), "1.0".into())]);
    }
    #[test]
    fn scoped_and_transitive_npm_packages_are_protected() {
        let json = serde_json::json!({"packages": {"node_modules/@scope/lib": {"version": "1.0.0"}, "node_modules/a/node_modules/b": {"version": "2.0.0"}}});
        let mut refs = Vec::new();
        npm_references(&json, &mut refs);
        assert!(refs.contains(&("@scope/lib".into(), "1.0.0".into())));
        assert!(refs.contains(&("b".into(), "2.0.0".into())));
    }

    #[test]
    fn nested_plugin_dependencies_do_not_replace_the_plugins_coordinates() {
        let refs = maven_references("<project><build><plugins><plugin><groupId>org.plugin</groupId><artifactId>build</artifactId><version>1.0</version><dependencies><dependency><groupId>org.dep</groupId><artifactId>lib</artifactId><version>2.0</version></dependency></dependencies></plugin></plugins></build></project>").unwrap();
        assert_eq!(
            refs,
            vec![
                ("org.dep:lib".into(), "2.0".into()),
                ("org.plugin:build".into(), "1.0".into())
            ]
        );
    }

    #[test]
    fn indexed_matching_keeps_shared_and_transitive_references_without_rescanning() {
        let mut projects = Vec::new();
        for project in ["/fixture/first", "/fixture/second"] {
            projects.push(Project {
                path: project.into(),
                ecosystem: "Node".into(),
                manifests: vec![],
                coverage_complete: false,
                notes: vec![],
                references: (0..2_000)
                    .map(|i| (format!("lib-{i}"), "1.0".into()))
                    .collect(),
                integrities: vec![("ab".repeat(64), "lib-1999".into(), "1.0".into())],
            });
        }
        let protected = crate::safety::ProtectedPaths::for_home("/fixture");
        let mut reader = Reader::new(&protected, &|| false);
        let index = ReferenceIndex::new(&projects, &mut reader);
        let expected: Vec<PathBuf> = vec!["/fixture/first".into(), "/fixture/second".into()];
        assert_eq!(
            index.references(CacheKind::Yarn, "lib-1999", Some("1.0")),
            expected
        );
        assert!(index
            .references(CacheKind::Gradle, "lib-1999", Some("1.0"))
            .is_empty());
        let path =
            std::path::Path::new("/fixture/npm/content-v2/sha512/ab/ab").join("ab".repeat(62));
        assert_eq!(
            index.npm_identity(&path, std::path::Path::new("/fixture/npm")),
            Some(("lib-1999".into(), "1.0".into(), expected))
        );
        let mut stopped = Reader::new(&protected, &|| true);
        assert!(ReferenceIndex::new(&projects, &mut stopped)
            .references(CacheKind::Npm, "lib-1999", Some("1.0"))
            .is_empty());
        assert!(!stopped.complete);
    }
}
