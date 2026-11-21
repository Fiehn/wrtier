//! Spike: which rule loads ~70 MB lazily. `cargo run --release --bin jump -- <novel.txt>`
use harper_core::linting::LintGroup;
use harper_core::parsers::PlainEnglish;
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            CURRENT.fetch_add(layout.size(), Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        CURRENT.fetch_sub(layout.size(), Relaxed);
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;
fn mb(b: isize) -> String {
    format!("{:.1} MB", b as f64 / 1_048_576.0)
}

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    let start = raw.find("*** START OF").and_then(|i| raw[i..].find('\n').map(|j| i + j + 1)).unwrap_or(0);
    let end = raw.find("*** END OF").unwrap_or(raw.len());
    let paragraphs: Vec<String> = raw[start..end]
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ").replace('_', ""))
        .filter(|p| !p.is_empty())
        .collect();
    let dict = FstDictionary::curated();
    let mut group = LintGroup::new_curated(dict.clone(), Dialect::British);
    // Find the paragraph where the heap jumps by more than 20 MB.
    let mut k = None;
    for (i, p) in paragraphs.iter().enumerate().take(200) {
        let before = CURRENT.load(Relaxed) as isize;
        let _ = group.organized_lints(&Document::new(p, &PlainEnglish, &*dict));
        let d = CURRENT.load(Relaxed) as isize - before;
        if d > 20 * 1_048_576 {
            println!("paragraph {i} grew the heap by {}: “{}…”", mb(d), p.chars().take(80).collect::<String>());
            k = Some(i);
            break;
        }
    }
    let Some(k) = k else { return println!("no jump found") };
    // In a fresh process state we can't redo it, so start again: new process run with JUMP_AT.
    println!("rerun with JUMP_AT={k}");
    if let Ok(at) = std::env::var("JUMP_AT") {
        let _ = at;
    }
}
