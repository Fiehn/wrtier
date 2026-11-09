//! Conflict copies: when two Devices change a file before syncing, Syncthing keeps both and
//! renames one, e.g. `work.sync-conflict-20261105-082020-JGMDN7C.db`. They're expected, not
//! errors (ADR 0002): a History file's copy is merged back, a Text file's copy is read like an
//! External edit, and then the copy is deleted.
//!
//! A copy that can't be absorbed (two Devices sharing a Device ID, a file that isn't a History,
//! a damaged file) is never deleted: it's kept and reported.

use std::path::{Path, PathBuf};
use crate::change::Change;
use crate::doc::Doc;
use crate::storage;

/// Every Conflict copy of `path`, next to it, oldest name first.
///
/// Syncthing names them `<name>.sync-conflict-<YYYYMMDD>-<HHMMSS>-<device>.<extension>`, and a
/// copy of a copy gets a second such part. Only names that match exactly count.
pub fn conflict_copies(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem), Some(ext)) = (
        path.parent(),
        path.file_stem().and_then(|s| s.to_str()),
        path.extension().and_then(|s| s.to_str()),
    ) else {
        return Vec::new();
    };
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut copies: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            is_conflict_name(&name, stem, ext).then(|| e.path())
        })
        .collect();
    copies.sort();
    copies
}

/// Is `name` a Conflict copy of `<stem>.<ext>`?
fn is_conflict_name(name: &str, stem: &str, ext: &str) -> bool {
    let Some(middle) = name.strip_prefix(stem).and_then(|rest| rest.strip_suffix(ext)).and_then(|m| m.strip_suffix('.')) else {
        return false;
    };
    // `middle` is one or more ".sync-conflict-YYYYMMDD-HHMMSS-DEVICE" parts.
    let parts: Vec<&str> = middle.split(".sync-conflict-").collect();
    parts.len() >= 2 && parts[0].is_empty() && parts[1..].iter().all(|p| {
        let fields: Vec<&str> = p.split('-').collect();
        fields.len() == 3
            && fields[0].len() == 8 && fields[0].bytes().all(|b| b.is_ascii_digit())
            && fields[1].len() == 6 && fields[1].bytes().all(|b| b.is_ascii_digit())
            && !fields[2].is_empty() && fields[2].bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// The file a Conflict copy is a copy of (`Rain.sync-conflict-…-ABC.db` → `Rain.db`, next to
/// it), or None if `copy` isn't named like a Conflict copy.
pub fn main_of(copy: &Path) -> Option<PathBuf> {
    let name = copy.file_name()?.to_str()?;
    let stem = &name[..name.find(".sync-conflict-")?];
    let (_, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && is_conflict_name(name, stem, ext)).then(|| copy.with_file_name(format!("{stem}.{ext}")))
}

/// What absorbing the Conflict copies did.
#[derive(Debug, Default)]
pub struct Absorbed {
    /// History copies merged into the main file, then deleted.
    pub merged: Vec<PathBuf>,
    /// Text file copies read (recognised, or recorded as an External edit), then deleted.
    pub read: Vec<PathBuf>,
    /// Copies that couldn't be absorbed and were left in place, with the reason.
    pub kept: Vec<(PathBuf, String)>,
    /// Text file copies written from Changes this History doesn't have yet: left in place, to
    /// be read once the History catches up. (Not a problem: it's only waiting.)
    pub waiting: Vec<PathBuf>,
    /// The Changes that came in: from merged History copies, and from External edits.
    pub changes: Vec<Change>,
}

/// Absorbs every Conflict copy of a Work's History file (`db`) and Text file into the main
/// files and into the open `doc`, and deletes them. History copies go first, so a Text file
/// another Device wrote is recognised as such.
pub fn absorb_conflict_copies(db: &Path, text_file: &Path, doc: &mut Doc, now_ms: u64) -> Absorbed {
    let mut absorbed = Absorbed::default();

    for copy in conflict_copies(db) {
        match storage::merge(db, &copy) {
            Ok(()) => match std::fs::remove_file(&copy) {
                Ok(()) => absorbed.merged.push(copy),
                Err(e) => absorbed.kept.push((copy, format!("merged, but couldn't delete it: {e}"))),
            },
            Err(e) => absorbed.kept.push((copy, e.to_string())),
        }
    }
    // Bring the open Doc up to date with whatever the merges brought in.
    match storage::read_all(db) {
        Ok(all) => {
            let new: Vec<Change> = all.into_iter().filter(|c| !doc.has(c.id)).collect();
            doc.apply_all(&new);
            absorbed.changes.extend(new);
        }
        Err(e) => absorbed.kept.push((db.to_path_buf(), format!("couldn't read the merged History: {e}"))),
    }

    for copy in conflict_copies(text_file) {
        match storage::read_text_file(db, &copy, doc, now_ms) {
            Ok(storage::TextFileRead::NotYet) => absorbed.waiting.push(copy),
            Ok(read) => {
                if let storage::TextFileRead::Imported(changes) = read {
                    absorbed.changes.extend(changes);
                }
                match std::fs::remove_file(&copy) {
                    Ok(()) => absorbed.read.push(copy),
                    Err(e) => absorbed.kept.push((copy, format!("read, but couldn't delete it: {e}"))),
                }
            }
            Err(e) => absorbed.kept.push((copy, e.to_string())),
        }
    }
    absorbed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::DeviceId;

    #[test]
    fn syncthing_names_are_recognised_exactly() {
        let yes = [
            "test.sync-conflict-20261105-082020-JGMDN7C.db",
            "test.sync-conflict-20261105-082310-US2Y42W.db",
            "test.sync-conflict-20261105-082020-JGMDN7C.sync-conflict-20261106-101500-US2Y42W.db",
        ];
        let no = [
            "test.db",
            "test2.sync-conflict-20261105-082020-JGMDN7C.db",  // another Work
            "test.sync-conflict-20261105-082020-JGMDN7C.typ", // another kind of file
            "test.sync-conflict-2026115-082020-JGMDN7C.db",   // a date that isn't 8 digits
            "test.sync-conflict-bad.db",
            "test.sync-conflict-20261105-082020-.db",
            "test.db-journal",
        ];
        for name in yes {
            assert!(is_conflict_name(name, "test", "db"), "{name}");
        }
        for name in no {
            assert!(!is_conflict_name(name, "test", "db"), "{name}");
        }
    }

    #[test]
    fn a_copy_names_its_main_file() {
        let main = |name: &str| main_of(Path::new("vault/Short stories").join(name).as_path())
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned());
        assert_eq!(main("Rain.sync-conflict-20261105-082020-JGMDN7C.db").as_deref(), Some("Rain.db"));
        assert_eq!(main("The Long Night.sync-conflict-20261105-082020-JGMDN7C.typ").as_deref(), Some("The Long Night.typ"));
        assert_eq!(main("Rain.sync-conflict-20261105-082020-JGMDN7C.sync-conflict-20261106-101500-US2Y42W.db").as_deref(), Some("Rain.db"));
        assert_eq!(main("Rain.db"), None);
        assert_eq!(main("Rain.sync-conflict-bad.db"), None);
        assert_eq!(main("Rain.sync-conflict-20261105-082020-JGMDN7C"), None); // no extension
        assert_eq!(main(".sync-conflict-20261105-082020-JGMDN7C.db"), None);  // no name
    }

    /// A Vault folder with a Work `work.db`/`work.typ`, written by the laptop.
    fn work(dir: &Path) -> (PathBuf, PathBuf, Doc, Vec<Change>) {
        let (db, typ) = (dir.join("work.db"), dir.join("work.typ"));
        let mut laptop = Doc::new(DeviceId(1));
        let changes = vec![laptop.insert_local(0, "abcdef", 1000)];
        storage::save(&db, &changes, &laptop).unwrap();
        storage::write_text_file(&db, &typ, &laptop, "", 1500).unwrap();
        (db, typ, laptop, changes)
    }

    #[test]
    fn a_history_copy_is_merged_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let (db, typ, mut laptop, base) = work(dir.path());
        // The phone wrote its own Changes to the file at the same time: Syncthing kept both.
        let mut phone = Doc::new(DeviceId(2));
        phone.apply_all(&base);
        let theirs = phone.insert_local(6, "XY", 2000);
        let copy = dir.path().join("work.sync-conflict-20261105-082020-JGMDN7C.db");
        storage::save(&copy, &[base[0].clone(), theirs.clone()], &phone).unwrap();
        let mine = laptop.insert_local(0, "Z", 2000);
        storage::save(&db, &[mine], &laptop).unwrap();

        let absorbed = absorb_conflict_copies(&db, &typ, &mut laptop, 3000);
        assert_eq!(absorbed.merged, vec![copy.clone()]);
        assert!(!copy.exists());
        assert_eq!(absorbed.changes, vec![theirs]);
        assert_eq!(laptop.text(), "ZabcdefXY");                       // the open Doc has it
        assert_eq!(storage::load(&db, DeviceId(1)).unwrap().1.text(), "ZabcdefXY"); // so does the file
    }

    #[test]
    fn a_text_file_copy_is_read_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let (db, typ, mut laptop, _) = work(dir.path());
        // One copy the app wrote (nothing to do), one edited in another editor.
        let written = dir.path().join("work.sync-conflict-20261105-082041-JGMDN7C.typ");
        std::fs::copy(&typ, &written).unwrap();
        let edited = dir.path().join("work.sync-conflict-20261105-082310-US2Y42W.typ");
        std::fs::write(&edited, "abcdef and more").unwrap();

        let absorbed = absorb_conflict_copies(&db, &typ, &mut laptop, 3000);
        assert_eq!(absorbed.read.len(), 2);
        assert!(!written.exists() && !edited.exists());
        assert_eq!(laptop.text(), "abcdef and more");
        assert!(!absorbed.changes.is_empty());
    }

    #[test]
    fn a_copy_that_cant_be_absorbed_is_kept_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let (db, typ, mut laptop, _) = work(dir.path());
        // Some other database that happens to have a conflict copy's name (like the spike's).
        let foreign = dir.path().join("work.sync-conflict-20261105-082458-JGMDN7C.db");
        {
            let conn = rusqlite::Connection::open(&foreign).unwrap();
            conn.execute_batch("CREATE TABLE rows (x TEXT); INSERT INTO rows VALUES ('mine');").unwrap();
        }
        let before = std::fs::read(&foreign).unwrap();
        // And a History from a Device that wrongly shares the laptop's Device ID.
        let clash = dir.path().join("work.sync-conflict-20261105-082020-JGMDN7C.db");
        let mut twin = Doc::new(DeviceId(1));
        let theirs = twin.insert_local(0, "different", 1000);
        storage::save(&clash, &[theirs], &twin).unwrap();

        let absorbed = absorb_conflict_copies(&db, &typ, &mut laptop, 3000);
        assert!(absorbed.merged.is_empty());
        assert_eq!(absorbed.kept.len(), 2);
        assert!(absorbed.kept.iter().any(|(p, why)| *p == foreign && why.contains("not a History")));
        assert!(absorbed.kept.iter().any(|(p, why)| *p == clash && why.contains("same ID")));
        assert_eq!(std::fs::read(&foreign).unwrap(), before, "left exactly as it was");
        assert!(clash.exists());
        assert_eq!(laptop.text(), "abcdef");
    }
}
