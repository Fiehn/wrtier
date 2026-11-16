//! Writing the `.docx`: a zip of a few XML parts, written by hand.
//!
//! Word is strict about the order of elements inside `pPr`, `rPr`, `sectPr` and `settings`
//! (ECMA-376's sequences), so each is written in schema order.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;

use crate::measure::{Align, Run};
use crate::probe::Setup;
use crate::styles::{Kind, Laid, Props};
use crate::{Comment, Metadata, Note, What};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
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

/// A run's formatting beyond its paragraph's style; only italic and bold when `plain`.
fn run_props(run: &Run, style: &Props, plain: bool) -> Props {
    let italic = (style.italic.unwrap_or(false) != run.italic).then_some(run.italic);
    let bold = (style.bold.unwrap_or(false) != run.bold).then_some(run.bold);
    if plain {
        return Props { italic, bold, ..Props::default() };
    }
    let fill = run.fill.trim_start_matches('#').chars().take(6).collect::<String>().to_uppercase();
    Props {
        family: (style.family.as_deref() != Some(run.family.as_str())).then(|| run.family.clone()),
        size: style.size.is_none_or(|s| (s - run.size).abs() >= 0.05).then_some(run.size),
        italic,
        bold,
        fill: (style.fill.as_deref().unwrap_or("000000") != fill).then_some(fill),
        ..Props::default()
    }
}

/// One run of `text`: tabs, soft hyphens, non-breaking hyphens and line breaks are elements of
/// their own.
fn run_xml(out: &mut String, text: &str, props: &str) {
    if text.is_empty() {
        return;
    }
    let _ = write!(out, "<w:r>{props}");
    let mut plain = String::new();
    let flush = |out: &mut String, plain: &mut String| {
        if !plain.is_empty() {
            let _ = write!(out, "<w:t xml:space=\"preserve\">{}</w:t>", esc(plain));
            plain.clear();
        }
    };
    for c in text.chars() {
        let element = match c {
            '\t' => "<w:tab/>",
            '\u{ad}' => "<w:softHyphen/>",
            '\u{2011}' => "<w:noBreakHyphen/>",
            '\n' => "<w:br/>",
            _ => {
                plain.push(c);
                continue;
            }
        };
        flush(out, &mut plain);
        out.push_str(element);
    }
    flush(out, &mut plain);
    out.push_str("</w:r>");
}

/// A place in the paragraphs: paragraph, run, character in the run.
type Place = (usize, usize, usize);

/// A comment thread written into the document: the Word IDs of the comment and its replies,
/// all anchored on the same stretch of text.
struct Thread<'a> {
    comment: &'a Comment,
    ids: Vec<usize>,
}

/// Where each comment starts and ends (before the character at the first place, after the one
/// at the second); `None` for one none of whose text is in the document. A comment on the
/// document as a whole sits at its very start.
fn anchors(laid: &Laid, comments: &[Comment]) -> Vec<Option<(Place, Place)>> {
    let mut found: Vec<Option<(Place, Place)>> = vec![None; comments.len()];
    let start = (0, 0, 0);
    for (c, comment) in comments.iter().enumerate() {
        if comment.ranges.is_empty() {
            found[c] = Some((start, start));
        }
    }
    let wanted: Vec<usize> = (0..comments.len()).filter(|c| !comments[*c].ranges.is_empty()).collect();
    if wanted.is_empty() {
        return found;
    }
    for (p, para) in laid.paras.iter().enumerate() {
        for (r, run) in para.runs.iter().enumerate() {
            for (k, at) in run.at.iter().enumerate() {
                let Some(at) = at else { continue };
                for &c in &wanted {
                    if comments[c].ranges.iter().any(|range| range.contains(at)) {
                        let here = (p, r, k);
                        found[c] = Some(match found[c] {
                            Some((first, _)) => (first, (p, r, k + 1)),
                            None => (here, (p, r, k + 1)),
                        });
                    }
                }
            }
        }
    }
    found
}

/// Markers to put in before characters: comment range starts and ends, and the reference run
/// after an end, keyed by place.
fn markers(threads: &[Thread], anchors: &[Option<(Place, Place)>]) -> BTreeMap<Place, String> {
    let mut at: BTreeMap<Place, String> = BTreeMap::new();
    // Ends before starts at one place: a comment ending where another begins.
    let mut ends: BTreeMap<Place, String> = BTreeMap::new();
    for (t, thread) in threads.iter().enumerate() {
        let Some((start, end)) = anchors[t] else { continue };
        for id in &thread.ids {
            let _ = write!(at.entry(start).or_default(), "<w:commentRangeStart w:id=\"{id}\"/>");
            let close = format!("<w:commentRangeEnd w:id=\"{id}\"/><w:r><w:commentReference w:id=\"{id}\"/></w:r>");
            // A comment on no text (the whole document) ends right after it starts.
            let into = if start == end { &mut at } else { &mut ends };
            into.entry(end).or_default().push_str(&close);
        }
    }
    for (place, xml) in ends {
        let entry = at.entry(place).or_default();
        *entry = xml + entry;
    }
    at
}

/// A section's properties; `next_page` for a section that ends where the next page begins.
/// With no page given (`plain`), Word's own page.
fn sect_pr(setup: &Setup, next_page: bool, plain: bool) -> String {
    if plain {
        return "<w:sectPr/>".into();
    }
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

fn document_xml(laid: &Laid, setup: &Setup, marks: &BTreeMap<Place, String>) -> String {
    let resolved: Vec<Props> = (0..laid.styles.len()).map(|s| laid.resolved(s)).collect();
    // Each paragraph's properties and content, apart, so a section break can go in before it.
    let mut paras: Vec<(Props, Option<&str>, Option<String>, String)> = Vec::new();
    for (p, para) in laid.paras.iter().enumerate() {
        let style = &laid.styles[para.style];
        let id = (style.kind != Kind::Normal).then_some(style.id.as_str());
        let mut content = String::new();
        for (r, run) in para.runs.iter().enumerate() {
            let props = rpr(&run_props(run, &resolved[para.style], laid.plain), None);
            // The run in pieces, with the markers that fall inside it between them.
            let chars: Vec<char> = run.text.chars().collect();
            let mut from = 0;
            for (&(_, _, k), xml) in marks.range((p, r, 0)..=(p, r, chars.len())) {
                run_xml(&mut content, &chars[from..k].iter().collect::<String>(), &props);
                content.push_str(xml);
                from = k;
            }
            run_xml(&mut content, &chars[from..].iter().collect::<String>(), &props);
        }
        // Word 2013 and later drop the space before a paragraph at the top of a page after a
        // page break, but not at the start of a section: so a paragraph that opens a page
        // (its style still says so, for paragraphs added in Word) begins a section, and keeps
        // its distance from the top of the page.
        let opens_page = para.direct.page_break.or(resolved[para.style].page_break) == Some(true);
        if opens_page && p > 0 && !laid.plain && let Some(before) = paras.last_mut() {
            before.2 = Some(sect_pr(setup, true, false));
        }
        paras.push((para.direct.clone(), id, None, content));
    }
    let mut body = String::new();
    for (direct, id, section, content) in &paras {
        let mut props = ppr(direct, *id, None);
        if let Some(section) = section {
            props = match props.strip_suffix("</w:pPr>") {
                Some(open) => format!("{open}{section}</w:pPr>"),
                None => format!("<w:pPr>{section}</w:pPr>"),
            };
        }
        let _ = write!(body, "<w:p>{props}{content}</w:p>");
    }
    if laid.paras.is_empty() {
        let start = marks.get(&(0, 0, 0)).cloned().unwrap_or_default();
        let _ = write!(body, "<w:p>{start}</w:p>");
    }
    format!("{HEAD}<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:body>{body}{}</w:body></w:document>", sect_pr(setup, false, laid.plain))
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

fn settings_xml(setup: &Setup, plain: bool) -> String {
    let mirror = if setup.margins.mirrored && !plain { "<w:mirrorMargins/>" } else { "" };
    let hyphenate = if setup.hyphenate && !plain { "<w:autoHyphenation/>" } else { "" };
    // doNotExpandShiftReturn: a forced line break in a justified paragraph isn't stretched,
    // as in Typst.
    format!(
        "{HEAD}<w:settings xmlns:w=\"{W}\">{mirror}<w:defaultTabStop w:val=\"720\"/>{hyphenate}\
         <w:characterSpacingControl w:val=\"doNotCompress\"/>\
         <w:compat><w:doNotExpandShiftReturn/>\
         <w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/>\
         </w:compat></w:settings>")
}

/// Word's paragraph IDs (`w14:paraId`), which tie a reply to its comment: 8 hex digits below
/// 0x80000000, the same for the same seed.
struct ParaIds(u64);

impl ParaIds {
    fn next(&mut self) -> String {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        format!("{:08X}", ((z as u32) & 0x7FFF_FFFF).max(1))
    }
}

/// `comments.xml` and `commentsExtended.xml`: each comment and reply with its text as
/// paragraphs, and which comment each reply answers.
fn comments_xml(threads: &[Thread], seed: u64) -> (String, String) {
    let mut ids = ParaIds(seed);
    let mut comments = format!("{HEAD}<w:comments xmlns:w=\"{W}\" xmlns:w14=\"{W14}\" xmlns:mc=\"{MC}\" mc:Ignorable=\"w14\">");
    let mut extended = format!("{HEAD}<w15:commentsEx xmlns:w15=\"{W15}\" xmlns:mc=\"{MC}\" mc:Ignorable=\"w15\">");
    let mut one = |out: &mut String, id: usize, author: &str, initials: &str, date: &str, text: &str| -> String {
        let _ = write!(out, "<w:comment w:id=\"{id}\" w:author=\"{}\" w:date=\"{}\" w:initials=\"{}\">", esc(author), esc(date), esc(initials));
        let lines: Vec<&str> = if text.is_empty() { vec![""] } else { text.lines().collect() };
        let mut last = String::new();
        for line in lines {
            last = ids.next();
            let _ = write!(out, "<w:p w14:paraId=\"{last}\">");
            run_xml(out, line, "");
            out.push_str("</w:p>");
        }
        out.push_str("</w:comment>");
        last
    };
    for thread in threads {
        let c = thread.comment;
        let parent = one(&mut comments, thread.ids[0], &c.author, &c.initials, &c.date, &c.text);
        let _ = write!(extended, "<w15:commentEx w15:paraId=\"{parent}\" w15:done=\"0\"/>");
        for (reply, id) in c.replies.iter().zip(&thread.ids[1..]) {
            let para = one(&mut comments, *id, &reply.author, &reply.initials, &reply.date, &reply.text);
            let _ = write!(extended, "<w15:commentEx w15:paraId=\"{para}\" w15:paraIdParent=\"{parent}\" w15:done=\"0\"/>");
        }
    }
    comments.push_str("</w:comments>");
    extended.push_str("</w15:commentsEx>");
    (comments, extended)
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

fn content_types(comments: bool) -> String {
    let comments = if comments {
        "<Override PartName=\"/word/comments.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml\"/>\
         <Override PartName=\"/word/commentsExtended.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml\"/>"
    } else {
        ""
    };
    format!(
        "{HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
         <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
         <Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>\
         {comments}\
         <Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
         </Types>")
}

const PACKAGE_RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
    <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
    <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
    </Relationships>";

fn document_rels(comments: bool) -> String {
    let comments = if comments {
        "<Relationship Id=\"rIdComments\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments\" Target=\"comments.xml\"/>\
         <Relationship Id=\"rIdCommentsEx\" Type=\"http://schemas.microsoft.com/office/2011/relationships/commentsExtended\" Target=\"commentsExtended.xml\"/>"
    } else {
        ""
    };
    format!(
        "{HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
         <Relationship Id=\"rIdSettings\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>\
         {comments}</Relationships>")
}

/// The parts of the package, by name, and a note for each comment that couldn't be placed.
pub(crate) fn parts(laid: &Laid, setup: &Setup, meta: &Metadata, comments: &[Comment]) -> (Vec<(&'static str, String)>, Vec<Note>) {
    let anchors = anchors(laid, comments);
    let mut notes = Vec::new();
    let mut threads: Vec<Thread> = Vec::new();
    let mut placed: Vec<Option<(Place, Place)>> = Vec::new();
    let mut next_id = 0;
    for (comment, anchor) in comments.iter().zip(anchors) {
        if anchor.is_none() {
            let text: String = comment.text.chars().take(40).collect();
            notes.push(Note { style: None, origin: comment.origin.clone(), what: What::CommentNotPlaced { text } });
            continue;
        }
        let ids: Vec<usize> = (next_id..next_id + 1 + comment.replies.len()).collect();
        next_id += ids.len();
        threads.push(Thread { comment, ids });
        placed.push(anchor);
    }
    let marks = markers(&threads, &placed);
    let any = !threads.is_empty();
    let mut parts = vec![
        ("[Content_Types].xml", content_types(any)),
        ("_rels/.rels", format!("{HEAD}{PACKAGE_RELS}")),
        ("word/document.xml", document_xml(laid, setup, &marks)),
        ("word/styles.xml", styles_xml(laid, setup)),
        ("word/settings.xml", settings_xml(setup, laid.plain)),
        ("word/_rels/document.xml.rels", document_rels(any)),
        ("docProps/core.xml", core_xml(meta)),
    ];
    if any {
        let (comments, extended) = comments_xml(&threads, meta.seed);
        parts.push(("word/comments.xml", comments));
        parts.push(("word/commentsExtended.xml", extended));
    }
    (parts, notes)
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
