//! The first, simple implementation of the History engine, frozen as a reference.
//!
//! It recomputes everything from scratch after every Change, keeps one hash table entry per
//! character, and was checked against 100,000+ random three-Device histories. The fast
//! implementation in `fugue`, `layout` and `doc` must give exactly the same results for every
//! action; `tests/against_reference.rs` checks that. Don't optimize this module: being slow and
//! obviously right is its job.
#![allow(dead_code)]

pub mod fugue;
pub mod layout;
pub mod doc;
