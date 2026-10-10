//! ★ 2026-10-10 (Windows TDR hunt, shape F) — **hardware order at a vblank: latch, complete, THEN the vblank event.**
//!
//! On NVDisplay the window update pending for a vblank is latched AT the vblank, and the head's vblank / `LAST_DATA` event
//! is raised after it; a driver's VSync handler therefore always reads the post-latch state (GET past the UPDATE, the
//! ARMED words, the new flip's notifier BEGUN and the flipped-away one's FINISHED). The display thread used to raise the
//! head-timing event at the tick and deliver the latch's completions (`Effect`s and GETs, queued behind the console copy)
//! ~0.9 ms later, so the guest's handler could see a pre-, mid- or post-latch state. [measured, Windows runs 268-279] it
//! normally ran before the latch pass (0.50-0.57 ms); every first stuck flip had it at or after the pass.
//!
//! The gate makes the order structural: a head's frame edge (the event bits AND the interrupt) is held until every queued
//! completion produced up to and including that vblank has been delivered, so no guest read can observe the event of a
//! vblank together with a pre-latch word of the same vblank. Pure bookkeeping: no lock, no wait — the display thread
//! asks [`VblankGate::due`] once per pass. A completion that cannot be delivered for [`EDGE_CAP`] (a console copy stuck
//! behind the host) releases the edge anyway, counted, rather than stopping the guest's vblanks.
//!
//! [measured, Windows runs 282-284] with this order, a window notifier written FINISHED at its own latch (or BEGUN
//! with no flip-away FINISHED) stuck every first flip: kayfabe's post-latch state was wrong, not the order. [measured,
//! run 287] with BEGUN at the latch and FINISHED at the flip-away (`kf_disp::engine::Effect::Notify`): 0 TDR in boot,
//! sign-in and Edge. [`EDGE_CAP`] is a liveness backstop for a console copy stuck behind the host, never the ordering
//! mechanism (0 forced edges in runs 282-287).

use std::time::{Duration, Instant};

/// Heads the gate tracks (the display model's ceiling).
pub const HEADS: usize = kf_disp::ports::MAX_HEADS;

/// The longest a frame edge may wait for its completions (a stuck console copy), after which it is raised and counted.
pub const EDGE_CAP: Duration = Duration::from_millis(100);

/// Per-head held edges and the queue's push/delivery counts.
#[derive(Debug, Default)]
pub struct VblankGate {
    pushed: u64,
    delivered: u64,
    /// Per head: the push count the edge waits for, and when it was held.
    held: [Option<(u64, Instant)>; HEADS],
    /// Edges raised by [`EDGE_CAP`] rather than by delivery.
    pub forced: u64,
}

impl VblankGate {
    /// One completion item was queued.
    pub fn pushed(&mut self) {
        self.pushed += 1;
    }

    /// The queue now holds `remaining` items: everything else that was pushed has been delivered (FIFO) or dropped.
    pub fn remaining(&mut self, remaining: usize) {
        self.delivered = self.pushed.saturating_sub(remaining as u64);
    }

    /// Head `h` ticked a vblank AFTER this vblank's completions were pushed: its edge waits for all of them. A second
    /// tick while the first edge is held merges into it (the event is a level).
    pub fn tick(&mut self, h: usize, now: Instant) {
        if let Some(slot) = self.held.get_mut(h) {
            let at = slot.map_or(now, |(_, t)| t);
            *slot = Some((self.pushed, at));
        }
    }

    /// The heads whose edge may be raised now (each returned once), in head order.
    pub fn due(&mut self, now: Instant) -> Vec<usize> {
        let mut out = Vec::new();
        for (h, slot) in self.held.iter_mut().enumerate() {
            if let Some((need, at)) = *slot {
                if self.delivered >= need {
                    *slot = None;
                    out.push(h);
                } else if now.duration_since(at) >= EDGE_CAP {
                    *slot = None;
                    self.forced += 1;
                    out.push(h);
                }
            }
        }
        out
    }

    /// The earliest moment a held edge reaches [`EDGE_CAP`] (the display thread's wake-up), if any is held.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.held.iter().flatten().map(|(_, at)| *at + EDGE_CAP).min()
    }

    /// Whether head `h` has an edge held.
    #[must_use]
    pub fn holding(&self, h: usize) -> bool {
        self.held.get(h).is_some_and(Option::is_some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One vblank's worth of guest-visible state, as the guest's VSync handler would read it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Seen {
        event: bool,
        latched: bool,
        notified: bool,
        get_past_update: bool,
    }

    /// The model of one display pass sequence at a vblank with a pending window flip: three completions (Latched,
    /// Notify, GET) queued behind a console copy that completes after `copy_done_after` passes; `gated` = the edge goes
    /// through the gate (else it is raised at the tick, the old order). Returns every state the guest's VSync handler
    /// could read: one observation between every two atomic steps of every pass.
    fn simulate(gated: bool, copy_done_after: usize) -> (Vec<Seen>, VblankGate) {
        let t0 = Instant::now();
        let mut gate = VblankGate::default();
        let mut queue: Vec<&str> = Vec::new();
        let mut s = Seen { event: false, latched: false, notified: false, get_past_update: false };
        let mut obs = vec![s];
        for item in ["Latched", "Notify", "Get"] {
            queue.push(item);
            gate.pushed();
        }
        if gated {
            gate.tick(0, t0);
        } else {
            s.event = true;
            obs.push(s);
        }
        for pass in 0..=copy_done_after + 1 {
            if pass >= copy_done_after {
                while !queue.is_empty() {
                    match queue.remove(0) {
                        "Latched" => s.latched = true,
                        "Notify" => s.notified = true,
                        _ => s.get_past_update = true,
                    }
                    gate.remaining(queue.len());
                    obs.push(s);
                }
            }
            for h in gate.due(t0) {
                assert_eq!(h, 0);
                s.event = true;
                obs.push(s);
            }
            obs.push(s);
        }
        (obs, gate)
    }

    fn half_applied(o: &Seen) -> bool {
        o.event && !(o.latched && o.notified && o.get_past_update)
    }

    /// ★ The interleaving test: whatever point of whatever pass the guest's VSync handler runs at, if it sees the
    /// vblank's event it sees the whole latch (the hardware order) — for every copy latency modelled.
    #[test]
    fn a_guest_vsync_handler_never_sees_the_event_without_the_whole_latch() {
        for copy_done_after in 0..6 {
            let (obs, gate) = simulate(true, copy_done_after);
            assert!(obs.last().is_some_and(|o| o.event), "the edge is raised once the completions landed");
            assert!(!obs.iter().any(half_applied), "copy after {copy_done_after}: {obs:?}");
            assert_eq!(gate.forced, 0);
        }
    }

    /// The control: the old order (edge at the tick, completions after the copy) produces exactly the observation the
    /// invariant forbids — so the test above is not vacuous.
    #[test]
    fn the_old_order_lets_the_guest_see_a_half_applied_latch() {
        for copy_done_after in 0..6 {
            let (obs, _) = simulate(false, copy_done_after);
            assert!(obs.iter().any(half_applied), "copy after {copy_done_after}");
        }
    }

    /// A vblank with nothing to complete raises at once; a held edge merges a second tick; a stuck copy is capped.
    #[test]
    fn empty_vblanks_raise_now_merged_ticks_raise_once_and_a_stuck_copy_is_capped() {
        let t0 = Instant::now();
        let mut g = VblankGate::default();
        g.tick(1, t0);
        assert_eq!(g.due(t0), vec![1]);
        assert!(g.due(t0).is_empty(), "returned once");
        g.pushed();
        g.tick(0, t0);
        g.tick(0, t0 + Duration::from_millis(16));
        assert!(g.due(t0 + Duration::from_millis(16)).is_empty());
        assert!(g.holding(0));
        g.remaining(0);
        assert_eq!(g.due(t0 + Duration::from_millis(17)), vec![0]);
        g.pushed();
        g.tick(2, t0);
        assert!(g.due(t0 + EDGE_CAP - Duration::from_millis(1)).is_empty());
        assert_eq!(g.deadline(), Some(t0 + EDGE_CAP));
        assert_eq!(g.due(t0 + EDGE_CAP), vec![2]);
        assert_eq!(g.deadline(), None);
        assert_eq!(g.forced, 1);
        assert!(g.tick_out_of_range_is_ignored());
    }

    impl VblankGate {
        fn tick_out_of_range_is_ignored(&mut self) -> bool {
            self.tick(HEADS + 3, Instant::now());
            !self.held.iter().any(Option::is_some)
        }
    }
}
