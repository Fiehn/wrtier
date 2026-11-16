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
use typst::layout::{BlockElem, Frame, FrameItem, FrameKind, Point};
use typst::math::EquationElem;
use typst::model::{FootnoteElem, FootnoteEntry, HeadingElem, ParElem};
use typst::pdf::{PdfMarkerTag, PdfMarkerTagKind};
use typst::syntax::{Source, Span};
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
    pub small_caps: bool,
    pub underline: bool,
    pub strike: bool,
    /// Where each of its characters came from in the main source file (byte offset), if it
    /// came from there; all `None` when not asked for (see `measure_in`).
    pub at: Vec<Option<usize>>,
    /// It's a footnote's number, the footnote's index in the order they were met (from 0): in
    /// the body, the reference to it; in the footnote's own text, its number there. Its text is
    /// the number as Typst showed it; Word numbers footnotes itself.
    pub note: Option<usize>,
}

/// Where a unit is in a list: which list (the outermost one, as the walk met them), how deep,
/// whether numbered, and its item's marker as shown ("•", "2.") and where that starts.
#[derive(Clone, Debug, PartialEq)]
pub struct ListPlace {
    pub list: usize,
    pub level: usize,
    pub numbered: bool,
    pub marker: String,
    /// Where the marker starts, in points from the page's left.
    pub marker_left: f64,
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
    /// Share of characters set in italic, in bold (weight ≥ 600), in small caps, underlined
    /// and struck through.
    pub italic: f64,
    pub bold: f64,
    pub small_caps: f64,
    pub underline: f64,
    pub strike: f64,
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
    /// The first text of a list item's body: where it is in the list.
    pub list: Option<ListPlace>,
    /// It's (some of) the text of footnote n (see `Run::note`), set at the foot of its page,
    /// not part of the body.
    pub note: Option<usize>,
    /// Where the block it's in ends on the right, inside its inset, in points from the page's
    /// left: how far in its lines may go, which ragged lines don't show.
    pub right_bound: Option<f64>,
}

/// Something on the pages the `.docx` doesn't carry as it was set (see `crate::Uncarried`).
#[derive(Clone, Debug, PartialEq)]
pub struct Skipped {
    pub what: crate::Uncarried,
    /// Where it starts in the main source file (byte offset), if it came from there.
    pub at: Option<usize>,
    /// The target it's in, if any.
    pub target: Option<usize>,
}

/// What the pages hold: their text, and what of them can't be carried.
#[derive(Clone, Debug, Default)]
pub struct Measured {
    pub units: Vec<Unit>,
    pub skipped: Vec<Skipped>,
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
        // A line reaches the right edge when it ends there or hangs punctuation a little past it.
        let edge = self.right_edge();
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
        if self.align() == Align::Justified {
            return Some(self.area.right - self.right_edge());
        }
        // Ragged lines end anywhere: the block they're in says where they may go.
        self.right_bound.map(|b| self.area.right - b).filter(|r| *r > TOL)
    }

    /// Where lines but the last end: the area's right edge when one ends there, else where most
    /// do (a short note's two lines, one hanging a comma, have no most).
    fn right_edge(&self) -> f64 {
        let hi = self.area.right;
        let body = &self.lines[..self.lines.len() - 1];
        if body.iter().any(|x| (x.right - hi).abs() < TOL) { hi } else { most_common(body.iter().map(|x| x.right)).unwrap_or(hi) }
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
    small_caps: bool,
    underline: bool,
    strike: bool,
    text: String,
    /// The first text of a list item's body, and where its list is.
    list: Option<ListPlace>,
    /// Where each character came from in the main source (see `Run::at`).
    sources: Vec<Option<usize>>,
    /// A footnote's number (see `Run::note`).
    note: Option<usize>,
    /// Text of footnote n (see `Unit::note`).
    in_note: Option<usize>,
    /// Set off its line (a superscript in an equation, a footnote's number): it's on the line
    /// of the text nearest it, whatever its own baseline.
    inline: bool,
    /// See `Unit::right_bound`.
    right_bound: Option<f64>,
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
    /// The hard groups (blocks) being walked, innermost last: where each ends on the right.
    rights: Vec<f64>,
    /// The main source file, to say where things came from in it.
    main: Option<&'a Source>,
    /// Whether each character's place in `main` is wanted (see `Run::at`).
    places: bool,
    small_caps: crate::fonts::SmallCaps,
    /// A list item's marker, read and waiting for its body's first text: its text and left edge.
    marker: Option<(String, f64)>,
    /// The outermost lists met so far, to number them.
    lists: Vec<Location>,
    /// The footnotes met so far, in order, to number them.
    notes: Vec<Location>,
    skipped: Vec<Skipped>,
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
        self.inside("artifact")
    }

    /// Inside an element of this name (as Typst names it: "table", "underline").
    fn inside(&self, name: &str) -> bool {
        self.open.iter().any(|(_, c)| c.elem().name() == name)
    }

    /// The innermost target around what's being read.
    fn current_target(&self) -> Option<usize> {
        self.open.iter().rev().find_map(|(_, c)| self.target_of(c))
    }

    /// Footnote `loc`'s index, in the order footnotes were met.
    fn note_index(&mut self, loc: Location) -> usize {
        self.notes.iter().position(|l| *l == loc).unwrap_or_else(|| {
            self.notes.push(loc);
            self.notes.len() - 1
        })
    }

    /// Notes that `what`, from `span`, isn't carried; once for each place it comes from.
    fn skip(&mut self, what: crate::Uncarried, span: Span) {
        if span.is_detached() {
            return;
        }
        let at = self.main.filter(|m| span.id() == Some(m.id())).and_then(|m| m.find(span)).map(|n| n.range().start);
        let skipped = Skipped { what, at, target: self.current_target() };
        if !self.skipped.contains(&skipped) {
            self.skipped.push(skipped);
        }
    }

    fn walk(&mut self, frame: &Frame, offset: Point) {
        for (pos, item) in frame.items() {
            let at = offset + *pos;
            match item {
                FrameItem::Group(group) => {
                    let t = group.transform;
                    let hard = group.frame.kind() == FrameKind::Hard && !self.in_artifact();
                    let left = at + Point::new(t.tx, t.ty);
                    if hard {
                        self.blocks.push((self.page, at.x.to_pt(), group.frame.width().to_pt()));
                        self.rights.push(left.x.to_pt() + group.frame.width().to_pt());
                    }
                    self.walk(&group.frame, left);
                    if hard {
                        self.rights.pop();
                    }
                }
                FrameItem::Tag(tag) => self.tag(tag),
                FrameItem::Text(text) if !self.in_artifact() => self.text(at, text),
                FrameItem::Image(_, _, span) if !self.in_artifact() => self.skip(crate::Uncarried::Image, *span),
                // Underlines and strike-throughs are carried as such, a table's lines with the
                // table; anything else drawn among the text isn't.
                FrameItem::Shape(_, span) if !self.in_artifact() && !["underline", "strike", "table", "grid"].iter().any(|n| self.inside(n)) => {
                    self.skip(crate::Uncarried::Drawing, *span);
                }
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
                if !self.in_artifact() {
                    if content.is::<EquationElem>() {
                        self.skip(crate::Uncarried::Equation, content.span());
                    } else if matches!(content.elem().name(), "table" | "grid") && !self.inside("table") && !self.inside("grid") {
                        self.skip(crate::Uncarried::Table, content.span());
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

    fn text(&mut self, at_point: Point, item: &TextItem) {
        // A list item's marker isn't text: Word numbers the list itself.
        let tag = self.open.iter().rev().find_map(|(_, c)| c.to_packed::<PdfMarkerTag>()).map(|t| t.kind.clone());
        if matches!(tag, Some(PdfMarkerTagKind::ListItemLabel)) {
            match &mut self.marker {
                Some((text, _)) => text.push_str(&item.text),
                None => self.marker = Some((item.text.to_string(), at_point.x.to_pt())),
            }
            return;
        }
        // A footnote: its number (in a `Label` marker), in the text or in the note at the foot
        // of the page, is the footnote's number in Word; the rest of the note is its text. A
        // footnote that refers to another one keeps its number as text.
        let numbering = matches!(tag, Some(PdfMarkerTagKind::Label));
        let footnote = self.open.iter().rev().find_map(|(loc, c)| {
            if let Some(entry) = c.to_packed::<FootnoteEntry>() {
                entry.note.location().map(|l| (l, true))
            } else {
                c.to_packed::<FootnoteElem>().filter(|f| !f.is_ref()).map(|_| (*loc, false))
            }
        });
        let (note, in_note) = match footnote {
            Some((loc, in_entry)) => {
                let n = self.note_index(loc);
                (numbering.then_some(n), in_entry.then_some(n))
            }
            None => (None, None),
        };
        // The innermost paragraph, heading, list item's body, table cell, footnote or equation
        // set apart is the unit; failing that, a labelled target.
        let body = |c: &Content| c.to_packed::<PdfMarkerTag>().is_some_and(|t| matches!(t.kind, PdfMarkerTagKind::ListItemBody));
        let display = |c: &Content| c.to_packed::<EquationElem>().is_some_and(|e| e.block.get(StyleChain::default()));
        let unit = self.open.iter().rev()
            .find(|(_, c)| c.is::<ParElem>() || c.is::<HeadingElem>() || body(c) || c.is::<FootnoteEntry>() || display(c)
                || c.elem().name() == "cell" || self.target_of(c).is_some())
            .map(|(loc, _)| UnitKey::Element(*loc));
        let lists: Vec<&(Location, Content)> = self.open.iter().filter(|(_, c)| matches!(c.elem().name(), "list" | "enum")).collect();
        let list = match (self.marker.take(), lists.first(), lists.last()) {
            (Some((marker, marker_left)), Some((outer, _)), Some((_, inner))) => {
                let list = self.lists.iter().position(|l| l == outer).unwrap_or_else(|| {
                    self.lists.push(*outer);
                    self.lists.len() - 1
                });
                Some(ListPlace { list, level: lists.len() - 1, numbered: inner.elem().name() == "enum", marker: marker.trim().to_string(), marker_left })
            }
            _ => None,
        };
        let unit = unit.unwrap_or_else(|| {
            let same = self.pieces.last().is_some_and(|p| matches!(p.unit, UnitKey::Loose(n) if n == self.loose));
            if !same {
                self.loose += 1;
            }
            UnitKey::Loose(self.loose)
        });
        let target = if in_note.is_some() { None } else { self.current_target() };
        let inline = note.is_some() || self.inside("equation");
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
        let place = self.main.filter(|_| self.places).map_or_else(Vec::new, |main| sources(item, main));
        let (underline, strike) = (self.inside("underline"), self.inside("strike"));
        let small_caps = self.small_caps.of(item);
        // A block's inset isn't a frame of its own: its lines just stop short of its right edge.
        let size = item.size.to_pt();
        let right_bound = self.open.iter().rev().find_map(|(_, c)| c.to_packed::<BlockElem>()).zip(self.rights.last()).map(|(block, right)| {
            let inset = block.inset.get(StyleChain::default()).right.map_or(0.0, |r| r.abs.abs.to_pt() + r.abs.em.get() * size);
            right - inset
        });
        self.pieces.push(Piece {
            unit, target, scope: self.scopes.last().cloned(), page: self.page, at: at_point, width: visible,
            cap, metrics: word_metrics(&item.font, item.size.to_pt(), cap),
            family: info.family.clone(), size: item.size.to_pt(),
            italic: info.variant.style != FontStyle::Normal, bold: info.variant.weight.to_number() >= 600,
            fill, small_caps, underline, strike, text: item.text.to_string(), list, sources: place,
            note, in_note, inline, right_bound,
        });
    }
}

/// Where each character of a text item came from in `main` (byte offsets): a glyph's span and
/// offset give its first character's place, and the others in its cluster follow on.
fn sources(item: &TextItem, main: &Source) -> Vec<Option<usize>> {
    let mut by_byte: Vec<Option<usize>> = vec![None; item.text.len()];
    for g in &item.glyphs {
        let (span, offset) = g.span;
        if span.id() != Some(main.id()) {
            continue;
        }
        // Text a function made (`#lorem`, a counter) has its offset into what it made, not into
        // the source: it only counts when it stays inside the node it points at.
        let Some(node) = main.find(span) else { continue };
        let start = node.range().start + usize::from(offset);
        if !node.range().contains(&start) {
            continue;
        }
        let range = usize::from(g.range.start)..usize::from(g.range.end).min(item.text.len());
        for (k, b) in range.enumerate() {
            by_byte[b] = Some(start + k);
        }
    }
    item.text.char_indices().map(|(b, _)| by_byte[b]).collect()
}

/// Word's measures for a font: the `OS/2` Windows ascent and descent, which is what Word on
/// Windows makes a single line of, falling back to the `hhea` ones.
pub(crate) fn word_metrics(font: &typst::text::FontInstance, size: f64, cap: f64) -> Metrics {
    let ttf = font.ttf();
    let per_em = f64::from(ttf.units_per_em().max(1));
    let (ascent, descent) = match ttf.tables().os2 {
        Some(os2) => (f64::from(os2.windows_ascender()), f64::from(os2.windows_descender()).abs()),
        None => (f64::from(ttf.ascender()), f64::from(ttf.descender()).abs() + f64::from(ttf.line_gap())),
    };
    Metrics { ascent: ascent / per_em * size, descent: descent / per_em * size, cap }
}

fn walk<'a>(document: &PagedDocument, options: &'a Options, main: Option<&'a Source>, places: bool) -> Walk<'a> {
    let mut walk = Walk {
        targets: &options.targets, scope_labels: options.scope.as_ref(), scopes: Vec::new(),
        open: Vec::new(), pieces: Vec::new(), page: 0, loose: 0, blocks: Vec::new(), main, places,
        small_caps: crate::fonts::SmallCaps::default(), marker: None, lists: Vec::new(), notes: Vec::new(), skipped: Vec::new(), rights: Vec::new(),
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
    let walk = walk(document, options, None, false);
    let units = group_all(walk.pieces);
    let areas = areas(&units, &walk.blocks, document.pages().len());
    units.into_iter().map(|mut u| { u.area = areas[u.page]; u }).collect()
}

/// As `measure`, with each page's body area given (by its index, and the scope of the text on
/// it, which may set its own page), and what can't be carried,
/// with where in `main`, the source file the document was compiled from, it came from. With
/// `places`, each run also says where its characters came from in it.
pub fn measure_in(document: &PagedDocument, options: &Options, area: impl Fn(usize, Option<&str>) -> Area, main: &Source, places: bool) -> Measured {
    let walk = walk(document, options, Some(main), places);
    let units = group_all(walk.pieces).into_iter().map(|mut u| { u.area = area(u.page, u.scope.as_deref()); u }).collect();
    Measured { units, skipped: walk.skipped }
}

/// A soft hyphen: what Typst sets as the hyphen it adds where it breaks a word.
const SHY: &str = "\u{ad}";

/// Whether a line ending in `text` was wrapped by Typst rather than broken by the writer: a
/// wrapped line keeps the space it was broken at, or ends where a word may break (a hyphen
/// or dash). A forced break (`\`) leaves neither, as Typst trims it away.
pub(crate) fn wraps_after(text: &str) -> bool {
    text.chars().last().is_some_and(|c| c.is_whitespace() || "-\u{2010}\u{2013}\u{2014}/".contains(c))
}

/// A run set as a piece is (its text and places left to fill in).
fn run_of(p: &Piece) -> Run {
    Run {
        text: String::new(), family: p.family.clone(), size: p.size, italic: p.italic, bold: p.bold, fill: p.fill.clone(),
        small_caps: p.small_caps, underline: p.underline, strike: p.strike, at: Vec::new(), note: p.note,
    }
}

/// A piece's characters' places, one per character (`None` where not known).
fn places(p: &Piece) -> Vec<Option<usize>> {
    let n = p.text.chars().count();
    if p.sources.len() == n { p.sources.clone() } else { vec![None; n] }
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
                runs.push(Run { text: "\n".into(), at: vec![None], ..run_of(p) });
            }
        }
        hyphenated = false;
        baseline = Some(y);
        match runs.last_mut() {
            Some(r) if r.text != "\n" && Run { text: r.text.clone(), at: r.at.clone(), ..run_of(p) } == *r => {
                r.text.push_str(&p.text);
                r.at.extend(places(p));
            }
            _ => runs.push(Run { text: p.text.clone(), at: places(p), ..run_of(p) }),
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
    let units: Vec<&Unit> = units.iter().filter(|u| u.note.is_none()).collect();
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

/// The body's units, then the footnotes': each in the order their first piece was found, and
/// each on its own, so a page's footnotes don't come between a paragraph and its rest on the
/// next page.
fn group_all(pieces: Vec<Piece>) -> Vec<Unit> {
    let (notes, body): (Vec<Piece>, Vec<Piece>) = pieces.into_iter().partition(|p| p.in_note.is_some());
    let mut units = group(body);
    units.extend(group(notes));
    units
}

/// Puts each piece set off its line (see `Piece::inline`) on the baseline of the nearest piece
/// that isn't, before or after it in reading order; when none is, on the largest piece's.
fn snap(pieces: &mut [Piece]) {
    let set: Vec<Option<f64>> = pieces.iter().map(|p| (!p.inline).then(|| p.at.y.to_pt())).collect();
    let fallback = pieces.iter().max_by(|a, b| a.size.total_cmp(&b.size)).map(|p| p.at.y);
    for i in 0..pieces.len() {
        if !pieces[i].inline {
            continue;
        }
        let y = pieces[i].at.y.to_pt();
        let before = set[..i].iter().rev().find_map(|x| *x);
        let after = set[i + 1..].iter().find_map(|x| *x);
        let nearest = match (before, after) {
            (Some(b), Some(a)) => Some(if (b - y).abs() <= (a - y).abs() { b } else { a }),
            (b, a) => b.or(a),
        };
        if let Some(to) = nearest.map(typst::layout::Abs::pt).or(fallback) {
            pieces[i].at.y = to;
        }
    }
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
        let mut pieces = by_key.remove(&key).unwrap();
        snap(&mut pieces);
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
            list: pieces.iter().find_map(|p| p.list.clone()),
            note: pieces[0].in_note,
            right_bound: most_common(pieces.iter().filter_map(|p| p.right_bound)),
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
            small_caps: pieces.iter().filter(|p| p.small_caps).map(chars).sum::<f64>() / total,
            underline: pieces.iter().filter(|p| p.underline).map(chars).sum::<f64>() / total,
            strike: pieces.iter().filter(|p| p.strike).map(chars).sum::<f64>() / total,
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
