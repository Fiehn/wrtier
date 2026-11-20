//! A Work's Images (ADR 0021): pictures stored under a name, shown where the text writes
//! `#image("map.png")`.
//!
//! Each Image is a record, `image/<name in lowercase>`, whose one field says which picture it
//! has: its kind, size and hash, never the bytes. The History keeps every value it ever had.
//! The bytes of each Image's current picture are in the History file (`storage::put_picture`);
//! a picture an Image no longer has is a Past picture, kept as a file in
//! `<Work>.images/past/` (the only copy, which the Author may delete). `settle` keeps it that way.
//!
//! A picture that's needed but gone (a Past picture deleted, or one an undo brings back) is
//! stood in for by the picture the Image has on hand, with a notice: see `picture_for`.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::change::{image_record, IMAGE_FIELD, IMAGE_PREFIX};
use crate::doc::Doc;
use crate::storage;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The kinds of picture an Image can hold: what Typst reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
    Pdf,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Png, Kind::Jpeg, Kind::Gif, Kind::Webp, Kind::Svg, Kind::Pdf];

    /// The extension a name of this kind ends in (the first, for a kind with two).
    pub fn extension(self) -> &'static str {
        match self {
            Kind::Png => "png",
            Kind::Jpeg => "jpg",
            Kind::Gif => "gif",
            Kind::Webp => "webp",
            Kind::Svg => "svg",
            Kind::Pdf => "pdf",
        }
    }

    /// Every extension a name of this kind may end in.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Kind::Jpeg => &["jpg", "jpeg"],
            Kind::Png => &["png"],
            Kind::Gif => &["gif"],
            Kind::Webp => &["webp"],
            Kind::Svg => &["svg"],
            Kind::Pdf => &["pdf"],
        }
    }

    /// The kind a name says it is, by its extension.
    pub fn of_name(name: &str) -> Option<Kind> {
        let (_, extension) = name.rsplit_once('.')?;
        let extension = extension.to_lowercase();
        Kind::ALL.into_iter().find(|k| k.extensions().contains(&extension.as_str()))
    }

    /// The kind the bytes are, by what they start with: never by a file's name, which may lie.
    pub fn of_bytes(bytes: &[u8]) -> Option<Kind> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Kind::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Kind::Jpeg)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(Kind::Gif)
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some(Kind::Webp)
        } else if bytes.starts_with(b"%PDF-") {
            Some(Kind::Pdf)
        } else if looks_like_svg(bytes) {
            Some(Kind::Svg)
        } else {
            None
        }
    }

    fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.extension() == s)
    }
}

/// An SVG file: text whose first element is `<svg` (after an XML declaration, comments or a
/// doctype). Gzipped SVG isn't read.
fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let Ok(text) = std::str::from_utf8(head).or_else(|e| std::str::from_utf8(&head[..e.valid_up_to()])) else { return false };
    let mut rest = text.trim_start_matches('\u{feff}').trim_start();
    loop {
        if rest.starts_with("<svg") {
            return true;
        }
        let skip = if rest.starts_with("<?") {
            rest.find("?>").map(|i| i + 2)
        } else if rest.starts_with("<!--") {
            rest.find("-->").map(|i| i + 3)
        } else if rest.starts_with("<!") {
            rest.find('>').map(|i| i + 1)
        } else {
            None
        };
        match skip {
            Some(n) => rest = rest[n..].trim_start(),
            None => return false,
        }
    }
}

/// Which picture an Image has, as its record holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageValue {
    /// Its name as the Author gave it (the record's is in lowercase).
    pub name: String,
    pub kind: Kind,
    /// The picture's size, in bytes.
    pub size: u64,
    /// SHA-256 of the picture, as hex.
    pub hash: String,
}

impl ImageValue {
    /// As the record's field holds it: "png 10423 <hash> map.png" (the name last, as it may
    /// hold spaces).
    pub fn to_value(&self) -> String {
        format!("{} {} {} {}", self.kind.extension(), self.size, self.hash, self.name)
    }

    /// From `to_value`; None for an empty value (a deleted Image) or anything else.
    pub fn parse(value: &str) -> Option<ImageValue> {
        let mut parts = value.splitn(4, ' ');
        let kind = Kind::parse(parts.next()?)?;
        let size = parts.next()?.parse().ok()?;
        let hash = parts.next()?.to_string();
        let name = parts.next()?.to_string();
        (hash.len() == 64 && !name.is_empty()).then_some(ImageValue { name, kind, size, hash })
    }

    /// The value for a picture's bytes under `name`. None if they aren't a kind of picture
    /// Typst reads.
    pub fn of(name: &str, bytes: &[u8]) -> Option<ImageValue> {
        Some(ImageValue { name: name.to_string(), kind: Kind::of_bytes(bytes)?, size: bytes.len() as u64, hash: hash_of(bytes) })
    }
}

/// SHA-256 of a picture, as hex: what it's known by.
pub fn hash_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let mut hex = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// The Work's Images (not deleted), sorted by name, capitals aside.
pub fn images_of(doc: &Doc) -> Vec<ImageValue> {
    let mut found: Vec<ImageValue> = doc.records(IMAGE_PREFIX).into_iter()
        .filter_map(|record| ImageValue::parse(doc.field(&record, IMAGE_FIELD)?))
        .collect();
    found.sort_by_key(|v| v.name.to_lowercase());
    found
}

/// The Image called `name` (capitals aside), if the Work has one.
pub fn image_of(doc: &Doc, name: &str) -> Option<ImageValue> {
    doc.field(&image_record(name), IMAGE_FIELD).and_then(ImageValue::parse)
}

/// Every picture the Image called `name` was ever given, in effect or not, oldest first, with
/// when (ms since 1970). An Image deleted and added again has both lives here.
pub fn history_of(doc: &Doc, name: &str) -> Vec<(u64, ImageValue)> {
    doc.field_history(&image_record(name), IMAGE_FIELD).into_iter()
        .filter_map(|(_, hlc, value)| Some((hlc.0, ImageValue::parse(value)?)))
        .collect()
}

/// Every picture any Image of the Work was ever given: its hash, the latest value naming it
/// (for a Past picture's name) and when it was first given (ms since 1970).
fn every_picture(doc: &Doc) -> BTreeMap<String, (ImageValue, u64)> {
    let mut found: BTreeMap<String, (ImageValue, u64)> = BTreeMap::new();
    for record in doc.records_ever(IMAGE_PREFIX) {
        for (_, hlc, value) in doc.field_history(&record, IMAGE_FIELD) {
            let Some(value) = ImageValue::parse(value) else { continue };
            found.entry(value.hash.clone())
                .and_modify(|(latest, first)| {
                    *latest = value.clone();
                    *first = (*first).min(hlc.0);
                })
                .or_insert((value, hlc.0));
        }
    }
    found
}

/// Every picture any Image of the Work was ever given, by hash: what the app itself has
/// written beside the Text file (a copy holding anything else came from outside).
pub fn known_pictures(doc: &Doc) -> BTreeSet<String> {
    every_picture(doc).into_keys().collect()
}

/// The folder beside a Work's History holding its Images: `<Work>.images`. It has copies of the
/// current pictures, for the Text file (written again whenever needed), and `past/`.
pub fn images_dir(db: &Path) -> PathBuf {
    let stem = db.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    db.with_file_name(format!("{stem}.images"))
}

/// The folder of Past pictures inside `images_dir`. Never a name an Image can have.
pub const PAST: &str = "past";

pub fn past_dir(db: &Path) -> PathBuf {
    images_dir(db).join(PAST)
}

/// How long a hash is in a Past picture's file name: enough to tell any two apart.
const HASH_IN_NAME: usize = 16;

/// A Past picture's file name: the Image's name, when the picture was first given (in UTC), and
/// the start of its hash, which is how it's found again: "map 2026-11-20 ab12cd34ef56ab78.png".
/// The same on every Device, so two that set one aside write the same file.
pub fn past_file_name(value: &ImageValue, given_ms: u64) -> String {
    let stem = match value.name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => value.name.as_str(),
    };
    let (year, month, day) = utc_date(given_ms);
    format!("{stem} {year:04}-{month:02}-{day:02} {}.{}", &value.hash[..HASH_IN_NAME], value.kind.extension())
}

/// A time (ms since 1970) as a date in UTC (Howard Hinnant's algorithm).
fn utc_date(ms: u64) -> (i64, u32, u32) {
    let days = ((ms / 1000) as i64).div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// A Past picture of the Work, by hash: its file and bytes, if it's in the Past pictures folder
/// and really is that picture.
pub fn find_past(db: &Path, hash: &str) -> Option<(PathBuf, Vec<u8>)> {
    let short = hash.get(..HASH_IN_NAME)?;
    let entries = std::fs::read_dir(past_dir(db)).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((stem, _)) = name.rsplit_once('.') else { continue };
        if !stem.ends_with(&format!(" {short}")) {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else { continue };
        if hash_of(&bytes) == hash {
            return Some((entry.path(), bytes));
        }
    }
    None
}

/// A picture's bytes: from the History file, else from the Past pictures folder.
pub fn bytes_of(db: &Path, hash: &str) -> Option<Vec<u8>> {
    if let Ok(Some(bytes)) = storage::picture(db, hash) {
        return Some(bytes);
    }
    find_past(db, hash).map(|(_, bytes)| bytes)
}

/// A picture to show for an Image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub hash: String,
    pub kind: Kind,
    pub bytes: Vec<u8>,
    /// It isn't the picture asked for, which is gone (a deleted Past picture), but the one the
    /// Image has on hand: say so wherever it's shown.
    pub stand_in: bool,
}

/// The picture to show for `value` (an Image as it is now, or as it was at a Milestone): its
/// own, from the History file or the Past pictures; or, if that's gone, the one on hand for an
/// Image of that name in `current` (the Work as it is now), marked as a stand-in. None if
/// there's neither.
pub fn picture_for(db: &Path, current: &Doc, value: &ImageValue) -> Option<Picture> {
    if let Some(bytes) = bytes_of(db, &value.hash) {
        return Some(Picture { hash: value.hash.clone(), kind: value.kind, bytes, stand_in: false });
    }
    let held: BTreeSet<String> = storage::picture_hashes(db).unwrap_or_default().into_iter().collect();
    history_of(current, &value.name).into_iter().rev()
        .find(|(_, v)| held.contains(&v.hash))
        .and_then(|(_, v)| {
            let bytes = storage::picture(db, &v.hash).ok()??;
            Some(Picture { hash: v.hash, kind: v.kind, bytes, stand_in: true })
        })
}

/// What `settle` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Settled {
    /// Pictures moved out to the Past pictures folder.
    pub set_aside: usize,
    /// Pictures brought back from it (an undo, a Revert or a Milestone made them current again).
    pub brought_back: usize,
}

/// Keeps the History file holding exactly each Image's current picture (ADR 0021): a picture
/// needed now that's in the Past pictures folder comes back in; one no Image has now leaves for
/// it, written there (to disk) before it's dropped, so its only copy is never lost. An Image
/// whose picture is gone keeps the picture it had on hand, as its stand-in.
pub fn settle(db: &Path, doc: &Doc) -> Result<Settled> {
    let mut settled = Settled::default();
    let mut held: BTreeSet<String> = storage::picture_hashes(db)?.into_iter().collect();
    let current = images_of(doc);
    for value in &current {
        if held.contains(&value.hash) {
            continue;
        }
        if let Some((path, bytes)) = find_past(db, &value.hash) {
            storage::put_picture(db, &value.hash, &bytes)?;
            held.insert(value.hash.clone());
            let _ = std::fs::remove_file(path); // the History file has it now
            settled.brought_back += 1;
        }
    }
    let mut keep: BTreeSet<String> = current.iter().map(|v| v.hash.clone()).filter(|h| held.contains(h)).collect();
    for value in current.iter().filter(|v| !held.contains(&v.hash)) {
        let on_hand = history_of(doc, &value.name).into_iter().rev().find(|(_, v)| held.contains(&v.hash));
        if let Some((_, v)) = on_hand {
            keep.insert(v.hash);
        }
    }
    let leaving: Vec<String> = held.difference(&keep).cloned().collect();
    if leaving.is_empty() {
        return Ok(settled);
    }
    let known = every_picture(doc);
    let past = past_dir(db);
    let mut dropped = Vec::new();
    for hash in leaving {
        if find_past(db, &hash).is_none() {
            let Some(bytes) = storage::picture(db, &hash)? else { continue };
            let (value, given) = match known.get(&hash) {
                Some((value, given)) => (value.clone(), *given),
                // Only in the file (its Change hasn't arrived, or was never saved): kept as it is.
                None => continue,
            };
            std::fs::create_dir_all(&past)?;
            let file = past.join(past_file_name(&value, given));
            let partial = past.join(format!(".{}.writing", &hash[..HASH_IN_NAME]));
            {
                use std::io::Write;
                let mut out = std::fs::File::create(&partial)?;
                out.write_all(&bytes)?;
                out.sync_all()?; // on the disk before the History file lets go of it
            }
            std::fs::rename(&partial, &file)?;
        }
        dropped.push(hash);
    }
    settled.set_aside = dropped.len();
    storage::drop_pictures(db, &dropped)?;
    Ok(settled)
}

/// Where `image("…")` calls in some Typst name their picture: the byte range of each string's
/// contents (as written, escapes and all), in order. A comment or string that happens to
/// hold such a call counts too; it's harmless.
pub fn named_in(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("image(") {
        let at = from + i;
        from = at + "image(".len();
        // A whole name: `image(` or `std.image(`, not `my-image(`.
        let before = at.checked_sub(1).map(|b| bytes[b]);
        if before.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            continue;
        }
        let mut k = from;
        while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t' || bytes[k] == b'\n' || bytes[k] == b'\r') {
            k += 1;
        }
        if bytes.get(k) != Some(&b'"') {
            continue;
        }
        let start = k + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'"' && bytes[end] != b'\n' {
            end += if bytes[end] == b'\\' { 2 } else { 1 };
        }
        if bytes.get(end) == Some(&b'"') {
            found.push(start..end);
            from = end + 1;
        }
    }
    found
}

/// A Typst string's contents for `s` (escaped).
fn escaped(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Where an included Work's Images are, as the compiled text names them: an Inclusion's text
/// written out names `image("/.included/<key>/map.png")`, so its names never meet the including
/// Work's. `key` is `included_key` of the included Work's ID.
pub const INCLUDED: &str = "/.included/";

/// A Work ID as a folder name (IDs may hold `:` and `@`).
pub fn included_key(work: &str) -> String {
    work.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// The Typst with every Image the Work's own text names (a name with no `/`) found in `folder`
/// instead, and every included Work's (`INCLUDED`) in a folder of its own inside it: how the
/// Text file reaches the copies beside it.
pub fn into_folder(text: &str, folder: &str) -> String {
    let prefix = format!("{}/", escaped(folder));
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for range in named_in(text) {
        let name = &text[range.clone()];
        if name.is_empty() {
            continue;
        }
        if name.starts_with(INCLUDED) {
            out.push_str(&text[last..range.start]);
            out.push_str(&prefix);
            last = range.start + INCLUDED.len();
        } else if !name.contains('/') {
            out.push_str(&text[last..range.start]);
            out.push_str(&prefix);
            last = range.start;
        }
    }
    out.push_str(&text[last..]);
    out
}

/// `into_folder` undone: names in `folder` (and no deeper) go back to being the Work's own.
pub fn out_of_folder(text: &str, folder: &str) -> String {
    let prefix = format!("{}/", escaped(folder));
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for range in named_in(text) {
        let name = &text[range.clone()];
        let Some(rest) = name.strip_prefix(&prefix) else { continue };
        if rest.contains('/') || rest.is_empty() {
            continue;
        }
        out.push_str(&text[last..range.start]);
        last = range.start + prefix.len();
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::DeviceId;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-one";
    const PNG2: &[u8] = b"\x89PNG\r\n\x1a\n-two";
    const PNG3: &[u8] = b"\x89PNG\r\n\x1a\n-three";

    #[test]
    fn kinds_are_read_from_the_bytes_and_names() {
        assert_eq!(Kind::of_bytes(PNG), Some(Kind::Png));
        assert_eq!(Kind::of_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Kind::Jpeg));
        assert_eq!(Kind::of_bytes(b"GIF89a.."), Some(Kind::Gif));
        assert_eq!(Kind::of_bytes(b"RIFF\0\0\0\0WEBPVP8 "), Some(Kind::Webp));
        assert_eq!(Kind::of_bytes(b"%PDF-1.7"), Some(Kind::Pdf));
        assert_eq!(Kind::of_bytes(b"<?xml version=\"1.0\"?>\n<!-- made by hand -->\n<!DOCTYPE svg>\n<svg xmlns=\"\"/>"), Some(Kind::Svg));
        assert_eq!(Kind::of_bytes(b"<html><svg/></html>"), None);
        assert_eq!(Kind::of_bytes(b"hello"), None);
        assert_eq!(Kind::of_name("Map.JPEG"), Some(Kind::Jpeg));
        assert_eq!(Kind::of_name("map"), None);
    }

    #[test]
    fn a_value_reads_back_and_a_name_may_hold_spaces() {
        let value = ImageValue::of("The old map.png", PNG).unwrap();
        assert_eq!(ImageValue::parse(&value.to_value()), Some(value));
        assert_eq!(ImageValue::parse(""), None, "deleted");
    }

    #[test]
    fn past_pictures_are_named_by_image_date_and_hash() {
        let value = ImageValue::of("map.png", PNG).unwrap();
        // 2026-11-20 12:00 UTC
        let name = past_file_name(&value, 1_795_176_000_000);
        assert_eq!(name, format!("map 2026-11-20 {}.png", &value.hash[..16]));
    }

    #[test]
    fn names_are_found_in_image_calls_only() {
        let text = r#"#image("map.png", width: 80%) #my-image("no.png") #figure(std.image( "a\"b.svg"))"#;
        let names: Vec<&str> = named_in(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(names, vec!["map.png", r#"a\"b.svg"#]);
    }

    #[test]
    fn the_text_file_reaches_the_copies_and_reads_back_unchanged() {
        let text = "#image(\"map.png\")\n#image(\"/.included/w1/x.png\")\nimage text";
        let filed = into_folder(text, "The \"Long\" Night.images");
        assert_eq!(filed, "#image(\"The \\\"Long\\\" Night.images/map.png\")\n#image(\"The \\\"Long\\\" Night.images/w1/x.png\")\nimage text");
        // An Inclusion's block is dropped when the file is read back: only the Work's own names
        // need to come back.
        let own = "#image(\"map.png\")\nimage text";
        assert_eq!(out_of_folder(&into_folder(own, "The \"Long\" Night.images"), "The \"Long\" Night.images"), own);
    }

    #[test]
    fn a_text_file_names_the_copies_and_an_edit_beside_one_reads_back_as_the_works_text() {
        let dir = tempfile::tempdir().unwrap();
        let (db, typ) = (dir.path().join("Rain.db"), dir.path().join("Rain.typ"));
        let mut doc = Doc::new(DeviceId(1));
        let change = doc.insert_local(0, "#image(\"map.png\")\n", 1_000);
        storage::save(&db, &[change], &doc).unwrap();
        storage::write_text_file(&db, &typ, &doc, "", 1_000).unwrap();
        let written = std::fs::read_to_string(&typ).unwrap();
        assert!(written.starts_with("#image(\"Rain.images/map.png\")\n"), "{written}");
        assert_eq!(storage::read_text_file(&db, &typ, &mut doc, 2_000).unwrap(), storage::TextFileRead::Known);
        std::fs::write(&typ, written.replace("map.png\")", "map.png\")!")).unwrap();
        assert!(matches!(storage::read_text_file(&db, &typ, &mut doc, 3_000).unwrap(), storage::TextFileRead::Imported(_)));
        assert_eq!(doc.text(), "#image(\"map.png\")!\n");
    }

    fn set(doc: &mut Doc, db: &Path, name: &str, bytes: &[u8], now: u64) -> ImageValue {
        let value = ImageValue::of(name, bytes).unwrap();
        storage::put_picture(db, &value.hash, bytes).unwrap();
        let change = doc.set_field_local(&image_record(name), IMAGE_FIELD, &value.to_value(), now);
        storage::save(db, &[change], doc).unwrap();
        value
    }

    #[test]
    fn only_current_pictures_stay_in_the_history_file_and_old_ones_come_back_when_needed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("Rain.db");
        let mut doc = Doc::new(DeviceId(1));
        let one = set(&mut doc, &db, "map.png", PNG, 1_000);
        let two = set(&mut doc, &db, "Map.png", PNG2, 2_000);
        assert_eq!(images_of(&doc), vec![two.clone()], "one Image, whatever the capitals");
        let settled = settle(&db, &doc).unwrap();
        assert_eq!(settled.set_aside, 1);
        assert_eq!(storage::picture_hashes(&db).unwrap(), vec![two.hash.clone()]);
        let (file, bytes) = find_past(&db, &one.hash).expect("the old one is a Past picture");
        assert_eq!(bytes, PNG);
        assert!(file.starts_with(dir.path().join("Rain.images").join("past")));
        // Undoing the replacement: the old picture comes back in, the new one goes out.
        let latest = doc.field_history(&image_record("map.png"), IMAGE_FIELD).last().unwrap().0;
        let revert = doc.revert_changes_local(&[latest], false, 3_000).unwrap();
        storage::save(&db, &[revert], &doc).unwrap();
        assert_eq!(settle(&db, &doc).unwrap(), Settled { set_aside: 1, brought_back: 1 });
        assert_eq!(storage::picture_hashes(&db).unwrap(), vec![one.hash.clone()]);
        assert!(find_past(&db, &one.hash).is_none(), "it's in the History file now");
        assert!(find_past(&db, &two.hash).is_some());
        assert!(settle(&db, &doc).unwrap() == Settled::default(), "nothing more to do");
    }

    #[test]
    fn a_deleted_past_picture_is_stood_in_for_and_comes_back_when_its_file_does() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("Rain.db");
        let mut doc = Doc::new(DeviceId(1));
        let one = set(&mut doc, &db, "map.png", PNG, 1_000);
        set(&mut doc, &db, "map.png", PNG2, 2_000);
        let three = set(&mut doc, &db, "map.png", PNG3, 3_000);
        settle(&db, &doc).unwrap();
        // The Author deletes the first picture's file.
        let (file, _) = find_past(&db, &one.hash).unwrap();
        std::fs::remove_file(&file).unwrap();
        // A Milestone that had it shows the current one, marked.
        let shown = picture_for(&db, &doc, &one).unwrap();
        assert!(shown.stand_in);
        assert_eq!(shown.hash, three.hash);
        // Giving the Image it back: the History file keeps the picture on hand.
        let back = doc.set_field_local(&image_record("map.png"), IMAGE_FIELD, &one.to_value(), 4_000);
        storage::save(&db, &[back], &doc).unwrap();
        settle(&db, &doc).unwrap();
        assert_eq!(storage::picture_hashes(&db).unwrap(), vec![three.hash.clone()], "kept as the stand-in");
        let now = picture_for(&db, &doc, &image_of(&doc, "map.png").unwrap()).unwrap();
        assert!(now.stand_in);
        // Its file turns up again (another Device still had it): it's shown, and the stand-in leaves.
        std::fs::write(&file, PNG).unwrap();
        settle(&db, &doc).unwrap();
        assert_eq!(storage::picture_hashes(&db).unwrap(), vec![one.hash.clone()]);
        assert!(!picture_for(&db, &doc, &image_of(&doc, "map.png").unwrap()).unwrap().stand_in);
    }

    #[test]
    fn deleting_an_image_sets_its_picture_aside_and_merging_brings_both_devices_pictures() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("Rain.db");
        let copy = dir.path().join("Rain.sync-conflict.db");
        let mut doc = Doc::new(DeviceId(1));
        let one = set(&mut doc, &db, "map.png", PNG, 1_000);
        std::fs::copy(&db, &copy).unwrap();
        // The phone gives it another picture meanwhile, in its copy.
        let (_, mut phone) = storage::load(&copy, DeviceId(2)).unwrap();
        let two = set(&mut phone, &copy, "map.png", PNG2, 2_000);
        // The laptop deletes it.
        let gone = doc.set_field_local(&image_record("map.png"), IMAGE_FIELD, "", 1_500);
        storage::save(&db, &[gone], &doc).unwrap();
        settle(&db, &doc).unwrap();
        assert!(find_past(&db, &one.hash).is_some());
        assert!(storage::picture_hashes(&db).unwrap().is_empty());
        // The copy merges in: the phone's later picture wins, and its bytes came with it.
        storage::merge(&db, &copy).unwrap();
        let (_, merged) = storage::load(&db, DeviceId(1)).unwrap();
        assert_eq!(image_of(&merged, "map.png"), Some(two.clone()));
        assert_eq!(merged.merge_notices().len(), 1, "two Devices disagreed about it");
        settle(&db, &merged).unwrap();
        assert_eq!(storage::picture_hashes(&db).unwrap(), vec![two.hash]);
    }

    #[test]
    fn two_devices_giving_an_image_the_same_picture_is_no_disagreement() {
        let mut laptop = Doc::new(DeviceId(1));
        let mut phone = Doc::new(DeviceId(2));
        let value = ImageValue::of("ornament.svg", b"<svg/>").unwrap().to_value();
        let a = laptop.set_field_local(&image_record("ornament.svg"), IMAGE_FIELD, &value, 1_000);
        let b = phone.set_field_local(&image_record("ornament.svg"), IMAGE_FIELD, &value, 1_001);
        laptop.apply(&b);
        phone.apply(&a);
        assert!(laptop.merge_notices().is_empty());
        assert_eq!(images_of(&laptop), images_of(&phone));
    }
}
