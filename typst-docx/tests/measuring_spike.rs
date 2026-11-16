//! Spike: can a target's look be measured from the laid-out pages reliably enough to give each
//! target one style? Compiles a small book in two real Looks, built the way the app builds
//! them, and prints how much each target varies.
//! `cargo test -p typst-docx --test measuring_spike -- --nocapture`

#[macro_use]
mod common;

use common::compile;
use typst_docx::measure::{measure, summary, Options, Selector, Target};

fn source(styles: &[&str]) -> String {
    source_with(styles, true)
}

fn source_with(styles: &[&str], markers: bool) -> String {
    common::source(styles, &common::book(markers), false)
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

