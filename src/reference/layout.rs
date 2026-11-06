use std::collections::HashMap;
use rustc_hash::{FxHashMap, FxHashSet};
use crate::change::Span;
use super::fugue::Tree;
use crate::ids::{ChangeId, CharId, DeviceId};

// One Piece is a move
pub type Piece = (ChangeId, usize);

// MoveInfo contains the information about a move operation
#[derive(Clone)]
pub struct MoveInfo {
    pub id: ChangeId,
    pub pieces: Vec<Span>,
    pub seen: Vec<(DeviceId, u32)>,
}

impl MoveInfo {
    // Had the device that made this move seen "other" at the time of the move?
    fn saw(&self, other: ChangeId) -> bool {
        self.seen.iter().any(|(device, counter)| {
            device == &other.device && counter >= &other.counter
        })
    }
}

#[derive(Default)]
pub struct Layout {
    owner: FxHashMap<CharId, Piece>, // draw at that piece's anchor, missing = drawn where it is
    spans: FxHashMap<Piece, Span>, // the pieces of every move that happened
    // The tree's fixed order, walked once in `compute` and reused for drawing,
    // and the positions in it of every piece's first and last node (only those, not every node).
    order: Vec<CharId>,
    pos: FxHashMap<CharId, usize>,
    // For Merge notices. Both keep the characters involved (not anchors), in fixed Change order.
    skipped: Vec<(ChangeId, Vec<CharId>)>, // Moves skipped because they made a loop, with the text they'd have moved
    contested: Vec<(ChangeId, ChangeId, Vec<CharId>)>, // (later, earlier, text): both took it without knowing about each other
}

impl Layout {
    /// `typed_into`: for a piece's anchor, the nodes that were typed or dropped into that piece.
    pub fn compute(tree: &Tree, moves: &[MoveInfo], typed_into: &HashMap<CharId, Vec<CharId>>) -> Layout {
        // 1. let order = tree.all_in_order();, plus a pos: HashMap<CharId, usize> giving each node's index in order.
        // 2. For each Move m, keep a taken: Vec<(CharId, Option<Piece>)> that records each node it takes, with its previous owner. Then for each (k, span) in m.pieces.iter().enumerate():
            //look up lo and hi in pos, and skip the span if either is missing;
            //for each node in &order[lo..=hi], if may_take says yes, record it in taken and set its owner to (m.id, k);
            //if the previous owner was a Move m hadn't seen, add (m.id, p) to a local contested list. Add each pair only once.
        // 3. If makes_loop(m) is true:
            // undo taken in reverse: put back the previous owner, or remove the node if there wasn't one;
            // add m.id to skipped.
        // 4. Otherwise:
            // store each piece in spans (span.clone());
            // move the local contested into the real one. A skipped Move shouldn't leave notices behind.
        
        // 1. Walk the tree once. Only the pieces' first and last nodes need a position,
        //    so look those up in one pass instead of building a table of every node.
        let order: Vec<CharId> = tree.all_in_order();
        let endpoints: FxHashSet<CharId> = moves.iter()
            .flat_map(|m| m.pieces.iter().flat_map(|s| [s.first, s.last]))
            .collect();
        let mut pos: FxHashMap<CharId, usize> = FxHashMap::default();
        if !endpoints.is_empty() {
            for (i, id) in order.iter().enumerate() {
                if endpoints.contains(id) {
                    pos.insert(*id, i);
                }
            }
        }
        // 2.
        let mut layout = Layout::default();
        for m in moves {
            let mut contested: Vec<(ChangeId, ChangeId, Vec<CharId>)> = Vec::new();
            let mut taken: Vec<(CharId, Option<Piece>)> = Vec::new();
            for (k, span) in m.pieces.iter().enumerate() {
                let lo = match pos.get(&span.first) {
                    Some(&i) => i,
                    None => continue,
                };
                let hi = match pos.get(&span.last) {
                    Some(&i) => i,
                    None => continue,
                };
                for &id in &order[lo..=hi] {
                    if Self::may_take(m, span, layout.owner(id)) {
                        let prev_owner = layout.owner(id);
                        taken.push((id, prev_owner));
                        layout.owner.insert(id, (m.id, k));
                        if let Some((other, _)) = prev_owner {
                            // Only text this Move knew about counts: text typed later isn't what the two fought over.
                            if !m.saw(other) && !tree.is_anchor(id) && m.saw(id.change) {
                                // One entry per pair of Moves, collecting every character they both took.
                                match contested.iter_mut().find(|(_, earlier, _)| *earlier == other) {
                                    Some((_, _, chars)) => chars.push(id),
                                    None => contested.push((m.id, other, vec![id])),
                                }
                            }
                        }
                    }
                }
                // Text typed (or Moves dropped) into this piece belongs to it, even if an
                // earlier Move's range also covers its place in the tree. A later Move can
                // still take it under the normal rules.
                let anchor = CharId { change: m.id, offset: k as u32 };
                for &id in typed_into.get(&anchor).into_iter().flatten() {
                    let prev_owner = layout.owner(id);
                    if tree.contains(id) && prev_owner != Some((m.id, k)) {
                        taken.push((id, prev_owner));
                        layout.owner.insert(id, (m.id, k));
                    }
                }
            }
            // 3.
            if layout.makes_loop(m) {
                // Keep the text it would have moved, for the Merge notice, then undo.
                let would_move: Vec<CharId> = taken.iter()
                    .map(|&(id, _)| id)
                    .filter(|&id| !tree.is_anchor(id) && m.saw(id.change)) // only text it knew about
                    .collect();
                for (id, prev_owner) in taken.into_iter().rev() {
                    match prev_owner {
                        None => { layout.owner.remove(&id); },
                        Some(piece) => { layout.owner.insert(id, piece); },
                    }
                }
                layout.skipped.push((m.id, would_move));
            } else {
                // 4.
                for (k, span) in m.pieces.iter().enumerate() {
                    layout.spans.insert((m.id, k), span.clone());
                }
                layout.contested.extend(contested);
            }
        }

        layout.order = order;
        layout.pos = pos;
        layout
    }
    fn may_take(m: &MoveInfo, span: &Span, current: Option<Piece>) -> bool {
        // Rule thatt pieces doesnt steal from moves it knew about
        // we tae out own (none) and take if its where we moved it from or !m.saw(other)
        match current {
            None => true,
            // The pieces of one Move never take from each other: an earlier piece already claimed it.
            // (Without this, `!m.saw(other)` is true, because a Move hasn't "seen" itself.)
            Some((other, _)) if other == m.id => false,
            // Take it from the exact piece we selected it from, or from a Move we didn't know about.
            Some((other, k)) => span.from == Some(CharId { change: other, offset: k as u32 }) || !m.saw(other),
        }
    }
    fn makes_loop(&self, m: &MoveInfo) -> bool {
        // for each pice k of m:
        // start at its anchor
        // follow owner upwards:
        // no owner means you reached main text, so no loop
        // if owner (x,j) with x == m.id, then we looped back to our own move, so we have a loop
        // otherwise continue to the anchor of (x,j) and repeat
        // Limit the number of steps to self.owner.len() + 1 as a safety net.
        for k in 0..m.pieces.len() {
            let mut steps = 0;
            let mut current = Some((m.id, k));
            while let Some(piece) = current {
                if steps > self.owner.len() {
                    panic!("Loop detection exceeded maximum steps");
                }
                steps += 1;
                let anchor = CharId { change: piece.0, offset: piece.1 as u32 };
                match self.owner.get(&anchor) {
                    None => break, // reached main text
                    Some(&next_piece) => {
                        if next_piece.0 == m.id {
                            return true; // loop detected
                        }
                        current = Some(next_piece);
                    }
                }
            }
        }
        false
    }

    pub fn reading_order(&self, tree: &Tree) -> Vec<(CharId, Option<Piece>)> {
        // reading_order and draw: reading_order builds order and pos, then calls draw with level = None. draw goes through the nodes of one level:
        //which nodes: for None, all of order; for Some(piece), the slice from that piece's first to its last;
        //skip any node whose owner isn't level;
        //an anchor of a piece that's in spans: recurse with draw(..., Some(that piece), out);
        //an anchor of a skipped Move: draw nothing;
        //a character: out.push((node, level)).

        // Reuse the order and positions `compute` already worked out (the tree hasn't changed since).
        let mut out: Vec<(CharId, Option<Piece>)> = Vec::with_capacity(self.order.len());
        self.draw(tree, &self.order, &self.pos, None, &mut out);
        out

    }
    fn draw(&self, tree: &Tree, order: &[CharId], pos: &FxHashMap<CharId, usize>, level: Option<Piece>,
        out: &mut Vec<(CharId, Option<Piece>)>) {
        // Determine the range of nodes to consider based on the level
        let (start, end) = match level {
            None => (0, order.len()),
            Some(piece) => {
                let span = match self.spans.get(&piece) {
                    Some(span) => span,
                    None => return, // If the piece is not in spans, do nothing
                };
                let start = match pos.get(&span.first) {
                    Some(&i) => i,
                    None => return, // If the first node is not in pos, do nothing
                };
                let end = match pos.get(&span.last) {
                    Some(&i) => i + 1, // end is exclusive
                    None => return, // If the last node is not in pos, do nothing
                };
                (start, end)
            }
        };
        for &node in &order[start..end] {
            // Only draw nodes that belong to this level.
            if self.owner(node) != level {
                continue;
            }
            if tree.is_anchor(node) {
                // An anchor: draw that piece's text here, if its Move happened.
                let piece = (node.change, node.offset as usize);
                if self.spans.contains_key(&piece) {
                    self.draw(tree, order, pos, Some(piece), out);
                }
                // An anchor of a skipped Move draws nothing.
            } else {
                out.push((node, level));
            }
        }
    }
    pub fn owner(&self, id: CharId) -> Option<Piece> {
        self.owner.get(&id).copied()
    }
    pub fn span(&self, piece: Piece) -> Option<&Span> {
        self.spans.get(&piece)
    }
    /// Updates the layout for text just typed on this Device, instead of recomputing everything.
    /// `left` is the node the text was placed right after, which is exactly where Fugue puts it
    /// in the fixed order, and `piece` is the piece it was typed into (None = the main text).
    /// The Doc checks the result against a full recompute in debug builds.
    pub fn insert_typed(&mut self, left: Option<CharId>, nodes: &[CharId], piece: Option<Piece>) {
        let at = match left {
            None => 0,
            Some(l) => self.order.iter().position(|&id| id == l).expect("left is in the tree") + 1,
        };
        self.order.splice(at..at, nodes.iter().copied());
        for p in self.pos.values_mut() {
            if *p >= at {
                *p += nodes.len();
            }
        }
        if let Some(piece) = piece {
            for &id in nodes {
                self.owner.insert(id, piece);
            }
        }
    }

    pub fn is_skipped(&self, id: ChangeId) -> bool {
        self.skipped.iter().any(|(m, _)| *m == id)
    }
    /// Moves skipped because they made a loop, with the text they would have moved.
    pub fn skipped(&self) -> &[(ChangeId, Vec<CharId>)] {
        &self.skipped
    }
    /// (later Move, earlier Move) pairs that took the same text without knowing about each other.
    pub fn contested(&self) -> Vec<(ChangeId, ChangeId)> {
        self.contested.iter().map(|&(later, earlier, _)| (later, earlier)).collect()
    }
    /// The same, with the characters both took (the later Move got them).
    pub fn contested_text(&self) -> &[(ChangeId, ChangeId, Vec<CharId>)] {
        &self.contested
    }
}
