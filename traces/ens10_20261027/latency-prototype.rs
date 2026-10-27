//! Standalone ENS10 latency recorder prototype: NOT wired into the native adapters.
//! Explicit turn/notification association is supplied by the caller, never inferred from a
//! GPU-wide vector alone. No marker implies incomplete coverage. No scheduling decisions.
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Id {
    life: u64,
    channel: u32,
    generation: u64,
    turn: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cause {
    Own,
    Other,
    Split,
    Scan,
    Unknown,
}
impl Cause {
    fn slot(self) -> usize {
        self as usize
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stages {
    observe_publish: u64,
    publish_marker: u64,
    marker_raise: u64,
    total: u64,
}
#[derive(Debug, Clone)]
struct Sample {
    id: Id,
    observed: u64,
    published: Option<u64>,
    covering_marker: Option<u64>,
    marked: Option<u64>,
    raised: Option<(u32, u64)>,
    expected_vector: u32,
    wait: Option<(Cause, u64)>,
    waits: [u64; 5],
    ambiguous: bool,
}
impl Sample {
    fn new(id: Id, observed: u64, expected_vector: u32) -> Self {
        Self {
            id,
            observed,
            published: None,
            covering_marker: None,
            marked: None,
            raised: None,
            expected_vector,
            wait: None,
            waits: [0; 5],
            ambiguous: false,
        }
    }
    fn close_wait(&mut self, now: u64) {
        if let Some((cause, start)) = self.wait.take() {
            if let Some(ns) = now.checked_sub(start) {
                self.waits[cause.slot()] = self.waits[cause.slot()].saturating_add(ns);
            } else {
                self.ambiguous = true;
            }
        }
    }
    fn gate(&mut self, cause: Option<Cause>, now: u64) {
        if self.published.is_some() {
            return;
        }
        if self.wait.map(|(c, _)| c) != cause {
            self.close_wait(now);
            self.wait = cause.map(|c| (c, now));
        }
    }
    fn publish(&mut self, now: u64) {
        if self.published.is_none() {
            self.close_wait(now);
            self.published = Some(now);
            self.ambiguous |= now < self.observed;
        }
    }
    fn cover(&mut self, marker: u64) {
        if self.published.is_some() && self.covering_marker.is_none() {
            self.covering_marker = Some(marker);
        }
    }
    fn marker(&mut self, marker: u64, now: u64) {
        if self.covering_marker != Some(marker) || self.marked.is_some() {
            return;
        }
        self.marked = Some(now);
        self.ambiguous |= self.published.is_none_or(|p| now < p);
        // A prior notification is retained as an observation-order case, not replaced.
        self.ambiguous |= self.raised.is_some();
    }
    fn notify(&mut self, vector: u32, successful: bool, now: u64) {
        if vector != self.expected_vector || !successful || self.raised.is_some() {
            return;
        }
        self.raised = Some((vector, now));
        self.ambiguous |= self.marked.is_none_or(|m| now < m);
    }
    fn stages(&self) -> Option<Stages> {
        if self.ambiguous {
            return None;
        }
        let (p, m, (_, r)) = (self.published?, self.marked?, self.raised?);
        let s = Stages {
            observe_publish: p.checked_sub(self.observed)?,
            publish_marker: m.checked_sub(p)?,
            marker_raise: r.checked_sub(m)?,
            total: r.checked_sub(self.observed)?,
        };
        (s.observe_publish
            .checked_add(s.publish_marker)?
            .checked_add(s.marker_raise)?
            == s.total)
            .then_some(s)
    }
}

#[derive(Debug)]
struct Recorder<const N: usize> {
    pending: VecDeque<Sample>,
    slow: VecDeque<Sample>,
    evicted: u64,
    canceled: u64,
    maxima: [u64; 3],
    complete: u64,
    first_dump_requested: bool,
    dumps: u64,
}
impl<const N: usize> Recorder<N> {
    fn new() -> Self {
        assert!(N > 0);
        Self {
            pending: VecDeque::with_capacity(N),
            slow: VecDeque::with_capacity(N),
            evicted: 0,
            canceled: 0,
            maxima: [0; 3],
            complete: 0,
            first_dump_requested: false,
            dumps: 0,
        }
    }
    fn observe(&mut self, s: Sample) {
        if self.pending.iter().any(|old| old.id == s.id) {
            return;
        }
        if self.pending.len() == N {
            self.pending.pop_front();
            self.evicted += 1;
        }
        self.pending.push_back(s);
    }
    fn sample(&mut self, id: Id) -> Option<&mut Sample> {
        self.pending.iter_mut().find(|s| s.id == id)
    }
    fn retire(&mut self, id: Id, threshold: u64) {
        let Some(index) = self.pending.iter().position(|s| s.id == id) else {
            return;
        };
        let Some(stages) = self.pending[index].stages() else {
            return;
        };
        let s = self.pending.remove(index).unwrap();
        self.complete += 1;
        for (max, ns) in self.maxima.iter_mut().zip([
            stages.observe_publish,
            stages.publish_marker,
            stages.marker_raise,
        ]) {
            *max = (*max).max(ns);
        }
        if stages.total > threshold {
            if self.slow.len() == N {
                self.slow.pop_front();
            }
            self.slow.push_back(s);
            if !self.first_dump_requested {
                self.first_dump_requested = true;
                self.dumps += 1;
            }
        }
    }
    fn reset(&mut self, life: u64) {
        let before = self.pending.len();
        self.pending.retain(|s| s.id.life != life);
        self.canceled += (before - self.pending.len()) as u64;
    }
    fn slow_pending(&self, now: u64, threshold: u64) -> Vec<Id> {
        self.pending
            .iter()
            .filter(|s| now.saturating_sub(s.observed) > threshold)
            .map(|s| s.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(life: u64, turn: u64) -> Id {
        Id {
            life,
            channel: 7,
            generation: 1,
            turn,
        }
    }
    fn sample(delay: u64) -> Sample {
        let mut s = Sample::new(id(1, 1), 10, 19);
        s.gate(Some(Cause::Own), 12);
        s.publish(20 + delay);
        s.cover(1);
        s.marker(1, 30 + delay);
        s.notify(19, true, 40 + delay);
        s
    }
    #[test]
    fn sum_is_exact_for_every_simulated_stage_length() {
        for a in 0..8 {
            for b in 0..8 {
                for c in 0..8 {
                    let mut s = Sample::new(id(1, 1), 11, 3);
                    s.publish(11 + a);
                    s.cover(9);
                    s.marker(9, 11 + a + b);
                    s.notify(3, true, 11 + a + b + c);
                    let d = s.stages().unwrap();
                    assert_eq!(d.total, a + b + c);
                    assert_eq!(
                        (d.observe_publish, d.publish_marker, d.marker_raise),
                        (a, b, c)
                    );
                }
            }
        }
    }
    #[test]
    fn own_gate_hold_changes_only_observe_to_publish() {
        let first = sample(0);
        let held = sample(500);
        assert_eq!(
            held.waits[Cause::Own.slot()],
            first.waits[Cause::Own.slot()] + 500
        );
        let a = first.stages().unwrap();
        let b = held.stages().unwrap();
        assert_eq!(b.observe_publish, a.observe_publish + 500);
        assert_eq!(
            (b.publish_marker, b.marker_raise),
            (a.publish_marker, a.marker_raise)
        );
    }
    #[test]
    fn scan_budget_hold_changes_only_observe_to_publish() {
        let mut s = Sample::new(id(1, 1), 10, 19);
        s.gate(Some(Cause::Scan), 10);
        s.gate(Some(Cause::Scan), 90);
        s.publish(110);
        s.cover(1);
        s.marker(1, 120);
        s.notify(19, true, 130);
        assert_eq!(s.waits[Cause::Scan.slot()], 100);
        assert_eq!(s.stages().unwrap().publish_marker, 10);
        assert_eq!(s.stages().unwrap().marker_raise, 10);
    }
    #[test]
    fn out_of_order_vector_is_retained_and_never_made_complete() {
        let mut s = Sample::new(id(1, 1), 10, 19);
        s.publish(20);
        s.cover(1);
        s.notify(19, true, 25);
        s.marker(1, 30);
        s.notify(19, true, 40);
        assert_eq!(s.raised, Some((19, 25)));
        assert!(s.stages().is_none());
    }
    #[test]
    fn held_or_failed_signal_does_not_complete_a_record() {
        let mut s = Sample::new(id(1, 1), 10, 19);
        s.publish(20);
        s.cover(1);
        s.marker(1, 30);
        s.notify(19, false, 31);
        assert!(s.stages().is_none());
        s.notify(19, true, 40);
        assert_eq!(s.stages().unwrap().marker_raise, 10);
    }
    #[test]
    fn wrong_vector_does_not_complete_a_packet() {
        let mut s = Sample::new(id(1, 1), 10, 19);
        s.publish(20);
        s.cover(1);
        s.marker(1, 30);
        s.notify(20, true, 31);
        assert!(s.stages().is_none());
        s.notify(19, true, 40);
        assert_eq!(s.stages().unwrap().marker_raise, 10);
    }
    #[test]
    fn missing_marker_never_gains_synthetic_coverage() {
        let mut s = Sample::new(id(1, 1), 10, 19);
        s.publish(20);
        s.marker(1, 30);
        s.notify(19, true, 40);
        assert!(s.marked.is_none());
        assert!(s.stages().is_none());
    }
    #[test]
    fn retries_preserve_the_oldest_demand_observation() {
        let mut r = Recorder::<2>::new();
        r.observe(Sample::new(id(1, 1), 10, 19));
        r.observe(Sample::new(id(1, 1), 90, 19));
        assert_eq!(r.sample(id(1, 1)).unwrap().observed, 10);
    }
    #[test]
    fn reset_and_token_reuse_do_not_cross_associate() {
        let mut r = Recorder::<2>::new();
        r.observe(Sample::new(id(1, 1), 10, 19));
        r.reset(1);
        r.observe(Sample::new(id(2, 1), 90, 19));
        assert!(r.sample(id(1, 1)).is_none());
        assert_eq!(r.canceled, 1);
        assert_eq!(r.sample(id(2, 1)).unwrap().observed, 90);
    }
    #[test]
    fn capacity_is_bounded_and_every_eviction_counted() {
        let mut r = Recorder::<2>::new();
        for turn in 0..100 {
            r.observe(Sample::new(id(1, turn), 0, 19));
        }
        assert_eq!(r.pending.len(), 2);
        assert_eq!(r.evicted, 98);
        assert_eq!(r.slow_pending(100, 50).len(), 2);
    }
    #[test]
    fn history_and_first_slow_dump_are_bounded() {
        let mut r = Recorder::<2>::new();
        for turn in 0..100 {
            let mut s = sample(100);
            s.id = id(1, turn);
            r.observe(s);
            r.retire(id(1, turn), 50);
        }
        assert_eq!(r.slow.len(), 2);
        assert_eq!(r.dumps, 1);
        assert_eq!(r.complete, 100);
        assert_eq!(r.maxima, [110, 10, 10]);
    }
    #[test]
    fn fast_samples_request_no_dump_and_keep_no_slow_history() {
        let mut r = Recorder::<2>::new();
        r.observe(sample(0));
        r.retire(id(1, 1), 50);
        assert!(r.slow.is_empty());
        assert_eq!(r.dumps, 0);
        assert_eq!(r.complete, 1);
    }
    #[test]
    fn cause_switches_close_intervals_without_resetting_repeated_holds() {
        let mut s = Sample::new(id(1, 1), 0, 19);
        s.gate(Some(Cause::Other), 10);
        s.gate(Some(Cause::Other), 15);
        s.gate(Some(Cause::Split), 20);
        s.gate(Some(Cause::Unknown), 30);
        s.publish(40);
        assert_eq!(s.waits, [0, 10, 10, 0, 10]);
        s.gate(Some(Cause::Own), 100);
        assert_eq!(s.waits, [0, 10, 10, 0, 10]);
    }
}
