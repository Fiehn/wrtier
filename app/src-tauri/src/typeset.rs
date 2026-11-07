//! Typesetting a Work with Typst, for the Page preview: its pages, its errors and warnings,
//! and the fonts it used.
//!
//! Each open Work keeps one Typst `World` alive between compiles and only swaps its text, which
//! is what makes recompiles incremental (Phase 0b). Only one compile runs at a time: the caller
//! holds the `Typesetter`'s lock for the whole compile. Compiling reads the text and never
//! writes to the History.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use serde::Serialize;
use typst::diag::{FileError, FileResult, Severity, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::introspection::PagedPosition;
use typst::layout::{Abs, Frame, FrameItem, Point};
use typst::syntax::{FileId, LinkedNode, RootedPath, Side, Source, Span, SyntaxKind, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook, FontInfo};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World, WorldExt};
use typst_kit::fonts::{FontPath, FontStore};
use typst_layout::PagedDocument;

/// A font found on this Device: where it is and what it is, without its data.
#[derive(Clone)]
struct DeviceFont {
    path: PathBuf,
    index: u32,
    info: FontInfo,
}

/// The Device's own fonts, found once per run: scanning them reads every font file.
fn device_fonts() -> &'static [DeviceFont] {
    static FOUND: OnceLock<Vec<DeviceFont>> = OnceLock::new();
    FOUND.get_or_init(|| {
        typst_kit::fonts::system()
            .map(|(path, info)| DeviceFont { path: path.path, index: path.index, info })
            .collect()
    })
}

/// Finds the Device's fonts in the background (about half a second), so the first Page
/// preview doesn't wait for it.
pub fn warm_up() {
    std::thread::spawn(|| {
        device_fonts();
    });
}

/// Every font a compile can use, in the order the rule gives: the Vault's `fonts/` folder, then
/// the fonts bundled in the app (the default), then the Device's own. A Device font is only
/// used for a family neither the Vault nor the app has, so the Vault and the bundled fonts
/// always look the same everywhere, and a Device font is known by its family alone.
pub struct Fonts {
    store: FontStore,
    /// The families (lowercase) that only this Device has: using one is flagged.
    device_only: HashSet<String>,
    /// What the Vault's `fonts/` folder held when these were loaded.
    vault_stamp: Vec<(String, u64)>,
}

/// The files in a fonts folder, with their sizes: if it changes, the fonts are loaded again.
fn folder_stamp(dir: &Path) -> Vec<(String, u64)> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
            } else {
                found.push((path.display().to_string(), meta.len()));
            }
        }
    }
    found.sort();
    found
}

impl Fonts {
    /// `device`: also use the Device's own fonts (tests leave them out, to be the same everywhere).
    fn load(vault_fonts: &Path, device: bool) -> Fonts {
        let mut store = FontStore::new();
        let mut known = HashSet::new();
        let mut add = |store: &mut FontStore, entries: Vec<(FontPath, FontInfo)>| {
            for (path, info) in entries {
                known.insert(info.family.to_lowercase());
                store.push((path, info));
            }
        };
        if vault_fonts.is_dir() {
            add(&mut store, typst_kit::fonts::scan(vault_fonts).collect());
        }
        for (font, info) in typst_kit::fonts::embedded() {
            known.insert(info.family.to_lowercase());
            store.push((font, info));
        }
        let mut device_only = HashSet::new();
        if device {
            for font in device_fonts() {
                let family = font.info.family.to_lowercase();
                if known.contains(&family) {
                    continue;
                }
                device_only.insert(family);
                store.push((FontPath { path: font.path.clone(), index: font.index }, font.info.clone()));
            }
        }
        Fonts { store, device_only, vault_stamp: folder_stamp(vault_fonts) }
    }

    /// Did this font come from this Device? (Vault and bundled fonts are the same everywhere.)
    fn from_device(&self, font: &Font) -> bool {
        self.device_only.contains(&font.info().family.to_lowercase())
    }
}

/// One Work, as Typst sees it: a single in-memory main file and the fonts.
struct WorkWorld {
    library: LazyHash<Library>,
    fonts: Arc<Fonts>,
    main: Source,
}

impl World for WorkWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        self.fonts.store.book()
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() { Ok(self.main.clone()) } else { Err(FileError::AccessDenied) }
    }
    fn file(&self, _id: FileId) -> FileResult<Bytes> {
        Err(FileError::AccessDenied) // images come in v2
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.store.font(index)
    }
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None // a Work renders the same whatever day it is
    }
}

impl typst_ide::IdeWorld for WorkWorld {
    fn upcast(&self) -> &dyn World {
        self
    }
}

/// One open Work's preview: its World, and the last pages that compiled.
struct Preview {
    world: WorkWorld,
    last_good: Option<PagedDocument>,
}

/// A page's size in points, so the preview can lay it out before its picture arrives.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageSize {
    pub width: f64,
    pub height: f64,
}

/// One page's picture, if it changed since the one the preview has.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PagePicture {
    /// Changes whenever anything on the page does.
    pub hash: String,
    /// The page as SVG; None if it's the same as the one the preview already has.
    pub svg: Option<String>,
}

/// An error or warning, with where it is in the Work's text (UTF-16, as the editor counts).
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub message: String,
    pub hints: Vec<String>,
    pub from: Option<usize>,
    pub to: Option<usize>,
    /// 1-based, for "Line 812: …".
    pub line: Option<usize>,
}

/// A spot on the pages: the page (from 0), and points from its top left.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub page: usize,
    pub x: f64,
    pub y: f64,
}

/// Where a click on the pages leads.
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JumpTo {
    /// A place in the Work's text (UTF-16, as the editor counts).
    Text { at: usize },
    /// Another spot on the pages, such as where a link inside the Work points.
    Pages { spot: Spot },
}

/// What a compile gives the Page preview.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Compiled {
    /// The pages to show: this compile's, or the last ones that compiled if it failed. Their
    /// pictures are asked for one by one, for the pages in view only: hashing every page of a
    /// novel on each compile would cost more than the compile itself.
    pub pages: Vec<PageSize>,
    /// The pages are the last ones that compiled, not this text's.
    pub stale: bool,
    pub errors: Vec<Problem>,
    pub warnings: Vec<Problem>,
    /// Families that came from this Device, not the Vault: other Devices may render differently.
    pub device_fonts: Vec<String>,
    pub ms: u64,
}

/// Every open Work's preview, and the fonts they share.
pub struct Typesetter {
    previews: HashMap<String, Preview>,
    fonts: Option<Arc<Fonts>>,
    /// The Vault's `fonts/` folder the fonts were loaded from.
    fonts_dir: PathBuf,
    /// Also use the Device's own fonts.
    device_fonts: bool,
}

impl Default for Typesetter {
    fn default() -> Self {
        Typesetter::new(true)
    }
}

fn main_id() -> FileId {
    FileId::unique(RootedPath::new(VirtualRoot::Project, VirtualPath::new("main.typ").expect("a valid path")))
}

impl Typesetter {
    pub fn new(device_fonts: bool) -> Self {
        Typesetter { previews: HashMap::new(), fonts: None, fonts_dir: PathBuf::new(), device_fonts }
    }

    /// The fonts for the Vault whose `fonts/` folder this is, loaded again if the folder changed.
    fn fonts(&mut self, fonts_dir: &Path) -> Arc<Fonts> {
        let current = self.fonts.as_ref()
            .filter(|f| self.fonts_dir == fonts_dir && f.vault_stamp == folder_stamp(fonts_dir));
        if let Some(fonts) = current {
            return fonts.clone();
        }
        let fonts = Arc::new(Fonts::load(fonts_dir, self.device_fonts));
        self.fonts = Some(fonts.clone());
        self.fonts_dir = fonts_dir.to_path_buf();
        for preview in self.previews.values_mut() {
            preview.world.fonts = fonts.clone();
        }
        fonts
    }

    /// Compiles a Work's text as it is now. `fonts_dir`: its Vault's `fonts/` folder.
    pub fn compile(&mut self, id: &str, fonts_dir: &Path, text: String) -> Compiled {
        let start = Instant::now();
        let fonts = self.fonts(fonts_dir);
        let preview = self.previews.entry(id.to_string()).or_insert_with(|| Preview {
            world: WorkWorld {
                library: LazyHash::new(Library::default()),
                fonts: fonts.clone(),
                main: Source::new(main_id(), String::new()),
            },
            last_good: None,
        });
        preview.world.main.replace(&text);
        let outcome = catch_unwind(AssertUnwindSafe(|| typst::compile::<PagedDocument>(&preview.world)));
        // Forget cached work this compile didn't use, or memory grows with every edit (Phase 0b).
        comemo::evict(1);
        let mut compiled = match outcome {
            Ok(warned) => {
                let warnings = problems(&preview.world, &warned.warnings);
                match warned.output {
                    Ok(document) => {
                        let device_fonts = fonts_from_device(&fonts, &document);
                        preview.last_good = Some(document);
                        Compiled { pages: Vec::new(), stale: false, errors: Vec::new(), warnings, device_fonts, ms: 0 }
                    }
                    Err(errors) => Compiled {
                        pages: Vec::new(), stale: true, errors: problems(&preview.world, &errors),
                        warnings, device_fonts: Vec::new(), ms: 0,
                    },
                }
            }
            Err(panic) => {
                // Typst itself failed: start this Work's preview afresh next time.
                let message = crate::vault::panic_message(panic.as_ref());
                self.previews.remove(id);
                let problem = Problem { message: format!("Typst stopped unexpectedly: {message}"), hints: Vec::new(), from: None, to: None, line: None };
                return Compiled { pages: Vec::new(), stale: true, errors: vec![problem], warnings: Vec::new(), device_fonts: Vec::new(), ms: start.elapsed().as_millis() as u64 };
            }
        };
        if let Some(document) = &self.previews[id].last_good {
            compiled.pages = document.pages().iter().map(|page| {
                let size = page.frame.size();
                PageSize { width: size.x.to_pt(), height: size.y.to_pt() }
            }).collect();
        }
        compiled.ms = start.elapsed().as_millis() as u64;
        compiled
    }

    /// One page of the last pages that compiled. `known`: the hash of the picture the preview
    /// already has, so an unchanged page isn't sent again.
    pub fn page(&self, id: &str, page: usize, known: Option<&str>) -> Result<PagePicture, String> {
        let document = self.previews.get(id).and_then(|p| p.last_good.as_ref())
            .ok_or_else(|| format!("{id} has no pages yet."))?;
        let page = document.pages().get(page).ok_or_else(|| format!("{id} has no page {}.", page + 1))?;
        let hash = format!("{:032x}", typst::utils::hash128(page));
        let svg = (known != Some(hash.as_str()))
            .then(|| typst_svg::svg(page, &typst_svg::SvgOptions { render_bleed: false, pretty: false }));
        Ok(PagePicture { hash, svg })
    }

    /// Where a click on a page leads. `x` and `y` are points from the page's top left.
    pub fn jump(&self, id: &str, page: usize, x: f64, y: f64) -> Option<JumpTo> {
        let preview = self.previews.get(id)?;
        let document = preview.last_good.as_ref()?;
        let position = PagedPosition { page: NonZeroUsize::new(page + 1)?, point: Point::new(Abs::pt(x), Abs::pt(y)) };
        match typst_ide::jump_from_click(&preview.world, document, &position)? {
            typst_ide::Jump::File(file, at) if file == preview.world.main.id() => {
                Some(JumpTo::Text { at: preview.world.main.lines().byte_to_utf16(at)? })
            }
            typst_ide::Jump::Position(p) => Some(JumpTo::Pages {
                spot: Spot { page: p.page.get() - 1, x: p.point.x.to_pt(), y: p.point.y.to_pt() },
            }),
            _ => None, // links out of the Work aren't followed from the preview
        }
    }

    /// Where a place in the Work's text (UTF-16) is on the pages, for keeping it in view.
    pub fn locate(&self, id: &str, at: usize) -> Option<Spot> {
        let preview = self.previews.get(id)?;
        let source = &preview.world.main;
        locate(preview.last_good.as_ref()?, source, source.lines().utf16_to_byte(at)?)
    }

    /// The Work's tab closed (or it was renamed): its preview isn't needed.
    pub fn forget(&mut self, id: &str) {
        self.previews.remove(id);
    }
}

/// Where the text at `cursor` (a byte offset) ended up on the pages: the glyph written from
/// the cursor's stretch of text that comes last at or before the cursor, so it lands on the
/// cursor's line, not at the start of its paragraph. A cursor in markup (on the `=` of a
/// heading, inside `#scene-break()`) uses the nearest text after it, or else before it.
fn locate(document: &PagedDocument, source: &Source, cursor: usize) -> Option<Spot> {
    let is_text = |n: &LinkedNode| matches!(n.kind(), SyntaxKind::Text | SyntaxKind::MathText);
    let root = LinkedNode::new(source.root());
    let at_cursor = root.leaf_at(cursor, Side::Before).filter(is_text)
        .or_else(|| root.leaf_at(cursor, Side::After).filter(is_text));
    let (node, within) = match at_cursor {
        Some(node) => {
            let within = cursor.saturating_sub(node.offset()).min(node.len());
            (node, within)
        }
        None => {
            let start = root.leaf_at(cursor, Side::After).or_else(|| root.leaf_at(cursor, Side::Before))?;
            let after = std::iter::successors(Some(start.clone()), |n| n.next_leaf()).take(200).find(is_text).map(|n| (n, 0));
            after.or_else(|| {
                std::iter::successors(Some(start), |n| n.prev_leaf()).take(200).find(is_text).map(|n| {
                    let len = n.len();
                    (n, len)
                })
            })?
        }
    };
    let within = u16::try_from(within).unwrap_or(u16::MAX);
    let mut best: Option<(u16, Spot)> = None;
    for (index, page) in document.pages().iter().enumerate() {
        if let Some((offset, point)) = find_glyph(&page.frame, node.span(), within) {
            if best.is_none_or(|(b, _)| offset > b) {
                best = Some((offset, Spot { page: index, x: point.x.to_pt(), y: point.y.to_pt() }));
            }
        }
    }
    best.map(|(_, spot)| spot)
}

/// In a frame: the glyph from `span` with the largest offset not past `within`, and where it is
/// (on its baseline).
fn find_glyph(frame: &Frame, span: Span, within: u16) -> Option<(u16, Point)> {
    let mut best: Option<(u16, Point)> = None;
    let mut consider = |offset: u16, point: Point| {
        if best.is_none_or(|(b, _)| offset > b) {
            best = Some((offset, point));
        }
    };
    for &(pos, ref item) in frame.items() {
        match item {
            FrameItem::Group(group) => {
                if let Some((offset, point)) = find_glyph(&group.frame, span, within) {
                    consider(offset, pos + point.transform(group.transform));
                }
            }
            FrameItem::Text(text) => {
                let mut x = pos.x;
                for glyph in &text.glyphs {
                    if glyph.span.0 == span && glyph.span.1 <= within {
                        consider(glyph.span.1, Point::new(x, pos.y));
                    }
                    x += glyph.x_advance.at(text.size);
                }
            }
            _ => {}
        }
    }
    best
}

/// Typst's diagnostics, with their place in the Work's text.
fn problems(world: &WorkWorld, diagnostics: &[SourceDiagnostic]) -> Vec<Problem> {
    let lines = world.main.lines();
    diagnostics.iter().map(|d| {
        let range = (d.span.id() == Some(world.main.id())).then(|| world.range(d.span)).flatten();
        let (from, to, line) = match range {
            Some(r) => (lines.byte_to_utf16(r.start), lines.byte_to_utf16(r.end), lines.byte_to_line(r.start).map(|l| l + 1)),
            None => (None, None, None),
        };
        let mut message = d.message.to_string();
        if d.severity == Severity::Warning && message.is_empty() {
            message = "warning".into();
        }
        Problem { message, hints: d.hints.iter().map(|h| h.v.to_string()).collect(), from, to, line }
    }).collect()
}

/// The families on the pages that came from this Device rather than the Vault or the app.
fn fonts_from_device(fonts: &Fonts, document: &PagedDocument) -> Vec<String> {
    fn walk(frame: &Frame, fonts: &Fonts, found: &mut Vec<String>) {
        for (_, item) in frame.items() {
            match item {
                FrameItem::Group(group) => walk(&group.frame, fonts, found),
                FrameItem::Text(text) => {
                    let font = text.font.font();
                    if fonts.from_device(font) && !found.contains(&font.info().family) {
                        found.push(font.info().family.clone());
                    }
                }
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    if fonts.device_only.is_empty() {
        return found;
    }
    for page in document.pages() {
        walk(&page.frame, fonts, &mut found);
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(t: &mut Typesetter, text: &str) -> Compiled {
        t.compile("Rain", Path::new("no-fonts-here"), text.to_string())
    }

    #[test]
    fn compiles_pages() {
        let mut t = Typesetter::new(false);
        let c = compile(&mut t, "= Rain\n\nThe rain had stopped.\n\n#pagebreak()\n\nEnd.");
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert!(!c.stale);
        assert_eq!(c.pages.len(), 2);
        assert!(c.pages[0].width > 0.0 && c.pages[0].height > c.pages[0].width);
        let picture = t.page("Rain", 0, None).unwrap();
        assert!(picture.svg.unwrap().starts_with("<svg"));
        assert!(t.page("Rain", 0, Some(&picture.hash)).unwrap().svg.is_none(), "an unchanged page isn't sent again");
        assert!(t.page("Rain", 2, None).is_err());
    }

    #[test]
    fn an_error_keeps_the_last_pages_and_says_where() {
        let mut t = Typesetter::new(false);
        let good = compile(&mut t, "Hello.\n\n#pagebreak()\n\nWorld.");
        // "é" is one UTF-16 unit but two bytes: positions must be in UTF-16.
        let bad = compile(&mut t, "Hello é.\n\n#nothing-like-this\n\nWorld.");
        assert!(bad.stale);
        assert_eq!(bad.pages, good.pages);
        let error = &bad.errors[0];
        assert_eq!(error.line, Some(3));
        assert_eq!(error.from, Some("Hello é.\n\n#".len() - 1)); // the span starts after the `#`
        assert!(error.to > error.from);
        let fixed = compile(&mut t, "Hello é.\n\nWorld.");
        assert!(!fixed.stale && fixed.errors.is_empty());
        assert_eq!(fixed.pages.len(), 1);
    }

    #[test]
    fn only_changed_pages_change_their_hash() {
        let mut t = Typesetter::new(false);
        compile(&mut t, "One.\n#pagebreak()\nTwo.");
        let before: Vec<String> = (0..2).map(|i| t.page("Rain", i, None).unwrap().hash).collect();
        compile(&mut t, "One.\n#pagebreak()\nTwo, edited.");
        assert!(t.page("Rain", 0, Some(&before[0])).unwrap().svg.is_none());
        assert!(t.page("Rain", 1, Some(&before[1])).unwrap().svg.is_some());
    }

    /// About 300 pages (as in Phase 0b): 40 Chapters of 30 paragraphs.
    fn novel(edit: &str) -> String {
        let words = ["the", "rain", "had", "stopped", "and", "she", "walked", "down", "to", "harbour",
            "where", "boats", "lay", "quiet", "under", "a", "grey", "sky", "nobody", "spoke"];
        let mut text = String::from("#set page(width: 13cm, height: 20cm)\n#set par(justify: true)\n\n");
        let mut n: usize = 7;
        for chapter in 1..=40 {
            text.push_str(&format!("= Chapter {chapter}\n\n"));
            for paragraph in 0..30 {
                for _ in 0..90 {
                    n = (n * 31 + 17) % 9973;
                    text.push_str(words[n % words.len()]);
                    text.push(' ');
                }
                if chapter == 20 && paragraph == 0 {
                    text.push_str(edit);
                }
                text.push_str("\n\n");
            }
        }
        text
    }

    #[test]
    fn a_novel_recompiles_quickly_after_a_small_edit() {
        let mut t = Typesetter::new(false);
        let first = compile(&mut t, &novel(""));
        assert!(first.pages.len() > 250, "{} pages", first.pages.len());
        // The pages in view, about where the edit is (Chapter 20 starts near page 150).
        let in_view = 145..160;
        let before: Vec<String> = in_view.clone().map(|i| t.page("Rain", i, None).unwrap().hash).collect();
        let edited = compile(&mut t, &novel("edited "));
        let start = Instant::now();
        let redrawn = in_view.zip(&before).filter(|(i, hash)| t.page("Rain", *i, Some(hash)).unwrap().svg.is_some()).count();
        let drawn_ms = start.elapsed().as_millis();
        eprintln!("{} pages: first compile {} ms, recompile {} ms, {redrawn} of 15 pages in view redrawn, checked in {drawn_ms} ms",
            first.pages.len(), first.ms, edited.ms);
        assert!(redrawn >= 1 && redrawn < 15);
        // Following the cursor looks through every page on each move.
        let text = novel("edited ");
        let start = Instant::now();
        let spot = t.locate("Rain", text.find("edited").unwrap()).unwrap();
        eprintln!("found the cursor on page {} in {} ms", spot.page + 1, start.elapsed().as_millis());
        assert!((140..170).contains(&spot.page));
        // Generous, for slow test machines; on desktop it's far below this.
        assert!(edited.ms < 1000, "recompile took {} ms", edited.ms);
    }

    /// UTF-16 position of `needle` in `text` (the texts here are ASCII, so bytes will do).
    fn at(text: &str, needle: &str) -> usize {
        text.find(needle).unwrap()
    }

    /// Clicking just above the baseline where `locate` put a place leads back to about there.
    fn round_trip(t: &Typesetter, text: &str, needle: &str) -> (Spot, usize) {
        let cursor = at(text, needle) + 2;
        let spot = t.locate("Rain", cursor).unwrap_or_else(|| panic!("{needle:?} isn't on the pages"));
        let Some(JumpTo::Text { at: landed }) = t.jump("Rain", spot.page, spot.x + 1.0, spot.y - 3.0) else {
            panic!("a click on {needle:?} didn't lead to the text");
        };
        assert!(landed.abs_diff(cursor) <= 3, "{needle:?}: cursor {cursor}, click landed at {landed}");
        (spot, landed)
    }

    #[test]
    fn clicks_and_the_cursor_meet_in_the_same_place() {
        let mut t = Typesetter::new(false);
        let long = "word ".repeat(400);
        let text = format!("= The Long Night\n\nShe walked {long}down to the harbour.\n\n#pagebreak()\n\nThe boats lay quiet.");
        compile(&mut t, &text);
        round_trip(&t, &text, "Long Night");
        let (start, _) = round_trip(&t, &text, "walked");
        // The end of a long paragraph is lines below its start, not at it.
        let (end, _) = round_trip(&t, &text, "harbour");
        assert!(end.page > start.page || end.y > start.y + 50.0, "{start:?} → {end:?}");
        let (later, _) = round_trip(&t, &text, "boats");
        assert!(later.page > start.page);
        // A cursor in markup finds the nearest text: on the `=`, the heading.
        let heading = t.locate("Rain", 0).unwrap();
        assert_eq!(heading.page, 0);
        assert!(heading.y < start.y);
    }

    #[test]
    fn inside_a_story_element() {
        // Story elements are functions that wrap their body (step 3 adds the real ones).
        let mut t = Typesetter::new(false);
        let text = "#let dream(body) = block(inset: 2em, emph(body))\n\nAwake.\n\n#dream[I was falling through the floor]\n\nAwake again.";
        compile(&mut t, text);
        let (spot, _) = round_trip(&t, text, "falling");
        let (before, _) = round_trip(&t, text, "Awake.");
        assert!(spot.x > before.x, "the inset moves it right: {before:?} → {spot:?}");
        // On `#dream[`: the text inside it.
        assert!(t.locate("Rain", at(text, "#dream[")).is_some());
    }

    #[test]
    fn nothing_to_find_before_a_compile() {
        let t = Typesetter::new(false);
        assert_eq!(t.locate("Rain", 0), None);
        assert_eq!(t.jump("Rain", 0, 10.0, 10.0), None);
    }

    #[test]
    fn the_date_never_depends_on_today() {
        let mut t = Typesetter::new(false);
        let c = compile(&mut t, "#datetime.today().display()");
        assert!(!c.errors.is_empty(), "today() must not be available: {:?}", c.pages);
    }

    #[test]
    fn device_fonts_only_fill_in_families_the_app_lacks() {
        let start = Instant::now();
        let fonts = Fonts::load(Path::new("no-fonts-here"), true);
        eprintln!("{} Device-only families, scanned in {} ms", fonts.device_only.len(), start.elapsed().as_millis());
        assert!(!fonts.device_only.contains("libertinus serif"));
        let mut t = Typesetter::new(true);
        let c = compile(&mut t, "Hello, with the default font.");
        assert!(c.device_fonts.is_empty(), "{:?}", c.device_fonts);
        // A family only this Device has, if it has one, is flagged when used.
        if let Some(family) = fonts.device_only.iter().find(|f| f.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ')) {
            let c = compile(&mut t, &format!("#set text(font: \"{family}\")\nHello."));
            assert_eq!(c.device_fonts.iter().map(|f| f.to_lowercase()).collect::<Vec<_>>(), vec![family.clone()]);
        }
    }

    #[test]
    fn a_missing_font_is_a_warning_and_the_default_is_used() {
        let mut t = Typesetter::new(false);
        let c = compile(&mut t, "#set text(font: \"No Such Font\")\nHello.");
        assert!(c.errors.is_empty());
        assert_eq!(c.pages.len(), 1);
        assert!(c.warnings.iter().any(|w| w.message.contains("No Such Font") || w.message.to_lowercase().contains("font")), "{:?}", c.warnings);
    }
}
