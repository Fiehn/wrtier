//! Find and replace: replacing matches the interface found, word by word.
//!
//! The interface compares each match with its replacement word by word and sends only the
//! words that differ, so markup and the rest of the text keep their History. Each of those
//! edits is one Change. A replaced word's new text goes right after its first old character
//! in the tree, inside the old word rather than at its edge, so a Comment that covered the old
//! word covers the new one. When one action made several Changes (a Replace all, or one match
//! that rewrote two words), a `replace_record` ties them together: undo and the History view
//! take them as one.

use super::Doc;
use crate::change::{replace_record, Change};
use crate::ids::ChangeId;

/// One word-level edit: replace `len` characters at `index` (in the text as it was before any
/// of the action's edits) with `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceEdit {
    pub index: usize,
    pub len: usize,
    pub text: String,
}

/// One Find and replace action that made several Changes, as the History view lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replacement {
    pub find: String,
    pub with: String,
    /// How many matches it replaced.
    pub count: usize,
    /// Its Changes, in the order they were made.
    pub changes: Vec<ChangeId>,
}

impl Doc {
    /// Replaces each of `edits` (which mustn't overlap), as one action. `find`, `with` and
    /// `count` say what it was, for the History view. Returns the Changes recorded.
    pub fn replace_matches_local(&mut self, edits: &[ReplaceEdit], find: &str, with: &str, count: usize, now_ms: u64) -> Vec<Change> {
        self.assert_no_burst();
        let mut sorted: Vec<&ReplaceEdit> = edits.iter().collect();
        // Last first: each one only touches the tree after the earlier ones' places, so what's
        // shown (`visible`, the text as it was) stays right for them all, and is worked out
        // again once at the end. (Updating it after each, as typing does, would take seconds
        // for a few thousand matches in a novel.)
        sorted.sort_by_key(|e| std::cmp::Reverse(e.index));
        let mut changes = Vec::new();
        for edit in sorted {
            let end = (edit.index + edit.len).min(self.visible.len());
            assert!(edit.index <= end, "index {} is past the end of the text ({})", edit.index, self.len());
            let doomed: Vec<u32> = self.visible[edit.index..end].to_vec();
            // Inside the old word: after its first character. Nothing to replace: where it's put.
            let left = match doomed.first() {
                Some(&first) if !edit.text.is_empty() => Some(first),
                _ => self.node_before(edit.index),
            };
            if let Some((change, _)) = self.text_change(left, &doomed, &edit.text, None, now_ms) {
                self.apply_change(&change);
                changes.push(change);
            }
        }
        self.refresh();
        if changes.len() > 1 {
            let record = replace_record(changes[0].id);
            let ids = changes.iter().map(|c| c.id.to_string()).collect::<Vec<_>>().join(",");
            let fields = [("changes", ids), ("find", find.to_string()), ("with", with.to_string()), ("count", count.to_string())];
            for (field, value) in fields {
                let change = self.set_field_local(&record, field, &value, now_ms);
                changes.push(change);
            }
        }
        changes
    }

    /// Every Find and replace action that made several Changes.
    fn all_replacements(&self) -> Vec<Replacement> {
        self.records("replace/")
            .into_iter()
            .filter_map(|record| {
                let changes: Vec<ChangeId> = self.field(&record, "changes")?.split(',').filter_map(|id| id.parse().ok()).collect();
                Some(Replacement {
                    find: self.field(&record, "find").unwrap_or_default().to_string(),
                    with: self.field(&record, "with").unwrap_or_default().to_string(),
                    count: self.field(&record, "count").and_then(|c| c.parse().ok()).unwrap_or(changes.len()),
                    changes,
                })
            })
            .collect()
    }

    /// The Changes of the Find and replace action `change` belongs to, if it made several: undo
    /// takes them back together.
    pub fn replace_group(&self, change: ChangeId) -> Option<Vec<ChangeId>> {
        self.all_replacements().into_iter().find(|r| r.changes.contains(&change)).map(|r| r.changes)
    }

    /// The Find and replace actions with a Change among `changes`, for the History view.
    pub fn replacements(&self, changes: &[ChangeId]) -> Vec<Replacement> {
        let mut found: Vec<Replacement> = self.all_replacements().into_iter().filter(|r| r.changes.iter().any(|c| changes.contains(c))).collect();
        found.sort_by_key(|r| r.changes.iter().filter_map(|c| self.made_at(*c)).min());
        found
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::change::AUTHOR;
    use crate::ids::DeviceId;

    fn edit(index: usize, len: usize, text: &str) -> ReplaceEdit {
        ReplaceEdit { index, len, text: text.into() }
    }

    #[test]
    fn replaces_each_edit_as_its_own_change_and_ties_them_together() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The *rain* had stopped. Rain again.", 1000);
        // "rain had" → "snow has" (two words), and "Rain" → "Snow".
        let changes = doc.replace_matches_local(&[edit(5, 4, "snow"), edit(11, 3, "has"), edit(24, 4, "Snow")], "rain", "snow", 2, 2000);
        assert_eq!(doc.text(), "The *snow* has stopped. Snow again.");
        let text: Vec<ChangeId> = changes.iter().filter(|c| matches!(c.kind, crate::change::ChangeKind::Replace { .. })).map(|c| c.id).collect();
        assert_eq!(text.len(), 3);
        let group = doc.replace_group(text[1]).expect("the three are one action");
        assert_eq!(group, text);
        let listed = doc.replacements(&[text[0]]);
        assert_eq!(listed, vec![Replacement { find: "rain".into(), with: "snow".into(), count: 2, changes: text.clone() }]);
        // The grouping isn't content: a Milestone doesn't capture it, a Session doesn't list it.
        assert!(changes[3..].iter().all(|c| !doc.content_set.contains(&c.id)));
    }

    #[test]
    fn one_change_needs_no_group() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "rain", 1000);
        let changes = doc.replace_matches_local(&[edit(0, 4, "snow")], "rain", "snow", 1, 2000);
        assert_eq!(changes.len(), 1);
        assert_eq!(doc.replace_group(changes[0].id), None);
    }

    #[test]
    fn a_replaced_word_keeps_its_comment() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.", 1000);
        let on_word = doc.comment_local(4, 4, AUTHOR, "too gloomy?", 1500).id; // exactly "rain"
        let around = doc.comment_local(0, 12, AUTHOR, "slow start", 1600).id; // "The rain had"
        doc.replace_matches_local(&[edit(4, 4, "drizzle")], "rain", "drizzle", 1, 2000);
        assert_eq!(doc.text(), "The drizzle had stopped.");
        let comments = doc.comments();
        let ranges = |id: ChangeId| comments.iter().find(|c| c.id == id).unwrap().ranges.clone();
        assert_eq!(ranges(on_word), vec![(4, 11)], "not detached: it covers the new word");
        assert_eq!(ranges(around), vec![(0, 15)]);
    }

    #[test]
    fn insertions_and_deletions_go_where_they_belong() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "the rain fell", 1000);
        // "rain fell" → "heavy rain" as word edits: insert "heavy " before, delete " fell".
        doc.replace_matches_local(&[edit(4, 0, "heavy "), edit(8, 5, "")], "rain fell", "heavy rain", 1, 2000);
        assert_eq!(doc.text(), "the heavy rain");
    }

    #[test]
    fn a_revert_of_the_group_takes_it_all_back() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "rain, rain, rain", 1000);
        let changes = doc.replace_matches_local(&[edit(0, 4, "snow"), edit(6, 4, "snow"), edit(12, 4, "snow")], "rain", "snow", 3, 2000);
        assert_eq!(doc.text(), "snow, snow, snow");
        let group = doc.replace_group(changes[0].id).unwrap();
        doc.revert_changes_local(&group, true, 3000);
        assert_eq!(doc.text(), "rain, rain, rain");
    }
}
