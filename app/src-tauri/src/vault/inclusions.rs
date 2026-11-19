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
use crate::work::{CommentHighlight, Op, OpenWork};
use super::{err, file_stamp, Vault};
use super::annotations::{annotations_of, on_note, printed, Ann, NoteShown};

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
    /// For a Chapter: the first character of the heading that came after it when it was
    /// included (or the Author last looked): a new one between them shows a note on the bar.
    pub next: Option<CharId>,
}

impl Target {
    pub fn of(doc: &Doc, at: CharId) -> Option<Target> {
        let record = inclusion_record(at);
        let work = doc.field(&record, "work")?.to_string();
        let chapter = doc.field(&record, "chapter").and_then(|c| c.parse().ok());
        let styles = doc.field(&record, "styles").unwrap_or("").lines()
            .map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
        let next = doc.field(&record, "next").and_then(|c| c.parse().ok());
        Some(Target { work, chapter, styles, next })
    }

    /// The Changes recording this as what the Inclusion at `at` includes.
    pub fn record(&self, doc: &mut Doc, at: CharId, now_ms: u64) -> Vec<Change> {
        let record = inclusion_record(at);
        let mut changes = vec![doc.set_field_local(&record, "work", &self.work, now_ms)];
        if let Some(chapter) = self.chapter {
            changes.push(doc.set_field_local(&record, "chapter", &chapter.to_string(), now_ms));
        }
        if let Some(next) = self.next {
            changes.push(doc.set_field_local(&record, "next", &next.to_string(), now_ms));
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

/// A `.db`'s stamp, if it has one.
type Stamp = Option<crate::stamps::Stamp>;

/// Included Works read from disk: `.db` and version ("" for the latest) → the file as it was
/// read (to see when it changes) and the Work. Each entry is a whole Work, and every newer
/// version an including Work records adds one, so an entry nothing used for a while is let go
/// (`sweep`, from the timer). An open tab works its Inclusions out every few seconds, which
/// keeps what it shows.
#[derive(Default)]
pub struct DocCache {
    entries: HashMap<(PathBuf, String), (Stamp, Arc<Doc>, std::time::Instant)>,
}

/// How long an entry nothing uses is kept.
const KEEP_UNUSED: std::time::Duration = std::time::Duration::from_secs(60);

impl DocCache {
    /// An entry, if there is one (which counts as using it).
    pub fn get(&mut self, key: &(PathBuf, String)) -> Option<(Stamp, Arc<Doc>)> {
        let (stamp, doc, used) = self.entries.get_mut(key)?;
        *used = std::time::Instant::now();
        Some((*stamp, doc.clone()))
    }

    /// An entry's stamp, without counting as a use.
    pub fn stamp_of(&self, key: &(PathBuf, String)) -> Option<Stamp> {
        self.entries.get(key).map(|(stamp, _, _)| *stamp)
    }

    pub fn insert(&mut self, key: (PathBuf, String), stamp: Stamp, doc: Arc<Doc>) {
        self.entries.insert(key, (stamp, doc, std::time::Instant::now()));
    }

    /// Lets go of what nothing used for a while.
    pub fn sweep(&mut self) {
        self.entries.retain(|_, (_, _, used)| used.elapsed() < KEEP_UNUSED);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod doc_cache_tests {
    use super::*;

    #[test]
    fn an_entry_nothing_used_for_a_while_is_let_go() {
        let mut cache = DocCache::default();
        let doc = Arc::new(Doc::new(DeviceId(1)));
        let (latest, old) = ((PathBuf::from("Rain.db"), String::new()), (PathBuf::from("Rain.db"), "00000001:3".to_string()));
        cache.insert(latest.clone(), None, doc.clone());
        cache.insert(old.clone(), None, doc);
        let Some(long_ago) = std::time::Instant::now().checked_sub(KEEP_UNUSED * 2) else { return };
        cache.entries.get_mut(&old).unwrap().2 = long_ago;
        cache.sweep();
        assert_eq!(cache.len(), 1);
        assert!(cache.get(&latest).is_some(), "the one used lately stays");
    }
}

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
    /// The included Chapter's title ("The Navidson Record"), if it includes one and it was found.
    pub chapter: Option<String>,
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
    /// Where the included Work's own text is in `plain` (not its own Inclusions'): runs of
    /// (start in `plain`, start in the included Work's `committed_text`, length), in characters.
    pub runs: Vec<(usize, usize, usize)>,
    /// The included Work's Comments on that text, ranges in `plain` (UTF-16); names left to fill in.
    pub comments: Vec<CommentHighlight>,
    /// Where the included Work's own text is in `typst`: runs of (start in `plain`, start in
    /// `typst`, length), in UTF-16, so a `.docx` can put its Comments where the text came out.
    pub in_typst: Vec<(usize, usize, usize)>,
    /// The included Work's author name, for its Author's Comments.
    pub author: String,
    /// A new Chapter found right after the included one (see `Target::next`), if one.
    pub split: Option<SplitNote>,
    /// Where each Work's own text is in `plain`, at any depth of Inclusions: runs of (start in
    /// `plain`, start in that Work's `committed_text`, length) in characters, and that Work's ID.
    pub all_runs: Vec<(usize, usize, usize, String)>,
    /// The Annotations printed in the included text (ADR 0020), by the Works around it, places
    /// in `plain` (UTF-16), in the order they print.
    pub notes: Vec<NoteShown>,
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
pub(super) enum DocRef<'a> {
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
    pub(super) fn doc(&mut self, located: &Located, version: Option<&[(DeviceId, u32)]>) -> Result<DocRef<'a>, String> {
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
            if version.is_some() || seen == stamp {
                return Ok(DocRef::Read(doc));
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
        self.cache.insert(key, stamp, doc.clone());
        Ok(DocRef::Read(doc))
    }
}

/// The Inclusions in `doc`, worked out, in reading order. `stack`: the IDs of the Works being
/// worked out around it (itself last), so a Work that would include itself shows a placeholder.
pub(super) fn expand(ctx: &mut Ctx, doc: &Doc, stack: &mut Vec<String>) -> Vec<Shown> {
    expand_with(ctx, doc, stack, &[])
}

/// `expand`, with the Annotations of the Works around `doc` (`around`), which print on the text
/// of the Works it includes too.
fn expand_with(ctx: &mut Ctx, doc: &Doc, stack: &mut Vec<String>, around: &[Ann]) -> Vec<Shown> {
    let inclusions = doc.inclusions();
    if inclusions.is_empty() {
        return Vec::new();
    }
    let owner = stack.last().cloned().unwrap_or_default();
    let mut notes = around.to_vec();
    notes.extend(annotations_of(doc, &owner));
    inclusions.into_iter().map(|(at, index)| shown(ctx, doc, at, index, stack, &notes)).collect()
}

fn shown(ctx: &mut Ctx, including: &Doc, at: CharId, index: usize, stack: &mut Vec<String>, notes: &[Ann]) -> Shown {
    let mut s = Shown {
        at, index, target: Target::of(including, at), name: None, label: "An Inclusion".into(), chapter: None,
        missing: None, plain: String::new(), typst: String::new(), version: None, language: String::new(), elements: Vec::new(), title: None, starts: 0, runs: Vec::new(), comments: Vec::new(), in_typst: Vec::new(), author: String::new(), split: None,
        all_runs: Vec::new(), notes: Vec::new(),
    };
    let result = (|| {
        let target = s.target.clone().ok_or("This Inclusion has lost what it includes.")?;
        let located = ctx.index.get(&target.work).cloned()
            .ok_or("The Work it includes isn't in this Vault (yet). It may still be syncing.")?;
        let name = located.id.rsplit('/').next().unwrap_or(&located.id).to_string();
        s.label = name.clone();
        s.name = Some(located.id.clone());
        if located.duplicate {
            return Err(format!("Two Works claim to be {name}: move one to the trash, so it's clear which is meant (see Sync problems)."));
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
                let heading = heading.trim_start_matches('=').trim().to_string();
                s.label = format!("{name}: {heading}");
                s.chapter = Some(heading);
                s.starts = chars[..range.0].iter().map(|c| c.len_utf16()).sum();
                // A new heading between this Chapter and the one that followed it when it was
                // included: it was split, or a Chapter was put after it (live only).
                if let (false, Some(was)) = (ctx.recorded, target.next) {
                    let visible = doc.visible();
                    let now = visible.get(range.1).map(|&(id, _)| id);
                    let still_later = doc.index_of_char(was).is_some_and(|i| i > range.1);
                    if now.is_some_and(|n| n != was) && still_later {
                        let title: String = chars[range.1..].iter().take_while(|&&c| c != '\n').collect();
                        s.split = Some(SplitNote {
                            title: title.trim_start_matches('=').trim().to_string(),
                            at: chars[..range.1].iter().map(|c| c.len_utf16()).sum(),
                        });
                    }
                }
                range
            }
        };
        stack.push(target.work.clone());
        let all_nested = expand_with(ctx, &doc, stack, notes);
        stack.pop();
        // The Annotations on this Work's text here: each printed after the last of its
        // characters shown (ADR 0020), by where it ends; those on Annotations go in theirs.
        let mut ends: HashMap<usize, Vec<(&Ann, Vec<usize>)>> = HashMap::new();
        for ann in notes.iter().filter(|a| a.note.work == target.work && !a.spans.is_empty()) {
            let covered: Vec<usize> = doc.covered(&ann.spans).into_iter()
                .filter(|&i| i >= from && i < to && chars[i] != INCLUSION)
                .collect();
            if let Some(&last) = covered.last() {
                ends.entry(last).or_default().push((ann, covered));
            }
        }
        // Where each of this Work's characters is in `plain` (UTF-16), to say what they cover.
        let mut placed_at: HashMap<usize, (usize, usize)> = HashMap::new();
        let mut placed: Vec<(&Ann, Vec<usize>, usize)> = Vec::new();
        let nested: Vec<&Shown> = all_nested.iter().filter(|n| n.index >= from && n.index < to).collect();
        let (mut plain, mut typst) = (String::new(), String::new());
        let mut plain_len = 0; // in characters
        let mut runs: Vec<(usize, usize, usize)> = Vec::new();
        let mut in_typst: Vec<(usize, usize, usize)> = Vec::new(); // UTF-16
        let (mut plain_u16, mut typst_u16) = (0, 0);
        let mut next = nested.iter().peekable();
        for (i, &c) in chars.iter().enumerate().take(to).skip(from) {
            match next.peek() {
                Some(n) if n.index == i => {
                    let inner = match &n.missing {
                        Some(why) => format!("[{}: {why}]", n.label),
                        None => n.plain.clone(),
                    };
                    if n.missing.is_none() {
                        s.all_runs.extend(n.all_runs.iter().map(|(p, b, len, w)| (p + plain_len, *b, *len, w.clone())));
                        s.notes.extend(n.notes.iter().map(|note| shifted(note, plain_u16)));
                    }
                    plain_len += inner.chars().count();
                    plain_u16 += inner.encode_utf16().count();
                    plain.push_str(&inner);
                    let block = inclusion_block(n.at, &n.label, &n.typst);
                    typst_u16 += block.encode_utf16().count();
                    typst.push_str(&block);
                    next.next();
                }
                _ => {
                    match runs.last_mut() {
                        Some((p, b, len)) if *p + *len == plain_len && *b + *len == i => *len += 1,
                        _ => runs.push((plain_len, i, 1)),
                    }
                    plain_len += 1;
                    match in_typst.last_mut() {
                        Some((p, t, len)) if *p + *len == plain_u16 && *t + *len == typst_u16 => *len += c.len_utf16(),
                        _ => in_typst.push((plain_u16, typst_u16, c.len_utf16())),
                    }
                    placed_at.insert(i, (plain_u16, plain_u16 + c.len_utf16()));
                    plain_u16 += c.len_utf16();
                    typst_u16 += c.len_utf16();
                    plain.push(c);
                    typst.push(c);
                    // Annotations ending here: their footnotes, right after it.
                    if let Some(here) = ends.remove(&i) {
                        for (ann, covered) in here {
                            let footnote = format!("#footnote[{}]", printed(ann, notes, 0));
                            typst_u16 += footnote.encode_utf16().count();
                            typst.push_str(&footnote);
                            placed.push((ann, covered, plain_u16));
                        }
                    }
                }
            }
        }
        s.comments = comments_in(&doc, &plain, &runs);
        s.all_runs.extend(runs.iter().map(|&(p, b, len)| (p, b, len, target.work.clone())));
        s.all_runs.sort_by_key(|r| r.0);
        for (ann, covered, at) in placed {
            let mut ranges: Vec<(usize, usize)> = Vec::new();
            for (a, b) in covered.iter().filter_map(|i| placed_at.get(i).copied()) {
                match ranges.last_mut() {
                    Some((_, end)) if *end == a => *end = b,
                    _ => ranges.push((a, b)),
                }
            }
            s.notes.push(NoteShown {
                id: ann.id.clone(), owner: ann.owner.clone(), owner_label: String::new(), mine: false, at, ranges,
                body: ann.note.body.clone(), quote: ann.note.quote.clone(), notes: on_note(ann, notes, at, 0),
            });
        }
        // In the order they print: by place (a stable sort keeps the order they were placed in).
        s.notes.sort_by_key(|n| n.at);
        s.runs = runs;
        s.author = preamble.author.clone();
        let mode = if ctx.recorded { Version::Recorded } else { Version::Library };
        let styles: Vec<String> = target.styles.iter()
            .filter_map(|style| preamble::find(including, ctx.shelf, mode, Kind::Style, style))
            .map(|used| used.content)
            .collect();
        // A whole Work's title, and its author's name when it isn't the including Work's.
        let heading = target.chapter.is_none().then(|| {
            let author = preamble.author.trim();
            let shown = !author.is_empty() && author != Preamble::of(including).author.trim();
            (title.as_str(), shown.then_some(author))
        });
        let (wrapped, at) = wrap(&typst, &preamble.language, heading, &styles, &name);
        s.typst = wrapped;
        let opening = s.typst[..at].encode_utf16().count();
        s.in_typst = in_typst.into_iter().map(|(p, t, len)| (p, t + opening, len)).collect();
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

/// A place in the shown text (`text()`, with the open burst's draft) as a place in
/// `committed_text`: the draft isn't there, and what it's typed over still is.
fn committed_at(doc: &Doc, at: usize) -> usize {
    let Some(burst) = doc.burst() else { return at };
    let text_len = doc.len();
    let committed_len = doc.visible().len();
    let draft = burst.draft.chars().count();
    let hidden = committed_len + draft - text_len;
    let start = doc.burst_at();
    if at <= start {
        at
    } else if at <= start + draft {
        start
    } else {
        at - draft + hidden
    }
}

/// The included Work's Comments on its own text in `plain` (`runs` says where that is), with
/// their ranges in `plain` (UTF-16). A Comment none of whose text is shown there is left out.
pub(super) fn comments_in(doc: &Doc, plain: &str, runs: &[(usize, usize, usize)]) -> Vec<CommentHighlight> {
    let comments = doc.comments();
    if comments.is_empty() || runs.is_empty() {
        return Vec::new();
    }
    // UTF-16 position of each character of `plain` (and its end).
    let mut utf16 = Vec::with_capacity(plain.len() + 1);
    let mut at = 0;
    for c in plain.chars() {
        utf16.push(at);
        at += c.len_utf16();
    }
    utf16.push(at);
    comments.into_iter().filter_map(|c| {
        let mut ranges = Vec::new();
        for &(from, to) in &c.ranges {
            let (from, to) = (committed_at(doc, from), committed_at(doc, to));
            for &(p, b, len) in runs {
                let (lo, hi) = (from.max(b), to.min(b + len));
                if lo < hi {
                    ranges.push((utf16[p + lo - b], utf16[p + hi - b]));
                }
            }
        }
        // Detached ones aren't shown on the Inclusion, and nor are Work comments: they're
        // about the included Work as a whole, so they show only in it.
        if ranges.is_empty() {
            return None;
        }
        let id = c.id.to_string();
        Some(crate::work::CommentHighlight {
            id, ranges, on_work: false, on_annotation: None, annotation_gone: false, sender: c.sender, name: String::new(), colour: String::new(), body: c.body,
            resolved: c.resolved, quote: c.quote, time_ms: c.hlc.0, other_body: None,
            replies: c.replies.into_iter().map(|r| crate::work::CommentReply {
                id: r.id.to_string(), sender: r.sender, name: String::new(), colour: String::new(),
                body: r.body, time_ms: r.hlc.0, other_body: None,
            }).collect(),
        })
    }).collect()
}

/// The included text as Typst that renders in the including Work's Look: in its own language,
/// a whole Work under its title (`heading`: the title, and the author's name if it's to be
/// shown) with its Chapters a level down, then its own Styles, kept apart from the text around
/// it by paragraph breaks. The title is labelled `<inclusion-title>` and the author's name is
/// `<inclusion-author>` metadata, so a Style ("Inclusion titles") can show them its own way.
///
/// With Styles of its own, the text after the title is bracketed by `SCOPE_START` (naming the
/// Work, `name`) and `SCOPE_END` metadata, invisible and moving nothing, so a `.docx` Export
/// can give it styles of its own ("Normal (Zampanò)"). Returns the Typst, and where in it (in
/// bytes) `text` starts.
fn wrap(text: &str, language: &str, heading: Option<(&str, Option<&str>)>, styles: &[String], name: &str) -> (String, usize) {
    let mut out = String::from("#[\n\n");
    if !language.trim().is_empty() {
        out.push_str(&format!("#set text({})\n", preamble::text_language(language)));
    }
    if let Some((title, author)) = heading {
        out.push_str(&format!("#heading(level: 1, {})<inclusion-title>\n", preamble::string(title)));
        if let Some(author) = author {
            out.push_str(&format!("#metadata({})<inclusion-author>\n", preamble::string(author)));
        }
        out.push_str("#set heading(offset: 1)\n");
    }
    let scoped = !styles.is_empty();
    if scoped {
        out.push_str(&format!("#metadata({})<{SCOPE_START}>\n", preamble::string(name)));
    }
    for style in styles {
        out.push_str(style);
        if !style.ends_with('\n') {
            out.push('\n');
        }
    }
    let at = out.len();
    out.push_str(text);
    out.push_str("\n\n");
    if scoped {
        out.push_str(&format!("#metadata(none)<{SCOPE_END}>\n"));
    }
    out.push(']');
    (out, at)
}

/// The labels of the markers around an Inclusion with Styles of its own (see `wrap`).
pub const SCOPE_START: &str = "wrtier-inclusion-start";
pub const SCOPE_END: &str = "wrtier-inclusion-end";

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

/// The Inclusion whose character is at `at` (UTF-16) in `text` (the editor's text, which may
/// hold the open burst), matched in order to `shown`.
pub(super) fn inclusion_at<'a>(text: &str, at: usize, shown: &'a [Shown]) -> Option<&'a Shown> {
    let mut utf16 = 0;
    let mut k = 0;
    for c in text.chars() {
        if c == INCLUSION {
            if utf16 == at {
                return shown.get(k);
            }
            k += 1;
        }
        utf16 += c.len_utf16();
    }
    None
}

/// An Annotation shown in an Inclusion inside this one, with its places moved by `by` (UTF-16).
fn shifted(note: &NoteShown, by: usize) -> NoteShown {
    NoteShown {
        at: note.at + by,
        ranges: note.ranges.iter().map(|&(a, b)| (a + by, b + by)).collect(),
        notes: note.notes.iter().map(|n| shifted(n, by)).collect(),
        ..note.clone()
    }
}

/// What the Text file writes out for each Inclusion.
pub fn expansions(shown: &[Shown]) -> storage::Expansions {
    shown.iter().map(|s| (s.at, inclusion_block(s.at, &s.label, &s.typst))).collect()
}

/// Where Comments are in the Work's Typst body (as `typst_body` writes it), as byte ranges:
/// `own`, the Work's own, with ranges in `text` (UTF-16), and each Inclusion's Comments on its
/// included text (`Shown::comments`), wherever that text came out. A Work comment has no
/// ranges; a Comment on text that isn't in the body (a missing Inclusion's) has none either,
/// and is left out.
pub fn comments_in_body(text: &str, shown: &[Shown], own: Vec<CommentHighlight>) -> Vec<(CommentHighlight, Vec<std::ops::Range<usize>>)> {
    let (body, blocks) = typst_body(text, shown);
    // UTF-16 position in the body → byte, for every position (an Export's worth of memory).
    let mut byte_at: Vec<usize> = Vec::with_capacity(body.len() + 1);
    for (b, c) in body.char_indices() {
        byte_at.extend(std::iter::repeat_n(b, c.len_utf16()));
    }
    byte_at.push(body.len());
    let byte = |u: usize| byte_at.get(u).copied().unwrap_or(body.len());
    // A place in `text` (UTF-16) → in the body: each block before it is longer than its one character.
    let in_body = |at: usize| at + blocks.iter().filter(|(b, _)| *b < at).map(|(_, len)| len - 1).sum::<usize>();
    let mut out = Vec::new();
    for highlight in own {
        let ranges = highlight.ranges.iter().map(|&(from, to)| byte(in_body(from))..byte(in_body(to))).collect();
        out.push((highlight, ranges));
    }
    for (s, (at, _)) in shown.iter().zip(&blocks) {
        if s.missing.is_some() {
            continue;
        }
        // The Inclusion's Typst starts on the line after its block's first line.
        let block = inclusion_block(s.at, &s.label, &s.typst);
        let typst_start = in_body(*at) + block[..block.find('\n').map_or(0, |n| n + 1)].encode_utf16().count();
        // A place in the included text as shown (`plain`) → in `typst`.
        let place = |p: usize, end: bool| s.in_typst.iter()
            .find(|(from, _, len)| if end { p > *from && p <= from + len } else { p >= *from && p < from + len })
            .map(|(from, t, _)| t + (p - from));
        for highlight in &s.comments {
            let ranges: Vec<_> = highlight.ranges.iter()
                .filter_map(|&(from, to)| Some(byte(typst_start + place(from, false)?)..byte(typst_start + place(to, true)?)))
                .collect();
            if !ranges.is_empty() {
                out.push((highlight.clone(), ranges));
            }
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
    /// The included Chapter's title, if it includes one Chapter and it was found.
    pub chapter: Option<String>,
    /// The included text, read-only.
    pub text: String,
    pub missing: Option<String>,
    pub language: String,
    pub whole: bool,
    /// Where the included text starts in its own Work (UTF-16): "Go to" opens it there.
    pub starts: usize,
    /// The included Work's Comments on its text here, ranges in `text` (UTF-16): they belong to that
    /// Work, and show in both.
    pub comments: Vec<CommentHighlight>,
    /// A new Chapter found right after the included one (see `Target::next`), if one.
    pub split: Option<SplitNote>,
    /// Styles given to this Inclusion only.
    pub styles: Vec<String>,
    /// The Annotations printed in the included text (by this Work, and by the Works it
    /// includes), places in `text` (UTF-16), in the order they print.
    pub annotations: Vec<NoteShown>,
}

/// A new Chapter right after an included one, not included: the included Chapter was split, or
/// a Chapter was put after it.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SplitNote {
    pub title: String,
    /// Where its heading starts in its Work's text (UTF-16), to include it too.
    pub at: usize,
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
                    at, work: s.name.clone(), label: s.label.clone(), chapter: s.chapter.clone(), text: s.plain.clone(),
                    missing: s.missing.clone(), language: s.language.clone(),
                    whole: s.target.as_ref().is_some_and(|t| t.chapter.is_none()),
                    starts: s.starts,
                    comments: s.comments.clone(),
                    split: s.split.clone(),
                    styles: s.target.as_ref().map(|t| t.styles.clone()).unwrap_or_default(),
                    annotations: s.notes.clone(),
                });
            }
        }
        at += c.len_utf16();
    }
    out
}

impl Vault {
    /// A `.db`'s Work ID (`storage::identity`), read once and kept while the file is unchanged.
    /// A different Work can come to have the same path (one renamed away, another made or moved
    /// there), and an empty one gets its ID with its first Change: either way the file's time
    /// differs, so it's read again.
    pub(super) fn identity_of(&mut self, db: &Path) -> Option<String> {
        let stamp = std::fs::metadata(db).and_then(|m| m.modified()).ok();
        if let Some((at, known)) = self.identities.get(db) {
            if stamp.is_some() && *at == stamp {
                return known.clone();
            }
        }
        let read = storage::identity(db).ok().flatten();
        self.identities.insert(db.to_path_buf(), (stamp, read.clone()));
        read
    }

    /// Works in the Vault (`dbs`, its Histories) that have the same Work ID, because one was
    /// copied by hand: a Sync problem until one is removed, since an Inclusion of it can't tell
    /// which is meant. One problem per Work, on the copies after the first.
    pub(super) fn duplicate_problems(&mut self, dbs: &[PathBuf]) -> Vec<super::SyncProblem> {
        let mut by_identity: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for db in dbs {
            if let Some(identity) = self.identity_of(db) {
                by_identity.entry(identity).or_default().push(db.clone());
            }
        }
        let mut groups: Vec<Vec<String>> = by_identity.into_values()
            .filter(|dbs| dbs.len() > 1)
            .map(|dbs| {
                // The one with the plainest name first ("Rain" before "Old/Rain copy"): it names them.
                let mut ids: Vec<String> = dbs.iter().map(|db| self.id_of(db)).collect();
                let short = |id: &String| id.rsplit('/').next().unwrap_or(id).chars().count();
                ids.sort_by_key(|id| (short(id), id.to_lowercase()));
                ids
            })
            .collect();
        groups.sort();
        groups.into_iter().map(|ids| {
            let name = ids[0].rsplit('/').next().unwrap_or(&ids[0]).to_string();
            let count = if ids.len() == 2 { "Two".to_string() } else { ids.len().to_string() };
            super::SyncProblem {
                path: ids[1..].join(", "),
                reason: format!(
                    "{count} Works claim to be {name} ({}): one was copied from the other outside the app. Move all but one to the trash, so it's clear which one Inclusions show.",
                    ids.join(", "),
                ),
                newer: false,
            }
        }).collect()
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
        let shelf = self.shelf();
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
        let shelf = self.shelf();
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
        let mut shown = self.shown_in(id, false)?;
        for s in &mut shown {
            let author = &s.author;
            for highlight in &mut s.comments {
                (highlight.name, highlight.colour) = self.included_sender(author, &highlight.sender);
                for reply in &mut highlight.replies {
                    (reply.name, reply.colour) = self.included_sender(author, &reply.sender);
                }
            }
        }
        let me = self.works.get(&self.current_id(id)).and_then(|w| w.identity()).unwrap_or_default().to_string();
        for s in &mut shown {
            self.name_owners(&mut s.notes, &me);
        }
        let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
        Ok(views(&work.text(), &shown))
    }

    /// The name and colour of a Comment's sender in an included Work (`author`: its author name).
    pub(super) fn included_sender(&self, author: &str, sender: &str) -> (String, String) {
        match sender.strip_prefix("reviewer/") {
            None => (if author.trim().is_empty() { "Author".into() } else { author.to_string() }, "author".into()),
            Some(id) => match self.reviewer(id) {
                Some(r) => (r.name, r.colour),
                None => ("Unknown reviewer".into(), "unknown".into()),
            },
        }
    }

    /// A new Comment on included text, made in the including Work `id` but kept in the Work it
    /// includes (ADR 0015): `at` is where the Inclusion's character is in `id`'s text, `from..to`
    /// what was selected in the included text as shown (UTF-16). It can't reach past the
    /// included Work's own text (into its own Inclusions).
    pub fn comment_on_inclusion(&mut self, id: &str, at: usize, from: usize, to: usize, sender: &str, body: &str, now_ms: u64) -> Result<(), String> {
        let shown = self.shown_in(id, false)?;
        let text = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?.text();
        let s = inclusion_at(&text, at, &shown).ok_or("That Inclusion isn't there any more.")?;
        let work = s.name.clone().ok_or("The Work it includes can't be found.")?;
        let chars = |p: usize| crate::positions::Positions::new(&s.plain).to_char(p).map_err(|e| e.to_string());
        let (from, to) = (chars(from)?, chars(to)?);
        if from >= to {
            return Err("Select some of the included text to comment on.".into());
        }
        let &(p, b, _) = s.runs.iter().find(|&&(p, _, len)| from >= p && to <= p + len)
            .ok_or("A Comment can't reach past the edge of the included text: select less.")?;
        let (from, to) = (b + from - p, b + to - p);
        self.edit_other(&work, now_ms, |text| {
            let positions = crate::positions::Positions::new(text);
            vec![Op::Comment { from: positions.to_utf16(from), to: positions.to_utf16(to), sender: sender.into(), body: body.into(), sender_name: None }]
        })
    }

    /// The Author saw the note about a new Chapter after the Inclusion whose character is at
    /// `at` (UTF-16) in `id` (and included it too, or dismissed it): the Inclusion now remembers
    /// the heading that follows its Chapter now, so the note goes.
    pub fn settle_split(&mut self, id: &str, at: usize, now_ms: u64) -> Result<(), String> {
        let shown = self.shown_in(id, false)?;
        let key = self.current_id(id);
        let work = self.works.get(&key).ok_or_else(|| format!("{id} isn't open."))?;
        let found = inclusion_at(&work.text(), at, &shown).ok_or("That Inclusion isn't there any more.")?;
        let Some(target) = &found.target else { return Ok(()) };
        if found.split.is_none() {
            return Ok(());
        }
        // The heading after the Chapter now, in the included Work as it is now.
        let index = self.work_index();
        let located = index.get(&target.work).cloned().ok_or("The Work it includes can't be found.")?;
        let shelf = self.shelf();
        let next = {
            let mut ctx = Ctx { works: &self.works, index: &index, cache: &mut self.included_docs, shelf: &shelf, device: self.device, recorded: false };
            let doc = ctx.doc(&located, None)?;
            let chars: Vec<char> = doc.committed_text().chars().collect();
            let start = target.chapter.and_then(|c| doc.index_of_char(c)).ok_or("The Chapter it includes is gone.")?;
            let (_, end) = chapter_range(&chars, start).ok_or("The Chapter it includes is gone.")?;
            doc.visible().get(end).map(|&(id, _)| id)
        };
        let Some(next) = next else { return Ok(()) };
        let record = inclusion_record(found.at);
        self.with_work(id, now_ms, |w, _| {
            w.close_burst(now_ms);
            w.record_fields(&[(record, "next".into(), next.to_string())], now_ms);
            w.save();
            Ok(())
        })
    }

    /// Gives the Inclusion whose character is at `at` (UTF-16) in `id` these Styles (from the
    /// Style library, by name), for it only, on top of the Work's Look. Recorded in `id`, like
    /// what it includes, so Milestones keep them; the Styles' content is recorded on the next edit.
    pub fn set_inclusion_styles(&mut self, id: &str, at: usize, styles: &[String], now_ms: u64) -> Result<(), String> {
        if styles.iter().any(|s| s.trim().is_empty() || s.contains('\n')) {
            return Err("Not a Style's name.".into());
        }
        let shown = self.shown_in(id, false)?;
        let key = self.current_id(id);
        let text = self.works.get(&key).ok_or_else(|| format!("{id} isn't open."))?.text();
        let found = inclusion_at(&text, at, &shown).ok_or("That Inclusion isn't there any more.")?;
        let record = inclusion_record(found.at);
        let shelf = self.shelf();
        let generation = self.library_generation;
        self.with_work(id, now_ms, |w, _| {
            w.close_burst(now_ms);
            w.record_fields(&[(record, "styles".into(), styles.join("\n"))], now_ms);
            // The Styles' content too, now, so the Text file and Exports have them.
            w.record_library(&shelf, generation, now_ms);
            w.save();
            Ok(())
        })?;
        self.refresh_text_file_inclusions(id);
        Ok(())
    }

    /// Comment ops (by Comment ID: Reply, Resolve, Edit, Delete…) on a Work that may not be
    /// open, from the Comments panel of a Work that includes it.
    pub fn comment_in(&mut self, work: &str, ops: Vec<Op>, now_ms: u64) -> Result<(), String> {
        if ops.iter().any(|op| !matches!(op, Op::Reply { .. } | Op::EditComment { .. } | Op::SetCommentSender { .. } | Op::Resolve { .. } | Op::DeleteComment { .. })) {
            return Err("Only a Comment's Replies and settings can be changed this way.".into());
        }
        self.edit_other(work, now_ms, |_| ops)
    }

    /// Edits another Work (`work`: its ID in the Vault) from elsewhere: opened here for it if it
    /// isn't open, and closed again. Its open burst closes first, so `ops` (made from its text by
    /// `make`) see the text as its History has it.
    fn edit_other(&mut self, work: &str, now_ms: u64, make: impl FnOnce(&str) -> Vec<Op>) -> Result<(), String> {
        let opened_here = !self.works.contains_key(&self.current_id(work));
        if opened_here {
            self.open_work(work, now_ms)?;
        }
        let result = (|| {
            let text = self.with_work(work, now_ms, |w, _| {
                w.close_burst(now_ms);
                w.save();
                Ok(w.text())
            })?;
            self.edit(work, make(&text), now_ms).map(|_| ())
        })();
        if opened_here {
            self.close_work(work, now_ms);
        }
        result
    }

    /// The Chapters of a Work (an ID in the Vault), to pick one to include: each one's title and
    /// where its heading starts (UTF-16).
    pub fn chapters_of(&mut self, work: &str) -> Result<Vec<ChapterChoice>, String> {
        let (db, _) = self.paths(work)?;
        if !db.exists() {
            return Err(format!("{work} isn't in the Vault any more."));
        }
        let text = match self.works.values().find(|w| w.db == db) {
            Some(w) => w.doc.committed_text(),
            None => storage::load(&db, self.device).map_err(err)?.1.committed_text(),
        };
        let mut found = Vec::new();
        let mut at = 0;
        for line in text.split_inclusive('\n') {
            if let Some(title) = line.strip_prefix("= ") {
                let title = wrtier::footnotes::without_footnotes(title).trim().replace(['*', '_', '`'], "");
                found.push(ChapterChoice { title: if title.is_empty() { "Untitled".into() } else { title }, at });
            }
            at += line.encode_utf16().count();
        }
        Ok(found)
    }

    /// Can the Work `work` (an ID in the Vault) be included in the open Work `id`? Not if that
    /// would make a Work include itself (ADR 0015): `id` itself, or a Work that already includes
    /// `id` (directly, or through Works it includes). Asked before the Inclusion is put in the
    /// text, so a refusal changes nothing.
    pub fn check_inclusion(&mut self, id: &str, work: &str) -> Result<(), String> {
        let target = self.target_for(work, None)?;
        self.refuse_loop(id, work, &target)
    }

    /// `check_inclusion`, for a Target already worked out.
    pub(super) fn refuse_loop(&mut self, id: &str, work: &str, target: &Target) -> Result<(), String> {
        let Some(me) = self.works.get(&self.current_id(id)).and_then(|w| w.identity()).map(str::to_string) else { return Ok(()) };
        let name = work.rsplit('/').next().unwrap_or(work);
        if target.work == me {
            return Err("A Work can't include itself.".into());
        }
        let index = self.work_index();
        let mut seen = std::collections::HashSet::new();
        let mut todo = vec![target.work.clone()];
        while let Some(next) = todo.pop() {
            if !seen.insert(next.clone()) {
                continue;
            }
            let Some(located) = index.get(&next) else { continue };
            let read;
            let doc = match self.works.values().find(|w| w.db == located.db) {
                Some(open) => &open.doc,
                None => match storage::load(&located.db, self.device) {
                    Ok((_, doc)) => {
                        read = doc;
                        &read
                    }
                    Err(_) => continue, // can't be read now: nothing it includes can be known
                },
            };
            let includes: Vec<String> = doc.inclusions().into_iter().filter_map(|(at, _)| Target::of(doc, at)).map(|t| t.work).collect();
            if includes.contains(&me) {
                return Err(format!("{name} already includes this Work, so including it here would make it include itself."));
            }
            todo.extend(includes);
        }
        Ok(())
    }

    /// "New Collection from selected Works…": a new Work `id` holding an Inclusion of each of
    /// `works` (IDs in the Vault), whole and in that order, with a blank line between them.
    /// Every one is checked before anything is made.
    pub fn create_collection(&mut self, id: &str, works: &[String], now_ms: u64) -> Result<crate::vault::OpenedWork, String> {
        if works.is_empty() {
            return Err("Select the Works to collect first.".into());
        }
        for work in works {
            self.target_for(work, None)?;
        }
        let created = self.create_work(id, now_ms)?;
        // One character each, on lines of their own: at 0, 3, 6…
        let mut ops = vec![Op::Paste { from: 0, to: 0, text: vec![INCLUSION.to_string(); works.len()].join("\n\n") }];
        ops.extend(works.iter().enumerate().map(|(k, work)| Op::Include { at: 3 * k, work: work.clone(), chapter: None, target: None }));
        self.edit(&created.id, ops, now_ms)?;
        let text = self.works.get(&self.current_id(&created.id)).map(|w| w.text()).unwrap_or_default();
        Ok(crate::vault::OpenedWork { text, ..created })
    }

    /// What an Inclusion of the Work `work` (an ID in the Vault) should record: its ID, and the
    /// character starting the heading of the Chapter that starts at `chapter` (a position in
    /// its text, UTF-16), if one.
    pub(super) fn target_for(&mut self, work: &str, chapter: Option<usize>) -> Result<Target, String> {
        let (db, _) = self.paths(work)?;
        // (Loading a `.db` that isn't there would make an empty one.)
        if !db.exists() {
            return Err(format!("{work} isn't in the Vault any more."));
        }
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
        let (chapter, next) = match chapter {
            None => (None, None),
            Some(at) => {
                let text = doc.committed_text();
                let index = crate::positions::Positions::new(&text).to_char(at).map_err(|e| e.to_string())?;
                let chars: Vec<char> = text.chars().collect();
                let (_, end) = chapter_range(&chars, index).ok_or("There's no Chapter heading there.")?;
                let visible = doc.visible();
                (Some(visible[index].0), visible.get(end).map(|&(id, _)| id))
            }
        };
        Ok(Target { work: identity, chapter, styles: Vec::new(), next })
    }
}

/// A Work's ID under `base` (the trash): its path from there, without the extension.
pub(super) fn relative_id(base: &Path, db: &Path) -> String {
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
    use crate::vault::Export;
    use crate::work::Origin;
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
    fn asking_for_the_inclusions_again_reads_neither_the_library_nor_the_vault_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        v.inclusions("Collected").unwrap();
        let (reads, index) = (v.shelves.reads(), v.work_index());
        v.inclusions("Collected").unwrap();
        v.inclusions("Collected").unwrap();
        assert_eq!(v.shelves.reads(), reads, "the Style library isn't read again");
        assert!(Arc::ptr_eq(&index, &v.work_index()), "nor is the Vault walked again");

        // Once the library changes, it's read again (once).
        std::fs::write(dir.path().join("styles/Mine.typ"), "// Style: Mine\n").unwrap();
        assert!(v.library_changed());
        v.inclusions("Collected").unwrap();
        v.inclusions("Collected").unwrap();
        assert_eq!(v.shelves.reads(), reads + 1);
    }

    #[test]
    fn the_timer_and_other_commands_on_other_threads_keep_the_typing_in_order() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::Mutex;
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(Mutex::new(collected(dir.path())));
        let clock = Arc::new(AtomicU64::new(20_000));
        let stop = Arc::new(AtomicBool::new(false));
        let run = |f: fn(&mut Vault, u64)| {
            let (vault, clock, stop) = (vault.clone(), clock.clone(), stop.clone());
            std::thread::spawn(move || while !stop.load(Ordering::Relaxed) {
                let now = clock.fetch_add(10, Ordering::Relaxed);
                f(&mut vault.lock().unwrap(), now);
            })
        };
        let others = [
            run(|v, now| { v.tick(now); }),
            run(|v, _| { v.inclusions("Collected").unwrap(); }),
            run(|v, now| { v.list(now); }),
        ];
        for _ in 0..40 {
            let now = clock.fetch_add(10, Ordering::Relaxed);
            vault.lock().unwrap().edit("Collected", vec![typing(0, "a")], now).unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        for thread in others {
            thread.join().expect("no thread panicked");
        }
        let mut v = Arc::try_unwrap(vault).ok().unwrap().into_inner().unwrap();
        let (text, _) = v.work_text("Collected", clock.load(Ordering::Relaxed)).unwrap();
        assert!(text.starts_with(&format!("{}My introduction.", "a".repeat(40))), "{text}");
        // And it's all saved: reopened, it's the same.
        v.close_work("Collected", clock.load(Ordering::Relaxed));
        let mut again = open(dir.path());
        assert_eq!(again.open_work("Collected", clock.load(Ordering::Relaxed)).unwrap().text, text);
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
        v.open_work("Rain", 12_000).unwrap();
        // Including "Collected" back in "Rain" is refused, before anything changes, and so is
        // including a Work in itself.
        let refused = v.check_inclusion("Rain", "Collected").unwrap_err();
        assert!(refused.contains("Collected already includes this Work"), "{refused}");
        assert_eq!(v.check_inclusion("Collected", "Collected").unwrap_err(), "A Work can't include itself.");
        assert!(v.check_inclusion("Collected", "Rain").is_ok(), "one more of it is fine");
        assert!(v.edit("Rain", vec![
            Op::Paste { from: 0, to: 0, text: INCLUSION.to_string() },
            Op::Include { at: 0, work: "Collected".into(), chapter: None, target: None },
        ], 12_050).is_err());
        // A loop can still arrive by merging (each Device made its half): as if, here.
        let target = v.target_for("Collected", None).unwrap();
        v.with_work("Rain", 12_100, |w, name| w.edit(vec![
            Op::Paste { from: 0, to: 0, text: INCLUSION.to_string() },
            Op::Include { at: 0, work: "Collected".into(), chapter: None, target: Some(target) },
        ], name, None, 12_100)).unwrap();
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
    fn a_comment_on_included_text_is_kept_in_the_included_work_and_shown_in_both() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        // Commented in "Collected", on "rain had" in the included text ("Rain" isn't open).
        let from = RAIN.find("rain had").unwrap();
        v.comment_on_inclusion("Collected", 18, from, from + 8, "author", "Too plain?", 12_000).unwrap();
        assert!(!v.works.contains_key("Rain"), "opened only to record it, then closed");
        let (_, rain) = storage::load(&dir.path().join("Rain.db"), crate::vault::tests_support::ME).unwrap();
        let comments = rain.comments();
        assert_eq!((comments.len(), comments[0].quote.as_str(), comments[0].body.as_str()), (1, "rain had", "Too plain?"));
        assert!(storage::load(&dir.path().join("Collected.db"), crate::vault::tests_support::ME).unwrap().1.comments().is_empty());
        // Shown on the Inclusion, where it is in the included text.
        let views = v.inclusions("Collected").unwrap();
        let highlight = &views[0].comments[0];
        assert_eq!((highlight.ranges.clone(), highlight.name.as_str(), highlight.colour.as_str()), (vec![(from, from + 8)], "Author", "author"));
        // Replied to from "Collected": also kept in "Rain".
        let reply = Op::Reply { comment: highlight.id.clone(), sender: "author".into(), body: "Leave it.".into(), sender_name: None };
        v.comment_in("Rain", vec![reply], 13_000).unwrap();
        assert_eq!(v.inclusions("Collected").unwrap()[0].comments[0].replies[0].body, "Leave it.");
        // A Comment made in "Rain" itself shows on the Inclusion too, and one outside it doesn't.
        v.open_work("Rain", 14_000).unwrap();
        let went = RAIN.find("went").unwrap();
        v.edit("Rain", vec![Op::Comment { from: went, to: went + 4, sender: "author".into(), body: "Where?".into(), sender_name: None }], 14_100).unwrap();
        assert_eq!(v.inclusions("Collected").unwrap()[0].comments.len(), 2);
        // Typing in "Rain" before them (its burst still open) doesn't shift them on the
        // Inclusion, which shows what "Rain" has recorded.
        v.edit("Rain", vec![typing(0, "Prologue. ")], 15_000).unwrap();
        assert!(v.works["Rain"].doc.burst().is_some());
        let views = v.inclusions("Collected").unwrap();
        assert_eq!(views[0].comments[0].ranges, vec![(from, from + 8)]);
        assert_eq!(views[0].comments[1].ranges, vec![(went, went + 4)]);
    }

    #[test]
    fn an_annotation_prints_as_a_footnote_on_included_text_and_lives_in_the_including_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        let from = RAIN.find("rain had").unwrap();
        let ann = v.annotate("Collected", 18, from, from + 8, "Truant on _this_.", 12_000).unwrap();
        // Shown on the Inclusion, after what it covers, as this Work's own.
        let views = v.inclusions("Collected").unwrap();
        let note = &views[0].annotations[0];
        assert_eq!((note.id.as_str(), note.at, note.ranges.clone(), note.mine), (ann.as_str(), from + 8, vec![(from, from + 8)], true));
        assert_eq!((note.quote.as_str(), note.body.as_str(), note.owner_label.as_str()), ("rain had", "Truant on _this_.", "Collected"));
        // Printed right there, and it compiles.
        let (body, header, blocks) = v.typeset_input("Collected", None, 12_100).unwrap();
        assert!(body.contains("The rain had#footnote[Truant on _this_.] stopped."), "{body}");
        compiles(&header, body, blocks);
        // Kept in "Collected", not in "Rain".
        let rain = storage::load(&dir.path().join("Rain.db"), crate::vault::tests_support::ME).unwrap().1;
        assert!(rain.records("annotation/").is_empty());
        // Changed, then undone: it reads as before; undone again, it's gone.
        v.set_annotation_body("Collected", &ann, "Changed.", 13_000).unwrap();
        assert_eq!(v.inclusions("Collected").unwrap()[0].annotations[0].body, "Changed.");
        v.edit("Collected", vec![Op::Undo], 13_100).unwrap();
        assert_eq!(v.inclusions("Collected").unwrap()[0].annotations[0].body, "Truant on _this_.");
        v.edit("Collected", vec![Op::Undo], 13_200).unwrap();
        assert!(v.inclusions("Collected").unwrap()[0].annotations.is_empty());
        // And redone.
        v.edit("Collected", vec![Op::Redo], 13_300).unwrap();
        assert_eq!(v.inclusions("Collected").unwrap()[0].annotations.len(), 1);
        // Brackets that don't pair up are refused.
        assert!(v.set_annotation_body("Collected", &ann, "a ] b", 13_400).is_err());
        // Deleted: gone, and not detached.
        v.delete_annotation("Collected", &ann, 13_500).unwrap();
        assert!(v.inclusions("Collected").unwrap()[0].annotations.is_empty());
        assert!(v.detached_annotations("Collected").unwrap().is_empty());
    }

    #[test]
    fn an_annotation_follows_its_text_and_detaches_while_it_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        let went = RAIN.find("went home").unwrap();
        let ann = v.annotate("Collected", 18, went, went + 9, "Where?", 12_000).unwrap();
        // Text typed before it in "Rain" moves it along.
        v.open_work("Rain", 13_000).unwrap();
        v.edit("Rain", vec![typing(0, "Prologue. ")], 13_100).unwrap();
        v.tick(30_000);
        assert_eq!(v.inclusions("Collected").unwrap()[0].annotations[0].at, went + 9 + 10);
        // All of its text deleted in "Rain": detached, not printed, listed with its quote.
        let at = 10 + went;
        v.edit("Rain", vec![Op::Delete { from: at, to: at + 9, origin: Origin::Input }], 31_000).unwrap();
        v.tick(60_000);
        assert!(v.inclusions("Collected").unwrap()[0].annotations.is_empty());
        let (body, _, _) = v.typeset_input("Collected", None, 61_000).unwrap();
        assert!(!body.contains("Where?"));
        let detached = v.detached_annotations("Collected").unwrap();
        assert_eq!(detached.len(), 1);
        assert_eq!((detached[0].id.as_str(), detached[0].quote.as_str(), detached[0].label.as_str()), (ann.as_str(), "went home", "Rain"));
        // The deletion undone in "Rain": attached again.
        v.edit("Rain", vec![Op::Undo], 62_000).unwrap();
        assert_eq!(v.inclusions("Collected").unwrap()[0].annotations.len(), 1);
        assert!(v.detached_annotations("Collected").unwrap().is_empty());
    }

    #[test]
    fn deleting_an_inclusion_detaches_its_annotations_and_including_again_reattaches_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        let from = RAIN.find("rain").unwrap();
        v.annotate("Collected", 18, from, from + 4, "Rain, of course.", 12_000).unwrap();
        v.edit("Collected", vec![Op::Delete { from: 18, to: 19, origin: Origin::Input }], 13_000).unwrap();
        v.tick(30_000);
        assert_eq!(v.detached_annotations("Collected").unwrap().len(), 1);
        include(&mut v, "Collected", 18, "Rain", None, 31_000);
        assert!(v.detached_annotations("Collected").unwrap().is_empty());
        assert_eq!(v.inclusions("Collected").unwrap()[0].annotations[0].body, "Rain, of course.");
    }

    #[test]
    fn annotations_print_where_their_work_is_included_and_can_be_annotated_from_there() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        let from = RAIN.find("stopped").unwrap();
        let truant = v.annotate("Collected", 18, from, from + 7, "Truant on Zampano.", 12_000).unwrap();
        v.tick(30_000);
        v.close_work("Collected", 31_000);
        // "Book" includes "Collected", which includes "Rain".
        v.create_work("Book", 32_000).unwrap();
        v.edit("Book", vec![typing(0, "Editors.\n\n")], 32_100).unwrap();
        v.tick(50_000);
        include(&mut v, "Book", 10, "Collected", None, 51_000);
        let views = v.inclusions("Book").unwrap();
        let shown = &views[0];
        let note = &shown.annotations[0];
        assert_eq!((note.body.as_str(), note.mine, note.owner_label.as_str()), ("Truant on Zampano.", false, "Collected"));
        assert_eq!(&shown.text[note.ranges[0].0..note.ranges[0].1], "stopped");
        // The Editors on Zampano's words, two Inclusions down.
        let went = shown.text.find("went").unwrap();
        v.annotate("Book", 10, went, went + 4, "Editors on Zampano.", 52_000).unwrap();
        // And on Truant's note itself, printed at the end of its footnote.
        let owner = note.owner.clone();
        v.annotate_annotation("Book", &owner, &truant, "Editors on Truant.", 53_000).unwrap();
        let views = v.inclusions("Book").unwrap();
        let notes = &views[0].annotations;
        assert_eq!(notes.iter().map(|n| n.body.as_str()).collect::<Vec<_>>(), vec!["Truant on Zampano.", "Editors on Zampano."]);
        assert_eq!(notes[0].notes[0].body, "Editors on Truant.");
        assert!(notes[1].mine && notes[0].notes[0].owner != owner && notes[0].notes[0].mine);
        let (body, header, blocks) = v.typeset_input("Book", None, 54_000).unwrap();
        assert!(body.contains("stopped#footnote[Truant on Zampano.#footnote[Editors on Truant.]]."), "{body}");
        assert!(body.contains("went#footnote[Editors on Zampano.] home"), "{body}");
        compiles(&header, body, blocks);
        // Neither of the Editors' notes is in "Collected".
        v.open_work("Collected", 55_000).unwrap();
        let theirs = v.inclusions("Collected").unwrap();
        assert_eq!(theirs[0].annotations.len(), 1);
        assert!(theirs[0].annotations[0].notes.is_empty());
    }

    #[test]
    fn a_replace_all_in_the_text_and_annotations_is_one_action() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        let from = RAIN.find("rain").unwrap();
        let ann = v.annotate("Collected", 18, from, from + 4, "The rain again.", 12_000).unwrap();
        // "My introduction." → "My preface." in the text, and in the Annotation.
        let replace = Op::Replace {
            edits: vec![crate::work::ReplaceOpEdit { from: 3, to: 15, text: "preface".into() }],
            find: "introduction".into(), with: "preface".into(), count: 2,
            annotations: vec![crate::work::AnnotationEdit { id: ann.clone(), body: "The preface again.".into() }],
        };
        v.edit("Collected", vec![replace], 13_000).unwrap();
        assert!(v.works["Collected"].text().starts_with("My preface."));
        assert_eq!(v.inclusions("Collected").unwrap()[0].annotations[0].body, "The preface again.");
        v.edit("Collected", vec![Op::Undo], 14_000).unwrap();
        assert!(v.works["Collected"].text().starts_with("My introduction."));
        assert_eq!(v.inclusions("Collected").unwrap()[0].annotations[0].body, "The rain again.", "one undo takes both back");
        // Only in an Annotation: still one action, which undo takes back.
        let only = Op::Replace {
            edits: Vec::new(), find: "rain".into(), with: "snow".into(), count: 1,
            annotations: vec![crate::work::AnnotationEdit { id: ann.clone(), body: "The snow again.".into() }],
        };
        v.edit("Collected", vec![only], 15_000).unwrap();
        assert_eq!(v.inclusions("Collected").unwrap()[0].annotations[0].body, "The snow again.");
        v.edit("Collected", vec![Op::Undo], 16_000).unwrap();
        assert_eq!(v.inclusions("Collected").unwrap()[0].annotations[0].body, "The rain again.");
    }

    #[test]
    fn a_comment_on_an_annotation_belongs_to_its_work_and_detaches_while_it_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        let from = RAIN.find("rain").unwrap();
        let ann = v.annotate("Collected", 18, from, from + 4, "A note.", 12_000).unwrap();
        let op = Op::AnnotationComment { annotation: ann.clone(), quote: "A note.".into(), sender: "author".into(), body: "Cut it?".into(), sender_name: None };
        let state = v.edit("Collected", vec![op, Op::Highlights], 13_000).unwrap();
        let comment = state.highlights.unwrap().remove(0);
        assert_eq!((comment.on_annotation.as_deref(), comment.on_work, comment.annotation_gone), (Some(ann.as_str()), false, false));
        v.delete_annotation("Collected", &ann, 14_000).unwrap();
        let state = v.edit("Collected", vec![Op::Highlights], 15_000).unwrap();
        assert!(state.highlights.unwrap()[0].annotation_gone, "detached while the Annotation is deleted");
        // A Comment on an Annotation that isn't there is refused.
        let gone = Op::AnnotationComment { annotation: ann.clone(), quote: String::new(), sender: "author".into(), body: "?".into(), sender_name: None };
        assert!(v.edit("Collected", vec![gone], 16_000).is_err());
    }

    #[test]
    fn a_milestone_prints_exactly_the_annotations_it_had() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        let from = RAIN.find("rain").unwrap();
        let ann = v.annotate("Collected", 18, from, from + 4, "Before.", 12_000).unwrap();
        v.make_milestone("Collected", "Sent", 13_000).unwrap();
        let sent = v.milestones("Collected", 13_100).unwrap()[0].id.clone();
        v.set_annotation_body("Collected", &ann, "After.", 14_000).unwrap();
        let went = RAIN.find("went").unwrap();
        v.annotate("Collected", 18, went, went + 4, "New.", 14_100).unwrap();
        let (then, header, blocks) = v.typeset_input_at("Collected", &sent, None, 15_000).unwrap();
        assert!(then.contains("#footnote[Before.]") && !then.contains("After.") && !then.contains("New."), "{then}");
        compiles(&header, then, blocks);
        // The Text file prints them too, and still compiles on its own.
        v.close_work("Collected", 16_000);
        let file = std::fs::read_to_string(dir.path().join("Collected.typ")).unwrap();
        assert!(file.contains("rain#footnote[After.]") && file.contains("went#footnote[New.]"), "{file}");
        let mut t = crate::typeset::Typesetter::new(false);
        assert!(t.compile("file", Path::new("none"), file).errors.is_empty());
    }

    /// The text each Word comment in a `.docx` covers, by its comment's text.
    fn commented(docx: &[u8]) -> Vec<(String, String)> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
        let mut read = |name: &str| {
            let mut xml = String::new();
            std::io::Read::read_to_string(&mut zip.by_name(name).unwrap(), &mut xml).unwrap();
            xml
        };
        let (document, comments) = (read("word/document.xml"), read("word/comments.xml"));
        let (document, comments) = (roxmltree::Document::parse(&document).unwrap(), roxmltree::Document::parse(&comments).unwrap());
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        comments.descendants().filter(|n| n.has_tag_name("comment")).map(|c| {
            let id = c.attribute((w, "id")).unwrap();
            let body: String = c.descendants().filter(|n| n.has_tag_name("t")).filter_map(|n| n.text()).collect();
            let mut inside = false;
            let mut text = String::new();
            for n in document.descendants() {
                let this = n.attribute((w, "id")) == Some(id);
                if n.has_tag_name("commentRangeStart") && this {
                    inside = true;
                } else if n.has_tag_name("commentRangeEnd") && this {
                    break;
                } else if inside && n.has_tag_name("t") {
                    text.push_str(n.text().unwrap_or(""));
                }
            }
            (body, text)
        }).collect()
    }

    #[test]
    fn a_docx_report_says_which_lines_hold_what_it_cant_carry() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        // An equation in the included Work; one and a drawn line after the Inclusion.
        v.open_work("Rain", 12_000).unwrap();
        let end = RAIN.find("stopped.").unwrap() + "stopped.".len();
        v.edit("Rain", vec![typing(end, " Then $x$.")], 12_100).unwrap();
        v.open_work("Collected", 12_200).unwrap();
        v.edit("Collected", vec![typing(19, "\n\nAfter it, $y$.\n\n#line(length: 1cm)")], 12_300).unwrap();
        v.tick(20_000);
        let input = v.export_input("Collected", Export::default(), 21_000).unwrap();
        let (header, options) = crate::docx::prepare(&input, "Collected", false, false);
        let made = crate::typeset::Typesetter::new(false).export_docx(Path::new("none"), &header, input.text.clone(), &input.inclusions, &options).unwrap();
        let report: Vec<String> = crate::docx::report(&made.docx.report, &made.lines, &input.made, false).into_iter().map(|l| l.text).collect();
        assert!(report.contains(&"Equations on lines 3 (in included text) and 5 are kept as plain text, without their layout.".to_string()), "{report:?}");
        assert!(report.contains(&"A drawn line or shape on line 7 is left out.".to_string()), "lines after the Inclusion count as the Work's own: {report:?}");
    }

    #[test]
    fn a_docx_carries_the_comments_on_its_own_and_its_included_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        // In the Collection: one on its own text, with a Reply; one on the whole Work; a resolved one.
        v.open_work("Collected", 12_000).unwrap();
        let comment = |from: usize, to: usize, body: &str| Op::Comment { from, to, sender: "author".into(), body: body.into(), sender_name: None };
        v.edit("Collected", vec![comment(3, 15, "Shorter?"), comment(0, 2, "Done.")], 12_100).unwrap();
        v.edit("Collected", vec![Op::WorkComment { sender: "author".into(), body: "Whole thing.".into(), sender_name: None }], 12_200).unwrap();
        let highlights = v.works["Collected"].highlights();
        let id = |body: &str| highlights.iter().find(|m| m.body == body).unwrap().id.clone();
        v.edit("Collected", vec![Op::Reply { comment: id("Shorter?"), sender: "author".into(), body: "No.".into(), sender_name: None }], 12_300).unwrap();
        v.edit("Collected", vec![Op::Resolve { id: id("Done."), resolved: true }], 12_400).unwrap();
        // In "Rain", which it includes, after a Chapter heading of its own.
        v.open_work("Rain", 13_000).unwrap();
        let went = RAIN.find("went home").unwrap();
        v.edit("Rain", vec![comment(went, went + 9, "Where?")], 13_100).unwrap();
        v.tick(20_000);

        let input = v.export_input("Collected", Export { comments: true, ..Export::default() }, 21_000).unwrap();
        let mut found: Vec<(String, String)> = input.comments.iter()
            .map(|(m, ranges)| (m.body.clone(), ranges.iter().map(|r| input.text[r.clone()].to_string()).collect()))
            .collect();
        found.sort();
        assert_eq!(found, [
            ("Shorter?".to_string(), "introduction".to_string()),
            ("Where?".to_string(), "went home".to_string()),
            ("Whole thing.".to_string(), String::new()),
        ], "in the body as compiled; the resolved one isn't");

        let header = crate::docx::with_probe(&input.header);
        let options = typst_docx::Options {
            targets: crate::docx::targets(&input.header.elements, &input.made),
            comments: crate::docx::comments(input.comments.clone(), header.text.len()),
            ..typst_docx::Options::default()
        };
        let made = crate::typeset::Typesetter::new(false).export_docx(Path::new("none"), &header, input.text, &input.inclusions, &options).unwrap().docx;
        assert!(!made.report.iter().any(|n| matches!(n.what, typst_docx::What::CommentNotPlaced { .. })), "{:?}", made.report);
        let mut placed = commented(&made.bytes);
        placed.sort();
        assert_eq!(placed, [
            ("No.".to_string(), "introduction".to_string()),
            ("Shorter?".to_string(), "introduction".to_string()),
            ("Where?".to_string(), "went home".to_string()),
            ("Whole thing.".to_string(), String::new()),
        ]);
        // Without being asked, or from a Milestone, none.
        assert!(v.export_input("Collected", Export::default(), 22_000).unwrap().comments.is_empty());
    }

    #[test]
    fn a_comment_on_included_text_stops_at_the_included_works_own_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        let two = RAIN.find("= Two").unwrap();
        include(&mut v, "Collected", 0, "Rain", Some(two), 12_000);
        // Only the Chapter is shown: positions are in what's shown, not in the whole story.
        let shown = &v.inclusions("Collected").unwrap()[0];
        let at = shown.text.find("went").unwrap();
        v.comment_on_inclusion("Collected", 0, at, at + 4, "author", "Here.", 13_000).unwrap();
        let (_, rain) = storage::load(&dir.path().join("Rain.db"), crate::vault::tests_support::ME).unwrap();
        assert_eq!(rain.comments()[0].quote, "went");
        // Nothing selected, or past the end: refused.
        assert!(v.comment_on_inclusion("Collected", 0, at, at, "author", "x", 13_100).is_err());
        assert!(v.comment_on_inclusion("Collected", 0, at, 999, "author", "x", 13_200).is_err());
    }

    #[test]
    fn a_chapter_split_in_its_own_work_shows_a_note_until_it_is_settled() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        include(&mut v, "Collected", 0, "Rain", Some(0), 12_000); // Chapter One
        assert_eq!(v.inclusions("Collected").unwrap()[0].split, None);
        // In "Rain", Chapter One is split: a new heading in the middle of it.
        v.open_work("Rain", 13_000).unwrap();
        let rain_at = RAIN.find("The rain").unwrap();
        v.edit("Rain", vec![typing(rain_at, "= Interlude\n\n")], 13_100).unwrap();
        v.tick(20_000);
        let view = &v.inclusions("Collected").unwrap()[0];
        assert_eq!(view.text, "= One\n\n", "only the part before the new heading is included now");
        assert_eq!(view.split, Some(SplitNote { title: "Interlude".into(), at: rain_at }));
        // The Page preview at recorded versions, the Text file and Exports never show it.
        assert!(v.shown_in("Collected", true).unwrap().iter().all(|s| s.split.is_none()));
        // Settled (included too, or dismissed): it goes, and stays gone.
        v.settle_split("Collected", 0, 21_000).unwrap();
        assert_eq!(v.inclusions("Collected").unwrap()[0].split, None);
        // Joining Chapters back (deleting the new heading) brings no note either.
        v.edit("Rain", vec![Op::Delete { from: rain_at, to: rain_at + 13, origin: Origin::Input }], 22_000).unwrap();
        v.tick(30_000);
        assert_eq!(v.inclusions("Collected").unwrap()[0].split, None);
    }

    #[test]
    fn an_inclusion_gets_styles_of_its_own_recorded_in_the_including_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        v.set_inclusion_styles("Collected", 18, &["Book text".into()], 12_000).unwrap();
        assert_eq!(v.inclusions("Collected").unwrap()[0].styles, vec!["Book text".to_string()]);
        // Applied inside the Inclusion, in its block (scoped to it there).
        let (body, header, blocks) = v.typeset_input("Collected", None, 12_100).unwrap();
        let block = &body[body.find("// ---- Included by wrtier").unwrap()..];
        assert!(block.contains("first-line-indent: 1.2em"), "{block}");
        compiles(&header, body, blocks);
        // Its content is recorded in "Collected" at once, so the Text file and Milestones have it.
        let work = &v.works["Collected"];
        assert!(work.doc.field("style/Book text", "content").is_some_and(|c| c.contains("first-line-indent")));
        v.close_work("Collected", 13_000);
        let file = std::fs::read_to_string(dir.path().join("Collected.typ")).unwrap();
        let block = &file[file.find("// ---- Included by wrtier").unwrap()..];
        assert!(block.contains("first-line-indent: 1.2em"));
        // Taken off again.
        v.open_work("Collected", 14_000).unwrap();
        v.set_inclusion_styles("Collected", 18, &[], 14_100).unwrap();
        assert!(v.inclusions("Collected").unwrap()[0].styles.is_empty());
        assert!(v.set_inclusion_styles("Collected", 0, &["Book text".into()], 14_200).is_err(), "no Inclusion there");
    }

    #[test]
    fn an_export_from_a_milestone_is_the_book_as_it_was_sent() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        v.make_milestone("Collected", "Sent", 12_000).unwrap();
        let sent = v.milestones("Collected", 12_100).unwrap().remove(0);
        v.open_work("Rain", 13_000).unwrap();
        v.edit("Rain", vec![typing(RAIN.len(), " Changed later.")], 13_100).unwrap();
        v.tick(20_000);
        let then = v.export_input_at("Collected", &sent.id, Export::default(), 21_000).unwrap();
        assert!(then.text.contains("She went home.") && !then.text.contains("Changed later."));
        assert_eq!(then.date_ms, sent.time_ms, "dated when the Milestone was named");
        // The same input, so the same PDF, on every Device and every day.
        let again = v.export_input_at("Collected", &sent.id, Export::default(), 99_000).unwrap();
        assert_eq!((then.text.clone(), then.header.text.clone()), (again.text, again.header.text));
        let mut t = crate::typeset::Typesetter::new(false);
        t.export_pdf(Path::new("none"), &then.header, then.text, then.date_ms, &then.ident).expect("it typesets");
        // An Export of now has the later text.
        assert!(v.export_input("Collected", Export::default(), 22_000).unwrap().text.contains("Changed later."));
    }

    /// The pages' text without spaces (lines break and justify differently from font to font).
    fn squeezed(pages: &[String]) -> Vec<String> {
        pages.iter().map(|p| p.split_whitespace().collect()).collect()
    }

    #[test]
    fn a_whole_included_work_starts_on_a_page_of_its_own_under_its_title_and_author() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = open(dir.path());
        v.create_work("Rain", 1000).unwrap();
        let p = Preamble { looks: vec!["Paperback".into()], title: "Rain at Night".into(), author: "Ana Lind".into(), ..Preamble::default() };
        v.set_preamble("Rain", &p, 1100).unwrap();
        v.edit("Rain", vec![typing(0, RAIN)], 1200).unwrap();
        v.close_work("Rain", 1300);
        v.create_work("Collected", 2000).unwrap();
        v.edit("Collected", vec![typing(0, "My introduction.\n\n")], 2100).unwrap();
        v.tick(10_000);
        include(&mut v, "Collected", 18, "Rain", None, 11_000);
        // The default Paperback Look has "Inclusion titles", last.
        let (body, header, blocks) = v.typeset_input("Collected", None, 12_000).unwrap();
        assert!(body.contains("<inclusion-title>") && body.contains("#metadata(\"Ana Lind\")<inclusion-author>"), "{body}");
        let mut t = crate::typeset::Typesetter::new(false);
        let c = t.compile_body("c", Path::new("none"), &header, body, blocks);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        let pages = squeezed(&t.page_texts("c"));
        let title = pages.iter().position(|p| p.contains("RainatNight")).expect("the title is on a page");
        assert!(pages[0].contains("Myintroduction.") && title > 0, "after a page break: {pages:?}");
        assert!(pages[title].contains("AnaLind"), "with its author's name: {pages:?}");
        assert_eq!(pages.iter().map(|p| p.matches("RainatNight").count()).sum::<usize>(), 1, "shown once, not also as a Chapter: {pages:?}");
        // By the same author as the Collection: no author line.
        let mut mine = v.preamble("Collected", 13_000).unwrap();
        mine.author = "Ana Lind".into();
        v.set_preamble("Collected", &mine, 13_100).unwrap();
        let (body, _, _) = v.typeset_input("Collected", None, 13_200).unwrap();
        assert!(body.contains("<inclusion-title>") && !body.contains("<inclusion-author>"), "{body}");
    }

    #[test]
    fn a_collection_is_made_of_the_works_given_in_their_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        v.create_work("Sun", 12_000).unwrap();
        v.edit("Sun", vec![typing(0, "It was sunny.")], 12_100).unwrap();
        v.close_work("Sun", 12_200);
        let made = v.create_collection("Short stories/Both", &["Sun".into(), "Rain".into()], 13_000).unwrap();
        assert_eq!((made.id.as_str(), made.text.clone()), ("Short stories/Both", format!("{INCLUSION}\n\n{INCLUSION}")));
        let views = v.inclusions("Short stories/Both").unwrap();
        let shown: Vec<(&str, &str)> = views.iter().map(|v| (v.label.as_str(), v.text.as_str())).collect();
        assert_eq!(shown, vec![("Sun", "It was sunny."), ("Rain", RAIN)]);
        // One that can't be included (gone meanwhile): nothing is made, and no empty file left.
        assert!(v.create_collection("Gone", &["Rain".into(), "Nowhere".into()], 14_000).is_err());
        assert!(!dir.path().join("Gone.db").exists() && !dir.path().join("Nowhere.db").exists());
    }

    #[test]
    fn two_works_with_one_id_are_a_sync_problem_until_one_goes() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = collected(dir.path());
        assert!(v.list(12_000).problems.is_empty());
        // "Rain" copied by hand: both claim to be the same Work.
        std::fs::create_dir_all(dir.path().join("Old")).unwrap();
        std::fs::copy(dir.path().join("Rain.db"), dir.path().join("Old/Rain copy.db")).unwrap();
        let problems = v.list(13_000).problems;
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].path, "Old/Rain copy");
        assert!(problems[0].reason.starts_with("Two Works claim to be Rain (Rain, Old/Rain copy)"), "{}", problems[0].reason);
        assert!(problems[0].reason.contains("trash"));
        // The Inclusion can't tell which is meant either.
        assert!(v.inclusions("Collected").unwrap()[0].missing.as_deref().is_some_and(|m| m.starts_with("Two Works claim to be")));
        // One goes to the trash: no problem, and the Inclusion shows the one left.
        v.trash_work("Old/Rain copy", 14_000).unwrap();
        assert!(v.list(14_100).problems.is_empty());
        assert!(v.inclusions("Collected").unwrap()[0].missing.is_none());
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
            at, index: 2, target: None, name: None, label: "Rain".into(), chapter: None, missing: None,
            plain: "Rain.".into(), typst: "#[\nRain.\n]".into(), version: None, language: String::new(),
            elements: Vec::new(), title: None, starts: 0, runs: Vec::new(), comments: Vec::new(), in_typst: Vec::new(), author: String::new(), split: None,
            all_runs: Vec::new(), notes: Vec::new(),
        };
        let (body, blocks) = typst_body(&format!("A\n{INCLUSION}\nB"), &[s]);
        assert!(body.starts_with("A\n// ---- Included by wrtier") && body.ends_with("\nB"), "{body}");
        assert_eq!(blocks, vec![(2, body.encode_utf16().count() - 4)]);
    }
}
