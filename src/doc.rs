//! A Work's text as one Device sees it: the History's Changes applied to a Fugue tree,
//! laid out with the Moves in effect.
//!
//! Everything shown is derived from the set of Changes. `refresh` recomputes it all; typing,
//! backspacing and dismissing update it directly instead, and in debug builds (the tests) every
//! such shortcut is checked against a full recompute. `crate::reference::doc` is the first,
//! simple version; this one must behave identically (`tests/against_reference.rs`).

use crate::change::{comment_record, device_record, is_free_text, is_meta_record, milestone_record, session_record, Change, ChangeKind, InsertRun, CharRange, NoticeKey, Span};
use crate::text_file;
use crate::fugue::Tree;
use crate::ids::{ChangeId, DeviceId, Hlc, CharId};
use crate::layout::{Layout, MoveInfo, Piece};
use rustc_hash::FxHashSet;
use std::collections::{HashMap, HashSet};

mod colours;
mod compare;
mod history;
mod snippets;
pub use colours::SessionText;
pub use compare::{Comparison, Stretch, Version};
pub use history::{LeftOut, Reverted, SessionPart, Summary};
pub use snippets::{FieldLine, MovedFrom, Segment, SegmentKind, Snippet, Snippets};

pub struct Doc {
    tree: Tree,
    device: DeviceId,
    next_counter: u32,
    clock: Hlc,
    applied: HashSet<ChangeId>,                    // to avoid applying the same Change twice
    reverted_by: HashMap<ChangeId, Vec<ChangeId>>, // a Change → the Reverts that target it
    seen: HashMap<DeviceId, u32>,                  // the highest counter applied from each Device
    moves: Vec<(Hlc, MoveInfo)>,                   // every Move applied, with its clock for sorting
    typed_into: HashMap<CharId, Vec<CharId>>,      // a piece's anchor → nodes typed or dropped into it
    dismissals: Vec<(ChangeId, NoticeKey)>,        // every Dismiss Change, and the notice it hides
    fields: HashMap<(String, String), Vec<FieldValue>>, // (record, field) → every value set
    content: Vec<(ChangeId, Hlc)>,                 // every Change to the Work's content, as applied
    content_set: HashSet<ChangeId>,                // the same, for lookups
    milestones: Vec<(ChangeId, Hlc, String, Vec<(DeviceId, u32)>)>, // every Milestone: id, time, name, what it holds
    comments: Vec<CommentMade>,                    // every Comment, as made
    replies: Vec<ReplyMade>,                       // every Reply, as made
    session: Option<(ChangeId, u64)>,              // this Device's current Session: (its first Change, time of the last)
    sessions: HashMap<ChangeId, Vec<ChangeId>>,    // every Session → its Changes
    made: HashMap<ChangeId, history::Made>,        // every Change → when, and what it did (for the History view)
    layout: Layout,
    // Worked out by `refresh` (or a fast path) after every Change, so reading the text doesn't redo it.
    visible: Vec<u32>, // the visible characters' tree nodes, in reading order
    burst: Option<Burst>, // the open typing burst, if any
}

/// One value a field was set to.
#[derive(Clone, Debug)]
struct FieldValue {
    hlc: Hlc,
    id: ChangeId,
    value: String,
    /// The value it replaced, as the setting Device saw it.
    replaces: Option<ChangeId>,
}

/// A typing burst: one stretch of new text at one spot, kept as a draft until it closes.
///
/// While open it lives only here (and, later, in the recovery file), never in the `.db`, so it
/// can grow and shrink freely: typos fixed inside it are never recorded. It's anchored to the
/// character before it, not to an index, so Changes merged in from other Devices don't move it
/// and don't close it. When it closes it becomes one Change: an Insert, or a Replace if it was
/// typed over a selection. An empty one records nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Burst {
    pub kind: BurstKind,
    /// Reserved when the burst opened; the Change gets it when the burst closes.
    pub id: ChangeId,
    /// The character the draft follows (None = the start of the text).
    pub left: Option<CharId>,
    /// A selection it was typed over: hidden while the burst is open, deleted when it closes.
    pub replacing: Vec<CharId>,
    pub draft: String,
    pub opened_ms: u64,
    pub last_ms: u64,
    at: usize,     // where the draft shows in the shown text
    hidden: usize, // how many characters of `replacing` are currently hidden
}

/// What kind of burst is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BurstKind {
    /// New text (possibly typed over a selection): becomes an Insert, or a Replace.
    Typing,
    /// Backspacing or deleting forwards at one spot: the deleted characters are only hidden
    /// until it closes, and then become one Delete, however many keys it took.
    Deleting,
}

/// What pasting a cut did (`Doc::cut_to_move`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CutPasted {
    /// The cut text moved to where it was pasted: one Move.
    Moved(Change),
    /// Pasted back where it was cut from: nothing to record.
    Unchanged,
    /// It couldn't move as one stretch any more, so the cut was recorded as a deletion (if
    /// anything was left to delete); the paste is then new text.
    Deleted(Option<Change>),
}

/// A burst closes after this long without typing...
pub const BURST_PAUSE_MS: u64 = 5_000;
/// ...or after this long of nonstop typing. Both are for tuning; they don't affect correctness.
pub const BURST_MAX_MS: u64 = 60_000;

/// A message for the Author about text the merge had to decide on its own.
/// It asks for nothing and changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeNotice {
    pub key: NoticeKey,
    /// The affected characters that are visible, in reading order: where "show me" jumps to.
    pub chars: Vec<CharId>,
    /// The same characters as text, to quote in the History view.
    pub text: String,
    /// The same characters as places in the text: the first and last of each run that sits
    /// together, in reading order ("Show me" steps through them).
    pub places: Vec<(CharId, CharId)>,
}

/// A named point in the History the Author can return to or compare against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Milestone {
    pub id: ChangeId,
    pub hlc: Hlc,
    pub name: String,
    /// What it holds: the highest Change counter the naming Device had seen from each Device.
    pub seen: Vec<(DeviceId, u32)>,
}

/// A Comment as its Change made it (later edits are fields).
#[derive(Clone, Debug)]
struct CommentMade {
    id: ChangeId,
    hlc: Hlc,
    pieces: Vec<Span>,
    quote: String,
    sender: String,
    body: String,
}

/// A Reply as its Change made it (later edits are fields).
#[derive(Clone, Debug)]
struct ReplyMade {
    id: ChangeId,
    hlc: Hlc,
    comment: ChangeId,
    sender: String,
    body: String,
}

/// A Comment as it is now, for the Comments panel and the editor's highlights.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentView {
    pub id: ChangeId,
    /// When it was made (edits don't change it).
    pub hlc: Hlc,
    /// `change::AUTHOR` or a `change::reviewer_record`.
    pub sender: String,
    pub body: String,
    pub resolved: bool,
    /// The text it was attached to, as selected.
    pub quote: String,
    /// Where its visible characters are in the text the Author sees (`text()`): start..end in
    /// characters, in reading order. Several when a Move split its text or text was Moved into
    /// it; empty when it's a Detached comment.
    pub ranges: Vec<(usize, usize)>,
    /// Its Replies still in effect, in the fixed Change order.
    pub replies: Vec<ReplyView>,
}

impl CommentView {
    /// None of its text is visible: it's listed apart, with its quote, until some comes back.
    pub fn is_detached(&self) -> bool {
        self.ranges.is_empty()
    }
}

/// A Reply as it is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyView {
    pub id: ChangeId,
    pub hlc: Hlc,
    pub sender: String,
    pub body: String,
}

/// What returning to a Milestone would do, for the Revert preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReturnPreview {
    /// Every content Change that isn't part of the Milestone: all of them get reverted.
    pub reverts: Vec<ChangeId>,
    /// Those made before the Milestone on another Device that the naming Device hadn't seen yet.
    pub made_earlier_elsewhere: Vec<ChangeId>,
}

/// Does a Milestone holding `seen` include `change`?
fn holds(seen: &[(DeviceId, u32)], change: ChangeId) -> bool {
    seen.iter().any(|&(d, c)| d == change.device && change.counter <= c)
}

/// A new Session starts after this long without a Change from this Device (to be tuned in use).
pub const SESSION_GAP_MS: u64 = 30 * 60 * 1000;

/// The anchor node of a piece.
fn anchor_of(piece: Piece) -> CharId {
    CharId { change: piece.0, offset: piece.1 as u32 }
}

impl Doc {
    pub fn new(device: DeviceId) -> Self {
        Doc {
            tree: Tree::new(),
            device,
            next_counter: 1,
            clock: Hlc(0),
            applied: HashSet::new(),
            reverted_by: HashMap::new(),
            seen: HashMap::new(),
            moves: Vec::new(),
            typed_into: HashMap::new(),
            dismissals: Vec::new(),
            fields: HashMap::new(),
            content: Vec::new(),
            content_set: HashSet::new(),
            milestones: Vec::new(),
            comments: Vec::new(),
            replies: Vec::new(),
            session: None, // a Work just opened: the first Change starts a new Session
            sessions: HashMap::new(),
            made: HashMap::new(),
            layout: Layout::default(),
            visible: Vec::new(),
            burst: None,
        }
    }

    // ---- Reading ----

    /// The text the Author sees: the History's text, plus the open burst's draft
    /// (minus any selection it's typed over).
    pub fn text(&self) -> String {
        let Some(burst) = &self.burst else {
            return self.visible.iter().filter_map(|&node| self.tree.char_at(node)).collect();
        };
        let hidden = self.hidden_by(burst);
        let mut text = String::new();
        let mut shown = 0;
        for &node in self.visible.iter().filter(|n| !hidden.contains(n)) {
            if shown == burst.at {
                text.push_str(&burst.draft);
            }
            text.extend(self.tree.char_at(node));
            shown += 1;
        }
        if shown <= burst.at {
            text.push_str(&burst.draft); // the draft is at the very end
        }
        text
    }

    /// The length of the text the Author sees, in characters.
    pub fn len(&self) -> usize {
        match &self.burst {
            None => self.visible.len(),
            Some(b) => self.visible.len() - b.hidden + b.draft.chars().count(),
        }
    }

    /// The open typing burst, if any.
    pub fn burst(&self) -> Option<&Burst> {
        self.burst.as_ref()
    }

    /// The History's visible characters in reading order, with the piece each is drawn in.
    /// (Without the open burst's draft, which has no characters in the History yet.)
    pub fn visible(&self) -> Vec<(CharId, Option<Piece>)> {
        self.visible.iter().map(|&node| (self.tree.id_of(node), self.layout.level_at(node))).collect()
    }

    pub fn is_in_effect(&self, id: ChangeId) -> bool {
        match self.reverted_by.get(&id) {
            Some(reverters) => reverters.iter().all(|&r| !self.is_in_effect(r)),
            None => true,
        }
    }

    /// The Merge notices the Author hasn't dismissed, worked out from the History.
    /// Every Device gets the same list, in the same order.
    pub fn merge_notices(&self) -> Vec<MergeNotice> {
        let node_set = |chars: &[CharId]| -> FxHashSet<u32> { chars.iter().filter_map(|&id| self.tree.index_of(id)).collect() };
        let mut found: Vec<(NoticeKey, FxHashSet<u32>)> = Vec::new();
        for (later, earlier, chars) in self.layout.contested_text() {
            found.push((NoticeKey::Overlap { later: *later, earlier: *earlier }, node_set(chars)));
        }
        for (skipped, chars) in self.layout.skipped() {
            found.push((NoticeKey::Loop { skipped: *skipped }, node_set(chars)));
        }
        // External edits compared against the current text because no written version was found:
        // they point at the text that edit wrote.
        let mut unknown: Vec<ChangeId> = self.fields.keys()
            .filter(|(record, field)| field == "base" && self.field(record, "base") == Some("unknown"))
            .filter_map(|(record, _)| record.strip_prefix("session/")?.parse().ok())
            .collect();
        unknown.sort();
        for session in unknown {
            let changes: FxHashSet<ChangeId> = self.sessions.get(&session).into_iter().flatten().copied().collect();
            let nodes = self.visible.iter().copied().filter(|&n| changes.contains(&self.tree.change_at(n))).collect();
            found.push((NoticeKey::UnknownTextFile { session }, nodes));
        }
        // Free text outside the Work's text (the Preamble's raw Typst) edited on two Devices:
        // it points at no text in the Work; `concurrent_field` gives the version that lost.
        for (_, _, later, earlier) in self.concurrent_fields() {
            found.push((NoticeKey::ConcurrentField { later, earlier }, FxHashSet::default()));
        }
        found.into_iter()
            .filter(|(key, _)| !self.is_dismissed(*key))
            .map(|(key, involved)| {
                // Walk the visible text so the characters come out in reading order; a gap in
                // where they sit starts a new place.
                let found: Vec<(usize, u32)> = self.visible.iter().copied().enumerate().filter(|(_, n)| involved.contains(n)).collect();
                let mut places: Vec<(CharId, CharId)> = Vec::new();
                for (i, &(at, node)) in found.iter().enumerate() {
                    let id = self.tree.id_of(node);
                    match places.last_mut() {
                        Some(place) if found[i - 1].0 + 1 == at => place.1 = id,
                        _ => places.push((id, id)),
                    }
                }
                MergeNotice {
                    key,
                    chars: found.iter().map(|&(_, n)| self.tree.id_of(n)).collect(),
                    text: found.iter().filter_map(|&(_, n)| self.tree.char_at(n)).collect(),
                    places,
                }
            })
            .collect()
    }

    /// This Device's ID.
    /// Has this Doc applied the Change?
    pub fn has(&self, id: ChangeId) -> bool {
        self.applied.contains(&id)
    }

    pub fn device(&self) -> DeviceId {
        self.device
    }

    /// How many Changes the History holds: it goes up whenever one is made or taken in.
    pub fn change_count(&self) -> usize {
        self.applied.len()
    }

    /// The version this Device has: the highest Change counter it has applied from each Device.
    pub fn version(&self) -> Vec<(DeviceId, u32)> {
        let mut version: Vec<(DeviceId, u32)> = self.seen.iter().map(|(&d, &c)| (d, c)).collect();
        version.sort();
        version
    }

    /// The History's text, without the open burst's draft: what the Text file holds.
    pub fn committed_text(&self) -> String {
        self.visible.iter().filter_map(|&node| self.tree.char_at(node)).collect()
    }

    /// The Inclusions in the History's text (without the open burst's draft), in reading
    /// order: each one's character, and where it is in `committed_text` (in characters).
    pub fn inclusions(&self) -> Vec<(CharId, usize)> {
        self.visible.iter().enumerate()
            .filter(|&(_, &node)| self.tree.char_at(node) == Some(crate::change::INCLUSION))
            .map(|(i, &node)| (self.tree.id_of(node), i))
            .collect()
    }

    /// Where a character is in `committed_text` (in characters), if it's visible.
    pub fn index_of_char(&self, id: CharId) -> Option<usize> {
        let node = self.tree.index_of(id)?;
        self.visible.iter().position(|&n| n == node)
    }

    /// The current value of a record's field: the latest in the fixed Change order among the
    /// Changes in effect. None if it was never set, or every value was reverted.
    pub fn field(&self, record: &str, field: &str) -> Option<&str> {
        self.field_value(record, field).map(|v| v.value.as_str())
    }

    /// The value in effect, with the Change that set it.
    fn field_value(&self, record: &str, field: &str) -> Option<&FieldValue> {
        self.fields.get(&(record.to_string(), field.to_string()))?
            .iter()
            .filter(|v| self.is_in_effect(v.id))
            .max_by_key(|v| (v.hlc, v.id.device))
    }

    /// Every field of a record that has a value in effect, with that value, sorted by field.
    pub fn fields_of(&self, record: &str) -> Vec<(String, String)> {
        let mut found: Vec<(String, String)> = self.fields.keys()
            .filter(|(r, _)| r == record)
            .filter_map(|(r, field)| Some((field.clone(), self.field(r, field)?.to_string())))
            .collect();
        found.sort();
        found
    }

    /// The records whose name starts with `prefix` (such as "reviewer/") that have a field in
    /// effect, sorted.
    pub fn records(&self, prefix: &str) -> Vec<String> {
        let mut found: Vec<String> = self.fields.keys()
            .filter(|(record, field)| record.starts_with(prefix) && self.field_value(record, field).is_some())
            .map(|(record, _)| record.clone())
            .collect();
        found.sort();
        found.dedup();
        found
    }

    /// The value a field had in an earlier Change, such as the version a Merge notice says lost.
    pub fn field_set_by(&self, record: &str, field: &str, change: ChangeId) -> Option<&str> {
        self.fields.get(&(record.to_string(), field.to_string()))?
            .iter().find(|v| v.id == change).map(|v| v.value.as_str())
    }

    /// Free-text fields whose value in effect was set without seeing another Device's edit of
    /// the same value: (record, field, the value shown, the latest one that lost). The notice
    /// goes once the Author edits the field again (that edit replaces the shown value).
    fn concurrent_fields(&self) -> Vec<(String, String, ChangeId, ChangeId)> {
        let mut found = Vec::new();
        for ((record, field), values) in &self.fields {
            if !is_free_text(record, field) {
                continue;
            }
            let Some(shown) = self.field_value(record, field) else { continue };
            let lost = values.iter()
                .filter(|v| v.id != shown.id && v.replaces == shown.replaces && v.value != shown.value && self.is_in_effect(v.id))
                .max_by_key(|v| (v.hlc, v.id.device));
            if let Some(lost) = lost {
                found.push((record.clone(), field.clone(), shown.id, lost.id));
            }
        }
        found.sort();
        found
    }

    /// A Device's readable name, if it has one.
    pub fn device_name(&self, device: DeviceId) -> Option<&str> {
        self.field(&device_record(device), "name")
    }

    /// Dismissed if a Dismiss of it is in effect, so reverting the Dismiss brings it back.
    fn is_dismissed(&self, key: NoticeKey) -> bool {
        self.dismissals.iter().any(|&(id, k)| k == key && self.is_in_effect(id))
    }

    /// The text of a deleted range, deleted characters included: the readable copy in storage.
    pub fn text_of(&self, range: &CharRange) -> String {
        (range.first.offset..range.first.offset + range.len)
            .filter_map(|offset| self.tree.char_of(CharId { change: range.first.change, offset }))
            .collect()
    }

    /// Readable copy of a moved piece: the visible text from its first to its last character.
    /// A piece can hold text from many Changes, so walk the visible text rather than offsets.
    /// Only used for the readable column in storage, so an empty string is fine if it's gone.
    pub fn piece_text(&self, span: &Span) -> String {
        let (Some(first), Some(last)) = (self.tree.index_of(span.first), self.tree.index_of(span.last)) else {
            return String::new();
        };
        let start = self.visible.iter().position(|&n| n == first);
        let end = self.visible.iter().position(|&n| n == last);
        match (start, end) {
            (Some(s), Some(e)) if s <= e => self.visible[s..=e].iter().filter_map(|&n| self.tree.char_at(n)).collect(),
            _ => String::new(),
        }
    }

    pub fn depends_on(&self, target: ChangeId) -> Vec<ChangeId> {
        self.tree.inside(target)
    }

    // ---- Applying Changes ----

    /// Applies many Changes, e.g. when opening a Work, and works out the text once at the end.
    /// They must be in the fixed Change order.
    pub fn apply_all(&mut self, changes: &[Change]) {
        for change in changes {
            self.apply_change(change);
        }
        self.refresh();
    }

    pub fn apply(&mut self, change: &Change) {
        let new = !self.applied.contains(&change.id);
        self.apply_change(change);
        match &change.kind {
            // Dismissing a notice, setting a field or commenting changes no text: nothing to recompute.
            ChangeKind::Dismiss { .. } | ChangeKind::SetField { .. } | ChangeKind::Milestone { .. }
            | ChangeKind::Comment { .. } | ChangeKind::Reply { .. } => self.debug_check(),
            _ if new => self.refresh(),
            _ => {} // already had it
        }
    }

    /// Recomputes everything derived from the Changes: what's undone, the layout, the visible text.
    fn refresh(&mut self) {
        self.refresh_undone();
        self.layout = Layout::compute(&self.tree, &self.in_effect_moves(), &self.typed_into);
        self.visible = self.computed_visible(&self.layout);
        self.place_burst(); // merges don't close a burst; it stays after its character
    }

    fn refresh_undone(&mut self) {
        // Only Changes that have been reverted can be undone, so check just those.
        let undone: Vec<ChangeId> = self.reverted_by.keys().copied()
            .filter(|&id| !self.is_in_effect(id))
            .collect();
        self.tree.set_undone(undone);
    }

    /// The visible characters in reading order, according to `layout`.
    fn computed_visible(&self, layout: &Layout) -> Vec<u32> {
        layout.reading_nodes(&self.tree).into_iter().filter(|&n| self.tree.is_visible_at(n)).collect()
    }

    /// The Moves in effect, in the fixed Change order: what the layout is computed from.
    fn in_effect_moves(&self) -> Vec<MoveInfo> {
        let mut in_effect: Vec<(Hlc, MoveInfo)> = self.moves.iter()
            .filter(|(_, info)| self.is_in_effect(info.id))
            .cloned()
            .collect();
        in_effect.sort_by_key(|(hlc, info)| (*hlc, info.id.device));
        in_effect.into_iter().map(|(_, info)| info).collect()
    }

    /// In debug builds (the tests): recompute everything from scratch, without touching the
    /// cached state, and check every shortcut gave the same visible text and pieces.
    fn debug_check(&self) {
        if cfg!(debug_assertions) {
            let layout = Layout::compute(&self.tree, &self.in_effect_moves(), &self.typed_into);
            let fresh: Vec<(u32, Option<Piece>)> = self.computed_visible(&layout).into_iter()
                .map(|n| (n, layout.level_at(n)))
                .collect();
            let cached: Vec<(u32, Option<Piece>)> = self.visible.iter().map(|&n| (n, self.layout.level_at(n))).collect();
            assert_eq!(cached, fresh, "a shortcut disagrees with recomputing from scratch");
        }
    }

    /// Records one Change without recomputing what's derived from it; call `refresh` after.
    fn apply_change(&mut self, change: &Change) {
        if !self.applied.insert(change.id) {
            return; // already applied
        }
        let seen = self.seen.entry(change.id.device).or_insert(0);
        *seen = (*seen).max(change.id.counter);
        self.sessions.entry(change.session).or_default().push(change.id);
        self.made.insert(change.id, history::Made::of(change));
        self.clock.observe(change.hlc);
        if change.id.device == self.device {
            // A Change of ours, e.g. loaded from the file: never reuse its counter.
            self.next_counter = self.next_counter.max(change.id.counter + 1);
        }
        // Is it part of the Work's content (what a Milestone captures and returning reverts)?
        // A Revert is content if anything it reverts is; Device names, Milestones, dismissed
        // notices and Comments aren't.
        let is_content = match &change.kind {
            ChangeKind::Insert(_) | ChangeKind::Delete(_) | ChangeKind::Replace { .. } | ChangeKind::Move { .. } => true,
            ChangeKind::Revert { targets } => targets.iter().any(|t| self.content_set.contains(t)),
            ChangeKind::SetField { record, .. } => !is_meta_record(record),
            ChangeKind::Dismiss { .. } | ChangeKind::Milestone { .. } | ChangeKind::Comment { .. } | ChangeKind::Reply { .. } => false,
        };
        if is_content {
            self.content.push((change.id, change.hlc));
            self.content_set.insert(change.id);
        }
        match &change.kind {
            ChangeKind::Insert(run) => {
                self.tree.insert_run(change.id, &run.text, run.parent, run.side, run.right_origin);
                self.note_into(run.into, change.id, run.text.chars().count());
            }
            ChangeKind::Delete(ranges) => {
                for range in ranges {
                    self.tree.delete_range(range.first, range.len, change.id);
                }
            }
            ChangeKind::Replace { deleted, inserted } => {
                for range in deleted {
                    self.tree.delete_range(range.first, range.len, change.id);
                }
                self.tree.insert_run(change.id, &inserted.text, inserted.parent, inserted.side, inserted.right_origin);
                self.note_into(inserted.into, change.id, inserted.text.chars().count());
            }
            ChangeKind::Revert { targets } => {
                for target in targets {
                    self.reverted_by.entry(*target).or_default().push(change.id);
                }
            }
            ChangeKind::Move { pieces, parent, side, right_origin, seen, into } => {
                self.tree.insert_anchors(change.id, pieces.len(), *parent, *side, *right_origin);
                self.note_into(*into, change.id, pieces.len());
                self.moves.push((change.hlc, MoveInfo { id: change.id, pieces: pieces.clone(), seen: seen.clone() }));
            }
            ChangeKind::Dismiss { notice } => {
                // Changes nothing in the text; merge_notices() checks this list.
                self.dismissals.push((change.id, *notice));
            }
            ChangeKind::SetField { record, field, value, replaces } => {
                // Changes nothing in the text; field() picks the latest value in effect.
                self.fields.entry((record.clone(), field.clone())).or_default()
                    .push(FieldValue { hlc: change.hlc, id: change.id, value: value.clone(), replaces: *replaces });
            }
            ChangeKind::Milestone { name, seen } => {
                // Changes nothing in the text; milestones() lists them.
                self.milestones.push((change.id, change.hlc, name.clone(), seen.clone()));
            }
            ChangeKind::Comment { pieces, quote, sender, body } => {
                // Changes nothing in the text; comments() works out what it covers.
                self.comments.push(CommentMade {
                    id: change.id, hlc: change.hlc, pieces: pieces.clone(),
                    quote: quote.clone(), sender: sender.clone(), body: body.clone(),
                });
            }
            ChangeKind::Reply { comment, sender, body } => {
                self.replies.push(ReplyMade { id: change.id, hlc: change.hlc, comment: *comment, sender: sender.clone(), body: body.clone() });
            }
        }
    }

    /// Remember that the `count` nodes of `change` went into the piece with anchor `into`.
    fn note_into(&mut self, into: Option<CharId>, change: ChangeId, count: usize) {
        if let Some(anchor) = into {
            let nodes = (0..count as u32).map(|offset| CharId { change, offset });
            self.typed_into.entry(anchor).or_default().extend(nodes);
        }
    }

    // ---- Local edits ----

    /// Inserts text at once, e.g. a paste. (Typing goes through `type_text`, which drafts it first.)
    pub fn insert_local(&mut self, index: usize, text: &str, now_ms: u64) -> Change {
        self.assert_no_burst();
        assert!(index <= self.len(), "index {index} is past the end of the text ({})", self.len());
        let left = self.node_before(index);
        self.commit_text(left, &[], text, None, now_ms).expect("an insertion records a Change")
    }

    pub fn delete_local(&mut self, index: usize, len: usize, now_ms: u64) -> Change {
        self.assert_no_burst();
        let end = (index + len).min(self.visible.len());
        assert!(index <= end, "index {index} is past the end of the text ({})", self.len());
        let doomed: Vec<u32> = self.visible[index..end].to_vec();
        self.commit_text(None, &doomed, "", None, now_ms).expect("a deletion records a Change")
    }

    /// Replaces `len` characters at `index` with `text` at once, e.g. a paste over a selection:
    /// one "replaced selection" Change. (Typing over a selection goes through `replace_text`.)
    pub fn replace_local(&mut self, index: usize, len: usize, text: &str, now_ms: u64) -> Change {
        self.assert_no_burst();
        let end = (index + len).min(self.visible.len());
        assert!(index <= end, "index {index} is past the end of the text ({})", self.len());
        let left = self.node_before(index);
        let doomed: Vec<u32> = self.visible[index..end].to_vec();
        self.commit_text(left, &doomed, text, None, now_ms).expect("a replacement records a Change")
    }

    /// The one place text edits become Changes: delete `doomed` (visible nodes, in reading
    /// order) and put `text` right after `left`, as an Insert, Delete or Replace. Returns None
    /// if there's nothing to record. Updates what's shown directly instead of `refresh`; in
    /// debug builds that's checked against a full recompute.
    fn commit_text(&mut self, left: Option<u32>, doomed: &[u32], text: &str, reserved: Option<ChangeId>, now_ms: u64) -> Option<Change> {
        let ranges = self.ranges_of(doomed);
        if ranges.is_empty() && text.is_empty() {
            if let Some(id) = reserved {
                self.unreserve(id); // an empty burst records nothing, not even a gap in the counters
            }
            return None;
        }
        // Where the new text goes, worked out before anything changes.
        let (left_id, into) = self.placement_after(left);
        let (parent, side, right_origin) = self.tree.place_after(left_id);
        let run = InsertRun { text: text.to_string(), parent, side, right_origin, into };
        let kind = match (ranges.is_empty(), text.is_empty()) {
            (true, _) => ChangeKind::Insert(run),
            (false, true) => ChangeKind::Delete(ranges),
            (false, false) => ChangeKind::Replace { deleted: ranges, inserted: run },
        };
        let change = self.new_change(reserved, kind, now_ms);

        // Shortcut instead of `refresh`. A deletion moves nothing: it only hides characters.
        // New text lands right after `left` in the tree, shows right after it on screen, and
        // belongs to the piece it was typed into.
        let doomed: FxHashSet<u32> = doomed.iter().copied().collect();
        let at = self.shown_index_after(left, &doomed);
        self.visible.retain(|n| !doomed.contains(n));
        self.apply_change(&change);
        let count = text.chars().count() as u32;
        if count > 0 {
            let first = self.tree.index_of(CharId { change: change.id, offset: 0 }).expect("just inserted");
            let nodes: Vec<u32> = (first..first + count).collect(); // a Change's nodes are consecutive
            self.layout.insert_typed(&nodes, into.map(|a| (a.change, a.offset as usize)));
            self.visible.splice(at..at, nodes);
        }
        self.place_burst();
        self.debug_check();
        Some(change)
    }

    /// Visible nodes (in reading order) as ranges of consecutive characters of one Change.
    fn ranges_of(&self, nodes: &[u32]) -> Vec<CharRange> {
        let mut ranges: Vec<CharRange> = Vec::new();
        for &node in nodes {
            let id = self.tree.id_of(node);
            // Extend the last range if this character follows on in the same Change.
            if let Some(last) = ranges.last_mut() {
                if last.first.change == id.change && last.first.offset + last.len == id.offset {
                    last.len += 1;
                    continue;
                }
            }
            ranges.push(CharRange { first: id, len: 1 });
        }
        ranges
    }

    /// The visible node just before `index` (None = the start).
    fn node_before(&self, index: usize) -> Option<u32> {
        index.checked_sub(1).map(|i| self.visible[i])
    }

    /// Where new text at `index` goes: the node it goes after (None = the very start),
    /// and the anchor of the piece it goes into (None = the main text).
    fn left_of(&self, index: usize) -> (Option<CharId>, Option<CharId>) {
        self.placement_after(self.node_before(index))
    }

    /// Where new text right after `node` goes in the tree. Applies the edge rule: right after
    /// the last character of a piece, the text goes after that piece's anchor instead.
    fn placement_after(&self, node: Option<u32>) -> (Option<CharId>, Option<CharId>) {
        self.placement_after_in(&self.layout, node)
    }

    /// The same, for the text as laid out by `layout` (e.g. an older version of it).
    fn placement_after_in(&self, layout: &Layout, node: Option<u32>) -> (Option<CharId>, Option<CharId>) {
        let Some(mut node) = node else { return (None, None) };
        let mut level = layout.level_at(node);
        while let Some(piece) = level {
            let span = layout.span(piece).expect("a drawn piece has a span");
            if self.tree.id_of(node) != span.last {
                break;
            }
            node = self.tree.index_of(anchor_of(piece)).expect("a drawn piece has an anchor");
            level = layout.level_at(node);
        }
        (Some(self.tree.id_of(node)), level.map(anchor_of))
    }

    /// The pieces of a Move of `nodes` (consecutive visible nodes of the text laid out by
    /// `layout`): one per run of nodes drawn in the same place.
    fn pieces_of(&self, layout: &Layout, nodes: &[u32]) -> Vec<Span> {
        let mut groups: Vec<(Option<Piece>, Span)> = Vec::new();
        for &node in nodes {
            let (id, level) = (self.tree.id_of(node), layout.level_at(node));
            if let Some((last_level, span)) = groups.last_mut() {
                if *last_level == level {
                    span.last = id;
                    continue;
                }
            }
            // `from` is the anchor of the piece this text is drawn in (None = the main text).
            groups.push((level, Span { first: id, last: id, from: level.map(anchor_of) }));
        }
        groups.into_iter().map(|(_, span)| span).collect()
    }

    /// Where text placed right after `left` shows: the number of visible characters before it,
    /// not counting `hidden` ones.
    fn shown_index_after(&self, left: Option<u32>, hidden: &FxHashSet<u32>) -> usize {
        let Some(left) = left else { return 0 };
        if let Some(i) = self.visible.iter().position(|&n| n == left) {
            return self.visible[..=i].iter().filter(|n| !hidden.contains(n)).count();
        }
        // The character is no longer visible (e.g. deleted on another Device): count what's
        // shown before its place in the reading order.
        let mut count = 0;
        for node in self.layout.reading_nodes(&self.tree) {
            if node == left {
                break;
            }
            if self.tree.is_visible_at(node) && !hidden.contains(&node) {
                count += 1;
            }
        }
        count
    }

    // ---- Typing bursts: what the editor calls ----
    //
    // Positions are in the text the Author sees (`text()`), draft included. Each call returns the
    // Changes it recorded, if any, for saving.

    /// Types `text` at `index`: into the open burst if it's there, otherwise into a new burst.
    pub fn type_text(&mut self, index: usize, text: &str, now_ms: u64) -> Vec<Change> {
        let mut recorded: Vec<Change> = self.tick(now_ms).into_iter().collect();
        assert!(index <= self.len(), "index {index} is past the end of the text ({})", self.len());
        if let Some(burst) = self.burst.as_mut().filter(|b| b.kind == BurstKind::Typing) {
            let len = burst.draft.chars().count();
            if (burst.at..=burst.at + len).contains(&index) {
                let at = byte_index(&burst.draft, index - burst.at);
                burst.draft.insert_str(at, text);
                burst.last_ms = now_ms;
                return recorded;
            }
        }
        // Somewhere else, or after deleting: typing is another kind of action.
        recorded.extend(self.close_burst(now_ms));
        self.open_burst(BurstKind::Typing, index, 0, text, now_ms);
        recorded
    }

    /// Deletes `len` characters at `index`. Inside a typing burst it just shrinks the draft.
    /// Otherwise it starts, or grows, a deletion burst: backspacing or deleting forwards at the
    /// same spot all becomes one Delete when the burst closes.
    pub fn delete_text(&mut self, index: usize, len: usize, now_ms: u64) -> Vec<Change> {
        let mut recorded: Vec<Change> = self.tick(now_ms).into_iter().collect();
        assert!(index <= self.len(), "index {index} is past the end of the text ({})", self.len());
        let len = len.min(self.len() - index);
        if len == 0 {
            return recorded;
        }
        let end = index + len;
        let mut rest = len; // what's left to delete after any part inside a typing draft
        match self.burst.as_ref().map(|b| (b.kind, b.at, b.draft.chars().count())) {
            // Right at the gap of a deletion: it grows, to the left (backspace) and/or the right.
            Some((BurstKind::Deleting, at, _)) if index <= at && at <= end => {
                let before = self.shown_nodes(index, at);
                let after = self.shown_nodes(at, end);
                let new_left = index.checked_sub(1).and_then(|i| self.shown_nodes(i, index).first().copied());
                let ids = |nodes: &[u32]| nodes.iter().map(|&n| self.tree.id_of(n)).collect::<Vec<CharId>>();
                let (before, after, new_left) = (ids(&before), ids(&after), new_left.map(|n| self.tree.id_of(n)));
                let burst = self.burst.as_mut().expect("matched above");
                if !before.is_empty() {
                    burst.left = new_left; // the gap now starts further left
                }
                burst.replacing.splice(0..0, before);
                burst.replacing.extend(after);
                burst.last_ms = now_ms;
                self.place_burst();
                return recorded;
            }
            // In a typing burst: the part inside the draft just leaves the draft.
            Some((BurstKind::Typing, at, draft_len)) => {
                let (start, stop) = (index.max(at), end.min(at + draft_len));
                let inside = stop.saturating_sub(start);
                if inside > 0 {
                    let burst = self.burst.as_mut().expect("matched above");
                    let (a, b) = (byte_index(&burst.draft, start - at), byte_index(&burst.draft, stop - at));
                    burst.draft.replace_range(a..b, "");
                    burst.last_ms = now_ms;
                }
                if inside == len {
                    return recorded; // all inside: the burst stays open
                }
                // It reaches past the draft (e.g. backspacing past its start): close the typing
                // burst and start a deletion for the rest. With the part inside the draft gone,
                // the rest starts at `index` in every case.
                rest = len - inside;
                recorded.extend(self.close_burst(now_ms));
            }
            // A deletion somewhere else: that one is done.
            Some(_) => recorded.extend(self.close_burst(now_ms)),
            None => {}
        }
        self.open_burst(BurstKind::Deleting, index, rest, "", now_ms);
        recorded
    }

    /// The visible nodes at positions start..end of the shown text, skipping what the open burst
    /// hides. Only used when no draft is showing.
    fn shown_nodes(&self, start: usize, end: usize) -> Vec<u32> {
        let hidden = self.burst.as_ref().map(|b| self.hidden_by(b)).unwrap_or_default();
        self.visible.iter().copied().filter(|n| !hidden.contains(n)).skip(start).take(end - start).collect()
    }

    /// Types `text` over the selection of `len` characters at `index`. Within the open burst
    /// it edits the draft; otherwise it opens a burst that will record a "replaced selection".
    pub fn replace_text(&mut self, index: usize, len: usize, text: &str, now_ms: u64) -> Vec<Change> {
        if len == 0 {
            return self.type_text(index, text, now_ms);
        }
        let mut recorded: Vec<Change> = self.tick(now_ms).into_iter().collect();
        if let Some(burst) = self.burst.as_mut().filter(|b| b.kind == BurstKind::Typing) {
            let draft_len = burst.draft.chars().count();
            if index >= burst.at && index + len <= burst.at + draft_len {
                let (a, b) = (byte_index(&burst.draft, index - burst.at), byte_index(&burst.draft, index + len - burst.at));
                burst.draft.replace_range(a..b, text);
                burst.last_ms = now_ms;
                return recorded;
            }
        }
        recorded.extend(self.close_burst(now_ms));
        assert!(index + len <= self.len(), "the selection reaches past the end of the text ({})", self.len());
        self.open_burst(BurstKind::Typing, index, len, text, now_ms);
        recorded
    }

    /// The cursor moved: leaving the open burst closes it.
    pub fn cursor_moved(&mut self, index: usize, now_ms: u64) -> Option<Change> {
        if let Some(change) = self.tick(now_ms) {
            return Some(change);
        }
        let burst = self.burst.as_ref()?;
        if (burst.at..=burst.at + burst.draft.chars().count()).contains(&index) {
            return None;
        }
        self.close_burst(now_ms)
    }

    /// Call regularly: closes the burst after a pause, or after a long stretch of nonstop typing.
    pub fn tick(&mut self, now_ms: u64) -> Option<Change> {
        let burst = self.burst.as_ref()?;
        let paused = now_ms.saturating_sub(burst.last_ms) >= BURST_PAUSE_MS;
        let too_long = now_ms.saturating_sub(burst.opened_ms) >= BURST_MAX_MS;
        if paused || too_long { self.close_burst(now_ms) } else { None }
    }

    /// Closes the open burst (also when another kind of action starts, or the Work closes),
    /// recording its Change. Its time is set now.
    pub fn close_burst(&mut self, now_ms: u64) -> Option<Change> {
        let burst = self.burst.take()?;
        let left = burst.left.map(|id| self.tree.index_of(id).expect("a burst's character stays in the tree"));
        let replacing: FxHashSet<u32> = burst.replacing.iter().filter_map(|&id| self.tree.index_of(id)).collect();
        let doomed: Vec<u32> = self.visible.iter().copied().filter(|n| replacing.contains(n)).collect();
        self.commit_text(left, &doomed, &burst.draft, Some(burst.id), now_ms)
    }

    /// Drops the open burst without recording anything: its draft disappears and whatever it
    /// hid shows again. (Undo inside a burst, or a cut that's undone.)
    pub fn cancel_burst(&mut self) {
        if let Some(burst) = self.burst.take() {
            self.unreserve(burst.id);
        }
    }

    /// Opens a burst again from what a recovery file kept of it, after the app stopped with it
    /// open. Its reserved ID is taken again. False if it can't be: a burst is open, its Change is
    /// already in the History (it was saved after all), or a character it's anchored to isn't.
    #[allow(clippy::too_many_arguments)]
    pub fn resume_burst(&mut self, kind: BurstKind, id: ChangeId, left: Option<CharId>, replacing: Vec<CharId>,
                        draft: String, opened_ms: u64, last_ms: u64) -> bool {
        let known = |c: &CharId| self.tree.index_of(*c).is_some();
        if self.burst.is_some() || id.device != self.device || self.has(id) || !left.iter().all(known) || !replacing.iter().all(known) {
            return false;
        }
        self.next_counter = self.next_counter.max(id.counter + 1);
        self.burst = Some(Burst { kind, id, left, replacing, draft, opened_ms, last_ms, at: 0, hidden: 0 });
        self.place_burst();
        true
    }

    /// Is the open burst a typing burst that would put back exactly what it hides? (E.g. typing
    /// over a selection, then undoing back to the start.) Closing it would record a pointless
    /// replacement, so the caller cancels it instead.
    pub fn burst_changes_nothing(&self) -> bool {
        let Some(burst) = self.burst.as_ref().filter(|b| b.kind == BurstKind::Typing) else { return false };
        let hidden = self.hidden_by(burst);
        let hidden_text: String = self.visible.iter().filter(|n| hidden.contains(n)).filter_map(|&n| self.tree.char_at(n)).collect();
        hidden_text == burst.draft
    }

    /// Turns the open deletion burst (a cut) into a Move of the text it hides, so the text keeps
    /// its History. `to` is where it's pasted, in the shown text (where the cut text is absent).
    pub fn cut_to_move(&mut self, to: usize, now_ms: u64) -> CutPasted {
        let burst = self.burst.as_ref().filter(|b| b.kind == BurstKind::Deleting).expect("a cut is open (a deletion burst)");
        assert!(to <= self.len(), "to {to} is past the end of the text ({})", self.len());
        let hidden = self.hidden_by(burst);
        let positions: Vec<usize> = self.visible.iter().enumerate().filter(|(_, n)| hidden.contains(n)).map(|(i, _)| i).collect();
        let whole = positions.last().is_some_and(|&last| last + 1 - positions[0] == positions.len());
        if !whole {
            // Something was merged into the middle of it: it's no longer one stretch of text.
            return CutPasted::Deleted(self.close_burst(now_ms));
        }
        let (from, len) = (positions[0], positions.len());
        self.cancel_burst();
        if to == from {
            return CutPasted::Unchanged; // pasted back where it was
        }
        let to = if to < from { to } else { to + len }; // in the text with the cut text back
        CutPasted::Moved(self.move_local(from, len, to, now_ms))
    }

    /// Opens a burst at `index` (no burst may be open), hiding the `replacing` characters there.
    fn open_burst(&mut self, kind: BurstKind, index: usize, replacing: usize, text: &str, now_ms: u64) {
        let left = self.node_before(index).map(|n| self.tree.id_of(n));
        let replacing: Vec<CharId> = self.visible[index..index + replacing].iter().map(|&n| self.tree.id_of(n)).collect();
        let id = self.reserve_id();
        self.burst = Some(Burst {
            kind, id, left, replacing,
            draft: text.to_string(),
            opened_ms: now_ms,
            last_ms: now_ms,
            at: index,
            hidden: 0,
        });
        self.place_burst();
    }

    /// Works out where the open burst shows, from the character it follows.
    fn place_burst(&mut self) {
        let Some(burst) = &self.burst else { return };
        let hidden = self.hidden_by(burst);
        let left = burst.left.map(|id| self.tree.index_of(id).expect("a burst's character stays in the tree"));
        let at = self.shown_index_after(left, &hidden);
        let burst = self.burst.as_mut().expect("checked above");
        burst.at = at;
        burst.hidden = hidden.len();
    }

    /// The visible nodes the burst's selection hides.
    fn hidden_by(&self, burst: &Burst) -> FxHashSet<u32> {
        burst.replacing.iter()
            .filter_map(|&id| self.tree.index_of(id))
            .filter(|&n| self.tree.is_visible_at(n))
            .collect()
    }

    fn assert_no_burst(&self) {
        assert!(self.burst.is_none(), "close the typing burst first: another kind of action closes it");
    }

    pub fn move_local(&mut self, from: usize, len: usize, to: usize, now_ms: u64) -> Change {
        self.assert_no_burst();
        assert!(len > 0, "len must be greater than 0");
        assert!(from + len <= self.len(), "from + len must be less than or equal to the text length");
        assert!(to <= from || to >= from + len, "to must be less than or equal to from or greater than or equal to from + len");
        assert!(to <= self.len(), "to {to} is past the end of the text ({})", self.len());

        // One piece per run of selected characters drawn in the same place.
        let pieces = self.pieces_of(&self.layout, &self.visible[from..from + len]);

        let (left, into) = self.left_of(to);
        let (parent, side, right_origin) = self.tree.place_after(left);

        let mut seen: Vec<(DeviceId, u32)> = self.seen.iter().map(|(&device, &counter)| (device, counter)).collect();
        seen.sort();

        self.commit_local(ChangeKind::Move { pieces, parent, side, right_origin, seen, into }, now_ms)
    }

    pub fn revert_local(&mut self, target: ChangeId, with_dependents: bool, now_ms: u64) -> Change {
        let mut targets = vec![target];
        if with_dependents {
            targets.extend(self.depends_on(target));
        }
        self.commit_local(ChangeKind::Revert { targets }, now_ms)
    }

    pub fn restore_local(&mut self, target: ChangeId, now_ms: u64) -> Option<Change> {
        let reverters = self.reverted_by.get(&target).cloned().unwrap_or_default();
        let in_effect: Vec<ChangeId> = reverters.into_iter().filter(|&r| self.is_in_effect(r)).collect();
        if in_effect.is_empty() {
            None
        } else {
            Some(self.commit_local(ChangeKind::Revert { targets: in_effect }, now_ms))
        }
    }

    pub fn dismiss_local(&mut self, notice: NoticeKey, now_ms: u64) -> Change {
        self.commit_local(ChangeKind::Dismiss { notice }, now_ms)
    }

    /// Sets a field of a record, such as a Device's name.
    pub fn set_field_local(&mut self, record: &str, field: &str, value: &str, now_ms: u64) -> Change {
        let replaces = self.field_value(record, field).map(|v| v.id);
        let kind = ChangeKind::SetField { record: record.to_string(), field: field.to_string(), value: value.to_string(), replaces };
        self.commit_local(kind, now_ms)
    }

    /// A Merge notice about a free-text field: which field it is, and the version that lost.
    pub fn concurrent_field(&self, key: NoticeKey) -> Option<(String, String, String)> {
        let NoticeKey::ConcurrentField { later, earlier } = key else { return None };
        let (record, field, _, _) = self.concurrent_fields().into_iter().find(|(_, _, l, e)| *l == later && *e == earlier)?;
        let lost = self.field_set_by(&record, &field, earlier)?.to_string();
        Some((record, field, lost))
    }

    // ---- Milestones ----

    /// Names the current point in the History: exactly the Changes this Device has seen.
    pub fn make_milestone_local(&mut self, name: &str, now_ms: u64) -> Change {
        let mut seen: Vec<(DeviceId, u32)> = self.seen.iter().map(|(&d, &c)| (d, c)).collect();
        seen.sort();
        self.commit_local(ChangeKind::Milestone { name: name.to_string(), seen }, now_ms)
    }

    pub fn rename_milestone_local(&mut self, milestone: ChangeId, name: &str, now_ms: u64) -> Change {
        self.set_field_local(&milestone_record(milestone), "name", name, now_ms)
    }

    /// The Milestones that haven't been deleted (reverted), in the fixed Change order,
    /// with their current names.
    pub fn milestones(&self) -> Vec<Milestone> {
        let mut list: Vec<Milestone> = self.milestones.iter()
            .filter(|(id, ..)| self.is_in_effect(*id))
            .map(|(id, hlc, name, seen)| Milestone {
                id: *id,
                hlc: *hlc,
                name: self.field(&milestone_record(*id), "name").unwrap_or(name).to_string(),
                seen: seen.clone(),
            })
            .collect();
        list.sort_by_key(|m| (m.hlc, m.id.device));
        list
    }

    fn milestone(&self, id: ChangeId) -> &Vec<(DeviceId, u32)> {
        self.milestones.iter().find(|(m, ..)| *m == id).map(|(_, _, _, seen)| seen).expect("no such Milestone")
    }

    /// Is `change` part of Milestone `milestone`?
    pub fn in_milestone(&self, milestone: ChangeId, change: ChangeId) -> bool {
        holds(self.milestone(milestone), change)
    }

    /// The text as it was at a Milestone, without changing anything.
    pub fn text_at_milestone(&self, milestone: ChangeId) -> String {
        let seen = self.milestone(milestone);
        self.text_among(&|c| holds(seen, c))
    }

    /// What returning to a Milestone would revert: every content Change that isn't part of it.
    /// `made_earlier_elsewhere` are the ones made before the Milestone on another Device (which
    /// that Device simply hadn't seen yet), so the Author can choose to keep them.
    pub fn return_preview(&self, milestone: ChangeId) -> ReturnPreview {
        let seen = self.milestone(milestone);
        let (hlc, device) = self.milestones.iter().find(|(m, ..)| *m == milestone).map(|(m, hlc, ..)| (*hlc, m.device)).expect("checked above");
        // In the fixed Change order (not the order this Device happened to receive them).
        let mut outside: Vec<(ChangeId, Hlc)> = self.content.iter().copied().filter(|&(id, _)| !holds(seen, id)).collect();
        outside.sort_by_key(|&(id, h)| (h, id.device));
        let made_earlier_elsewhere = outside.iter()
            .filter(|&&(id, h)| h < hlc && id.device != device)
            .map(|&(id, _)| id)
            .collect();
        let reverts = outside.into_iter().map(|(id, _)| id).collect();
        ReturnPreview { reverts, made_earlier_elsewhere }
    }

    /// Returns the Work's content to a Milestone: one Revert of every content Change that isn't
    /// part of it, except those in `keep`. Nothing is removed, and the Revert can itself be
    /// reverted. None if there's nothing to revert.
    pub fn return_to_milestone_local(&mut self, milestone: ChangeId, keep: &[ChangeId], now_ms: u64) -> Option<Change> {
        let targets: Vec<ChangeId> = self.return_preview(milestone).reverts.into_iter()
            .filter(|id| !keep.contains(id))
            .collect();
        if targets.is_empty() {
            return None;
        }
        Some(self.commit_local(ChangeKind::Revert { targets }, now_ms))
    }

    // ---- Comments ----

    /// Adds a Comment on the `len` characters at `index` of the text (no burst may be open; the
    /// caller closes it first). The first body and sender are part of the Change itself.
    pub fn comment_local(&mut self, index: usize, len: usize, sender: &str, body: &str, now_ms: u64) -> Change {
        self.assert_no_burst();
        assert!(len > 0, "a Comment needs some text to attach to");
        assert!(index + len <= self.len(), "the selection reaches past the end of the text ({})", self.len());
        let selected = &self.visible[index..index + len];
        let quote: String = selected.iter().filter_map(|&n| self.tree.char_at(n)).collect();
        let pieces = self.comment_pieces(selected);
        self.commit_local(ChangeKind::Comment { pieces, quote, sender: sender.to_string(), body: body.to_string() }, now_ms)
    }

    /// Ranges in the tree's fixed order that hold exactly the `selected` nodes: a new piece
    /// starts wherever a visible character that wasn't selected lies between them in the tree
    /// (such as text Moved out of the passage earlier, now shown elsewhere). Deleted characters
    /// and anchors don't split a piece; text later typed inside a piece is covered by it.
    fn comment_pieces(&self, selected: &[u32]) -> Vec<Span> {
        let order = self.tree.order();
        let pos = positions(&order, self.tree.len());
        let chosen: FxHashSet<u32> = selected.iter().copied().collect();
        let lo = selected.iter().map(|&n| pos[n as usize]).min().expect("a Comment covers some text") as usize;
        let hi = selected.iter().map(|&n| pos[n as usize]).max().expect("a Comment covers some text") as usize;
        let mut pieces: Vec<Span> = Vec::new();
        let mut open: Option<Span> = None;
        for &node in &order[lo..=hi] {
            if chosen.contains(&node) {
                let id = self.tree.id_of(node);
                match open.as_mut() {
                    Some(span) => span.last = id,
                    None => open = Some(Span { first: id, last: id, from: None }),
                }
            } else if self.tree.is_visible_at(node) {
                pieces.extend(open.take());
            }
        }
        pieces.extend(open);
        pieces
    }

    /// Replies to a Comment.
    pub fn reply_local(&mut self, comment: ChangeId, sender: &str, body: &str, now_ms: u64) -> Change {
        assert!(self.comments.iter().any(|c| c.id == comment), "no such Comment");
        self.commit_local(ChangeKind::Reply { comment, sender: sender.to_string(), body: body.to_string() }, now_ms)
    }

    /// Changes the body of a Comment or Reply.
    pub fn edit_comment_local(&mut self, id: ChangeId, body: &str, now_ms: u64) -> Change {
        self.set_field_local(&comment_record(id), "body", body, now_ms)
    }

    /// Changes who sent a Comment or Reply.
    pub fn set_comment_sender_local(&mut self, id: ChangeId, sender: &str, now_ms: u64) -> Change {
        self.set_field_local(&comment_record(id), "sender", sender, now_ms)
    }

    /// Resolves a Comment, or opens it again.
    pub fn resolve_comment_local(&mut self, id: ChangeId, resolved: bool, now_ms: u64) -> Change {
        self.set_field_local(&comment_record(id), "resolved", if resolved { "yes" } else { "no" }, now_ms)
    }

    /// The Comments not deleted (reverted), as they are now: Comments with visible text in text
    /// order, then Detached comments in the fixed Change order. Every Device gets the same list.
    /// A deleted Comment's Replies go with it.
    pub fn comments(&self) -> Vec<CommentView> {
        if self.comments.is_empty() {
            return Vec::new(); // most Works: no need to walk the tree
        }
        let order = self.tree.order();
        let pos = positions(&order, self.tree.len());
        let shown = self.shown_positions();
        let mut list: Vec<CommentView> = self.comments.iter()
            .filter(|c| self.is_in_effect(c.id))
            .map(|c| {
                // Every visible character in its pieces' ranges, as positions in the shown text.
                let mut at: Vec<u32> = Vec::new();
                for span in &c.pieces {
                    let (Some(first), Some(last)) = (self.tree.index_of(span.first), self.tree.index_of(span.last)) else { continue };
                    let (lo, hi) = (pos[first as usize] as usize, pos[last as usize] as usize);
                    if lo <= hi {
                        at.extend(order[lo..=hi].iter().map(|&n| shown[n as usize]).filter(|&p| p != NOT_SHOWN));
                    }
                }
                at.sort_unstable();
                at.dedup();
                // The open burst's draft isn't in the tree yet: typed between two characters the
                // Comment covers, it will be covered once it's recorded, so it's shown that way now.
                if let Some(b) = self.burst.as_ref().filter(|b| !b.draft.is_empty() && b.at > 0) {
                    let (start, len) = (b.at as u32, b.draft.chars().count() as u32);
                    if at.binary_search(&(start - 1)).is_ok() && at.binary_search(&(start + len)).is_ok() {
                        let i = at.partition_point(|&p| p < start);
                        at.splice(i..i, start..start + len);
                    }
                }
                let mut ranges: Vec<(usize, usize)> = Vec::new();
                for p in at.into_iter().map(|p| p as usize) {
                    match ranges.last_mut() {
                        Some((_, end)) if *end == p => *end = p + 1,
                        _ => ranges.push((p, p + 1)),
                    }
                }
                let record = comment_record(c.id);
                let mut replies: Vec<&ReplyMade> = self.replies.iter().filter(|r| r.comment == c.id && self.is_in_effect(r.id)).collect();
                replies.sort_by_key(|r| (r.hlc, r.id.device));
                CommentView {
                    id: c.id,
                    hlc: c.hlc,
                    sender: self.field(&record, "sender").unwrap_or(&c.sender).to_string(),
                    body: self.field(&record, "body").unwrap_or(&c.body).to_string(),
                    resolved: self.field(&record, "resolved") == Some("yes"),
                    quote: c.quote.clone(),
                    ranges,
                    replies: replies.into_iter().map(|r| {
                        let record = comment_record(r.id);
                        ReplyView {
                            id: r.id,
                            hlc: r.hlc,
                            sender: self.field(&record, "sender").unwrap_or(&r.sender).to_string(),
                            body: self.field(&record, "body").unwrap_or(&r.body).to_string(),
                        }
                    }).collect(),
                }
            })
            .collect();
        // Attached ones by where they start; Detached ones after them; ties in the fixed Change order.
        list.sort_by_key(|c| (c.ranges.first().map_or(usize::MAX, |r| r.0), c.hlc, c.id.device));
        list
    }

    /// Per tree node: its position in the text the Author sees (open burst included), or
    /// NOT_SHOWN if it isn't shown (deleted, an anchor, or hidden by the open burst).
    fn shown_positions(&self) -> Vec<u32> {
        let mut shown = vec![NOT_SHOWN; self.tree.len()];
        let hidden = self.burst.as_ref().map(|b| self.hidden_by(b)).unwrap_or_default();
        // The draft shows just before the character at `at`, pushing it and the rest along.
        let (at, draft) = self.burst.as_ref().map_or((usize::MAX, 0), |b| (b.at, b.draft.chars().count()));
        let mut i = 0;
        for &node in self.visible.iter().filter(|n| !hidden.contains(n)) {
            shown[node as usize] = (if i >= at { i + draft } else { i }) as u32;
            i += 1;
        }
        shown
    }

    /// The text if only the Changes `member` accepts had ever happened: Reverts outside the set
    /// don't count, and nor do deletions or Moves outside it.
    fn text_among(&self, member: &dyn Fn(ChangeId) -> bool) -> String {
        let (_, nodes) = self.view_among(member);
        nodes.into_iter().filter_map(|n| self.tree.char_at(n)).collect()
    }

    /// The layout and the visible nodes (in reading order) if only the Changes `member` accepts
    /// had ever happened.
    fn view_among(&self, member: &dyn Fn(ChangeId) -> bool) -> (Layout, Vec<u32>) {
        let in_effect = |id: ChangeId| member(id) && self.is_in_effect_among(id, member);
        let mut moves: Vec<(Hlc, MoveInfo)> = self.moves.iter().filter(|(_, m)| in_effect(m.id)).cloned().collect();
        moves.sort_by_key(|(hlc, m)| (*hlc, m.id.device));
        let moves: Vec<MoveInfo> = moves.into_iter().map(|(_, m)| m).collect();
        let layout = Layout::compute(&self.tree, &moves, &self.typed_into);
        let nodes = layout.reading_nodes(&self.tree).into_iter()
            .filter(|&n| in_effect(self.tree.change_at(n)) && !self.tree.deleters_at(n).iter().any(|&d| in_effect(d)))
            .collect();
        (layout, nodes)
    }

    // ---- External edits ----

    /// Records an edit made to the Text file outside the app.
    ///
    /// `base` is the version the file was written from (the highest Change counter it held from
    /// each Device). The file is compared with the text of that version, not the current text,
    /// so edits made in the app since then are kept, and the External edit merges like Changes
    /// from an offline Device. None if no written version is known: then it's compared with the
    /// current text, and the Author gets a Merge notice.
    ///
    /// A block deleted in one place and found unchanged in another becomes a Move, so its
    /// History goes with it. Everything else becomes one deletion and insertions. All of it is
    /// one new Session, labelled "External edit". Returns the Changes recorded.
    pub fn import_text(&mut self, base: Option<&[(DeviceId, u32)]>, file_text: &str, now_ms: u64) -> Vec<Change> {
        self.assert_no_burst();
        let known = base.is_some();
        let base: Vec<(DeviceId, u32)> = base.map_or_else(|| self.version(), |b| b.to_vec());
        let (layout, nodes) = {
            let base = &base;
            self.view_among(&|c| holds(base, c))
        };
        let old: Vec<char> = nodes.iter().filter_map(|&n| self.tree.char_at(n)).collect();
        // Editors may add a byte order mark or Windows line endings; the Work's text has neither.
        let new: Vec<char> = file_text.strip_prefix('\u{feff}').unwrap_or(file_text).replace("\r\n", "\n").chars().collect();
        let plan = text_file::plan(&old, &new);
        if plan.hunks.is_empty() {
            return Vec::new(); // nothing was edited outside the app
        }

        self.session = None; // an External edit is a Session of its own
        let mut recorded: Vec<Change> = Vec::new();
        // Deleted outright: one deletion.
        let doomed: Vec<u32> = plan.deleted.iter().flat_map(|&(s, l)| nodes[s..s + l].iter().copied()).collect();
        if !doomed.is_empty() {
            let kind = ChangeKind::Delete(self.ranges_of(&doomed));
            recorded.push(self.import_change(kind, now_ms));
        }
        // Each hunk's new text, in order: inserted text and moved blocks, placed one after the
        // other, starting right after the unchanged text before the hunk (as it was in `base`).
        for (h, hunk) in plan.hunks.iter().enumerate() {
            let mut parts: Vec<(usize, Option<(usize, usize)>)> = plan.inserted.iter()
                .filter(|&&(ih, ..)| ih == h)
                .map(|&(_, start, len)| (start, None))
                .chain(plan.moved.iter().filter(|m| m.hunk == h).map(|m| (m.new_start, Some((m.old_start, m.len)))))
                .collect();
            parts.sort_by_key(|&(start, _)| start);
            let (mut left, into) = self.placement_after_in(&layout, hunk.old_start.checked_sub(1).map(|i| nodes[i]));
            for (start, moved) in parts {
                let (parent, side, right_origin) = self.tree.place_after(left);
                match moved {
                    None => {
                        let len = plan.inserted.iter().find(|&&(_, s, _)| s == start).expect("listed above").2;
                        let text: String = new[start..start + len].iter().collect();
                        let change = self.import_change(ChangeKind::Insert(InsertRun { text, parent, side, right_origin, into }), now_ms);
                        left = Some(CharId { change: change.id, offset: len as u32 - 1 });
                        recorded.push(change);
                    }
                    Some((old_start, len)) => {
                        let pieces = self.pieces_of(&layout, &nodes[old_start..old_start + len]);
                        let count = pieces.len() as u32;
                        // It only knew `base`: it takes text from Moves made since, like an offline Device would.
                        let kind = ChangeKind::Move { pieces, parent, side, right_origin, seen: base.clone(), into };
                        let change = self.import_change(kind, now_ms);
                        left = Some(CharId { change: change.id, offset: count - 1 }); // after its last anchor
                        recorded.push(change);
                    }
                }
            }
        }
        // Label the Session, for the History view (and the Merge notice if the version was unknown).
        let record = session_record(recorded[0].session);
        recorded.push(self.import_change(ChangeKind::SetField { record: record.clone(), field: "source".into(), value: "External edit".into(), replaces: None }, now_ms));
        if !known {
            recorded.push(self.import_change(ChangeKind::SetField { record, field: "base".into(), value: "unknown".into(), replaces: None }, now_ms));
        }
        self.session = None; // what's typed next is a new Session
        self.refresh();
        recorded
    }

    /// Makes and applies one Change of an External edit (the text is worked out once, at the end).
    fn import_change(&mut self, kind: ChangeKind, now_ms: u64) -> Change {
        let change = self.new_change(None, kind, now_ms);
        self.apply_change(&change);
        change
    }

    /// Like `is_in_effect`, counting only Reverts that `member` accepts.
    fn is_in_effect_among(&self, id: ChangeId, member: &dyn Fn(ChangeId) -> bool) -> bool {
        match self.reverted_by.get(&id) {
            Some(reverters) => reverters.iter().filter(|&&r| member(r)).all(|&r| !self.is_in_effect_among(r, member)),
            None => true,
        }
    }

    /// Gives this Device a readable name, shown in the History.
    pub fn set_device_name_local(&mut self, name: &str, now_ms: u64) -> Change {
        let record = device_record(self.device);
        self.set_field_local(&record, "name", name, now_ms)
    }

    /// Takes the next Change counter for this Device.
    fn reserve_id(&mut self) -> ChangeId {
        let id = ChangeId { device: self.device, counter: self.next_counter };
        self.next_counter += 1;
        id
    }

    /// Gives back a reserved ID that was never used, if nothing has taken a counter since.
    fn unreserve(&mut self, id: ChangeId) {
        if self.next_counter == id.counter + 1 {
            self.next_counter = id.counter;
        }
    }

    /// Makes a new Change from this Device: a reserved ID or the next one, a fresh clock
    /// value, and its Session.
    fn new_change(&mut self, reserved: Option<ChangeId>, kind: ChangeKind, now_ms: u64) -> Change {
        let id = reserved.unwrap_or_else(|| self.reserve_id());
        // Same Session unless this is the first Change since the Work was opened, or there
        // was a long pause. (A clock that jumped backwards counts as no pause.)
        let session = match self.session {
            Some((first, last)) if now_ms.saturating_sub(last) < SESSION_GAP_MS => first,
            _ => id,
        };
        self.session = Some((session, now_ms));
        Change { id, hlc: self.clock.tick(now_ms), session, kind }
    }

    /// Makes a new Change and applies it the general way.
    fn commit_local(&mut self, kind: ChangeKind, now_ms: u64) -> Change {
        self.assert_no_burst();
        let change = self.new_change(None, kind, now_ms);
        self.apply(&change);
        change
    }
}

/// A node that isn't shown, in `Doc::shown_positions`.
const NOT_SHOWN: u32 = u32::MAX;

/// Per tree node: its position in `order` (the tree's fixed order).
fn positions(order: &[u32], len: usize) -> Vec<u32> {
    let mut pos = vec![0u32; len];
    for (i, &node) in order.iter().enumerate() {
        pos[node as usize] = i as u32;
    }
    pos
}

/// The byte position of the `chars`-th character of `s` (or its end).
fn byte_index(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ids::Side;

    #[test]
    fn test_doc_creation() {
        let device_id = DeviceId(1);
        let doc = Doc::new(device_id);
        assert_eq!(doc.device, device_id);
        assert_eq!(doc.next_counter, 1);
        assert_eq!(doc.clock, Hlc(0));
        assert_eq!(doc.text(), "");
    }

    #[test]
    fn test_insert_at_zero() {
        let mut doc = Doc::new(DeviceId(1));
        let change = doc.insert_local(0, "Hello", 1000);
        assert_eq!(doc.text(), "Hello");
        assert_eq!(change.id.device, DeviceId(1));
        assert_eq!(change.id.counter, 1);
        assert_eq!(change.hlc, Hlc(1000));
        match change.kind {
            ChangeKind::Insert(run) => {
                assert_eq!(run.text, "Hello");
                assert_eq!(run.parent, None);
                assert_eq!(run.side, Side::Right);
                assert_eq!(run.right_origin, None);
            }
            _ => panic!("Expected Insert change"),
        }
    }

    #[test]
    fn delete_across_two_changes() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "cat", 1000);
        doc.insert_local(1, "h", 2000);              // "chat": h is from a different Change
        let change = doc.delete_local(1, 2, 3000);   // delete "ha"
        assert_eq!(doc.text(), "ct");
        match change.kind {
            ChangeKind::Delete(ranges) => assert_eq!(ranges.len(), 2),
            _ => panic!("expected a Delete"),
        }
    }

    #[test]
    fn applying_twice_is_harmless() {
        let mut doc = Doc::new(DeviceId(1));
        let change = doc.insert_local(0, "cat", 1000);
        doc.insert_local(3, "s", 2000);
        doc.apply(&change);
        assert_eq!(doc.text(), "cats");
    }

    #[test]
    fn two_doc() {
        // doc 1 types cat and doc 2 applies that change.
        // eahc types at the end without seeing the others text
        // each applies the others change
        // the texts must be equal.
        let mut doc1 = Doc::new(DeviceId(1));
        let mut doc2 = Doc::new(DeviceId(2));

        let change1 = doc1.insert_local(0, "cat", 1000);
        let change2 = doc2.insert_local(0, "dog", 1000);

        doc1.apply(&change2);
        doc2.apply(&change1);

        assert_eq!(doc1.text(), doc2.text());
    }

    #[test]
    fn loading_continues_the_counter() {
        let mut doc = Doc::new(DeviceId(1));
        let first = doc.insert_local(0, "cat", 1000);

        let mut reloaded = Doc::new(DeviceId(1)); // the app restarted
        reloaded.apply(&first);                   // loaded from the .db
        let next = reloaded.insert_local(3, "s", 2000);

        assert_ne!(next.id, first.id);
    }

    #[test]
        fn revert_insert_hides_it() {
            let mut doc = Doc::new(DeviceId(1));
            doc.insert_local(0, "cat", 1000);
            let sat = doc.insert_local(3, " sat", 2000);
            doc.revert_local(sat.id, false, 3000);
            assert_eq!(doc.text(), "cat");
            assert!(!doc.is_in_effect(sat.id));
        }
        #[test]
    fn revert_delete_brings_it_back() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "cat", 1000);
        let del = doc.delete_local(1, 1, 2000); // "ct"
        doc.revert_local(del.id, false, 3000);
        assert_eq!(doc.text(), "cat");
    }

    #[test]
    fn deleted_twice_stays_deleted_after_one_revert() {
        let mut a = Doc::new(DeviceId(1));
        let mut b = Doc::new(DeviceId(2));
        let cat = a.insert_local(0, "cat", 1000);
        b.apply(&cat);
        let del_a = a.delete_local(1, 1, 2000); // both delete the "a" offline
        let del_b = b.delete_local(1, 1, 2000);
        a.apply(&del_b);
        a.revert_local(del_a.id, false, 3000);
        assert_eq!(a.text(), "ct"); // b's deletion still holds
    }

    #[test]
    fn reverting_a_revert_restores() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "cat", 1000);
        let sat = doc.insert_local(3, " sat", 2000);
        let r = doc.revert_local(sat.id, false, 3000);
        doc.revert_local(r.id, false, 4000);
        assert_eq!(doc.text(), "cat sat");
    }

    #[test]
    fn restore_undoes_reverts_from_two_devices() {
        let mut a = Doc::new(DeviceId(1));
        let mut b = Doc::new(DeviceId(2));
        let cat = a.insert_local(0, "cat", 1000);
        b.apply(&cat);
        a.revert_local(cat.id, false, 2000); // both revert it offline
        let rb = b.revert_local(cat.id, false, 2000);
        a.apply(&rb);
        assert_eq!(a.text(), "");
        a.restore_local(cat.id, 3000).expect("cat had Reverts to undo");
        assert_eq!(a.text(), "cat");
    }

    #[test]
    fn reverting_one_character_with_dependents() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "cat", 1000);
        let s = doc.insert_local(3, "s", 2000);   // a one-character Change
        doc.revert_local(s.id, true, 3000);
        assert_eq!(doc.text(), "cat");
    }

    fn abcdef() -> Doc {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abcdef", 1000);
        doc
    }

    #[test]
    fn move_cd_to_the_end() {
        let mut doc = abcdef();
        doc.move_local(2, 2, 6, 2000);
        assert_eq!(doc.text(), "abefcd");
    }

    #[test]
    fn typing_inside_moved_text_stays_inside() {
        let mut doc = abcdef();
        doc.move_local(2, 2, 6, 2000);         // "abefcd"
        doc.insert_local(5, "X", 3000);        // between c and d
        assert_eq!(doc.text(), "abefcXd");
    }

    #[test]
    fn typing_right_after_moved_text_shows_there() {
        let mut doc = abcdef();
        doc.move_local(2, 2, 6, 2000);         // "abefcd"
        doc.insert_local(6, "X", 3000);        // after d, which is the edge rule
        assert_eq!(doc.text(), "abefcdX");     // without the rule: "abXefcd"
    }

    #[test]
    fn moving_part_of_moved_text() {
        let mut doc = abcdef();
        doc.move_local(2, 2, 6, 2000);         // "abefcd"
        doc.move_local(5, 1, 0, 3000);         // "d" to the start
        assert_eq!(doc.text(), "dabefc");
        assert!(doc.merge_notices().is_empty()); // one Device alone never needs a notice
    }

    #[test]
    fn a_later_piece_does_not_take_an_earlier_piece_s_text() {
        // Found by the property test. After "c" moves to the start, the tree order is still
        // a b c d. Selecting "cabd" makes two pieces: [c] and [a..d]. The second piece's range
        // covers c in the tree, but c already belongs to the first piece of the same Move.
        let mut doc = abcdef();
        doc.move_local(2, 1, 0, 2000);         // "cabdef"
        doc.move_local(0, 4, 6, 3000);         // "cabd" to the end
        assert_eq!(doc.text(), "efcabd");
    }

    #[test]
    fn a_piece_only_takes_from_the_piece_it_was_selected_from() {
        // Also found by the property test. The Move of "cabd" has two pieces: [c] and [a..d].
        // Moving "abd" (the second piece's text) must not take c, which sits inside a..d in
        // the tree but belongs to the first piece.
        let mut doc = abcdef();
        doc.move_local(2, 1, 0, 2000);         // "cabdef"
        doc.move_local(0, 4, 6, 3000);         // "efcabd"
        doc.move_local(3, 3, 0, 4000);         // "abd" to the start
        assert_eq!(doc.text(), "abdefc");
    }

    #[test]
    fn text_typed_into_a_piece_stays_in_that_piece() {
        // Found by the property test: three overlapping Moves, then typing. The "b" lands in
        // the tree right after a space that the oldest Move's range also covers, so without
        // `into` that Move would take the "b" and draw it somewhere else.
        let mut phone = Doc::new(DeviceId(2));
        let mut laptop = Doc::new(DeviceId(0));
        let mut tablet = Doc::new(DeviceId(1));
        let base = phone.insert_local(0, "aa  a", 1000);
        let first = phone.move_local(2, 2, 5, 2000);   // "aaa  "
        for doc in [&mut laptop, &mut tablet] {
            doc.apply(&base);
            doc.apply(&first);
        }
        laptop.move_local(1, 3, 4, 3000);              // laptop and tablet both move, offline
        let other = tablet.move_local(0, 3, 3, 3000);
        laptop.apply(&other);
        assert_eq!(laptop.text(), "aa a ");
        laptop.insert_local(3, "b", 4000);
        assert_eq!(laptop.text(), "aa ba ");
    }

    #[test]
    fn a_selection_across_two_places_becomes_two_pieces() {
        let mut doc = abcdef();
        doc.move_local(2, 2, 6, 2000);         // "abefcd"
        doc.insert_local(6, "X", 3000);        // "abefcdX"
        let mv = doc.move_local(4, 3, 0, 4000); // "cdX" to the start
        assert_eq!(doc.text(), "cdXabef");
        match mv.kind {
            ChangeKind::Move { pieces, .. } => assert_eq!(pieces.len(), 2),
            _ => panic!("expected a Move"),
        }
    }

    #[test]
    fn text_moved_out_earlier_stays_out() {
        let mut doc = abcdef();
        doc.move_local(2, 2, 6, 2000);         // "abefcd"
        doc.move_local(0, 4, 6, 3000);         // "abef" to the end
        assert_eq!(doc.text(), "cdabef");
        assert!(doc.merge_notices().is_empty()); // the second Move knew about the first
    }

    #[test]
    fn reverting_a_move_puts_it_back() {
        let mut doc = abcdef();
        let mv = doc.move_local(2, 2, 6, 2000);
        doc.revert_local(mv.id, false, 3000);
        assert_eq!(doc.text(), "abcdef");
    }

    /// The laptop types "abcdef" and the phone receives it.
    fn laptop_and_phone() -> (Doc, Doc) {
        let mut laptop = Doc::new(DeviceId(1));
        let mut phone = Doc::new(DeviceId(2));
        let base = laptop.insert_local(0, "abcdef", 1000);
        phone.apply(&base);
        (laptop, phone)
    }

    #[test]
    fn text_typed_inside_on_another_device_follows_the_move() {
        let (mut laptop, mut phone) = laptop_and_phone();
        let mv = laptop.move_local(2, 2, 6, 2000);
        let typed = phone.insert_local(3, "X", 2000); // between c and d
        laptop.apply(&typed);
        phone.apply(&mv);
        assert_eq!(laptop.text(), "abefcXd");
        assert_eq!(phone.text(), "abefcXd");
    }

    #[test]
    fn a_deletion_wins_wherever_the_text_went() {
        let (mut laptop, mut phone) = laptop_and_phone();
        let mv = laptop.move_local(2, 2, 6, 2000);
        let del = phone.delete_local(2, 1, 2000);     // delete "c"
        laptop.apply(&del);
        phone.apply(&mv);
        assert_eq!(laptop.text(), "abefd");
        assert_eq!(phone.text(), "abefd");
    }

    #[test]
    fn moves_into_each_other_lose_nothing() {
        let (mut laptop, mut phone) = laptop_and_phone();
        let a = laptop.move_local(0, 2, 5, 2000);     // "ab" between e and f
        let b = phone.move_local(4, 2, 1, 2000);      // "ef" between a and b
        laptop.apply(&b);
        phone.apply(&a);
        assert_eq!(laptop.text(), "cdeabf");
        assert_eq!(phone.text(), "cdeabf");
        assert!(laptop.layout.is_skipped(b.id));
    }

    #[test]
    fn overlapping_moves_give_a_merge_notice_on_both_devices() {
        // The laptop moves "bcd" to the end; the phone, offline, moves "cde" to the start.
        let (mut laptop, mut phone) = laptop_and_phone();
        let a = laptop.move_local(1, 3, 6, 2000);
        let b = phone.move_local(2, 3, 0, 2000);
        assert!(laptop.merge_notices().is_empty());   // nothing to say before they meet
        laptop.apply(&b);
        phone.apply(&a);
        assert_eq!(laptop.text(), "cdeafb");

        let notices = laptop.merge_notices();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].key, NoticeKey::Overlap { later: b.id, earlier: a.id });
        assert_eq!(notices[0].text, "cd");             // the text both Moves took
        assert_eq!(notices[0].places, [(notices[0].chars[0], notices[0].chars[1])], "one place, from c to d");
        assert_eq!(phone.merge_notices(), notices);    // worked out the same way everywhere
    }

    #[test]
    fn typing_into_contested_text_does_not_grow_the_notice() {
        // Found by comparing against the reference: a notice quotes the text the two Moves
        // fought over, not text typed there afterwards by someone who saw both.
        let (mut laptop, mut phone) = laptop_and_phone();
        let a = laptop.move_local(1, 3, 6, 2000);
        let b = phone.move_local(2, 3, 0, 2000);
        laptop.apply(&b);
        phone.apply(&a);
        assert_eq!(laptop.text(), "cdeafb");
        laptop.insert_local(1, "XYZ", 3000);          // inside "cd", which both Moves took
        assert_eq!(laptop.text(), "cXYZdeafb");
        let notice = laptop.merge_notices().remove(0);
        assert_eq!(notice.text, "cd");
        // "XYZ" now sits between them: two places to show.
        let (c, d) = (notice.chars[0], notice.chars[1]);
        assert_eq!(notice.places, [(c, c), (d, d)]);
    }

    #[test]
    fn moves_into_each_other_give_a_merge_notice() {
        let (mut laptop, mut phone) = laptop_and_phone();
        let a = laptop.move_local(0, 2, 5, 2000);     // "ab" between e and f
        let b = phone.move_local(4, 2, 1, 2000);      // "ef" between a and b
        laptop.apply(&b);
        phone.apply(&a);
        let notices = laptop.merge_notices();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].key, NoticeKey::Loop { skipped: b.id });
        assert_eq!(notices[0].text, "ef");             // the text that stayed where it was
    }

    #[test]
    fn dismissing_a_notice_syncs_and_can_be_undone() {
        let (mut laptop, mut phone) = laptop_and_phone();
        let a = laptop.move_local(1, 3, 6, 2000);
        let b = phone.move_local(2, 3, 0, 2000);
        laptop.apply(&b);
        phone.apply(&a);
        let key = laptop.merge_notices()[0].key;

        let dismiss = laptop.dismiss_local(key, 3000);
        assert!(laptop.merge_notices().is_empty());
        phone.apply(&dismiss);                         // dismissed on the laptop = gone on the phone too
        assert!(phone.merge_notices().is_empty());

        phone.revert_local(dismiss.id, false, 4000);   // a Dismiss is a Change, so it can be reverted
        assert_eq!(phone.merge_notices().len(), 1);
    }

    // ---- Typing bursts ----

    /// The Changes' kinds and texts, for short asserts.
    fn summary(changes: &[Change]) -> Vec<String> {
        changes.iter().map(|c| match &c.kind {
            ChangeKind::Insert(run) => format!("insert {}", run.text),
            ChangeKind::Delete(ranges) => format!("delete {}", ranges.iter().map(|r| r.len).sum::<u32>()),
            ChangeKind::Replace { deleted, inserted } => {
                format!("replace {} with {}", deleted.iter().map(|r| r.len).sum::<u32>(), inserted.text)
            }
            other => format!("{other:?}"),
        }).collect()
    }

    #[test]
    fn a_typo_fixed_inside_a_burst_is_never_recorded() {
        let mut doc = Doc::new(DeviceId(1));
        let mut recorded = doc.type_text(0, "teh", 1000);
        recorded.extend(doc.delete_text(1, 2, 1100));   // "t"
        recorded.extend(doc.type_text(1, "he", 1200));  // "the"
        assert_eq!(doc.text(), "the");
        assert!(recorded.is_empty(), "nothing is recorded while the burst is open");
        recorded.extend(doc.close_burst(1300));
        assert_eq!(summary(&recorded), ["insert the"]);
    }

    #[test]
    fn a_pause_or_a_long_stretch_closes_the_burst() {
        let mut doc = Doc::new(DeviceId(1));
        doc.type_text(0, "a", 0);
        assert!(doc.tick(BURST_PAUSE_MS - 1).is_none());
        let closed = doc.type_text(1, "b", BURST_PAUSE_MS); // typing again after a pause
        assert_eq!(summary(&closed), ["insert a"]);

        // Nonstop typing, one key a second: the burst closes after a minute anyway.
        let start = 1_000_000;
        let mut doc = Doc::new(DeviceId(1));
        let mut recorded = Vec::new();
        for second in 0..=BURST_MAX_MS / 1000 {
            let at = doc.len();
            recorded.extend(doc.type_text(at, "x", start + second * 1000));
        }
        assert_eq!(recorded.len(), 1);
    }

    #[test]
    fn backspacing_past_the_start_closes_the_burst_and_starts_a_deletion() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abc", 1000);
        doc.type_text(3, "XY", 2000);                       // "abcXY"
        let mut recorded = doc.delete_text(2, 2, 2100);     // "cX": past the burst's start
        assert_eq!(doc.text(), "abY");
        assert_eq!(summary(&recorded), ["insert Y"]);       // the typing is recorded...
        assert_eq!(doc.burst().unwrap().kind, BurstKind::Deleting); // ...and a deletion has started
        recorded.extend(doc.close_burst(2200));
        assert_eq!(summary(&recorded), ["insert Y", "delete 1"]);

        // Deleting the whole burst and more records only the deletion.
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abc", 1000);
        doc.type_text(3, "XY", 2000);
        let mut recorded = doc.delete_text(2, 3, 2100);     // "cXY"
        recorded.extend(doc.close_burst(2200));
        assert_eq!(doc.text(), "ab");
        assert_eq!(summary(&recorded), ["delete 1"]);
    }

    #[test]
    fn holding_backspace_is_one_deletion() {
        let mut doc = abcdef();
        let mut recorded = Vec::new();
        for (i, at) in [5, 4, 3].into_iter().enumerate() {
            recorded.extend(doc.delete_text(at, 1, 2000 + i as u64 * 100)); // backspace, backspace, backspace
        }
        assert_eq!(doc.text(), "abc");                      // gone from the screen at once...
        assert!(recorded.is_empty());                       // ...but not recorded yet
        recorded.extend(doc.close_burst(3000));
        assert_eq!(summary(&recorded), ["delete 3"]);
        assert_eq!(doc.text(), "abc");
    }

    #[test]
    fn backspace_and_forward_delete_at_one_spot_are_one_deletion() {
        let mut doc = abcdef();
        let mut recorded = doc.delete_text(2, 1, 2000);     // backspace "c": "abdef", gap at 2
        recorded.extend(doc.delete_text(2, 1, 2100));       // forward delete "d": "abef"
        recorded.extend(doc.delete_text(1, 1, 2200));       // backspace "b": "aef"
        recorded.extend(doc.close_burst(2300));
        assert_eq!(doc.text(), "aef");
        assert_eq!(summary(&recorded), ["delete 3"]);
    }

    #[test]
    fn typing_after_deleting_is_another_change() {
        let mut doc = abcdef();
        let mut recorded = doc.delete_text(5, 1, 2000);     // backspace "f"
        recorded.extend(doc.delete_text(4, 1, 2100));       // backspace "e"
        recorded.extend(doc.type_text(4, "XY", 2200));      // then type: the deletion is done
        assert_eq!(summary(&recorded), ["delete 2"]);
        recorded.extend(doc.close_burst(2300));
        assert_eq!(summary(&recorded), ["delete 2", "insert XY"]);
        assert_eq!(doc.text(), "abcdXY");
    }

    #[test]
    fn deleting_somewhere_else_closes_the_first_deletion() {
        let mut doc = abcdef();
        let mut recorded = doc.delete_text(5, 1, 2000);     // "f"
        recorded.extend(doc.delete_text(0, 1, 2100));       // "a", far away
        recorded.extend(doc.close_burst(2200));
        assert_eq!(summary(&recorded), ["delete 1", "delete 1"]);
        assert_eq!(doc.text(), "bcde");
    }

    #[test]
    fn a_deletion_burst_survives_merges() {
        let (mut laptop, mut phone) = laptop_and_phone();     // "abcdef"
        laptop.delete_text(3, 1, 2000);                        // backspace "d": "abcef"
        let front = phone.insert_local(0, "12", 2000);
        laptop.apply(&front);
        assert_eq!(laptop.text(), "12abcef");
        laptop.delete_text(4, 1, 2100);                        // backspace "c", still at the gap
        let closed = laptop.close_burst(2200).unwrap();
        assert_eq!(summary(&[closed.clone()]), ["delete 2"]);
        phone.apply(&closed);
        assert_eq!(laptop.text(), "12abef");
        assert_eq!(phone.text(), "12abef");
    }

    #[test]
    fn an_empty_burst_records_nothing_and_keeps_no_gap() {
        let mut doc = Doc::new(DeviceId(1));
        doc.type_text(0, "oops", 1000);
        doc.delete_text(0, 4, 1100);
        assert!(doc.close_burst(1200).is_none());
        let next = doc.insert_local(0, "a", 1300);
        assert_eq!(next.id.counter, 1); // the reserved counter was given back
    }

    #[test]
    fn a_bursts_id_is_reserved_when_it_opens() {
        let mut doc = Doc::new(DeviceId(1));
        doc.type_text(0, "a", 1000);
        let reserved = doc.burst().unwrap().id;
        let closed = doc.close_burst(5000).unwrap();
        assert_eq!(closed.id, reserved);
        assert_eq!(closed.hlc, Hlc(5000)); // its time is set when it closes
    }

    #[test]
    fn merges_dont_close_a_burst_and_it_stays_after_its_character() {
        let (mut laptop, mut phone) = laptop_and_phone();     // "abcdef"
        laptop.type_text(3, "XY", 2000);                       // drafting after "c"
        let front = phone.insert_local(0, "123", 2000);        // the phone types elsewhere...
        let after_c = phone.insert_local(6, "Q", 2001);        // ...and right after "c" too
        laptop.apply(&front);
        laptop.apply(&after_c);
        assert!(laptop.burst().is_some(), "merging didn't close the burst");
        assert_eq!(laptop.text(), "123abcXYQdef");             // the draft is still right after "c"
        let closed = laptop.close_burst(3000).unwrap();
        assert_eq!(laptop.text(), "123abcXYQdef");             // closing changes nothing on screen
        phone.apply(&closed);
        assert_eq!(phone.text(), "123abcXYQdef");
    }

    #[test]
    fn a_burst_survives_its_character_being_deleted_elsewhere() {
        let (mut laptop, mut phone) = laptop_and_phone();     // "abcdef"
        laptop.type_text(3, "XY", 2000);                       // after "c"
        let gone = phone.delete_local(1, 2, 2000);             // the phone deletes "bc"
        laptop.apply(&gone);
        assert_eq!(laptop.text(), "aXYdef");
        let closed = laptop.close_burst(3000).unwrap();
        phone.apply(&closed);
        assert_eq!(laptop.text(), "aXYdef");
        assert_eq!(phone.text(), "aXYdef");
    }

    #[test]
    fn typing_over_a_selection_is_one_replaced_selection() {
        let mut doc = abcdef();
        doc.replace_text(1, 3, "XY", 2000);                    // over "bcd"
        assert_eq!(doc.text(), "aXYef");
        assert_eq!(doc.len(), 5);
        doc.type_text(3, "Z", 2100);                           // keep typing
        let closed = doc.close_burst(2200).into_iter().collect::<Vec<_>>();
        assert_eq!(summary(&closed), ["replace 3 with XYZ"]);
        assert_eq!(doc.text(), "aXYZef");
    }

    #[test]
    fn a_paste_over_a_selection_is_one_change_too() {
        let mut doc = abcdef();
        let change = doc.replace_local(1, 3, "XY", 2000);
        assert_eq!(summary(&[change]), ["replace 3 with XY"]);
        assert_eq!(doc.text(), "aXYef");
    }

    #[test]
    fn moving_the_cursor_out_of_the_burst_closes_it() {
        let mut doc = abcdef();
        doc.type_text(3, "XY", 2000);                          // "abcXYdef", draft at 3..5
        assert!(doc.cursor_moved(5, 2100).is_none());          // still at its end
        assert!(doc.cursor_moved(1, 2200).is_some());          // left it
    }

    #[test]
    #[should_panic(expected = "close the typing burst first")]
    fn other_actions_need_the_burst_closed_first() {
        let mut doc = abcdef();
        doc.type_text(3, "XY", 2000);
        doc.move_local(0, 1, 6, 2100);
    }

    // ---- External edits ----

    #[test]
    fn an_external_edit_becomes_changes_that_reproduce_the_file() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.", 1000);
        let version = doc.version();
        let recorded = doc.import_text(Some(&version), "The rain had eased.", 2000);
        assert_eq!(doc.text(), "The rain had eased.");
        let kinds: Vec<String> = summary(&recorded).into_iter().filter(|s| !s.starts_with("SetField")).collect();
        assert_eq!(kinds, ["delete 7", "insert eased"]); // one word for one word: readable
    }

    #[test]
    fn a_moved_line_keeps_its_characters() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "one\ntwo\nthree\n", 1000);
        let mut before: Vec<CharId> = doc.visible().iter().map(|&(id, _)| id).collect();
        let version = doc.version();
        let recorded = doc.import_text(Some(&version), "two\nthree\none\n", 2000);
        assert_eq!(doc.text(), "two\nthree\none\n");
        assert!(recorded.iter().any(|c| matches!(c.kind, ChangeKind::Move { .. })));
        // Same characters, so their History (and later their Comments) go with them.
        let mut after: Vec<CharId> = doc.visible().iter().map(|&(id, _)| id).collect();
        before.sort();
        after.sort();
        assert_eq!(after, before);
    }

    #[test]
    fn edits_made_in_the_app_since_the_file_was_written_are_kept() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.\n", 1000);
        let written = doc.version();                           // the Text file holds this version
        doc.insert_local(22, "Gulls circled.\n", 2000);        // then the app goes on
        // Meanwhile the file was edited in another editor, starting from what was written.
        doc.import_text(Some(&written), "The rain had eased.\n", 3000);
        assert_eq!(doc.text(), "The rain had eased.\nGulls circled.\n");
    }

    #[test]
    fn an_unknown_version_gives_a_merge_notice() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.", 1000);
        let recorded = doc.import_text(None, "The rain had eased.", 2000);
        assert_eq!(doc.text(), "The rain had eased.");
        let notices = doc.merge_notices();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].key, NoticeKey::UnknownTextFile { session: recorded[0].session });
        assert_eq!(notices[0].text, "eased");
        doc.dismiss_local(notices[0].key, 3000);
        assert!(doc.merge_notices().is_empty());
    }

    #[test]
    fn an_external_edit_is_a_labelled_session_of_its_own() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abc", 1000);
        let version = doc.version();
        let recorded = doc.import_text(Some(&version), "xbc", 1001);
        let session = recorded[0].session;
        assert!(recorded.iter().all(|c| c.session == session));
        assert_ne!(session, ChangeId { device: DeviceId(1), counter: 1 }); // not the typing's Session
        assert_eq!(doc.field(&session_record(session), "source"), Some("External edit"));
        let next = doc.insert_local(0, "y", 1002);                       // only a moment later...
        assert_ne!(next.session, session);                               // ...but a new Session
    }

    #[test]
    fn an_unchanged_file_records_nothing() {
        let mut doc = abcdef();
        let version = doc.version();
        assert!(doc.import_text(Some(&version), "abcdef", 2000).is_empty());
        doc.insert_local(6, "g", 3000);                                  // the app moved on, the file didn't
        assert!(doc.import_text(Some(&version), "abcdef", 4000).is_empty());
        assert_eq!(doc.text(), "abcdefg");
    }

    #[test]
    fn windows_line_endings_and_a_byte_order_mark_are_ignored() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "one\ntwo\n", 1000);
        let version = doc.version();
        assert!(doc.import_text(Some(&version), "\u{feff}one\r\ntwo\r\n", 2000).is_empty());
    }

    // ---- Milestones ----

    #[test]
    fn returning_to_a_milestone_puts_the_text_back_and_removes_nothing() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped. Gulls circled.", 1000);
        let draft = doc.make_milestone_local("Draft 1", 2000).id;
        doc.delete_local(13, 7, 3000);                         // "stopped"
        doc.insert_local(13, "eased", 4000);
        doc.move_local(0, 20, 32, 5000);                       // shuffle things around
        let later = doc.text();
        assert_ne!(later, "The rain had stopped. Gulls circled.");
        assert_eq!(doc.text_at_milestone(draft), "The rain had stopped. Gulls circled.");

        let back = doc.return_to_milestone_local(draft, &[], 6000).unwrap();
        assert_eq!(doc.text(), "The rain had stopped. Gulls circled.");
        doc.revert_local(back.id, false, 7000);                // nothing was removed: undo the return
        assert_eq!(doc.text(), later);
    }

    #[test]
    fn returning_undoes_reverts_made_after_the_milestone() {
        let mut doc = Doc::new(DeviceId(1));
        let abc = doc.insert_local(0, "abc", 1000);
        let draft = doc.make_milestone_local("Draft", 2000).id;
        doc.revert_local(abc.id, false, 3000);
        assert_eq!(doc.text(), "");
        doc.return_to_milestone_local(draft, &[], 4000).unwrap();
        assert_eq!(doc.text(), "abc");
    }

    #[test]
    fn returning_leaves_device_names_and_later_milestones_alone() {
        let mut doc = Doc::new(DeviceId(1));
        doc.set_device_name_local("Laptop", 1000);
        doc.insert_local(0, "abc", 1500);
        let first = doc.make_milestone_local("First", 2000).id;
        doc.set_device_name_local("Work laptop", 3000);
        doc.insert_local(3, "def", 3500);
        doc.make_milestone_local("Second", 4000);
        doc.return_to_milestone_local(first, &[], 5000).unwrap();
        assert_eq!(doc.text(), "abc");
        assert_eq!(doc.device_name(DeviceId(1)), Some("Work laptop"));
        let names: Vec<String> = doc.milestones().into_iter().map(|m| m.name).collect();
        assert_eq!(names, ["First", "Second"]);
    }

    #[test]
    fn the_preview_calls_out_changes_made_earlier_on_another_device() {
        let (mut laptop, mut phone) = laptop_and_phone();     // "abcdef"
        let offline = phone.insert_local(6, "X", 1500);        // the phone, offline...
        let draft = laptop.make_milestone_local("Draft", 2000).id; // ...before the laptop names a Milestone
        let after = laptop.insert_local(0, "Y", 3000);
        laptop.apply(&offline);
        let preview = laptop.return_preview(draft);
        assert_eq!(preview.reverts, vec![offline.id, after.id]);
        assert_eq!(preview.made_earlier_elsewhere, vec![offline.id]);

        laptop.return_to_milestone_local(draft, &[offline.id], 4000).unwrap(); // keep the phone's "X"
        assert_eq!(laptop.text(), "abcdefX");
    }

    #[test]
    fn renaming_and_deleting_a_milestone() {
        let mut doc = abcdef();
        let draft = doc.make_milestone_local("Draft 2", 2000).id;
        doc.rename_milestone_local(draft, "Draft 2, sent to editor", 3000);
        assert_eq!(doc.milestones()[0].name, "Draft 2, sent to editor");
        doc.revert_local(draft, false, 4000);                  // deleting it is reverting it
        assert!(doc.milestones().is_empty());
    }

    #[test]
    fn nothing_to_return_to_records_nothing() {
        let mut doc = abcdef();
        let draft = doc.make_milestone_local("Draft", 2000).id;
        assert!(doc.return_to_milestone_local(draft, &[], 3000).is_none());
    }

    #[test]
    fn a_session_lasts_until_a_long_pause() {
        let mut doc = Doc::new(DeviceId(1));
        let a = doc.insert_local(0, "a", 1_000);
        let b = doc.insert_local(1, "b", 1_000 + 60_000);                   // a minute later
        let c = doc.insert_local(2, "c", 1_000 + 60_000 + SESSION_GAP_MS); // after a long pause
        assert_eq!(a.session, a.id);                                         // the first Change starts it
        assert_eq!(b.session, a.id);
        assert_eq!(c.session, c.id);                                         // a new Session
    }

    #[test]
    fn opening_the_work_starts_a_new_session() {
        let mut doc = Doc::new(DeviceId(1));
        let a = doc.insert_local(0, "a", 1_000);
        let mut reopened = Doc::new(DeviceId(1));
        reopened.apply(&a);
        let b = reopened.insert_local(1, "b", 2_000); // only a second later, but the Work was reopened
        assert_eq!(b.session, b.id);
    }

    #[test]
    fn the_latest_device_name_wins_and_reverting_brings_the_old_one_back() {
        let mut doc = Doc::new(DeviceId(1));
        assert_eq!(doc.device_name(DeviceId(1)), None);
        doc.set_device_name_local("Laptop", 1000);
        let rename = doc.set_device_name_local("Work laptop", 2000);
        assert_eq!(doc.device_name(DeviceId(1)), Some("Work laptop"));
        doc.revert_local(rename.id, false, 3000);
        assert_eq!(doc.device_name(DeviceId(1)), Some("Laptop"));
    }

    #[test]
    fn concurrent_renames_agree_on_every_device() {
        let (mut laptop, mut phone) = laptop_and_phone();
        let record = device_record(DeviceId(1));
        let a = laptop.set_field_local(&record, "name", "Laptop", 2000);
        let b = phone.set_field_local(&record, "name", "Old laptop", 2000); // same time: Device ID decides
        laptop.apply(&b);
        phone.apply(&a);
        assert_eq!(laptop.device_name(DeviceId(1)), Some("Old laptop"));
        assert_eq!(phone.device_name(DeviceId(1)), Some("Old laptop"));
    }

    #[test]
    fn raw_typst_edited_on_two_devices_gets_a_notice_with_the_version_that_lost() {
        use crate::change::PREAMBLE_RECORD;
        let (mut laptop, mut phone) = laptop_and_phone();
        let first = laptop.set_field_local(PREAMBLE_RECORD, "raw", "#set par(leading: 0.7em)", 1000);
        phone.apply(&first);
        assert_eq!(first.kind, ChangeKind::SetField { record: PREAMBLE_RECORD.into(), field: "raw".into(), value: "#set par(leading: 0.7em)".into(), replaces: None });
        // Both edit it offline, an hour apart: the later wins, but neither had seen the other.
        let a = phone.set_field_local(PREAMBLE_RECORD, "raw", "#set par(leading: 0.8em)", 2_000);
        let b = laptop.set_field_local(PREAMBLE_RECORD, "raw", "#show link: underline", 3_602_000);
        laptop.apply(&a);
        phone.apply(&b);
        for doc in [&laptop, &phone] {
            assert_eq!(doc.field(PREAMBLE_RECORD, "raw"), Some("#show link: underline"));
            let notices = doc.merge_notices();
            assert_eq!(notices.len(), 1);
            assert_eq!(notices[0].key, NoticeKey::ConcurrentField { later: b.id, earlier: a.id });
            let (_, field, lost) = doc.concurrent_field(notices[0].key).unwrap();
            assert_eq!((field.as_str(), lost.as_str()), ("raw", "#set par(leading: 0.8em)"));
        }
        // Editing it again (having seen the notice) settles it.
        laptop.set_field_local(PREAMBLE_RECORD, "raw", "#set par(leading: 0.8em)\n#show link: underline", 3_700_000);
        assert!(laptop.merge_notices().is_empty());
    }

    #[test]
    fn edits_that_saw_each_other_or_agree_get_no_notice() {
        use crate::change::PREAMBLE_RECORD;
        let (mut laptop, mut phone) = laptop_and_phone();
        let a = laptop.set_field_local(PREAMBLE_RECORD, "raw", "one", 1000);
        phone.apply(&a);
        let b = phone.set_field_local(PREAMBLE_RECORD, "raw", "two", 2000); // saw "one"
        laptop.apply(&b);
        assert!(laptop.merge_notices().is_empty());
        // The same text set on both, offline: nothing was lost.
        let c = laptop.set_field_local(PREAMBLE_RECORD, "raw", "three", 3000);
        let d = phone.set_field_local(PREAMBLE_RECORD, "raw", "three", 3000);
        laptop.apply(&d);
        phone.apply(&c);
        assert!(laptop.merge_notices().is_empty() && phone.merge_notices().is_empty());
        // Other fields (Device names, settings) take the later value quietly.
        let record = device_record(DeviceId(1));
        let e = laptop.set_field_local(&record, "name", "Laptop", 4000);
        let f = phone.set_field_local(&record, "name", "Old laptop", 4000);
        laptop.apply(&f);
        phone.apply(&e);
        assert!(laptop.merge_notices().is_empty());
    }

    #[test]
    fn reverting_a_move_removes_its_notice() {
        let (mut laptop, mut phone) = laptop_and_phone();
        laptop.move_local(1, 3, 6, 2000);
        let b = phone.move_local(2, 3, 0, 2000);
        laptop.apply(&b);
        assert_eq!(laptop.merge_notices().len(), 1);
        laptop.revert_local(b.id, false, 3000);        // nothing left to decide about
        assert!(laptop.merge_notices().is_empty());
    }

    #[test]
    fn a_cancelled_burst_records_nothing() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abcdef", 1000);
        doc.type_text(3, "XYZ", 2000);
        doc.cancel_burst();
        assert_eq!(doc.text(), "abcdef");
        doc.delete_text(1, 2, 3000);
        assert_eq!(doc.text(), "adef");
        doc.cancel_burst();
        assert_eq!(doc.text(), "abcdef");
        // The reserved IDs were given back: the next Change follows the first without a gap.
        assert_eq!(doc.insert_local(0, "!", 4000).id.counter, 2);
    }

    #[test]
    fn a_burst_resumes_from_what_was_kept_of_it() {
        // The app stopped with a burst open over a selection; the History is loaded again.
        let mut before = Doc::new(DeviceId(1));
        let saved = vec![before.insert_local(0, "The rain had stopped.", 1000)];
        before.replace_text(13, 7, "eased", 2000);
        let kept = before.burst().unwrap().clone();
        before.close_burst(3000);

        let mut after = Doc::new(DeviceId(1));
        after.apply_all(&saved);
        assert!(after.resume_burst(kept.kind, kept.id, kept.left, kept.replacing.clone(), kept.draft.clone(), kept.opened_ms, kept.last_ms));
        assert_eq!(after.text(), "The rain had eased.");
        let change = after.close_burst(kept.last_ms).unwrap();
        assert_eq!(change.id, kept.id, "it keeps the ID it reserved");
        assert_eq!(after.text(), before.text());
        assert!(!after.resume_burst(kept.kind, kept.id, kept.left, kept.replacing, kept.draft, 0, 0), "not twice");
    }

    #[test]
    fn a_burst_that_puts_back_what_it_hides_changes_nothing() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abcdef", 1000);
        doc.replace_text(1, 2, "X", 2000); // "aXdef"
        assert!(!doc.burst_changes_nothing());
        doc.replace_text(1, 1, "bc", 2100); // undone back to the start
        assert_eq!(doc.text(), "abcdef");
        assert!(doc.burst_changes_nothing());
    }

    #[test]
    fn pasting_a_cut_moves_the_text() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abcdef", 1000);
        let before = doc.visible();
        doc.delete_text(1, 2, 2000); // cut "bc"
        assert_eq!(doc.text(), "adef");
        let CutPasted::Moved(change) = doc.cut_to_move(3, 3000) else { panic!("a Move") }; // after "ade"
        assert!(matches!(change.kind, ChangeKind::Move { .. }));
        assert_eq!(doc.text(), "adebcf");
        // The moved characters are the same ones: their History goes with them.
        let id = |doc: &Doc, i: usize| doc.visible()[i].0;
        assert_eq!(id(&doc, 3), before[1].0);
        assert_eq!(id(&doc, 4), before[2].0);
        // And to the left of where it came from.
        doc.delete_text(4, 2, 4000); // cut "cf"
        let CutPasted::Moved(_) = doc.cut_to_move(0, 5000) else { panic!("a Move") };
        assert_eq!(doc.text(), "cfadeb");
    }

    #[test]
    fn pasting_a_cut_back_where_it_was_records_nothing() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abcdef", 1000);
        doc.delete_text(1, 2, 2000);
        assert_eq!(doc.cut_to_move(1, 3000), CutPasted::Unchanged);
        assert_eq!(doc.text(), "abcdef");
        assert!(doc.burst().is_none());
        assert_eq!(doc.insert_local(0, "!", 4000).id.counter, 2);
    }

    // ---- Comments ----

    use crate::change::AUTHOR;

    /// "abcdef" with an open Comment by the Author on "bcd".
    fn commented() -> (Doc, ChangeId) {
        let mut doc = abcdef();
        let id = doc.comment_local(1, 3, AUTHOR, "too abrupt?", 2000).id;
        (doc, id)
    }

    fn ranges(doc: &Doc) -> Vec<(usize, usize)> {
        doc.comments()[0].ranges.clone()
    }

    #[test]
    fn a_comment_covers_what_was_selected() {
        let (doc, id) = commented();
        let comments = doc.comments();
        assert_eq!(comments.len(), 1);
        let c = &comments[0];
        assert_eq!((c.id, c.sender.as_str(), c.body.as_str(), c.quote.as_str(), c.resolved), (id, AUTHOR, "too abrupt?", "bcd", false));
        assert_eq!(c.ranges, [(1, 4)]);
        assert!(c.replies.is_empty());
    }

    #[test]
    fn a_comment_follows_a_move_into_every_part() {
        let (mut doc, _) = commented();
        doc.move_local(2, 2, 6, 3000); // "cd" to the end
        assert_eq!(doc.text(), "abefcd");
        assert_eq!(ranges(&doc), [(1, 2), (4, 6)]);
    }

    #[test]
    fn a_comment_survives_a_partial_deletion() {
        let (mut doc, _) = commented();
        doc.delete_local(2, 1, 3000); // "c"
        assert_eq!(doc.text(), "abdef");
        assert_eq!(ranges(&doc), [(1, 3)]);
    }

    #[test]
    fn a_full_deletion_detaches_it_and_a_revert_reattaches_it() {
        let (mut doc, _) = commented();
        let deletion = doc.delete_local(1, 3, 3000);
        let c = &doc.comments()[0];
        assert!(c.is_detached());
        assert_eq!(c.quote, "bcd");
        doc.revert_local(deletion.id, false, 4000);
        assert_eq!(ranges(&doc), [(1, 4)]);
    }

    #[test]
    fn text_typed_inside_is_covered_but_not_at_the_edges() {
        let (mut doc, _) = commented();
        doc.insert_local(2, "X", 3000);
        assert_eq!(doc.text(), "abXcdef");
        assert_eq!(ranges(&doc), [(1, 5)]);
        doc.insert_local(1, "<", 4000);
        doc.insert_local(6, ">", 5000);
        assert_eq!(doc.text(), "a<bXcd>ef");
        assert_eq!(ranges(&doc), [(2, 6)]);
    }

    #[test]
    fn what_remains_after_the_original_text_is_deleted_keeps_it_attached() {
        let (mut doc, _) = commented();
        doc.insert_local(2, "XY", 3000); // "abXYcdef"
        doc.delete_local(1, 1, 4000); // "b"
        doc.delete_local(3, 2, 5000); // "cd"
        assert_eq!(doc.text(), "aXYef");
        assert_eq!(ranges(&doc), [(1, 3)]);
    }

    #[test]
    fn text_moved_into_it_leaves_a_gap() {
        let (mut doc, _) = commented();
        doc.move_local(5, 1, 2, 3000); // "f" between "b" and "c"
        assert_eq!(doc.text(), "abfcde");
        assert_eq!(ranges(&doc), [(1, 2), (3, 5)]);
    }

    #[test]
    fn text_moved_out_earlier_isnt_covered() {
        let mut doc = abcdef();
        doc.move_local(2, 2, 6, 2000); // "cd" to the end
        assert_eq!(doc.text(), "abefcd");
        let change = doc.comment_local(0, 4, AUTHOR, "", 3000); // "abef"
        let ChangeKind::Comment { pieces, quote, .. } = &change.kind else { panic!("a Comment") };
        assert_eq!((pieces.len(), quote.as_str()), (2, "abef"));
        assert_eq!(ranges(&doc), [(0, 4)]);
    }

    #[test]
    fn ranges_count_the_open_burst() {
        let (mut doc, _) = commented();
        doc.type_text(0, "XY", 3000); // still a draft
        assert_eq!(doc.text(), "XYabcdef");
        assert_eq!(ranges(&doc), [(3, 6)]);
        doc.delete_text(4, 1, 3100); // closes the typing burst, then hides "c" in a deletion burst
        assert_eq!(doc.text(), "XYabdef");
        assert_eq!(ranges(&doc), [(3, 5)]);
    }

    #[test]
    fn a_draft_typed_inside_counts_as_covered_before_it_is_recorded() {
        let (mut doc, _) = commented();
        doc.type_text(2, "XY", 3000); // inside "bcd", still a draft
        assert_eq!(doc.text(), "abXYcdef");
        assert_eq!(ranges(&doc), [(1, 6)]);
        doc.close_burst(4000);
        assert_eq!(ranges(&doc), [(1, 6)], "the same once it's recorded");
        doc.type_text(6, "!", 5000); // right after "d": at the edge
        assert_eq!(ranges(&doc), [(1, 6)]);
    }

    #[test]
    fn comments_come_in_text_order_with_detached_ones_last() {
        let mut doc = abcdef();
        let late = doc.comment_local(4, 2, AUTHOR, "ef", 2000).id;
        let early = doc.comment_local(0, 2, AUTHOR, "ab", 3000).id;
        let gone = doc.comment_local(2, 2, AUTHOR, "cd", 4000).id;
        doc.delete_local(2, 2, 5000);
        let order: Vec<ChangeId> = doc.comments().iter().map(|c| c.id).collect();
        assert_eq!(order, [early, late, gone]);
        assert!(doc.comments()[2].is_detached());
    }

    #[test]
    fn replies_edits_and_resolving() {
        let (mut doc, id) = commented();
        let reply = doc.reply_local(id, "reviewer/anna", "agreed", 3000).id;
        doc.reply_local(id, AUTHOR, "fixed", 4000);
        doc.edit_comment_local(id, "far too abrupt?", 5000);
        doc.edit_comment_local(reply, "agreed!", 6000);
        doc.set_comment_sender_local(id, "reviewer/anna", 7000);
        let resolve = doc.resolve_comment_local(id, true, 8000);
        let c = &doc.comments()[0];
        assert_eq!((c.body.as_str(), c.sender.as_str(), c.resolved), ("far too abrupt?", "reviewer/anna", true));
        let replies: Vec<(&str, &str)> = c.replies.iter().map(|r| (r.sender.as_str(), r.body.as_str())).collect();
        assert_eq!(replies, [("reviewer/anna", "agreed!"), (AUTHOR, "fixed")]);
        // Replying to a resolved Comment doesn't reopen it; reopening is a field like resolving.
        doc.reply_local(id, AUTHOR, "one more thing", 9000);
        assert!(doc.comments()[0].resolved);
        doc.revert_local(resolve.id, false, 10_000);
        assert!(!doc.comments()[0].resolved);
    }

    #[test]
    fn deleting_a_comment_takes_its_replies_and_restoring_brings_them_back() {
        let (mut laptop, mut phone) = laptop_and_phone();
        let comment = laptop.comment_local(1, 3, AUTHOR, "too abrupt?", 2000);
        phone.apply(&comment);
        // Offline: the phone deletes it, the laptop replies to it.
        let delete = phone.revert_local(comment.id, false, 3000);
        let reply = laptop.reply_local(comment.id, AUTHOR, "or is it?", 4000);
        phone.apply(&reply);
        laptop.apply(&delete);
        for doc in [&laptop, &phone] {
            assert!(doc.comments().is_empty());
        }
        let restore = laptop.restore_local(comment.id, 5000).unwrap();
        phone.apply(&restore);
        for doc in [&laptop, &phone] {
            assert_eq!(doc.comments()[0].replies.len(), 1);
        }
        // Deleting just a Reply.
        laptop.revert_local(reply.id, false, 6000);
        assert!(laptop.comments()[0].replies.is_empty());
    }

    #[test]
    fn a_body_edited_on_two_devices_gets_a_notice_and_resolving_just_takes_the_later() {
        let (mut laptop, mut phone) = laptop_and_phone();
        let comment = laptop.comment_local(1, 3, AUTHOR, "too abrupt?", 2000);
        phone.apply(&comment);
        let a = phone.edit_comment_local(comment.id, "too slow?", 3000);
        let b = laptop.edit_comment_local(comment.id, "too abrupt!", 4000);
        let resolve = phone.resolve_comment_local(comment.id, true, 5000);
        let reopen = laptop.resolve_comment_local(comment.id, false, 6000);
        for change in [&a, &resolve] { laptop.apply(change); }
        for change in [&b, &reopen] { phone.apply(change); }
        for doc in [&laptop, &phone] {
            let c = &doc.comments()[0];
            assert_eq!((c.body.as_str(), c.resolved), ("too abrupt!", false));
            let notices = doc.merge_notices();
            assert_eq!(notices.len(), 1, "only the body gets a notice");
            assert_eq!(notices[0].key, NoticeKey::ConcurrentField { later: b.id, earlier: a.id });
            let (record, field, lost) = doc.concurrent_field(notices[0].key).unwrap();
            assert_eq!((record, field.as_str(), lost.as_str()), (comment_record(comment.id), "body", "too slow?"));
        }
    }

    #[test]
    fn returning_to_a_milestone_keeps_comments() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abc", 1000);
        let milestone = doc.make_milestone_local("Draft 1", 2000).id;
        doc.insert_local(3, "def", 3000);
        let on_new = doc.comment_local(3, 3, AUTHOR, "new text", 4000).id;
        let on_old = doc.comment_local(0, 3, "reviewer/anna", "old text", 5000).id;
        doc.reply_local(on_old, AUTHOR, "noted", 6000);
        doc.resolve_comment_local(on_old, true, 7000);
        let preview = doc.return_preview(milestone);
        assert_eq!(preview.reverts.len(), 1, "only the typing: Comments aren't content");
        doc.return_to_milestone_local(milestone, &[], 8000).unwrap();
        assert_eq!(doc.text(), "abc");
        let comments = doc.comments();
        assert_eq!(comments.iter().map(|c| c.id).collect::<Vec<_>>(), [on_old, on_new]);
        assert_eq!((comments[0].ranges.as_slice(), comments[0].resolved, comments[0].replies.len()), ([(0, 3)].as_slice(), true, 1));
        assert!(comments[1].is_detached(), "its text went with the return");
    }
}