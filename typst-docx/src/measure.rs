//! Measuring what each target looks like on the laid-out pages.
//!
//! A `show` rule is a program, so the `.docx` can't be read off the source. Instead this walks
//! the compiled pages: Typst leaves Start and End tags in the frames around every paragraph,
//! heading and labelled element, so each run of text can be traced back to the paragraph it
//! belongs to and the target around it. Each paragraph-like stretch of text becomes a `Unit`
//! with its font, size, alignment, indents and spacing as they were actually set.
//!
//! What Typst's frames give (0.15): a page's body is flattened into the page's own frame, not
//! kept as a group, so the body area isn't given and is worked out from what's on the pages.
//! Headers, footers and page numbers sit inside `artifact` tags (for PDF accessibility), which
//! is how they're told apart from the text. Blocks are hard groups, full width by default.

use std::collections::HashMap;
use std::fmt::Write as _;

use typst::foundations::{Content, Repr, StyleChain, Value};
use typst::introspection::MetadataElem;
use typst::introspection::{Location, Tag};
use typst::layout::{Frame, FrameItem, FrameKind, Point};
use typst::model::{HeadingElem, ParElem};
use typst::text::{FontStyle, TextItem};
use typst::visualize::Paint;
use typst_layout::PagedDocument;

/// What a target picks out of the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    /// An element carrying this label, such as `<epigraph>`.
    Label(String),
    /// A heading at this level.
    Heading(usize),
}

/// A kind of text that gets one named style in the `.docx`.
#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub selector: Selector,
}

/// What to measure and how to tell it apart.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub targets: Vec<Target>,
    /// Labels of two `metadata` markers that bracket a stretch of the document set its own way
    /// (such as text included from elsewhere, with styles of its own). Every unit between them
    /// is in the scope named by the start marker's value, so its variation is accounted for.
    pub scope: Option<(String, String)>,
}

/// One line of a unit, in points from the page's top left.
#[derive(Clone, Debug)]
pub struct Line {
    pub baseline: f64,
    /// The top of the line's text (baseline minus cap height), where Typst measures from.
    pub top: f64,
    pub left: f64,
    pub right: f64,
}

/// The body area of a page (inside its margins), in points.
#[derive(Clone, Copy, Debug, Default)]
pub struct Area {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// A stretch of a unit's text set one way, in reading order.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The text as written: lines wrapped by Typst are joined again (a hyphen it added is
    /// gone), and a forced line break is a run of its own holding just "\n".
    pub text: String,
    pub family: String,
    /// In points.
    pub size: f64,
    pub italic: bool,
    pub bold: bool,
    pub fill: String,
}

/// A font's vertical measures at the size a unit is set in, in points: what Word works out a
/// line's height from (on Windows, the `OS/2` table's Windows ascent and descent).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub ascent: f64,
    pub descent: f64,
    pub cap: f64,
}

impl Metrics {
    /// The height of one single-spaced line in Word.
    pub fn line(&self) -> f64 {
        self.ascent + self.descent
    }
}

/// How a unit's lines sit between its edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Align {
    Left,
    Centre,
    Right,
    Justified,
    /// Too few lines to tell left from justified.
    Unknown,
}

/// One paragraph-like stretch of text as it was set, on one page.
#[derive(Clone, Debug)]
pub struct Unit {
    /// Index into the targets; `None` for body text.
    pub target: Option<usize>,
    /// The target of the unit before it (`Some(None)` when that was body text).
    pub after: Option<Option<usize>>,
    /// It's the rest of a paragraph begun on an earlier page.
    pub continues: bool,
    /// The scope it's in: the start marker's value (see `Options::scope`).
    pub scope: Option<String>,
    pub page: usize,
    pub area: Area,
    pub lines: Vec<Line>,
    /// The most used font family, weighted by characters.
    pub family: String,
    /// The most used size, in points.
    pub size: f64,
    /// Share of characters set in italic, and in bold (weight ≥ 600).
    pub italic: f64,
    pub bold: f64,
    /// The most used text colour.
    pub fill: String,
    /// Whether it's the first unit on its page.
    pub first_on_page: bool,
    /// From the previous unit's last baseline to this one's first, on the same page.
    pub gap_before: Option<f64>,
    /// The first 40 characters, to recognise it by.
    pub text: String,
    /// All of its text, run by run.
    pub runs: Vec<Run>,
    /// Its last line ended in a hyphen Typst added, so a continuation on the next page carries
    /// on the same word.
    pub ends_hyphenated: bool,
    /// The most used font's measures, at the unit's size.
    pub metrics: Metrics,
}

/// How close two edges must be to count as the same, in points.
const TOL: f64 = 0.6;
/// How far off centre a centred line may sit, from hanging punctuation.
const CENTRE_TOL: f64 = 3.0;
/// How far punctuation may hang past the right edge, in ems: it scales with the glyph, and a
/// monospaced full stop is wide.
const OVERHANG_EM: f64 = 0.5;

impl Unit {
    pub fn align(&self) -> Align {
        let l = &self.lines;
        let (lo, hi) = (self.area.left, self.area.right);
        // Typst hangs punctuation a little into the margin (`overhang`), so a centred line
        // ending in a full stop sits a point or two off centre.
        let centred = l.iter().all(|x| ((x.left - lo) - (hi - x.right)).abs() < CENTRE_TOL && x.left - lo > CENTRE_TOL);
        if centred {
            return Align::Centre;
        }
        let body = &l[..l.len() - 1];
        if body.is_empty() {
            return Align::Unknown; // one line: nothing to compare
        }
        // The right edge is where most lines but the last end; a line reaches it when it ends
        // there or hangs punctuation a little past it.
        let edge = most_common(body.iter().map(|x| x.right)).unwrap_or(hi);
        let reaches = |x: &Line| x.right > edge - TOL && x.right < edge + OVERHANG_EM * self.size;
        let flush_right = body.iter().filter(|x| reaches(x)).count() * 10 >= body.len() * 9;
        let starts_even = l[1..].iter().all(|x| (x.left - l[l.len() - 1].left).abs() < TOL);
        match body.len() {
            // Two lines: the first reaching the area's right edge is the only sign.
            1 if (hi - l[0].right).abs() < TOL => Align::Justified,
            1 => Align::Left,
            _ if flush_right && starts_even => Align::Justified,
            _ if flush_right => Align::Right,
            _ => Align::Left,
        }
    }

    /// Where the lines after the first start, from the area's left edge (needs two lines).
    pub fn left_indent(&self) -> Option<f64> {
        (self.lines.len() >= 2).then(|| self.lines[1..].iter().map(|x| x.left).fold(f64::MAX, f64::min) - self.area.left)
    }

    /// How much further in the first line starts than the rest (needs two lines, and not the
    /// rest of a paragraph from the page before).
    pub fn first_line_indent(&self) -> Option<f64> {
        if self.continues {
            return None;
        }
        self.left_indent().map(|rest| self.lines[0].left - self.area.left - rest)
    }

    /// Where a one-line unit that isn't centred starts, from the area's left edge.
    pub fn start(&self) -> Option<f64> {
        (self.lines.len() == 1 && self.align() != Align::Centre).then(|| self.lines[0].left - self.area.left)
    }

    /// From the right edge in to where justified lines end.
    pub fn right_indent(&self) -> Option<f64> {
        let body = &self.lines[..self.lines.len() - 1];
        (self.align() == Align::Justified).then(|| self.area.right - most_common(body.iter().map(|x| x.right)).unwrap_or(self.area.right))
    }

    /// The distance between baselines inside the unit (the median).
    pub fn pitch(&self) -> Option<f64> {
        let mut d: Vec<f64> = self.lines.windows(2).map(|w| w[1].baseline - w[0].baseline).collect();
        d.sort_by(f64::total_cmp);
        d.get(d.len() / 2).copied()
    }

    /// From the top of the body area to the top of the first line.
    pub fn from_top(&self) -> f64 {
        self.lines[0].top - self.area.top
    }
}

/// A run of text the walk found, before it's grouped into units.
struct Piece {
    unit: UnitKey,
    target: Option<usize>,
    scope: Option<String>,
    page: usize,
    at: Point,
    width: f64,
    cap: f64,
    metrics: Metrics,
    family: String,
    size: f64,
    italic: bool,
    bold: bool,
    fill: String,
    text: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum UnitKey {
    /// A paragraph, heading or labelled element.
    Element(Location),
    /// Text in none of those, numbered by where it was found.
    Loose(usize),
}

struct Walk<'a> {
    targets: &'a [Target],
    scope_labels: Option<&'a (String, String)>,
    /// Scopes begun and not yet ended, innermost last.
    scopes: Vec<String>,
    /// The elements whose Start tag has been seen but not their End, outermost first.
    open: Vec<(Location, Content)>,
    pieces: Vec<Piece>,
    page: usize,
    loose: usize,
    /// Hard groups (blocks) in the body, per page: (left, width).
    blocks: Vec<(usize, f64, f64)>,
}

impl Walk<'_> {
    fn target_of(&self, content: &Content) -> Option<usize> {
        let label = content.label().map(|l| l.resolve().to_string());
        let level = content.to_packed::<HeadingElem>().map(|h| h.resolve_level(StyleChain::default()).get());
        // A label is more specific than a heading level, so it's looked for first.
        self.targets.iter().position(|t| matches!(&t.selector, Selector::Label(n) if Some(n) == label.as_ref()))
            .or_else(|| self.targets.iter().position(|t| matches!(t.selector, Selector::Heading(n) if Some(n) == level)))
    }

    /// Inside a header, footer or page number: furniture of the page, not text.
    fn in_artifact(&self) -> bool {
        self.open.iter().any(|(_, c)| c.elem().name() == "artifact")
    }

    fn walk(&mut self, frame: &Frame, offset: Point) {
        for (pos, item) in frame.items() {
            let at = offset + *pos;
            match item {
                FrameItem::Group(group) => {
                    let t = group.transform;
                    if group.frame.kind() == FrameKind::Hard && !self.in_artifact() {
                        self.blocks.push((self.page, at.x.to_pt(), group.frame.width().to_pt()));
                    }
                    self.walk(&group.frame, at + Point::new(t.tx, t.ty));
                }
                FrameItem::Tag(tag) => self.tag(tag),
                FrameItem::Text(text) if !self.in_artifact() => self.text(at, text),
                _ => {}
            }
        }
    }

    fn tag(&mut self, tag: &Tag) {
        match tag {
            Tag::Start(content, _) => {
                if let (Some((start, end)), Some(label)) = (self.scope_labels, content.label()) {
                    let label = label.resolve().to_string();
                    if label == *start {
                        let name = content.to_packed::<MetadataElem>().map(|m| match &m.value {
                            Value::Str(s) => s.to_string(),
                            other => other.repr().to_string(),
                        });
                        self.scopes.push(name.unwrap_or_default());
                    } else if label == *end {
                        self.scopes.pop();
                    }
                }
                if let Some(loc) = content.location() {
                    self.open.push((loc, content.clone()));
                }
            }
            Tag::End(loc, ..) => {
                if let Some(i) = self.open.iter().rposition(|(l, _)| l == loc) {
                    self.open.remove(i);
                }
            }
        }
    }

    fn text(&mut self, at: Point, item: &TextItem) {
        // The innermost paragraph or heading is the unit; failing that, a labelled target.
        let unit = self.open.iter().rev()
            .find(|(_, c)| c.is::<ParElem>() || c.is::<HeadingElem>() || self.target_of(c).is_some())
            .map(|(loc, _)| UnitKey::Element(*loc));
        let unit = unit.unwrap_or_else(|| {
            let same = self.pieces.last().is_some_and(|p| matches!(p.unit, UnitKey::Loose(n) if n == self.loose));
            if !same {
                self.loose += 1;
            }
            UnitKey::Loose(self.loose)
        });
        let target = self.open.iter().rev().find_map(|(_, c)| self.target_of(c));
        let font = item.font.font();
        let info = font.info();
        let fill = match &item.fill {
            Paint::Solid(c) => c.to_hex().to_string(),
            other => format!("{other:?}"),
        };
        // Up to the last visible glyph: a line's last run ends in the space before the break,
        // which would make justified lines look ragged.
        let mut width = 0.0;
        let mut visible = 0.0;
        for g in &item.glyphs {
            width += g.x_advance.at(item.size).to_pt();
            let range = usize::from(g.range.start)..usize::from(g.range.end);
            if item.text.get(range).is_some_and(|s| !s.trim().is_empty()) {
                visible = width;
            }
        }
        let cap = item.font.metrics().cap_height.at(item.size).to_pt();
        self.pieces.push(Piece {
            unit, target, scope: self.scopes.last().cloned(), page: self.page, at, width: visible,
            cap, metrics: word_metrics(&item.font, item.size.to_pt(), cap),
            family: info.family.clone(), size: item.size.to_pt(),
            italic: info.variant.style != FontStyle::Normal, bold: info.variant.weight.to_number() >= 600,
            fill, text: item.text.to_string(),
        });
    }
}

/// Word's measures for a font: the `OS/2` Windows ascent and descent, which is what Word on
/// Windows makes a single line of, falling back to the `hhea` ones.
fn word_metrics(font: &typst::text::FontInstance, size: f64, cap: f64) -> Metrics {
    let ttf = font.ttf();
    let per_em = f64::from(ttf.units_per_em().max(1));
    let (ascent, descent) = match ttf.tables().os2 {
        Some(os2) => (f64::from(os2.windows_ascender()), f64::from(os2.windows_descender()).abs()),
        None => (f64::from(ttf.ascender()), f64::from(ttf.descender()).abs() + f64::from(ttf.line_gap())),
    };
    Metrics { ascent: ascent / per_em * size, descent: descent / per_em * size, cap }
}

fn walk<'a>(document: &PagedDocument, options: &'a Options) -> Walk<'a> {
    let mut walk = Walk {
        targets: &options.targets, scope_labels: options.scope.as_ref(), scopes: Vec::new(),
        open: Vec::new(), pieces: Vec::new(), page: 0, loose: 0, blocks: Vec::new(),
    };
    for (n, page) in document.pages().iter().enumerate() {
        walk.page = n;
        walk.walk(&page.frame, Point::zero());
    }
    walk
}

/// Every paragraph-like unit of text in the body of the pages, in order. Headers, footers and
/// page numbers are left out: they belong to the page, not to a target. Each page's body area
/// is guessed from what's on it (see `measure_in` for when it's known).
pub fn measure(document: &PagedDocument, options: &Options) -> Vec<Unit> {
    let walk = walk(document, options);
    let units = group(walk.pieces);
    let areas = areas(&units, &walk.blocks, document.pages().len());
    units.into_iter().map(|mut u| { u.area = areas[u.page]; u }).collect()
}

/// As `measure`, with each page's body area given (by its index).
pub fn measure_in(document: &PagedDocument, options: &Options, area: impl Fn(usize) -> Area) -> Vec<Unit> {
    let units = group(walk(document, options).pieces);
    units.into_iter().map(|mut u| { u.area = area(u.page); u }).collect()
}

/// A soft hyphen: what Typst sets as the hyphen it adds where it breaks a word.
const SHY: &str = "\u{ad}";

/// Whether a line ending in `text` was wrapped by Typst rather than broken by the writer: a
/// wrapped line keeps the space it was broken at, or ends where a word may break (a hyphen
/// or dash). A forced break (`\`) leaves neither, as Typst trims it away.
pub(crate) fn wraps_after(text: &str) -> bool {
    text.chars().last().is_some_and(|c| c.is_whitespace() || "-\u{2010}\u{2013}\u{2014}/".contains(c))
}

/// The unit's text, run by run, with Typst's line wrapping undone. Returns whether the last
/// line ends in a hyphen Typst added.
fn runs_of(pieces: &[Piece]) -> (Vec<Run>, bool) {
    let mut runs: Vec<Run> = Vec::new();
    let mut baseline: Option<f64> = None;
    let mut hyphenated = false;
    for p in pieces {
        let y = p.at.y.to_pt();
        let new_line = baseline.is_some_and(|b| (b - y).abs() >= 0.5);
        if p.text == SHY {
            hyphenated = true;
            continue;
        }
        if new_line {
            let wrapped = hyphenated || runs.last().is_some_and(|r| wraps_after(&r.text));
            // Some languages repeat a hyphen at the start of the next line; it isn't text.
            if hyphenated && p.text == "-" {
                hyphenated = false;
                continue;
            }
            if !wrapped {
                runs.push(Run { text: "\n".into(), family: p.family.clone(), size: p.size, italic: p.italic, bold: p.bold, fill: p.fill.clone() });
            }
        }
        hyphenated = false;
        baseline = Some(y);
        match runs.last_mut() {
            Some(r) if r.text != "\n" && r.family == p.family && r.size == p.size && r.italic == p.italic && r.bold == p.bold && r.fill == p.fill => {
                r.text.push_str(&p.text);
            }
            _ => runs.push(Run { text: p.text.clone(), family: p.family.clone(), size: p.size, italic: p.italic, bold: p.bold, fill: p.fill.clone() }),
        }
    }
    (runs, hyphenated)
}

fn most_common(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut count: Vec<(f64, usize)> = Vec::new();
    for v in values {
        let v = (v * 10.0).round() / 10.0;
        match count.iter_mut().find(|(k, _)| (*k - v).abs() < 0.05) {
            Some((_, n)) => *n += 1,
            None => count.push((v, 1)),
        }
    }
    count.into_iter().max_by_key(|(_, n)| *n).map(|(v, _)| v)
}

/// The body area of each page, worked out from what's on it: its width is the commonest width
/// of a full-width block; its left edge is where a page's lines and blocks start furthest left
/// (borrowed from the page two before when a page has only centred lines, as facing pages
/// differ); its top is the commonest top of a page's first line.
fn areas(units: &[Unit], blocks: &[(usize, f64, f64)], pages: usize) -> Vec<Area> {
    let width = most_common(blocks.iter().map(|b| b.2)).unwrap_or(0.0);
    let top = most_common(units.iter().filter(|u| u.first_on_page && u.continues).map(|u| u.lines[0].top))
        .or_else(|| most_common(units.iter().filter(|u| u.first_on_page).map(|u| u.lines[0].top)))
        .unwrap_or(0.0);
    let mut lefts: Vec<Option<f64>> = vec![None; pages];
    for (page, left) in lefts.iter_mut().enumerate() {
        let from_blocks = blocks.iter().filter(|b| b.0 == page && (b.2 - width).abs() < TOL).map(|b| b.1);
        // A line after a paragraph's first starts at the left edge unless the paragraph is indented.
        let from_lines = units.iter().filter(|u| u.page == page).flat_map(|u| u.lines.iter().skip(1).map(|l| l.left));
        *left = from_blocks.chain(from_lines).reduce(f64::min);
    }
    (0..pages).map(|p| {
        let left = lefts[p].or_else(|| p.checked_sub(2).and_then(|q| lefts[q])).or_else(|| lefts.get(p + 2).copied().flatten()).unwrap_or(0.0);
        Area { left, top, right: left + width, bottom: f64::INFINITY }
    }).collect()
}

/// Groups pieces into units, in the order their first piece was found.
fn group(pieces: Vec<Piece>) -> Vec<Unit> {
    let mut order: Vec<(UnitKey, usize)> = Vec::new();
    let mut by_key: HashMap<(UnitKey, usize), Vec<Piece>> = HashMap::new();
    for p in pieces {
        // A paragraph broken across pages is measured as one unit per page.
        let key = (p.unit, p.page);
        if !by_key.contains_key(&key) {
            order.push(key);
        }
        by_key.entry(key).or_default().push(p);
    }
    let mut units: Vec<Unit> = Vec::new();
    let mut last_key: Option<UnitKey> = None;
    for key in order {
        let pieces = by_key.remove(&key).unwrap();
        if pieces.iter().all(|p| p.text.trim().is_empty() || p.text == "\u{ad}") {
            continue;
        }
        let mut lines: Vec<Line> = Vec::new();
        let mut sorted: Vec<&Piece> = pieces.iter().collect();
        sorted.sort_by(|a, b| a.at.y.to_pt().total_cmp(&b.at.y.to_pt()));
        for p in sorted {
            let (y, x) = (p.at.y.to_pt(), p.at.x.to_pt());
            match lines.last_mut() {
                Some(l) if (l.baseline - y).abs() < 0.5 => {
                    l.left = l.left.min(x);
                    l.right = l.right.max(x + p.width);
                    l.top = l.top.min(y - p.cap);
                }
                _ => lines.push(Line { baseline: y, top: y - p.cap, left: x, right: x + p.width }),
            }
        }
        let chars = |p: &Piece| p.text.chars().filter(|c| !c.is_whitespace()).count().max(1) as f64;
        let total: f64 = pieces.iter().map(chars).sum();
        let most = |f: &dyn Fn(&Piece) -> String| {
            let mut count: HashMap<String, f64> = HashMap::new();
            for p in &pieces {
                *count.entry(f(p)).or_default() += chars(p);
            }
            count.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))).map(|(k, _)| k).unwrap_or_default()
        };
        let page = pieces[0].page;
        let previous = units.last();
        let first_on_page = previous.is_none_or(|u| u.page != page);
        let gap_before = previous.filter(|u| u.page == page).map(|u| lines[0].baseline - u.lines.last().unwrap().baseline);
        let continues = matches!(key.0, UnitKey::Element(_)) && last_key == Some(key.0);
        let text: String = pieces.iter().map(|p| p.text.as_str()).collect::<String>().chars().take(40).collect();
        let (runs, ends_hyphenated) = runs_of(&pieces);
        let main = most(&|p| format!("{}|{:.2}", p.family, p.size));
        let metrics = pieces.iter().find(|p| format!("{}|{:.2}", p.family, p.size) == main).map_or(Metrics::default(), |p| p.metrics);
        units.push(Unit {
            runs,
            ends_hyphenated,
            metrics,
            target: pieces[0].target,
            after: previous.map(|u| u.target),
            continues,
            scope: pieces[0].scope.clone(),
            page,
            area: Area::default(),
            family: most(&|p| p.family.clone()),
            size: most(&|p| format!("{:.2}", p.size)).parse().unwrap_or(0.0),
            italic: pieces.iter().filter(|p| p.italic).map(chars).sum::<f64>() / total,
            bold: pieces.iter().filter(|p| p.bold).map(chars).sum::<f64>() / total,
            fill: most(&|p| p.fill.clone()),
            first_on_page,
            gap_before,
            lines,
            text,
        });
        last_key = Some(key.0);
    }
    units
}

/// How much each target varies: for each measured property, its values and how many units
/// had each. A target that gets one style has one value per property.
pub fn summary(units: &[Unit], targets: &[Target]) -> String {
    type Prop<'a> = (&'a str, Box<dyn Fn(&Unit) -> String + 'a>);
    let name = |t: Option<usize>| t.map_or("Body".to_string(), |i| targets[i].name.clone());
    let mut out = String::new();
    // Each target, then each target within each scope, in the order first found.
    let mut groups: Vec<(Option<usize>, Option<String>)> = vec![(None, None)];
    groups.extend((0..targets.len()).map(|i| (Some(i), None)));
    for u in units {
        if u.scope.is_some() && !groups.iter().any(|g| g.0 == u.target && g.1 == u.scope) {
            groups.push((u.target, u.scope.clone()));
        }
    }
    let r = |x: f64| format!("{:.1}", (x * 10.0).round() / 10.0 + 0.0);
    for (t, scope) in groups {
        let us: Vec<&Unit> = units.iter().filter(|u| u.target == t && u.scope == scope).collect();
        if us.is_empty() {
            continue;
        }
        let within = scope.as_ref().map_or(String::new(), |s| format!(" ({s})"));
        let _ = writeln!(out, "== {}{within} ({} units)", name(t), us.len());
        let props: Vec<Prop> = vec![
            ("font", Box::new(|u| format!("{} {}pt", u.family, r(u.size)))),
            ("italic/bold", Box::new(|u| format!("{}/{}", r(u.italic), r(u.bold)))),
            ("fill", Box::new(|u| u.fill.clone())),
            ("align", Box::new(|u| format!("{:?}", u.align()))),
            ("left indent", Box::new(move |u| u.left_indent().map_or("-".into(), r))),
            ("first-line indent", Box::new(move |u| u.first_line_indent().map_or("-".into(), r))),
            ("one-line start", Box::new(move |u| u.start().map_or("-".into(), r))),
            ("right indent", Box::new(move |u| u.right_indent().map_or("-".into(), r))),
            ("line pitch", Box::new(move |u| u.pitch().map_or("-".into(), r))),
            ("first on page", Box::new(|u| if u.continues { "-".into() } else { u.first_on_page.to_string() })),
            ("from top (first on page)", Box::new(move |u| if u.first_on_page && !u.continues { r(u.from_top()) } else { "-".into() })),
            ("gap before", Box::new(move |u| u.gap_before.map_or("-".into(), r))),
        ];
        for (prop, f) in &props {
            let mut count: Vec<(String, usize)> = Vec::new();
            for u in &us {
                let v = f(u);
                match count.iter_mut().find(|(k, _)| *k == v) {
                    Some((_, n)) => *n += 1,
                    None => count.push((v, 1)),
                }
            }
            let flag = if count.iter().filter(|(k, _)| k != "-").count() > 1 { "  <- varies" } else { "" };
            let vals: Vec<String> = count.iter().map(|(k, n)| format!("{k} ×{n}")).collect();
            let _ = writeln!(out, "  {prop:<26} {}{flag}", vals.join(", "));
        }
        // Where variation in the first-line indent comes from: what each kind follows.
        let mut by_after: Vec<(String, String, usize)> = Vec::new();
        for u in us.iter().filter(|u| u.first_line_indent().is_some()) {
            let after = u.after.map_or("start".to_string(), name);
            let indent = u.first_line_indent().map_or("-".to_string(), r);
            match by_after.iter_mut().find(|(a, i, _)| *a == after && *i == indent) {
                Some((_, _, n)) => *n += 1,
                None => by_after.push((after, indent, 1)),
            }
        }
        if by_after.iter().map(|(_, i, _)| i).collect::<std::collections::HashSet<_>>().len() > 1 {
            let _ = writeln!(out, "  first-line indent by what comes before:");
            for (a, i, n) in by_after {
                let _ = writeln!(out, "    after {a:<20} {i} ×{n}");
            }
        }
    }
    out
}
