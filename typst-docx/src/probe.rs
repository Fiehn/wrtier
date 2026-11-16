//! The page and paragraph settings, as Typst resolved them.
//!
//! Formatting set with `set page/text/par` can't be read reliably off a Style's source (an
//! expression, a variable, `auto`), and the laid-out pages don't keep it all (margins, whether
//! text is hyphenated). So the caller puts `snippet()` into the document after its own `set`
//! rules, where Typst evaluates it in `context` and leaves the resolved values in a `metadata`
//! element, which is invisible and moves nothing. This reads them back.

use typst::foundations::{Dict, Label, Value};
use typst::introspection::{Introspector, MetadataElem};
use typst::layout::Abs;
use typst::utils::PicoStr;
use typst_layout::PagedDocument;

use crate::measure::Area;

/// The label the probe's `metadata` carries.
pub const LABEL: &str = "typst-docx-probe";

/// What the caller puts into its document, after the `set` rules that apply to its text and
/// before the text: a line of Typst.
pub fn snippet() -> String {
    format!(
        "#context [#metadata((width: page.width, height: page.height, flipped: page.flipped, \
         margin: page.margin, binding: page.binding, size: text.size, lang: text.lang, \
         region: text.region, hyphenate: text.hyphenate, justify: par.justify, \
         indent: par.first-line-indent))<{LABEL}>]\n"
    )
}

/// Page margins in points. With two-sided margins, `left` is the inside on the right-hand
/// (odd) pages and `right` the outside, as Word's mirrored margins are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Margins {
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
    /// Inside and outside rather than left and right.
    pub mirrored: bool,
}

/// The settings that apply to the document's text.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    /// Page size in points.
    pub width: f64,
    pub height: f64,
    pub margins: Margins,
    /// The binding is on the left (the inside margin is on the left of odd pages).
    pub binding_left: bool,
    /// The text's size in points.
    pub size: f64,
    /// E.g. "en", "da".
    pub lang: String,
    /// E.g. "GB", if one is set.
    pub region: Option<String>,
    pub hyphenate: bool,
    pub justify: bool,
    /// In points.
    pub first_line_indent: f64,
    /// The first paragraph after a heading or block is indented too.
    pub indent_all: bool,
}

impl Setup {
    /// The body area of a page, by its index from 0.
    pub fn area(&self, page: usize) -> Area {
        let m = &self.margins;
        // Page 1 is a right-hand page: with the binding on the left, its inside is on the left.
        let inside_left = self.binding_left == (page % 2 == 0);
        let (left, right) = if !m.mirrored || inside_left { (m.left, m.right) } else { (m.right, m.left) };
        Area { left, top: m.top, right: self.width - right, bottom: self.height - m.bottom }
    }

    /// The text's language for Word, such as "en-GB".
    pub fn language(&self) -> String {
        match &self.region {
            Some(r) => format!("{}-{r}", self.lang),
            None => self.lang.clone(),
        }
    }
}

fn field<'a>(dict: &'a Dict, key: &str) -> Result<&'a Value, String> {
    dict.get(key).map_err(|_| format!("the probe has no {key}"))
}

/// A length in points; `em` resolves against `size`.
fn points(value: &Value, size: f64) -> Option<f64> {
    match value {
        Value::Length(l) => Some(l.abs.to_pt() + l.em.get() * size),
        _ => None,
    }
}

/// A margin in points: `auto` is Typst's default, a ratio is of the page's side.
fn margin(value: Option<&Value>, whole: f64, default: f64, size: f64) -> f64 {
    match value {
        Some(Value::Length(l)) => l.abs.to_pt() + l.em.get() * size,
        Some(Value::Ratio(r)) => r.get() * whole,
        Some(Value::Relative(r)) => r.rel.get() * whole + r.abs.abs.to_pt() + r.abs.em.get() * size,
        _ => default,
    }
}

/// Reads the probe's values from the compiled document; `None` if it has no probe.
pub fn read(document: &PagedDocument) -> Option<Result<Setup, String>> {
    let label = Label::new(PicoStr::intern(LABEL))?;
    let found = document.introspector().query(&typst::foundations::Selector::Label(label));
    let first = found.first()?.to_packed::<MetadataElem>()?;
    let Value::Dict(dict) = &first.value else {
        return Some(Err("the probe isn't a dictionary".into()));
    };
    Some(setup(dict, document))
}

fn setup(dict: &Dict, document: &PagedDocument) -> Result<Setup, String> {
    let size = points(field(dict, "size")?, 0.0).unwrap_or(11.0);
    // An `auto` side grows with the content; the first page shows how big it came out.
    let page = document.pages().first().map(|p| p.frame.size());
    let mut width = points(field(dict, "width")?, size).or(page.map(|s| s.x.to_pt())).unwrap_or(Abs::inf().to_pt());
    let mut height = points(field(dict, "height")?, size).or(page.map(|s| s.y.to_pt())).unwrap_or(Abs::inf().to_pt());
    if matches!(field(dict, "flipped")?, Value::Bool(true)) {
        std::mem::swap(&mut width, &mut height);
    }
    let shorter = if width.min(height).is_finite() { width.min(height) } else { 595.28 };
    let default = 2.5 / 21.0 * shorter;
    let sides = match field(dict, "margin")? {
        Value::Dict(d) => Some(d.clone()),
        _ => None,
    };
    let side = |key: &str| sides.as_ref().and_then(|d| d.get(key).ok());
    let (mirrored, left_key, right_key) = if side("inside").is_some() || side("outside").is_some() {
        (true, "inside", "outside")
    } else {
        (false, "left", "right")
    };
    // `page.margin` itself, when it isn't a dictionary, applies to every side.
    let all = match field(dict, "margin")? {
        v @ (Value::Length(_) | Value::Ratio(_) | Value::Relative(_)) => Some(v),
        _ => None,
    };
    let get = |key: &str, whole: f64| margin(side(key).or(all), whole, default, size);
    let margins = Margins {
        top: get("top", height),
        bottom: get("bottom", height),
        left: get(left_key, width),
        right: get(right_key, width),
        mirrored,
    };
    let binding_left = match field(dict, "binding")? {
        Value::Auto => true, // left to right text
        other => !format!("{other:?}").to_lowercase().contains("right"),
    };
    let text = |key: &str| -> Result<Option<String>, String> {
        Ok(match field(dict, key)? {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        })
    };
    let justify = matches!(field(dict, "justify")?, Value::Bool(true));
    let hyphenate = match field(dict, "hyphenate")? {
        Value::Bool(b) => *b,
        _ => justify, // auto: hyphenated when justified
    };
    let (first_line_indent, indent_all) = match field(dict, "indent")? {
        Value::Dict(d) => (
            d.get("amount").ok().and_then(|v| points(v, size)).unwrap_or(0.0),
            matches!(d.get("all"), Ok(Value::Bool(true))),
        ),
        other => (points(other, size).unwrap_or(0.0), false),
    };
    Ok(Setup {
        width, height, margins, binding_left, size,
        lang: text("lang")?.unwrap_or_else(|| "en".into()),
        region: text("region")?,
        hyphenate, justify, first_line_indent, indent_all,
    })
}
