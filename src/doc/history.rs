//! The History view's reading of a Work: its Sessions, cut where a Milestone falls inside one,
//! with what each part did. Read-only: nothing here records a Change.
//!
//! A Session lists only its Changes to the Work's content (text, Preamble, and Reverts of
//! those), the same set a Milestone captures. Comments, Milestones, dismissed notices and Device
//! names aren't listed, and a Session with nothing to list is left out.

use super::{holds, positions, Doc};
use crate::change::{is_library_copy, session_record, Change, ChangeKind, CharRange};
use crate::ids::{ChangeId, CharId, DeviceId, Hlc};

/// What one Change did, kept so the History view can say it without the Change itself.
#[derive(Clone, Debug)]
pub(super) struct Made {
    hlc: Hlc,
    session: ChangeId,
    did: Did,
}

#[derive(Clone, Debug)]
pub(super) enum Did {
    /// Wrote its own characters (its nodes in the tree) and deleted these.
    Text { deleted: Vec<CharRange> },
    Revert(Vec<ChangeId>),
    Move,
    Field { record: String, field: String },
    /// A Comment or a Reply.
    Comment,
    Milestone,
    /// A dismissed notice: nothing the History view lists.
    Other,
}

impl Made {
    pub(super) fn of(change: &Change) -> Made {
        let did = match &change.kind {
            ChangeKind::Insert(_) => Did::Text { deleted: Vec::new() },
            ChangeKind::Delete(ranges) | ChangeKind::Replace { deleted: ranges, .. } => Did::Text { deleted: ranges.clone() },
            ChangeKind::Revert { targets } => Did::Revert(targets.clone()),
            ChangeKind::Move { .. } => Did::Move,
            ChangeKind::SetField { record, field, .. } => Did::Field { record: record.clone(), field: field.clone() },
            ChangeKind::Comment { .. } | ChangeKind::Reply { .. } => Did::Comment,
            ChangeKind::Milestone { .. } => Did::Milestone,
            ChangeKind::Dismiss { .. } => Did::Other,
        };
        Made { hlc: change.hlc, session: change.session, did }
    }

    pub(super) fn did(&self) -> &Did {
        &self.did
    }

    /// The Session it was made in.
    pub(super) fn session(&self) -> ChangeId {
        self.session
    }

    /// Its place in the fixed Change order.
    pub(super) fn hlc_key(&self, id: ChangeId) -> (Hlc, DeviceId) {
        (self.hlc, id.device)
    }
}

/// What some Changes did, followed through Reverts: each entry names the Change in the set
/// (`by`) it comes from. A Revert's entries are its targets' with the direction flipped, so a
/// Revert of a Revert comes out right by itself. Fixed once the Changes exist.
#[derive(Default)]
pub(super) struct Effect {
    /// Characters made to appear, and to disappear: (tree node, by).
    pub(super) shown: Vec<(u32, ChangeId)>,
    pub(super) hidden: Vec<(u32, ChangeId)>,
    /// (the Move, true = made / false = taken back, by).
    pub(super) moves: Vec<(ChangeId, bool, ChangeId)>,
    /// (the field Change, true = set / false = taken back, by).
    pub(super) fields: Vec<(ChangeId, bool, ChangeId)>,
}

/// One row of the History view: a Session, or the part of one on one side of a Milestone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPart {
    /// The Session it belongs to (the ID of the Session's first Change).
    pub session: ChangeId,
    /// Its Changes to the Work's content, in the fixed Change order.
    pub changes: Vec<ChangeId>,
    pub device: DeviceId,
    /// When its first and last listed Changes were made.
    pub start: Hlc,
    pub end: Hlc,
    /// A later part of a Session a Milestone cut through: shown as "(continued)".
    pub continued: bool,
    /// Where the Session came from, if not typed in the app: "External edit".
    pub source: Option<String>,
    /// Words it added and removed, as it left them: later Changes never alter these. A word
    /// counts once if the part touched any of its letters.
    pub added_words: usize,
    pub removed_words: usize,
    pub reverted: Reverted,
    /// Milestones named after it that don't hold it: it was made on a Device the naming
    /// Device hadn't synced with yet. In the fixed Change order.
    pub not_in: Vec<ChangeId>,
}

/// What reverting a Session part leaves alone, for the preview's "Not reverted: …".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeftOut {
    /// The Work's copies of library files the app recorded (their records, e.g. "style/Book text").
    pub library: Vec<String>,
    /// Changes already reverted.
    pub already_reverted: usize,
    /// Comments and Replies made in it (deleting them is the Comments tab's job).
    pub comments: usize,
    /// Milestones named in it, still there.
    pub milestones: Vec<ChangeId>,
}

/// When some Changes were made and the words they added and removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub start: Hlc,
    pub end: Hlc,
    pub added_words: usize,
    pub removed_words: usize,
}

/// How much of a Session part is still in effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reverted {
    No,
    Partly,
    All,
}

/// How far apart (in the tree's fixed order) two touched letters may be and still be one word.
/// Longer gaps aren't checked: no word is that long, and it keeps counting cheap.
const LONGEST_GAP: u32 = 64;

/// Letters, digits and apostrophes make words; everything else (spaces, punctuation, Typst
/// markup) separates them.
pub(super) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '’'
}

impl Doc {
    /// Every Session with something to list, cut where a Milestone falls inside it, newest
    /// first (by when each part's last Change was made).
    pub fn sessions(&self) -> Vec<SessionPart> {
        let order = self.tree.order();
        let pos = positions(&order, self.tree.len());
        let milestones = self.milestones();
        let mut parts = Vec::new();
        for (&session, ids) in &self.sessions {
            let mut listed: Vec<ChangeId> = ids.iter().copied().filter(|id| self.content_set.contains(id)).collect();
            if listed.is_empty() {
                continue;
            }
            listed.sort_by_key(|id| (self.made[id].hlc, id.counter));
            // A Milestone cuts the Session between two Changes when it holds one and not the
            // other. A Device's Changes count up, so each Milestone cuts at most once.
            let mut start = 0;
            for i in 1..=listed.len() {
                let cut = i == listed.len()
                    || milestones.iter().any(|m| holds(&m.seen, listed[i - 1]) != holds(&m.seen, listed[i]));
                if cut {
                    parts.push(self.part(session, start > 0, &listed[start..i], &milestones, &order, &pos));
                    start = i;
                }
            }
        }
        parts.sort_by(|a, b| (b.end, b.device).cmp(&(a.end, a.device)));
        parts
    }

    fn part(&self, session: ChangeId, continued: bool, changes: &[ChangeId], milestones: &[super::Milestone], order: &[u32], pos: &[u32]) -> SessionPart {
        let device = session.device;
        let start = self.made[&changes[0]].hlc;
        let end = self.made[changes.last().expect("a part has Changes")].hlc;
        let mut effect = Effect::default();
        for &id in changes {
            self.effect(id, id, true, &mut effect);
        }
        let added = effect.shown.into_iter().map(|(n, _)| n).collect();
        let removed = effect.hidden.into_iter().map(|(n, _)| n).collect();
        let span = ((start, device), (end, device));
        let in_effect = changes.iter().filter(|&&id| self.is_in_effect(id)).count();
        SessionPart {
            session,
            changes: changes.to_vec(),
            device,
            start,
            end,
            continued,
            source: self.field(&session_record(session), "source").map(str::to_string),
            added_words: self.count_words(added, span, order, pos),
            removed_words: self.count_words(removed, span, order, pos),
            reverted: match in_effect {
                0 => Reverted::All,
                n if n == changes.len() => Reverted::No,
                _ => Reverted::Partly,
            },
            not_in: milestones.iter()
                .filter(|m| (m.hlc, m.id.device) > (end, device) && !holds(&m.seen, changes[0]))
                .map(|m| m.id)
                .collect(),
        }
    }

    /// Adds what Change `id` did to `out`, as part of `by` (`forward` false: taken back by a Revert).
    pub(super) fn effect(&self, id: ChangeId, by: ChangeId, forward: bool, out: &mut Effect) {
        match self.made.get(&id).map(|m| &m.did) {
            Some(Did::Text { deleted }) => {
                let written = self.tree.nodes_of(id).filter(|&n| !self.tree.is_anchor_at(n)).map(|n| (n, by));
                let deleted = deleted.iter().flat_map(|range| {
                    (range.first.offset..range.first.offset + range.len)
                        .filter_map(move |offset| self.tree.index_of(CharId { change: range.first.change, offset }))
                        .map(move |n| (n, by))
                });
                if forward {
                    out.shown.extend(written);
                    out.hidden.extend(deleted);
                } else {
                    out.hidden.extend(written);
                    out.shown.extend(deleted);
                }
            }
            Some(Did::Revert(targets)) => {
                for &target in targets {
                    self.effect(target, by, !forward, out);
                }
            }
            Some(Did::Move) => out.moves.push((id, forward, by)),
            Some(Did::Field { .. }) => out.fields.push((id, forward, by)),
            Some(Did::Comment | Did::Milestone | Did::Other) | None => {}
        }
    }

    /// How many words the `nodes` touch. Two touched letters are in one word when nothing but
    /// letters lies between them in the tree's fixed order, as the text was during `span`
    /// (first and last Change, by the fixed order): text written after it is ignored, and so is
    /// text deleted before it. So the count is what the Changes did when they were made.
    fn count_words(&self, nodes: Vec<u32>, span: ((Hlc, DeviceId), (Hlc, DeviceId)), order: &[u32], pos: &[u32]) -> usize {
        let mut at: Vec<u32> = nodes.into_iter()
            .filter(|&n| self.tree.char_at(n).is_some_and(is_word_char))
            .map(|n| pos[n as usize])
            .collect();
        at.sort_unstable();
        at.dedup();
        let key = |id: ChangeId| self.made.get(&id).map(|m| (m.hlc, id.device));
        let (first, last) = span;
        // Was this node part of the text while the Changes were made?
        let there = |n: u32| {
            key(self.tree.change_at(n)).is_some_and(|k| k <= last)
                && !self.tree.deleters_at(n).iter().any(|&d| key(d).is_some_and(|k| k < first))
        };
        let joined = |from: u32, to: u32| {
            to - from <= LONGEST_GAP
                && order[from as usize + 1..to as usize].iter()
                    .all(|&n| self.tree.is_anchor_at(n) || !there(n) || self.tree.char_at(n).is_some_and(is_word_char))
        };
        (0..at.len()).filter(|&i| i == 0 || !joined(at[i - 1], at[i])).count()
    }

    /// What reverting these Changes (a Session part, or some of it) would take: those still in
    /// effect that change the Work's content, except the Work's copies of Style library files,
    /// which the app records on its own to mirror the library.
    pub fn revertable(&self, changes: &[ChangeId]) -> Vec<ChangeId> {
        changes.iter().copied()
            .filter(|&id| self.content_set.contains(&id) && self.is_in_effect(id) && !self.only_library(id))
            .collect()
    }

    /// Every Milestone ever named, deleted ones too (with true), in the fixed Change order, with
    /// their current names: the History view shows a deleted one struck through, to restore.
    pub fn milestones_and_deleted(&self) -> Vec<(super::Milestone, bool)> {
        let mut list: Vec<(super::Milestone, bool)> = self.milestones.iter()
            .map(|(id, hlc, name, seen)| (super::Milestone {
                id: *id,
                hlc: *hlc,
                name: self.field(&crate::change::milestone_record(*id), "name").unwrap_or(name).to_string(),
                seen: seen.clone(),
            }, !self.is_in_effect(*id)))
            .collect();
        list.sort_by_key(|(m, _)| (m.hlc, m.id.device));
        list
    }

    /// Where these characters are in the text the Author sees (open burst included): from the
    /// first to just after the last, or (no last) the empty range just before the first. None
    /// if the first isn't shown any more.
    pub fn shown_range(&self, first: CharId, last: Option<CharId>) -> Option<(usize, usize)> {
        let shown = self.shown_positions();
        let at = |id: CharId| self.tree.index_of(id).map(|n| shown[n as usize]).filter(|&p| p != super::NOT_SHOWN);
        let start = at(first)? as usize;
        match last.and_then(at) {
            Some(end) => {
                let end = end as usize;
                Some((start.min(end), start.max(end) + 1))
            }
            None => Some((start, start)),
        }
    }

    /// The Changes inside these that reverting them would also revert by default: those that
    /// added text strictly inside text they wrote (and so on), still in effect, not already
    /// among them. In the fixed Change order.
    pub fn dependents(&self, changes: &[ChangeId]) -> Vec<ChangeId> {
        let given: rustc_hash::FxHashSet<ChangeId> = changes.iter().copied().collect();
        let mut found: Vec<ChangeId> = self.tree.inside_all(changes).into_iter()
            .filter(|id| !given.contains(id) && self.is_in_effect(*id) && self.content_set.contains(id))
            .collect();
        found.sort_by_key(|id| self.made[id].hlc_key(*id));
        found.dedup();
        found
    }

    /// Reverts some Changes (a Session, a snippet, a Preamble line) as one Revert, with their
    /// dependents if asked. Only those `revertable` gives are reverted. None if that's nothing.
    pub fn revert_changes_local(&mut self, changes: &[ChangeId], with_dependents: bool, now_ms: u64) -> Option<Change> {
        let mut targets = self.revertable(changes);
        if with_dependents {
            targets.extend(self.dependents(&targets));
        }
        if targets.is_empty() {
            return None;
        }
        Some(self.commit_local(ChangeKind::Revert { targets }, now_ms))
    }

    /// Brings Changes back by reverting every Revert of them still in effect, as one Revert. A
    /// Revert is taken back whole, so whatever else it reverted comes back too. None if none of
    /// them is reverted.
    pub fn restore_changes_local(&mut self, changes: &[ChangeId], now_ms: u64) -> Option<Change> {
        let reverts = self.reverts_of(changes);
        if reverts.is_empty() {
            return None;
        }
        Some(self.commit_local(ChangeKind::Revert { targets: reverts }, now_ms))
    }

    /// What restoring these Changes would bring back: everything the Reverts it takes back had
    /// taken, for "Restore (brings back 5 places)".
    pub fn restored_by(&self, changes: &[ChangeId]) -> Vec<ChangeId> {
        let mut back: Vec<ChangeId> = changes.iter()
            .flat_map(|id| self.reverted_by.get(id).cloned().unwrap_or_default())
            .filter(|&r| self.is_in_effect(r))
            .flat_map(|r| match self.made.get(&r).map(|m| &m.did) {
                Some(Did::Revert(targets)) => targets.clone(),
                _ => Vec::new(),
            })
            .collect();
        back.sort_by_key(|id| self.made[id].hlc_key(*id));
        back.dedup();
        back
    }

    /// Would this Change be in effect if a Revert of `targets` were made now?
    pub(super) fn in_effect_if_reverted(&self, id: ChangeId, targets: &rustc_hash::FxHashSet<ChangeId>) -> bool {
        if targets.contains(&id) {
            return false;
        }
        match self.reverted_by.get(&id) {
            Some(reverters) => reverters.iter().all(|&r| !self.in_effect_if_reverted(r, targets)),
            None => true,
        }
    }

    /// Would this character be visible if a Revert of `targets` were made now?
    pub(super) fn visible_if_reverted(&self, node: u32, targets: &rustc_hash::FxHashSet<ChangeId>) -> bool {
        !self.tree.is_anchor_at(node)
            && self.in_effect_if_reverted(self.tree.change_at(node), targets)
            && !self.tree.deleters_at(node).iter().any(|&d| self.in_effect_if_reverted(d, targets))
    }

    /// Do any of these Changes set (or take back) a Preamble setting? Returning to a Milestone
    /// then says the Preamble goes back too.
    pub fn touches_preamble(&self, changes: &[ChangeId]) -> bool {
        changes.iter().any(|id| match self.made.get(id).map(|m| &m.did) {
            Some(Did::Field { record, .. }) => record == crate::change::PREAMBLE_RECORD,
            Some(Did::Revert(targets)) => self.touches_preamble(targets),
            _ => false,
        })
    }

    /// When a Change was made, if the Work has it.
    pub fn made_at(&self, id: ChangeId) -> Option<Hlc> {
        self.made.get(&id).map(|m| m.hlc)
    }

    /// The Reverts in effect that take back any of these Changes, in the fixed Change order:
    /// what restoring them reverts.
    pub fn reverts_of(&self, changes: &[ChangeId]) -> Vec<ChangeId> {
        let mut reverts: Vec<ChangeId> = changes.iter()
            .flat_map(|id| self.reverted_by.get(id).cloned().unwrap_or_default())
            .filter(|&r| self.is_in_effect(r))
            .collect();
        reverts.sort_by_key(|id| self.made[id].hlc_key(*id));
        reverts.dedup();
        reverts
    }

    /// The latest of `reverts_of`: "Reverted by Laptop, 9 Nov".
    pub fn latest_revert_of(&self, changes: &[ChangeId]) -> Option<ChangeId> {
        self.reverts_of(changes).last().copied()
    }

    /// Changes grouped by their Session, each group in the fixed Change order, groups by their
    /// first Change: the dependents in a Revert preview, and what returning to a Milestone could keep.
    pub fn by_session(&self, changes: &[ChangeId]) -> Vec<(ChangeId, Vec<ChangeId>)> {
        let mut sorted: Vec<ChangeId> = changes.iter().copied().filter(|id| self.made.contains_key(id)).collect();
        sorted.sort_by_key(|id| self.made[id].hlc_key(*id));
        sorted.dedup();
        let mut groups: Vec<(ChangeId, Vec<ChangeId>)> = Vec::new();
        for id in sorted {
            let session = self.made[&id].session;
            match groups.iter_mut().find(|(s, _)| *s == session) {
                Some((_, group)) => group.push(id),
                None => groups.push((session, vec![id])),
            }
        }
        groups
    }

    /// When some Changes were made, and the words they added and removed (counted as for a
    /// Session: as they were made).
    pub fn summary(&self, changes: &[ChangeId]) -> Option<Summary> {
        let mut sorted: Vec<ChangeId> = changes.iter().copied().filter(|id| self.made.contains_key(id)).collect();
        sorted.sort_by_key(|id| self.made[id].hlc_key(*id));
        let (first, last) = (*sorted.first()?, *sorted.last()?);
        let mut effect = Effect::default();
        for &id in &sorted {
            self.effect(id, id, true, &mut effect);
        }
        let order = self.tree.order();
        let pos = positions(&order, self.tree.len());
        let span = (self.made[&first].hlc_key(first), self.made[&last].hlc_key(last));
        Some(Summary {
            start: self.made[&first].hlc,
            end: self.made[&last].hlc,
            added_words: self.count_words(effect.shown.into_iter().map(|(n, _)| n).collect(), span, &order, &pos),
            removed_words: self.count_words(effect.hidden.into_iter().map(|(n, _)| n).collect(), span, &order, &pos),
        })
    }

    /// What reverting these Changes (a Session part) leaves alone. Comments and Milestones count
    /// if they were made in the same Session, between the part's first and last Change (from
    /// the Session's start, for its first part; to its end, for its last).
    pub fn left_out(&self, changes: &[ChangeId]) -> LeftOut {
        let mut out = LeftOut::default();
        for &id in changes {
            if !self.content_set.contains(&id) {
                continue;
            }
            if !self.is_in_effect(id) {
                out.already_reverted += 1;
            } else if self.only_library(id) {
                if let Some(Did::Field { record, .. }) = self.made.get(&id).map(|m| &m.did) {
                    if !out.library.contains(record) {
                        out.library.push(record.clone());
                    }
                }
            }
        }
        let Some(session) = changes.first().and_then(|id| self.made.get(id)).map(|m| m.session) else { return out };
        let all = self.sessions.get(&session).cloned().unwrap_or_default();
        let content: Vec<u32> = all.iter().filter(|id| self.content_set.contains(id)).map(|id| id.counter).collect();
        let (lo, hi) = (changes.iter().map(|c| c.counter).min().unwrap_or(0), changes.iter().map(|c| c.counter).max().unwrap_or(0));
        let lo = if content.iter().any(|&c| c < lo) { lo } else { 0 };
        let hi = if content.iter().any(|&c| c > hi) { hi } else { u32::MAX };
        for id in all.into_iter().filter(|id| (lo..=hi).contains(&id.counter)) {
            match self.made.get(&id).map(|m| &m.did) {
                Some(Did::Comment) if self.is_in_effect(id) => out.comments += 1,
                Some(Did::Milestone) if self.is_in_effect(id) => out.milestones.push(id),
                _ => {}
            }
        }
        out
    }

    /// How many Comments now attached to text would be Detached comments after returning to a
    /// Milestone (keeping nothing): none of the text they cover is in the Milestone's text.
    pub fn detached_by_return(&self, milestone: ChangeId) -> usize {
        let seen = self.milestone(milestone).clone();
        let (_, nodes) = self.view_among(&|c| holds(&seen, c));
        let there: rustc_hash::FxHashSet<u32> = nodes.into_iter().collect();
        let order = self.tree.order();
        let pos = positions(&order, self.tree.len());
        let attached: Vec<ChangeId> = self.comments().into_iter().filter(|c| !c.is_detached()).map(|c| c.id).collect();
        self.comments.iter()
            .filter(|c| attached.contains(&c.id))
            .filter(|c| !c.pieces.iter().any(|span| {
                let (Some(first), Some(last)) = (self.tree.index_of(span.first), self.tree.index_of(span.last)) else { return false };
                order[pos[first as usize] as usize..=pos[last as usize] as usize].iter().any(|n| there.contains(n))
            }))
            .count()
    }

    /// Does this Change only touch the Work's copies of library files?
    fn only_library(&self, id: ChangeId) -> bool {
        match self.made.get(&id).map(|m| &m.did) {
            Some(Did::Field { record, .. }) => is_library_copy(record),
            Some(Did::Revert(targets)) => targets.iter()
                .filter(|t| self.content_set.contains(t))
                .all(|&t| self.only_library(t)),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::{session_record, AUTHOR};
    use crate::doc::SESSION_GAP_MS;

    const LATER: u64 = SESSION_GAP_MS + 1; // long enough to start a new Session

    fn counts(part: &SessionPart) -> (usize, usize) {
        (part.added_words, part.removed_words)
    }

    #[test]
    fn a_milestone_named_mid_session_cuts_it_in_two() {
        let mut doc = Doc::new(DeviceId(1));
        let a = doc.insert_local(0, "The rain ", 1_000);
        let b = doc.insert_local(9, "had stopped.", 2_000);
        let draft = doc.make_milestone_local("Draft 1", 3_000);
        let c = doc.insert_local(21, " Gulls circled.", 4_000);
        let parts = doc.sessions();
        assert_eq!(parts.len(), 2, "{parts:?}");
        // Newest first: the part after the Milestone, then the part it holds.
        assert_eq!(parts[0].changes, vec![c.id]);
        assert!(parts[0].continued);
        assert_eq!(parts[1].changes, vec![a.id, b.id]);
        assert!(!parts[1].continued);
        assert_eq!(parts[0].session, parts[1].session, "still one Session");
        assert!(parts.iter().all(|p| p.not_in.is_empty()));
        assert!(!parts.iter().any(|p| p.changes.contains(&draft.id)), "Milestones aren't listed");
    }

    #[test]
    fn a_session_made_earlier_on_a_device_that_had_not_synced_is_not_in_the_milestone() {
        let mut laptop = Doc::new(DeviceId(1));
        let mut phone = Doc::new(DeviceId(2));
        let base = laptop.insert_local(0, "The rain had stopped.", 500);
        phone.apply(&base);
        let offline = phone.insert_local(21, " Gulls circled.", 1_000); // not synced yet
        let draft = laptop.make_milestone_local("Draft 2", 2_000);
        laptop.apply(&offline);
        let parts = laptop.sessions();
        let phones = parts.iter().find(|p| p.device == DeviceId(2)).expect("the phone's Session");
        assert_eq!(phones.not_in, vec![draft.id]);
        assert!(phones.end < draft.hlc, "older than the Milestone, yet not part of it");
        let laptops = parts.iter().find(|p| p.device == DeviceId(1)).expect("the laptop's Session");
        assert!(laptops.not_in.is_empty());
    }

    #[test]
    fn counts_are_what_a_session_did_and_later_changes_never_alter_them() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.", 1_000);
        let first = doc.sessions();
        assert_eq!(counts(&first[0]), (4, 0));
        doc.delete_local(0, doc.len(), 1_000 + LATER); // all of it, in a later Session
        doc.insert_local(0, "Snow", 1_000 + 2 * LATER); // and something new, later still
        let parts = doc.sessions();
        assert_eq!(parts.len(), 3);
        assert_eq!(counts(&parts[2]), (4, 0), "the first Session still says what it did");
        assert_eq!(counts(&parts[1]), (0, 4));
        assert_eq!(counts(&parts[0]), (1, 0));
    }

    #[test]
    fn a_changed_word_is_one_added_and_one_removed() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.", 1_000);
        doc.delete_local(13, 7, 1_000 + LATER);
        doc.insert_local(13, "eased", 2_000 + LATER);
        assert_eq!(counts(&doc.sessions()[0]), (1, 1));
        // Fixing a typo inside a word touches one word.
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stoped.", 1_000);
        doc.insert_local(16, "p", 1_000 + LATER);
        assert_eq!(counts(&doc.sessions()[0]), (1, 0));
    }

    #[test]
    fn a_word_typed_in_two_bursts_counts_once_and_deleted_words_count_apart() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "hel", 1_000);
        doc.insert_local(3, "lo there", 2_000);
        assert_eq!(counts(&doc.sessions()[0]), (2, 0));
        doc.delete_local(0, doc.len(), 2_000 + LATER);
        assert_eq!(counts(&doc.sessions()[0]), (0, 2));
    }

    #[test]
    fn a_revert_counts_the_words_it_takes_out_and_restoring_counts_them_back() {
        let mut doc = Doc::new(DeviceId(1));
        let rain = doc.insert_local(0, "The rain had stopped.", 1_000);
        doc.revert_local(rain.id, true, 1_000 + LATER);
        let reverted = doc.sessions();
        assert_eq!(counts(&reverted[0]), (0, 4));
        assert_eq!(reverted[1].reverted, Reverted::All);
        doc.restore_local(rain.id, 1_000 + 2 * LATER).expect("it was reverted");
        let restored = doc.sessions();
        assert_eq!(counts(&restored[0]), (4, 0));
        assert_eq!(restored[2].reverted, Reverted::No);
        assert_eq!(restored[1].reverted, Reverted::All, "the Revert itself is now undone");
    }

    #[test]
    fn partly_reverted_and_the_external_edit_label() {
        let mut doc = Doc::new(DeviceId(1));
        let a = doc.insert_local(0, "The rain ", 1_000);
        doc.insert_local(9, "had stopped.", 2_000);
        doc.revert_local(a.id, false, 2_000 + LATER);
        assert_eq!(doc.sessions()[1].reverted, Reverted::Partly);

        let version = doc.version();
        doc.import_text(Some(&version), "had stopped. Gulls circled.", 3_000 + 2 * LATER);
        let newest = &doc.sessions()[0];
        assert_eq!(newest.source.as_deref(), Some("External edit"));
        assert_eq!(doc.field(&session_record(newest.session), "source"), Some("External edit"));
        assert_eq!(counts(newest), (2, 0));
    }

    #[test]
    fn sessions_with_only_comments_are_left_out() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain had stopped.", 1_000);
        doc.comment_local(0, 8, AUTHOR, "too abrupt?", 1_000 + LATER);
        doc.set_device_name_local("Laptop", 2_000 + LATER);
        assert_eq!(doc.sessions().len(), 1);
    }

    #[test]
    fn reverting_a_session_is_one_revert_and_restoring_brings_back_all_it_took() {
        let mut doc = Doc::new(DeviceId(1));
        let rain = doc.insert_local(0, "The rain had stopped.", 1_000);
        let del = doc.delete_local(13, 7, 1_000 + LATER);
        let eased = doc.insert_local(13, "eased", 2_000 + LATER);
        assert_eq!(doc.dependents(&[rain.id]), vec![eased.id], "the deletion isn't a dependent");

        let revert = doc.revert_changes_local(&[rain.id], true, 3_000 + LATER).expect("something to revert");
        assert_eq!(doc.text(), "");
        assert_eq!(doc.restored_by(&[eased.id]), vec![rain.id, eased.id], "restoring one brings back both");
        doc.restore_changes_local(&[eased.id], 4_000 + LATER).expect("it was reverted");
        assert_eq!(doc.text(), "The rain had eased.");
        assert!(!doc.is_in_effect(revert.id));
        assert!(doc.restore_changes_local(&[rain.id], 5_000 + LATER).is_none(), "nothing left to restore");

        // Without dependents, "eased" stays behind. Reverting what's already reverted does nothing.
        doc.revert_changes_local(&[rain.id], false, 6_000 + LATER).unwrap();
        assert_eq!(doc.text(), "eased");
        assert!(doc.revert_changes_local(&[rain.id, del.id], false, 7_000 + LATER).is_some(), "the deletion still can be");
        assert!(doc.revert_changes_local(&[rain.id], false, 8_000 + LATER).is_none());
    }

    #[test]
    fn what_reverting_a_session_leaves_out_and_what_a_return_would_detach() {
        let mut doc = Doc::new(DeviceId(1));
        let text = doc.insert_local(0, "The rain had stopped.", 1_000);
        let draft = doc.make_milestone_local("Draft 1", 1_500);
        let gulls = doc.insert_local(21, " Gulls circled.", 2_000);
        doc.set_field_local("style/Book text", "content", "#set text(size: 11pt)", 3_000);
        doc.comment_local(22, 5, AUTHOR, "new?", 4_000); // on "Gulls", written after Draft 1
        doc.comment_local(4, 4, AUTHOR, "rain?", 5_000); // on "rain", in Draft 1
        let draft2 = doc.make_milestone_local("Draft 2", 6_000); // at the end of the sitting
        let parts = doc.sessions();
        let later = parts.iter().find(|p| p.changes.contains(&gulls.id)).unwrap();
        let left = doc.left_out(&later.changes);
        assert_eq!(left.library, vec!["style/Book text".to_string()]);
        assert_eq!(left.comments, 2);
        assert_eq!(left.milestones, vec![draft2.id], "named in this part; Draft 1 is the divider before it");
        let earlier = parts.iter().find(|p| p.changes.contains(&text.id)).unwrap();
        assert!(doc.left_out(&earlier.changes).milestones.is_empty());
        assert_eq!(doc.detached_by_return(draft.id), 1, "only the Comment on \"Gulls\"");

        let summary = doc.summary(&later.changes).unwrap();
        assert_eq!((summary.added_words, summary.removed_words), (2, 0));
        assert_eq!(doc.by_session(&[gulls.id, text.id]).len(), 1, "one Session");
    }

    #[test]
    fn reverting_a_session_leaves_library_copies_and_comments_alone() {
        let mut doc = Doc::new(DeviceId(1));
        let text = doc.insert_local(0, "The rain had stopped.", 1_000);
        let style = doc.set_field_local("style/Book text", "content", "#set text(size: 11pt)", 2_000);
        let title = doc.set_field_local("preamble", "title", "Rain", 3_000);
        let comment = doc.comment_local(0, 8, AUTHOR, "too abrupt?", 4_000);
        let undone = doc.insert_local(21, " Gulls.", 5_000);
        doc.revert_local(undone.id, false, 6_000);
        let part = doc.sessions().into_iter().find(|p| p.changes.contains(&text.id)).unwrap();
        assert!(part.changes.contains(&style.id), "a recorded Style is listed");
        assert!(!part.changes.contains(&comment.id), "a Comment isn't");
        // The Revert of `undone` is in effect, so reverting the Session would restore `undone`.
        let targets = doc.revertable(&part.changes);
        assert!(targets.contains(&text.id) && targets.contains(&title.id));
        assert!(!targets.contains(&style.id), "{targets:?}");
        assert!(!targets.contains(&undone.id), "already reverted");
        assert_eq!(targets.len(), 3, "the text, the title and the Revert: {targets:?}");
    }
}
