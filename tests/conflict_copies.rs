//! Conflict copies, end to end through real files: three Devices make random Changes of every
//! kind, each saves to its own copy of the History, and Syncthing leaves them as Conflict
//! copies next to the main file. Absorbing them must delete every copy and leave the main file
//! and the open Doc with exactly what a Work with all the Changes shows.

use proptest::prelude::*;
use wrtier::change::Change;
use wrtier::conflicts::absorb_conflict_copies;
use wrtier::doc::Doc;
use wrtier::ids::DeviceId;
use wrtier::storage;

#[derive(Debug, Clone)]
enum Action {
    Insert { device: usize, at: u16, text: String },
    Delete { device: usize, at: u16, len: u8 },
    Move { device: usize, from: u16, len: u8, to: u16 },
    Revert { device: usize, pick: u16 },
    Rename { device: usize, name: u8 },
    Mark { device: usize },
    Sync { from: usize, to: usize },
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        4 => (0..3usize, any::<u16>(), "[a-z ]{1,5}").prop_map(|(device, at, text)| Action::Insert { device, at, text }),
        2 => (0..3usize, any::<u16>(), 1..4u8).prop_map(|(device, at, len)| Action::Delete { device, at, len }),
        1 => (0..3usize, any::<u16>(), any::<u8>(), any::<u16>()).prop_map(|(device, from, len, to)| Action::Move { device, from, len, to }),
        1 => (0..3usize, any::<u16>()).prop_map(|(device, pick)| Action::Revert { device, pick }),
        1 => (0..3usize, 0..3u8).prop_map(|(device, name)| Action::Rename { device, name }),
        1 => (0..3usize).prop_map(|device| Action::Mark { device }),
        2 => (0..3usize, 0..3usize).prop_map(|(from, to)| Action::Sync { from, to }),
    ]
}

fn run(actions: &[Action]) {
    let mut docs: Vec<Doc> = (0..3).map(|d| Doc::new(DeviceId(d))).collect();
    let mut known: Vec<Vec<Change>> = vec![vec![], vec![], vec![]]; // what each Device has
    let mut now = 1_000_000u64;
    for action in actions {
        now += 10;
        let made = match action {
            Action::Insert { device: d, at, text } => {
                let n = docs[*d].len();
                Some((*d, docs[*d].insert_local(*at as usize % (n + 1), text, now)))
            }
            Action::Delete { device: d, at, len } => {
                let n = docs[*d].len();
                (n > 0).then(|| {
                    let at = *at as usize % n;
                    (*d, docs[*d].delete_local(at, (*len as usize).min(n - at), now))
                })
            }
            Action::Move { device: d, from, len, to } => {
                let n = docs[*d].len();
                if n == 0 { None } else {
                    let from = *from as usize % n;
                    let len = 1 + *len as usize % (n - from);
                    let to = *to as usize % (n + 1);
                    (to <= from || to >= from + len).then(|| (*d, docs[*d].move_local(from, len, to, now)))
                }
            }
            Action::Revert { device: d, pick } => (!known[*d].is_empty()).then(|| {
                let target = known[*d][*pick as usize % known[*d].len()].id;
                (*d, docs[*d].revert_local(target, false, now))
            }),
            Action::Rename { device: d, name } => {
                Some((*d, docs[*d].set_device_name_local(["Laptop", "Phone", "Desk"][*name as usize], now)))
            }
            Action::Mark { device: d } => Some((*d, docs[*d].make_milestone_local("Draft", now))),
            Action::Sync { from, to } => {
                if from != to {
                    let mut batch: Vec<Change> = known[*from].iter().filter(|c| !docs[*to].has(c.id)).cloned().collect();
                    batch.sort_by_key(|c| c.order_key());
                    for c in batch {
                        docs[*to].apply(&c);
                        known[*to].push(c);
                    }
                }
                None
            }
        };
        if let Some((d, change)) = made {
            known[d].push(change);
        }
    }

    // Each Device saved its own copy; Syncthing kept them all as Conflict copies.
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("work.db");
    let typ = dir.path().join("work.typ");
    let copies = [
        dir.path().join("work.sync-conflict-20261105-082020-JGMDN7C.db"),
        dir.path().join("work.sync-conflict-20261105-082020-JGMDN7C.sync-conflict-20261106-101500-US2Y42W.db"),
    ];
    storage::save(&db, &known[0], &docs[0]).unwrap();
    storage::save(&copies[0], &known[1], &docs[1]).unwrap();
    storage::save(&copies[1], &known[2], &docs[2]).unwrap();

    let (_, mut opened) = storage::load(&db, DeviceId(0)).unwrap();
    let absorbed = absorb_conflict_copies(&db, &typ, &mut opened, now + 1);
    assert!(absorbed.kept.is_empty(), "nothing was kept: {:?}", absorbed.kept);
    assert!(copies.iter().all(|c| !c.exists()), "every copy is gone");

    // What a Work with every Change shows.
    let mut all: Vec<Change> = known.iter().flatten().cloned().collect();
    all.sort_by_key(|c| c.order_key());
    all.dedup_by_key(|c| c.id);
    let mut everything = Doc::new(DeviceId(9));
    everything.apply_all(&all);

    let (stored, reopened) = storage::load(&db, DeviceId(0)).unwrap();
    assert_eq!(stored, all, "the main file holds every Change, exactly");
    assert_eq!(opened.text(), everything.text(), "the open Doc");
    assert_eq!(reopened.text(), everything.text(), "the main file, reopened");
    assert_eq!(opened.merge_notices(), everything.merge_notices());
    for d in 0..3 {
        assert_eq!(opened.device_name(DeviceId(d)), everything.device_name(DeviceId(d)));
    }
    assert_eq!(opened.milestones(), everything.milestones());
}

proptest! {
    // Real files in every case, so fewer cases than the other property tests.
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn conflict_copies_absorb_into_exactly_everything(actions in prop::collection::vec(action(), 1..50)) {
        run(&actions);
    }
}
