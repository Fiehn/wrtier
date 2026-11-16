//! Translations written by hand: the third layer, over what's measured and what the probe
//! reads.
//!
//! A source file (a Typst style, say) may hold one block of them, in a comment:
//!
//! ```text
//! /* .docx
//! Normal: <w:spacing w:line="480" w:lineRule="auto"/>
//! Epigraph: <w:ind w:left="1440"/> <w:i/>
//! Page: <w:pgMar w:top="1440" w:bottom="1440"
//!         w:left="1440" w:right="1440"/>
//! */
//! ```
//!
//! Each line names a target by its Word style name (or `Page`, for every section's page) and
//! gives raw paragraph, run or section properties for it; a line starting with spaces carries
//! on the one before. Properties may come in any order: they're put in Word's. Several sources
//! are given in stacking order, and a later one wins, one property element at a time.

/// A source that may hold a block of translations, and where it came from (handed back in the
/// report, uninterpreted).
#[derive(Clone, Debug, Default)]
pub struct Translation {
    pub origin: Option<String>,
    pub source: String,
}

/// The kinds of properties a `.docx` style or section has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Paragraph,
    Run,
    Section,
}

/// Paragraph properties, in Word's order (ECMA-376 `CT_PPrBase`).
pub(crate) const PARAGRAPH: &[&str] = &[
    "pStyle", "keepNext", "keepLines", "pageBreakBefore", "framePr", "widowControl", "numPr", "suppressLineNumbers",
    "pBdr", "shd", "tabs", "suppressAutoHyphens", "kinsoku", "wordWrap", "overflowPunct", "topLinePunct", "autoSpaceDE",
    "autoSpaceDN", "bidi", "adjustRightInd", "snapToGrid", "spacing", "ind", "contextualSpacing", "mirrorIndents",
    "suppressOverlap", "jc", "textDirection", "textAlignment", "textboxTightWrap", "outlineLvl", "divId", "cnfStyle",
];

/// Run properties, in Word's order (`CT_RPr`).
pub(crate) const RUN: &[&str] = &[
    "rStyle", "rFonts", "b", "bCs", "i", "iCs", "caps", "smallCaps", "strike", "dstrike", "outline", "shadow", "emboss",
    "imprint", "noProof", "snapToGrid", "vanish", "webHidden", "color", "spacing", "w", "kern", "position", "sz", "szCs",
    "highlight", "u", "effect", "bdr", "shd", "fitText", "vertAlign", "rtl", "cs", "em", "lang", "eastAsianLayout",
    "specVanish", "oMath",
];

/// Section properties a translation may set, in Word's order (`CT_SectPr`, without the header
/// and footer references, which the converter makes).
pub(crate) const SECTION: &[&str] = &[
    "footnotePr", "endnotePr", "type", "pgSz", "pgMar", "paperSrc", "pgBorders", "lnNumType", "pgNumType", "cols",
    "formProt", "vAlign", "noEndnote", "titlePg", "textDirection", "bidi", "rtlGutter", "docGrid",
];

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// One property element: its local name, its kind and its XML as written.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Property {
    pub name: String,
    pub kind: Kind,
    pub xml: String,
}

/// A line of a block: a target and its properties.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub target: String,
    pub properties: Vec<Property>,
}

/// Where in a source its block is: the text after `/* .docx` up to `*/`, and the line it starts on.
fn block(source: &str) -> Option<(&str, usize)> {
    let start = source.find("/* .docx")?;
    let body_start = start + "/* .docx".len();
    let end = source[body_start..].find("*/").map_or(source.len(), |e| body_start + e);
    let line = source[..body_start].matches('\n').count() + 1;
    Some((&source[body_start..end], line))
}

/// What kind a property element is, for a target (`Page` takes section properties). Paragraph
/// and run properties share a few names; `spacing` with a `w:val` is letter spacing (a run's).
fn kind_of(name: &str, node: &roxmltree::Node, page: bool) -> Option<Kind> {
    if page {
        return SECTION.contains(&name).then_some(Kind::Section);
    }
    if name == "spacing" {
        return Some(if node.attribute((W, "val")).is_some() { Kind::Run } else { Kind::Paragraph });
    }
    if PARAGRAPH.contains(&name) && name != "pStyle" {
        Some(Kind::Paragraph)
    } else if RUN.contains(&name) && name != "rStyle" {
        Some(Kind::Run)
    } else {
        None
    }
}

/// The source's block, read: its entries, and what's wrong with any line (as "line n: …",
/// counted in the whole source). A source with no block has neither.
pub(crate) fn read(source: &str) -> (Vec<Entry>, Vec<String>) {
    let Some((text, first_line)) = block(source) else { return (Vec::new(), Vec::new()) };
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    // Lines joined with their continuations, each with the line it starts on.
    let mut logical: Vec<(usize, String)> = Vec::new();
    for (n, line) in text.split('\n').enumerate() {
        let number = first_line + n;
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) && let Some((_, last)) = logical.last_mut() {
            last.push(' ');
            last.push_str(line.trim());
        } else {
            logical.push((number, line.trim().to_string()));
        }
    }
    for (number, line) in logical {
        let Some((target, xml)) = line.split_once(':').filter(|(t, _)| !t.contains('<')) else {
            problems.push(format!("line {number}: should be a style's name, a colon, then its properties"));
            continue;
        };
        let target = target.trim().to_string();
        if target.is_empty() {
            problems.push(format!("line {number}: names no style"));
            continue;
        }
        let page = target.eq_ignore_ascii_case("Page");
        let wrapped = format!("<r xmlns:w=\"{W}\">{xml}</r>");
        let doc = match roxmltree::Document::parse(&wrapped) {
            Ok(doc) => doc,
            Err(e) => {
                problems.push(format!("line {number}: isn't well-formed XML ({e})"));
                continue;
            }
        };
        let mut properties = Vec::new();
        let mut bad = Vec::new();
        for node in doc.root_element().children() {
            if node.is_text() {
                if !node.text().unwrap_or("").trim().is_empty() {
                    bad.push("text outside an element".to_string());
                }
                continue;
            }
            if !node.is_element() {
                continue;
            }
            let name = node.tag_name().name();
            match (node.tag_name().namespace() == Some(W)).then(|| kind_of(name, &node, page)).flatten() {
                Some(kind) => properties.push(Property { name: name.to_string(), kind, xml: wrapped[node.range()].to_string() }),
                None if page => bad.push(format!("<w:{name}> isn't a page property Word knows")),
                None => bad.push(format!("<w:{name}> isn't a paragraph or text property Word knows")),
            }
        }
        if !bad.is_empty() {
            problems.push(format!("line {number}: {}", bad.join("; ")));
        } else if properties.is_empty() {
            problems.push(format!("line {number}: gives {target} no properties"));
        } else {
            entries.push(Entry { target, properties });
        }
    }
    (entries, problems)
}

/// What's wrong with a source's block of translations, for showing where it's edited.
pub fn problems(source: &str) -> Vec<String> {
    read(source).1
}

/// Every target's hand-written properties, from sources in stacking order: for each target
/// (as named), its properties, a later source's replacing an earlier one's of the same name.
pub(crate) fn merged(translations: &[Translation]) -> Vec<(String, Vec<Property>)> {
    let mut out: Vec<(String, Vec<Property>)> = Vec::new();
    for t in translations {
        for entry in read(&t.source).0 {
            let at = match out.iter().position(|(target, _)| target.eq_ignore_ascii_case(&entry.target)) {
                Some(at) => at,
                None => {
                    out.push((entry.target.clone(), Vec::new()));
                    out.len() - 1
                }
            };
            for p in entry.properties {
                let list = &mut out[at].1;
                match list.iter_mut().find(|q| q.name == p.name && q.kind == p.kind) {
                    Some(q) => *q = p,
                    None => list.push(p),
                }
            }
        }
    }
    out
}

/// Properties (generated ones as `(name, xml)`) with hand-written ones over them, in Word's
/// order for their kind.
pub(crate) fn overlay(generated: Vec<(String, String)>, hand: &[&Property], order: &[&str]) -> String {
    let mut all = generated;
    for p in hand {
        match all.iter_mut().find(|(n, _)| *n == p.name) {
            Some(slot) => slot.1 = p.xml.clone(),
            None => all.push((p.name.clone(), p.xml.clone())),
        }
    }
    all.sort_by_key(|(n, _)| order.iter().position(|o| o == n).unwrap_or(usize::MAX));
    all.into_iter().map(|(_, x)| x).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_is_read_line_by_line() {
        let source = "#set text(size: 11pt)\n/* .docx\nNormal: <w:spacing w:line=\"480\" w:lineRule=\"auto\"/>\nEpigraph: <w:ind w:left=\"1440\"/> <w:i/>\n  <w:spacing w:val=\"20\"/>\nPage: <w:pgMar w:top=\"1440\"/>\n*/\n";
        let (entries, problems) = read(source);
        assert!(problems.is_empty(), "{problems:?}");
        let kinds: Vec<(String, Vec<(String, Kind)>)> = entries.iter()
            .map(|e| (e.target.clone(), e.properties.iter().map(|p| (p.name.clone(), p.kind)).collect()))
            .collect();
        assert_eq!(kinds, [
            ("Normal".into(), vec![("spacing".into(), Kind::Paragraph)]),
            ("Epigraph".into(), vec![("ind".into(), Kind::Paragraph), ("i".into(), Kind::Run), ("spacing".into(), Kind::Run)]),
            ("Page".into(), vec![("pgMar".into(), Kind::Section)]),
        ]);
        assert_eq!(entries[1].properties[0].xml, "<w:ind w:left=\"1440\"/>");
    }

    #[test]
    fn what_word_wouldnt_take_is_a_problem_with_its_line() {
        let source = "x\n/* .docx\nNormal <w:i/>\nEpigraph: <w:i>\nHeading 1: <w:frobnicate/>\nPage: <w:jc w:val=\"center\"/>\nFine: <w:b/>\n*/";
        let problems = problems(source);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(problems[0].starts_with("line 3: should be"));
        assert!(problems[1].starts_with("line 4: isn't well-formed"));
        assert!(problems[2].starts_with("line 5: <w:frobnicate>"));
        assert!(problems[3].starts_with("line 6: <w:jc> isn't a page property"));
        assert!(super::problems("#set text(size: 11pt)").is_empty(), "no block, no problems");
    }

    #[test]
    fn a_later_source_wins_one_property_at_a_time() {
        let a = Translation { origin: None, source: "/* .docx\nEpigraph: <w:i/> <w:ind w:left=\"720\"/>\n*/".into() };
        let b = Translation { origin: None, source: "/* .docx\nepigraph: <w:ind w:left=\"1440\"/>\n*/".into() };
        let merged = merged(&[a, b]);
        assert_eq!(merged.len(), 1);
        let xml: Vec<&str> = merged[0].1.iter().map(|p| p.xml.as_str()).collect();
        assert_eq!(xml, ["<w:i/>", "<w:ind w:left=\"1440\"/>"]);
        let refs: Vec<&Property> = merged[0].1.iter().collect();
        let paragraph: Vec<&Property> = refs.iter().copied().filter(|p| p.kind == Kind::Paragraph).collect();
        assert_eq!(
            overlay(vec![("jc".into(), "<w:jc w:val=\"both\"/>".into()), ("ind".into(), "<w:ind w:left=\"0\"/>".into())], &paragraph, PARAGRAPH),
            "<w:ind w:left=\"1440\"/><w:jc w:val=\"both\"/>",
        );
    }
}
