//! Spike: does Word use a CFF-outline `.otf` font (Libertinus Serif, as Typst bundles it)
//! embedded in a `.docx`, on a PC that doesn't have it installed?
//! Writes `fonts-embedded.docx` to the folder in SPIKE_DOCX_DIR:
//! `SPIKE_DOCX_DIR=… cargo test -p typst-docx --test font_spike -- --ignored --nocapture`
//! Libertinus Serif's four faces are embedded whole (not subset); a control paragraph uses New
//! Computer Modern, referenced but not embedded, so Word must substitute it.

use std::io::Write;
use typst::text::{FontStyle, FontWeight};
use typst_kit::fonts::FontStore;
// Word XORs the first 32 bytes of the font file with the `w:fontKey` GUID (ECMA-376 Part 1, 17.8.1).
use typst_docx::fonts::obfuscate;

/// One face of Typst's bundled fonts, as the bytes of its file.
fn face(store: &FontStore, family: &str, style: FontStyle, weight: FontWeight) -> Vec<u8> {
    let book = store.book();
    let index = (0..).map_while(|i| book.info(i).map(|f| (i, f)))
        .find(|(_, f)| f.family == family && f.variant.style == style && f.variant.weight == weight)
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("{family} {style:?} {weight:?} isn't bundled"));
    store.font(index).unwrap().data().as_slice().to_vec()
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn para(text: &str, run_props: &str) -> String {
    format!("<w:p><w:r><w:rPr>{run_props}</w:rPr><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p>")
}

#[test]
#[ignore]
fn write_a_docx_with_embedded_fonts() {
    let dir = std::env::var("SPIKE_DOCX_DIR").expect("set SPIKE_DOCX_DIR to where it goes");
    let mut store = FontStore::new();
    for (font, info) in typst_kit::fonts::embedded() {
        store.push((font, info));
    }
    let family = "Libertinus Serif";
    // (relationship, fontTable element, style, weight, GUID): fixed keys, so the file is the same every time.
    let faces = [
        ("rIdF1", "embedRegular", FontStyle::Normal, FontWeight::REGULAR, "{8A5B3C11-0D2E-4F60-9A71-1B2C3D4E5F01}"),
        ("rIdF2", "embedBold", FontStyle::Normal, FontWeight::BOLD, "{8A5B3C11-0D2E-4F60-9A71-1B2C3D4E5F02}"),
        ("rIdF3", "embedItalic", FontStyle::Italic, FontWeight::REGULAR, "{8A5B3C11-0D2E-4F60-9A71-1B2C3D4E5F03}"),
        ("rIdF4", "embedBoldItalic", FontStyle::Italic, FontWeight::BOLD, "{8A5B3C11-0D2E-4F60-9A71-1B2C3D4E5F04}"),
    ];

    let body = [
        para("Libertinus Serif, embedded", "<w:b/><w:sz w:val=\"32\"/>"),
        para("Regular: The night is long that never finds the day. Æblegrød, smørrebrød &amp; søster. Ligatures: fi fl ffi. Numbers: 1234567890.", ""),
        para("Italic: She didn’t quite believe it, not yet.", "<w:i/>"),
        para("Bold: NOT YET.", "<w:b/>"),
        para("Bold italic: Come home.", "<w:b/><w:i/>"),
        para("Type a sentence on the next line and check it is Libertinus too:", ""),
        para("", ""),
        para("Control: this line names New Computer Modern, which is NOT embedded and NOT installed, so Word must show a substitute.",
            "<w:rFonts w:ascii=\"New Computer Modern\" w:hAnsi=\"New Computer Modern\"/>"),
    ].concat();
    let document = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>");
    let styles = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"{W}\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{family}\" w:hAnsi=\"{family}\" w:eastAsia=\"{family}\" w:cs=\"{family}\"/><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/><w:lang w:val=\"en-GB\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"300\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style></w:styles>");
    // Embed fonts, and keep them embedded when Word saves (not just the characters used).
    let settings = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:settings xmlns:w=\"{W}\"><w:embedTrueTypeFonts/></w:settings>");
    let embeds: String = faces.iter().map(|(rid, el, _, _, guid)| format!("<w:{el} r:id=\"{rid}\" w:fontKey=\"{guid}\"/>")).collect();
    let font_table = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:fonts xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:font w:name=\"{family}\"><w:family w:val=\"roman\"/><w:pitch w:val=\"variable\"/>{embeds}</w:font><w:font w:name=\"New Computer Modern\"><w:family w:val=\"roman\"/></w:font></w:fonts>");
    let font_rels: String = faces.iter().enumerate().map(|(i, (rid, ..))| format!("<Relationship Id=\"{rid}\" Type=\"{R}/font\" Target=\"fonts/font{}.odttf\"/>", i + 1)).collect();
    let parts: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"odttf\" ContentType=\"application/vnd.openxmlformats-officedocument.obfuscatedFont\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/><Override PartName=\"/word/fontTable.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml\"/></Types>".into()),
        ("_rels/.rels".into(), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".into()),
        ("word/_rels/document.xml.rels".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdStyles\" Type=\"{R}/styles\" Target=\"styles.xml\"/><Relationship Id=\"rIdSettings\" Type=\"{R}/settings\" Target=\"settings.xml\"/><Relationship Id=\"rIdFonts\" Type=\"{R}/fontTable\" Target=\"fontTable.xml\"/></Relationships>").into_bytes()),
        ("word/_rels/fontTable.xml.rels".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{font_rels}</Relationships>").into_bytes()),
        ("word/document.xml".into(), document.into_bytes()),
        ("word/styles.xml".into(), styles.into_bytes()),
        ("word/settings.xml".into(), settings.into_bytes()),
        ("word/fontTable.xml".into(), font_table.into_bytes()),
    ].into_iter().chain(faces.iter().enumerate().map(|(i, (_, _, style, weight, guid))| {
        (format!("word/fonts/font{}.odttf", i + 1), obfuscate(&face(&store, family, *style, *weight), guid))
    })).collect();

    let path = std::path::Path::new(&dir).join("fonts-embedded.docx");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in &parts {
        zip.start_file(name.as_str(), options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    let sizes: Vec<String> = parts.iter().filter(|(n, _)| n.ends_with(".odttf")).map(|(n, b)| format!("{n} {} KB", b.len() / 1024)).collect();
    println!("wrote {}\n{}", path.display(), sizes.join("\n"));
}

#[test]
fn obfuscating_twice_gives_the_font_back() {
    let font: Vec<u8> = (0..100u8).collect();
    let guid = "{8A5B3C11-0D2E-4F60-9A71-1B2C3D4E5F01}";
    let once = obfuscate(&font, guid);
    assert_ne!(once[..32], font[..32]);
    assert_eq!(once[32..], font[32..]);
    assert_eq!(obfuscate(&once, guid), font);
    // The key is the GUID's bytes from last to first: the first font byte meets 0x01.
    assert_eq!(once[0], font[0] ^ 0x01);
    assert_eq!(once[15], font[15] ^ 0x8A);
}
