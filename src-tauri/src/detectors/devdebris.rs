//! Developer output is nominated by name, then verified before suggestion.
use super::{developer, naming::display_name};
use crate::evidence::EvidenceKind;
use crate::model::{human_bytes, Category, Risk};
use crate::scanning::{walk::FileEntry, CandidateSink, Detector, Finding, ScanContext};
use std::collections::HashSet;
use std::path::PathBuf;

const MIN_INTERESTING_BYTES: u64 = 100 * 1024 * 1024;
#[derive(Default)]
pub struct DeveloperDebrisDetector {
    found: HashSet<PathBuf>,
    preliminary: bool,
}
impl DeveloperDebrisDetector {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn preliminary() -> Self {
        Self {
            preliminary: true,
            ..Self::default()
        }
    }
}
impl Detector for DeveloperDebrisDetector {
    fn id(&self) -> &'static str {
        "developer_debris"
    }
    fn category(&self) -> Category {
        Category::DeveloperDebris
    }
    fn rummaging_note(&self) -> &'static str {
        "Checking what your build tools left"
    }
    fn enabled(&self, ctx: &ScanContext) -> bool {
        ctx.options.include_developer_debris
    }
    fn observe(&mut self, entry: &FileEntry, _ctx: &ScanContext) {
        if entry.is_dir
            && [
                "target",
                "node_modules",
                "build",
                "dist",
                ".next",
                ".nuxt",
                ".parcel-cache",
                "obj",
                "pods",
                ".venv",
                "vendor",
                ".gradle",
            ]
            .contains(&entry.name_lower())
        {
            self.found.insert(entry.path.clone());
        }
    }
    fn finish(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {
        let mut source_cache = developer::InspectionCache::default();
        let mut found: Vec<_> = std::mem::take(&mut self.found).into_iter().collect();
        found.sort();
        for path in found {
            if ctx.cancelled() {
                return;
            }
            // A user's Keep decision also prevents targeted reads.
            if ctx.ignores.covers(&path, None, Category::DeveloperDebris) {
                continue;
            }
            let Some(state) = developer::inspect(
                &path,
                &ctx.protected,
                &ctx.options.all_roots(),
                &ctx.processes,
                ctx.options.developer_stale_days,
                self.preliminary,
                &|| ctx.cancelled(),
                &mut source_cache,
            ) else {
                continue;
            };
            if state.output.bytes < MIN_INTERESTING_BYTES {
                continue;
            }
            let project = display_name(&state.project);
            let size = human_bytes(state.output.bytes);
            let prefix = if state.output.complete {
                ""
            } else {
                "At least "
            };
            let remark = if state.verified(ctx.now_unix) {
                format!("{prefix}{size} of {} for {project}.\n\nYour tools can rebuild this; the next build may take longer. Moving it to the drawer keeps it recoverable and does not free space yet.", state.artifact_kind.label())
            } else {
                format!(
                    "{prefix}{size} for {project}.\n\n{}",
                    state.caution(ctx.now_unix)
                )
            };
            let entry = FileEntry::for_directory(
                &path,
                state.output.bytes,
                state.output.newest_unix.unwrap_or(0),
            );
            let finding = Finding::new(
                "developer_debris",
                Category::DeveloperDebris,
                &entry,
                format!("{} · {project}", display_name(&path)),
            )
            .size(state.output.bytes)
            .risk(Risk::Low)
            .classified()
            .with(EvidenceKind::DeveloperArtifact {
                state: Box::new(state),
            });
            sink.emit(finding.saying(remark));
        }
    }
}

#[cfg(test)]
#[path = "developer_tests.rs"]
mod tests;
