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
use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use wrtier::change::{Change, ChangeKind, NoticeKey};
use wrtier::conflicts::conflict_copies;
use wrtier::doc::{BurstKind, CutPasted, Doc, MergeNotice, Stretch};
use wrtier::ids::{ChangeId, CharId, DeviceId};
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
    /// Send the Comments' highlights with the answer (`WorkState::marks`). Asked through the
    /// queue, so their positions fit the editor's text the way patches do.
    Marks,
    /// "Colour text by Session" is on: send the newest Sessions' text with the answer
    /// (`WorkState::tints`), through the queue like `Marks`. Never sent while it's off.
    Tints,
    // Comments. Each closes the open burst first (nothing else is recorded while one is open).
    // `sender` is `author` or `reviewer/<id>`; the Vault fills in `sender_name`, the Reviewer's
    // current name, which the Work records just before (so its `.db` reads on its own).
    /// A new Comment on `from..to`.
    Comment { from: usize, to: usize, sender: String, body: String, #[serde(skip)] sender_name: Option<String> },
    Reply { comment: String, sender: String, body: String, #[serde(skip)] sender_name: Option<String> },
    /// A new body for a Comment or Reply.
    EditComment { id: String, body: String },
    SetCommentSender { id: String, sender: String, #[serde(skip)] sender_name: Option<String> },
    Resolve { id: String, resolved: bool },
    /// Deletes a Comment (with its Replies) or a Reply: a Revert, so it can be restored.
    DeleteComment { id: String },
    // The History view's actions. Each closes the open burst first, and each is one Revert that
    // Ctrl+Z can take back, like typing.
    /// Reverts these Changes (a Session, a snippet, a Preamble line), with their dependents if
    /// asked; only those still in effect that change the content are taken.
    Revert { changes: Vec<String>, dependents: bool },
    /// Brings these Changes back: reverts every Revert of them still in effect, whole.
    Restore { changes: Vec<String> },
    /// Returns the content to a Milestone, except the Changes in `keep`.
    ReturnTo { milestone: String, keep: Vec<String> },
    /// Where some characters are now (a History snippet was clicked): answered in
    /// `WorkState::located`, in the editor's text, through the queue so it fits like patches.
    Locate { first: String, last: Option<String> },
    /// "History of this text": the History tab narrows to `from..to` (or, empty, the paragraph
    /// the cursor is in). The open burst closes first. Answered in `WorkState::stretch`.
    Stretch { from: usize, to: usize },
    /// The Inclusion character the editor just pasted at `at` includes `work` (its ID in the
    /// Vault), or the Chapter starting at `chapter` in its text (UTF-16) (ADR 0015). The Vault
    /// fills in `target`, the record it gets, since only it can find the Work.
    Include { at: usize, work: String, chapter: Option<usize>, #[serde(skip)] target: Option<crate::vault::Target> },
    /// Find and replace: the words that differ between the matches and their replacements
    /// (`edits`, in the text as it was before any of them, not overlapping), as one action that
    /// Ctrl+Z takes back whole. `find`, `with` and `count` say what it was, for the History view.
    /// The open burst closes first.
    Replace { edits: Vec<ReplaceOpEdit>, find: String, with: String, count: usize },
}

/// One word-level edit of an `Op::Replace`: `from..to` (UTF-16) becomes `text`.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ReplaceOpEdit {
    pub from: usize,
    pub to: usize,
    pub text: String,
}

impl Op {
    /// The Reviewer a Comment op names, for the Vault to fill in their name.
    pub fn sender_mut(&mut self) -> Option<(&str, &mut Option<String>)> {
        match self {
            Op::Comment { sender, sender_name, .. } | Op::Reply { sender, sender_name, .. } | Op::SetCommentSender { sender, sender_name, .. } => {
                Some((sender.as_str(), sender_name))
            }
            _ => None,
        }
    }
}

/// A Reply, as the Comments panel shows it.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReplyMark {
    pub id: String,
    pub sender: String,
    pub name: String,
    pub colour: String,
    pub body: String,
    pub time_ms: u64,
    /// The other version of its body, if two Devices edited it without seeing each other.
    pub other_body: Option<String>,
}

/// How many of the newest Sessions "Colour text by Session" tints.
pub const TINTED_SESSIONS: usize = 5;

/// The text one of the newest Sessions wrote, tinted in the editor.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tint {
    /// Where its text is: from..to in UTF-16, in order.
    pub ranges: Vec<(usize, usize)>,
    /// 0 for the newest Session (the strongest tint), then 1, 2...
    pub rank: usize,
    /// The Device's name as the app shows it, filled in by the Vault.
    pub device: String,
    #[serde(skip)]
    pub device_id: DeviceId,
    /// When the Session's last Change was made (ms since 1970).
    pub time_ms: u64,
}

/// A Comment's highlight in the editor.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub id: String,
    /// Where its text is: from..to in UTF-16, in reading order. Empty: it's detached.
    pub ranges: Vec<(usize, usize)>,
    /// `author` or `reviewer/<id>`.
    pub sender: String,
    /// Who sent it, as the app shows them, and their colour slot (or "author", or "unknown").
    /// Filled in by the Vault, which knows the Reviewers.
    pub name: String,
    pub colour: String,
    pub body: String,
    pub resolved: bool,
    /// The text it was attached to, as selected.
    pub quote: String,
    /// When it was made (ms since 1970).
    pub time_ms: u64,
    pub replies: Vec<ReplyMark>,
    /// The other version of its body, if two Devices edited it without seeing each other.
    pub other_body: Option<String>,
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
            // Undo, redo, a sync and the History view's actions may change the text (`may_differ`);
            // the rest never do.
            _ => text.to_string(),
        }
    }

    /// Can the History's text end up different from what the editor shows? Then the editor gets
    /// patches, and the editor never sends anything after such an op in the same batch.
    fn may_differ(&self, cut_waiting: bool) -> bool {
        match self {
            Op::Undo | Op::Redo | Op::Sync | Op::Revert { .. } | Op::Restore { .. } | Op::ReturnTo { .. } => true,
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
    /// A Style (or Look, or Story element) changed in the library since the Work recorded it.
    pub style_notice: Option<String>,
    /// How many Changes the History holds. When it goes up (a burst was recorded, something
    /// was taken in, a Move or Revert), the editor asks for the highlights again (`Op::Marks`).
    pub changes: usize,
    /// How many Merge notices the History tab shows (not dismissed): "History (n)".
    pub notices: usize,
    /// The Comments' highlights, in the answer to a batch with `Op::Marks`.
    pub marks: Option<Vec<Mark>>,
    /// The newest Sessions' text, in the answer to a batch with `Op::Tints`.
    pub tints: Option<Vec<Tint>>,
    /// Where an `Op::Locate`'s characters are (UTF-16, in the editor's text), in the answer to a
    /// batch with one; None in it if they aren't shown any more. Left out of other answers
    /// (so the interface can tell "not asked" from "not shown").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub located: Option<Option<(usize, usize)>>,
    /// The text an `Op::Stretch` picked, quoted for the History tab's bar, in the answer to a
    /// batch with one; None in it if there was no text there (an empty line).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stretch: Option<Option<String>>,
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
    /// The Style library as of which this Work last recorded what it uses (see `Vault`'s
    /// library generation): an edit after the library changed records the new versions first.
    styles_checked: Option<u64>,
    pub style_notice: Option<String>,
    /// The Vault's folder: the Text file's header falls back to its Style library for anything
    /// the Work hasn't recorded yet.
    pub vault_root: PathBuf,
    /// What the Text file writes out for each Inclusion (ADR 0015), kept up to date by the Vault,
    /// which can find the included Works.
    pub inclusions: storage::Expansions,
    /// The text "History of this text" is about, held by identity (see `Op::Stretch`).
    pub stretch: Option<Stretch>,
    /// The answer to an `Op::Stretch` in the batch being applied.
    stretch_answer: Option<Option<String>>,
    /// How many Merge notices the History tab shows, as of how many Changes the History held:
    /// they only change when a Change is made or taken in.
    notice_count: Cell<Option<(usize, usize)>>,
}

/// The Merge notices the History tab shows: all but a Comment body edited on two Devices,
/// which stays on its Comment's card.
pub fn history_notices(doc: &Doc) -> Vec<MergeNotice> {
    doc.merge_notices().into_iter()
        .filter(|n| !doc.concurrent_field(n.key).is_some_and(|(record, _, _)| record.starts_with("comment/")))
        .collect()
}

/// Names a Merge notice for the interface, the same way on every Device.
pub fn notice_key(key: NoticeKey) -> String {
    match key {
        NoticeKey::Overlap { later, earlier } => format!("overlap:{later}:{earlier}"),
        NoticeKey::Loop { skipped } => format!("loop:{skipped}"),
        NoticeKey::UnknownTextFile { session } => format!("unknown:{session}"),
        NoticeKey::ConcurrentField { later, earlier } => format!("field:{later}:{earlier}"),
    }
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
            styles_checked: None,
            style_notice: None,
            vault_root: PathBuf::new(),
            inclusions: storage::Expansions::new(),
            stretch: None,
            stretch_answer: None,
            notice_count: Cell::new(None),
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
            style_notice: self.style_notice.clone(),
            changes: self.doc.change_count(),
            notices: self.notice_count(),
            marks: None,
            tints: None,
            located: None,
            stretch: None,
        }
    }

    /// How many Merge notices the History tab shows, worked out again only after a Change.
    fn notice_count(&self) -> usize {
        let changes = self.doc.change_count();
        match self.notice_count.get() {
            Some((at, count)) if at == changes => count,
            _ => {
                let count = history_notices(&self.doc).len();
                self.notice_count.set(Some((changes, count)));
                count
            }
        }
    }

    /// Dismisses a Merge notice (named by `notice_key`): recorded as a Change, so it's gone on
    /// every Device. The open burst closes first. Not on the undo list: Ctrl+Z is for the text.
    pub fn dismiss_notice(&mut self, key: &str, now_ms: u64) -> Result<WorkState, String> {
        let notice = history_notices(&self.doc).into_iter().find(|n| notice_key(n.key) == key)
            .ok_or("That notice was already dismissed (perhaps on another Device).")?;
        self.close_burst(now_ms);
        let change = self.doc.dismiss_local(notice.key, now_ms);
        self.unsaved.push(change);
        self.save();
        self.keep_recovery(false);
        Ok(self.state(Vec::new()))
    }

    /// The Comments' highlights, in the text the editor shows (open burst included). Names and
    /// colours are left for the Vault to fill in.
    pub fn marks(&self) -> Vec<Mark> {
        let comments = self.doc.comments();
        if comments.is_empty() {
            return Vec::new();
        }
        // Bodies edited on two Devices without seeing each other: the version that lost, by Comment or Reply.
        let mut other: HashMap<String, String> = HashMap::new();
        for notice in self.doc.merge_notices() {
            if let Some((record, field, lost)) = self.doc.concurrent_field(notice.key) {
                if let (Some(id), "body") = (record.strip_prefix("comment/"), field.as_str()) {
                    other.insert(id.to_string(), lost);
                }
            }
        }
        comments.into_iter().map(|c| {
            let id = c.id.to_string();
            Mark {
                ranges: c.ranges.iter().map(|&(from, to)| (self.positions.to_utf16(from), self.positions.to_utf16(to))).collect(),
                sender: c.sender,
                name: String::new(),
                colour: String::new(),
                body: c.body,
                resolved: c.resolved,
                quote: c.quote,
                time_ms: c.hlc.0,
                replies: c.replies.into_iter().map(|r| {
                    let id = r.id.to_string();
                    ReplyMark { other_body: other.get(&id).cloned(), id, sender: r.sender, name: String::new(), colour: String::new(), body: r.body, time_ms: r.hlc.0 }
                }).collect(),
                other_body: other.get(&id).cloned(),
                id,
            }
        }).collect()
    }

    /// The newest Sessions' text, in the text the editor shows (open burst included). Device
    /// names are left for the Vault to fill in.
    pub fn tints(&self) -> Vec<Tint> {
        self.doc.newest_sessions_text(TINTED_SESSIONS).into_iter().enumerate().map(|(rank, s)| Tint {
            ranges: s.ranges.iter().map(|&(from, to)| (self.positions.to_utf16(from), self.positions.to_utf16(to))).collect(),
            rank,
            device: String::new(),
            device_id: s.device,
            time_ms: s.end.0,
        }).collect()
    }

    /// A Comment's or Reply's ID from the interface, if it's one the Work has (and not deleted).
    fn comment_id(&self, id: &str, reply_ok: bool) -> Result<ChangeId, String> {
        let parsed: ChangeId = id.parse().map_err(|_| format!("Not a Comment: {id}"))?;
        let found = self.doc.comments().iter().any(|c| c.id == parsed || (reply_ok && c.replies.iter().any(|r| r.id == parsed)));
        if found { Ok(parsed) } else { Err("That Comment was deleted (perhaps on another Device).".into()) }
    }

    /// Before a Comment or Reply from a Reviewer is recorded: the Work records their current
    /// name, if it has none for them or an older one.
    fn record_sender(&mut self, sender: &str, name: Option<&str>, now_ms: u64) {
        let Some(name) = name else { return };
        if sender.starts_with("reviewer/") && self.doc.field(sender, "name") != Some(name) {
            let change = self.doc.set_field_local(sender, "name", name, now_ms);
            self.unsaved.push(change);
        }
    }

    /// The Comment ops: each closes the open burst (and a waiting cut) first.
    fn apply_comment_op(&mut self, op: Op, now_ms: u64) -> Result<(), String> {
        self.close_burst(now_ms);
        let change = match op {
            Op::Comment { from, to, sender, body, sender_name } => {
                let (f, t) = self.chars(from, to)?;
                if f == t {
                    return Err("A Comment needs some text to attach to.".into());
                }
                self.record_sender(&sender, sender_name.as_deref(), now_ms);
                self.doc.comment_local(f, t - f, &sender, &body, now_ms)
            }
            Op::Reply { comment, sender, body, sender_name } => {
                let comment = self.comment_id(&comment, false)?;
                self.record_sender(&sender, sender_name.as_deref(), now_ms);
                self.doc.reply_local(comment, &sender, &body, now_ms)
            }
            Op::EditComment { id, body } => {
                let id = self.comment_id(&id, true)?;
                self.doc.edit_comment_local(id, &body, now_ms)
            }
            Op::SetCommentSender { id, sender, sender_name } => {
                let id = self.comment_id(&id, true)?;
                self.record_sender(&sender, sender_name.as_deref(), now_ms);
                self.doc.set_comment_sender_local(id, &sender, now_ms)
            }
            Op::Resolve { id, resolved } => {
                let id = self.comment_id(&id, false)?;
                self.doc.resolve_comment_local(id, resolved, now_ms)
            }
            Op::DeleteComment { id } => {
                let id = self.comment_id(&id, true)?;
                self.doc.revert_local(id, false, now_ms)
            }
            _ => unreachable!("only Comment ops come here"),
        };
        self.unsaved.push(change);
        Ok(())
    }

    /// The History view's actions: each closes the open burst (and a waiting cut) first, and
    /// its Revert joins this sitting's undo list, so Ctrl+Z takes it back. (Deleting a Comment
    /// is a Revert too, but never on that list: undo leaves Comments alone.)
    fn apply_history_op(&mut self, op: Op, now_ms: u64) -> Result<(), String> {
        self.close_burst(now_ms);
        let ids = |list: &[String]| -> Result<Vec<ChangeId>, String> {
            list.iter().map(|id| id.parse().map_err(|_| format!("Not a Change: {id}"))).collect()
        };
        let change = match op {
            Op::Revert { changes, dependents } => self.doc.revert_changes_local(&ids(&changes)?, dependents, now_ms),
            Op::Restore { changes } => self.doc.restore_changes_local(&ids(&changes)?, now_ms),
            Op::ReturnTo { milestone, keep } => {
                let milestone: ChangeId = milestone.parse().map_err(|_| format!("Not a Milestone: {milestone}"))?;
                if !self.doc.milestones().iter().any(|m| m.id == milestone) {
                    return Err("That Milestone was deleted (perhaps on another Device).".into());
                }
                self.doc.return_to_milestone_local(milestone, &ids(&keep)?, now_ms)
            }
            _ => unreachable!("only the History view's ops come here"),
        };
        if let Some(change) = change {
            self.authored();
            self.sitting.push(change.id);
            self.unsaved.push(change);
        }
        Ok(())
    }

    /// A short quote of the text at `from..to` (characters), or of the paragraph at `from` if
    /// that's empty: for "History of “The rain had…”".
    fn quote_of(&self, from: usize, to: usize) -> String {
        let text: Vec<char> = self.doc.text().chars().collect();
        let (from, to) = if from < to {
            (from, to.min(text.len()))
        } else {
            let from = from.min(text.len());
            let start = text[..from].iter().rposition(|&c| c == '\n').map_or(0, |i| i + 1);
            let end = text[from..].iter().position(|&c| c == '\n').map_or(text.len(), |i| from + i);
            (start, end)
        };
        let quote: String = text[from..to].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
        if quote.chars().count() > 40 {
            format!("{}…", quote.chars().take(39).collect::<String>().trim_end())
        } else {
            quote
        }
    }

    /// Where some characters are in the editor's text (UTF-16), if they're shown.
    fn locate(&self, first: &str, last: Option<&str>) -> Option<(usize, usize)> {
        let first: CharId = first.parse().ok()?;
        let last: Option<CharId> = match last {
            Some(l) => Some(l.parse().ok()?),
            None => None,
        };
        let (from, to) = self.doc.shown_range(first, last)?;
        Some((self.positions.to_utf16(from), self.positions.to_utf16(to)))
    }

    /// Gives a Milestone a new name. The open burst closes first (nothing else may be recorded
    /// while one is open).
    pub fn rename_milestone(&mut self, milestone: ChangeId, name: &str, now_ms: u64) -> Result<WorkState, String> {
        if !self.doc.milestones().iter().any(|m| m.id == milestone) {
            return Err("That Milestone was deleted (perhaps on another Device).".into());
        }
        self.close_burst(now_ms);
        let change = self.doc.rename_milestone_local(milestone, name, now_ms);
        self.unsaved.push(change);
        self.save();
        self.keep_recovery(false);
        Ok(self.state(Vec::new()))
    }

    /// Deletes a Milestone (reverts it), or brings a deleted one back (restores it). Neither is
    /// on the undo list: Ctrl+Z is for the text.
    pub fn set_milestone_deleted(&mut self, milestone: ChangeId, deleted: bool, now_ms: u64) -> Result<WorkState, String> {
        if !self.doc.milestones_and_deleted().iter().any(|(m, _)| m.id == milestone) {
            return Err(format!("Not a Milestone of this Work: {milestone}"));
        }
        self.close_burst(now_ms);
        let change = if deleted {
            self.doc.is_in_effect(milestone).then(|| self.doc.revert_local(milestone, false, now_ms))
        } else {
            self.doc.restore_local(milestone, now_ms)
        };
        self.unsaved.extend(change);
        self.save();
        self.keep_recovery(false);
        Ok(self.state(Vec::new()))
    }

    /// Records what the app keeps up to date on its own (record, field, value), such as the
    /// versions of the Works it includes (ADR 0015). Only while no burst is open: nothing else
    /// may be recorded then. Returns whether it recorded them; saved with the next save.
    pub fn record_fields(&mut self, fields: &[(String, String, String)], now_ms: u64) -> bool {
        if self.doc.burst().is_some() || self.cut.is_some() {
            return false;
        }
        for (record, field, value) in fields {
            let change = self.doc.set_field_local(record, field, value, now_ms);
            self.unsaved.push(change);
        }
        true
    }

    /// Records the library files it uses that changed (or were never recorded) now, whether or
    /// not that's due: after it started using new ones. Only while no burst is open.
    pub fn record_library(&mut self, shelf: &crate::preamble::Shelf, generation: u64, now_ms: u64) {
        self.styles_checked = None;
        self.record_styles(Some((shelf, generation)), now_ms);
    }

    /// Does this Work need to record the Style library as of `generation` before an edit?
    pub fn styles_due(&self, generation: u64) -> bool {
        self.styles_checked != Some(generation)
    }

    /// Records the library files the Work uses that changed (or were never recorded), as of
    /// `generation`. Only while no burst is open: nothing else may be recorded then.
    fn record_styles(&mut self, library: Option<(&crate::preamble::Shelf, u64)>, now_ms: u64) {
        let Some((shelf, generation)) = library else { return };
        if self.doc.burst().is_some() || !self.styles_due(generation) {
            return;
        }
        self.styles_checked = Some(generation);
        let changes = crate::preamble::record_unrecorded(&mut self.doc, shelf, now_ms);
        self.unsaved.extend(changes);
        self.style_notice = None;
    }

    /// Before an Export: the open burst closes and changed Styles are recorded, so the Export is
    /// exactly a version in the History. (A cut waiting for its paste stays.)
    pub fn prepare_export(&mut self, shelf: &crate::preamble::Shelf, generation: u64, now_ms: u64) -> WorkState {
        if self.cut.is_none() {
            self.close_burst(now_ms);
        }
        self.styles_checked = None;
        self.record_styles(Some((shelf, generation)), now_ms);
        self.save();
        self.keep_recovery(false);
        self.state(Vec::new())
    }

    /// Names the current point in the History. The open burst closes, and changed Styles are
    /// recorded first, so the Milestone holds what the Page preview shows now.
    pub fn make_milestone(&mut self, name: &str, shelf: &crate::preamble::Shelf, generation: u64, now_ms: u64) -> WorkState {
        self.close_burst(now_ms);
        self.styles_checked = None;
        self.record_styles(Some((shelf, generation)), now_ms);
        let change = self.doc.make_milestone_local(name, now_ms);
        self.unsaved.push(change);
        self.save();
        self.keep_recovery(false);
        self.state(Vec::new())
    }

    /// The Author changed the Preamble (in the dialog): it's recorded at once, with the
    /// library files it now uses. An open burst closes first.
    pub fn set_preamble(&mut self, preamble: &crate::preamble::Preamble, shelf: &crate::preamble::Shelf, generation: u64, now_ms: u64) -> WorkState {
        self.close_burst(now_ms);
        let changes = preamble.record(&mut self.doc, now_ms);
        self.unsaved.extend(changes);
        self.styles_checked = None;
        self.record_styles(Some((shelf, generation)), now_ms);
        self.save();
        self.keep_recovery(false);
        self.state(Vec::new())
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
    /// `library`: the Style library and its generation, if it changed since this Work last
    /// recorded what it uses (see `styles_due`).
    pub fn edit(&mut self, ops: Vec<Op>, device_name: Option<&str>, library: Option<(&crate::preamble::Shelf, u64)>, now_ms: u64) -> Result<WorkState, String> {
        let want_marks = ops.contains(&Op::Marks);
        let want_tints = ops.contains(&Op::Tints);
        let locate = ops.iter().rev().find_map(|op| match op {
            Op::Locate { first, last } => Some((first.clone(), last.clone())),
            _ => None,
        });
        if ops.iter().any(|op| !matches!(op, Op::Cursor { .. } | Op::Sync | Op::Marks | Op::Tints | Op::Locate { .. } | Op::Stretch { .. })) {
            self.name_this_device(device_name, now_ms)?;
            self.record_styles(library, now_ms);
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
        let marks = want_marks.then(|| self.marks());
        let tints = want_tints.then(|| self.tints());
        let located = locate.map(|(first, last)| self.locate(&first, last.as_deref()));
        let stretch = self.stretch_answer.take();
        result.map(|()| WorkState { took_in, marks, tints, located, stretch, ..self.state(patches) })
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
            Op::Marks | Op::Tints | Op::Locate { .. } => {} // answered at the end of the batch
            Op::Stretch { from, to } => {
                let (f, t) = self.chars(from, to)?;
                self.close_burst(now_ms);
                let picked = self.doc.stretch_at(f, t - f);
                let answer = picked.as_ref().map(|_| self.quote_of(f, t));
                if picked.is_some() {
                    self.stretch = picked;
                }
                self.stretch_answer = Some(answer);
            }
            Op::Replace { edits, find, with, count } => {
                self.authored();
                self.close_burst(now_ms);
                let mut chars = Vec::with_capacity(edits.len());
                for e in &edits {
                    let (f, t) = self.chars(e.from, e.to)?;
                    chars.push(wrtier::doc::ReplaceEdit { index: f, len: t - f, text: e.text.clone() });
                }
                let mut sorted = chars.clone();
                sorted.sort_by_key(|e| e.index);
                if sorted.windows(2).any(|w| w[0].index + w[0].len > w[1].index) {
                    return Err("a replacement whose parts overlap".into());
                }
                let changes = self.doc.replace_matches_local(&chars, &find, &with, count, now_ms);
                self.recorded(changes);
                // Last first, so each one's place still holds.
                for e in sorted.iter().rev() {
                    self.positions.edit(e.index, e.len, &e.text);
                }
            }
            Op::Include { at, target, .. } => {
                let target = target.ok_or("that Work couldn't be found to include")?;
                let (f, _) = self.chars(at, at)?;
                self.close_burst(now_ms);
                let chars = self.doc.visible();
                let found = (self.doc.text().chars().nth(f) == Some(wrtier::change::INCLUSION)).then(|| chars.get(f)).flatten();
                let (id, _) = found.ok_or("there's no Inclusion there to record what it includes")?;
                let changes = target.record(&mut self.doc, *id, now_ms);
                self.unsaved.extend(changes);
            }
            op @ (Op::Comment { .. } | Op::Reply { .. } | Op::EditComment { .. } | Op::SetCommentSender { .. }
                | Op::Resolve { .. } | Op::DeleteComment { .. }) => self.apply_comment_op(op, now_ms)?,
            op @ (Op::Revert { .. } | Op::Restore { .. } | Op::ReturnTo { .. }) => self.apply_history_op(op, now_ms)?,
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
    /// one, otherwise reverts this sitting's latest Change still in effect. A Replace all (or a
    /// Replace that rewrote several words) is taken back whole, as one Revert.
    fn undo(&mut self, now_ms: u64) {
        if self.doc.burst().is_some() {
            self.cut = None;
            self.doc.cancel_burst();
            return;
        }
        let latest = self.sitting.iter().rev().copied().find(|&id| self.doc.is_in_effect(id));
        if let Some(target) = latest {
            let change = match self.doc.replace_group(target) {
                Some(group) => self.doc.revert_changes_local(&group, true, now_ms),
                None => Some(self.doc.revert_local(target, true, now_ms)),
            };
            self.unsaved.extend(change);
            // Redo restores it by taking back that Revert, which brings the whole group back.
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
        let mut header_dropped = false;
        if self.doc.burst().is_some() {
            self.typ_waiting = true;
        } else {
            let texts = conflict_copies(&self.typ).into_iter().chain(self.typ.exists().then(|| self.typ.clone()));
            for path in texts.collect::<Vec<_>>() {
                // The header as it's in the file, to tell whether someone edited it there.
                let header_in_file = std::fs::read_to_string(&path).ok()
                    .and_then(|c| wrtier::text_file::split_header(&wrtier::text_file::split_marker(&c).0).0);
                let read = storage::read_text_file(&self.db, &path, &mut self.doc, now_ms);
                let is_copy = path != self.typ;
                if !is_copy {
                    self.typ_newer = matches!(read, Ok(storage::TextFileRead::NotYet));
                }
                match read {
                    Ok(storage::TextFileRead::Imported(changes)) => {
                        // Edits inside the header aren't the Work's text: dropped, and said so.
                        if header_in_file.is_some_and(|h| h != self.text_file_header()) {
                            header_dropped = true;
                        }
                        edited |= !changes.is_empty();
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
        if header_dropped {
            lines.push(format!("Edits to the Preamble at the top of {} were dropped: it's made from the Work's History. \
                Change it with Preamble… on the Work.", file_name(&self.typ)));
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

    /// What goes at the top of the Text file: the Work's Preamble, from its recorded copies.
    fn text_file_header(&self) -> String {
        crate::preamble::text_file_header(&self.doc, &crate::preamble::Shelf::read(&self.vault_root))
    }

    /// Writes the Text file if it lags behind. Never over a file this Work hasn't read yet (an
    /// edit made in another program, taken in at the next `Sync`), or one written from Changes
    /// that haven't arrived yet (it's newer).
    fn write_text_file(&mut self, now_ms: u64) {
        if !self.text_file_behind || !self.unsaved.is_empty() || self.typ_newer || stamp(&self.typ) != self.typ_stamp || self.is_lost() {
            return;
        }
        match storage::write_text_file_with(&self.db, &self.typ, &self.doc, &self.text_file_header(), &self.inclusions, now_ms) {
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
        storage::write_text_file(&db, &typ, &doc, "", 100).unwrap();
        let (_, doc) = storage::load(&db, DeviceId(1)).unwrap();
        let recovery = Recovery::for_work(&dir.path().join("recovery"), dir.path(), "w");
        let mine = storage::read_all(&db).unwrap();
        (dir, OpenWork::new(db, typ, doc, 100, recovery, Vec::new(), mine))
    }

    /// A Text file's text, without its header and marker line.
    fn text_of(path: &Path) -> String {
        let (body, _) = wrtier::text_file::split_marker(&std::fs::read_to_string(path).unwrap());
        wrtier::text_file::split_header(&body).1
    }

    /// Another Device (the phone) adds `text` at `at` straight into the Work's `.db`, as if
    /// Syncthing had delivered the phone's copy.
    fn phone_types(w: &OpenWork, at: usize, text: &str) {
        let (_, mut phone) = storage::load(&w.db, DeviceId(2)).unwrap();
        let mut changes = vec![phone.set_device_name_local("Phone", 5000)];
        changes.push(phone.insert_local(at, text, 5000));
        storage::save(&w.db, &changes, &phone).unwrap();
    }

    fn marks(state: &WorkState) -> Vec<(usize, usize)> {
        state.marks.as_ref().expect("asked for")[0].ranges.clone()
    }

    #[test]
    fn highlights_come_in_utf16_and_follow_a_move() {
        let (_dir, mut w) = work("a😊bcdef");
        w.doc.comment_local(2, 3, wrtier::change::AUTHOR, "too abrupt?", 500); // "bcd"
        let state = w.edit(vec![Op::Marks], None, None, 1000).unwrap();
        assert_eq!(marks(&state), [(3, 6)], "the emoji counts twice");
        assert_eq!(state.marks.as_ref().unwrap()[0].body, "too abrupt?");
        // Only asked for: a batch without `Marks` doesn't carry them.
        let before = state.changes;
        let state = w.edit(vec![typing(5, "X")], None, None, 1100).unwrap(); // between "c" and "d", still a draft
        assert!(state.marks.is_none());
        assert_eq!(state.changes, before, "nothing recorded while the burst is open");
        let state = w.edit(vec![Op::Marks], None, None, 1200).unwrap();
        assert_eq!(marks(&state), [(3, 7)], "the draft typed inside is covered");
        // Dragging "cd" to the end records a Move: the count goes up, and the Comment is in both places.
        let state = w.edit(vec![Op::Move { from: 4, to: 7, at: 9 }], None, None, 1300).unwrap();
        assert_eq!(w.text(), "a😊befcXd");
        assert!(state.changes > before);
        let state = w.edit(vec![Op::Marks], None, None, 1400).unwrap();
        assert_eq!(marks(&state), [(3, 4), (6, 9)]);
    }

    fn comment_op(from: usize, to: usize, sender: &str, body: &str) -> Op {
        Op::Comment { from, to, sender: sender.into(), body: body.into(), sender_name: None }
    }

    #[test]
    fn adding_a_comment_closes_the_burst_and_everything_else_is_recorded_too() {
        let (_dir, mut w) = work("The rain had stopped.");
        w.edit(vec![typing(21, " Then")], None, None, 1000).unwrap(); // a burst is open
        let state = w.edit(vec![comment_op(4, 8, wrtier::change::AUTHOR, "too wet?"), Op::Marks], None, None, 1100).unwrap();
        assert!(state.burst.is_none(), "the burst was recorded first");
        let mark = state.marks.unwrap().remove(0);
        assert_eq!((mark.ranges.as_slice(), mark.quote.as_str(), mark.body.as_str()), ([(4, 8)].as_slice(), "rain", "too wet?"));
        let id = mark.id;
        let reviewer = Op::Reply { comment: id.clone(), sender: "reviewer/7f3a0001".into(), body: "agreed".into(), sender_name: Some("Anna".into()) };
        let state = w.edit(vec![reviewer, Op::EditComment { id: id.clone(), body: "far too wet?".into() }, Op::Resolve { id: id.clone(), resolved: true }, Op::Marks], None, None, 1200).unwrap();
        let mark = state.marks.unwrap().remove(0);
        assert_eq!((mark.body.as_str(), mark.resolved, mark.replies.len()), ("far too wet?", true, 1));
        assert_eq!(w.doc.field("reviewer/7f3a0001", "name"), Some("Anna"), "the Reviewer's name was recorded with the Reply");
        // Saved: reopening the Work has them.
        let (_, reopened) = storage::load(&w.db, DeviceId(1)).unwrap();
        assert_eq!(reopened.comments()[0].replies.len(), 1);
        // Deleting it takes the Reply along; acting on it afterwards is refused, and undo never touches Comments.
        let state = w.edit(vec![Op::DeleteComment { id: id.clone() }, Op::Marks], None, None, 1300).unwrap();
        assert!(state.marks.unwrap().is_empty());
        assert!(w.edit(vec![Op::Resolve { id, resolved: false }], None, None, 1400).is_err());
        w.edit(vec![Op::Undo], None, None, 1500).unwrap();
        assert_eq!(w.text(), "The rain had stopped.", "undo took back the typing, not the deletion of the Comment");
        assert!(w.doc.comments().is_empty());
        assert!(w.edit(vec![comment_op(3, 3, wrtier::change::AUTHOR, "")], None, None, 1600).is_err(), "no text to attach to");
    }

    #[test]
    fn a_body_edited_on_two_devices_shows_the_other_version() {
        let (_dir, mut w) = work("abcdef");
        let state = w.edit(vec![comment_op(1, 4, wrtier::change::AUTHOR, "first"), Op::Marks], None, None, 1000).unwrap();
        let id = state.marks.unwrap().remove(0).id;
        // The phone edits the body offline; so does this Device.
        let (_, mut phone) = storage::load(&w.db, DeviceId(2)).unwrap();
        w.edit(vec![Op::EditComment { id: id.clone(), body: "the laptop's".into() }], None, None, 3000).unwrap();
        let change = phone.edit_comment_local(id.parse().unwrap(), "the phone's", 2000);
        storage::save(&w.db, &[change], &phone).unwrap(); // Syncthing delivers the phone's copy
        notice_and_sync(&mut w, 4000);
        let mark = w.edit(vec![Op::Marks], None, None, 5000).unwrap().marks.unwrap().remove(0);
        assert_eq!((mark.body.as_str(), mark.other_body.as_deref()), ("the laptop's", Some("the phone's")));
        // Editing it again settles it.
        let mark = w.edit(vec![Op::EditComment { id, body: "the phone's".into() }, Op::Marks], None, None, 6000).unwrap().marks.unwrap().remove(0);
        assert_eq!((mark.body.as_str(), mark.other_body), ("the phone's", None));
    }

    /// The timer noticed something; the editor sends a `Sync`, as it does when `outside` goes up.
    fn notice_and_sync(w: &mut OpenWork, now_ms: u64) -> WorkState {
        let before = w.state(Vec::new()).outside;
        w.tick(now_ms);
        assert!(w.state(Vec::new()).outside > before, "the change was noticed");
        w.edit(vec![Op::Sync], None, None, now_ms + 1).unwrap()
    }

    #[test]
    fn another_devices_changes_arrive_without_disturbing_the_open_burst() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "X")], None, None, 1000).unwrap(); // a burst is open
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
        assert!(std::fs::read_to_string(&w.typ).unwrap().contains("// wrtier version:"));
    }

    #[test]
    fn an_edit_in_vim_waits_for_the_open_burst() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "X")], None, None, 1000).unwrap();
        std::fs::write(&w.typ, "abc!").unwrap();
        notice_and_sync(&mut w, 2000);
        assert_eq!(w.text(), "abcX", "not yet: the burst is open");
        w.tick(9000); // the burst closes, and the waiting edit counts as a change again
        w.edit(vec![Op::Sync], None, None, 9001).unwrap();
        assert!(w.text().contains('X') && w.text().contains('!'), "{}", w.text());
    }

    #[test]
    fn a_db_that_lacks_this_devices_changes_gets_them_again() {
        let (dir, mut w) = work("abc");
        let older = dir.path().join("older.db");
        std::fs::copy(&w.db, &older).unwrap();
        w.edit(vec![typing(3, "d")], None, None, 1000).unwrap();
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
        let state = w.edit(vec![typing(0, "Hello"), typing(5, " world")], Some("Laptop"), None, 1000).unwrap();
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
        let state = w.edit(vec![typing(3, "X")], None, None, 1000).unwrap(); // after the emoji
        assert_eq!(w.text(), "a😊Xb");
        assert_eq!(state.len, 5);
        assert!(w.edit(vec![typing(2, "!")], None, None, 1100).is_err(), "inside the emoji");
        assert!(w.edit(vec![typing(99, "!")], None, None, 1200).is_err(), "past the end");
        assert_eq!(w.text(), "a😊Xb", "nothing happened");
    }

    #[test]
    fn a_paste_is_its_own_change_at_once() {
        let (_dir, mut w) = work("ab");
        w.edit(vec![typing(1, "x"), Op::Paste { from: 3, to: 3, text: "PASTED".into() }], None, None, 1000).unwrap();
        assert_eq!(w.text(), "axbPASTED");
        assert_eq!(saved(&w), vec!["insert", "insert", "insert"], "the open burst closed first, then the paste");
    }

    #[test]
    fn cutting_and_pasting_the_same_text_is_a_move() {
        let (_dir, mut w) = work("One. Two. Three.");
        w.edit(vec![Op::Cut { from: 0, to: 5 }], None, None, 1000).unwrap();
        assert_eq!(w.text(), "Two. Three.");
        w.edit(vec![Op::Cursor { pos: 11 }], None, None, 20_000).unwrap(); // look around for a while
        w.tick(30_000);
        assert_eq!(saved(&w), vec!["insert"], "the cut waits for its paste");
        let state = w.edit(vec![Op::Paste { from: 11, to: 11, text: "One. ".into() }], None, None, 31_000).unwrap();
        assert_eq!(w.text(), "Two. Three.One. ");
        assert!(state.patches.is_empty(), "the editor already shows it");
        assert_eq!(saved(&w), vec!["insert", "move"]);
    }

    #[test]
    fn a_cut_followed_by_typing_is_a_deletion() {
        let (_dir, mut w) = work("One. Two.");
        w.edit(vec![Op::Cut { from: 0, to: 5 }, typing(0, "X")], None, None, 1000).unwrap();
        w.close(2000);
        assert_eq!(w.text(), "XTwo.");
        assert_eq!(saved(&w), vec!["insert", "delete", "insert"]);
    }

    #[test]
    fn dragging_a_selection_is_a_move() {
        let (_dir, mut w) = work("abcdef");
        w.edit(vec![Op::Move { from: 0, to: 2, at: 6 }], None, None, 1000).unwrap();
        assert_eq!(w.text(), "cdefab");
        assert_eq!(saved(&w), vec!["insert", "move"]);
    }

    #[test]
    fn undo_inside_the_burst_edits_the_draft() {
        let (_dir, mut w) = work("");
        w.edit(vec![typing(0, "Hello"), typing(5, " world")], None, None, 1000).unwrap();
        // The editor undoes " world" itself and tells us.
        let state = w.edit(vec![Op::Delete { from: 5, to: 11, origin: Origin::Undo }], None, None, 1100).unwrap();
        assert!(state.patches.is_empty());
        w.close(2000);
        assert_eq!(w.text(), "Hello");
        assert_eq!(saved(&w), vec!["insert"], "the undone typing was never recorded");
    }

    fn replace_op(edits: &[(usize, usize, &str)], find: &str, with: &str, count: usize) -> Op {
        let edits = edits.iter().map(|&(from, to, text)| ReplaceOpEdit { from, to, text: text.into() }).collect();
        Op::Replace { edits, find: find.into(), with: with.into(), count }
    }

    #[test]
    fn a_replace_all_is_one_action_for_undo_and_redo() {
        let (_dir, mut w) = work("Rain fell. The *rain* had stopped.");
        // Typing first: the replacement closes the burst, which stays its own action.
        w.edit(vec![typing(34, " Then")], None, None, 1000).unwrap();
        let state = w.edit(vec![replace_op(&[(0, 4, "Snow"), (16, 20, "snow"), (22, 25, "has")], "rain", "snow", 2)], None, None, 2000).unwrap();
        assert!(state.patches.is_empty(), "the editor already shows it");
        assert_eq!(w.text(), "Snow fell. The *snow* has stopped. Then");
        let state = w.edit(vec![Op::Undo], None, None, 3000).unwrap();
        assert_eq!(w.text(), "Rain fell. The *rain* had stopped. Then", "all three words back, the typing stays");
        assert_eq!(editor_after("Snow fell. The *snow* has stopped. Then", &state), w.text());
        w.edit(vec![Op::Redo], None, None, 4000).unwrap();
        assert_eq!(w.text(), "Snow fell. The *snow* has stopped. Then");
        w.edit(vec![Op::Undo], None, None, 5000).unwrap();
        w.edit(vec![Op::Undo], None, None, 6000).unwrap();
        assert_eq!(w.text(), "Rain fell. The *rain* had stopped.");
    }

    #[test]
    fn a_replacement_whose_parts_overlap_is_refused() {
        let (_dir, mut w) = work("rain");
        assert!(w.edit(vec![replace_op(&[(0, 3, "x"), (2, 4, "y")], "rain", "xy", 1)], None, None, 1000).is_err());
    }

    #[test]
    fn undo_beyond_the_burst_reverts_the_latest_change_and_redo_restores_it() {
        let (_dir, mut w) = work("The rain.");
        w.edit(vec![typing(8, " had stopped")], None, None, 1000).unwrap();
        w.tick(10_000); // closed and saved
        assert_eq!(w.text(), "The rain had stopped.");
        let state = w.edit(vec![Op::Undo], None, None, 11_000).unwrap();
        assert_eq!(w.text(), "The rain.");
        assert_eq!(editor_after("The rain had stopped.", &state), "The rain.");
        let state = w.edit(vec![Op::Redo], None, None, 12_000).unwrap();
        assert_eq!(editor_after("The rain.", &state), "The rain had stopped.");
        assert_eq!(saved(&w), vec!["insert", "insert", "revert", "revert"]);
        // Undo only goes back through this sitting: the text from before stays.
        w.edit(vec![Op::Undo], None, None, 13_000).unwrap();
        w.edit(vec![Op::Undo], None, None, 14_000).unwrap();
        assert_eq!(w.text(), "The rain.");
    }

    /// "The rain had stopped." with "stopped" changed to "eased" in the app: returns the ID of
    /// the Change that wrote the sentence.
    fn rain_eased(w: &mut OpenWork) -> String {
        let first = storage::read_all(&w.db).unwrap()[0].id;
        w.edit(vec![Op::Delete { from: 13, to: 20, origin: Origin::Input }, typing(13, "eased")], None, None, 1000).unwrap();
        w.tick(10_000); // closed and saved
        assert_eq!(w.text(), "The rain had eased.");
        first.to_string()
    }

    #[test]
    fn a_revert_from_the_history_view_takes_its_dependents_and_ctrl_z_brings_it_back() {
        let (_dir, mut w) = work("The rain had stopped.");
        let first = rain_eased(&mut w);
        let state = w.edit(vec![Op::Revert { changes: vec![first], dependents: true }], None, None, 11_000).unwrap();
        assert_eq!(w.text(), "", "\"eased\" was typed inside the sentence, so it goes too");
        assert_eq!(editor_after("The rain had eased.", &state), "");
        let state = w.edit(vec![Op::Undo], None, None, 12_000).unwrap();
        assert_eq!(w.text(), "The rain had eased.");
        assert_eq!(editor_after("", &state), "The rain had eased.");
        let state = w.edit(vec![Op::Redo], None, None, 13_000).unwrap();
        assert_eq!(w.text(), "");
        assert_eq!(editor_after("The rain had eased.", &state), "");
    }

    #[test]
    fn a_revert_without_its_dependents_leaves_them() {
        let (_dir, mut w) = work("The rain had stopped.");
        let first = rain_eased(&mut w);
        w.edit(vec![Op::Revert { changes: vec![first.clone()], dependents: false }], None, None, 11_000).unwrap();
        assert_eq!(w.text(), "eased");
        // Restore brings the sentence back; Ctrl+Z takes the Restore back again.
        w.edit(vec![Op::Restore { changes: vec![first] }], None, None, 12_000).unwrap();
        assert_eq!(w.text(), "The rain had eased.");
        w.edit(vec![Op::Undo], None, None, 13_000).unwrap();
        assert_eq!(w.text(), "eased");
    }

    #[test]
    fn returning_to_a_milestone_and_undoing_it_leaves_the_text_exactly_as_before() {
        let (_dir, mut w) = work("The rain had stopped.");
        let draft = w.doc.make_milestone_local("Draft 1", 500).id;
        rain_eased(&mut w);
        w.edit(vec![typing(19, " Gulls circled.")], None, None, 20_000).unwrap(); // a burst still open
        let state = w.edit(vec![Op::ReturnTo { milestone: draft.to_string(), keep: Vec::new() }], None, None, 21_000).unwrap();
        assert_eq!(w.text(), "The rain had stopped.", "the open burst was recorded first, then taken back too");
        assert_eq!(editor_after("The rain had eased. Gulls circled.", &state), "The rain had stopped.");
        w.edit(vec![Op::Undo], None, None, 22_000).unwrap();
        assert_eq!(w.text(), "The rain had eased. Gulls circled.");
    }

    #[test]
    fn ctrl_z_never_brings_back_a_deleted_comment() {
        let (_dir, mut w) = work("The rain had stopped.");
        rain_eased(&mut w);
        w.edit(vec![comment_op(0, 8, "author", "too abrupt?")], None, None, 20_000).unwrap();
        let id = w.doc.comments()[0].id.to_string();
        w.edit(vec![Op::DeleteComment { id }], None, None, 21_000).unwrap();
        w.edit(vec![Op::Undo], None, None, 22_000).unwrap();
        assert!(w.doc.comments().is_empty(), "still deleted");
        assert_ne!(w.text(), "The rain had eased.", "undo took back the latest text Change instead");
    }

    #[test]
    fn undo_with_a_burst_open_drops_the_burst() {
        let (_dir, mut w) = work("abc");
        let state = w.edit(vec![typing(3, "def"), Op::Undo], None, None, 1000).unwrap();
        assert_eq!(w.text(), "abc");
        assert_eq!(editor_after("abcdef", &state), "abc");
        assert!(state.burst.is_none());
    }

    #[test]
    fn undoing_a_cut_puts_it_back() {
        let (_dir, mut w) = work("One. Two.");
        w.edit(vec![Op::Cut { from: 0, to: 5 }], None, None, 1000).unwrap();
        // The editor's undo re-inserts the cut text.
        let state = w.edit(vec![Op::Type { from: 0, to: 0, text: "One. ".into(), origin: Origin::Undo }], None, None, 1100).unwrap();
        assert_eq!(w.text(), "One. Two.");
        assert!(state.patches.is_empty());
        w.close(2000);
        assert_eq!(saved(&w), vec!["insert"], "nothing recorded");
    }

    #[test]
    fn redo_after_undoing_a_whole_draft_types_it_again() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "def")], None, None, 1000).unwrap();
        let state = w.edit(vec![Op::Delete { from: 3, to: 6, origin: Origin::Undo }], None, None, 1100).unwrap();
        assert_eq!(w.text(), "abc");
        assert!(state.burst.is_none(), "the empty burst was dropped");
        let state = w.edit(vec![Op::Type { from: 3, to: 3, text: "def".into(), origin: Origin::Redo }], None, None, 1200).unwrap();
        assert_eq!(w.text(), "abcdef");
        assert!(state.patches.is_empty(), "the editor already shows it");
        w.close(2000);
        assert_eq!(saved(&w), vec!["insert", "insert"], "recorded once, as typing");
    }

    #[test]
    fn redo_after_undoing_a_deletion_deletes_again() {
        let (_dir, mut w) = work("abcdef");
        w.edit(vec![Op::Delete { from: 3, to: 6, origin: Origin::Input }], None, None, 1000).unwrap();
        w.edit(vec![Op::Type { from: 3, to: 3, text: "def".into(), origin: Origin::Undo }], None, None, 1100).unwrap();
        assert_eq!(w.text(), "abcdef");
        let state = w.edit(vec![Op::Delete { from: 3, to: 6, origin: Origin::Redo }], None, None, 1200).unwrap();
        assert_eq!(w.text(), "abc");
        assert!(state.patches.is_empty());
        w.close(2000);
        assert_eq!(saved(&w), vec!["insert", "delete"]);
    }

    #[test]
    fn the_editors_undo_after_the_burst_closed_becomes_a_revert() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "def")], None, None, 1000).unwrap();
        w.tick(10_000); // the timer closed it before the editor's undo arrived
        let state = w.edit(vec![Op::Delete { from: 3, to: 6, origin: Origin::Undo }], None, None, 10_100).unwrap();
        assert_eq!(w.text(), "abc");
        assert_eq!(editor_after("abc", &state), "abc");
        assert_eq!(saved(&w), vec!["insert", "insert", "revert"]);
    }

    #[test]
    fn the_open_burst_is_kept_until_it_is_saved() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "def")], None, None, 1000).unwrap();
        assert!(w.recovery.json.exists(), "kept from the first keystroke");
        w.tick(10_000); // closed and saved
        assert!(!w.recovery.json.exists(), "nothing left to keep");
    }

    #[test]
    fn the_text_file_follows_on_leaving_and_every_30_seconds() {
        let (_dir, mut w) = work("abc");
        w.edit(vec![typing(3, "d")], None, None, 1000).unwrap();
        w.tick(7000); // saved, but the Text file waits
        assert_eq!(text_of(&w.typ), "abc");
        w.tick(31_000);
        assert_eq!(text_of(&w.typ), "abcd");
        w.edit(vec![typing(4, "e")], None, None, 32_000).unwrap();
        w.leave(33_000);
        assert_eq!(text_of(&w.typ), "abcde");
    }

    #[test]
    fn a_text_file_edited_outside_is_not_overwritten_before_it_is_taken_in() {
        let (_dir, mut w) = work("abc");
        std::fs::write(&w.typ, "edited in vim").unwrap();
        w.edit(vec![typing(3, "d")], None, None, 1000).unwrap();
        w.leave(2000);
        assert_eq!(std::fs::read_to_string(&w.typ).unwrap(), "edited in vim");
    }
}
