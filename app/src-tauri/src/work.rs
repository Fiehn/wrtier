//! A Work open in the app: its History in memory, and what the editor does to it.
//!
//! The editor applies every edit at once and sends it here, in order. Typing and deleting go
//! into typing bursts; a paste is its own Change; a cut waits for a paste, so pasting the same
//! text in the same Work is a Move; dragging a selection is a Move. Undo works inside the open
//! burst, and beyond it reverts this sitting's latest Change still in effect.

use crate::patch::{self, Patch};
use crate::positions::{byte_at_utf16, replace_utf16, Positions};
use crate::recovery::Recovery;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use wrtier::change::{Change, ChangeKind};
use wrtier::conflicts::conflict_copies;
use wrtier::doc::{BurstKind, CutPasted, Doc};
use wrtier::ids::ChangeId;
use wrtier::storage;

/// The Text file is written at most this often while writing (and whenever the Author leaves).
pub const TEXT_FILE_EVERY_MS: u64 = 30_000;

/// Who made an edit in the editor: the Author, or the editor's own undo/redo inside a burst.
#[derive(Deserialize, Default, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    #[default]
    Input,
    Undo,
    Redo,
}

/// One thing the editor did. Positions are UTF-16, in the text as it was just before this op.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    /// The cursor moved (no text changed).
    Cursor { pos: usize },
    /// Typing, or typing over a selection.
    Type { from: usize, to: usize, text: String, #[serde(default)] origin: Origin },
    /// Backspace, Delete, or deleting a selection.
    Delete { from: usize, to: usize, #[serde(default)] origin: Origin },
    /// Pasting or dropping text in (over `from..to` if something was selected).
    Paste { from: usize, to: usize, text: String },
    Cut { from: usize, to: usize },
    /// Dragging the selection `from..to` to `at` (a position before the move).
    Move { from: usize, to: usize, at: usize },
    /// Ctrl+Z / Ctrl+Shift+Z beyond what the editor can undo inside the open burst.
    Undo,
    Redo,
    /// Take in what changed outside the app (`WorkState::outside` said so). It comes through
    /// the editor's queue, so it's in order with the typing and the editor gets patches.
    Sync,
}

impl Op {
    /// What the editor shows after this op: the edit as it was made, before the History has a say.
    fn as_the_editor_shows_it(&self, text: &str) -> String {
        match self {
            Op::Type { from, to, text: t, .. } | Op::Paste { from, to, text: t } => replace_utf16(text, *from, *to, t),
            Op::Delete { from, to, .. } | Op::Cut { from, to } => replace_utf16(text, *from, *to, ""),
            Op::Move { from, to, at } => {
                let moved = &text[byte_at_utf16(text, *from)..byte_at_utf16(text, *to)];
                let without = replace_utf16(text, *from, *to, "");
                let at = if at > from { at - (to - from) } else { *at };
                replace_utf16(&without, at, at, moved)
            }
            Op::Cursor { .. } | Op::Undo | Op::Redo | Op::Sync => text.to_string(),
        }
    }

    /// Can the History's text end up different from what the editor shows? Then the editor gets
    /// patches, and the editor never sends anything after such an op in the same batch.
    fn may_differ(&self, cut_waiting: bool) -> bool {
        match self {
            Op::Undo | Op::Redo | Op::Sync => true,
            Op::Type { origin, .. } | Op::Delete { origin, .. } => *origin != Origin::Input,
            Op::Paste { .. } => cut_waiting,
            _ => false,
        }
    }
}

/// What the editor needs to know after a batch of ops (and after the timer saved something).
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkState {
    /// The text's length (UTF-16): the editor checks it's the same as its own.
    pub len: usize,
    /// Edits to apply to the editor's text as it was after the batch.
    pub patches: Vec<Patch>,
    /// The open burst's ID: when it changes, the editor's undo steps no longer apply.
    pub burst: Option<String>,
    /// Why the last save failed, if it did (the Changes stay in memory and are retried).
    pub save_error: Option<String>,
    /// Why the Text file isn't being written, if it isn't.
    pub text_file_problem: Option<String>,
    /// Why the recovery file can't be kept, if it can't.
    pub recovery_problem: Option<String>,
    /// Goes up each time something changed outside the app: the editor then sends a `Sync`.
    pub outside: u32,
    /// What a `Sync` took in, to tell the Author (only in the answer to that `Sync`).
    pub took_in: Option<String>,
    /// A Conflict copy that couldn't be taken in, if any.
    pub sync_problem: Option<String>,
    /// The Work's History moved away underneath the open tab (on another Device), if it did.
    pub moved: Option<Whereabouts>,
}

/// Where a Work's History went when it disappeared from under an open tab.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Whereabouts {
    /// Renamed or moved inside the Vault: the tab follows it to its new ID.
    Renamed { to: String },
    /// Moved to the trash: saving pauses until it's restored or the tab is closed.
    Trashed { db: PathBuf },
    /// Nowhere in the Vault: saving pauses.
    Gone,
    /// The History in its file went its own way (another copy of it was edited separately):
    /// saving pauses, since mixing the two would break it. What's here can go to a new Work.
    Diverged,
}

/// When a file was last modified, and its size: if either changes, someone else wrote it.
type Stamp = Option<(std::time::SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

pub struct OpenWork {
    pub db: PathBuf,
    pub typ: PathBuf,
    pub doc: Doc,
    positions: Positions,
    /// Has this Device's name been checked (and recorded if needed) this sitting?
    named: bool,
    /// The content Changes made here since the Work was opened: what undo goes back through.
    sitting: Vec<ChangeId>,
    /// Changes undone, most recent last: what redo restores.
    redo: Vec<ChangeId>,
    /// The text of a cut waiting to be pasted. Its deletion burst is open in `doc`.
    cut: Option<String>,
    /// Changes not saved yet (only if saving failed).
    unsaved: Vec<Change>,
    save_error: Option<String>,
    /// Does the Text file lag behind the `.db`?
    text_file_behind: bool,
    text_file_written_ms: u64,
    text_file_problem: Option<String>,
    /// Keeps what's only in memory (the open burst, a waiting cut, unsaved Changes) on disk.
    recovery: Recovery,
    recovery_problem: Option<String>,
    /// Every Change this Device made in this Work that it knows of. If Syncthing delivers a
    /// `.db` without some of them, they're saved into it again.
    mine: Vec<Change>,
    /// The `.db` and Text file as this Work last wrote or read them.
    db_stamp: Stamp,
    typ_stamp: Stamp,
    /// Counts the times something changed outside (see `WorkState::outside`).
    outside: u32,
    /// An edit to the Text file waits for the open burst to close (it's recorded at once).
    typ_waiting: bool,
    /// The Text file was written from Changes that haven't arrived yet: it's newer than ours.
    typ_newer: bool,
    /// Conflict copies already tried: only new ones count as a change from outside.
    copies_tried: HashSet<PathBuf>,
    took_in: Option<String>,
    sync_problem: Option<String>,
    /// What tells this Work from every other (see `storage::identity`): how it's found again
    /// if it's moved or renamed elsewhere.
    identity: Option<String>,
    pub whereabouts: Option<Whereabouts>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Is this a Change to the text that undo can take back?
pub fn undoable(change: &Change) -> bool {
    matches!(change.kind, ChangeKind::Insert(_) | ChangeKind::Delete(_) | ChangeKind::Replace { .. } | ChangeKind::Move { .. })
}

impl OpenWork {
    /// `sitting`: Changes already made in this sitting (recovered ones), which undo can take back.
    /// `mine`: the Changes this Device made in the Work, as loaded.
    pub fn new(db: PathBuf, typ: PathBuf, doc: Doc, now_ms: u64, recovery: Recovery, sitting: Vec<ChangeId>, mine: Vec<Change>) -> Self {
        let positions = Positions::new(&doc.text());
        let (db_stamp, typ_stamp) = (stamp(&db), stamp(&typ));
        OpenWork {
            identity: storage::identity(&db).ok().flatten(),
            whereabouts: None,
            mine,
            db_stamp,
            typ_stamp,
            outside: 0,
            typ_waiting: false,
            typ_newer: false,
            copies_tried: HashSet::new(),
            took_in: None,
            sync_problem: None,
            db, typ, doc, positions,
            named: false,
            sitting,
            redo: Vec::new(),
            cut: None,
            unsaved: Vec::new(),
            save_error: None,
            text_file_behind: false,
            text_file_written_ms: now_ms,
            text_file_problem: None,
            recovery,
            recovery_problem: None,
        }
    }

    pub fn text(&self) -> String {
        self.doc.text()
    }

    pub fn state(&self, patches: Vec<Patch>) -> WorkState {
        WorkState {
            len: self.positions.utf16_len(),
            patches,
            burst: self.doc.burst().map(|b| b.id.to_string()),
            save_error: self.save_error.clone(),
            text_file_problem: self.text_file_problem.clone(),
            recovery_problem: self.recovery_problem.clone(),
            outside: self.outside,
            took_in: None,
            sync_problem: self.sync_problem.clone(),
            moved: self.whereabouts.clone(),
        }
    }

    /// What tells this Work from every other (see `storage::identity`).
    pub fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }

    /// Has its History disappeared from where this tab has it (renamed, trashed or deleted
    /// elsewhere), or gone its own way? Then nothing is saved: saving would create a new,
    /// broken History there, or mix two.
    pub fn is_lost(&self) -> bool {
        self.whereabouts == Some(Whereabouts::Diverged) || !self.db.exists()
    }

    /// Does the History in `db` agree with every Change this Device made in this Work? (If
    /// not, it's a copy that went its own way, and this tab must not move onto it.)
    pub fn agrees_with(&self, db: &Path) -> bool {
        storage::agrees_with(db, &self.mine).unwrap_or(false)
    }

    /// The History's file has the same IDs as this Work's own Changes for different ones.
    fn diverged(&mut self) {
        self.whereabouts = Some(Whereabouts::Diverged);
        self.save_error = None;
    }

    /// Drops the open Work without saving (its History is gone and the Author chose to close
    /// it): its recovery file goes too.
    pub fn discard(&mut self) {
        let _ = self.recovery.clear();
    }

    /// Points the open Work at its History's new place (renamed here or on another Device).
    /// Its recovery file moves along.
    pub fn relocate(&mut self, db: PathBuf, typ: PathBuf, recovery: Recovery) {
        let _ = self.recovery.clear();
        self.recovery = recovery;
        self.db = db;
        self.typ = typ;
        self.db_stamp = stamp(&self.db);
        self.typ_stamp = stamp(&self.typ);
        self.copies_tried.clear();
        self.text_file_behind = true;
        self.keep_recovery(true);
    }

    /// Updates the recovery file with what's only in memory. `flush`: also make sure it's on the
    /// disk itself (the timer does that about once a second).
    fn keep_recovery(&mut self, flush: bool) {
        let mut result = self.recovery.keep(&self.doc, self.cut.is_some(), &self.unsaved);
        if flush && result.is_ok() {
            result = self.recovery.flush();
        }
        self.recovery_problem = result.err().map(|e| format!("Couldn't keep a recovery copy of what you're typing: {e}"));
    }

    pub fn has_cut(&self) -> bool {
        self.cut.is_some()
    }

    /// Applies a batch of ops from the editor, saves, and says what the editor needs to know.
    pub fn edit(&mut self, ops: Vec<Op>, device_name: Option<&str>, now_ms: u64) -> Result<WorkState, String> {
        if ops.iter().any(|op| !matches!(op, Op::Cursor { .. } | Op::Sync)) {
            self.name_this_device(device_name, now_ms)?;
        }
        let count = ops.len();
        let mut patches = Vec::new();
        let mut result = Ok(());
        for (i, op) in ops.into_iter().enumerate() {
            let shows = op.may_differ(self.cut.is_some()).then(|| op.as_the_editor_shows_it(&self.doc.text()));
            result = self.apply(op, now_ms);
            if result.is_err() {
                break;
            }
            if let Some(shows) = shows {
                let text = self.doc.text();
                self.positions = Positions::new(&text);
                if shows != text {
                    if i + 1 != count {
                        result = Err("the editor sent more after an edit the History changed".into());
                        break;
                    }
                    patches = patch::patches(&shows, &text);
                }
            }
        }
        self.save();
        self.keep_recovery(false);
        #[cfg(debug_assertions)]
        assert_eq!(self.positions, Positions::new(&self.doc.text()), "the position map drifted from the text");
        let took_in = self.took_in.take();
        result.map(|()| WorkState { took_in, ..self.state(patches) })
    }

    /// Records this Device's name in the Work before its first edit in a sitting, while no burst
    /// can be open yet (nothing else may be recorded while one is), if it's missing or old.
    fn name_this_device(&mut self, device_name: Option<&str>, now_ms: u64) -> Result<(), String> {
        if self.named || self.doc.burst().is_some() {
            return Ok(());
        }
        self.named = true;
        let Some(name) = device_name else { return Ok(()) };
        if self.doc.device_name(self.doc.device()) != Some(name) {
            let change = self.doc.set_device_name_local(name, now_ms);
            self.unsaved.push(change);
        }
        Ok(())
    }

    fn chars(&self, from: usize, to: usize) -> Result<(usize, usize), String> {
        let (f, t) = (self.positions.to_char(from).map_err(err)?, self.positions.to_char(to).map_err(err)?);
        if f > t {
            return Err(format!("a range that ends before it starts ({from}..{to})"));
        }
        Ok((f, t))
    }

    /// Changes the engine recorded: saved soon; the Author's edits are what undo goes back through.
    fn recorded(&mut self, changes: impl IntoIterator<Item = Change>) {
        for change in changes {
            if undoable(&change) {
                self.sitting.push(change.id);
            }
            self.unsaved.push(change);
        }
    }

    /// A new edit by the Author: what was undone can't be redone any more.
    fn authored(&mut self) {
        self.redo.clear();
    }

    /// A cut that won't become a Move: it's a deletion now.
    fn finish_cut(&mut self, now_ms: u64) {
        if self.cut.take().is_some() {
            let change = self.doc.close_burst(now_ms);
            self.recorded(change);
        }
    }

    /// Closes the open burst (a waiting cut becomes a deletion).
    pub fn close_burst(&mut self, now_ms: u64) {
        self.finish_cut(now_ms);
        let change = self.doc.close_burst(now_ms);
        self.recorded(change);
    }

    fn apply(&mut self, op: Op, now_ms: u64) -> Result<(), String> {
        match op {
            Op::Cursor { pos } => {
                if self.cut.is_none() { // looking around between a cut and its paste changes nothing
                    let at = self.positions.to_char(pos).map_err(err)?;
                    let change = self.doc.cursor_moved(at, now_ms);
                    self.recorded(change);
                }
            }
            Op::Type { from, to, text, origin } => {
                let (f, t) = self.chars(from, to)?;
                // The editor's redo inside a draft is just typing again (it opens the burst again
                // if undo had emptied it); only its undo needs care.
                if origin == Origin::Undo {
                    return self.editor_undo(f, t, &text, now_ms);
                }
                self.authored();
                self.finish_cut(now_ms);
                let changes = if f == t { self.doc.type_text(f, &text, now_ms) } else { self.doc.replace_text(f, t - f, &text, now_ms) };
                self.recorded(changes);
                self.positions.edit(f, t - f, &text);
            }
            Op::Delete { from, to, origin } => {
                let (f, t) = self.chars(from, to)?;
                if origin == Origin::Undo {
                    return self.editor_undo(f, t, "", now_ms);
                }
                self.authored();
                self.finish_cut(now_ms);
                let changes = self.doc.delete_text(f, t - f, now_ms);
                self.recorded(changes);
                self.positions.edit(f, t - f, "");
            }
            Op::Paste { from, to, text } => {
                let (f, t) = self.chars(from, to)?;
                self.authored();
                if f == t && self.cut.as_deref() == Some(text.as_str()) {
                    // The cut text pasted unchanged in the same Work: a Move, so it keeps its History.
                    self.cut = None;
                    match self.doc.cut_to_move(f, now_ms) {
                        CutPasted::Moved(change) => self.recorded([change]),
                        CutPasted::Unchanged => {}
                        CutPasted::Deleted(change) => {
                            self.recorded(change);
                            let change = self.doc.insert_local(f, &text, now_ms);
                            self.recorded([change]);
                        }
                    }
                } else {
                    self.close_burst(now_ms);
                    let change = if f == t { self.doc.insert_local(f, &text, now_ms) } else { self.doc.replace_local(f, t - f, &text, now_ms) };
                    self.recorded([change]);
                }
                self.positions.edit(f, t - f, &text);
            }
            Op::Cut { from, to } => {
                let (f, t) = self.chars(from, to)?;
                self.authored();
                self.close_burst(now_ms);
                if f < t {
                    let text: String = self.doc.text().chars().skip(f).take(t - f).collect();
                    let changes = self.doc.delete_text(f, t - f, now_ms); // hidden, not recorded yet
                    self.recorded(changes);
                    self.cut = Some(text);
                    self.positions.edit(f, t - f, "");
                }
            }
            Op::Move { from, to, at } => {
                let (f, t) = self.chars(from, to)?;
                let a = self.positions.to_char(at).map_err(err)?;
                self.authored();
                self.close_burst(now_ms);
                if f < t && (a < f || a > t) {
                    let change = self.doc.move_local(f, t - f, a, now_ms);
                    self.recorded([change]);
                    self.positions = Positions::new(&self.doc.text());
                }
            }
            Op::Undo => self.undo(now_ms),
            Op::Redo => self.redo(now_ms),
            Op::Sync => self.take_in(now_ms)?,
        }
        Ok(())
    }

    /// The editor's own undo inside the open burst. Typed text is a draft, so its steps are just
    /// edits to the draft; undoing a deletion puts back everything it hid. A burst undone back to
    /// where it started is dropped (and the editor keeps its redo steps).
    fn editor_undo(&mut self, f: usize, t: usize, text: &str, now_ms: u64) -> Result<(), String> {
        match self.doc.burst().map(|b| b.kind) {
            Some(BurstKind::Typing) => {
                let changes = match (f == t, text.is_empty()) {
                    (_, true) => self.doc.delete_text(f, t - f, now_ms),
                    (true, false) => self.doc.type_text(f, text, now_ms),
                    (false, false) => self.doc.replace_text(f, t - f, text, now_ms),
                };
                self.recorded(changes);
                if self.doc.burst_changes_nothing() {
                    self.doc.cancel_burst(); // back to where it started
                }
            }
            // Undoing a deletion (or a cut): all of it comes back.
            Some(BurstKind::Deleting) => {
                self.cut = None;
                self.doc.cancel_burst();
            }
            // The burst closed in the meantime: it's a Change now, so undo means reverting it.
            None => self.undo(now_ms),
        }
        Ok(())
    }

    /// Ctrl+Z beyond the editor's own undo: drops the open burst (or waiting cut) if there is
    /// one, otherwise reverts this sitting's latest Change still in effect.
    fn undo(&mut self, now_ms: u64) {
        if self.doc.burst().is_some() {
            self.cut = None;
            self.doc.cancel_burst();
            return;
        }
        let latest = self.sitting.iter().rev().copied().find(|&id| self.doc.is_in_effect(id));
        if let Some(target) = latest {
            let change = self.doc.revert_local(target, true, now_ms);
            self.unsaved.push(change);
            self.redo.push(target);
        }
    }

    /// Ctrl+Shift+Z: restores the Change undo reverted last.
    fn redo(&mut self, now_ms: u64) {
        self.close_burst(now_ms);
        while let Some(target) = self.redo.pop() {
            if let Some(change) = self.doc.restore_local(target, now_ms) {
                self.unsaved.push(change);
                return;
            }
        }
    }

    /// Saves Changes not saved yet. If that fails they stay here and are tried again.
    pub fn save(&mut self) {
        if self.unsaved.is_empty() || self.is_lost() {
            return; // (lost: the recovery file keeps them until the History is found again)
        }
        match storage::save(&self.db, &self.unsaved, &self.doc) {
            Ok(()) => {
                if self.identity.is_none() {
                    // An old Work with no Changes until now: known by its first one from now on.
                    self.identity = storage::identity(&self.db).ok().flatten();
                }
                self.mine.append(&mut self.unsaved);
                self.save_error = None;
                self.text_file_behind = true;
                self.db_stamp = stamp(&self.db); // our own write: not a change from outside
            }
            Err(e) if e.downcast_ref::<storage::SameIdDifferentContent>().is_some() => self.diverged(),
            Err(e) => self.save_error = Some(format!("Couldn't save to {}: {e}. Your writing is kept and saving is tried again.", file_name(&self.db))),
        }
    }

    /// The timer: closes a burst after a pause, retries saving, and writes the Text file when due.
    pub fn tick(&mut self, now_ms: u64) {
        if self.cut.is_none() { // a cut waits for its paste, however long that takes
            let change = self.doc.tick(now_ms);
            self.recorded(change);
        }
        self.save();
        self.keep_recovery(true);
        if now_ms.saturating_sub(self.text_file_written_ms) >= TEXT_FILE_EVERY_MS {
            self.write_text_file(now_ms);
        }
        self.look_outside();
    }

    /// Has anything changed outside the app: the `.db` or Text file written by someone else
    /// (Syncthing, another editor), or a new Conflict copy? Then `outside` goes up, and the
    /// editor sends a `Sync` to take it in.
    fn look_outside(&mut self) {
        let copies = conflict_copies(&self.db).into_iter().chain(conflict_copies(&self.typ));
        let new_copy = copies.into_iter().any(|copy| !self.copies_tried.contains(&copy));
        let typ_ready = self.typ_waiting && self.doc.burst().is_none();
        if stamp(&self.db) != self.db_stamp || stamp(&self.typ) != self.typ_stamp || new_copy || typ_ready {
            self.outside += 1;
            // Noted until the `Sync` reads them, so this counts once.
            self.db_stamp = stamp(&self.db);
            self.typ_stamp = stamp(&self.typ);
            self.typ_waiting = false;
            self.copies_tried.extend(conflict_copies(&self.db).into_iter().chain(conflict_copies(&self.typ)));
        }
    }

    /// Takes in what changed outside the app, in order with the editor's edits (`Op::Sync`).
    fn take_in(&mut self, now_ms: u64) -> Result<(), String> {
        if self.is_lost() {
            return Ok(()); // the Vault looks for where it went
        }
        let mut from: BTreeSet<wrtier::ids::DeviceId> = BTreeSet::new();
        let mut problems: Vec<String> = Vec::new();
        // Conflict copies of the History merge in first (that works with a burst open).
        for copy in conflict_copies(&self.db) {
            match storage::merge(&self.db, &copy) {
                Ok(()) => { let _ = std::fs::remove_file(&copy); }
                Err(e) => problems.push(format!("{} was kept: {e}", file_name(&copy))),
            }
        }
        // Then the History itself: whatever it has that this Work doesn't yet. Unless it has
        // this Device's own IDs for different Changes: then it's a copy that went its own way,
        // and taking anything from it would mix the two.
        let in_file = storage::read_all(&self.db).map_err(err)?;
        let mine: HashMap<ChangeId, wrtier::ids::Hlc> = self.mine.iter().map(|c| (c.id, c.hlc)).collect();
        if in_file.iter().any(|c| mine.get(&c.id).is_some_and(|&hlc| hlc != c.hlc)) {
            self.diverged();
            return Ok(());
        }
        let ids: HashSet<ChangeId> = in_file.iter().map(|c| c.id).collect();
        let new: Vec<Change> = in_file.into_iter().filter(|c| !self.doc.has(c.id)).collect();
        from.extend(new.iter().map(|c| c.id.device).filter(|&d| d != self.doc.device()));
        self.doc.apply_all(&new);
        // Changes this Device saved must stay saved, even if the `.db` that arrived lacks them.
        let missing: Vec<Change> = self.mine.iter().filter(|c| !ids.contains(&c.id)).cloned().collect();
        if !missing.is_empty() {
            match storage::save(&self.db, &missing, &self.doc) {
                Ok(()) => {}
                Err(e) if e.downcast_ref::<storage::SameIdDifferentContent>().is_some() => {
                    self.diverged();
                    return Ok(());
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        if !new.is_empty() {
            self.text_file_behind = true;
        }
        // The Text file and its copies. An edit there is recorded at once, which can't happen
        // while a burst is open: it waits (a few seconds at most).
        let mut edited = false;
        if self.doc.burst().is_some() {
            self.typ_waiting = true;
        } else {
            let texts = conflict_copies(&self.typ).into_iter().chain(self.typ.exists().then(|| self.typ.clone()));
            for path in texts.collect::<Vec<_>>() {
                let read = storage::read_text_file(&self.db, &path, &mut self.doc, now_ms);
                let is_copy = path != self.typ;
                if !is_copy {
                    self.typ_newer = matches!(read, Ok(storage::TextFileRead::NotYet));
                }
                match read {
                    Ok(storage::TextFileRead::Imported(changes)) => {
                        edited = true;
                        self.mine.extend(changes);
                        self.text_file_behind = true; // written again, with its marker line
                        if is_copy { let _ = std::fs::remove_file(&path); }
                    }
                    Ok(storage::TextFileRead::Known) => if is_copy { let _ = std::fs::remove_file(&path); },
                    Ok(storage::TextFileRead::NotYet) => {} // its History hasn't arrived yet: read again when it does
                    Err(e) => problems.push(format!("{} couldn't be read: {e}", file_name(&path))),
                }
            }
        }
        self.db_stamp = stamp(&self.db);
        self.typ_stamp = stamp(&self.typ);
        self.copies_tried = conflict_copies(&self.db).into_iter().chain(conflict_copies(&self.typ)).collect();
        self.sync_problem = (!problems.is_empty()).then(|| problems.join(" "));
        let mut lines = Vec::new();
        if !from.is_empty() {
            let names: Vec<String> = from.iter().map(|&d| self.doc.device_name(d).map(str::to_string).unwrap_or_else(|| d.to_string())).collect();
            lines.push(format!("Took in changes from {}.", names.join(", ")));
        }
        if edited {
            lines.push(format!("Took in an edit made to {} in another program.", file_name(&self.typ)));
        }
        self.took_in = (!lines.is_empty()).then(|| lines.join(" "));
        Ok(())
    }

    /// The Author left the Work (another Work, the window lost focus): save everything and bring
    /// the Text file up to date, so it's current for any other program. A waiting cut stays.
    pub fn leave(&mut self, now_ms: u64) {
        if self.cut.is_none() {
            self.close_burst(now_ms);
        }
        self.save();
        self.keep_recovery(true);
        self.write_text_file(now_ms);
    }

    /// Closing the Work or quitting: nothing stays open. (If saving fails, the recovery file
    /// keeps the Changes, and they're saved when the Vault is opened again.)
    pub fn close(&mut self, now_ms: u64) {
        self.close_burst(now_ms);
        self.save();
        self.keep_recovery(true);
        self.write_text_file(now_ms);
    }

    /// Writes the Text file if it lags behind. Never over a file this Work hasn't read yet (an
    /// edit made in another program, taken in at the next `Sync`), or one written from Changes
    /// that haven't arrived yet (it's newer).
    fn write_text_file(&mut self, now_ms: u64) {
        if !self.text_file_behind || !self.unsaved.is_empty() || self.typ_newer || stamp(&self.typ) != self.typ_stamp || self.is_lost() {
            return;
        }
        match storage::write_text_file(&self.db, &self.typ, &self.doc, now_ms) {
            Ok(()) => {
                self.text_file_behind = false;
                self.text_file_written_ms = now_ms;
                self.text_file_problem = None;
                // Our own writes (the file, and its record in the `.db`): not changes from outside.
                self.typ_stamp = stamp(&self.typ);
                self.db_stamp = stamp(&self.db);
            }
            Err(e) => self.text_file_problem = Some(format!("Couldn't write {}: {e}", file_name(&self.typ))),
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrtier::ids::DeviceId;

    /// A Work with `text` saved in a temporary folder, as the app opens it.
    fn work(text: &str) -> (tempfile::TempDir, OpenWork) {
        let dir = tempfile::tempdir().unwrap();
        let (db, typ) = (dir.path().join("w.db"), dir.path().join("w.typ"));
        let mut doc = Doc::new(DeviceId(1));
        if !text.is_empty() {
            storage::save(&db, &[doc.insert_local(0, text, 100)], &doc).unwrap();
        } else {
            storage::save(&db, &[], &doc).unwrap();
        }
        storage::write_text_file(&db, &typ, &doc, 100).unwrap();
        let (_, doc) = storage::load(&db, DeviceId(1)).unwrap();
        let recovery = Recovery::for_work(&dir.path().join("recovery"), dir.path(), "w");
        let mine = storage::read_all(&db).unwrap();
        (dir, OpenWork::new(db, typ, doc, 100, recovery, Vec::new(), mine))
    }

    /// A Text file's text, without its marker line.
    fn text_of(path: &Path) -> String {
        wrtier::text_file::split_marker(&std::fs::read_to_string(path).unwrap()).0
    }

    /// Another Device (the phone) adds `text` at `at` straight into the Work's `.db`, as if
    /// Syncthing had delivered the phone's copy.
    fn phone_types(w: &OpenWork, at: usize, text: &str) {
        let (_, mut phone) = storage::load(&w.db, DeviceId(2)).unwrap();
        let mut changes = vec![phone.set_device_name_local("Phone", 5000)];
        changes.push(phone.insert_local(at, text, 5000));
        storage::save(&w.db, &changes, &phone).unwrap();
    }

    /// The timer noticed something; the editor sends a `Sync`, as it does when `outside` goes up.
    fn notice_and_sync(w: &mut OpenWork, now_ms: u64) -> WorkState {
        let before = w.state(Vec::new()).outside;
        w.tick(now_ms);
        assert!(w.state(Vec::new()).outside > before, "the change was noticed");
        w.edit(vec![Op::Sync], None, now_ms + 1).unwrap()
    }

    #[test]
    fn another_devices_changes_arrive_without_disturbing_the_open_burst() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "X")], None, 1000).unwrap(); // a burst is open
        phone_types(&w, 0, "P");
        let state = notice_and_sync(&mut w, 2000);
        assert_eq!(w.text(), "PabcX");
        assert_eq!(editor_after("abcX", &state), "PabcX", "the editor gets the phone's text as a patch");
        assert!(state.burst.is_some(), "and the burst stays open");
        assert_eq!(state.took_in.as_deref(), Some("Took in changes from Phone."));
        w.close(9000);
        assert_eq!(saved(&w), vec!["insert", "insert", "insert"]);
    }

    #[test]
    fn an_edit_in_vim_is_taken_in_while_the_work_is_open() {
        let (_dir, mut w) = work("The rain had stopped.");
        std::fs::write(&w.typ, "The rain had eased.").unwrap(); // vim, without the marker line
        let state = notice_and_sync(&mut w, 1000);
        assert_eq!(w.text(), "The rain had eased.");
        assert_eq!(editor_after("The rain had stopped.", &state), "The rain had eased.");
        assert!(state.took_in.unwrap().contains("another program"));
        w.leave(2000);
        assert_eq!(text_of(&w.typ), "The rain had eased.", "written again, with its marker");
        assert!(std::fs::read_to_string(&w.typ).unwrap().contains("// Wrtier version:"));
    }

    #[test]
    fn an_edit_in_vim_waits_for_the_open_burst() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "X")], None, 1000).unwrap();
        std::fs::write(&w.typ, "abc!").unwrap();
        notice_and_sync(&mut w, 2000);
        assert_eq!(w.text(), "abcX", "not yet: the burst is open");
        w.tick(9000); // the burst closes, and the waiting edit counts as a change again
        w.edit(vec![Op::Sync], None, 9001).unwrap();
        assert!(w.text().contains('X') && w.text().contains('!'), "{}", w.text());
    }

    #[test]
    fn a_db_that_lacks_this_devices_changes_gets_them_again() {
        let (dir, mut w) = work("abc");
        let older = dir.path().join("older.db");
        std::fs::copy(&w.db, &older).unwrap();
        w.edit(vec![typing(3, "d")], None, 1000).unwrap();
        w.tick(8000); // "d" saved
        std::fs::copy(&older, &w.db).unwrap(); // Syncthing puts back a copy from before it
        notice_and_sync(&mut w, 9000);
        assert_eq!(storage::load(&w.db, DeviceId(1)).unwrap().1.text(), "abcd");
    }

    #[test]
    fn a_conflict_copy_is_taken_in_while_the_work_is_open() {
        let (dir, mut w) = work("abc");
        let copy = dir.path().join("w.sync-conflict-20261106-120000-ABCDEFG.db");
        std::fs::copy(&w.db, &copy).unwrap();
        let (_, mut phone) = storage::load(&copy, DeviceId(2)).unwrap();
        storage::save(&copy, &[phone.insert_local(3, "!", 5000)], &phone).unwrap();
        notice_and_sync(&mut w, 6000);
        assert_eq!(w.text(), "abc!");
        assert!(!copy.exists(), "merged and deleted");
    }

    fn typing(from: usize, text: &str) -> Op {
        Op::Type { from, to: from, text: text.into(), origin: Origin::Input }
    }

    /// The kinds of Change saved in the `.db`, in order, without Device names.
    fn saved(work: &OpenWork) -> Vec<&'static str> {
        storage::read_all(&work.db).unwrap().iter().filter_map(|c| match c.kind {
            ChangeKind::Insert(_) => Some("insert"),
            ChangeKind::Delete(_) => Some("delete"),
            ChangeKind::Replace { .. } => Some("replace"),
            ChangeKind::Move { .. } => Some("move"),
            ChangeKind::Revert { .. } => Some("revert"),
            _ => None,
        }).collect()
    }

    /// What the editor would show after applying `state`'s patches to `shown`.
    fn editor_after(shown: &str, state: &WorkState) -> String {
        patch::apply(shown, &state.patches)
    }

    #[test]
    fn typing_is_saved_when_the_burst_closes() {
        let (_dir, mut w) = work("");
        let state = w.edit(vec![typing(0, "Hello"), typing(5, " world")], Some("Laptop"), 1000).unwrap();
        assert_eq!(state.len, 11);
        assert!(state.burst.is_some());
        assert!(saved(&w).is_empty(), "nothing recorded while the burst is open");
        w.tick(3000);
        assert!(saved(&w).is_empty());
        w.tick(7000); // a pause of more than 5 s
        assert_eq!(saved(&w), vec!["insert"]);
        let (_, doc) = storage::load(&w.db, DeviceId(1)).unwrap();
        assert_eq!(doc.text(), "Hello world");
        assert_eq!(doc.device_name(DeviceId(1)), Some("Laptop"), "named before the first edit");
    }

    #[test]
    fn positions_from_the_editor_are_utf16() {
        let (_dir, mut w) = work("a😊b");
        let state = w.edit(vec![typing(3, "X")], None, 1000).unwrap(); // after the emoji
        assert_eq!(w.text(), "a😊Xb");
        assert_eq!(state.len, 5);
        assert!(w.edit(vec![typing(2, "!")], None, 1100).is_err(), "inside the emoji");
        assert!(w.edit(vec![typing(99, "!")], None, 1200).is_err(), "past the end");
        assert_eq!(w.text(), "a😊Xb", "nothing happened");
    }

    #[test]
    fn a_paste_is_its_own_change_at_once() {
        let (_dir, mut w) = work("ab");
        w.edit(vec![typing(1, "x"), Op::Paste { from: 3, to: 3, text: "PASTED".into() }], None, 1000).unwrap();
        assert_eq!(w.text(), "axbPASTED");
        assert_eq!(saved(&w), vec!["insert", "insert", "insert"], "the open burst closed first, then the paste");
    }

    #[test]
    fn cutting_and_pasting_the_same_text_is_a_move() {
        let (_dir, mut w) = work("One. Two. Three.");
        w.edit(vec![Op::Cut { from: 0, to: 5 }], None, 1000).unwrap();
        assert_eq!(w.text(), "Two. Three.");
        w.edit(vec![Op::Cursor { pos: 11 }], None, 20_000).unwrap(); // look around for a while
        w.tick(30_000);
        assert_eq!(saved(&w), vec!["insert"], "the cut waits for its paste");
        let state = w.edit(vec![Op::Paste { from: 11, to: 11, text: "One. ".into() }], None, 31_000).unwrap();
        assert_eq!(w.text(), "Two. Three.One. ");
        assert!(state.patches.is_empty(), "the editor already shows it");
        assert_eq!(saved(&w), vec!["insert", "move"]);
    }

    #[test]
    fn a_cut_followed_by_typing_is_a_deletion() {
        let (_dir, mut w) = work("One. Two.");
        w.edit(vec![Op::Cut { from: 0, to: 5 }, typing(0, "X")], None, 1000).unwrap();
        w.close(2000);
        assert_eq!(w.text(), "XTwo.");
        assert_eq!(saved(&w), vec!["insert", "delete", "insert"]);
    }

    #[test]
    fn dragging_a_selection_is_a_move() {
        let (_dir, mut w) = work("abcdef");
        w.edit(vec![Op::Move { from: 0, to: 2, at: 6 }], None, 1000).unwrap();
        assert_eq!(w.text(), "cdefab");
        assert_eq!(saved(&w), vec!["insert", "move"]);
    }

    #[test]
    fn undo_inside_the_burst_edits_the_draft() {
        let (_dir, mut w) = work("");
        w.edit(vec![typing(0, "Hello"), typing(5, " world")], None, 1000).unwrap();
        // The editor undoes " world" itself and tells us.
        let state = w.edit(vec![Op::Delete { from: 5, to: 11, origin: Origin::Undo }], None, 1100).unwrap();
        assert!(state.patches.is_empty());
        w.close(2000);
        assert_eq!(w.text(), "Hello");
        assert_eq!(saved(&w), vec!["insert"], "the undone typing was never recorded");
    }

    #[test]
    fn undo_beyond_the_burst_reverts_the_latest_change_and_redo_restores_it() {
        let (_dir, mut w) = work("The rain.");
        w.edit(vec![typing(8, " had stopped")], None, 1000).unwrap();
        w.tick(10_000); // closed and saved
        assert_eq!(w.text(), "The rain had stopped.");
        let state = w.edit(vec![Op::Undo], None, 11_000).unwrap();
        assert_eq!(w.text(), "The rain.");
        assert_eq!(editor_after("The rain had stopped.", &state), "The rain.");
        let state = w.edit(vec![Op::Redo], None, 12_000).unwrap();
        assert_eq!(editor_after("The rain.", &state), "The rain had stopped.");
        assert_eq!(saved(&w), vec!["insert", "insert", "revert", "revert"]);
        // Undo only goes back through this sitting: the text from before stays.
        w.edit(vec![Op::Undo], None, 13_000).unwrap();
        w.edit(vec![Op::Undo], None, 14_000).unwrap();
        assert_eq!(w.text(), "The rain.");
    }

    #[test]
    fn undo_with_a_burst_open_drops_the_burst() {
        let (_dir, mut w) = work("abc");
        let state = w.edit(vec![typing(3, "def"), Op::Undo], None, 1000).unwrap();
        assert_eq!(w.text(), "abc");
        assert_eq!(editor_after("abcdef", &state), "abc");
        assert!(state.burst.is_none());
    }

    #[test]
    fn undoing_a_cut_puts_it_back() {
        let (_dir, mut w) = work("One. Two.");
        w.edit(vec![Op::Cut { from: 0, to: 5 }], None, 1000).unwrap();
        // The editor's undo re-inserts the cut text.
        let state = w.edit(vec![Op::Type { from: 0, to: 0, text: "One. ".into(), origin: Origin::Undo }], None, 1100).unwrap();
        assert_eq!(w.text(), "One. Two.");
        assert!(state.patches.is_empty());
        w.close(2000);
        assert_eq!(saved(&w), vec!["insert"], "nothing recorded");
    }

    #[test]
    fn redo_after_undoing_a_whole_draft_types_it_again() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "def")], None, 1000).unwrap();
        let state = w.edit(vec![Op::Delete { from: 3, to: 6, origin: Origin::Undo }], None, 1100).unwrap();
        assert_eq!(w.text(), "abc");
        assert!(state.burst.is_none(), "the empty burst was dropped");
        let state = w.edit(vec![Op::Type { from: 3, to: 3, text: "def".into(), origin: Origin::Redo }], None, 1200).unwrap();
        assert_eq!(w.text(), "abcdef");
        assert!(state.patches.is_empty(), "the editor already shows it");
        w.close(2000);
        assert_eq!(saved(&w), vec!["insert", "insert"], "recorded once, as typing");
    }

    #[test]
    fn redo_after_undoing_a_deletion_deletes_again() {
        let (_dir, mut w) = work("abcdef");
        w.edit(vec![Op::Delete { from: 3, to: 6, origin: Origin::Input }], None, 1000).unwrap();
        w.edit(vec![Op::Type { from: 3, to: 3, text: "def".into(), origin: Origin::Undo }], None, 1100).unwrap();
        assert_eq!(w.text(), "abcdef");
        let state = w.edit(vec![Op::Delete { from: 3, to: 6, origin: Origin::Redo }], None, 1200).unwrap();
        assert_eq!(w.text(), "abc");
        assert!(state.patches.is_empty());
        w.close(2000);
        assert_eq!(saved(&w), vec!["insert", "delete"]);
    }

    #[test]
    fn the_editors_undo_after_the_burst_closed_becomes_a_revert() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "def")], None, 1000).unwrap();
        w.tick(10_000); // the timer closed it before the editor's undo arrived
        let state = w.edit(vec![Op::Delete { from: 3, to: 6, origin: Origin::Undo }], None, 10_100).unwrap();
        assert_eq!(w.text(), "abc");
        assert_eq!(editor_after("abc", &state), "abc");
        assert_eq!(saved(&w), vec!["insert", "insert", "revert"]);
    }

    #[test]
    fn the_open_burst_is_kept_until_it_is_saved() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "def")], None, 1000).unwrap();
        assert!(w.recovery.json.exists(), "kept from the first keystroke");
        w.tick(10_000); // closed and saved
        assert!(!w.recovery.json.exists(), "nothing left to keep");
    }

    #[test]
    fn the_text_file_follows_on_leaving_and_every_30_seconds() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "d")], None, 1000).unwrap();
        w.tick(7000); // saved, but the Text file waits
        assert_eq!(text_of(&w.typ), "abc");
        w.tick(31_000);
        assert_eq!(text_of(&w.typ), "abcd");
        w.edit(vec![typing(4, "e")], None, 32_000).unwrap();
        w.leave(33_000);
        assert_eq!(text_of(&w.typ), "abcde");
    }

    #[test]
    fn a_text_file_edited_outside_is_not_overwritten_before_it_is_taken_in() {
        let (_dir, mut w) = work("abc");
        std::fs::write(&w.typ, "edited in vim").unwrap();
        w.edit(vec![typing(3, "d")], None, 1000).unwrap();
        w.leave(2000);
        assert_eq!(std::fs::read_to_string(&w.typ).unwrap(), "edited in vim");
    }
}
