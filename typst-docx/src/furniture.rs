//! Headers and footers, read from what each page shows.
//!
//! In Typst nearly every useful header needs `context`, so it can't be translated from the
//! source; instead the text each page shows outside its body (inside `artifact` tags) is read,
//! the page's number is turned into a `PAGE` field and the current heading's title into a
//! `STYLEREF` field, and one pattern is looked for that Word can repeat: the same on every page;
//! or different on the first page of each section (a Chapter's opening page, as each starts a
//! section); and/or different on even pages. When none fits, only the page number is kept.

use std::fmt::Write as _;

use typst::introspection::{Location, Tag};
use typst::layout::{Frame, FrameItem, Point};
use typst::text::{FontStyle, TextItem};
use typst::visualize::Paint;
use typst_layout::PagedDocument;

use crate::measure::Area;

/// How a stretch of text is set.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Look {
    pub family: String,
    pub size: f64,
    pub italic: bool,
    pub bold: bool,
    pub fill: String,
    pub small_caps: bool,
}

/// A piece of a header line: text, or a field showing what it showed on the page it was read from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Piece {
    Text(String),
    /// `PAGE`, or `STYLEREF "<style>"` (with `\* Upper` when it was in capitals).
    Field { instr: String, shown: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Left,
    Centre,
    Right,
}

/// What a line shows at one place across it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Part {
    pub side: Side,
    pub pieces: Vec<(Piece, Look)>,
}

pub(crate) type Lines = Vec<Vec<Part>>;

/// What a header or footer shows on which pages.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Set {
    pub default: Lines,
    /// On the first page of each section, if that differs.
    pub first: Option<Lines>,
    /// On even pages, if they differ.
    pub even: Option<Lines>,
}

impl Set {
    pub fn is_empty(&self) -> bool {
        self.default.is_empty() && self.first.as_ref().is_none_or(Vec::is_empty) && self.even.as_ref().is_none_or(Vec::is_empty)
    }
}

/// Page numbers as Word should count them in a section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Decimal,
    LowerRoman,
    UpperRoman,
}

pub(crate) struct Furniture {
    pub header: Set,
    pub footer: Set,
    /// From the page top to the header's first line, and from the footer's last line to the
    /// page bottom, in points (as Word measures them), if there's one.
    pub header_from_top: Option<f64>,
    pub footer_from_bottom: Option<f64>,
    /// Each section's first page number, if it doesn't carry on from the page before, and how
    /// its numbers are written; by section.
    pub numbering: Vec<(Option<u64>, Format)>,
    /// No single pattern fitted, so only the page number was kept.
    pub simplified: bool,
}

/// A run of text outside the body, where it sits.
struct Item {
    page: usize,
    baseline: f64,
    left: f64,
    right: f64,
    ascent: f64,
    descent: f64,
    text: String,
    look: Look,
}

fn items(document: &PagedDocument) -> Vec<Item> {
    fn walk(frame: &Frame, at: Point, page: usize, open: &mut Vec<Location>, out: &mut Vec<Item>, caps: &mut crate::fonts::SmallCaps) {
        for (pos, item) in frame.items() {
            let p = at + *pos;
            match item {
                FrameItem::Group(g) => walk(&g.frame, p + Point::new(g.transform.tx, g.transform.ty), page, open, out, caps),
                FrameItem::Tag(Tag::Start(c, _)) if c.elem().name() == "artifact" => open.extend(c.location()),
                FrameItem::Tag(Tag::End(loc, ..)) => open.retain(|l| l != loc),
                FrameItem::Text(t) if !open.is_empty() && !t.text.trim().is_empty() => out.push(item_of(t, p, page, caps.of(t))),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    let mut caps = crate::fonts::SmallCaps::default();
    for (n, page) in document.pages().iter().enumerate() {
        walk(&page.frame, Point::zero(), n, &mut Vec::new(), &mut out, &mut caps);
    }
    out
}

fn item_of(t: &TextItem, at: Point, page: usize, small_caps: bool) -> Item {
    let size = t.size.to_pt();
    let width: f64 = t.glyphs.iter().map(|g| g.x_advance.at(t.size).to_pt()).sum();
    let cap = t.font.metrics().cap_height.at(t.size).to_pt();
    let m = crate::measure::word_metrics(&t.font, size, cap);
    let info = t.font.font().info();
    let fill = match &t.fill {
        Paint::Solid(c) => c.to_hex().trim_start_matches('#').chars().take(6).collect::<String>().to_uppercase(),
        _ => "000000".into(),
    };
    Item {
        page, baseline: at.y.to_pt(), left: at.x.to_pt(), right: at.x.to_pt() + width, ascent: m.ascent, descent: m.descent,
        text: t.text.to_string(),
        look: Look { family: info.family.clone(), size, italic: info.variant.style != FontStyle::Normal, bold: info.variant.weight.to_number() >= 600, fill, small_caps },
    }
}

/// Roman numerals, lower case.
fn roman(mut n: u64) -> String {
    let mut out = String::new();
    for (value, s) in [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")] {
        while n >= value {
            out.push_str(s);
            n -= value;
        }
    }
    out
}

/// Where `needle` is in `hay` as a word of its own (not inside a longer word or number), the
/// last such place.
fn last_word(hay: &str, needle: &str) -> Option<usize> {
    hay.rmatch_indices(needle).map(|(i, _)| i).find(|&i| {
        let before = hay[..i].chars().last();
        let after = hay[i + needle.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphanumeric())
    })
}

/// A part's text, char by char with its look, turned into pieces: the latest heading titles
/// (`titles`: Word style name and its text) become STYLEREF fields, then the page number a
/// PAGE field (its last appearance as a word of its own, so a number in a title isn't taken).
fn pieces(chars: &[(char, usize)], looks: &[Look], titles: &[(String, String)], number: u64) -> (Vec<(Piece, Look)>, Option<Format>) {
    let text: String = chars.iter().map(|(c, _)| c).collect();
    // Byte ranges in `text` that become fields, with their instructions.
    let mut fields: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for (style, title) in titles {
        let title = title.trim();
        if title.is_empty() {
            continue;
        }
        let upper = title.to_uppercase();
        let found = text.find(title).map(|i| (i, title.len(), ""))
            .or_else(|| (upper != title).then(|| text.find(&upper).map(|i| (i, upper.len(), " \\* Upper"))).flatten());
        if let Some((at, len, switch)) = found {
            if !fields.iter().any(|(r, _)| r.start < at + len && at < r.end) {
                fields.push((at..at + len, format!("STYLEREF \"{style}\"{switch}")));
            }
        }
    }
    let mut format = None;
    let candidates = [(number.to_string(), Format::Decimal), (roman(number), Format::LowerRoman), (roman(number).to_uppercase(), Format::UpperRoman)];
    let free = |at: usize, len: usize, fields: &[(std::ops::Range<usize>, String)]| !fields.iter().any(|(r, _)| r.start < at + len && at < r.end);
    let mut best: Option<(usize, usize, Format)> = None;
    for (n, f) in &candidates {
        let mut hay = text.as_str();
        // The last place it's a word of its own and not inside a title.
        while let Some(at) = last_word(hay, n) {
            if free(at, n.len(), &fields) {
                if best.is_none_or(|(b, _, _)| at > b) {
                    best = Some((at, n.len(), *f));
                }
                break;
            }
            hay = &hay[..at];
        }
    }
    if let Some((at, len, f)) = best {
        fields.push((at..at + len, "PAGE".into()));
        format = Some(f);
    }
    fields.sort_by_key(|(r, _)| r.start);
    // Back into pieces, each with the look of its first character.
    let look_at = |byte: usize| {
        let k = text[..byte].chars().count();
        looks[chars.get(k).map_or(0, |c| c.1)].clone()
    };
    let mut out: Vec<(Piece, Look)> = Vec::new();
    let mut at = 0;
    let push_text = |out: &mut Vec<(Piece, Look)>, from: usize, to: usize| {
        // Split where the look changes.
        let mut start = from;
        let mut i = from;
        for c in text[from..to].chars() {
            let here = look_at(i);
            if i > start && here != look_at(start) {
                out.push((Piece::Text(text[start..i].to_string()), look_at(start)));
                start = i;
            }
            i += c.len_utf8();
        }
        if start < to {
            out.push((Piece::Text(text[start..to].to_string()), look_at(start)));
        }
    };
    for (range, instr) in &fields {
        push_text(&mut out, at, range.start);
        out.push((Piece::Field { instr: instr.clone(), shown: text[range.clone()].to_string() }, look_at(range.start)));
        at = range.end;
    }
    push_text(&mut out, at, text.len());
    (out, format)
}

/// What a page shows in one region, as lines of parts, and how its page number is written.
fn lines_of(items: &[&Item], area: Area, titles: &[(String, String)], number: u64) -> (Lines, Option<Format>) {
    let mut sorted: Vec<&&Item> = items.iter().collect();
    sorted.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.left.total_cmp(&b.left)));
    let mut rows: Vec<Vec<&Item>> = Vec::new();
    for item in sorted {
        match rows.last_mut() {
            Some(row) if (row[0].baseline - item.baseline).abs() < 0.5 => row.push(item),
            _ => rows.push(vec![item]),
        }
    }
    let mut format = None;
    let mut lines = Vec::new();
    for row in rows {
        // Clusters: text set apart by a wide gap is in another place across the line.
        let mut clusters: Vec<Vec<&Item>> = Vec::new();
        for item in row {
            match clusters.last_mut() {
                Some(c) if item.left - c.last().unwrap().right < 2.0 * item.look.size => c.push(item),
                _ => clusters.push(vec![item]),
            }
        }
        let mut parts = Vec::new();
        for c in clusters {
            let (left, right) = (c[0].left, c.last().unwrap().right);
            let side = if (left - area.left).abs() < 2.0 {
                Side::Left
            } else if (right - area.right).abs() < 0.5 * c[0].look.size + 1.0 {
                Side::Right
            } else if (((left + right) / 2.0) - (area.left + area.right) / 2.0).abs() < 3.0 {
                Side::Centre
            } else {
                Side::Left
            };
            let looks: Vec<Look> = c.iter().map(|i| i.look.clone()).collect();
            let chars: Vec<(char, usize)> = c.iter().enumerate().flat_map(|(k, i)| i.text.chars().map(move |ch| (ch, k))).collect();
            let (pieces, f) = pieces(&chars, &looks, titles, number);
            format = format.or(f);
            parts.push(Part { side, pieces });
        }
        lines.push(parts);
    }
    (lines, format)
}

/// Lines with the fields' shown text left out, to compare pages by.
fn shape(lines: &Lines) -> String {
    let mut out = String::new();
    for line in lines {
        for part in line {
            let _ = write!(out, "{:?}:", part.side);
            for (piece, _) in &part.pieces {
                match piece {
                    Piece::Text(t) => out.push_str(t),
                    Piece::Field { instr, .. } => {
                        let _ = write!(out, "{{{instr}}}");
                    }
                }
            }
            out.push('|');
        }
        out.push('\n');
    }
    out
}

/// One pattern for a region, given what each page shows, which pages begin a section, and the
/// page numbers (for odd and even).
fn pattern(shown: &[Lines], section_first: &[bool], numbers: &[u64]) -> Option<Set> {
    let pages: Vec<usize> = (0..shown.len()).collect();
    let same = |pages: &[usize]| -> Option<Option<Lines>> {
        let first = pages.first()?;
        pages.iter().all(|p| shape(&shown[*p]) == shape(&shown[*first])).then(|| Some(shown[*first].clone()))
    };
    let all = |ps: Vec<usize>| -> Option<Lines> { match same(&ps) { Some(l) => l, None => None }.or(if ps.is_empty() { Some(Vec::new()) } else { None }) };
    if let Some(lines) = all(pages.clone()) {
        return Some(Set { default: lines, first: None, even: None });
    }
    let (firsts, rest): (Vec<usize>, Vec<usize>) = pages.iter().partition(|p| section_first[**p]);
    let (odd, even): (Vec<usize>, Vec<usize>) = rest.iter().partition(|p| numbers[**p] % 2 == 1);
    // The rest alike, or alike by odd and even.
    let (default, even_lines) = match all(rest.clone()) {
        Some(lines) => (lines, None),
        None => (all(odd.clone())?, Some(all(even.clone())?)),
    };
    // Each section's first page like the rest (on its side), or all alike among themselves.
    let like_rest = firsts.iter().all(|p| {
        let expected = if numbers[*p] % 2 == 0 { even_lines.as_ref().unwrap_or(&default) } else { &default };
        shape(&shown[*p]) == shape(expected)
    });
    let first = if like_rest { None } else { Some(all(firsts)?) };
    Some(Set { default, first, even: even_lines })
}

/// The headers and footers of the document. `areas`: each page's body area; `section_first`:
/// whether each page begins a section; `titles`: by page, the latest heading of each heading
/// style on or before it (Word style name, text).
pub(crate) fn read(document: &PagedDocument, areas: &[Area], section_first: &[bool], titles: &[Vec<(String, String)>]) -> Furniture {
    let items = items(document);
    let numbers: Vec<u64> = document.pages().iter().map(|p| p.number).collect();
    let mut formats: Vec<Option<Format>> = vec![None; numbers.len()];
    let mut regions: [Vec<Lines>; 2] = [Vec::new(), Vec::new()];
    for (p, area) in areas.iter().enumerate() {
        for (r, region) in regions.iter_mut().enumerate() {
            let here: Vec<&Item> = items.iter()
                .filter(|i| i.page == p && if r == 0 { i.baseline < area.top } else { i.baseline > area.bottom })
                .collect();
            let (lines, f) = lines_of(&here, *area, titles.get(p).map_or(&[][..], Vec::as_slice), numbers[p]);
            formats[p] = formats[p].or(f);
            region.push(lines);
        }
    }
    let [headers, footers] = regions;
    let header = pattern(&headers, section_first, &numbers);
    let footer = pattern(&footers, section_first, &numbers);
    let simplified = header.is_none() || footer.is_none();
    let (header, footer) = match (header, footer) {
        (Some(h), Some(f)) => (h, f),
        // No pattern fits: just the page number, where it's most often shown.
        _ => only_numbers(&headers, &footers),
    };
    let distance = |r: usize| -> Option<f64> {
        let mut values: Vec<f64> = items.iter().filter(|i| i.page < areas.len()).filter_map(|i| {
            let a = areas[i.page];
            let page_height = document.pages()[i.page].frame.height().to_pt();
            match r {
                0 if i.baseline < a.top => Some(i.baseline - i.ascent),
                1 if i.baseline > a.bottom => Some(page_height - (i.baseline + i.descent)),
                _ => None,
            }
        }).collect();
        values.sort_by(f64::total_cmp);
        values.first().copied().map(|v| v.max(0.0))
    };
    // Each section's numbering: restarted where a section's first number doesn't follow on.
    let mut numbering = Vec::new();
    for p in 0..numbers.len() {
        if section_first[p] || p == 0 {
            let follows = p > 0 && numbers[p] == numbers[p - 1] + 1;
            let start = (!follows && !(p == 0 && numbers[p] == 1)).then_some(numbers[p]);
            let end = (p + 1..numbers.len()).find(|q| section_first[*q]).unwrap_or(numbers.len());
            let format = formats[p..end].iter().flatten().next().copied().unwrap_or(Format::Decimal);
            numbering.push((start, format));
        }
    }
    Furniture { header, footer, header_from_top: distance(0), footer_from_bottom: distance(1), numbering, simplified }
}

/// A header and footer holding only the page number, on the side and in the region where it's
/// shown on most pages.
fn only_numbers(headers: &[Lines], footers: &[Lines]) -> (Set, Set) {
    let mut count: Vec<((usize, Side), usize, Look)> = Vec::new();
    for (r, region) in [headers, footers].iter().enumerate() {
        for lines in region.iter() {
            for part in lines.iter().flatten() {
                if let Some((_, look)) = part.pieces.iter().find(|(p, _)| matches!(p, Piece::Field { instr, .. } if instr == "PAGE")) {
                    match count.iter_mut().find(|(k, _, _)| *k == (r, part.side)) {
                        Some((_, n, _)) => *n += 1,
                        None => count.push(((r, part.side), 1, look.clone())),
                    }
                }
            }
        }
    }
    let Some(((r, side), _, look)) = count.into_iter().max_by_key(|(_, n, _)| *n) else {
        return (Set::default(), Set::default());
    };
    let set = Set { default: vec![vec![Part { side, pieces: vec![(Piece::Field { instr: "PAGE".into(), shown: "1".into() }, look)] }]], first: None, even: None };
    if r == 0 { (set, Set::default()) } else { (Set::default(), set) }
}

/// A footer with the page number centred, for a document with no Look.
pub(crate) fn plain(look: Look) -> Furniture {
    let set = Set { default: vec![vec![Part { side: Side::Centre, pieces: vec![(Piece::Field { instr: "PAGE".into(), shown: "1".into() }, look)] }]], first: None, even: None };
    Furniture { header: Set::default(), footer: set, header_from_top: None, footer_from_bottom: None, numbering: Vec::new(), simplified: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look { family: "F".into(), size: 10.0, italic: false, bold: false, fill: "000000".into(), small_caps: false }
    }

    fn texts(pieces: &[(Piece, Look)]) -> Vec<String> {
        pieces.iter().map(|(p, _)| match p {
            Piece::Text(t) => t.clone(),
            Piece::Field { instr, shown } => format!("{{{instr}={shown}}}"),
        }).collect()
    }

    #[test]
    fn page_numbers_and_titles_become_fields() {
        let chars = |s: &str| s.chars().map(|c| (c, 0)).collect::<Vec<_>>();
        let titles = vec![("heading 1".to_string(), "Chapter 3".to_string())];
        let (p, f) = pieces(&chars("Fiehn / Chapter 3 / 3"), &[look()], &titles, 3);
        assert_eq!(texts(&p), ["Fiehn / ", "{STYLEREF \"heading 1\"=Chapter 3}", " / ", "{PAGE=3}"]);
        assert_eq!(f, Some(Format::Decimal));
        let (p, f) = pieces(&chars("CHAPTER 3 · xiv"), &[look()], &titles, 14);
        assert_eq!(texts(&p), ["{STYLEREF \"heading 1\" \\* Upper=CHAPTER 3}", " · ", "{PAGE=xiv}"]);
        assert_eq!(f, Some(Format::LowerRoman));
        // A number inside a word isn't the page number.
        let (p, _) = pieces(&chars("Rain2"), &[look()], &[], 2);
        assert_eq!(texts(&p), ["Rain2"]);
    }

    #[test]
    fn a_pattern_by_section_and_by_side() {
        let line = |s: &str| vec![vec![Part { side: Side::Centre, pieces: vec![(Piece::Text(s.into()), look())] }]];
        let numbers = [1, 2, 3, 4, 5, 6];
        let firsts = [true, false, false, true, false, false];
        // Nothing on a Chapter's first page, the title on odd pages and the author on even.
        let shown: Vec<Lines> = vec![vec![], line("Author"), line("Title"), vec![], line("Title"), line("Author")];
        let set = pattern(&shown, &firsts, &numbers).unwrap();
        assert_eq!((set.default, set.even, set.first), (line("Title"), Some(line("Author")), Some(vec![])));
        // The same everywhere.
        let shown: Vec<Lines> = vec![line("1"); 6];
        assert_eq!(pattern(&shown, &firsts, &numbers).unwrap(), Set { default: line("1"), first: None, even: None });
        // No pattern.
        let shown: Vec<Lines> = vec![line("a"), line("b"), line("c"), line("d"), line("e"), line("f")];
        assert!(pattern(&shown, &firsts, &numbers).is_none());
    }

    #[test]
    fn roman_numerals() {
        assert_eq!([roman(1), roman(4), roman(14), roman(1994)], ["i", "iv", "xiv", "mcmxciv"]);
    }
}
