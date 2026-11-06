//! A Vault: the folder marked by `vault.db`, the Works in it, and the ones open in the app.
//!
//! Listing follows one rule: hide a file only when it's certain not to be a Work. Conflict
//! copies are never listed; they're absorbed as soon as they're seen, for every Work, open or
//! not, and one that can't be absorbed is a Sync problem.

use serde::Serialize;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use wrtier::conflicts::{self, absorb_conflict_copies, conflict_copies, Absorbed};
use wrtier::doc::Doc;
use wrtier::ids::DeviceId;
use wrtier::storage::{self, Probe};

/// The file that makes a folder a Vault.
pub const VAULT_FILE: &str = "vault.db";
/// Folders at the top of the Vault that hold something other than Works.
const NOT_WORKS: &[&str] = &["styles", "themes", "fonts"];

/// A Work open in the app: its History, held in memory, and where its files are.
pub struct OpenWork {
    pub db: PathBuf,
    pub typ: PathBuf,
    pub doc: Doc,
}

pub struct Vault {
    pub root: PathBuf,
    device: DeviceId,
    /// `vault.db`: the Vault's Devices and Vault settings.
    vault_doc: Doc,
    /// Open Works by ID: the path from the Vault's folder without the extension ("Short stories/Rain").
    pub works: HashMap<String, OpenWork>,
}

/// A file the app couldn't take in: kept untouched and reported until it's resolved.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SyncProblem {
    pub path: String,
    pub reason: String,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EntryKind {
    /// A History: opens as a Work.
    Work,
    /// A Text file without a History: opening it makes one.
    NewWork,
    /// A Text file the History of `of` wrote, left behind when that Work was renamed.
    Leftover { of: String },
    /// A History from a newer version of the app: it can't be opened.
    Newer { version: i64 },
    /// Couldn't be read right now: it may be a Work.
    Unreadable { reason: String },
}

#[derive(Serialize, Debug)]
pub struct Entry {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: EntryKind,
}

#[derive(Serialize, Debug)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub problems: Vec<SyncProblem>,
}

#[derive(Serialize, Debug)]
pub struct OpenedWork {
    pub id: String,
    pub name: String,
    pub text: String,
    pub problems: Vec<SyncProblem>,
}

pub enum Opened {
    /// The folder has no `vault.db`: ask before making it a Vault.
    NotAVault,
    Vault(Vault),
}

/// The files found in a Vault.
#[derive(Default)]
struct Files {
    dbs: Vec<PathBuf>,
    typs: Vec<PathBuf>,
    copies: Vec<PathBuf>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// What a caught panic said.
pub fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic.downcast_ref::<&str>().map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown error".to_string())
}

/// Records this Device's current name in a Work it just saved Changes to, if the Work has no
/// name for it yet or an older one. That keeps the `.db` readable on its own.
fn record_name(db: &Path, doc: &mut Doc, name: Option<&str>, now_ms: u64) -> Result<(), String> {
    let Some(name) = name else { return Ok(()) };
    if doc.device_name(doc.device()) == Some(name) {
        return Ok(());
    }
    let change = doc.set_device_name_local(name, now_ms);
    storage::save(db, &[change], doc).map_err(err)
}

impl Vault {
    /// Opens the Vault in `root`. Without a `vault.db` it's not a Vault yet: then it's made one
    /// only if `create` (the Author said yes).
    pub fn open(root: &Path, device: DeviceId, create: bool) -> Result<Opened, String> {
        if !root.is_dir() {
            return Err(format!("{} isn't a folder", root.display()));
        }
        let file = root.join(VAULT_FILE);
        // Only a Conflict copy of `vault.db` arrived so far: it holds the whole file.
        if !file.exists() {
            if let Some(copy) = conflict_copies(&file).into_iter().next() {
                std::fs::rename(&copy, &file).map_err(err)?;
            }
        }
        if !file.exists() {
            if !create {
                return Ok(Opened::NotAVault);
            }
            storage::save(&file, &[], &Doc::new(device)).map_err(err)?;
        }
        match storage::probe(&file) {
            Probe::History => {}
            Probe::NewerHistory(_) => return Err("This Vault was set up by a newer version of the app.".into()),
            Probe::NotHistory => return Err(format!("The {VAULT_FILE} in this folder belongs to another program.")),
            Probe::Unreadable(e) => return Err(format!("{VAULT_FILE} can't be read right now: {e}")),
        }
        let (_, vault_doc) = storage::load(&file, device).map_err(err)?;
        let mut vault = Vault { root: root.to_path_buf(), device, vault_doc, works: HashMap::new() };
        vault.absorb_vault_copies(&mut Vec::new()); // any left are reported when the Works are listed
        Ok(Opened::Vault(vault))
    }

    pub fn device(&self) -> DeviceId {
        self.device
    }

    /// This Device's name, as the Vault knows it. None until the Author has given one.
    pub fn device_name(&self) -> Option<String> {
        self.vault_doc.device_name(self.device).map(str::to_string)
    }

    pub fn set_device_name(&mut self, name: &str, now_ms: u64) -> Result<(), String> {
        let change = self.vault_doc.set_device_name_local(name, now_ms);
        storage::save(&self.root.join(VAULT_FILE), &[change], &self.vault_doc).map_err(err)
    }

    /// Merges Conflict copies of `vault.db` into it.
    fn absorb_vault_copies(&mut self, problems: &mut Vec<SyncProblem>) {
        let file = self.root.join(VAULT_FILE);
        let mut merged = false;
        for copy in conflict_copies(&file) {
            match storage::merge(&file, &copy) {
                Ok(()) => {
                    merged = true;
                    if let Err(e) = std::fs::remove_file(&copy) {
                        problems.push(self.problem(&copy, format!("merged, but couldn't delete it: {e}")));
                    }
                }
                Err(e) => problems.push(self.problem(&copy, e.to_string())),
            }
        }
        if merged {
            match storage::load(&file, self.device) {
                Ok((_, doc)) => self.vault_doc = doc,
                Err(e) => problems.push(self.problem(&file, e.to_string())),
            }
        }
    }

    /// A path inside the Vault, as shown to the Author: relative, with `/` between folders.
    fn relative(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
    }

    /// A Work's ID: its path without the extension.
    fn id_of(&self, path: &Path) -> String {
        let relative = self.relative(path);
        match relative.rsplit_once('.') {
            Some((id, _)) => id.to_string(),
            None => relative,
        }
    }

    fn problem(&self, path: &Path, reason: String) -> SyncProblem {
        SyncProblem { path: self.relative(path), reason }
    }

    /// A Work's `.db` and `.typ` from its ID. The ID comes from the interface, so it must stay
    /// inside the Vault.
    fn paths(&self, id: &str) -> Result<(PathBuf, PathBuf), String> {
        let parts: Vec<&str> = id.split('/').collect();
        let bad = |p: &&str| p.is_empty() || *p == "." || *p == ".." || p.contains(['\\', ':']);
        if parts.iter().any(bad) {
            return Err(format!("not a Work in this Vault: {id:?}"));
        }
        let (name, folders) = parts.split_last().expect("split gives at least one part");
        let dir = folders.iter().fold(self.root.clone(), |dir, part| dir.join(part));
        // Not `with_extension`: a name like "Draft 1.5" has a dot of its own.
        Ok((dir.join(format!("{name}.db")), dir.join(format!("{name}.typ"))))
    }

    fn walk(&self) -> Files {
        fn walk_dir(dir: &Path, top: bool, files: &mut Files) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let (path, name) = (entry.path(), entry.file_name().to_string_lossy().into_owned());
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_dir() {
                    // Syncthing's own folders (.stfolder, .stversions with old copies of every
                    // file), a Work's images, and the Style library, themes and fonts.
                    let skip = name.starts_with('.') || name.ends_with(".images") || (top && NOT_WORKS.contains(&name.as_str()));
                    if !skip {
                        walk_dir(&path, false, files);
                    }
                } else if kind.is_file() {
                    if conflicts::main_of(&path).is_some() {
                        files.copies.push(path);
                    } else if name.ends_with(".db") && !(top && name == VAULT_FILE) {
                        files.dbs.push(path);
                    } else if name.ends_with(".typ") {
                        files.typs.push(path);
                    }
                }
            }
        }
        let mut files = Files::default();
        walk_dir(&self.root, true, &mut files);
        files.copies.sort();
        files
    }

    /// Every Work in the Vault, after absorbing any Conflict copies.
    pub fn list(&mut self, now_ms: u64) -> Listing {
        let mut problems = Vec::new();
        let mut files = self.walk();

        // A copy whose main file is missing holds the whole file: it becomes the main file.
        let mut with_copies: HashSet<PathBuf> = HashSet::new();
        for copy in std::mem::take(&mut files.copies) {
            let main = conflicts::main_of(&copy).expect("only Conflict copies were collected");
            if main.exists() {
                with_copies.insert(main);
                continue;
            }
            match std::fs::rename(&copy, &main) {
                Ok(()) if main.extension().is_some_and(|e| e == "db") => files.dbs.push(main),
                Ok(()) if main.extension().is_some_and(|e| e == "typ") => files.typs.push(main),
                Ok(()) => {}
                Err(e) => problems.push(self.problem(&copy, format!("couldn't make it the main file: {e}"))),
            }
        }
        self.absorb_vault_copies(&mut problems);

        let mut entries = Vec::new();
        let mut histories = Vec::new();
        for db in &files.dbs {
            let (id, name) = (self.id_of(db), name_of(db));
            match storage::probe(db) {
                Probe::History => {
                    let typ = db.with_extension("typ");
                    if with_copies.contains(db) || with_copies.contains(&typ) {
                        self.absorb(db, &typ, now_ms, &mut problems);
                    }
                    histories.push(db.clone());
                    entries.push(Entry { id, name, kind: EntryKind::Work });
                }
                Probe::NewerHistory(version) => {
                    problems.push(self.problem(db, "made by a newer version of the app".into()));
                    entries.push(Entry { id, name, kind: EntryKind::Newer { version } });
                }
                Probe::NotHistory => {} // some other program's database
                Probe::Unreadable(reason) => {
                    problems.push(self.problem(db, format!("can't be read right now: {reason}")));
                    entries.push(Entry { id, name, kind: EntryKind::Unreadable { reason } });
                }
            }
        }

        let dbs: HashSet<&PathBuf> = files.dbs.iter().collect();
        for typ in &files.typs {
            if dbs.contains(&typ.with_extension("db")) {
                continue; // a Work's own Text file
            }
            let (id, name) = (self.id_of(typ), name_of(typ));
            let kind = match std::fs::read_to_string(typ) {
                Ok(text) => match histories.iter().find(|db| storage::wrote_text(db, &text).unwrap_or(false)) {
                    Some(db) => EntryKind::Leftover { of: self.id_of(db) },
                    None => EntryKind::NewWork,
                },
                Err(e) => {
                    problems.push(self.problem(typ, format!("can't be read right now: {e}")));
                    EntryKind::Unreadable { reason: e.to_string() }
                }
            };
            entries.push(Entry { id, name, kind });
        }
        entries.sort_by_key(|e| e.id.to_lowercase());
        Listing { entries, problems }
    }

    /// Absorbs a Work's Conflict copies: into its open History if it's open, otherwise into
    /// one loaded just for this.
    fn absorb(&mut self, db: &Path, typ: &Path, now_ms: u64, problems: &mut Vec<SyncProblem>) {
        let (id, name, me) = (self.id_of(db), self.device_name(), self.device);
        let report = |vault: &Vault, absorbed: Absorbed, problems: &mut Vec<SyncProblem>| {
            for (path, reason) in absorbed.kept {
                problems.push(vault.problem(&path, reason));
            }
        };
        if let Some(work) = self.works.get_mut(&id) {
            let result = catch_unwind(AssertUnwindSafe(|| -> Result<Absorbed, String> {
                let absorbed = absorb_conflict_copies(db, typ, &mut work.doc, now_ms);
                if absorbed.changes.iter().any(|c| c.id.device == me) {
                    record_name(db, &mut work.doc, name.as_deref(), now_ms)?;
                }
                storage::write_text_file(db, typ, &work.doc, now_ms).map_err(err)?;
                Ok(absorbed)
            }));
            match result {
                Ok(Ok(absorbed)) => report(self, absorbed, problems),
                Ok(Err(e)) => problems.push(self.problem(db, e)),
                Err(panic) => {
                    // Its state in memory may be half-updated: throw it away and reopen it from disk.
                    let what = panic_message(panic.as_ref());
                    self.works.remove(&id);
                    let reason = match self.open_work(&id, now_ms) {
                        Ok(_) => format!("something went wrong taking in another Device's copy ({what}); the Work was reopened from disk"),
                        Err(e) => format!("something went wrong taking in another Device's copy ({what}), and the Work couldn't be reopened: {e}"),
                    };
                    problems.push(self.problem(db, reason));
                }
            }
        } else {
            let result = catch_unwind(AssertUnwindSafe(|| -> Result<Absorbed, String> {
                let (_, mut doc) = storage::load(db, me).map_err(err)?;
                let absorbed = absorb_conflict_copies(db, typ, &mut doc, now_ms);
                if absorbed.changes.iter().any(|c| c.id.device == me) {
                    record_name(db, &mut doc, name.as_deref(), now_ms)?;
                }
                Ok(absorbed)
            }));
            match result {
                Ok(Ok(absorbed)) => report(self, absorbed, problems),
                Ok(Err(e)) => problems.push(self.problem(db, e)),
                Err(panic) => problems.push(self.problem(db, format!("something went wrong taking in another Device's copy: {}", panic_message(panic.as_ref())))),
            }
        }
    }

    /// Opens a Work: a History, or a Text file without one (which gets a History now).
    pub fn open_work(&mut self, id: &str, now_ms: u64) -> Result<OpenedWork, String> {
        let name = id.rsplit('/').next().unwrap_or(id).to_string();
        if let Some(work) = self.works.get(id) {
            return Ok(OpenedWork { id: id.to_string(), name, text: work.doc.text(), problems: Vec::new() });
        }
        let (db, typ) = self.paths(id)?;
        let (me, device_name) = (self.device, self.device_name());
        let mut problems = Vec::new();
        let result = catch_unwind(AssertUnwindSafe(|| -> Result<Doc, String> {
            if db.exists() {
                match storage::probe(&db) {
                    Probe::History => {}
                    Probe::NewerHistory(_) => return Err(format!("{name} was made by a newer version of the app.")),
                    Probe::NotHistory => return Err(format!("{name}.db belongs to another program.")),
                    Probe::Unreadable(e) => return Err(format!("{name} can't be read right now: {e}")),
                }
                let (_, mut doc) = storage::load(&db, me).map_err(err)?;
                let mut mine = false; // did this Device just save Changes to it?
                if !conflict_copies(&db).is_empty() || !conflict_copies(&typ).is_empty() {
                    let absorbed = absorb_conflict_copies(&db, &typ, &mut doc, now_ms);
                    mine = absorbed.changes.iter().any(|c| c.id.device == me);
                    problems.extend(absorbed.kept.into_iter().map(|(path, reason)| self.problem(&path, reason)));
                }
                // An External edit made while the app was closed becomes Changes now.
                mine |= !storage::sync_text_file(&db, &typ, &mut doc, now_ms).map_err(err)?.is_empty();
                if mine {
                    record_name(&db, &mut doc, device_name.as_deref(), now_ms)?;
                }
                Ok(doc)
            } else if typ.exists() {
                // A Text file without a History: the whole file is one External edit, written
                // from an empty version (so it's not "unknown", and there's no Merge notice).
                let text = std::fs::read_to_string(&typ).map_err(err)?;
                let mut doc = Doc::new(me);
                let changes = doc.import_text(Some(&[]), &text, now_ms);
                storage::save(&db, &changes, &doc).map_err(err)?;
                storage::write_text_file(&db, &typ, &doc, now_ms).map_err(err)?;
                record_name(&db, &mut doc, device_name.as_deref(), now_ms)?;
                Ok(doc)
            } else {
                Err(format!("{name} isn't in the Vault anymore."))
            }
        }));
        let doc = match result {
            Ok(result) => result?,
            Err(panic) => return Err(format!("Something went wrong opening {name}: {}", panic_message(panic.as_ref()))),
        };
        let text = doc.text();
        self.works.insert(id.to_string(), OpenWork { db, typ, doc });
        Ok(OpenedWork { id: id.to_string(), name, text, problems })
    }

    pub fn close_work(&mut self, id: &str) {
        self.works.remove(id);
    }

    /// Deletes a Text file left behind by a renamed Work, after checking that it still is one.
    pub fn delete_leftover(&mut self, id: &str) -> Result<(), String> {
        let (db, typ) = self.paths(id)?;
        if db.exists() {
            return Err(format!("{id} has a History: it's a Work, not a leftover."));
        }
        let text = std::fs::read_to_string(&typ).map_err(err)?;
        let wrote = self.walk().dbs.iter()
            .any(|db| storage::probe(db) == Probe::History && storage::wrote_text(db, &text).unwrap_or(false));
        if !wrote {
            return Err(format!("{id} was edited since its Work was renamed, so it's kept."));
        }
        std::fs::remove_file(&typ).map_err(err)
    }
}

/// A file's name without its extension.
fn name_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite_free::*;

    const ME: DeviceId = DeviceId(1);
    const OTHER: DeviceId = DeviceId(2);

    fn open(root: &Path) -> Vault {
        match Vault::open(root, ME, true).unwrap() {
            Opened::Vault(v) => v,
            Opened::NotAVault => panic!("asked to create it"),
        }
    }

    /// A Work `name` with `text`, written by `device`, with its Text file.
    fn work(root: &Path, name: &str, text: &str, device: DeviceId) -> (PathBuf, Doc) {
        let (db, typ) = (root.join(format!("{name}.db")), root.join(format!("{name}.typ")));
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let mut doc = Doc::new(device);
        storage::save(&db, &[doc.insert_local(0, text, 1000)], &doc).unwrap();
        storage::write_text_file(&db, &typ, &doc, 1000).unwrap();
        (db, doc)
    }

    fn kinds(listing: &Listing) -> Vec<(&str, &EntryKind)> {
        listing.entries.iter().map(|e| (e.id.as_str(), &e.kind)).collect()
    }

    #[test]
    fn a_folder_becomes_a_vault_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(Vault::open(dir.path(), ME, false).unwrap(), Opened::NotAVault));
        assert!(!dir.path().join(VAULT_FILE).exists());
        let mut vault = open(dir.path());
        assert_eq!(vault.device_name(), None);
        vault.set_device_name("Laptop", 2000).unwrap();
        // Another start of the app finds it.
        let Opened::Vault(again) = Vault::open(dir.path(), ME, false).unwrap() else { panic!("it's a Vault now") };
        assert_eq!(again.device_name().as_deref(), Some("Laptop"));
    }

    #[test]
    fn a_vault_file_from_another_program_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        create_foreign_db(&dir.path().join(VAULT_FILE));
        assert!(Vault::open(dir.path(), ME, true).is_err());
    }

    #[test]
    fn listing_shows_works_and_hides_only_what_surely_isnt_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        work(root, "The Long Night", "It was dark.", ME);
        work(root, "Short stories/Rain", "The rain had stopped.", ME);
        create_foreign_db(&root.join("addresses.db"));                           // hidden
        std::fs::write(root.join("Lone.typ"), "Written in vim.").unwrap();      // a new Work
        std::fs::create_dir_all(root.join(".stversions")).unwrap();             // Syncthing's old copies
        work(root, ".stversions/Rain~20261101", "old", ME);
        std::fs::create_dir_all(root.join("styles")).unwrap();
        std::fs::write(root.join("styles/paperback.typ"), "#set page(width: 5in)").unwrap();
        let newer = root.join("Future.db");
        storage::save(&newer, &[], &Doc::new(ME)).unwrap();
        set_user_version(&newer, storage::STORAGE_VERSION + 1);

        let listing = vault.list(2000);
        assert_eq!(kinds(&listing), vec![
            ("Future", &EntryKind::Newer { version: storage::STORAGE_VERSION + 1 }),
            ("Lone", &EntryKind::NewWork),
            ("Short stories/Rain", &EntryKind::Work),
            ("The Long Night", &EntryKind::Work),
        ]);
        assert_eq!(listing.problems.len(), 1, "the newer History is a Sync problem: {:?}", listing.problems);
    }

    #[test]
    fn a_text_file_left_by_a_renamed_work_is_recognised() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        let (db, _) = work(root, "Rain", "The rain had stopped.", ME);
        std::fs::rename(&db, root.join("Rain (draft 1).db")).unwrap(); // renamed in Explorer
        let listing = vault.list(2000);
        assert_eq!(kinds(&listing), vec![
            ("Rain", &EntryKind::Leftover { of: "Rain (draft 1)".into() }),
            ("Rain (draft 1)", &EntryKind::Work),
        ]);
        vault.delete_leftover("Rain").unwrap();
        assert!(!root.join("Rain.typ").exists());
    }

    #[test]
    fn an_edited_leftover_is_a_new_work_and_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        let (db, _) = work(root, "Rain", "The rain had stopped.", ME);
        std::fs::rename(&db, root.join("Rain (draft 1).db")).unwrap();
        std::fs::write(root.join("Rain.typ"), "The rain had stopped. And then?").unwrap();
        assert_eq!(kinds(&vault.list(2000))[0], ("Rain", &EntryKind::NewWork));
        assert!(vault.delete_leftover("Rain").is_err());
        assert!(root.join("Rain.typ").exists());
    }

    #[test]
    fn conflict_copies_are_absorbed_for_works_that_arent_open() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        let (db, laptop) = work(root, "Rain", "abc", ME);
        // The phone wrote its own Change at the same time: Syncthing kept both.
        let mut phone = Doc::new(OTHER);
        let (base, _) = storage::load(&db, OTHER).unwrap();
        phone.apply_all(&base);
        let theirs = phone.insert_local(3, "XY", 2000);
        let copy = root.join("Rain.sync-conflict-20261105-082020-JGMDN7C.db");
        let mut all = base.clone();
        all.push(theirs);
        storage::save(&copy, &all, &phone).unwrap();
        drop(laptop);

        let listing = vault.list(3000);
        assert_eq!(kinds(&listing), vec![("Rain", &EntryKind::Work)], "the copy isn't listed");
        assert!(!copy.exists());
        assert_eq!(storage::load(&db, ME).unwrap().1.text(), "abcXY");
        assert!(listing.problems.is_empty());
    }

    #[test]
    fn a_copy_whose_main_file_is_missing_becomes_the_main_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        let (db, _) = work(root, "Rain", "abc", ME);
        std::fs::rename(&db, root.join("Rain.sync-conflict-20261105-082020-JGMDN7C.db")).unwrap();
        assert_eq!(kinds(&vault.list(2000)), vec![("Rain", &EntryKind::Work)]);
        assert!(db.exists());
    }

    #[test]
    fn a_copy_that_cant_be_absorbed_is_a_sync_problem() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        work(root, "Rain", "abc", ME);
        let copy = root.join("Rain.sync-conflict-20261105-082020-JGMDN7C.db");
        create_foreign_db(&copy);
        let listing = vault.list(2000);
        assert_eq!(listing.problems.len(), 1);
        assert_eq!(listing.problems[0].path, "Rain.sync-conflict-20261105-082020-JGMDN7C.db");
        assert!(copy.exists(), "kept untouched");
    }

    #[test]
    fn opening_a_work_only_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.set_device_name("Laptop", 1500).unwrap();
        let (db, _) = work(root, "Rain", "The rain had stopped.", OTHER);
        let before = storage::read_all(&db).unwrap().len();
        let opened = vault.open_work("Rain", 2000).unwrap();
        assert_eq!(opened.text, "The rain had stopped.");
        assert_eq!(storage::read_all(&db).unwrap().len(), before, "reading never writes");
    }

    #[test]
    fn an_external_edit_found_on_opening_records_this_devices_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.set_device_name("Laptop", 1500).unwrap();
        let (db, _) = work(root, "Rain", "The rain had stopped.", OTHER);
        std::fs::write(root.join("Rain.typ"), "The rain had eased.").unwrap(); // edited in vim
        assert_eq!(vault.open_work("Rain", 2000).unwrap().text, "The rain had eased.");
        let (_, doc) = storage::load(&db, ME).unwrap();
        assert_eq!(doc.device_name(ME), Some("Laptop"));
    }

    #[test]
    fn a_text_file_without_a_history_gets_one_when_opened() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.set_device_name("Laptop", 1500).unwrap();
        std::fs::write(root.join("Lone.typ"), "Written in vim.\r\n").unwrap();
        let opened = vault.open_work("Lone", 2000).unwrap();
        assert_eq!(opened.text, "Written in vim.\n");
        let (_, doc) = storage::load(&root.join("Lone.db"), ME).unwrap();
        assert_eq!(doc.text(), "Written in vim.\n");
        assert_eq!(doc.device_name(ME), Some("Laptop"));
        assert!(doc.merge_notices().is_empty(), "no 'unknown version' notice for a brand-new Work");
        assert_eq!(kinds(&vault.list(3000)), vec![("Lone", &EntryKind::Work)]);
    }

    #[test]
    fn names_with_dots_and_folders_find_their_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        work(root, "Short stories/Draft 1.5", "dots", ME);
        assert_eq!(kinds(&vault.list(2000)), vec![("Short stories/Draft 1.5", &EntryKind::Work)]);
        assert_eq!(vault.open_work("Short stories/Draft 1.5", 2000).unwrap().text, "dots");
    }

    #[test]
    fn an_id_from_the_interface_cant_leave_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        for id in ["../outside", "a/../../b", "", "/abs", "C:/x", "a\\b", "a//b"] {
            assert!(vault.open_work(id, 2000).is_err(), "{id:?}");
        }
    }

    /// Small helpers that write SQLite files directly, as another program would.
    mod rusqlite_free {
        use std::path::Path;

        pub fn create_foreign_db(path: &Path) {
            sqlite(path, "CREATE TABLE rows (x TEXT); INSERT INTO rows VALUES ('mine');");
        }

        pub fn set_user_version(path: &Path, version: i64) {
            sqlite(path, &format!("PRAGMA user_version = {version};"));
        }

        fn sqlite(path: &Path, sql: &str) {
            rusqlite::Connection::open(path).unwrap().execute_batch(sql).unwrap();
        }
    }
}
