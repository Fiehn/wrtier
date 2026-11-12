use crate::ids::{ChangeId, CharId, DeviceId, Hlc, Side};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InsertRun {
    pub text: String,
    pub parent: Option<CharId>,
    pub side: Side,
    pub right_origin: Option<CharId>,
    /// The anchor of the moved piece the text was typed into; None = the main text.
    /// Its place in the tree alone can't say this once Moves overlap.
    pub into: Option<CharId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharRange {
    pub first: CharId,
    pub len: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Insert(InsertRun),
    Delete(Vec<CharRange>),
    Replace {deleted: Vec<CharRange>, inserted: InsertRun},
    Revert {targets: Vec<ChangeId>},
    Move {
        pieces: Vec<Span>, // In reading order; one anchpr per piece
        parent: Option<CharId>, // where the first anchor goes
        side: Side,
        right_origin: Option<CharId>,
        seen: Vec<(DeviceId, u32)>, // history of the move
        into: Option<CharId>,       // the piece the anchors were dropped into, like InsertRun::into
    },
    /// Hides a Merge notice. Notices are worked out from the History; dismissing one is the only part recorded.
    Dismiss { notice: NoticeKey },
    /// Sets one field of a record: anything that isn't the Work's text, such as a Device's name.
    /// The latest value in the fixed Change order wins; older values stay in the History,
    /// and reverting this Change brings the previous value back.
    /// `replaces`: the Change whose value the Device saw when it set this one (None if the field
    /// had none). Two values that replaced the same one were set without seeing each other,
    /// whatever their times: for free text that's worth a Merge notice.
    SetField { record: String, field: String, value: String, replaces: Option<ChangeId> },
    /// A named point in the History: exactly the Changes the naming Device had seen, as the
    /// highest counter it had from each Device. Rename it with a field on `milestone_record`;
    /// delete it by reverting this Change.
    Milestone { name: String, seen: Vec<(DeviceId, u32)> },
    /// A Comment on some text. Its ID is the Comment's ID. It covers every node in its pieces'
    /// ranges (in the tree's fixed order), so text later typed inside it is covered too; the
    /// pieces are split wherever the selection skipped visible text, so they cover exactly what
    /// was selected. `quote` is the selected text, kept for the Comments panel and readable rows.
    /// `sender` is `AUTHOR` or a `reviewer_record`. Later edits are fields on `comment_record`.
    Comment { pieces: Vec<Span>, quote: String, sender: String, body: String },
    /// A Reply to a Comment. Later edits are fields on its own `comment_record`.
    Reply { comment: ChangeId, sender: String, body: String },
}

/// The sender of a Comment or Reply written by the Author (not a Reviewer).
pub const AUTHOR: &str = "author";

/// A Reviewer: the sender of their Comments, and the record holding their name in a Work
/// (and their name, role and colour in `vault.db`). `id` is the Reviewer's random ID.
pub fn reviewer_record(id: &str) -> String {
    format!("reviewer/{id}")
}

/// The record holding a Comment's or Reply's later edits: its body, sender, and (for a
/// Comment) whether it's resolved.
pub fn comment_record(comment: ChangeId) -> String {
    format!("comment/{comment}")
}

/// The record holding a Device's own settings, such as its readable name.
pub fn device_record(device: DeviceId) -> String {
    format!("device/{device}")
}

/// The record holding a Milestone's settings, such as a new name.
pub fn milestone_record(milestone: ChangeId) -> String {
    format!("milestone/{milestone}")
}

/// The record describing a Session, e.g. that it came from an External edit.
pub fn session_record(session: ChangeId) -> String {
    format!("session/{session}")
}

/// The record tying together the Changes of one Find and replace action (a Replace all, or one
/// Replace that rewrote several words), so undo and the History view take them as one. Named
/// by its first Change. Fields: `changes` (their IDs, joined by commas), `find` and `with`
/// (what was searched for and the replacement, as typed) and `count` (how many matches).
pub fn replace_record(first: ChangeId) -> String {
    format!("replace/{first}")
}

/// Records that aren't part of the Work's content: returning to a Milestone leaves them alone.
/// Comments aren't content either: feedback stays when the text goes back. Nor is how Find
/// and replace grouped its Changes.
pub fn is_meta_record(record: &str) -> bool {
    ["device/", "milestone/", "session/", "comment/", "reviewer/", "replace/"].iter().any(|prefix| record.starts_with(prefix))
}

/// The record holding a Work's Preamble: its Looks, extra Styles, Story elements and settings.
pub const PREAMBLE_RECORD: &str = "preamble";

/// A Work's recorded copy of a Style (`style/<name>`), Look (`look/<name>`) or Story element
/// (`element/<name>`) from the Style library, or the version of another Work it includes
/// (`included/<Work ID>`, ADR 0015). The app records these on its own, to mirror the library
/// and the included Works, so reverting a Session leaves them alone.
pub fn is_library_copy(record: &str) -> bool {
    ["style/", "look/", "element/", "included/"].iter().any(|prefix| record.starts_with(prefix))
}

/// The character that stands for an Inclusion in a Work's text: another Work, or one Chapter
/// of it, shown there live. It's always on a line of its own, and being one character, it's
/// added, deleted, moved and reverted whole. What it includes is in its `inclusion_record`.
pub const INCLUSION: char = '\u{FFFC}';

/// The record saying what an Inclusion includes, named by its character: the included Work's
/// ID (`work`), the character starting the included Chapter's heading line (`chapter`, absent
/// for the whole Work) and Styles for this Inclusion only (`styles`, one name per line).
pub fn inclusion_record(at: CharId) -> String {
    format!("inclusion/{at}")
}

/// The record holding the version of an included Work this Work last showed (ADR 0015): its
/// `version` field is the highest Change counter it had from each Device, as `ChangeId`s
/// joined by commas. Named by the included Work's ID.
pub fn included_record(work: &str) -> String {
    format!("included/{work}")
}

/// Fields holding free text the Author writes (in v1 a single value, not a text sequence): two
/// Devices editing one without seeing each other's edit get a Merge notice, so the version that
/// lost can be copied back. Other fields (names, settings) just take the later value quietly.
pub fn is_free_text(record: &str, field: &str) -> bool {
    (record == PREAMBLE_RECORD && field == "raw") || (record.starts_with("comment/") && field == "body")
}

/// Names a Merge notice the same way on every Device, by the Moves involved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoticeKey {
    /// Two Moves made without knowing about each other took the same text; the later one got it.
    Overlap { later: ChangeId, earlier: ChangeId },
    /// A Move would have put text inside itself, so it was skipped and its text stayed where it was.
    Loop { skipped: ChangeId },
    /// The Text file was edited outside the app, but the version it was written from couldn't be
    /// found, so the edit was compared against the current text instead. Named by its Session.
    UnknownTextFile { session: ChangeId },
    /// Two Devices edited the same free-text field (such as the Preamble's raw Typst) without
    /// seeing each other's edit: `later` is shown, `earlier` is the version that lost.
    ConcurrentField { later: ChangeId, earlier: ChangeId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub id: ChangeId,
    pub hlc: Hlc,
    /// The Session it belongs to: the ID of the Session's first Change.
    /// A Session is one Device's Changes in one sitting, and never spans Devices or Works.
    pub session: ChangeId,
    pub kind: ChangeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub first: CharId,
    pub last: CharId,
    /// The anchor of the piece this text was drawn in when selected; None = the main text.
    /// It names the piece, not just the Move: a Move can have several pieces, and
    /// this piece may only take text from the one it was selected from.
    pub from: Option<CharId>,
}


impl Change {
    pub fn order_key(&self) -> (Hlc, DeviceId) {
        (self.hlc, self.id.device)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    
    // test that Change::order_key() returns a tuple of (Hlc, DeviceId) and that the ordering is correct
    #[test]
    fn test_ordering() {
        let change1 = Change {
            id: ChangeId { device: DeviceId(9), counter: 1 },
            hlc: Hlc(100),
            session: ChangeId { device: DeviceId(9), counter: 1 },
            kind: ChangeKind::Insert(InsertRun {
                text: "Hello".to_string(),
                parent: None,
                side: Side::Right,
                right_origin: None,
                into: None,
            }),
        };

        let change2 = Change {
            id: ChangeId { device: DeviceId(1), counter: 2 },
            hlc: Hlc(200),
            session: ChangeId { device: DeviceId(1), counter: 2 },
            kind: ChangeKind::Insert(InsertRun {
                text: "World".to_string(),
                parent: Some(CharId { offset: 4, change: change1.id }),
                side: Side::Right,
                right_origin: None,
                into: None,
            }),
        };

        assert!(change1.order_key() < change2.order_key());
    }
}
