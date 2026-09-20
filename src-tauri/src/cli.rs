//! A small command-line surface for the dry run.
//!
//! Scuttle is a desktop application; this exists because "what would it do on
//! *this* machine, and why" is a question you want answerable from a terminal
//! — in CI, in a bug report, or while writing a detector. It classifies
//! everything and changes nothing.

use std::io::Write;

use crate::model::{human_bytes, RecommendedAction};
use crate::platform;
use crate::scanning::{self, IgnoreSet, ScanContext, ScanOptions, SilentObserver};

/// Returns true when a flag was handled and the process should exit.
pub fn handle_arguments() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return true;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("scuttle {}", env!("CARGO_PKG_VERSION"));
        return true;
    }
    if args.iter().any(|a| a == "--dry-run") {
        let developer_debris = args.iter().any(|a| a == "--developer-debris");
        dry_run(developer_debris);
        return true;
    }
    false
}

fn print_help() {
    println!(
        "Scuttle {} — your computer leaves stuff everywhere.

Run with no arguments to open the application.

  --dry-run              Rummage and report what Scuttle would do.
                         Changes nothing.
  --developer-debris     Include build output and package caches in the
                         dry run.
  --version              Print the version.
  --help                 This.

The dry run prints full paths. Everything else in Scuttle deliberately
does not; see docs/privacy.md.",
        env!("CARGO_PKG_VERSION")
    );
}

fn dry_run(developer_debris: bool) {
    let platform = platform::current();
    let roots: Vec<_> = platform
        .default_scan_roots()
        .into_iter()
        .map(|known| known.path)
        .collect();

    if roots.is_empty() {
        eprintln!("Scuttle found nowhere to look on this machine.");
        return;
    }

    let options = ScanOptions {
        roots,
        include_developer_debris: developer_debris,
        ..Default::default()
    };

    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ctx = ScanContext::new(
        options.clone(),
        std::sync::Arc::clone(&platform),
        IgnoreSet::default(),
        cancel,
    );

    println!("Rummaging through:");
    for root in &options.roots {
        println!("  {}", root.display());
    }
    println!("\nNothing will be modified.\n");
    let _ = std::io::stdout().flush();

    let detectors = crate::detectors::default_set(&options);
    let outcome = scanning::run("dry-run", &ctx, detectors, &SilentObserver);

    let mut sections: [(&str, Vec<_>); 3] = [
        ("Would quarantine", Vec::new()),
        ("Would put in front of you", Vec::new()),
        ("Would only point at", Vec::new()),
    ];
    for candidate in &outcome.candidates {
        let index = match candidate.recommended_action {
            RecommendedAction::Quarantine => 0,
            RecommendedAction::Review => 1,
            RecommendedAction::InspectOnly => 2,
        };
        sections[index].1.push(candidate);
    }

    for (title, mut candidates) in sections {
        println!("{title}: {}", candidates.len());
        candidates.sort_by_key(|c| std::cmp::Reverse(c.size));
        for candidate in candidates.iter().take(12) {
            println!(
                "\n  {}  ({})  [{}]",
                candidate.display_name,
                human_bytes(candidate.size),
                candidate.category.slug()
            );
            println!("    {}", candidate.path.display());
            println!(
                "    confidence {} · risk {} · found by {}",
                candidate.confidence.label(),
                candidate.risk.label(),
                candidate.detector
            );
            for reason in &candidate.evidence {
                println!(
                    "      {} {}",
                    if reason.negative { "✗" } else { "✓" },
                    reason.summary
                );
            }
        }
        if candidates.len() > 12 {
            println!("\n  … and {} more", candidates.len() - 12);
        }
        println!();
    }

    let summary = &outcome.summary;
    println!(
        "{} files looked at in {} ms. {} findings. Nothing modified.",
        summary.files_seen,
        summary.duration_ms,
        outcome.candidates.len()
    );
    println!(
        "  walk {} ms · probe {} ms · finish {} ms",
        summary.walk_ms, summary.probe_ms, summary.finish_ms
    );
    let hiccups = &outcome.summary.hiccups;
    if hiccups.total() > 0 {
        println!(
            "{} places could not be read ({} permission denied, {} vanished mid-walk, {} unreadable).",
            hiccups.total(),
            hiccups.permission_denied,
            hiccups.vanished,
            hiccups.unreadable
        );
    }
    if ctx.processes.is_empty() {
        println!("Could not read the process list, so nothing claims to be unused.");
    }
    if ctx.apps.is_empty() {
        println!("No installed applications discovered — ghost detection is untrustworthy here.");
    } else {
        println!("{} installed applications known.", ctx.apps.len());
    }
}
