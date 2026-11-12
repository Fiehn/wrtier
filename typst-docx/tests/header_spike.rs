//! Spike: can the Manuscript Style's header ("Surname / Title / page") read the Work's title and
//! author, and in what form can a `.docx` get it?
//! (A) which ways of writing such a header compile in Typst 0.15;
//! (B) whether the header each page actually shows (its text inside `artifact` tags) can be
//!     turned into a `.docx` header: fixed text, a PAGE field, a STYLEREF to the Chapter title.
//! `cargo test -p typst-docx --test header_spike -- --nocapture`

mod common;

use common::{compile, try_compile};
use typst::introspection::{Location, Tag};
use typst::layout::{Frame, FrameItem, Point};
use typst_docx::measure::{measure, Options, Selector, Target};
use typst_layout::PagedDocument;

/// What the Preamble writes, plus (for one variant) the settings as variables of their own.
const PREAMBLE: &str = "#set document(title: \"Rain\", author: \"R. L. Fiehn\")\n#set text(lang: \"en\")\n";
const SETTINGS_AS_VARIABLES: &str = "#let work-title = \"Rain\"\n#let work-author = \"R. L. Fiehn\"\n";
const PAGE: &str = "#set page(paper: \"us-letter\", margin: 1in)\n#set text(size: 12pt)\n#set par(first-line-indent: 0.5in, leading: 1.3em, spacing: 1.3em)\n#show heading.where(level: 1): it => { pagebreak(weak: true); v(30%); align(center, text(size: 12pt, weight: \"regular\", it.body)); v(2em) }\n";

/// Ways a Style might write the manuscript header.
const VARIANTS: &[(&str, &str)] = &[
    ("numbering only (today's Manuscript)", "#set page(numbering: \"1\", number-align: right + top)"),
    ("fixed text + counter, no context", "#set page(header: align(right)[Fiehn / Rain / #counter(page).display()])"),
    ("fixed text + counter in context", "#set page(header: align(right)[Fiehn / Rain / #context counter(page).display()])"),
    ("document fields, no context", "#set page(header: align(right)[#document.title / #counter(page).display()])"),
    ("document fields in context", "#set page(header: context align(right)[#document.author.first().split(\" \").last() / #document.title / #counter(page).display()])"),
    ("settings as variables", "#set page(header: align(right)[#work-author.split(\" \").last() / #work-title / #context counter(page).display()])"),
    ("running Chapter title, none on its first page", "#set page(header: context { let here-page = here().page(); let opening = query(heading.where(level: 1)).any(h => h.location().page() == here-page); let before = query(heading.where(level: 1).before(here())); if not opening and before.len() > 0 { align(right)[#before.last().body / #counter(page).display()] } })"),
];

fn book() -> String {
    let mut t = String::new();
    for chapter in ["The Long Night", "The Morning", "After"] {
        t.push_str(&format!("= {chapter}\n\n"));
        for n in [120, 90, 150, 80] {
            t.push_str(&format!("#lorem({n})\n\n"));
        }
    }
    t
}

fn source(style: &str) -> String {
    format!("{PREAMBLE}{SETTINGS_AS_VARIABLES}#let look(doc) = [\n{PAGE}{style}\n#doc\n]\n#show: look\n{}", book())
}

/// The text inside `artifact` tags on each page, by line (top to bottom), with the page's
/// logical number.
fn furniture(doc: &PagedDocument) -> Vec<(u64, Vec<String>)> {
    fn walk(frame: &Frame, at: Point, open: &mut Vec<Location>, out: &mut Vec<(f64, f64, String)>) {
        for (pos, item) in frame.items() {
            let p = at + *pos;
            match item {
                FrameItem::Group(g) => walk(&g.frame, p + Point::new(g.transform.tx, g.transform.ty), open, out),
                FrameItem::Tag(Tag::Start(c, _)) if c.elem().name() == "artifact" => open.extend(c.location()),
                FrameItem::Tag(Tag::End(loc, ..)) => open.retain(|l| l != loc),
                FrameItem::Text(t) if !open.is_empty() => out.push((p.y.to_pt(), p.x.to_pt(), t.text.to_string())),
                _ => {}
            }
        }
    }
    doc.pages().iter().map(|page| {
        let mut open = Vec::new();
        let mut texts = Vec::new();
        walk(&page.frame, Point::zero(), &mut open, &mut texts);
        texts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut lines: Vec<(f64, String)> = Vec::new();
        for (y, _, t) in texts {
            match lines.last_mut() {
                Some((ly, s)) if (*ly - y).abs() < 0.5 => s.push_str(&t),
                _ => lines.push((y, t)),
            }
        }
        (page.number, lines.into_iter().map(|(_, s)| s).collect())
    }).collect()
}

/// The `.docx` header that shows what each page showed, if there's one: page numbers become
/// {PAGE}, the latest Chapter title becomes {STYLEREF Heading 1}. Pages showing nothing are
/// reported apart (Word's "different first page" covers them when they're Chapters' first).
fn template(pages: &[(u64, Vec<String>)], chapter_on: &dyn Fn(usize) -> Option<String>) -> Result<(String, Vec<usize>), String> {
    let mut found: Option<String> = None;
    let mut empty = Vec::new();
    for (i, (number, lines)) in pages.iter().enumerate() {
        let shown = lines.join(" | ");
        if shown.trim().is_empty() {
            empty.push(i + 1);
            continue;
        }
        let mut t = shown.clone();
        if let Some(chapter) = chapter_on(i).filter(|c| !c.is_empty()) {
            t = t.replace(&chapter, "{STYLEREF Heading 1}");
        }
        // The page number as the last number in the line (so "Rain 2" isn't mistaken).
        let n = number.to_string();
        if let Some(at) = t.rfind(&n) {
            t.replace_range(at..at + n.len(), "{PAGE}");
        }
        match &found {
            None => found = Some(t),
            Some(f) if *f == t => {}
            Some(f) => return Err(format!("page {} shows {shown:?}, which doesn't fit {f:?}", i + 1)),
        }
    }
    Ok((found.unwrap_or_default(), empty))
}

#[test]
fn which_headers_compile_and_what_a_docx_can_make_of_them() {
    let targets = vec![Target { name: "Heading 1".into(), selector: Selector::Heading(1) }];
    for (name, style) in VARIANTS {
        println!("\n== {name}\n   {style}");
        let doc = match try_compile(&source(style)) {
            Ok(doc) => doc,
            Err(errors) => {
                println!("   doesn't compile: {}", errors.join("; "));
                continue;
            }
        };
        let pages = furniture(&doc);
        for (n, lines) in pages.iter().take(4) {
            println!("   page {n}: {lines:?}");
        }
        // The Chapter in force on each page: the last Chapter heading on it or before it.
        let units = measure(&doc, &Options { targets: targets.clone(), scope: None });
        let chapter_on = |page: usize| units.iter().rev().find(|u| u.target == Some(0) && u.page <= page).map(|u| u.text.clone());
        match template(&pages, &chapter_on) {
            Ok((t, empty)) => println!("   .docx header: {t:?}{}", if empty.is_empty() { String::new() } else { format!(", nothing on pages {empty:?}") }),
            Err(e) => println!("   no single .docx header: {e}"),
        }
    }
    let _ = compile;
}

/// The Manuscript Style the app ships gives "Surname / Title / page" in the PDF, and a `.docx`
/// header made from the pages, with or without a title and author in the Work's settings.
#[test]
fn the_shipped_manuscript_header() {
    let style = include_str!("../../app/src-tauri/defaults/styles/Manuscript.typ");
    let no_chapter = |_: usize| None;
    for (preamble, expected) in [
        (PREAMBLE, "Fiehn / Rain / {PAGE}"),
        ("#set document(title: \"Rain\")\n", "Rain / {PAGE}"),
        ("", "{PAGE}"),
    ] {
        let source = format!("{preamble}#let look(doc) = [\n{style}\n#doc\n]\n#show: look\n{}", book());
        let pages = furniture(&compile(&source));
        assert_eq!(pages[1].1, vec![expected.replace("{PAGE}", "2")]);
        assert_eq!(template(&pages, &no_chapter).unwrap().0, expected);
    }
}
