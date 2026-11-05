use proptest::prelude::*;
use wrtier::change::Change;
use wrtier::doc::Doc;
use wrtier::ids::DeviceId;

// Random thing that can happen.
#[derive(Debug, Clone)]
enum Action {
    Insert { device: usize, at: u16, text: String },
    Delete { device: usize, at: u16, len: u8 },
    Sync { from: usize, to: usize },
}

// Generate a random Action.
fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        (0..3usize, any::<u16>(), "[a-z ]{1,5}").prop_map(|(device, at, text)| Action::Insert { device: device, at, text }),
        (0..3usize, any::<u16>(), 1..4u8).prop_map(|(device, at, len)| Action::Delete { device: device, at, len }),
        (0..3usize, 0..3usize).prop_map(|(from, to)| Action::Sync { from, to }),
    ]
}

fn run(actions: &[Action]) {
    let mut docs = vec![
        Doc::new(DeviceId(0)),
        Doc::new(DeviceId(1)),
        Doc::new(DeviceId(2)),
    ];
    let mut all: Vec<Change> = Vec::new();
    let mut changes: Vec<Vec<Change>> = vec![vec![], vec![], vec![]];
    let skew: [u64; 3] = [0, 0, 60_000]; // each device's clock is skewed by this many ms
    let mut now_ms = 1_000_000u64;

    for action in actions {
        match action {
            Action::Insert { device, at, text } => {
                let index_ = *at as usize % (docs[*device].text().chars().count() + 1);
                let change = docs[*device].insert_local(index_, text, now_ms - skew[*device]);
                let typed: String = docs[*device].text().chars().skip(index_).take(text.chars().count()).collect();
                assert_eq!(&typed, text);
                all.push(change.clone());
                changes[*device].push(change);
                now_ms += 1;
            }
            Action::Delete { device, at, len } => {
                let index_ = *at as usize % (docs[*device].text().chars().count() + 1);
                let len = *len as usize;
                // If the index is at the end of the text, we can't delete anything, so skip this action.
                if index_ >= docs[*device].text().chars().count() {
                    continue;
                }
                let change = docs[*device].delete_local(index_, len, now_ms - skew[*device]);
                all.push(change.clone());
                changes[*device].push(change);
                now_ms += 1;
            }
            Action::Sync { from, to } => {
                if *from == *to {
                    continue;
                }
                // copy of changes[from]
                // sort with bacth.sort_by_key(|c| c.order_key())
                // apply to docs[to]
                // add to changes[to]
                let mut batch = changes[*from].clone();
                batch.sort_by_key(|c| c.order_key());
                for change in batch {
                    if changes[*to].iter().any(|c| c.id == change.id) {
                        continue;
                    }
                    docs[*to].apply(&change);
                    changes[*to].push(change);
                }
            }
        }
    }
    // sync all devices with each other
    for i in 0..docs.len() {
        for j in 0..docs.len() {
            if i != j {
                let mut batch = changes[i].clone();
                batch.sort_by_key(|c| c.order_key());
                for change in batch {
                    docs[j].apply(&change);
                }
            }
        }
    }
    // check that all docs have the same text
    assert_eq!(docs[0].text(), docs[1].text());
    assert_eq!(docs[1].text(), docs[2].text());
    // Compare the final text with what would be expected if all changes were applied in order of their HLC and DeviceId.
    let expected = expected_chars(&all);
    for doc in &docs {
        let mut actual: Vec<char> = doc.text().chars().collect();
        actual.sort();
        assert_eq!(actual, expected);
    }

}

use std::collections::HashSet;
use wrtier::change::ChangeKind;
use wrtier::ids::CharId;

/// Every character that should still be visible: inserted, and not deleted.
fn expected_chars(all: &[Change]) -> Vec<char> {
    let mut deleted: HashSet<CharId> = HashSet::new();
    for c in all {
        let ranges = match &c.kind {
            ChangeKind::Delete(r) => r,
            ChangeKind::Replace { deleted, .. } => deleted,
            ChangeKind::Insert(_) => continue,
        };
        for r in ranges {
            for offset in r.first.offset..r.first.offset + r.len {
                deleted.insert(CharId { change: r.first.change, offset });
            }
        }
    }

    let mut chars = Vec::new();
    for c in all {
        let run = match &c.kind {
            ChangeKind::Insert(run) => run,
            ChangeKind::Replace { inserted, .. } => inserted,
            ChangeKind::Delete(_) => continue,
        };
        for (i, ch) in run.text.chars().enumerate() {
            if !deleted.contains(&CharId { change: c.id, offset: i as u32 }) {
                chars.push(ch);
            }
        }
    }
    chars.sort();
    chars
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn three_devices_convergence(actions in prop::collection::vec(action(), 1..60)) {
        run(&actions);
    }
}
