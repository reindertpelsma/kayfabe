//! EXPECTED RED (race/correctness audit 2026-10-21, finding RC-02): a worker pass that serves NOTHING may
//! park while a flagged, claimable token is still pending.
//!
//! `RungBitmap::scan` returns at most `limit` (the worker's `SCAN_LIMIT` = 64) tokens, re-publishes the
//! remainder WITHOUT a bump, and `worker_pass` serves only tokens it can `claim()` (state RUNG). A bit left
//! over an IDLE / DEAD word ("the bitmap is a hint; the word is the truth") is a stale bit and costs one
//! look. If the first 64 returned tokens are all stale, the pass serves 0, `served == 0` sends
//! `worker::run_with_relay` to `try_park(seen)`, `seen` was loaded before the pass and nothing bumped the
//! wake word since, so the park is ACCEPTED while a live RUNG token (beyond the limit) sits in the bitmap.
//! Nobody rings it again: the doorbell was consumed by the first scan's bump.
//!
//! Loop invariant under test (the one `worker.rs` states): "if the pass served nothing and the word has not
//! moved, no flagged token may be left behind".
use kf_core::plane::{HostOps, Translatable, Vmm};
use kf_core::{HostSlice, Owner, Plane, Step, VmCaps};
use kf_trap::token::{Route, State};
use kf_trap::trap::Class;

#[derive(Default)]
struct Null;
impl HostOps for Null {
    fn wake_worker(&self) {}

    fn ring_host(&self, _t: u32) {}
    fn run_translated(&self, _t: u32, _s: u64) -> bool {
        true
    }
    fn run_emulated(&self, _t: u32, _s: u64) {}
    fn apply_register(&self, _b: u8, _off: u32, _v: u64, _w: u8) {}
    fn operands_translatable(&self, _t: u32, _s: u64) -> Translatable {
        Translatable::Yes
    }
    fn forge_completion(&self, _t: u32) {}
    fn refuse_and_poison(&self, _t: u32) {}
    fn fault_channel(&self, _t: u32) {}
    fn map_guest_slice(&self, _s: HostSlice) {}
    fn teardown_step(&self, _s: Step) {}
}

/// One worker iteration exactly as `worker::run_with_relay` runs it up to the park decision.
fn iteration(p: &Plane<'_>, host: &Null, limit: usize) -> (usize, bool) {
    let mut scratch = Vec::with_capacity(limit);
    let seen = p.worker_wake().seen();
    let n = p.worker_pass(host, &mut scratch, limit);
    // `n == 0` and no relay obligation: the loop goes to `try_park(seen)`.
    let parked = n == 0 && p.worker_wake().try_park(seen);
    (n, parked)
}

#[test]
fn a_pass_that_served_nothing_must_not_park_beside_a_pending_live_token() {
    const LIMIT: usize = 64; // kf_chan::worker::SCAN_LIMIT
    let vmm = Vmm::new();
    let p = Plane::new(&vmm, 256, 0xff);
    let host = Null;
    let mut caps = VmCaps::from_declared(512, 8, 8, 8);

    // 64 tokens that were rung and then retired (a guest freeing channels): stale bits over DEAD words.
    for t in 0..LIMIT as u32 {
        p.allocate_channel(&mut caps, t, Route::Translated, 0x100 + t, Owner::User)
            .unwrap();
        p.trap_write(Class::Doorbell, 0, 0, u64::from(t), 4);
        assert!(p.retire_channel_token(&mut caps, t), "retire from RUNG");
    }
    // One live token in the next bitmap word (scan order is by word), rung after the stale ones.
    let live = 70u32;
    p.allocate_channel(&mut caps, live, Route::Translated, 0x200, Owner::User)
        .unwrap();
    p.trap_write(Class::Doorbell, 0, 0, u64::from(live), 4);
    assert_eq!(p.tokens[live as usize].load().state, State::Rung);
    assert!(p.bits.bit(live));

    let (n, parked) = iteration(&p, &host, LIMIT);
    let still_pending = p.bits.bit(live) && p.tokens[live as usize].load().state == State::Rung;
    if parked {
        p.worker_wake().unpark();
    }
    assert!(
        n > 0 || !parked || !still_pending,
        "RC-02: the pass served {n} tokens (all {LIMIT} returned by scan() were stale), the worker was allowed to PARK \
         (try_park accepted), and live token {live} is still RUNG with its bit set: a lost wake until the next unrelated ring"
    );
}
