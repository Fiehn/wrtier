//! Snippets: what some Changes did, paragraph by paragraph, shown in the text as it is now.
//! The History view uses them for a Session, a Revert preview and a comparison. Read-only.
//!
//! Every character, deleted ones included, has a place in the reading order, so a snippet is a
//! stretch of that order with each character marked against the Changes: added, added and since
//! deleted, removed, removed and since back, moved, or untouched. Untouched deleted characters
//! aren't shown. A word with changed letters is shown whole, as the old word struck through and
//! the new one highlighted, so "stopped" → "stops" reads as one word replaced, not as letters.
//!
//! Snippets are split at line breaks the Changes didn't touch, so text they wrote, deleted or
//! moved across several paragraphs stays one snippet (shortened in the middle when long).

use super::history::{is_word_char, Did, Effect};
use super::{positions, Doc};
use crate::change::{is_library_copy, PREAMBLE_RECORD};
use crate::ids::{ChangeId, CharId};
use crate::layout::Layout;
use rustc_hash::{FxHashMap, FxHashSet};

/// A paragraph up to this long (in characters) is shown whole.
const WHOLE_PARAGRAPH: usize = 300;
/// Otherwise, about a sentence either side of what changed, up to this long.
const CONTEXT: usize = 160;
/// A changed stretch longer than this is shortened in the middle...
const LONG_RUN: usize = 400;
/// ...keeping about this much at either end.
const KEEP_OF_RUN: usize = 150;
/// How much text before a Move's old place is quoted ("after '…the harbour wall.'").
const QUOTE: usize = 40;

/// What some Changes did: their snippets in reading order, then their Preamble lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippets {
    pub snippets: Vec<Snippet>,
    pub fields: Vec<FieldLine>,
}

/// One paragraph (or run of paragraphs) the Changes touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub segments: Vec<Segment>,
    /// The Changes (of those asked about) that touched it, in the fixed Change order:
    /// what reverting this snippet would revert.
    pub changes: Vec<ChangeId>,
    /// Where it is in the History's visible text (`committed_text`), start..end in characters:
    /// what it touched that's still visible, or an empty range where its text was.
    pub at: (usize, usize),
    /// The Moves shown in it, with where their text came from.
    pub moves: Vec<MovedFrom>,
    /// What to select when it's clicked, held by identity so it stays right while the Author
    /// writes: its first and last visible changed characters, or (nothing of it visible) the
    /// visible character just after where it was, with None. None if there's nothing to go to.
    pub find: Option<(CharId, Option<CharId>)>,
    /// Some of its Changes are still in effect (it can be reverted).
    pub in_effect: bool,
    /// The latest Revert in effect that took some of its Changes back (it can be restored).
    pub reverted_by: Option<ChangeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub kind: SegmentKind,
    pub text: String,
    /// It isn't as the Changes left it any more: added text since deleted, or removed text
    /// that's back.
    pub faded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    /// Context the Changes didn't touch.
    Plain,
    /// Highlighted: what they added (a whole word, if they changed some of its letters).
    Added,
    /// Struck through: what they removed (a whole word, as it was).
    Removed,
    /// Text they moved here (or back here).
    Moved,
    /// Context left out: "…".
    Cut,
    /// The middle of a long changed stretch, left out: "…+212 words…".
    Elided { words: usize },
}

/// Where a Move's text came from, for "Moved here from Chapter 3, after '…'".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovedFrom {
    pub change: ChangeId,
    /// A Revert of the Move put the text back here (rather than the Move bringing it).
    pub back: bool,
    /// The Chapter (top-level heading) the place it came from is in, if any.
    pub chapter: Option<String>,
    /// The text just before that place, on its line; empty at the start of a line.
    pub after: String,
    /// It came from earlier in the text than where it is now.
    pub earlier: bool,
}

/// A Preamble setting, or a recorded library file, the Changes set or took back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldLine {
    /// The Change (of those asked about) it comes from.
    pub by: ChangeId,
    /// The Change that set the value (`by` itself, or what a Revert in `by` took back).
    pub change: ChangeId,
    pub record: String,
    pub field: String,
    pub value: Option<String>,
    /// The value it replaced, as the setting Device saw it.
    pub before: Option<String>,
    /// A Revert took this setting back (rather than making it).
    pub taken_back: bool,
    /// A Work's copy of a library file, recorded by the app on its own: shown as one quiet line.
    pub library: bool,
    /// `by` is still in effect (it can be reverted; otherwise restored).
    pub in_effect: bool,
}

/// How one character stands against the Changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mark {
    Plain,
    Added,
    /// Added by them, deleted since.
    Gone,
    Removed,
    /// Removed by them, back since.
    Back,
    Moved,
}

impl Mark {
    fn visible(self) -> bool {
        matches!(self, Mark::Plain | Mark::Added | Mark::Back | Mark::Moved)
    }
}

pub(super) struct Item {
    pub(super) node: u32,
    pub(super) ch: char,
    pub(super) mark: Mark,
    /// Its index in the visible text, or (not visible) the index of the next visible character.
    pub(super) vis: usize,
}

/// Which Changes touched each character.
#[derive(Default)]
struct Touch {
    shown: bool,
    hidden: bool,
    by: Vec<ChangeId>,
}

/// Where the text is drawn (a fresh layout: typing updates the Doc's own by a shortcut that
/// doesn't keep its reading order), and what the Moves among the Changes did: Move → the
/// Change it comes from.
#[derive(Default)]
struct Moved {
    layout: Layout,
    here: FxHashMap<ChangeId, ChangeId>,
    back: FxHashMap<ChangeId, ChangeId>,
    /// Nodes a taken-back Move put back where they were → that Move.
    back_nodes: FxHashMap<u32, ChangeId>,
}

impl Doc {
    /// What these Changes did (the content ones; others are ignored), as snippets in reading
    /// order and Preamble lines.
    pub fn snippets(&self, changes: &[ChangeId]) -> Snippets {
        self.snippets_within(changes, None, false)
    }

    /// What reverting these Changes now would do, for a preview: text it would take out is
    /// removed, text it would bring back is added, and text it would move back is moved (with
    /// `MovedFrom` saying where it would go). Nothing is faded, and there's nothing to `find`.
    /// Restoring is reverting the Reverts, so its preview is this, given the Reverts.
    pub fn revert_preview(&self, changes: &[ChangeId]) -> Snippets {
        self.snippets_within(changes, None, true)
    }

    /// The same, clipped to a stretch of text ("History of this text"): only its characters
    /// are shown, so a snippet doesn't spill into the paragraph around it.
    pub fn snippets_in(&self, changes: &[ChangeId], stretch: &super::Stretch) -> Snippets {
        let keep = self.stretch_nodes(stretch);
        self.snippets_within(changes, Some(&keep), false)
    }

    /// `preview`: as if a Revert of `changes` were made now (see `revert_preview`).
    fn snippets_within(&self, changes: &[ChangeId], keep: Option<&FxHashSet<u32>>, preview: bool) -> Snippets {
        let mut effect = Effect::default();
        for &id in changes {
            if self.content_set.contains(&id) {
                self.effect(id, id, !preview, &mut effect);
            }
        }
        // For a preview: what the Revert would target.
        let targets: FxHashSet<ChangeId> = changes.iter().copied().filter(|id| self.content_set.contains(id)).collect();
        let mut touch: FxHashMap<u32, Touch> = FxHashMap::default();
        for &(n, by) in &effect.shown {
            let t = touch.entry(n).or_default();
            t.shown = true;
            t.by.push(by);
        }
        for &(n, by) in &effect.hidden {
            let t = touch.entry(n).or_default();
            t.hidden = true;
            t.by.push(by);
        }

        let mut moved = Moved { layout: Layout::compute(&self.tree, &self.in_effect_moves(), &self.typed_into), ..Moved::default() };
        for &(m, made, by) in &effect.moves {
            let happened = self.is_in_effect(m) && !moved.layout.is_skipped(m);
            if preview {
                // Text a Move put here that the Revert would send back (drawn here now), or
                // text it would move again (back where it was now).
                if !made && happened {
                    moved.here.insert(m, by);
                } else if made && !self.is_in_effect(m) {
                    moved.back.insert(m, by);
                }
            } else if made && happened {
                moved.here.insert(m, by);
            } else if !made && !self.is_in_effect(m) {
                moved.back.insert(m, by);
            }
        }
        let (order, pos) = if effect.moves.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            let order = self.tree.order();
            let pos = positions(&order, self.tree.len());
            (order, pos)
        };
        for &m in moved.back.keys() {
            for span in self.pieces_moved_by(m) {
                let (Some(first), Some(last)) = (self.tree.index_of(span.first), self.tree.index_of(span.last)) else { continue };
                for &n in &order[pos[first as usize] as usize..=pos[last as usize] as usize] {
                    moved.back_nodes.entry(n).or_insert(m);
                }
            }
        }

        // Every character in reading order, marked; untouched deleted ones left out.
        let mut items = Vec::new();
        let mut vis = 0;
        for node in moved.layout.reading_nodes(&self.tree) {
            if keep.is_some_and(|k| !k.contains(&node)) {
                continue;
            }
            let visible = self.tree.is_visible_at(node);
            let mark = match touch.get(&node).map(|t| (t.shown, t.hidden)) {
                // A preview: what would be visible once reverted, against what is now (a later
                // deletion still in effect keeps text gone, whatever the Revert brings back).
                Some(_) if preview => match (visible, self.visible_if_reverted(node, &targets)) {
                    (true, false) => Mark::Removed,
                    (false, true) => Mark::Added,
                    (false, false) => continue,
                    (true, true) => if self.mover(node, &moved).is_some() { Mark::Moved } else { Mark::Plain },
                },
                Some((true, false)) => if visible { Mark::Added } else { Mark::Gone },
                Some((false, true)) => if visible { Mark::Back } else { Mark::Removed },
                // Untouched, or both added and removed by them (nothing to show for it).
                _ if !visible => continue,
                _ if self.mover(node, &moved).is_some() => Mark::Moved,
                _ => Mark::Plain,
            };
            let ch = self.tree.char_at(node).expect("reading order has no anchors");
            items.push(Item { node, ch, mark, vis });
            if visible {
                vis += 1;
            }
        }

        let mut where_from = None; // worked out once, only if a snippet shows a Move
        let mut snippets = Vec::new();
        for paragraph in items.split(|it| it.ch == '\n' && it.mark == Mark::Plain) {
            if paragraph.iter().all(|it| it.mark == Mark::Plain) {
                continue;
            }
            let mut by: Vec<ChangeId> = Vec::new();
            let mut moves: Vec<(ChangeId, bool)> = Vec::new();
            for it in paragraph {
                match it.mark {
                    Mark::Plain => {}
                    Mark::Moved => {
                        let m = self.mover(it.node, &moved).expect("marked as moved");
                        let back = moved.back.contains_key(&m);
                        by.push(if back { moved.back[&m] } else { moved.here[&m] });
                        if !moves.contains(&(m, back)) {
                            moves.push((m, back));
                        }
                    }
                    _ => by.extend(touch[&it.node].by.iter().copied()),
                }
            }
            by.sort_by_key(|id| self.made[id].hlc_key(*id));
            by.dedup();
            let shown: Vec<usize> = paragraph.iter().filter(|it| it.mark != Mark::Plain && it.mark.visible()).map(|it| it.vis).collect();
            let at = match (shown.iter().min(), shown.iter().max()) {
                (Some(&lo), Some(&hi)) => (lo, hi + 1),
                _ => {
                    let first = paragraph.iter().find(|it| it.mark != Mark::Plain).map_or(0, |it| it.vis);
                    (first, first)
                }
            };
            let moves = moves.into_iter().map(|(m, back)| {
                let wf = where_from.get_or_insert_with(|| WhereFrom::new(self, &self.visible));
                let to = paragraph.iter().find(|it| it.mark == Mark::Moved).map_or(at.0, |it| it.vis);
                let visible = |n: u32| self.tree.is_visible_at(n);
                let drawn_by_it = |n: u32| self.mover(n, &moved) == Some(m);
                self.moved_from(m, back, to, &order, &pos, wf, &visible, &drawn_by_it)
            }).collect();
            let find = if preview { None } else { find_in(self, paragraph, &self.visible) };
            let in_effect = by.iter().any(|&id| self.is_in_effect(id));
            let reverted_by = self.latest_revert_of(&by);
            snippets.push(Snippet { segments: trimmed(elided(segments(paragraph))), changes: by, at, moves, find, in_effect, reverted_by });
        }

        let fields = effect.fields.iter().filter_map(|&(change, set, by)| {
            let Did::Field { record, field } = self.made.get(&change)?.did() else { return None };
            let library = is_library_copy(record);
            if record != PREAMBLE_RECORD && !library {
                return None;
            }
            let replaces = self.fields.get(&(record.clone(), field.clone()))?.iter().find(|v| v.id == change)?.replaces;
            Some(FieldLine {
                by,
                change,
                record: record.clone(),
                field: field.clone(),
                value: self.field_set_by(record, field, change).map(str::to_string),
                before: replaces.and_then(|r| self.field_set_by(record, field, r)).map(str::to_string),
                taken_back: !set,
                library,
                in_effect: self.is_in_effect(by),
            })
        }).collect();

        Snippets { snippets, fields }
    }

    /// The pieces a Move took.
    fn pieces_moved_by(&self, m: ChangeId) -> Vec<crate::change::Span> {
        self.moves.iter().find(|(_, info)| info.id == m).map(|(_, info)| info.pieces.clone()).unwrap_or_default()
    }

    /// The Move (among the Changes) that put this node where it's drawn, if any.
    fn mover(&self, node: u32, moved: &Moved) -> Option<ChangeId> {
        if let Some((m, _)) = moved.layout.level_at(node) {
            if moved.here.contains_key(&m) {
                return Some(m);
            }
        }
        moved.back_nodes.get(&node).copied()
    }

    /// Where a Move's text came from: the visible text just before its old place. For a Move
    /// that happened, that's before its first character in the tree; for one taken back, before
    /// its anchor (where it had put the text).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn moved_from(&self, m: ChangeId, back: bool, to: usize, order: &[u32], pos: &[u32], wf: &WhereFrom,
                             visible: &dyn Fn(u32) -> bool, drawn_by_it: &dyn Fn(u32) -> bool) -> MovedFrom {
        let start = if back {
            self.tree.index_of(CharId { change: m, offset: 0 })
        } else {
            self.pieces_moved_by(m).first().and_then(|span| self.tree.index_of(span.first))
        };
        let mut origin = None;
        if let Some(start) = start {
            let mut i = pos[start as usize] as usize;
            while i > 0 {
                i -= 1;
                let n = order[i];
                if visible(n) && !drawn_by_it(n) {
                    origin = Some(wf.vis_of[n as usize] as usize);
                    break;
                }
            }
        }
        let (chapter, after) = match origin {
            Some(at) => (wf.chapter_at(at), wf.quote_before(at + 1)),
            None => (None, String::new()), // the very start of the text
        };
        MovedFrom { change: m, back, chapter, after, earlier: origin.is_none_or(|o| o < to) }
    }
}

/// The visible text, its Chapters and every node's place in it: for describing where moved
/// text came from.
pub(super) struct WhereFrom {
    text: Vec<char>,
    /// Per tree node: its index in the visible text.
    vis_of: Vec<u32>,
    /// Each Chapter's first character and its title, in order.
    chapters: Vec<(usize, String)>,
}

impl WhereFrom {
    /// For a version of the text whose visible nodes, in reading order, are isible.
    pub(super) fn new(doc: &Doc, visible: &[u32]) -> WhereFrom {
        let mut vis_of = vec![u32::MAX; doc.tree.len()];
        for (i, &n) in visible.iter().enumerate() {
            vis_of[n as usize] = i as u32;
        }
        let text: Vec<char> = visible.iter().filter_map(|&n| doc.tree.char_at(n)).collect();
        let mut chapters = Vec::new();
        let mut line_start = 0;
        for i in 0..=text.len() {
            if i == text.len() || text[i] == '\n' {
                let line: String = text[line_start..i].iter().collect();
                if let Some(title) = line.strip_prefix("= ") {
                    chapters.push((line_start, title.trim().to_string()));
                }
                line_start = i + 1;
            }
        }
        WhereFrom { text, vis_of, chapters }
    }

    fn chapter_at(&self, at: usize) -> Option<String> {
        self.chapters.iter().rev().find(|(start, _)| *start <= at).map(|(_, title)| title.clone())
    }

    /// Up to `QUOTE` characters of the line before `end`, starting at a word.
    fn quote_before(&self, end: usize) -> String {
        let end = end.min(self.text.len());
        let line_start = self.text[..end].iter().rposition(|&c| c == '\n').map_or(0, |i| i + 1);
        let mut start = line_start.max(end.saturating_sub(QUOTE));
        if start > line_start {
            // Start at the next word, not halfway into one.
            while start < end && !self.text[start - 1].is_whitespace() {
                start += 1;
            }
        }
        self.text[start..end].iter().collect::<String>().trim().to_string()
    }
}

/// What clicking a snippet selects (see Snippet::find). isible: that version's visible
/// nodes in reading order.
pub(super) fn find_in(doc: &Doc, paragraph: &[Item], visible: &[u32]) -> Option<(CharId, Option<CharId>)> {
    let mut shown = paragraph.iter().filter(|it| it.mark != Mark::Plain && it.mark.visible());
    if let Some(first) = shown.next() {
        let last = shown.last().unwrap_or(first);
        return Some((doc.tree.id_of(first.node), Some(doc.tree.id_of(last.node))));
    }
    let gone = paragraph.iter().find(|it| it.mark != Mark::Plain)?;
    visible.get(gone.vis).map(|&n| (doc.tree.id_of(n), None))
}

/// A paragraph's segments, with changed words shown whole.
pub(super) fn segments(items: &[Item]) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < items.len() {
        if is_word_char(items[i].ch) {
            let end = items[i..].iter().position(|it| !is_word_char(it.ch)).map_or(items.len(), |k| i + k);
            word(&items[i..end], &mut out);
            i = end;
        } else {
            let it = &items[i];
            let (kind, faded) = match it.mark {
                Mark::Plain => (SegmentKind::Plain, false),
                Mark::Moved => (SegmentKind::Moved, false),
                Mark::Added => (SegmentKind::Added, false),
                Mark::Gone => (SegmentKind::Added, true),
                Mark::Removed => (SegmentKind::Removed, false),
                Mark::Back => (SegmentKind::Removed, true),
            };
            push(&mut out, kind, faded, &it.ch.to_string());
            i += 1;
        }
    }
    out
}

/// One word. If its letters were changed, it's the old word struck through and the new one
/// highlighted, each whole.
fn word(items: &[Item], out: &mut Vec<Segment>) {
    let marks = |wanted: &[Mark]| items.iter().filter(|it| wanted.contains(&it.mark)).collect::<Vec<_>>();
    let removed = marks(&[Mark::Removed, Mark::Back]);
    let added = marks(&[Mark::Added, Mark::Gone]);
    if removed.is_empty() && added.is_empty() {
        for it in items {
            let kind = if it.mark == Mark::Moved { SegmentKind::Moved } else { SegmentKind::Plain };
            push(out, kind, false, &it.ch.to_string());
        }
        return;
    }
    let text = |wanted: &[Mark]| marks(wanted).iter().map(|it| it.ch).collect::<String>();
    let partly = items.iter().any(|it| matches!(it.mark, Mark::Plain | Mark::Moved));
    let old = text(&[Mark::Plain, Mark::Moved, Mark::Removed, Mark::Back]);
    let new = text(&[Mark::Plain, Mark::Moved, Mark::Added, Mark::Gone]);
    if (partly || !removed.is_empty()) && !old.is_empty() {
        let faded = !removed.is_empty() && removed.iter().all(|it| it.mark == Mark::Back);
        push(out, SegmentKind::Removed, faded, &old);
    }
    if (partly || !added.is_empty()) && !new.is_empty() {
        let faded = !added.is_empty() && added.iter().all(|it| it.mark == Mark::Gone);
        push(out, SegmentKind::Added, faded, &new);
    }
}

/// Adds text, joining it to the last segment when that's the same kind.
fn push(out: &mut Vec<Segment>, kind: SegmentKind, faded: bool, text: &str) {
    match out.last_mut() {
        Some(last) if last.kind == kind && last.faded == faded => last.text.push_str(text),
        _ => out.push(Segment { kind, text: text.to_string(), faded }),
    }
}

pub(super) fn words_in(text: &str) -> usize {
    text.split(|c: char| !is_word_char(c)).filter(|w| !w.is_empty()).count()
}

/// Long changed stretches shortened in the middle: "…+212 words…".
pub(super) fn elided(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out = Vec::new();
    for seg in segments {
        let chars: Vec<char> = seg.text.chars().collect();
        if seg.kind == SegmentKind::Plain || chars.len() <= LONG_RUN {
            out.push(seg);
            continue;
        }
        let head = snap_back(&chars, KEEP_OF_RUN);
        let tail = snap_forward(&chars, chars.len() - KEEP_OF_RUN);
        let part = |range: std::ops::Range<usize>| chars[range].iter().collect::<String>();
        out.push(Segment { text: part(0..head), ..seg.clone() });
        out.push(Segment { kind: SegmentKind::Elided { words: words_in(&part(head..tail)) }, text: String::new(), faded: seg.faded });
        out.push(Segment { text: part(tail..chars.len()), ..seg });
    }
    out
}

/// A long paragraph's context cut down to about a sentence either side of what changed.
pub(super) fn trimmed(segments: Vec<Segment>) -> Vec<Segment> {
    let length: usize = segments.iter().map(|s| s.text.chars().count()).sum();
    if length <= WHOLE_PARAGRAPH {
        return segments;
    }
    let changed: Vec<bool> = segments.iter().map(|s| s.kind != SegmentKind::Plain).collect();
    let mut out = Vec::new();
    for (i, seg) in segments.into_iter().enumerate() {
        if seg.kind != SegmentKind::Plain {
            out.push(seg);
            continue;
        }
        let chars: Vec<char> = seg.text.chars().collect();
        let head = if changed[..i].iter().any(|&c| c) { sentence_after(&chars) } else { 0 };
        let tail = if changed[i + 1..].iter().any(|&c| c) { sentence_before(&chars) } else { chars.len() };
        if head >= tail {
            out.push(seg);
            continue;
        }
        if head > 0 {
            out.push(Segment { kind: SegmentKind::Plain, text: chars[..head].iter().collect(), faded: false });
        }
        out.push(Segment { kind: SegmentKind::Cut, text: String::new(), faded: false });
        if tail < chars.len() {
            out.push(Segment { kind: SegmentKind::Plain, text: chars[tail..].iter().collect(), faded: false });
        }
    }
    out
}

fn ends_sentence(chars: &[char], i: usize) -> bool {
    matches!(chars[i], '.' | '!' | '?') && chars.get(i + 1).is_none_or(|c| c.is_whitespace())
}

/// How much of the text after a change to keep: to the end of the sentence it's in, at most
/// `CONTEXT` characters.
fn sentence_after(chars: &[char]) -> usize {
    match (0..chars.len().min(CONTEXT)).find(|&i| ends_sentence(chars, i)) {
        Some(i) => i + 1,
        None if chars.len() <= CONTEXT => chars.len(),
        None => snap_back(chars, CONTEXT),
    }
}

/// Where to start the text kept before a change: the start of the sentence it's in (or the
/// one before, if that's barely begun), at most `CONTEXT` characters back.
fn sentence_before(chars: &[char]) -> usize {
    let floor = chars.len().saturating_sub(CONTEXT);
    let start_after = |end: usize| (floor..end).rev().find(|&i| ends_sentence(chars, i)).map(|i| i + 1);
    let mut start = start_after(chars.len());
    if let Some(s) = start {
        if chars.len() - s < 20 {
            start = start_after(s.saturating_sub(1)).or(Some(s));
        }
    }
    match start {
        Some(s) => s + chars[s..].iter().take_while(|c| c.is_whitespace()).count(),
        None if floor == 0 => 0,
        None => snap_forward(chars, floor),
    }
}

/// `at`, moved back to just after a space so no word is cut (unless there's none).
fn snap_back(chars: &[char], at: usize) -> usize {
    (1..=at).rev().find(|&i| chars[i - 1].is_whitespace()).unwrap_or(at)
}

/// `at`, moved forward to the start of the next word.
fn snap_forward(chars: &[char], at: usize) -> usize {
    (at..chars.len()).find(|&i| i > 0 && chars[i - 1].is_whitespace()).unwrap_or(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::SESSION_GAP_MS;
    use crate::ids::DeviceId;

    const LATER: u64 = SESSION_GAP_MS + 1;

    /// A snippet as one line: [+added] [-removed] [~+added, since deleted] [>moved] … …+n words…
    fn render(snippet: &Snippet) -> String {
        snippet.segments.iter().map(|s| {
            let fade = if s.faded { "~" } else { "" };
            match s.kind {
                SegmentKind::Plain => s.text.clone(),
                SegmentKind::Added => format!("[{fade}+{}]", s.text),
                SegmentKind::Removed => format!("[{fade}-{}]", s.text),
                SegmentKind::Moved => format!("[>{}]", s.text),
                SegmentKind::Cut => "…".into(),
                SegmentKind::Elided { words } => format!("…+{words} words…"),
            }
        }).collect()
    }

    fn rendered(doc: &Doc, changes: &[ChangeId]) -> Vec<String> {
        doc.snippets(changes).snippets.iter().map(render).collect()
    }

    fn newest_session(doc: &Doc) -> Vec<ChangeId> {
        doc.sessions()[0].changes.clone()
    }

    #[test]
    fn a_changed_word_reads_as_one_word_replaced() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.", 1_000);
        doc.delete_local(13, 7, 1_000 + LATER);
        doc.insert_local(13, "eased", 2_000 + LATER);
        assert_eq!(rendered(&doc, &newest_session(&doc)), ["The rain had [-stopped][+eased]."]);

        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.", 1_000);
        doc.delete_local(17, 3, 1_000 + LATER); // "ped"
        doc.insert_local(17, "s", 2_000 + LATER);
        assert_eq!(rendered(&doc, &newest_session(&doc)), ["The rain had [-stopped][+stops]."]);
    }

    #[test]
    fn a_revert_preview_shows_what_would_go_and_a_restore_preview_what_would_come_back() {
        let mut doc = Doc::new(DeviceId(1));
        let rain = doc.insert_local(0, "The rain had stopped.", 1_000);
        doc.delete_local(13, 7, 1_000 + LATER);
        let eased = doc.insert_local(13, "eased", 2_000 + LATER);
        let render_preview = |doc: &Doc, changes: &[ChangeId]| doc.revert_preview(changes).snippets.iter().map(render).collect::<Vec<_>>();
        assert_eq!(render_preview(&doc, &[rain.id]), ["[-The rain had ]eased[-.]"], "without its dependents, \"eased\" stays");
        assert_eq!(render_preview(&doc, &[rain.id, eased.id]), ["[-The rain had eased.]"]);
        let preview = doc.revert_preview(&[rain.id]);
        assert!(preview.snippets[0].find.is_none() && preview.snippets[0].in_effect);
        assert_eq!(doc.text(), "The rain had eased.", "a preview changes nothing");

        let revert = doc.revert_changes_local(&[rain.id], true, 3_000 + LATER).unwrap();
        let reverts = doc.reverts_of(&[eased.id]);
        assert_eq!(reverts, vec![revert.id]);
        assert_eq!(render_preview(&doc, &reverts), ["[+The rain had eased.]"]);
        let shown = doc.snippets(&[rain.id]).snippets;
        assert!(!shown[0].in_effect);
        assert_eq!(shown[0].reverted_by, Some(revert.id));
    }

    #[test]
    fn a_preview_of_reverting_a_move_says_where_the_text_goes_back_to() {
        let mut doc = Doc::new(DeviceId(1));
        let text = "= One\nThe rain had stopped.\n= Two\nGulls circled.\n";
        doc.insert_local(0, text, 1_000);
        let mv = doc.move_local(text.find("Gulls").unwrap(), 15, text.find("= Two").unwrap(), 2_000);
        let preview = doc.revert_preview(&[mv.id]).snippets;
        assert_eq!(preview.iter().map(render).collect::<Vec<_>>(), ["[>Gulls circled.\n]= Two"]);
        assert_eq!(preview[0].moves[0].chapter.as_deref(), Some("Two"), "it goes back into Chapter Two");
    }

    #[test]
    fn clicking_a_snippet_finds_its_text_even_after_more_typing() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.", 1_000);
        doc.delete_local(13, 7, 1_000 + LATER);
        let eased = doc.insert_local(13, "eased", 2_000 + LATER);
        let find = doc.snippets(&[eased.id]).snippets[0].find.expect("something to go to");
        assert_eq!(doc.shown_range(find.0, find.1), Some((13, 18)));
        doc.type_text(0, "Outside, ", 3_000 + LATER); // a burst still open, before it
        assert_eq!(doc.shown_range(find.0, find.1), Some((22, 27)));
        // Text since deleted: the place where it was.
        doc.close_burst(4_000 + LATER);
        doc.delete_local(22, 5, 5_000 + LATER);
        let find = doc.snippets(&[eased.id]).snippets[0].find.expect("its place");
        assert_eq!(find.1, None);
        assert_eq!(doc.shown_range(find.0, find.1), Some((22, 22)));
    }

    #[test]
    fn each_paragraph_is_its_own_snippet_and_says_where_it_is() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "One.\nTwo.\nThree.", 1_000);
        let a = doc.insert_local(3, " more", 1_000 + LATER);
        let b = doc.insert_local(20, " more", 2_000 + LATER); // after "Three"
        let snippets = doc.snippets(&[a.id, b.id]).snippets;
        assert_eq!(snippets.iter().map(render).collect::<Vec<_>>(), ["One[+ more].", "Three[+ more]."]);
        assert_eq!(snippets[0].changes, vec![a.id]);
        assert_eq!(snippets[1].changes, vec![b.id]);
        assert_eq!(snippets[0].at, (3, 8));
        assert_eq!(&doc.text()[3..8], " more");
    }

    #[test]
    fn text_added_and_since_deleted_shows_faded() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "First.\n\nThird.", 1_000);
        let second = doc.insert_local(7, "Second paragraph.\n", 1_000 + LATER);
        doc.delete_local(7, 18, 1_000 + 2 * LATER);
        assert_eq!(doc.text(), "First.\n\nThird.");
        let snippets = doc.snippets(&[second.id]).snippets;
        assert_eq!(snippets.iter().map(render).collect::<Vec<_>>(), ["[~+Second paragraph.\n]"]);
        assert_eq!(snippets[0].at, (7, 7), "where it was");
    }

    #[test]
    fn a_move_is_one_snippet_that_names_its_chapter() {
        let mut doc = Doc::new(DeviceId(1));
        let text = "= One\nThe rain had stopped.\n= Two\nThe harbour wall. Gulls circled.\n";
        doc.insert_local(0, text, 1_000);
        let from = text.find("Gulls").unwrap();
        let to = text.find("= Two").unwrap();
        let mv = doc.move_local(from, "Gulls circled.\n".len(), to, 1_000 + LATER);
        assert_eq!(doc.text(), "= One\nThe rain had stopped.\nGulls circled.\n= Two\nThe harbour wall. ");
        let snippets = doc.snippets(&[mv.id]);
        assert_eq!(snippets.snippets.iter().map(render).collect::<Vec<_>>(), ["[>Gulls circled.\n]= Two"]);
        let moved = &snippets.snippets[0].moves;
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].chapter.as_deref(), Some("Two"));
        assert_eq!(moved[0].after, "The harbour wall.");
        assert!(!moved[0].earlier, "it came from later in the text");
        assert!(!moved[0].back);
        assert_eq!(snippets.snippets[0].changes, vec![mv.id]);
    }

    #[test]
    fn reverting_a_move_shows_the_text_moved_back() {
        let mut doc = Doc::new(DeviceId(1));
        let text = "= One\nThe rain had stopped.\n= Two\nGulls circled.\n";
        doc.insert_local(0, text, 1_000);
        let mv = doc.move_local(text.find("Gulls").unwrap(), 15, text.find("= Two").unwrap(), 2_000);
        let revert = doc.revert_local(mv.id, false, 1_000 + LATER);
        assert_eq!(doc.text(), text);
        let snippets = doc.snippets(&[revert.id]).snippets;
        assert_eq!(snippets.iter().map(render).collect::<Vec<_>>(), ["[>Gulls circled.\n]"]);
        assert!(snippets[0].moves[0].back);
        assert_eq!(snippets[0].moves[0].chapter.as_deref(), Some("One"), "it had been moved into Chapter One");
    }

    #[test]
    fn a_revert_shows_what_it_took_out_and_restoring_it_fades_that() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped. ", 1_000);
        let gulls = doc.insert_local(22, "Gulls circled.", 2_000);
        let revert = doc.revert_local(gulls.id, false, 1_000 + LATER);
        assert_eq!(rendered(&doc, &[revert.id]), ["The rain had stopped. [-Gulls circled.]"]);
        doc.restore_local(gulls.id, 1_000 + 2 * LATER);
        assert_eq!(rendered(&doc, &[revert.id]), ["The rain had stopped. [~-Gulls circled.]"]);
    }

    #[test]
    fn a_long_paragraph_keeps_about_a_sentence_either_side() {
        let mut doc = Doc::new(DeviceId(1));
        let sentence = "The rain had stopped and the gulls circled over the harbour wall. ";
        let text = sentence.repeat(5) + "She walked along the quiet wall. " + &sentence.repeat(5);
        doc.insert_local(0, &text, 1_000);
        let at = text.find("quiet").unwrap();
        let change = doc.insert_local(at, "very ", 1_000 + LATER);
        assert_eq!(rendered(&doc, &[change.id]), ["…She walked along the [+very ]quiet wall.…"]);
    }

    #[test]
    fn a_long_stretch_is_shortened_in_the_middle() {
        let mut doc = Doc::new(DeviceId(1));
        let text = "word ".repeat(300);
        let paste = doc.insert_local(0, &text, 1_000);
        let shown = rendered(&doc, &[paste.id]);
        assert_eq!(shown.len(), 1);
        assert!(shown[0].starts_with("[+word word"), "{}", shown[0]);
        assert!(shown[0].contains("]…+240 words…[+"), "{}", shown[0]);
    }

    #[test]
    fn preamble_edits_are_lines_and_library_copies_are_marked() {
        let mut doc = Doc::new(DeviceId(1));
        doc.set_field_local(PREAMBLE_RECORD, "title", "Rain", 1_000);
        let retitled = doc.set_field_local(PREAMBLE_RECORD, "title", "The Rain", 1_000 + LATER);
        let style = doc.set_field_local("style/Book text", "content", "#set text(size: 11pt)", 2_000 + LATER);
        let lines = doc.snippets(&[retitled.id, style.id]).fields;
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].value.as_deref(), lines[0].before.as_deref()), (Some("The Rain"), Some("Rain")));
        assert!(!lines[0].library && lines[1].library);
        let revert = doc.revert_local(retitled.id, false, 3_000 + LATER);
        let undone = doc.snippets(&[revert.id]).fields;
        assert!(undone[0].taken_back);
        assert_eq!(undone[0].change, retitled.id);
        assert_eq!(undone[0].by, revert.id);
    }
}
