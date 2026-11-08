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

/// Records that aren't part of the Work's content: returning to a Milestone leaves them alone.
pub fn is_meta_record(record: &str) -> bool {
    record.starts_with("device/") || record.starts_with("milestone/") || record.starts_with("session/")
}

/// The record holding a Work's Preamble: its Looks, extra Styles, Story elements and settings.
pub const PREAMBLE_RECORD: &str = "preamble";

/// Fields holding free text the Author writes (in v1 a single value, not a text sequence): two
/// Devices editing one without seeing each other's edit get a Merge notice, so the version that
/// lost can be copied back. Other fields (names, settings) just take the later value quietly.
pub fn is_free_text(record: &str, field: &str) -> bool {
    record == PREAMBLE_RECORD && field == "raw"
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
