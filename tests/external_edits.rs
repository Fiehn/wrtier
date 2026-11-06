//! External edits: the Text file edited in another editor, then read back.
//!
//! A random History is written to the Text file. The app makes more edits (or none); the file
//! is edited separately, starting from what was written. Then:
//! - a Device that only has the written version imports the file: its text must be exactly
//!   the file;
//! - the real Device, with the app's later edits too, imports the file: it must end with exactly
//!   the same characters as merging the app's edits with the file's edits as two offline
//!   Devices would, and if the app made no edits, exactly the file.

use proptest::prelude::*;
use wrtier::change::Change;
use wrtier::doc::Doc;
use wrtier::ids::DeviceId;

#[derive(Debug, Clone)]
enum AppEdit {
    Insert { at: u16, text: String },
    Delete { at: u16, len: u8 },
    Move { from: u16, len: u8, to: u16 },
}

fn app_edit() -> impl Strategy<Value = AppEdit> {
    prop_oneof![
        4 => (any::<u16>(), "[a-z ]{1,6}(\n)?").prop_map(|(at, text)| AppEdit::Insert { at, text }),
        2 => (any::<u16>(), 1..6u8).prop_map(|(at, len)| AppEdit::Delete { at, len }),
        1 => (any::<u16>(), 1..40u8, any::<u16>()).prop_map(|(from, len, to)| AppEdit::Move { from, len, to }),
    ]
}

/// An edit made to the file in another editor, on its lines.
#[derive(Debug, Clone)]
enum FileEdit {
    InsertLine { at: u16, text: String },
    DeleteLine { at: u16 },
    MoveLine { from: u16, to: u16 },
    EditInLine { line: u16, at: u8, delete: u8, text: String },
}

fn file_edit() -> impl Strategy<Value = FileEdit> {
    prop_oneof![
        (any::<u16>(), "[a-z ]{0,30}").prop_map(|(at, text)| FileEdit::InsertLine { at, text }),
        any::<u16>().prop_map(|at| FileEdit::DeleteLine { at }),
        (any::<u16>(), any::<u16>()).prop_map(|(from, to)| FileEdit::MoveLine { from, to }),
        (any::<u16>(), any::<u8>(), 0..5u8, "[a-z]{0,6}").prop_map(|(line, at, delete, text)| FileEdit::EditInLine { line, at, delete, text }),
    ]
}

/// Applies one app edit to `doc`, recording its Change.
fn apply_app(doc: &mut Doc, edit: &AppEdit, now: u64, out: &mut Vec<Change>) {
    let n = doc.len();
    match edit {
        AppEdit::Insert { at, text } => out.push(doc.insert_local(*at as usize % (n + 1), text, now)),
        AppEdit::Delete { at, len } => {
            if n > 0 {
                let at = *at as usize % n;
                out.push(doc.delete_local(at, (*len as usize).min(n - at), now));
            }
        }
        AppEdit::Move { from, len, to } => {
            if n > 0 {
                let from = *from as usize % n;
                let len = 1 + *len as usize % (n - from);
                let to = *to as usize % (n + 1);
                if to <= from || to >= from + len {
                    out.push(doc.move_local(from, len, to, now));
                }
            }
        }
    }
}

/// Applies one file edit to the file's lines.
fn apply_file(lines: &mut Vec<String>, edit: &FileEdit) {
    let n = lines.len();
    match edit {
        FileEdit::InsertLine { at, text } => lines.insert(*at as usize % (n + 1), text.clone()),
        FileEdit::DeleteLine { at } => {
            if n > 0 {
                lines.remove(*at as usize % n);
            }
        }
        FileEdit::MoveLine { from, to } => {
            if n > 0 {
                let line = lines.remove(*from as usize % n);
                lines.insert(*to as usize % n, line);
            }
        }
        FileEdit::EditInLine { line, at, delete, text } => {
            if n > 0 {
                let line = &mut lines[*line as usize % n];
                let mut chars: Vec<char> = line.chars().collect();
                let at = *at as usize % (chars.len() + 1);
                let end = (at + *delete as usize).min(chars.len());
                chars.splice(at..end, text.chars());
                *line = chars.into_iter().collect();
            }
        }
    }
}

fn sorted_chars(s: &str) -> Vec<char> {
    let mut chars: Vec<char> = s.chars().collect();
    chars.sort();
    chars
}

fn run(history: &[AppEdit], later: &[AppEdit], edits: &[FileEdit]) {
    // 1. A History, written to the Text file.
    let mut doc = Doc::new(DeviceId(1));
    let mut written: Vec<Change> = vec![doc.insert_local(0, "first line\nsecond line\nthird line\n", 1000)];
    let mut now = 2000;
    for edit in history {
        apply_app(&mut doc, edit, now, &mut written);
        now += 10;
    }
    let version = doc.version();
    let file_text = doc.committed_text();

    // 2. The app goes on.
    let mut app_edits: Vec<Change> = Vec::new();
    for edit in later {
        apply_app(&mut doc, edit, now, &mut app_edits);
        now += 10;
    }

    // 3. Meanwhile the file is edited in another editor, from what was written.
    let ends_with_newline = file_text.ends_with('\n');
    let mut lines: Vec<String> = file_text.lines().map(str::to_string).collect();
    for edit in edits {
        apply_file(&mut lines, edit);
    }
    let mut edited = lines.join("\n");
    if ends_with_newline && !lines.is_empty() {
        edited.push('\n');
    }

    // 4. A Device that only has the written version: the import gives exactly the file.
    let mut offline = Doc::new(DeviceId(2));
    offline.apply_all(&written);
    offline.import_text(Some(&version), &edited, now);
    assert_eq!(offline.text(), edited, "importing against the version it was written from");

    // 5. The real Device imports too. Same characters as merging both sides; exactly the
    //    file if the app made no edits of its own.
    doc.import_text(Some(&version), &edited, now + 1);
    for c in &app_edits {
        offline.apply(c);
    }
    assert_eq!(sorted_chars(&doc.text()), sorted_chars(&offline.text()), "the app's edits and the file's edits both survive");
    if app_edits.is_empty() {
        assert_eq!(doc.text(), edited);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn external_edits_merge_with_the_apps_edits(
        history in prop::collection::vec(app_edit(), 0..20),
        later in prop::collection::vec(app_edit(), 0..6),
        edits in prop::collection::vec(file_edit(), 1..8),
    ) {
        run(&history, &later, &edits);
    }
}
