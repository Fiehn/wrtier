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
    Revert { device: usize, pick: u16, deps: bool },
    Restore { device: usize, pick: u16 },
    Move { device: usize, from: u16, len: u8, to: u16 },
}

// Generate a random Action.
fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        (0..3usize, any::<u16>(), "[a-z ]{1,5}").prop_map(|(device, at, text)| Action::Insert { device: device, at, text }),
        (0..3usize, any::<u16>(), 1..4u8).prop_map(|(device, at, len)| Action::Delete { device: device, at, len }),
        (0..3usize, 0..3usize).prop_map(|(from, to)| Action::Sync { from, to }),
        (0..3usize, any::<u16>(), any::<bool>()).prop_map(|(device, pick, deps)| Action::Revert { device, pick, deps }),
        (0..3usize, any::<u16>()).prop_map(|(device, pick)| Action::Restore { device, pick }),
        (0..3usize, any::<u16>(), any::<u8>(), any::<u16>())
            .prop_map(|(device, from, len, to)| Action::Move { device, from, len, to }),
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
    let mut now_ms: u64 = 1_000_000u64;

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
            Action::Revert { device, pick, deps } => {
                // Revert any Change this Device knows about (its own or synced).
                if changes[*device].is_empty() {
                    continue;
                }
                let target = changes[*device][*pick as usize % changes[*device].len()].id;
                let change = docs[*device].revert_local(target, *deps, now_ms - skew[*device]);
                all.push(change.clone());
                changes[*device].push(change);
                now_ms += 1;
            }
            Action::Restore { device, pick } => {
                if changes[*device].is_empty() {
                    continue;
                }
                let target = changes[*device][*pick as usize % changes[*device].len()].id;
                // Nothing to restore if the target has no Revert in effect.
                if let Some(change) = docs[*device].restore_local(target, now_ms - skew[*device]) {
                    all.push(change.clone());
                    changes[*device].push(change);
                    now_ms += 1;
                }
            }
            Action::Move { device, from, len, to } => {
                let before: Vec<char> = docs[*device].text().chars().collect();
                let n = before.len();
                if n == 0 {
                    continue;
                }
                // Turn the random numbers into a valid Move: 1..=rest characters from `from`,
                // to anywhere that isn't strictly inside the selection.
                let from = *from as usize % n;
                let len = 1 + *len as usize % (n - from);
                let to = *to as usize % (n + 1);
                if to > from && to < from + len {
                    continue;
                }

                // What the Author expects to see: cut the selection out, put it back at `to`
                // (which shifts left by `len` if it was after the selection).
                let moved: Vec<char> = before[from..from + len].to_vec();
                let mut expected: Vec<char> = before[..from].iter().chain(&before[from + len..]).copied().collect();
                let at = if to <= from { to } else { to - len };
                expected.splice(at..at, moved);
                let expected: String = expected.into_iter().collect();

                let change = docs[*device].move_local(from, len, to, now_ms - skew[*device]);
                assert_eq!(docs[*device].text(), expected, "move {from}+{len} to {to}");
                all.push(change.clone());
                changes[*device].push(change);
                now_ms += 1;
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

use std::collections::HashMap;
use wrtier::change::ChangeKind;
use wrtier::ids::{ChangeId, CharId};

/// The "in effect" rule, written a second time without using Doc:
/// a Change is in effect unless a Revert that is itself in effect targets it.
fn in_effect(id: ChangeId, reverted_by: &HashMap<ChangeId, Vec<ChangeId>>) -> bool {
    match reverted_by.get(&id) {
        None => true,
        Some(reverts) => reverts.iter().all(|&r| !in_effect(r, reverted_by)),
    }
}

/// Every character that should still be visible: its Change is in effect,
/// and every Change that deleted it is not.
fn expected_chars(all: &[Change]) -> Vec<char> {
    let mut reverted_by: HashMap<ChangeId, Vec<ChangeId>> = HashMap::new();
    for c in all {
        if let ChangeKind::Revert { targets } = &c.kind {
            for &t in targets {
                reverted_by.entry(t).or_default().push(c.id);
            }
        }
    }

    // Each character → the Changes that deleted it.
    let mut deleted_by: HashMap<CharId, Vec<ChangeId>> = HashMap::new();
    for c in all {
        let ranges = match &c.kind {
            ChangeKind::Delete(r) => r,
            ChangeKind::Replace { deleted, .. } => deleted,
            ChangeKind::Insert(_) | ChangeKind::Revert { .. } => continue,
            ChangeKind::Move { .. } => continue, // Move changes do not delete characters.
        };
        for r in ranges {
            for offset in r.first.offset..r.first.offset + r.len {
                deleted_by.entry(CharId { change: r.first.change, offset }).or_default().push(c.id);
            }
        }
    }

    let mut chars = Vec::new();
    for c in all {
        let run = match &c.kind {
            ChangeKind::Insert(run) => run,
            ChangeKind::Replace { inserted, .. } => inserted,
            ChangeKind::Delete(_) | ChangeKind::Revert { .. } => continue,
            ChangeKind::Move { .. } => continue, // Move changes do not add characters to the text.
        };
        if !in_effect(c.id, &reverted_by) {
            continue;
        }
        for (i, ch) in run.text.chars().enumerate() {
            let id = CharId { change: c.id, offset: i as u32 };
            let still_deleted = deleted_by
                .get(&id)
                .map_or(false, |dels| dels.iter().any(|&d| in_effect(d, &reverted_by)));
            if !still_deleted {
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
