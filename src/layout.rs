use std::collections::{HashMap, HashSet};
use crate::change::Span;
use crate::fugue::Tree;
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
    owner: HashMap<CharId, Piece>, // draw at that piece's anchor, missing = drawn where it is
    spans: HashMap<Piece, Span>, // the pieces of every move that happened
    skipped: HashSet<ChangeId>, // moves skipped because they made a loop
    contested: Vec<(ChangeId, ChangeId)>, // (later, earlier): took the same text without knowing about it
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
        
        // 1.
        let order: Vec<CharId> = tree.all_in_order();
        let pos: HashMap<CharId, usize> = order.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        // 2.
        let mut layout = Layout::default();
        for m in moves {
            let mut contested: Vec<(ChangeId, ChangeId)> = Vec::new();
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
                            if !m.saw(other) && !contested.contains(&(m.id, other)) {
                                contested.push((m.id, other));
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
                    if pos.contains_key(&id) && prev_owner != Some((m.id, k)) {
                        taken.push((id, prev_owner));
                        layout.owner.insert(id, (m.id, k));
                    }
                }
            }
            // 3.
            if layout.makes_loop(m) {
                for (id, prev_owner) in taken.into_iter().rev() {
                    match prev_owner {
                        None => { layout.owner.remove(&id); },
                        Some(piece) => { layout.owner.insert(id, piece); },
                    }
                }
                layout.skipped.insert(m.id);
            } else {
                // 4.
                for (k, span) in m.pieces.iter().enumerate() {
                    layout.spans.insert((m.id, k), span.clone());
                }
                layout.contested.extend(contested);
            }
        }   

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

        // Build order and pos
        let order: Vec<CharId> = tree.all_in_order();
        let pos: HashMap<CharId, usize> = order.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        let mut out: Vec<(CharId, Option<Piece>)> = Vec::new();

        self.draw(tree, &order, &pos, None, &mut out);
        out

    }
    fn draw(&self, tree: &Tree, order: &[CharId], pos: &HashMap<CharId, usize>, level: Option<Piece>, 
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
    pub fn is_skipped(&self, id: ChangeId) -> bool {
        self.skipped.contains(&id)
    }
    pub fn contested(&self) -> &[(ChangeId, ChangeId)] {
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