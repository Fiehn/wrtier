//! Inclusions (ADR 0015): another Work, or one Chapter of it, shown live inside a Work's text.
//!
//! An Inclusion is one character in the including Work's text (`change::INCLUSION`), on a line
//! of its own, with a record saying what it includes. Here it's worked out: the included Work is
//! found by its ID, wherever it is in the Vault, and its text (with its own Inclusions) is put
//! where the character is, for the editor, the Page preview, Exports and the Text file.
//!
//! The Page preview shows each included Work as it is now. Everything that must stay as it was
//! (Milestones, Exports, the Text file) shows the version the including Work recorded, which it
//! records on the same rule as Styles: just before a Device's first edit in a sitting, and before
//! any Export.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use serde::Serialize;
use wrtier::change::{included_record, inclusion_record, Change, INCLUSION};
use wrtier::doc::Doc;
use wrtier::ids::{CharId, DeviceId};
use wrtier::storage;
use wrtier::text_file::{inclusion_block, parse_version, version_text};
use crate::library::Kind;
use crate::preamble::{self, Preamble, Shelf, Version};
use crate::work::OpenWork;
use super::{err, file_stamp, walk_works, Vault};

/// A Work's version: the highest Change counter it holds from each Device.
pub type VersionOf = Vec<(DeviceId, u32)>;

/// What an Inclusion's record says it includes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// The included Work's ID (`storage::identity`), which follows it through renames.
    pub work: String,
    /// The first character of the included Chapter's heading line; None for the whole Work.
    pub chapter: Option<CharId>,
    /// Styles for this Inclusion only, on top of the including Work's Look.
    pub styles: Vec<String>,
}

impl Target {
    pub fn of(doc: &Doc, at: CharId) -> Option<Target> {
        let record = inclusion_record(at);
        let work = doc.field(&record, "work")?.to_string();
        let chapter = doc.field(&record, "chapter").and_then(|c| c.parse().ok());
        let styles = doc.field(&record, "styles").unwrap_or("").lines()
            .map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
        Some(Target { work, chapter, styles })
    }

    /// The Changes recording this as what the Inclusion at `at` includes.
    pub fn record(&self, doc: &mut Doc, at: CharId, now_ms: u64) -> Vec<Change> {
        let record = inclusion_record(at);
        let mut changes = vec![doc.set_field_local(&record, "work", &self.work, now_ms)];
        if let Some(chapter) = self.chapter {
            changes.push(doc.set_field_local(&record, "chapter", &chapter.to_string(), now_ms));
        }
        if !self.styles.is_empty() {
            changes.push(doc.set_field_local(&record, "styles", &self.styles.join("\n"), now_ms));
        }
        changes
    }
}

/// Where a Work is, found by its ID.
#[derive(Clone, Debug)]
pub struct Located {
    pub db: PathBuf,
    /// Its ID in the Vault ("Short stories/Rain"), or in the trash.
    pub id: String,
    pub trashed: bool,
    /// Another `.db` has the same ID (a Work copied by hand): which one is meant can't be told.
    pub duplicate: bool,
}

/// Every Work in the Vault and its trash, by ID.
pub type Index = HashMap<String, Located>;

/// Included Works read from disk: `.db` and version ("" for the latest) → the file as it was
/// read (to see when it changes) and the Work.
pub type DocCache = HashMap<(PathBuf, String), (Option<(std::time::SystemTime, u64)>, Arc<Doc>)>;

/// One Inclusion, worked out.
#[derive(Clone, Debug)]
pub struct Shown {
    pub at: CharId,
    /// Where its character is in the including Work's `committed_text` (in characters).
    pub index: usize,
    pub target: Option<Target>,
    /// The included Work's ID in the Vault, if it was found.
    pub name: Option<String>,
    /// What it includes, as the Author reads it: "Zampanò", or "Zampanò: The Navidson Record".
    pub label: String,
    /// Why it can't be shown, if it can't.
    pub missing: Option<String>,
    /// The included text as written there (its own Inclusions written out the same way).
    pub plain: String,
    /// The Typst that stands in its place, without the Text file's marker lines.
    pub typst: String,
    /// The version of the included Work shown, if it was found.
    pub version: Option<VersionOf>,
    /// The included Work's language, for the spellchecker and quotation marks.
    pub language: String,
    /// The Author's own Story elements its text uses (its own, and those its Inclusions use).
    pub elements: Vec<String>,
    /// The included Work's title, which heads a whole-Work Inclusion; None for one Chapter.
    pub title: Option<String>,
    /// Where the included text starts in its own Work (UTF-16), for "Go to".
    pub starts: usize,
}

/// What working out Inclusions reads from.
pub(super) struct Ctx<'a> {
    pub works: &'a HashMap<String, OpenWork>,
    pub index: &'a Index,
    pub cache: &'a mut DocCache,
    pub shelf: &'a Shelf,
    pub device: DeviceId,
    /// Show the versions the including Works recorded (a Milestone, an Export, the Text
    /// file), not the latest.
    pub recorded: bool,
}

/// An included Work: the open one, or one read from disk.
enum DocRef<'a> {
    Open(&'a Doc),
    Read(Arc<Doc>),
}

impl std::ops::Deref for DocRef<'_> {
    type Target = Doc;
    fn deref(&self) -> &Doc {
        match self {
            DocRef::Open(doc) => doc,
            DocRef::Read(doc) => doc,
        }
    }
}

impl<'a> Ctx<'a> {
    /// An included Work, as it is now or at `version`.
    fn doc(&mut self, located: &Located, version: Option<&[(DeviceId, u32)]>) -> Result<DocRef<'a>, String> {
        let works = self.works;
        if version.is_none() {
            if let Some(open) = works.values().find(|w| w.db == located.db) {
                return Ok(DocRef::Open(&open.doc));
            }
        }
        let key = (located.db.clone(), version.map(version_text).unwrap_or_default());
        let stamp = file_stamp(&located.db);
        if let Some((seen, doc)) = self.cache.get(&key) {
            // A version never changes once the file has all of it; the latest does.
            if version.is_some() || *seen == stamp {
                return Ok(DocRef::Read(doc.clone()));
            }
        }
        let doc = match version {
            None => storage::load(&located.db, self.device).map_err(err)?.1,
            Some(version) => {
                let held = |id: wrtier::ids::ChangeId| version.iter().any(|&(d, n)| d == id.device && id.counter <= n);
                let changes = storage::read_all(&located.db).map_err(err)?;
                let mut doc = Doc::new(self.device);
                doc.apply_all(&changes.into_iter().filter(|c| held(c.id)).collect::<Vec<_>>());
                let have: HashMap<DeviceId, u32> = doc.version().into_iter().collect();
                if version.iter().any(|(d, n)| have.get(d).is_none_or(|h| h < n)) {
                    // The file hasn't caught up with the version yet: show what's here, but
                    // don't keep it, so the rest shows once it arrives.
                    return Ok(DocRef::Read(Arc::new(doc)));
                }
                doc
            }
        };
        let doc = Arc::new(doc);
        self.cache.insert(key, (stamp, doc.clone()));
        Ok(DocRef::Read(doc))
    }
}

/// The Inclusions in `doc`, worked out, in reading order. `stack`: the IDs of the Works being
/// worked out around it (itself last), so a Work that would include itself shows a placeholder.
pub(super) fn expand(ctx: &mut Ctx, doc: &Doc, stack: &mut Vec<String>) -> Vec<Shown> {
    doc.inclusions().into_iter().map(|(at, index)| shown(ctx, doc, at, index, stack)).collect()
}

fn shown(ctx: &mut Ctx, including: &Doc, at: CharId, index: usize, stack: &mut Vec<String>) -> Shown {
    let mut s = Shown {
        at, index, target: Target::of(including, at), name: None, label: "An Inclusion".into(),
        missing: None, plain: String::new(), typst: String::new(), version: None, language: String::new(), elements: Vec::new(), title: None, starts: 0,
    };
    let result = (|| {
        let target = s.target.clone().ok_or("This Inclusion has lost what it includes.")?;
        let located = ctx.index.get(&target.work).cloned()
            .ok_or("The Work it includes isn't in this Vault (yet). It may still be syncing.")?;
        let name = located.id.rsplit('/').next().unwrap_or(&located.id).to_string();
        s.label = name.clone();
        s.name = Some(located.id.clone());
        if located.duplicate {
            return Err(format!("Two Works claim to be {name}: remove or rename one, so it's clear which is meant."));
        }
        if located.trashed {
            return Err(format!("{name} is in the trash."));
        }
        if stack.contains(&target.work) {
            return Err(format!("Inclusion loop: {name} would include itself."));
        }
        let version = if ctx.recorded {
            including.field(&included_record(&target.work), "version").and_then(parse_version)
        } else {
            None
        };
        let doc = ctx.doc(&located, version.as_deref()).map_err(|e| format!("{name} can't be opened right now: {e}"))?;
        let preamble = Preamble::of(&doc);
        let title = if preamble.title.trim().is_empty() { name.clone() } else { preamble.title.clone() };
        let chars: Vec<char> = doc.committed_text().chars().collect();
        let (from, to) = match target.chapter {
            None => (0, chars.len()),
            Some(chapter) => {
                let range = doc.index_of_char(chapter).and_then(|start| chapter_range(&chars, start));
                let range = range.ok_or_else(|| format!("The Chapter it includes is no longer in {name}."))?;
                let heading: String = chars[range.0..].iter().take_while(|&&c| c != '\n').collect();
                s.label = format!("{name}: {}", heading.trim_start_matches('=').trim());
                s.starts = chars[..range.0].iter().map(|c| c.len_utf16()).sum();
                range
            }
        };
        stack.push(target.work.clone());
        let all_nested = expand(ctx, &doc, stack);
        stack.pop();
        let nested: Vec<&Shown> = all_nested.iter().filter(|n| n.index >= from && n.index < to).collect();
        let (mut plain, mut typst) = (String::new(), String::new());
        let mut next = nested.iter().peekable();
        for (i, &c) in chars.iter().enumerate().take(to).skip(from) {
            match next.peek() {
                Some(n) if n.index == i => {
                    match &n.missing {
                        Some(why) => plain.push_str(&format!("[{}: {why}]", n.label)),
                        None => plain.push_str(&n.plain),
                    }
                    typst.push_str(&inclusion_block(n.at, &n.label, &n.typst));
                    next.next();
                }
                _ => {
                    plain.push(c);
                    typst.push(c);
                }
            }
        }
        let mode = if ctx.recorded { Version::Recorded } else { Version::Library };
        let styles: Vec<String> = target.styles.iter()
            .filter_map(|style| preamble::find(including, ctx.shelf, mode, Kind::Style, style))
            .map(|used| used.content)
            .collect();
        s.typst = wrap(&typst, &preamble.language, target.chapter.is_none().then_some(title.as_str()), &styles);
        s.title = target.chapter.is_none().then_some(title);
        s.plain = plain;
        s.version = Some(doc.version());
        let mut elements = preamble.elements.clone();
        for name in preamble::included_elements(&doc).into_iter().chain(nested.iter().flat_map(|n| n.elements.clone())) {
            if !elements.contains(&name) {
                elements.push(name);
            }
        }
        s.elements = elements;
        s.language = preamble.language;
        Ok(())
    })();
    if let Err(why) = result {
        s.typst = format!("#[\n\n#emph({})\n\n]", preamble::string(&format!("[{}: {why}]", s.label)));
        s.missing = Some(why);
    }
    s
}

/// The included text as Typst that renders in the including Work's Look: in its own language,
/// a whole Work under its title with its Chapters a level down, then its own Styles, kept apart
/// from the text around it by paragraph breaks.
fn wrap(text: &str, language: &str, title: Option<&str>, styles: &[String]) -> String {
    let mut out = String::from("#[\n\n");
    if !language.trim().is_empty() {
        out.push_str(&format!("#set text(lang: {})\n", preamble::string(language.trim())));
    }
    if let Some(title) = title {
        out.push_str(&format!("#heading(level: 1, {})\n#set heading(offset: 1)\n", preamble::string(title)));
    }
    for style in styles {
        out.push_str(style);
        if !style.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str(text);
    out.push_str("\n\n]");
    out
}

/// The Chapter whose heading line starts at `start`: from there to the next Chapter or Part,
/// or the end (as the Chapter list reads them). None if no Chapter starts there.
pub fn chapter_range(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let line_starts = |i: usize| i == 0 || chars.get(i - 1) == Some(&'\n');
    let begins = |i: usize, s: &str| s.chars().enumerate().all(|(k, c)| chars.get(i + k) == Some(&c));
    if !line_starts(start) || !begins(start, "= ") {
        return None;
    }
    let is_part = |i: usize| begins(i, "#part") && matches!(chars.get(i + 5..).and_then(|r| r.iter().find(|c| !c.is_whitespace())), Some('[' | '('));
    let mut i = start;
    loop {
        match chars[i..].iter().position(|&c| c == '\n') {
            None => return Some((start, chars.len())),
            Some(n) => i += n + 1,
        }
        if begins(i, "= ") || is_part(i) {
            return Some((start, i));
        }
    }
}

/// The Work's text with every Inclusion written out as Typst, and where each one is: the
/// position of its character in `text` and how long its Typst is (both UTF-16), so places in
/// the pages can be traced back to the editor's text. `text` may hold the open burst; its
/// Inclusions are matched to `shown` in order.
pub fn typst_body(text: &str, shown: &[Shown]) -> (String, Vec<(usize, usize)>) {
    let mut out = String::with_capacity(text.len());
    let mut blocks = Vec::new();
    let mut next = shown.iter();
    let mut at = 0;
    for c in text.chars() {
        if c == INCLUSION {
            let block = match next.next() {
                Some(s) => inclusion_block(s.at, &s.label, &s.typst),
                None => String::new(), // one only in the open burst: nothing to show yet
            };
            blocks.push((at, block.encode_utf16().count()));
            out.push_str(&block);
        } else {
            out.push(c);
        }
        at += c.len_utf16();
    }
    (out, blocks)
}

/// What the Text file writes out for each Inclusion.
pub fn expansions(shown: &[Shown]) -> storage::Expansions {
    shown.iter().map(|s| (s.at, inclusion_block(s.at, &s.label, &s.typst))).collect()
}

/// The Work's text for a `.docx` Export, which keeps only structure and emphasis: each whole
/// Work it includes under a heading with its title, its own Chapters a level down; a Chapter as
/// it is; a missing one as a line saying so.
pub fn docx_body(text: &str, shown: &[Shown]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut next = shown.iter();
    for c in text.chars() {
        if c != INCLUSION {
            out.push(c);
            continue;
        }
        let Some(s) = next.next() else { continue };
        if let Some(why) = &s.missing {
            out.push_str(&format!("_[{}: {why}]_", s.label.replace(['_', '*'], "")));
            continue;
        }
        match &s.title {
            Some(title) => {
                out.push_str(&format!("= {title}\n\n"));
                for line in s.plain.split_inclusive('\n') {
                    if line.starts_with('=') {
                        out.push('=');
                    }
                    out.push_str(line);
                }
            }
            None => out.push_str(&s.plain),
        }
    }
    out
}

/// An Inclusion as the editor shows it.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InclusionView {
    /// Where its character is in the editor's text (UTF-16).
    pub at: usize,
    /// The included Work's ID in the Vault, to open it ("Go to"); None if it wasn't found.
    pub work: Option<String>,
    pub label: String,
    /// The included text, read-only.
    pub text: String,
    pub missing: Option<String>,
    pub language: String,
    pub whole: bool,
    /// Where the included text starts in its own Work (UTF-16): "Go to" opens it there.
    pub starts: usize,
}

/// A Chapter that can be included.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChapterChoice {
    pub title: String,
    /// Where its heading line starts in its Work's text (UTF-16).
    pub at: usize,
}

/// The Inclusions as the editor shows them, matched in order to the Inclusion characters in
/// `text` (the editor's text, which may hold the open burst).
pub fn views(text: &str, shown: &[Shown]) -> Vec<InclusionView> {
    let mut out = Vec::new();
    let mut next = shown.iter();
    let mut at = 0;
    for c in text.chars() {
        if c == INCLUSION {
            if let Some(s) = next.next() {
                out.push(InclusionView {
                    at, work: s.name.clone(), label: s.label.clone(), text: s.plain.clone(),
                    missing: s.missing.clone(), language: s.language.clone(),
                    whole: s.target.as_ref().is_some_and(|t| t.chapter.is_none()),
                    starts: s.starts,
                });
            }
        }
        at += c.len_utf16();
    }
    out
}

impl Vault {
    /// Every Work in the Vault and its trash, by ID. Each `.db`'s ID is read once and kept.
    pub(super) fn work_index(&mut self) -> Index {
        let trash = self.trash_root();
        let mut index: Index = HashMap::new();
        let mut seen = std::collections::HashSet::new();
        for (db, trashed) in walk_works(&self.root).dbs.into_iter().map(|d| (d, false))
            .chain(walk_works(&trash).dbs.into_iter().map(|d| (d, true)))
        {
            seen.insert(db.clone());
            let identity = match self.identities.get(&db) {
                Some(known) => known.clone(),
                None => {
                    let read = storage::identity(&db).ok().flatten();
                    self.identities.insert(db.clone(), read.clone());
                    read
                }
            };
            let Some(identity) = identity else { continue };
            let id = if trashed { relative_id(&trash, &db) } else { self.id_of(&db) };
            match index.get_mut(&identity) {
                // The Vault's own copy wins over one in the trash; two in the Vault are a problem.
                Some(found) if found.trashed && !trashed => *found = Located { db, id, trashed, duplicate: false },
                Some(found) if found.trashed == trashed => found.duplicate = true,
                Some(_) => {}
                None => {
                    index.insert(identity, Located { db, id, trashed, duplicate: false });
                }
            }
        }
        self.identities.retain(|db, _| seen.contains(db));
        index
    }

    /// The Inclusions of an open Work, worked out. `recorded`: at the versions it recorded.
    pub(super) fn shown_in(&mut self, id: &str, recorded: bool) -> Result<Vec<Shown>, String> {
        let id = self.current_id(id);
        let work = self.works.get(&id).ok_or_else(|| format!("{id} isn't open."))?;
        // Most Works include nothing: then the Vault isn't looked through at all.
        if work.doc.inclusions().is_empty() {
            return Ok(Vec::new());
        }
        let index = self.work_index();
        let shelf = Shelf::read(&self.root);
        let work = &self.works[&id];
        let mut ctx = Ctx { works: &self.works, index: &index, cache: &mut self.included_docs, shelf: &shelf, device: self.device, recorded };
        let mut stack = work.identity().map(|i| vec![i.to_string()]).unwrap_or_default();
        Ok(expand(&mut ctx, &work.doc, &mut stack))
    }

    /// The Inclusions of a Work that isn't open (or of a Work as it was at a Milestone),
    /// worked out at the versions it recorded.
    pub(super) fn shown_in_doc(&mut self, doc: &Doc) -> Vec<Shown> {
        self.shown_in_doc_as(doc, true)
    }

    /// `shown_in_doc`, as the included Works are now (`recorded` false) or as recorded.
    fn shown_in_doc_as(&mut self, doc: &Doc, recorded: bool) -> Vec<Shown> {
        if doc.inclusions().is_empty() {
            return Vec::new();
        }
        let index = self.work_index();
        let shelf = Shelf::read(&self.root);
        let identity = doc.field(storage::WORK_RECORD, storage::WORK_ID_FIELD).map(str::to_string);
        let mut ctx = Ctx { works: &self.works, index: &index, cache: &mut self.included_docs, shelf: &shelf, device: self.device, recorded };
        expand(&mut ctx, doc, &mut identity.into_iter().collect())
    }

    /// `record_inclusions`, for a Work that isn't open: the Changes, to be saved by the caller.
    pub(super) fn record_inclusions_in(&mut self, doc: &mut Doc, now_ms: u64) -> Vec<Change> {
        let shown = self.shown_in_doc_as(doc, false);
        unrecorded(doc, &shown).into_iter()
            .map(|(record, field, value)| doc.set_field_local(&record, &field, &value, now_ms))
            .collect()
    }

    /// Records what an open Work keeps about its Inclusions (the versions shown, and the Story
    /// elements they use) where that changed, unless a burst is open (then it's done before a
    /// later edit). Called before edits, Exports and Milestones.
    pub(super) fn record_inclusions(&mut self, id: &str, now_ms: u64) {
        let key = self.current_id(id);
        let Some(work) = self.works.get(&key) else { return };
        if work.doc.burst().is_some() || work.doc.inclusions().is_empty() {
            return;
        }
        let Ok(shown) = self.shown_in(id, false) else { return };
        let Some(work) = self.works.get_mut(&key) else { return };
        let fields = unrecorded(&work.doc, &shown);
        if !fields.is_empty() && work.record_fields(&fields, now_ms) {
            self.refresh_text_file_inclusions(id);
        }
    }

    /// Works out again what an open Work's Text file writes for its Inclusions: at the versions
    /// it recorded.
    pub(super) fn refresh_text_file_inclusions(&mut self, id: &str) {
        let key = self.current_id(id);
        let Ok(shown) = self.shown_in(id, true) else { return };
        if let Some(work) = self.works.get_mut(&key) {
            work.inclusions = expansions(&shown);
        }
    }

    /// The Inclusions of a Work (open or not) that can't be shown now, each as "label: why",
    /// so an Export can ask before going ahead without them.
    pub fn missing_inclusions(&mut self, id: &str) -> Result<Vec<String>, String> {
        let shown = if self.works.contains_key(&self.current_id(id)) {
            self.shown_in(id, false)?
        } else {
            let (db, _) = self.paths(id)?;
            let doc = storage::load(&db, self.device).map_err(err)?.1;
            self.shown_in_doc_as(&doc, false)
        };
        Ok(shown.iter().filter_map(|s| s.missing.as_ref().map(|why| format!("{}: {why}", s.label))).collect())
    }

    /// The Inclusions of an open Work, for the editor.
    pub fn inclusions(&mut self, id: &str) -> Result<Vec<InclusionView>, String> {
        let shown = self.shown_in(id, false)?;
        let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
        Ok(views(&work.text(), &shown))
    }

    /// The Chapters of a Work (an ID in the Vault), to pick one to include: each one's title and
    /// where its heading starts (UTF-16).
    pub fn chapters_of(&mut self, work: &str) -> Result<Vec<ChapterChoice>, String> {
        let (db, _) = self.paths(work)?;
        let text = match self.works.values().find(|w| w.db == db) {
            Some(w) => w.doc.committed_text(),
            None => storage::load(&db, self.device).map_err(err)?.1.committed_text(),
        };
        let mut found = Vec::new();
        let mut at = 0;
        for line in text.split_inclusive('\n') {
            if let Some(title) = line.strip_prefix("= ") {
                let title = title.trim().replace(['*', '_', '`'], "");
                found.push(ChapterChoice { title: if title.is_empty() { "Untitled".into() } else { title }, at });
            }
            at += line.encode_utf16().count();
        }
        Ok(found)
    }

    /// What an Inclusion of the Work `work` (an ID in the Vault) should record: its ID, and the
    /// character starting the heading of the Chapter that starts at `chapter` (a position in
    /// its text, UTF-16), if one.
    pub(super) fn target_for(&mut self, work: &str, chapter: Option<usize>) -> Result<Target, String> {
        let (db, _) = self.paths(work)?;
        let open = self.works.values().find(|w| w.db == db);
        let loaded;
        let doc = match open {
            Some(w) => &w.doc,
            None => {
                loaded = storage::load(&db, self.device).map_err(err)?.1;
                &loaded
            }
        };
        let identity = doc.field(storage::WORK_RECORD, storage::WORK_ID_FIELD).map(str::to_string)
            .or_else(|| storage::identity(&db).ok().flatten())
            .ok_or_else(|| format!("{work} has no ID yet: open it once, then include it."))?;
        let chapter = match chapter {
            None => None,
            Some(at) => {
                let text = doc.committed_text();
                let index = crate::positions::Positions::new(&text).to_char(at).map_err(|e| e.to_string())?;
                let chars: Vec<char> = text.chars().collect();
                chapter_range(&chars, index).ok_or("There's no Chapter heading there.")?;
                Some(doc.visible()[index].0)
            }
        };
        Ok(Target { work: identity, chapter, styles: Vec::new() })
    }
}

/// A Work's ID under `base` (the trash): its path from there, without the extension.
fn relative_id(base: &Path, db: &Path) -> String {
    let relative = db.strip_prefix(base).unwrap_or(db);
    let joined = relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
    joined.strip_suffix(".db").map(str::to_string).unwrap_or(joined)
}

/// What an including Work should record now (ADR 0015), where it differs from what it
/// recorded: the version shown of each Work it includes directly, and the Story elements its
/// Inclusions use. Returns (record, field, value).
pub fn unrecorded(doc: &Doc, shown: &[Shown]) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for s in shown {
        let (Some(target), Some(version)) = (&s.target, &s.version) else { continue };
        let record = included_record(&target.work);
        let version = version_text(version);
        if doc.field(&record, "version") != Some(version.as_str()) && !out.iter().any(|(r, _, _)| *r == record) {
            out.push((record, "version".into(), version));
        }
    }
    // Only ever added to: an element an Inclusion stopped using costs nothing to keep.
    let mut elements = preamble::included_elements(doc);
    let before = elements.len();
    for name in shown.iter().flat_map(|s| &s.elements) {
        if !elements.contains(name) {
            elements.push(name.clone());
        }
    }
    if elements.len() != before {
        out.push((preamble::INCLUDED_ELEMENTS.into(), "names".into(), elements.join("\n")));
    }
    out
}

/// The Story elements the Inclusions use, for a Page preview that shows them as they are now.
pub fn elements_of(shown: &[Shown]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in shown.iter().flat_map(|s| &s.elements) {
        if !out.contains(name) {
            out.push(name.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests_support::open;
    use crate::work::{Op, Origin};
    use crate::typeset::Block;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn typing(from: usize, text: &str) -> Op {
        Op::Type { from, to: from, text: text.into(), origin: Origin::Input }
    }

    /// Pastes an Inclusion character at `at` in `id` (on a line of its own) and says it
    /// includes `work`, or its Chapter starting at `chapter`.
    fn include(v: &mut Vault, id: &str, at: usize, work: &str, chapter: Option<usize>, now_ms: u64) {
        v.edit(id, vec![
            Op::Paste { from: at, to: at, text: INCLUSION.to_string() },
            Op::Include { at, work: work.into(), chapter, target: None },
        ], now_ms).unwrap();
    }

    const RAIN: &str = "= One\n\nThe rain had stopped.\n\n= Two\n\nShe went home.";

    /// A Vault with a short story, "Rain" (in Danish, two Chapters), and "Collected", which
    /// has an introduction of its own and then includes all of "Rain".
    fn collected(dir: &Path) -> Vault {
        let mut v = open(dir);
        v.create_work("Rain", 1000).unwrap();
        let p = Preamble { looks: vec!["Paperback".into()], title: "Rain at Night".into(), language: "da".into(), ..Preamble::default() };
        v.set_preamble("Rain", &p, 1100).unwrap();
        v.edit("Rain", vec![typing(0, RAIN)], 1200).unwrap();
        v.close_work("Rain", 1300);
        v.create_work("Collected", 2000).unwrap();
        v.edit("Collected", vec![typing(0, "My introduction.\n\n")], 2100).unwrap();
        v.tick(10_000); // the typing burst closes
        include(&mut v, "Collected", 18, "Rain", None, 11_000);
        v
    }

    fn compiles(header: &crate::preamble::Header, body: String, blocks: Vec<Block>) -> usize {
        let mut t = crate::typeset::Typesetter::new(false);
        let c = t.compile_body("x", Path::new("none"), header, body, blocks);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        c.pages.len()
    }

    #[test]
    fn a_work_shows_another_and_typesets_it_in_its_own_look() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        let views = v.inclusions("Collected").unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!((views[0].at, views[0].label.as_str(), views[0].text.as_str()), (18, "Rain", RAIN));
        assert!(views[0].whole && views[0].missing.is_none());
        let (body, header, blocks) = v.typeset_input("Collected", None, 12_000).unwrap();
        assert!(body.contains("The rain had stopped.") && body.contains("lang: \"da\"") && body.contains("\"Rain at Night\""), "{body}");
        assert!(compiles(&header, body, blocks) >= 2, "the Collection's own page, then the story's");
    }

    #[test]
    fn the_preview_is_live_and_a_milestone_keeps_the_version_it_showed() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        v.make_milestone("Collected", "Sent", 12_000).unwrap();
        let sent = v.milestones("Collected", 12_100).unwrap()[0].id.clone();
        // The story changes after the Milestone, in its own tab.
        v.open_work("Rain", 13_000).unwrap();
        v.edit("Rain", vec![typing(RAIN.len(), " Changed later.")], 13_100).unwrap();
        v.tick(20_000);
        let (now, _, _) = v.typeset_input("Collected", None, 20_100).unwrap();
        assert!(now.contains("Changed later."), "the Page preview follows the story");
        assert!(v.inclusions("Collected").unwrap()[0].text.ends_with("Changed later."));
        let (then, header, blocks) = v.typeset_input_at("Collected", &sent, None, 20_200).unwrap();
        assert!(then.contains("She went home.") && !then.contains("Changed later."), "the Milestone shows what was sent");
        compiles(&header, then, blocks);
    }

    #[test]
    fn the_text_file_writes_out_the_recorded_version_and_reads_back_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        v.edit("Collected", vec![typing(0, "Note. ")], 12_000).unwrap(); // records the version shown
        v.close_work("Collected", 13_000);
        let file = std::fs::read_to_string(dir.path().join("Collected.typ")).unwrap();
        assert!(file.contains("The rain had stopped.") && file.contains("// ---- Included by wrtier"), "{file}");
        assert!(!file.contains(INCLUSION));
        // It compiles on its own, and opening the Work again finds nothing new in it.
        let mut t = crate::typeset::Typesetter::new(false);
        let c = t.compile("file", Path::new("none"), file);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        let before = storage::read_all(&dir.path().join("Collected.db")).unwrap().len();
        let opened = v.open_work("Collected", 14_000).unwrap();
        assert_eq!(opened.notice, None);
        assert_eq!(storage::read_all(&dir.path().join("Collected.db")).unwrap().len(), before);
        assert_eq!(opened.text.matches(INCLUSION).count(), 1);
    }

    #[test]
    fn a_chapter_can_be_included_and_goes_missing_with_its_heading() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        let two = RAIN.find("= Two").unwrap();
        include(&mut v, "Collected", 0, "Rain", Some(two), 12_000);
        let views = v.inclusions("Collected").unwrap();
        assert_eq!((views[0].label.as_str(), views[0].text.as_str(), views[0].whole), ("Rain: Two", "= Two\n\nShe went home.", false));
        // The heading is deleted in the story: the Inclusion says so.
        v.open_work("Rain", 13_000).unwrap();
        v.edit("Rain", vec![Op::Delete { from: two, to: two + 6, origin: Origin::Input }], 13_100).unwrap();
        v.tick(20_000);
        let views = v.inclusions("Collected").unwrap();
        assert_eq!(views[0].missing.as_deref(), Some("The Chapter it includes is no longer in Rain."));
        let (body, header, blocks) = v.typeset_input("Collected", None, 20_100).unwrap();
        assert!(body.contains("no longer in Rain"));
        compiles(&header, body, blocks);
    }

    #[test]
    fn a_loop_and_a_trashed_work_show_placeholders() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        // "Rain" includes "Collected" back.
        v.open_work("Rain", 12_000).unwrap();
        include(&mut v, "Rain", 0, "Collected", None, 12_100);
        let views = v.inclusions("Collected").unwrap();
        assert!(views[0].missing.is_none(), "Rain itself is still shown");
        // Inside it, where Collected would come again (it's what is being shown), a note instead.
        assert_eq!(views[0].text, format!("[Collected: Inclusion loop: Collected would include itself.]{RAIN}"));
        let (body, header, blocks) = v.typeset_input("Collected", None, 12_200).unwrap();
        assert!(body.contains("Inclusion loop"), "{body}");
        compiles(&header, body, blocks);
        v.close_work("Rain", 12_300);
        v.trash_work("Rain", 12_400).unwrap();
        let views = v.inclusions("Collected").unwrap();
        assert_eq!(views[0].missing.as_deref(), Some("Rain is in the trash."));
    }

    #[test]
    fn places_in_the_preview_map_around_inclusions() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        v.edit("Collected", vec![typing(19, "\nAfterword.")], 12_000).unwrap();
        let (body, header, blocks) = v.typeset_input("Collected", None, 12_100).unwrap();
        let mut t = crate::typeset::Typesetter::new(false);
        let c = t.compile_body("c", Path::new("none"), &header, body, blocks);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        // "Afterword." is after the Inclusion: its place on the pages and back gives the same spot.
        let at = "My introduction.\n\n\u{FFFC}\nAfter".encode_utf16().count();
        let spot = t.locate("c", at).expect("on the pages");
        assert_eq!(spot.page + 1, c.pages.len(), "on the last page, after the story");
    }

    #[test]
    fn a_chapter_runs_to_the_next_chapter_or_part() {
        let text = chars("Preface.\n= One\nFirst.\n== Inside\nStill one.\n= Two\nSecond.\n#part[Book II]\n= Three\nEnd.");
        let one = text.iter().collect::<String>().find("= One").unwrap();
        let (from, to) = chapter_range(&text, one).unwrap();
        assert_eq!(text[from..to].iter().collect::<String>(), "= One\nFirst.\n== Inside\nStill one.\n");
        let two = text.iter().collect::<String>().find("= Two").unwrap();
        let (from, to) = chapter_range(&text, two).unwrap();
        assert_eq!(text[from..to].iter().collect::<String>(), "= Two\nSecond.\n");
        let three = text.iter().collect::<String>().find("= Three").unwrap();
        assert_eq!(chapter_range(&text, three).unwrap().1, text.len());
        assert_eq!(chapter_range(&text, 0), None, "no heading at the start");
        assert_eq!(chapter_range(&text, one + 1), None, "not at a line's start");
    }

    #[test]
    fn places_in_the_pages_trace_back_to_the_inclusions_character() {
        let at: CharId = "00000001:1+0".parse().unwrap();
        let s = Shown {
            at, index: 2, target: None, name: None, label: "Rain".into(), missing: None,
            plain: "Rain.".into(), typst: "#[\nRain.\n]".into(), version: None, language: String::new(),
            elements: Vec::new(), title: None, starts: 0,
        };
        let (body, blocks) = typst_body(&format!("A\n{INCLUSION}\nB"), &[s]);
        assert!(body.starts_with("A\n// ---- Included by wrtier") && body.ends_with("\nB"), "{body}");
        assert_eq!(blocks, vec![(2, body.encode_utf16().count() - 4)]);
    }
}
