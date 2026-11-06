//! Typing bursts, the way the editor will drive them, against a plain model of what the Author sees.
//!
//! One Device: after every action the shown text must equal the model, and reopening the Work
//! from nothing but the recorded Changes must give exactly what the Author saw.
//! Two Devices: bursts open on both while Changes sync back and forth (merges don't close a
//! burst); in the end both agree, and reopening from the Changes gives the same text.

use proptest::prelude::*;
use wrtier::change::Change;
use wrtier::doc::Doc;
use wrtier::ids::DeviceId;

#[derive(Debug, Clone)]
enum Action {
    Type { at: u16, text: String },
    Delete { at: u16, len: u8 },
    Replace { at: u16, len: u8, text: String },
    Cursor { at: u16 },
    Wait { ms: u16 },
    Close,
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        6 => (any::<u16>(), "[a-c ]{1,3}").prop_map(|(at, text)| Action::Type { at, text }),
        3 => (any::<u16>(), 1..4u8).prop_map(|(at, len)| Action::Delete { at, len }),
        1 => (any::<u16>(), 1..4u8, "[x-z]{0,2}").prop_map(|(at, len, text)| Action::Replace { at, len, text }),
        1 => any::<u16>().prop_map(|at| Action::Cursor { at }),
        1 => (0..8000u16).prop_map(|ms| Action::Wait { ms }),
        1 => Just(Action::Close),
    ]
}

/// Runs one editor action on `doc` and the same edit on `model`; returns what was recorded.
fn act(doc: &mut Doc, model: &mut Vec<char>, action: &Action, now: u64) -> Vec<Change> {
    let n = model.len();
    match action {
        Action::Type { at, text } => {
            let at = *at as usize % (n + 1);
            model.splice(at..at, text.chars());
            doc.type_text(at, text, now)
        }
        Action::Delete { at, len } => {
            if n == 0 {
                return doc.tick(now).into_iter().collect();
            }
            let at = *at as usize % n;
            let len = (*len as usize).min(n - at);
            model.drain(at..at + len);
            doc.delete_text(at, len, now)
        }
        Action::Replace { at, len, text } => {
            if n == 0 {
                return doc.tick(now).into_iter().collect();
            }
            let at = *at as usize % n;
            let len = (*len as usize).min(n - at);
            model.splice(at..at + len, text.chars());
            doc.replace_text(at, len, text, now)
        }
        Action::Cursor { at } => doc.cursor_moved(*at as usize % (n + 1), now).into_iter().collect(),
        Action::Wait { .. } => doc.tick(now).into_iter().collect(),
        Action::Close => doc.close_burst(now).into_iter().collect(),
    }
}

fn run_one(actions: &[Action]) {
    let mut doc = Doc::new(DeviceId(1));
    let mut model: Vec<char> = Vec::new();
    let mut recorded: Vec<Change> = Vec::new();
    let mut now = 1_000_000u64;
    for action in actions {
        now += match action { Action::Wait { ms } => *ms as u64, _ => 150 };
        recorded.extend(act(&mut doc, &mut model, action, now));
        assert_eq!(doc.text(), model.iter().collect::<String>(), "after {action:?}");
        assert_eq!(doc.len(), model.len());
    }
    recorded.extend(doc.close_burst(now + 1));
    let shown: String = model.iter().collect();
    assert_eq!(doc.text(), shown, "closing the burst changes nothing on screen");

    // The recorded Changes alone give exactly what the Author saw.
    let mut reopened = Doc::new(DeviceId(1));
    reopened.apply_all(&recorded);
    assert_eq!(reopened.text(), shown, "reopening from the recorded Changes");
}

fn run_two(actions: &[(bool, Action)], syncs: &[bool]) {
    let mut docs = [Doc::new(DeviceId(1)), Doc::new(DeviceId(2))];
    let mut models: [Vec<char>; 2] = [Vec::new(), Vec::new()];
    let mut recorded: [Vec<Change>; 2] = [Vec::new(), Vec::new()];
    let mut delivered = [0usize; 2]; // how many of the other Device's Changes each has applied
    let mut now = 1_000_000u64;
    for (step, (on_phone, action)) in actions.iter().enumerate() {
        let d = *on_phone as usize;
        now += match action { Action::Wait { ms } => *ms as u64, _ => 150 };
        // The model can't predict merges, so start each local action from what the Device shows.
        models[d] = docs[d].text().chars().collect();
        let changes = act(&mut docs[d], &mut models[d], action, now);
        recorded[d].extend(changes);
        assert_eq!(docs[d].text(), models[d].iter().collect::<String>(), "after {action:?}");
        if syncs.get(step).copied().unwrap_or(false) {
            // Deliver everything the other Device has recorded so far, bursts open or not.
            let other = 1 - d;
            for c in &recorded[other][delivered[d]..] {
                docs[d].apply(c);
            }
            delivered[d] = recorded[other].len();
        }
    }
    for d in 0..2 {
        recorded[d].extend(docs[d].close_burst(now + 1));
    }
    for d in 0..2 {
        let other = 1 - d;
        for c in &recorded[other][delivered[d]..] {
            docs[d].apply(c);
        }
    }
    assert_eq!(docs[0].text(), docs[1].text(), "both Devices agree");

    let mut all: Vec<Change> = recorded.iter().flatten().cloned().collect();
    all.sort_by_key(|c| c.order_key());
    let mut reopened = Doc::new(DeviceId(1));
    reopened.apply_all(&all);
    assert_eq!(reopened.text(), docs[0].text(), "reopening from all the Changes");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn bursts_show_and_record_exactly_what_was_typed(actions in prop::collection::vec(action(), 1..80)) {
        run_one(&actions);
    }

    #[test]
    fn bursts_on_two_devices_converge(
        actions in prop::collection::vec((any::<bool>(), action()), 1..80),
        syncs in prop::collection::vec(any::<bool>(), 80),
    ) {
        run_two(&actions, &syncs);
    }
}
