//! The `.docx` Export: a Work in standard manuscript format, keeping only its structure and
//! emphasis (Looks don't apply). The text is read with Typst's own parser, so it's understood
//! exactly as the Page preview understands it; each Story element does what its definition's
//! ".docx:" line says (v1's fixed menu, `library::DOCX`). Anything a `.docx` can't carry (Typst
//! code, maths, Typst's own functions) is listed in the "left out" report; a function with text
//! inside it keeps its text.
//!
//! The file is written by hand: a `.docx` is a zip of a few XML files.

use std::io::Write;
use serde::Serialize;
use typst::syntax::{LinkedNode, Source, SyntaxKind};
use crate::preamble::ElementForm;

/// How a paragraph is set. Each is a style in the `.docx`, so the Author can restyle it in Word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    /// Ordinary prose: indented first line.
    Body,
    /// A Chapter's title: a new page, centred, a third of the way down.
    Chapter,
    /// A Part's title: a new page, centred, further down.
    Part,
    /// A scene break: a centred `#`.
    SceneBreak,
    Centered,
    /// Set in from both margins (an epigraph, a letter).
    Block,
}

impl Style {
    fn id(self) -> &'static str {
        match self {
            Style::Body => "Body",
            Style::Chapter => "ChapterHeading",
            Style::Part => "PartHeading",
            Style::SceneBreak => "SceneBreak",
            Style::Centered => "Centered",
            Style::Block => "BlockText",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Run {
    text: String,
    italic: bool,
    bold: bool,
    /// A line break (Typst's `\` at a line's end) instead of text.
    br: bool,
}

#[derive(Debug, PartialEq)]
struct Para {
    style: Style,
    runs: Vec<Run>,
}

/// Something the `.docx` couldn't carry, and where it is.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LeftOut {
    /// 1-based, in the Work's text.
    pub line: usize,
    pub what: String,
}

/// What goes around the text: the running header, the document's properties, its font.
pub struct Manuscript<'a> {
    pub title: &'a str,
    pub author: &'a str,
    /// E.g. "en"; empty for English.
    pub language: &'a str,
    /// "Times New Roman" or "Courier New".
    pub font: &'a str,
    /// The time of the Work's newest Change: the document's date, never today's.
    pub date_ms: u64,
}

/// Turns the Work's text into manuscript paragraphs.
struct Reader<'a> {
    source: &'a Source,
    elements: &'a [ElementForm],
    paras: Vec<Para>,
    runs: Vec<Run>,
    /// How paragraphs are set here (inside an epigraph, a heading…).
    style: Style,
    italic: u32,
    bold: u32,
    left_out: Vec<LeftOut>,
}

impl Reader<'_> {
    fn line(&self, node: &LinkedNode) -> usize {
        self.source.lines().byte_to_line(node.offset()).map_or(1, |l| l + 1)
    }

    fn leave_out(&mut self, node: &LinkedNode, what: String) {
        let line = self.line(node);
        self.left_out.push(LeftOut { line, what });
    }

    fn push_text(&mut self, text: &str) {
        // No space at the start of a line after a line break.
        let text = if self.runs.last().is_some_and(|r| r.br) { text.trim_start() } else { text };
        if text.is_empty() {
            return;
        }
        let (italic, bold) = (self.italic > 0, self.bold > 0);
        match self.runs.last_mut() {
            Some(run) if !run.br && run.italic == italic && run.bold == bold => run.text.push_str(text),
            _ => self.runs.push(Run { text: text.to_string(), italic, bold, br: false }),
        }
    }

    /// The paragraph so far ends here (a blank line, a heading, a block).
    fn flush(&mut self) {
        let mut runs = std::mem::take(&mut self.runs);
        if let Some(first) = runs.first_mut() {
            first.text = first.text.trim_start().to_string();
        }
        if let Some(last) = runs.last_mut() {
            last.text = last.text.trim_end().to_string();
        }
        runs.retain(|r| r.br || !r.text.is_empty());
        if runs.iter().any(|r| !r.text.trim().is_empty()) {
            self.paras.push(Para { style: self.style, runs });
        }
    }

    /// The last character written in this paragraph, for working out which way a quote faces.
    fn last_char(&self) -> Option<char> {
        self.runs.iter().rev().find_map(|r| if r.br { Some('\n') } else { r.text.chars().last() })
    }

    /// Reads the content of `node` as paragraphs in `style` (a heading, an epigraph…).
    fn as_paragraphs(&mut self, node: &LinkedNode, style: Style) {
        self.flush();
        let outer = std::mem::replace(&mut self.style, style);
        self.read(node);
        self.flush();
        self.style = outer;
    }

    fn read(&mut self, node: &LinkedNode) {
        match node.kind() {
            SyntaxKind::Markup | SyntaxKind::ContentBlock => {
                let mut after_hash = false;
                for child in node.children() {
                    if child.kind() == SyntaxKind::Hash {
                        after_hash = true;
                        continue;
                    }
                    if after_hash {
                        self.code(&child);
                        after_hash = false;
                    } else {
                        self.read(&child);
                    }
                }
            }
            SyntaxKind::Text => self.push_text(node.get().leaf_text()),
            SyntaxKind::Space => {
                if !self.runs.is_empty() {
                    self.push_text(" ");
                }
            }
            SyntaxKind::Parbreak => self.flush(),
            SyntaxKind::Linebreak => {
                // Nor at the end of the line before it.
                if let Some(last) = self.runs.last_mut().filter(|r| !r.br) {
                    last.text.truncate(last.text.trim_end().len());
                }
                self.runs.push(Run { br: true, ..Run::default() });
            }
            SyntaxKind::Escape => self.push_text(&unescape(node.get().leaf_text())),
            SyntaxKind::Shorthand => self.push_text(match node.get().leaf_text().as_str() {
                "~" => "\u{a0}",
                "---" => "—",
                "--" => "–",
                "..." => "…",
                "-?" => "\u{ad}",
                other => other,
            }),
            SyntaxKind::SmartQuote => {
                let opening = self.last_char().is_none_or(|c| c.is_whitespace() || "([{“‘—–-/".contains(c));
                let double = node.get().leaf_text() == "\"";
                self.push_text(match (double, opening) {
                    (true, true) => "“",
                    (true, false) => "”",
                    (false, true) => "‘",
                    (false, false) => "’",
                });
            }
            SyntaxKind::Strong => {
                self.bold += 1;
                for child in node.children() {
                    self.read(&child);
                }
                self.bold -= 1;
            }
            SyntaxKind::Emph => {
                self.italic += 1;
                for child in node.children() {
                    self.read(&child);
                }
                self.italic -= 1;
            }
            SyntaxKind::Raw => {
                let text = node.get().full_text();
                self.push_text(text.trim_matches('`'));
            }
            SyntaxKind::Link => self.push_text(&node.get().full_text()),
            SyntaxKind::Heading => {
                let level = node.children().find(|c| c.kind() == SyntaxKind::HeadingMarker).map_or(1, |m| m.get().leaf_text().len());
                let body = node.children().find(|c| c.kind() == SyntaxKind::Markup);
                if let Some(body) = body {
                    if level == 1 {
                        self.as_paragraphs(&body, Style::Chapter); // a Chapter
                    } else {
                        self.bold += 1;
                        self.as_paragraphs(&body, Style::Centered);
                        self.bold -= 1;
                    }
                }
            }
            SyntaxKind::ListItem | SyntaxKind::EnumItem | SyntaxKind::TermItem => {
                self.flush();
                self.push_text(if node.kind() == SyntaxKind::EnumItem { "– " } else { "• " });
                for child in node.children().filter(|c| c.kind() == SyntaxKind::Markup) {
                    self.read(&child);
                }
                self.flush();
            }
            SyntaxKind::Equation => self.leave_out(node, "maths ($…$)".into()),
            SyntaxKind::Ref => self.leave_out(node, format!("a reference ({})", node.get().leaf_text())),
            // Comments, labels and the markers themselves aren't text.
            _ => {}
        }
    }

    /// Code after a `#`: a Story element, one of Typst's functions, or a rule.
    fn code(&mut self, node: &LinkedNode) {
        let first_line = || node.get().full_text().lines().next().unwrap_or("").chars().take(60).collect::<String>();
        match node.kind() {
            SyntaxKind::FuncCall => self.call(node),
            SyntaxKind::LetBinding | SyntaxKind::SetRule | SyntaxKind::ShowRule => {
                self.leave_out(node, format!("#{} (Typst code)", first_line()));
            }
            _ => self.leave_out(node, format!("#{} (Typst code)", first_line())),
        }
    }

    fn call(&mut self, node: &LinkedNode) {
        let name = node.children().next().filter(|c| c.kind() == SyntaxKind::Ident).map(|c| c.get().leaf_text().to_string());
        let bodies: Vec<LinkedNode> = node.children()
            .filter(|c| c.kind() == SyntaxKind::Args)
            .flat_map(|args| args.children().filter(|c| c.kind() == SyntaxKind::ContentBlock).collect::<Vec<_>>())
            .collect();
        let element = name.as_deref().and_then(|n| self.elements.iter().find(|e| e.name == n));
        let behaviour = element.and_then(|e| e.docx.clone());
        let read_bodies = |me: &mut Self, style: Option<Style>| {
            for body in &bodies {
                match style {
                    Some(style) => me.as_paragraphs(body, style),
                    None => me.read(body),
                }
            }
        };
        match behaviour.as_deref() {
            Some("chapter heading") => read_bodies(self, Some(Style::Chapter)),
            Some("part heading") => read_bodies(self, Some(Style::Part)),
            Some("centred #") => {
                self.flush();
                self.paras.push(Para { style: Style::SceneBreak, runs: vec![Run { text: "#".into(), ..Run::default() }] });
            }
            Some("indented block") => read_bodies(self, Some(Style::Block)),
            Some("centred paragraph") => read_bodies(self, Some(Style::Centered)),
            Some("plain text") => read_bodies(self, None),
            Some("left out") => {} // as its definition says
            _ => {
                // One of Typst's own functions (or an element without a ".docx:" line): its text
                // is kept, the rest left out.
                let shown = name.unwrap_or_else(|| "a function".into());
                if bodies.is_empty() {
                    self.leave_out(node, format!("#{shown}(…)"));
                } else {
                    self.leave_out(node, format!("#{shown}[…]: only its text is kept"));
                    read_bodies(self, None);
                }
            }
        }
    }
}

/// `\#` → `#`, `\u{1F600}` → 😀.
fn unescape(escape: &str) -> String {
    let rest = escape.strip_prefix('\\').unwrap_or(escape);
    if let Some(hex) = rest.strip_prefix("u{").and_then(|h| h.strip_suffix('}')) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(String::from).unwrap_or_default();
    }
    rest.to_string()
}

/// Reads a Work's text into manuscript paragraphs, and what was left out.
fn read(text: &str, elements: &[ElementForm]) -> (Vec<Para>, Vec<LeftOut>) {
    let source = Source::detached(text);
    let mut reader = Reader {
        source: &source, elements, paras: Vec::new(), runs: Vec::new(), style: Style::Body,
        italic: 0, bold: 0, left_out: Vec::new(),
    };
    reader.read(&LinkedNode::new(source.root()));
    reader.flush();
    (reader.paras, reader.left_out)
}

/// Text for XML.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn document_xml(paras: &[Para]) -> String {
    let mut body = String::new();
    for p in paras {
        body.push_str(&format!("<w:p><w:pPr><w:pStyle w:val=\"{}\"/></w:pPr>", p.style.id()));
        for run in &p.runs {
            if run.br {
                body.push_str("<w:r><w:br/></w:r>");
                continue;
            }
            body.push_str("<w:r>");
            if run.italic || run.bold {
                body.push_str("<w:rPr>");
                if run.bold { body.push_str("<w:b/>"); }
                if run.italic { body.push_str("<w:i/>"); }
                body.push_str("</w:rPr>");
            }
            body.push_str(&format!("<w:t xml:space=\"preserve\">{}</w:t></w:r>", esc(&run.text)));
        }
        body.push_str("</w:p>");
    }
    if paras.is_empty() {
        body.push_str("<w:p/>");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:body>{body}\
         <w:sectPr><w:headerReference w:type=\"default\" r:id=\"rIdHeader\"/>\
         <w:pgSz w:w=\"12240\" w:h=\"15840\"/>\
         <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/>\
         </w:sectPr></w:body></w:document>")
}

fn styles_xml(font: &str, language: &str) -> String {
    let lang = if language.is_empty() { "en-US".to_string() } else { esc(language) };
    let font = esc(font);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:styles xmlns:w=\"{W}\">\
         <w:docDefaults><w:rPrDefault><w:rPr>\
         <w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:eastAsia=\"{font}\" w:cs=\"{font}\"/>\
         <w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/><w:lang w:val=\"{lang}\"/>\
         </w:rPr></w:rPrDefault>\
         <w:pPrDefault><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"480\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault>\
         </w:docDefaults>\
         <w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Body\"><w:name w:val=\"Manuscript Body\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/>\
         <w:pPr><w:ind w:firstLine=\"720\"/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"ChapterHeading\"><w:name w:val=\"Chapter Heading\"/><w:basedOn w:val=\"Normal\"/>\
         <w:next w:val=\"Body\"/><w:qFormat/><w:pPr><w:keepNext/><w:pageBreakBefore/><w:spacing w:before=\"3600\" w:after=\"480\"/>\
         <w:jc w:val=\"center\"/><w:outlineLvl w:val=\"0\"/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"PartHeading\"><w:name w:val=\"Part Heading\"/><w:basedOn w:val=\"Normal\"/>\
         <w:next w:val=\"Body\"/><w:qFormat/><w:pPr><w:keepNext/><w:pageBreakBefore/><w:spacing w:before=\"5040\" w:after=\"480\"/>\
         <w:jc w:val=\"center\"/><w:outlineLvl w:val=\"0\"/></w:pPr><w:rPr><w:caps/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"SceneBreak\"><w:name w:val=\"Scene Break\"/><w:basedOn w:val=\"Normal\"/>\
         <w:next w:val=\"Body\"/><w:qFormat/><w:pPr><w:jc w:val=\"center\"/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Centered\"><w:name w:val=\"Centred\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/>\
         <w:pPr><w:jc w:val=\"center\"/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"BlockText\"><w:name w:val=\"Block Text\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/>\
         <w:pPr><w:ind w:left=\"1440\" w:right=\"720\"/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Header\"><w:name w:val=\"header\"/><w:basedOn w:val=\"Normal\"/>\
         <w:pPr><w:spacing w:line=\"240\" w:lineRule=\"auto\"/><w:jc w:val=\"right\"/></w:pPr></w:style>\
         </w:styles>")
}

/// "Surname / Title / 7" at the top right of every page.
fn header_xml(m: &Manuscript) -> String {
    let surname = m.author.split_whitespace().last().unwrap_or("");
    let lead: Vec<&str> = [surname, m.title.trim()].into_iter().filter(|s| !s.is_empty()).collect();
    let lead = if lead.is_empty() { String::new() } else { format!("{} / ", lead.join(" / ")) };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:hdr xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:p><w:pPr><w:pStyle w:val=\"Header\"/></w:pPr>\
         <w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>\
         <w:fldSimple w:instr=\" PAGE \"><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:hdr>",
        esc(&lead))
}

fn core_xml(m: &Manuscript) -> String {
    let (y, mo, d, h, mi, s) = crate::typeset::utc_parts(m.date_ms);
    let date = format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
         <dc:title>{}</dc:title><dc:creator>{}</dc:creator>\
         <dcterms:created xsi:type=\"dcterms:W3CDTF\">{date}</dcterms:created>\
         <dcterms:modified xsi:type=\"dcterms:W3CDTF\">{date}</dcterms:modified>\
         </cp:coreProperties>",
        esc(m.title), esc(m.author))
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
    <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
    <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
    <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
    <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
    <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
    <Override PartName=\"/word/header1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>\
    <Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
    </Types>";

const PACKAGE_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
    <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
    <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
    <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
    </Relationships>";

const DOCUMENT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
    <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
    <Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
    <Relationship Id=\"rIdHeader\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header1.xml\"/>\
    </Relationships>";

/// A `.docx` Export.
#[derive(Debug)]
pub struct ExportedDocx {
    pub bytes: Vec<u8>,
    pub left_out: Vec<LeftOut>,
    pub paragraphs: usize,
}

/// The Work's text (without its Preamble) as a manuscript `.docx`. `elements`: its Story
/// elements, with what each does in a `.docx`.
pub fn docx(text: &str, elements: &[ElementForm], m: &Manuscript) -> Result<ExportedDocx, String> {
    let (paras, left_out) = read(text, elements);
    let parts = [
        ("[Content_Types].xml", CONTENT_TYPES.to_string()),
        ("_rels/.rels", PACKAGE_RELS.to_string()),
        ("word/document.xml", document_xml(&paras)),
        ("word/styles.xml", styles_xml(m.font, m.language)),
        ("word/header1.xml", header_xml(m)),
        ("word/_rels/document.xml.rels", DOCUMENT_RELS.to_string()),
        ("docProps/core.xml", core_xml(m)),
    ];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    // A fixed time and permissions: the same version gives the same file.
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    let fail = |e: &dyn std::fmt::Display| format!("The .docx couldn't be made: {e}");
    for (name, content) in parts {
        zip.start_file(name, options).map_err(|e| fail(&e))?;
        zip.write_all(content.as_bytes()).map_err(|e| fail(&e))?;
    }
    let bytes = zip.finish().map_err(|e| fail(&e))?.into_inner();
    Ok(ExportedDocx { bytes, left_out, paragraphs: paras.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elements() -> Vec<ElementForm> {
        crate::library::built_in_elements().map(|(name, content)| ElementForm::of(name, content)).collect()
    }

    fn paras(text: &str) -> Vec<(Style, String)> {
        let (paras, _) = read(text, &elements());
        paras.into_iter().map(|p| {
            let text = p.runs.iter().map(|r| if r.br { "|".to_string() } else { r.text.clone() }).collect();
            (p.style, text)
        }).collect()
    }

    #[test]
    fn prose_becomes_manuscript_paragraphs() {
        let p = paras("= The Long Night\n\nThe rain had\nstopped -- or \"nearly\" had...\n\nShe said, \"It's late.\"");
        assert_eq!(p, [
            (Style::Chapter, "The Long Night".to_string()),
            (Style::Body, "The rain had stopped – or “nearly” had…".to_string()),
            (Style::Body, "She said, “It’s late.”".to_string()),
        ]);
    }

    #[test]
    fn emphasis_is_italic_and_strong_is_bold() {
        let (paras, _) = read("Not _now_, *never*.", &elements());
        let runs = &paras[0].runs;
        assert_eq!(runs.iter().find(|r| r.text == "now").map(|r| (r.italic, r.bold)), Some((true, false)));
        assert_eq!(runs.iter().find(|r| r.text == "never").map(|r| (r.italic, r.bold)), Some((false, true)));
    }

    #[test]
    fn story_elements_do_what_their_definitions_say() {
        let p = paras("#part[Book One]\n\n#dedication[For M.]\n\n#epigraph[Long night. \\\n— Someone]\n\nOne.\n\n#scene-break()\n\nTwo.\n\n#block-text[Dear Anna,\n\nCome home.]");
        assert_eq!(p, [
            (Style::Part, "Book One".to_string()),
            (Style::Centered, "For M.".to_string()),
            (Style::Block, "Long night.|— Someone".to_string()),
            (Style::Body, "One.".to_string()),
            (Style::SceneBreak, "#".to_string()),
            (Style::Body, "Two.".to_string()),
            (Style::Block, "Dear Anna,".to_string()),
            (Style::Block, "Come home.".to_string()),
        ]);
    }

    #[test]
    fn what_a_docx_cant_carry_is_reported_and_text_is_kept() {
        let text = "#set text(size: 12pt)\n\nThe sum $a + b$ is #text(fill: red)[red].\n\n#pagebreak()\n\nA \\# sign.";
        let (paras, left_out) = read(text, &elements());
        let all: String = paras.iter().flat_map(|p| p.runs.iter().map(|r| r.text.clone())).collect::<Vec<_>>().join("\n");
        assert!(all.contains("is red."), "{all}");
        assert!(all.contains("A # sign."));
        let what: Vec<(usize, &str)> = left_out.iter().map(|l| (l.line, l.what.as_str())).collect();
        assert_eq!(what, [
            (1, "#set text(size: 12pt) (Typst code)"),
            (3, "maths ($…$)"),
            (3, "#text[…]: only its text is kept"),
            (5, "#pagebreak(…)"),
        ]);
    }

    /// Writes a sample manuscript to the path in WRTIER_DOCX_OUT, for opening in Word and
    /// LibreOffice by hand (`cargo test -p wrtier_app write_a_sample_docx -- --ignored`).
    #[test]
    #[ignore]
    fn write_a_sample_docx() {
        let path = std::env::var("WRTIER_DOCX_OUT").expect("set WRTIER_DOCX_OUT to where it goes");
        let text = "#dedication[For M., who waited.]\n\n#part[Book One]\n\n= The Long Night\n\n\
            #epigraph[The night is long that never finds the day. \\\n— Macbeth]\n\n\
            The rain had stopped -- or \"nearly\" had... She didn't _quite_ believe it, *not yet*.\n\n\
            Æblegrød, smørrebrød & søster: \"Hvad så?\" said Ånund.\n\n#scene-break()\n\n\
            == A smaller heading\n\nAfter the break, the letter came.\n\n\
            #block-text[Dear Anna,\n\nCome home. The garden misses you.\n\nYours, \\\nM.]\n\nShe went.";
        let m = Manuscript { title: "The Long Night", author: "R. L. Fiehn", language: "en", font: "Times New Roman", date_ms: 1_794_268_799_000 };
        let made = docx(text, &elements(), &m).unwrap();
        assert!(made.left_out.is_empty(), "{:?}", made.left_out);
        std::fs::write(&path, made.bytes).unwrap();
    }

    #[test]
    fn the_file_is_a_well_formed_docx_and_the_same_every_time() {
        let m = Manuscript { title: "Rain", author: "R. L. Fiehn", language: "en", font: "Times New Roman", date_ms: 1_700_000_000_000 };
        let text = "= One\n\nThe rain had _stopped_ & \"so\" had <it>.\n\n#scene-break()\n\nAfter.";
        let a = docx(text, &elements(), &m).unwrap();
        assert_eq!(docx(text, &elements(), &m).unwrap().bytes, a.bytes, "the same version, the same file");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(a.bytes)).unwrap();
        for name in ["[Content_Types].xml", "_rels/.rels", "word/document.xml", "word/styles.xml", "word/header1.xml", "word/_rels/document.xml.rels", "docProps/core.xml"] {
            let mut xml = String::new();
            std::io::Read::read_to_string(&mut zip.by_name(name).unwrap(), &mut xml).unwrap();
            roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{name} isn't well-formed XML: {e}"));
            if name == "word/header1.xml" {
                assert!(xml.contains("Fiehn / Rain / "));
            }
            if name == "docProps/core.xml" {
                assert!(xml.contains("2023-11-14T22:13:20Z"));
            }
        }
    }
}
