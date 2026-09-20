//! Dry run: what Scuttle *would* do, and why.
//!
//! Nothing here modifies anything. It exists so detector authors and anyone
//! debugging a surprising result can see the classification, the verdict and
//! the evidence behind it without having to trust a screenshot.

use serde::Serialize;

use crate::model::{human_bytes, Category, Confidence, RecommendedAction, Risk};
use crate::scanning::{ScanContext, ScanOptions, ScanOutcome};

#[derive(Debug, Clone, Serialize)]
pub struct DryRunRow {
    pub detector: String,
    pub category: Category,
    /// Deliberately the full path: this is the one place detailed paths are
    /// shown, and only because the user explicitly asked for them.
    pub path: String,
    pub display_name: String,
    pub size: u64,
    pub size_human: String,
    pub confidence: Confidence,
    pub risk: Risk,
    pub action: RecommendedAction,
    pub evidence: Vec<String>,
    pub negative_evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DryRunReport {
    pub roots: Vec<String>,
    pub developer_debris: bool,
    pub files_seen: u64,
    pub duration_ms: u64,
    /// Rows Scuttle would offer to move into the drawer.
    pub would_quarantine: Vec<DryRunRow>,
    /// Rows Scuttle would put in front of the user to decide.
    pub would_review: Vec<DryRunRow>,
    /// Rows Scuttle would only point at.
    pub would_surface: Vec<DryRunRow>,
    /// Protected-path rules that were consulted, so the table is auditable.
    pub protected_rules_active: usize,
    pub detectors_run: Vec<String>,
    pub notes: Vec<String>,
}

pub(crate) fn build(
    options: &ScanOptions,
    outcome: &ScanOutcome,
    ctx: &ScanContext,
) -> DryRunReport {
    let mut would_quarantine = Vec::new();
    let mut would_review = Vec::new();
    let mut would_surface = Vec::new();

    for candidate in &outcome.candidates {
        let row = DryRunRow {
            detector: candidate.detector.clone(),
            category: candidate.category,
            path: candidate.path.display().to_string(),
            display_name: candidate.display_name.clone(),
            size: candidate.size,
            size_human: human_bytes(candidate.size),
            confidence: candidate.confidence,
            risk: candidate.risk,
            action: candidate.recommended_action,
            evidence: candidate
                .evidence
                .iter()
                .filter(|e| !e.negative)
                .map(|e| e.summary.clone())
                .collect(),
            negative_evidence: candidate
                .evidence
                .iter()
                .filter(|e| e.negative)
                .map(|e| e.summary.clone())
                .collect(),
        };
        match candidate.recommended_action {
            RecommendedAction::Quarantine => would_quarantine.push(row),
            RecommendedAction::Review => would_review.push(row),
            RecommendedAction::InspectOnly => would_surface.push(row),
        }
    }

    let mut notes = vec![format!(
        "{} files looked at, {} findings, nothing modified.",
        outcome.summary.files_seen,
        outcome.candidates.len()
    )];
    if outcome.summary.hiccups.total() > 0 {
        notes.push(format!(
            "{} places could not be read ({} permission, {} vanished mid-walk).",
            outcome.summary.hiccups.total(),
            outcome.summary.hiccups.permission_denied,
            outcome.summary.hiccups.vanished
        ));
    }
    if ctx.processes.is_empty() {
        notes.push(
            "Could not read the process list, so no finding claims that nothing is using it."
                .into(),
        );
    }
    if ctx.apps.is_empty() {
        notes.push(
            "No installed applications were discovered — ghost detection cannot be trusted in this run."
                .into(),
        );
    }

    DryRunReport {
        roots: options
            .roots
            .iter()
            .map(|r| r.display().to_string())
            .collect(),
        developer_debris: options.include_developer_debris,
        files_seen: outcome.summary.files_seen,
        duration_ms: outcome.summary.duration_ms,
        would_quarantine,
        would_review,
        would_surface,
        protected_rules_active: 0,
        detectors_run: crate::detectors::default_set(options)
            .iter()
            .map(|d| d.id().to_string())
            .collect(),
        notes,
    }
}
