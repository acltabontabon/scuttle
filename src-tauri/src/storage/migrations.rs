//! Schema migrations.
//!
//! Migrations exist from the first release so that the second one is not a
//! crisis. Each entry is applied once, in order, inside a transaction, and
//! `PRAGMA user_version` records how far we have got.
//!
//! Rules: never edit a migration that has shipped; append a new one instead.

use rusqlite::Connection;

use crate::Result;

/// The ordered list. Index + 1 is the resulting `user_version`.
pub const MIGRATIONS: &[&str] = &[
    // 1 — the shape of a rummage.
    r#"
    CREATE TABLE scan_runs (
        id                 TEXT PRIMARY KEY,
        started_unix       INTEGER NOT NULL,
        finished_unix      INTEGER,
        roots              TEXT NOT NULL,
        files_seen         INTEGER NOT NULL DEFAULT 0,
        bytes_seen         INTEGER NOT NULL DEFAULT 0,
        candidates_found   INTEGER NOT NULL DEFAULT 0,
        reclaimable_bytes  INTEGER NOT NULL DEFAULT 0,
        duration_ms        INTEGER NOT NULL DEFAULT 0,
        cancelled          INTEGER NOT NULL DEFAULT 0,
        hiccups            TEXT NOT NULL DEFAULT '{}'
    );

    CREATE TABLE findings (
        id                 TEXT PRIMARY KEY,
        scan_id            TEXT NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
        detector           TEXT NOT NULL,
        category           TEXT NOT NULL,
        target_kind        TEXT NOT NULL,
        path               TEXT NOT NULL,
        display_name       TEXT NOT NULL,
        associated_app     TEXT,
        size               INTEGER NOT NULL,
        confidence         TEXT NOT NULL,
        risk               TEXT NOT NULL,
        recommended_action TEXT NOT NULL,
        remark             TEXT,
        modified_unix      INTEGER,
        accessed_unix      INTEGER,
        created_unix       INTEGER,
        fingerprint        TEXT NOT NULL,
        group_members      TEXT NOT NULL DEFAULT '[]'
    );
    CREATE INDEX findings_scan ON findings(scan_id);
    CREATE INDEX findings_category ON findings(scan_id, category);

    -- Evidence is its own table because it is the part a person reads when
    -- deciding whether to trust a finding, and it should be queryable.
    CREATE TABLE finding_evidence (
        finding_id  TEXT NOT NULL REFERENCES findings(id) ON DELETE CASCADE,
        position    INTEGER NOT NULL,
        kind        TEXT NOT NULL,
        summary     TEXT NOT NULL,
        weight      INTEGER NOT NULL,
        negative    INTEGER NOT NULL,
        risk_floor  TEXT,
        PRIMARY KEY (finding_id, position)
    );

    CREATE TABLE ignored_paths (
        path         TEXT PRIMARY KEY,
        created_unix INTEGER NOT NULL
    );
    CREATE TABLE ignored_apps (
        name         TEXT PRIMARY KEY,
        created_unix INTEGER NOT NULL
    );
    CREATE TABLE ignored_categories (
        category     TEXT PRIMARY KEY,
        created_unix INTEGER NOT NULL
    );

    CREATE TABLE quarantine_items (
        id              TEXT PRIMARY KEY,
        finding_id      TEXT,
        original_path   TEXT NOT NULL,
        stored_path     TEXT NOT NULL,
        display_name    TEXT NOT NULL,
        category        TEXT NOT NULL,
        size            INTEGER NOT NULL,
        content_hash    TEXT,
        evidence        TEXT NOT NULL DEFAULT '[]',
        quarantined_unix INTEGER NOT NULL,
        expires_unix    INTEGER NOT NULL,
        status          TEXT NOT NULL,
        resolved_unix   INTEGER
    );
    CREATE INDEX quarantine_status ON quarantine_items(status, expires_unix);

    CREATE TABLE cleanup_history (
        id          TEXT PRIMARY KEY,
        action      TEXT NOT NULL,
        display_name TEXT NOT NULL,
        category    TEXT NOT NULL,
        size        INTEGER NOT NULL,
        at_unix     INTEGER NOT NULL,
        outcome     TEXT NOT NULL
    );

    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
    // 2 — reviewed contents, and a journal for interrupted moves.
    //
    // `finding_entries` is the set of files a directory finding was reviewed
    // as: the files that existed, and were eligible, when it was scanned. A move
    // acts on that set and nothing else. `outcome` records what became of each
    // entry so a retry never re-touches what already moved.
    //
    // `drawer_entries` is the per-file checkpoint of a move into (or a restore
    // out of) the drawer. It is written ahead of the operation, so that after a
    // crash it is possible to tell what had and had not happened. It is also
    // the manifest a restore works from: exactly the files that were recorded.
    r#"
    ALTER TABLE findings ADD COLUMN snapshot_state TEXT NOT NULL DEFAULT 'none';
    ALTER TABLE findings ADD COLUMN snapshot_at_unix INTEGER;
    ALTER TABLE findings ADD COLUMN snapshot_files INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE findings ADD COLUMN snapshot_bytes INTEGER NOT NULL DEFAULT 0;

    CREATE TABLE finding_entries (
        finding_id TEXT NOT NULL REFERENCES findings(id) ON DELETE CASCADE,
        rel        TEXT NOT NULL,
        size       INTEGER NOT NULL,
        mtime_ns   INTEGER,
        created_ns INTEGER,
        file_id    TEXT,
        outcome    TEXT,
        PRIMARY KEY (finding_id, rel)
    ) WITHOUT ROWID;

    ALTER TABLE quarantine_items ADD COLUMN mode TEXT NOT NULL DEFAULT 'whole';
    ALTER TABLE quarantine_items ADD COLUMN item_count INTEGER NOT NULL DEFAULT 1;
    ALTER TABLE quarantine_items ADD COLUMN attention INTEGER NOT NULL DEFAULT 0;

    CREATE TABLE drawer_entries (
        record_id TEXT NOT NULL REFERENCES quarantine_items(id) ON DELETE CASCADE,
        rel       TEXT NOT NULL,
        state     TEXT NOT NULL,
        size      INTEGER NOT NULL,
        PRIMARY KEY (record_id, rel)
    ) WITHOUT ROWID;
    CREATE INDEX drawer_entries_state ON drawer_entries(record_id, state);
    "#,
    // 3 — background checks.
    //
    // `scan_runs.kind` separates a rummage the user started from a background
    // check, which reads no file contents and so cannot find duplicates or
    // near-identical screenshots. The findings screen needs to know which it
    // is looking at, because a partial look must never be reported as a whole
    // one. Existing rows are rummages, which is what the default says.
    //
    // `background_seen` is what stops Scuttle mentioning the same thing twice.
    // It holds a hash of category and path — never a path — and rows are
    // pruned by age.
    r#"
    ALTER TABLE scan_runs ADD COLUMN kind TEXT NOT NULL DEFAULT 'full';

    CREATE TABLE background_seen (
        key       TEXT PRIMARY KEY,
        seen_unix INTEGER NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX background_seen_age ON background_seen(seen_unix);
    "#,
    // 4 — the Drawer keeps what cannot be rebuilt.
    //
    // `keep` marks items the retention sweep must never remove: a person's own
    // files, an application's data, and anything moved past a caution. Only
    // rebuildable things (caches, build output) and re-downloadable ones may
    // expire on their own. Before 0.1.0 everything expired after its retention
    // window, so every existing row is kept — including anything an alpha
    // build mistook for an installer — except caches and build output, which
    // were only ever rebuildable.
    r#"
    ALTER TABLE quarantine_items ADD COLUMN keep INTEGER NOT NULL DEFAULT 1;
    UPDATE quarantine_items SET keep = 0 WHERE category IN ('caches', 'developer_debris');
    "#,
];

/// Bring a connection up to the current schema.
pub fn apply(conn: &mut Connection) -> Result<u32> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let version = index as u32 + 1;
        if version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // `user_version` does not accept a bound parameter.
        tx.execute_batch(&format!("PRAGMA user_version = {version}"))?;
        tx.commit()?;
    }

    Ok(MIGRATIONS.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_to_a_fresh_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        let version = apply(&mut conn).unwrap();
        assert_eq!(version, MIGRATIONS.len() as u32);
        let stored: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, version);
    }

    #[test]
    fn applying_twice_is_a_no_op() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();
        // Would fail with "table already exists" if migrations re-ran.
        apply(&mut conn).unwrap();
    }

    #[test]
    fn every_expected_table_exists() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();
        for table in [
            "scan_runs",
            "findings",
            "finding_evidence",
            "ignored_paths",
            "ignored_apps",
            "ignored_categories",
            "quarantine_items",
            "finding_entries",
            "drawer_entries",
            "cleanup_history",
            "settings",
            "background_seen",
        ] {
            let count: u32 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "{table} is missing");
        }
    }

    #[test]
    fn deleting_a_scan_takes_its_findings_and_evidence_with_it() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        conn.execute_batch(
            "INSERT INTO scan_runs (id, started_unix, roots) VALUES ('s1', 0, '[]');
             INSERT INTO findings (id, scan_id, detector, category, target_kind, path,
                 display_name, size, confidence, risk, recommended_action, fingerprint)
               VALUES ('f1','s1','d','ghosts','file','/x','x',1,'high','low','quarantine','{}');
             INSERT INTO finding_evidence (finding_id, position, kind, summary, weight, negative)
               VALUES ('f1', 0, '{}', 'because', 10, 0);
             DELETE FROM scan_runs WHERE id = 's1';",
        )
        .unwrap();
        let findings: u32 = conn
            .query_row("SELECT count(*) FROM findings", [], |r| r.get(0))
            .unwrap();
        let evidence: u32 = conn
            .query_row("SELECT count(*) FROM finding_evidence", [], |r| r.get(0))
            .unwrap();
        assert_eq!(findings, 0);
        assert_eq!(evidence, 0);
    }

    #[test]
    fn a_v1_database_upgrades_without_losing_its_drawer() {
        // A record written under the first schema must survive the second.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.execute_batch("PRAGMA user_version = 1").unwrap();
        conn.execute_batch(
            "INSERT INTO quarantine_items (id, original_path, stored_path, display_name,
                category, size, quarantined_unix, expires_unix, status)
             VALUES ('old', '/a', '/q/old/a', 'a', 'installers', 5, 1, 2, 'held');",
        )
        .unwrap();

        apply(&mut conn).unwrap();

        let (mode, count, attention): (String, i64, i64) = conn
            .query_row(
                "SELECT mode, item_count, attention FROM quarantine_items WHERE id='old'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((mode.as_str(), count, attention), ("whole", 1, 0));
    }

    #[test]
    fn scans_from_before_background_checks_are_still_rummages() {
        // Anything recorded before there was such a thing as a background
        // check was a rummage the user started, and must keep being reported
        // as one.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.execute_batch(MIGRATIONS[1]).unwrap();
        conn.execute_batch("PRAGMA user_version = 2").unwrap();
        conn.execute_batch(
            "INSERT INTO scan_runs (id, started_unix, roots) VALUES ('old', 1, '[]');",
        )
        .unwrap();

        apply(&mut conn).unwrap();

        let kind: String = conn
            .query_row("SELECT kind FROM scan_runs WHERE id='old'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(kind, "full");
    }

    #[test]
    fn an_alpha_drawer_keeps_everything_that_cannot_be_rebuilt() {
        // A 0.1.0-alpha.2 database: three migrations, and a Drawer holding a
        // cache, an "installer" an alpha build took from inside an
        // application, and a person's screenshot. After upgrading, only the
        // cache may still expire; the rest wait for a person.
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..3] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch("PRAGMA user_version = 3").unwrap();
        conn.execute_batch(
            "INSERT INTO quarantine_items (id, original_path, stored_path, display_name,
                category, size, quarantined_unix, expires_unix, status, content_hash)
             VALUES
               ('cache', '/c', '/q/cache/c', 'c', 'caches', 5, 1, 2, 'held', NULL),
               ('app', 'C:/Users/x/AppData/Local/Discord/app-1.0.9001/Discord.exe',
                  '/q/app/Discord.exe', 'Discord.exe', 'installers', 5, 1, 2, 'held', 'abc'),
               ('shot', '/s.png', '/q/shot/s.png', 's.png', 'screenshots', 5, 1, 2, 'held', NULL);",
        )
        .unwrap();

        apply(&mut conn).unwrap();

        let keep = |id: &str| -> i64 {
            conn.query_row("SELECT keep FROM quarantine_items WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(keep("cache"), 0);
        assert_eq!(
            keep("app"),
            1,
            "an alpha's mistake must not expire behind anyone"
        );
        assert_eq!(keep("shot"), 1);
        let (hash, path): (String, String) = conn
            .query_row(
                "SELECT content_hash, original_path FROM quarantine_items WHERE id='app'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(hash, "abc", "recovery metadata survives");
        assert!(path.ends_with("Discord.exe"));
    }
}
