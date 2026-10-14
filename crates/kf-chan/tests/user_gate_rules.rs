//! ★ 2026-10-14 — GPU-free rules for the USER-ring relay's GATE (`kf_chan::user_gate`), derived
//! from the public hardware text and the owner's rules, NOT from what the code does today
//! (`docs/design/V3_USER_RING_EVENT_LOOP_20261014.md`, sections 2 and 5):
//!
//! * `dev_pbdma.ref` `GP_ENTRY1_SYNC_WAIT`: Host does not begin fetching the entry's pushbuffer
//!   segment until the PRECEDING segment has been processed; for a `LENGTH == 0` entry no further GP
//!   entry is processed until the wait finishes. A SYNC_WAIT is a property of THAT entry, every time
//!   the ring reaches it -- also one ring lap later.
//! * the PBDMA parses method packets ACROSS GP-entry boundaries (a packet may be open at the end of
//!   a segment, a `LENGTH == 0` control entry carries no byte), so what the relay publishes, parsed
//!   the way the PBDMA parses it, must generate exactly the guest's methods -- for every split of a
//!   stream into segments and every placement of SYNC_WAIT.
//! * the owner's rule: correctness never depends on a clock. A refusal's finality may not wait for
//!   time to pass.
//!
//! (At 77eaf7620 three of these failed and are marked; each
//! says why; the others pin behaviour that must keep holding.)
#![allow(clippy::too_many_lines)]

use kf_abi::submit::{MethodForm, method_header_decode, method_header_inc, method_header_non_inc};
use kf_chan::user_gate::{Gate, Grace, REFUSE_RETRIES, SyncGate, Witness, witness};
use kf_chan::user_scan::{Fetch, Scan};

const SYNC: u64 = 1 << 63;
fn plain() -> u64 {
    kf_abi::submit::gp_entry(0x1000, 16).unwrap()
}
fn sync_seg() -> u64 {
    plain() | SYNC
}

// ------------------------------------------------------------------------------------------------
// the gate across a ring lap

/// ★ FAILED at 77eaf7620 (a production defect found while writing the model): `SyncGate::cleared`
/// remembers "the entry at (generation, cursor) was already waited for" so a RETRIED turn does not
/// publish a second barrier -- but it is never forgotten when the cursor moves on. After one full
/// lap of non-SYNC_WAIT entries the ring reaches the same `(generation, cursor)` with a NEW entry
/// that carries SYNC_WAIT, and the gate waves it through: the relay reads a pushbuffer the hardware
/// has not fetched yet (the original `Legacy`/`MethodOverflow` stale-read, one lap later).
#[test]
fn a_sync_wait_entry_one_ring_lap_later_is_gated_again() {
    const ENTRIES: u32 = 8;
    let mut g = SyncGate::default();
    assert_eq!(g.entry(sync_seg(), 1, 5, 0), Gate::PublishBarrier);
    g.published(2);
    assert_eq!(
        g.entry(sync_seg(), 1, 5, 2),
        Gate::Proceed,
        "witness fetched: proceed once"
    );
    // the cursor moves on through a whole lap of plain entries (no SYNC_WAIT anywhere)
    for k in 1..=ENTRIES {
        let c = (5 + k) % ENTRIES;
        if c == 5 {
            break;
        }
        assert_eq!(g.entry(plain(), 1, c, 2), Gate::Proceed);
    }
    // the same slot again, a NEW entry, SYNC_WAIT set: the hardware waits for the preceding
    // segment again, so the relay must publish a barrier again before reading it
    assert_eq!(
        g.entry(sync_seg(), 1, 5, 2),
        Gate::PublishBarrier,
        "a SYNC_WAIT entry reached one lap later was waved through on a stale `cleared` marker"
    );
}

/// The retried-turn behaviour `cleared` exists for must survive the fix: the SAME entry asked
/// twice in a row (no cursor movement in between) needs one barrier only.
#[test]
fn a_retried_turn_at_the_same_gated_entry_publishes_one_barrier() {
    let mut g = SyncGate::default();
    assert_eq!(g.entry(sync_seg(), 1, 3, 0), Gate::PublishBarrier);
    g.published(2);
    assert_eq!(g.entry(sync_seg(), 1, 3, 1), Gate::Wait);
    assert_eq!(g.entry(sync_seg(), 1, 3, 2), Gate::Proceed);
    assert_eq!(g.entry(sync_seg(), 1, 3, 2), Gate::Proceed);
    assert_eq!(g.entry(sync_seg(), 1, 3, 9), Gate::Proceed);
}

/// A `LENGTH == 0` SYNC_WAIT entry is gated like any other (the hardware waits on it too).
#[test]
fn a_zero_length_sync_wait_entry_is_gated() {
    let ctl = SYNC | 1 << 41; // LEVEL, LENGTH 0, OPCODE NOP, SYNC_WAIT
    let mut g = SyncGate::default();
    assert_eq!(g.entry(ctl, 1, 0, 0), Gate::PublishBarrier);
    g.published(2);
    assert_eq!(g.entry(ctl, 1, 0, 1), Gate::Wait);
    assert_eq!(g.entry(ctl, 1, 0, 2), Gate::Proceed);
    // and without SYNC_WAIT it is not
    assert_eq!(g.entry(1 << 41, 1, 1, 2), Gate::Proceed);
}

/// A migration starts a new generation: a SYNC_WAIT entry at cursor 0 of the new ring is gated even
/// though an entry at cursor 0 of the old ring was cleared.
#[test]
fn a_new_ring_generation_is_gated_afresh() {
    let mut g = SyncGate::default();
    assert_eq!(g.entry(sync_seg(), 1, 0, 0), Gate::PublishBarrier);
    g.published(2);
    assert_eq!(g.entry(sync_seg(), 1, 0, 2), Gate::Proceed);
    assert_eq!(g.entry(sync_seg(), 2, 0, 2), Gate::PublishBarrier);
}

// ------------------------------------------------------------------------------------------------
// refusal finality without a clock

/// Refusal finality is a COUNT of re-reads and needs no clock. (At 77eaf7620 `Grace::refused` took an
/// `Instant` and retried for 50 ms of wall clock, so with the clock frozen a refusal was never final:
/// this test could not even be written without a clock. The owner's rule: no timer carries
/// correctness.) A refusal of a segment that is not behind an unreached SYNC_WAIT is deterministic
/// (the bytes are written before the GP entry, the GP entry before `GP_PUT`, `GP_PUT` before the
/// doorbell; behind a SYNC_WAIT the gate has waited for the drain).
#[test]
fn a_refusal_becomes_final_after_a_bounded_number_of_rereads_without_any_clock() {
    let mut g = Grace::default();
    let mut retried = 0;
    while g.refused(1, 5) {
        retried += 1;
        assert!(retried <= 64, "the refusal is never final");
    }
    assert_eq!(retried, REFUSE_RETRIES);
}

/// A refusal is retried at least once (the defence in depth the measured 15/19 changed-under-the-scan
/// refusals motivated), and a different entry restarts it.
#[test]
fn a_refusal_is_retried_at_least_once_and_a_different_entry_restarts_it() {
    let mut g = Grace::default();
    assert!(g.refused(1, 5), "first refusal is retried");
    assert!(g.refused(1, 6), "another entry starts its own retry");
    assert!(g.refused(2, 6), "another ring generation too");
    g.clear();
    assert!(!g.retrying());
}

// ------------------------------------------------------------------------------------------------
// the PBDMA parse model of the relay's private ring

fn methods(segments: &[Vec<u32>]) -> Result<Vec<(u32, u32, u32)>, usize> {
    let words: Vec<u32> = segments.iter().flatten().copied().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let h = method_header_decode(words[i]).ok_or(i)?;
        i += 1;
        match h.form {
            MethodForm::Immediate => out.push((h.subchannel, h.method, h.immd)),
            MethodForm::EndPbSegment | MethodForm::SubDeviceMask => {}
            _ => {
                for k in 0..h.arg_words {
                    let Some(&v) = words.get(i) else {
                        return Ok(out); // still open at the end of what was published
                    };
                    let m = match h.form {
                        MethodForm::NonIncrementing => h.method,
                        MethodForm::IncrementOnce => h.method + if k > 0 { 4 } else { 0 },
                        _ => h.method + 4 * k as u32,
                    };
                    out.push((h.subchannel, m, v));
                    i += 1;
                }
            }
        }
    }
    Ok(out)
}

/// One guest GP entry: a segment (words, SYNC_WAIT) or a `LENGTH == 0` control entry (SYNC_WAIT).
#[derive(Clone, Debug)]
enum E {
    Seg(Vec<u32>, bool),
    Ctl(bool),
}

/// The relay's gating loop over a guest ring, transliterated from `Pump::turn` with the REAL
/// `SyncGate`, `Scan` and `Fetch`: the hardware completes the private ring at once (the
/// interleaving is the loop model's job), every entry is scanned with the carried state, a gate
/// authors a barrier and the witness `witness(carried)` picks. `ring_entries` lets the cursor wrap.
/// Returns the private ring's segments in GP order (a control entry = an empty segment).
fn relay(ring: &[E], ring_entries: u32, pick: impl Fn(bool) -> Witness) -> Vec<Vec<u32>> {
    let mut scan = Scan::default();
    let mut fetch = Fetch::new(64).unwrap();
    let mut gate = SyncGate::default();
    let mut private = Vec::new();
    let mut n = 0usize;
    let mut guard = 0;
    while n < ring.len() {
        guard += 1;
        assert!(guard < 10_000, "relay did not terminate");
        let cursor = (n as u32) % ring_entries;
        let lap = 1 + (n as u32) / ring_entries;
        let (raw, words, sync) = match &ring[n] {
            E::Seg(w, s) => (
                kf_abi::submit::gp_entry(0x1000 * u64::from(n as u32 + 1), w.len() as u64 * 4)
                    .unwrap()
                    | if *s { SYNC } else { 0 },
                Some(w),
                *s,
            ),
            E::Ctl(s) => (if *s { SYNC | 1 << 41 } else { 1 << 41 }, None, *s),
        };
        let _ = sync;
        // the generation stays 1: the ring wraps within one generation, which is exactly the
        // situation a stale `cleared` marker bites in
        let _ = lap;
        match gate.entry(raw, 1, cursor, fetch.fetched()) {
            Gate::PublishBarrier => {
                fetch.publish(None).unwrap();
                private.push(Vec::new());
                fetch.publish(None).unwrap();
                private.push(match pick(!scan.at_boundary()) {
                    Witness::Bytes => marker(),
                    Witness::Control => Vec::new(),
                });
                gate.published(fetch.issued());
            }
            Gate::Wait => {
                let get = (fetch.issued() & 63) as u32;
                fetch.observe(get).unwrap();
            }
            Gate::Proceed => {
                if let Some(w) = words {
                    assert_eq!(
                        scan.segment(w, |_| false),
                        Ok(None),
                        "the scan refused a legal stream"
                    );
                }
                let slot = fetch.publish(Some((1, cursor + 1))).unwrap();
                let _ = slot;
                private.push(words.cloned().unwrap_or_default());
                // the hardware consumes what was published (not past a witness it has not reached)
                let get = (fetch.issued() & 63) as u32;
                fetch.observe(get).unwrap();
                n += 1;
            }
        }
    }
    private
}

/// The batch marker segment (`UserStorage::write_host_marker`): `NON_STALL_INTERRUPT`, data 0.
fn marker() -> Vec<u32> {
    vec![
        method_header_inc(0, kf_abi::submit::fifo::NON_STALL_INTERRUPT, 1).unwrap(),
        0,
    ]
}
/// What the PBDMA generates from the private ring, minus the host's own marker method.
fn guest_methods(private: &[Vec<u32>]) -> Result<Vec<(u32, u32, u32)>, usize> {
    methods(private).map(|m| {
        m.into_iter()
            .filter(|m| !(m.0 == 0 && m.1 == kf_abi::submit::fifo::NON_STALL_INTERRUPT))
            .collect()
    })
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn header(form: u64, sub: u32, method: u32, count: u32) -> u32 {
    match form {
        0 => method_header_inc(sub, method, count).unwrap(),
        1 => method_header_non_inc(sub, method, count).unwrap(),
        2 => (5 << 29) | (count << 16) | (sub << 13) | (method / 4), // ONE_INC
        _ => (4 << 29) | ((count & 0x1fff) << 16) | (sub << 13) | (method / 4), // IMMD_DATA
    }
}

/// A random legal method stream: packets of every form with random subchannels, methods and counts
/// and RANDOM argument words (data that may look like a header -- the desync hazard).
fn stream(r: &mut Rng, packets: usize) -> Vec<u32> {
    let mut w = Vec::new();
    for _ in 0..packets {
        let form = r.below(4);
        let sub = r.below(5) as u32; // hardware subchannels; software ones need a binding
        let method = 0x100 + 4 * r.below(0x300) as u32;
        let count = 1 + r.below(6) as u32;
        let h = header(form, sub, method, count);
        let d = method_header_decode(h).expect("generated header decodes");
        w.push(h);
        for _ in 0..d.arg_words {
            w.push(r.next() as u32);
        }
    }
    w
}

/// Cut `words` at the positions in `cuts` (sorted, unique, inside the stream).
fn split(words: &[u32], cuts: &[usize]) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    let mut from = 0;
    for &c in cuts {
        out.push(words[from..c].to_vec());
        from = c;
    }
    out.push(words[from..].to_vec());
    out
}

fn check(name: &str, ring: &[E], ring_entries: u32) {
    let guest: Vec<Vec<u32>> = ring
        .iter()
        .map(|e| match e {
            E::Seg(w, _) => w.clone(),
            E::Ctl(_) => Vec::new(),
        })
        .collect();
    let expected = methods(&guest)
        .unwrap_or_else(|at| panic!("{name}: the guest's own stream does not decode at {at}"));
    let private = relay(ring, ring_entries, witness);
    assert_eq!(
        guest_methods(&private).as_ref(),
        Ok(&expected),
        "{name}: the PBDMA generates something other than the guest's methods (ring {ring:?}, private {private:?})"
    );
}

/// ★ EXHAUSTIVE over a stream that crosses every kind of boundary: every set of cut positions
/// (1024) x every SYNC_WAIT mask over the resulting segments. Packet splits at SYNC_WAIT in the
/// first/middle/last/back-to-back positions are all in the product.
#[test]
fn every_split_and_every_sync_wait_placement_generates_the_guests_methods() {
    let h = |sub, m, n| method_header_inc(sub, m, n).unwrap();
    let words = vec![
        h(1, 0x1b0, 3),
        0xC000_1234, // data: as a header it is SEC_OP RESERVED
        0,
        0x7fff_ffff,
        header(3, 2, 0x300, 9), // IMMD
        header(1, 0, 0x100, 2),
        0xe000_0000, // data that looks like a header
        7,
        header(2, 3, 0x400, 2),
        1,
        2,
    ];
    assert_eq!(words.len(), 11);
    let positions = words.len() - 1;
    let mut cases = 0u32;
    for cut_mask in 0u32..(1 << positions) {
        let cuts: Vec<usize> = (0..positions)
            .filter(|p| cut_mask >> p & 1 == 1)
            .map(|p| p + 1)
            .collect();
        let segs = split(&words, &cuts);
        for sync_mask in 0u32..(1 << segs.len()) {
            let ring: Vec<E> = segs
                .iter()
                .enumerate()
                .map(|(i, s)| E::Seg(s.clone(), sync_mask >> i & 1 == 1))
                .collect();
            check("exhaustive", &ring, 16);
            cases += 1;
        }
    }
    assert_eq!(cases, 2 * 3u32.pow(10));
}

/// Random streams with `LENGTH == 0` control entries (SYNC_WAIT or not) between the segments, at
/// the first and last position and back to back, over a small ring so the cursor wraps.
#[test]
fn random_splits_with_control_entries_and_ring_wrap_generate_the_guests_methods() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    for case in 0..4000 {
        let packets = 1 + r.below(5) as usize;
        let words = stream(&mut r, packets);
        let ncut = r.below(5) as usize;
        let mut cuts: Vec<usize> = Vec::new();
        if words.len() > 1 {
            for _ in 0..ncut {
                cuts.push(1 + r.below(words.len() as u64 - 1) as usize);
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        let mut ring = Vec::new();
        if r.below(3) == 0 {
            ring.push(E::Ctl(r.below(2) == 1));
        }
        for s in split(&words, &cuts) {
            ring.push(E::Seg(s, r.below(2) == 1));
            while r.below(4) == 0 {
                ring.push(E::Ctl(r.below(2) == 1)); // back to back
            }
        }
        if r.below(3) == 0 {
            ring.push(E::Ctl(true));
        }
        check(&format!("random#{case}"), &ring, 4 + 4 * r.below(3) as u32);
    }
}

/// ★ FAILED at 77eaf7620 through the gate defect above: with a small ring the SAME slot is a
/// SYNC_WAIT entry twice, a lap apart, with non-SYNC_WAIT entries between. The relay must gate
/// both: the second one, waved through, would be scanned before the hardware fetches it (in the
/// loop model: a stale read; here: no barrier is authored for it).
#[test]
fn the_same_slot_gated_twice_a_lap_apart_authors_two_barriers() {
    let ring: Vec<E> = vec![
        E::Seg(vec![method_header_inc(0, 0x100, 1).unwrap(), 1], false),
        E::Seg(vec![method_header_inc(0, 0x104, 1).unwrap(), 2], true), // slot 1, SYNC_WAIT
        E::Seg(vec![method_header_inc(0, 0x108, 1).unwrap(), 3], false),
        E::Seg(vec![method_header_inc(0, 0x10c, 1).unwrap(), 4], false),
        E::Seg(vec![method_header_inc(0, 0x110, 1).unwrap(), 5], false),
        E::Seg(vec![method_header_inc(0, 0x114, 1).unwrap(), 6], true), // slot 1 again, SYNC_WAIT
    ];
    let private = relay(&ring, 4, witness);
    let empties = private
        .iter()
        .filter(|s| s.is_empty() || **s == marker())
        .count();
    // two gated entries -> two (barrier, witness) pairs = four marker entries
    assert_eq!(empties, 4, "private ring: {private:?}");
}
