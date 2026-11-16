//! What the spikes share: a `World` with Typst's bundled fonts and one in-memory file.

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_kit::fonts::FontStore;
use typst_layout::PagedDocument;

struct TestWorld {
    library: LazyHash<Library>,
    fonts: FontStore,
    main: Source,
}

impl World for TestWorld {
    fn library(&self) -> &LazyHash<Library> { &self.library }
    fn book(&self) -> &LazyHash<FontBook> { self.fonts.book() }
    fn main(&self) -> FileId { self.main.id() }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() { Ok(self.main.clone()) } else { Err(FileError::AccessDenied) }
    }
    fn file(&self, _id: FileId) -> FileResult<Bytes> { Err(FileError::AccessDenied) }
    fn font(&self, index: usize) -> Option<Font> { self.fonts.font(index) }
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> { None }
}

/// Compiles `text`, or says why it doesn't.
#[allow(dead_code)]
pub fn try_compile(text: &str) -> Result<PagedDocument, Vec<String>> {
    try_compile_source(text).map(|(document, _)| document)
}

/// Compiles `text`, with the source it was compiled from (which comments are anchored in).
#[allow(dead_code)]
pub fn try_compile_source(text: &str) -> Result<(PagedDocument, Source), Vec<String>> {
    let mut fonts = FontStore::new();
    for (font, info) in typst_kit::fonts::embedded() {
        fonts.push((font, info));
    }
    let id = FileId::new(RootedPath::new(VirtualRoot::Project, VirtualPath::new("main.typ").unwrap()));
    let world = TestWorld { library: LazyHash::new(Library::default()), fonts, main: Source::new(id, text.into()) };
    let document = typst::compile::<PagedDocument>(&world).output.map_err(|errors| errors.iter().map(|e| e.message.to_string()).collect::<Vec<_>>())?;
    Ok((document, world.main))
}

#[allow(dead_code)]
pub fn compile_source(text: &str) -> (PagedDocument, Source) {
    try_compile_source(text).unwrap_or_else(|e| panic!("doesn't compile: {e:?}"))
}

#[allow(dead_code)]
pub fn compile(text: &str) -> PagedDocument {
    try_compile(text).unwrap_or_else(|e| panic!("doesn't compile: {e:?}"))
}

/// A file of the app's default Style library.

macro_rules! library { ($f:literal) => { include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../app/src-tauri/defaults/styles/", $f)) } }

#[allow(dead_code)]
pub const ELEMENTS: [&str; 5] = [
    library!("elements/block-text.typ"), library!("elements/dedication.typ"), library!("elements/epigraph.typ"),
    library!("elements/part.typ"), library!("elements/scene-break.typ"),
];

#[allow(dead_code)]
pub const PAPERBACK: [&str; 5] = [
    library!("Paperback page.typ"), library!("Book text.typ"), library!("Chapter openings.typ"),
    library!("Page numbers.typ"), library!("Inclusion titles.typ"),
];

#[allow(dead_code)]
pub const MANUSCRIPT: [&str; 2] = [library!("Manuscript.typ"), library!("Inclusion titles.typ")];

/// The book: everything the default Story elements and Styles do, plus an Inclusion with a
/// Style of its own, written out as the app writes one, bracketed by scope markers when
/// `markers`.
#[allow(dead_code)]
pub fn book(markers: bool) -> String {
    let p = |n: usize| format!("#lorem({n})\n\n");
    let mut t = String::new();
    t.push_str("#dedication[For M., who waited.]\n\n#part[Book One]\n\n= The Long Night\n\n");
    t.push_str("#epigraph[The night is long that never finds the day. \\\n— Macbeth]\n\n");
    for n in [70, 45, 90, 30, 60] { t.push_str(&p(n)); }
    t.push_str("#scene-break()\n\n");
    for n in [80, 50, 65] { t.push_str(&p(n)); }
    t.push_str("#block-text[Dear Anna,\n\nCome home. The garden misses you, and so does the dog, who sits by the gate every evening as if you might come up the road.\n\nYours, \\\nM.]\n\n");
    for n in [55, 75] { t.push_str(&p(n)); }
    t.push_str("= The Morning\n\n");
    for n in [60, 40, 85] { t.push_str(&p(n)); }
    t.push_str("== A smaller heading\n\n");
    for n in [50, 70] { t.push_str(&p(n)); }
    if markers { t.push_str("#metadata(\"Zampanò\")<scope-start>\n"); }
    t.push_str("#[\n\n#heading(level: 1, \"Zampanò\")<inclusion-title>\n#metadata(\"J. Truant\")<inclusion-author>\n#set heading(offset: 1)\n");
    t.push_str("#set text(font: \"DejaVu Sans Mono\")\n");
    t.push_str("= Exploration A\n\n");
    for n in [60, 45, 70] { t.push_str(&p(n)); }
    t.push_str("\n\n]\n");
    if markers { t.push_str("#metadata(none)<scope-end>\n"); }
    t.push_str("\n= After\n\n");
    for n in [60, 50] { t.push_str(&p(n)); }
    t
}

/// A document as the app's Preamble builds it: the Story elements, then the Look wrapped
/// around `text`, with the probe (when `probe`) after the Look's own rules.
#[allow(dead_code)]
pub fn source(styles: &[&str], text: &str, probe: bool) -> String {
    let mut s = String::from("#set document(title: \"Rain\", author: \"R. L. Fiehn\")\n#set text(lang: \"en\")\n");
    for e in ELEMENTS { s.push_str(e); s.push('\n'); }
    s.push_str("#let wrtier-look-1(doc) = [\n");
    for st in styles { s.push_str(st); s.push('\n'); }
    if probe { s.push_str(&typst_docx::probe::snippet()); }
    s.push_str("#doc\n]\n#show: wrtier-look-1\n");
    s.push_str(text);
    s
}
