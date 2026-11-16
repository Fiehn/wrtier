//! Fonts to embed in the `.docx`.
//!
//! Word only uses an embedded font with TrueType outlines (the font spike: it ignored
//! Libertinus as Typst bundles it, with CFF outlines, and drew Calibri). So a CFF face is
//! converted first, as fontTools' `otf2ttf` does: each glyph's cubic curves become quadratic
//! ones within a font unit, its contours are reversed (TrueType's outer contours run clockwise),
//! and `glyf`/`loca` replace `CFF `. Embedding is only for faces whose licence (`OS/2` fsType)
//! allows editing; the font is embedded whole, not subset, so text typed in Word can use it.

use kurbo::{BezPath, PathEl, Point};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{GlyphId, MetadataProvider};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::read::{FontRef, TableProvider};
use write_fonts::tables::glyf::{GlyfLocaBuilder, Glyph, SimpleGlyph};
use write_fonts::tables::head::Head;
use write_fonts::tables::maxp::Maxp;
use write_fonts::types::Tag;
use write_fonts::FontBuilder;

/// How far a quadratic curve may stray from the cubic it replaces, in font units.
const ACCURACY: f64 = 1.0;

/// Why a face can't be embedded.
#[derive(Clone, Debug, PartialEq)]
pub enum NotEmbedded {
    /// Its licence doesn't allow embedding for editing (`OS/2` fsType).
    Licence,
    /// Its outlines couldn't be made TrueType ones (the reason).
    Unconvertible(String),
}

/// The face's licence allows embedding it in a document that can be edited: installable (no
/// bits set) or editable, and not restricted, preview-and-print only or bitmap only.
fn may_embed(font: &FontRef) -> bool {
    let Ok(os2) = font.os2() else { return true };
    let fs_type = os2.fs_type();
    const RESTRICTED: u16 = 0x0002;
    const PREVIEW_AND_PRINT: u16 = 0x0004;
    const BITMAP_ONLY: u16 = 0x0200;
    fs_type & (RESTRICTED | PREVIEW_AND_PRINT | BITMAP_ONLY) == 0
}

/// A pen that collects a glyph's outline.
#[derive(Default)]
struct Collect(BezPath);

impl OutlinePen for Collect {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to((f64::from(cx0), f64::from(cy0)), (f64::from(x), f64::from(y)));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to((f64::from(cx0), f64::from(cy0)), (f64::from(cx1), f64::from(cy1)), (f64::from(x), f64::from(y)));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

/// `path` with its cubic curves as quadratic ones.
fn quadratic(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut at = Point::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::CurveTo(c0, c1, end) => {
                for (_, _, quad) in kurbo::CubicBez::new(at, c0, c1, end).to_quads(ACCURACY) {
                    out.quad_to(quad.p1, quad.p2);
                }
                at = end;
            }
            PathEl::MoveTo(p) => {
                out.move_to(p);
                at = p;
            }
            PathEl::LineTo(p) => {
                out.line_to(p);
                at = p;
            }
            PathEl::QuadTo(c, p) => {
                out.quad_to(c, p);
                at = p;
            }
            PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

/// A face (`index` in `data`, which may be a collection) as a TrueType font Word can embed.
pub fn embeddable(data: &[u8], index: u32) -> Result<Vec<u8>, NotEmbedded> {
    let unconvertible = |e: &dyn std::fmt::Display| NotEmbedded::Unconvertible(e.to_string());
    let font = FontRef::from_index(data, index).map_err(|e| unconvertible(&e))?;
    if !may_embed(&font) {
        return Err(NotEmbedded::Licence);
    }
    let mut builder = FontBuilder::new();
    if font.table_data(Tag::new(b"glyf")).is_none() {
        if font.table_data(Tag::new(b"CFF2")).is_some() {
            return Err(NotEmbedded::Unconvertible("its outlines are variable (CFF2)".into()));
        }
        if font.table_data(Tag::new(b"CFF ")).is_none() {
            return Err(NotEmbedded::Unconvertible("it has no outlines Word can use".into()));
        }
        let glyphs = font.maxp().map_err(|e| unconvertible(&e))?.num_glyphs();
        let outlines = font.outline_glyphs();
        let mut glyf = GlyfLocaBuilder::new();
        let (mut max_points, mut max_contours) = (0u16, 0u16);
        for gid in 0..glyphs {
            let mut pen = Collect::default();
            if let Some(outline) = outlines.get(GlyphId::new(u32::from(gid))) {
                outline.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen)
                    .map_err(|e| unconvertible(&format!("glyph {gid}: {e}")))?;
            }
            let glyph = if pen.0.elements().is_empty() {
                Glyph::Empty
            } else {
                let path = quadratic(&pen.0).reverse_subpaths();
                let simple = SimpleGlyph::from_bezpath(&path).map_err(|e| unconvertible(&format!("glyph {gid}: {e:?}")))?;
                max_contours = max_contours.max(simple.contours.len() as u16);
                max_points = max_points.max(simple.contours.iter().map(|c| c.len()).sum::<usize>() as u16);
                Glyph::Simple(simple)
            };
            glyf.add_glyph(&glyph).map_err(|e| unconvertible(&format!("glyph {gid}: {e:?}")))?;
        }
        let (glyf, loca, format) = glyf.build();
        let mut head: Head = font.head().map_err(|e| unconvertible(&e))?.to_owned_table();
        head.index_to_loc_format = format as i16;
        let maxp = Maxp {
            num_glyphs: glyphs,
            max_points: Some(max_points),
            max_contours: Some(max_contours),
            max_composite_points: Some(0),
            max_composite_contours: Some(0),
            max_zones: Some(1),
            max_twilight_points: Some(0),
            max_storage: Some(0),
            max_function_defs: Some(0),
            max_instruction_defs: Some(0),
            max_stack_elements: Some(0),
            max_size_of_instructions: Some(0),
            max_component_elements: Some(0),
            max_component_depth: Some(0),
        };
        let fail = |e: write_fonts::BuilderError| NotEmbedded::Unconvertible(e.to_string());
        builder.add_table(&glyf).map_err(fail)?;
        builder.add_table(&loca).map_err(fail)?;
        builder.add_table(&head).map_err(fail)?;
        builder.add_table(&maxp).map_err(fail)?;
        // What only CFF outlines need goes; everything else is kept as it is.
        let cff_only = [Tag::new(b"CFF "), Tag::new(b"VORG")];
        for record in font.table_directory().table_records() {
            let tag = record.tag();
            if !cff_only.contains(&tag) && !builder.contains(tag) && let Some(data) = font.data_for_tag(tag) {
                builder.add_raw(tag, data.as_bytes());
            }
        }
        return Ok(builder.build());
    }
    // TrueType already: the face on its own (out of a collection, if it was in one).
    builder.copy_missing_tables(font);
    Ok(builder.build())
}

/// Which of a family's four faces Word knows a font as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Slot {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Slot {
    /// Its element in `fontTable.xml`.
    pub fn element(self) -> &'static str {
        match self {
            Slot::Regular => "embedRegular",
            Slot::Bold => "embedBold",
            Slot::Italic => "embedItalic",
            Slot::BoldItalic => "embedBoldItalic",
        }
    }
}

/// A family the document uses, and its faces that are embedded: each obfuscated, with its key.
pub(crate) struct Family {
    pub name: String,
    pub faces: Vec<(Slot, Vec<u8>, String)>,
}

/// Every face the document's text is set in (its body, headers and footers), by family and
/// slot; of several faces in one slot (a Light and a Regular), the one nearest regular weight.
fn used(document: &typst_layout::PagedDocument) -> Vec<(String, Slot, typst::text::Font)> {
    fn walk(frame: &typst::layout::Frame, out: &mut Vec<typst::text::Font>) {
        for (_, item) in frame.items() {
            match item {
                typst::layout::FrameItem::Group(g) => walk(&g.frame, out),
                typst::layout::FrameItem::Text(t) if !out.contains(t.font.font()) => out.push(t.font.font().clone()),
                _ => {}
            }
        }
    }
    let mut fonts = Vec::new();
    for page in document.pages() {
        walk(&page.frame, &mut fonts);
    }
    let mut chosen: Vec<(String, Slot, typst::text::Font)> = Vec::new();
    for font in fonts {
        let info = font.info();
        let bold = info.variant.weight.to_number() >= 600;
        let italic = info.variant.style != typst::text::FontStyle::Normal;
        let slot = match (bold, italic) {
            (false, false) => Slot::Regular,
            (true, false) => Slot::Bold,
            (false, true) => Slot::Italic,
            (true, true) => Slot::BoldItalic,
        };
        let nearness = |f: &typst::text::Font| {
            let w = i32::from(f.info().variant.weight.to_number());
            (w - if bold { 700 } else { 400 }).abs()
        };
        match chosen.iter_mut().find(|(family, s, _)| *family == info.family && *s == slot) {
            Some(entry) if nearness(&font) < nearness(&entry.2) => entry.2 = font.clone(),
            Some(_) => {}
            None => chosen.push((info.family.clone(), slot, font.clone())),
        }
    }
    chosen.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    chosen
}

/// A GUID for an embedded font's key, the same for the same seed and face.
fn key(seed: u64, n: u64) -> String {
    let mix = |mut z: u64| {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let (a, b) = (mix(seed ^ n.wrapping_mul(2)), mix(seed ^ n.wrapping_mul(2).wrapping_add(1)));
    format!("{{{:08X}-{:04X}-{:04X}-{:04X}-{:012X}}}", a >> 32, (a >> 16) & 0xFFFF, a & 0xFFFF, b >> 48, b & 0xFFFF_FFFF_FFFF)
}

/// The document's font families, with the faces that can be embedded, ready to embed; and what
/// couldn't be (family, why).
pub(crate) fn collect(document: &typst_layout::PagedDocument, seed: u64) -> (Vec<Family>, Vec<(String, NotEmbedded)>) {
    let mut families: Vec<Family> = Vec::new();
    let mut not = Vec::new();
    for (n, (name, slot, font)) in used(document).into_iter().enumerate() {
        if !families.iter().any(|f| f.name == name) {
            families.push(Family { name: name.clone(), faces: Vec::new() });
        }
        match embeddable(font.data().as_slice(), font.index()) {
            Ok(ttf) => {
                let guid = key(seed, n as u64);
                let family = families.iter_mut().find(|f| f.name == name).expect("added above");
                family.faces.push((slot, obfuscate(&ttf, &guid), guid));
            }
            Err(why) => {
                if !not.iter().any(|(f, w)| *f == name && *w == why) {
                    not.push((name, why));
                }
            }
        }
    }
    (families, not)
}

/// The glyphs a font's small-caps features (`smcp`, `c2sc`) put in place of letters: how small
/// caps are told apart in a laid-out text, which keeps no trace of the feature itself.
fn small_cap_glyphs(font: &typst::text::Font) -> std::collections::HashSet<u16> {
    use write_fonts::read::tables::gsub::{SingleSubst, SubstitutionSubtables};
    let mut out = std::collections::HashSet::new();
    let Ok(f) = FontRef::from_index(font.data().as_slice(), font.index()) else { return out };
    let Ok(gsub) = f.gsub() else { return out };
    let (Ok(features), Ok(lookups)) = (gsub.feature_list(), gsub.lookup_list()) else { return out };
    for record in features.feature_records() {
        if ![Tag::new(b"smcp"), Tag::new(b"c2sc")].contains(&record.feature_tag()) {
            continue;
        }
        let Ok(feature) = record.feature(features.offset_data()) else { continue };
        for index in feature.lookup_list_indices() {
            let Ok(lookup) = lookups.lookups().get(usize::from(index.get())) else { continue };
            // One glyph for another, or for a sequence (a font may write its one-for-one
            // substitutions that way), or one of several alternates.
            match lookup.subtables() {
                Ok(SubstitutionSubtables::Single(subtables)) => {
                    for subtable in subtables.iter().flatten() {
                        match subtable {
                            SingleSubst::Format1(s) => {
                                let delta = i32::from(s.delta_glyph_id());
                                for gid in s.coverage().into_iter().flat_map(|c| c.iter().collect::<Vec<_>>()) {
                                    out.insert(((i32::from(gid.to_u16()) + delta).rem_euclid(65536)) as u16);
                                }
                            }
                            SingleSubst::Format2(s) => out.extend(s.substitute_glyph_ids().iter().map(|g| g.get().to_u16())),
                        }
                    }
                }
                Ok(SubstitutionSubtables::Multiple(subtables)) => {
                    for subtable in subtables.iter().flatten() {
                        for sequence in subtable.sequences().iter().flatten() {
                            out.extend(sequence.substitute_glyph_ids().iter().map(|g| g.get().to_u16()));
                        }
                    }
                }
                Ok(SubstitutionSubtables::Alternate(subtables)) => {
                    for subtable in subtables.iter().flatten() {
                        for set in subtable.alternate_sets().iter().flatten() {
                            out.extend(set.alternate_glyph_ids().iter().map(|g| g.get().to_u16()));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Which text is set in small caps, by its glyphs, remembering each font's small caps.
#[derive(Default)]
pub(crate) struct SmallCaps(std::collections::HashMap<typst::text::Font, std::collections::HashSet<u16>>);

impl SmallCaps {
    pub fn of(&mut self, item: &typst::text::TextItem) -> bool {
        let font = item.font.font();
        let glyphs = self.0.entry(font.clone()).or_insert_with(|| small_cap_glyphs(font));
        !glyphs.is_empty() && item.glyphs.iter().any(|g| glyphs.contains(&g.id))
    }
}

/// Word's obfuscation of an embedded font (ECMA-376 Part 1, 17.8.1): the first 32 bytes XORed
/// with the 16 bytes of the `w:fontKey` GUID read from its last hex pair to its first. Doing it
/// twice gives the font back.
pub fn obfuscate(font: &[u8], guid: &str) -> Vec<u8> {
    let hex: String = guid.chars().filter(char::is_ascii_hexdigit).collect();
    let mut key: Vec<u8> = (0..16).filter_map(|i| u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()).collect();
    key.reverse();
    let mut out = font.to_vec();
    if key.len() == 16 {
        for (i, byte) in out.iter_mut().take(32).enumerate() {
            *byte ^= key[i % 16];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A regular face of one of Typst's bundled fonts.
    fn bundled(family: &str, italic: bool) -> (Vec<u8>, u32) {
        let mut store = typst_kit::fonts::FontStore::new();
        for (font, info) in typst_kit::fonts::embedded() {
            store.push((font, info));
        }
        let book = store.book();
        let index = (0..).map_while(|i| book.info(i).map(|f| (i, f)))
            .find(|(_, f)| f.family == family && (f.variant.style != typst::text::FontStyle::Normal) == italic && f.variant.weight.to_number() == 400)
            .map(|(i, _)| i)
            .unwrap_or_else(|| panic!("{family} isn't bundled"));
        let font = store.font(index).unwrap();
        (font.data().as_slice().to_vec(), font.index())
    }

    #[test]
    fn a_cff_face_becomes_truetype_with_the_same_glyphs() {
        let (data, index) = bundled("Libertinus Serif", false);
        let original = FontRef::from_index(&data, index).unwrap();
        assert!(original.table_data(Tag::new(b"CFF ")).is_some(), "Typst bundles it with CFF outlines");
        let ttf = embeddable(&data, index).unwrap();
        let converted = FontRef::new(&ttf).unwrap();
        assert!(converted.table_data(Tag::new(b"CFF ")).is_none() && converted.table_data(Tag::new(b"glyf")).is_some());
        assert_eq!(converted.maxp().unwrap().num_glyphs(), original.maxp().unwrap().num_glyphs());
        assert_eq!(converted.cmap().unwrap().map_codepoint('Æ'), original.cmap().unwrap().map_codepoint('Æ'));
        // The same shape: a glyph's bounds within a unit or two.
        let bounds = |font: &FontRef, c: char| {
            let gid = font.cmap().unwrap().map_codepoint(c).unwrap();
            let mut pen = Collect::default();
            font.outline_glyphs().get(gid).unwrap().draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).unwrap();
            kurbo::Shape::bounding_box(&pen.0)
        };
        for c in ['a', 'g', 'Ø', '&'] {
            let (a, b) = (bounds(&original, c), bounds(&converted, c));
            for (x, y) in [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)] {
                assert!((x - y).abs() <= 2.0, "{c}: {a:?} against {b:?}");
            }
        }
        // The same every time.
        assert_eq!(embeddable(&data, index).unwrap(), ttf);
    }

    #[test]
    fn a_truetype_face_is_kept() {
        let (data, index) = bundled("DejaVu Sans Mono", false);
        let ttf = embeddable(&data, index).unwrap();
        assert!(FontRef::new(&ttf).unwrap().table_data(Tag::new(b"glyf")).is_some());
    }

    #[test]
    fn obfuscating_twice_gives_the_font_back() {
        let font: Vec<u8> = (0..100u8).collect();
        let guid = "{8A5B3C11-0D2E-4F60-9A71-1B2C3D4E5F01}";
        let once = obfuscate(&font, guid);
        assert_eq!(once[0], font[0] ^ 0x01);
        assert_eq!(once[15], font[15] ^ 0x8A);
        assert_eq!(once[32..], font[32..]);
        assert_eq!(obfuscate(&once, guid), font);
    }
}

