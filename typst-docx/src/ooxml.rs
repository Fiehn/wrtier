//! Writing the `.docx`: a zip of a few XML parts, written by hand.
//!
//! Word is strict about the order of elements inside `pPr`, `rPr`, `sectPr` and `settings`
//! (ECMA-376's sequences), so each is written in schema order.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;

use crate::fonts::Family;
use crate::furniture::{Format, Furniture, Lines, Look, Piece, Side};
use crate::translate::{self, Kind as TKind, Property};
use crate::measure::{Align, Run};
use crate::probe::{Setup, Setups};
use crate::styles::{Kind, Laid, ListItem, Props};
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

/// Paragraph properties as (element name, XML), in schema order.
fn ppr_items(p: &Props, style: Option<&str>, outline: Option<u8>) -> Vec<(String, String)> {
    let mut x: Vec<(String, String)> = Vec::new();
    let mut add = |name: &str, xml: String| x.push((name.to_string(), xml));
    if let Some(id) = style {
        add("pStyle", format!("<w:pStyle w:val=\"{}\"/>", esc(id)));
    }
    if let Some(k) = p.keep_next {
        add("keepNext", if k { "<w:keepNext/>".into() } else { "<w:keepNext w:val=\"0\"/>".into() });
    }
    if let Some(b) = p.page_break {
        add("pageBreakBefore", if b { "<w:pageBreakBefore/>".into() } else { "<w:pageBreakBefore w:val=\"0\"/>".into() });
    }
    if p.before.is_some() || p.after.is_some() || p.line.is_some() {
        let mut e = String::from("<w:spacing");
        if let Some(b) = p.before {
            let _ = write!(e, " w:before=\"{}\"", tw(b));
        }
        if let Some(a) = p.after {
            let _ = write!(e, " w:after=\"{}\"", tw(a));
        }
        if let Some(l) = p.line {
            let _ = write!(e, " w:line=\"{}\" w:lineRule=\"auto\"", (l * 240.0).round() as i64);
        }
        e.push_str("/>");
        add("spacing", e);
    }
    if p.left.is_some() || p.right.is_some() || p.first_line.is_some() {
        let mut e = String::from("<w:ind");
        if let Some(l) = p.left {
            let _ = write!(e, " w:left=\"{}\"", tw(l));
        }
        if let Some(r) = p.right {
            let _ = write!(e, " w:right=\"{}\"", tw(r));
        }
        match p.first_line {
            Some(f) if f < 0.0 => { let _ = write!(e, " w:hanging=\"{}\"", tw(-f)); }
            Some(f) => { let _ = write!(e, " w:firstLine=\"{}\"", tw(f)); }
            None => {}
        }
        e.push_str("/>");
        add("ind", e);
    }
    if let Some(a) = p.align {
        let jc = match a {
            Align::Left | Align::Unknown => "left",
            Align::Centre => "center",
            Align::Right => "right",
            Align::Justified => "both",
        };
        add("jc", format!("<w:jc w:val=\"{jc}\"/>"));
    }
    if let Some(level) = outline {
        add("outlineLvl", format!("<w:outlineLvl w:val=\"{}\"/>", level.saturating_sub(1)));
    }
    x
}

/// Run properties as (element name, XML), in schema order; `lang` only in the document's defaults.
fn rpr_items(p: &Props, lang: Option<&str>) -> Vec<(String, String)> {
    let mut x: Vec<(String, String)> = Vec::new();
    if let Some(f) = &p.family {
        let f = esc(f);
        x.push(("rFonts".into(), format!("<w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:eastAsia=\"{f}\" w:cs=\"{f}\"/>")));
    }
    for (name, on) in [("b", p.bold), ("i", p.italic)] {
        if let Some(on) = on {
            let v = if on { "" } else { " w:val=\"0\"" };
            x.push((name.into(), format!("<w:{name}{v}/>")));
            x.push((format!("{name}Cs"), format!("<w:{name}Cs{v}/>")));
        }
    }
    for (name, on) in [("smallCaps", p.small_caps), ("strike", p.strike)] {
        if let Some(on) = on {
            x.push((name.into(), if on { format!("<w:{name}/>") } else { format!("<w:{name} w:val=\"0\"/>") }));
        }
    }
    if let Some(c) = &p.fill {
        x.push(("color".into(), format!("<w:color w:val=\"{}\"/>", esc(c))));
    }
    if let Some(s) = p.size {
        let half = (s * 2.0).round() as i64;
        x.push(("sz".into(), format!("<w:sz w:val=\"{half}\"/>")));
        x.push(("szCs".into(), format!("<w:szCs w:val=\"{half}\"/>")));
    }
    if let Some(u) = p.underline {
        x.push(("u".into(), format!("<w:u w:val=\"{}\"/>", if u { "single" } else { "none" })));
    }
    if let Some(l) = lang {
        x.push(("lang".into(), format!("<w:lang w:val=\"{}\"/>", esc(l))));
    }
    // In Word's order, whichever order they were made in.
    x.sort_by_key(|(n, _)| translate::RUN.iter().position(|o| o == n).unwrap_or(usize::MAX));
    x
}

/// Run properties, in schema order; `lang` only in the document's defaults.
fn rpr(p: &Props, lang: Option<&str>) -> String {
    wrap("rPr", rpr_items(p, lang).into_iter().map(|(_, x)| x).collect())
}

/// `<w:name>…</w:name>` around what's given, or nothing for nothing.
fn wrap(name: &str, inner: String) -> String {
    if inner.is_empty() { inner } else { format!("<w:{name}>{inner}</w:{name}>") }
}

/// The hand-written properties of one kind (see `translate`) for the targets `named` accepts.
fn hand<'a>(hand: &'a [(String, Vec<Property>)], named: impl Fn(&str) -> bool, kind: TKind) -> Vec<&'a Property> {
    hand.iter().filter(|(t, _)| named(t)).flat_map(|(_, ps)| ps.iter().filter(|p| p.kind == kind)).collect()
}

/// A run's formatting beyond its paragraph's style; only italic and bold when `plain`.
fn run_props(run: &Run, style: &Props, plain: bool) -> Props {
    let italic = (style.italic.unwrap_or(false) != run.italic).then_some(run.italic);
    let bold = (style.bold.unwrap_or(false) != run.bold).then_some(run.bold);
    let flag = |style: Option<bool>, run: bool| (style.unwrap_or(false) != run).then_some(run);
    let (small_caps, underline, strike) = (flag(style.small_caps, run.small_caps), flag(style.underline, run.underline), flag(style.strike, run.strike));
    if plain {
        return Props { italic, bold, ..Props::default() };
    }
    let fill = run.fill.trim_start_matches('#').chars().take(6).collect::<String>().to_uppercase();
    Props {
        family: (style.family.as_deref() != Some(run.family.as_str())).then(|| run.family.clone()),
        size: style.size.is_none_or(|s| (s - run.size).abs() >= 0.05).then_some(run.size),
        italic,
        bold,
        small_caps,
        underline,
        strike,
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
/// document as a whole sits at its very start. A comment stays in the story it starts in (the
/// body, or one footnote): Word's can't run from one into another.
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
                        found[c] = match found[c] {
                            Some((first, _)) if laid.paras[first.0].note == para.note => Some((first, (p, r, k + 1))),
                            Some(other) => Some(other),
                            None => Some((here, (p, r, k + 1))),
                        };
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

/// A list level's number format and text, from its first item's marker: a bullet as it is; a
/// number (digits, letters or roman numerals) as Word's `%n`, the marker's last number being
/// this level's and any before it its parents' ("1.a." at the second level: "%1.%2."); and the
/// number it starts at.
fn level_format(marker: &str, level: usize, numbered: bool) -> (&'static str, String, u32) {
    let tokens: Vec<(usize, usize)> = {
        let mut out = Vec::new();
        let mut start: Option<usize> = None;
        for (i, c) in marker.char_indices().chain(std::iter::once((marker.len(), ' '))) {
            match (start, c.is_alphanumeric()) {
                (None, true) => start = Some(i),
                (Some(s), false) => {
                    out.push((s, i));
                    start = None;
                }
                _ => {}
            }
        }
        out
    };
    let Some(&(from, to)) = tokens.last().filter(|_| numbered) else {
        return ("bullet", marker.to_string(), 1);
    };
    let last = &marker[from..to];
    let roman = |s: &str| s.chars().all(|c| "ivxlcdm".contains(c.to_ascii_lowercase()));
    let roman_value = |s: &str| {
        let value = |c: char| match c.to_ascii_lowercase() { 'i' => 1, 'v' => 5, 'x' => 10, 'l' => 50, 'c' => 100, 'd' => 500, _ => 1000 };
        let digits: Vec<u32> = s.chars().map(value).collect();
        digits.iter().enumerate().map(|(i, &d)| if digits.get(i + 1).is_some_and(|&n| n > d) { -(d as i64) } else { d as i64 }).sum::<i64>().max(1) as u32
    };
    let (format, start) = if let Ok(n) = last.parse::<u32>() {
        ("decimal", n)
    } else if roman(last) && (last.starts_with(['i', 'I']) || last.len() > 1) {
        (if last.chars().all(char::is_lowercase) { "lowerRoman" } else { "upperRoman" }, roman_value(last))
    } else {
        let c = last.chars().next().unwrap_or('a');
        (if c.is_lowercase() { "lowerLetter" } else { "upperLetter" }, (c.to_ascii_lowercase() as u32).saturating_sub('a' as u32) + 1)
    };
    let mut text = String::new();
    let mut at = 0;
    let n = tokens.len();
    for (k, &(a, b)) in tokens.iter().enumerate() {
        text.push_str(&marker[at..a]);
        let of = (level + 1 + k).saturating_sub(n);
        let _ = write!(text, "%{}", of + 1);
        at = b;
    }
    text.push_str(&marker[at..]);
    (format, text, start)
}

/// `numbering.xml`: each list a Word list of its own (so each starts its numbering afresh, as in
/// Typst), its levels as its items showed them. None without lists.
fn numbering_xml(laid: &Laid) -> Option<String> {
    let mut lists: std::collections::BTreeMap<usize, std::collections::BTreeMap<usize, &ListItem>> = Default::default();
    for item in laid.paras.iter().filter_map(|p| p.list.as_ref()) {
        lists.entry(item.list).or_default().entry(item.level).or_insert(item);
    }
    if lists.is_empty() {
        return None;
    }
    let mut abstracts = String::new();
    let mut nums = String::new();
    for (list, levels) in &lists {
        let _ = write!(abstracts, "<w:abstractNum w:abstractNumId=\"{list}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>");
        let deepest = levels.keys().max().copied().unwrap_or(0);
        for level in 0..=deepest {
            let (format, text, start, left, hanging) = match levels.get(&level) {
                Some(item) => {
                    let (format, text, start) = level_format(&item.marker, level, item.numbered);
                    (format, text, start, item.left, item.hanging)
                }
                // A level no item shows: a bullet, a step further in.
                None => ("bullet", "•".to_string(), 1, 18.0 * (level as f64 + 1.0), 18.0),
            };
            let _ = write!(abstracts,
                "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"{start}\"/><w:numFmt w:val=\"{format}\"/><w:lvlText w:val=\"{}\"/><w:lvlJc w:val=\"left\"/>\
                 <w:pPr><w:ind w:left=\"{}\" w:hanging=\"{}\"/></w:pPr></w:lvl>",
                esc(&text), tw(left), tw(hanging));
        }
        abstracts.push_str("</w:abstractNum>");
        let _ = write!(nums, "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{list}\"/></w:num>", list + 1);
    }
    Some(format!("{HEAD}<w:numbering xmlns:w=\"{W}\">{abstracts}{nums}</w:numbering>"))
}

/// What every section shares, and what each has of its own.
pub(crate) struct Sections<'a> {
    /// The hand-written page properties, over every section's.
    page: Vec<&'a Property>,
    /// The header and footer references, which the first section holds and the others follow.
    refs: String,
    /// Each section's first page differs ("different first page").
    title_page: bool,
    furniture: &'a Furniture,
}

/// Section `k`'s properties; `next_page` for a section that ends where the next page begins.
/// With no page given (`plain`), Word's own page. `own`: the page of a stretch set its own way,
/// which the hand-written page properties (the document's) don't go over.
fn sect_pr(setups: &Setups, own: Option<&Setup>, k: usize, next_page: bool, plain: bool, sections: &Sections) -> String {
    let setup = own.unwrap_or(&setups.main);
    let hand: &[&Property] = if own.is_some() { &[] } else { &sections.page };
    let refs = if k == 0 { sections.refs.as_str() } else { "" };
    let numbering = match sections.furniture.numbering.get(k) {
        Some((start, format)) if start.is_some() || *format != Format::Decimal => {
            let fmt = match format {
                Format::Decimal => "",
                Format::LowerRoman => " w:fmt=\"lowerRoman\"",
                Format::UpperRoman => " w:fmt=\"upperRoman\"",
            };
            let start = start.map(|n| format!(" w:start=\"{n}\"")).unwrap_or_default();
            format!("<w:pgNumType{fmt}{start}/>")
        }
        _ => String::new(),
    };
    let title = if sections.title_page { "<w:titlePg/>" } else { "" };
    let items = |list: Vec<(&str, String)>| translate::overlay(
        list.into_iter().filter(|(_, x)| !x.is_empty()).map(|(n, x)| (n.to_string(), x)).collect(), hand, translate::SECTION);
    if plain {
        return format!("<w:sectPr>{refs}{}</w:sectPr>", items(vec![("pgNumType", numbering), ("titlePg", title.to_string())]));
    }
    let m = &setup.margins;
    // Where the header and footer were set; else as Typst puts them by default (`header-ascent`
    // puts a header's baseline 30% of the top margin above the body).
    let header = tw(sections.furniture.header_from_top.unwrap_or(m.top * 0.4));
    let footer = tw(sections.furniture.footer_from_bottom.unwrap_or(m.bottom * 0.4));
    let kind = if next_page { "<w:type w:val=\"nextPage\"/>" } else { "" };
    let size = format!("<w:pgSz w:w=\"{}\" w:h=\"{}\"/>", tw(setup.width), tw(setup.height));
    let margins = format!(
        "<w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"{header}\" w:footer=\"{footer}\" w:gutter=\"0\"/>",
        tw(m.top), tw(m.right), tw(m.bottom), tw(m.left));
    let body = items(vec![("type", kind.to_string()), ("pgSz", size), ("pgMar", margins), ("pgNumType", numbering), ("titlePg", title.to_string())]);
    format!("<w:sectPr>{refs}{body}</w:sectPr>")
}

/// Whether paragraph `i` opens a page, and so starts a section (see `document_xml`).
fn opens_page(laid: &Laid, resolved: &[Props], i: usize) -> bool {
    let p = &laid.paras[i];
    i > 0 && !laid.plain && p.direct.page_break.or(resolved[p.style].page_break) == Some(true)
}

/// The page paragraph `i`'s text is set on when it's its own (see `Setups::own`).
fn own_page<'a>(laid: &Laid, setups: &'a Setups, i: usize) -> Option<&'a Setup> {
    if laid.plain {
        return None;
    }
    setups.own(laid.styles[laid.paras[i].style].key.scope.as_deref())
}

/// For each paragraph, whether it starts a section: a body paragraph that opens a page, or whose
/// page is set otherwise than the one before's.
fn section_breaks(laid: &Laid, setups: &Setups) -> Vec<bool> {
    let resolved: Vec<Props> = (0..laid.styles.len()).map(|s| laid.resolved(s)).collect();
    let mut breaks = vec![false; laid.paras.len()];
    let mut before: Option<usize> = None;
    for i in (0..laid.paras.len()).filter(|i| laid.paras[*i].note.is_none()) {
        if let Some(b) = before {
            breaks[i] = opens_page(laid, &resolved, i) || own_page(laid, setups, i) != own_page(laid, setups, b);
        }
        before = Some(i);
    }
    breaks
}

/// For each page of `pages`, whether a section starts on it (the first page always does).
pub(crate) fn section_starts(laid: &Laid, setups: &Setups, pages: usize) -> Vec<bool> {
    let mut starts = vec![false; pages];
    if let Some(first) = starts.first_mut() {
        *first = true;
    }
    for (i, starts_one) in section_breaks(laid, setups).into_iter().enumerate() {
        if starts_one && let Some(page) = starts.get_mut(laid.paras[i].page) {
            *page = true;
        }
    }
    starts
}

/// The footnotes the `.docx` has: those with text whose reference is in the body (in order).
fn footnotes(laid: &Laid) -> Vec<usize> {
    let referenced: std::collections::BTreeSet<usize> = laid.paras.iter().filter(|p| p.note.is_none())
        .flat_map(|p| p.runs.iter().filter_map(|r| r.note)).collect();
    let with_text: std::collections::BTreeSet<usize> = laid.paras.iter().filter_map(|p| p.note).collect();
    referenced.intersection(&with_text).copied().collect()
}

/// A footnote's ID in Word: -1 and 0 are its separators'.
fn footnote_id(n: usize) -> usize {
    n + 1
}

/// Word's number format for footnotes numbered as `shown` was (Typst's "1", "a", "i", "*").
fn footnote_format(shown: &str) -> &'static str {
    match shown.trim().chars().next() {
        Some(c) if c.is_ascii_digit() => "decimal",
        Some('*' | '†' | '‡' | '§') => "chicago",
        Some('i' | 'v' | 'x') => "lowerRoman",
        Some('I' | 'V' | 'X') => "upperRoman",
        Some(c) if c.is_lowercase() => "lowerLetter",
        Some(c) if c.is_uppercase() => "upperLetter",
        _ => "decimal",
    }
}

/// Paragraph `p`'s runs, with the comment markers that fall inside them. A footnote's number is
/// Word's: a reference to the footnote in the body (when the footnote is in the `.docx`, else its
/// number as shown), the footnote's own mark in its text.
fn para_content(laid: &Laid, p: usize, resolved: &[Props], marks: &BTreeMap<Place, String>, notes: &[usize]) -> String {
    let para = &laid.paras[p];
    let mut content = String::new();
    for (r, run) in para.runs.iter().enumerate() {
        let chars: Vec<char> = run.text.chars().collect();
        let mark = match run.note {
            Some(_) if para.note.is_some() => Some("<w:footnoteRef/>".to_string()),
            Some(n) if notes.contains(&n) => Some(format!("<w:footnoteReference w:id=\"{}\"/>", footnote_id(n))),
            _ => None,
        };
        if let Some(mark) = mark {
            for (_, xml) in marks.range((p, r, 0)..=(p, r, chars.len())) {
                content.push_str(xml);
            }
            let _ = write!(content, "<w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr>{mark}</w:r>");
            continue;
        }
        let props = rpr(&run_props(run, &resolved[para.style], laid.plain), None);
        // The run in pieces, with the markers that fall inside it between them.
        let mut from = 0;
        for (&(_, _, k), xml) in marks.range((p, r, 0)..=(p, r, chars.len())) {
            run_xml(&mut content, &chars[from..k].iter().collect::<String>(), &props);
            content.push_str(xml);
            from = k;
        }
        run_xml(&mut content, &chars[from..].iter().collect::<String>(), &props);
    }
    content
}

/// Paragraph `p`'s properties: its style (unless Normal) and what it has of its own.
fn para_props(laid: &Laid, p: usize) -> Vec<(String, String)> {
    let para = &laid.paras[p];
    let style = &laid.styles[para.style];
    let id = (style.kind != Kind::Normal).then_some(style.id.as_str());
    let mut items = ppr_items(&para.direct, id, None);
    if let Some(item) = &para.list {
        items.push(("numPr".into(), format!("<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr>", item.level, item.list + 1)));
    }
    items.sort_by_key(|(n, _)| translate::PARAGRAPH.iter().position(|o| o == n).unwrap_or(usize::MAX));
    items
}

/// `footnotes.xml`: Word's separators, then each footnote's paragraphs. None without footnotes.
fn footnotes_xml(laid: &Laid, marks: &BTreeMap<Place, String>, notes: &[usize]) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    let resolved: Vec<Props> = (0..laid.styles.len()).map(|s| laid.resolved(s)).collect();
    let separator = |kind: &str, id: i32| format!(
        "<w:footnote w:type=\"{kind}\" w:id=\"{id}\"><w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:{kind}/></w:r></w:p></w:footnote>");
    let mut x = format!("{HEAD}<w:footnotes xmlns:w=\"{W}\" xmlns:r=\"{R}\">{}{}", separator("separator", -1), separator("continuationSeparator", 0));
    for &n in notes {
        let _ = write!(x, "<w:footnote w:id=\"{}\">", footnote_id(n));
        for p in (0..laid.paras.len()).filter(|p| laid.paras[*p].note == Some(n)) {
            let props = wrap("pPr", para_props(laid, p).into_iter().map(|(_, x)| x).collect());
            let _ = write!(x, "<w:p>{props}{}</w:p>", para_content(laid, p, &resolved, marks, notes));
        }
        x.push_str("</w:footnote>");
    }
    x.push_str("</w:footnotes>");
    Some(x)
}

fn document_xml(laid: &Laid, setups: &Setups, marks: &BTreeMap<Place, String>, sections: &Sections, notes: &[usize]) -> String {
    let resolved: Vec<Props> = (0..laid.styles.len()).map(|s| laid.resolved(s)).collect();
    let breaks = section_breaks(laid, setups);
    // Each paragraph's properties and content, apart, so a section break can go in before it.
    let mut paras: Vec<(String, Option<String>, String)> = Vec::new();
    let mut section = 0;
    let body: Vec<usize> = (0..laid.paras.len()).filter(|p| laid.paras[*p].note.is_none()).collect();
    // The page the section being written is set on, when it's a stretch's own.
    let mut own = body.first().and_then(|p| own_page(laid, setups, *p));
    for p in body {
        let content = para_content(laid, p, &resolved, marks, notes);
        // Word 2013 and later drop the space before a paragraph at the top of a page after a
        // page break, but not at the start of a section: so a paragraph that opens a page
        // (its style still says so, for paragraphs added in Word) begins a section, and keeps
        // its distance from the top of the page.
        if breaks[p] && let Some(before) = paras.last_mut() {
            before.1 = Some(sect_pr(setups, own, section, true, false, sections));
            section += 1;
            own = own_page(laid, setups, p);
        }
        paras.push((wrap("pPr", para_props(laid, p).into_iter().map(|(_, x)| x).collect()), None, content));
    }
    let mut body = String::new();
    for (props, section, content) in &paras {
        let mut props = props.clone();
        if let Some(section) = section {
            props = match props.strip_suffix("</w:pPr>") {
                Some(open) => format!("{open}{section}</w:pPr>"),
                None => format!("<w:pPr>{section}</w:pPr>"),
            };
        }
        let _ = write!(body, "<w:p>{props}{content}</w:p>");
    }
    if paras.is_empty() {
        let start = marks.get(&(0, 0, 0)).cloned().unwrap_or_default();
        let _ = write!(body, "<w:p>{start}</w:p>");
    }
    format!("{HEAD}<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:body>{body}{}</w:body></w:document>", sect_pr(setups, own, section, false, laid.plain, sections))
}

/// The most used look among a region's pieces.
fn region_look(sets: &[&Lines]) -> Option<Look> {
    let mut count: Vec<(Look, usize)> = Vec::new();
    for lines in sets {
        for (piece, look) in lines.iter().flatten().flat_map(|part| &part.pieces) {
            let n = match piece {
                Piece::Text(t) => t.chars().count(),
                Piece::Field { shown, .. } => shown.chars().count(),
            };
            match count.iter_mut().find(|(l, _)| l == look) {
                Some((_, c)) => *c += n,
                None => count.push((look.clone(), n)),
            }
        }
    }
    count.into_iter().rev().max_by_key(|(_, n)| *n).map(|(l, _)| l)
}

fn look_props(look: &Look) -> Props {
    Props { family: Some(look.family.clone()), size: Some(look.size), italic: Some(look.italic), bold: Some(look.bold), fill: Some(look.fill.clone()), small_caps: Some(look.small_caps), ..Props::default() }
}

/// Word's "header" and "footer" styles: the region's most used look, single-spaced, with a
/// centre and a right tab stop across the body's width, as Word's own have.
fn furniture_styles(furniture: &Furniture, setup: &Setup, plain: bool, hands: &[(String, Vec<Property>)]) -> String {
    let width = if plain { 9360 } else { tw(setup.width - setup.margins.left - setup.margins.right) };
    let mut x = String::new();
    for (id, name, set) in [("Header", "header", &furniture.header), ("Footer", "footer", &furniture.footer)] {
        let sets: Vec<&Lines> = [Some(&set.default), set.first.as_ref(), set.even.as_ref()].into_iter().flatten().collect();
        let look = if plain { None } else { region_look(&sets) };
        let paragraph = vec![
            ("tabs".to_string(), format!("<w:tabs><w:tab w:val=\"center\" w:pos=\"{}\"/><w:tab w:val=\"right\" w:pos=\"{width}\"/></w:tabs>", width / 2)),
            ("spacing".to_string(), "<w:spacing w:before=\"0\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/>".to_string()),
            ("ind".to_string(), "<w:ind w:left=\"0\" w:right=\"0\" w:firstLine=\"0\"/>".to_string()),
            ("jc".to_string(), "<w:jc w:val=\"left\"/>".to_string()),
        ];
        let run = look.map(|l| rpr_items(&look_props(&l), None)).unwrap_or_default();
        let _ = write!(x, "<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/><w:basedOn w:val=\"Normal\"/>{}{}</w:style>",
            wrap("pPr", translate::overlay(paragraph, &hand(hands, |t| t.eq_ignore_ascii_case(name), TKind::Paragraph), translate::PARAGRAPH)),
            wrap("rPr", translate::overlay(run, &hand(hands, |t| t.eq_ignore_ascii_case(name), TKind::Run), translate::RUN)));
    }
    x
}

/// A header or footer part: each line a paragraph in the region's style; text on several sides
/// of a line is set apart by the style's tab stops.
fn furniture_xml(root: &str, style: &str, lines: &Lines, style_look: Option<&Look>) -> String {
    let mut body = String::new();
    let base = style_look.map(look_props).unwrap_or_default();
    for line in lines {
        let jc = match line.as_slice() {
            [only] => match only.side {
                Side::Left => "",
                Side::Centre => "<w:jc w:val=\"center\"/>",
                Side::Right => "<w:jc w:val=\"right\"/>",
            },
            _ => "",
        };
        let _ = write!(body, "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/>{jc}</w:pPr>");
        let mut at = Side::Left;
        for part in line {
            if line.len() > 1 {
                let tabs = match (at, part.side) {
                    (Side::Left, Side::Centre) | (Side::Centre, Side::Right) => 1,
                    (Side::Left, Side::Right) => 2,
                    _ => 0,
                };
                for _ in 0..tabs {
                    body.push_str("<w:r><w:tab/></w:r>");
                }
                at = part.side;
            }
            for (piece, look) in &part.pieces {
                let own = look_props(look);
                let direct = Props {
                    family: (own.family != base.family).then(|| look.family.clone()),
                    size: (own.size != base.size).then_some(look.size),
                    italic: (own.italic != base.italic.or(Some(false))).then_some(look.italic),
                    bold: (own.bold != base.bold.or(Some(false))).then_some(look.bold),
                    fill: (own.fill != base.fill).then(|| look.fill.clone()),
                    small_caps: (own.small_caps != base.small_caps.or(Some(false))).then_some(look.small_caps),
                    ..Props::default()
                };
                let props = rpr(&direct, None);
                match piece {
                    Piece::Text(t) => run_xml(&mut body, t, &props),
                    Piece::Field { instr, shown } => {
                        let _ = write!(body, "<w:fldSimple w:instr=\" {} \">", esc(instr));
                        run_xml(&mut body, shown, &props);
                        body.push_str("</w:fldSimple>");
                    }
                }
            }
        }
        body.push_str("</w:p>");
    }
    if lines.is_empty() {
        let _ = write!(body, "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr></w:p>");
    }
    format!("{HEAD}<w:{root} xmlns:w=\"{W}\" xmlns:r=\"{R}\">{body}</w:{root}>")
}

fn styles_xml(laid: &Laid, setup: &Setup, furniture: &Furniture, hands: &[(String, Vec<Property>)], footnotes: bool) -> String {
    let normal = laid.resolved(0);
    let defaults = Props { family: normal.family.clone(), size: normal.size, ..Props::default() };
    let mut x = format!(
        "{HEAD}<w:styles xmlns:w=\"{W}\"><w:docDefaults><w:rPrDefault>{}</w:rPrDefault>\
         <w:pPrDefault><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>",
        rpr(&defaults, Some(&setup.language())));
    for (i, s) in laid.styles.iter().enumerate() {
        let own = laid.own(i);
        let resolved = laid.resolved(i);
        // What a style based on one with hand-written properties would otherwise take from
        // them: it states its own (measured) values for those.
        let mut forced: Vec<String> = Vec::new();
        let mut base = s.based_on;
        while let Some(b) = base {
            forced.extend(hands.iter().filter(|(t, _)| laid.answers_to(b, t)).flat_map(|(_, ps)| ps.iter().map(|p| p.name.clone())));
            base = laid.styles[b].based_on;
        }
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
        // Word inherits an element's attributes one by one, so a forced element is stated whole.
        let with_forced = |mine: Vec<(String, String)>, all: Vec<(String, String)>| {
            let mut items = mine;
            for (name, xml) in all {
                if forced.contains(&name) {
                    match items.iter_mut().find(|(n, _)| *n == name) {
                        Some(item) => item.1 = xml,
                        None => items.push((name, xml)),
                    }
                }
            }
            items
        };
        let mut paragraph = with_forced(ppr_items(&own, None, outline), ppr_items(&resolved, None, outline));
        let mut run = with_forced(rpr_items(&own, None), rpr_items(&resolved, None));
        if s.kind == Kind::Normal {
            // What Typst does to every paragraph, which Word does only when told: avoiding
            // widows and orphans, and kerning (from 1 pt up). A book draft keeps Word's
            // widow control.
            if setup.widows || laid.plain {
                paragraph.push(("widowControl".into(), "<w:widowControl/>".into()));
            }
            if setup.kerning && !laid.plain {
                run.push(("kern".into(), "<w:kern w:val=\"2\"/>".into()));
            }
        }
        x.push_str(&wrap("pPr", translate::overlay(paragraph, &hand(hands, |t| laid.answers_to(i, t), TKind::Paragraph), translate::PARAGRAPH)));
        x.push_str(&wrap("rPr", translate::overlay(run, &hand(hands, |t| laid.answers_to(i, t), TKind::Run), translate::RUN)));
        x.push_str("</w:style>");
    }
    x.push_str(&furniture_styles(furniture, setup, laid.plain, hands));
    if footnotes {
        // The footnote's number, in the text and in the note, as Word's own is.
        let run = vec![("vertAlign".to_string(), "<w:vertAlign w:val=\"superscript\"/>".to_string())];
        let _ = write!(x, "<w:style w:type=\"character\" w:styleId=\"FootnoteReference\"><w:name w:val=\"footnote reference\"/><w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/>{}</w:style>",
            wrap("rPr", translate::overlay(run, &hand(hands, |t| t.eq_ignore_ascii_case("footnote reference"), TKind::Run), translate::RUN)));
    }
    x.push_str("</w:styles>");
    x
}

fn settings_xml(setup: &Setup, plain: bool, odd_and_even: bool, embedded: bool, footnotes: Option<&str>) -> String {
    // Embedded whole, and kept so when Word saves (no `saveSubsetFonts`), so text typed in
    // Word can use them too.
    let embed = if embedded { "<w:embedTrueTypeFonts/>" } else { "" };
    let mirror = if setup.margins.mirrored && !plain { "<w:mirrorMargins/>" } else { "" };
    let hyphenate = if setup.hyphenate && !plain { "<w:autoHyphenation/>" } else { "" };
    let sides = if odd_and_even { "<w:evenAndOddHeaders/>" } else { "" };
    // How footnotes are numbered, and the separators every footnote section has.
    let notes = footnotes.map(|format| format!(
        "<w:footnotePr><w:numFmt w:val=\"{format}\"/><w:footnote w:id=\"-1\"/><w:footnote w:id=\"0\"/></w:footnotePr>")).unwrap_or_default();
    // doNotExpandShiftReturn: a forced line break in a justified paragraph isn't stretched,
    // as in Typst.
    format!(
        "{HEAD}<w:settings xmlns:w=\"{W}\">{embed}{mirror}<w:defaultTabStop w:val=\"720\"/>{hyphenate}{sides}\
         <w:characterSpacingControl w:val=\"doNotCompress\"/>{notes}\
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

fn content_types(comments: bool, furniture: &[(String, String, &str)], fonts: bool, numbering: bool, footnotes: bool) -> String {
    let footnotes = if footnotes {
        "<Override PartName=\"/word/footnotes.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml\"/>"
    } else {
        ""
    };
    let numbering = if numbering {
        "<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>"
    } else {
        ""
    };
    let fonts = if fonts {
        "<Default Extension=\"odttf\" ContentType=\"application/vnd.openxmlformats-officedocument.obfuscatedFont\"/>\
         <Override PartName=\"/word/fontTable.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml\"/>"
    } else {
        ""
    };
    let furniture: String = furniture.iter().map(|(part, _, kind)| format!(
        "<Override PartName=\"/word/{part}\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml\"/>")).collect();
    let comments = if comments {
        "<Override PartName=\"/word/comments.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml\"/>\
         <Override PartName=\"/word/commentsExtended.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml\"/>"
    } else {
        ""
    };
    format!(
        "{HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>{fonts}\
         <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
         <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
         <Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>\
         {comments}{furniture}{numbering}{footnotes}\
         <Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
         </Types>")
}

const PACKAGE_RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
    <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
    <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
    </Relationships>";

fn document_rels(comments: bool, furniture: &[(String, String, &str)], fonts: bool, numbering: bool, footnotes: bool) -> String {
    let footnotes = if footnotes {
        "<Relationship Id=\"rIdFootnotes\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes\" Target=\"footnotes.xml\"/>"
    } else {
        ""
    };
    let numbering = if numbering {
        "<Relationship Id=\"rIdNumbering\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/>"
    } else {
        ""
    };
    let fonts = if fonts {
        "<Relationship Id=\"rIdFontTable\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable\" Target=\"fontTable.xml\"/>"
    } else {
        ""
    };
    let furniture: String = furniture.iter().map(|(part, id, kind)| format!(
        "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{part}\"/>")).collect();
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
         {comments}{furniture}{fonts}{numbering}{footnotes}</Relationships>")
}

/// The parts of the package, by name, and a note for each comment that couldn't be placed.
/// `fontTable.xml`, naming each family and its embedded faces, and its relationships to them.
fn font_table(families: &[Family]) -> (String, String, Vec<(String, Vec<u8>)>) {
    let mut table = format!("{HEAD}<w:fonts xmlns:w=\"{W}\" xmlns:r=\"{R}\">");
    let mut rels = format!("{HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    let mut files = Vec::new();
    for family in families {
        let _ = write!(table, "<w:font w:name=\"{}\">", esc(&family.name));
        for (slot, bytes, guid) in &family.faces {
            let n = files.len() + 1;
            let _ = write!(table, "<w:{} r:id=\"rIdFont{n}\" w:fontKey=\"{guid}\"/>", slot.element());
            let _ = write!(rels, "<Relationship Id=\"rIdFont{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/font\" Target=\"fonts/font{n}.odttf\"/>");
            files.push((format!("word/fonts/font{n}.odttf"), bytes.clone()));
        }
        table.push_str("</w:font>");
    }
    table.push_str("</w:fonts>");
    rels.push_str("</Relationships>");
    (table, rels, files)
}

pub(crate) fn parts(laid: &Laid, setups: &Setups, meta: &Metadata, comments: &[Comment], furniture: &Furniture, families: &[Family], hands: &[(String, Vec<Property>)]) -> (Vec<(String, Vec<u8>)>, Vec<Note>) {
    let setup = &setups.main;
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

    // Headers and footers: a part for each kind of page that differs. "Different first page"
    // and "different odd and even" apply to both, so a region without its own takes its
    // default there.
    let (header, footer) = (&furniture.header, &furniture.footer);
    let title_page = header.first.is_some() || footer.first.is_some();
    let odd_and_even = header.even.is_some() || footer.even.is_some();
    let mut furniture_parts: Vec<(String, String, &str)> = Vec::new(); // (part, rId, kind)
    let mut furniture_xmls: Vec<String> = Vec::new();
    let mut refs = String::new();
    for (kind, root, style, set) in [("header", "hdr", "Header", header), ("footer", "ftr", "Footer", footer)] {
        if set.is_empty() {
            continue;
        }
        let sets: Vec<&Lines> = [Some(&set.default), set.first.as_ref(), set.even.as_ref()].into_iter().flatten().collect();
        let look = if laid.plain { None } else { region_look(&sets) };
        let pages = [
            ("default", Some(&set.default)),
            ("first", title_page.then(|| set.first.as_ref().unwrap_or(&set.default))),
            ("even", odd_and_even.then(|| set.even.as_ref().unwrap_or(&set.default))),
        ];
        for (which, lines) in pages {
            let Some(lines) = lines else { continue };
            let n = furniture_parts.len() + 1;
            let (part, id) = (format!("{kind}{n}.xml"), format!("rId{kind}{n}"));
            let _ = write!(refs, "<w:{kind}Reference w:type=\"{which}\" r:id=\"{id}\"/>");
            furniture_xmls.push(furniture_xml(root, style, lines, look.as_ref()));
            furniture_parts.push((part, id, kind));
        }
    }
    let page = hand(hands, |t| t.eq_ignore_ascii_case("Page"), TKind::Section);
    let sections = Sections { page, refs, title_page, furniture };

    let embedded = families.iter().any(|f| !f.faces.is_empty());
    let numbering = numbering_xml(laid);
    let footnote_ids = footnotes(laid);
    let footnotes = footnotes_xml(laid, &marks, &footnote_ids);
    // Numbered as the first footnote's reference showed it.
    let format = footnote_ids.first().and_then(|n| laid.paras.iter().filter(|p| p.note.is_none()).flat_map(|p| &p.runs).find(|r| r.note == Some(*n)))
        .map(|r| footnote_format(&r.text));
    let mut parts: Vec<(String, String)> = vec![
        ("[Content_Types].xml".to_string(), content_types(any, &furniture_parts, embedded, numbering.is_some(), footnotes.is_some())),
        ("_rels/.rels".to_string(), format!("{HEAD}{PACKAGE_RELS}")),
        ("word/document.xml".to_string(), document_xml(laid, setups, &marks, &sections, &footnote_ids)),
        ("word/styles.xml".to_string(), styles_xml(laid, setup, furniture, hands, footnotes.is_some())),
        ("word/settings.xml".to_string(), settings_xml(setup, laid.plain, odd_and_even, embedded, format)),
        ("word/_rels/document.xml.rels".to_string(), document_rels(any, &furniture_parts, embedded, numbering.is_some(), footnotes.is_some())),
        ("docProps/core.xml".to_string(), core_xml(meta)),
    ];
    if let Some(footnotes) = footnotes {
        parts.push(("word/footnotes.xml".to_string(), footnotes));
    }
    for ((part, _, _), xml) in furniture_parts.iter().zip(furniture_xmls) {
        parts.push((format!("word/{part}"), xml));
    }
    if any {
        let (comments, extended) = comments_xml(&threads, meta.seed);
        parts.push(("word/comments.xml".to_string(), comments));
        parts.push(("word/commentsExtended.xml".to_string(), extended));
    }
    if let Some(numbering) = numbering {
        parts.push(("word/numbering.xml".to_string(), numbering));
    }
    let mut files = Vec::new();
    if embedded {
        let (table, rels, fonts) = font_table(families);
        parts.push(("word/fontTable.xml".to_string(), table));
        parts.push(("word/_rels/fontTable.xml.rels".to_string(), rels));
        files = fonts;
    }
    let mut parts: Vec<(String, Vec<u8>)> = parts.into_iter().map(|(name, xml)| (name, xml.into_bytes())).collect();
    parts.extend(files);
    if furniture.simplified {
        notes.push(Note { style: None, origin: None, what: What::FurnitureSimplified });
    }
    (parts, notes)
}

/// The zipped package. The same input gives the same bytes: no times or permissions vary.
pub(crate) fn zip(parts: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for (name, content) in parts {
        zip.start_file(name.as_str(), options).map_err(|e| e.to_string())?;
        zip.write_all(content).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_levels_as_word_numbers_them() {
        assert_eq!(level_format("•", 0, false), ("bullet", "•".to_string(), 1));
        assert_eq!(level_format("1.", 0, true), ("decimal", "%1.".to_string(), 1));
        assert_eq!(level_format("3)", 0, true), ("decimal", "%1)".to_string(), 3));
        assert_eq!(level_format("(a)", 1, true), ("lowerLetter", "(%2)".to_string(), 1));
        assert_eq!(level_format("1.a.", 1, true), ("lowerLetter", "%1.%2.".to_string(), 1));
        assert_eq!(level_format("i.", 2, true), ("lowerRoman", "%3.".to_string(), 1));
        assert_eq!(level_format("IV.", 0, true), ("upperRoman", "%1.".to_string(), 4));
        assert_eq!(level_format("C.", 0, true), ("upperLetter", "%1.".to_string(), 3), "one letter that isn't i is a letter");
    }
}
