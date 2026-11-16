//! The `.docx` of a book in the app's default Looks: its styles, text and page.
//! `SPIKE_DOCX_DIR=… cargo test -p typst-docx --test convert -- --ignored` writes the samples
//! for opening in Word and LibreOffice by hand.

#[macro_use]
mod common;

use std::io::Read;

use common::{book, compile_source, source, MANUSCRIPT, PAPERBACK};
use typst_docx::{Comment, Metadata, Options, Reply, Role, Selector, Target, What};

/// Converts `text`, compiled.
fn convert_text(text: &str, options: &Options) -> Result<typst_docx::Docx, String> {
    let (document, main) = compile_source(text);
    typst_docx::convert(&document, &main, options)
}

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
        ..Options::default()
    }
}

fn parts(bytes: &[u8]) -> std::collections::HashMap<String, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = std::collections::HashMap::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        if file.name().unwrap().ends_with(".odttf") {
            continue; // an embedded font (see `fonts_are_embedded_whole_as_truetype`)
        }
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
    let docx = convert_text(&source(&PAPERBACK, &book(true), true), &options()).unwrap();
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

const TEXT: &str = "= One\n\n#lorem(400)\n\nThe extraordinarily incomprehensible circumstances surrounding the uncharacteristically disproportionate thing went on, and _on_, and *on*.\n\n#epigraph[Line one \\\nline two, _upright_]\n\nShe said, \"It's late.\"";

/// The text of the runs between comment `id`'s range start and end.
fn commented(document: &str, id: &str) -> String {
    let doc = roxmltree::Document::parse(document).unwrap();
    let mut inside = false;
    let mut text = String::new();
    for n in doc.descendants() {
        let this = n.attribute(("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "id")) == Some(id);
        if n.has_tag_name("commentRangeStart") && this {
            inside = true;
        } else if n.has_tag_name("commentRangeEnd") && this {
            break;
        } else if inside && n.has_tag_name("t") {
            text.push_str(n.text().unwrap_or(""));
        }
    }
    text
}

#[test]
fn comments_sit_on_the_text_they_were_made_on() {
    let text = source(&PAPERBACK, TEXT, true);
    let range = |s: &str| { let at = text.rfind(s).unwrap(); at..at + s.len() };
    let comment = |text: &str, ranges, replies| Comment {
        author: "Anna".into(), initials: "A".into(), date: "2026-11-01T09:00:00Z".into(),
        text: text.into(), ranges, replies, origin: None,
    };
    let reply = Reply { author: "R. L. Fiehn".into(), initials: "RF".into(), date: "2026-11-02T09:00:00Z".into(), text: "Agreed.".into() };
    let options = Options {
        comments: vec![
            // Across the edge of an emphasised word, and Typst's hyphenation of "uncharacteristically".
            comment("Too long?", vec![range("uncharacteristically disproportionate"), range("_on_, and")], vec![reply]),
            comment("Lovely book.\nTruly.", vec![], vec![]),
            // In the Look's own source: nothing of it is text.
            comment("Lost", vec![range("#show heading")], vec![]),
            // After text a function made (`#lorem`), whose offsets aren't into the source.
            comment("Late", vec![range("late")], vec![]),
        ],
        ..options()
    };
    let docx = convert_text(&text, &options).unwrap();
    let parts = parts(&docx.bytes);
    let xml = &parts["word/document.xml"];
    assert_eq!(commented(xml, "0"), "uncharacteristically disproportionate thing went on, and on, and", "first to last character covered");
    assert_eq!(commented(xml, "1"), commented(xml, "0"), "a reply sits where its comment does");
    assert_eq!(commented(xml, "2"), "", "a comment on the Work is at its start, on nothing");
    assert_eq!(commented(xml, "3"), "late");
    let comments = &parts["word/comments.xml"];
    assert_eq!(comments.matches("<w:comment ").count(), 4);
    assert!(comments.contains("w:author=\"R. L. Fiehn\""));
    assert_eq!(comments.matches("<w:p ").count(), 5, "one paragraph per line");
    assert_eq!(parts["word/commentsExtended.xml"].matches("paraIdParent").count(), 1);
    assert!(parts["[Content_Types].xml"].contains("comments+xml"));
    assert_eq!(docx.report.iter().filter(|n| matches!(&n.what, What::CommentNotPlaced { text } if text == "Lost")).count(), 1);
}

#[test]
fn plain_keeps_the_structure_and_emphasis_only() {
    let docx = convert_text(&source(&PAPERBACK, TEXT, true), &Options { plain: true, ..options() }).unwrap();
    let parts = parts(&docx.bytes);
    let (xml, styles) = (&parts["word/document.xml"], &parts["word/styles.xml"]);
    assert!(!xml.contains("<w:pgSz") && !xml.contains("<w:pgMar"), "Word's own page");
    assert!(!parts["word/settings.xml"].contains("mirrorMargins") && !parts["word/settings.xml"].contains("autoHyphenation"));
    assert!(styles.contains("w:ascii=\"Times New Roman\"") && !styles.contains("Libertinus"), "{styles}");
    assert!(styles.contains("<w:name w:val=\"Epigraph\"/>"), "the element keeps its named style");
    let paras = paragraphs(xml);
    assert_eq!(paras.iter().find(|(_, t)| t.starts_with("Line one")).unwrap().0, "Epigraph");
    let doc = roxmltree::Document::parse(xml).unwrap();
    let italic = |text: &str| doc.descendants().find(|n| n.has_tag_name("r") && n.descendants().any(|t| t.has_tag_name("t") && t.text() == Some(text)))
        .unwrap_or_else(|| panic!("no run {text:?}"))
        .descendants().any(|n| n.has_tag_name("i") && n.attributes().count() == 0);
    assert!(italic("on"), "_on_ is italic");
    assert!(italic("upright"), "emphasis in the (italic) epigraph is italic here");
    assert!(!italic("Line one"), "the epigraph's own italic is the Look's, not the text's");
    assert!(!xml.contains("Libertinus") && !xml.contains("<w:sz "), "no fonts or sizes on runs");
    assert!(docx.report.is_empty(), "{:?}", docx.report);
}

#[test]
fn the_text_reads_as_written() {
    let doc = source(&PAPERBACK, TEXT, true);
    let parts = parts(&convert_text(&doc, &options()).unwrap().bytes);
    let xml = &parts["word/document.xml"];
    let paras = paragraphs(xml);
    let texts: Vec<&str> = paras.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(texts[0], "One");
    assert!(texts[1].starts_with("Lorem ipsum"));
    assert_eq!(texts[2..], [
        "The extraordinarily incomprehensible circumstances surrounding the uncharacteristically disproportionate thing went on, and on, and on.",
        "Line one|line two, upright",
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
    parts(&convert_text(&source(styles, &book(true), true), &options()).unwrap().bytes)
}

/// Three Chapters of a few pages each.
fn chapters() -> String {
    let mut t = String::new();
    for chapter in ["The Long Night", "The Morning", "After"] {
        t.push_str(&format!("= {chapter}\n\n"));
        for n in [120, 90, 150, 80, 110] {
            t.push_str(&format!("#lorem({n})\n\n"));
        }
    }
    t
}

/// Each header or footer part's paragraphs, as text with fields as `{instruction}`, and which
/// kind of page it's for, by part.
fn furniture(parts: &std::collections::HashMap<String, String>) -> Vec<(String, String)> {
    let doc = roxmltree::Document::parse(&parts["word/document.xml"]).unwrap();
    let rels = &parts["word/_rels/document.xml.rels"];
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let mut out: Vec<(String, String)> = doc.descendants()
        .filter(|n| n.has_tag_name("headerReference") || n.has_tag_name("footerReference"))
        .map(|n| {
            let id = n.attribute((r, "id")).unwrap();
            let target = rels.split("<Relationship ").find(|x| x.contains(&format!("Id=\"{id}\""))).unwrap();
            let part = target.split("Target=\"").nth(1).unwrap().split('"').next().unwrap();
            let part_xml = &parts[&format!("word/{part}")];
            let part_doc = roxmltree::Document::parse(part_xml).unwrap();
            let text: Vec<String> = part_doc.descendants().filter(|p| p.has_tag_name("p")).map(|p| {
                p.descendants().filter_map(|n| {
                    if n.has_tag_name("fldSimple") { Some(format!("{{{}}}", n.attribute((w, "instr")).unwrap().trim())) }
                    else if n.has_tag_name("t") && !n.ancestors().any(|a| a.has_tag_name("fldSimple")) { n.text().map(String::from) }
                    else if n.has_tag_name("tab") && n.parent().is_some_and(|p| p.has_tag_name("r")) { Some("→".into()) }
                    else { None }
                }).collect()
            }).collect();
            (format!("{} {}", n.tag_name().name().trim_end_matches("Reference"), n.attribute((w, "type")).unwrap()), text.join(" | "))
        })
        .collect();
    out.sort();
    out
}

#[test]
fn page_numbers_become_a_footer_and_the_manuscript_header_a_header() {
    let paperback = parts_of(&PAPERBACK);
    assert_eq!(furniture(&paperback), [("footer default".to_string(), "{PAGE}".to_string())]);
    assert!(!paperback["word/document.xml"].contains("titlePg"), "the same on every page");
    let footer = paperback.keys().find(|k| k.starts_with("word/footer")).unwrap();
    assert!(paperback[footer].contains("<w:jc w:val=\"center\"/>"));
    let manuscript = parts_of(&MANUSCRIPT);
    assert_eq!(furniture(&manuscript), [("header default".to_string(), "Fiehn / Rain / {PAGE}".to_string())]);
    let header = manuscript.keys().find(|k| k.starts_with("word/header")).unwrap();
    assert!(manuscript[header].contains("<w:jc w:val=\"right\"/>"));
    // Its style is the header's font, single-spaced.
    assert!(manuscript["word/styles.xml"].contains("w:styleId=\"Header\"><w:name w:val=\"header\"/>"));
}

#[test]
fn a_running_chapter_title_is_a_field_and_the_opening_page_differs() {
    let running = "#set page(header: context { let here-page = here().page(); \
        let opening = query(heading.where(level: 1)).any(h => h.location().page() == here-page); \
        let before = query(heading.where(level: 1).before(here())); \
        if not opening and before.len() > 0 [#smallcaps(before.last().body) #h(1fr) #counter(page).display()] })";
    let mut styles = PAPERBACK.to_vec();
    styles.push(running);
    let options = Options { targets: targets()[..1].to_vec(), ..options() };
    let parts = parts(&convert_text(&source(&styles, &chapters(), true), &options).unwrap().bytes);
    let found = furniture(&parts);
    assert_eq!(found, [
        ("footer default".to_string(), "{PAGE}".to_string()),
        ("footer first".to_string(), "{PAGE}".to_string()),
        // Chapters are Heading 2 here (the test's Work has Parts); title left, number right.
        ("header default".to_string(), "{STYLEREF \"heading 2\"}→→{PAGE}".to_string()),
        ("header first".to_string(), String::new()),
    ], "nothing on a Chapter's first page; the page number below every page");
    assert!(parts["word/document.xml"].matches("<w:titlePg/>").count() >= 3, "in every section");
}

#[test]
fn odd_and_even_pages_can_differ() {
    let sides = "#set page(header: context { if calc.odd(here().page()) { align(right)[Rain] } else { align(left)[R. L. Fiehn] } })";
    let mut styles = PAPERBACK.to_vec();
    styles.push(sides);
    let options = Options { targets: targets()[..1].to_vec(), ..options() };
    let parts = parts(&convert_text(&source(&styles, &chapters(), true), &options).unwrap().bytes);
    let found = furniture(&parts);
    assert!(found.contains(&("header default".to_string(), "Rain".to_string())), "{found:?}");
    assert!(found.contains(&("header even".to_string(), "R. L. Fiehn".to_string())), "{found:?}");
    assert!(parts["word/settings.xml"].contains("<w:evenAndOddHeaders/>"));
}

#[test]
fn headers_with_no_pattern_keep_the_page_number_and_say_so() {
    let random = "#set page(header: context [Page #counter(page).display() of a #(\"long\", \"short\", \"wide\").at(calc.rem(here().page() * 7, 3)) book])";
    let mut styles = PAPERBACK.to_vec();
    styles.push(random);
    let docx = convert_text(&source(&styles, &chapters(), true), &Options { targets: targets()[..1].to_vec(), ..options() }).unwrap();
    assert!(docx.report.iter().any(|n| n.what == What::FurnitureSimplified), "{:?}", docx.report);
    let found = furniture(&parts(&docx.bytes));
    assert!(found.iter().all(|(_, t)| t == "{PAGE}"), "{found:?}");
}

#[test]
fn prose_only_has_a_page_number_at_the_foot() {
    let parts = parts(&convert_text(&source(&PAPERBACK, &chapters(), true), &Options { plain: true, ..options() }).unwrap().bytes);
    assert_eq!(furniture(&parts), [("footer default".to_string(), "{PAGE}".to_string())]);
}

#[test]
fn the_probe_moves_nothing() {
    use typst::layout::FrameItem;
    let positions = |probe: bool| -> Vec<String> {
        let (doc, _) = compile_source(&source(&PAPERBACK, &book(true), probe));
        doc.pages().iter().enumerate().flat_map(|(n, p)| p.frame.items().filter_map(move |(pos, item)| {
            matches!(item, FrameItem::Group(_) | FrameItem::Text(_)).then(|| format!("{n} {:.2} {:.2}", pos.x.to_pt(), pos.y.to_pt()))
        })).collect()
    };
    assert_eq!(positions(true), positions(false));
}

#[test]
fn without_a_probe_it_says_so() {
    assert!(convert_text(&source(&PAPERBACK, "Text.", false), &options()).err().unwrap().contains("probe"));
}

#[test]
fn the_same_input_gives_the_same_file() {
    let text = source(&PAPERBACK, &book(true), true);
    assert_eq!(convert_text(&text, &options()).unwrap().bytes, convert_text(&text, &options()).unwrap().bytes);
}

/// Writes `Paperback.docx`, `Manuscript.docx` and `A4 draft.docx`, with the library's own
/// translations, to the folder in SPIKE_DOCX_DIR, with Typst's own PDF of each to compare with
/// Word's, and prints the report.
#[test]
#[ignore]
fn write_samples() {
    let dir = std::path::PathBuf::from(std::env::var("SPIKE_DOCX_DIR").expect("set SPIKE_DOCX_DIR"));
    for (name, styles) in [("Paperback", &PAPERBACK[..]), ("Manuscript", &MANUSCRIPT[..]), ("A4 draft", &common::A4_DRAFT[..])] {
        let (document, main) = compile_source(&source(styles, &book(true), true));
        let docx = typst_docx::convert(&document, &main, &Options { embed_fonts: true, translations: shipped(styles), ..options() }).unwrap();
        println!("{name}: {:#?}", docx.report);
        let pdf = typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|e| format!("{e:?}")).unwrap();
        std::fs::write(dir.join(format!("{name}-typst.pdf")), pdf).unwrap();
        std::fs::write(dir.join(format!("{name}.docx")), docx.bytes).unwrap();
    }
    // The book with comments (one with a reply, one on the whole Work), and plain.
    let text = source(&PAPERBACK, &book(true), true);
    let range = |s: &str| { let at = text.find(s).unwrap(); at..at + s.len() };
    let person = |name: &str, initials: &str, date: &str, text: &str| (name.to_string(), initials.to_string(), date.to_string(), text.to_string());
    let (a, ai, ad, at) = person("Anna", "A", "2026-11-01T09:00:00Z", "Is this the right quote?");
    let comments = vec![
        Comment { author: a, initials: ai, date: ad, text: at, ranges: vec![range("The night is long")], origin: None,
            replies: vec![Reply { author: "R. L. Fiehn".into(), initials: "RF".into(), date: "2026-11-02T09:00:00Z".into(), text: "Yes, Act 4.".into() }] },
        Comment { author: "Anna".into(), initials: "A".into(), date: "2026-11-03T09:00:00Z".into(), text: "Lovely book overall.".into(), ..Comment::default() },
        Comment { author: "Anna".into(), initials: "A".into(), date: "2026-11-03T10:00:00Z".into(), text: "Garden?".into(), ranges: vec![range("The garden misses you")], ..Comment::default() },
    ];
    // A running Chapter title (none on a Chapter's first page), with Typst's PDF to compare.
    let running = "#set page(header: context { let here-page = here().page();         let opening = query(heading.where(level: 1)).any(h => h.location().page() == here-page);         let before = query(heading.where(level: 1).before(here()));         if not opening and before.len() > 0 [#smallcaps(before.last().body) #h(1fr) #counter(page).display()] })";
    let mut styles = PAPERBACK.to_vec();
    styles.push(running);
    let (document, main) = compile_source(&source(&styles, &chapters(), true));
    let docx = typst_docx::convert(&document, &main, &Options { targets: targets()[..1].to_vec(), ..options() }).unwrap();
    std::fs::write(dir.join("Running.docx"), docx.bytes).unwrap();
    std::fs::write(dir.join("Running-typst.pdf"), typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|e| format!("{e:?}")).unwrap()).unwrap();
    // Lists, small caps, underline and strike-through.
    let marks = "= Lists and marks

Some #underline[underlined], #strike[struck] and #smallcaps[Small Caps] words, and _emphasis_.

        - one
- two, which runs on long enough to wrap onto a second line of the page
  - nested

Between.

        #enum(start: 3)[third][fourth]

#set enum(full: true, numbering: \"1.a.\")
+ first
  + inner
  + inner again
+ second
";
    let (document, main) = compile_source(&source(&PAPERBACK, marks, true));
    let docx = typst_docx::convert(&document, &main, &Options { embed_fonts: true, ..options() }).unwrap();
    std::fs::write(dir.join("Marks.docx"), docx.bytes).unwrap();
    std::fs::write(dir.join("Marks-typst.pdf"), typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|e| format!("{e:?}")).unwrap()).unwrap();
    for (name, plain) in [("Commented", false), ("Plain", true)] {
        let (document, main) = compile_source(&text);
        let docx = typst_docx::convert(&document, &main, &Options { comments: comments.clone(), plain, ..options() }).unwrap();
        std::fs::write(dir.join(format!("{name}.docx")), docx.bytes).unwrap();
    }
}

/// Each embedded font file in a `.docx`, by name.
fn font_files(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let name = file.name().unwrap().to_string();
        if name.ends_with(".odttf") {
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            out.push((name, data));
        }
    }
    out
}

#[test]
fn fonts_are_embedded_whole_as_truetype() {
    let text = source(&PAPERBACK, TEXT, true);
    let docx = convert_text(&text, &Options { embed_fonts: true, ..options() }).unwrap();
    let parts = parts(&docx.bytes);
    assert!(parts["word/settings.xml"].contains("<w:embedTrueTypeFonts/>"));
    assert!(!parts["word/settings.xml"].contains("saveSubsetFonts"), "kept whole when Word saves");
    let table = roxmltree::Document::parse(&parts["word/fontTable.xml"]).unwrap();
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let libertinus = table.descendants().find(|n| n.has_tag_name("font") && n.attribute((w, "name")) == Some("Libertinus Serif")).unwrap();
    let embeds: Vec<&str> = libertinus.children().filter(|n| n.is_element()).map(|n| n.tag_name().name()).collect();
    assert_eq!(embeds, ["embedRegular", "embedBold", "embedItalic"], "the faces the text uses: _on_, *on*");
    let files = font_files(&docx.bytes);
    assert_eq!(files.len(), 3);
    // Obfuscated with its key; undone, it's a TrueType font.
    let key = libertinus.children().find(|n| n.has_tag_name("embedRegular")).unwrap().attribute((w, "fontKey")).unwrap();
    let font = typst_docx::fonts::obfuscate(&files[0].1, key);
    assert_eq!(font[..4], [0, 1, 0, 0], "TrueType outlines");
    assert!(!docx.report.iter().any(|n| matches!(n.what, What::FontNotEmbedded { .. })), "{:?}", docx.report);
    assert_eq!(convert_text(&text, &Options { embed_fonts: true, ..options() }).unwrap().bytes, docx.bytes, "the same file every time");
    // Not asked, or prose only: none.
    assert!(font_files(&convert_text(&text, &options()).unwrap().bytes).is_empty());
    assert!(font_files(&convert_text(&text, &Options { embed_fonts: true, plain: true, ..options() }).unwrap().bytes).is_empty());
}

#[test]
fn hand_written_translations_go_over_what_was_measured() {
    use typst_docx::translate::Translation;
    let translation = |origin: &str, source: &str| Translation { origin: Some(origin.into()), source: source.into() };
    let options = Options {
        translations: vec![
            // An element's own file, then a Style over it.
            translation("epigraph.typ", "/* .docx\nEpigraph: <w:ind w:left=\"720\"/> <w:i w:val=\"0\"/>\n*/"),
            translation("Manuscript bits.typ", "#set text(size: 12pt)\n/* .docx\nEpigraph: <w:ind w:left=\"1440\"/>\nNormal: <w:sz w:val=\"28\"/> <w:spacing w:line=\"480\" w:lineRule=\"auto\"/>\nHeader: <w:jc w:val=\"right\"/>\nPage: <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/>\n*/"),
        ],
        ..options()
    };
    let parts = parts(&convert_text(&source(&MANUSCRIPT, &book(true), true), &options).unwrap().bytes);
    let styles = &parts["word/styles.xml"];
    let style = |name: &str| {
        let at = styles.find(&format!("<w:name w:val=\"{name}\"/>")).unwrap_or_else(|| panic!("no {name}"));
        let start = styles[..at].rfind("<w:style ").unwrap();
        styles[start..start + styles[start..].find("</w:style>").unwrap()].to_string()
    };
    let epigraph = style("Epigraph");
    assert!(epigraph.contains("<w:ind w:left=\"1440\"/>"), "the later source wins: {epigraph}");
    assert!(epigraph.contains("<w:i w:val=\"0\"/>"), "the earlier one's other properties stay: {epigraph}");
    assert!(epigraph.find("<w:ind").unwrap() < epigraph.find("<w:jc").unwrap_or(usize::MAX), "in Word's order");
    let normal = style("Normal");
    assert!(normal.contains("<w:sz w:val=\"28\"/>") && normal.contains("w:line=\"480\""), "{normal}");
    // Based on Normal: it says its own measured size and line spacing, not the hand-written ones.
    let first = style("First Paragraph");
    assert!(first.contains("<w:sz w:val=\"24\"/>") && first.contains("w:line=\"413\""), "{first}");
    assert!(style("header").contains("<w:jc w:val=\"right\"/>"));
    let document = &parts["word/document.xml"];
    let sections = document.matches("<w:sectPr").count();
    assert!(sections > 3);
    assert_eq!(document.matches("<w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\"").count(), sections, "every section's page");
}

/// A style's `<w:style>` element in styles.xml, found by its name in Word.
fn style_named(styles: &str, name: &str) -> String {
    let at = styles.find(&format!("<w:name w:val=\"{name}\"/>")).unwrap_or_else(|| panic!("no {name}"));
    let start = styles[..at].rfind("<w:style ").unwrap();
    styles[start..start + styles[start..].find("</w:style>").unwrap()].to_string()
}

/// The library's own files as translations, in the order the app gives them: the Story
/// elements, then the Look's Styles from the top.
fn shipped(look: &[&str]) -> Vec<typst_docx::translate::Translation> {
    common::ELEMENTS.iter().chain(look).map(|s| typst_docx::translate::Translation { origin: None, source: s.to_string() }).collect()
}

#[test]
fn the_shipped_translations_have_no_mistakes() {
    for (name, source) in common::STYLES {
        assert_eq!(typst_docx::translate::problems(source), Vec::<String>::new(), "{name}");
    }
    for source in common::ELEMENTS {
        assert_eq!(typst_docx::translate::problems(source), Vec::<String>::new(), "{}", source.lines().next().unwrap());
    }
}

#[test]
fn the_shipped_translations_land_on_their_styles() {
    // Manuscript: double-spaced prose, the Chapter (Word's heading 2 under the Part) by its own name.
    let manuscript_options = Options { translations: shipped(&MANUSCRIPT), ..options() };
    let manuscript = parts(&convert_text(&source(&MANUSCRIPT, &book(true), true), &manuscript_options).unwrap().bytes);
    let styles = &manuscript["word/styles.xml"];
    let normal = style_named(styles, "Normal");
    assert!(normal.contains("<w:widowControl/>") && normal.contains("w:line=\"480\""), "{normal}");
    assert!(style_named(styles, "First Paragraph").contains("w:line=\"480\""));
    let chapter = style_named(styles, "heading 2");
    assert!(chapter.contains("<w:keepLines/>") && chapter.contains("<w:suppressAutoHyphens/>"), "{chapter}");
    assert!(style_named(styles, "heading 1").contains("<w:suppressAutoHyphens/>"), "the Part's own");
    assert!(!style_named(styles, "heading 3").contains("<w:keepLines/>"), "not a level-2 heading");
    let epigraph = style_named(styles, "Epigraph");
    assert!(epigraph.contains("<w:keepNext/>") && epigraph.contains("<w:keepLines/>"), "{epigraph}");
    assert!(!epigraph.contains("w:line=\"480\""), "its own measured line spacing: {epigraph}");
    assert!(style_named(styles, "Scene break").contains("<w:keepNext/>"));
    assert!(style_named(styles, "Dedication").contains("<w:keepLines/>"));
    assert!(style_named(styles, "Inclusion title").contains("<w:suppressAutoHyphens/>"));
    assert!(style_named(styles, "header").contains("<w:jc w:val=\"right\"/>"));
    let document = &manuscript["word/document.xml"];
    assert_eq!(document.matches("<w:pgSz w:w=\"12240\" w:h=\"15840\" w:code=\"1\"/>").count(), document.matches("<w:sectPr").count());

    // Paperback and A4 draft: their paper, kerning and widow control, a centred footer.
    for (look, page, paper) in [(&PAPERBACK[..], "w:w=\"7920\" w:h=\"12240\"", "6"), (&common::A4_DRAFT[..], "w:w=\"11906\" w:h=\"16838\"", "9")] {
        let options = Options { translations: shipped(look), ..options() };
        let converted = parts(&convert_text(&source(look, &book(true), true), &options).unwrap().bytes);
        let document = &converted["word/document.xml"];
        let size = format!("<w:pgSz {page} w:code=\"{paper}\"/>");
        assert_eq!(document.matches(&size).count(), document.matches("<w:sectPr").count(), "{size}");
        // The page written by hand is the one the probe reads.
        let measured = parts(&convert_text(&source(look, &book(true), true), &self::options()).unwrap().bytes);
        assert!(measured["word/document.xml"].contains(&format!("<w:pgSz {page}/>")), "{page}");
        let styles = &converted["word/styles.xml"];
        let normal = style_named(styles, "Normal");
        assert!(normal.contains("<w:widowControl/>") && normal.contains("<w:kern w:val=\"2\"/>"), "{normal}");
        assert!(style_named(styles, "footer").contains("<w:jc w:val=\"center\"/>"));
        assert!(style_named(styles, "heading 2").contains("<w:keepLines/>"), "Chapter openings' Chapter");
    }
}

#[test]
fn a_translation_names_a_style_by_its_target_unless_word_has_that_name() {
    use typst_docx::translate::Translation;
    let hand = |source: &str| Options { translations: vec![Translation { origin: None, source: source.into() }], ..options() };
    let styles = |options: &Options| parts(&convert_text(&source(&PAPERBACK, &book(true), true), options).unwrap().bytes)["word/styles.xml"].clone();
    // "Chapter" is Word's heading 2 here, under the Part.
    let by_target = styles(&hand("/* .docx\nChapter: <w:keepLines/>\n*/"));
    assert!(style_named(&by_target, "heading 2").contains("<w:keepLines/>"));
    assert!(!style_named(&by_target, "heading 1").contains("<w:keepLines/>"));
    // "Heading 2" is Word's name for the Chapter, so it isn't the level-2 heading's (Word's heading 3).
    let by_word = styles(&hand("/* .docx\nHeading 2: <w:keepLines/>\n*/"));
    assert!(style_named(&by_word, "heading 2").contains("<w:keepLines/>"));
    assert!(!style_named(&by_word, "heading 3").contains("<w:keepLines/>"));
}

#[test]
fn small_caps_underline_and_strike_through_are_kept() {
    let text = "= One\n\nPlain #underline[under] #strike[struck] #smallcaps[Small Caps] and an #emph[ef]fi#smallcaps[cient] office.";
    let marked = parts(&convert_text(&source(&PAPERBACK, text, true), &options()).unwrap().bytes);
    let doc = roxmltree::Document::parse(&marked["word/document.xml"]).unwrap();
    let props = |text: &str| {
        let run = doc.descendants().find(|n| n.has_tag_name("r") && n.descendants().any(|t| t.has_tag_name("t") && t.text() == Some(text)))
            .unwrap_or_else(|| panic!("no run {text:?}"));
        run.descendants().filter(|n| n.is_element() && n.parent().is_some_and(|p| p.has_tag_name("rPr"))).map(|n| n.tag_name().name().to_string()).collect::<Vec<_>>()
    };
    assert_eq!(props("under"), ["u"]);
    assert_eq!(props("struck"), ["strike"]);
    assert_eq!(props("Small Caps"), ["smallCaps"]);
    assert!(props(" office.").is_empty(), "a ligature (ffi) isn't small caps");
    // A heading the Look sets in small caps: its style is.
    let caps = "#show heading.where(level: 1): smallcaps";
    let mut styles = PAPERBACK.to_vec();
    styles.push(caps);
    let heading = &parts(&convert_text(&source(&styles, "= One\n\nText.", true), &options()).unwrap().bytes)["word/styles.xml"];
    let at = heading.find("w:styleId=\"Heading2\"").unwrap();
    assert!(heading[at..at + heading[at..].find("</w:style>").unwrap()].contains("<w:smallCaps/>"));
}

#[test]
fn lists_are_word_lists_numbered_by_word() {
    let text = "= One\n\nBefore.\n\n- one\n- two\n  - nested\n\nBetween.\n\n#enum(start: 3)[third][fourth]\n\n#set enum(full: true, numbering: \"1.a.\")\n+ first\n  + inner\n";
    let parts = parts(&convert_text(&source(&PAPERBACK, text, true), &options()).unwrap().bytes);
    let xml = &parts["word/document.xml"];
    let paras = paragraphs(xml);
    let texts: Vec<&str> = paras.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(texts, ["One", "Before.", "one", "two", "nested", "Between.", "third", "fourth", "first", "inner"], "markers aren't text");
    assert!(paras[2..5].iter().all(|(s, _)| s == "ListParagraph"));
    let doc = roxmltree::Document::parse(xml).unwrap();
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let num = |text: &str| {
        let p = doc.descendants().find(|n| n.has_tag_name("p") && n.descendants().any(|t| t.has_tag_name("t") && t.text() == Some(text))).unwrap();
        let get = |tag: &str| p.descendants().find(|n| n.has_tag_name(tag)).and_then(|n| n.attribute((w, "val"))).map(String::from);
        (get("numId"), get("ilvl"))
    };
    assert_eq!(num("one"), (Some("1".into()), Some("0".into())));
    assert_eq!(num("nested"), (Some("1".into()), Some("1".into())));
    assert_eq!(num("third"), (Some("2".into()), Some("0".into())), "a list of its own: numbered afresh");
    assert_eq!(num("inner"), (Some("3".into()), Some("1".into())));
    assert_eq!(num("Between."), (None, None));
    let numbering = &parts["word/numbering.xml"];
    for expected in ["<w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"•\"/>", "<w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"‣\"/>",
                     "<w:start w:val=\"3\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%1.\"/>",
                     "<w:numFmt w:val=\"lowerLetter\"/><w:lvlText w:val=\"%1.%2.\"/>"] {
        assert!(numbering.contains(expected), "{expected} in {numbering}");
    }
    assert!(parts["word/styles.xml"].contains("<w:name w:val=\"List Paragraph\"/>"));
}
