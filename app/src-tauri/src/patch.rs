//! Patches: how the Rust side tells the editor that the text changed underneath it (an undo
//! that reverts a Change, and later Changes merged from other Devices). The smallest set of
//! edits is sent, so the cursor and everything else in the editor stay where they are.

use crate::positions::Positions;
use serde::Serialize;
use similar::{capture_diff_slices_deadline, Algorithm, DiffTag};
use std::time::{Duration, Instant};

/// Replace UTF-16 positions `from..to` of the editor's text with `insert`. A list of them is
/// in positions of the text before any of them, sorted and not overlapping (as CodeMirror takes it).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub from: usize,
    pub to: usize,
    pub insert: String,
}

/// The edits that turn `old` into `new`.
pub fn patches(old: &str, new: &str) -> Vec<Patch> {
    let (old_chars, new_chars): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    // Edits are nearly always in a few places: skip the common start and end before diffing.
    let prefix = old_chars.iter().zip(&new_chars).take_while(|(a, b)| a == b).count();
    let room = old_chars.len().min(new_chars.len()) - prefix;
    let suffix = old_chars.iter().rev().zip(new_chars.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
    let a = &old_chars[prefix..old_chars.len() - suffix];
    let b = &new_chars[prefix..new_chars.len() - suffix];
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    let positions = Positions::new(old);
    // A diff that takes too long just gets coarser, never wrong.
    let ops = capture_diff_slices_deadline(Algorithm::Myers, a, b, Some(Instant::now() + Duration::from_millis(50)));
    let mut patches: Vec<Patch> = Vec::new();
    for op in ops {
        let (tag, old_range, new_range) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            continue;
        }
        let from = positions.to_utf16(prefix + old_range.start);
        let to = positions.to_utf16(prefix + old_range.end);
        let insert: String = b[new_range].iter().collect();
        // A deletion right before an insertion at the same spot is one replacement.
        match patches.last_mut() {
            Some(last) if last.to == from => {
                last.to = to;
                last.insert.push_str(&insert);
            }
            _ => patches.push(Patch { from, to, insert }),
        }
    }
    patches
}

/// `text` with `patches` applied (for tests, and to check them).
pub fn apply(text: &str, patches: &[Patch]) -> String {
    let mut result = text.to_string();
    for patch in patches.iter().rev() {
        result = crate::positions::replace_utf16(&result, patch.from, patch.to, &patch.insert);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(old: &str, new: &str) -> Vec<Patch> {
        let patches = patches(old, new);
        assert_eq!(apply(old, &patches), new, "{old:?} → {new:?}: {patches:?}");
        patches
    }

    #[test]
    fn an_edit_stays_where_it_was_made() {
        // Diffed by character, so letters both words share stay ("stopped" → "eased" keeps
        // "s" and "ed"), but nothing outside the changed word is touched.
        let patches = check("The rain had stopped.", "The rain had eased.");
        assert!(patches.iter().all(|p| p.from >= 13 && p.to <= 20), "{patches:?}");
        assert!(check("abc", "abc").is_empty());
        assert_eq!(check("", "new"), vec![Patch { from: 0, to: 0, insert: "new".into() }]);
        assert_eq!(check("gone", ""), vec![Patch { from: 0, to: 4, insert: String::new() }]);
    }

    #[test]
    fn edits_in_two_places_stay_two_patches() {
        // A reverted Move: text goes back from the end to the start. The cursor in the middle
        // ("cursor here") must not be inside any patch.
        let old = "One. Two. cursor here. Three. Moved.";
        let new = "Moved. One. Two. cursor here. Three.";
        let patches = check(old, new);
        let cursor = old.find("cursor").unwrap();
        assert!(patches.iter().all(|p| !(p.from < cursor && cursor < p.to)), "{patches:?}");
    }

    #[test]
    fn positions_are_in_utf16() {
        assert_eq!(check("😊 a 😊", "😊 b 😊"), vec![Patch { from: 3, to: 4, insert: "b".into() }]);
        check("😊😊😊", "😊x😊");
        check("a😊b", "😊");
    }
}
