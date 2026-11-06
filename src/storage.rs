use std::error::Error;
use std::path::Path;
use rusqlite::{params, Connection};
use crate::change::{Change, ChangeKind, CharRange, InsertRun, NoticeKey, Span};
use crate::doc::Doc;
use crate::ids::{ChangeId, CharId, Side, DeviceId, Hlc};
use std::collections::HashMap;


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

// Inner functions take a connection
fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
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
        "INSERT OR IGNORE INTO changes (id, device, counter, hlc, time, kind) VALUES (?1, ?2, ?3, ?4, datetime(?4 / 1000, 'unixepoch'), ?5)",
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
            },
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
            };
            conn.execute(
                "INSERT OR IGNORE INTO dismissals (change, notice, move, other_move) VALUES (?1, ?2, ?3, ?4)",
                params![change.id.to_string(), kind, mv.to_string(), other.map(|o| o.to_string())],
            )?;
        }
    }
    Ok(())
}

fn read_changes(conn: &Connection) -> Result<Vec<Change>> {
    // Read every row of deletions into a HashMap<ChangeId, Vec<CharRange>> 

    // Pass 1: every deletion range, grouped by the change it belongs to.
    let mut ranges: HashMap<ChangeId, Vec<CharRange>> = HashMap::new();
    let mut stmt = conn.prepare("SELECT change, first_char, len FROM deletions ORDER BY rowid")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u32>(2)?))
    })?;
    for row in rows {
        let (change, first, len) = row?;
        let change: ChangeId = change.parse()?;
        let first: CharId = first.parse()?;
        ranges.entry(change).or_default().push(CharRange { first, len });
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
            (notice, other) => return Err(format!("Invalid dismissal: {notice} {other:?}").into()),
        };
        dismissals.insert(change, key);
    }

    // Pass 4: read every change
    let mut stmt = conn.prepare(
        "SELECT c.id, c.hlc, c.kind, i.text, i.parent, i.side, i.right_origin, i.into_anchor
        FROM changes c LEFT JOIN insertions i ON i.change = c.id
        ORDER BY c.hlc, c.device",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?, // id
            row.get::<_, i64>(1)?,    // hlc
            row.get::<_, String>(2)?, // kind
            (
                row.get::<_, Option<String>>(3)?, // text
                row.get::<_, Option<String>>(4)?, // parent
                row.get::<_, Option<String>>(5)?, // side
                row.get::<_, Option<String>>(6)?, // right_origin
                row.get::<_, Option<String>>(7)?, // into_anchor
            ),
        ))
    })?;

    let mut changes = Vec::new();
    for row in rows {
        let (id, hlc, kind, insertion) = row?;
        let id: ChangeId = id.parse()?;
        let kind = match kind.as_str() {
            "insert" => ChangeKind::Insert(insert_run_from(insertion)?),
            "delete" => {
                // ChangeKind::Delete with ranges from the HashMap
                let ranges = ranges.remove(&id).ok_or(format!("Missing deletion ranges for change {}", id))?;
                ChangeKind::Delete(ranges)
            }
            "replace" => {
                // Both of the above
                let ranges = ranges.remove(&id).ok_or(format!("Missing deletion ranges for change {}", id))?;
                ChangeKind::Replace { deleted: ranges, inserted: insert_run_from(insertion)? }
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
            other => return Err(format!("Unknown change kind: {}", other).into()),
        };
        changes.push(Change { id, hlc: Hlc(hlc as u64), kind });
    }
    Ok(changes)
}

/// The `insertions` columns of one row: text, parent, side, right_origin, into_anchor.
type InsertionRow = (Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);

/// Builds an InsertRun from its row. Insert and Replace both use it.
fn insert_run_from((text, parent, side, right_origin, into): InsertionRow) -> Result<InsertRun> {
    Ok(InsertRun {
        text: text.ok_or("Missing text for an insertion")?,
        parent: parent.map(|p| p.parse()).transpose()?,
        side: side_from_str(side.as_deref().unwrap_or(""))?,
        right_origin: right_origin.map(|r| r.parse()).transpose()?,
        into: into.map(|a| a.parse()).transpose()?,
    })
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
pub fn load(path: &Path, device: DeviceId) -> Result<(Vec<Change>, Doc)> {
    let conn = Connection::open(path)?;
    migrate(&conn)?;
    let changes = read_changes(&conn)?;
    let mut doc = Doc::new(device);
    for change in &changes {
        doc.apply(change);
    }
    Ok((changes, doc))
}
pub fn merge(main: &Path, other: &Path) -> Result<()> {
    // ATTACH the other file and run INSERT OR IGNORE INTO main.changes SELECT * FROM other.changes
    migrate(&Connection::open(other)?)?;   // bring the other file up to date first
    let conn = Connection::open(main)?;
    migrate(&conn)?;
    conn.execute("ATTACH DATABASE ?1 AS other", [other.to_string_lossy()])?;
    conn.execute("INSERT OR IGNORE INTO changes SELECT * FROM other.changes", [])?;
    conn.execute("INSERT OR IGNORE INTO insertions SELECT * FROM other.insertions", [])?;
    conn.execute("INSERT OR IGNORE INTO deletions SELECT * FROM other.deletions", [])?;
    conn.execute("INSERT OR IGNORE INTO revert_targets SELECT * FROM other.revert_targets", [])?;
    conn.execute("INSERT OR IGNORE INTO moves SELECT * FROM other.moves", [])?;
    conn.execute("INSERT OR IGNORE INTO move_pieces SELECT * FROM other.move_pieces", [])?;
    conn.execute("INSERT OR IGNORE INTO move_seen SELECT * FROM other.move_seen", [])?;
    conn.execute("INSERT OR IGNORE INTO dismissals SELECT * FROM other.dismissals", [])?;
    Ok(())
    // TODO: Checks of same id different content.
}

#[cfg(test)]
mod test {
    use super::*;
    
    #[test]
    fn test_migrate() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
        assert_eq!(version, 5);
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