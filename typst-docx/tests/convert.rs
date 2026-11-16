//! The `.docx` of a book in the app's default Looks: its styles, text and page.
//! `SPIKE_DOCX_DIR=… cargo test -p typst-docx --test convert -- --ignored` writes the samples
//! for opening in Word and LibreOffice by hand.

#[macro_use]
mod common;

use std::io::Read;

use common::{book, compile, source, MANUSCRIPT, PAPERBACK};
use typst_docx::{convert, Metadata, Options, Role, Selector, Target};

fn targets() -> Vec<Target> {
    let t = |name: &str, selector: Selector, role: Role| Target { name: name.into(), selector, role, origin: Some(format!("{name}.typ")) };
    let label = |name: &str, l: &str| t(name, Selector::Label(l.into()), Role::Paragraph);
    // The first target at a heading level is Word's "heading n", so Chapters come first.
    vec![
        t("Chapter", Selector::Heading(1), Role::Heading(2)),
        t("Heading 2", Selector::Heading(2), Role::Heading(3)),
        label("Dedication", "dedication"),
        // The Part is `heading above chapters`, so Chapters go a level down.
        t("Part", Selector::Label("part".into()), Role::Heading(1)),
        label("Epigraph", "epigraph"),
        label("Scene break", "scene-break"),
        label("Block text", "block-text"),
        t("Inclusion title", Selector::Label("inclusion-title".into()), Role::Heading(2)),
        t("Inclusion author", Selector::Label("inclusion-author".into()), Role::LeftOut),
    ]
}

fn options() -> Options {
    Options {
        targets: targets(),
        scope: Some(("scope-start".into(), "scope-end".into())),
        metadata: Metadata { title: "Rain".into(), author: "R. L. Fiehn".into(), date: "2026-11-16T10:00:00Z".into(), seed: 7 },
    }
}

fn parts(bytes: &[u8]) -> std::collections::HashMap<String, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = std::collections::HashMap::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let mut xml = String::new();
        file.read_to_string(&mut xml).unwrap();
        let name = file.name().unwrap().to_string();
        roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{name} isn't well-formed XML: {e}"));
        out.insert(name, xml);
    }
    out
}

/// Each paragraph's style ID ("" for Normal) and its text, from document.xml.
fn paragraphs(document: &str) -> Vec<(String, String)> {
    let doc = roxmltree::Document::parse(document).unwrap();
    doc.descendants().filter(|n| n.has_tag_name("p")).map(|p| {
        let style = p.descendants().find(|n| n.has_tag_name("pStyle")).and_then(|n| n.attributes().next().map(|a| a.value().to_string())).unwrap_or_default();
        let text = p.descendants().filter_map(|n| {
            if n.has_tag_name("t") { n.text().map(String::from) } else if n.has_tag_name("br") { Some("|".into()) } else { None }
        }).collect();
        (style, text)
    }).collect()
}

fn paperback() -> (std::collections::HashMap<String, String>, typst_docx::Docx) {
    let docx = convert(&compile(&source(&PAPERBACK, &book(true), true)), &options()).unwrap();
    (parts(&docx.bytes), docx)
}

#[test]
fn each_kind_of_text_is_one_named_style() {
    let (parts, _) = paperback();
    let styles = &parts["word/styles.xml"];
    for name in ["Normal", "First Paragraph", "heading 1", "heading 2", "heading 3", "Epigraph", "Dedication", "Scene break", "Block text", "Normal (Zampanò)"] {
        assert!(styles.contains(&format!("<w:name w:val=\"{name}\"/>")), "no {name} style in {styles}");
    }
    let paras = paragraphs(&parts["word/document.xml"]);
    let style_of = |text: &str| paras.iter().find(|(_, t)| t.starts_with(text)).map(|(s, _)| s.as_str()).unwrap_or_else(|| panic!("no paragraph {text:?}"));
    assert_eq!(style_of("Book One"), "Heading1");
    assert_eq!(style_of("The Long Night"), "Heading2");
    assert_eq!(style_of("A smaller heading"), "Heading3");
    assert_eq!(style_of("The night is long"), "Epigraph");
    assert_eq!(style_of("For M., who waited."), "Dedication");
    assert!(!paras.iter().any(|(_, t)| t.contains("J. Truant")), "the Inclusion's author is left out");
    // A second target at Chapter level keeps its own look, based on Heading 2 and in the outline.
    assert_eq!(style_of("Zampanò"), "InclusionTitleZampan");
    assert!(styles.contains("<w:style w:type=\"paragraph\" w:styleId=\"InclusionTitle\"><w:name w:val=\"Inclusion title\"/><w:basedOn w:val=\"Heading2\"/>"), "{styles}");
}

#[test]
fn chapters_open_a_new_page_and_prose_after_a_break_is_flush() {
    let (parts, _) = paperback();
    let styles = roxmltree::Document::parse(&parts["word/styles.xml"]).unwrap();
    let style = |name: &str| styles.descendants().find(|n| n.has_tag_name("style") && n.descendants().any(|c| c.has_tag_name("name") && c.attribute(("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "val")) == Some(name))).unwrap();
    let has = |name: &str, tag: &str| style(name).descendants().any(|n| n.has_tag_name(tag));
    assert!(has("heading 2", "pageBreakBefore"), "Chapters open a new page");
    assert!(!has("Epigraph", "pageBreakBefore"));
    let first = style("First Paragraph");
    let ind = first.descendants().find(|n| n.has_tag_name("ind")).expect("First Paragraph sets its own indent");
    assert_eq!(ind.attribute(("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "firstLine")), Some("0"));
}

#[test]
fn a_paragraph_that_opens_a_page_starts_a_section() {
    let (parts, _) = paperback();
    let xml = &parts["word/document.xml"];
    let paras = paragraphs(xml);
    // Book One, two Chapters, Zampanò and After; the dedication is first, so needs none.
    let opening = paras.iter().filter(|(s, _)| ["Heading1", "Heading2", "InclusionTitleZampan"].contains(&s.as_str())).count();
    assert_eq!(opening, 5);
    assert_eq!(xml.matches("<w:type w:val=\"nextPage\"/>").count(), opening, "a section break before each");
}

#[test]
fn a_one_line_paragraph_after_prose_is_flush_like_a_longer_one() {
    let (parts, _) = paperback();
    let paras = paragraphs(&parts["word/document.xml"]);
    let style_of = |text: &str| paras.iter().find(|(_, t)| t.starts_with(text)).map(|(s, _)| s.clone()).unwrap();
    assert_eq!(style_of("Dear Anna,"), "BlockTextFirst");
    assert_eq!(style_of("Come home."), "BlockText");
}

#[test]
fn the_text_reads_as_written() {
    let doc = compile(&source(&PAPERBACK, "= One\n\nThe extraordinarily incomprehensible circumstances surrounding the uncharacteristically disproportionate thing went on, and _on_, and *on*.\n\n#epigraph[Line one \\\nline two]\n\nShe said, \"It's late.\"", true));
    let parts = parts(&convert(&doc, &options()).unwrap().bytes);
    let xml = &parts["word/document.xml"];
    let paras = paragraphs(xml);
    let texts: Vec<&str> = paras.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(texts, [
        "One",
        "The extraordinarily incomprehensible circumstances surrounding the uncharacteristically disproportionate thing went on, and on, and on.",
        "Line one|line two",
        "She said, “It’s late.”",
    ]);
    assert!(!xml.contains('\u{ad}'), "Typst's hyphens are gone");
    // The epigraph is italic as a style, so its text needs no italic runs.
    let doc = roxmltree::Document::parse(xml).unwrap();
    let run_with = |text: &str| doc.descendants().find(|n| n.has_tag_name("r") && n.descendants().any(|t| t.has_tag_name("t") && t.text() == Some(text))).unwrap_or_else(|| panic!("no run {text:?}"));
    assert!(run_with("on").descendants().any(|n| n.has_tag_name("i")), "_on_ is italic");
}

#[test]
fn the_page_comes_from_the_probe() {
    let (parts, _) = paperback();
    let xml = &parts["word/document.xml"];
    // 5.5 × 8.5 in, inside 0.9 in, outside 0.65 in, top 0.75 in, bottom 0.85 in.
    assert!(xml.contains("<w:pgSz w:w=\"7920\" w:h=\"12240\"/>"), "{}", &xml[xml.len() - 400..]);
    assert!(xml.contains("w:top=\"1080\" w:right=\"936\" w:bottom=\"1224\" w:left=\"1296\""));
    assert!(parts["word/settings.xml"].contains("<w:mirrorMargins/>"));
    assert!(parts["word/settings.xml"].contains("<w:autoHyphenation/>"), "Book text hyphenates");
    let manuscript = parts_of(&MANUSCRIPT);
    assert!(manuscript["word/document.xml"].contains("<w:pgSz w:w=\"12240\" w:h=\"15840\"/>"), "US Letter");
    assert!(!manuscript["word/settings.xml"].contains("mirrorMargins"));
}

fn parts_of(styles: &[&str]) -> std::collections::HashMap<String, String> {
    parts(&convert(&compile(&source(styles, &book(true), true)), &options()).unwrap().bytes)
}

#[test]
fn the_probe_moves_nothing() {
    use typst::layout::FrameItem;
    let positions = |probe: bool| -> Vec<String> {
        let doc = compile(&source(&PAPERBACK, &book(true), probe));
        doc.pages().iter().enumerate().flat_map(|(n, p)| p.frame.items().filter_map(move |(pos, item)| {
            matches!(item, FrameItem::Group(_) | FrameItem::Text(_)).then(|| format!("{n} {:.2} {:.2}", pos.x.to_pt(), pos.y.to_pt()))
        })).collect()
    };
    assert_eq!(positions(true), positions(false));
}

#[test]
fn without_a_probe_it_says_so() {
    let doc = compile(&source(&PAPERBACK, "Text.", false));
    assert!(convert(&doc, &options()).err().unwrap().contains("probe"));
}

#[test]
fn the_same_input_gives_the_same_file() {
    let doc = compile(&source(&PAPERBACK, &book(true), true));
    assert_eq!(convert(&doc, &options()).unwrap().bytes, convert(&doc, &options()).unwrap().bytes);
}

/// Writes `Paperback.docx` and `Manuscript.docx` to the folder in SPIKE_DOCX_DIR, with Typst's
/// own PDF of each to compare with Word's, and prints the report.
#[test]
#[ignore]
fn write_samples() {
    let dir = std::path::PathBuf::from(std::env::var("SPIKE_DOCX_DIR").expect("set SPIKE_DOCX_DIR"));
    for (name, styles) in [("Paperback", &PAPERBACK[..]), ("Manuscript", &MANUSCRIPT[..])] {
        let document = compile(&source(styles, &book(true), true));
        let docx = convert(&document, &options()).unwrap();
        println!("{name}: {:#?}", docx.report);
        let pdf = typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|e| format!("{e:?}")).unwrap();
        std::fs::write(dir.join(format!("{name}-typst.pdf")), pdf).unwrap();
        std::fs::write(dir.join(format!("{name}.docx")), docx.bytes).unwrap();
    }
}
