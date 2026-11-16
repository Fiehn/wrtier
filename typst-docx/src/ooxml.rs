//! Writing the `.docx`: a zip of a few XML parts, written by hand.
//!
//! Word is strict about the order of elements inside `pPr`, `rPr`, `sectPr` and `settings`
//! (ECMA-376's sequences), so each is written in schema order.

use std::fmt::Write as _;
use std::io::Write as _;

use crate::measure::{Align, Run};
use crate::probe::Setup;
use crate::styles::{Kind, Laid, Props};
use crate::Metadata;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// Points to twentieths of a point.
fn tw(pt: f64) -> i64 {
    (pt * 20.0).round() as i64
}

/// Text for XML, without the characters XML can't hold.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => {}
            c => out.push(c),
        }
    }
    out
}

/// Paragraph properties, in schema order.
fn ppr(p: &Props, style: Option<&str>, outline: Option<u8>) -> String {
    let mut x = String::new();
    if let Some(id) = style {
        let _ = write!(x, "<w:pStyle w:val=\"{}\"/>", esc(id));
    }
    if let Some(k) = p.keep_next {
        x.push_str(if k { "<w:keepNext/>" } else { "<w:keepNext w:val=\"0\"/>" });
    }
    if let Some(b) = p.page_break {
        x.push_str(if b { "<w:pageBreakBefore/>" } else { "<w:pageBreakBefore w:val=\"0\"/>" });
    }
    if p.before.is_some() || p.after.is_some() || p.line.is_some() {
        x.push_str("<w:spacing");
        if let Some(b) = p.before {
            let _ = write!(x, " w:before=\"{}\"", tw(b));
        }
        if let Some(a) = p.after {
            let _ = write!(x, " w:after=\"{}\"", tw(a));
        }
        if let Some(l) = p.line {
            let _ = write!(x, " w:line=\"{}\" w:lineRule=\"auto\"", (l * 240.0).round() as i64);
        }
        x.push_str("/>");
    }
    if p.left.is_some() || p.right.is_some() || p.first_line.is_some() {
        x.push_str("<w:ind");
        if let Some(l) = p.left {
            let _ = write!(x, " w:left=\"{}\"", tw(l));
        }
        if let Some(r) = p.right {
            let _ = write!(x, " w:right=\"{}\"", tw(r));
        }
        match p.first_line {
            Some(f) if f < 0.0 => { let _ = write!(x, " w:hanging=\"{}\"", tw(-f)); }
            Some(f) => { let _ = write!(x, " w:firstLine=\"{}\"", tw(f)); }
            None => {}
        }
        x.push_str("/>");
    }
    if let Some(a) = p.align {
        let jc = match a {
            Align::Left | Align::Unknown => "left",
            Align::Centre => "center",
            Align::Right => "right",
            Align::Justified => "both",
        };
        let _ = write!(x, "<w:jc w:val=\"{jc}\"/>");
    }
    if let Some(level) = outline {
        let _ = write!(x, "<w:outlineLvl w:val=\"{}\"/>", level.saturating_sub(1));
    }
    if x.is_empty() { x } else { format!("<w:pPr>{x}</w:pPr>") }
}

/// Run properties, in schema order; `lang` only in the document's defaults.
fn rpr(p: &Props, lang: Option<&str>) -> String {
    let mut x = String::new();
    if let Some(f) = &p.family {
        let f = esc(f);
        let _ = write!(x, "<w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:eastAsia=\"{f}\" w:cs=\"{f}\"/>");
    }
    let flag = |x: &mut String, name: &str, on: Option<bool>| {
        if let Some(on) = on {
            let v = if on { "" } else { " w:val=\"0\"" };
            let _ = write!(x, "<w:{name}{v}/><w:{name}Cs{v}/>");
        }
    };
    flag(&mut x, "b", p.bold);
    flag(&mut x, "i", p.italic);
    if let Some(c) = &p.fill {
        let _ = write!(x, "<w:color w:val=\"{}\"/>", esc(c));
    }
    if let Some(s) = p.size {
        let half = (s * 2.0).round() as i64;
        let _ = write!(x, "<w:sz w:val=\"{half}\"/><w:szCs w:val=\"{half}\"/>");
    }
    if let Some(l) = lang {
        let _ = write!(x, "<w:lang w:val=\"{}\"/>", esc(l));
    }
    if x.is_empty() { x } else { format!("<w:rPr>{x}</w:rPr>") }
}

/// A run's formatting beyond its paragraph's style.
fn run_props(run: &Run, style: &Props) -> Props {
    let fill = run.fill.trim_start_matches('#').chars().take(6).collect::<String>().to_uppercase();
    Props {
        family: (style.family.as_deref() != Some(run.family.as_str())).then(|| run.family.clone()),
        size: style.size.is_none_or(|s| (s - run.size).abs() >= 0.05).then_some(run.size),
        italic: (style.italic.unwrap_or(false) != run.italic).then_some(run.italic),
        bold: (style.bold.unwrap_or(false) != run.bold).then_some(run.bold),
        fill: (style.fill.as_deref().unwrap_or("000000") != fill).then_some(fill),
        ..Props::default()
    }
}

/// A run's text: tabs, soft hyphens and non-breaking hyphens are elements of their own.
fn run_xml(out: &mut String, run: &Run, props: &str) {
    if run.text == "\n" {
        let _ = write!(out, "<w:r>{props}<w:br/></w:r>");
        return;
    }
    let _ = write!(out, "<w:r>{props}");
    let mut text = String::new();
    let flush = |out: &mut String, text: &mut String| {
        if !text.is_empty() {
            let _ = write!(out, "<w:t xml:space=\"preserve\">{}</w:t>", esc(text));
            text.clear();
        }
    };
    for c in run.text.chars() {
        let element = match c {
            '\t' => "<w:tab/>",
            '\u{ad}' => "<w:softHyphen/>",
            '\u{2011}' => "<w:noBreakHyphen/>",
            '\n' => "<w:br/>",
            _ => {
                text.push(c);
                continue;
            }
        };
        flush(out, &mut text);
        out.push_str(element);
    }
    flush(out, &mut text);
    out.push_str("</w:r>");
}

/// A section's properties; `next_page` for a section that ends where the next page begins.
fn sect_pr(setup: &Setup, next_page: bool) -> String {
    let m = &setup.margins;
    // Typst puts a header's baseline 30% of the top margin above the body (`header-ascent`).
    let header = tw(m.top * 0.4);
    let footer = tw(m.bottom * 0.4);
    let kind = if next_page { "<w:type w:val=\"nextPage\"/>" } else { "" };
    format!(
        "<w:sectPr>{kind}<w:pgSz w:w=\"{}\" w:h=\"{}\"/>\
         <w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"{header}\" w:footer=\"{footer}\" w:gutter=\"0\"/>\
         </w:sectPr>",
        tw(setup.width), tw(setup.height), tw(m.top), tw(m.right), tw(m.bottom), tw(m.left))
}

fn document_xml(laid: &Laid, setup: &Setup) -> String {
    let resolved: Vec<Props> = (0..laid.styles.len()).map(|s| laid.resolved(s)).collect();
    // Each paragraph's properties and runs, apart, so a section break can go in before it.
    let mut paras: Vec<(Props, Option<&str>, Option<String>, String)> = Vec::new();
    for (i, p) in laid.paras.iter().enumerate() {
        let style = &laid.styles[p.style];
        let id = (style.kind != Kind::Normal).then_some(style.id.as_str());
        let mut runs = String::new();
        for run in &p.runs {
            run_xml(&mut runs, run, &rpr(&run_props(run, &resolved[p.style]), None));
        }
        // Word 2013 and later drop the space before a paragraph at the top of a page after a
        // page break, but not at the start of a section: so a paragraph that opens a page
        // (its style still says so, for paragraphs added in Word) begins a section, and keeps
        // its distance from the top of the page.
        let opens_page = p.direct.page_break.or(resolved[p.style].page_break) == Some(true);
        if opens_page && i > 0 && let Some(before) = paras.last_mut() {
            before.2 = Some(sect_pr(setup, true));
        }
        paras.push((p.direct.clone(), id, None, runs));
    }
    let mut body = String::new();
    for (direct, id, section, runs) in &paras {
        let mut props = ppr(direct, *id, None);
        if let Some(section) = section {
            props = match props.strip_suffix("</w:pPr>") {
                Some(open) => format!("{open}{section}</w:pPr>"),
                None => format!("<w:pPr>{section}</w:pPr>"),
            };
        }
        let _ = write!(body, "<w:p>{props}{runs}</w:p>");
    }
    if laid.paras.is_empty() {
        body.push_str("<w:p/>");
    }
    format!("{HEAD}<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:body>{body}{}</w:body></w:document>", sect_pr(setup, false))
}

fn styles_xml(laid: &Laid, setup: &Setup) -> String {
    let normal = laid.resolved(0);
    let defaults = Props { family: normal.family.clone(), size: normal.size, ..Props::default() };
    let mut x = format!(
        "{HEAD}<w:styles xmlns:w=\"{W}\"><w:docDefaults><w:rPrDefault>{}</w:rPrDefault>\
         <w:pPrDefault><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>",
        rpr(&defaults, Some(&setup.language())));
    for (i, s) in laid.styles.iter().enumerate() {
        let own = laid.own(i);
        let default = if s.kind == Kind::Normal { " w:default=\"1\"" } else { "" };
        let _ = write!(x, "<w:style w:type=\"paragraph\"{default} w:styleId=\"{}\"><w:name w:val=\"{}\"/>", esc(&s.id), esc(&s.name));
        if let Some(b) = s.based_on {
            let _ = write!(x, "<w:basedOn w:val=\"{}\"/>", esc(&laid.styles[b].id));
        }
        if let Some(n) = s.next {
            let _ = write!(x, "<w:next w:val=\"{}\"/>", esc(&laid.styles[n].id));
        }
        x.push_str("<w:qFormat/>");
        let outline = match s.kind {
            Kind::Heading(n) => Some(n),
            _ => None,
        };
        x.push_str(&ppr(&own, None, outline));
        x.push_str(&rpr(&own, None));
        x.push_str("</w:style>");
    }
    x.push_str("</w:styles>");
    x
}

fn settings_xml(setup: &Setup) -> String {
    let mirror = if setup.margins.mirrored { "<w:mirrorMargins/>" } else { "" };
    let hyphenate = if setup.hyphenate { "<w:autoHyphenation/>" } else { "" };
    // doNotExpandShiftReturn: a forced line break in a justified paragraph isn't stretched,
    // as in Typst.
    format!(
        "{HEAD}<w:settings xmlns:w=\"{W}\">{mirror}<w:defaultTabStop w:val=\"720\"/>{hyphenate}\
         <w:characterSpacingControl w:val=\"doNotCompress\"/>\
         <w:compat><w:doNotExpandShiftReturn/>\
         <w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/>\
         </w:compat></w:settings>")
}

fn core_xml(m: &Metadata) -> String {
    let date = esc(&m.date);
    format!(
        "{HEAD}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
         <dc:title>{}</dc:title><dc:creator>{}</dc:creator>\
         <dcterms:created xsi:type=\"dcterms:W3CDTF\">{date}</dcterms:created>\
         <dcterms:modified xsi:type=\"dcterms:W3CDTF\">{date}</dcterms:modified>\
         </cp:coreProperties>",
        esc(&m.title), esc(&m.author))
}

const CONTENT_TYPES: &str = "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
    <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
    <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
    <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
    <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
    <Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>\
    <Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
    </Types>";

const PACKAGE_RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
    <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
    <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
    </Relationships>";

const DOCUMENT_RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
    <Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
    <Relationship Id=\"rIdSettings\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>\
    </Relationships>";

/// The parts of the package, by name.
pub(crate) fn parts(laid: &Laid, setup: &Setup, meta: &Metadata) -> Vec<(&'static str, String)> {
    vec![
        ("[Content_Types].xml", format!("{HEAD}{CONTENT_TYPES}")),
        ("_rels/.rels", format!("{HEAD}{PACKAGE_RELS}")),
        ("word/document.xml", document_xml(laid, setup)),
        ("word/styles.xml", styles_xml(laid, setup)),
        ("word/settings.xml", settings_xml(setup)),
        ("word/_rels/document.xml.rels", format!("{HEAD}{DOCUMENT_RELS}")),
        ("docProps/core.xml", core_xml(meta)),
    ]
}

/// The zipped package. The same input gives the same bytes: no times or permissions vary.
pub(crate) fn zip(parts: &[(&str, String)]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for (name, content) in parts {
        zip.start_file(*name, options).map_err(|e| e.to_string())?;
        zip.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}
