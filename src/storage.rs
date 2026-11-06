use std::error::Error;
use std::path::Path;
use rusqlite::{params, Connection, OptionalExtension};
use crate::change::{Change, ChangeKind, CharRange, InsertRun, NoticeKey, Span};
use crate::doc::Doc;
use crate::ids::{ChangeId, CharId, Side, DeviceId, Hlc};
use std::collections::HashMap;
use rustc_hash::FxHashMap;


type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// How a Side is written in the database.
fn side_to_str(side: Side) -> &'static str {
    match side {
        Side::Left => "L",
        Side::Right => "R",
    }
}

/// The other direction; anything else means the file is damaged.
fn side_from_str(s: &str) -> Result<Side> {
    match s {
        "L" => Ok(Side::Left),
        "R" => Ok(Side::Right),
        other => Err(format!("Invalid side: {other:?}").into()),
    }
}

const SCHEMA: &str = "
CREATE TABLE changes (
    id      TEXT PRIMARY KEY, -- '00000002a:1'
    device  TEXT NOT NULL,
    counter INTEGER NOT NULL,
    hlc     INTEGER NOT NULL, -- Ordering
    time    INTEGER NOT NULL, -- Readable copy of HLC
    kind    TEXT NOT NULL -- 'insert', 'delete', or 'replace'
);
CREATE TABLE insertions (
    change          TEXT PRIMARY KEY REFERENCES changes(id),
    text            TEXT NOT NULL,
    parent          TEXT, -- NULL = root
    side            TEXT NOT NULL CHECK (side IN ('L', 'R')),
    right_origin    TEXT
);
CREATE TABLE deletions (
    change          TEXT REFERENCES changes(id),
    first_char      TEXT NOT NULL,
    len             INTEGER NOT NULL,
    deleted_text    TEXT NOT NULL, -- readable copy
    PRIMARY KEY (change, first_char)
);
";

/// The newest storage version this app writes. A file with a higher one was written by a newer
/// app, which may store things this one doesn't understand, so it isn't touched.
pub const STORAGE_VERSION: i64 = 8;

// Inner functions take a connection
fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > STORAGE_VERSION {
        return Err(format!("this History was written by a newer version of the app (storage version {version})").into());
    }
    if version == 0 {
        // Version 0 means a new, empty file. One that already has tables is some other
        // database: never turn it into a History (or merge it and delete it).
        let tables: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?;
        if tables > 0 {
            return Err("this is not a History file".into());
        }
    }
    if version < 1 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA} PRAGMA user_version = 1; COMMIT;"))?;
    }
    if version < 2 {
        conn.execute_batch("
        BEGIN;
        CREATE TABLE revert_targets (
            change TEXT NOT NULL REFERENCES changes(id),
            target TEXT NOT NULL REFERENCES changes(id),
            PRIMARY KEY (change, target)
        );
        PRAGMA user_version = 2;
        COMMIT;
        ")?;
    }
    if version < 3 {
        conn.execute_batch("
        BEGIN;
        CREATE TABLE moves (
            change TEXT PRIMARY KEY REFERENCES changes(id),
            parent TEXT,
            side TEXT NOT NULL CHECK (side IN ('L', 'R')),
            right_origin TEXT
            );
        CREATE TABLE move_pieces (
            change TEXT NOT NULL REFERENCES changes(id),
            piece INTEGER NOT NULL, -- index of the piece in the move
            first_char TEXT NOT NULL,
            last_char TEXT NOT NULL,
            from_anchor TEXT, -- anchor of the piece it was selected from; NULL = the main text
            moved_text TEXT NOT NULL, -- readable copy of the text in the piece
            PRIMARY KEY (change, piece)
            );
        CREATE TABLE move_seen (
            change TEXT NOT NULL REFERENCES changes(id),
            device TEXT NOT NULL,
            counter INTEGER NOT NULL,
            PRIMARY KEY (change, device)
        );
        PRAGMA user_version = 3;
        COMMIT;
        ")?;
    }
    if version < 4 {
        // The piece text was typed or dropped into. Existing rows get NULL: the main text.
        conn.execute_batch("
        BEGIN;
        ALTER TABLE insertions ADD COLUMN into_anchor TEXT;
        ALTER TABLE moves ADD COLUMN into_anchor TEXT;
        PRAGMA user_version = 4;
        COMMIT;
        ")?;
    }
    if version < 5 {
        // Dismissed Merge notices. The notice is named by the Moves involved.
        conn.execute_batch("
        BEGIN;
        CREATE TABLE dismissals (
            change      TEXT PRIMARY KEY REFERENCES changes(id),
            notice      TEXT NOT NULL CHECK (notice IN ('overlap', 'loop')),
            move        TEXT NOT NULL, -- overlap: the later Move; loop: the skipped Move
            other_move  TEXT           -- overlap: the earlier Move; loop: NULL
        );
        PRAGMA user_version = 5;
        COMMIT;
        ")?;
    }
    if version < 6 {
        // Sessions, record fields (such as Device names), and a readable view of the History.
        conn.execute_batch("
        BEGIN;
        ALTER TABLE changes ADD COLUMN session TEXT; -- the Session's first Change; NULL in older rows = its own
        CREATE TABLE fields (
            change  TEXT PRIMARY KEY REFERENCES changes(id),
            record  TEXT NOT NULL,  -- e.g. 'device/0000002a'
            field   TEXT NOT NULL,  -- e.g. 'name'
            value   TEXT NOT NULL
        );
        -- Every Change as a readable sentence: when, which Device, which Session, what kind, what text.
        -- (The Device's latest name; the engine itself also applies Reverts to names.)
        CREATE VIEW history AS
        SELECT
            c.time                          AS time,
            COALESCE(names.name, c.device)  AS device,
            COALESCE(c.session, c.id)       AS session,
            c.kind                          AS kind,
            COALESCE(
                i.text,
                (SELECT group_concat(d.deleted_text, '') FROM deletions d WHERE d.change = c.id),
                (SELECT group_concat(p.moved_text, ' | ') FROM move_pieces p WHERE p.change = c.id),
                (SELECT f.record || ' ' || f.field || ' = ' || f.value FROM fields f WHERE f.change = c.id)
            )                               AS text,
            c.id                            AS id
        FROM changes c
        LEFT JOIN insertions i ON i.change = c.id
        LEFT JOIN (
            SELECT substr(f.record, 8) AS device, f.value AS name, MAX(fc.hlc) AS hlc
            FROM fields f JOIN changes fc ON fc.id = f.change
            WHERE f.record LIKE 'device/%' AND f.field = 'name'
            GROUP BY f.record
        ) names ON names.device = c.device
        ORDER BY c.hlc, c.device;
        PRAGMA user_version = 6;
        COMMIT;
        ")?;
    }
    if version < 7 {
        // Milestones, and the readable view rebuilt so a Milestone row shows its name.
        conn.execute_batch("
        BEGIN;
        CREATE TABLE milestones (
            change  TEXT PRIMARY KEY REFERENCES changes(id),
            name    TEXT NOT NULL        -- the name it was given (renames are fields)
        );
        CREATE TABLE milestone_seen (    -- what it holds: the highest counter seen from each Device
            change  TEXT NOT NULL REFERENCES changes(id),
            device  TEXT NOT NULL,
            counter INTEGER NOT NULL,
            PRIMARY KEY (change, device)
        );
        DROP VIEW history;
        CREATE VIEW history AS
        SELECT
            c.time                          AS time,
            COALESCE(names.name, c.device)  AS device,
            COALESCE(c.session, c.id)       AS session,
            c.kind                          AS kind,
            COALESCE(
                i.text,
                (SELECT group_concat(d.deleted_text, '') FROM deletions d WHERE d.change = c.id),
                (SELECT group_concat(p.moved_text, ' | ') FROM move_pieces p WHERE p.change = c.id),
                (SELECT f.record || ' ' || f.field || ' = ' || f.value FROM fields f WHERE f.change = c.id),
                (SELECT m.name FROM milestones m WHERE m.change = c.id)
            )                               AS text,
            c.id                            AS id
        FROM changes c
        LEFT JOIN insertions i ON i.change = c.id
        LEFT JOIN (
            SELECT substr(f.record, 8) AS device, f.value AS name, MAX(fc.hlc) AS hlc
            FROM fields f JOIN changes fc ON fc.id = f.change
            WHERE f.record LIKE 'device/%' AND f.field = 'name'
            GROUP BY f.record
        ) names ON names.device = c.device
        ORDER BY c.hlc, c.device;
        PRAGMA user_version = 7;
        COMMIT;
        ")?;
    }
    if version < 8 {
        // Text files: every one the app writes is recorded with the version it was built from.
        // And `dismissals` accepts a third kind of notice; SQLite can't change a CHECK in place,
        // so the table is rebuilt.
        conn.execute_batch("
        BEGIN;
        CREATE TABLE text_files (
            hash     TEXT PRIMARY KEY,  -- SHA-256 of the file's contents
            version  TEXT NOT NULL,     -- what it was built from: 'device:counter' per Device, comma-separated
            device   TEXT NOT NULL,     -- which Device wrote it
            written  INTEGER NOT NULL,  -- when, in ms since 1970: the latest is the one to compare with
            time     TEXT NOT NULL      -- readable copy
        );
        CREATE TABLE dismissals_new (
            change      TEXT PRIMARY KEY REFERENCES changes(id),
            notice      TEXT NOT NULL CHECK (notice IN ('overlap', 'loop', 'unknown_text_file')),
            move        TEXT NOT NULL, -- overlap: the later Move; loop: the skipped Move; unknown_text_file: the Session
            other_move  TEXT           -- overlap: the earlier Move; otherwise NULL
        );
        INSERT INTO dismissals_new SELECT change, notice, move, other_move FROM dismissals;
        DROP TABLE dismissals;
        ALTER TABLE dismissals_new RENAME TO dismissals;
        PRAGMA user_version = 8;
        COMMIT;
        ")?;
    }
    Ok(())
}
/// One row in `insertions`. Insert and Replace both write one.
fn write_insertion(conn: &Connection, change: ChangeId, run: &InsertRun) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO insertions (change, text, parent, side, right_origin, into_anchor)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            change.to_string(),
            run.text,
            run.parent.map(|id| id.to_string()),
            side_to_str(run.side),
            run.right_origin.map(|id| id.to_string()),
            run.into.map(|id| id.to_string()),
        ],
    )?;
    Ok(())
}

fn write_change(conn: &Connection, change: &Change, doc: &Doc) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO changes (id, device, counter, hlc, time, kind, session)
         VALUES (?1, ?2, ?3, ?4, datetime(?4 / 1000, 'unixepoch'), ?5, ?6)",
        params![
            change.id.to_string(),
            change.id.device.to_string(),
            change.id.counter,
            change.hlc.0 as i64,
            match &change.kind {
                ChangeKind::Insert(_) => "insert",
                ChangeKind::Delete(_) => "delete",
                ChangeKind::Replace { .. } => "replace",
                ChangeKind::Revert { .. } => "revert",
                ChangeKind::Move { .. } => "move",
                ChangeKind::Dismiss { .. } => "dismiss",
                ChangeKind::SetField { .. } => "field",
                ChangeKind::Milestone { .. } => "milestone",
            },
            change.session.to_string(),
        ],
    )?;
    match &change.kind {
        ChangeKind::Insert(run) => write_insertion(conn, change.id, run)?,
        ChangeKind::Delete(ranges) => {
            for range in ranges {
                let text = doc.text_of(range);
                conn.execute(
                    "INSERT OR IGNORE INTO deletions (change, first_char, len, deleted_text) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        change.id.to_string(),
                        range.first.to_string(),
                        range.len,
                        text,
                    ],
                )?;
            }
        }
        ChangeKind::Replace { deleted, inserted } => {
            for range in deleted {
                let text = doc.text_of(range);
                conn.execute(
                    "INSERT OR IGNORE INTO deletions (change, first_char, len, deleted_text) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        change.id.to_string(),
                        range.first.to_string(),
                        range.len,
                        text,
                    ],
                )?;
            }
            write_insertion(conn, change.id, inserted)?;
        }
        ChangeKind::Revert { targets } => {
            // One row per target
            for target in targets {
                conn.execute(
                    "INSERT OR IGNORE INTO revert_targets (change, target) VALUES (?1, ?2)",
                    params![
                        change.id.to_string(),
                        target.to_string(),
                    ],
                )?;
            }
        }
        ChangeKind::Move { pieces, parent, side, right_origin, seen, into } => {
            // Where the anchors go: the same fields an insertion has.
            conn.execute(
                "INSERT OR IGNORE INTO moves (change, parent, side, right_origin, into_anchor) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    change.id.to_string(),
                    parent.map(|id| id.to_string()),
                    side_to_str(*side),
                    right_origin.map(|id| id.to_string()),
                    into.map(|id| id.to_string()),
                ],
            )?;
            // One row per piece; `piece` is its index, which keeps them in reading order.
            for (k, span) in pieces.iter().enumerate() {
                conn.execute(
                    "INSERT OR IGNORE INTO move_pieces (change, piece, first_char, last_char, from_anchor, moved_text)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        change.id.to_string(),
                        k as i64,
                        span.first.to_string(),
                        span.last.to_string(),
                        span.from.map(|id| id.to_string()),
                        doc.piece_text(span),
                    ],
                )?;
            }
            // What the moving Device had seen: one row per Device.
            for (device, counter) in seen {
                conn.execute(
                    "INSERT OR IGNORE INTO move_seen (change, device, counter) VALUES (?1, ?2, ?3)",
                    params![change.id.to_string(), device.to_string(), counter],
                )?;
            }
        }
        ChangeKind::Dismiss { notice } => {
            let (kind, mv, other) = match notice {
                NoticeKey::Overlap { later, earlier } => ("overlap", later, Some(earlier)),
                NoticeKey::Loop { skipped } => ("loop", skipped, None),
                NoticeKey::UnknownTextFile { session } => ("unknown_text_file", session, None),
            };
            conn.execute(
                "INSERT OR IGNORE INTO dismissals (change, notice, move, other_move) VALUES (?1, ?2, ?3, ?4)",
                params![change.id.to_string(), kind, mv.to_string(), other.map(|o| o.to_string())],
            )?;
        }
        ChangeKind::SetField { record, field, value } => {
            conn.execute(
                "INSERT OR IGNORE INTO fields (change, record, field, value) VALUES (?1, ?2, ?3, ?4)",
                params![change.id.to_string(), record, field, value],
            )?;
        }
        ChangeKind::Milestone { name, seen } => {
            conn.execute(
                "INSERT OR IGNORE INTO milestones (change, name) VALUES (?1, ?2)",
                params![change.id.to_string(), name],
            )?;
            for (device, counter) in seen {
                conn.execute(
                    "INSERT OR IGNORE INTO milestone_seen (change, device, counter) VALUES (?1, ?2, ?3)",
                    params![change.id.to_string(), device.to_string(), counter],
                )?;
            }
        }
    }
    Ok(())
}

fn read_changes(conn: &Connection) -> Result<Vec<Change>> {
    // Read every row of deletions into a HashMap<ChangeId, Vec<CharRange>> 

    // Pass 1: every deletion range, grouped by the change it belongs to.
    // (The two big passes read text straight from SQLite's buffer instead of copying it into a String first.)
    let mut ranges: FxHashMap<ChangeId, Vec<CharRange>> = FxHashMap::default();
    let mut stmt = conn.prepare("SELECT change, first_char, len FROM deletions ORDER BY rowid")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let change: ChangeId = column(row, 0)?.parse()?;
        let first: CharId = column(row, 1)?.parse()?;
        ranges.entry(change).or_default().push(CharRange { first, len: row.get(2)? });
    }
    // Pass 2: read every row of revert_targets into a HashMap<ChangeId, Vec<ChangeId>>
    let mut revert_targets: HashMap<ChangeId, Vec<ChangeId>> = HashMap::new();
    let mut stmt = conn.prepare("SELECT change, target FROM revert_targets ORDER BY rowid")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (change, target) = row?;
        let change: ChangeId = change.parse()?;
        let target: ChangeId = target.parse()?;
        revert_targets.entry(change).or_default().push(target);
    }

    // Pass 3a: read every row of move_seen into a HashMap<ChangeId, Vec<(DeviceId, u32)>>
    let mut move_seen: HashMap<ChangeId, Vec<(DeviceId, u32)>> = HashMap::new();
    // Device IDs are fixed-width hex, so sorting the text sorts the numbers too.
    let mut stmt = conn.prepare("SELECT change, device, counter FROM move_seen ORDER BY change, device")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u32>(2)?))
    })?;
    for row in rows {
        let (change, device, counter) = row?;
        let change: ChangeId = change.parse()?;
        let device: DeviceId = device.parse()?;
        move_seen.entry(change).or_default().push((device, counter));
    }
    // Pass 3b: where each Move's anchors go: (parent, side, right_origin, into).
    let mut moves: HashMap<ChangeId, (Option<CharId>, Side, Option<CharId>, Option<CharId>)> = HashMap::new();
    let mut stmt = conn.prepare("SELECT change, parent, side, right_origin, into_anchor FROM moves")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,         // change
            row.get::<_, Option<String>>(1)?, // parent
            row.get::<_, String>(2)?,         // side
            row.get::<_, Option<String>>(3)?, // right_origin
            row.get::<_, Option<String>>(4)?, // into_anchor
        ))
    })?;
    for row in rows {
        let (change, parent, side, right_origin, into) = row?;
        let change: ChangeId = change.parse()?;
        let parent: Option<CharId> = parent.map(|p| p.parse()).transpose()?;
        let side = side_from_str(&side)?;
        let right_origin: Option<CharId> = right_origin.map(|r| r.parse()).transpose()?;
        let into: Option<CharId> = into.map(|a| a.parse()).transpose()?;
        moves.insert(change, (parent, side, right_origin, into));
    }
    // Pass 3c: every piece, grouped by its Move, in piece order.
    // (moved_text is only a readable copy, so it isn't read back.)
    let mut move_pieces: HashMap<ChangeId, Vec<Span>> = HashMap::new();
    let mut stmt = conn.prepare("SELECT change, first_char, last_char, from_anchor FROM move_pieces ORDER BY change, piece")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,         // change
            row.get::<_, String>(1)?,         // first_char
            row.get::<_, String>(2)?,         // last_char
            row.get::<_, Option<String>>(3)?, // from_anchor
        ))
    })?;
    for row in rows {
        let (change, first, last, from) = row?;
        let change: ChangeId = change.parse()?;
        let first: CharId = first.parse()?;
        let last: CharId = last.parse()?;
        let from: Option<CharId> = from.map(|f| f.parse()).transpose()?;
        move_pieces.entry(change).or_default().push(Span { first, last, from });
    }

    // Pass 3d: which notice each Dismiss hides.
    let mut dismissals: HashMap<ChangeId, NoticeKey> = HashMap::new();
    let mut stmt = conn.prepare("SELECT change, notice, move, other_move FROM dismissals")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,         // change
            row.get::<_, String>(1)?,         // notice
            row.get::<_, String>(2)?,         // move
            row.get::<_, Option<String>>(3)?, // other_move
        ))
    })?;
    for row in rows {
        let (change, notice, mv, other) = row?;
        let change: ChangeId = change.parse()?;
        let mv: ChangeId = mv.parse()?;
        let key = match (notice.as_str(), other) {
            ("overlap", Some(other)) => NoticeKey::Overlap { later: mv, earlier: other.parse()? },
            ("loop", None) => NoticeKey::Loop { skipped: mv },
            ("unknown_text_file", None) => NoticeKey::UnknownTextFile { session: mv },
            (notice, other) => return Err(format!("Invalid dismissal: {notice} {other:?}").into()),
        };
        dismissals.insert(change, key);
    }

    // Pass 3e: which field each SetField sets.
    let mut fields: HashMap<ChangeId, (String, String, String)> = HashMap::new();
    let mut stmt = conn.prepare("SELECT change, record, field, value FROM fields")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let change: ChangeId = column(row, 0)?.parse()?;
        fields.insert(change, (row.get(1)?, row.get(2)?, row.get(3)?));
    }

    // Pass 3f: Milestones: their names, and what each holds.
    let mut milestone_names: HashMap<ChangeId, String> = HashMap::new();
    let mut stmt = conn.prepare("SELECT change, name FROM milestones")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        milestone_names.insert(column(row, 0)?.parse()?, row.get(1)?);
    }
    let mut milestone_seen: HashMap<ChangeId, Vec<(DeviceId, u32)>> = HashMap::new();
    // Device IDs are fixed-width hex, so sorting the text sorts the numbers too.
    let mut stmt = conn.prepare("SELECT change, device, counter FROM milestone_seen ORDER BY change, device")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let change: ChangeId = column(row, 0)?.parse()?;
        let device: DeviceId = column(row, 1)?.parse()?;
        milestone_seen.entry(change).or_default().push((device, row.get(2)?));
    }

    // Pass 4: read every change
    let mut stmt = conn.prepare(
        "SELECT c.id, c.hlc, c.kind, i.text, i.parent, i.side, i.right_origin, i.into_anchor, c.session
        FROM changes c LEFT JOIN insertions i ON i.change = c.id
        ORDER BY c.hlc, c.device",
    )?;
    let mut changes = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id: ChangeId = column(row, 0)?.parse()?;
        let hlc: i64 = row.get(1)?;
        // Rows from before Sessions were stored count as a Session of their own.
        let session: ChangeId = parse_optional(optional_column(row, 8)?)?.unwrap_or(id);
        let kind = match column(row, 2)? {
            "insert" => ChangeKind::Insert(insert_run_from(row)?),
            "delete" => {
                // ChangeKind::Delete with ranges from the HashMap
                let ranges = ranges.remove(&id).ok_or(format!("Missing deletion ranges for change {}", id))?;
                ChangeKind::Delete(ranges)
            }
            "replace" => {
                // Both of the above
                let ranges = ranges.remove(&id).ok_or(format!("Missing deletion ranges for change {}", id))?;
                ChangeKind::Replace { deleted: ranges, inserted: insert_run_from(row)? }
            }
            "revert" => {
                let targets = revert_targets.remove(&id).ok_or(format!("Missing revert targets for change {}", id))?;
                ChangeKind::Revert { targets }
            }
            "move" => {
                let (parent, side, right_origin, into) = moves.remove(&id).ok_or(format!("Missing move info for change {}", id))?;
                let pieces = move_pieces.remove(&id).ok_or(format!("Missing move pieces for change {}", id))?;
                // A Move made before its Device had seen anything has no rows here; that's not an error.
                let seen = move_seen.remove(&id).unwrap_or_default();
                ChangeKind::Move { pieces, parent, side, right_origin, seen, into }
            }
            "dismiss" => {
                let notice = dismissals.remove(&id).ok_or(format!("Missing dismissal for change {}", id))?;
                ChangeKind::Dismiss { notice }
            }
            "field" => {
                let (record, field, value) = fields.remove(&id).ok_or(format!("Missing field for change {}", id))?;
                ChangeKind::SetField { record, field, value }
            }
            "milestone" => {
                let name = milestone_names.remove(&id).ok_or(format!("Missing Milestone name for change {}", id))?;
                // A Milestone made before its Device had seen anything holds nothing; that's not an error.
                let seen = milestone_seen.remove(&id).unwrap_or_default();
                ChangeKind::Milestone { name, seen }
            }
            other => return Err(format!("Unknown change kind: {}", other).into()),
        };
        changes.push(Change { id, hlc: Hlc(hlc as u64), session, kind });
    }
    Ok(changes)
}

/// Builds an InsertRun from columns 3..=7 of a Change row (text, parent, side, right_origin,
/// into_anchor). Insert and Replace both use it.
fn insert_run_from(row: &rusqlite::Row) -> Result<InsertRun> {
    Ok(InsertRun {
        text: optional_column(row, 3)?.ok_or("Missing text for an insertion")?.to_string(),
        parent: parse_optional(optional_column(row, 4)?)?,
        side: side_from_str(optional_column(row, 5)?.unwrap_or(""))?,
        right_origin: parse_optional(optional_column(row, 6)?)?,
        into: parse_optional(optional_column(row, 7)?)?,
    })
}

/// A text column, borrowed from SQLite's buffer (no copy).
fn column<'row>(row: &'row rusqlite::Row, i: usize) -> Result<&'row str> {
    Ok(row.get_ref(i)?.as_str()?)
}

/// A text column that may be NULL, borrowed from SQLite's buffer.
fn optional_column<'row>(row: &'row rusqlite::Row, i: usize) -> Result<Option<&'row str>> {
    Ok(row.get_ref(i)?.as_str_or_null()?)
}

/// Parses an optional ID; a damaged one is an error, never silently None.
fn parse_optional<T: std::str::FromStr<Err = String>>(s: Option<&str>) -> Result<Option<T>> {
    Ok(s.map(|s| s.parse::<T>()).transpose()?)
}

// public functions open the file do one thing then close it.
pub fn save(path: &Path, changes: &[Change], doc: &Doc) -> Result<()> {
    // Open the database, migrate if necessary, and write all changes.
    let mut conn = Connection::open(path)?;
    migrate(&conn)?;
    let tx = conn.transaction()?;
    for change in changes {
        write_change(&tx, change, doc)?;
    }
    tx.commit()?;
    Ok(())
}
/// Every Change in a History file, in the fixed Change order.
pub fn read_all(path: &Path) -> Result<Vec<Change>> {
    let conn = Connection::open(path)?;
    migrate(&conn)?;
    read_changes(&conn)
}

pub fn load(path: &Path, device: DeviceId) -> Result<(Vec<Change>, Doc)> {
    let conn = Connection::open(path)?;
    migrate(&conn)?;
    let changes = read_changes(&conn)?;
    let mut doc = Doc::new(device);
    doc.apply_all(&changes); // works out the text once, not after every Change
    Ok((changes, doc))
}
/// Every table that holds Changes:
/// (name, column naming the Change, what the engine reads (must match between copies), columns to copy).
/// Readable copies such as `deleted_text` are copied but not compared: they're written when a
/// Change is saved and may legitimately differ.
const TABLES: &[(&str, &str, &str, &str)] = &[
    ("changes", "id", "id, device, counter, hlc, kind, COALESCE(session, id)", "id, device, counter, hlc, time, kind, session"),
    ("insertions", "change", "change, text, parent, side, right_origin, into_anchor", "change, text, parent, side, right_origin, into_anchor"),
    ("deletions", "change", "change, first_char, len", "change, first_char, len, deleted_text"),
    ("revert_targets", "change", "change, target", "change, target"),
    ("moves", "change", "change, parent, side, right_origin, into_anchor", "change, parent, side, right_origin, into_anchor"),
    ("move_pieces", "change", "change, piece, first_char, last_char, from_anchor", "change, piece, first_char, last_char, from_anchor, moved_text"),
    ("move_seen", "change", "change, device, counter", "change, device, counter"),
    ("dismissals", "change", "change, notice, move, other_move", "change, notice, move, other_move"),
    ("fields", "change", "change, record, field, value", "change, record, field, value"),
    ("milestones", "change", "change, name", "change, name"),
    ("milestone_seen", "change", "change, device, counter", "change, device, counter"),
];

/// Two copies of a History hold the same Change ID with different content. That can only
/// happen if two Devices used the same Device ID, so the merge refuses instead of guessing.
#[derive(Debug)]
pub struct SameIdDifferentContent {
    pub ids: Vec<String>,
}

impl std::fmt::Display for SameIdDifferentContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "refusing to merge: Changes {} have the same ID but different content in the two files \
                   (two Devices are probably using the same Device ID)", self.ids.join(", "))
    }
}

impl Error for SameIdDifferentContent {}

/// Merges a Conflict copy (`other`) into `main`: every Change from both, nothing lost.
/// All or nothing: if the copies disagree about any Change, or anything fails, `main` is untouched.
pub fn merge(main: &Path, other: &Path) -> Result<()> {
    if !other.is_file() {
        // Opening it would quietly create an empty database.
        return Err(format!("no such file: {}", other.display()).into());
    }
    migrate(&Connection::open(other)?)?; // bring the other file up to date first
    let conn = Connection::open(main)?;
    migrate(&conn)?;
    conn.execute("ATTACH DATABASE ?1 AS other", [other.to_string_lossy()])?;
    let result = merge_attached(&conn);
    conn.execute("DETACH DATABASE other", [])?;
    result
}

fn merge_attached(conn: &Connection) -> Result<()> {
    // 1. Every Change both copies have must be identical, in every table.
    let mut differing: Vec<String> = Vec::new();
    for (table, key, compare, _) in TABLES {
        for (a, b) in [("main", "other"), ("other", "main")] {
            let sql = format!(
                "SELECT {key} FROM (
                     SELECT {compare} FROM {a}.{table} WHERE {key} IN (SELECT id FROM {b}.changes)
                     EXCEPT
                     SELECT {compare} FROM {b}.{table} WHERE {key} IN (SELECT id FROM {a}.changes)
                 )"
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let id: String = row.get(0)?;
                if !differing.contains(&id) {
                    differing.push(id);
                }
            }
        }
    }
    if !differing.is_empty() {
        differing.sort();
        return Err(Box::new(SameIdDifferentContent { ids: differing }));
    }

    // 2. Copy everything `main` doesn't have yet, in one transaction. (Text file records too:
    //    the same contents may be recorded by both, with any version that produced them.)
    let tx = conn.unchecked_transaction()?;
    for (table, _, _, columns) in TABLES {
        tx.execute(&format!("INSERT OR IGNORE INTO main.{table} ({columns}) SELECT {columns} FROM other.{table}"), [])?;
    }
    let columns = "hash, version, device, written, time";
    tx.execute(&format!("INSERT OR IGNORE INTO main.text_files ({columns}) SELECT {columns} FROM other.text_files"), [])?;
    tx.commit()?;
    Ok(())
}

// ---- The Text file ----

/// SHA-256 of a Text file's contents, as hex.
fn text_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A version as stored: "device:counter" per Device, comma-separated (readable, and sorted).
fn version_to_str(version: &[(DeviceId, u32)]) -> String {
    version.iter().map(|&(device, counter)| ChangeId { device, counter }.to_string()).collect::<Vec<_>>().join(",")
}

fn version_from_str(s: &str) -> Result<Vec<(DeviceId, u32)>> {
    s.split(',').filter(|p| !p.is_empty()).map(|p| {
        let id: ChangeId = p.parse()?;
        Ok((id.device, id.counter))
    }).collect()
}

/// Writes the Work's text to its Text file, and records the file's hash and the version it was
/// built from, so the app recognises it later (on any Device, also in Conflict copies).
/// The file is written next to itself first and then renamed over it, so a crash never leaves
/// half a file.
pub fn write_text_file(db: &Path, text_file: &Path, doc: &Doc, now_ms: u64) -> Result<()> {
    let text = doc.committed_text();
    let conn = Connection::open(db)?;
    migrate(&conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO text_files (hash, version, device, written, time)
         VALUES (?1, ?2, ?3, ?4, datetime(?4 / 1000, 'unixepoch'))",
        params![text_hash(&text), version_to_str(&doc.version()), doc.device().to_string(), now_ms as i64],
    )?;
    drop(conn); // recorded before the file exists, so a file the app wrote is always recognised
    let partial = text_file.with_extension("typ.writing");
    std::fs::write(&partial, &text)?;
    std::fs::rename(&partial, text_file)?;
    Ok(())
}

/// Reads a Text file (or a Conflict copy of one). If it holds text the app wrote, on any
/// Device, there's nothing to do. Otherwise it was edited outside the app: the difference from
/// the version last written is recorded as Changes in `doc` and saved. Returns those Changes.
pub fn import_text_file(db: &Path, text_file: &Path, doc: &mut Doc, now_ms: u64) -> Result<Vec<Change>> {
    let text = std::fs::read_to_string(text_file)?;
    let conn = Connection::open(db)?;
    migrate(&conn)?;
    let known: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM text_files WHERE hash = ?1)", [text_hash(&text)], |r| r.get(0))?;
    if known {
        return Ok(Vec::new()); // the app wrote exactly this
    }
    let last: Option<String> = conn
        .query_row("SELECT version FROM text_files ORDER BY written DESC, device DESC LIMIT 1", [], |r| r.get(0))
        .optional()?;
    drop(conn);
    let base = last.map(|v| version_from_str(&v)).transpose()?;
    let changes = doc.import_text(base.as_deref(), &text, now_ms);
    save(db, &changes, doc)?;
    Ok(changes)
}

/// Keeps a Work's Text file in step: records an External edit if there is one, then writes the
/// current text. Returns the Changes the External edit became (empty if there was none).
pub fn sync_text_file(db: &Path, text_file: &Path, doc: &mut Doc, now_ms: u64) -> Result<Vec<Change>> {
    let changes = if text_file.exists() { import_text_file(db, text_file, doc, now_ms)? } else { Vec::new() };
    write_text_file(db, text_file, doc, now_ms)?;
    Ok(changes)
}

#[cfg(test)]
mod test {
    use super::*;
    
    #[test]
    fn test_migrate() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
        assert_eq!(version, 8);
    }

    #[test]
    fn test_write_and_read_change() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut doc = Doc::new(DeviceId(1));
        let change = doc.insert_local(0, "Hello", 1000);
        write_change(&conn, &change, &doc).unwrap();
        let changes = read_changes(&conn).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0], change);
    }
    #[test]
    fn delete_with_two_ranges_round_trips() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut doc = Doc::new(DeviceId(1));
        let a = doc.insert_local(0, "cat", 1000);
        let b = doc.insert_local(1, "h", 2000);   // "chat"
        let c = doc.delete_local(1, 2, 3000);     // delete "ha": two ranges
        for change in [&a, &b, &c] {
            write_change(&conn, change, &doc).unwrap();
        }

        assert_eq!(read_changes(&conn).unwrap(), vec![a, b, c]);

        // The readable copy holds the deleted letters.
        let deleted: Vec<String> = conn
            .prepare("SELECT deleted_text FROM deletions ORDER BY rowid").unwrap()
            .query_map([], |r| r.get(0)).unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(deleted, vec!["h", "a"]);
    }
    #[test]
    fn save_load() {
        // tempfile::tempdir() then dir.path().join("test.db") then save then load and check equality.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut doc = Doc::new(DeviceId(1));

        let a = doc.insert_local(0, "cat", 1000);
        let b = doc.insert_local(1, "h", 2000);   // "chat"
        let c = doc.delete_local(1, 2, 3000);     // delete "ha": two ranges

        let changes = vec![a, b, c];

        save(&path, &changes, &doc).unwrap();

        let (loaded_changes, loaded_doc) = load(&path, DeviceId(1)).unwrap();

        assert_eq!(loaded_changes, changes);
        assert_eq!(loaded_doc.text(), doc.text());
    }

    #[test]
    fn revert_with_dependents_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("work.db");
        let mut doc = Doc::new(DeviceId(1));
        let rain = doc.insert_local(0, "The rain had stopped.", 1000);
        let del = doc.delete_local(13, 7, 2000);
        let eased = doc.insert_local(13, "eased", 3000);
        let revert = doc.revert_local(rain.id, true, 4000);
        assert_eq!(doc.text(), "");

        let changes = vec![rain, del, eased, revert];
        save(&path, &changes, &doc).unwrap();
        let (loaded, loaded_doc) = load(&path, DeviceId(1)).unwrap();

        assert_eq!(loaded, changes);
        assert_eq!(loaded_doc.text(), "");
    }

    #[test]
    fn merge_two() {
        // Create two databases, each with some changes, then merge them and check that the merged database has all changes.
        let dir = tempfile::tempdir().unwrap();
        let path1 = dir.path().join("db1.db");
        let path2 = dir.path().join("db2.db");

        let mut doc1 = Doc::new(DeviceId(1));

        let a = doc1.insert_local(0, "cat", 1000);
        let b = doc1.insert_local(1, "h", 2000);   // "chat"
        save(&path1, &[a.clone(), b.clone()], &doc1).unwrap();

        let mut doc2 = Doc::new(DeviceId(2));
        let c = doc2.insert_local(0, "dog", 1000);
        save(&path2, &[c.clone()], &doc2).unwrap();

        merge(&path1, &path2).unwrap();

        let (merged_changes, merged_doc) = load(&path1, DeviceId(1)).unwrap();

        assert_eq!(merged_changes.len(), 3);
        assert!(merged_changes.contains(&a));
        assert!(merged_changes.contains(&b));
        assert!(merged_changes.contains(&c));
        assert_eq!(merged_doc.text(), "chatdog");
    }

    /// A Doc whose Device has a name and has typed and deleted something.
    fn named_history() -> (Doc, Vec<Change>) {
        let mut doc = Doc::new(DeviceId(1));
        let changes = vec![
            doc.set_device_name_local("Laptop", 1000),
            doc.insert_local(0, "The rain had stopped.", 2000),
            doc.delete_local(13, 7, 3000), // "stopped"
        ];
        (doc, changes)
    }

    #[test]
    fn sessions_and_fields_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("work.db");
        let (doc, changes) = named_history();
        save(&path, &changes, &doc).unwrap();
        let (loaded, loaded_doc) = load(&path, DeviceId(1)).unwrap();
        assert_eq!(loaded, changes); // sessions included
        assert_eq!(loaded_doc.device_name(DeviceId(1)), Some("Laptop"));
    }

    #[test]
    fn the_history_view_reads_like_sentences() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("work.db");
        let (doc, changes) = named_history();
        save(&path, &changes, &doc).unwrap();
        let conn = Connection::open(&path).unwrap();
        let rows: Vec<(String, String, String)> = conn
            .prepare("SELECT device, kind, text FROM history").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap()
            .map(|r| r.unwrap())
            .collect();
        let laptop = |kind: &str, text: &str| ("Laptop".to_string(), kind.to_string(), text.to_string());
        assert_eq!(rows, vec![
            laptop("field", "device/00000001 name = Laptop"),
            laptop("insert", "The rain had stopped."),
            laptop("delete", "stopped"),
        ]);
    }

    #[test]
    fn milestones_round_trip_and_read_well() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("work.db");
        let (mut doc, mut changes) = named_history();
        let draft = doc.make_milestone_local("Draft 2, sent to editor", 4000);
        changes.push(draft.clone());
        changes.push(doc.insert_local(0, "Later. ", 5000));
        save(&path, &changes, &doc).unwrap();
        let (loaded, loaded_doc) = load(&path, DeviceId(1)).unwrap();
        assert_eq!(loaded, changes);
        assert_eq!(loaded_doc.milestones()[0].name, "Draft 2, sent to editor");
        assert_eq!(loaded_doc.text_at_milestone(draft.id), "The rain had .");

        let conn = Connection::open(&path).unwrap();
        let text: String = conn.query_row("SELECT text FROM history WHERE kind = 'milestone'", [], |r| r.get(0)).unwrap();
        assert_eq!(text, "Draft 2, sent to editor");
    }

    #[test]
    fn files_this_app_cant_safely_handle_are_refused_untouched() {
        let dir = tempfile::tempdir().unwrap();
        // Written by a newer version of the app.
        let newer = dir.path().join("newer.db");
        {
            let conn = Connection::open(&newer).unwrap();
            migrate(&conn).unwrap();
            conn.execute_batch(&format!("PRAGMA user_version = {}", STORAGE_VERSION + 1)).unwrap();
        }
        let err = load(&newer, DeviceId(1)).err().expect("refused");
        assert!(err.to_string().contains("newer version"), "{err}");
        // Some other database entirely.
        let foreign = dir.path().join("foreign.db");
        Connection::open(&foreign).unwrap().execute_batch("CREATE TABLE rows (x TEXT)").unwrap();
        let before = std::fs::read(&foreign).unwrap();
        assert!(load(&foreign, DeviceId(1)).err().expect("refused").to_string().contains("not a History"));
        assert_eq!(std::fs::read(&foreign).unwrap(), before);
        // A merge with a file that doesn't exist doesn't create one.
        let missing = dir.path().join("missing.db");
        assert!(merge(&newer, &missing).is_err());
        assert!(!missing.exists());
    }

    #[test]
    fn the_app_recognises_its_own_text_file() {
        let dir = tempfile::tempdir().unwrap();
        let (db, typ) = (dir.path().join("work.db"), dir.path().join("work.typ"));
        let mut doc = Doc::new(DeviceId(1));
        let typed = doc.insert_local(0, "The rain had stopped.\n", 1000);
        save(&db, &[typed], &doc).unwrap();
        write_text_file(&db, &typ, &doc, 1500).unwrap();
        assert_eq!(std::fs::read_to_string(&typ).unwrap(), "The rain had stopped.\n");
        assert!(sync_text_file(&db, &typ, &mut doc, 2000).unwrap().is_empty());
    }

    #[test]
    fn an_external_edit_is_recorded_saved_and_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let (db, typ) = (dir.path().join("work.db"), dir.path().join("work.typ"));
        let mut doc = Doc::new(DeviceId(1));
        let typed = doc.insert_local(0, "The rain had stopped.\n", 1000);
        save(&db, &[typed.clone()], &doc).unwrap();
        write_text_file(&db, &typ, &doc, 1500).unwrap();
        let more = doc.insert_local(22, "Gulls circled.\n", 2000); // the app goes on, unwritten
        save(&db, &[more], &doc).unwrap();

        std::fs::write(&typ, "The rain had eased.\n").unwrap(); // edited in vim
        let recorded = sync_text_file(&db, &typ, &mut doc, 3000).unwrap();
        assert!(!recorded.is_empty());
        assert_eq!(doc.text(), "The rain had eased.\nGulls circled.\n");
        assert_eq!(std::fs::read_to_string(&typ).unwrap(), doc.text()); // rewritten with both
        let (_, reopened) = load(&db, DeviceId(1)).unwrap();
        assert_eq!(reopened.text(), doc.text());                         // and saved
        assert!(sync_text_file(&db, &typ, &mut doc, 4000).unwrap().is_empty()); // recognised now
    }

    #[test]
    fn a_text_file_written_on_another_device_is_recognised_after_a_merge() {
        let dir = tempfile::tempdir().unwrap();
        let (laptop_db, phone_db) = (dir.path().join("laptop.db"), dir.path().join("phone.db"));
        let copy = dir.path().join("work.sync-conflict.typ");
        let mut laptop = Doc::new(DeviceId(1));
        let mut phone = Doc::new(DeviceId(2));
        let base = laptop.insert_local(0, "abc", 1000);
        phone.apply(&base);
        let theirs = phone.insert_local(3, "def", 2000);
        save(&laptop_db, &[base.clone()], &laptop).unwrap();
        save(&phone_db, &[base, theirs], &phone).unwrap();
        write_text_file(&phone_db, &copy, &phone, 2500).unwrap(); // the phone's Text file, as a Conflict copy
        merge(&laptop_db, &phone_db).unwrap();
        let (_, mut laptop) = load(&laptop_db, DeviceId(1)).unwrap();
        assert!(import_text_file(&laptop_db, &copy, &mut laptop, 3000).unwrap().is_empty());
    }

    #[test]
    fn merge_refuses_the_same_id_with_different_content() {
        // Two Devices that wrongly share Device ID 1 both make Change 00000001:1.
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.db"), dir.path().join("b.db"));
        let mut laptop = Doc::new(DeviceId(1));
        let mut clone = Doc::new(DeviceId(1));
        let mine = laptop.insert_local(0, "mine", 1000);
        let theirs = clone.insert_local(0, "theirs", 1000);
        save(&a, &[mine.clone()], &laptop).unwrap();
        save(&b, &[theirs], &clone).unwrap();

        let err = merge(&a, &b).unwrap_err();
        let err = err.downcast_ref::<SameIdDifferentContent>().expect("the specific error");
        assert_eq!(err.ids, vec!["00000001:1".to_string()]);
        // Nothing was copied: the main file is exactly as it was.
        let (kept, doc) = load(&a, DeviceId(1)).unwrap();
        assert_eq!(kept, vec![mine]);
        assert_eq!(doc.text(), "mine");
    }

    #[test]
    fn merge_accepts_shared_changes_and_is_harmless_twice() {
        // Both files hold the same "abcdef"; each adds its own Change; merging twice changes nothing more.
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.db"), dir.path().join("b.db"));
        let mut laptop = Doc::new(DeviceId(1));
        let mut phone = Doc::new(DeviceId(2));
        let base = laptop.insert_local(0, "abcdef", 1000);
        phone.apply(&base);
        let x = laptop.insert_local(6, "X", 2000);
        let y = phone.delete_local(0, 1, 2000);
        save(&a, &[base.clone(), x], &laptop).unwrap();
        save(&b, &[base, y], &phone).unwrap();
        merge(&a, &b).unwrap();
        merge(&a, &b).unwrap();
        let (all, doc) = load(&a, DeviceId(1)).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(doc.text(), "bcdefX");
    }

    #[test]
    fn dismissals_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("work.db");
        let mut laptop = Doc::new(DeviceId(1));
        let mut phone = Doc::new(DeviceId(2));
        let base = laptop.insert_local(0, "abcdef", 1000);
        phone.apply(&base);
        let a = laptop.move_local(1, 3, 6, 2000);      // overlapping Moves, made offline
        let b = phone.move_local(2, 3, 0, 2000);
        laptop.apply(&b);
        let key = laptop.merge_notices()[0].key;
        let dismiss = laptop.dismiss_local(key, 3000);

        let changes = vec![base, a, b, dismiss];
        save(&path, &changes, &laptop).unwrap();
        let (loaded, loaded_doc) = load(&path, DeviceId(1)).unwrap();
        assert_eq!(loaded, changes);
        assert!(loaded_doc.merge_notices().is_empty());   // still dismissed after loading
    }

    #[test]
    fn moves_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("work.db");
        let mut doc = Doc::new(DeviceId(1));
        let a = doc.insert_local(0, "abcdef", 1000);
        let b = doc.move_local(2, 2, 6, 2000);    // "abefcd"
        let c = doc.insert_local(6, "X", 3000);   // "abefcdX"
        let d = doc.move_local(4, 3, 0, 4000);    // "cdX" to the start: two pieces
        assert_eq!(doc.text(), "cdXabef");

        let changes = vec![a, b, c, d];
        save(&path, &changes, &doc).unwrap();
        let (loaded, loaded_doc) = load(&path, DeviceId(1)).unwrap();

        assert_eq!(loaded, changes);              // every field, including pieces and seen
        assert_eq!(loaded_doc.text(), "cdXabef");

        // The readable copies hold the moved text.
        let conn = Connection::open(&path).unwrap();
        let moved: Vec<String> = conn
            .prepare("SELECT moved_text FROM move_pieces ORDER BY change, piece").unwrap()
            .query_map([], |r| r.get(0)).unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(moved, vec!["cd", "cd", "X"]);
    }
}