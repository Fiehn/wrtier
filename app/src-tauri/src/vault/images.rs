//! A Work's Images, as the Vault sees them (ADR 0021): what the Images tab lists, the pictures
//! a compile can use, and a picture taken in from a file before it's given to an Image.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::Serialize;
use typst::foundations::Bytes;
use wrtier::doc::Doc;
use wrtier::images::{self as stored, ImageValue, Kind};
use wrtier::storage;
use crate::images::{clean_name, kind_label, File, Files, IncludedCopy};
use super::inclusions::Shown;
use super::{err, Vault};

/// Pictures read lately, by hash, so a compile doesn't read them from the History again; let
/// go once they add up to more than `LIMIT`.
#[derive(Default)]
pub struct PictureCache {
    held: HashMap<String, Bytes>,
    size: usize,
}

impl PictureCache {
    const LIMIT: usize = 256 * 1024 * 1024;

    fn get(&self, hash: &str) -> Option<Bytes> {
        self.held.get(hash).cloned()
    }

    fn keep(&mut self, hash: &str, bytes: Bytes) {
        if self.size + bytes.len() > Self::LIMIT {
            self.held.clear();
            self.size = 0;
        }
        self.size += bytes.len();
        self.held.insert(hash.to_string(), bytes);
    }
}

/// A picture taken in from a file and kept in the Work's History, ready for `Op::SetImage`.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct KeptPicture {
    pub hash: String,
    /// The name it'd have: its file's, made into one an Image can have.
    pub name: String,
    pub kind: &'static str,
    pub size: u64,
    /// An Image of the Work already has that name (whatever its capitals): its name. The
    /// Author chooses between replacing it and keeping both.
    pub clash: Option<String>,
    /// Keeping both would name it this.
    pub other_name: String,
    /// That Image already has exactly this picture: there's nothing to replace.
    pub same: bool,
}

/// One Image, as the Images tab lists it.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageRow {
    pub name: String,
    pub kind: &'static str,
    pub size: u64,
    pub hash: String,
    /// Where the text names it (UTF-16, where each name starts), for going there.
    pub uses: Vec<usize>,
    /// Its Past pictures still kept, and how much room they take.
    pub past: usize,
    pub past_size: u64,
    /// Those Past pictures, newest first.
    pub past_pictures: Vec<PastPicture>,
    /// Its picture is gone (a deleted Past picture brought back): the one on hand stands in.
    pub stand_in: bool,
}

/// A Past picture of an Image (ADR 0021), still kept in the Past pictures folder.
#[derive(Serialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PastPicture {
    pub hash: String,
    /// When the Image was first given it (ms since 1970).
    pub given_ms: u64,
    /// Its file's size, in bytes.
    pub size: u64,
}

/// The Images of one included Work, under its name.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IncludedImages {
    /// Its ID in the Vault, to open it.
    pub work: String,
    pub label: String,
    pub images: Vec<ImageRow>,
}

/// What the Images tab shows for the Work in front.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImagesView {
    pub own: Vec<ImageRow>,
    pub included: Vec<IncludedImages>,
    /// The Past pictures folder, to open.
    pub past_folder: String,
}

/// The copies of the included Works' Images an including Work's Text file needs (ADR 0021).
pub(super) fn included_copies(shown: &[Shown]) -> Vec<IncludedCopy> {
    let mut seen = BTreeSet::new();
    shown.iter().flat_map(|s| s.images.iter())
        .filter(|i| seen.insert(i.path.clone()))
        .map(|i| IncludedCopy { path: i.path.clone(), value: i.value.clone(), db: i.db.clone() })
        .collect()
}

/// The Past pictures still kept of the Image `name` (not its current picture), newest first.
fn past_of(db: &Path, doc: &Doc, name: &str, current: &str) -> Vec<PastPicture> {
    // Each picture it had, by the start of its hash (as its file is named): the whole hash, and
    // when it was first given.
    let mut had: std::collections::BTreeMap<String, (String, u64)> = std::collections::BTreeMap::new();
    for (ms, v) in stored::history_of(doc, name) {
        if v.hash != current {
            had.entry(v.hash[..16].to_string()).or_insert((v.hash, ms));
        }
    }
    let Ok(entries) = std::fs::read_dir(stored::past_dir(db)) else { return Vec::new() };
    let mut found: Vec<PastPicture> = Vec::new();
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some((stem, _)) = file.rsplit_once('.') else { continue };
        let Some((hash, given_ms)) = stem.rsplit_once(' ').and_then(|(_, short)| had.get(short)) else { continue };
        if found.iter().any(|p| p.hash == *hash) {
            continue; // the same picture twice (written by two Devices at once)
        }
        found.push(PastPicture { hash: hash.clone(), given_ms: *given_ms, size: entry.metadata().map(|m| m.len()).unwrap_or(0) });
    }
    found.sort_by(|a, b| b.given_ms.cmp(&a.given_ms).then(a.hash.cmp(&b.hash)));
    found
}

/// The rows for a Work's Images. `text`: its text, to find where it names them (None for an
/// included Work's, whose uses aren't in the Work in front's text).
fn rows(db: &Path, doc: &Doc, text: Option<&str>) -> Vec<ImageRow> {
    let held: BTreeSet<String> = storage::picture_hashes(db).unwrap_or_default().into_iter().collect();
    let named: Vec<(usize, String)> = match text {
        Some(text) => {
            let utf16 = crate::positions::utf16_at_bytes(text);
            stored::named_in(text).into_iter().map(|r| (utf16[r.start], text[r].to_lowercase())).collect()
        }
        None => Vec::new(),
    };
    stored::images_of(doc).into_iter().map(|v| {
        let past_pictures = past_of(db, doc, &v.name, &v.hash);
        let lower = v.name.to_lowercase();
        ImageRow {
            uses: named.iter().filter(|(_, n)| *n == lower).map(|(at, _)| *at).collect(),
            stand_in: !held.contains(&v.hash) && stored::find_past(db, &v.hash).is_none(),
            past: past_pictures.len(),
            past_size: past_pictures.iter().map(|p| p.size).sum(),
            past_pictures,
            kind: kind_label(v.kind), size: v.size, hash: v.hash, name: v.name,
        }
    }).collect()
}

impl Vault {
    /// Takes in a picture from a file (picked, or dragged onto the text) for the open Work `id`:
    /// kept in its History, ready to be given to an Image with `Op::SetImage`. Refused if it
    /// isn't a kind of picture the app can show.
    pub fn keep_picture(&mut self, id: &str, file_name: &str, bytes: &[u8]) -> Result<KeptPicture, String> {
        let kind = Kind::of_bytes(bytes)
            .ok_or("That file isn't a picture the app can show: it takes PNG, JPEG, GIF, WebP, SVG and PDF.")?;
        let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
        let value = ImageValue::of(&clean_name(file_name, kind), bytes).expect("a picture, as checked");
        let clash = stored::image_of(&work.doc, &value.name);
        let other_name = crate::images::free_name(&value.name, |n| stored::image_of(&work.doc, n).is_some());
        storage::put_picture(&work.db, &value.hash, bytes).map_err(err)?;
        Ok(KeptPicture {
            same: clash.as_ref().is_some_and(|c| c.hash == value.hash),
            clash: clash.map(|c| c.name),
            other_name,
            kind: kind_label(kind), size: value.size, hash: value.hash, name: value.name,
        })
    }

    /// The Images of the open Work `id`, and of the Works it includes, for the Images tab.
    pub fn images(&mut self, id: &str) -> Result<ImagesView, String> {
        let shown = self.shown_in(id, false)?;
        let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
        let own = rows(&work.db, &work.doc, Some(&work.text()));
        let past_folder = stored::past_dir(&work.db).display().to_string();
        let mut dbs: Vec<PathBuf> = Vec::new();
        for image in shown.iter().flat_map(|s| s.images.iter()) {
            if !dbs.contains(&image.db) {
                dbs.push(image.db.clone());
            }
        }
        let index = self.work_index();
        let mut included = Vec::new();
        for db in dbs {
            let Ok((_, doc)) = storage::load(&db, self.device) else { continue };
            let work = index.values().find(|l| l.db == db).map(|l| l.id.clone()).unwrap_or_default();
            let label = work.rsplit('/').next().unwrap_or(&work).to_string();
            let images = rows(&db, &doc, None);
            if !images.is_empty() {
                included.push(IncludedImages { work, label, images });
            }
        }
        Ok(ImagesView { own, included, past_folder })
    }

    /// Is `hash` a Past picture of one of the open Work's own Images: a picture one of them had,
    /// and none has now?
    fn is_past_picture(doc: &Doc, hash: &str) -> bool {
        let current = stored::images_of(doc);
        !current.iter().any(|v| v.hash == hash)
            && doc.records_ever(wrtier::change::IMAGE_PREFIX).iter()
                .flat_map(|record| doc.field_history(record, wrtier::change::IMAGE_FIELD))
                .any(|(_, _, value)| ImageValue::parse(value).is_some_and(|v| v.hash == hash))
    }

    /// One of the open Work's Past pictures, for its thumbnail in the Images tab.
    pub fn past_picture(&self, id: &str, hash: &str) -> Result<Vec<u8>, String> {
        let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
        if !Self::is_past_picture(&work.doc, hash) {
            return Err("That isn't a Past picture of this Work.".into());
        }
        stored::find_past(&work.db, hash).map(|(_, bytes)| bytes).ok_or_else(|| "That Past picture has been deleted.".into())
    }

    /// Deletes some of the open Work's Past pictures from its Past pictures folder, for good, on
    /// every Device (ADR 0021): a Milestone or undo that needs one then shows the picture on
    /// hand instead. Only ever Past pictures of its own Images; returns how many went.
    pub fn delete_past_pictures(&mut self, id: &str, hashes: &[String]) -> Result<usize, String> {
        let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
        let mut deleted = 0;
        for hash in hashes {
            if !Self::is_past_picture(&work.doc, hash) {
                continue;
            }
            while let Some((path, _)) = stored::find_past(&work.db, hash) {
                std::fs::remove_file(&path).map_err(|e| format!("Couldn't delete {}: {e}", path.display()))?;
                deleted += 1;
            }
        }
        Ok(deleted)
    }

    /// The open Work's Past pictures folder.
    pub fn past_pictures_folder(&self, id: &str) -> Result<PathBuf, String> {
        let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
        Ok(stored::past_dir(&work.db))
    }

    /// An Image's picture, for its thumbnail: of the open Work `id`, or of the Work `work` (by
    /// its ID in the Vault) that it includes. With its kind.
    pub fn image_picture(&mut self, id: &str, name: &str, work: Option<&str>) -> Result<(Vec<u8>, Kind), String> {
        let find = |db: &Path, doc: &Doc| -> Result<(Vec<u8>, Kind), String> {
            let value = stored::image_of(doc, name).ok_or_else(|| format!("There's no Image called \"{name}\"."))?;
            let picture = stored::picture_for(db, doc, &value).ok_or("Its picture isn't kept anywhere any more.")?;
            Ok((picture.bytes, picture.kind))
        };
        match work {
            None => {
                let open = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
                find(&open.db, &open.doc)
            }
            Some(other) => {
                let (db, _) = self.paths(other)?;
                let (_, doc) = storage::load(&db, self.device).map_err(err)?;
                find(&db, &doc)
            }
        }
    }
}

/// The pictures a compile of `doc` can use (the Work whose History is `db`, as it is now or at
/// a Milestone): its Images', and those of the Works `shown` includes, at the versions shown.
/// `current`: the Work as it is now, whose pictures stand in for any that are gone (`doc`
/// itself if None). A free function, so the caller can lend a Work's `Doc` and the cache at once.
pub(super) fn image_files(cache: &mut PictureCache, device: wrtier::ids::DeviceId, db: &Path, doc: &Doc, current: Option<&Doc>, shown: &[Shown]) -> Files {
    let mut files = BTreeMap::new();
    for value in stored::images_of(doc) {
        if let Some(file) = picture_file(cache, db, current.unwrap_or(doc), &value) {
            files.insert(value.name.to_lowercase(), file);
        }
    }
    let mut loaded: HashMap<PathBuf, Option<Doc>> = HashMap::new();
    for image in shown.iter().flat_map(|s| s.images.iter()) {
        if files.contains_key(&image.path) {
            continue;
        }
        let file = match cache.get(&image.value.hash) {
            Some(bytes) => Some(File { name: image.value.name.clone(), hash: image.value.hash.clone(), kind: image.value.kind, bytes, stand_in: false }),
            None => {
                // The included Work as it is now, for a stand-in (read only when needed).
                let now = loaded.entry(image.db.clone()).or_insert_with(|| storage::load(&image.db, device).ok().map(|(_, d)| d));
                now.as_ref().and_then(|now| picture_file(cache, &image.db, now, &image.value))
            }
        };
        if let Some(file) = file {
            files.insert(image.path.clone(), file);
        }
    }
    Files::new(files)
}

/// One Image's picture for a compile (see `image_files`).
fn picture_file(cache: &mut PictureCache, db: &Path, current: &Doc, value: &ImageValue) -> Option<File> {
    if let Some(bytes) = cache.get(&value.hash) {
        return Some(File { name: value.name.clone(), hash: value.hash.clone(), kind: value.kind, bytes, stand_in: false });
    }
    let picture = stored::picture_for(db, current, value)?;
    let bytes = Bytes::new(picture.bytes);
    if !picture.stand_in {
        cache.keep(&value.hash, bytes.clone());
    }
    Some(File { name: value.name.clone(), hash: picture.hash, kind: picture.kind, bytes, stand_in: picture.stand_in })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests_support::open;
    use crate::work::{Op, Origin};
    use wrtier::change::INCLUSION;

    const RED: &[u8] = include_bytes!("../../test-pictures/red.png");
    const BLUE: &[u8] = include_bytes!("../../test-pictures/blue.png");

    fn typing(from: usize, text: &str) -> Op {
        Op::Type { from, to: from, text: text.into(), origin: Origin::Input }
    }

    /// Adds a picture to `id` as the Image it's named for, and writes it in at `at` on a line of
    /// its own, as Insert Image does. Returns its name.
    fn add(v: &mut Vault, id: &str, file: &str, bytes: &[u8], at: usize, now_ms: u64) -> String {
        let kept = v.keep_picture(id, file, bytes).unwrap();
        let line = format!("#image(\"{}\")\n", kept.name);
        v.edit(id, vec![Op::SetImage { name: kept.name.clone(), hash: kept.hash.clone() }, typing(at, &line)], now_ms).unwrap();
        v.tick(now_ms + 10_000); // the typing burst closes
        kept.name
    }

    fn compile(header: &crate::preamble::Header, body: String, blocks: Vec<crate::typeset::Block>) -> crate::typeset::Compiled {
        let mut t = crate::typeset::Typesetter::new(false);
        let c = t.compile_body("x", Path::new("none"), header, body, blocks);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        c
    }

    fn image(v: &Vault, id: &str, name: &str) -> Option<ImageValue> {
        stored::image_of(&v.works[id].doc, name)
    }

    #[test]
    fn a_content_address_gives_its_file_name() {
        assert_eq!(crate::content_name("content://com.android.externalstorage.documents/document/primary%3APictures%2Fold%20map.png"), "old map.png");
        assert_eq!(crate::content_name("content://media/external/images/media/42"), "42");
    }

    #[test]
    fn an_image_shows_on_the_pages_and_the_text_file_reaches_a_copy_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = open(dir.path());
        v.create_work("Rain", 1_000).unwrap();
        let name = add(&mut v, "Rain", "map.png", RED, 0, 2_000);
        assert_eq!(name, "map.png");
        let (body, header, blocks) = v.typeset_input("Rain", None, 20_000).unwrap();
        assert!(header.images.get("map.png").is_some());
        let compiled = compile(&header, body, blocks);
        assert!(compiled.warnings.is_empty(), "{:?}", compiled.warnings);
        v.leave("Rain", 21_000).unwrap();
        let typ = std::fs::read_to_string(dir.path().join("Rain.typ")).unwrap();
        assert!(typ.contains("#image(\"Rain.images/map.png\")"), "{typ}");
        assert_eq!(std::fs::read(dir.path().join("Rain.images/map.png")).unwrap(), RED);
        let view = v.images("Rain").unwrap();
        assert_eq!(view.own.len(), 1);
        assert_eq!((view.own[0].uses.clone(), view.own[0].past), (vec![8], 0));
    }

    #[test]
    fn replacing_keeps_the_name_sets_the_old_picture_aside_and_undo_brings_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = open(dir.path());
        v.create_work("Rain", 1_000).unwrap();
        add(&mut v, "Rain", "map.png", RED, 0, 2_000);
        let kept = v.keep_picture("Rain", "MAP.PNG", BLUE).unwrap();
        assert_eq!((kept.clash.as_deref(), kept.other_name.as_str(), kept.same), (Some("map.png"), "MAP 2.png", false));
        v.edit("Rain", vec![Op::SetImage { name: kept.name.clone(), hash: kept.hash.clone() }], 30_000).unwrap();
        let now = image(&v, "Rain", "map.png").unwrap();
        assert_eq!((now.name.as_str(), now.hash.as_str()), ("map.png", stored::hash_of(BLUE).as_str()), "it keeps its name");
        let db = dir.path().join("Rain.db");
        assert_eq!(storage::picture_hashes(&db).unwrap(), vec![stored::hash_of(BLUE)], "only the current picture is in the History file");
        assert!(stored::find_past(&db, &stored::hash_of(RED)).is_some(), "the old one is a Past picture");
        assert_eq!(v.images("Rain").unwrap().own[0].past, 1);
        v.edit("Rain", vec![Op::Undo], 31_000).unwrap();
        assert_eq!(image(&v, "Rain", "map.png").unwrap().hash, stored::hash_of(RED));
        assert_eq!(storage::picture_hashes(&db).unwrap(), vec![stored::hash_of(RED)]);
        assert!(stored::find_past(&db, &stored::hash_of(BLUE)).is_some());
    }

    #[test]
    fn renaming_rewrites_every_use_and_one_undo_takes_it_all_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = open(dir.path());
        v.create_work("Rain", 1_000).unwrap();
        add(&mut v, "Rain", "map.png", RED, 0, 2_000);
        let more = "#figure(image(\"Map.png\", width: 50%), caption: [The map])\n";
        v.edit("Rain", vec![typing(17, more)], 20_000).unwrap();
        v.tick(40_000);
        let before = v.works["Rain"].text();
        let state = v.edit("Rain", vec![Op::RenameImage { from: "map.png".into(), to: "Westmarch.png".into() }], 50_000).unwrap();
        assert!(!state.patches.is_empty(), "the editor hears of the new text");
        let text = v.works["Rain"].text();
        assert_eq!(text, before.replace("\"map.png\"", "\"Westmarch.png\"").replace("\"Map.png\"", "\"Westmarch.png\""));
        assert!(image(&v, "Rain", "map.png").is_none());
        assert_eq!(image(&v, "Rain", "westmarch.png").unwrap().name, "Westmarch.png");
        v.edit("Rain", vec![Op::Undo], 51_000).unwrap();
        assert_eq!(v.works["Rain"].text(), before);
        assert!(image(&v, "Rain", "map.png").is_some());
        assert!(image(&v, "Rain", "Westmarch.png").is_none());
        // A name another Image has is refused.
        add(&mut v, "Rain", "river.png", BLUE, 0, 60_000);
        assert!(v.edit("Rain", vec![Op::RenameImage { from: "map.png".into(), to: "River.png".into() }], 80_000).is_err());
    }

    #[test]
    fn a_name_no_image_has_typesets_with_a_grey_box_and_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = open(dir.path());
        v.create_work("Rain", 1_000).unwrap();
        v.edit("Rain", vec![typing(0, "Before.\n\n#image(\"mpa.png\")\n")], 2_000).unwrap();
        let (body, header, blocks) = v.typeset_input("Rain", None, 3_000).unwrap();
        let compiled = compile(&header, body, blocks);
        assert_eq!(compiled.warnings.len(), 1, "{:?}", compiled.warnings);
        assert!(compiled.warnings[0].message.contains("No Image called \"mpa.png\" in this Work"));
        assert_eq!((compiled.warnings[0].line, compiled.warnings[0].from), (Some(3), Some(17)));
    }

    #[test]
    fn two_included_works_each_show_their_own_image_of_the_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = open(dir.path());
        for (id, bytes) in [("Rain", RED), ("Snow", BLUE)] {
            v.create_work(id, 1_000).unwrap();
            add(&mut v, id, "map.png", bytes, 0, 2_000);
            v.close_work(id, 30_000);
        }
        v.create_work("Collected", 40_000).unwrap();
        for (at, work) in [(0, "Rain"), (2, "Snow")] {
            v.edit("Collected", vec![
                Op::Paste { from: at, to: at, text: format!("{INCLUSION}\n") },
                Op::Include { at, work: work.into(), chapter: None, target: None },
            ], 41_000 + at as u64).unwrap();
        }
        let (body, header, blocks) = v.typeset_input("Collected", None, 50_000).unwrap();
        let hashes: BTreeSet<String> = header.images.paths().filter(|p| p.starts_with(".included/")).map(|p| header.images.get(p).unwrap().hash.clone()).collect();
        assert_eq!(hashes, [stored::hash_of(RED), stored::hash_of(BLUE)].into_iter().collect());
        assert!(header.images.get("map.png").is_none(), "the Collection has no Image of its own");
        let compiled = compile(&header, body, blocks);
        assert!(compiled.warnings.is_empty(), "{:?}", compiled.warnings);
        let view = v.images("Collected").unwrap();
        assert_eq!(view.included.iter().map(|g| g.label.as_str()).collect::<Vec<_>>(), vec!["Rain", "Snow"]);
        // The Text file reaches copies of both, each in its Work's folder.
        v.leave("Collected", 60_000).unwrap();
        let typ = std::fs::read_to_string(dir.path().join("Collected.typ")).unwrap();
        let named: Vec<String> = stored::named_in(&typ).into_iter().map(|r| typ[r].to_string()).collect();
        assert_eq!(named.len(), 2, "{typ}");
        for path in &named {
            assert!(path.starts_with("Collected.images/"), "{path}");
            assert!(dir.path().join(path).is_file(), "{path}");
        }
    }

    #[test]
    fn a_copy_changed_in_another_program_replaces_its_image() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = open(dir.path());
        v.create_work("Rain", 1_000).unwrap();
        add(&mut v, "Rain", "map.png", RED, 0, 2_000);
        v.leave("Rain", 20_000).unwrap();
        let copy = dir.path().join("Rain.images/map.png");
        std::fs::write(&copy, BLUE).unwrap(); // saved in an image editor
        std::fs::write(dir.path().join("Rain.images/ornament.png"), RED).unwrap(); // put there by hand
        let state = v.edit("Rain", vec![Op::Sync], 30_000).unwrap();
        assert!(state.took_in.as_deref().is_some_and(|t| t.contains("\"map.png\"")), "{:?}", state.took_in);
        assert_eq!(image(&v, "Rain", "map.png").unwrap().hash, stored::hash_of(BLUE));
        assert_eq!(image(&v, "Rain", "ornament.png").unwrap().hash, stored::hash_of(RED), "a new Image");
        // A copy deleted is written again, and never deletes its Image.
        std::fs::remove_file(&copy).unwrap();
        v.edit("Rain", vec![Op::Sync], 40_000).unwrap();
        assert!(image(&v, "Rain", "map.png").is_some());
        v.edit("Rain", vec![typing(0, "x")], 41_000).unwrap(); // something for the Text file to catch up on
        v.leave("Rain", 50_000).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), BLUE);
    }

    #[test]
    fn past_pictures_are_listed_newest_first_used_again_or_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = open(dir.path());
        v.create_work("Rain", 1_000).unwrap();
        add(&mut v, "Rain", "map.png", RED, 0, 2_000);
        let blue = v.keep_picture("Rain", "map.png", BLUE).unwrap();
        v.edit("Rain", vec![Op::SetImage { name: "map.png".into(), hash: blue.hash.clone() }], 30_000).unwrap();
        // A third PNG, made by hand (Typst never reads it here).
        let green = b"\x89PNG\r\n\x1a\n-green".to_vec();
        let kept = v.keep_picture("Rain", "map.png", &green).unwrap();
        v.edit("Rain", vec![Op::SetImage { name: "map.png".into(), hash: kept.hash.clone() }], 40_000).unwrap();
        let row = &v.images("Rain").unwrap().own.into_iter().find(|r| r.name == "map.png").unwrap();
        let listed: Vec<&str> = row.past_pictures.iter().map(|p| p.hash.as_str()).collect();
        assert_eq!(listed, vec![blue.hash.as_str(), stored::hash_of(RED).as_str()], "newest first");
        assert_eq!((row.past, row.past_size), (2, (BLUE.len() + RED.len()) as u64));
        assert_eq!(v.past_picture("Rain", &blue.hash).unwrap(), BLUE);
        assert!(v.past_picture("Rain", &kept.hash).is_err(), "the current picture isn't a Past one");
        // Used again: it's the Image's picture once more, back in the History file.
        v.edit("Rain", vec![Op::SetImage { name: "map.png".into(), hash: blue.hash.clone() }], 50_000).unwrap();
        assert_eq!(image(&v, "Rain", "map.png").unwrap().hash, blue.hash);
        assert!(storage::picture_hashes(&dir.path().join("Rain.db")).unwrap().contains(&blue.hash));
        // Deleted: gone from the folder; the current picture can't be deleted this way.
        assert_eq!(v.delete_past_pictures("Rain", &[stored::hash_of(RED), blue.hash.clone()]).unwrap(), 1);
        assert!(stored::find_past(&dir.path().join("Rain.db"), &stored::hash_of(RED)).is_none());
        let row = v.images("Rain").unwrap().own.into_iter().find(|r| r.name == "map.png").unwrap();
        assert_eq!(row.past_pictures.iter().map(|p| p.hash.clone()).collect::<Vec<_>>(), vec![kept.hash.clone()]);
    }

    #[test]
    fn a_docx_carries_the_pictures_as_words_own() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = open(dir.path());
        v.create_work("Rain", 1_000).unwrap();
        add(&mut v, "Rain", "map.png", RED, 0, 2_000);
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#c00"/></svg>"##;
        let end = v.works["Rain"].text().encode_utf16().count();
        add(&mut v, "Rain", "ornament.svg", svg, end, 20_000);
        let input = v.export_input("Rain", crate::vault::Export::default(), 50_000).unwrap();
        let (header, options) = crate::docx::prepare(&input, "Rain", false, false);
        let made = crate::typeset::Typesetter::new(false).export_docx(Path::new("none"), &header, input.text, &input.inclusions, &options).unwrap().docx;
        assert!(!made.report.iter().any(|n| matches!(n.what, typst_docx::What::NotCarried { what: typst_docx::Uncarried::Image, .. })), "{:?}", made.report);
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&made.bytes)).unwrap();
        let mut read = |name: &str| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut zip.by_name(name).unwrap(), &mut bytes).unwrap();
            bytes
        };
        assert_eq!(read("word/media/picture1.png"), RED, "the picture as it is");
        assert!(Kind::of_bytes(&read("word/media/picture2.png")) == Some(Kind::Png), "the SVG's PNG");
        assert_eq!(read("word/media/picture3.svg"), svg);
        let document = String::from_utf8(read("word/document.xml")).unwrap();
        assert_eq!(document.matches("<w:drawing>").count(), 2);
        assert!(document.contains("asvg:svgBlip"));
        roxmltree::Document::parse(&document).expect("well-formed");
        let rels = String::from_utf8(read("word/_rels/document.xml.rels")).unwrap();
        assert_eq!(rels.matches("relationships/image").count(), 3);
        let types = String::from_utf8(read("[Content_Types].xml")).unwrap();
        assert!(types.contains("Extension=\"png\"") && types.contains("Extension=\"svg\""), "{types}");
    }

    #[test]
    fn a_milestone_whose_past_picture_was_deleted_shows_the_current_one_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = open(dir.path());
        v.create_work("Rain", 1_000).unwrap();
        add(&mut v, "Rain", "map.png", RED, 0, 2_000);
        v.make_milestone("Rain", "Sent", 20_000).unwrap();
        let kept = v.keep_picture("Rain", "map.png", BLUE).unwrap();
        v.edit("Rain", vec![Op::SetImage { name: kept.name, hash: kept.hash }], 30_000).unwrap();
        let milestone = v.milestones("Rain", 31_000).unwrap()[0].id.clone();
        // While the Past picture is kept, the Milestone shows it.
        let (_, header, _) = v.typeset_input_at("Rain", &milestone, None, 32_000).unwrap();
        let file = header.images.get("map.png").unwrap();
        assert_eq!((file.hash.clone(), file.stand_in), (stored::hash_of(RED), false));
        // Deleted: the current picture stands in, with a warning.
        let (past, _) = stored::find_past(&dir.path().join("Rain.db"), &stored::hash_of(RED)).unwrap();
        std::fs::remove_file(past).unwrap();
        v.pictures = PictureCache::default(); // as after the app starts again
        let (body, header, blocks) = v.typeset_input_at("Rain", &milestone, None, 33_000).unwrap();
        let file = header.images.get("map.png").unwrap();
        assert_eq!((file.hash.clone(), file.stand_in), (stored::hash_of(BLUE), true));
        let compiled = compile(&header, body, blocks);
        assert!(compiled.warnings.iter().any(|w| w.message.contains("was deleted")), "{:?}", compiled.warnings);
    }
}
