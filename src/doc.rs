use crate::change::{Change, ChangeKind, InsertRun, CharRange};
use crate::fugue::Tree;
use crate::ids::{ChangeId, DeviceId, Hlc, CharId};
use std::collections::HashSet;
use std::collections::HashMap;
use crate::layout::{Layout, MoveInfo, Piece};
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
    layout: Layout, 
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
            layout: Layout::default(),
        }
    }

    pub fn text(&self) -> String {
        // build it from visible() with char_of
        let mut result = String::new();
        for (id, _) in self.visible() {
            if let Some(ch) = self.tree.char_of(id) {
                result.push(ch);
            }
        }
        result
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

    pub fn apply(&mut self, change: &Change) {
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
            }
            ChangeKind::Revert { targets } => {
                // The Revert change itself doesn't modify the tree, but it affects which changes are considered undone.
                // We need to update the reverted_by map and refresh the undone set.
                for target in targets {
                    self.reverted_by.entry(*target).or_default().push(change.id);
                }
            }
            ChangeKind::Move { pieces, parent, side, right_origin, seen } => {
                // self.tree.insert_anchors(&mut self, change: ChangeId, count: usize, parent: Option<CharId>, side: Side, ro: Option<CharId>)
                self.tree.insert_anchors(change.id, pieces.len(), *parent, *side, *right_origin);
                // push (change.hlc, MoveInfo { id: change.id, pieces: pieces.clone(), seen: seen.clone() }) onto self.moves
                self.moves.push((change.hlc, MoveInfo { id: change.id, pieces: pieces.clone(), seen: seen.clone() }));
            }
        }
        self.refresh_undone();
        self.refresh_layout();
    }
    pub fn insert_local(&mut self, index: usize, text: &str, now_ms: u64) -> Change {
        // 1. Call place(index)
        let left = self.left_of(index);
        let (parent, side, right_origin) = self.tree.place_after(left);
        
        // 2. build InsertRun
        let run = InsertRun {
            text: text.to_string(),
            parent,
            side,
            right_origin,
        };

        self.commit_local(ChangeKind::Insert(run), now_ms)
    }
    pub fn refresh_layout(&mut self) {
        // collect the Moves that are is_in_effect()
        // sort them by HLC and DeviceId
        // self.layout = Layout::compute(&self.tree, &infos);
        // collect first and assign last
        let mut in_effect: Vec<(Hlc, MoveInfo)> = self.moves.iter()
            .filter(|(_, info)| self.is_in_effect(info.id))
            .cloned()
            .collect();
        in_effect.sort_by_key(|(hlc, info)| (*hlc, info.id.device));
        let infos: Vec<MoveInfo> = in_effect.into_iter().map(|(_, info)| info).collect();
        self.layout = Layout::compute(&self.tree, &infos);


    }
    pub fn visible(&self) -> Vec<(CharId, Option<Piece>)> {
        self.layout.reading_order(&self.tree).into_iter()
            .filter(|(id, _)| self.tree.is_visible(*id))
            .collect()
    }

    pub fn delete_local(&mut self, index: usize, len: usize, now_ms: u64) -> Change {
        // Take ids from visible
        let ids = self.visible().into_iter().skip(index).take(len).map(|(id, _)| id).collect::<Vec<_>>();
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
    fn left_of(&self, index: usize) -> Option<CharId> {
        // The node new text at "index" goes after (None = the very start).
        // Applies the edge rule: right after the last character of a piece,
        // the text goes after that piece's anchor instead.
        if index == 0 {
            return None;
        }
        let (mut node, mut level) = self.visible()[index - 1];
        while let Some(piece) = level {
            let span = self.layout.span(piece).unwrap();
            if node != span.last {
                break;
            }
            node = CharId { change: piece.0, offset: piece.1 as u32 };
            level = self.layout.owner(node);
        }
        Some(node)
    }
    pub fn move_local(&mut self, from: usize, len: usize, to: usize, now_ms: u64) -> Change {
        assert!(len > 0, "len must be greater than 0");
        let text_len = self.visible().len();
        assert!(from + len <= text_len, "from + len must be less than or equal to the text length");
        // Check that to <= from || to >= from + len
        assert!(to <= from || to >= from + len, "to must be less than or equal to from or greater than or equal to from + len");

        let visible = self.visible();
        let mut groups: Vec<(Option<Piece>, Span)> = Vec::new();
        for &(id, level) in visible.iter().skip(from).take(len) {
            if let Some((last_level, span)) = groups.last_mut() {
                if *last_level == level {
                    span.last = id;
                    continue;
                }
            }
            groups.push((level, Span { first: id, last: id, from: level.map(|p| p.0) }));
        }
        let pieces: Vec<Span> = groups.into_iter().map(|(_, span)| span).collect();

        let (parent, side, right_origin) = self.tree.place_after(self.left_of(to));

        let mut seen: Vec<(DeviceId, u32)> = self.seen.iter().map(|(&device, &counter)| (device, counter)).collect();
        seen.sort_by(|a, b| a.0.cmp(&b.0));

        self.commit_local(ChangeKind::Move { pieces, parent, side, right_origin, seen }, now_ms)
    }

    fn commit_local(&mut self, kind: ChangeKind, now_ms: u64) -> Change {
        let change = Change {
            id: ChangeId { device: self.device, counter: self.next_counter },
            hlc: self.clock.tick(now_ms),
            kind,
        };
        self.next_counter += 1;
        self.apply(&change);
        change
    }

    pub fn depends_on(&self, target: ChangeId) -> Vec<ChangeId> { self.tree.inside(target) }
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



}