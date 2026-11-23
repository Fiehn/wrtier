//! Harper is used patched (ADR 0023): its dictionary kept once, not twice, and its rules' caches
//! kept small. The published crate fails this test, so a patch that silently stops applying (an
//! upgrade, a `[patch]` that no longer matches) fails it too.
//!
//! Its own test binary: every byte allocated is counted, which would slow every other test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use wrtier_app_lib::grammar::Grammar;

struct Counting;
static HEAP: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            HEAP.fetch_add(layout.size(), Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        HEAP.fetch_sub(layout.size(), Relaxed);
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

const MB: usize = 1024 * 1024;

fn heap() -> usize {
    HEAP.load(Relaxed)
}

#[test]
fn harper_stays_within_its_memory_budget() {
    let start = heap();
    let mut grammar = Grammar::default();
    grammar.check("en-GB", &["The first check loads everything.".to_string()], &[]).unwrap();
    let loaded = heap() - start;
    // A long Work, every sentence different, checked a paragraph at a time through the same rules.
    // Patched, the caches hold 64 entries each; published, 1,000.
    let paragraphs: Vec<String> = (0..1500)
        .map(|i| format!("On day {i} the {} ships left the harbour, and {} sailors, tired and cold, watched them go.", i * 7 + 3, i * 13 + 5))
        .collect();
    for batch in paragraphs.chunks(40) {
        grammar.check("en-GB", batch, &[]).unwrap();
    }
    let grown = heap().saturating_sub(start + loaded);
    eprintln!("Harper: {} MB to load, {} MB more after 1,500 paragraphs", loaded / MB, grown / MB);
    // Patched: the dictionary (61 MB), its FST (2.5), the rules (11.5) and the tagger (4.6), about
    // 75 MB in all. Published: about 136 MB, the dictionary twice.
    assert!(loaded < 100 * MB, "Harper took {} MB to load: is the patch applied?", loaded / MB);
    assert!(grown < 40 * MB, "Harper's caches grew by {} MB: is the patch applied?", grown / MB);
}
