use std::collections::HashMap;
use crate::ids::{CharId, Side, ChangeId};
use std::collections::HashSet;

struct Node {
    ch: Option<char>, // None = Move Anchor
    parent: Option<CharId>, // None = root
    side: Side,
    right_origin: Option<CharId>,
    deleted_by: Vec<ChangeId>, 
    left_children: Vec<CharId>, // Always kept sorted
    right_children: Vec<CharId>, // Always kept sorted
}

pub struct Tree {
    nodes: HashMap<CharId, Node>,
    root_children: Vec<CharId>, // The root only ever has right children
    undone: HashSet<ChangeId> 
}

enum Visit { Expand(CharId), Output(CharId) }


impl Tree {
    pub fn new() -> Self {
        Tree {
            nodes: HashMap::new(),
            root_children: Vec::new(),
            undone: HashSet::new(),
        }
    }

    pub fn is_visible(&self, id: CharId) -> bool { 
        // A node is visible if it has not been deleted by any change that is not undone.
        if self.undone.contains(&id.change) { return false; }
        // if a node's char is None, it is a Move Anchor and is not visible.
        if let Some(node) = self.nodes.get(&id) {
            if node.ch.is_none() { return false; }
            for del in &node.deleted_by {
                if !self.undone.contains(del) {
                    return false;
                }
            }
            true
        } else {
            false
        }
    }
    pub fn is_anchor(&self, id: CharId) -> bool {
        if let Some(node) = self.nodes.get(&id) {
            node.ch.is_none()
        } else {
            false
        }
    }

    pub fn set_undone(&mut self, undone: HashSet<ChangeId>) {
        self.undone = undone;
    }

    pub fn insert_node(&mut self, id: CharId, ch: Option<char>, parent: Option<CharId>, side: Side, right_origin: Option<CharId>) {
        assert!(!self.nodes.contains_key(&id), "node {id} inserted twice");
        let node = Node {
            ch,
            parent,
            side,
            right_origin,
            deleted_by: Vec::new(),
            left_children: Vec::new(),
            right_children: Vec::new(),
        };
        self.nodes.insert(id, node);

        
        if let Some(parent_id) = parent {
            let parent_node = self.nodes.get_mut(&parent_id)
                .expect("a parent must be inserted before its children");
            let list = match side {
                Side::Left => &mut parent_node.left_children,
                Side::Right => &mut parent_node.right_children,
            };
            let pos = list.binary_search(&id).unwrap_or_else(|p| p);
            list.insert(pos, id);
        } else {
            // A child of the root
            let pos = self.root_children.binary_search(&id).unwrap_or_else(|p| p);
            self.root_children.insert(pos, id);
        }
    
    }

    pub fn char_of(&self, id: CharId) -> Option<char> {
        self.nodes.get(&id).and_then(|n| n.ch)
    }

    pub fn delete(&mut self, id: CharId, by: ChangeId) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.deleted_by.push(by);
        }
    }
    pub fn delete_range(&mut self, first: CharId, len: u32, by: ChangeId) {
        for offset in first.offset..first.offset + len {
            let id = CharId { change: first.change, offset };
            self.delete(id, by);
        }
    }

    pub fn all_in_order(&self) -> Vec<CharId> {
        let mut result = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<Visit> =
            self.root_children.iter().rev().map(|&id| Visit::Expand(id)).collect();
        while let Some(visit) = stack.pop() {
            match visit {
                Visit::Output(id) => result.push(id),
                Visit::Expand(id) => {
                    let node = &self.nodes[&id];
                    // A stack pops the last item first, so push in reverse reading order:
                    // right children, then the node itself, then left children.
                    for &c in node.right_children.iter().rev() { stack.push(Visit::Expand(c)); }
                    stack.push(Visit::Output(id));
                    for &c in node.left_children.iter().rev() { stack.push(Visit::Expand(c)); }
                }
            }
        }
        result
    }
    

    pub fn text(&self) -> String {
        let mut result = String::new();
        for id in self.all_in_order() {
            if let Some(node) = self.nodes.get(&id) {
                if !self.is_visible(id) { continue; }
                // make ch not an Option<char> because we already checked that it is visible
                result.push( node.ch.expect("visible nodes must have a char"));
            }
        }
        result
    }

    pub fn visible_id(&self, index: usize) -> Option<CharId> {
        let mut count = 0;
        for id in self.all_in_order() {
            if let Some(node) = self.nodes.get(&id) {
                if !self.is_visible(id) { continue; }
                if count == index {
                    return Some(id);
                }
                count += 1;
            }
        }
        None
    }
    pub fn visible_ids(&self) -> Vec<CharId> {
        let mut result = Vec::new();
        for id in self.all_in_order() {
            if self.is_visible(id) {
                result.push(id);
            }
        }
        result
    }

    pub fn inside(&self, change: ChangeId) -> Vec<ChangeId> {
        let order = self.all_in_order();
        let lo = order.iter().position(|&id| id.change == change);
        let hi = order.iter().rposition(|&id| id.change == change);
        if lo.is_none() || hi.is_none() || lo == hi {
            return Vec::new();
        }
        let lo = lo.unwrap();
        let hi = hi.unwrap();
        // Skip anchors and the change itself, and return the unique ChangeIds of all other nodes in between.
        let mut result: Vec<ChangeId> = order[lo + 1..hi]
            .iter()
            .map(|&id| id.change)
            .filter(|&c| c != change && self.char_of(CharId { change: c, offset: 0 }).is_some())
            .collect();
        result.sort_unstable();
        result.dedup();
        result
    }

    // Decides where to place new text typed at a given visual index.
    // returns (parent, side, right_origin) for the new node to be inserted.
    pub fn place(&self, index: usize) -> (Option<CharId>, Side, Option<CharId>) {
        // place(index) just becomes self.place_after(<the visible id before index>)
        let left = if index == 0 {
            None
        } else {
            self.visible_id(index - 1)
        };

        self.place_after(left)
    }

    pub fn place_after(&self, left: Option<CharId>) -> (Option<CharId>, Side, Option<CharId>) {
        // place_after(left: Option<CharId>) contains the current logic from rightOrigin onwards, and place(index) just becomes self.place_after(<the visible id before index>). The Doc will need to choose left itself because of rule 4, and left can be an anchor.
        let order = self.all_in_order();
        let right_origin = match left {
            None => order.first().copied(),
            Some(l) => {
                let pos = order.iter().position(|&id| id == l).unwrap();
                order.get(pos + 1).copied()
            }
        };
        let has_right_children = match left {
            None => !self.root_children.is_empty(),
            Some(l) => !self.nodes[&l].right_children.is_empty(),
        };
        if !has_right_children {
            (left, Side::Right, right_origin)
        } else {
            (right_origin, Side::Left, right_origin)
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
        let items: Vec<Option<char>> = text.chars().map(Some).collect();
        self.insert_chain(change, items, parent, side, ro);
    }
    pub fn insert_anchors(&mut self, change: ChangeId, count: usize, parent: Option<CharId>, side: Side, ro: Option<CharId>) {
        let items: Vec<Option<char>> = vec![None; count];
        self.insert_chain(change, items, parent, side, ro);
    }

    pub fn visible_id_ranges(&self, index: usize, len: usize) -> Vec<CharId> {
        self.visible_ids().into_iter().skip(index).take(len).collect()
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

}