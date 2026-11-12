//! Turns a compiled Typst document into an editable `.docx` whose formatting lives in named Word
//! styles, one per target (a label or a heading level), so whoever opens it can restyle every
//! epigraph, or every chapter heading, in one place.
//!
//! The caller compiles the document with its own `World`; this crate only reads the result.
//! It knows Typst and OOXML, nothing about the program that calls it.
//!
//! So far: `measure`, which reads what each target looks like from the laid-out pages.

pub mod measure;
