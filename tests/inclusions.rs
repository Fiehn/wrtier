//! Inclusions in the Text file (ADR 0015): each one is written out in full between two marker
//! lines, and reading the file back turns the block into the Inclusion's character again.

use std::collections::HashMap;
use wrtier::change::{inclusion_record, INCLUSION};
use wrtier::doc::Doc;
use wrtier::ids::{CharId, DeviceId};
use wrtier::storage::{self, Expansions};
use wrtier::text_file::{collapse_inclusions, inclusion_block};

/// A Work with two Inclusions between paragraphs of its own, saved with its Text file written.
fn collection(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf, Doc, Expansions) {
    let db = dir.join("Collected.db");
    let typ = dir.join("Collected.typ");
    let mut doc = Doc::new(DeviceId(1));
    let text = format!("Before the first.\n{INCLUSION}\nBetween them, a long paragraph of the Collection's own.\n{INCLUSION}\nAfter.");
    let mut changes = vec![doc.insert_local(0, &text, 1000)];
    let found = doc.inclusions();
    assert_eq!(found.len(), 2);
    let mut blocks = HashMap::new();
    for (i, &(id, _)) in found.iter().enumerate() {
        changes.push(doc.set_field_local(&inclusion_record(id), "work", &format!("story-{i}"), 1000));
        blocks.insert(id, inclusion_block(id, &format!("Story {i}"), &format!("#[\nThe text of story {i}.\n]")));
    }
    storage::save(&db, &changes, &doc).unwrap();
    storage::write_text_file_with(&db, &typ, &doc, "", &blocks, 1000).unwrap();
    (db, typ, doc, blocks)
}

#[test]
fn inclusions_are_written_out_and_read_back_as_one_character() {
    let dir = tempfile::tempdir().unwrap();
    let (db, typ, mut doc, _) = collection(dir.path());
    let file = std::fs::read_to_string(&typ).unwrap();
    assert!(file.contains("The text of story 0.") && file.contains("The text of story 1."), "{file}");
    assert!(!file.contains(INCLUSION), "the character itself never reaches the file");
    // Unchanged, it's the app's own file: nothing to take in.
    assert_eq!(storage::read_text_file(&db, &typ, &mut doc, 2000).unwrap(), storage::TextFileRead::Known);
}

#[test]
fn edits_inside_a_block_are_dropped_and_edits_outside_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (db, typ, mut doc, _) = collection(dir.path());
    let before = doc.committed_text();
    let file = std::fs::read_to_string(&typ).unwrap()
        .replace("The text of story 0.", "Rewritten in vim.")
        .replace("After.", "After, and more.");
    std::fs::write(&typ, file).unwrap();
    let changes = storage::import_text_file(&db, &typ, &mut doc, 2000).unwrap();
    assert!(!changes.is_empty());
    assert_eq!(doc.committed_text(), before.replace("After.", "After, and more."));
    assert_eq!(doc.inclusions().len(), 2);
}

#[test]
fn a_block_moved_in_another_editor_keeps_what_it_includes() {
    let dir = tempfile::tempdir().unwrap();
    let (db, typ, mut doc, blocks) = collection(dir.path());
    let ids: Vec<CharId> = doc.inclusions().into_iter().map(|(id, _)| id).collect();
    // Move the second block to the very top in vim.
    let file = std::fs::read_to_string(&typ).unwrap();
    let second = &blocks[&ids[1]];
    let file = format!("{second}\n{}", file.replacen(&format!("{second}\n"), "", 1));
    std::fs::write(&typ, &file).unwrap();
    storage::import_text_file(&db, &typ, &mut doc, 2000).unwrap();
    let now = doc.inclusions();
    assert_eq!(now.len(), 2);
    assert_eq!(now[0].1, 0, "the moved Inclusion is first");
    let work = |id: CharId| doc.field(&inclusion_record(id), "work").map(str::to_string);
    assert_eq!(work(now[0].0).as_deref(), Some("story-1"));
    assert_eq!(work(now[1].0).as_deref(), Some("story-0"));
    // And it's saved: a fresh load agrees.
    let (_, loaded) = storage::load(&db, DeviceId(1)).unwrap();
    assert_eq!(loaded.field(&inclusion_record(loaded.inclusions()[0].0), "work"), Some("story-1"));
}

#[test]
fn nested_blocks_collapse_to_the_outer_inclusion() {
    let a: CharId = "00000001:1+0".parse().unwrap();
    let b: CharId = "00000002:1+0".parse().unwrap();
    let inner = inclusion_block(b, "Inner", "Deep.");
    let outer = inclusion_block(a, "Outer", &format!("Start.\n{inner}\nEnd."));
    let (text, named) = collapse_inclusions(&format!("One.\n{outer}\nTwo."));
    assert_eq!(text, format!("One.\n{INCLUSION}\nTwo."));
    assert_eq!(named, vec![Some(a)]);
    // A block whose end line was deleted is left as text.
    let broken = inclusion_block(a, "Outer", "Text.");
    let broken = broken.rsplit_once('\n').unwrap().0;
    let (text, named) = collapse_inclusions(&format!("{broken}\nMore."));
    assert!(text.contains("Text.") && named.is_empty());
}
