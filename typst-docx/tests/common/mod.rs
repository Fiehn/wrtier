//! What the spikes share: a `World` with Typst's bundled fonts and one in-memory file.

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_kit::fonts::FontStore;
use typst_layout::PagedDocument;

struct TestWorld {
    library: LazyHash<Library>,
    fonts: FontStore,
    main: Source,
}

impl World for TestWorld {
    fn library(&self) -> &LazyHash<Library> { &self.library }
    fn book(&self) -> &LazyHash<FontBook> { self.fonts.book() }
    fn main(&self) -> FileId { self.main.id() }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() { Ok(self.main.clone()) } else { Err(FileError::AccessDenied) }
    }
    fn file(&self, _id: FileId) -> FileResult<Bytes> { Err(FileError::AccessDenied) }
    fn font(&self, index: usize) -> Option<Font> { self.fonts.font(index) }
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> { None }
}

/// Compiles `text`, or says why it doesn't.
#[allow(dead_code)]
pub fn try_compile(text: &str) -> Result<PagedDocument, Vec<String>> {
    let mut fonts = FontStore::new();
    for (font, info) in typst_kit::fonts::embedded() {
        fonts.push((font, info));
    }
    let id = FileId::new(RootedPath::new(VirtualRoot::Project, VirtualPath::new("main.typ").unwrap()));
    let world = TestWorld { library: LazyHash::new(Library::default()), fonts, main: Source::new(id, text.into()) };
    typst::compile::<PagedDocument>(&world).output.map_err(|errors| errors.iter().map(|e| e.message.to_string()).collect())
}

#[allow(dead_code)]
pub fn compile(text: &str) -> PagedDocument {
    try_compile(text).unwrap_or_else(|e| panic!("doesn't compile: {e:?}"))
}
