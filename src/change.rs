use crate::ids::{ChangeId, CharId, DeviceId, Hlc, Side};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InsertRun {
    pub text: String,
    pub parent: Option<CharId>,
    pub side: Side,
    pub right_origin: Option<CharId>,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub id: ChangeId,
    pub hlc: Hlc,
    pub kind: ChangeKind,
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
            kind: ChangeKind::Insert(InsertRun {
                text: "Hello".to_string(),
                parent: None,
                side: Side::Right,
                right_origin: None,
            }),
        };

        let change2 = Change {
            id: ChangeId { device: DeviceId(1), counter: 2 },
            hlc: Hlc(200),
            kind: ChangeKind::Insert(InsertRun {
                text: "World".to_string(),
                parent: Some(CharId { offset: 4, change: change1.id }),
                side: Side::Right,
                right_origin: None,
            }),
        };

        assert!(change1.order_key() < change2.order_key());
    }
}
