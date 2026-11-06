//! The Fugue position tree (plain Fugue, ADR 0009).
//!
//! Every character, and every Move's anchor, is a node that never moves. Reading order of a node:
//! its left children, itself, its right children; siblings are kept in ID order.
//!
//! All nodes live in one array and point at each other by index, so the tree needs no memory
//! allocation per character, and a typed run (a chain of right children) sits side by side in
//! memory. A Change's nodes are always consecutive in the array, so finding a node by its ID
//! takes one table lookup per Change, not per character.
//!
//! `crate::reference::fugue` is the first, simple version; this one must behave identically.

use rustc_hash::{FxHashMap, FxHashSet};
use crate::ids::{CharId, Side, ChangeId};

/// "No node": the root as a parent, or the end of a list.
pub const NONE: u32 = u32::MAX;

struct Node {
    ch: Option<char>, // None = a Move's anchor
    change: u32,      // index into `Tree::changes`
    offset: u32,      // position within its Change
    parent: u32,      // NONE = the root
    side: Side,
    deleted: bool,    // someone deleted it (who: `Tree::deleted_by`)
    first_left: u32,  // first left child; siblings follow through `next`, in ID order
    first_right: u32, // first right child
    next: u32,        // next sibling on the same side of the same parent
}

/// Where a Change's nodes are in the array.
struct ChangeEntry {
    index: u32, // index into `Tree::changes`
    first: u32, // its first node
    count: u32, // how many nodes it has
}

pub struct Tree {
    nodes: Vec<Node>,
    changes: Vec<ChangeId>,
    by_change: FxHashMap<ChangeId, ChangeEntry>,
    first_root: u32, // the root's first child (the root only has right children)
    deleted_by: FxHashMap<u32, Vec<ChangeId>>,
    undone: FxHashSet<ChangeId>,
    change_undone: Vec<bool>, // per Change index: the hot path of `is_visible_at`
}

enum Visit {
    List(u32),   // visit a sibling list, starting here
    Output(u32), // output this node
}

impl Tree {
    pub fn new() -> Self {
        Tree {
            nodes: Vec::new(),
            changes: Vec::new(),
            by_change: FxHashMap::default(),
            first_root: NONE,
            deleted_by: FxHashMap::default(),
            undone: FxHashSet::default(),
            change_undone: Vec::new(),
        }
    }

    // ---- Finding nodes ----

    /// The array index of a node, if the tree has it.
    pub fn index_of(&self, id: CharId) -> Option<u32> {
        let entry = self.by_change.get(&id.change)?;
        (id.offset < entry.count).then_some(entry.first + id.offset)
    }

    /// The ID of the node at an array index.
    pub fn id_of(&self, index: u32) -> CharId {
        let node = &self.nodes[index as usize];
        CharId { change: self.changes[node.change as usize], offset: node.offset }
    }

    /// How many nodes the tree has (characters and anchors, deleted or not).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the tree has this node.
    pub fn contains(&self, id: CharId) -> bool {
        self.index_of(id).is_some()
    }

    pub fn char_of(&self, id: CharId) -> Option<char> {
        self.index_of(id).and_then(|i| self.char_at(i))
    }

    /// The Change that wrote the node at an array index.
    pub fn change_at(&self, index: u32) -> ChangeId {
        self.changes[self.nodes[index as usize].change as usize]
    }

    /// The Changes that deleted the node at an array index (empty if none did).
    pub fn deleters_at(&self, index: u32) -> &[ChangeId] {
        if self.nodes[index as usize].deleted { &self.deleted_by[&index] } else { &[] }
    }

    pub fn char_at(&self, index: u32) -> Option<char> {
        self.nodes[index as usize].ch
    }

    pub fn is_anchor(&self, id: CharId) -> bool {
        self.index_of(id).is_some_and(|i| self.is_anchor_at(i))
    }

    pub fn is_anchor_at(&self, index: u32) -> bool {
        self.nodes[index as usize].ch.is_none()
    }

    // ---- Visibility ----

    /// Visible: a character (not an anchor) whose Change isn't undone, and every Change that
    /// deleted it is undone.
    pub fn is_visible(&self, id: CharId) -> bool {
        self.index_of(id).is_some_and(|i| self.is_visible_at(i))
    }

    pub fn is_visible_at(&self, index: u32) -> bool {
        let node = &self.nodes[index as usize];
        if node.ch.is_none() || self.change_undone[node.change as usize] {
            return false;
        }
        !node.deleted || self.deleted_by[&index].iter().all(|d| self.undone.contains(d))
    }

    pub fn set_undone(&mut self, undone: impl IntoIterator<Item = ChangeId>) {
        self.undone = undone.into_iter().collect();
        self.change_undone.iter_mut().for_each(|u| *u = false);
        for change in &self.undone {
            if let Some(entry) = self.by_change.get(change) {
                self.change_undone[entry.index as usize] = true;
            }
        }
    }

    // ---- Changing the tree ----

    pub fn insert_node(&mut self, id: CharId, ch: Option<char>, parent: Option<CharId>, side: Side, _right_origin: Option<CharId>) {
        let at = self.nodes.len() as u32;
        // A Change's nodes sit side by side: this must be its next node, right at the end.
        let change = match self.by_change.get_mut(&id.change) {
            Some(entry) => {
                assert!(
                    id.offset == entry.count && entry.first + entry.count == at,
                    "node {id} inserted twice, or not right after the rest of its Change"
                );
                entry.count += 1;
                entry.index
            }
            None => {
                assert!(id.offset == 0, "node {id}: a Change's nodes must start at offset 0");
                let index = self.changes.len() as u32;
                self.changes.push(id.change);
                self.change_undone.push(false);
                self.by_change.insert(id.change, ChangeEntry { index, first: at, count: 1 });
                index
            }
        };
        let parent = match parent {
            Some(p) => self.index_of(p).expect("a parent must be inserted before its children"),
            None => NONE,
        };
        self.nodes.push(Node {
            ch,
            change,
            offset: id.offset,
            parent,
            side,
            deleted: false,
            first_left: NONE,
            first_right: NONE,
            next: NONE,
        });

        // Link it into its parent's list of children on that side, keeping ID order.
        let head = self.head(parent, side);
        let (mut prev, mut current) = (NONE, head);
        while current != NONE && self.id_of(current) < id {
            prev = current;
            current = self.nodes[current as usize].next;
        }
        self.nodes[at as usize].next = current;
        if prev == NONE {
            self.set_head(parent, side, at);
        } else {
            self.nodes[prev as usize].next = at;
        }
    }

    /// The first child on one side of a node (or of the root).
    fn head(&self, parent: u32, side: Side) -> u32 {
        match (parent, side) {
            (NONE, _) => self.first_root,
            (p, Side::Left) => self.nodes[p as usize].first_left,
            (p, Side::Right) => self.nodes[p as usize].first_right,
        }
    }

    fn set_head(&mut self, parent: u32, side: Side, child: u32) {
        match (parent, side) {
            (NONE, _) => self.first_root = child,
            (p, Side::Left) => self.nodes[p as usize].first_left = child,
            (p, Side::Right) => self.nodes[p as usize].first_right = child,
        }
    }

    pub fn delete(&mut self, id: CharId, by: ChangeId) {
        if let Some(i) = self.index_of(id) {
            self.nodes[i as usize].deleted = true;
            self.deleted_by.entry(i).or_default().push(by);
        }
    }

    pub fn delete_range(&mut self, first: CharId, len: u32, by: ChangeId) {
        for offset in first.offset..first.offset + len {
            self.delete(CharId { change: first.change, offset }, by);
        }
    }

    /// Insert `text` as one run: the first char where (parent, side) says,
    /// each next char as the right child of the one before.
    pub fn insert_chain(&mut self, change: ChangeId, items: Vec<Option<char>>, parent: Option<CharId>, side: Side, ro: Option<CharId>) {
        let mut prev: Option<CharId> = None;
        for (i, ch) in items.into_iter().enumerate() {
            let id = CharId { change, offset: i as u32 };
            match prev {
                None => self.insert_node(id, ch, parent, side, ro),
                Some(p) => self.insert_node(id, ch, Some(p), Side::Right, ro),
            }
            prev = Some(id);
        }
    }

    pub fn insert_run(&mut self, change: ChangeId, text: &str, parent: Option<CharId>, side: Side, ro: Option<CharId>) {
        self.insert_chain(change, text.chars().map(Some).collect(), parent, side, ro);
    }

    pub fn insert_anchors(&mut self, change: ChangeId, count: usize, parent: Option<CharId>, side: Side, ro: Option<CharId>) {
        self.insert_chain(change, vec![None; count], parent, side, ro);
    }

    // ---- The fixed order ----

    /// Every node's array index in the fixed order. One walk, no lookups by ID.
    pub fn order(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<Visit> = Vec::new();
        if self.first_root != NONE {
            stack.push(Visit::List(self.first_root));
        }
        while let Some(visit) = stack.pop() {
            match visit {
                Visit::Output(i) => out.push(i),
                Visit::List(i) => {
                    // A stack pops the last item first, so push in reverse reading order:
                    // the later siblings, this node's right children, the node, its left children.
                    let node = &self.nodes[i as usize];
                    if node.next != NONE { stack.push(Visit::List(node.next)); }
                    if node.first_right != NONE { stack.push(Visit::List(node.first_right)); }
                    stack.push(Visit::Output(i));
                    if node.first_left != NONE { stack.push(Visit::List(node.first_left)); }
                }
            }
        }
        out
    }

    /// Every node's ID in the fixed order.
    pub fn all_in_order(&self) -> Vec<CharId> {
        self.order().into_iter().map(|i| self.id_of(i)).collect()
    }

    /// The first node of the subtree under `i` in the fixed order: keep taking the first left child.
    fn first_in(&self, mut i: u32) -> u32 {
        while self.nodes[i as usize].first_left != NONE {
            i = self.nodes[i as usize].first_left;
        }
        i
    }

    /// The node right after `i` in the fixed order, found from the tree's links.
    fn successor(&self, i: u32) -> Option<u32> {
        // Something after it in its own subtree: the start of its first right child.
        let node = &self.nodes[i as usize];
        if node.first_right != NONE {
            return Some(self.first_in(node.first_right));
        }
        // Otherwise climb until there's a sibling after us, or we were a left child.
        let mut current = i;
        loop {
            let node = &self.nodes[current as usize];
            if node.next != NONE {
                return Some(self.first_in(node.next));
            }
            match (node.parent, node.side) {
                (NONE, _) => return None,                 // the last root child: nothing after
                (p, Side::Left) => return Some(p),        // after the left children comes the parent
                (p, Side::Right) => current = p,          // after the right children: keep climbing
            }
        }
    }

    // ---- Placing new text ----

    /// Decides where to place new text typed at a visible index, ignoring Moves.
    pub fn place(&self, index: usize) -> (Option<CharId>, Side, Option<CharId>) {
        let left = if index == 0 { None } else { self.visible_id(index - 1) };
        self.place_after(left)
    }

    /// Where new text goes so that it comes right after `left` in the fixed order:
    /// returns (parent, side, right_origin).
    pub fn place_after(&self, left: Option<CharId>) -> (Option<CharId>, Side, Option<CharId>) {
        let left_index = left.map(|l| self.index_of(l).expect("left is in the tree"));
        // rightOrigin: the node right after `left` in the fixed order (deleted ones included).
        let right_origin = match left_index {
            None => (self.first_root != NONE).then(|| self.first_in(self.first_root)),
            Some(l) => self.successor(l),
        }
        .map(|i| self.id_of(i));
        // In debug builds (the tests), check against the plain way: walk the whole tree.
        debug_assert_eq!(right_origin, {
            let order = self.all_in_order();
            match left {
                None => order.first().copied(),
                Some(l) => order.get(order.iter().position(|&id| id == l).unwrap() + 1).copied(),
            }
        });
        let has_right_children = match left_index {
            None => self.first_root != NONE,
            Some(l) => self.nodes[l as usize].first_right != NONE,
        };
        if !has_right_children {
            (left, Side::Right, right_origin)
        } else {
            (right_origin, Side::Left, right_origin)
        }
    }

    // ---- Whole-tree helpers (they ignore Moves; the Doc's layout handles those) ----

    pub fn text(&self) -> String {
        self.order().into_iter()
            .filter(|&i| self.is_visible_at(i))
            .filter_map(|i| self.char_at(i))
            .collect()
    }

    pub fn visible_ids(&self) -> Vec<CharId> {
        self.order().into_iter()
            .filter(|&i| self.is_visible_at(i))
            .map(|i| self.id_of(i))
            .collect()
    }

    pub fn visible_id(&self, index: usize) -> Option<CharId> {
        self.visible_ids().get(index).copied()
    }

    pub fn visible_id_ranges(&self, index: usize, len: usize) -> Vec<CharId> {
        self.visible_ids().into_iter().skip(index).take(len).collect()
    }

    /// Every other Change with a character strictly between this Change's first and last node
    /// in the fixed order: the Changes that depend on it when it's reverted.
    pub fn inside(&self, change: ChangeId) -> Vec<ChangeId> {
        let Some(entry) = self.by_change.get(&change) else { return Vec::new() };
        let order = self.order();
        let of_change = |i: &u32| self.nodes[*i as usize].change == entry.index;
        let (Some(lo), Some(hi)) = (order.iter().position(of_change), order.iter().rposition(of_change)) else {
            return Vec::new();
        };
        if hi <= lo + 1 {
            return Vec::new();
        }
        let mut result: Vec<ChangeId> = order[lo + 1..hi].iter()
            .map(|&i| &self.nodes[i as usize])
            .filter(|n| n.change != entry.index && n.ch.is_some()) // a Move's anchor is never a dependent
            .map(|n| self.changes[n.change as usize])
            .collect();
        result.sort_unstable();
        result.dedup();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{DeviceId, ChangeId};

    fn id(counter: u32, offset: u32) -> CharId {
        CharId { change: ChangeId { device: DeviceId(1), counter }, offset }
    }

    #[test]
    fn cat_then_chat() {
        let mut t = Tree::new();
        // "cat" typed in one go: one Change, a chain of right children.
        t.insert_node(id(1, 0), Some('c'), None, Side::Right, None);
        t.insert_node(id(1, 1), Some('a'), Some(id(1, 0)), Side::Right, None);
        t.insert_node(id(1, 2), Some('t'), Some(id(1, 1)), Side::Right, None);
        assert_eq!(t.text(), "cat");

        // "h" at index 1: c already has a right child (a), so h becomes a's LEFT child.
        let (parent, side, ro) = t.place(1);
        assert_eq!((parent, side, ro), (Some(id(1, 1)), Side::Left, Some(id(1, 1))));
        t.insert_node(id(2, 0), Some('h'), parent, side, ro);
        assert_eq!(t.text(), "chat");
    }

    fn change(device: u32, counter: u32) -> ChangeId {
        ChangeId { device: DeviceId(device), counter }
    }

    fn cat() -> Tree {
        let mut t = Tree::new();
        t.insert_run(change(1, 1), "cat", None, Side::Right, None);
        t
    }

    #[test]
    fn hat_mat() {
        // Both Devices start from the same synced "cat".
        // Each one decides where its text goes on its own copy.
        let (p1, s1, ro1) = cat().place(3); // laptop, Device 1
        let (p2, s2, ro2) = cat().place(3); // phone, Device 2

        let hat = change(1, 2); // the laptop's next Change
        let mat = change(2, 1); // the phone's first Change

        // Merge in one order...
        let mut a = cat();
        a.insert_run(hat, " hat", p1, s1, ro1);
        a.insert_run(mat, " mat", p2, s2, ro2);

        // ...and in the other.
        let mut b = cat();
        b.insert_run(mat, " mat", p2, s2, ro2);
        b.insert_run(hat, " hat", p1, s1, ro1);

        assert_eq!(a.text(), b.text());
        assert!(a.text() == "cat hat mat" || a.text() == "cat mat hat",
                "interleaved: {}", a.text());
    }

    #[test]
    fn delete_test() {
        // Delete the "a", and check that the text is "ct".
        // Call place(1) and insert an "o". The text should be "cot".
        let mut t = cat();
        t.delete(id(1, 1), change(1, 2));
        assert_eq!(t.text(), "ct");
        let (p, s, ro) = t.place(1);
        t.insert_run(change(1, 2), "o", p, s, ro);
        assert_eq!(t.text(), "cot");
    }

    #[test]
    fn anchors_are_invisible() {
        let mut t = cat();
        let (p, s, ro) = t.place(3);
        t.insert_anchors(change(1, 2), 1, p, s, ro);
        assert_eq!(t.text(), "cat");
        assert_eq!(t.visible_ids().len(), 3);
        assert_eq!(t.all_in_order().len(), 4); // still a node in the tree
    }

    #[test]
    fn anchors_are_not_dependents() {
        let mut t = cat();
        let (p, s, ro) = t.place(1);
        t.insert_anchors(change(1, 2), 1, p, s, ro); // an anchor between c and a
        assert!(t.inside(change(1, 1)).is_empty());
    }

    #[test]
    fn typing_after_an_anchor() {
        let mut t = cat();
        let (p, s, ro) = t.place(3);
        let anchor = CharId { change: change(1, 2), offset: 0 };
        t.insert_anchors(anchor.change, 1, p, s, ro);
        let (p, s, ro) = t.place_after(Some(anchor));
        t.insert_run(change(1, 3), "s", p, s, ro);
        assert_eq!(t.text(), "cats");
        assert_eq!(t.all_in_order().last().unwrap().change, change(1, 3)); // after the anchor
    }

    #[test]
    #[should_panic(expected = "inserted twice")]
    fn a_node_cannot_be_inserted_twice() {
        let mut t = cat();
        t.insert_node(id(1, 1), Some('x'), None, Side::Right, None);
    }

    #[test]
    fn siblings_stay_in_id_order_whatever_order_they_arrive_in() {
        // Three Devices type at the very start of "cat" at once; every arrival order agrees.
        let runs = [(change(3, 1), "z"), (change(2, 1), "y"), (change(4, 1), "w")];
        let (p, s, ro) = cat().place(0);
        let texts: Vec<String> = [[0, 1, 2], [2, 1, 0], [1, 2, 0]].iter().map(|arrival| {
            let mut t = cat();
            for &k in arrival {
                t.insert_run(runs[k].0, runs[k].1, p, s, ro);
            }
            t.text()
        }).collect();
        assert!(texts.iter().all(|t| *t == texts[0]), "{texts:?}");
    }
}
