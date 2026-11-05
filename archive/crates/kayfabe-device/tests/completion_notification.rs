//! ★★★ **The completion notification** (`execution_plane_increments.md` §14.18) — this
//! device announcing that an engine finished the guest's work.
//!
//! # ⊘ Why "a served doorbell raises a vector" is not the test
//!
//! Serving `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` for index 35 is a **promise**, and a
//! promise has three separate ways to be broken that all look alike from outside:
//!
//! 1. **announcing work that did not happen** — a raise on a `Served` (forwarded, finished
//!    on a host engine at an instant this device is not standing at) or on a `Refused`. That
//!    is the `#146`/E10e shape, one field over: a notification for work whose end we did not
//!    witness;
//! 2. **not announcing work that did** — a completion whose engine resolves to no vector,
//!    silently. The guest waits forever and every counter reads healthy;
//! 3. **announcing it where the guest cannot see it** — the message delivered, the pending
//!    bit latched in a leaf whose `LEAF_EN` is clear. The guest's own non-stall scan is
//!    `intrReadRegLeaf(j) & intrReadRegLeafEnSet(j)`
//!    (`ogkm-580: intr_nonstall_tu102.c:344-346`), so that vector is invisible to it — and
//!    from outside this is indistinguishable from (2).
//!
//! Every test below is written against one of those three, and the counters exist so the
//! three are distinguishable in a boot log rather than by argument.
//!
//! ★ The vector arithmetic is re-derived here from `dev_ctrl_defines.h` rather than
//! imported, on purpose: this must be the SECOND description, the one that disagrees when
//! the first one moves.

use std::sync::{Arc, Mutex};

use kayfabe_device::doorbell::{DoorbellPort, DoorbellRefused, DoorbellReport, doorbell_reg};
use kayfabe_device::plane::{ReadOutcome, RegPlane};
use kayfabe_device::{ChipProfile, FaultTag, NanoClock, SteppingClock, abi};

const BAR_REGS: u8 = kayfabe_abi::pcibars::bus_bar::REGS as u8;

/// `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` (`ogkm-580: turing/tu102/dev_vm.h:28`).
const VF: u64 = 0x00B8_0000;
/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF(i)` (`ampere/ga102/dev_vm.h:49`).
const fn leaf(i: u64) -> u64 {
    VF + 0x1000 + i * 4
}
/// `..._CPU_INTR_LEAF_EN_SET(i)` (`ga102/dev_vm.h:53`).
const fn leaf_en_set(i: u64) -> u64 {
    VF + 0x1200 + i * 4
}
/// `..._CPU_INTR_TOP(i)` (`ga102/dev_vm.h:26`).
const fn top(i: u64) -> u64 {
    VF + 0x1600 + i * 4
}
/// `..._CPU_INTR_TOP_EN_SET(i)` (`ga102/dev_vm.h:33`).
const fn top_en_set(i: u64) -> u64 {
    VF + 0x1608 + i * 4
}

/// `RM_ENGINE_TYPE_COPY0` (`ogkm-580: gpu_engine_type.h:43`).
const RM_COPY0: u32 = 9;
/// The engine the scrubber was **observed** to bind — `[measured 2026-08-08, boots
/// cebind_p35 and cup2_p35 at 5a035e0]`, `NV2080_ENGINE_TYPE` 11 = `COPY2`, identical in
/// RM space.
const RM_COPY2: u32 = RM_COPY0 + 2;
/// `MC_ENGINE_IDX_CE2` = 17's `vectorNonStall` in the captured `GA106_INTR_TABLE`, spelled
/// out here as the SECOND description — see this file's header.
const CE2_VECTOR: u32 = 0x07;
/// `MC_ENGINE_IDX_CE2` (`ogkm-580: engine_idx.h:56`), re-derived rather than imported.
const CE2_MC_IDX: u16 = 15 + 2;
/// `7 / 32`, by `NV_CTRL_INTR_GPU_VECTOR_TO_LEAF_REG` (`ogkm-580: dev_ctrl_defines.h:70`).
const CE2_LEAF: u64 = 0;
/// `7 % 32`, by `NV_CTRL_INTR_GPU_VECTOR_TO_LEAF_BIT` (`:71`).
const CE2_BIT: u32 = 7;
/// `leafReg / 2`, by `NV_CTRL_INTR_GPU_VECTOR_TO_SUBTREE` (`:77-78`).
const CE2_SUBTREE: u32 = 0;

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn plane() -> RegPlane {
    RegPlane::new(
        chip(),
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable")
}

fn db_off() -> u64 {
    doorbell_reg(chip()).expect("GA106 names a usermode register group")
}

/// What a test port answers with. ⊘ The three arms are the three the plane must treat
/// differently, and the port answers a **fixed** shape — it decodes and routes nothing.
#[derive(Debug, Clone, Copy)]
enum Answer {
    /// The shell's CPU executor really moved bytes, on a channel bound to this engine.
    LocalWith(Option<u32>),
    /// A forwarded doorbell: the work finishes on a HOST engine, elsewhere.
    Forwarded,
    /// Refused by name.
    Refused,
}

#[derive(Debug, Clone)]
struct Port {
    answer: Arc<Mutex<Answer>>,
}

impl Port {
    fn new(answer: Answer) -> Port {
        Port {
            answer: Arc::new(Mutex::new(answer)),
        }
    }
    fn set(&self, a: Answer) {
        *self.answer.lock().expect("test lock") = a;
    }
}

impl DoorbellPort for Port {
    fn ring(&self, token: u64) -> DoorbellReport {
        match *self.answer.lock().expect("test lock") {
            Answer::LocalWith(engine) => DoorbellReport::ServedLocally {
                token,
                proc: 1,
                chan: 2,
                engine,
                note: String::from("cpu-ce: 1 gp, 1 launch, 4 B, 1 sem"),
            },
            Answer::Forwarded => DoorbellReport::Served {
                token,
                proc: 1,
                chan: 2,
                host_token: 0xdead_beef,
                scheduled_now: false,
            },
            Answer::Refused => DoorbellReport::Refused {
                token,
                refusal: DoorbellRefused {
                    kind: FaultTag("Test::Refused"),
                    why: String::from("the test port refuses"),
                },
            },
        }
    }
}

/// Install `answer` and ring once; hand back the plane so its counters can be read.
fn ring_once(answer: Answer) -> RegPlane {
    let p = plane();
    p.set_doorbell(Box::new(Port::new(answer)));
    p.write(BAR_REGS, db_off(), 4, 0x0001_0002);
    p
}

// =====================================================================================
// 0. The chip's own table, read independently
// =====================================================================================

/// ⊘ The vector this file asserts everywhere is the one the **captured table** publishes,
/// not one this test typed. A table edit that moved it must redden here first, with a
/// message naming the row — otherwise every assertion below silently changes meaning.
#[test]
fn the_vector_under_test_is_the_one_this_chips_captured_table_publishes_for_ce2() {
    let row = chip()
        .intr_table
        .iter()
        .find(|e| e.engine_idx == CE2_MC_IDX)
        .expect("GA106's captured interrupt table has a CE2 row");
    assert_eq!(
        row.vector_non_stall, CE2_VECTOR,
        "MC_ENGINE_IDX {CE2_MC_IDX}'s vectorNonStall moved"
    );
    // …and the arithmetic that turns it into a place, from `dev_ctrl_defines.h:70-78`.
    assert_eq!(u64::from(CE2_VECTOR / 32), CE2_LEAF);
    assert_eq!(CE2_VECTOR % 32, CE2_BIT);
    assert_eq!((CE2_VECTOR / 32) / 2, CE2_SUBTREE);
}

// =====================================================================================
// 1. ★★★ The completion IS announced, and the guest can find it
// =====================================================================================

/// ★★★★ **THE TEST**: a copy this shell really performed, on the engine the scrubber was
/// measured to bind, raises the vector that engine's captured row publishes — and the
/// guest's own scan finds it pending.
///
/// ★ *"measured to bind"* names a run: `[measured 2026-08-08, boots `cebind_p35` and
/// `cup2_p35` at rev `5a035e0`, and `ship_7a881a7` at rev `7a881a7`]` — the device's own
/// channel-bind census printed `engineType 11 (COPY2)` in each, the last of them in the
/// shipping configuration.
///
/// ⊘ The last two assertions are not decoration. `intrGetPendingNonStall_TU102` reads
/// `TOP(0)` first and skips the whole subtree if its bit is clear
/// (`ogkm-580: intr_nonstall_tu102.c:243-248`), then reads `LEAF & LEAF_EN_SET`
/// (`:253-255`). A device that delivered the message and answered either register with
/// zero would interrupt the guest and still hang it — the worst of the three outcomes,
/// because the boot log looks identical to a healthy one.
#[test]
fn a_copy_this_shell_performed_raises_the_bound_engines_vector_and_the_guest_sees_it_pending() {
    let p = plane();
    p.set_doorbell(Box::new(Port::new(Answer::LocalWith(Some(RM_COPY2)))));

    // The guest enables the non-stall subtree and the CE's own leaf vector, which is what
    // `intrEnableTopNonstall_TU102` (`intr_nonstall_tu102.c:89-120`) and `intrEnableLeaf`
    // do. ★ Done BEFORE the ring so `nonstall_masked` can be asserted zero below — with a
    // real meaning rather than as a vacuous default.
    p.write(BAR_REGS, top_en_set(0), 4, 1 << CE2_SUBTREE);
    p.write(BAR_REGS, leaf_en_set(CE2_LEAF), 4, 1 << CE2_BIT);

    let c = p.counters();
    assert_eq!(
        (c.nonstall_raises, c.nonstall_unvectored),
        (0, 0),
        "a vector was announced by set-up, before any work was done"
    );

    let out = p.write(BAR_REGS, db_off(), 4, 0x0001_0002);
    assert!(
        out.doorbell.as_ref().expect("a doorbell").is_served(),
        "the port served it locally"
    );
    assert!(
        out.raise_cpu_intr,
        "★ the copy completed and NOTHING told the guest — this is the promise made by \
         serving notifier index 35, broken"
    );

    let c = p.counters();
    assert_eq!(c.nonstall_raises, 1, "exactly one announcement");
    assert_eq!(
        c.nonstall_unvectored, 0,
        "a completion the device could not announce"
    );
    assert_eq!(
        c.nonstall_masked, 0,
        "the guest had enabled both the subtree and the leaf, so nothing may be masked"
    );
    // ⊘ And the guest's loopback counter is UNTOUCHED. Two causes, two numbers: folding
    // them would make a boot that delivered the wrong number of vectors undiagnosable as
    // to which.
    assert_eq!(
        c.cpu_intr_raises, 0,
        "a completion was counted as the guest's own LEAF_TRIGGER self-test"
    );

    // ── now the guest's own scan, which is a different question entirely ─────────────
    let top0 = p.read(BAR_REGS, top(0), 4);
    assert!(
        matches!(top0, ReadOutcome::CpuIntr(v) if v & (1 << CE2_SUBTREE) != 0),
        "TOP(0) does not report subtree {CE2_SUBTREE} pending, so the guest's non-stall \
         scan skips the whole subtree without ever reading a leaf: {top0:?}"
    );
    let l = p.read(BAR_REGS, leaf(CE2_LEAF), 4);
    assert!(
        matches!(l, ReadOutcome::CpuIntr(v) if v & (1 << CE2_BIT) != 0),
        "LEAF({CE2_LEAF}) does not report bit {CE2_BIT} pending — the message was \
         delivered and the ISR finds nothing: {l:?}"
    );
}

/// ⊘ The vector is the **bound engine's**, not a constant. CE3 and CE4 publish different
/// ones, and a port that latched a hard-coded 0x07 would pass every test above.
#[test]
fn the_vector_follows_the_engine_the_channel_was_bound_to() {
    // CE3 → `MC_ENGINE_IDX` 18 → 0x08, which is a different BIT of the same leaf…
    let p = ring_once(Answer::LocalWith(Some(RM_COPY0 + 3)));
    let l = p.read(BAR_REGS, leaf(0), 4);
    assert!(
        matches!(l, ReadOutcome::CpuIntr(v) if v == 1 << 8),
        "CE3's vector is 0x08 and nothing else may be pending: {l:?}"
    );
    // …and CE4 → 19 → 0x0a, which is neither of the other two.
    let q = ring_once(Answer::LocalWith(Some(RM_COPY0 + 4)));
    let l = q.read(BAR_REGS, leaf(0), 4);
    assert!(
        matches!(l, ReadOutcome::CpuIntr(v) if v == 1 << 10),
        "CE4's vector is 0x0a: {l:?}"
    );
}

// =====================================================================================
// 2. ⊘ Nothing else announces anything
// =====================================================================================

/// ★★★ A **forwarded** doorbell announces nothing. Its work finishes on a host engine, at
/// an instant this device is not standing at; a vector here would be a notification for
/// work whose end we did not witness.
#[test]
fn a_forwarded_doorbell_announces_nothing_because_this_device_did_not_witness_its_end() {
    let p = ring_once(Answer::Forwarded);
    let c = p.counters();
    assert_eq!((c.doorbells, c.doorbells_served), (1, 1), "it WAS served");
    assert_eq!(
        (c.nonstall_raises, c.nonstall_unvectored, c.nonstall_masked),
        (0, 0, 0),
        "a forwarded doorbell moved a completion counter"
    );
    assert!(
        matches!(p.read(BAR_REGS, leaf(0), 4), ReadOutcome::CpuIntr(0)),
        "a forwarded doorbell latched a pending bit"
    );
}

/// ⊘ And a **refused** one is the same claim with the sign flipped.
#[test]
fn a_refused_doorbell_announces_nothing() {
    let p = ring_once(Answer::Refused);
    let c = p.counters();
    assert_eq!(c.doorbells_refused, 1);
    assert_eq!(
        (c.nonstall_raises, c.nonstall_unvectored, c.nonstall_masked),
        (0, 0, 0)
    );
}

/// ⊘ A guest write that is not a doorbell at all announces nothing — the control every
/// assertion above needs.
#[test]
fn a_write_that_is_not_a_doorbell_announces_nothing() {
    let p = plane();
    p.set_doorbell(Box::new(Port::new(Answer::LocalWith(Some(RM_COPY2)))));
    p.write(BAR_REGS, db_off() + 4, 4, 0x0001_0002);
    let c = p.counters();
    assert_eq!(c.doorbells, 0);
    assert_eq!((c.nonstall_raises, c.nonstall_unvectored), (0, 0));
}

// =====================================================================================
// 3. ★★★ A completion this device CANNOT announce is LOUD
// =====================================================================================

/// ★★★ **The number that must be zero in a healthy boot.** Work happened and nothing told
/// the guest; the three ways that can occur are three variants of
/// `kayfabe_device::nonstall::NoNonStallVector` and all three land here, because to the
/// guest they are one event.
///
/// ⊘ And `raise_cpu_intr` is **false** in every one of them. Delivering a message with
/// nothing pending sends the ISR looking for an interrupt that is not there.
#[test]
fn a_completion_with_no_vector_is_counted_loudly_and_delivers_no_message() {
    // (a) the guest never bound an engine to this channel…
    // (b) …the engine is a CE whose captured row publishes INTR_VECTOR_INVALID (CE0/CE1,
    //     the two that would have grounded the refusal on hardware's own authority)…
    // (c) …and an engine this device owns no completion moment for at all.
    for (name, engine) in [
        ("no bind", None),
        ("CE0, vectorNonStall INVALID", Some(RM_COPY0)),
        ("CE1, vectorNonStall INVALID", Some(RM_COPY0 + 1)),
        ("GR0, not a copy engine", Some(1)),
    ] {
        let p = plane();
        p.set_doorbell(Box::new(Port::new(Answer::LocalWith(engine))));
        let out = p.write(BAR_REGS, db_off(), 4, 0x0001_0002);
        assert!(
            !out.raise_cpu_intr,
            "{name}: a message was delivered with nothing pending"
        );
        let c = p.counters();
        assert_eq!(c.doorbells_served, 1, "{name}: the copy still happened");
        assert_eq!(
            (c.nonstall_raises, c.nonstall_unvectored),
            (0, 1),
            "{name}: a completion went unannounced without being counted"
        );
        assert!(
            matches!(p.read(BAR_REGS, leaf(0), 4), ReadOutcome::CpuIntr(0)),
            "{name}: something was latched anyway"
        );
    }
}

/// ★★★ **The invariant that makes a silent completion unrepresentable**: every locally
/// served doorbell is either announced or counted as unannounced, and neither can absorb
/// the other.
#[test]
fn every_local_serving_is_either_announced_or_counted_as_unannounced() {
    let port = Port::new(Answer::LocalWith(Some(RM_COPY2)));
    let p = plane();
    p.set_doorbell(Box::new(port.clone()));

    p.write(BAR_REGS, db_off(), 4, 1); // announced
    port.set(Answer::LocalWith(None));
    p.write(BAR_REGS, db_off(), 4, 2); // unvectored
    port.set(Answer::LocalWith(Some(RM_COPY0)));
    p.write(BAR_REGS, db_off(), 4, 3); // unvectored (CE0 publishes INVALID)
    port.set(Answer::LocalWith(Some(RM_COPY0 + 3)));
    p.write(BAR_REGS, db_off(), 4, 4); // announced
    port.set(Answer::Forwarded);
    p.write(BAR_REGS, db_off(), 4, 5); // neither: not a local serving
    port.set(Answer::Refused);
    p.write(BAR_REGS, db_off(), 4, 6); // neither

    let c = p.counters();
    assert_eq!(c.doorbells, 6);
    assert_eq!(c.nonstall_raises, 2);
    assert_eq!(c.nonstall_unvectored, 2);
    assert_eq!(
        c.nonstall_raises + c.nonstall_unvectored,
        4,
        "the four LOCAL servings, and only those, are accounted for"
    );
}

// =====================================================================================
// 4. ★★ Announced where the guest cannot see it — the third failure, made visible
// =====================================================================================

/// ★★ The guest's non-stall scan ANDs the leaf with `LEAF_EN_SET`
/// (`ogkm-580: intr_nonstall_tu102.c:253-255`), so a vector latched while that bit is clear
/// is **invisible to the ISR even though the message was delivered**. This device raises
/// anyway — `kayfabe_device::cpuintr`'s standing decision — and records the disagreement,
/// which is the only thing that could ever tell that hang from "we never raised".
#[test]
fn a_vector_the_guests_own_enables_would_hide_is_raised_and_recorded_as_masked() {
    let p = plane();
    p.set_doorbell(Box::new(Port::new(Answer::LocalWith(Some(RM_COPY2)))));

    // ⊘ No `LEAF_EN_SET`, no `TOP_EN_SET` — the state a real GIN would have masked in.
    let out = p.write(BAR_REGS, db_off(), 4, 0x0001_0002);
    assert!(
        out.raise_cpu_intr,
        "gating on the enables would make a bookkeeping error and a missing implementation \
         the same boot log — see kayfabe_device::cpuintr"
    );
    let c = p.counters();
    assert_eq!(c.nonstall_raises, 1);
    assert_eq!(
        c.nonstall_masked, 1,
        "★ the disagreement was not recorded, so a hung scrubber is indistinguishable \
         from a device that never raised"
    );
    // Enabling and ringing again must NOT be masked — otherwise the counter is a constant
    // rather than a reading.
    p.write(BAR_REGS, top_en_set(0), 4, 1 << CE2_SUBTREE);
    p.write(BAR_REGS, leaf_en_set(CE2_LEAF), 4, 1 << CE2_BIT);
    p.write(BAR_REGS, db_off(), 4, 0x0001_0002);
    let c = p.counters();
    assert_eq!((c.nonstall_raises, c.nonstall_masked), (2, 1));
}

// =====================================================================================
// 5. The device's own lifetime
// =====================================================================================

/// ★★★ **A pending bit must NOT survive a device reset**, and this test is a defect this
/// increment found rather than a property it inherited.
///
/// Before §14.18 the only producer of a pending bit was `_osVerifyInterrupts`' loopback,
/// which clears its own bit before returning — so `RegPlane::device_reset` left the tree
/// alone and `RegPlane::residue` recorded the reason as *"the arrays are transient"*. A
/// **completion** vector breaks that: this device latches it and only a guest that lives
/// long enough to run `_intrServiceNonStallLeaf_TU102` clears it. A guest that resets in
/// between would hand the next one a `MC_ENGINE_IDX_CE2` bit pending for a copy that never
/// happened in its life — a fabricated completion notification, across a device life.
///
/// ⊘ It is also the register block's own stated reset: `..._LEAF_VALUE_INIT` and both
/// `_EN_*_VALUE_INIT` are zero (`ogkm-580: ampere/ga102/dev_vm.h:52,56,60`).
#[test]
fn a_pending_completion_bit_does_not_survive_into_the_next_device_life() {
    let p = plane();
    p.set_doorbell(Box::new(Port::new(Answer::LocalWith(Some(RM_COPY2)))));
    p.write(BAR_REGS, db_off(), 4, 0x0001_0002);
    assert!(
        matches!(p.read(BAR_REGS, leaf(CE2_LEAF), 4), ReadOutcome::CpuIntr(v) if v == 1 << CE2_BIT),
        "the precondition: something IS pending before the reset"
    );

    p.device_reset();

    assert!(
        matches!(p.read(BAR_REGS, leaf(CE2_LEAF), 4), ReadOutcome::CpuIntr(0)),
        "★ a completion bit survived the reset — the next guest's first non-stall scan \
         finds CE2 pending for work done in somebody else's device life"
    );
    assert!(
        matches!(p.read(BAR_REGS, top(0), 4), ReadOutcome::CpuIntr(0)),
        "TOP is derived from LEAF and must have gone with it"
    );
    // ⊘ The COUNTERS are cumulative across a guest-driven reset, deliberately and unlike
    // the doorbell log: they are this QEMU process's audit, printed once at teardown, and
    // a guest that could zero them by resetting its own device could hide every completion
    // it failed to receive. `RegPlane::residue` is where a *cross-life* comparison lives.
    let c = p.counters();
    assert_eq!(
        c.nonstall_raises, 1,
        "the audit is not the guest's to erase"
    );
}
