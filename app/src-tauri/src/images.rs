//! Images in the app (ADR 0021): their names, the pictures a compile can use, and what stands
//! in for one that's missing. Where they're kept is the engine's (`wrtier::images`).

use std::collections::BTreeMap;
use std::sync::Arc;

use typst::foundations::Bytes;
pub use wrtier::images::{Kind, ImageValue, INCLUDED};

/// One picture a compile can use, by the path the text names it by.
#[derive(Clone)]
pub struct File {
    /// The Image's name as the Author gave it.
    pub name: String,
    pub hash: String,
    pub kind: Kind,
    pub bytes: Bytes,
    /// Not the picture asked for (that one was deleted): the one on hand, shown with a notice.
    pub stand_in: bool,
}

/// The pictures one compile can use: the Work's own Images by name (`map.png`), and those of
/// the Works it includes under `INCLUDED` (`.included/<key>/map.png`), all in lowercase, as
/// names are the same whatever their capitals.
#[derive(Clone, Default)]
pub struct Files(Arc<BTreeMap<String, File>>);

impl Files {
    pub fn new(files: BTreeMap<String, File>) -> Files {
        Files(Arc::new(files))
    }

    /// The picture a path in the text names (`map.png`, `/.included/<key>/map.png`), if any.
    pub fn get(&self, path: &str) -> Option<&File> {
        self.0.get(&path.trim_start_matches('/').to_lowercase())
    }

    /// Every path, as the text would name it.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// The Work's own Images (not included Works'), by name as the Author gave it.
    pub fn own(&self) -> impl Iterator<Item = &File> {
        self.0.iter().filter(|(path, _)| !path.contains('/')).map(|(_, f)| f)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl PartialEq for Files {
    fn eq(&self, other: &Files) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || self.0.len() == other.0.len() && self.0.iter().zip(other.0.iter()).all(|((a, x), (b, y))| a == b && x.hash == y.hash && x.stand_in == y.stand_in)
    }
}

impl std::fmt::Debug for Files {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

/// What a name the text uses but no Image has shows instead: a grey box with a cross, in the
/// kind its name says (Typst reads a picture by its name's extension).
pub fn placeholder(name: &str) -> Bytes {
    let bytes: &'static [u8] = match Kind::of_name(name) {
        Some(Kind::Jpeg) => include_bytes!("../placeholders/missing.jpg"),
        Some(Kind::Gif) => include_bytes!("../placeholders/missing.gif"),
        Some(Kind::Webp) => include_bytes!("../placeholders/missing.webp"),
        Some(Kind::Svg) => include_bytes!("../placeholders/missing.svg"),
        Some(Kind::Pdf) => include_bytes!("../placeholders/missing.pdf"),
        Some(Kind::Png) | None => include_bytes!("../placeholders/missing.png"),
    };
    Bytes::new(bytes)
}

/// Windows' reserved file names, which no Image can have: its copy beside the Text file couldn't
/// be written.
const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
                            "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];

/// Can an Image of `kind` be called `name`? It must work as a file name on every Device (its copy
/// is written beside the Text file), end in its kind's extension, and never be the Past pictures
/// folder.
pub fn check_name(name: &str, kind: Kind) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("An Image needs a name.".into());
    }
    if name.chars().count() > 200 {
        return Err("An Image's name can be at most 200 characters.".into());
    }
    if let Some(c) = name.chars().find(|c| r#"<>:"\|?*/"#.contains(*c) || c.is_control()) {
        return Err(format!("An Image's name can't contain {c:?}."));
    }
    if name.starts_with('.') || name.starts_with(' ') || name.ends_with(' ') || name.ends_with('.') {
        return Err("An Image's name can't start or end with a dot or a space.".into());
    }
    if name.eq_ignore_ascii_case(wrtier::images::PAST) {
        return Err(format!("\"{}\" is the Past pictures folder.", wrtier::images::PAST));
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if RESERVED.contains(&stem.trim().to_string().as_str()) {
        return Err(format!("{name:?} is reserved by Windows."));
    }
    if Kind::of_name(name) != Some(kind) {
        return Err(format!("This picture is a {}, so its name must end in .{}.", kind_label(kind), kind.extension()));
    }
    Ok(())
}

/// A kind as the Author reads it.
pub fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Png => "PNG",
        Kind::Jpeg => "JPEG",
        Kind::Gif => "GIF",
        Kind::Webp => "WebP",
        Kind::Svg => "SVG",
        Kind::Pdf => "PDF",
    }
}

/// A file's name made into one an Image of `kind` can have (see `check_name`): characters it
/// can't have dropped, a reserved name given a suffix, and its kind's extension (the bytes say
/// what it is, whatever the file was called).
pub fn clean_name(file_name: &str, kind: Kind) -> String {
    let stem = match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file_name,
    };
    let mut stem: String = stem.chars().filter(|c| !r#"<>:"\|?*/"#.contains(*c) && !c.is_control()).collect();
    stem = stem.trim().trim_matches('.').trim().chars().take(180).collect();
    if stem.is_empty() || stem.eq_ignore_ascii_case(wrtier::images::PAST) {
        stem = "image".into();
    }
    if RESERVED.contains(&stem.to_ascii_uppercase().as_str()) {
        stem.push_str(" image");
    }
    // Keep the extension the file had, if it's one of its kind's ("photo.jpeg" stays).
    let extension = file_name.rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .filter(|e| kind.extensions().contains(&e.as_str()))
        .unwrap_or_else(|| kind.extension().to_string());
    format!("{stem}.{extension}")
}

/// `name` with " 2", " 3"… before its extension until `taken` says no: "map.png" → "map 2.png".
pub fn free_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..).map(|n| if extension.is_empty() { format!("{stem} {n}") } else { format!("{stem} {n}.{extension}") })
        .find(|candidate| !taken(candidate))
        .expect("some number is free")
}

// ---- The copies beside the Text file ----

use std::path::{Path, PathBuf};
use wrtier::change::{image_record, Change, IMAGE_FIELD};
use wrtier::doc::Doc;
use wrtier::images::{images_dir, images_of, image_of, history_of};
use wrtier::storage;

/// An included Work's Image whose copy goes in the including Work's folder, in a folder of that
/// Work's own: `path` is `.included/<key>/<name>` (see `vault::IncludedImage`).
pub struct IncludedCopy {
    pub path: String,
    pub value: ImageValue,
    pub db: PathBuf,
}

/// Files in the folder that aren't anyone's: being written by the sync tool or an editor, or
/// hidden.
fn passing(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('~') || name.ends_with('~') || name.ends_with(".tmp") || name.ends_with(".writing")
        || name.contains(".sync-conflict-") || name.eq_ignore_ascii_case("desktop.ini") || name.eq_ignore_ascii_case("thumbs.db")
}

/// What taking in the copies found.
#[derive(Debug, Default)]
pub struct TakenIn {
    /// The Changes recording it (already saved).
    pub changes: Vec<Change>,
    /// The Images it changed or added, by name, for the line saying so.
    pub names: Vec<String>,
}

/// Takes in what was done to the copies of a Work's Images outside the app (ADR 0021): a copy
/// changed there replaces its Image; a new picture put there is a new Image, named after its
/// file. A copy deleted is just written again later, and never deletes its Image. A copy of a
/// picture an Image of that name once had, but no Image has under it now (left from a rename or
/// deletion, here or on another Device), is removed, if the picture is still kept elsewhere. Nothing in a folder
/// (`past/`, included Works') is ever taken in.
pub fn take_in_copies(db: &Path, doc: &mut Doc, now_ms: u64) -> Result<TakenIn, String> {
    let dir = images_dir(db);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(TakenIn::default()) };
    let mut files: Vec<(String, PathBuf)> = entries.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(name, _)| !passing(name))
        .collect();
    files.sort();
    let mut taken = TakenIn::default();
    for (file_name, path) in files {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Some(kind) = Kind::of_bytes(&bytes) else { continue }; // not a picture
        let hash = wrtier::images::hash_of(&bytes);
        let had = image_of(doc, &file_name);
        if let Some(had) = &had {
            if had.hash == hash || history_of(doc, &had.name).iter().any(|(_, v)| v.hash == hash) {
                continue; // the app's own copy (or an older one of it, written again soon)
            }
        } else if had_once(doc, &file_name, &hash) {
            // Left from an Image renamed or deleted: gone, if the picture is kept elsewhere.
            if wrtier::images::bytes_of(db, &hash).is_some() {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        // A picture another Image has already (taken in under a better name last time).
        if images_of(doc).iter().any(|v| v.hash == hash) {
            continue;
        }
        // Changed or added outside the app.
        let name = match &had {
            Some(had) if had.kind == kind => had.name.clone(),
            _ => {
                let wanted = if check_name(&file_name, kind).is_ok() && had.is_none() { file_name.clone() } else { clean_name(&file_name, kind) };
                free_name(&wanted, |n| image_of(doc, n).is_some())
            }
        };
        let value = ImageValue { name: name.clone(), kind, size: bytes.len() as u64, hash: hash.clone() };
        storage::put_picture(db, &hash, &bytes).map_err(|e| e.to_string())?;
        taken.changes.push(doc.set_field_local(&image_record(&name), IMAGE_FIELD, &value.to_value(), now_ms));
        taken.names.push(name);
    }
    if !taken.changes.is_empty() {
        storage::save(db, &taken.changes, doc).map_err(|e| e.to_string())?;
    }
    Ok(taken)
}

/// Writes the copies of a Work's current Images beside its Text file (and of its Inclusions',
/// in a folder per included Work), so the Text file compiles on its own. A file is only written
/// over if it holds a picture the app wrote (anything else is an edit made outside, taken in
/// first); a copy no Image needs any more is removed. Nothing is written for a Work without
/// Images.
pub fn write_copies(db: &Path, doc: &Doc, included: &[IncludedCopy]) -> Result<(), String> {
    let dir = images_dir(db);
    let own = images_of(doc);
    if own.is_empty() && included.is_empty() && !dir.exists() {
        return Ok(());
    }
    // A copy the app wrote: a picture an Image of its name had (else it was changed outside).
    let ours = |path: &Path, name: &str| -> bool {
        match std::fs::read(path) {
            Ok(bytes) => had_once(doc, name, &wrtier::images::hash_of(&bytes)),
            Err(_) => true, // not there
        }
    };
    let write = |path: &Path, bytes: &[u8]| -> Result<(), String> {
        if std::fs::read(path).is_ok_and(|old| old == bytes) {
            return Ok(()); // already so: nothing for the sync tool to send
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let partial = path.with_file_name(format!(".{}.writing", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        std::fs::write(&partial, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&partial, path).map_err(|e| e.to_string())
    };
    for value in &own {
        let path = dir.join(&value.name);
        if !ours(&path, &value.name) {
            continue; // changed outside: taken in first
        }
        if let Some(picture) = wrtier::images::picture_for(db, doc, value) {
            write(&path, &picture.bytes)?;
        }
    }
    // Copies no Image needs any more.
    let wanted: std::collections::BTreeSet<String> = own.iter().map(|v| v.name.to_lowercase()).collect();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_file = entry.file_type().is_ok_and(|t| t.is_file());
            if is_file && !passing(&name) && !wanted.contains(&name.to_lowercase()) {
                if let Ok(bytes) = std::fs::read(entry.path()) {
                    let hash = wrtier::images::hash_of(&bytes);
                    if had_once(doc, &name, &hash) && wrtier::images::bytes_of(db, &hash).is_some() {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
    // Included Works' copies: generated whole, so whatever isn't wanted goes.
    let mut keys: std::collections::BTreeMap<String, Vec<(&str, &IncludedCopy)>> = std::collections::BTreeMap::new();
    for copy in included {
        let rest = copy.path.strip_prefix(&INCLUDED[1..]).unwrap_or(&copy.path);
        if let Some((key, _)) = rest.split_once('/') {
            keys.entry(key.to_string()).or_default().push((key, copy));
        }
    }
    for (key, copies) in &keys {
        for (_, copy) in copies {
            if let Some(bytes) = wrtier::images::bytes_of(&copy.db, &copy.value.hash) {
                write(&dir.join(key).join(&copy.value.name), &bytes)?;
            }
        }
        let names: std::collections::BTreeSet<String> = copies.iter().map(|(_, c)| c.value.name.to_lowercase()).collect();
        if let Ok(entries) = std::fs::read_dir(dir.join(key)) {
            for entry in entries.flatten() {
                if !names.contains(&entry.file_name().to_string_lossy().to_lowercase()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|t| t.is_dir()) && name != wrtier::images::PAST && !keys.contains_key(&name) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(())
}

/// Did an Image called `name` ever have the picture `hash`? Then a copy of it by that name is
/// one the app wrote.
fn had_once(doc: &Doc, name: &str, hash: &str) -> bool {
    history_of(doc, name).iter().any(|(_, v)| v.hash == hash)
}

/// What the Images folder holds, to notice a copy changed outside the app: each file's name,
/// size and time (not in its folders).
pub fn folder_stamp(db: &Path) -> Vec<(String, u64, Option<std::time::SystemTime>)> {
    let Ok(entries) = std::fs::read_dir(images_dir(db)) else { return Vec::new() };
    let mut found: Vec<_> = entries.flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| (e.file_name().to_string_lossy().into_owned(), meta.len(), meta.modified().ok()))
        })
        .filter(|(name, _, _)| !passing(name))
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_must_work_as_files_everywhere_and_say_their_kind() {
        assert!(check_name("map.png", Kind::Png).is_ok());
        assert!(check_name("The old map.JPEG", Kind::Jpeg).is_ok());
        assert!(check_name("map.png", Kind::Jpeg).is_err(), "a JPEG called .png");
        assert!(check_name("a:b.png", Kind::Png).is_err());
        assert!(check_name("maps/old.png", Kind::Png).is_err());
        assert!(check_name("CON.png", Kind::Png).is_err());
        assert!(check_name(".hidden.png", Kind::Png).is_err());
        assert!(check_name("past", Kind::Png).is_err());
    }

    #[test]
    fn a_files_name_is_cleaned_into_one_an_image_can_have() {
        assert_eq!(clean_name("map.png", Kind::Png), "map.png");
        assert_eq!(clean_name("photo.jpeg", Kind::Jpeg), "photo.jpeg");
        assert_eq!(clean_name("what?.png", Kind::Png), "what.png");
        assert_eq!(clean_name("IMG_1.png", Kind::Jpeg), "IMG_1.jpg", "the bytes say what it is");
        assert_eq!(clean_name("nul.gif", Kind::Gif), "nul image.gif");
        assert_eq!(clean_name("...", Kind::Svg), "image.svg");
        for name in ["map.png", "what?.png", "nul.gif", "..."] {
            let kind = Kind::of_name(name).unwrap_or(Kind::Svg);
            assert!(check_name(&clean_name(name, kind), kind).is_ok(), "{name}");
        }
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let taken = ["map.png", "map 2.png"];
        assert_eq!(free_name("map.png", |n| taken.contains(&n)), "map 3.png");
        assert_eq!(free_name("river.png", |n| taken.contains(&n)), "river.png");
    }

    #[test]
    fn every_placeholder_is_the_kind_its_name_says() {
        for kind in Kind::ALL {
            let name = format!("x.{}", kind.extension());
            assert_eq!(Kind::of_bytes(&placeholder(&name)[..]), Some(kind), "{name}");
        }
    }
}
