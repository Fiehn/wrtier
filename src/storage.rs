use std::error::Error;
use std::path::Path;
use rusqlite::{params, Connection};
use crate::change::{Change, ChangeKind, CharRange, InsertRun};
use crate::doc::Doc;
use crate::ids::{ChangeId, CharId, Side, DeviceId, Hlc};
use std::collections::HashMap;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

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
            },
        ],
    )?;
    match &change.kind {
        ChangeKind::Insert(run) => {
            conn.execute(
                "INSERT OR IGNORE INTO insertions (change, text, parent, side, right_origin) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    change.id.to_string(),
                    run.text,
                    run.parent.map(|id| id.to_string()),
                    match run.side {
                        Side::Left => "L",
                        Side::Right => "R",
                    },
                    run.right_origin.map(|id| id.to_string()),
                ],
            )?;
        }
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
            conn.execute(
                "INSERT OR IGNORE INTO insertions (change, text, parent, side, right_origin) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    change.id.to_string(),
                    inserted.text,
                    inserted.parent.map(|id| id.to_string()),
                    match inserted.side {
                        Side::Left => "L",
                        Side::Right => "R",
                    },
                    inserted.right_origin.map(|id| id.to_string()),
                ],
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

    // Pass 2: read every change
    let mut stmt = conn.prepare(
        "SELECT c.id, c.hlc, c.kind, i.text, i.parent, i.side, i.right_origin
        FROM changes c LEFT JOIN insertions i ON i.change = c.id
        ORDER BY c.hlc, c.device",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?, // id
            row.get::<_, i64>(1)?,    // hlc
            row.get::<_, String>(2)?, // kind
            row.get::<_, Option<String>>(3)?, // text
            row.get::<_, Option<String>>(4)?, // parent
            row.get::<_, Option<String>>(5)?, // side
            row.get::<_, Option<String>>(6)?, // right_origin            
        ))
    })?;

    let mut changes = Vec::new();
    for row in rows {
        let (id, hlc, kind, text, parent, side, right_origin) = row?;
        let id: ChangeId = id.parse()?;
        let kind = match kind.as_str() {
            "insert" => {
                let text = text.ok_or("Missing text for insert")?;
                let parent = parent.map(|p| p.parse()).transpose()?;
                let side = match side.as_deref() {
                    Some("L") => Side::Left,
                    Some("R") => Side::Right,
                    _ => return Err(format!("Invalid side: {:?}", side).into()),
                };
                let right_origin = right_origin.map(|ro| ro.parse()).transpose()?;
                ChangeKind::Insert(InsertRun { text, parent, side, right_origin })
            }
            "delete" => {
                // ChangeKind::Delete with ranges from the HashMap
                let ranges = ranges.remove(&id).ok_or(format!("Missing deletion ranges for change {}", id))?;
                ChangeKind::Delete(ranges)
            }
            "replace" => {
                // Both of the above
                let ranges = ranges.remove(&id).ok_or(format!("Missing deletion ranges for change {}", id))?;
                let text = text.ok_or("Missing text for replace")?;
                let parent = parent.map(|p| p.parse()).transpose()?;
                let side = match side.as_deref() {
                    Some("L") => Side::Left,
                    Some("R") => Side::Right,
                    _ => return Err(format!("Invalid side: {:?}", side).into()),
                };
                let right_origin = right_origin.map(|ro| ro.parse()).transpose()?;
                ChangeKind::Replace { deleted: ranges, inserted: InsertRun { text, parent, side, right_origin } }
            }
            other => return Err(format!("Unknown change kind: {}", other).into()),
        };
        changes.push(Change { id, hlc: Hlc(hlc as u64), kind });
    }
    Ok(changes)
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
    let conn = Connection::open(main)?;
    migrate(&conn)?;
    conn.execute("ATTACH DATABASE ?1 AS other", [other.to_string_lossy()])?;
    conn.execute("INSERT OR IGNORE INTO changes SELECT * FROM other.changes", [])?;
    conn.execute("INSERT OR IGNORE INTO insertions SELECT * FROM other.insertions", [])?;
    conn.execute("INSERT OR IGNORE INTO deletions SELECT * FROM other.deletions", [])?;
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
        assert_eq!(version, 1);
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
}