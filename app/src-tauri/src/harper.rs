//! Harper's own thread (ADR 0023): everything that reads text with Harper runs here, one thing at
//! a time, so Harper's per-thread state exists once. Its noun-phrase chunker is a thread-local
//! with its own cache of 10,000 sentences, so Harper run on whichever pool thread is free would
//! grow one copy per thread.
//!
//! Two queues: word types first (the Syntax highlighting Mod asks for what's on screen, and it
//! shows as soon as it's answered), then grammar (the Grammar checker asks for a whole Work in
//! batches, which can take half a minute the first time). A scroll never waits behind a novel.
//!
//! The thread starts with the app and holds nothing until the first job; Harper loads then
//! (grammar.rs). A job that panics is answered with an error, and the next one starts afresh.

use crate::grammar::Grammar;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex};

type Job = Box<dyn FnOnce(&mut Grammar) + Send>;

/// Which queue a job joins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Queue {
    /// Before anything in `Later`: what the Author is looking at.
    First,
    Later,
}

#[derive(Default)]
struct Queues {
    first: VecDeque<Job>,
    later: VecDeque<Job>,
}

/// Harper's thread, and the way to give it jobs.
pub struct HarperThread {
    queues: Arc<(Mutex<Queues>, Condvar)>,
}

impl HarperThread {
    pub fn start() -> HarperThread {
        let queues: Arc<(Mutex<Queues>, Condvar)> = Arc::default();
        let theirs = queues.clone();
        std::thread::Builder::new()
            .name("harper".into())
            .spawn(move || {
                let mut grammar = Grammar::default();
                loop {
                    let job = next(&theirs);
                    if catch_unwind(AssertUnwindSafe(|| job(&mut grammar))).is_err() {
                        grammar = Grammar::default();
                    }
                }
            })
            .expect("Harper's thread starts");
        HarperThread { queues }
    }

    /// Queues `f` to run on Harper's thread, after what's already in `queue` (and, for `Later`,
    /// after everything in `First` too); awaiting the result gives back what it returns. It's
    /// queued now, not when first awaited.
    pub fn run<T, F>(&self, queue: Queue, f: F) -> impl Future<Output = Result<T, String>> + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&mut Grammar) -> T + Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move |grammar| {
            let _ = tx.send(f(grammar));
        });
        {
            let (lock, ready) = &*self.queues;
            let mut queues = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match queue {
                Queue::First => queues.first.push_back(job),
                Queue::Later => queues.later.push_back(job),
            }
            ready.notify_one();
        }
        // The answer is dropped unsent only if the job panicked.
        async move { rx.await.map_err(|_| "Something went wrong reading the text with Harper.".to_string()) }
    }
}

/// The next job, waiting for one if there's none.
fn next(queues: &(Mutex<Queues>, Condvar)) -> Job {
    let (lock, ready) = queues;
    let mut queues = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if let Some(job) = queues.first.pop_front().or_else(|| queues.later.pop_front()) {
            return job;
        }
        queues = ready.wait(queues).unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn what_is_on_screen_goes_before_grammar_already_waiting() {
        let harper = HarperThread::start();
        let order = Arc::new(Mutex::new(Vec::new()));
        // The thread is held until all three are queued, so the others wait behind the first.
        let (release, held) = mpsc::channel::<()>();
        let busy = harper.run(Queue::Later, move |_| held.recv().unwrap());
        let o = order.clone();
        let grammar = harper.run(Queue::Later, move |_| o.lock().unwrap().push("grammar"));
        let o = order.clone();
        let words = harper.run(Queue::First, move |_| o.lock().unwrap().push("words"));
        tauri::async_runtime::block_on(async {
            release.send(()).unwrap();
            busy.await.unwrap();
            grammar.await.unwrap();
            words.await.unwrap();
        });
        assert_eq!(*order.lock().unwrap(), ["words", "grammar"]);
    }

    #[test]
    fn a_job_that_panics_is_an_error_and_the_next_one_runs() {
        let harper = HarperThread::start();
        tauri::async_runtime::block_on(async {
            let failed = harper.run(Queue::First, |_| -> u8 { panic!("boom") }).await;
            assert!(failed.is_err());
            assert_eq!(harper.run(Queue::First, |_| 7).await, Ok(7));
        });
    }
}
