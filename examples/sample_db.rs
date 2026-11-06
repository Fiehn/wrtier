//! Writes `sample.db`: a small History to open in DB Browser for SQLite and check it reads
//! like plain sentences (Phase 1's "done when"). Look at the `history` view first.
//!
//!     cargo run --example sample_db

use std::path::Path;
use wrtier::change::Change;
use wrtier::doc::Doc;
use wrtier::ids::DeviceId;
use wrtier::storage;

const MIN: u64 = 60_000;

fn main() {
    let path = Path::new("sample.db");
    if path.exists() {
        std::fs::remove_file(path).expect("remove the old sample.db");
    }
    let start = 1_783_000_000_000; // a fixed date, so every run writes the same file

    // The laptop names itself and writes the opening.
    let mut laptop = Doc::new(DeviceId(0x1a2b3c4d));
    let mut history: Vec<Change> = vec![
        laptop.set_device_name_local("Laptop", start),
        laptop.insert_local(0, "The rain had stopped. She walked along the harbour wall.\n", start + MIN),
    ];
    let end = laptop.len();
    let gulls = "Gulls circled over the boats.\n";
    history.push(laptop.insert_local(end, gulls, start + 2 * MIN));
    // Move the second line to the top.
    let len = gulls.chars().count();
    history.push(laptop.move_local(laptop.len() - len, len, 0, start + 3 * MIN));
    storage::save(path, &history, &laptop).expect("save the laptop's History");

    // Later, on the phone, offline: a new Session that edits a word.
    let mut phone = Doc::new(DeviceId(0x5e6f7a8b));
    phone.apply_all(&history);
    let mut offline = vec![phone.set_device_name_local("Phone", start + 90 * MIN)];
    let at = phone.text().find("stopped").expect("the word is there");
    offline.push(phone.delete_local(at, 7, start + 91 * MIN));
    offline.push(phone.insert_local(at, "eased", start + 92 * MIN));

    // The phone's file comes back as a Conflict copy and is merged in.
    let conflict = Path::new("sample.sync-conflict.db");
    if conflict.exists() {
        std::fs::remove_file(conflict).expect("remove the old conflict copy");
    }
    let mut phone_history = history.clone();
    phone_history.extend(offline);
    storage::save(conflict, &phone_history, &phone).expect("save the phone's copy");
    storage::merge(path, conflict).expect("merge the conflict copy");
    std::fs::remove_file(conflict).expect("conflict copies are deleted after merging");

    let (_, doc) = storage::load(path, DeviceId(0x1a2b3c4d)).expect("load");
    println!("Wrote {} with this text:\n\n{}", path.display(), doc.text());
}
