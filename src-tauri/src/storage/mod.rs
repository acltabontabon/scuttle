//! Local persistence, behind a repository boundary.
//!
//! Scuttle stores what it needs to answer "what did you find, what did I
//! decide, and what is still recoverable" — and nothing else. There is no
//! index of every file on the machine. Everything here stays on the machine.

pub mod entries;
pub mod migrations;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::ScuttleError;
use crate::evidence::{Evidence, EvidenceKind};
use crate::model::{
    Category, CleanupCandidate, Confidence, GroupMember, RecommendedAction, Risk, StateFingerprint,
    TargetKind,
};
use crate::scanning::{IgnoreKind, IgnoreSet, ScanOptions, ScanSummary};
pub use entries::{journal, JournalEntry, SnapshotEntry, SnapshotInfo, SnapshotState};

use crate::Result;

/// The user's preferences. Small on purpose.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Empty means "wherever the platform says is sensible".
    ///
    /// There is deliberately no separate exclusion list: telling Scuttle to
    /// leave a particular path alone is what the ignore list is for, and that
    /// has an interface. Two overlapping mechanisms would mean two places to
    /// look when Scuttle does not do what you expected.
    pub scan_roots: Vec<PathBuf>,
    pub include_developer_debris: bool,
    /// 7, 14 or 30.
    pub quarantine_retention_days: u32,
    pub heavy_threshold: u64,
    /// "system", "light" or "dark".
    pub appearance: String,
    /// `None` follows the operating system.
    pub reduced_motion: Option<bool>,
    pub has_rummaged_before: bool,

    // ---- staying in the menu bar / system tray -------------------------
    //
    // Each of these three depends on the one above it, and `save_settings`
    // enforces that rather than trusting the caller: a toggle that promises
    // something the app cannot deliver is worse than no toggle.
    /// Closing the window leaves Scuttle running behind its tray icon.
    pub background_mode: bool,
    /// Look around occasionally, on its own. Needs `background_mode`.
    pub background_checks: bool,
    /// Say when a background check turned something new up. Needs
    /// `background_checks`.
    pub background_notify: bool,
    /// Start at login, through the platform's own per-user mechanism. This is
    /// a mirror for the interface; the truth is whatever the OS reports.
    pub launch_at_login: bool,
    /// Whether the one-off "Scuttle stays in the menu bar now" note has been
    /// shown. Explaining it once is the point.
    pub background_intro_seen: bool,

    // ---- updates -------------------------------------------------------
    /// Look for a newer Scuttle on its own, shortly after starting and then
    /// about once a day. Looking is all it does: nothing is downloaded until
    /// the person says so, and nothing is installed without a restart they
    /// agreed to.
    pub auto_check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            scan_roots: Vec::new(),
            include_developer_debris: false,
            quarantine_retention_days: 14,
            heavy_threshold: 1024 * 1024 * 1024,
            appearance: "system".into(),
            reduced_motion: None,
            has_rummaged_before: false,
            background_mode: false,
            background_checks: false,
            background_notify: false,
            launch_at_login: false,
            background_intro_seen: false,
            auto_check_updates: true,
        }
    }
}

/// What the background scheduler needs to remember between ticks — and
/// between runs, so that restarting Scuttle does not earn it a fresh check.
///
/// Deliberately not part of [`Settings`]: none of it is a preference, and it
/// changes far more often than anything the user chose.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BackgroundState {
    /// The last check that ran to completion without being cancelled. The
    /// once-a-day gate is measured from here.
    pub last_completed_unix: i64,
    /// The last check that started, however it ended.
    pub last_attempt_unix: i64,
    /// The last time a summary was sent, for the notification cooldown.
    pub last_notified_unix: i64,
    /// "Pause until tomorrow", persisted so it survives a restart.
    pub paused_until_unix: i64,
    /// When the scheduler last woke. A gap much larger than the tick interval
    /// means the machine slept, which is a reason to settle rather than to
    /// catch up.
    pub last_tick_unix: i64,
    /// When the machine was last noticed to have woken. Checks stay quiet for
    /// a while after this — long enough that waking a laptop is never followed
    /// shortly by a scan.
    pub woke_unix: i64,
    /// The scan id of a completed check the user has not looked at yet.
    pub pending_review: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuarantineStatus {
    /// Sitting in the drawer, recoverable.
    Held,
    /// Put back where it came from.
    Restored,
    /// Gone for good.
    Removed,
    /// A move into the drawer is in progress, or was interrupted. Invisible to
    /// every listing until it is finalised or reconciled, so a half-finished
    /// move is never presented as held content.
    Moving,
    /// A restore is in progress, or was interrupted.
    Restoring,
    /// A move that put nothing in the drawer.
    Abandoned,
}

impl QuarantineStatus {
    fn as_str(&self) -> &'static str {
        match self {
            QuarantineStatus::Held => "held",
            QuarantineStatus::Restored => "restored",
            QuarantineStatus::Removed => "removed",
            QuarantineStatus::Moving => "moving",
            QuarantineStatus::Restoring => "restoring",
            QuarantineStatus::Abandoned => "abandoned",
        }
    }
    fn parse(s: &str) -> QuarantineStatus {
        match s {
            "restored" => QuarantineStatus::Restored,
            "removed" => QuarantineStatus::Removed,
            "moving" => QuarantineStatus::Moving,
            "restoring" => QuarantineStatus::Restoring,
            "abandoned" => QuarantineStatus::Abandoned,
            _ => QuarantineStatus::Held,
        }
    }
}

/// What a drawer record holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordMode {
    /// One item that moved whole: a file, or a directory renamed in one step.
    #[default]
    Whole,
    /// The reviewed files of a shared folder (a cache root). The folder itself
    /// never moved; the record holds exactly the files listed for it.
    Contents,
}

impl RecordMode {
    fn as_str(&self) -> &'static str {
        match self {
            RecordMode::Whole => "whole",
            RecordMode::Contents => "contents",
        }
    }
    fn parse(s: &str) -> RecordMode {
        match s {
            "contents" => RecordMode::Contents,
            _ => RecordMode::Whole,
        }
    }
}

/// One thing in the drawer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineRecord {
    pub id: String,
    pub finding_id: Option<String>,
    pub original_path: PathBuf,
    pub stored_path: PathBuf,
    pub display_name: String,
    pub category: Category,
    pub size: u64,
    pub content_hash: Option<String>,
    /// The reasons Scuttle gave at the time, frozen. If the detector changes
    /// its mind later, the record of what the user was told does not.
    pub evidence: Vec<Evidence>,
    pub quarantined_unix: i64,
    pub expires_unix: i64,
    pub status: QuarantineStatus,
    pub resolved_unix: Option<i64>,
    #[serde(default)]
    pub mode: RecordMode,
    /// Files held. One for a whole item.
    #[serde(default = "one")]
    pub item_count: u64,
    /// True when an interrupted move left something Scuttle could not settle on
    /// its own. Nothing is deleted on that account; the person is told.
    #[serde(default)]
    pub attention: bool,
}

fn one() -> u64 {
    1
}

/// Who asked for a scan, and therefore how much it was allowed to do.
///
/// A [`ScanKind::Glance`] never opens a file: it runs the detectors that work
/// from names, sizes and dates, and leaves duplicate hashing and screenshot
/// decoding to a rummage the user started. That makes it cheap enough to run
/// unattended, and it also makes it incomplete — which is why this is stored
/// rather than inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanKind {
    /// A rummage someone asked for.
    Full,
    /// A background check.
    Glance,
}

impl ScanKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScanKind::Full => "full",
            ScanKind::Glance => "glance",
        }
    }

    /// Anything unrecognised is treated as a rummage: over-reporting how
    /// thorough a scan was is the dangerous direction, so an unknown value
    /// from a newer version should not silently become "glance".
    pub fn parse(raw: &str) -> ScanKind {
        match raw {
            "glance" => ScanKind::Glance,
            _ => ScanKind::Full,
        }
    }
}

/// A summary of a past rummage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanRecord {
    pub id: String,
    pub kind: ScanKind,
    pub started_unix: i64,
    pub finished_unix: Option<i64>,
    pub roots: Vec<PathBuf>,
    pub files_seen: u64,
    pub bytes_seen: u64,
    pub candidates_found: u64,
    pub reclaimable_bytes: u64,
    pub duration_ms: u64,
    pub cancelled: bool,
    /// Places the scan could not read. Persisted all along, but never read
    /// back, so the findings screen reported a clean sweep every time even
    /// when half a home folder had been refused.
    pub hiccups: crate::scanning::HiccupSummary,
}

/// Something the user did, kept so the app can say what happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub action: String,
    pub display_name: String,
    pub category: Category,
    pub size: u64,
    pub at_unix: i64,
    pub outcome: String,
}

/// The repository. Every SQL statement in Scuttle lives behind this type.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        Store::prepare(&mut conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    pub fn in_memory() -> Result<Store> {
        let mut conn = Connection::open_in_memory()?;
        Store::prepare(&mut conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    fn prepare(conn: &mut Connection) -> Result<()> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;
        migrations::apply(conn)?;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A poisoned lock means another thread panicked mid-write. The
        // database itself is transactional, so recovering the guard is safe
        // and far better than taking the whole app down.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- scans ---------------------------------------------------------

    pub fn begin_scan(
        &self,
        scan_id: &str,
        kind: ScanKind,
        options: &ScanOptions,
        started_unix: i64,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO scan_runs (id, kind, started_unix, roots) VALUES (?1, ?2, ?3, ?4)",
            params![
                scan_id,
                kind.as_str(),
                started_unix,
                serde_json::to_string(&options.roots)?
            ],
        )?;
        Ok(())
    }

    pub fn finish_scan(&self, summary: &ScanSummary, finished_unix: i64) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE scan_runs SET finished_unix = ?2, files_seen = ?3, bytes_seen = ?4,
                candidates_found = ?5, reclaimable_bytes = ?6, duration_ms = ?7,
                cancelled = ?8, hiccups = ?9
             WHERE id = ?1",
            params![
                summary.scan_id,
                finished_unix,
                summary.files_seen,
                summary.bytes_seen,
                summary.candidates_found,
                summary.reclaimable_bytes,
                summary.duration_ms,
                summary.cancelled as i32,
                serde_json::to_string(&summary.hiccups)?,
            ],
        )?;
        Ok(())
    }

    /// The most recent completed rummage.
    pub fn latest_scan(&self) -> Result<Option<ScanRecord>> {
        let conn = self.lock();
        let record = conn
            .query_row(
                "SELECT id, started_unix, finished_unix, roots, files_seen, bytes_seen,
                        candidates_found, reclaimable_bytes, duration_ms, cancelled, hiccups,
                        kind
                 FROM scan_runs WHERE finished_unix IS NOT NULL
                 ORDER BY started_unix DESC LIMIT 1",
                [],
                |row| {
                    Ok(ScanRecord {
                        id: row.get(0)?,
                        kind: ScanKind::parse(&row.get::<_, String>(11)?),
                        started_unix: row.get(1)?,
                        finished_unix: row.get(2)?,
                        roots: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                        files_seen: row.get(4)?,
                        bytes_seen: row.get(5)?,
                        candidates_found: row.get(6)?,
                        reclaimable_bytes: row.get(7)?,
                        duration_ms: row.get(8)?,
                        cancelled: row.get::<_, i32>(9)? != 0,
                        hiccups: row
                            .get::<_, Option<String>>(10)?
                            .and_then(|raw| serde_json::from_str(&raw).ok())
                            .unwrap_or_default(),
                    })
                },
            )
            .optional()?;
        Ok(record)
    }

    /// Keep the last few scans and drop the rest. Scuttle is not an archive.
    pub fn prune_scans(&self, keep: usize) -> Result<usize> {
        let conn = self.lock();
        let removed = conn.execute(
            "DELETE FROM scan_runs WHERE id NOT IN (
                 SELECT id FROM scan_runs ORDER BY started_unix DESC LIMIT ?1
             )",
            params![keep as i64],
        )?;
        Ok(removed)
    }

    // ---- findings ------------------------------------------------------

    pub fn save_candidates(&self, scan_id: &str, candidates: &[CleanupCandidate]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for candidate in candidates {
            tx.execute(
                "INSERT OR REPLACE INTO findings (id, scan_id, detector, category, target_kind,
                    path, display_name, associated_app, size, confidence, risk,
                    recommended_action, remark, modified_unix, accessed_unix, created_unix,
                    fingerprint, group_members)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
                params![
                    candidate.id,
                    scan_id,
                    candidate.detector,
                    candidate.category.slug(),
                    target_kind_str(candidate.target_kind),
                    path_to_text(&candidate.path),
                    candidate.display_name,
                    candidate.associated_app,
                    candidate.size,
                    confidence_str(candidate.confidence),
                    risk_str(candidate.risk),
                    action_str(candidate.recommended_action),
                    candidate.remark,
                    candidate.modified_unix,
                    candidate.accessed_unix,
                    candidate.created_unix,
                    serde_json::to_string(&candidate.fingerprint)?,
                    serde_json::to_string(&candidate.group)?,
                ],
            )?;
            for (position, evidence) in candidate.evidence.iter().enumerate() {
                tx.execute(
                    "INSERT OR REPLACE INTO finding_evidence
                        (finding_id, position, kind, summary, weight, negative, risk_floor)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        candidate.id,
                        position as i64,
                        serde_json::to_string(&evidence.kind)?,
                        evidence.summary,
                        evidence.weight,
                        evidence.negative as i32,
                        evidence.risk_floor.map(risk_str),
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn candidates_for_scan(&self, scan_id: &str) -> Result<Vec<CleanupCandidate>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT id, detector, category, target_kind, path, display_name, associated_app,
                    size, confidence, risk, recommended_action, remark, modified_unix,
                    accessed_unix, created_unix, fingerprint, group_members
             FROM findings WHERE scan_id = ?1 ORDER BY size DESC",
        )?;
        let rows = statement.query_map(params![scan_id], row_to_candidate)?;
        let mut out = Vec::new();
        for row in rows {
            let mut candidate = row?;
            candidate.evidence = read_evidence(&conn, &candidate.id)?;
            out.push(candidate);
        }
        Ok(out)
    }

    pub fn candidate(&self, id: &str) -> Result<CleanupCandidate> {
        let conn = self.lock();
        let mut candidate = conn
            .query_row(
                "SELECT id, detector, category, target_kind, path, display_name, associated_app,
                        size, confidence, risk, recommended_action, remark, modified_unix,
                        accessed_unix, created_unix, fingerprint, group_members
                 FROM findings WHERE id = ?1",
                params![id],
                row_to_candidate,
            )
            .optional()?
            .ok_or_else(|| ScuttleError::not_found("That finding"))?;
        candidate.evidence = read_evidence(&conn, id)?;
        Ok(candidate)
    }

    /// Forget a finding entirely, so a "keep" decision does not leave it
    /// sitting in the results.
    pub fn forget_candidate(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM findings WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---- ignores -------------------------------------------------------

    pub fn ignore_path(&self, path: &Path, now_unix: i64) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR REPLACE INTO ignored_paths (path, created_unix) VALUES (?1, ?2)",
            params![path_to_text(path), now_unix],
        )?;
        Ok(())
    }

    pub fn ignore_app(&self, name: &str, now_unix: i64) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR REPLACE INTO ignored_apps (name, created_unix) VALUES (?1, ?2)",
            params![name.to_lowercase(), now_unix],
        )?;
        Ok(())
    }

    pub fn ignore_category(&self, category: Category, now_unix: i64) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR REPLACE INTO ignored_categories (category, created_unix) VALUES (?1, ?2)",
            params![category.slug(), now_unix],
        )?;
        Ok(())
    }

    /// Stop ignoring one entry, whichever kind it is.
    ///
    /// The three tables are keyed by their own value — a path, a lowercased
    /// app name, a category slug — so one delete per kind is all this needs.
    /// Nothing here touches a file: an ignore is only a note saying "do not
    /// mention this again", and removing it makes the thing eligible to turn
    /// up in the next rummage.
    pub fn unignore(&self, kind: IgnoreKind, value: &str) -> Result<()> {
        let conn = self.lock();
        let sql = match kind {
            IgnoreKind::Path => "DELETE FROM ignored_paths WHERE path = ?1",
            IgnoreKind::App => "DELETE FROM ignored_apps WHERE name = ?1",
            IgnoreKind::Category => "DELETE FROM ignored_categories WHERE category = ?1",
        };
        conn.execute(sql, params![value])?;
        Ok(())
    }

    pub fn clear_ignores(&self) -> Result<()> {
        let conn = self.lock();
        conn.execute_batch(
            "DELETE FROM ignored_paths; DELETE FROM ignored_apps; DELETE FROM ignored_categories;",
        )?;
        Ok(())
    }

    pub fn ignore_set(&self) -> Result<IgnoreSet> {
        let conn = self.lock();
        let collect = |sql: &str| -> Result<Vec<String>> {
            let mut statement = conn.prepare(sql)?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.filter_map(std::result::Result::ok).collect())
        };
        Ok(IgnoreSet {
            paths: collect("SELECT path FROM ignored_paths")?
                .into_iter()
                .map(PathBuf::from)
                .collect(),
            apps: collect("SELECT name FROM ignored_apps")?,
            categories: collect("SELECT category FROM ignored_categories")?
                .iter()
                .filter_map(|s| Category::from_slug(s))
                .collect(),
        })
    }

    // ---- quarantine ----------------------------------------------------

    pub fn insert_quarantine(&self, record: &QuarantineRecord) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO quarantine_items (id, finding_id, original_path, stored_path,
                display_name, category, size, content_hash, evidence, quarantined_unix,
                expires_unix, status, resolved_unix, mode, item_count, attention)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                record.id,
                record.finding_id,
                path_to_text(&record.original_path),
                path_to_text(&record.stored_path),
                record.display_name,
                record.category.slug(),
                record.size,
                record.content_hash,
                serde_json::to_string(&record.evidence)?,
                record.quarantined_unix,
                record.expires_unix,
                record.status.as_str(),
                record.resolved_unix,
                record.mode.as_str(),
                record.item_count,
                record.attention as i32,
            ],
        )?;
        Ok(())
    }

    pub fn quarantine_record(&self, id: &str) -> Result<QuarantineRecord> {
        let conn = self.lock();
        conn.query_row(
            "SELECT id, finding_id, original_path, stored_path, display_name, category, size,
                    content_hash, evidence, quarantined_unix, expires_unix, status, resolved_unix,
                    mode, item_count, attention
             FROM quarantine_items WHERE id = ?1",
            params![id],
            row_to_quarantine,
        )
        .optional()?
        .ok_or_else(|| ScuttleError::not_found("That quarantined item"))
    }

    /// Everything still in the drawer, newest first.
    pub fn held_quarantine(&self) -> Result<Vec<QuarantineRecord>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT id, finding_id, original_path, stored_path, display_name, category, size,
                    content_hash, evidence, quarantined_unix, expires_unix, status, resolved_unix,
                    mode, item_count, attention
             FROM quarantine_items WHERE status = 'held' ORDER BY quarantined_unix DESC",
        )?;
        let rows = statement.query_map([], row_to_quarantine)?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    pub fn expired_quarantine(&self, now_unix: i64) -> Result<Vec<QuarantineRecord>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT id, finding_id, original_path, stored_path, display_name, category, size,
                    content_hash, evidence, quarantined_unix, expires_unix, status, resolved_unix,
                    mode, item_count, attention
             FROM quarantine_items WHERE status = 'held' AND attention = 0 AND expires_unix <= ?1",
        )?;
        let rows = statement.query_map(params![now_unix], row_to_quarantine)?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    pub fn set_quarantine_status(
        &self,
        id: &str,
        status: QuarantineStatus,
        resolved_unix: i64,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE quarantine_items SET status = ?2, resolved_unix = ?3 WHERE id = ?1",
            params![id, status.as_str(), resolved_unix],
        )?;
        Ok(())
    }

    // ---- history -------------------------------------------------------

    pub fn record_history(&self, entry: &HistoryEntry) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO cleanup_history (id, action, display_name, category, size, at_unix, outcome)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                entry.id,
                entry.action,
                entry.display_name,
                entry.category.slug(),
                entry.size,
                entry.at_unix,
                entry.outcome,
            ],
        )?;
        Ok(())
    }

    pub fn history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT id, action, display_name, category, size, at_unix, outcome
             FROM cleanup_history ORDER BY at_unix DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                action: row.get(1)?,
                display_name: row.get(2)?,
                category: Category::from_slug(&row.get::<_, String>(3)?)
                    .unwrap_or(Category::Oddments),
                size: row.get(4)?,
                at_unix: row.get(5)?,
                outcome: row.get(6)?,
            })
        })?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    // ---- settings ------------------------------------------------------

    pub fn settings(&self) -> Result<Settings> {
        let conn = self.lock();
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'settings'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    /// Fold the write-ahead log into the database file.
    ///
    /// Nothing depends on this for correctness — committed writes are already
    /// durable in the log — but before Scuttle is replaced by a newer version
    /// it leaves the database as one whole file for that version to open.
    pub fn checkpoint(&self) -> Result<()> {
        let conn = self.lock();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('settings', ?1)",
            params![serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    /// Where everything currently in the drawer came from.
    ///
    /// Used to keep a background check from announcing something the user has
    /// already decided about and put away.
    pub fn quarantined_paths(&self) -> Result<Vec<PathBuf>> {
        let conn = self.lock();
        let mut stmt =
            conn.prepare("SELECT original_path FROM quarantine_items WHERE status = 'held'")?;
        let rows = stmt.query_map([], |row| Ok(PathBuf::from(row.get::<_, String>(0)?)))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    // ---- background scheduling ------------------------------------------

    /// Reads back as `Default` if it is missing or unreadable, which means a
    /// corrupt row costs at most one skipped day, never a burst of checks.
    pub fn background_state(&self) -> Result<BackgroundState> {
        let conn = self.lock();
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'background'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    pub fn save_background_state(&self, state: &BackgroundState) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('background', ?1)",
            params![serde_json::to_string(state)?],
        )?;
        Ok(())
    }

    /// Which of these keys Scuttle has not mentioned before.
    ///
    /// Reading and writing are separate on purpose: a check that ends badly,
    /// or is not worth mentioning, must not burn the keys it saw. Only
    /// [`Store::remember_notified`] does that, and only once something has
    /// actually been said.
    pub fn unseen_notices(&self, keys: &[String]) -> Result<Vec<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT 1 FROM background_seen WHERE key = ?1")?;
        let mut fresh = Vec::new();
        for key in keys {
            let known = stmt
                .query_row(params![key], |_| Ok(()))
                .optional()?
                .is_some();
            if !known {
                fresh.push(key.clone());
            }
        }
        Ok(fresh)
    }

    pub fn remember_notified(&self, keys: &[String], now_unix: i64) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for key in keys {
            tx.execute(
                "INSERT OR REPLACE INTO background_seen (key, seen_unix) VALUES (?1, ?2)",
                params![key, now_unix],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Forget notices older than `older_than_unix`, so the table does not
    /// grow without bound. A file that comes back after six months is worth
    /// mentioning again.
    pub fn prune_notices(&self, older_than_unix: i64) -> Result<usize> {
        let conn = self.lock();
        Ok(conn.execute(
            "DELETE FROM background_seen WHERE seen_unix < ?1",
            params![older_than_unix],
        )?)
    }
}

// ---- row mapping --------------------------------------------------------

fn row_to_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<CleanupCandidate> {
    let size: u64 = row.get(7)?;
    let group =
        serde_json::from_str::<Vec<GroupMember>>(&row.get::<_, String>(16)?).unwrap_or_default();

    Ok(CleanupCandidate {
        id: row.get(0)?,
        detector: row.get(1)?,
        category: Category::from_slug(&row.get::<_, String>(2)?).unwrap_or(Category::Oddments),
        target_kind: parse_target_kind(&row.get::<_, String>(3)?),
        path: PathBuf::from(row.get::<_, String>(4)?),
        display_name: row.get(5)?,
        associated_app: row.get(6)?,
        size,
        // Derived rather than stored: no column to migrate, and no chance of
        // it disagreeing with the group it is a sum of.
        group_bytes: crate::model::group_footprint(&group, size),
        confidence: parse_confidence(&row.get::<_, String>(8)?),
        risk: parse_risk(&row.get::<_, String>(9)?),
        recommended_action: parse_action(&row.get::<_, String>(10)?),
        remark: row.get(11)?,
        modified_unix: row.get(12)?,
        accessed_unix: row.get(13)?,
        created_unix: row.get(14)?,
        fingerprint: serde_json::from_str::<StateFingerprint>(&row.get::<_, String>(15)?)
            .unwrap_or_default(),
        group,
        evidence: Vec::new(),
    })
}

fn read_evidence(conn: &Connection, finding_id: &str) -> Result<Vec<Evidence>> {
    let mut statement = conn.prepare(
        "SELECT kind, summary, weight, negative, risk_floor
         FROM finding_evidence WHERE finding_id = ?1 ORDER BY position",
    )?;
    let rows = statement.query_map(params![finding_id], |row| {
        let kind: String = row.get(0)?;
        Ok(Evidence {
            kind: serde_json::from_str::<EvidenceKind>(&kind).unwrap_or(
                EvidenceKind::Unclassified {
                    reason: "a reason Scuttle can no longer read".into(),
                },
            ),
            summary: row.get(1)?,
            weight: row.get(2)?,
            negative: row.get::<_, i32>(3)? != 0,
            risk_floor: row.get::<_, Option<String>>(4)?.map(|s| parse_risk(&s)),
        })
    })?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
}

pub(crate) fn row_to_quarantine(row: &rusqlite::Row<'_>) -> rusqlite::Result<QuarantineRecord> {
    Ok(QuarantineRecord {
        id: row.get(0)?,
        finding_id: row.get(1)?,
        original_path: PathBuf::from(row.get::<_, String>(2)?),
        stored_path: PathBuf::from(row.get::<_, String>(3)?),
        display_name: row.get(4)?,
        category: Category::from_slug(&row.get::<_, String>(5)?).unwrap_or(Category::Oddments),
        size: row.get(6)?,
        content_hash: row.get(7)?,
        evidence: serde_json::from_str(&row.get::<_, String>(8)?).unwrap_or_default(),
        quarantined_unix: row.get(9)?,
        expires_unix: row.get(10)?,
        status: QuarantineStatus::parse(&row.get::<_, String>(11)?),
        resolved_unix: row.get(12)?,
        mode: RecordMode::parse(&row.get::<_, String>(13)?),
        item_count: row.get::<_, i64>(14)?.max(0) as u64,
        attention: row.get::<_, i64>(15)? != 0,
    })
}

/// Paths are stored lossily-but-stably. A path that cannot round-trip through
/// UTF-8 is rare, and the safety layer re-validates the stored text against
/// the live filesystem before anything is acted on.
fn path_to_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn target_kind_str(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::File => "file",
        TargetKind::Directory => "directory",
    }
}
fn parse_target_kind(s: &str) -> TargetKind {
    match s {
        "directory" => TargetKind::Directory,
        _ => TargetKind::File,
    }
}
fn confidence_str(c: Confidence) -> &'static str {
    match c {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}
fn parse_confidence(s: &str) -> Confidence {
    match s {
        "high" => Confidence::High,
        "medium" => Confidence::Medium,
        _ => Confidence::Low,
    }
}
fn risk_str(r: Risk) -> &'static str {
    match r {
        Risk::Low => "low",
        Risk::Moderate => "moderate",
        Risk::High => "high",
        Risk::Protected => "protected",
    }
}
fn parse_risk(s: &str) -> Risk {
    match s {
        "low" => Risk::Low,
        "moderate" => Risk::Moderate,
        "high" => Risk::High,
        // An unreadable risk level defaults to the most cautious answer.
        _ => Risk::Protected,
    }
}
fn action_str(a: RecommendedAction) -> &'static str {
    match a {
        RecommendedAction::Quarantine => "quarantine",
        RecommendedAction::Review => "review",
        RecommendedAction::InspectOnly => "inspect_only",
    }
}
fn parse_action(s: &str) -> RecommendedAction {
    match s {
        "quarantine" => RecommendedAction::Quarantine,
        "review" => RecommendedAction::Review,
        _ => RecommendedAction::InspectOnly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::ev;

    fn candidate(id: &str, path: &str) -> CleanupCandidate {
        CleanupCandidate {
            id: id.into(),
            detector: "installers".into(),
            category: Category::Installers,
            target_kind: TargetKind::File,
            path: PathBuf::from(path),
            display_name: "Chrome.dmg".into(),
            associated_app: Some("Google Chrome".into()),
            size: 212_000_000,
            group_bytes: 212_000_000,
            confidence: Confidence::High,
            risk: Risk::Low,
            recommended_action: RecommendedAction::Quarantine,
            evidence: vec![
                ev(EvidenceKind::InstallerFormat { ext: "dmg".into() }),
                ev(EvidenceKind::InstalledAppSupersedes {
                    app: "Google Chrome".into(),
                }),
                ev(EvidenceKind::UntouchedFor { days: 184 }),
            ],
            remark: Some("Google Chrome is already installed.".into()),
            modified_unix: Some(1_700_000_000),
            accessed_unix: Some(1_700_000_001),
            created_unix: None,
            group: vec![GroupMember {
                path: PathBuf::from("/other/Chrome.dmg"),
                size: 212_000_000,
                modified_unix: Some(1_700_000_000),
                suggested_keep: true,
            }],
            fingerprint: StateFingerprint {
                size: 212_000_000,
                modified_unix: Some(1_700_000_000),
                is_dir: false,
                child_count: None,
            },
        }
    }

    fn store_with_scan() -> Store {
        let store = Store::in_memory().unwrap();
        store
            .begin_scan(
                "s1",
                ScanKind::Full,
                &ScanOptions {
                    roots: vec!["/tmp".into()],
                    ..Default::default()
                },
                100,
            )
            .unwrap();
        store
    }

    #[test]
    fn a_candidate_survives_a_round_trip_intact() {
        let store = store_with_scan();
        let original = candidate("f1", "/Users/x/Downloads/Chrome.dmg");
        store
            .save_candidates("s1", std::slice::from_ref(&original))
            .unwrap();

        let read = store.candidate("f1").unwrap();
        assert_eq!(read.display_name, original.display_name);
        assert_eq!(read.path, original.path);
        assert_eq!(read.size, original.size);
        assert_eq!(read.confidence, original.confidence);
        assert_eq!(read.risk, original.risk);
        assert_eq!(read.recommended_action, original.recommended_action);
        assert_eq!(read.fingerprint, original.fingerprint);
        assert_eq!(read.group.len(), 1);
        assert!(read.group[0].suggested_keep);
    }

    #[test]
    fn evidence_keeps_its_order_and_its_sentences() {
        let store = store_with_scan();
        let original = candidate("f1", "/x/Chrome.dmg");
        store
            .save_candidates("s1", std::slice::from_ref(&original))
            .unwrap();
        let read = store.candidate("f1").unwrap();

        assert_eq!(read.evidence.len(), 3);
        assert_eq!(read.evidence, original.evidence);
        assert!(read.evidence[2].summary.contains("184 days"));
    }

    #[test]
    fn an_unreadable_risk_level_falls_back_to_the_most_cautious_answer() {
        // Corruption or a future schema must never read as "safe to delete".
        assert_eq!(parse_risk("something-from-the-future"), Risk::Protected);
        assert_eq!(
            parse_action("something-from-the-future"),
            RecommendedAction::InspectOnly
        );
    }

    #[test]
    fn a_missing_finding_is_not_found_rather_than_a_crash() {
        let store = Store::in_memory().unwrap();
        let err = store.candidate("nope").unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn ignores_round_trip_and_can_be_cleared() {
        let store = Store::in_memory().unwrap();
        store.ignore_path(Path::new("/Users/x/Keep"), 1).unwrap();
        store.ignore_app("Old Game", 1).unwrap();
        store.ignore_category(Category::Screenshots, 1).unwrap();

        let ignores = store.ignore_set().unwrap();
        assert_eq!(ignores.paths, vec![PathBuf::from("/Users/x/Keep")]);
        assert_eq!(
            ignores.apps,
            vec!["old game".to_string()],
            "app names are folded"
        );
        assert_eq!(ignores.categories, vec![Category::Screenshots]);

        store.clear_ignores().unwrap();
        let ignores = store.ignore_set().unwrap();
        assert!(
            ignores.paths.is_empty() && ignores.apps.is_empty() && ignores.categories.is_empty()
        );
    }

    #[test]
    fn ignoring_the_same_thing_twice_is_harmless() {
        let store = Store::in_memory().unwrap();
        store.ignore_path(Path::new("/x"), 1).unwrap();
        store.ignore_path(Path::new("/x"), 2).unwrap();
        assert_eq!(store.ignore_set().unwrap().paths.len(), 1);
    }

    #[test]
    fn a_quarantine_record_keeps_the_evidence_it_was_given() {
        let store = Store::in_memory().unwrap();
        let record = QuarantineRecord {
            id: "q1".into(),
            finding_id: Some("f1".into()),
            original_path: "/Users/x/Downloads/Chrome.dmg".into(),
            stored_path: "/Users/x/.scuttle/quarantine/q1/Chrome.dmg".into(),
            display_name: "Chrome.dmg".into(),
            category: Category::Installers,
            size: 1000,
            content_hash: Some("abc".into()),
            evidence: vec![ev(EvidenceKind::InstallerFormat { ext: "dmg".into() })],
            quarantined_unix: 500,
            expires_unix: 500 + 14 * 86_400,
            status: QuarantineStatus::Held,
            resolved_unix: None,
            mode: crate::storage::RecordMode::Whole,
            item_count: 1,
            attention: false,
        };
        store.insert_quarantine(&record).unwrap();

        let held = store.held_quarantine().unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].evidence.len(), 1);
        assert_eq!(held[0].original_path, record.original_path);

        store
            .set_quarantine_status("q1", QuarantineStatus::Restored, 900)
            .unwrap();
        assert!(store.held_quarantine().unwrap().is_empty());
        assert_eq!(
            store.quarantine_record("q1").unwrap().status,
            QuarantineStatus::Restored
        );
    }

    #[test]
    fn expiry_only_returns_things_still_being_held() {
        let store = Store::in_memory().unwrap();
        for (id, expires, status) in [
            ("q1", 100, QuarantineStatus::Held),
            ("q2", 100, QuarantineStatus::Restored),
            ("q3", 9_999, QuarantineStatus::Held),
        ] {
            store
                .insert_quarantine(&QuarantineRecord {
                    id: id.into(),
                    finding_id: None,
                    original_path: "/a".into(),
                    stored_path: "/b".into(),
                    display_name: id.into(),
                    category: Category::Caches,
                    size: 1,
                    content_hash: None,
                    evidence: vec![],
                    quarantined_unix: 0,
                    expires_unix: expires,
                    status,
                    resolved_unix: None,
                    mode: crate::storage::RecordMode::Whole,
                    item_count: 1,
                    attention: false,
                })
                .unwrap();
        }
        let expired = store.expired_quarantine(500).unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, "q1");
    }

    #[test]
    fn settings_default_until_they_are_saved() {
        let store = Store::in_memory().unwrap();
        let defaults = store.settings().unwrap();
        assert_eq!(defaults.quarantine_retention_days, 14);
        assert!(!defaults.include_developer_debris);

        let updated = Settings {
            quarantine_retention_days: 30,
            include_developer_debris: true,
            ..defaults
        };
        store.save_settings(&updated).unwrap();
        let read = store.settings().unwrap();
        assert_eq!(read.quarantine_retention_days, 30);
        assert!(read.include_developer_debris);
    }

    #[test]
    fn update_checks_are_on_by_default_including_for_settings_saved_before_they_existed() {
        assert!(Settings::default().auto_check_updates);

        // What an alpha.2 install has in its database: no such key.
        let store = Store::in_memory().unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('settings', ?1)",
                params![r#"{"appearance":"dark","background_mode":true}"#],
            )
            .unwrap();
        }
        let loaded = store.settings().unwrap();
        assert!(loaded.auto_check_updates);
        assert_eq!(loaded.appearance, "dark", "what was saved is kept");

        // And turning it off is remembered.
        store
            .save_settings(&Settings {
                auto_check_updates: false,
                ..loaded
            })
            .unwrap();
        assert!(!store.settings().unwrap().auto_check_updates);
    }

    #[test]
    fn a_checkpoint_before_an_update_leaves_the_data_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scuttle.db");
        {
            let store = Store::open(&path).unwrap();
            store
                .save_settings(&Settings {
                    appearance: "light".into(),
                    ..Default::default()
                })
                .unwrap();
            store.checkpoint().unwrap();
            // A memory database has no log to fold and must not mind being asked.
            Store::in_memory().unwrap().checkpoint().unwrap();
        }
        // The next version opens the same file and finds everything.
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.settings().unwrap().appearance, "light");
    }

    #[test]
    fn settings_saved_by_an_older_version_still_load() {
        // `sound` and `excluded_paths` were dropped. Anyone who ran an earlier
        // build has them sitting in their database; reading it must not reset
        // every other preference they set.
        let store = Store::in_memory().unwrap();
        let conn = store.lock();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('settings', ?1)",
            params![
                r#"{"scan_roots":["/Users/x/Downloads"],"excluded_paths":["/Users/x/Secret"],
                    "include_developer_debris":true,"quarantine_retention_days":30,
                    "heavy_threshold":123,"appearance":"dark","reduced_motion":true,
                    "sound":true,"has_rummaged_before":true}"#
            ],
        )
        .unwrap();
        drop(conn);

        let settings = store.settings().unwrap();
        assert_eq!(settings.quarantine_retention_days, 30);
        assert_eq!(settings.appearance, "dark");
        assert_eq!(settings.heavy_threshold, 123);
        assert!(settings.include_developer_debris);
        assert_eq!(settings.reduced_motion, Some(true));
        assert_eq!(
            settings.scan_roots,
            vec![PathBuf::from("/Users/x/Downloads")]
        );
    }

    #[test]
    fn settings_from_a_newer_version_fall_back_rather_than_vanish() {
        // A downgrade, or a corrupted row: defaults beat losing the file.
        let store = Store::in_memory().unwrap();
        let conn = store.lock();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('settings', ?1)",
            params!["{ not json at all"],
        )
        .unwrap();
        drop(conn);

        let settings = store.settings().unwrap();
        assert_eq!(settings.quarantine_retention_days, 14);
    }

    #[test]
    fn forgetting_a_finding_takes_its_evidence_with_it() {
        let store = store_with_scan();
        store
            .save_candidates("s1", &[candidate("f1", "/x")])
            .unwrap();
        store.forget_candidate("f1").unwrap();
        assert_eq!(store.candidate("f1").unwrap_err().code(), "not_found");
        assert!(store.candidates_for_scan("s1").unwrap().is_empty());
    }

    #[test]
    fn pruning_keeps_only_the_most_recent_scans() {
        let store = Store::in_memory().unwrap();
        for i in 0..5 {
            store
                .begin_scan(
                    &format!("s{i}"),
                    ScanKind::Full,
                    &ScanOptions::default(),
                    i as i64 * 100,
                )
                .unwrap();
        }
        store.prune_scans(2).unwrap();
        let conn = store.lock();
        let count: u32 = conn
            .query_row("SELECT count(*) FROM scan_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn the_latest_scan_ignores_ones_that_never_finished() {
        let store = Store::in_memory().unwrap();
        store
            .begin_scan("done", ScanKind::Full, &ScanOptions::default(), 100)
            .unwrap();
        store
            .finish_scan(
                &ScanSummary {
                    scan_id: "done".into(),
                    files_seen: 10,
                    bytes_seen: 20,
                    candidates_found: 1,
                    reclaimable_bytes: 5,
                    duration_ms: 3,
                    walk_ms: 1,
                    probe_ms: 1,
                    finish_ms: 1,
                    cancelled: false,
                    hiccups: Default::default(),
                },
                200,
            )
            .unwrap();
        store
            .begin_scan("abandoned", ScanKind::Full, &ScanOptions::default(), 300)
            .unwrap();

        let latest = store.latest_scan().unwrap().unwrap();
        assert_eq!(latest.id, "done");
        assert_eq!(latest.files_seen, 10);
    }

    #[test]
    fn a_scan_remembers_the_places_it_could_not_read() {
        // These were written on every scan and never read back, so the
        // findings screen reported a clean sweep even when a whole home
        // folder had been refused. A partial scan has to be able to say so.
        let store = Store::in_memory().unwrap();
        store
            .begin_scan("s", ScanKind::Full, &ScanOptions::default(), 100)
            .unwrap();
        store
            .finish_scan(
                &ScanSummary {
                    scan_id: "s".into(),
                    files_seen: 10,
                    bytes_seen: 20,
                    candidates_found: 1,
                    reclaimable_bytes: 5,
                    duration_ms: 3,
                    walk_ms: 1,
                    probe_ms: 1,
                    finish_ms: 1,
                    cancelled: false,
                    hiccups: crate::scanning::HiccupSummary {
                        permission_denied: 4,
                        unreadable: 2,
                        vanished: 1,
                        loops_avoided: 0,
                    },
                },
                200,
            )
            .unwrap();

        let latest = store.latest_scan().unwrap().unwrap();
        assert_eq!(latest.hiccups.permission_denied, 4);
        assert_eq!(latest.hiccups.unreadable, 2);
        assert_eq!(latest.hiccups.vanished, 1);
    }

    #[test]
    fn history_comes_back_newest_first() {
        let store = Store::in_memory().unwrap();
        for (id, at) in [("h1", 100), ("h2", 300), ("h3", 200)] {
            store
                .record_history(&HistoryEntry {
                    id: id.into(),
                    action: "quarantine".into(),
                    display_name: id.into(),
                    category: Category::Installers,
                    size: 1,
                    at_unix: at,
                    outcome: "ok".into(),
                })
                .unwrap();
        }
        let history = store.history(10).unwrap();
        assert_eq!(
            history.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
            vec!["h2", "h3", "h1"]
        );
    }
}
