//! Spike: can a target's look be measured from the laid-out pages reliably enough to give each
//! target one style? Compiles a small book in two real Looks, built the way the app builds
//! them, and prints how much each target varies.
//! `cargo test -p typst-docx --test measuring_spike -- --nocapture`

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, Source, VirtualPath, VirtualRoot, RootedPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_docx::measure::{measure, summary, Options, Selector, Target};
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

fn compile(text: &str) -> PagedDocument {
    let mut fonts = FontStore::new();
    for (font, info) in typst_kit::fonts::embedded() {
        fonts.push((font, info));
    }
    let id = FileId::new(RootedPath::new(VirtualRoot::Project, VirtualPath::new("main.typ").unwrap()));
    let world = TestWorld { library: LazyHash::new(Library::default()), fonts, main: Source::new(id, text.into()) };
    let warned = typst::compile::<PagedDocument>(&world);
    match warned.output {
        Ok(doc) => doc,
        Err(errors) => panic!("doesn't compile: {:?}", errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>()),
    }
}

macro_rules! library { ($f:literal) => { include_str!(concat!("../../app/src-tauri/defaults/styles/", $f)) } }

const ELEMENTS: [&str; 5] = [
    library!("elements/block-text.typ"), library!("elements/dedication.typ"), library!("elements/epigraph.typ"),
    library!("elements/part.typ"), library!("elements/scene-break.typ"),
];

/// The book: everything the default Story elements and Styles do, plus an Inclusion with a
/// Style of its own, written out as the app writes one.
fn book(markers: bool) -> String {
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
    // An Inclusion of a whole Work, with "Typewriter" as its own Style (inclusions::wrap),
    // bracketed by scope markers when `markers`.
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

fn source(styles: &[&str]) -> String {
    source_with(styles, true)
}

/// The source as the Preamble builds it: elements, then the Look wrapped around the text.
fn source_with(styles: &[&str], markers: bool) -> String {
    let mut s = String::from("#set document(title: \"Rain\", author: \"R. L. Fiehn\")\n#set text(lang: \"en\")\n");
    for e in ELEMENTS { s.push_str(e); s.push('\n'); }
    s.push_str("#let wrtier-look-1(doc) = [\n");
    for st in styles { s.push_str(st); s.push('\n'); }
    s.push_str("#doc\n]\n#show: wrtier-look-1\n");
    s.push_str(&book(markers));
    s
}

fn options() -> Options {
    Options { targets: targets(), scope: Some(("scope-start".into(), "scope-end".into())) }
}

fn targets() -> Vec<Target> {
    let label = |n: &str, l: &str| Target { name: n.into(), selector: Selector::Label(l.into()) };
    vec![
        label("Dedication", "dedication"), label("Part", "part"), label("Epigraph", "epigraph"),
        label("Scene break", "scene-break"), label("Block text", "block-text"),
        label("Inclusion title", "inclusion-title"), label("Inclusion author", "inclusion-author"),
        Target { name: "Heading 1".into(), selector: Selector::Heading(1) },
        Target { name: "Heading 2".into(), selector: Selector::Heading(2) },
    ]
}

fn run(look: &str, styles: &[&str]) {
    let doc = compile(&source(styles));
    let targets = targets();
    let units = measure(&doc, &options());
    // The markers must not move anything: every line sits where it does without them.
    let plain = measure(&compile(&source_with(styles, false)), &options());
    let lines = |us: &[typst_docx::measure::Unit]| us.iter().flat_map(|u| u.lines.iter().map(move |l| (u.page, format!("{:.2} {:.2} {:.2}", l.baseline, l.left, l.right)))).collect::<Vec<_>>();
    assert_eq!(lines(&units), lines(&plain), "{look}: the scope markers changed the layout");
    println!("\n######## {look}: {} pages, {} units\n{}", doc.pages().len(), units.len(), summary(&units, &targets));
    if std::env::var("SPIKE_UNITS").is_ok() {
        for u in &units {
            let name = u.target.map_or("Body", |i| &targets[i].name);
            println!("p{} {:<16} {:?} lines={} area={:.1}..{:.1} first={:.1}..{:.1} {:?}", u.page + 1, name, u.align(), u.lines.len(), u.area.left, u.area.right, u.lines[0].left, u.lines[0].right, u.text);
        }
    }
    assert!(units.iter().any(|u| u.target.is_none()), "found body text");
}

#[test]
fn paperback() {
    run("Paperback", &[library!("Paperback page.typ"), library!("Book text.typ"), library!("Chapter openings.typ"), library!("Page numbers.typ"), library!("Inclusion titles.typ")]);
}

#[test]
fn manuscript() {
    run("Manuscript", &[library!("Manuscript.typ"), library!("Inclusion titles.typ")]);
}

#[test]
#[ignore]
fn page_structure() {
    use typst::layout::FrameItem;
    let doc = compile(&source(&[library!("Paperback page.typ"), library!("Book text.typ"), library!("Chapter openings.typ"), library!("Page numbers.typ"), library!("Inclusion titles.typ")]));
    for (n, page) in doc.pages().iter().enumerate().take(5) {
        println!("page {} size {:?}", n + 1, page.frame.size());
        for (pos, item) in page.frame.items() {
            match item {
                FrameItem::Group(g) => println!("  group at {:?} size {:?} kind {:?} items {}", pos, g.frame.size(), g.frame.kind(), g.frame.items().count()),
                FrameItem::Text(t) => println!("  text at {:?} {:?}", pos, t.text),
                FrameItem::Tag(t) => println!("  tag {:?}", match t { typst::introspection::Tag::Start(c, _) => format!("start {}", c.elem().name()), _ => "end".into() }),
                _ => println!("  other at {:?}", pos),
            }
        }
    }
}

