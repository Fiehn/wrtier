//! Comparing two versions of a Work (a Milestone, or the Work as it is now), and "History of
//! this text". Read-only.
//!
//! A comparison works on characters, not on the two texts: each character is visible in one
//! version, both, or neither, and is drawn in some Move's piece or in the main text. So a passage
//! moved between the two shows as one Move, not as a deletion and an insertion.
//!
//! "History of this text" holds a stretch of characters by identity, as a Comment does: the
//! selected ones and every character that ever lay between them (deleted text, and text since
//! moved away). It stays right while the Author writes and while Changes arrive.

use super::snippets::{elided, find_in, segments, trimmed, words_in, Item, Mark, WhereFrom};
use super::{holds, positions, Doc, SegmentKind, Snippet};
use crate::change::Span;
use crate::ids::ChangeId;
use rustc_hash::FxHashSet;

/// One side of a comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Now,
    Milestone(ChangeId),
}

/// What changed from one version to another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparison {
    /// In the later side's reading order. Their `changes` are empty: a comparison has no
    /// Revert, and `at` is in the later side's text (the editor's only when that's Now).
    pub snippets: Vec<Snippet>,
    pub added_words: usize,
    pub removed_words: usize,
    /// How many Moves differ between the two.
    pub moves: usize,
}

/// A stretch of text held by identity: pieces in the tree's fixed order, like a Comment's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stretch {
    pub spans: Vec<Span>,
}

impl Doc {
    /// Which Changes make up a version.
    fn member_of(&self, version: Version) -> Box<dyn Fn(ChangeId) -> bool + '_> {
        match version {
            Version::Now => Box::new(|_| true),
            Version::Milestone(m) => {
                let seen = self.milestone(m).clone();
                Box::new(move |c| holds(&seen, c))
            }
        }
    }

    /// What changed from `from` to `to`, shown in `to`'s reading order: text only `to` has is
    /// added, text only `from` has is removed, and text drawn in a different place is moved.
    pub fn compare(&self, from: Version, to: Version) -> Comparison {
        let (layout_a, nodes_a) = self.view_among(&*self.member_of(from));
        let (layout_b, nodes_b) = self.view_among(&*self.member_of(to));
        let mut in_a = vec![false; self.tree.len()];
        let mut in_b = vec![false; self.tree.len()];
        for &n in &nodes_a {
            in_a[n as usize] = true;
        }
        for &n in &nodes_b {
            in_b[n as usize] = true;
        }

        let mut items = Vec::new();
        let mut vis = 0;
        for node in layout_b.reading_nodes(&self.tree) {
            let (a, b) = (in_a[node as usize], in_b[node as usize]);
            let mark = match (a, b) {
                (true, true) if layout_a.level_at(node) != layout_b.level_at(node) => Mark::Moved,
                (true, true) => Mark::Plain,
                (false, true) => Mark::Added,
                (true, false) => Mark::Removed,
                (false, false) => continue,
            };
            let ch = self.tree.char_at(node).expect("reading order has no anchors");
            items.push(Item { node, ch, mark, vis });
            if b {
                vis += 1;
            }
        }

        // The Move that put a moved character where `to` draws it, or (drawn in the main text
        // there) the one that had moved it in `from`: (Move, moved back).
        let mover = |n: u32| {
            layout_b.level_at(n).map(|p| (p.0, false)).or_else(|| layout_a.level_at(n).map(|p| (p.0, true)))
        };
        let mut placed = None; // the tree's order and every node's place in it, if a Move needs them
        let mut where_from = None;
        let mut all_moves = FxHashSet::default();
        let (mut added_words, mut removed_words) = (0, 0);
        let mut snippets = Vec::new();
        for paragraph in items.split(|it| it.ch == '\n' && it.mark == Mark::Plain) {
            if paragraph.iter().all(|it| it.mark == Mark::Plain) {
                continue;
            }
            let segs = segments(paragraph);
            for seg in &segs {
                match seg.kind {
                    SegmentKind::Added => added_words += words_in(&seg.text),
                    SegmentKind::Removed => removed_words += words_in(&seg.text),
                    _ => {}
                }
            }
            let shown: Vec<usize> = paragraph.iter().filter(|it| matches!(it.mark, Mark::Added | Mark::Moved)).map(|it| it.vis).collect();
            let at = match (shown.iter().min(), shown.iter().max()) {
                (Some(&lo), Some(&hi)) => (lo, hi + 1),
                _ => {
                    let first = paragraph.iter().find(|it| it.mark != Mark::Plain).map_or(0, |it| it.vis);
                    (first, first)
                }
            };
            let mut moved: Vec<(ChangeId, bool)> = Vec::new();
            for it in paragraph.iter().filter(|it| it.mark == Mark::Moved) {
                if let Some(m) = mover(it.node) {
                    if !moved.contains(&m) {
                        moved.push(m);
                    }
                }
            }
            let moves = moved.into_iter().map(|(m, back)| {
                all_moves.insert((m, back));
                let (order, pos) = placed.get_or_insert_with(|| {
                    let order = self.tree.order();
                    let pos = positions(&order, self.tree.len());
                    (order, pos)
                });
                let wf = where_from.get_or_insert_with(|| WhereFrom::new(self, &nodes_b));
                let to = paragraph.iter().find(|it| it.mark == Mark::Moved).map_or(at.0, |it| it.vis);
                let visible = |n: u32| in_b[n as usize];
                let drawn_by_it = |n: u32| mover(n).map(|(x, _)| x) == Some(m);
                self.moved_from(m, back, to, order, pos, wf, &visible, &drawn_by_it)
            }).collect();
            // Only the Work as it is now is in the editor to go to.
            let find = if to == Version::Now { find_in(self, paragraph, &nodes_b) } else { None };
            snippets.push(Snippet { segments: trimmed(elided(segs)), changes: Vec::new(), at, moves, find, in_effect: false, reverted_by: None });
        }
        Comparison { snippets, added_words, removed_words, moves: all_moves.len() }
    }

    // ---- History of this text ----

    /// The stretch of text for "History of this text": the `len` characters at `index` of the
    /// History's visible text (`committed_text`), or with `len` 0 the paragraph (line) the
    /// index is in, with its line break, so text moved away from its end still counts. None if
    /// the paragraph is empty.
    ///
    /// Unlike a Comment's, its pieces are split only where the text is drawn in a different
    /// place (as a Move's are), so text since moved away from between its characters is part of
    /// it.
    pub fn stretch_at(&self, index: usize, len: usize) -> Option<Stretch> {
        let index = index.min(self.visible.len());
        let is_break = |i: usize| self.tree.char_at(self.visible[i]) == Some('\n');
        let (from, to) = if len > 0 {
            (index, (index + len).min(self.visible.len()))
        } else {
            let start = (0..index).rev().find(|&i| is_break(i)).map_or(0, |i| i + 1);
            let end = (index..self.visible.len()).find(|&i| is_break(i));
            if end == Some(start) || start == self.visible.len() {
                return None; // an empty line
            }
            (start, end.map_or(self.visible.len(), |e| e + 1))
        };
        (from < to).then(|| Stretch { spans: self.pieces_of(&self.layout, &self.visible[from..to]) })
    }

    /// Every node that is, or ever was, part of the stretch: everything from each piece's first
    /// to its last character in the tree's fixed order, deleted and moved-away text included.
    pub(super) fn stretch_nodes(&self, stretch: &Stretch) -> FxHashSet<u32> {
        let (order, _, ranges) = self.stretch_ranges(stretch);
        ranges.iter().flat_map(|&(lo, hi)| order[lo as usize..=hi as usize].iter().copied()).collect()
    }

    /// The tree's order, every node's place in it, and the stretch's pieces as ranges of places.
    fn stretch_ranges(&self, stretch: &Stretch) -> (Vec<u32>, Vec<u32>, Vec<(u32, u32)>) {
        let order = self.tree.order();
        let pos = positions(&order, self.tree.len());
        let ranges = stretch.spans.iter()
            .filter_map(|span| Some((self.tree.index_of(span.first)?, self.tree.index_of(span.last)?)))
            .map(|(first, last)| (pos[first as usize], pos[last as usize]))
            .filter(|(lo, hi)| lo <= hi)
            .collect();
        (order, pos, ranges)
    }

    /// The Changes to the Work's content that touched the stretch, in the fixed Change order:
    /// those that wrote or deleted its characters, Moves that took any of it or dropped text into
    /// it, and every Revert of those (and of those Reverts).
    pub fn changes_in(&self, stretch: &Stretch) -> Vec<ChangeId> {
        let (order, pos, ranges) = self.stretch_ranges(stretch);
        let mut found: FxHashSet<ChangeId> = FxHashSet::default();
        for &(lo, hi) in &ranges {
            for &n in &order[lo as usize..=hi as usize] {
                found.insert(self.tree.change_at(n)); // an anchor's is the Move that dropped text here
                found.extend(self.tree.deleters_at(n).iter().copied());
            }
        }
        for (_, info) in &self.moves {
            let took_some = info.pieces.iter().any(|span| {
                let (Some(first), Some(last)) = (self.tree.index_of(span.first), self.tree.index_of(span.last)) else { return false };
                let (a, b) = (pos[first as usize], pos[last as usize]);
                ranges.iter().any(|&(lo, hi)| a <= hi && lo <= b)
            });
            if took_some {
                found.insert(info.id);
            }
        }
        let mut todo: Vec<ChangeId> = found.iter().copied().collect();
        while let Some(id) = todo.pop() {
            for &r in self.reverted_by.get(&id).into_iter().flatten() {
                if found.insert(r) {
                    todo.push(r);
                }
            }
        }
        let mut list: Vec<ChangeId> = found.into_iter().filter(|id| self.content_set.contains(id)).collect();
        list.sort_by_key(|id| self.made[id].hlc_key(*id));
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::SESSION_GAP_MS;
    use crate::ids::DeviceId;

    const LATER: u64 = SESSION_GAP_MS + 1;

    fn render(snippet: &Snippet) -> String {
        snippet.segments.iter().map(|s| match s.kind {
            SegmentKind::Plain => s.text.clone(),
            SegmentKind::Added => format!("[+{}]", s.text),
            SegmentKind::Removed => format!("[-{}]", s.text),
            SegmentKind::Moved => format!("[>{}]", s.text),
            SegmentKind::Cut => "…".into(),
            SegmentKind::Elided { words } => format!("…+{words} words…"),
        }).collect()
    }

    #[test]
    fn a_chapter_moved_between_two_milestones_is_one_move() {
        let mut doc = Doc::new(DeviceId(1));
        let text = "= One\nThe rain had stopped.\n= Two\nGulls circled.\n= Three\nShe walked home.\n";
        doc.insert_local(0, text, 1_000);
        let draft1 = doc.make_milestone_local("Draft 1", 2_000).id;
        let three = text.find("= Three").unwrap();
        doc.move_local(three, text.len() - three, 0, 3_000);
        let draft2 = doc.make_milestone_local("Draft 2", 4_000).id;
        doc.insert_local(0, "Not in either. ", 5_000);

        let c = doc.compare(Version::Milestone(draft1), Version::Milestone(draft2));
        assert_eq!(c.snippets.iter().map(render).collect::<Vec<_>>(), ["[>= Three\nShe walked home.\n]= One"]);
        assert_eq!((c.added_words, c.removed_words, c.moves), (0, 0, 1));
        let from = &c.snippets[0].moves[0];
        assert_eq!(from.chapter.as_deref(), Some("Two"));
        assert!(!from.earlier && !from.back);
    }

    #[test]
    fn a_milestone_against_now_and_back() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.", 1_000);
        let draft = doc.make_milestone_local("Draft", 2_000).id;
        doc.delete_local(13, 7, 3_000);
        doc.insert_local(13, "eased", 4_000);
        let since = doc.compare(Version::Milestone(draft), Version::Now);
        assert_eq!(since.snippets.iter().map(render).collect::<Vec<_>>(), ["The rain had [-stopped][+eased]."]);
        assert_eq!((since.added_words, since.removed_words, since.moves), (1, 1, 0));
        assert_eq!(since.snippets[0].at, (13, 18), "where \"eased\" is now");
        // The other way round, as returning to the Milestone would show it.
        let back = doc.compare(Version::Now, Version::Milestone(draft));
        assert_eq!(back.snippets.iter().map(render).collect::<Vec<_>>(), ["The rain had [-eased][+stopped]."]);
        assert!(doc.compare(Version::Now, Version::Now).snippets.is_empty());
    }

    #[test]
    fn the_history_of_a_paragraph_includes_text_moved_out_of_it() {
        let mut doc = Doc::new(DeviceId(1));
        let text = "The rain had stopped. Gulls circled.\nShe walked home.";
        let written = doc.insert_local(0, text, 1_000);
        let gulls = text.find("Gulls").unwrap();
        let moved = doc.move_local(gulls, "Gulls circled.".len(), text.len(), 1_000 + LATER);
        let elsewhere = doc.insert_local(doc.len(), " Then slept.", 2_000 + LATER);
        assert_eq!(doc.text(), "The rain had stopped. \nShe walked home.Gulls circled. Then slept.");

        let stretch = doc.stretch_at(5, 0).expect("the first paragraph");
        let changes = doc.changes_in(&stretch);
        assert_eq!(changes, vec![written.id, moved.id]);
        assert!(!changes.contains(&elsewhere.id));

        // Its snippets show the moved text where it went, and nothing else around it.
        let shown = doc.snippets_in(&[moved.id], &stretch).snippets;
        assert_eq!(shown.iter().map(render).collect::<Vec<_>>(), ["[>Gulls circled.]"]);
    }

    #[test]
    fn a_stretch_stays_right_while_writing() {
        let mut doc = Doc::new(DeviceId(1));
        let rain = doc.insert_local(0, "The rain had stopped.", 1_000);
        let stretch = doc.stretch_at(4, 4).expect("\"rain\"");
        let before = doc.insert_local(0, "Outside, ", 2_000);  // before it: not part of it
        let inside = doc.insert_local(15, "-", 3_000);        // "ra-in": strictly inside, so part of it
        let edge = doc.insert_local(18, "y", 4_000);          // "ra-iny": at its edge, so not
        let deleted = doc.delete_local(14, 1, 5_000);         // the "a"
        assert_eq!(doc.text(), "Outside, The r-iny had stopped.");
        let changes = doc.changes_in(&stretch);
        assert_eq!(changes, vec![rain.id, inside.id, deleted.id]);
        assert!(!changes.contains(&before.id) && !changes.contains(&edge.id));
        // A Revert of something in it is in it too.
        let revert = doc.revert_local(deleted.id, false, 6_000);
        assert!(doc.changes_in(&stretch).contains(&revert.id));
    }
    #[test]
    fn an_empty_paragraph_has_no_stretch() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "One.\n\nTwo.", 1_000);
        assert!(doc.stretch_at(5, 0).is_none());
        assert!(doc.stretch_at(0, 0).is_some());
    }
}
