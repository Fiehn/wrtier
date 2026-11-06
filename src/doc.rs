//! A Work's text as one Device sees it: the History's Changes applied to a Fugue tree,
//! laid out with the Moves in effect.
//!
//! Everything shown is derived from the set of Changes. `refresh` recomputes it all; typing,
//! backspacing and dismissing update it directly instead, and in debug builds (the tests) every
//! such shortcut is checked against a full recompute. `crate::reference::doc` is the first,
//! simple version; this one must behave identically (`tests/against_reference.rs`).

use crate::change::{Change, ChangeKind, InsertRun, CharRange, NoticeKey, Span};
use crate::fugue::Tree;
use crate::ids::{ChangeId, DeviceId, Hlc, CharId};
use crate::layout::{Layout, MoveInfo, Piece};
use rustc_hash::FxHashSet;
use std::collections::{HashMap, HashSet};

pub struct Doc {
    tree: Tree,
    device: DeviceId,
    next_counter: u32,
    clock: Hlc,
    applied: HashSet<ChangeId>,                    // to avoid applying the same Change twice
    reverted_by: HashMap<ChangeId, Vec<ChangeId>>, // a Change â†’ the Reverts that target it
    seen: HashMap<DeviceId, u32>,                  // the highest counter applied from each Device
    moves: Vec<(Hlc, MoveInfo)>,                   // every Move applied, with its clock for sorting
    typed_into: HashMap<CharId, Vec<CharId>>,      // a piece's anchor â†’ nodes typed or dropped into it
    dismissals: Vec<(ChangeId, NoticeKey)>,        // every Dismiss Change, and the notice it hides
    layout: Layout,
    // Worked out by `refresh` (or a fast path) after every Change, so reading the text doesn't redo it.
    visible: Vec<u32>, // the visible characters' tree nodes, in reading order
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
            layout: Layout::default(),
            visible: Vec::new(),
        }
    }

    // ---- Reading ----

    pub fn text(&self) -> String {
        self.visible.iter().filter_map(|&node| self.tree.char_at(node)).collect()
    }

    /// The number of visible characters.
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    /// The visible characters in reading order, with the piece each is drawn in.
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
    /// Every Device gets the same list, in the fixed Change order.
    pub fn merge_notices(&self) -> Vec<MergeNotice> {
        let overlaps = self.layout.contested_text().iter()
            .map(|(later, earlier, chars)| (NoticeKey::Overlap { later: *later, earlier: *earlier }, chars));
        let loops = self.layout.skipped().iter()
            .map(|(skipped, chars)| (NoticeKey::Loop { skipped: *skipped }, chars));
        overlaps.chain(loops)
            .filter(|(key, _)| !self.is_dismissed(*key))
            .map(|(key, involved)| {
                // Walk the visible text so the characters come out in reading order.
                let involved: FxHashSet<u32> = involved.iter().filter_map(|&id| self.tree.index_of(id)).collect();
                let nodes: Vec<u32> = self.visible.iter().copied().filter(|n| involved.contains(n)).collect();
                MergeNotice {
                    key,
                    chars: nodes.iter().map(|&n| self.tree.id_of(n)).collect(),
                    text: nodes.iter().filter_map(|&n| self.tree.char_at(n)).collect(),
                }
            })
            .collect()
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
            // Dismissing a notice changes no text: nothing to recompute.
            ChangeKind::Dismiss { .. } => self.debug_check(),
            _ if new => self.refresh(),
            _ => {} // already had it
        }
    }

    /// Recomputes everything derived from the Changes: what's undone, the layout, the visible text.
    fn refresh(&mut self) {
        self.refresh_undone();
        self.layout = Layout::compute(&self.tree, &self.in_effect_moves(), &self.typed_into);
        self.visible = self.computed_visible(&self.layout);
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
        self.clock.observe(change.hlc);
        if change.id.device == self.device {
            // A Change of ours, e.g. loaded from the file: never reuse its counter.
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

    pub fn insert_local(&mut self, index: usize, text: &str, now_ms: u64) -> Change {
        assert!(index <= self.len(), "index {index} is past the end of the text ({})", self.len());
        let (left, into) = self.left_of(index);
        let (parent, side, right_origin) = self.tree.place_after(left);
        let run = InsertRun { text: text.to_string(), parent, side, right_origin, into };
        let change = self.new_change(ChangeKind::Insert(run), now_ms);
        self.apply_change(&change);

        // Shortcut instead of `refresh`: typed text lands right after `left` in the tree, shows
        // at exactly `index`, and belongs to the piece it was typed into. Nothing else changes.
        let count = text.chars().count() as u32;
        let nodes: Vec<u32> = match self.tree.index_of(CharId { change: change.id, offset: 0 }) {
            Some(first) => (first..first + count).collect(), // a Change's nodes are consecutive
            None => Vec::new(),                               // empty text: nothing to show
        };
        self.layout.insert_typed(&nodes, into.map(|a| (a.change, a.offset as usize)));
        self.visible.splice(index..index, nodes);
        self.debug_check();
        change
    }

    pub fn delete_local(&mut self, index: usize, len: usize, now_ms: u64) -> Change {
        let end = (index + len).min(self.visible.len());
        assert!(index <= end, "index {index} is past the end of the text ({})", self.len());
        let mut ranges: Vec<CharRange> = Vec::new();
        for &node in &self.visible[index..end] {
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
        let change = self.new_change(ChangeKind::Delete(ranges), now_ms);
        self.apply_change(&change);

        // Shortcut instead of `refresh`: a deletion moves nothing, it only hides these characters.
        self.visible.drain(index..end);
        self.debug_check();
        change
    }

    /// Where new text at `index` goes: the node it goes after (None = the very start),
    /// and the anchor of the piece it goes into (None = the main text).
    /// Applies the edge rule: right after the last character of a piece,
    /// the text goes after that piece's anchor instead.
    fn left_of(&self, index: usize) -> (Option<CharId>, Option<CharId>) {
        if index == 0 {
            return (None, None);
        }
        let mut node = self.visible[index - 1];
        let mut level = self.layout.level_at(node);
        while let Some(piece) = level {
            let span = self.layout.span(piece).expect("a drawn piece has a span");
            if self.tree.id_of(node) != span.last {
                break;
            }
            node = self.tree.index_of(anchor_of(piece)).expect("a drawn piece has an anchor");
            level = self.layout.level_at(node);
        }
        (Some(self.tree.id_of(node)), level.map(anchor_of))
    }

    pub fn move_local(&mut self, from: usize, len: usize, to: usize, now_ms: u64) -> Change {
        assert!(len > 0, "len must be greater than 0");
        assert!(from + len <= self.len(), "from + len must be less than or equal to the text length");
        assert!(to <= from || to >= from + len, "to must be less than or equal to from or greater than or equal to from + len");
        assert!(to <= self.len(), "to {to} is past the end of the text ({})", self.len());

        // One piece per run of selected characters drawn in the same place.
        let mut groups: Vec<(Option<Piece>, Span)> = Vec::new();
        for &node in &self.visible[from..from + len] {
            let (id, level) = (self.tree.id_of(node), self.layout.level_at(node));
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

    /// Makes a new Change from this Device, with the next counter and a fresh clock value.
    fn new_change(&mut self, kind: ChangeKind, now_ms: u64) -> Change {
        let change = Change {
            id: ChangeId { device: self.device, counter: self.next_counter },
            hlc: self.clock.tick(now_ms),
            kind,
        };
        self.next_counter += 1;
        change
    }

    /// Makes a new Change and applies it the general way.
    fn commit_local(&mut self, kind: ChangeKind, now_ms: u64) -> Change {
        let change = self.new_change(kind, now_ms);
        self.apply(&change);
        change
    }
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
        assert_eq!(laptop.merge_notices()[0].text, "cd");
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



}