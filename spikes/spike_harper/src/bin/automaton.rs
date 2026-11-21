//! Spike: the Levenshtein builders SpellCheck keeps per thread. `cargo run --release --bin automaton`
//! Minimal repro for an upstream issue; uses only harper-core's public API.

use harper_core::linting::{LintGroup, Linter};
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

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

fn heap_mb() -> f64 {
    CURRENT.load(Relaxed) as f64 / 1_048_576.0
}

/// Lint `text` with SpellCheck alone, reporting what the heap kept and how long it took.
fn spell(label: &str, text: &str) {
    let before = heap_mb();
    let mut group = LintGroup::new_curated(FstDictionary::curated(), Dialect::American);
    group.set_all_rules_to(Some(false));
    group.config.set_rule_enabled("SpellCheck", true);
    let doc = Document::new_plain_english_curated(text);
    let t = Instant::now();
    let lints = group.lint(&doc);
    let took = t.elapsed().as_secs_f64() * 1000.0;
    drop(lints);
    drop(group);
    drop(doc);
    println!("{label:<55} kept {:+6.1} MB, {took:7.1} ms", heap_mb() - before);
}

fn main() {
    let _ = FstDictionary::curated(); // load the dictionary first, so it isn't counted below
    println!("heap after loading the dictionary: {:.1} MB\n", heap_mb());
    spell("main thread, \"The cat sat.\" (no misspelling)", "The cat sat.");
    spell("main thread, \"Teh cat sat.\" (found within 2)", "Teh cat sat.");
    spell("main thread, \"Saintsbury wrote it.\" (found within 3)", "Saintsbury wrote it.");
    spell("main thread, \"Kvothe wrote it.\" (again)", "Kvothe wrote it.");
    spell("main thread, \"Xyzzyplugh wrote it.\" (nothing within 3)", "Xyzzyplugh wrote it.");
    spell("main thread, \"Qwrtzplk wrote it.\" (nothing within 3, again)", "Qwrtzplk wrote it.");
    spell("main thread, \"Xyzzyplugh wrote it.\" (third time)", "Xyzzyplugh wrote it.");
    let t = std::thread::spawn(|| {
        spell("second thread, \"Saintsbury wrote it.\"", "Saintsbury wrote it.");
        spell("second thread, \"Xyzzyplugh wrote it.\"", "Xyzzyplugh wrote it.");
    });
    t.join().unwrap();
    println!("\nheap after the second thread exited: {:.1} MB", heap_mb());
}
