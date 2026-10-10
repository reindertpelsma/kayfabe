//! ★ Review 4 item 4 — **the act thread's queue with delayed re-queues, and no sleeping.**
//!
//! A steer retry (`chan::EngineObj`) used to `thread::sleep(1 ms)` on the act thread before trying
//! again, so N stuck steers delayed every unrelated act by N ms (rule: no thread that serves input may
//! stall). Now a retry is parked in a delay list with a `not_before` and the act thread goes on with
//! whatever else is queued; it blocks only until the earliest of (a new act arrives, the earliest
//! deadline). Queued acts always run before due retries, so a stuck steer can never delay an
//! unrelated act, whatever their number.

use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Instant;

/// The queue the act thread drains, plus the retries waiting for their time.
pub struct ActQueue<T> {
    rx: Receiver<T>,
    delayed: Vec<(Instant, T)>,
}

impl<T> ActQueue<T> {
    /// A queue over `rx`.
    #[must_use]
    pub fn new(rx: Receiver<T>) -> Self {
        ActQueue {
            rx,
            delayed: Vec::new(),
        }
    }

    /// Park `item` until `not_before` (it runs after every act already queued, once due).
    pub fn delay(&mut self, not_before: Instant, item: T) {
        self.delayed.push((not_before, item));
    }

    /// Retries parked.
    #[must_use]
    pub fn parked(&self) -> usize {
        self.delayed.len()
    }

    /// The next act to run: a queued one if any, else the earliest DUE parked one, else wait for the
    /// earlier of a new act and the earliest deadline. `None` once the sender is gone.
    pub fn next_act(&mut self) -> Option<T> {
        loop {
            match self.rx.try_recv() {
                Ok(x) => return Some(x),
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) => {}
            }
            let now = Instant::now();
            let due = self
                .delayed
                .iter()
                .enumerate()
                .filter(|(_, (t, _))| *t <= now)
                .min_by_key(|(_, (t, _))| *t)
                .map(|(i, _)| i);
            if let Some(i) = due {
                return Some(self.delayed.swap_remove(i).1);
            }
            match self.delayed.iter().map(|(t, _)| *t).min() {
                Some(t) => match self.rx.recv_timeout(t.saturating_duration_since(now)) {
                    Ok(x) => return Some(x),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return None,
                },
                None => return self.rx.recv().ok(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[derive(Debug)]
    enum Item {
        /// A steer that never completes: re-parked 1 ms ahead until it has been tried `n` times.
        Stuck(u32),
        /// An unrelated act, with its enqueue time.
        Other(Instant),
    }

    /// N stuck steers add no latency to unrelated acts: with the old `sleep(1 ms)` per retry the
    /// unrelated acts below waited ~N ms each; parked retries cost them nothing.
    #[test]
    fn stuck_steers_add_no_latency_to_unrelated_acts() {
        let (tx, rx) = mpsc::channel::<Item>();
        const STUCK: u32 = 50;
        const TRIES: u32 = 20;
        for _ in 0..STUCK {
            tx.send(Item::Stuck(0)).unwrap();
        }
        let worker = std::thread::spawn(move || {
            let mut q = ActQueue::new(rx);
            let (mut worst, mut others, mut retries) = (Duration::ZERO, 0u32, 0u32);
            while let Some(it) = q.next_act() {
                match it {
                    Item::Stuck(n) if n + 1 < TRIES => {
                        retries += 1;
                        q.delay(Instant::now() + Duration::from_millis(1), Item::Stuck(n + 1));
                    }
                    Item::Stuck(_) => {}
                    Item::Other(t0) => {
                        worst = worst.max(t0.elapsed());
                        others += 1;
                    }
                }
            }
            (worst, others, retries, q.parked())
        });
        // Unrelated acts arrive while the 50 steers retry.
        for _ in 0..40 {
            tx.send(Item::Other(Instant::now())).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(40));
        drop(tx);
        let (worst, others, retries, parked) = worker.join().unwrap();
        eprintln!("unrelated acts: worst latency {worst:?} over {others} acts; {retries} retries");
        assert_eq!(others, 40);
        assert_eq!(retries, STUCK * (TRIES - 1), "every stuck steer got all its tries");
        assert_eq!(parked, 0);
        // The sleeping design cost 50 x 1 ms = 50 ms per retry round; here: scheduling only.
        assert!(worst < Duration::from_millis(15), "worst latency {worst:?}");
    }

    #[test]
    fn a_parked_act_runs_when_due_and_not_before() {
        let (tx, rx) = mpsc::channel::<u32>();
        let mut q = ActQueue::new(rx);
        let t0 = Instant::now();
        q.delay(t0 + Duration::from_millis(30), 7);
        tx.send(1).unwrap();
        assert_eq!(q.next_act(), Some(1), "queued first");
        assert_eq!(q.next_act(), Some(7));
        assert!(t0.elapsed() >= Duration::from_millis(30), "not before its time");
        drop(tx);
        assert_eq!(q.next_act(), None);
    }
}
