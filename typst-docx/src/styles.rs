//! From measured units to Word styles and paragraphs.
//!
//! Each target becomes one named style holding what most of its units have in common. Prose is
//! Word's Normal; a target with a heading role is Word's built-in "heading n". Two kinds of
//! variant are made, each based on the style it varies:
//!
//! - a flush variant ("First Paragraph" for prose, "Epigraph, first" for others), for the
//!   paragraphs of an indented target that come out flush because of what's before them. The
//!   spike showed that's decided entirely by what comes before, so it's decided per kind of
//!   predecessor, not per paragraph;
//! - a scoped variant ("Normal (Zampanò)"), for units between the caller's scope markers.
//!
//! What's left over (a paragraph that doesn't fit its style) becomes direct formatting on just
//! that paragraph, and is counted for the report.

use std::collections::HashMap;

use crate::measure::{Align, Run, Unit};
use crate::probe::Setup;
use crate::{Note, Role, Target, What};

/// How close two lengths must be to count as the same, in points.
const SAME: f64 = 1.0;

/// Which style a paragraph gets.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    /// The target, after heading targets of one level are merged; `None` for prose.
    pub target: Option<usize>,
    pub scope: Option<String>,
    /// The flush variant.
    pub first: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Normal,
    Heading(u8),
    Custom,
}

/// Paragraph and run formatting; `None` leaves it to the style it's based on.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Props {
    pub family: Option<String>,
    pub size: Option<f64>,
    pub italic: Option<bool>,
    pub bold: Option<bool>,
    /// Hex, without `#`, such as "000000".
    pub fill: Option<String>,
    pub align: Option<Align>,
    pub left: Option<f64>,
    pub right: Option<f64>,
    /// Negative for a hanging indent.
    pub first_line: Option<f64>,
    /// Line spacing as a multiple of the font's single line.
    pub line: Option<f64>,
    pub before: Option<f64>,
    pub after: Option<f64>,
    pub page_break: Option<bool>,
    pub keep_next: Option<bool>,
}

impl Props {
    /// `self` with `over`'s values on top.
    fn and(&self, over: &Props) -> Props {
        Props {
            family: over.family.clone().or(self.family.clone()),
            size: over.size.or(self.size),
            italic: over.italic.or(self.italic),
            bold: over.bold.or(self.bold),
            fill: over.fill.clone().or(self.fill.clone()),
            align: over.align.or(self.align),
            left: over.left.or(self.left),
            right: over.right.or(self.right),
            first_line: over.first_line.or(self.first_line),
            line: over.line.or(self.line),
            before: over.before.or(self.before),
            after: over.after.or(self.after),
            page_break: over.page_break.or(self.page_break),
            keep_next: over.keep_next.or(self.keep_next),
        }
    }

    /// Only what differs from `base`.
    fn beyond(&self, base: &Props) -> Props {
        fn keep<T: PartialEq + Clone>(mine: &Option<T>, base: &Option<T>) -> Option<T> {
            if mine == base { None } else { mine.clone() }
        }
        fn keep_len(mine: Option<f64>, base: Option<f64>) -> Option<f64> {
            match (mine, base) {
                (Some(m), Some(b)) if (m - b).abs() < 0.05 => None,
                _ => mine,
            }
        }
        Props {
            family: keep(&self.family, &base.family),
            size: keep_len(self.size, base.size),
            italic: keep(&self.italic, &base.italic),
            bold: keep(&self.bold, &base.bold),
            fill: keep(&self.fill, &base.fill),
            align: keep(&self.align, &base.align),
            left: keep_len(self.left, base.left),
            right: keep_len(self.right, base.right),
            first_line: keep_len(self.first_line, base.first_line),
            line: match (self.line, base.line) {
                (Some(m), Some(b)) if (m - b).abs() < 0.005 => None,
                (m, _) => m,
            },
            before: keep_len(self.before, base.before),
            after: keep_len(self.after, base.after),
            page_break: keep(&self.page_break, &base.page_break),
            keep_next: keep(&self.keep_next, &base.keep_next),
        }
    }
}

pub(crate) struct Style {
    pub id: String,
    /// As Word shows it ("heading 1" is how Word names its built-in Heading 1).
    pub name: String,
    pub kind: Kind,
    pub based_on: Option<usize>,
    /// The style Word gives the paragraph typed after one in this style.
    pub next: Option<usize>,
    /// Everything this style sets, including what it shares with its base (see `own`).
    pub props: Props,
    pub key: Key,
}

pub(crate) struct Para {
    pub style: usize,
    pub runs: Vec<Run>,
    /// Paragraph formatting the style doesn't explain.
    pub direct: Props,
}

pub(crate) struct Laid {
    pub styles: Vec<Style>,
    pub paras: Vec<Para>,
    pub notes: Vec<Note>,
    /// Plain styles, and only italic and bold on runs (see `make_plain`).
    pub plain: bool,
}

impl Laid {
    /// Everything that applies to a paragraph in style `i`, through its bases.
    pub fn resolved(&self, i: usize) -> Props {
        let s = &self.styles[i];
        match s.based_on {
            Some(b) => self.resolved(b).and(&s.props),
            None => s.props.clone(),
        }
    }

    /// What style `i` sets beyond its base.
    pub fn own(&self, i: usize) -> Props {
        match self.styles[i].based_on {
            Some(b) => self.styles[i].props.beyond(&self.resolved(b)),
            None => self.styles[i].props.clone(),
        }
    }
}

/// The most common value, after rounding to `step`; ties go to the smaller value.
fn mode(values: impl Iterator<Item = f64>, step: f64) -> Option<f64> {
    let mut count: Vec<(i64, usize)> = Vec::new();
    for v in values {
        let k = (v / step).round() as i64;
        match count.iter_mut().find(|(x, _)| *x == k) {
            Some((_, n)) => *n += 1,
            None => count.push((k, 1)),
        }
    }
    count.sort();
    count.iter().rev().max_by_key(|(_, n)| *n).map(|(k, _)| *k as f64 * step)
}

fn mode_of<T: Clone + PartialEq>(values: impl Iterator<Item = T>) -> Option<T> {
    let mut count: Vec<(T, usize)> = Vec::new();
    for v in values {
        match count.iter_mut().find(|(x, _)| *x == v) {
            Some((_, n)) => *n += 1,
            None => count.push((v, 1)),
        }
    }
    // The first found wins a tie: max_by_key keeps the last, so search reversed.
    count.into_iter().rev().max_by_key(|(_, n)| *n).map(|(v, _)| v)
}

/// A unit and where it sits among the units that are kept.
struct Kept<'a> {
    unit: &'a Unit,
    /// Its index in the measured units, to tell whether two kept units were neighbours.
    index: usize,
    target: Option<usize>,
}

/// Line spacing of a unit as a multiple of its font's single line, if it has two lines.
fn multiple(u: &Unit) -> Option<f64> {
    let line = u.metrics.line();
    u.pitch().filter(|_| line > 0.0).map(|p| p / line)
}

/// What `units` have in common.
fn measured(units: &[&Unit], kind: Kind) -> Props {
    let starts: Vec<&&Unit> = units.iter().filter(|u| !u.continues).collect();
    // A forced line break leaves a short line, which looks ragged even in justified text, so
    // such units only count when they're centred or set right.
    let clean: Vec<&&Unit> = units.iter().filter(|u| !u.runs.iter().any(|r| r.text == "
")).collect();
    let align = mode_of(clean.iter().map(|u| u.align()).filter(|a| *a != Align::Unknown))
        .or_else(|| mode_of(units.iter().map(|u| u.align()).filter(|a| matches!(a, Align::Centre | Align::Right))));
    let left = mode(starts.iter().filter_map(|u| u.left_indent()), 0.1)
        .or_else(|| (align != Some(Align::Centre)).then(|| mode(starts.iter().filter_map(|u| u.start()), 0.1)).flatten());
    Props {
        family: mode_of(units.iter().map(|u| u.family.clone())),
        size: mode(units.iter().map(|u| u.size), 0.05),
        italic: Some(units.iter().filter(|u| u.italic > 0.5).count() * 2 > units.len()),
        bold: Some(units.iter().filter(|u| u.bold > 0.5).count() * 2 > units.len()),
        fill: mode_of(units.iter().map(|u| hex(&u.fill))),
        // One-line units that aren't centred: set left (a heading isn't justified).
        align: align.or_else(|| (left.is_some() && !clean.is_empty()).then_some(Align::Left)),
        left,
        right: mode(units.iter().filter_map(|u| u.right_indent()), 0.1),
        first_line: mode(starts.iter().filter_map(|u| u.first_line_indent()), 0.1),
        line: mode(units.iter().filter_map(|u| multiple(u)), 0.01),
        before: None,
        after: None,
        page_break: None,
        keep_next: matches!(kind, Kind::Heading(_)).then_some(true),
    }
}

/// How far a unit's first line is indented: measured against its own later lines, or, for a
/// one-line unit that isn't centred, against `left`, where its style's lines start.
fn indent_of(u: &Unit, left: Option<f64>) -> Option<f64> {
    if u.continues {
        return None;
    }
    u.first_line_indent().or_else(|| u.start().zip(left).map(|(start, left)| start - left))
}

/// "#000000ff" or "#000000" → "000000".
fn hex(fill: &str) -> String {
    fill.trim_start_matches('#').chars().take(6).collect::<String>().to_uppercase()
}

/// Whether the units at `positions` start their pages by a page break of their own. Each must
/// be at the top of a page; and since one may get there by chance, either several are, or one
/// shows it: the page before it ended with room to spare, or it's the document's first.
fn start_pages(kept: &[Kept], positions: &[usize]) -> bool {
    let all_on_top = positions.iter().all(|i| kept[*i].unit.first_on_page && !kept[*i].unit.continues);
    let shown = |i: usize| match i.checked_sub(1).map(|j| kept[j].unit) {
        None => true,
        Some(before) => {
            let room = before.area.bottom - before.lines.last().map_or(before.area.bottom, |l| l.baseline);
            room > 0.25 * (before.area.bottom - before.area.top)
        }
    };
    all_on_top && (positions.len() >= 2 || positions.iter().any(|i| shown(*i)))
}

/// Style IDs are names with only ASCII letters and digits ("Normal (Zampanò)" → "NormalZampan").
fn id_for(name: &str, taken: &[Style]) -> String {
    let base: String = name.split(|c: char| !c.is_alphanumeric()).map(|w| {
        let w: String = w.chars().filter(char::is_ascii_alphanumeric).collect();
        let mut c = w.chars();
        c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
    }).collect();
    let base = if base.is_empty() { "Style".to_string() } else { base };
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|s| s.id == id) {
        id = format!("{base}{n}");
        n += 1;
    }
    id
}

pub(crate) fn lay_out(units: &[Unit], targets: &[Target], setup: &Setup) -> Laid {
    // The first target at each heading level is Word's built-in "heading n"; any other at that
    // level is a style of its own based on it, so it keeps its look and its place in the outline.
    let primary = |t: usize| match targets[t].role {
        Role::Heading(n) => targets.iter().position(|o| o.role == Role::Heading(n)).filter(|p| *p != t),
        _ => None,
    };
    let kept: Vec<Kept> = units.iter().enumerate()
        .filter(|(_, u)| u.target.is_none_or(|t| targets[t].role != Role::LeftOut))
        .map(|(index, unit)| Kept { unit, index, target: unit.target })
        .collect();

    // The flush variant: per target and scope, an indented style's units that are flush after
    // a given kind of predecessor.
    let mut flush_after: HashMap<(Option<usize>, Option<String>), Vec<Option<Option<usize>>>> = HashMap::new();
    {
        // Where each group's lines start, for one-line units, whose indent is only where they start.
        let mut lefts: HashMap<(Option<usize>, Option<String>), Vec<f64>> = HashMap::new();
        for k in &kept {
            if let Some(left) = k.unit.left_indent().filter(|_| !k.unit.continues) {
                lefts.entry((k.target, k.unit.scope.clone())).or_default().push(left);
            }
        }
        let lefts: HashMap<_, f64> = lefts.into_iter().filter_map(|(g, l)| mode(l.into_iter(), 0.1).map(|m| (g, m))).collect();
        let mut indents: HashMap<(Option<usize>, Option<String>), Vec<(Option<Option<usize>>, f64)>> = HashMap::new();
        for (i, k) in kept.iter().enumerate() {
            let left = lefts.get(&(k.target, k.unit.scope.clone())).copied();
            if let Some(indent) = indent_of(k.unit, left) {
                let before = i.checked_sub(1).map(|j| kept[j].target);
                indents.entry((k.target, k.unit.scope.clone())).or_default().push((before, indent));
            }
        }
        for (group, list) in indents {
            let indented = mode(list.iter().map(|(_, x)| *x), 0.1).is_some_and(|m| m > SAME);
            if !indented {
                continue;
            }
            let mut classes: Vec<Option<Option<usize>>> = list.iter().map(|(c, _)| *c).collect();
            classes.dedup();
            let flush: Vec<_> = classes.into_iter()
                .filter(|c| mode(list.iter().filter(|(x, _)| x == c).map(|(_, x)| *x), 0.1).is_some_and(|m| m.abs() < SAME))
                .collect();
            flush_after.insert(group, flush);
        }
    }

    // Each kept unit's key; a unit continuing a paragraph from the page before takes its key.
    let mut keys: Vec<Key> = Vec::with_capacity(kept.len());
    for (i, k) in kept.iter().enumerate() {
        if k.unit.continues && i > 0 {
            keys.push(keys[i - 1].clone());
            continue;
        }
        let before = i.checked_sub(1).map(|j| kept[j].target);
        let first = flush_after.get(&(k.target, k.unit.scope.clone())).is_some_and(|f| f.contains(&before));
        keys.push(Key { target: k.target, scope: k.unit.scope.clone(), first });
    }

    // The styles, Normal first, each after the style it's based on.
    let mut laid = Laid { styles: Vec::new(), paras: Vec::new(), notes: Vec::new(), plain: false };
    let name_of = |key: &Key| -> (String, Kind) {
        let (name, kind) = match key.target {
            None if key.first => ("First Paragraph".to_string(), Kind::Custom),
            None => ("Normal".to_string(), Kind::Normal),
            Some(t) => match targets[t].role {
                Role::Heading(n) if primary(t).is_none() => (format!("heading {n}"), Kind::Heading(n)),
                Role::Heading(n) => (targets[t].name.clone(), Kind::Heading(n)),
                _ if key.first => (format!("{}, first", targets[t].name), Kind::Custom),
                _ => (targets[t].name.clone(), Kind::Custom),
            },
        };
        match &key.scope {
            Some(scope) => (format!("{name} ({scope})"), if kind == Kind::Normal { Kind::Custom } else { kind }),
            None => (name, kind),
        }
    };
    fn style_for(laid: &mut Laid, key: &Key, name_of: &dyn Fn(&Key) -> (String, Kind), primary: &dyn Fn(usize) -> Option<usize>) -> usize {
        if let Some(i) = laid.styles.iter().position(|s| s.key == *key) {
            return i;
        }
        let normal = Key { target: None, scope: None, first: false };
        let heading = key.target.and_then(primary);
        let base = if key.scope.is_some() {
            Some(Key { scope: None, ..key.clone() })
        } else if key.first {
            Some(Key { first: false, ..key.clone() })
        } else if let Some(p) = heading {
            Some(Key { target: Some(p), scope: None, first: false })
        } else if *key != normal {
            Some(normal)
        } else {
            None
        };
        let based_on = base.map(|b| style_for(laid, &b, name_of, primary));
        let (name, kind) = name_of(key);
        let id = match kind {
            Kind::Normal => "Normal".to_string(),
            Kind::Heading(n) if key.scope.is_none() && heading.is_none() => format!("Heading{n}"),
            _ => id_for(&name, &laid.styles),
        };
        laid.styles.push(Style { id, name, kind, based_on, next: None, props: Props::default(), key: key.clone() });
        laid.styles.len() - 1
    }
    style_for(&mut laid, &Key { target: None, scope: None, first: false }, &name_of, &primary);
    let style_of: Vec<usize> = keys.iter().map(|k| style_for(&mut laid, k, &name_of, &primary)).collect();

    // What each style's units have in common (layer 1).
    for s in 0..laid.styles.len() {
        let mine: Vec<&Unit> = kept.iter().zip(&style_of).filter(|(_, x)| **x == s).map(|(k, _)| k.unit).collect();
        if mine.is_empty() {
            continue;
        }
        let mut props = measured(&mine, laid.styles[s].kind);
        let positions: Vec<usize> = (0..kept.len()).filter(|i| style_of[*i] == s && !kept[*i].unit.continues).collect();
        if !positions.is_empty() && start_pages(&kept, &positions) && laid.styles[s].kind != Kind::Normal {
            props.page_break = Some(true);
            // Down from the top of the body to the first line, as Word places a line: its
            // ascent, with the extra of a line spacing over single above the text.
            let line = props.line.unwrap_or(1.0);
            props.before = mode(positions.iter().map(|i| {
                let u = kept[*i].unit;
                u.from_top() + u.metrics.cap - u.metrics.ascent - (line - 1.0).max(0.0) * u.metrics.line()
            }), 0.5).map(|b| b.max(0.0));
        }
        laid.styles[s].props = props;
    }

    // Normal from the probe (layer 2), over what was measured; Word's defaults where neither says.
    {
        let n = &mut laid.styles[0].props;
        n.size = Some(setup.size);
        n.align = Some(if setup.justify { Align::Justified } else { n.align.filter(|a| *a != Align::Justified).unwrap_or(Align::Left) });
        n.first_line = Some(setup.first_line_indent);
        n.family.get_or_insert_with(|| "Times New Roman".into());
        n.italic.get_or_insert(false);
        n.bold.get_or_insert(false);
        n.fill.get_or_insert_with(|| "000000".into());
        n.left = Some(n.left.unwrap_or(0.0));
        n.right = Some(n.right.unwrap_or(0.0));
        n.line.get_or_insert(1.0);
    }

    // The space before and after each style, fitted to the gaps measured between neighbours.
    let line_of = |laid: &Laid, s: usize| laid.resolved(s).line.unwrap_or(1.0);
    let pairs: Vec<(usize, usize, usize, f64)> = (1..kept.len()).filter_map(|i| {
        let (a, b) = (kept[i - 1].unit, kept[i].unit);
        if kept[i].index != kept[i - 1].index + 1 || b.continues || a.page != b.page {
            return None;
        }
        let gap = b.gap_before?;
        let natural = a.metrics.descent + b.metrics.ascent + (line_of(&laid, style_of[i]) - 1.0).max(0.0) * b.metrics.line();
        Some((i, style_of[i - 1], style_of[i], gap - natural))
    }).collect();
    let n = laid.styles.len();
    let fixed: Vec<Option<f64>> = (0..n).map(|s| laid.styles[s].props.before.filter(|_| laid.styles[s].props.page_break == Some(true))).collect();
    let misfits = |before: &[f64], after: &[f64]| pairs.iter().filter(|(_, a, b, x)| (after[*a] + before[*b] - x).abs() > SAME).count();
    let (mut before, mut after) = (vec![0.0; n], vec![0.0; n]);
    let mut best = (misfits(&before, &after), before.clone(), after.clone());
    for _ in 0..8 {
        for s in 0..n {
            if let Some(f) = fixed[s] {
                before[s] = f;
            } else if let Some(m) = mode(pairs.iter().filter(|p| p.2 == s).map(|p| p.3 - after[p.1]), 0.5) {
                before[s] = m.max(0.0);
            }
        }
        for s in 0..n {
            if let Some(m) = mode(pairs.iter().filter(|p| p.1 == s).map(|p| p.3 - before[p.2]), 0.5) {
                after[s] = m.max(0.0);
            }
        }
        let now = misfits(&before, &after);
        if now < best.0 {
            best = (now, before.clone(), after.clone());
        }
    }
    let (_, before, after) = best;
    for s in 0..n {
        laid.styles[s].props.before = Some(before[s]);
        laid.styles[s].props.after = Some(after[s]);
    }

    // What's typed after each style in Word: what most often followed it (Word's default is
    // the same style again, so that's left unsaid).
    for s in 0..n {
        let followers = (1..kept.len()).filter(|i| style_of[i - 1] == s).map(|i| style_of[i]);
        laid.styles[s].next = mode_of(followers).filter(|f| *f != s);
    }

    // The paragraphs, with what their style doesn't explain.
    let resolved: Vec<Props> = (0..n).map(|s| laid.resolved(s)).collect();
    let mut direct_count = vec![0usize; n];
    let misfit: HashMap<usize, f64> = pairs.iter()
        .filter(|(_, a, b, x)| (after[*a] + before[*b] - x).abs() > SAME)
        .map(|(i, a, _, x)| (*i, (x - after[*a]).max(0.0)))
        .collect();
    for (i, k) in kept.iter().enumerate() {
        let u = k.unit;
        if u.continues && let Some(para) = laid.paras.last_mut() {
            // The rest of the paragraph from the page before: join the line it was cut at.
            let joins = kept[i - 1].unit.ends_hyphenated || para.runs.last().is_some_and(|r| crate::measure::wraps_after(&r.text));
            if !joins {
                para.runs.push(Run { text: "\n".into(), ..u.runs.first().cloned().unwrap_or_else(|| para.runs[0].clone()) });
            }
            para.runs.extend(u.runs.iter().cloned());
            continue;
        }
        let s = style_of[i];
        let r = &resolved[s];
        let mut direct = Props::default();
        let forced = u.runs.iter().any(|r| r.text == "\n");
        let a = u.align();
        let loose = |x: Align| matches!(x, Align::Left | Align::Justified);
        if a != Align::Unknown && Some(a) != r.align && !(loose(a) && r.align.is_none_or(loose) && (forced || u.lines.len() < 3)) {
            direct.align = Some(a);
        }
        let differs = |m: Option<f64>, s: Option<f64>| m.is_some_and(|m| (m - s.unwrap_or(0.0)).abs() > SAME);
        let indent = indent_of(u, r.left.or(Some(0.0)));
        if r.align != Some(Align::Centre) && differs(indent, r.first_line) {
            direct.first_line = indent;
        }
        if differs(u.left_indent(), r.left) {
            direct.left = u.left_indent();
        }
        if let Some(b) = misfit.get(&i) {
            direct.before = Some(*b);
        }
        if direct != Props::default() {
            direct_count[s] += 1;
        }
        laid.paras.push(Para { style: s, runs: u.runs.clone(), direct });
    }
    for (s, count) in direct_count.into_iter().enumerate() {
        if count > 0 {
            let origin = laid.styles[s].key.target.and_then(|t| targets[t].origin.clone());
            laid.notes.push(Note { style: Some(laid.styles[s].name.clone()), origin, what: What::FormattedDirectly { paragraphs: count } });
        }
    }
    laid
}

/// Keeps only what the text itself says, in plain styles: a book draft in Times New Roman 12 pt,
/// single-spaced, indented, with black bold headings. Every target keeps its named style (with
/// nothing of its own, so an editor can still find every epigraph); emphasis is what differs
/// from the paragraph as it was set, so `_emph_` in an italic epigraph, set upright there, is
/// italic here.
pub(crate) fn make_plain(laid: &mut Laid) {
    let measured: Vec<Props> = (0..laid.styles.len()).map(|s| laid.resolved(s)).collect();
    for p in &mut laid.paras {
        let m = &measured[p.style];
        for r in &mut p.runs {
            r.italic = r.italic != m.italic.unwrap_or(false);
            r.bold = r.bold != m.bold.unwrap_or(false);
        }
        p.direct = Props::default();
    }
    laid.notes.retain(|n| !matches!(n.what, What::FormattedDirectly { .. }));
    for s in &mut laid.styles {
        s.props = match s.kind {
            Kind::Normal => Props {
                family: Some("Times New Roman".into()), size: Some(12.0), italic: Some(false), bold: Some(false),
                fill: Some("000000".into()), align: Some(Align::Left), left: Some(0.0), right: Some(0.0),
                first_line: Some(36.0), line: Some(1.0), before: Some(0.0), after: Some(0.0),
                page_break: None, keep_next: None,
            },
            Kind::Heading(n) => Props {
                size: Some(match n { 1 => 16.0, 2 => 14.0, _ => 12.0 }),
                bold: Some(true),
                first_line: Some(0.0),
                before: Some(if n == 1 { 24.0 } else { 12.0 }),
                after: Some(6.0),
                keep_next: Some(true),
                ..Props::default()
            },
            Kind::Custom if s.key.first => Props { first_line: Some(0.0), ..Props::default() },
            Kind::Custom => Props::default(),
        };
    }
    laid.plain = true;
}
