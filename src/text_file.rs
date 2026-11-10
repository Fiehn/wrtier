//! Comparing an edited Text file with the text the app wrote: what was deleted, what was
//! inserted, and which blocks were moved.
//!
//! Pure functions on characters; `Doc::import_text` turns the result into Changes.

use std::time::{Duration, Instant};
use similar::{capture_diff_slices_deadline, Algorithm, DiffOp};
use crate::ids::{ChangeId, DeviceId};

/// The last line of every Text file the app writes says which version the text above it was
/// written from: the highest Change counter it held from each Device. Another Device that gets
/// the file before the History it came from then knows to wait, and an edit made in another
/// editor is compared against exactly that version (ADR 0010). It's a Typst comment, so it
/// changes nothing when the file is compiled, and it's never part of the Work's text.
const MARKER: &str = "// wrtier version:";

/// A Text file's contents: `text`, then the marker line.
pub fn with_marker(text: &str, version: &[(DeviceId, u32)]) -> String {
    let version: Vec<String> = version.iter().map(|&(device, counter)| ChangeId { device, counter }.to_string()).collect();
    format!("{text}\n{MARKER} {}\n", version.join(", "))
}

/// A Text file's text, and the version its marker line names (None if the line is missing or
/// was edited, as in a file from elsewhere). A byte order mark and Windows line endings, which
/// other editors may add, are dropped first.
pub fn split_marker(file: &str) -> (String, Option<Vec<(DeviceId, u32)>>) {
    let file = file.strip_prefix('\u{feff}').unwrap_or(file).replace("\r\n", "\n");
    let body = file.strip_suffix('\n').unwrap_or(&file); // an editor may drop the last line break
    if let Some((text, last)) = body.rsplit_once('\n') {
        if let Some(version) = last.strip_prefix(MARKER).and_then(|rest| parse_version(rest.trim())) {
            return (text.to_string(), Some(version));
        }
    }
    (file, None)
}

/// The first line of the header the app puts at the top of a Text file: the Work's Preamble as
/// Typst (its Story elements, Styles and Looks), so the file compiles on its own (ADR 0011).
/// Like the marker line, it's never part of the Work's text: it's added when the file is
/// written and taken off when it's read. Only its start is checked, so an edit to the rest of
/// this line still finds it.
pub const HEADER_START: &str = "// ---- This Work's Preamble, made by wrtier from its History. Change it in the app's Preamble dialog: edits here are dropped. ----\n";
const HEADER_START_CHECK: &str = "// ---- This Work's Preamble";
/// The header's last line.
pub const HEADER_END: &str = "// ---- End of the Preamble ----\n";

/// A Text file's contents: the header (empty for none), `text`, then the marker line.
pub fn with_header_and_marker(header: &str, text: &str, version: &[(DeviceId, u32)]) -> String {
    with_marker(&format!("{header}{text}"), version)
}

/// Takes the header off the top of a Text file's text (after `split_marker`): the header as it
/// is in the file, and the Work's text after it. Without a header (an older Text file, or one
/// from elsewhere), or with its end line gone, all of it is text.
pub fn split_header(text: &str) -> (Option<String>, String) {
    if !text.starts_with(HEADER_START_CHECK) {
        return (None, text.to_string());
    }
    let end = HEADER_END.trim_end();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        at += line.len();
        if line.trim_end() == end {
            return (Some(text[..at].to_string()), text[at..].to_string());
        }
    }
    (None, text.to_string())
}

fn parse_version(s: &str) -> Option<Vec<(DeviceId, u32)>> {
    s.split(',').map(str::trim).filter(|p| !p.is_empty())
        .map(|p| p.parse::<ChangeId>().ok().map(|id| (id.device, id.counter)))
        .collect()
}

/// A deleted block that reappears exactly elsewhere is recorded as a Move if it's at least
/// this many characters, or at least one whole line.
pub const MOVE_MIN_CHARS: usize = 40;

/// Comparing gives up refining after this long and settles for a coarser (still correct) result,
/// so a completely rewritten file can't hang the app.
const DIFF_DEADLINE: Duration = Duration::from_secs(2);

/// One stretch where the file differs: `old[old_start..][..old_len]` became `new[new_start..][..new_len]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
}

/// A block deleted in one place and inserted, unchanged, in another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moved {
    pub old_start: usize,
    pub len: usize,
    pub new_start: usize,
    /// The hunk it lands in (where it's inserted).
    pub hunk: usize,
}

/// What an edit did. Every part of every hunk is accounted for exactly once: each deleted
/// old character is in `deleted` or a `moved` block, each new character in `inserted` or one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditPlan {
    pub hunks: Vec<Hunk>,
    pub moved: Vec<Moved>,
    /// Old ranges (start, len) deleted outright.
    pub deleted: Vec<(usize, usize)>,
    /// New ranges (hunk, start, len) inserted as new text.
    pub inserted: Vec<(usize, usize, usize)>,
}

/// Compares `old` with `new`.
pub fn plan(old: &[char], new: &[char]) -> EditPlan {
    let hunks = hunks(old, new);
    let mut deleted: Vec<(usize, usize, usize)> = Vec::new(); // (hunk, start, len)
    let mut inserted: Vec<(usize, usize, usize)> = Vec::new();
    for (h, hunk) in hunks.iter().enumerate() {
        if hunk.old_len > 0 {
            deleted.push((h, hunk.old_start, hunk.old_len));
        }
        if hunk.new_len > 0 {
            inserted.push((h, hunk.new_start, hunk.new_len));
        }
    }
    let mut moved: Vec<Moved> = Vec::new();

    // 1. A deleted block found inside inserted text elsewhere. The whole block, or failing that
    //    the block without the spaces at its edges (cutting a passage from mid-line usually
    //    takes a space along on one side, and pasting adds one on the other).
    let mut i = 0;
    'deleted: while i < deleted.len() {
        let (dh, ds, dl) = deleted[i];
        for (k0, len) in candidates(&old[ds..ds + dl]) {
            let block = &old[ds + k0..ds + k0 + len];
            if !movable(block, old, ds + k0) {
                continue;
            }
            if let Some((j, k)) = find_in(&inserted, new, block, dh) {
                let (ih, is, il) = inserted[j];
                moved.push(Moved { old_start: ds + k0, len, new_start: is + k, hunk: ih });
                split(&mut inserted, j, (ih, is, il), k, len);
                split(&mut deleted, i, (dh, ds, dl), k0, len);
                continue 'deleted; // look at what's left of this segment (now at `i`) again
            }
        }
        i += 1;
    }
    // 2. An inserted block found inside deleted text elsewhere (e.g. one paragraph of a deleted
    //    chapter pasted somewhere else).
    let mut i = 0;
    'inserted: while i < inserted.len() {
        let (ih, is, il) = inserted[i];
        for (k0, len) in candidates(&new[is..is + il]) {
            let block = &new[is + k0..is + k0 + len];
            if !movable(block, new, is + k0) {
                continue;
            }
            if let Some((j, k)) = find_in(&deleted, old, block, ih) {
                let (dh, ds, dl) = deleted[j];
                moved.push(Moved { old_start: ds + k, len, new_start: is + k0, hunk: ih });
                split(&mut deleted, j, (dh, ds, dl), k, len);
                split(&mut inserted, i, (ih, is, il), k0, len);
                continue 'inserted;
            }
        }
        i += 1;
    }

    deleted.sort_by_key(|&(_, s, _)| s);
    inserted.sort_by_key(|&(_, s, _)| s);
    moved.sort_by_key(|m| m.new_start);
    EditPlan {
        hunks,
        moved,
        deleted: deleted.into_iter().map(|(_, s, l)| (s, l)).collect(),
        inserted,
    }
}

/// The stretches where `old` and `new` differ, with touching changes merged into one hunk.
///
/// Compared word by word, not character by character: "stopped" → "eased" is one replacement,
/// not fragments of letters the two happen to share. Every other character (space,
/// punctuation, line break) is a unit of its own.
fn hunks(old: &[char], new: &[char]) -> Vec<Hunk> {
    let (old_units, new_units) = (units(old), units(new));
    let old_slices: Vec<&[char]> = old_units.iter().map(|&(s, l)| &old[s..s + l]).collect();
    let new_slices: Vec<&[char]> = new_units.iter().map(|&(s, l)| &new[s..s + l]).collect();
    let ops = capture_diff_slices_deadline(Algorithm::Myers, &old_slices, &new_slices, Some(Instant::now() + DIFF_DEADLINE));
    // Unit positions → character positions.
    let at = |units: &[(usize, usize)], text_len: usize, i: usize| units.get(i).map_or(text_len, |&(s, _)| s);
    let mut hunks: Vec<Hunk> = Vec::new();
    // Where we are in each sequence. A Delete's `new_index` and an Insert's `old_index` from
    // `similar` aren't always where the change sits (found by the property test), so they're
    // taken from these cursors instead; the unchanged stretches are what keeps them right.
    let (mut o, mut n) = (0usize, 0usize);
    for op in ops {
        let (oi, ol, ni, nl) = match op {
            DiffOp::Equal { old_index, new_index, len } => {
                (o, n) = (old_index + len, new_index + len);
                continue;
            }
            DiffOp::Delete { old_index, old_len, .. } => (old_index, old_len, n, 0),
            DiffOp::Insert { new_index, new_len, .. } => (o, 0, new_index, new_len),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => (old_index, old_len, new_index, new_len),
        };
        (o, n) = (oi + ol, ni + nl);
        let old_start = at(&old_units, old.len(), oi);
        let old_len = at(&old_units, old.len(), oi + ol) - old_start;
        let new_start = at(&new_units, new.len(), ni);
        let new_len = at(&new_units, new.len(), ni + nl) - new_start;
        // Merge with the previous hunk if no unchanged text separates them.
        if let Some(last) = hunks.last_mut() {
            if last.old_start + last.old_len == old_start && last.new_start + last.new_len == new_start {
                last.old_len += old_len;
                last.new_len += new_len;
                continue;
            }
        }
        hunks.push(Hunk { old_start, old_len, new_start, new_len });
    }
    hunks
}

/// The units text is compared in, as (start, len): each run of letters and digits is one unit,
/// every other character is a unit of its own.
fn units(text: &[char]) -> Vec<(usize, usize)> {
    let mut units: Vec<(usize, usize)> = Vec::new();
    for (i, c) in text.iter().enumerate() {
        match units.last_mut() {
            Some((s, l)) if c.is_alphanumeric() && text[*s].is_alphanumeric() && *s + *l == i => *l += 1,
            _ => units.push((i, 1)),
        }
    }
    units
}

/// Big enough to be a Move: at least `MOVE_MIN_CHARS`, or whole lines (starts a line, ends one).
fn movable(block: &[char], text: &[char], start: usize) -> bool {
    let whole_lines = (start == 0 || text[start - 1] == '\n') && block.last() == Some(&'\n');
    !block.is_empty() && (block.len() >= MOVE_MIN_CHARS || whole_lines)
}

/// The blocks of a segment worth looking for elsewhere, as (offset, len): the whole segment,
/// then the segment without the spaces at its edges.
fn candidates(segment: &[char]) -> Vec<(usize, usize)> {
    let lead = segment.iter().take_while(|c| *c == &' ').count();
    let trail = segment[lead..].iter().rev().take_while(|c| *c == &' ').count();
    let mut found = vec![(0, segment.len())];
    if (lead > 0 || trail > 0) && lead + trail < segment.len() {
        found.push((lead, segment.len() - lead - trail));
    }
    found
}

/// The first segment (not in hunk `exclude`) whose text contains `block`: (segment, offset).
fn find_in(segments: &[(usize, usize, usize)], text: &[char], block: &[char], exclude: usize) -> Option<(usize, usize)> {
    segments.iter().enumerate().find_map(|(j, &(h, s, l))| {
        if h == exclude || l < block.len() {
            return None;
        }
        text[s..s + l].windows(block.len()).position(|w| w == block).map(|k| (j, k))
    })
}

/// Takes `len` characters at offset `k` out of segment `j`, keeping what's left on either side.
fn split(segments: &mut Vec<(usize, usize, usize)>, j: usize, (h, s, l): (usize, usize, usize), k: usize, len: usize) {
    let pieces = [(h, s, k), (h, s + k + len, l - k - len)];
    segments.splice(j..=j, pieces.into_iter().filter(|&(_, _, l)| l > 0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_comes_off_exactly() {
        let version = vec![(DeviceId(0x7f8c3ffd), 120), (DeviceId(0x3e91a2b0), 48)];
        let file = with_marker("The rain had stopped.", &version);
        assert_eq!(file, "The rain had stopped.\n// wrtier version: 7f8c3ffd:120, 3e91a2b0:48\n");
        // Whatever the text ends with, it comes back as it was.
        for text in ["", "abc", "abc\n", "abc\n\n", "\n"] {
            assert_eq!(split_marker(&with_marker(text, &version)), (text.to_string(), Some(version.clone())), "{text:?}");
        }
        assert_eq!(split_marker(&with_marker("abc", &[])), ("abc".to_string(), Some(vec![])));
    }

    #[test]
    fn other_editors_may_touch_the_file() {
        let version = vec![(DeviceId(1), 5)];
        // Windows line endings, a byte order mark, and no line break after the marker.
        let file = format!("\u{feff}{}", with_marker("one\ntwo", &version).replace('\n', "\r\n"));
        assert_eq!(split_marker(&file), ("one\ntwo".to_string(), Some(version.clone())));
        assert_eq!(split_marker("abc\n// wrtier version: 00000001:5"), ("abc".to_string(), Some(version)));
        // An edited or missing marker: the whole file is text, compared the old way.
        assert_eq!(split_marker("abc\n// wrtier version: garbled\n").1, None);
        assert_eq!(split_marker("plain text from elsewhere\n"), ("plain text from elsewhere\n".to_string(), None));
    }

    #[test]
    fn the_header_comes_off_exactly() {
        let header = format!("{HEADER_START}#let part(title) = title\n{HEADER_END}");
        let version = vec![(DeviceId(1), 5)];
        for text in ["", "= Rain\n\nThe rain had stopped.", "\n"] {
            let file = with_header_and_marker(&header, text, &version);
            let (body, marked) = split_marker(&file);
            assert_eq!(marked, Some(version.clone()));
            assert_eq!(split_header(&body), (Some(header.clone()), text.to_string()), "{text:?}");
        }
        // No header (an older Text file), or its end line deleted: all of it is text.
        assert_eq!(split_header("= Rain\n"), (None, "= Rain\n".to_string()));
        let broken = format!("{HEADER_START}#let x = 1\nThe rain.");
        assert_eq!(split_header(&broken), (None, broken.clone()));
        // Edited inside, and with Windows line endings (split_marker turns them back).
        let edited = format!("{HEADER_START}#let part(title) = strong(title)\n{HEADER_END}Text.").replace('\n', "\r\n");
        let (body, _) = split_marker(&edited);
        assert_eq!(split_header(&body).1, "Text.");
    }

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn a_changed_word_is_one_hunk() {
        let p = plan(&chars("The rain had stopped."), &chars("The rain had eased."));
        assert!(p.moved.is_empty());
        assert_eq!(p.hunks.len(), 1);
    }

    #[test]
    fn a_moved_line_is_a_move() {
        let old = chars("one\ntwo\nthree\n");
        let new = chars("two\nthree\none\n");
        let p = plan(&old, &new);
        assert_eq!(p.moved, vec![Moved { old_start: 0, len: 4, new_start: 10, hunk: 1 }]);
        assert!(p.deleted.is_empty() && p.inserted.is_empty());
    }

    #[test]
    fn a_long_moved_block_is_a_move_even_mid_line() {
        // (The text left behind is longer than the block, so the block is what moved.)
        let block = "the gulls circled slowly over the harbour wall";
        let rest = "She walked on. The boats rocked at their moorings, and the town was quiet again.";
        let old = chars(&format!("A. {block} {rest}"));
        let new = chars(&format!("A. {rest} {block}"));
        let p = plan(&old, &new);
        assert_eq!(p.moved.len(), 1);
        assert_eq!(&old[p.moved[0].old_start..][..p.moved[0].len], &chars(block)[..]);
    }

    #[test]
    fn a_short_moved_word_is_just_deleted_and_inserted() {
        let p = plan(&chars("alpha beta gamma"), &chars("beta gamma alpha"));
        assert!(p.moved.is_empty());
    }

    #[test]
    fn part_of_a_deleted_chapter_pasted_elsewhere_is_a_move() {
        // Delete three lines near the top; paste just the middle one at the end.
        let old = chars("keep\nA line\nB line\nC line\nthe rest of the story\nmore\n");
        let new = chars("keep\nthe rest of the story\nmore\nB line\n");
        let p = plan(&old, &new);
        assert_eq!(p.moved.len(), 1);
        assert_eq!(&old[p.moved[0].old_start..][..p.moved[0].len], &chars("B line\n")[..]);
    }

    #[test]
    fn the_unchanged_text_between_hunks_lines_up() {
        // Found by the property test: `similar` reported one deletion's position in the new
        // text one too far, which put a space on the wrong side of a line break.
        let old = chars(" aaaafirst line\nsecond line\na \n ae\na athird lin");
        let new = chars("second line\n \n \na \n ae\na athird lin");
        let p = plan(&old, &new);
        let (mut o, mut n) = (0, 0);
        for h in &p.hunks {
            assert_eq!(old[o..h.old_start], new[n..h.new_start], "unchanged text before {h:?}");
            (o, n) = (h.old_start + h.old_len, h.new_start + h.new_len);
        }
        assert_eq!(old[o..], new[n..]);
    }

    #[test]
    fn everything_is_accounted_for_once() {
        let old = chars("one\ntwo\nthree\nfour\n");
        let new = chars("four\nzero\none\nthree\n");
        let p = plan(&old, &new);
        let mut old_count = vec![0; old.len()];
        let mut new_count = vec![0; new.len()];
        for &(s, l) in &p.deleted { (s..s + l).for_each(|i| old_count[i] += 1); }
        for &(_, s, l) in &p.inserted { (s..s + l).for_each(|i| new_count[i] += 1); }
        for m in &p.moved {
            (m.old_start..m.old_start + m.len).for_each(|i| old_count[i] += 1);
            (m.new_start..m.new_start + m.len).for_each(|i| new_count[i] += 1);
        }
        for h in &p.hunks {
            assert!((h.old_start..h.old_start + h.old_len).all(|i| old_count[i] == 1));
            assert!((h.new_start..h.new_start + h.new_len).all(|i| new_count[i] == 1));
        }
    }
}
