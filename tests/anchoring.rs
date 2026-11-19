//! Anchoring text by identity (an Annotation's pieces, a Comment's): `pieces_at` and `covered`
//! while two Devices go on writing, deleting and Moving text, and sync now and then.
//!
//! Whatever happens, a stretch's pieces cover exactly the stretch when they're made, every one
//! of its characters that's still visible stays covered, and every Device works out the same
//! coverage from the same History.

use proptest::prelude::*;
use wrtier::change::{Change, Span};
use wrtier::doc::Doc;
use wrtier::ids::{CharId, DeviceId};

#[derive(Debug, Clone)]
enum Action {
    Insert { device: usize, at: u16, text: String },
    Delete { device: usize, at: u16, len: u8 },
    Move { device: usize, from: u16, len: u8, to: u16 },
    Sync { to: usize },
    /// Anchor `len` characters at `at` on a Device.
    Anchor { device: usize, at: u16, len: u8 },
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        4 => (0..2usize, any::<u16>(), "[a-dé😀 ]{1,4}").prop_map(|(device, at, text)| Action::Insert { device, at, text }),
        2 => (0..2usize, any::<u16>(), 1..4u8).prop_map(|(device, at, len)| Action::Delete { device, at, len }),
        2 => (0..2usize, any::<u16>(), any::<u8>(), any::<u16>()).prop_map(|(device, from, len, to)| Action::Move { device, from, len, to }),
        2 => (0..2usize).prop_map(|to| Action::Sync { to }),
        2 => (0..2usize, any::<u16>(), 1..8u8).prop_map(|(device, at, len)| Action::Anchor { device, at, len }),
    ]
}

/// One stretch anchored: its pieces, and the characters it held when it was made.
struct Anchored {
    pieces: Vec<Span>,
    chars: Vec<CharId>,
}

/// Delivers everything the other Device has made that `to` doesn't have yet.
fn sync(docs: &mut [Doc; 2], made: &[Vec<Change>; 2], to: usize) {
    let mut batch: Vec<Change> = made[1 - to].iter().filter(|c| !docs[to].has(c.id)).cloned().collect();
    batch.sort_by_key(|c| c.order_key());
    for c in &batch {
        docs[to].apply(c);
    }
}

/// What the pieces cover must include every one of its characters still visible, on a Device
/// that has the characters the pieces start and end at (until then they cover nothing there).
fn check(doc: &Doc, anchored: &Anchored, when: &str) {
    let covered = doc.covered(&anchored.pieces);
    assert!(covered.windows(2).all(|w| w[0] < w[1]), "{when}: sorted and distinct: {covered:?}");
    assert!(covered.iter().all(|&i| i < doc.len()), "{when}: inside the text");
    if !anchored.pieces.iter().all(|s| doc.has(s.first.change) && doc.has(s.last.change)) {
        return;
    }
    for &id in &anchored.chars {
        if let Some(i) = doc.index_of_char(id) {
            assert!(covered.binary_search(&i).is_ok(), "{when}: {id} at {i} isn't covered: {covered:?}");
        }
    }
}

fn run(actions: &[Action]) {
    let mut docs = [Doc::new(DeviceId(1)), Doc::new(DeviceId(2))];
    let mut made: [Vec<Change>; 2] = [Vec::new(), Vec::new()];
    let base = docs[0].insert_local(0, "The rain had stopped. Gulls circled.", 1000);
    docs[1].apply(&base);
    made[0].push(base);
    let mut anchored: Vec<Anchored> = Vec::new();
    let mut now = 2000;
    for action in actions {
        now += 10;
        match action {
            Action::Insert { device: d, at, text } => {
                let at = *at as usize % (docs[*d].len() + 1);
                made[*d].push(docs[*d].insert_local(at, text, now));
            }
            Action::Delete { device: d, at, len } => {
                let n = docs[*d].len();
                if n > 0 {
                    let at = *at as usize % n;
                    made[*d].push(docs[*d].delete_local(at, (*len as usize).min(n - at), now));
                }
            }
            Action::Move { device: d, from, len, to } => {
                let n = docs[*d].len();
                if n > 0 {
                    let from = *from as usize % n;
                    let len = 1 + *len as usize % (n - from);
                    let to = *to as usize % (n + 1);
                    if to <= from || to >= from + len {
                        made[*d].push(docs[*d].move_local(from, len, to, now));
                    }
                }
            }
            Action::Sync { to } => sync(&mut docs, &made, *to),
            Action::Anchor { device: d, at, len } => {
                let n = docs[*d].len();
                if n > 0 {
                    let at = *at as usize % n;
                    let len = (*len as usize).min(n - at);
                    let pieces = docs[*d].pieces_at(at, len);
                    // Made: they cover exactly the selection.
                    assert_eq!(docs[*d].covered(&pieces), (at..at + len).collect::<Vec<_>>(), "anchoring {at}+{len}");
                    anchored.push(Anchored { pieces, chars: docs[*d].chars_at(at, at + len) });
                }
            }
        }
        for (d, doc) in docs.iter().enumerate() {
            for a in &anchored {
                check(doc, a, &format!("Device {d} after {action:?}"));
            }
        }
    }
    sync(&mut docs, &made, 0);
    sync(&mut docs, &made, 1);
    assert_eq!(docs[0].text(), docs[1].text());
    for a in &anchored {
        assert_eq!(docs[0].covered(&a.pieces), docs[1].covered(&a.pieces), "every Device covers the same text");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn anchored_text_stays_covered_whatever_happens(actions in prop::collection::vec(action(), 1..50)) {
        run(&actions);
    }
}
