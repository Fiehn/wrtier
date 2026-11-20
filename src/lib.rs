pub mod ids;
pub mod change;
pub mod fugue;
pub mod doc;
pub mod storage;
pub mod layout;
pub mod text_file;
pub mod conflicts;
pub mod footnotes;
pub mod images;
#[doc(hidden)]
pub mod reference; // the slow, simple version the fast one is tested against