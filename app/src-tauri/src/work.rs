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
use std::path::{Path, PathBuf};
use wrtier::change::{Change, ChangeKind};
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
            Op::Cursor { .. } | Op::Undo | Op::Redo => text.to_string(),
        }
    }

    /// Can the History's text end up different from what the editor shows? Then the editor gets
    /// patches, and the editor never sends anything after such an op in the same batch.
    fn may_differ(&self, cut_waiting: bool) -> bool {
        match self {
            Op::Undo | Op::Redo => true,
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
    pub fn new(db: PathBuf, typ: PathBuf, doc: Doc, now_ms: u64, recovery: Recovery, sitting: Vec<ChangeId>) -> Self {
        let positions = Positions::new(&doc.text());
        OpenWork {
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
        }
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
        if ops.iter().any(|op| !matches!(op, Op::Cursor { .. })) {
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
        result.map(|()| self.state(patches))
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
        if self.unsaved.is_empty() {
            return;
        }
        match storage::save(&self.db, &self.unsaved, &self.doc) {
            Ok(()) => {
                self.unsaved.clear();
                self.save_error = None;
                self.text_file_behind = true;
            }
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

    /// Writes the Text file if it lags behind, unless it was edited outside the app since the
    /// app last wrote it: then it's left alone, so that edit isn't lost.
    fn write_text_file(&mut self, now_ms: u64) {
        if !self.text_file_behind || !self.unsaved.is_empty() {
            return;
        }
        if let Ok(current) = std::fs::read_to_string(&self.typ) {
            // If that can't be checked, assume it was edited: never overwrite an edit made elsewhere.
            if !storage::wrote_text(&self.db, &current).unwrap_or(false) {
                self.text_file_problem = Some(format!(
                    "{} was edited outside the app while the Work was open. It's left as it is, and the edit is taken in when the Work is opened again.",
                    file_name(&self.typ)));
                return;
            }
        }
        match storage::write_text_file(&self.db, &self.typ, &self.doc, now_ms) {
            Ok(()) => {
                self.text_file_behind = false;
                self.text_file_written_ms = now_ms;
                self.text_file_problem = None;
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
        (dir, OpenWork::new(db, typ, doc, 100, recovery, Vec::new()))
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
        assert_eq!(std::fs::read_to_string(&w.typ).unwrap(), "abc");
        w.tick(31_000);
        assert_eq!(std::fs::read_to_string(&w.typ).unwrap(), "abcd");
        w.edit(vec![typing(4, "e")], None, 32_000).unwrap();
        w.leave(33_000);
        assert_eq!(std::fs::read_to_string(&w.typ).unwrap(), "abcde");
    }

    #[test]
    fn a_text_file_edited_outside_is_not_overwritten() {
        let (_dir, mut w) = work("abc");
        std::fs::write(&w.typ, "edited in vim").unwrap();
        w.edit(vec![typing(3, "d")], None, 1000).unwrap();
        w.leave(2000);
        assert_eq!(std::fs::read_to_string(&w.typ).unwrap(), "edited in vim");
        assert!(w.state(Vec::new()).text_file_problem.is_some());
    }
}
