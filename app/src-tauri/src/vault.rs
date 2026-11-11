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
use crate::library;
use crate::preamble::{self, Preamble, Shelf, Version};
use crate::typeset::Block;
use crate::recovery::{self, Recovery};
use crate::work::{undoable, Op, OpenWork, Whereabouts, WorkState};
use wrtier::change::Change;
use wrtier::ids::ChangeId;

/// The file that makes a folder a Vault.
pub const VAULT_FILE: &str = "vault.db";
/// Folders at the top of the Vault that hold something other than Works.
const NOT_WORKS: &[&str] = &["styles", "themes", "fonts"];

pub struct Vault {
    pub root: PathBuf,
    device: DeviceId,
    /// `vault.db`: the Vault's Devices, Reviewers and Vault settings.
    vault_doc: Doc,
    /// Open Works by ID: the path from the Vault's folder without the extension ("Short stories/Rain").
    pub works: HashMap<String, OpenWork>,
    /// This Device's recovery files (outside the Vault).
    recovery_dir: PathBuf,
    /// Works recovered since the Vault opened: the Changes recovered (undo can take them back
    /// when the Work is opened) and the line that tells the Author.
    recovered: HashMap<String, (Vec<ChangeId>, String)>,
    /// Why Syncthing couldn't be told to ignore SQLite's temporary files, if it couldn't.
    ignore_problem: Option<String>,
    /// Open Works that were renamed (here or elsewhere): old ID → new ID, so edits still on
    /// their way under the old ID find them.
    aliases: HashMap<String, String>,
    /// `vault.db` as this Vault last wrote or read it: if it changes, another Device's settings arrived.
    vault_stamp: Option<(std::time::SystemTime, u64)>,
    /// The Style library's files as last seen: if they change, the interface hears of it.
    library_stamp: Vec<(PathBuf, u64, Option<std::time::SystemTime>)>,
    /// The theme files as last seen: if they change, the interface hears of it.
    themes_stamp: Vec<(PathBuf, u64, Option<std::time::SystemTime>)>,
    /// Counts the library's changes: a Work that recorded what it uses as of an older count
    /// records the new versions before its next edit.
    library_generation: u64,
    /// Why the default Styles couldn't be written, if they couldn't.
    defaults_problem: Option<String>,
    /// Each `.db`'s Work ID, read once: how Inclusions find the Works they include.
    identities: HashMap<PathBuf, Option<String>>,
    /// Included Works read from disk (see `inclusions::DocCache`).
    included_docs: inclusions::DocCache,
}

/// What an Export needs (see `Vault::export_input`).
pub struct ExportInput {
    /// Its text, with its Inclusions written out as Typst.
    pub text: String,
    /// Its text for a `.docx`, with its Inclusions written out as plain text under headings.
    pub docx_text: String,
    /// Inclusions that couldn't be shown, and why: an Export says so.
    pub missing: Vec<String>,
    pub header: preamble::Header,
    /// Its title, author and language, for a `.docx`'s running header and properties.
    pub preamble: Preamble,
    /// The time of the Work's newest Change: the PDF's date, the same on every Device.
    pub date_ms: u64,
    /// The Work's own ID: the PDF's document ID comes from it.
    pub ident: String,
}

/// A Milestone, for the Page preview's picker.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneInfo {
    pub id: String,
    pub name: String,
    /// When it was named (ms since 1970).
    pub time_ms: u64,
}

/// A Work's Preamble as the dialog shows it.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PreambleView {
    pub preamble: Preamble,
    /// The other Device's version of the raw Typst, if both were edited without seeing each other.
    pub raw_other: Option<String>,
}

/// The record in `vault.db` saying which of the app's default Styles were written (ADR 0012):
/// one field per file, holding the version written.
const DEFAULTS_RECORD: &str = "defaults";

mod history;
mod inclusions;
mod mods;
mod moves;
mod reviewers;
mod settings;
pub use inclusions::{ChapterChoice, InclusionView, Target};
pub use history::{ComparisonView, HistoryView, RestorePreview, ReturnPreview, RevertPreview, SnippetsView};
pub use mods::Mods;
pub use moves::{TrashedWork, TRASH};
pub use reviewers::{Reviewer, COLOURS};
pub use settings::Settings;

fn file_stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Syncthing must never sync SQLite's temporary `-journal` files (Phase 0c). Its ignore
/// patterns don't sync themselves, so each Device adds the pattern when it opens the Vault, in
/// the `.stignore` of the Syncthing folder the Vault is in (the Vault itself, or a folder above it).
fn ignore_journals(root: &Path) -> Result<(), String> {
    let Some(folder) = root.ancestors().find(|dir| dir.join(".stfolder").exists()) else {
        return Ok(()); // not in a Syncthing folder (yet)
    };
    add_journal_ignore(folder)
}

/// Adds `*-journal` to the `.stignore` in `folder` (a Syncthing folder, or one about to be),
/// unless it's there already.
pub fn add_journal_ignore(folder: &Path) -> Result<(), String> {
    let path = folder.join(".stignore");
    let current = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    if current.lines().any(|line| line.trim() == "*-journal") {
        return Ok(());
    }
    let mut text = current;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("// Added by wrtier: SQLite's temporary files must never sync.\n*-journal\n");
    std::fs::write(&path, text).map_err(err)
}

/// A file the app couldn't take in: kept untouched and reported until it's resolved.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SyncProblem {
    pub path: String,
    pub reason: String,
    /// Made by a newer version of the app: updating this Device is the fix.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub newer: bool,
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
    /// A Text file that can't get a History, e.g. because another program's `.db` has its name.
    Blocked { reason: String },
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
    /// Every folder Works can be in ("Short stories", "Short stories/Old"), empty ones too.
    pub folders: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct OpenedWork {
    pub id: String,
    pub name: String,
    pub text: String,
    pub problems: Vec<SyncProblem>,
    /// What was recovered from before the app closed unexpectedly, if anything.
    pub notice: Option<String>,
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
    folders: Vec<PathBuf>,
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
    /// `recovery_dir`: where this Device keeps recovery files.
    pub fn open(root: &Path, device: DeviceId, create: bool, recovery_dir: &Path) -> Result<Opened, String> {
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
        let mut vault = Vault {
            root: root.to_path_buf(), device, vault_doc,
            works: HashMap::new(),
            recovery_dir: recovery_dir.to_path_buf(),
            recovered: HashMap::new(),
            ignore_problem: ignore_journals(root).err(),
            aliases: HashMap::new(),
            vault_stamp: file_stamp(&file),
            library_stamp: Vec::new(),
            themes_stamp: crate::themes::stamp(root),
            library_generation: 0,
            defaults_problem: None,
            identities: HashMap::new(),
            included_docs: HashMap::new(),
        };
        vault.absorb_vault_copies(&mut Vec::new()); // any left are reported when the Works are listed
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        vault.defaults_problem = vault.write_defaults(now_ms).err();
        vault.library_stamp = library::stamp(root);
        Ok(Opened::Vault(vault))
    }

    /// Writes the app's default Styles, Looks and Story elements the Vault has never had, and
    /// records them in `vault.db`, so one the Author deletes never comes back (ADR 0012).
    fn write_defaults(&mut self, now_ms: u64) -> Result<(), String> {
        let written = library::write_defaults(&self.root, |path| self.vault_doc.field(DEFAULTS_RECORD, path).is_some())?;
        let changes: Vec<Change> = written.iter()
            .map(|path| self.vault_doc.set_field_local(DEFAULTS_RECORD, path, library::DEFAULTS_VERSION, now_ms))
            .collect();
        if changes.is_empty() { Ok(()) } else { self.save_vault(&changes) }
    }

    /// The Style library, and its Conflict copies waiting for the Author.
    pub fn library(&self) -> library::Library {
        library::list(&self.root)
    }

    /// Has anything in the Style library changed since this was last asked (here or elsewhere)?
    /// If so, open Works get their Style update notices, and record the new versions before
    /// their next edit.
    pub fn library_changed(&mut self) -> bool {
        let now = library::stamp(&self.root);
        let changed = now != self.library_stamp;
        self.library_stamp = now;
        if changed {
            self.library_generation += 1;
            self.refresh_style_notices();
        }
        changed
    }

    /// Has a theme in `themes/` been added, changed or removed since this was last asked (here,
    /// in another editor, or from another Device)?
    pub fn themes_changed(&mut self) -> bool {
        let now = crate::themes::stamp(&self.root);
        let changed = now != self.themes_stamp;
        self.themes_stamp = now;
        changed
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
        self.save_vault(&[change])
    }

    /// Saves Changes to `vault.db` (and notes that this was our own write).
    fn save_vault(&mut self, changes: &[Change]) -> Result<(), String> {
        let file = self.root.join(VAULT_FILE);
        storage::save(&file, changes, &self.vault_doc).map_err(err)?;
        self.vault_stamp = file_stamp(&file);
        Ok(())
    }

    /// Merges Conflict copies of `vault.db` into it, and reads it again if it changed (another
    /// Device's settings or name arrived).
    fn absorb_vault_copies(&mut self, problems: &mut Vec<SyncProblem>) {
        let file = self.root.join(VAULT_FILE);
        let mut merged = file_stamp(&file) != self.vault_stamp;
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
                Ok((_, doc)) => {
                    self.vault_doc = doc;
                    self.vault_stamp = file_stamp(&file);
                }
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
        SyncProblem { path: self.relative(path), reason, newer: false }
    }

    /// A Work's `.db` and `.typ` from its ID. The ID comes from the interface, so it must stay
    /// inside the Vault.
    fn paths(&self, id: &str) -> Result<(PathBuf, PathBuf), String> {
        paths_under(&self.root, id)
    }

    fn walk(&self) -> Files {
        walk_works(&self.root)
    }

    /// Every Work in the Vault, after absorbing any Conflict copies.
    pub fn list(&mut self, now_ms: u64) -> Listing {
        let mut problems = Vec::new();
        if let Some(reason) = &self.ignore_problem {
            problems.push(SyncProblem { path: ".stignore".into(), reason: format!("couldn't add *-journal to it: {reason}"), newer: false });
        }
        if let Some(reason) = &self.defaults_problem {
            problems.push(SyncProblem { path: library::FOLDER.into(), reason: format!("couldn't write the default Styles: {reason}"), newer: false });
        }
        for conflict in library::list(&self.root).conflicts {
            problems.push(SyncProblem { path: conflict.copy, reason: format!("another version of {}: pick one in the Style library", conflict.path), newer: false });
        }
        // Writing kept from before the app closed unexpectedly goes back first.
        for id in recovery::kept_for(&self.recovery_dir, &self.root) {
            if let Err(problem) = self.recover(&id, now_ms) {
                problems.push(problem);
            }
        }
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
        let mut foreign: HashSet<PathBuf> = HashSet::new(); // other programs' databases
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
                    problems.push(SyncProblem { newer: true, ..self.problem(db, "made by a newer version of the app".into()) });
                    entries.push(Entry { id, name, kind: EntryKind::Newer { version } });
                }
                Probe::NotHistory => {
                    foreign.insert(db.clone()); // some other program's: not listed
                }
                Probe::Unreadable(reason) => {
                    problems.push(self.problem(db, format!("can't be read right now: {reason}")));
                    entries.push(Entry { id, name, kind: EntryKind::Unreadable { reason } });
                }
            }
        }

        problems.extend(self.duplicate_problems(&histories));

        let dbs: HashSet<&PathBuf> = files.dbs.iter().collect();
        for typ in &files.typs {
            let (id, name) = (self.id_of(typ), name_of(typ));
            let db = typ.with_extension("db");
            if foreign.contains(&db) {
                // Its History would need that file's name: show it, with why it can't open.
                let reason = format!("{name}.db next to it belongs to another program, so it can't get a History. Rename one of them.");
                entries.push(Entry { id, name, kind: EntryKind::Blocked { reason } });
                continue;
            }
            if dbs.contains(&db) {
                continue; // a Work's own Text file
            }
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
        let mut folders: Vec<String> = files.folders.iter().map(|dir| self.relative(dir)).collect();
        folders.sort_by_key(|f| f.to_lowercase());
        Listing { entries, problems, folders }
    }

    /// Absorbs the Conflict copies of a Work that isn't open, into a History loaded just for
    /// this. (An open Work's copies wait until it's closed or opened again: taking them in
    /// changes the text under the editor, which needs the file watcher's patches.)
    fn absorb(&mut self, db: &Path, typ: &Path, now_ms: u64, problems: &mut Vec<SyncProblem>) {
        let (id, name, me) = (self.id_of(db), self.device_name(), self.device);
        if self.works.contains_key(&id) {
            return;
        }
        let result = catch_unwind(AssertUnwindSafe(|| -> Result<Absorbed, String> {
            let (_, mut doc) = storage::load(db, me).map_err(err)?;
            let absorbed = absorb_conflict_copies(db, typ, &mut doc, now_ms);
            if absorbed.changes.iter().any(|c| c.id.device == me) {
                record_name(db, &mut doc, name.as_deref(), now_ms)?;
            }
            Ok(absorbed)
        }));
        match result {
            Ok(Ok(absorbed)) => {
                for (path, reason) in absorbed.kept {
                    problems.push(self.problem(&path, reason));
                }
            }
            Ok(Err(e)) => problems.push(self.problem(db, e)),
            Err(panic) => problems.push(self.problem(db, format!("something went wrong taking in another Device's copy: {}", panic_message(panic.as_ref())))),
        }
    }

    /// Opens a Work: a History, or a Text file without one (which gets a History now).
    pub fn open_work(&mut self, id: &str, now_ms: u64) -> Result<OpenedWork, String> {
        let name = id.rsplit('/').next().unwrap_or(id).to_string();
        if let Some(work) = self.works.get(id) {
            return Ok(OpenedWork { id: id.to_string(), name, text: work.text(), problems: Vec::new(), notice: None });
        }
        let (db, typ) = self.paths(id)?;
        let (me, device_name) = (self.device, self.device_name());
        let mut problems = Vec::new();
        if let Err(problem) = self.recover(id, now_ms) {
            problems.push(problem);
        }
        let mut header_dropped = false;
        // The History, and the Changes this Device made in it (see `OpenWork::mine`).
        let result = catch_unwind(AssertUnwindSafe(|| -> Result<(Doc, Vec<Change>), String> {
            if db.exists() {
                match storage::probe(&db) {
                    Probe::History => {}
                    Probe::NewerHistory(_) => return Err(format!("{name} was made by a newer version of the app.")),
                    Probe::NotHistory => return Err(format!("{name}.db belongs to another program.")),
                    Probe::Unreadable(e) => return Err(format!("{name} can't be read right now: {e}")),
                }
                let (loaded, mut doc) = storage::load(&db, me).map_err(err)?;
                let mut mine: Vec<Change> = loaded.into_iter().filter(|c| c.id.device == me).collect();
                let saved_before = mine.len();
                if !conflict_copies(&db).is_empty() || !conflict_copies(&typ).is_empty() {
                    let absorbed = absorb_conflict_copies(&db, &typ, &mut doc, now_ms);
                    mine.extend(absorbed.changes.into_iter().filter(|c| c.id.device == me));
                    problems.extend(absorbed.kept.into_iter().map(|(path, reason)| self.problem(&path, reason)));
                }
                // An External edit made while the app was closed becomes Changes now. Edits
                // inside the header are dropped (it's written again from the History): said so,
                // but only for a file the app didn't write itself.
                let header = preamble::text_file_header(&doc, &Shelf::read(&self.root));
                if let Ok(contents) = std::fs::read_to_string(&typ) {
                    let in_file = wrtier::text_file::split_header(&wrtier::text_file::split_marker(&contents).0).0;
                    header_dropped = !storage::wrote_text(&db, &contents).unwrap_or(true) && in_file.is_some_and(|h| h != header);
                }
                // Its Inclusions are written out at the versions it recorded (ADR 0015).
                let expansions = inclusions::expansions(&self.shown_in_doc(&doc));
                mine.extend(storage::sync_text_file_with(&db, &typ, &mut doc, &header, &expansions, now_ms).map_err(err)?);
                if mine.len() > saved_before {
                    record_name(&db, &mut doc, device_name.as_deref(), now_ms)?;
                }
                Ok((doc, mine))
            } else if typ.exists() {
                // A Text file without a History: the whole file is one External edit, written
                // from an empty version (so it's not "unknown", and there's no Merge notice).
                // (Without any marker line: that names a version of another History.)
                // (A header from another Work's Text file isn't text either.)
                let (text, _) = wrtier::text_file::split_marker(&std::fs::read_to_string(&typ).map_err(err)?);
                let (_, text) = wrtier::text_file::split_header(&text);
                let mut doc = Doc::new(me);
                let changes = doc.import_text(Some(&[]), &text, now_ms);
                storage::save(&db, &changes, &doc).map_err(err)?;
                let header = preamble::text_file_header(&doc, &Shelf::read(&self.root));
                storage::write_text_file(&db, &typ, &doc, &header, now_ms).map_err(err)?;
                record_name(&db, &mut doc, device_name.as_deref(), now_ms)?;
                Ok((doc, changes))
            } else {
                Err(format!("{name} isn't in the Vault anymore."))
            }
        }));
        let (doc, mine) = match result {
            Ok(result) => result?,
            Err(panic) => return Err(format!(
                "{name} can't be opened: its History looks damaged ({}). Its files are left as they are.",
                panic_message(panic.as_ref()))),
        };
        let text = doc.text();
        let (sitting, mut notice) = match self.recovered.remove(id) {
            Some((changes, notice)) => (changes, Some(notice)),
            None => (Vec::new(), None),
        };
        if header_dropped {
            let line = format!("Edits to the Preamble at the top of {name}.typ were dropped: it's made from the Work's History. \
                Change it with Preamble… on the Work.");
            notice = Some(notice.map_or(line.clone(), |n| format!("{n} {line}")));
        }
        let recovery = Recovery::for_work(&self.recovery_dir, &self.root, id);
        let mut work = OpenWork::new(db, typ, doc, now_ms, recovery, sitting, mine);
        work.style_notice = preamble::update_notice(&work.doc, &Shelf::read(&self.root));
        work.vault_root = self.root.clone();
        self.works.insert(id.to_string(), work);
        self.refresh_text_file_inclusions(id);
        Ok(OpenedWork { id: id.to_string(), name, text, problems, notice })
    }

    /// Puts back what a Work's recovery file kept from before the app closed unexpectedly: the
    /// open burst becomes the Change it would have become (at its last keystroke), unsaved
    /// Changes are saved, and a cut waiting for its paste goes back where it was. Then the
    /// recovery file is removed. If it can't be put back it's kept, and tried again later.
    fn recover(&mut self, id: &str, now_ms: u64) -> Result<(), SyncProblem> {
        if self.works.contains_key(id) {
            return Ok(()); // open: its recovery file is the live one
        }
        let mut files = Recovery::for_work(&self.recovery_dir, &self.root, id);
        let problem = |reason: String| SyncProblem {
            path: id.to_string(),
            reason: format!("writing that wasn't saved before the app closed couldn't be put back ({reason}). It's kept, and tried again."),
            newer: false,
        };
        let kept = match files.read() {
            Ok(Some(kept)) => kept,
            Ok(None) => return Ok(()),
            Err(e) => return Err(problem(e)),
        };
        let (db, typ) = self.paths(id).map_err(|e| problem(e))?;
        if storage::probe(&db) != Probe::History {
            return Err(problem(format!("{id}.db isn't a History in the Vault right now")));
        }
        let (me, device_name, root) = (self.device, self.device_name(), self.root.clone());
        let result = catch_unwind(AssertUnwindSafe(|| -> Result<(Vec<ChangeId>, Vec<String>), String> {
            let (_, mut doc) = storage::load(&db, me).map_err(err)?;
            let mut recovered: Vec<ChangeId> = Vec::new();
            let mut lines: Vec<String> = Vec::new();
            // Changes that failed to save: merged in like a Conflict copy (any already saved are skipped).
            if files.db.exists() {
                storage::merge(&db, &files.db).map_err(err)?;
                let new: Vec<Change> = storage::read_all(&db).map_err(err)?.into_iter().filter(|c| !doc.has(c.id)).collect();
                doc.apply_all(&new);
                recovered.extend(new.iter().filter(|c| undoable(c)).map(|c| c.id));
                if !new.is_empty() {
                    lines.push(format!("Saved {} Change{} that couldn't be saved before the app closed.", new.len(), if new.len() == 1 { "" } else { "s" }));
                }
            }
            if let Some(burst) = &kept.burst {
                if burst.resume(&mut doc)? {
                    if kept.cut {
                        doc.cancel_burst();
                        lines.push("A cut that hadn't been pasted yet was put back where it was.".into());
                    } else if let Some(change) = doc.close_burst(burst.last_ms) {
                        storage::save(&db, std::slice::from_ref(&change), &doc).map_err(err)?;
                        recovered.push(change.id);
                        lines.push(if burst.deleting {
                            "Recovered a deletion made just before the app closed unexpectedly.".into()
                        } else {
                            format!("Recovered {} characters typed just before the app closed unexpectedly.", burst.draft.chars().count())
                        });
                    }
                }
            }
            if !recovered.is_empty() {
                record_name(&db, &mut doc, device_name.as_deref(), now_ms)?;
            }
            let header = preamble::text_file_header(&doc, &Shelf::read(&root));
            storage::sync_text_file(&db, &typ, &mut doc, &header, now_ms).map_err(err)?;
            Ok((recovered, lines))
        }));
        let (changes, lines) = match result {
            Ok(Ok(done)) => done,
            Ok(Err(e)) => return Err(problem(e)),
            Err(panic) => return Err(problem(panic_message(panic.as_ref()))),
        };
        files.clear().map_err(|e| problem(e))?;
        if !lines.is_empty() {
            self.recovered.insert(id.to_string(), (changes, lines.join(" ")));
        }
        Ok(())
    }

    /// Makes a new, empty Work and opens it. `id` is its name, or a path inside the Vault
    /// ("Short stories/Rain"), whose folders are made if they don't exist yet.
    pub fn create_work(&mut self, id: &str, now_ms: u64) -> Result<OpenedWork, String> {
        let id = self.free_name(id)?;
        let (db, typ) = self.paths(&id)?;
        if let Some(dir) = db.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        // The ID that tells this Work from every other wherever it's moved (`storage::identity`),
        // and when it was made.
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| format!("couldn't make an ID for the Work: {e}"))?;
        let work_id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let mut doc = Doc::new(self.device);
        let given = doc.set_field_local(storage::WORK_RECORD, storage::WORK_ID_FIELD, &work_id, now_ms);
        let created = doc.set_field_local(storage::WORK_RECORD, "created", &now_ms.to_string(), now_ms);
        let mut changes = vec![given, created];
        // A new Work starts with the first default Look the library still has; the Preamble
        // dialog changes it. (Its content is recorded with the first edit.)
        let library = library::list(&self.root);
        let look = ["Paperback", "Manuscript", "A4 draft"].into_iter()
            .find(|name| library.items.iter().any(|i| i.kind == library::Kind::Look && i.name == *name));
        if let Some(look) = look {
            let preamble = Preamble { looks: vec![look.to_string()], ..Preamble::default() };
            changes.extend(preamble.record(&mut doc, now_ms));
        }
        storage::save(&db, &changes, &doc).map_err(err)?;
        let header = preamble::text_file_header(&doc, &Shelf::read(&self.root));
        storage::write_text_file(&db, &typ, &doc, &header, now_ms).map_err(err)?;
        self.give_new_work_mods(&work_id, now_ms)?;
        self.open_work(&id, now_ms)
    }

    /// Checks a name the Author typed for a Work (see `check_name`), and that no Work or file
    /// has it yet. Returns it tidied (spaces around the slashes removed).
    fn free_name(&self, id: &str) -> Result<String, String> {
        let parts: Vec<&str> = id.split('/').map(str::trim).collect();
        for (i, part) in parts.iter().enumerate() {
            check_name(part, i == 0 && parts.len() > 1, i == 0 && parts.len() == 1)?;
        }
        let id = parts.join("/");
        let (db, typ) = self.paths(&id)?;
        if db.exists() && storage::probe(&db) == Probe::NotHistory {
            return Err(format!("{id}.db already exists in the Vault and belongs to another program. Pick another name."));
        }
        if db.exists() || typ.exists() {
            return Err(format!("There's already a Work called {id}."));
        }
        Ok(id)
    }

    /// Runs `f` on an open Work. If the engine panics, that Work's state in memory may be
    /// half-updated: it's thrown away (never saved) and the Work is reopened from disk.
    fn with_work<T>(&mut self, id: &str, now_ms: u64, f: impl FnOnce(&mut OpenWork, Option<&str>) -> Result<T, String>) -> Result<T, String> {
        let id = &self.current_id(id);
        let name = self.device_name();
        let work = self.works.get_mut(id).ok_or_else(|| format!("{id} isn't open."))?;
        match catch_unwind(AssertUnwindSafe(|| f(work, name.as_deref()))) {
            Ok(result) => result,
            Err(panic) => {
                let what = panic_message(panic.as_ref());
                self.works.remove(id);
                Err(match self.open_work(id, now_ms) {
                    Ok(_) => format!("Something went wrong in {id} ({what}). It was reopened from disk, with what you had typed recovered."),
                    Err(e) => format!("Something went wrong in {id} ({what}), and it couldn't be reopened: {e}"),
                })
            }
        }
    }

    /// Edits from the editor. Any edit elsewhere means a cut waiting in another Work won't be
    /// pasted there: it's recorded as a deletion.
    pub fn edit(&mut self, id: &str, mut ops: Vec<Op>, now_ms: u64) -> Result<WorkState, String> {
        // A Comment from a Reviewer: the Work records their current name with it.
        for op in &mut ops {
            if let Some((sender, name)) = op.sender_mut() {
                *name = sender.strip_prefix("reviewer/").and_then(|r| self.reviewer(r)).map(|r| r.name);
            }
        }
        if ops.iter().any(|op| !matches!(op, Op::Cursor { .. } | Op::Marks | Op::Tints)) {
            let others: Vec<String> = self.works.iter().filter(|(other, w)| *other != id && w.has_cut()).map(|(o, _)| o.clone()).collect();
            for other in others {
                let _ = self.with_work(&other, now_ms, |w, _| {
                    w.close_burst(now_ms);
                    w.save();
                    Ok(())
                });
            }
        }
        // An Inclusion records the ID of the Work it includes, which only the Vault can find.
        for op in &mut ops {
            if let Op::Include { work, chapter, target, .. } = op {
                let found = self.target_for(work, *chapter)?;
                // Asked before the editor put it in (`check_inclusion`); this only catches a race.
                self.refuse_loop(id, work, &found)?;
                *target = Some(found);
            }
        }
        let edits = ops.iter().any(|op| !matches!(op, Op::Cursor { .. } | Op::Sync | Op::Marks | Op::Tints | Op::Find { .. } | Op::Stretch { .. }));
        // What might add or remove an Inclusion (not plain typing).
        let reshapes = ops.iter().any(|op| match op {
            Op::Cursor { .. } | Op::Marks | Op::Tints | Op::Find { .. } | Op::Stretch { .. } => false,
            Op::Type { from, to, .. } => from != to,
            _ => true,
        });
        if edits {
            self.record_inclusions(id, now_ms);
        }
        // Before the first edit since the Style library last changed, the Work records the
        // versions of everything it uses (read here only when that's due).
        let (root, generation) = (self.root.clone(), self.library_generation);
        let mut state = self.with_work(id, now_ms, |w, name| {
            let shelf = w.styles_due(generation).then(|| Shelf::read(&root));
            w.edit(ops, name, shelf.as_ref().map(|s| (s, generation)), now_ms)
        })?;
        if reshapes {
            self.refresh_text_file_inclusions(id);
        }
        if let (Some(marks), Some(work)) = (state.marks.as_mut(), self.works.get(&self.current_id(id))) {
            for mark in marks {
                (mark.name, mark.colour) = self.sender_label(&work.doc, &mark.sender);
                for reply in &mut mark.replies {
                    (reply.name, reply.colour) = self.sender_label(&work.doc, &reply.sender);
                }
            }
        }
        if let (Some(tints), Some(work)) = (state.tints.as_mut(), self.works.get(&self.current_id(id))) {
            let name = self.device_namer();
            for tint in tints {
                tint.device = name(&work.doc, tint.device_id);
            }
        }
        Ok(state)
    }

    /// A Work's Preamble, as its History has it now. The Work needn't be open.
    pub fn preamble(&mut self, id: &str, now_ms: u64) -> Result<Preamble, String> {
        Ok(self.preamble_for_dialog(id, now_ms)?.preamble)
    }

    /// What the Preamble dialog shows: the Preamble, and the other Device's raw Typst if two
    /// Devices edited it without seeing each other's edit.
    pub fn preamble_for_dialog(&mut self, id: &str, now_ms: u64) -> Result<PreambleView, String> {
        let view = |doc: &Doc| PreambleView { preamble: Preamble::of(doc), raw_other: Preamble::raw_lost(doc) };
        if self.works.contains_key(&self.current_id(id)) {
            return self.with_work(id, now_ms, |w, _| Ok(view(&w.doc)));
        }
        let (db, _) = self.paths(id)?;
        let (_, doc) = storage::load(&db, self.device).map_err(err)?;
        Ok(view(&doc))
    }

    /// Changes a Work's Preamble (from the dialog), recording the library files it now uses.
    /// A Work that isn't open is changed in its `.db` directly. Returns the open Work's state.
    pub fn set_preamble(&mut self, id: &str, preamble: &Preamble, now_ms: u64) -> Result<Option<WorkState>, String> {
        preamble.check()?;
        let shelf = Shelf::read(&self.root);
        let generation = self.library_generation;
        if self.works.contains_key(&self.current_id(id)) {
            return self.with_work(id, now_ms, |w, _| Ok(Some(w.set_preamble(preamble, &shelf, generation, now_ms))));
        }
        let (db, _) = self.paths(id)?;
        let (me, name) = (self.device, self.device_name());
        let (_, mut doc) = storage::load(&db, me).map_err(err)?;
        let mut changes = preamble.record(&mut doc, now_ms);
        changes.extend(preamble::record_unrecorded(&mut doc, &shelf, now_ms));
        storage::save(&db, &changes, &doc).map_err(err)?;
        record_name(&db, &mut doc, name.as_deref(), now_ms)?;
        Ok(None)
    }

    /// What a compile of an open Work needs: its text as it is now, and the Preamble's Typst
    /// with the Style library's current versions. `look`: the one to render (else its first).
    /// Its Inclusions are written out as the included Works are now, and `Block`s say where.
    pub fn typeset_input(&mut self, id: &str, look: Option<&str>, now_ms: u64) -> Result<(String, preamble::Header, Vec<Block>), String> {
        let shelf = Shelf::read(&self.root);
        let shown = self.shown_in(id, false)?;
        self.with_work(id, now_ms, |w, _| {
            let uses = preamble::uses_plus(&w.doc, &shelf, Version::Library, &inclusions::elements_of(&shown));
            let (body, places) = inclusions::typst_body(&w.text(), &shown);
            let blocks = Block::all(&body, &places);
            Ok((body, preamble::header(&w.doc, uses, look), blocks))
        })
    }

    /// A Work's Milestones, oldest first. The Work needn't be open.
    pub fn milestones(&mut self, id: &str, now_ms: u64) -> Result<Vec<MilestoneInfo>, String> {
        let list = |doc: &Doc| doc.milestones().into_iter()
            .map(|m| MilestoneInfo { id: m.id.to_string(), name: m.name, time_ms: m.hlc.0 })
            .collect();
        if self.works.contains_key(&self.current_id(id)) {
            return self.with_work(id, now_ms, |w, _| Ok(list(&w.doc)));
        }
        let (db, _) = self.paths(id)?;
        let (_, doc) = storage::load(&db, self.device).map_err(err)?;
        Ok(list(&doc))
    }

    /// Names the current point in a Work's History. Changed Styles are recorded first, so the
    /// Milestone holds the versions the Page preview shows now.
    pub fn make_milestone(&mut self, id: &str, name: &str, now_ms: u64) -> Result<Option<WorkState>, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 200 {
            return Err("A Milestone's name needs 1 to 200 characters.".into());
        }
        let shelf = Shelf::read(&self.root);
        let generation = self.library_generation;
        if self.works.contains_key(&self.current_id(id)) {
            // The versions of the included Works it shows now go in first (ADR 0015).
            self.with_work(id, now_ms, |w, _| {
                w.close_burst(now_ms);
                Ok(())
            })?;
            self.record_inclusions(id, now_ms);
            return self.with_work(id, now_ms, |w, _| Ok(Some(w.make_milestone(name, &shelf, generation, now_ms))));
        }
        let (db, _) = self.paths(id)?;
        let (me, device_name) = (self.device, self.device_name());
        let (_, mut doc) = storage::load(&db, me).map_err(err)?;
        let mut changes = self.record_inclusions_in(&mut doc, now_ms);
        changes.extend(preamble::record_unrecorded(&mut doc, &shelf, now_ms));
        changes.push(doc.make_milestone_local(name, now_ms));
        storage::save(&db, &changes, &doc).map_err(err)?;
        record_name(&db, &mut doc, device_name.as_deref(), now_ms)?;
        Ok(None)
    }

    /// The Work as it was at a Milestone: built from only the Changes the Milestone holds, so
    /// its text, its Preamble and its recorded Styles are exactly as they were then.
    fn at_milestone(&mut self, id: &str, milestone: &str, now_ms: u64) -> Result<Doc, String> {
        let milestone: ChangeId = milestone.parse().map_err(|_| format!("Not a Milestone: {milestone}"))?;
        let device = self.device;
        // What it holds: the highest counter the naming Device had seen from each Device.
        let seen = |doc: &Doc| doc.milestones().into_iter().find(|m| m.id == milestone).map(|m| m.seen);
        let (db, seen) = if self.works.contains_key(&self.current_id(id)) {
            self.with_work(id, now_ms, |w, _| {
                w.save(); // the Milestone (and what it holds) must be in the file
                Ok((w.db.clone(), seen(&w.doc)))
            })?
        } else {
            let (db, _) = self.paths(id)?;
            let (_, doc) = storage::load(&db, device).map_err(err)?;
            (db, seen(&doc))
        };
        let seen = seen.ok_or_else(|| "That Milestone isn't in this Work (deleted, or not here yet).".to_string())?;
        let held = |c: ChangeId| seen.iter().any(|&(d, n)| d == c.device && c.counter <= n);
        let changes = storage::read_all(&db).map_err(err)?;
        let mut doc = Doc::new(device);
        doc.apply_all(&changes.into_iter().filter(|c| held(c.id)).collect::<Vec<_>>());
        Ok(doc)
    }

    /// What a compile of a Work as it was at a Milestone needs: its text then, and a header
    /// from the copies it had recorded then (never today's Style library).
    /// Its Inclusions show the versions it had recorded then (ADR 0015).
    pub fn typeset_input_at(&mut self, id: &str, milestone: &str, look: Option<&str>, now_ms: u64) -> Result<(String, preamble::Header, Vec<Block>), String> {
        let doc = self.at_milestone(id, milestone, now_ms)?;
        let uses = preamble::uses(&doc, &Shelf::read(&self.root), Version::Recorded);
        let shown = self.shown_in_doc(&doc);
        let (body, places) = inclusions::typst_body(&doc.committed_text(), &shown);
        let blocks = Block::all(&body, &places);
        Ok((body, preamble::header(&doc, uses, look), blocks))
    }

    /// What an Export of a Work as it was at a Milestone needs: exactly what it showed then,
    /// with the Styles and the versions of included Works it had recorded (ADR 0015). Nothing is
    /// recorded. Its date is when the Milestone was named, the same on every Device.
    pub fn export_input_at(&mut self, id: &str, milestone: &str, look: Option<&str>, now_ms: u64) -> Result<ExportInput, String> {
        let doc = self.at_milestone(id, milestone, now_ms)?;
        let named: ChangeId = milestone.parse().map_err(|_| format!("Not a Milestone: {milestone}"))?;
        let date_ms = self.milestone_time(id, named, now_ms)?;
        let header = preamble::header(&doc, preamble::uses(&doc, &Shelf::read(&self.root), Version::Recorded), look);
        let shown = self.shown_in_doc(&doc);
        let missing: Vec<String> = shown.iter().filter_map(|s| s.missing.as_ref().map(|m| format!("{}: {m}", s.label))).collect();
        let committed = doc.committed_text();
        let docx_text = inclusions::docx_body(&committed, &shown);
        let (text, _) = inclusions::typst_body(&committed, &shown);
        let (db, _) = self.paths(id)?;
        let ident = storage::identity(&db).map_err(err)?.unwrap_or_else(|| id.to_string());
        Ok(ExportInput { text, docx_text, header, preamble: Preamble::of(&doc), date_ms, ident, missing })
    }

    /// When a Milestone was named.
    fn milestone_time(&mut self, id: &str, milestone: ChangeId, now_ms: u64) -> Result<u64, String> {
        let find = |doc: &Doc| doc.milestones().into_iter().find(|m| m.id == milestone).map(|m| m.hlc.0);
        let found = if self.works.contains_key(&self.current_id(id)) {
            self.with_work(id, now_ms, |w, _| Ok(find(&w.doc)))?
        } else {
            let (db, _) = self.paths(id)?;
            find(&storage::load(&db, self.device).map_err(err)?.1)
        };
        found.ok_or_else(|| "That Milestone isn't in this Work (deleted, or not here yet).".to_string())
    }

    /// What a PDF Export of a Work needs, after recording changed Styles (so the Export is
    /// exactly a version in its History): its text, the header from its recorded copies in
    /// `look` (else its first), the time of its newest Change, and its ID.
    pub fn export_input(&mut self, id: &str, look: Option<&str>, now_ms: u64) -> Result<ExportInput, String> {
        let shelf = Shelf::read(&self.root);
        let generation = self.library_generation;
        let typeset = |doc: &Doc| (doc.committed_text(), preamble::header(doc, preamble::uses(doc, &shelf, Version::Recorded), look), Preamble::of(doc));
        let (db, (text, header, preamble), shown) = if self.works.contains_key(&self.current_id(id)) {
            // The versions of the included Works it shows now go in first (ADR 0015).
            self.with_work(id, now_ms, |w, _| {
                if !w.has_cut() {
                    w.close_burst(now_ms);
                }
                Ok(())
            })?;
            self.record_inclusions(id, now_ms);
            let db = self.with_work(id, now_ms, |w, _| {
                w.prepare_export(&shelf, generation, now_ms);
                if let Some(problem) = &w.state(Vec::new()).save_error {
                    return Err(problem.clone());
                }
                Ok(w.db.clone())
            })?;
            let shown = self.shown_in(id, true)?;
            let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
            (db, typeset(&work.doc), shown)
        } else {
            let (db, _) = self.paths(id)?;
            let (me, name) = (self.device, self.device_name());
            let (_, mut doc) = storage::load(&db, me).map_err(err)?;
            let mut changes = self.record_inclusions_in(&mut doc, now_ms);
            changes.extend(preamble::record_unrecorded(&mut doc, &shelf, now_ms));
            if !changes.is_empty() {
                storage::save(&db, &changes, &doc).map_err(err)?;
                record_name(&db, &mut doc, name.as_deref(), now_ms)?;
            }
            let shown = self.shown_in_doc(&doc);
            (db, typeset(&doc), shown)
        };
        let missing: Vec<String> = shown.iter().filter_map(|s| s.missing.as_ref().map(|m| format!("{}: {m}", s.label))).collect();
        let docx_text = inclusions::docx_body(&text, &shown);
        let (text, _) = inclusions::typst_body(&text, &shown);
        let date_ms = storage::read_all(&db).map_err(err)?.iter().map(|c| c.hlc.0).max().unwrap_or(0);
        let ident = storage::identity(&db).map_err(err)?.unwrap_or_else(|| id.to_string());
        Ok(ExportInput { text, docx_text, header, preamble, date_ms, ident, missing })
    }

    /// The Style update notice of every open Work, as the library is now.
    fn refresh_style_notices(&mut self) {
        let shelf = Shelf::read(&self.root);
        for work in self.works.values_mut() {
            work.style_notice = preamble::update_notice(&work.doc, &shelf);
        }
    }

    /// The Author left the Work for now (another Work, or another window).
    pub fn leave(&mut self, id: &str, now_ms: u64) -> Result<WorkState, String> {
        self.with_work(id, now_ms, |w, _| {
            w.leave(now_ms);
            Ok(w.state(Vec::new()))
        })
    }

    /// The text as the History has it now, for an editor that fell out of step.
    pub fn work_text(&mut self, id: &str, now_ms: u64) -> Result<(String, WorkState), String> {
        self.with_work(id, now_ms, |w, _| Ok((w.text(), w.state(Vec::new()))))
    }

    /// The timer, about once a second: closes bursts after a pause, retries failed saves and
    /// writes Text files when due. Returns the Works whose state the editor should hear about.
    pub fn tick(&mut self, now_ms: u64) -> Vec<(String, WorkState)> {
        let ids: Vec<String> = self.works.keys().cloned().collect();
        let mut before = HashMap::new();
        for id in &ids {
            let _ = self.with_work(id, now_ms, |w, _| {
                before.insert(id.clone(), w.state(Vec::new()));
                w.tick(now_ms);
                Ok(())
            });
        }
        // A History that disappeared from under an open tab: find where it went.
        self.follow_lost();
        let mut changed = Vec::new();
        for id in ids {
            let current = self.current_id(&id);
            let Some(work) = self.works.get_mut(&current) else { continue };
            let state = work.state(Vec::new());
            if Some(&state) != before.get(&id) {
                changed.push((id, state)); // under the ID the editor knows it by
            }
            if matches!(work.whereabouts, Some(Whereabouts::Renamed { .. })) {
                work.whereabouts = None; // said once
            }
        }
        changed
    }

    /// An open Work's ID now, following renames.
    fn current_id(&self, id: &str) -> String {
        let mut id = id.to_string();
        while let Some(next) = self.aliases.get(&id) {
            if *next == id { break; }
            id = next.clone();
        }
        id
    }

    pub fn close_work(&mut self, id: &str, now_ms: u64) {
        let id = &self.current_id(id);
        // One whose History went to the trash is saved there, so restoring it brings back everything.
        self.save_into_trash(id);
        let _ = self.with_work(id, now_ms, |w, _| {
            w.close(now_ms);
            Ok(())
        });
        self.works.remove(id);
        self.aliases.retain(|_, to| to != id);
    }

    /// The app went to the background, where Android may end it without warning: every open
    /// Work closes its burst (a cut waiting for its paste becomes a deletion), saves, and writes
    /// its Text file now, so Syncthing can send it. The Works stay open. Returns their states.
    pub fn pause_all(&mut self, now_ms: u64) -> Vec<(String, WorkState)> {
        let ids: Vec<String> = self.works.keys().cloned().collect();
        ids.into_iter()
            .filter_map(|id| {
                let state = self.with_work(&id, now_ms, |w, _| {
                    w.close(now_ms);
                    Ok(w.state(Vec::new()))
                });
                state.ok().map(|state| (id, state))
            })
            .collect()
    }

    /// Quitting, or opening another Vault: every open Work is closed and saved.
    pub fn close_all(&mut self, now_ms: u64) {
        let ids: Vec<String> = self.works.keys().cloned().collect();
        for id in ids {
            self.close_work(&id, now_ms);
        }
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

/// Can `name` be part of a new Work's path? It must be a valid file name on Windows, Debian and
/// Android, and not something the Vault browser skips. `top_folder`: the first folder in the
/// path; `top_file`: a Work right in the Vault's folder.
fn check_name(name: &str, top_folder: bool, top_file: bool) -> Result<(), String> {
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
                                "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
    if name.is_empty() {
        return Err("A Work needs a name.".into());
    }
    if let Some(c) = name.chars().find(|c| r#"<>:"\|?*"#.contains(*c) || c.is_control()) {
        return Err(format!("A name can't contain {c:?}."));
    }
    if name.ends_with('.') {
        return Err("A name can't end with a dot.".into());
    }
    if name.starts_with('.') || name.ends_with(".images") {
        return Err(format!("{name:?} would be hidden from the Vault browser."));
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Err(format!("{name:?} is reserved by Windows."));
    }
    if top_folder && NOT_WORKS.contains(&name.to_lowercase().as_str()) {
        return Err(format!("The {name} folder is for the app's own files, not Works."));
    }
    if top_file && name.eq_ignore_ascii_case("vault") {
        return Err("\"vault\" is the Vault's own file.".into());
    }
    Ok(())
}

/// A Work's `.db` and `.typ` from its ID, under `base` (the Vault, or its trash). The ID comes
/// from the interface, so it must stay inside.
fn paths_under(base: &Path, id: &str) -> Result<(PathBuf, PathBuf), String> {
    let parts: Vec<&str> = id.split('/').collect();
    let bad = |p: &&str| p.is_empty() || *p == "." || *p == ".." || p.contains(['\\', ':']);
    if parts.iter().any(bad) {
        return Err(format!("not a Work in this Vault: {id:?}"));
    }
    let (name, folders) = parts.split_last().expect("split gives at least one part");
    let dir = folders.iter().fold(base.to_path_buf(), |dir, part| dir.join(part));
    // Not `with_extension`: a name like "Draft 1.5" has a dot of its own.
    Ok((dir.join(format!("{name}.db")), dir.join(format!("{name}.typ"))))
}

/// The Histories, Text files and Conflict copies under `root` (the Vault, or its trash).
fn walk_works(root: &Path) -> Files {
    fn walk_dir(dir: &Path, top: bool, files: &mut Files) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let (path, name) = (entry.path(), entry.file_name().to_string_lossy().into_owned());
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                // Syncthing's own folders (.stfolder, .stversions with old copies of every
                // file), the trash, a Work's images, and the Style library, themes and fonts.
                let skip = name.starts_with('.') || name.ends_with(".images") || (top && NOT_WORKS.contains(&name.as_str()));
                if !skip {
                    files.folders.push(path.clone());
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
    walk_dir(root, true, &mut files);
    files.copies.sort();
    files
}

/// A file's name without its extension.
fn name_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What tests of the Vault's parts share.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub const ME: DeviceId = DeviceId(1);

    /// Opens (making it a Vault if needed) `root`, with recovery files in a hidden folder in it.
    pub fn open(root: &Path) -> Vault {
        match Vault::open(root, ME, true, &root.join(".recovery")).unwrap() {
            Opened::Vault(v) => v,
            Opened::NotAVault => panic!("asked to create it"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite_free::*;

    const ME: DeviceId = DeviceId(1);
    const OTHER: DeviceId = DeviceId(2);

    fn open(root: &Path) -> Vault {
        match Vault::open(root, ME, true, &root.join(".recovery")).unwrap() {
            Opened::Vault(v) => v,
            Opened::NotAVault => panic!("asked to create it"),
        }
    }

    #[test]
    fn a_new_vault_gets_the_default_styles_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        assert_eq!(vault.library().items.len(), library::DEFAULTS.len());
        assert_eq!(vault.vault_doc.field(DEFAULTS_RECORD, "Paperback.look"), Some(library::DEFAULTS_VERSION));
        assert!(!vault.library_changed(), "writing them isn't a change from outside");
        // An edit in another editor is noticed (the timer asks every second).
        std::fs::write(dir.path().join("styles/Book text.typ"), "// Style: Book text\n// Edited in vim.\n\n#set text(size: 12pt)\n").unwrap();
        assert!(vault.library_changed());
        assert!(!vault.library_changed(), "and only once");
        // A default the Author deleted stays deleted, on this Device or another.
        std::fs::remove_file(dir.path().join("styles/A4 draft.look")).unwrap();
        drop(vault);
        let Opened::Vault(other) = Vault::open(dir.path(), OTHER, false, &dir.path().join(".recovery")).unwrap() else { panic!() };
        assert!(!dir.path().join("styles/A4 draft.look").exists());
        assert_eq!(other.library().items.len(), library::DEFAULTS.len() - 1);
        // The library isn't listed as Works.
        assert!(!other.walk().typs.iter().any(|t| t.starts_with(dir.path().join("styles"))));
    }

    #[test]
    fn a_new_work_starts_with_a_look_and_records_it_with_the_first_edit() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        assert_eq!(vault.preamble("Rain", 1000).unwrap().looks, ["Paperback"]);
        // Opening it records nothing; the first edit records what it uses.
        let has_copy = |v: &mut Vault| v.with_work("Rain", 0, |w, _| Ok(w.doc.field("look/Paperback", "content").is_some())).unwrap();
        assert!(!has_copy(&mut vault));
        vault.edit("Rain", vec![typing(0, "The rain had stopped.")], 2000).unwrap();
        assert!(has_copy(&mut vault));
        // It compiles with the Preamble in front of its text.
        let (text, header, _) = vault.typeset_input("Rain", None, 3000).unwrap();
        assert_eq!(text, "The rain had stopped.");
        assert!(header.text.contains("#show: wrtier-look-1"));
    }

    #[test]
    fn a_style_changed_elsewhere_is_noticed_then_recorded_by_the_next_edit() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "abc")], 2000).unwrap();
        vault.tick(20_000); // the burst closes
        let notice = |v: &mut Vault| v.with_work("Rain", 0, |w, _| Ok(w.state(Vec::new()).style_notice)).unwrap();
        assert_eq!(notice(&mut vault), None);
        std::fs::write(dir.path().join("styles/Book text.typ"), "// Style: Book text\n// Bigger.\n#set text(size: 13pt)\n").unwrap();
        assert!(vault.library_changed());
        assert!(notice(&mut vault).is_some_and(|n| n.starts_with("Book text has changed")));
        // The Page preview already uses the new one.
        assert!(vault.typeset_input("Rain", None, 21_000).unwrap().1.text.contains("13pt"));
        vault.edit("Rain", vec![typing(3, "d")], 22_000).unwrap();
        assert_eq!(notice(&mut vault), None);
        assert!(vault.with_work("Rain", 0, |w, _| Ok(w.doc.field("style/Book text", "content").unwrap().contains("13pt"))).unwrap());
    }

    #[test]
    fn the_preamble_of_a_closed_work_can_be_changed() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.close_work("Rain", 1500);
        let p = Preamble { looks: vec!["Manuscript".into(), "Paperback".into()], title: "Rain".into(), language: "en".into(), ..Preamble::default() };
        assert_eq!(vault.set_preamble("Rain", &p, 2000).unwrap(), None);
        assert_eq!(vault.preamble("Rain", 2000).unwrap(), p);
        assert!(vault.set_preamble("Rain", &Preamble { language: "English".into(), ..p.clone() }, 3000).is_err());
        // Open again: it renders in Manuscript, from its own records.
        vault.open_work("Rain", 4000).unwrap();
        let (_, header, _) = vault.typeset_input("Rain", None, 4000).unwrap();
        assert!(header.text.contains("// Look: Manuscript\n#let wrtier-look-1"), "{}", header.text);
        assert!(header.text.contains("#set document(title: \"Rain\")"));
    }

    /// A Work with two Looks and Story elements, written, closed: its Text file on disk.
    fn written_work(dir: &Path) -> (Vault, String) {
        let mut vault = open(dir);
        vault.create_work("Rain", 1000).unwrap();
        let p = Preamble { looks: vec!["Manuscript".into(), "Paperback".into()], title: "Rain".into(), language: "en".into(), ..Preamble::default() };
        vault.set_preamble("Rain", &p, 1500).unwrap();
        let text = "= The Long Night\n\n#epigraph[The night is long.]\n\nThe rain had stopped.\n\n#scene-break()\n\nShe went.";
        vault.edit("Rain", vec![typing(0, text)], 2000).unwrap();
        vault.close_work("Rain", 3000);
        let file = std::fs::read_to_string(dir.join("Rain.typ")).unwrap();
        (vault, file)
    }

    /// Step 6's check: the Text file alone (as `typst compile` sees it) gives the app's pages.
    #[test]
    fn the_text_file_compiles_on_its_own_to_the_apps_pages() {
        let dir = tempfile::tempdir().unwrap();
        let (mut vault, file) = written_work(dir.path());
        assert!(file.starts_with(wrtier::text_file::HEADER_START), "{file}");
        assert!(file.contains("// Look: Manuscript\n#let wrtier-look-1"));
        let mut t = crate::typeset::Typesetter::new(false);
        let alone = t.compile("Rain.typ", Path::new("no-fonts"), file.clone());
        assert!(alone.errors.is_empty() && alone.warnings.is_empty(), "{:?} {:?}", alone.errors, alone.warnings);
        // The app, in the Work's first Look.
        vault.open_work("Rain", 4000).unwrap();
        let (text, header, _) = vault.typeset_input("Rain", None, 4000).unwrap();
        assert!(!text.contains("Preamble"), "the header isn't the Work's text");
        let app = t.compile_with("Rain", Path::new("no-fonts"), &header, text);
        assert_eq!(alone.pages, app.pages);
        for i in 0..app.pages.len() {
            assert_eq!(t.page("Rain.typ", i, None).unwrap().svg, t.page("Rain", i, None).unwrap().svg, "page {}", i + 1);
        }
    }

    #[test]
    fn edits_in_the_header_are_dropped_and_edits_in_the_text_kept() {
        let dir = tempfile::tempdir().unwrap();
        let (mut vault, file) = written_work(dir.path());
        let edited = file.replace("#show: wrtier-look-1", "#show: wrtier-look-2").replace("had stopped", "had eased");
        std::fs::write(dir.path().join("Rain.typ"), &edited).unwrap();
        let opened = vault.open_work("Rain", 5000).unwrap();
        assert!(opened.text.contains("The rain had eased."), "{}", opened.text);
        assert!(!opened.text.contains("wrtier-look"));
        assert!(opened.notice.is_some_and(|n| n.contains("Edits to the Preamble at the top of Rain.typ were dropped")));
        // Written again with the History's header (when the Text file is next brought up to date).
        vault.leave("Rain", 6000).unwrap();
        let again = std::fs::read_to_string(dir.path().join("Rain.typ")).unwrap();
        assert!(again.contains("#show: wrtier-look-1") && again.contains("had eased"), "{again}");
    }

    #[test]
    fn an_older_text_file_without_a_header_is_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let (mut vault, file) = written_work(dir.path());
        // As an app from before step 6 would have written it, then edited elsewhere.
        let (body, _) = wrtier::text_file::split_marker(&file);
        let (_, text) = wrtier::text_file::split_header(&body);
        std::fs::write(dir.path().join("Rain.typ"), text.replace("She went.", "She went home.")).unwrap();
        let opened = vault.open_work("Rain", 5000).unwrap();
        assert!(opened.text.ends_with("She went home."), "{}", opened.text);
        assert!(opened.notice.is_none());
    }

    /// Every page of a compile, as SVG.
    fn pictures(t: &mut crate::typeset::Typesetter, key: &str, header: &preamble::Header, text: String) -> Vec<String> {
        let c = t.compile_with(key, Path::new("no-fonts"), header, text);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        (0..c.pages.len()).map(|i| t.page(key, i, None).unwrap().svg.unwrap()).collect()
    }

    /// Step 7's check: an old Milestone renders unchanged after a Style is edited.
    #[test]
    fn a_milestone_renders_as_it_was_after_a_style_is_edited() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "= The Long Night\n\nThe rain had stopped.")], 2000).unwrap();
        vault.tick(20_000);
        let mut t = crate::typeset::Typesetter::new(false);
        let (text, header, _) = vault.typeset_input("Rain", None, 21_000).unwrap();
        let before = pictures(&mut t, "now", &header, text);
        vault.make_milestone("Rain", "Draft 1", 22_000).unwrap();
        let milestone = vault.milestones("Rain", 22_000).unwrap().remove(0);
        assert_eq!(milestone.name, "Draft 1");

        // Afterwards: the Style changes in the library (and is recorded by the next edit), and
        // the text changes too.
        std::fs::write(dir.path().join("styles/Book text.typ"), "// Style: Book text\n// Bigger.\n#set text(size: 16pt)\n").unwrap();
        assert!(vault.library_changed());
        vault.edit("Rain", vec![typing(37, " Then it rained again.")], 30_000).unwrap();
        vault.tick(50_000);
        let (text, header, _) = vault.typeset_input("Rain", None, 51_000).unwrap();
        assert_ne!(pictures(&mut t, "now", &header, text), before, "the Work itself looks different now");

        // The Milestone: its text and its Styles as they were, page for page.
        let (text, header, _) = vault.typeset_input_at("Rain", &milestone.id, None, 52_000).unwrap();
        assert_eq!(text, "= The Long Night\n\nThe rain had stopped.");
        assert!(!header.text.contains("16pt"));
        assert_eq!(pictures(&mut t, "then", &header, text), before);

        // The same with the Work closed, and after the library's file is gone altogether.
        vault.close_work("Rain", 53_000);
        std::fs::remove_file(dir.path().join("styles/Book text.typ")).unwrap();
        let (text, header, _) = vault.typeset_input_at("Rain", &milestone.id, None, 54_000).unwrap();
        assert_eq!(pictures(&mut t, "then", &header, text), before);
        assert!(vault.typeset_input_at("Rain", "00000001:999", None, 54_000).is_err());
    }

    fn export(vault: &mut Vault, look: Option<&str>, now_ms: u64) -> Result<Vec<u8>, String> {
        let input = vault.export_input("Rain", look, now_ms)?;
        crate::typeset::Typesetter::new(false)
            .export_pdf(Path::new("no-fonts"), &input.header, input.text, input.date_ms, &input.ident)
            .map(|pdf| pdf.bytes)
    }

    /// Step 8's check: the same version gives the same PDF, byte for byte, whenever and on
    /// whichever Device it's exported.
    #[test]
    fn a_pdf_is_the_same_byte_for_byte_on_every_device() {
        let dir = tempfile::tempdir().unwrap();
        let (mut vault, _) = written_work(dir.path());
        vault.open_work("Rain", 4000).unwrap();
        let first = export(&mut vault, None, 5000).unwrap();
        assert!(first.starts_with(b"%PDF"));
        assert_eq!(export(&mut vault, None, 999_000).unwrap(), first, "exported later: the same");
        // Its date is the newest Change's, not the day it was exported (these run in 1970).
        let pdf = String::from_utf8_lossy(&first);
        assert!(pdf.contains("1970"), "dated from the History");
        // Another Device, with its own copy of the Vault, exporting the same version.
        vault.close_work("Rain", 6000);
        let Opened::Vault(mut other) = Vault::open(dir.path(), OTHER, false, &dir.path().join(".recovery2")).unwrap() else { panic!() };
        assert_eq!(export(&mut other, None, 7000).unwrap(), first, "another Device: the same");
        // Another Look is another PDF.
        assert_ne!(export(&mut other, Some("Paperback"), 8000).unwrap(), first);
    }

    #[test]
    fn a_work_that_doesnt_typeset_isnt_exported() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "The rain.\n\n#nothing-like-this")], 2000).unwrap();
        let refused = export(&mut vault, None, 3000).unwrap_err();
        assert!(refused.contains("line 3") && refused.contains("Nothing was exported"), "{refused}");
    }

    #[test]
    fn a_conflict_copy_in_the_library_is_a_sync_problem() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        std::fs::write(dir.path().join("styles/A4 page.sync-conflict-20261107-101500-ABCDEFG.typ"), "// Style: A4 page\n").unwrap();
        let listing = vault.list(1000);
        assert!(listing.problems.iter().any(|p| p.path == "styles/A4 page.sync-conflict-20261107-101500-ABCDEFG.typ" && p.reason.contains("A4 page.typ")), "{:?}", listing.problems);
    }

    /// A Work `name` with `text`, written by `device`, with its Text file.
    fn work(root: &Path, name: &str, text: &str, device: DeviceId) -> (PathBuf, Doc) {
        let (db, typ) = (root.join(format!("{name}.db")), root.join(format!("{name}.typ")));
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let mut doc = Doc::new(device);
        storage::save(&db, &[doc.insert_local(0, text, 1000)], &doc).unwrap();
        storage::write_text_file(&db, &typ, &doc, "", 1000).unwrap();
        (db, doc)
    }

    fn kinds(listing: &Listing) -> Vec<(&str, &EntryKind)> {
        listing.entries.iter().map(|e| (e.id.as_str(), &e.kind)).collect()
    }

    #[test]
    fn a_folder_becomes_a_vault_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(Vault::open(dir.path(), ME, false, &dir.path().join(".recovery")).unwrap(), Opened::NotAVault));
        assert!(!dir.path().join(VAULT_FILE).exists());
        let mut vault = open(dir.path());
        assert_eq!(vault.device_name(), None);
        vault.set_device_name("Laptop", 2000).unwrap();
        // Another start of the app finds it.
        let Opened::Vault(again) = Vault::open(dir.path(), ME, false, &dir.path().join(".recovery")).unwrap() else { panic!("it's a Vault now") };
        assert_eq!(again.device_name().as_deref(), Some("Laptop"));
    }

    #[test]
    fn a_vault_file_from_another_program_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        create_foreign_db(&dir.path().join(VAULT_FILE));
        assert!(Vault::open(dir.path(), ME, true, &dir.path().join(".recovery")).is_err());
    }

    #[test]
    fn listing_shows_works_and_hides_only_what_surely_isnt_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        work(root, "The Long Night", "It was dark.", ME);
        // (From another Device: two Works without a given ID, begun by one Device in the same
        // millisecond, would be one Work to `storage::identity`.)
        work(root, "Short stories/Rain", "The rain had stopped.", OTHER);
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
        assert!(listing.problems[0].newer, "updating is the fix");
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

    use crate::work::{Op, Origin};
    use wrtier::change::ChangeKind;

    fn typing(from: usize, text: &str) -> Op {
        Op::Type { from, to: from, text: text.into(), origin: Origin::Input }
    }

    /// The app stops without closing anything (a crash), then the Vault is opened again.
    fn crash_and_reopen(vault: Vault, root: &Path) -> Vault {
        std::mem::forget(vault); // nothing is closed or cleaned up
        let mut vault = open(root);
        vault.list(50_000);
        vault
    }

    #[test]
    fn typing_open_at_a_crash_is_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.set_device_name("Laptop", 500).unwrap();
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "The rain had stopped.")], 2000).unwrap();
        let typed = |root: &Path| storage::read_all(&root.join("Rain.db")).unwrap().iter().any(|c| matches!(c.kind, ChangeKind::Insert(_)));
        assert!(!typed(root), "the burst was still open");

        let mut vault = crash_and_reopen(vault, root);
        let saved = storage::read_all(&root.join("Rain.db")).unwrap();
        assert!(saved.iter().any(|c| matches!(c.kind, ChangeKind::Insert(_)) && c.hlc.0 >= 2000 && c.hlc.0 < 50_000),
                "recorded at its last keystroke: {saved:?}");
        let typ = std::fs::read_to_string(root.join("Rain.typ")).unwrap();
        assert_eq!(wrtier::text_file::split_header(&wrtier::text_file::split_marker(&typ).0).1, "The rain had stopped.");
        let opened = vault.open_work("Rain", 60_000).unwrap();
        assert_eq!(opened.text, "The rain had stopped.");
        assert!(opened.notice.unwrap().contains("Recovered 21 characters"));
        // It's part of the sitting: undo takes it back.
        vault.edit("Rain", vec![Op::Undo], 61_000).unwrap();
        assert_eq!(vault.works["Rain"].text(), "");
        assert!(recovery::kept_for(&root.join(".recovery"), root).is_empty(), "nothing left to recover");
    }

    #[test]
    fn going_to_the_background_puts_everything_on_disk_and_keeps_the_work_open() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "One. Two.")], 2000).unwrap();
        vault.tick(10_000); // saved
        // Cut, and typing still open, when the app goes to the background.
        vault.edit("Rain", vec![Op::Cut { from: 0, to: 5 }], 11_000).unwrap();
        let paused = vault.pause_all(12_000);
        assert_eq!(paused.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["Rain"]);
        let saved = storage::read_all(&root.join("Rain.db")).unwrap();
        assert!(saved.iter().any(|c| matches!(c.kind, ChangeKind::Delete(_))), "the waiting cut is a deletion: {saved:?}");
        let typ = std::fs::read_to_string(root.join("Rain.typ")).unwrap();
        assert_eq!(wrtier::text_file::split_header(&wrtier::text_file::split_marker(&typ).0).1, "Two.");
        // Still open: typing goes on as before.
        vault.edit("Rain", vec![typing(4, " Three.")], 13_000).unwrap();
        assert_eq!(vault.works["Rain"].text(), "Two. Three.");
    }

    #[test]
    fn a_cut_waiting_at_a_crash_goes_back_where_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "One. Two.")], 2000).unwrap();
        vault.tick(10_000); // saved
        vault.edit("Rain", vec![Op::Cut { from: 0, to: 5 }], 11_000).unwrap();
        let mut vault = crash_and_reopen(vault, root);
        let opened = vault.open_work("Rain", 60_000).unwrap();
        assert_eq!(opened.text, "One. Two.");
        assert!(opened.notice.unwrap().contains("cut"));
    }

    #[test]
    fn a_recovery_already_saved_isnt_recorded_twice() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "abc")], 2000).unwrap();
        // Keep a copy of the recovery file, then let the burst be saved normally.
        let files = Recovery::for_work(&root.join(".recovery"), root, "Rain");
        let copy = std::fs::read(&files.json).unwrap();
        vault.tick(10_000);
        std::fs::write(&files.json, copy).unwrap(); // as if the app crashed before removing it
        let mut vault = crash_and_reopen(vault, root);
        assert_eq!(vault.open_work("Rain", 60_000).unwrap().text, "abc");
        let inserts = storage::read_all(&root.join("Rain.db")).unwrap().iter().filter(|c| matches!(c.kind, ChangeKind::Insert(_))).count();
        assert_eq!(inserts, 1);
    }

    #[test]
    fn a_recovery_without_its_history_is_kept_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "abc")], 2000).unwrap();
        std::mem::forget(vault);
        std::fs::rename(root.join("Rain.db"), root.join("Elsewhere.db")).unwrap(); // gone, e.g. renamed on another Device
        let mut vault = open(root);
        let listing = vault.list(50_000);
        assert!(listing.problems.iter().any(|p| p.path == "Rain" && p.reason.contains("kept")), "{:?}", listing.problems);
        assert_eq!(recovery::kept_for(&root.join(".recovery"), root), vec!["Rain".to_string()]);
    }

    #[test]
    fn syncthing_is_told_to_ignore_sqlites_temporary_files() {
        // The Vault is a folder inside a Syncthing folder, whose .stignore has a pattern already.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".stfolder")).unwrap();
        std::fs::write(dir.path().join(".stignore"), "*.tmp").unwrap();
        let root = dir.path().join("Writing");
        std::fs::create_dir_all(&root).unwrap();
        open(&root);
        open(&root); // only once
        assert_eq!(std::fs::read_to_string(dir.path().join(".stignore")).unwrap(),
                   "*.tmp\n// Added by wrtier: SQLite's temporary files must never sync.\n*-journal\n");
        assert!(!root.join(".stignore").exists());
    }

    #[test]
    fn a_new_work_is_empty_open_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        let opened = vault.create_work("Rain", 1000).unwrap();
        assert_eq!((opened.id.as_str(), opened.text.as_str()), ("Rain", ""));
        assert!(root.join("Rain.db").exists() && root.join("Rain.typ").exists());
        vault.create_work(" Short stories / The Long Night ", 1000).unwrap();
        assert_eq!(kinds(&vault.list(2000)), vec![
            ("Rain", &EntryKind::Work),
            ("Short stories/The Long Night", &EntryKind::Work),
        ]);
        assert!(vault.create_work("Rain", 3000).is_err(), "already there");
    }

    #[test]
    fn a_text_file_next_to_another_programs_database_is_shown_blocked() {
        // Like the Phase 0c spike's test.db, with a test.typ added by hand.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        create_foreign_db(&root.join("test.db"));
        std::fs::write(root.join("test.typ"), "").unwrap();
        let listing = vault.list(1000);
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].id, "test");
        assert!(matches!(listing.entries[0].kind, EntryKind::Blocked { .. }));
        let refused = vault.create_work("test", 2000).unwrap_err();
        assert!(refused.contains("another program"), "{refused}");
    }

    #[test]
    fn a_new_works_name_must_work_everywhere() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        for bad in ["", "a:b", "what?", "end.", ".hidden", "Rain.images", "CON", "nul.txt", "styles/Rain", "vault", "a//b", "../out"] {
            assert!(vault.create_work(bad, 1000).is_err(), "{bad:?}");
        }
        assert!(vault.create_work("Draft 1.5", 1000).is_ok());
        assert!(vault.create_work("Notes/vault", 1000).is_ok(), "only the top-level name is the Vault's");
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
