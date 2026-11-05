use crate::change::{Change, ChangeKind, InsertRun, CharRange};
use crate::fugue::Tree;
use crate::ids::{ChangeId, DeviceId, Hlc};
use std::collections::HashSet;

pub struct Doc {
    tree: Tree,
    device: DeviceId,
    next_counter: u32,
    clock: Hlc,
    applied: HashSet<ChangeId>, // to avoid applying the same change twice
}

impl Doc {
    pub fn new(device: DeviceId) -> Self {
        Doc {
            tree: Tree::new(),
            device,
            next_counter: 1,
            clock: Hlc(0),
            applied: HashSet::new(),
        }
    }

    pub fn text(&self) -> String {
        self.tree.text()
    }

    pub fn apply(&mut self, change: &Change) {
        if !self.applied.insert(change.id) {
            return; // already applied
        }
        self.clock.observe(change.hlc);
        match &change.kind {
            ChangeKind::Insert(run) => {
                self.tree.insert_run(change.id, &run.text, run.parent, run.side, run.right_origin);
            }
            ChangeKind::Delete(ranges) => {
                for range in ranges {
                    self.tree.delete_range(range.first, range.len);
                }
            }
            ChangeKind::Replace { deleted, inserted } => {
                for range in deleted {
                    self.tree.delete_range(range.first, range.len);
                }
                self.tree.insert_run(change.id, &inserted.text, inserted.parent, inserted.side, inserted.right_origin);
            }
        }
    }
    pub fn insert_local(&mut self, index: usize, text: &str, now_ms: u64) -> Change {
        // 1. Call place(index)
        let (parent, side, right_origin) = self.tree.place(index);
        
        // 2. build InsertRun
        let run = InsertRun {
            text: text.to_string(),
            parent,
            side,
            right_origin,
        };

        self.commit_local(ChangeKind::Insert(run), now_ms)
    }
    pub fn delete_local(&mut self, index: usize, len: usize, now_ms: u64) -> Change {
        let ids = self.tree.visible_id_ranges(index, len);
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

}