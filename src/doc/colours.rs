//! "Colour text by Session": which of the text shown now the newest Sessions wrote. Read-only,
//! and only worked out when asked (the switch is off by default).
//!
//! A Session's text is the characters its Changes wrote that are still shown, wherever they
//! are now (a Move doesn't make text the moving Session's). The open burst's draft belongs to
//! the Session it will be recorded in.

use super::{BurstKind, Doc, NOT_SHOWN, SESSION_GAP_MS};
use crate::ids::{ChangeId, DeviceId, Hlc};
use std::collections::HashMap;

/// The text one of the newest Sessions wrote, as the Author sees it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionText {
    pub session: ChangeId,
    pub device: DeviceId,
    /// When its last Change to the content was made (or its draft last typed in).
    pub end: Hlc,
    /// Where its characters are in `text()` (open burst included): character ranges, in order.
    pub ranges: Vec<(usize, usize)>,
}

impl Doc {
    /// The text the newest `count` Sessions wrote that's still shown, newest first. Sessions
    /// with no text shown (only deletions, Moves or Preamble edits, or all of it gone) don't
    /// count towards `count`.
    pub fn newest_sessions_text(&self, count: usize) -> Vec<SessionText> {
        if count == 0 {
            return Vec::new();
        }
        let shown = self.shown_positions();
        // Each shown node's Session, looked up once per run of nodes from the same Change.
        let mut last: Option<(ChangeId, ChangeId)> = None;
        let mut session_of = |change: ChangeId| -> ChangeId {
            match last {
                Some((c, s)) if c == change => s,
                _ => {
                    let s = self.made.get(&change).map_or(change, |m| m.session());
                    last = Some((change, s));
                    s
                }
            }
        };

        // The Sessions with text shown, and when each ended.
        let mut ends: HashMap<ChangeId, Hlc> = HashMap::new();
        for &node in &self.visible {
            if shown[node as usize] == NOT_SHOWN {
                continue;
            }
            let session = session_of(self.tree.change_at(node));
            ends.entry(session).or_insert_with(|| self.session_end(session));
        }
        let draft = self.burst.as_ref()
            .filter(|b| b.kind == BurstKind::Typing && !b.draft.is_empty())
            .map(|b| {
                // The same rule `new_change` uses when the burst is recorded.
                let session = match self.session {
                    Some((first, last)) if b.last_ms.saturating_sub(last) < SESSION_GAP_MS => first,
                    _ => b.id,
                };
                (session, Hlc(b.last_ms), b.at, b.draft.chars().count())
            });
        if let Some((session, end, _, _)) = draft {
            let e = ends.entry(session).or_insert(end);
            *e = (*e).max(end);
        }

        let mut newest: Vec<(ChangeId, Hlc)> = ends.into_iter().collect();
        newest.sort_by(|a, b| (b.1, b.0.device).cmp(&(a.1, a.0.device)));
        newest.truncate(count);

        // Where the chosen Sessions' characters are.
        let mut at: HashMap<ChangeId, Vec<usize>> = newest.iter().map(|&(s, _)| (s, Vec::new())).collect();
        for &node in &self.visible {
            let p = shown[node as usize];
            if p == NOT_SHOWN {
                continue;
            }
            if let Some(list) = at.get_mut(&session_of(self.tree.change_at(node))) {
                list.push(p as usize);
            }
        }
        if let Some((session, _, start, len)) = draft {
            if let Some(list) = at.get_mut(&session) {
                list.extend(start..start + len);
                list.sort_unstable();
            }
        }

        newest.into_iter().map(|(session, end)| {
            let mut ranges: Vec<(usize, usize)> = Vec::new();
            for p in at.remove(&session).unwrap_or_default() {
                match ranges.last_mut() {
                    Some((_, e)) if *e == p => *e = p + 1,
                    _ => ranges.push((p, p + 1)),
                }
            }
            SessionText { session, device: session.device, end, ranges }
        }).collect()
    }

    /// When a Session's last Change to the content was made.
    fn session_end(&self, session: ChangeId) -> Hlc {
        self.sessions.get(&session).into_iter().flatten()
            .filter(|id| self.content_set.contains(id))
            .filter_map(|id| self.made.get(id).map(|m| m.hlc_key(*id).0))
            .max()
            .unwrap_or(Hlc(0))
    }
}

#[cfg(test)]
mod test {
    use crate::doc::{Doc, SESSION_GAP_MS};
    use crate::ids::DeviceId;

    /// The text each Session wrote, newest first.
    fn tinted(doc: &Doc, count: usize) -> Vec<String> {
        let text: Vec<char> = doc.text().chars().collect();
        doc.newest_sessions_text(count).iter()
            .map(|s| s.ranges.iter().map(|&(a, b)| text[a..b].iter().collect::<String>()).collect::<Vec<_>>().join("|"))
            .collect()
    }

    #[test]
    fn each_session_gets_the_text_it_wrote_newest_first() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "The rain.", 1_000);
        let later = 1_000 + SESSION_GAP_MS;
        doc.insert_local(8, " had eased", later);          // a new Session, inside the first's text
        doc.insert_local(19, " Gulls.", later + 2 * SESSION_GAP_MS);
        assert_eq!(doc.text(), "The rain had eased. Gulls.");
        assert_eq!(tinted(&doc, 5), [" Gulls.", " had eased", "The rain|."]);
        assert_eq!(tinted(&doc, 2), [" Gulls.", " had eased"], "only the newest");
        assert!(doc.newest_sessions_text(0).is_empty());
    }

    #[test]
    fn a_session_whose_text_is_all_gone_does_not_count() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abc", 1_000);
        doc.insert_local(3, "XYZ", 1_000 + SESSION_GAP_MS);
        doc.delete_local(3, 3, 1_000 + 2 * SESSION_GAP_MS); // a third Session: only a deletion
        assert_eq!(tinted(&doc, 1), ["abc"]);
    }

    #[test]
    fn the_draft_being_typed_belongs_to_its_session() {
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abc", 1_000);
        doc.type_text(3, "de", 2_000); // a burst, still open: the same Session
        assert_eq!(doc.text(), "abcde");
        assert_eq!(tinted(&doc, 5), ["abcde"]);
        doc.close_burst(3_000);
        doc.type_text(0, "X", 3_000 + SESSION_GAP_MS); // a new Session, not recorded yet
        let shown = doc.newest_sessions_text(5);
        assert_eq!(tinted(&doc, 5), ["X", "abcde"]);
        assert_eq!(shown[0].end.0, 3_000 + SESSION_GAP_MS);
    }
}
