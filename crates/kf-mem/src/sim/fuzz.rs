//! ★★ **Hostile-input fuzz of the apply path** (review addendum, 2026-10-10: *the guest is
//! untrusted; a guest-reachable panic on the VA thread is a denial-of-service security bug*).
//!
//! Arbitrary — mostly hostile — walker rows (VAs, lengths, GPAs, leaf sizes including the 0 an
//! unknown page-size code yields, apertures, kinds, flags, `u64::MAX` and wrap-around values) go
//! through the REAL [`apply_entry`] and [`BatchedVas`] over the host RM model, with the guest-RAM
//! layout closure hostile too. A debug test build has `overflow-checks` on, so an unchecked
//! add/sub/mul that a guest value can reach panics here and the seed is named; the run also
//! asserts, after every entry, the ledger invariants that must hold whatever was refused: nothing
//! rigid, no pins left, no host-model violation, bounded lock holds, one verdict per run, and a
//! clean retire.

use super::*;
use crate::apply::{ApplyCfg, DiffRun, apply_entry};
use crate::ledger::{AP_PEER, AP_SYS_NONCOHERENT};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering::Relaxed;

const BASE: u64 = 0x100_0000;
/// What the run exercised (so a green run cannot be one that only ever refused).
static PLACED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static REFUSED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn pick<T: Copy>(rng: &mut Rng, xs: &[T]) -> T {
    xs[(rng.next() % xs.len() as u64) as usize]
}

/// A VA: sane (in the model window), or hostile.
fn hv(rng: &mut Rng) -> u64 {
    match rng.below(14) {
        0 => 0,
        1 => 1,
        2 => 0xFFF,
        3 => 0x1000,
        4..=6 => BASE + rng.below(96) * P,
        7 => u64::MAX,
        8 => !0xFFF,
        9 => 1 << 47,
        10 => 1 << 63,
        11 => (1 << 40) + rng.below(1 << 20) * P,
        12 => BASE + rng.below(96 * P),
        _ => rng.next(),
    }
}

/// A length: sane, or hostile.
fn hl(rng: &mut Rng) -> u64 {
    match rng.below(14) {
        0 => 0,
        1 => 1,
        2 => 0xFFF,
        3..=6 => (1 + rng.below(24)) * P,
        7 => u64::MAX,
        8 => !0xFFF,
        9 => 1 << 32,
        10 => 1 << 40,
        11 => 1 << 47,
        12 => (1 + rng.below(1 << 21)) * P,
        _ => rng.next(),
    }
}

fn hleaf(rng: &mut Rng) -> u64 {
    pick(
        rng,
        &[
            0,
            0,
            P,
            P,
            0x1_0000,
            0x20_0000,
            0x2000_0000,
            1,
            3,
            0x800,
            0x1800,
            0x4000_0000,
            1 << 40,
            u64::MAX,
            1 << 63,
        ],
    )
}

fn hap(rng: &mut Rng) -> u8 {
    match rng.below(8) {
        0 | 1 => AP_SYS_COHERENT,
        2 | 3 => crate::ledger::AP_VIDMEM,
        4 => AP_PEER,
        5 => AP_SYS_NONCOHERENT,
        _ => (rng.next() & 0xFF) as u8,
    }
}

fn hrun(rng: &mut Rng) -> DiffRun {
    let mut r = DiffRun {
        // The walker emits UNMAPs only for placements it COMMITTED (the caller below does exactly
        // that); a hostile guest cannot forge one — it can only make the MAP runs hostile.
        unmap: false,
        va: hv(rng),
        len: hl(rng),
        at: if rng.below(3) == 0 {
            rng.next()
        } else {
            hv(rng) & !0xFFF
        },
        ap: hap(rng),
        held: rng.below(100) < 10,
        kind: (rng.next() & 0xFF) as u8,
        perm: MapPerm {
            read_only: rng.below(2) == 0,
            atomic_disable: rng.below(4) == 0,
            volatile: rng.below(4) == 0,
        },
        privileged: rng.below(100) < 10,
        leaf: hleaf(rng),
    };
    // Half the rows start SANE (so the placement code is reached, not only the refusals) and have
    // ONE field made hostile, or none.
    if rng.below(2) == 0 {
        r.unmap = false;
        r.held = false;
        r.va = BASE + rng.below(90) * P;
        r.len = (1 + rng.below(12)) * P;
        r.ap = if rng.below(3) == 0 {
            crate::ledger::AP_VIDMEM
        } else {
            AP_SYS_COHERENT
        };
        r.at = rng.below(4096) * P;
        r.leaf = pick(rng, &[0, P, 0x1_0000]);
        r.privileged = false;
        match rng.below(8) {
            0 => r.va = hv(rng),
            1 => r.len = hl(rng),
            2 => r.at = hv(rng),
            3 => r.leaf = hleaf(rng),
            4 => r.ap = hap(rng),
            _ => {}
        }
    }
    r
}

/// One seed: `steps` hostile entries; `Err` names the seed and what broke.
fn run_seed(seed: u64, steps: usize, low: bool, batching: bool) -> Result<(), String> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let sim = Sim::new(space(BASE + 32 * P, BASE + 64 * P));
    sim.0.borrow_mut().no_guard = true;
    let mut m = SimMirror::new(&sim, batching, low);
    // The model's host RM scans linearly (quadratic over many mappings): keep every row's grain
    // count small so a seed is fast — the bound's code path is the same at 2^20 as at 256.
    m.bv.max_leaf_pieces = 256;
    // Hostile layouts of guest RAM: identity, nothing, a wrapping answer, a huge answer.
    let layout = rng.below(4);
    let ram = move |gpa: u64, len: u64| -> Option<u64> {
        match layout {
            0 => Some(gpa),
            1 => (gpa < 1 << 40).then_some(gpa),
            2 => Some(gpa.wrapping_add(len)),
            _ => Some(u64::MAX - len),
        }
    };
    let cfg = ApplyCfg {
        store_bytes: pick(&mut rng, &[1 << 40, 0x1_0000_0000, 0, u64::MAX]),
        grain: pick(&mut rng, &[P, P, 0x1_0000, 0, 3]),
        ram_offset: &ram,
        usermode: None,
        per_map_kind: rng.below(2) == 0,
        carve: pick(&mut rng, &[1 << 40, 0x8000_0000, 0, u64::MAX]),
        carve_refuse: rng.below(2) == 0,
    };
    let mut live: Vec<DiffRun> = Vec::new();
    for step in 0..steps {
        let n = 1 + rng.below(8) as usize;
        // The walker's MAP runs of one entry are disjoint (they are the pieces of distinct guest
        // leaves); everything ELSE about them is hostile.
        let mut runs: Vec<DiffRun> = Vec::new();
        for _ in 0..n {
            let r = hrun(&mut rng);
            let (a, b) = (r.va, r.va.saturating_add(r.len));
            let clash = runs
                .iter()
                .filter(|x| !x.unmap && !r.unmap)
                .any(|x| x.va < b && a < x.va.saturating_add(x.len));
            if clash {
                continue;
            }
            // Over a live placement the walker emits that placement's UNMAP in the same entry
            // (a remap) — it never maps over a committed placement.
            let mut k = 0;
            while k < live.len() {
                let l = live[k];
                if l.va < b && a < l.va.saturating_add(l.len) {
                    let mut u = live.swap_remove(k);
                    u.unmap = true;
                    runs.push(u);
                } else {
                    k += 1;
                }
            }
            runs.push(r);
        }
        // The walker unmaps what it committed: sometimes retire a live placement exactly.
        if !live.is_empty() && rng.below(100) < 40 {
            let k = rng.below(live.len() as u64) as usize;
            let mut u = live.swap_remove(k);
            u.unmap = true;
            runs.push(u);
        }
        let out = catch_unwind(AssertUnwindSafe(|| apply_entry(&m, &runs, &cfg)))
            .map_err(|_| format!("seed {seed} step {step}: PANIC on runs {runs:x?}"))?;
        let ctx = |what: &str| format!("seed {seed} step {step}: {what}; runs {runs:x?}");
        if out.codes.len() != runs.len() {
            return Err(ctx("not one verdict per run"));
        }
        for (r, &c) in runs.iter().zip(&out.codes) {
            if !r.unmap && c == kf_cuda::abi::KFWR_ACK_APPLIED {
                live.push(*r);
            }
            // A HELD MAP is committed too (the walker retires it with a `held` UNMAP).
            if !r.unmap && c == kf_cuda::abi::KFWR_ACK_HELD {
                live.push(DiffRun { held: true, ..*r });
            }
            // A refused UNMAP leaves its placement committed: the walker emits it again.
            if r.unmap && c == kf_cuda::abi::KFWR_ACK_FAILED {
                live.push(DiffRun { unmap: false, ..*r });
            }
        }
        PLACED.fetch_add(out.mapped as u64, std::sync::atomic::Ordering::Relaxed);
        REFUSED.fetch_add(out.refused as u64, std::sync::atomic::Ordering::Relaxed);
        // (`unsafe_splits` may count: a hostile UNMAP run with an unaligned length cuts a 4 KiB
        // mapping, which is refused by name — correct. Nothing rigid may ever exist.)
        if m.bv.rigid_seen.load(Relaxed) != 0 {
            return Err(ctx("a rigid mapping appeared"));
        }
        let (touched, _) = m.bv.hold_stats();
        if touched > crate::batch::LEDGER_CHUNK as u64 + 1 {
            return Err(ctx(&format!("a ledger hold touched {touched}")));
        }
        if m.bv.micro.lock().unwrap().values().any(|r| r.pins != 0) {
            return Err(ctx("a micro reservation is still pinned"));
        }
        // No host mapping of ours is unrecorded: every mirror mapping on the host lies inside an
        // entry of the ledger (a reservation's mappings inside the entry made through it), and
        // every ledger entry has its host mapping (nothing recorded that the host does not hold).
        {
            let own = m.bv.own.lock().unwrap();
            let rm = sim.0.borrow();
            for x in rm.maps.iter().filter(|x| x.owner == Owner::Mirror) {
                let rec = own
                    .containing(x.va)
                    .is_some_and(|(s, om)| s.saturating_add(om.len) >= x.va.saturating_add(x.len));
                if !rec {
                    return Err(ctx(&format!(
                        "host mapping {:#x}+{:#x} (hDma {:#x}) is not in the ledger",
                        x.va, x.len, x.hdma
                    )));
                }
            }
            for (v, om) in own.within(0, u64::MAX) {
                let held = rm.maps.iter().any(|x| {
                    x.owner == Owner::Mirror
                        && x.va < v.saturating_add(om.len)
                        && v < x.va.saturating_add(x.len)
                });
                if !held {
                    return Err(ctx(&format!(
                        "ledger {v:#x}+{:#x} has no host mapping behind it",
                        om.len
                    )));
                }
            }
        }
        let rm = sim.0.borrow();
        if !rm.violations.is_empty() || rm.asserts != 0 || !rm.broken_mirror().is_empty() {
            return Err(ctx(&format!(
                "host model: {:?} asserts={} broken={:?}",
                rm.violations.first(),
                rm.asserts,
                rm.broken_mirror()
            )));
        }
    }
    // The other verbs, hostile: ranges that wrap, are huge or unaligned; the steer; the views.
    for _ in 0..if std::env::var_os("KF_FUZZ_NOVERBS").is_some() {
        0
    } else {
        16
    } {
        let (va, len) = (hv(&mut rng), hl(&mut rng));
        catch_unwind(AssertUnwindSafe(|| {
            let _ = m.bv.unmap_range(va, len, true);
            let _ = m.bv.unmap_run(va, Some(len), true);
            let _ = m.bv.unmap_run(va, None, true);
            let _ = m.bv.hand_to_host(va, len);
            let _ = m.bv.own_view(va, va.saturating_add(len));
            let _ = m.bv.micro_covers(va, va.saturating_add(len));
        }))
        .map_err(|_| format!("seed {seed}: PANIC in a hostile verb at {va:#x}+{len:#x}"))?;
    }
    // The retire, at the ledger level (the model's `rows` glue is not the subject here): every
    // owned span goes by range, then the batch objects and reservations are drained.
    let refused = catch_unwind(AssertUnwindSafe(|| {
        let r = m.bv.unmap_range(0, u64::MAX, true);
        let _ = m.bv.drain();
        usize::from(r.is_err())
    }))
    .map_err(|_| format!("seed {seed}: PANIC in retire"))?;
    // A refused row here is a host-model refusal injected by nobody: there are none.
    let _ = refused;
    if m.bv.leftovers() != 0 {
        let first =
            m.bv.own
                .lock()
                .unwrap()
                .within(0, u64::MAX)
                .first()
                .map(|&(v, _)| v);
        let again = first.map(|v| m.bv.unmap_range(v, P, true));
        return Err(format!(
            "seed {seed}: refused={refused}, a second unmap of the first: {again:?}; {} of ours: own={:x?} micro={:x?} book={} stuck={:?}",
            m.bv.leftovers(),
            m.bv.own.lock().unwrap().within(0, u64::MAX),
            m.bv.micro.lock().unwrap(),
            m.bv.book.lock().unwrap().len(),
            m.bv.stuck_objects.lock().unwrap()
        ));
    }
    let rm = sim.0.borrow();
    if rm.maps.iter().any(|x| x.owner == Owner::Mirror) || !rm.resv.is_empty() {
        return Err(format!(
            "seed {seed}: a mirror mapping or reservation survived the retire"
        ));
    }
    Ok(())
}

/// Replay one seed: `KF_FUZZ_REPLAY=seed,low,batching`.
#[test]
#[ignore = "debug replay"]
fn hostile_rows_replay() {
    let v = std::env::var("KF_FUZZ_REPLAY").unwrap();
    let v: Vec<&str> = v.split(',').collect();
    let r = run_seed(v[0].parse().unwrap(), 25, v[1] == "1", v[2] == "1");
    eprintln!("{r:?}");
}

/// Hostile rows through the whole apply path, many seeds, in every placement configuration.
#[test]
fn hostile_rows_never_panic_and_keep_the_ledger_invariants() {
    let seeds: u64 = std::env::var("KF_FUZZ_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let mut failures = Vec::new();
    for (low, batching) in [(true, true), (false, true), (true, false), (false, false)] {
        let base: u64 = std::env::var("KF_FUZZ_BASE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        for seed in base.saturating_add(1)..=base.saturating_add(seeds) {
            if let Err(e) = run_seed(seed, 25, low, batching) {
                failures.push(format!("low={low} batching={batching}: {e}"));
                if failures.len() >= 5 {
                    break;
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let (placed, refused) = (
        PLACED.load(std::sync::atomic::Ordering::Relaxed),
        REFUSED.load(std::sync::atomic::Ordering::Relaxed),
    );
    eprintln!(
        "hostile-row fuzz: {seeds} seeds x 4 configs x 25 entries: {placed} rows placed, {refused} refused"
    );
    assert!(
        placed > 1000 && refused > 1000,
        "the fuzz must reach both placement and refusal"
    );
}
