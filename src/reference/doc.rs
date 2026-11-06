use crate::change::{Change, ChangeKind, InsertRun, CharRange, NoticeKey};
use super::fugue::Tree;
use crate::ids::{ChangeId, DeviceId, Hlc, CharId};
use std::collections::HashSet;
use std::collections::HashMap;
use super::layout::{Layout, MoveInfo, Piece};
use crate::change::Span;

pub struct Doc {
    tree: Tree,
    device: DeviceId,
    next_counter: u32,
    clock: Hlc,
    applied: HashSet<ChangeId>, // to avoid applying the same change twice
    reverted_by: HashMap<ChangeId, Vec<ChangeId>>, // to track which changes have been reverted by which other changes
    seen: HashMap<DeviceId, u32>, // to track the highest counter seen from each device
    moves: Vec<(Hlc, MoveInfo)>, // to track the moves that have been applied, in order of their HLC
    typed_into: HashMap<CharId, Vec<CharId>>, // a piece's anchor → nodes typed or dropped into it
    dismissals: Vec<(ChangeId, NoticeKey)>,   // every Dismiss Change, and the notice it hides
    session: Option<(ChangeId, u64)>,         // this Device's current Session: (its first Change, time of the last)
    layout: Layout,
    // Worked out by `refresh` after every Change, so reading the text doesn't redo it.
    visible: Vec<(CharId, Option<Piece>)>,    // the visible characters in reading order, with their piece
}

/// A message for the Author about text the merge had to decide on its own.
/// It asks for nothing and changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeNotice {
    pub key: NoticeKey,
    /// The affected characters that are visible, in reading order: where "show me" jumps to.
    pub chars: Vec<CharId>,
    /// The same characters as text, to quote in the History view.
    pub text: String,
}

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
            session: None,
            layout: Layout::default(),
            visible: Vec::new(),
        }
    }

    pub fn text(&self) -> String {
        self.visible.iter().filter_map(|&(id, _)| self.tree.char_of(id)).collect()
    }

    pub fn is_in_effect(&self, id:ChangeId) -> bool {
        match self.reverted_by.get(&id) {
            Some(reverters) => reverters.iter().all(|&r| !self.is_in_effect(r)),
            None => true,
        }
    }
    fn refresh_undone(&mut self) {
        // Only changes that have been reverted can be undone, so chekc just those.
        let undone: HashSet<ChangeId> = self.reverted_by.keys().copied()
            .filter(|&id| !self.is_in_effect(id))
            .collect();
        self.tree.set_undone(undone);
    }
    pub fn revert_local(&mut self, target: ChangeId, with_dependents: bool, now_ms: u64) -> Change {
        let mut targets = vec![target];
        if with_dependents { targets.extend(self.depends_on(target)); }
        self.commit_local(ChangeKind::Revert { targets }, now_ms)
    }
    /// The Merge notices the Author hasn't dismissed, worked out from the History.
    /// Every Device gets the same list, in the fixed Change order.
    pub fn merge_notices(&self) -> Vec<MergeNotice> {
        let overlaps = self.layout.contested_text().iter()
            .map(|(later, earlier, chars)| (NoticeKey::Overlap { later: *later, earlier: *earlier }, chars));
        let loops = self.layout.skipped().iter()
            .map(|(skipped, chars)| (NoticeKey::Loop { skipped: *skipped }, chars));

        let visible = self.visible();
        overlaps.chain(loops)
            .filter(|(key, _)| !self.is_dismissed(*key))
            .map(|(key, involved)| {
                // Walk the visible text so the characters come out in reading order.
                let chars: Vec<CharId> = visible.iter()
                    .map(|&(id, _)| id)
                    .filter(|id| involved.contains(id))
                    .collect();
                let text = chars.iter().filter_map(|&id| self.tree.char_of(id)).collect();
                MergeNotice { key, chars, text }
            })
            .collect()
    }

    /// Dismissed if a Dismiss of it is in effect, so reverting the Dismiss brings it back.
    fn is_dismissed(&self, key: NoticeKey) -> bool {
        self.dismissals.iter().any(|&(id, k)| k == key && self.is_in_effect(id))
    }

    pub fn dismiss_local(&mut self, notice: NoticeKey, now_ms: u64) -> Change {
        self.commit_local(ChangeKind::Dismiss { notice }, now_ms)
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

    pub fn text_of(&self, range: &CharRange) -> String {
        let mut result = String::new();
        for offset in range.first.offset..range.first.offset + range.len {
            let id = CharId { change: range.first.change, offset };
            if let Some(ch) = self.tree.char_of(id) {
                result.push(ch);
            }
        }
        result
    }

    /// Applies many Changes, e.g. when opening a Work, and works out the text once at the end.
    /// They must be in the fixed Change order.
    pub fn apply_all(&mut self, changes: &[Change]) {
        for change in changes {
            self.apply_change(change);
        }
        self.refresh();
    }

    pub fn apply(&mut self, change: &Change) {
        self.apply_change(change);
        self.refresh();
    }

    /// Recomputes everything derived from the Changes: what's undone, the layout, the visible text.
    fn refresh(&mut self) {
        self.refresh_undone();
        self.refresh_layout();
        let visible: Vec<(CharId, Option<Piece>)> = self.layout.reading_order(&self.tree).into_iter()
            .filter(|&(id, _)| self.tree.is_visible(id))
            .collect();
        self.visible = visible;
    }

    /// Records one Change without recomputing what's derived from it; call `refresh` after.
    fn apply_change(&mut self, change: &Change) {
        if !self.applied.insert(change.id) {
            return; // already applied
        }
        // update seen: entry(change.id.device).or_insert(0) and take the max with change.id.counter
        self.seen.entry(change.id.device).and_modify(|c| *c = (*c).max(change.id.counter)).or_insert(change.id.counter);

        self.clock.observe(change.hlc);
        if change.id.device == self.device {
            // raise the counter if this change is from us, so we don't reuse the same counter using Max()
            self.next_counter = self.next_counter.max(change.id.counter + 1);
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
                // The Revert change itself doesn't modify the tree, but it affects which changes are considered undone.
                // We need to update the reverted_by map and refresh the undone set.
                for target in targets {
                    self.reverted_by.entry(*target).or_default().push(change.id);
                }
            }
            ChangeKind::Move { pieces, parent, side, right_origin, seen, into } => {
                // self.tree.insert_anchors(&mut self, change: ChangeId, count: usize, parent: Option<CharId>, side: Side, ro: Option<CharId>)
                self.tree.insert_anchors(change.id, pieces.len(), *parent, *side, *right_origin);
                self.note_into(*into, change.id, pieces.len());
                // push (change.hlc, MoveInfo { id: change.id, pieces: pieces.clone(), seen: seen.clone() }) onto self.moves
                self.moves.push((change.hlc, MoveInfo { id: change.id, pieces: pieces.clone(), seen: seen.clone() }));
            }
            ChangeKind::Dismiss { notice } => {
                // Changes nothing in the text; merge_notices() checks this list.
                self.dismissals.push((change.id, *notice));
            }
            ChangeKind::SetField { .. } => {} // fields aren't part of what the reference checks
        }
    }

    /// Remember that the `count` nodes of `change` went into the piece with anchor `into`.
    fn note_into(&mut self, into: Option<CharId>, change: ChangeId, count: usize) {
        if let Some(anchor) = into {
            let nodes = (0..count as u32).map(|offset| CharId { change, offset });
            self.typed_into.entry(anchor).or_default().extend(nodes);
        }
    }
    pub fn insert_local(&mut self, index: usize, text: &str, now_ms: u64) -> Change {
        // 1. Call place(index)
        let (left, into) = self.left_of(index);
        let (parent, side, right_origin) = self.tree.place_after(left);

        // 2. build InsertRun
        let run = InsertRun {
            text: text.to_string(),
            parent,
            side,
            right_origin,
            into,
        };
        self.commit_local(ChangeKind::Insert(run), now_ms)
    }
    pub fn refresh_layout(&mut self) {
        let layout = Layout::compute(&self.tree, &self.in_effect_moves(), &self.typed_into);
        self.layout = layout;
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

    /// The visible characters in reading order, with the piece each is drawn in.
    pub fn visible(&self) -> &[(CharId, Option<Piece>)] {
        &self.visible
    }

    /// The number of visible characters.
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    /// Readable copy of a moved piece: the visible text from its first to its last character.
    /// A piece can hold text from many Changes, so walk the visible text rather than offsets.
    /// Only used for the readable column in storage, so an empty string is fine if it's gone.
    pub fn piece_text(&self, span: &Span) -> String {
        let visible = self.visible();
        let start = visible.iter().position(|&(id, _)| id == span.first);
        let end = visible.iter().position(|&(id, _)| id == span.last);
        match (start, end) {
            (Some(s), Some(e)) if s <= e => visible[s..=e]
                .iter()
                .filter_map(|&(id, _)| self.tree.char_of(id))
                .collect(),
            _ => String::new(),
        }
    }
    
    pub fn delete_local(&mut self, index: usize, len: usize, now_ms: u64) -> Change {
        // Take ids from visible
        let ids = self.visible.iter().skip(index).take(len).map(|&(id, _)| id).collect::<Vec<_>>();
        let mut ranges: Vec<CharRange> = Vec::new();
        for id in ids {
            // Can this id extend the range we're building?
            if let Some(last) = ranges.last_mut() {
                if last.first.change == id.change && last.first.offset + last.len == id.offset {
                    last.len += 1;
                    continue; // done with this id, go to the next one
                }
            }
            // It can't (or there's no range yet), so start a new one.
            ranges.push(CharRange { first: id, len: 1 });
        }
        self.commit_local(ChangeKind::Delete(ranges), now_ms)
    }
    /// Where new text at `index` goes: the node it goes after (None = the very start),
    /// and the anchor of the piece it goes into (None = the main text).
    /// Applies the edge rule: right after the last character of a piece,
    /// the text goes after that piece's anchor instead.
    fn left_of(&self, index: usize) -> (Option<CharId>, Option<CharId>) {
        if index == 0 {
            return (None, None);
        }
        let (mut node, mut level) = self.visible[index - 1];
        while let Some(piece) = level {
            let span = self.layout.span(piece).unwrap();
            if node != span.last {
                break;
            }
            node = anchor_of(piece);
            level = self.layout.owner(node);
        }
        (Some(node), level.map(anchor_of))
    }
    pub fn move_local(&mut self, from: usize, len: usize, to: usize, now_ms: u64) -> Change {
        assert!(len > 0, "len must be greater than 0");
        assert!(from + len <= self.len(), "from + len must be less than or equal to the text length");
        // Check that to <= from || to >= from + len
        assert!(to <= from || to >= from + len, "to must be less than or equal to from or greater than or equal to from + len");

        let mut groups: Vec<(Option<Piece>, Span)> = Vec::new();
        for &(id, level) in &self.visible[from..from + len] {
            if let Some((last_level, span)) = groups.last_mut() {
                if *last_level == level {
                    span.last = id;
                    continue;
                }
            }
            // `from` is the anchor of the piece this text is drawn in (None = the main text).
            groups.push((level, Span { first: id, last: id, from: level.map(anchor_of) }));
        }
        let pieces: Vec<Span> = groups.into_iter().map(|(_, span)| span).collect();

        let (left, into) = self.left_of(to);
        let (parent, side, right_origin) = self.tree.place_after(left);

        let mut seen: Vec<(DeviceId, u32)> = self.seen.iter().map(|(&device, &counter)| (device, counter)).collect();
        seen.sort_by(|a, b| a.0.cmp(&b.0));

        self.commit_local(ChangeKind::Move { pieces, parent, side, right_origin, seen, into }, now_ms)
    }

    /// Makes a new Change from this Device, with the next counter and a fresh clock value.
    fn new_change(&mut self, kind: ChangeKind, now_ms: u64) -> Change {
        let id = ChangeId { device: self.device, counter: self.next_counter };
        // Sessions: the same rule as the fast engine, so both make identical Changes.
        let session = match self.session {
            Some((first, last)) if now_ms.saturating_sub(last) < crate::doc::SESSION_GAP_MS => first,
            _ => id,
        };
        self.session = Some((session, now_ms));
        let change = Change { id, hlc: self.clock.tick(now_ms), session, kind };
        self.next_counter += 1;
        change
    }

    /// Makes a new Change, applies it and recomputes everything (the slow, general path).
    fn commit_local(&mut self, kind: ChangeKind, now_ms: u64) -> Change {
        let change = self.new_change(kind, now_ms);
        self.apply(&change);
        change
    }

    pub fn depends_on(&self, target: ChangeId) -> Vec<ChangeId> { self.tree.inside(target) }
}
