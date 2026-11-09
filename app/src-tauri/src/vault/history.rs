//! The History view's answers for an open Work: its Sessions and Milestones, what a Session
//! did (its snippets), previews of reverting, restoring and returning to a Milestone, and
//! renaming, deleting and restoring Milestones, and the Merge notices (and dismissing them).
//!
//! None of this holds positions in the editor's text, so it's asked for beside the editor's
//! queue, not through it. A snippet says what to select by its characters' identities; the
//! editor asks for their place through the queue (`Op::Find`) when it's clicked. Reverting,
//! restoring and returning themselves are ops in the queue (`Op::Revert` and the rest).

use super::*;
use crate::work::{history_notices, notice_key};
use wrtier::change::NoticeKey;
use wrtier::doc::{Comparison, FieldLine, MergeNotice, Reverted, SegmentKind, Snippet, Snippets, Version};

/// The History tab's list: the newest Sessions, and every Milestone (deleted ones too).
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryView {
    /// Newest first.
    pub sessions: Vec<SessionRow>,
    /// Oldest first.
    pub milestones: Vec<MilestoneRow>,
    /// There are older Sessions than these ("Show older").
    pub more: bool,
    /// The Merge notices not dismissed, pinned at the top (none in "History of this text").
    pub notices: Vec<NoticeView>,
}

/// A Merge notice, for the History tab to say in a plain sentence.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NoticeView {
    /// What Dismiss sends back (`work::notice_key`).
    pub key: String,
    /// "overlap" (two Moves took the same text), "loop" (a Move was skipped), "externalEdit" (a
    /// Text file edited from an unknown version) or "preamble" (its raw Typst edited on two Devices).
    pub kind: &'static str,
    /// Who made the Change it's about, and when: the later Move, the skipped Move, the External
    /// edit, or the raw Typst that's used.
    pub device: String,
    pub time_ms: u64,
    /// The other side, for "overlap" (the earlier Move) and "preamble" (the edit that lost).
    pub other_device: Option<String>,
    /// The text it's about, shortened ("" if none of it is in the text now).
    pub quote: String,
    /// Where that text is, in reading order: "Show me" steps through them.
    pub places: Vec<FindView>,
}

/// A Session, or the part of one on one side of a Milestone.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    /// Names the row while it's shown (its first Change): which rows are open survives a refresh.
    pub key: String,
    /// Its Changes to the content: for its snippets, and for reverting or restoring it.
    pub changes: Vec<String>,
    /// The Device's name as the app shows it.
    pub device: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub continued: bool,
    /// "External edit", if it was one.
    pub source: Option<String>,
    pub added_words: usize,
    pub removed_words: usize,
    /// "no", "partly" or "all".
    pub reverted: &'static str,
    /// Milestones named after it that don't hold it.
    pub not_in: Vec<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneRow {
    pub id: String,
    pub name: String,
    pub time_ms: u64,
    pub deleted: bool,
    /// The Device that named it.
    pub device: String,
}

/// What some Changes did, as the History tab shows it (`wrtier::doc::Snippets`).
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SnippetsView {
    pub snippets: Vec<SnippetView>,
    pub fields: Vec<FieldView>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SnippetView {
    pub segments: Vec<SegmentView>,
    pub changes: Vec<String>,
    /// What to select when it's clicked: the first and last characters (or, last missing, the
    /// place just before the first). Asked for through the editor's queue (`Op::Find`).
    pub find: Option<FindView>,
    pub moves: Vec<MoveView>,
    /// Some of its Changes are still in effect: it can be reverted.
    pub in_effect: bool,
    /// The latest Revert that took some of it back: it can be restored.
    pub reverted_by: Option<RevertedByView>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FindView {
    pub first: String,
    pub last: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SegmentView {
    /// "plain", "added", "removed", "moved", "cut" or "elided".
    pub kind: &'static str,
    pub text: String,
    pub faded: bool,
    /// For "elided": how many words were left out.
    pub words: usize,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MoveView {
    pub back: bool,
    pub chapter: Option<String>,
    pub after: String,
    pub earlier: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FieldView {
    pub by: String,
    pub change: String,
    pub record: String,
    pub field: String,
    pub value: Option<String>,
    pub before: Option<String>,
    pub taken_back: bool,
    pub library: bool,
    /// The Change it comes from is still in effect: it can be reverted (otherwise restored).
    pub in_effect: bool,
}

/// "Reverted by Laptop, 9 Nov".
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RevertedByView {
    pub device: String,
    pub time_ms: u64,
}

/// Some Changes in one Session: the dependents in a Revert preview, and what a return to a
/// Milestone could keep.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub device: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub added_words: usize,
    pub removed_words: usize,
    pub changes: Vec<String>,
    pub snippets: SnippetsView,
}

/// What reverting some Changes would do.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RevertPreview {
    /// What they'd take out and bring back, without their dependents.
    pub goes: SnippetsView,
    /// Later Changes inside their text, by Session: reverted with them unless the Author says not.
    pub dependents: Vec<GroupView>,
    pub left_out: LeftOutView,
    /// Nothing to revert (all of it already is, or none of it changes the content).
    pub nothing: bool,
}

/// "Not reverted: 2 Comments, Milestone ‘Draft 2’, recorded copies of Book text".
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LeftOutView {
    /// Names of the library files whose recorded copies stay.
    pub library: Vec<String>,
    pub already_reverted: usize,
    pub comments: usize,
    /// Names of the Milestones named in it.
    pub milestones: Vec<String>,
}

/// What restoring some Changes would bring back: everything the Reverts it takes back took.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreview {
    pub back: SnippetsView,
    /// How many places (snippets and Preamble lines) come back.
    pub places: usize,
    pub nothing: bool,
}

/// A comparison, as the History tab shows it.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonView {
    pub snippets: Vec<SnippetView>,
    pub added_words: usize,
    pub removed_words: usize,
    pub moves: usize,
}

/// What returning to a Milestone would do.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReturnPreview {
    /// From now to the Milestone: what comes back is added, what goes is removed.
    pub since: ComparisonView,
    /// Made before the Milestone on another Device that hadn't synced: each can be kept.
    pub earlier: Vec<GroupView>,
    /// Comments that would become Detached comments.
    pub detached: usize,
    /// The Preamble changed since then, and goes back too.
    pub preamble: bool,
    /// The content is already as it was then.
    pub nothing: bool,
}

/// A Device's name as the app shows it, given the Work it's named in.
type Namer = dyn Fn(&Doc, DeviceId) -> String;

/// Snippets as the History tab shows them.
pub fn snippets_view(shown: Snippets, doc: &Doc, name: &Namer) -> SnippetsView {
    SnippetsView {
        snippets: shown.snippets.into_iter().map(|s| snippet_view(s, doc, name)).collect(),
        fields: shown.fields.into_iter().map(|f: FieldLine| FieldView {
            by: f.by.to_string(),
            change: f.change.to_string(),
            record: f.record,
            field: f.field,
            value: f.value,
            before: f.before,
            taken_back: f.taken_back,
            library: f.library,
            in_effect: f.in_effect,
        }).collect(),
    }
}

/// A comparison as the History tab shows it.
pub fn comparison_view(shown: Comparison, doc: &Doc, name: &Namer) -> ComparisonView {
    ComparisonView {
        snippets: shown.snippets.into_iter().map(|s| snippet_view(s, doc, name)).collect(),
        added_words: shown.added_words,
        removed_words: shown.removed_words,
        moves: shown.moves,
    }
}

fn ids(list: &[ChangeId]) -> Vec<String> {
    list.iter().map(|id| id.to_string()).collect()
}

fn snippet_view(s: Snippet, doc: &Doc, name: &Namer) -> SnippetView {
    SnippetView {
        segments: s.segments.into_iter().map(|seg| {
            let (kind, words) = match seg.kind {
                SegmentKind::Plain => ("plain", 0),
                SegmentKind::Added => ("added", 0),
                SegmentKind::Removed => ("removed", 0),
                SegmentKind::Moved => ("moved", 0),
                SegmentKind::Cut => ("cut", 0),
                SegmentKind::Elided { words } => ("elided", words),
            };
            SegmentView { kind, text: seg.text, faded: seg.faded, words }
        }).collect(),
        changes: ids(&s.changes),
        find: s.find.map(|(first, last)| FindView { first: first.to_string(), last: last.map(|l| l.to_string()) }),
        moves: s.moves.into_iter().map(|m| MoveView { back: m.back, chapter: m.chapter, after: m.after, earlier: m.earlier }).collect(),
        in_effect: s.in_effect,
        reverted_by: s.reverted_by.map(|r| RevertedByView { device: name(doc, r.device), time_ms: doc.made_at(r).map_or(0, |h| h.0) }),
    }
}

/// Some Changes in one Session, with their snippets (`preview`: what reverting them would do).
fn group_view(doc: &Doc, changes: &[ChangeId], preview: bool, name: &Namer) -> Option<GroupView> {
    let summary = doc.summary(changes)?;
    let shown = if preview { doc.revert_preview(changes) } else { doc.snippets(changes) };
    Some(GroupView {
        device: name(doc, changes[0].device),
        start_ms: summary.start.0,
        end_ms: summary.end.0,
        added_words: summary.added_words,
        removed_words: summary.removed_words,
        changes: ids(changes),
        snippets: snippets_view(shown, doc, name),
    })
}

/// The text a notice is about, on one line and at most about 60 characters.
fn short_quote(text: &str) -> String {
    let quote = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if quote.chars().count() > 60 {
        format!("{}…", quote.chars().take(59).collect::<String>().trim_end())
    } else {
        quote
    }
}

fn notice_view(n: MergeNotice, doc: &Doc, name: &Namer) -> NoticeView {
    let (kind, about, other) = match n.key {
        NoticeKey::Overlap { later, earlier } => ("overlap", later, Some(earlier)),
        NoticeKey::Loop { skipped } => ("loop", skipped, None),
        NoticeKey::UnknownTextFile { session } => ("externalEdit", session, None),
        NoticeKey::ConcurrentField { later, earlier } => ("preamble", later, Some(earlier)),
    };
    NoticeView {
        key: notice_key(n.key),
        kind,
        device: name(doc, about.device),
        time_ms: doc.made_at(about).map_or(0, |h| h.0),
        other_device: other.map(|o| name(doc, o.device)),
        quote: short_quote(&n.text),
        places: n.places.iter().map(|(first, last)| FindView { first: first.to_string(), last: Some(last.to_string()) }).collect(),
    }
}

/// The name of a library file from its record ("style/Book text" → "Book text").
fn library_name(record: &str) -> String {
    record.split_once('/').map_or(record, |(_, name)| name).to_string()
}

/// Change IDs from the interface.
pub fn change_ids(list: &[String]) -> Result<Vec<ChangeId>, String> {
    list.iter().map(|id| id.parse().map_err(|_| format!("Not a Change: {id}"))).collect()
}

impl Vault {
    /// How the app names Devices: by the current name in `vault.db`, else the one the Work
    /// recorded, else the ID. Taken before an open Work is borrowed.
    pub(super) fn device_namer(&self) -> impl Fn(&Doc, DeviceId) -> String + 'static {
        let known: HashMap<DeviceId, String> = self.vault_doc.records("device/").into_iter()
            .filter_map(|record| {
                let device: DeviceId = record.strip_prefix("device/")?.parse().ok()?;
                Some((device, self.vault_doc.device_name(device)?.to_string()))
            })
            .collect();
        move |doc: &Doc, device: DeviceId| known.get(&device).cloned()
            .or_else(|| doc.device_name(device).map(str::to_string))
            .unwrap_or_else(|| device.to_string())
    }

    /// An open Work's newest `limit` Sessions and all its Milestones. `filtered`: only the
    /// Sessions that touched the text "History of this text" is about, each with only the
    /// Changes that touched it.
    pub fn history(&mut self, id: &str, limit: usize, filtered: bool, now_ms: u64) -> Result<HistoryView, String> {
        let name = self.device_namer();
        self.with_work(id, now_ms, |w, _| {
            let doc = &w.doc;
            let mut parts = doc.sessions();
            if filtered {
                let stretch = w.stretch.as_ref().ok_or("There's no text to show the History of: select some first.")?;
                let touched: std::collections::HashSet<ChangeId> = doc.changes_in(stretch).into_iter().collect();
                for part in &mut parts {
                    part.changes.retain(|c| touched.contains(c));
                }
                parts.retain(|p| !p.changes.is_empty());
            }
            let more = parts.len() > limit;
            let sessions = parts.into_iter().take(limit).map(|p| SessionRow {
                key: p.changes[0].to_string(),
                changes: ids(&p.changes),
                device: name(doc, p.device),
                start_ms: p.start.0,
                end_ms: p.end.0,
                continued: p.continued,
                source: p.source,
                added_words: p.added_words,
                removed_words: p.removed_words,
                reverted: match p.reverted {
                    Reverted::No => "no",
                    Reverted::Partly => "partly",
                    Reverted::All => "all",
                },
                not_in: ids(&p.not_in),
            }).collect();
            let milestones = doc.milestones_and_deleted().into_iter().map(|(m, deleted)| MilestoneRow {
                id: m.id.to_string(),
                name: m.name,
                time_ms: m.hlc.0,
                deleted,
                device: name(doc, m.id.device),
            }).collect();
            let notices = if filtered {
                Vec::new()
            } else {
                history_notices(doc).into_iter().map(|n| notice_view(n, doc, &name)).collect()
            };
            Ok(HistoryView { sessions, milestones, more, notices })
        })
    }

    /// Dismisses one of an open Work's Merge notices, on every Device.
    pub fn dismiss_notice(&mut self, id: &str, key: &str, now_ms: u64) -> Result<WorkState, String> {
        self.with_work(id, now_ms, |w, _| w.dismiss_notice(key, now_ms))
    }

    /// What some of an open Work's Changes did (a Session's), as snippets. `filtered`: clipped
    /// to the text "History of this text" is about.
    pub fn snippets(&mut self, id: &str, changes: &[String], filtered: bool, now_ms: u64) -> Result<SnippetsView, String> {
        let changes = change_ids(changes)?;
        let name = self.device_namer();
        self.with_work(id, now_ms, |w, _| {
            let shown = match (filtered, &w.stretch) {
                (true, Some(stretch)) => w.doc.snippets_in(&changes, stretch),
                (true, None) => return Err("There's no text to show the History of: select some first.".into()),
                (false, _) => w.doc.snippets(&changes),
            };
            Ok(snippets_view(shown, &w.doc, &name))
        })
    }

    /// What changed in an open Work from one version to another (None: the Work as it is now).
    pub fn compare_versions(&mut self, id: &str, from: Option<&str>, to: Option<&str>, now_ms: u64) -> Result<ComparisonView, String> {
        let name = self.device_namer();
        self.with_work(id, now_ms, |w, _| {
            let version = |m: Option<&str>| -> Result<Version, String> {
                let Some(m) = m else { return Ok(Version::Now) };
                let m: ChangeId = m.parse().map_err(|_| format!("Not a Milestone: {m}"))?;
                if !w.doc.milestones().iter().any(|x| x.id == m) {
                    return Err("That Milestone was deleted (perhaps on another Device).".into());
                }
                Ok(Version::Milestone(m))
            };
            let (from, to) = (version(from)?, version(to)?);
            Ok(comparison_view(w.doc.compare(from, to), &w.doc, &name))
        })
    }

    /// What reverting some of an open Work's Changes would do, with and without their dependents.
    pub fn preview_revert(&mut self, id: &str, changes: &[String], now_ms: u64) -> Result<RevertPreview, String> {
        let changes = change_ids(changes)?;
        let name = self.device_namer();
        self.with_work(id, now_ms, |w, _| {
            let doc = &w.doc;
            let targets = doc.revertable(&changes);
            let dependents = doc.dependents(&targets);
            let left = doc.left_out(&changes);
            let milestones = doc.milestones_and_deleted();
            Ok(RevertPreview {
                goes: snippets_view(doc.revert_preview(&targets), doc, &name),
                dependents: doc.by_session(&dependents).iter().filter_map(|(_, group)| group_view(doc, group, true, &name)).collect(),
                left_out: LeftOutView {
                    library: left.library.iter().map(|r| library_name(r)).collect(),
                    already_reverted: left.already_reverted,
                    comments: left.comments,
                    milestones: left.milestones.iter()
                        .filter_map(|m| milestones.iter().find(|(x, _)| x.id == *m).map(|(x, _)| x.name.clone()))
                        .collect(),
                },
                nothing: targets.is_empty(),
            })
        })
    }

    /// What restoring some of an open Work's Changes would bring back.
    pub fn preview_restore(&mut self, id: &str, changes: &[String], now_ms: u64) -> Result<RestorePreview, String> {
        let changes = change_ids(changes)?;
        let name = self.device_namer();
        self.with_work(id, now_ms, |w, _| {
            let doc = &w.doc;
            let reverts = doc.reverts_of(&changes);
            let back = doc.revert_preview(&reverts);
            let places = back.snippets.len() + back.fields.iter().filter(|f| !f.library).count();
            Ok(RestorePreview { back: snippets_view(back, doc, &name), places, nothing: reverts.is_empty() })
        })
    }

    /// What returning an open Work to a Milestone would do.
    pub fn preview_return(&mut self, id: &str, milestone: &str, now_ms: u64) -> Result<ReturnPreview, String> {
        let milestone: ChangeId = milestone.parse().map_err(|_| format!("Not a Milestone: {milestone}"))?;
        let name = self.device_namer();
        self.with_work(id, now_ms, |w, _| {
            let doc = &w.doc;
            if !doc.milestones().iter().any(|m| m.id == milestone) {
                return Err("That Milestone was deleted (perhaps on another Device).".into());
            }
            let plan = doc.return_preview(milestone);
            Ok(ReturnPreview {
                since: comparison_view(doc.compare(Version::Now, Version::Milestone(milestone)), doc, &name),
                earlier: doc.by_session(&plan.made_earlier_elsewhere).iter().filter_map(|(_, group)| group_view(doc, group, false, &name)).collect(),
                detached: doc.detached_by_return(milestone),
                preamble: doc.touches_preamble(&plan.reverts),
                nothing: plan.reverts.is_empty(),
            })
        })
    }

    /// Gives a Milestone of an open Work a new name.
    pub fn rename_milestone(&mut self, id: &str, milestone: &str, name: &str, now_ms: u64) -> Result<WorkState, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 200 {
            return Err("A Milestone's name needs 1 to 200 characters.".into());
        }
        let milestone: ChangeId = milestone.parse().map_err(|_| format!("Not a Milestone: {milestone}"))?;
        self.with_work(id, now_ms, |w, _| w.rename_milestone(milestone, name, now_ms))
    }

    /// Deletes a Milestone of an open Work (a Revert of it), or brings a deleted one back.
    pub fn set_milestone_deleted(&mut self, id: &str, milestone: &str, deleted: bool, now_ms: u64) -> Result<WorkState, String> {
        let milestone: ChangeId = milestone.parse().map_err(|_| format!("Not a Milestone: {milestone}"))?;
        self.with_work(id, now_ms, |w, _| w.set_milestone_deleted(milestone, deleted, now_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::*;
    use super::*;
    use crate::work::{Op, Origin, Tint};

    fn typing(from: usize, text: &str) -> Op {
        Op::Type { from, to: from, text: text.into(), origin: Origin::Input }
    }

    /// A Vault with "Rain" open: "The rain had stopped." typed on this Device ("Laptop"), and
    /// Milestone "Draft 1" named after it.
    fn rain(dir: &Path) -> Vault {
        let mut vault = open(dir);
        vault.set_device_name("Laptop", 500).unwrap();
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "The rain had stopped.")], 2000).unwrap();
        vault.make_milestone("Rain", "Draft 1", 3000).unwrap(); // the burst closes first
        vault
    }

    fn texts(shown: &SnippetsView, kind: &str) -> Vec<String> {
        shown.snippets.iter().flat_map(|s| &s.segments).filter(|s| s.kind == kind).map(|s| s.text.clone()).collect()
    }

    #[test]
    fn the_history_lists_sessions_by_device_name_and_every_milestone() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = rain(dir.path());
        let history = vault.history("Rain", 50, false, 4000).unwrap();
        // Newest first: the typing, then making the Work (its first Look, a Preamble line).
        assert_eq!(history.sessions.len(), 2);
        assert!(!history.more);
        let made = vault.snippets("Rain", &history.sessions[1].changes, false, 4000).unwrap();
        assert!(made.snippets.is_empty());
        assert!(made.fields.iter().any(|f| f.field == "looks" && f.value.as_deref() == Some("Paperback")));
        let session = &history.sessions[0];
        assert_eq!((session.device.as_str(), session.added_words, session.reverted), ("Laptop", 4, "no"));
        let shown = vault.snippets("Rain", &session.changes, false, 4000).unwrap();
        assert_eq!(texts(&shown, "added"), ["The rain had stopped."]);

        let milestone = history.milestones[0].id.clone();
        assert_eq!((history.milestones[0].name.as_str(), history.milestones[0].deleted), ("Draft 1", false));
        vault.rename_milestone("Rain", &milestone, "First draft", 5000).unwrap();
        vault.set_milestone_deleted("Rain", &milestone, true, 6000).unwrap();
        let deleted = &vault.history("Rain", 50, false, 7000).unwrap().milestones[0];
        assert_eq!((deleted.name.as_str(), deleted.deleted), ("First draft", true), "still listed, to restore");
        assert!(vault.rename_milestone("Rain", &milestone, "Again", 7500).is_err(), "a deleted one can't be renamed");
        vault.set_milestone_deleted("Rain", &milestone, false, 8000).unwrap();
        assert!(!vault.history("Rain", 50, false, 9000).unwrap().milestones[0].deleted);
        assert_eq!(vault.history("Rain", 0, false, 9000).unwrap().sessions.len(), 0);
        assert!(vault.history("Rain", 0, false, 9000).unwrap().more);
    }

    #[test]
    fn a_clicked_snippet_is_found_where_it_is_now() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = rain(dir.path());
        let session = vault.history("Rain", 50, false, 4000).unwrap().sessions.remove(0);
        let find = vault.snippets("Rain", &session.changes, false, 4000).unwrap().snippets.remove(0).find.expect("something to go to");
        // More typed before it, still a draft: the place counts it.
        let state = vault.edit("Rain", vec![typing(0, "Outside, "), Op::Find { first: find.first, last: find.last }], 5000).unwrap();
        assert_eq!(state.found, Some(Some((9, 30))));
        let plain = vault.edit("Rain", vec![typing(9, "x")], 5100).unwrap();
        assert_eq!(plain.found, None, "only an answer to a Find carries it");
    }

    /// The phase's check, as the History tab does it: "stopped" changed to "eased" in a later
    /// sitting, then the first sitting reverted from its preview.
    #[test]
    fn the_rain_had_eased_reverted_from_the_history_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = rain(dir.path());
        let later = 3000 + wrtier::doc::SESSION_GAP_MS;
        vault.edit("Rain", vec![Op::Delete { from: 13, to: 20, origin: Origin::Input }, typing(13, "eased")], later).unwrap();
        vault.make_milestone("Rain", "Draft 2", later + 1000).unwrap(); // closes the burst
        let history = vault.history("Rain", 50, false, later + 2000).unwrap();
        let first = history.sessions.iter().find(|s| s.added_words == 4).expect("the first sitting").changes.clone();

        let preview = vault.preview_revert("Rain", &first, later + 2000).unwrap();
        assert!(!preview.nothing);
        assert_eq!(texts(&preview.goes, "removed"), ["The rain had ", "."], "without its dependents, \"eased\" stays");
        assert_eq!(preview.dependents.len(), 1, "\"eased\", from the later sitting");
        assert_eq!(preview.dependents[0].device, "Laptop");
        assert_eq!(texts(&preview.dependents[0].snippets, "removed"), ["eased"]);

        let state = vault.edit("Rain", vec![Op::Revert { changes: first.clone(), dependents: true }], later + 3000).unwrap();
        assert!(state.patches.iter().any(|p| p.insert.is_empty()), "the editor is told to take it out");
        assert_eq!(vault.with_work("Rain", 0, |w, _| Ok(w.text())).unwrap(), "");

        let restore = vault.preview_restore("Rain", &first, later + 4000).unwrap();
        assert_eq!(texts(&restore.back, "added"), ["The rain had eased."]);
        assert_eq!(restore.places, 1);
        vault.edit("Rain", vec![Op::Undo], later + 5000).unwrap();
        assert_eq!(vault.with_work("Rain", 0, |w, _| Ok(w.text())).unwrap(), "The rain had eased.");
    }

    #[test]
    fn a_return_preview_shows_what_comes_back_and_which_comments_would_detach() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = rain(dir.path());
        let draft = vault.history("Rain", 50, false, 4000).unwrap().milestones[0].id.clone();
        vault.edit("Rain", vec![typing(21, " Gulls circled.")], 5000).unwrap();
        vault.edit("Rain", vec![Op::Comment { from: 22, to: 27, sender: "author".into(), body: "new?".into(), sender_name: None }], 6000).unwrap();
        let preview = vault.preview_return("Rain", &draft, 7000).unwrap();
        assert!(!preview.nothing);
        assert_eq!(texts(&SnippetsView { snippets: preview.since.snippets, fields: Vec::new() }, "removed"), [" Gulls circled."]);
        assert_eq!(preview.detached, 1);
        assert!(preview.earlier.is_empty());
        assert!(!preview.preamble);
    }

    #[test]
    fn the_history_of_a_paragraph_leaves_out_what_never_touched_it_and_can_revert_from_there() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = rain(dir.path());
        let later = 3000 + wrtier::doc::SESSION_GAP_MS;
        vault.edit("Rain", vec![typing(21, "
Gulls circled.")], later).unwrap();
        vault.edit("Rain", vec![typing(28, "grey ")], later + 1000).unwrap(); // in the second paragraph
        // The cursor in the first paragraph, nothing selected: a burst is still open, and closes.
        let state = vault.edit("Rain", vec![Op::Stretch { from: 5, to: 5 }], later + 2000).unwrap();
        assert_eq!(state.stretch, Some(Some("The rain had stopped.".to_string())));
        let all = vault.history("Rain", 50, false, later + 3000).unwrap();
        let only = vault.history("Rain", 50, true, later + 3000).unwrap();
        assert_eq!(all.sessions.len(), 3, "making the Work, the first sitting, the later one");
        assert_eq!(only.sessions.len(), 2, "the later sitting only wrote at the paragraph's end: {:?}", only.sessions);
        let first = &only.sessions[0];
        let clipped = vault.snippets("Rain", &first.changes, true, later + 3000).unwrap();
        assert_eq!(texts(&clipped, "plain").concat() + &texts(&clipped, "added").concat(), "The rain had stopped.
");

        // Revert from the filtered view: only that paragraph's text goes.
        let sitting = only.sessions.iter().find(|s| s.added_words == 4).unwrap().changes.clone();
        vault.edit("Rain", vec![Op::Revert { changes: sitting, dependents: true }], later + 4000).unwrap();
        assert_eq!(vault.with_work("Rain", 0, |w, _| Ok(w.text())).unwrap(), "
Gulls grey circled.");

        // An empty line has no text to show the History of.
        let none = vault.edit("Rain", vec![Op::Stretch { from: 0, to: 0 }], later + 5000).unwrap();
        assert_eq!(none.stretch, Some(None));
    }

    /// The phase's check for Merge notices: two Devices move overlapping text offline. After
    /// syncing, the notice shows, "Show me" lands on the text, and Dismiss reaches the other Device.
    #[test]
    fn overlapping_moves_made_offline_give_a_notice_to_show_and_dismiss() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.set_device_name("Laptop", 500).unwrap();
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "abcdef")], 2000).unwrap();
        vault.make_milestone("Rain", "Draft 1", 2500).unwrap(); // closes the burst and saves
        let db = vault.with_work("Rain", 0, |w, _| Ok(w.db.clone())).unwrap();
        // Offline, the phone moves "cde" to the start, and the laptop "bcd" to the end.
        let (_, mut phone) = storage::load(&db, DeviceId(2)).unwrap();
        let phones = phone.move_local(2, 3, 0, 3000);
        let state = vault.edit("Rain", vec![Op::Move { from: 1, to: 4, at: 6 }], 4000).unwrap();
        assert_eq!(state.notices, 0, "nothing to say before they meet");
        // Syncthing delivers the phone's Move; the timer notices, and the editor sends a Sync.
        storage::save(&db, &[phones], &phone).unwrap();
        vault.tick(5000);
        let state = vault.edit("Rain", vec![Op::Sync], 5001).unwrap();
        assert_eq!(state.notices, 1, "the tab reads \"History (1)\"");

        let history = vault.history("Rain", 50, false, 6000).unwrap();
        let notice = &history.notices[0];
        assert_eq!((notice.kind, notice.device.as_str(), notice.quote.as_str()), ("overlap", "Laptop", "cd"), "the later Move took what both moved");
        assert!(notice.other_device.is_some());
        assert_eq!(notice.places.len(), 1);

        // "Show me" selects it.
        let place = &notice.places[0];
        let state = vault.edit("Rain", vec![Op::Find { first: place.first.clone(), last: place.last.clone() }], 6000).unwrap();
        let (from, to) = state.found.flatten().expect("it's in the text");
        let text = vault.with_work("Rain", 0, |w, _| Ok(w.text())).unwrap();
        assert_eq!(&text[from..to], "cd");

        // Dismissed here, it's gone on the phone too once the file reaches it.
        let key = notice.key.clone();
        let state = vault.dismiss_notice("Rain", &key, 7000).unwrap();
        assert_eq!(state.notices, 0);
        assert!(vault.history("Rain", 50, false, 7000).unwrap().notices.is_empty());
        let (_, phone) = storage::load(&db, DeviceId(2)).unwrap();
        assert!(phone.merge_notices().is_empty());
        assert!(vault.dismiss_notice("Rain", &key, 8000).is_err(), "already dismissed");
    }

    #[test]
    fn a_comment_body_edited_on_two_devices_stays_on_its_card() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = rain(dir.path());
        let state = vault.edit("Rain", vec![Op::Comment { from: 4, to: 8, sender: "author".into(), body: "first".into(), sender_name: None }, Op::Marks], 4000).unwrap();
        let id = state.marks.unwrap().remove(0).id;
        let db = vault.with_work("Rain", 0, |w, _| Ok(w.db.clone())).unwrap();
        let (_, mut phone) = storage::load(&db, DeviceId(2)).unwrap();
        let phones = phone.edit_comment_local(id.parse().unwrap(), "the phone's", 5000);
        vault.edit("Rain", vec![Op::EditComment { id, body: "the laptop's".into() }], 6000).unwrap();
        storage::save(&db, &[phones], &phone).unwrap();
        vault.tick(7000);
        let state = vault.edit("Rain", vec![Op::Sync], 7001).unwrap();
        assert_eq!(vault.with_work("Rain", 0, |w, _| Ok(w.doc.merge_notices().len())).unwrap(), 1, "the engine has a notice");
        assert_eq!(state.notices, 0, "but not for the History tab");
        assert!(vault.history("Rain", 50, false, 8000).unwrap().notices.is_empty());
    }

    /// The phase's check for "Colour text by Session": today's text is tinted, newest strongest,
    /// and nothing is worked out unless the editor asks (it only does while the switch is on).
    #[test]
    fn the_newest_sessions_text_is_tinted_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = rain(dir.path());
        let later = 3000 + wrtier::doc::SESSION_GAP_MS;
        let state = vault.edit("Rain", vec![typing(21, " Gulls circled.")], later).unwrap();
        assert!(state.tints.is_none(), "not asked for: nothing worked out");

        // Asked while the burst is still open: the draft counts as its Session's text.
        let state = vault.edit("Rain", vec![Op::Tints], later + 1000).unwrap();
        let tints = state.tints.expect("asked for");
        let text = vault.with_work("Rain", 0, |w, _| Ok(w.text())).unwrap();
        let shown = |t: &Tint| t.ranges.iter().map(|&(a, b)| &text[a..b]).collect::<Vec<_>>().concat();
        assert_eq!(tints.len(), 2);
        assert_eq!((tints[0].rank, tints[0].device.as_str(), shown(&tints[0]).as_str()), (0, "Laptop", " Gulls circled."));
        assert_eq!((tints[1].rank, shown(&tints[1]).as_str()), (1, "The rain had stopped."));
        assert_eq!(tints[0].time_ms, later, "\"Laptop · today 14:02\": when it was last typed in");
    }

    #[test]
    fn comparing_a_milestone_with_now() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = rain(dir.path());
        let draft = vault.history("Rain", 50, false, 4000).unwrap().milestones[0].id.clone();
        vault.edit("Rain", vec![Op::Delete { from: 13, to: 20, origin: Origin::Input }, typing(13, "eased")], 5000).unwrap();
        // "Now" is what's recorded: a burst being typed counts once it closes (the tab is drawn
        // again then).
        vault.with_work("Rain", 5500, |w, _| Ok(w.close_burst(5500))).unwrap();
        let since = vault.compare_versions("Rain", Some(&draft), None, 6000).unwrap();
        assert_eq!((since.added_words, since.removed_words, since.moves), (1, 1, 0));
        assert!(since.snippets[0].find.is_some(), "the later side is now: it can be gone to");
        let back = vault.compare_versions("Rain", None, Some(&draft), 6000).unwrap();
        assert!(back.snippets[0].find.is_none());
        assert!(vault.compare_versions("Rain", Some("nonsense"), None, 6000).is_err());
    }
}
