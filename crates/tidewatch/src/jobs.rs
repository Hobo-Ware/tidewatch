//! Runs blocking work (git, disk) on a background thread. The result comes back through a channel
//! and a repaint wakes the UI to collect it, so the render loop never waits.

use std::sync::mpsc::{Receiver, Sender, channel};

pub(crate) struct Jobs<T> {
    sender: Sender<T>,
    receiver: Receiver<T>,
}

impl<T: Send + 'static> Jobs<T> {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = channel();
        Self { sender, receiver }
    }

    pub(crate) fn spawn(&self, ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) {
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(work());
            ctx.request_repaint();
        });
    }

    /// Every result that finished since the last call, oldest first.
    pub(crate) fn drain(&self) -> Vec<T> {
        self.receiver.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn wait_for<T: Send + 'static>(jobs: &Jobs<T>, count: usize) -> Vec<T> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.len() < count && Instant::now() < deadline {
            got.extend(jobs.drain());
            std::thread::sleep(Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn result_arrives_through_the_channel() {
        let jobs = Jobs::new();
        jobs.spawn(&egui::Context::default(), || 21 * 2);
        assert_eq!(wait_for(&jobs, 1), [42]);
    }

    #[test]
    fn work_runs_off_the_calling_thread() {
        let jobs = Jobs::new();
        let caller = std::thread::current().id();
        jobs.spawn(&egui::Context::default(), move || std::thread::current().id());
        assert_ne!(wait_for(&jobs, 1)[0], caller);
    }

    #[test]
    fn drain_is_empty_until_work_finishes_and_then_empties_again() {
        let jobs: Jobs<u8> = Jobs::new();
        assert!(jobs.drain().is_empty());
        jobs.spawn(&egui::Context::default(), || 1);
        assert_eq!(wait_for(&jobs, 1), [1]);
        assert!(jobs.drain().is_empty());
    }

    #[test]
    fn finished_work_asks_for_a_repaint() {
        let ctx = egui::Context::default();
        let asked = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&asked);
        ctx.set_request_repaint_callback(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let jobs = Jobs::new();
        jobs.spawn(&ctx, || ());
        wait_for(&jobs, 1);
        let deadline = Instant::now() + Duration::from_secs(5);
        while asked.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(asked.load(Ordering::SeqCst) >= 1);
    }
}
