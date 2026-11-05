use std::path::Path;
use std::time::Instant;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_layout::PagedDocument;

/// Everything Typst needs to know about our "project": one in-memory main file and some fonts.
struct SpikeWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: Source,
}

impl World for SpikeWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }
    fn file(&self, _id: FileId) -> FileResult<Bytes> {
        Err(FileError::AccessDenied)
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

/// A generated novel we can compile, edit and recompile, timing each step.
pub struct Novel {
    world: SpikeWorld,
    edits: usize,
    font_count: usize,
    fonts_ms: u128,
}

impl Novel {
    /// Load fonts (from `fonts_dir` if given, plus Typst's built-in ones) and generate the text.
    pub fn new(fonts_dir: Option<&Path>) -> Result<Novel, String> {
        let start = Instant::now();
        let mut fonts: Vec<Font> = Vec::new();
        if let Some(dir) = fonts_dir {
            for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                if ext == "ttf" || ext == "otf" {
                    let data = std::fs::read(&path).map_err(|e| e.to_string())?;
                    fonts.extend(Font::iter(Bytes::new(data)));
                }
            }
        }
        for data in typst_assets::fonts() {
            fonts.extend(Font::iter(Bytes::new(data)));
        }
        let book = FontBook::from_fonts(&fonts);

        let id = FileId::unique(RootedPath::new(
            VirtualRoot::Project,
            VirtualPath::new("main.typ").map_err(|e| format!("{e:?}"))?,
        ));
        let main = Source::new(id, generate_novel(0));

        Ok(Novel {
            font_count: fonts.len(),
            world: SpikeWorld {
                library: LazyHash::new(Library::default()),
                book: LazyHash::new(book),
                fonts,
                main,
            },
            edits: 0,
            fonts_ms: start.elapsed().as_millis(),
        })
    }

    /// Change one word in the middle of the novel, like a small edit while writing.
    pub fn edit(&mut self) {
        self.edits += 1;
        self.world.main.replace(&generate_novel(self.edits));
    }

    /// Compile and report how long it took and how many pages came out.
    pub fn compile(&self) -> Result<String, String> {
        let start = Instant::now();
        let doc = compile(&self.world)?;
        // Forget cached work the last compile didn't use, so memory stays flat.
        comemo::evict(1);
        Ok(format!(
            "compile #{}: {} ms, {} pages ({} fonts loaded in {} ms)",
            self.edits,
            start.elapsed().as_millis(),
            doc.pages().len(),
            self.font_count,
            self.fonts_ms,
        ))
    }

    /// Time a full PDF export.
    pub fn pdf_ms(&self) -> Result<u128, String> {
        let doc = compile(&self.world)?;
        let start = Instant::now();
        typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default()).map_err(|e| format!("{e:?}"))?;
        Ok(start.elapsed().as_millis())
    }
}

fn compile(world: &SpikeWorld) -> Result<PagedDocument, String> {
    typst::compile::<PagedDocument>(world)
        .output
        .map_err(|errors| format!("{errors:?}"))
}

/// About 300 pages: 40 chapters of 30 paragraphs. `version` changes one word in chapter 20.
fn generate_novel(version: usize) -> String {
    let words = [
        "the", "rain", "had", "stopped", "and", "she", "walked", "down", "to", "harbour",
        "where", "boats", "lay", "quiet", "under", "a", "grey", "sky", "nobody", "spoke",
    ];
    let mut text = String::from("#set page(width: 13cm, height: 20cm)\n#set par(justify: true)\n\n");
    let mut n: usize = 7;
    for chapter in 1..=40 {
        text.push_str(&format!("= Chapter {chapter}\n\n"));
        for paragraph in 0..30 {
            for _ in 0..90 {
                n = (n * 31 + 17) % 9973;
                text.push_str(words[n % words.len()]);
                text.push(' ');
            }
            if chapter == 20 && paragraph == 0 {
                text.push_str(&format!("edited{version} "));
            }
            text.push_str("\n\n");
        }
    }
    text
}
