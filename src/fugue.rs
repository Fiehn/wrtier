use std::collections::HashMap;
use crate::ids::{CharId, Side, ChangeId};

struct Node {
    ch: char,
    parent: Option<CharId>, // None = root
    side: Side,
    right_origin: Option<CharId>,
    deleted: bool,
    left_children: Vec<CharId>, // Always kept sorted
    right_children: Vec<CharId>, // Always kept sorted
}

pub struct Tree {
    nodes: HashMap<CharId, Node>,
    root_children: Vec<CharId>, // The root only ever has right children
}

enum Visit { Expand(CharId), Output(CharId) }


impl Tree {
    pub fn new() -> Self {
        Tree {
            nodes: HashMap::new(),
            root_children: Vec::new(),
        }
    }

    pub fn insert_node(&mut self, id: CharId, ch: char, parent: Option<CharId>, side: Side, right_origin: Option<CharId>) {
        assert!(!self.nodes.contains_key(&id), "node {id} inserted twice");
        let node = Node {
            ch,
            parent,
            side,
            right_origin,
            deleted: false,
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
        self.nodes.get(&id).map(|n| n.ch)
    }

    pub fn delete(&mut self, id: CharId) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.deleted = true;
        }
    }
    pub fn delete_range(&mut self, first: CharId, len: u32) {
        for offset in first.offset..first.offset + len {
            self.delete(CharId { change: first.change, offset });
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
                if node.deleted { continue; }
                result.push(node.ch);
            }
        }
        result
    }

    pub fn visible_id(&self, index: usize) -> Option<CharId> {
        self.all_in_order().into_iter().filter(|id| !self.nodes[id].deleted).nth(index)
    }
    pub fn visible_ids(&self) -> Vec<CharId> {
        self.all_in_order().into_iter().filter(|id| !self.nodes[id].deleted).collect()
    }

    // Decides where to place new text typed at a given visual index.
    // returns (parent, side, right_origin) for the new node to be inserted.
    pub fn place(&self, index: usize) -> (Option<CharId>, Side, Option<CharId>) {
        // 1. leftOrigin = the visible node at index-1 or the root
        // 2. rightOrigin = the node after leftOrigin in all_in_order() (also including non-visible as in deleted) or None if leftOrigin is the last visible node
        // 3. if leftOrigin has no right child, (leftOrigin, Right, rightOrigin)
        // 4. else return (Some(rightOrigin), Left, rightOrigin)
        let order = self.all_in_order();

        // 1. leftOrigin: the visible char before the cursor. None = the root.
        let left_origin = if index == 0 {
            None
        } else {
            Some(self.visible_id(index - 1).expect("index is past the end of the text"))
        };
        // 2. rightOrigin: the very next node after leftOrigin, deleted ones included.
        let right_origin = match left_origin {
            None => order.first().copied(),
            Some(l) => {
                let pos = order.iter().position(|&id| id == l).unwrap();
                order.get(pos + 1).copied()
            }
        };
        // 3. and 4.
        let has_right_children = match left_origin {
            None => !self.root_children.is_empty(),
            Some(l) => !self.nodes[&l].right_children.is_empty(),
        };
        if !has_right_children {
            (left_origin, Side::Right, right_origin)
        } else {
            (right_origin, Side::Left, right_origin)
        }
    }
    /// Insert `text` as one run: the first char where (parent, side) says,
    /// each next char as the right child of the one before.
    pub fn insert_run(&mut self, change: ChangeId, text: &str,
                parent: Option<CharId>, side: Side, ro: Option<CharId>) {
        let mut prev: Option<CharId> = None;
        for (i, ch) in text.chars().enumerate() {
            let id = CharId { change, offset: i as u32 };
            match prev {
                None => self.insert_node(id, ch, parent, side, ro),
                Some(p) => self.insert_node(id, ch, Some(p), Side::Right, ro),
            }
            prev = Some(id);
        }
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
        t.insert_node(id(1, 0), 'c', None, Side::Right, None);
        t.insert_node(id(1, 1), 'a', Some(id(1, 0)), Side::Right, None);
        t.insert_node(id(1, 2), 't', Some(id(1, 1)), Side::Right, None);
        assert_eq!(t.text(), "cat");

        // "h" at index 1: c already has a right child (a), so h becomes a's LEFT child.
        let (parent, side, ro) = t.place(1);
        assert_eq!((parent, side, ro), (Some(id(1, 1)), Side::Left, Some(id(1, 1))));
        t.insert_node(id(2, 0), 'h', parent, side, ro);
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
    fn delete() {
        // Delete the "a", and check that the text is "ct".
        // Call place(1) and insert an "o". The text should be "cot".
        let mut t = cat();
        t.delete(id(1, 1));
        assert_eq!(t.text(), "ct");
        let (p, s, ro) = t.place(1);
        t.insert_run(change(1, 2), "o", p, s, ro);
        assert_eq!(t.text(), "cot");
    }
}