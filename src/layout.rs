//! Where every node is drawn, worked out from the Moves in effect (ADR 0008).
//!
//! Per node, the layout knows which Move piece it's drawn in (if any). Everything is kept in
//! plain arrays indexed by the tree's node numbers, so a full recompute is a few straight passes
//! over memory instead of a hash lookup per character.
//!
//! `crate::reference::layout` is the first, simple version; this one must behave identically.

use std::collections::HashMap;
use rustc_hash::FxHashMap;
use crate::change::Span;
use crate::fugue::{Tree, NONE};
use crate::ids::{ChangeId, CharId, DeviceId};

/// One piece of a Move: the Move's ID and the piece's index.
/// Its anchor is the node CharId { change: move id, offset: index }.
pub type Piece = (ChangeId, usize);

/// What the layout needs to know about one Move.
#[derive(Clone)]
pub struct MoveInfo {
    pub id: ChangeId,
    pub pieces: Vec<Span>,
    pub seen: Vec<(DeviceId, u32)>,
}

impl MoveInfo {
    /// Had the Device that made this Move seen `other` at the time?
    fn saw(&self, other: ChangeId) -> bool {
        self.seen.iter().any(|&(device, counter)| device == other.device && counter >= other.counter)
    }
}

/// One piece of a Move that happened.
struct Slot {
    piece: Piece,
    span: Span,
    anchor: u32,                   // its anchor's node, or NONE if the tree doesn't have it
    range: Option<(usize, usize)>, // positions of its first..=last node in the fixed order
}

#[derive(Default)]
pub struct Layout {
    owner: Vec<u32>,               // per node: the slot it's drawn in, or NONE = where it is
    slots: Vec<Slot>,              // the pieces of every Move that happened
    slot_of: FxHashMap<Piece, u32>,
    order: Vec<u32>,               // the tree's fixed order, walked once in `compute`
    // For Merge notices. Both keep the characters involved (not anchors), in fixed Change order.
    skipped: Vec<(ChangeId, Vec<CharId>)>, // Moves skipped because they made a loop, with the text they'd have moved
    contested: Vec<(ChangeId, ChangeId, Vec<CharId>)>, // (later, earlier, text): both took it without knowing about each other
}

impl Layout {
    /// `moves`: the Moves in effect, in the fixed Change order.
    /// `typed_into`: for a piece's anchor, the nodes that were typed or dropped into that piece.
    pub fn compute(tree: &Tree, moves: &[MoveInfo], typed_into: &HashMap<CharId, Vec<CharId>>) -> Layout {
        // Walk the tree once, and note every node's position in one array.
        let order = tree.order();
        let mut pos = vec![NONE; tree.len()];
        for (i, &node) in order.iter().enumerate() {
            pos[node as usize] = i as u32;
        }
        let mut layout = Layout { owner: vec![NONE; tree.len()], ..Layout::default() };

        for m in moves {
            // Give each piece a slot up front; a skipped Move gives them back.
            let base = layout.slots.len() as u32;
            for (k, span) in m.pieces.iter().enumerate() {
                let anchor = tree.index_of(CharId { change: m.id, offset: k as u32 }).unwrap_or(NONE);
                let range = match (tree.index_of(span.first), tree.index_of(span.last)) {
                    (Some(f), Some(l)) => Some((pos[f as usize] as usize, pos[l as usize] as usize)),
                    _ => None, // the tree doesn't have it (yet): nothing to take
                };
                layout.slots.push(Slot { piece: (m.id, k), span: span.clone(), anchor, range });
            }

            let mut contested: Vec<(ChangeId, ChangeId, Vec<CharId>)> = Vec::new();
            let mut taken: Vec<(u32, u32)> = Vec::new(); // (node, its owner before)
            for (k, span) in m.pieces.iter().enumerate() {
                let slot = base + k as u32;
                if let Some((lo, hi)) = layout.slots[slot as usize].range {
                    for &node in &order[lo..=hi] {
                        let before = layout.owner[node as usize];
                        let before_piece = (before != NONE).then(|| layout.slots[before as usize].piece);
                        if !Self::may_take(m, span, before_piece) {
                            continue;
                        }
                        taken.push((node, before));
                        layout.owner[node as usize] = slot;
                        if let Some((other, _)) = before_piece {
                            let id = tree.id_of(node);
                            // Only text this Move knew about counts: text typed later isn't what the two fought over.
                            if !m.saw(other) && !tree.is_anchor_at(node) && m.saw(id.change) {
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
                    if let Some(node) = tree.index_of(id) {
                        let before = layout.owner[node as usize];
                        if before != slot {
                            taken.push((node, before));
                            layout.owner[node as usize] = slot;
                        }
                    }
                }
            }

            if layout.makes_loop(m.id, base, m.pieces.len()) {
                // Keep the text it would have moved, for the Merge notice, then undo.
                let would_move: Vec<CharId> = taken.iter()
                    .filter(|&&(node, _)| !tree.is_anchor_at(node))
                    .map(|&(node, _)| tree.id_of(node))
                    .filter(|id| m.saw(id.change)) // only text it knew about
                    .collect();
                for (node, before) in taken.into_iter().rev() {
                    layout.owner[node as usize] = before;
                }
                layout.slots.truncate(base as usize);
                layout.skipped.push((m.id, would_move));
            } else {
                for k in 0..m.pieces.len() {
                    layout.slot_of.insert((m.id, k), base + k as u32);
                }
                layout.contested.extend(contested);
            }
        }

        layout.order = order;
        layout
    }

    /// May a piece of Move `m` (selected as `span`) take a node currently drawn in `current`?
    fn may_take(m: &MoveInfo, span: &Span, current: Option<Piece>) -> bool {
        match current {
            None => true,
            // The pieces of one Move never take from each other: an earlier piece already claimed it.
            // (Without this, `!m.saw(other)` is true, because a Move hasn't "seen" itself.)
            Some((other, _)) if other == m.id => false,
            // Take it from the exact piece we selected it from, or from a Move we didn't know about.
            Some((other, k)) => span.from == Some(CharId { change: other, offset: k as u32 }) || !m.saw(other),
        }
    }

    /// Would Move `id` (its pieces in slots base..base+count) end up drawn inside its own text?
    /// Follows each anchor's owner upwards: reaching the main text is fine, reaching `id` is a loop.
    fn makes_loop(&self, id: ChangeId, base: u32, count: usize) -> bool {
        for slot in base..base + count as u32 {
            let mut anchor = self.slots[slot as usize].anchor;
            let mut steps = 0;
            while anchor != NONE {
                let owner = self.owner[anchor as usize];
                if owner == NONE {
                    break; // reached the main text
                }
                let piece = &self.slots[owner as usize];
                if piece.piece.0 == id {
                    return true;
                }
                anchor = piece.anchor;
                steps += 1;
                assert!(steps <= self.slots.len(), "anchors form a loop that should have been prevented");
            }
        }
        false
    }

    /// Every character node (deleted ones too, no anchors) in reading order.
    pub fn reading_nodes(&self, tree: &Tree) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.order.len());
        self.draw(tree, NONE, &mut out);
        out
    }

    /// The same as IDs, with the piece each is drawn in (None = the main text).
    pub fn reading_order(&self, tree: &Tree) -> Vec<(CharId, Option<Piece>)> {
        self.reading_nodes(tree).into_iter().map(|n| (tree.id_of(n), self.level_at(n))).collect()
    }

    /// Draws one level: the main text (NONE), or one piece's slot.
    fn draw(&self, tree: &Tree, level: u32, out: &mut Vec<u32>) {
        let nodes = if level == NONE {
            &self.order[..]
        } else {
            match self.slots[level as usize].range {
                Some((lo, hi)) => &self.order[lo..=hi],
                None => return,
            }
        };
        for &node in nodes {
            if self.slot_at(node) != level {
                continue; // drawn somewhere else
            }
            if tree.is_anchor_at(node) {
                // An anchor: draw that piece's text here, if its Move happened.
                let id = tree.id_of(node);
                if let Some(&slot) = self.slot_of.get(&(id.change, id.offset as usize)) {
                    self.draw(tree, slot, out);
                }
            } else {
                out.push(node);
            }
        }
    }

    /// The slot a node is drawn in, or NONE.
    fn slot_at(&self, node: u32) -> u32 {
        self.owner.get(node as usize).copied().unwrap_or(NONE)
    }

    /// The piece a node is drawn in (None = the main text).
    pub fn level_at(&self, node: u32) -> Option<Piece> {
        let slot = self.slot_at(node);
        (slot != NONE).then(|| self.slots[slot as usize].piece)
    }

    pub fn span(&self, piece: Piece) -> Option<&Span> {
        self.slot_of.get(&piece).map(|&slot| &self.slots[slot as usize].span)
    }

    /// Updates the layout for text just typed on this Device, instead of recomputing everything:
    /// it belongs to the piece it was typed into (None = the main text).
    /// The Doc checks the result against a full recompute in debug builds.
    pub fn insert_typed(&mut self, nodes: &[u32], piece: Option<Piece>) {
        let slot = piece.map_or(NONE, |p| self.slot_of[&p]);
        if let Some(&last) = nodes.iter().max() {
            if self.owner.len() <= last as usize {
                self.owner.resize(last as usize + 1, NONE);
            }
        }
        for &node in nodes {
            self.owner[node as usize] = slot;
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


#[cfg(test)]
mod test {
    use super::*;

    /// These hand-built cases never type into a piece, so `typed_into` is empty.
    fn compute(t: &Tree, moves: &[MoveInfo]) -> Layout {
        Layout::compute(t, moves, &HashMap::new())
    }
    use crate::ids::Side;

    fn ch(device: u32, counter: u32) -> ChangeId { ChangeId { device: DeviceId(device), counter } }
    /// The character at `offset` in "abcdef".
    fn at(offset: u32) -> CharId { CharId { change: ch(1, 1), offset } }

    fn abcdef() -> Tree {
        let mut t = Tree::new();
        t.insert_run(ch(1, 1), "abcdef", None, Side::Right, None);
        t
    }
    /// Adds a one-piece Move's anchor right after `left` (None = the very start).
    fn anchor(t: &mut Tree, mv: ChangeId, left: Option<CharId>) {
        let (p, s, ro) = t.place_after(left);
        t.insert_anchors(mv, 1, p, s, ro);
    }
    /// A one-piece Move of first..=last, taken from the main text.
    fn one_piece(id: ChangeId, first: CharId, last: CharId, seen: &[(u32, u32)]) -> MoveInfo {
        MoveInfo {
            id,
            pieces: vec![Span { first, last, from: None }],
            seen: seen.iter().map(|&(d, c)| (DeviceId(d), c)).collect(),
        }
    }
    fn shown(t: &Tree, l: &Layout) -> String {
        l.reading_order(t).into_iter()
            .filter(|&(id, _)| t.is_visible(id))
            .map(|(id, _)| t.char_of(id).unwrap())
            .collect()
    }

    #[test]
    fn move_cd_to_the_end() {
        let mut t = abcdef();
        anchor(&mut t, ch(1, 2), Some(at(5)));
        let l = compute(&t, &[one_piece(ch(1, 2), at(2), at(3), &[(1, 1)])]);
        assert_eq!(shown(&t, &l), "abefcd");
    }

    #[test]
    fn text_typed_inside_on_another_device_follows() {
        let mut t = abcdef();
        let (p, s, ro) = t.place(3);                 // the phone types "X" between c and d...
        t.insert_run(ch(2, 1), "X", p, s, ro);
        anchor(&mut t, ch(1, 2), Some(at(5)));       // ...while the laptop moves "cd" to the end
        let l = compute(&t, &[one_piece(ch(1, 2), at(2), at(3), &[(1, 1)])]);
        assert_eq!(shown(&t, &l), "abefcXd");
    }

    #[test]
    fn text_typed_at_the_edge_stays() {
        let mut t = abcdef();
        let (p, s, ro) = t.place(4);                 // "X" right after d
        t.insert_run(ch(2, 1), "X", p, s, ro);
        anchor(&mut t, ch(1, 2), Some(at(5)));
        let l = compute(&t, &[one_piece(ch(1, 2), at(2), at(3), &[(1, 1)])]);
        assert_eq!(shown(&t, &l), "abXefcd");
    }

    #[test]
    fn text_moved_out_earlier_is_not_pulled_back() {
        // Move "cd" to the end ("abefcd"), then move "abef", which is what's on screen, after it.
        let mut t = abcdef();
        let (m1, m2) = (ch(1, 2), ch(1, 3));
        anchor(&mut t, m1, Some(at(5)));
        anchor(&mut t, m2, Some(CharId { change: m1, offset: 0 }));
        let l = compute(&t, &[
            one_piece(m1, at(2), at(3), &[(1, 1)]),
            one_piece(m2, at(0), at(5), &[(1, 2)]), // had seen m1
        ]);
        assert_eq!(shown(&t, &l), "cdabef");
    }

    #[test]
    fn overlapping_moves_the_later_one_wins() {
        // The laptop moves "bcd" to the end; the phone, offline, moves "cde" to the start.
        let mut t = abcdef();
        let (laptop, phone) = (ch(1, 2), ch(2, 1));
        anchor(&mut t, laptop, Some(at(5)));
        anchor(&mut t, phone, None);
        let l = compute(&t, &[
            one_piece(laptop, at(1), at(3), &[(1, 1)]),
            one_piece(phone, at(2), at(4), &[(1, 1)]), // hadn't seen the laptop's Move
        ]);
        assert_eq!(shown(&t, &l), "cdeafb");
        assert_eq!(l.contested().to_vec(), vec![(phone, laptop)]);
    }

    #[test]
    fn moves_into_each_other_skip_the_later_one() {
        // The laptop moves "ab" between e and f; the phone, offline, moves "ef" between a and b.
        let mut t = abcdef();
        let (laptop, phone) = (ch(1, 2), ch(2, 1));
        anchor(&mut t, laptop, Some(at(4)));
        anchor(&mut t, phone, Some(at(0)));
        let l = compute(&t, &[
            one_piece(laptop, at(0), at(1), &[(1, 1)]),
            one_piece(phone, at(4), at(5), &[(1, 1)]),
        ]);
        assert_eq!(shown(&t, &l), "cdeabf");
        assert!(l.is_skipped(phone));
    }
}