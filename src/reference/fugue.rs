use rustc_hash::{FxHashMap, FxHashSet};
use crate::ids::{CharId, Side, ChangeId};

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
    nodes: FxHashMap<CharId, Node>,
    root_children: Vec<CharId>, // The root only ever has right children
    undone: FxHashSet<ChangeId>,
}

enum Visit { Expand(CharId), Output(CharId) }


impl Tree {
    pub fn new() -> Self {
        Tree {
            nodes: FxHashMap::default(),
            root_children: Vec::new(),
            undone: FxHashSet::default(),
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

    pub fn set_undone(&mut self, undone: impl IntoIterator<Item = ChangeId>) {
        self.undone = undone.into_iter().collect();
    }

    /// Whether the tree has this node.
    pub fn contains(&self, id: CharId) -> bool {
        self.nodes.contains_key(&id)
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

    /// The first node of the subtree under `id` in the fixed order: keep taking the first left child.
    fn first_in(&self, mut id: CharId) -> CharId {
        while let Some(&child) = self.nodes[&id].left_children.first() {
            id = child;
        }
        id
    }

    /// The node right after `id` in the fixed order, found from the tree's links instead of
    /// walking the whole tree. Reading order of a node: its left children, itself, its right children.
    fn successor(&self, id: CharId) -> Option<CharId> {
        // Something after it in its own subtree: the start of its first right child.
        if let Some(&child) = self.nodes[&id].right_children.first() {
            return Some(self.first_in(child));
        }
        // Otherwise climb until we find a sibling after us, or we were a left child.
        let mut current = id;
        loop {
            let node = &self.nodes[&current];
            let siblings = match node.parent {
                None => &self.root_children,
                Some(p) => match node.side {
                    Side::Left => &self.nodes[&p].left_children,
                    Side::Right => &self.nodes[&p].right_children,
                },
            };
            let i = siblings.binary_search(&current).expect("a node is in its parent's child list");
            if let Some(&next) = siblings.get(i + 1) {
                return Some(self.first_in(next));
            }
            match (node.parent, node.side) {
                (None, _) => return None,                   // the last root child: nothing after
                (Some(p), Side::Left) => return Some(p),    // after the left children comes the parent
                (Some(p), Side::Right) => current = p,      // after the right children: keep climbing
            }
        }
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
        // rightOrigin: the node right after `left` in the fixed order (deleted ones included).
        // The plain way: walk the whole tree.
        let order = self.all_in_order();
        let right_origin = match left {
            None => order.first().copied(),
            Some(l) => order.get(order.iter().position(|&id| id == l).unwrap() + 1).copied(),
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
