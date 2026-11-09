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
use crate::preamble::Header;
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
    /// The Preamble put before the Work's text, and how far it shifts positions.
    header: Header,
    header_utf16: usize,
    header_lines: usize,
}

impl Preview {
    /// A place in the Work's text (UTF-16), as a byte offset in the compiled source.
    fn to_source(&self, at: usize) -> Option<usize> {
        self.world.main.lines().utf16_to_byte(at + self.header_utf16)
    }

    /// A byte offset in the compiled source, as a place in the Work's text (UTF-16); None if
    /// it's in the Preamble.
    fn to_text(&self, byte: usize) -> Option<usize> {
        self.world.main.lines().byte_to_utf16(byte)?.checked_sub(self.header_utf16)
    }
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

/// What can go at the cursor (after a `#`), from Typst's own autocompleter.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Completions {
    /// Where the text the completions replace starts (UTF-16): just after the `#`.
    pub from: usize,
    pub items: Vec<CompletionItem>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    pub label: String,
    /// What picking it types, as a snippet: `${}` marks where the cursor goes.
    pub apply: String,
    pub detail: Option<String>,
    /// "function", "type", "constant", "parameter", "syntax" or "other".
    pub kind: &'static str,
    /// The Work's own (its Story elements, and anything else it defines) or Typst's.
    pub story_element: bool,
}

/// A PDF Export.
#[derive(Debug)]
pub struct ExportedPdf {
    pub bytes: Vec<u8>,
    /// Families that came from this Device, not the Vault: another Device's PDF would differ.
    pub device_fonts: Vec<String>,
    pub pages: usize,
}

/// A time (ms since 1970) as a date and time in UTC, for the PDF's metadata.
fn datetime_utc(ms: u64) -> Datetime {
    let (year, month, day, hour, minute, second) = utc_parts(ms);
    Datetime::from_ymd_hms(year, month, day, hour, minute, second)
        .unwrap_or_else(|| Datetime::from_ymd(1970, 1, 1).expect("a valid date"))
}

/// A time (ms since 1970) as year, month, day, hour, minute and second in UTC.
pub fn utc_parts(ms: u64) -> (i32, u8, u8, u8, u8, u8) {
    let secs = (ms / 1000) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day, (rest / 3600) as u8, (rest % 3600 / 60) as u8, (rest % 60) as u8)
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

    /// A Work's preview, made if it has none yet, with its Preamble and its text as they are now.
    fn preview(&mut self, id: &str, fonts_dir: &Path, header: &Header, text: &str) -> &mut Preview {
        let fonts = self.fonts(fonts_dir);
        let preview = self.previews.entry(id.to_string()).or_insert_with(|| Preview {
            world: WorkWorld {
                library: LazyHash::new(Library::default()),
                fonts,
                main: Source::new(main_id(), String::new()),
            },
            last_good: None,
            header: Header::default(),
            header_utf16: 0,
            header_lines: 0,
        });
        preview.world.main.replace(&format!("{}{text}", header.text));
        if preview.header != *header {
            preview.header = header.clone();
            preview.header_utf16 = header.text.encode_utf16().count();
            preview.header_lines = header.text.matches('\n').count();
        }
        preview
    }

    /// What can go at `at` (UTF-16) in a Work's text as it is now: after a `#`, every Story
    /// element and Typst function in scope there. `explicit`: asked for (Ctrl+Space), not just
    /// typed, so it may also offer completions where nothing was started yet.
    pub fn complete(&mut self, id: &str, fonts_dir: &Path, header: &Header, text: &str, at: usize, explicit: bool) -> Option<Completions> {
        let preview = self.preview(id, fonts_dir, header, text);
        let cursor = preview.to_source(at)?;
        let source = &preview.world.main;
        let found = catch_unwind(AssertUnwindSafe(|| {
            typst_ide::autocomplete(&preview.world, preview.last_good.as_ref(), source, cursor, explicit)
        }));
        let (from, completions) = found.ok()??;
        let typst = preview.world.library.global.scope();
        // The Looks' wrappers are the app's own, not something to write.
        let completions = completions.into_iter().filter(|c| !c.label.starts_with("wrtier-look-"));
        let elements = &preview.header.elements;
        // A bracket already after the cursor: picking a function shouldn't type another.
        let bracketed = source.text()[cursor..].starts_with(['(', '[']);
        let items = completions.map(|c| {
            let mut kind = match c.kind {
                typst_ide::CompletionKind::Func => "function",
                typst_ide::CompletionKind::Type => "type",
                typst_ide::CompletionKind::Constant => "constant",
                typst_ide::CompletionKind::Param => "parameter",
                typst_ide::CompletionKind::Syntax => "syntax",
                _ => "other",
            };
            let mut apply = c.apply.map(|a| a.to_string()).unwrap_or_else(|| c.label.to_string());
            let mut detail = c.detail.map(|d| d.to_string());
            // Typst only adds brackets for its own functions; the Story elements are defined in
            // the Preamble, so their template comes from how each is written.
            let form = elements.iter().find(|e| e.name == c.label.as_str());
            if let Some(form) = form {
                kind = "function";
                if !bracketed {
                    apply = if form.body { format!("{}[${{}}]", form.name) } else { format!("{}()", form.name) };
                }
                detail = form.detail.clone().or(detail);
            }
            // The Work's Story elements, and anything else it defines that isn't part of Typst.
            let story_element = form.is_some() || matches!(kind, "function" | "constant") && typst.get(&c.label).is_none();
            CompletionItem { apply, label: c.label.to_string(), detail, kind, story_element }
        }).collect();
        Some(Completions { from: preview.to_text(from)?, items })
    }

    /// Compiles a Work's text without a Preamble (Typst's plain defaults).
    pub fn compile(&mut self, id: &str, fonts_dir: &Path, text: String) -> Compiled {
        self.compile_with(id, fonts_dir, &Header::default(), text)
    }

    /// Compiles a Work's text as it is now, after its Preamble. `fonts_dir`: its Vault's
    /// `fonts/` folder.
    pub fn compile_with(&mut self, id: &str, fonts_dir: &Path, header: &Header, text: String) -> Compiled {
        let start = Instant::now();
        let fonts = self.fonts(fonts_dir);
        let preview = self.preview(id, fonts_dir, header, &text);
        let outcome = catch_unwind(AssertUnwindSafe(|| typst::compile::<PagedDocument>(&preview.world)));
        // Forget cached work this compile didn't use, or memory grows with every edit (Phase 0b).
        comemo::evict(1);
        let mut compiled = match outcome {
            Ok(warned) => {
                let mut warnings = problems(preview, &warned.warnings);
                // What the Preamble names but nothing provides (a Look naming a missing Style…).
                warnings.extend(header.missing.iter().map(|m| Problem {
                    message: format!("In the Preamble: {m}"), hints: Vec::new(), from: None, to: None, line: None,
                }));
                match warned.output {
                    Ok(document) => {
                        let device_fonts = fonts_from_device(&fonts, &document);
                        preview.last_good = Some(document);
                        Compiled { pages: Vec::new(), stale: false, errors: Vec::new(), warnings, device_fonts, ms: 0 }
                    }
                    Err(errors) => Compiled {
                        pages: Vec::new(), stale: true, errors: problems(preview, &errors),
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
            // (A click on something the Preamble made, such as a Look's page number, leads nowhere.)
            typst_ide::Jump::File(file, at) if file == preview.world.main.id() => Some(JumpTo::Text { at: preview.to_text(at)? }),
            typst_ide::Jump::Position(p) => Some(JumpTo::Pages {
                spot: Spot { page: p.page.get() - 1, x: p.point.x.to_pt(), y: p.point.y.to_pt() },
            }),
            _ => None, // links out of the Work aren't followed from the preview
        }
    }

    /// Where a place in the Work's text (UTF-16) is on the pages, for keeping it in view.
    pub fn locate(&self, id: &str, at: usize) -> Option<Spot> {
        let preview = self.previews.get(id)?;
        locate(preview.last_good.as_ref()?, &preview.world.main, preview.to_source(at)?)
    }

    /// A PDF of a Work's text after its Preamble, refused if it doesn't compile. The same
    /// version gives the same bytes on every Device: the date is `date_ms` (the newest Change in
    /// it, never today's), the document's ID comes from `ident` (the Work's own ID), and fonts
    /// come from the Vault or the app (a Device font is flagged in what's returned).
    pub fn export_pdf(&mut self, fonts_dir: &Path, header: &Header, text: String, date_ms: u64, ident: &str) -> Result<ExportedPdf, String> {
        const KEY: &str = "\u{0}export"; // apart from every Work's preview
        let compiled = self.compile_with(KEY, fonts_dir, header, text);
        let result = (|| {
            if let Some(error) = compiled.errors.first() {
                let line = error.line.map(|l| format!("line {l}: ")).unwrap_or_default();
                return Err(format!("It doesn't typeset: {line}{}. Nothing was exported.", error.message));
            }
            let document = self.previews.get(KEY).and_then(|p| p.last_good.as_ref()).ok_or("Nothing to export.")?;
            let options = typst_pdf::PdfOptions {
                ident: typst::foundations::Smart::Custom(ident.to_string()),
                creator: typst::foundations::Smart::Custom(Some("Wrtier".to_string())),
                timestamp: Some(typst_pdf::Timestamp::new_utc(datetime_utc(date_ms))),
                ..typst_pdf::PdfOptions::default()
            };
            let bytes = typst_pdf::pdf(document, &options)
                .map_err(|errors| format!("The PDF couldn't be made: {}", errors.first().map(|e| e.message.as_str()).unwrap_or("unknown")))?;
            Ok(ExportedPdf { bytes, device_fonts: compiled.device_fonts.clone(), pages: compiled.pages.len() })
        })();
        self.previews.remove(KEY);
        result
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

/// Typst's diagnostics, with their place in the Work's text. One in the Preamble says which
/// file of the Style library it's in (and its line there), since it can't be shown in the text.
fn problems(preview: &Preview, diagnostics: &[SourceDiagnostic]) -> Vec<Problem> {
    let world = &preview.world;
    let lines = world.main.lines();
    let header_bytes = preview.header.text.len();
    diagnostics.iter().map(|d| {
        let range = (d.span.id() == Some(world.main.id())).then(|| world.range(d.span)).flatten();
        let mut message = d.message.to_string();
        if d.severity == Severity::Warning && message.is_empty() {
            message = "warning".into();
        }
        let (from, to, line) = match range {
            Some(r) if r.start >= header_bytes => (
                preview.to_text(r.start),
                preview.to_text(r.end),
                lines.byte_to_line(r.start).map(|l| l + 1 - preview.header_lines),
            ),
            Some(r) => {
                let place = match preview.header.segments.iter().find(|s| (s.start..s.end).contains(&r.start)) {
                    Some(s) => {
                        let line = preview.header.text[s.start..r.start].matches('\n').count() + 1;
                        format!("{}, line {line}", s.label)
                    }
                    None => "the Preamble".to_string(),
                };
                message = format!("{place}: {message} (in the Style library, not in your text)");
                (None, None, None)
            }
            None => (None, None, None),
        };
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
    }

    /// The timing behind Phase 3's "previews smoothly": run on its own
    /// (`cargo test -p wrtier_app novel_timing -- --ignored --nocapture`). Typst's cache is
    /// shared by the whole program and every compile clears what it didn't use, so tests
    /// compiling at the same time would make this recompile start from scratch. (The app
    /// compiles one Work at a time.)
    #[test]
    #[ignore]
    fn novel_timing() {
        let mut t = Typesetter::new(false);
        let first = compile(&mut t, &novel(""));
        let edited = compile(&mut t, &novel("edited "));
        eprintln!("{} pages: first compile {} ms, recompile {} ms", first.pages.len(), first.ms, edited.ms);
        // Generous, for slow machines: well under the 300 ms pause on desktop.
        assert!(edited.ms < 300, "recompile took {} ms", edited.ms);
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

    fn complete(t: &mut Typesetter, text: &str) -> Completions {
        let at = text.find('|').unwrap();
        let text = text.replace('|', "");
        t.complete("Rain", Path::new("no-fonts-here"), &Header::default(), &text, at, false).expect("completions")
    }

    #[test]
    fn a_hash_lists_story_elements_and_typst() {
        let mut t = Typesetter::new(false);
        let c = complete(&mut t, "#let dream(body) = emph(body)\n\nShe slept. #|");
        assert_eq!(c.from, "#let dream(body) = emph(body)\n\nShe slept. #".len());
        let dream = c.items.iter().find(|i| i.label == "dream").expect("the Work's own element is offered");
        assert!(dream.story_element);
        assert!(dream.apply.starts_with("dream"), "{:?}", dream.apply);
        let text = c.items.iter().find(|i| i.label == "text").expect("Typst's own functions too");
        assert!(!text.story_element);
        assert!(c.items.len() > 50, "everything Typst has: {}", c.items.len());
    }

    #[test]
    fn what_is_typed_after_the_hash_is_what_gets_replaced() {
        let mut t = Typesetter::new(false);
        let c = complete(&mut t, "#let dream(body) = emph(body)\n\n#dr|");
        assert_eq!(c.from, "#let dream(body) = emph(body)\n\n#".len());
        assert!(c.items.iter().any(|i| i.label == "dream"));
        // Plain prose offers nothing.
        let mut t = Typesetter::new(false);
        assert!(t.complete("Rain", Path::new("x"), &Header::default(), "The rain had", 12, false).is_none_or(|c| c.items.is_empty()));
    }

    /// A header like a Work's Preamble makes: a Story element, a Style that can go wrong.
    fn header_with(style: &str) -> Header {
        let mut h = Header::default();
        h.text.push_str(crate::preamble::HEADER_START);
        let element = "// Story element: Dream\n// A dream.\n#let dream(body) = [#block(inset: 2em, emph(body)) <dream>]\n";
        h.text.push_str(element);
        h.elements.push(crate::preamble::ElementForm::of("dream", element));
        let pause = "#let pause() = [#v(2em) <pause>]\n";
        h.text.push_str(pause);
        h.elements.push(crate::preamble::ElementForm::of("pause", pause));
        let start = h.text.len();
        h.text.push_str(style);
        h.segments.push(crate::preamble::Segment { start, end: h.text.len(), label: "styles/Odd.typ".into() });
        h.text.push_str(crate::preamble::HEADER_END);
        h
    }

    #[test]
    fn positions_stay_in_the_works_text_after_a_preamble() {
        let mut t = Typesetter::new(false);
        let h = header_with("// Style: Odd\n#set text(size: 12pt)\n");
        let text = "Awake.\n\n#dream[I was falling]\n\n#nothing";
        let c = t.compile_with("Rain", Path::new("x"), &h, text.to_string());
        let error = &c.errors[0];
        assert_eq!(error.line, Some(5), "lines count from the Work's text");
        assert_eq!(error.from, Some(at(text, "#nothing") + 1));
        // Clicks and the cursor land in the Work's text, not in the Preamble.
        let good = "Awake.\n\n#dream[I was falling]";
        t.compile_with("Rain", Path::new("x"), &h, good.to_string());
        round_trip(&t, good, "falling");
        // The Preamble's Story elements come first among the completions.
        let c = t.complete("Rain", Path::new("x"), &h, "Awake. #dr", 10, false).unwrap();
        assert_eq!(c.from, 8);
        assert!(!c.items.iter().any(|i| i.label.starts_with("wrtier-look")));
        // Picking one types it with its brackets, the cursor inside a body.
        let dream = c.items.iter().find(|i| i.label == "dream").expect("offered");
        assert!(dream.story_element);
        assert_eq!(dream.apply, "dream[${}]");
        assert_eq!(dream.detail.as_deref(), Some("Dream: A dream."));
        let c = t.complete("Rain", Path::new("x"), &h, "Awake. #pa", 10, false).unwrap();
        assert_eq!(c.items.iter().find(|i| i.label == "pause").unwrap().apply, "pause()");
        // With a bracket already there, none is added.
        let c = t.complete("Rain", Path::new("x"), &h, "Awake. #dr[falling]", 10, false).unwrap();
        assert_eq!(c.items.iter().find(|i| i.label == "dream").unwrap().apply, "dream");
    }

    #[test]
    fn an_error_in_the_style_library_names_its_file() {
        let mut t = Typesetter::new(false);
        let h = header_with("// Style: Odd\n// A line.\n#set text(size: twelve)\n");
        let c = t.compile_with("Rain", Path::new("x"), &h, "Hello.".to_string());
        let error = &c.errors[0];
        assert!(error.message.starts_with("styles/Odd.typ, line 3: "), "{}", error.message);
        assert_eq!((error.from, error.line), (None, None));
    }

    #[test]
    fn nothing_to_find_before_a_compile() {
        let t = Typesetter::new(false);
        assert_eq!(t.locate("Rain", 0), None);
        assert_eq!(t.jump("Rain", 0, 10.0, 10.0), None);
    }

    #[test]
    fn a_pdfs_date_is_the_right_day() {
        assert_eq!(datetime_utc(0), Datetime::from_ymd_hms(1970, 1, 1, 0, 0, 0).unwrap());
        assert_eq!(datetime_utc(1_700_000_000_000), Datetime::from_ymd_hms(2023, 11, 14, 22, 13, 20).unwrap());
        assert_eq!(datetime_utc(1_709_164_800_000), Datetime::from_ymd_hms(2024, 2, 29, 0, 0, 0).unwrap()); // a leap day
        assert_eq!(datetime_utc(1_794_268_799_000), Datetime::from_ymd_hms(2026, 11, 9, 23, 59, 59).unwrap());
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
