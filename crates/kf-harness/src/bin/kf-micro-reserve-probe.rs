//! ★★★ **Micro-reservation probe — THE decision point for batching the unreserved low range**
//! (`docs/design/V3_BATCHED_MAP.md` §8.1). NOT run by this branch; the owner runs it on the GPU host.
//!
//! The question: can an UNPRIVILEGED host client reserve a SMALL fixed VA range inside
//! `[1 MiB, 4.5 GiB)` of a twin space (`NV50_MEMORY_VIRTUAL`, `FIXED_ADDRESS_ALLOCATE | LAZY |
//! VIRTUAL`, `kf_host::HostRm::reserve_va`), map a scattered batch THROUGH it, and unmap part of it
//! EXACTLY (the rest still readable by the GPU)? `[measured gfx8]` RM refused ONE LARGE reservation
//! of `[1 MiB, 4 GiB)` with `NoMemory`; in ogkm-595.84 a FIXED reservation is a FIXED `eheapAlloc`
//! (`gpu_vaspace.c:1374-1386`, `NV_ERR_NO_MEMORY` when ANY existing heap block overlaps), and the
//! only VA the client RM itself withholds in a GSP-client space is the split window
//! `[4 GiB, 4.5 GiB)` (`gpu_vaspace.c:394-467`, `g_gpu_vaspace_nvoc.h:90-91`) — so a small range
//! over FREE VA is expected to be accepted (inferred, which is what this measures).
//!
//! ```text
//! KF3_WIN_USER_CHANNELS_PASSTHROUGH=1 kf-micro-reserve-probe reserve        # the decision
//! KF3_WIN_USER_CHANNELS_PASSTHROUGH=1 kf-micro-reserve-probe nv01-control   # the root cause, on hardware
//! ```
//!
//! - `reserve` (safe): a census of small reservations at Windows-like VAs (and the old LARGE one),
//!   then: reserve 8 pages at `0x4030000`, map 8 pages of a VRAM object of ours THROUGH it as ONE
//!   mapping, unmap page 3 by range, invalidate, and CE-copy pages 0 and 7 (must deliver their
//!   pattern), re-map page 3 into the hole (must deliver), take it all down, free the reservation,
//!   and show its VA is free again. PASS ⇒ batch the low range with micro reservations
//!   (★ D3, 2026-10-10: micro reservations are now the DEFAULT — this probe is gate 10 of
//!   `scripts/bench/v3_gates.sh`). The verdict is PASS (accepted, every remnant reads), FALLBACK
//!   (host RM REFUSED the reservation: not a failure — every big-leaf row there goes at 4 KiB grain and
//!   a row beyond `MAX_LEAF_PIECES` grains is refused by name, `V3_BATCHED_MAP.md` §8.8.3) or FAIL (an
//!   inconsistency: accepted but a remnant does not read, or an error after the accept).
//! - `nv01-control` (raises ONE host Xid 31 on this test's OWN channel — a result, not a failure):
//!   the same partial unmap through the space's `NV01` range. A FIXED one-page map at the remnant's
//!   start then SUCCEEDS (⇔ RM freed the remnant's whole VA block) and a CE read of page 7 FAULTS
//!   (`FAULT_PTE`); at process exit dmesg shows `NULL != pMemBlock @ gpu_vaspace.c:1639`. That is
//!   runs 242/243 reproduced in isolation.
//!
//! ⊘ Every object is this process's own; nothing is shared with a guest.

use kf_harness::{CeRig, Ledger};
use kf_host::{HostRm, MapBacking, MapPerm};
use kf_linux_raw::{CachePolicy, DevDir, HostOffset};

const P: u64 = 0x1000;
const PAGES: u64 = 8;
const OBJ_BYTES: u64 = 0x1_0000;
const PATTERN: u32 = 0x5EED_0000;
/// The run-243 neighbourhood (`0x4034000` faulted): an unreserved Windows process VA.
const V_RESERVE: u64 = 0x0403_0000;
const V_CONTROL: u64 = 0x0503_0000;

// ★ HONESTY (review 3 item 4): this probe proves a CAPABILITY OF THE HOST DRIVER — that it accepts a small
// FIXED reservation and unmaps part of what is mapped through it exactly, or (when it refuses them all)
// that a 2 MiB leaf reservation at the flat-FB-alias base still works. It cannot detect a defect in
// `kf-mem`'s placement ladder. The 6fafcc6e failure class (the flat alias refused ⇒ every guest kernel
// CE channel poisoned) is covered by the fast suite 30/30 on hardware AND by the GPU-free model test
// `kf_mem::sim::tests::the_flat_fb_alias_survives_a_host_that_refuses_the_big_reservation`, which pins
// that 3 965 leaves of 2 MiB land through the fallback ladder inside one refresh's budget.

/// How host RM answered one reservation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// A handle.
    Accepted,
    /// A host-RM REFUSAL status (`NV_ERR_NO_MEMORY`, insufficient permissions, any other RM status):
    /// the driver said no. This is what the 4 KiB floor exists for.
    Refused,
    /// Anything else — an interrupted syscall, a mis-placed FIXED map, one of this crate's own
    /// transport/encode codes (`0x4B..`): NOT a refusal, an inconsistency, FAIL.
    Error,
}

/// Classify a reservation's result (review 2 item 1: "any `reserve_va` Err ⇒ FALLBACK" let a
/// transport error pass the gate as a clean fallback).
fn classify(r: &Result<u32, kf_host::RmError>) -> Answer {
    match r {
        Ok(_) => Answer::Accepted,
        Err(kf_host::RmError::NoMemory | kf_host::RmError::InsufficientPermissions) => {
            Answer::Refused
        }
        // NOT a refusal: the crate's own named codes (0x4B00..0x4C00: `ABI_ENCODE_FAILED` 0x4B63, …)
        // and an ioctl failure's errno, which `kf-host` (`ioctl_error`) reports as
        // `Other(0x8000_0000 | errno)` — the driver never answered. Only a status the driver
        // itself returned is a refusal.
        Err(kf_host::RmError::Other(c))
            if !(0x4B00..0x4C00).contains(c) && c & 0x8000_0000 == 0 =>
        {
            Answer::Refused
        }
        Err(_) => Answer::Error,
    }
}

/// What the census of the 5 SMALL reservations says the box is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Posture {
    /// ≥ 4 of 5 accepted: reservations are expected to work; everything must pass.
    Expected,
    /// ALL 5 refused with a host-RM refusal status: the fallback configuration — the gate then has
    /// to PROVE the flat FB alias still places (the 6fafcc6e failure class).
    AllRefused,
    /// 1-3 accepted, or any non-refusal error: inconsistent, FAIL.
    Inconsistent,
}

fn posture(small: &[Answer]) -> Posture {
    let acc = small.iter().filter(|&&a| a == Answer::Accepted).count();
    let refused = small.iter().filter(|&&a| a == Answer::Refused).count();
    if small.contains(&Answer::Error) {
        Posture::Inconsistent
    } else if acc >= 4 {
        Posture::Expected
    } else if acc == 0 && refused == small.len() && !small.is_empty() {
        Posture::AllRefused
    } else {
        Posture::Inconsistent
    }
}

/// The flat FB alias below the carve-out in 2 MiB leaves (`0x1efbe0000 / 2 MiB`), and whether the
/// per-leaf reservation tier (2 amplified host calls per leaf) fits one refresh's amplification
/// budget — the production ladder's bound (`kf_mem::batch::REFRESH_AMPLIFICATION_BUDGET`).
const ALIAS_LEAVES: u64 = 0x1_efbe_0000 / 0x20_0000;

fn ladder_fits_budget(leaves: u64) -> bool {
    leaves.saturating_mul(2) <= kf_mem::batch::REFRESH_AMPLIFICATION_BUDGET
}

fn word(page: u64, w: u64) -> u32 {
    PATTERN | ((page as u32) << 8) | (w as u32 & 0xFF)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arm = args.get(1).map_or("reserve", String::as_str);
    let mut l = Ledger::default();
    println!("MICRO_RESERVE_START arm={arm} pid={}", std::process::id());
    // ★ Gate 10 (2026-10-10, review item 6): a reservation host RM REFUSES is not a failure — the
    // batched map falls back to the 4 KiB grain (`V3_BATCHED_MAP.md` §8.8.3) — so it is a distinct,
    // loudly printed verdict, FALLBACK. FAIL is an INCONSISTENCY: a reservation host RM accepted
    // whose remnants do not read, or any error after the accept.
    let fallback = match run(&mut l, arm) {
        Ok(f) => f,
        Err(e) => {
            l.check("run", false, e);
            false
        }
    };
    let v = l.verdict();
    let word = match (v, fallback) {
        (false, _) => "FAIL",
        (true, true) => "FALLBACK",
        (true, false) => "PASS",
    };
    if word == "FALLBACK" {
        println!(
            "MICRO_RESERVE_FALLBACK_ACTIVE host RM refused the small FIXED reservation: the batched map runs on the per-leaf / 4 KiB floor on this driver and the flat FB alias was PROVEN placeable there (V3_BATCHED_MAP.md 8.8.3, 8.8.7)"
        );
    }
    println!("MICRO_RESERVE_VERDICT arm={arm} {word}");
    std::process::exit(i32::from(!v));
}

#[allow(clippy::too_many_lines)]
fn run(l: &mut Ledger, arm: &str) -> Result<bool, String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    l.measure("session", format!("driver {}", rm.driver_version()));
    l.measure(
        "space_start",
        format!("{:?}", kf_host::channel::MirrorVaStart::from_env()),
    );
    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace: {e:?}"))?;

    // ── the census: small FIXED reservations at Windows-like VAs, and the old LARGE one ──────
    let census: [(u64, u64, &str); 6] = [
        (0x0040_0000, 0x8000, "4 MiB, 8 pages"),
        (V_RESERVE, 0x8000, "run-243 VA, 8 pages"),
        (0x1453_0000, 0x1_0000, "0x14530000, 64 KiB"),
        (0x8000_0000, 0x20_0000, "2 GiB, 2 MiB"),
        (0xFFE0_0000, 0x20_0000, "just below 4 GiB, 2 MiB"),
        (
            0x10_0000,
            (4 << 30) - 0x10_0000,
            "LARGE [1 MiB, 4 GiB) (the gfx8 refusal)",
        ),
    ];
    let mut answers: Vec<Answer> = Vec::new();
    for (va, len, what) in census {
        let r = rm.reserve_va(space.space, va, len);
        let a = classify(&r);
        answers.push(a);
        match &r {
            Ok(h) => {
                l.measure(
                    "reserve_census",
                    format!("{what} {va:#x}+{len:#x}: ACCEPTED"),
                );
                let _ = rm.free(*h);
            }
            Err(e) => l.measure(
                "reserve_census",
                format!("{what} {va:#x}+{len:#x}: {a:?} {e:?}"),
            ),
        }
    }
    // The 5 small ones decide (the 6th, the old LARGE one, is information).
    let small = &answers[..5];
    let post = posture(small);
    let accepted = answers.iter().filter(|&&a| a == Answer::Accepted).count();
    l.check(
        "reserve_small_accepted",
        post != Posture::Inconsistent,
        format!(
            "{accepted}/6 accepted, small ones {small:?} ⇒ {post:?} (Expected = ≥ 4 of 5 accepted; AllRefused = all 5 refused with a host-RM refusal status; anything else, a transport error included, FAILS)"
        ),
    );
    // ★ D1 (2026-10-10), INFORMATION ONLY (never gates): the flat FB alias of a guest-KERNEL space is
    // one 7.9 GiB row of 2 MiB leaves (`0x120000000+0x1efc00000`), which can ONLY be placed through
    // a reservation (2 031 616 grains > MAX_LEAF_PIECES). If host RM refuses this one, the placement
    // falls back to one reservation per 2 MiB leaf (`kf_mem::batch`, (a2)) — that fallback is
    // model-tested, this line is what says whether it is ever needed.
    {
        const ALIAS: (u64, u64) = (0x1_2000_0000, 0x1_efc0_0000);
        match rm.reserve_va(space.space, ALIAS.0, ALIAS.1) {
            Ok(h) => {
                l.measure(
                    "reserve_flat_fb_alias",
                    format!("{:#x}+{:#x} (7.9 GiB): ACCEPTED", ALIAS.0, ALIAS.1),
                );
                let _ = rm.free(h);
            }
            Err(e) => l.measure(
                "reserve_flat_fb_alias",
                format!(
                    "{:#x}+{:#x} (7.9 GiB): refused {e:?} — the per-2-MiB-leaf fallback applies",
                    ALIAS.0, ALIAS.1
                ),
            ),
        }
    }

    // ── the objects: a VRAM source of 8 patterned pages, a VRAM destination, a CE rig ────────
    let src = rm
        .alloc_device_local(OBJ_BYTES)
        .map_err(|e| format!("src: {e:?}"))?;
    let (_sn, src_cpu) = rm
        .map_cpu(src, OBJ_BYTES, CachePolicy::Uncached)
        .map_err(|e| format!("cpu src: {e:?}"))?;
    for p in 0..PAGES {
        for w in 0..(P / 4) {
            src_cpu
                .store_u32(HostOffset::new(p * P + w * 4), word(p, w))
                .map_err(|e| format!("fill: {e:?}"))?;
        }
    }
    let dst = rm
        .alloc_device_local(OBJ_BYTES)
        .map_err(|e| format!("dst: {e:?}"))?;
    let dst_va = rm
        .map(space, dst, MapBacking::Dedicated, 0, OBJ_BYTES, None, false)
        .map_err(|e| format!("map dst: {e:?}"))?;
    let (_dn, dst_cpu) = rm
        .map_cpu(dst, OBJ_BYTES, CachePolicy::Uncached)
        .map_err(|e| format!("cpu dst: {e:?}"))?;
    let mut rig = CeRig::new(&rm, space)?;
    // Copy one page from `va` into dst page `slot` and say whether page `page`'s pattern arrived.
    let mut read = |rm: &HostRm, va: u64, slot: u64, page: u64| -> Result<bool, String> {
        for w in 0..4 {
            dst_cpu
                .store_u32(HostOffset::new(slot * P + w * 4), 0xDEAD_0000)
                .map_err(|e| format!("stamp: {e:?}"))?;
        }
        let s = rig.copy(rm, va, dst_va + slot * P, P as u32)?;
        let got = dst_cpu
            .load_u32(HostOffset::new(slot * P + 4))
            .map_err(|e| format!("read: {e:?}"))?;
        Ok(s.ce_released && got == word(page, 1))
    };

    match arm {
        "reserve" => {
            if post == Posture::AllRefused {
                // ★ Review 2 item 1 — THE FALLBACK CONFIGURATION: every small reservation was
                // refused by host RM. The batched map then runs on the 4 KiB floor, and the one
                // row that CANNOT be placed on the floor is the guest-kernel flat FB alias
                // (2 030 080 grains > 2^20 per row; the 6fafcc6e fast-suite 0/30). So the gate
                // PROVES the alias still places by the production ladder's last tier — one
                // reservation per 2 MiB leaf — or FAILS, loudly: reserve a 2 MiB leaf at the
                // alias base, map 2 MiB of VRAM through it, and read its first and last page
                // through the CE; and the 3 965 leaves × 2 amplified calls must fit one refresh's
                // budget.
                println!(
                    "MICRO_RESERVE_FALLBACK_CONFIGURATION all 5 small reservations were refused by host RM: proving the flat FB alias places via per-2-MiB-leaf reservations"
                );
                let leaf = 0x20_0000u64;
                let alias_va = 0x1_2000_0000u64;
                let ok_budget = ladder_fits_budget(ALIAS_LEAVES);
                l.check(
                    "fallback_alias_ladder_fits_the_budget",
                    ok_budget,
                    format!(
                        "{ALIAS_LEAVES} leaves x 2 calls vs REFRESH_AMPLIFICATION_BUDGET {}",
                        kf_mem::batch::REFRESH_AMPLIFICATION_BUDGET
                    ),
                );
                let lr = rm.reserve_va(space.space, alias_va, leaf);
                let la = classify(&lr);
                let proven = match lr {
                    Ok(h2) => {
                        let big = rm
                            .alloc_device_local(leaf)
                            .map_err(|e| format!("2 MiB leaf object: {e:?}"))?;
                        let (_bn, big_cpu) = rm
                            .map_cpu(big, leaf, CachePolicy::Uncached)
                            .map_err(|e| format!("cpu 2 MiB leaf: {e:?}"))?;
                        let last = leaf / P - 1;
                        for page in [0, last] {
                            for w in 0..(P / 4) {
                                big_cpu
                                    .store_u32(HostOffset::new(page * P + w * 4), word(page, w))
                                    .map_err(|e| format!("fill 2 MiB leaf: {e:?}"))?;
                            }
                        }
                        rm.map_in(
                            h2,
                            big,
                            MapBacking::SharedSlice,
                            0,
                            leaf,
                            alias_va,
                            false,
                            0,
                            MapPerm::READ_WRITE,
                        )
                        .map_err(|e| format!("map the 2 MiB leaf through its reservation: {e:?}"))?;
                        let first = read(&rm, alias_va, 4, 0)?;
                        let end = read(&rm, alias_va + last * P, 5, last)?;
                        rm.unmap_in(h2, alias_va, leaf, false)
                            .map_err(|e| format!("teardown 2 MiB leaf: {e:?}"))?;
                        rm.free(h2).map_err(|e| format!("free 2 MiB leaf: {e:?}"))?;
                        first && end
                    }
                    Err(e) => {
                        println!(
                            "MICRO_RESERVE_FLAT_ALIAS_UNPLACEABLE a 2 MiB leaf reservation at the flat FB alias base is refused too ({la:?} {e:?}): in this configuration the 7.9 GiB alias row is refused by name and every guest kernel CE channel would be poisoned — the 6fafcc6e failure class (fast suite 0/30)"
                        );
                        false
                    }
                };
                l.check(
                    "fallback_flat_fb_alias_placeable",
                    proven,
                    "a 2 MiB leaf reserved at the alias base, mapped and read through the CE (first and last page)",
                );
                return Ok(proven && ok_budget);
            }
            if post == Posture::Inconsistent {
                // `reserve_small_accepted` already failed; nothing more is learned by going on.
                return Ok(false);
            }
            // Reservations are expected: this one MUST be accepted (a refusal here, with the
            // census accepting, is an inconsistency — FAIL, never a fallback).
            let h = rm
                .reserve_va(space.space, V_RESERVE, PAGES * P)
                .map_err(|e| format!("reserve {V_RESERVE:#x} after the census accepted: {e:?}"))?;
            rm.map_in(
                h,
                src,
                MapBacking::SharedSlice,
                0,
                PAGES * P,
                V_RESERVE,
                false,
                0,
                MapPerm::READ_WRITE,
            )
            .map_err(|e| format!("map through the reservation: {e:?}"))?;
            l.check(
                "whole_read_before",
                read(&rm, V_RESERVE + 7 * P, 0, 7)?,
                "page 7 via the reservation",
            );
            rm.unmap_in(h, V_RESERVE + 3 * P, P, false)
                .map_err(|e| format!("partial unmap: {e:?}"))?;
            rm.invalidate_tlb(space)
                .map_err(|e| format!("invalidate: {e:?}"))?;
            l.check(
                "remnant_left_reads",
                read(&rm, V_RESERVE, 1, 0)?,
                "page 0 after unmapping page 3",
            );
            l.check(
                "remnant_right_reads",
                read(&rm, V_RESERVE + 7 * P, 2, 7)?,
                "page 7 after unmapping page 3",
            );
            rm.map_in(
                h,
                src,
                MapBacking::SharedSlice,
                3 * P,
                P,
                V_RESERVE + 3 * P,
                false,
                0,
                MapPerm::READ_WRITE,
            )
            .map_err(|e| format!("re-map into the hole: {e:?}"))?;
            rm.invalidate_tlb(space)
                .map_err(|e| format!("invalidate: {e:?}"))?;
            l.check(
                "hole_remapped_reads",
                read(&rm, V_RESERVE + 3 * P, 3, 3)?,
                "page 3 re-mapped",
            );
            rm.unmap_in(h, V_RESERVE, PAGES * P, false)
                .map_err(|e| format!("teardown range: {e:?}"))?;
            rm.free(h).map_err(|e| format!("free reservation: {e:?}"))?;
            let back = rm.map_kind(
                space,
                src,
                MapBacking::SharedSlice,
                0,
                P,
                Some(V_RESERVE),
                false,
                0,
                MapPerm::READ_WRITE,
            );
            l.check(
                "va_free_after",
                back.is_ok(),
                format!("NV01 map at the freed VA: {back:?}"),
            );
            Ok(false)
        }
        "nv01-control" => {
            rm.map_kind(
                space,
                src,
                MapBacking::SharedSlice,
                0,
                PAGES * P,
                Some(V_CONTROL),
                false,
                0,
                MapPerm::READ_WRITE,
            )
            .map_err(|e| format!("map through the NV01 range: {e:?}"))?;
            l.check(
                "control_whole_read_before",
                read(&rm, V_CONTROL + 7 * P, 0, 7)?,
                "page 7 before",
            );
            rm.unmap_range(space, V_CONTROL + 3 * P, P, false)
                .map_err(|e| format!("partial unmap: {e:?}"))?;
            rm.invalidate_tlb(space)
                .map_err(|e| format!("invalidate: {e:?}"))?;
            // Signature 1: the remnant's VA block is free (a FIXED map at its start lands).
            let probe = rm.map_kind(
                space,
                src,
                MapBacking::SharedSlice,
                0,
                P,
                Some(V_CONTROL),
                false,
                0,
                MapPerm::READ_WRITE,
            );
            l.measure("control_probe_map_at_remnant", format!("{probe:?}"));
            l.check(
                "nv01_split_freed_the_block",
                probe.is_ok(),
                "Ok ⇔ RM freed the remnant's VA block (the root cause)",
            );
            if probe.is_ok() {
                let _ = rm.unmap(space, V_CONTROL, false);
            }
            // Signature 2: page 7 (a remnant RM still lists) no longer reads — EXPECTED Xid 31 on
            // this test's own channel; the copy's semaphore never lands.
            let r7 = read(&rm, V_CONTROL + 7 * P, 1, 7).unwrap_or(false);
            l.measure("control_remnant_read", format!("{r7}"));
            l.check(
                "nv01_remnant_lost",
                !r7,
                "a remnant of a split NV01 mapping faults (expected)",
            );
            println!(
                "MICRO_RESERVE_NOTE check dmesg: Xid 31 FAULT_PTE at {:#x}, and at exit NULL != pMemBlock @ gpu_vaspace.c:1639",
                V_CONTROL + 7 * P
            );
            Ok(false)
        }
        other => Err(format!("unknown arm {other:?} (reserve | nv01-control)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_host::RmError;

    #[test]
    fn only_a_host_rm_refusal_status_is_a_refusal() {
        assert_eq!(classify(&Ok(1)), Answer::Accepted);
        assert_eq!(classify(&Err(RmError::NoMemory)), Answer::Refused);
        assert_eq!(classify(&Err(RmError::InsufficientPermissions)), Answer::Refused);
        // An RM status the driver returned (NV_ERR_INVALID_ARGUMENT 0x1F, NV_ERR_NOT_SUPPORTED 0x56…).
        assert_eq!(classify(&Err(RmError::Other(0x1F))), Answer::Refused);
        // This crate's own codes, a cancelled syscall and a mis-placed FIXED map are not refusals.
        for e in [
            RmError::Other(kf_host::ABI_ENCODE_FAILED),
            RmError::Other(kf_host::IOCTL_NUMBER_UNBUILDABLE),
            RmError::Other(kf_host::ABI_DECODE_FAILED),
            RmError::Other(kf_host::NOT_ON_THIS_RUNG),
            // An ioctl failure: errno EINVAL (22) / ENOMEM (12) / EIO (5) — transport, not a refusal.
            RmError::Other(0x8000_0000 | 22),
            RmError::Other(0x8000_0000 | 12),
            RmError::Other(0x8000_0000 | 5),
            RmError::Interrupted,
            RmError::PlacementRefused { want: 1, got: 2 },
        ] {
            assert_eq!(classify(&Err(e)), Answer::Error, "{e:?}");
        }
    }

    #[test]
    fn the_fallback_posture_needs_all_five_small_refused_by_status() {
        use Answer::*;
        assert_eq!(posture(&[Accepted; 5]), Posture::Expected);
        assert_eq!(posture(&[Accepted, Accepted, Accepted, Accepted, Refused]), Posture::Expected);
        assert_eq!(posture(&[Refused; 5]), Posture::AllRefused);
        // Mixed (1-3 accepted): inconsistent.
        for k in 1..4 {
            let mut v = [Refused; 5];
            v[..k].fill(Accepted);
            assert_eq!(posture(&v), Posture::Inconsistent, "{k} accepted");
        }
        // One transport error among refusals (or acceptances) is not a fallback.
        assert_eq!(posture(&[Refused, Refused, Refused, Refused, Error]), Posture::Inconsistent);
        assert_eq!(posture(&[Accepted, Accepted, Accepted, Accepted, Error]), Posture::Inconsistent);
        assert_eq!(posture(&[]), Posture::Inconsistent);
    }

    #[test]
    fn the_alias_ladder_fits_one_refreshs_budget_and_a_4_gib_row_does_not() {
        assert!(ladder_fits_budget(ALIAS_LEAVES), "{ALIAS_LEAVES} leaves");
        assert_eq!(ALIAS_LEAVES, 3965);
        // A row that needs a million per-unit reservations (a 4 GiB row of 4 KiB units) does not.
        assert!(!ladder_fits_budget(1 << 20));
    }
}
