//! The fast History engine must behave exactly like the slow, simple reference
//! (`wrtier::reference`), for every action of every random three-Device history.
//!
//! Every action runs on both. Local actions must produce the identical Change (same placement,
//! same pieces, same everything), and after every action the text, the visible characters with
//! their pieces, and the Merge notices must match.

use proptest::prelude::*;
use wrtier::change::{Change, NoticeKey};
use wrtier::doc::Doc;
use wrtier::ids::{CharId, DeviceId};
use wrtier::reference::doc::Doc as RefDoc;

#[derive(Debug, Clone)]
enum Action {
    Insert { device: usize, at: u16, text: String },
    Delete { device: usize, at: u16, len: u8 },
    Sync { from: usize, to: usize },
    Revert { device: usize, pick: u16, deps: bool },
    Restore { device: usize, pick: u16 },
    Move { device: usize, from: u16, len: u8, to: u16 },
    Dismiss { device: usize, pick: u16 },
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        4 => (0..3usize, any::<u16>(), "[a-z ]{1,5}").prop_map(|(device, at, text)| Action::Insert { device, at, text }),
        2 => (0..3usize, any::<u16>(), 1..4u8).prop_map(|(device, at, len)| Action::Delete { device, at, len }),
        2 => (0..3usize, 0..3usize).prop_map(|(from, to)| Action::Sync { from, to }),
        1 => (0..3usize, any::<u16>(), any::<bool>()).prop_map(|(device, pick, deps)| Action::Revert { device, pick, deps }),
        1 => (0..3usize, any::<u16>()).prop_map(|(device, pick)| Action::Restore { device, pick }),
        2 => (0..3usize, any::<u16>(), any::<u8>(), any::<u16>()).prop_map(|(device, from, len, to)| Action::Move { device, from, len, to }),
        1 => (0..3usize, any::<u16>()).prop_map(|(device, pick)| Action::Dismiss { device, pick }),
    ]
}

/// Notices as plain data, so the two implementations' types can be compared.
fn notices_of(doc: &Doc) -> Vec<(NoticeKey, Vec<CharId>, String)> {
    doc.merge_notices().into_iter().map(|n| (n.key, n.chars, n.text)).collect()
}
fn ref_notices_of(doc: &RefDoc) -> Vec<(NoticeKey, Vec<CharId>, String)> {
    doc.merge_notices().into_iter().map(|n| (n.key, n.chars, n.text)).collect()
}

/// Everything the Author could see must be identical.
fn same(fast: &Doc, slow: &RefDoc, after: &str) {
    assert_eq!(fast.text(), slow.text(), "text differs after {after}");
    assert_eq!(fast.visible().to_vec(), slow.visible().to_vec(), "visible characters differ after {after}");
    assert_eq!(notices_of(fast), ref_notices_of(slow), "Merge notices differ after {after}");
}

fn run(actions: &[Action]) {
    let mut fast: Vec<Doc> = (0..3).map(|d| Doc::new(DeviceId(d))).collect();
    let mut slow: Vec<RefDoc> = (0..3).map(|d| RefDoc::new(DeviceId(d))).collect();
    let mut changes: Vec<Vec<Change>> = vec![vec![], vec![], vec![]];
    let skew: [u64; 3] = [0, 0, 60_000];
    let mut now = 1_000_000u64;

    for (step, action) in actions.iter().enumerate() {
        let after = format!("step {step}: {action:?}");
        // Each local action runs on both and returns (Device, fast Change, slow Change).
        let (d, fast_change, slow_change) = match action {
            Action::Insert { device: d, at, text } => {
                let i = *at as usize % (fast[*d].len() + 1);
                let t = now - skew[*d];
                (*d, Some(fast[*d].insert_local(i, text, t)), Some(slow[*d].insert_local(i, text, t)))
            }
            Action::Delete { device: d, at, len } => {
                let n = fast[*d].len();
                let i = *at as usize % (n + 1);
                if i >= n {
                    continue;
                }
                let t = now - skew[*d];
                let len = *len as usize;
                (*d, Some(fast[*d].delete_local(i, len, t)), Some(slow[*d].delete_local(i, len, t)))
            }
            Action::Sync { from, to } => {
                if from == to {
                    continue;
                }
                let mut batch = changes[*from].clone();
                batch.sort_by_key(|c| c.order_key());
                for c in batch {
                    if changes[*to].iter().any(|x| x.id == c.id) {
                        continue;
                    }
                    fast[*to].apply(&c);
                    slow[*to].apply(&c);
                    changes[*to].push(c);
                }
                same(&fast[*to], &slow[*to], &after);
                continue;
            }
            Action::Revert { device: d, pick, deps } => {
                if changes[*d].is_empty() {
                    continue;
                }
                let target = changes[*d][*pick as usize % changes[*d].len()].id;
                let t = now - skew[*d];
                (*d, Some(fast[*d].revert_local(target, *deps, t)), Some(slow[*d].revert_local(target, *deps, t)))
            }
            Action::Restore { device: d, pick } => {
                if changes[*d].is_empty() {
                    continue;
                }
                let target = changes[*d][*pick as usize % changes[*d].len()].id;
                let t = now - skew[*d];
                (*d, fast[*d].restore_local(target, t), slow[*d].restore_local(target, t))
            }
            Action::Move { device: d, from, len, to } => {
                let n = fast[*d].len();
                if n == 0 {
                    continue;
                }
                let from = *from as usize % n;
                let len = 1 + *len as usize % (n - from);
                let to = *to as usize % (n + 1);
                if to > from && to < from + len {
                    continue;
                }
                let t = now - skew[*d];
                (*d, Some(fast[*d].move_local(from, len, to, t)), Some(slow[*d].move_local(from, len, to, t)))
            }
            Action::Dismiss { device: d, pick } => {
                let notices = fast[*d].merge_notices();
                if notices.is_empty() {
                    continue;
                }
                let key = notices[*pick as usize % notices.len()].key;
                let t = now - skew[*d];
                (*d, Some(fast[*d].dismiss_local(key, t)), Some(slow[*d].dismiss_local(key, t)))
            }
        };
        // The Changes must be identical: same placement, same pieces, same everything.
        assert_eq!(fast_change, slow_change, "different Change from {after}");
        if let Some(c) = fast_change {
            changes[d].push(c);
        }
        now += 1;
        same(&fast[d], &slow[d], &after);
    }

    // Finally everyone gets everything, all at once (the way a Work is opened).
    for d in 0..3 {
        let mut all: Vec<Change> = changes.iter().flatten().cloned().collect();
        all.sort_by_key(|c| c.order_key());
        all.dedup_by_key(|c| c.id);
        let mut opened = Doc::new(DeviceId(d as u32));
        opened.apply_all(&all);
        let mut opened_slow = RefDoc::new(DeviceId(d as u32));
        opened_slow.apply_all(&all);
        same(&opened, &opened_slow, "opening the whole History");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn fast_engine_matches_the_reference(actions in prop::collection::vec(action(), 1..60)) {
        run(&actions);
    }
}
