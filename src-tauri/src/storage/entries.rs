//! The per-file tables: what a directory finding was reviewed as, and the
//! checkpoint of a move into or out of the drawer.
//!
//! Both are disk-backed and written in batches. A cache folder can hold a few
//! hundred thousand files; none of this ever holds them all in memory, and
//! none of it holds the store's lock across anything but one small statement
//! or one batch transaction.

use rusqlite::{params, OptionalExtension};

use super::{QuarantineStatus, Store};
use crate::quarantine::transfer::FailureKind;
use crate::Result;

/// Whether a directory finding has a usable reviewed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotState {
    /// No reviewed set: an older finding, or not the kind that has one.
    None,
    /// Every eligible file at scan time is recorded.
    Complete,
    /// More files than a finding can carry. It cannot be moved file by file.
    Truncated,
    /// Something happened since the set was taken that Scuttle will not guess
    /// its way past. Review the finding again to get a fresh set.
    NeedsRefresh,
}

impl SnapshotState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SnapshotState::None => "none",
            SnapshotState::Complete => "complete",
            SnapshotState::Truncated => "truncated",
            SnapshotState::NeedsRefresh => "needs_refresh",
        }
    }
    pub fn parse(s: &str) -> SnapshotState {
        match s {
            "complete" => SnapshotState::Complete,
            "truncated" => SnapshotState::Truncated,
            "needs_refresh" => SnapshotState::NeedsRefresh,
            _ => SnapshotState::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub state: SnapshotState,
    pub at_unix: Option<i64>,
    pub files: u64,
    pub bytes: u64,
}

/// One file of a reviewed set. `rel` is `/`-separated and relative to the
/// finding's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub rel: String,
    pub size: u64,
    pub mtime_ns: Option<i64>,
    pub created_ns: Option<i64>,
    pub file_id: Option<String>,
}

/// The state of one file in a drawer record's checkpoint.
pub mod journal {
    /// Intent written before the file is touched.
    pub const PENDING: &str = "pending";
    /// In the drawer, and counted.
    pub const DONE: &str = "done";
    /// Being restored.
    pub const RESTORING: &str = "restoring";
    /// Back out of the drawer.
    pub const RESTORED: &str = "restored";
    /// A verified copy is in the drawer but the original was still in place
    /// when the move was interrupted. It is not counted as moved, and nothing
    /// is deleted on its account.
    pub const COPIED: &str = "copied";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub rel: String,
    pub state: String,
    pub size: u64,
}

impl Store {
    // ---- reviewed sets -------------------------------------------------

    /// Start a fresh reviewed set for a finding, discarding any earlier one.
    pub fn begin_snapshot(&self, finding_id: &str) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM finding_entries WHERE finding_id = ?1",
            params![finding_id],
        )?;
        tx.execute(
            "UPDATE findings SET snapshot_state = 'none', snapshot_at_unix = NULL,
                    snapshot_files = 0, snapshot_bytes = 0
             WHERE id = ?1",
            params![finding_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn append_snapshot_entries(
        &self,
        finding_id: &str,
        entries: &[SnapshotEntry],
    ) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO finding_entries
                    (finding_id, rel, size, mtime_ns, created_ns, file_id, outcome)
                 VALUES (?1,?2,?3,?4,?5,?6,NULL)",
            )?;
            for entry in entries {
                insert.execute(params![
                    finding_id,
                    entry.rel,
                    entry.size,
                    entry.mtime_ns,
                    entry.created_ns,
                    entry.file_id,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Close a reviewed set. When `adopt_totals` is set the finding's own size
    /// becomes the set's total, so the figure a person reviews is exactly the
    /// set that can move.
    pub fn finish_snapshot(
        &self,
        finding_id: &str,
        state: SnapshotState,
        at_unix: i64,
        files: u64,
        bytes: u64,
        adopt_totals: bool,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE findings SET snapshot_state = ?2, snapshot_at_unix = ?3,
                    snapshot_files = ?4, snapshot_bytes = ?5,
                    size = CASE WHEN ?6 THEN ?5 ELSE size END
             WHERE id = ?1",
            params![
                finding_id,
                state.as_str(),
                at_unix,
                files,
                bytes,
                adopt_totals
            ],
        )?;
        Ok(())
    }

    pub fn snapshot_info(&self, finding_id: &str) -> Result<SnapshotInfo> {
        let conn = self.lock();
        let info = conn
            .query_row(
                "SELECT snapshot_state, snapshot_at_unix, snapshot_files, snapshot_bytes
                 FROM findings WHERE id = ?1",
                params![finding_id],
                |row| {
                    Ok(SnapshotInfo {
                        state: SnapshotState::parse(&row.get::<_, String>(0)?),
                        at_unix: row.get(1)?,
                        files: row.get::<_, i64>(2)?.max(0) as u64,
                        bytes: row.get::<_, i64>(3)?.max(0) as u64,
                    })
                },
            )
            .optional()?;
        info.ok_or_else(|| crate::ScuttleError::not_found("That finding"))
    }

    pub fn set_snapshot_state(&self, finding_id: &str, state: SnapshotState) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE findings SET snapshot_state = ?2 WHERE id = ?1",
            params![finding_id, state.as_str()],
        )?;
        Ok(())
    }

    /// The next entries still to act on, in key order after `after`: those
    /// never attempted and those whose last attempt failed in a way that
    /// trying again might fix. Entries that moved, changed, or were replaced
    /// are never offered again — that is what stops a retry widening a
    /// selection.
    pub fn snapshot_batch(
        &self,
        finding_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SnapshotEntry>> {
        let conn = self.lock();
        let retryable = FailureKind::RETRYABLE_SLUGS
            .iter()
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT rel, size, mtime_ns, created_ns, file_id FROM finding_entries
             WHERE finding_id = ?1 AND rel > ?2
               AND (outcome IS NULL OR outcome IN ({retryable}))
             ORDER BY rel LIMIT ?3"
        );
        let mut statement = conn.prepare_cached(&sql)?;
        let rows = statement.query_map(
            params![finding_id, after.unwrap_or(""), limit as i64],
            |row| {
                Ok(SnapshotEntry {
                    rel: row.get(0)?,
                    size: row.get::<_, i64>(1)?.max(0) as u64,
                    mtime_ns: row.get(2)?,
                    created_ns: row.get(3)?,
                    file_id: row.get(4)?,
                })
            },
        )?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    /// How many entries of the set are still actionable, and how many bytes.
    pub fn snapshot_actionable(&self, finding_id: &str) -> Result<(u64, u64)> {
        let conn = self.lock();
        let retryable = FailureKind::RETRYABLE_SLUGS
            .iter()
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT count(*), coalesce(sum(size), 0) FROM finding_entries
             WHERE finding_id = ?1 AND (outcome IS NULL OR outcome IN ({retryable}))"
        );
        let (count, bytes): (i64, i64) =
            conn.query_row(&sql, params![finding_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok((count.max(0) as u64, bytes.max(0) as u64))
    }

    /// How many entries were tried and did not move.
    pub fn snapshot_attempted_unmoved(&self, finding_id: &str) -> Result<u64> {
        let conn = self.lock();
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM finding_entries
             WHERE finding_id = ?1 AND outcome IS NOT NULL AND outcome != 'moved'",
            params![finding_id],
            |r| r.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    /// Set a finding's displayed size, for when its reviewed set has shrunk.
    pub fn set_finding_size(&self, finding_id: &str, size: u64) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE findings SET size = ?2 WHERE id = ?1",
            params![finding_id, size],
        )?;
        Ok(())
    }

    /// Replace a finding's remembered state with what is there now. For a file
    /// its size follows; a directory keeps the size it was measured at.
    pub fn update_fingerprint(
        &self,
        finding_id: &str,
        fingerprint: &crate::model::StateFingerprint,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE findings SET fingerprint = ?2,
                    size = CASE WHEN ?3 THEN ?4 ELSE size END
             WHERE id = ?1",
            params![
                finding_id,
                serde_json::to_string(fingerprint)?,
                !fingerprint.is_dir,
                fingerprint.size
            ],
        )?;
        Ok(())
    }

    /// Drop a record that never became anything, along with its checkpoint.
    pub fn delete_quarantine(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM quarantine_items WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// How many entries did not move, whatever the reason.
    pub fn snapshot_unmoved(&self, finding_id: &str) -> Result<u64> {
        let conn = self.lock();
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM finding_entries
             WHERE finding_id = ?1 AND (outcome IS NULL OR outcome != 'moved')",
            params![finding_id],
            |r| r.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    pub fn set_entry_outcomes(&self, finding_id: &str, outcomes: &[(String, &str)]) -> Result<()> {
        if outcomes.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut update = tx.prepare_cached(
                "UPDATE finding_entries SET outcome = ?3 WHERE finding_id = ?1 AND rel = ?2",
            )?;
            for (rel, outcome) in outcomes {
                update.execute(params![finding_id, rel, outcome])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // ---- the drawer's checkpoint ---------------------------------------

    /// Write intent for a batch, ahead of touching any of it.
    pub fn journal_add(&self, record_id: &str, entries: &[(String, u64)]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO drawer_entries (record_id, rel, state, size)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (rel, size) in entries {
                insert.execute(params![record_id, rel, journal::PENDING, size])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn journal_set(&self, record_id: &str, entries: &[(String, &str)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut update = tx.prepare_cached(
                "UPDATE drawer_entries SET state = ?3 WHERE record_id = ?1 AND rel = ?2",
            )?;
            for (rel, state) in entries {
                update.execute(params![record_id, rel, state])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn journal_remove(&self, record_id: &str, rels: &[String]) -> Result<()> {
        if rels.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut delete =
                tx.prepare_cached("DELETE FROM drawer_entries WHERE record_id = ?1 AND rel = ?2")?;
            for rel in rels {
                delete.execute(params![record_id, rel])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The next entries of a record in one of `states`, in key order.
    pub fn journal_page(
        &self,
        record_id: &str,
        state: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<JournalEntry>> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(
            "SELECT rel, state, size FROM drawer_entries
             WHERE record_id = ?1 AND state = ?2 AND rel > ?3
             ORDER BY rel LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![record_id, state, after.unwrap_or(""), limit as i64],
            |row| {
                Ok(JournalEntry {
                    rel: row.get(0)?,
                    state: row.get(1)?,
                    size: row.get::<_, i64>(2)?.max(0) as u64,
                })
            },
        )?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    /// `(files, bytes)` of a record's entries in `state`.
    pub fn journal_totals(&self, record_id: &str, state: &str) -> Result<(u64, u64)> {
        let conn = self.lock();
        let (count, bytes): (i64, i64) = conn.query_row(
            "SELECT count(*), coalesce(sum(size), 0) FROM drawer_entries
             WHERE record_id = ?1 AND state = ?2",
            params![record_id, state],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((count.max(0) as u64, bytes.max(0) as u64))
    }

    // ---- record lifecycle ----------------------------------------------

    /// Settle a record: its status, what it actually holds, and whether it
    /// needs a person's attention.
    pub fn settle_record(
        &self,
        id: &str,
        status: QuarantineStatus,
        size: u64,
        item_count: u64,
        attention: bool,
        resolved_unix: Option<i64>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE quarantine_items
             SET status = ?2, size = ?3, item_count = ?4, attention = ?5, resolved_unix = ?6
             WHERE id = ?1",
            params![
                id,
                status.as_str(),
                size,
                item_count,
                attention as i32,
                resolved_unix
            ],
        )?;
        Ok(())
    }

    pub fn set_record_status(&self, id: &str, status: QuarantineStatus) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE quarantine_items SET status = ?2 WHERE id = ?1",
            params![id, status.as_str()],
        )?;
        Ok(())
    }

    pub fn set_quarantine_restoring(&self, id: &str) -> Result<()> {
        self.set_record_status(id, QuarantineStatus::Restoring)
    }

    pub fn records_with_status(
        &self,
        status: QuarantineStatus,
    ) -> Result<Vec<super::QuarantineRecord>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT id, finding_id, original_path, stored_path, display_name, category, size,
                    content_hash, evidence, quarantined_unix, expires_unix, status, resolved_unix,
                    mode, item_count, attention
             FROM quarantine_items WHERE status = ?1",
        )?;
        let rows = statement.query_map(params![status.as_str()], super::row_to_quarantine)?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    /// Every record id, for spotting cells in the drawer that no record owns.
    pub fn quarantine_ids(&self) -> Result<Vec<String>> {
        let conn = self.lock();
        let mut statement = conn.prepare("SELECT id FROM quarantine_items")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }
}
