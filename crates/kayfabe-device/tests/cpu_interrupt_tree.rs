//! ★★★ **The CPU interrupt tree (`#151`)** — the block whose absence produced
//! `RmInitAdapter failed! (0x11:0x45:2134)`, tested as `_osVerifyInterrupts` drives it.
//!
//! # ⊘ Why "a trigger write raises a vector" is not the test
//!
//! `_osVerifyInterrupts` (`ogkm-580: src/nvidia/src/kernel/os/os_sanity.c:117-291`) is a
//! **closed loop with three separate places to lose**, and a device that gets one of them
//! right fails identically to one that got none:
//!
//! 1. the trigger write must be **claimed** — a write that falls through to the plane's
//!    unclaimed arm is silently dropped, and the guest spins for 4.3 s;
//! 2. a vector must be **raised** — and exactly one, because the guest counts nothing but
//!    still has an ISR that must not be entered for an interrupt that is not there;
//! 3. `CPU_INTR_LEAF(reg)` must **read back pending** when the ISR looks
//!    (`intrIsVectorPending_TU102`, `intr_tu102.c:729-744`). A device that delivers the
//!    message and answers the leaf with zero fails the test *having interrupted the guest*,
//!    which is the worst of the three outcomes because the boot log looks the same.
//!
//! ★ So every test here is written against the mechanism the guest's own code depends on,
//! transcribed from `intr_swintr_tu102.c` and `intr_tu102.c` rather than from this port's
//! implementation. ⊘ The offsets and the vector are re-derived in this file on purpose:
//! this must be the SECOND description, the one that disagrees when the first one moves.

use kayfabe_device::cpuintr;
use kayfabe_device::plane::{ReadOutcome, RegPlane};
use kayfabe_device::{NanoClock, SteppingClock, abi};

/// `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` (`ogkm-580: turing/tu102/dev_vm.h:28`),
/// selected for a physical function by `kern_gpu_tu102.c:92-101`.
const VF: u64 = 0x00B8_0000;
/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF(i)` (`ampere/ga102/dev_vm.h:49`).
const fn leaf(i: u64) -> u64 {
    VF + 0x1000 + i * 4
}
/// `..._CPU_INTR_LEAF_EN_SET(i)` (`ga102/dev_vm.h:53`).
const fn leaf_en_set(i: u64) -> u64 {
    VF + 0x1200 + i * 4
}
/// `..._CPU_INTR_LEAF_EN_CLEAR(i)` (`ga102/dev_vm.h:57`).
const fn leaf_en_clear(i: u64) -> u64 {
    VF + 0x1400 + i * 4
}
/// `..._CPU_INTR_TOP(i)` (`ga102/dev_vm.h:26`).
const fn top(i: u64) -> u64 {
    VF + 0x1600 + i * 4
}
/// `..._CPU_INTR_TOP_EN_SET(i)` (`ga102/dev_vm.h:33`).
const fn top_en_set(i: u64) -> u64 {
    VF + 0x1608 + i * 4
}
/// `..._CPU_INTR_TOP_EN_CLEAR(i)` (`ga102/dev_vm.h:41`).
const fn top_en_clear(i: u64) -> u64 {
    VF + 0x1610 + i * 4
}
/// `..._CPU_INTR_LEAF_TRIGGER` (`ga102/dev_vm.h:61`).
const TRIGGER: u64 = VF + 0x1640;

/// `NV_CTRL_CPU_DOORBELL_VECTORID_VALUE_CONSTANT` (`ogkm-580: turing/tu102/dev_ctrl.h:35`).
const DOORBELL: u64 = 129;
/// `129 / 32`, by `NV_CTRL_INTR_GPU_VECTOR_TO_LEAF_REG` (`dev_ctrl_defines.h:70`).
const DB_LEAF: u64 = 4;
/// `129 % 32`, by `NV_CTRL_INTR_GPU_VECTOR_TO_LEAF_BIT` (`dev_ctrl_defines.h:71`).
const DB_BIT: u32 = 1;
/// `leafReg / 2`, by `NV_CTRL_INTR_GPU_VECTOR_TO_SUBTREE` (`dev_ctrl_defines.h:77-78`).
const DB_SUBTREE: u32 = 2;

/// BAR index of the register aperture.
const REGS: u8 = kayfabe_abi::pcibars::bus_bar::REGS as u8;

fn plane() -> RegPlane {
    RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable")
}

/// ★★★ **THE WHOLE TEST: `_osVerifyInterrupts`, replayed write for write.**
///
/// Every access below is one HAL call in `os_sanity.c:212-249`, in the order the driver
/// makes them, and nothing else is asserted until the sequence is complete — because the
/// driver's own verdict is a single boolean read at the end of it.
#[test]
fn the_drivers_own_loopback_self_test_completes() {
    let p = plane();

    // `intrClearStallSWIntr_TU102` → `intrClearLeafVector` → write-1-to-clear the vector's
    // bit (`intr_swintr_tu102.c:56-68`, `intr_tu102.c:648-663`).
    p.write(REGS, leaf(DB_LEAF), 4, 1 << DB_BIT);
    // `intrEnableStallSWIntr_TU102` (`intr_swintr_tu102.c:72-90`): the leaf enable, then
    // the subtree's top enable.
    p.write(REGS, leaf_en_set(DB_LEAF), 4, 1 << DB_BIT);
    p.write(REGS, top_en_set(0), 4, 1 << DB_SUBTREE);

    // Nothing has been asked for yet. ⚠ Asserted, not assumed: a device that raised on the
    // ENABLE would pass every other assertion in this file and interrupt the guest before
    // its ISR was diverted to `osSanityTestIsr` (`os_sanity.c:232`).
    assert_eq!(
        p.counters().cpu_intr_raises,
        0,
        "a vector was raised by set-up, before the driver asked for one"
    );

    // `intrSetStallSWIntr_TU102` (`intr_swintr_tu102.c:40-51`) — the one write that is the
    // whole request.
    let w = p.write(REGS, TRIGGER, 4, DOORBELL);
    assert!(
        w.claimed,
        "★ CPU_INTR_LEAF_TRIGGER fell through to the unclaimed arm. That write is the \
         ENTIRE request; dropped, the guest spins for its full 4.3 s timeout and reports \
         NV_ERR_IRQ_NOT_FIRING with nothing else wrong."
    );
    assert!(
        w.raise_cpu_intr,
        "the trigger was claimed and no vector was asked for — the guest is never woken"
    );
    assert_eq!(p.counters().cpu_intr_raises, 1, "exactly one vector");

    // ── now the ISR, which is a different question entirely ──────────────────────────
    //
    // `osWaitForInterrupt` → `intrGetStallInterruptMode_TU102` → `intrIsVectorPending_TU102`
    // reads the LEAF register and tests the bit (`intr_tu102.c:737-743`). ★ This is the
    // assertion that catches "we delivered the message and answered the leaf with zero",
    // which fails the driver's test having already interrupted the guest.
    let pending = p.read(REGS, leaf(DB_LEAF), 4);
    assert!(
        matches!(pending, ReadOutcome::CpuIntr(_)),
        "the ISR's own read was not served by the interrupt tree: {pending:?}"
    );
    assert_eq!(
        pending.value() & (1 << DB_BIT),
        1 << DB_BIT,
        "★ the vector is not pending in CPU_INTR_LEAF({DB_LEAF}); intrIsVectorPending \
         returns FALSE and the self-test fails even though a vector was delivered"
    );
    // The summary the ISR reads first, on its way to the leaf.
    assert_eq!(
        p.read(REGS, top(0), 4).value() & (1 << DB_SUBTREE),
        1 << DB_SUBTREE,
        "CPU_INTR_TOP does not report the subtree the pending leaf belongs to"
    );

    // `osWaitForInterrupt` clears before it sets `interrupt_triggered` (`os_sanity.c:92`).
    p.write(REGS, leaf(DB_LEAF), 4, 1 << DB_BIT);
    assert_eq!(
        p.read(REGS, leaf(DB_LEAF), 4).value(),
        0,
        "write-1-to-clear did not clear the vector"
    );
    assert_eq!(
        p.read(REGS, top(0), 4).value(),
        0,
        "★ CPU_INTR_TOP still reports a pending subtree over an EMPTY leaf pair. The ISR \
         walks TOP to find leaves; a stale summary bit is an ISR that finds nothing, every \
         time, forever."
    );
}

/// ★★★ The evidence that decides whether gating would have been safe.
///
/// This port raises unconditionally and records the disagreement instead — see
/// `kayfabe_device::cpuintr`'s header for why. ⊘ That decision is only defensible while
/// somebody can check it, so this pins the meaning of the counter in both directions.
#[test]
fn the_masking_a_gating_model_would_have_applied_is_recorded_both_ways() {
    // The driver's real sequence enables first, so nothing is masked.
    let p = plane();
    p.write(REGS, leaf_en_set(DB_LEAF), 4, 1 << DB_BIT);
    p.write(REGS, top_en_set(0), 4, 1 << DB_SUBTREE);
    p.write(REGS, TRIGGER, 4, DOORBELL);
    assert_eq!(p.counters().cpu_intr_raises, 1);
    assert_eq!(
        p.counters().cpu_intr_masked,
        0,
        "★ the driver's own enable sequence read as MASKED. Either the enable bookkeeping \
         or the vector arithmetic is wrong, and gating on it would have cost the adapter."
    );

    // A trigger with the leaf enable withdrawn: silicon would have swallowed it.
    let q = plane();
    q.write(REGS, top_en_set(0), 4, 1 << DB_SUBTREE);
    q.write(REGS, leaf_en_clear(DB_LEAF), 4, 1 << DB_BIT);
    q.write(REGS, TRIGGER, 4, DOORBELL);
    assert_eq!(
        q.counters().cpu_intr_raises,
        1,
        "this device raises anyway, by decision — see cpuintr's header"
    );
    assert_eq!(
        q.counters().cpu_intr_masked,
        1,
        "the disagreement was not recorded, so nothing could ever justify turning gating on"
    );

    // And with the subtree disabled at the top instead — the other half of the gate.
    let r = plane();
    r.write(REGS, leaf_en_set(DB_LEAF), 4, 1 << DB_BIT);
    r.write(REGS, TRIGGER, 4, DOORBELL);
    assert_eq!(
        r.counters().cpu_intr_masked,
        1,
        "a subtree that was never top-enabled did not count as masked"
    );
}

/// ⊘ A vector naming a leaf row this chip does not have raises **nothing**.
///
/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF__SIZE_1` is 8 (`ga102/dev_vm.h:50`), so vectors
/// at or above 256 have no row. ★ The failure to avoid is not a panic — it is delivering a
/// message with no pending bit behind it, which sends the guest's ISR looking for an
/// interrupt that does not exist and gets it counted as spurious by its own kernel.
#[test]
fn a_vector_outside_the_declared_rows_latches_nothing_and_raises_nothing() {
    let p = plane();
    // 0xFFF is the widest `LEAF_TRIGGER_VECTOR` field there is (`ga102/dev_vm.h:62`).
    let w = p.write(REGS, TRIGGER, 4, 0xFFF);
    assert!(w.claimed, "the register is ours whatever vector is written");
    assert!(
        !w.raise_cpu_intr,
        "a message with nothing pending behind it"
    );
    assert_eq!(p.counters().cpu_intr_raises, 0);
    assert_eq!(
        p.counters().cpu_intr_accesses,
        1,
        "an out-of-range trigger is still an access and must still be visible"
    );
    for i in 0..8 {
        assert_eq!(p.read(REGS, leaf(i), 4).value(), 0, "leaf {i} was latched");
    }
    assert_eq!(p.read(REGS, top(0), 4).value(), 0);
}

/// ★★ Write-1-to-clear clears **one** bit, not the register.
///
/// `intrClearLeafVector_TU102` writes `NVBIT(bit)` (`intr_tu102.c:648-663`). ⊘ A plain
/// store would clear every other pending vector in the same leaf as a side effect of
/// clearing one, and the symptom is a lost interrupt in a subsystem nobody was watching —
/// never a failure at the site of the bug.
#[test]
fn clearing_one_vector_does_not_clear_its_neighbours() {
    let p = plane();
    // 128 and 129 share leaf 4; 130 too.
    for v in [128u64, 129, 130] {
        p.write(REGS, TRIGGER, 4, v);
    }
    assert_eq!(p.read(REGS, leaf(4), 4).value(), 0b111);
    p.write(REGS, leaf(4), 4, 1 << 1);
    assert_eq!(
        p.read(REGS, leaf(4), 4).value(),
        0b101,
        "clearing vector 129 disturbed 128 or 130"
    );
    assert_eq!(
        p.read(REGS, top(0), 4).value() & (1 << DB_SUBTREE),
        1 << DB_SUBTREE,
        "the subtree went quiet while two of its vectors were still pending"
    );
}

/// The register block's own read/write declarations, honoured.
///
/// ⊘ `CPU_INTR_TOP` is `R--4A` (`ga102/dev_vm.h:26`) and `CPU_INTR_LEAF_TRIGGER` is `-W-4R`
/// (`:61`). Accepting a write to the first would let the guest desynchronise TOP from LEAF
/// — and the ISR reads both. Answering the second with the last vector written would be
/// inventing a readable field on a write-only register.
#[test]
fn the_read_only_and_write_only_registers_are_both() {
    let p = plane();
    p.write(REGS, TRIGGER, 4, DOORBELL);
    assert_eq!(
        p.read(REGS, TRIGGER, 4).value(),
        0,
        "the write-only trigger answered with something"
    );

    let before = p.read(REGS, top(0), 4).value();
    p.write(REGS, top(0), 4, 0xFFFF_FFFF);
    assert_eq!(
        p.read(REGS, top(0), 4).value(),
        before,
        "a write to the read-only TOP register was taken; TOP can now disagree with LEAF"
    );
}

/// Enables are one state behind two write ports, and both alias read back to it.
///
/// `intrGetNonStallEnable_TU102` tests `mask & intrReadRegTopEnSet(...)`
/// (`intr_nonstall_tu102.c:65-69`), so the SET alias must report the *current* mask rather
/// than the last word written to it.
#[test]
fn the_enable_set_and_clear_aliases_are_two_ports_onto_one_state() {
    let p = plane();
    p.write(REGS, leaf_en_set(4), 4, 0b1010);
    assert_eq!(p.read(REGS, leaf_en_set(4), 4).value(), 0b1010);
    assert_eq!(p.read(REGS, leaf_en_clear(4), 4).value(), 0b1010);
    p.write(REGS, leaf_en_clear(4), 4, 0b0010);
    assert_eq!(
        p.read(REGS, leaf_en_set(4), 4).value(),
        0b1000,
        "EN_CLEAR did not withdraw from the state EN_SET reads"
    );
    p.write(REGS, top_en_set(0), 4, 0b11);
    p.write(REGS, top_en_clear(0), 4, 0b01);
    assert_eq!(p.read(REGS, top_en_clear(0), 4).value(), 0b10);
}

/// ★★ The offsets this port answers are exactly the rows the register block declares.
///
/// ⊘ Not a restatement of the constants — a check that the **decoder's boundaries** are the
/// declared `__SIZE_1`s. A row past the end answered as a register would be this port
/// inventing a block; unanswered, it falls to the plane's unclaimed arm, which counts and
/// names it.
#[test]
fn the_decoder_stops_where_the_register_block_stops() {
    assert!(cpuintr::decode(leaf(7)).is_some(), "leaf 7 is declared");
    assert!(cpuintr::decode(leaf(8)).is_none(), "leaf 8 is not");
    assert!(cpuintr::decode(top(0)).is_some(), "top 0 is declared");
    assert!(cpuintr::decode(top(1)).is_none(), "top 1 is not");
    // ⚠ `TOP_EN_CLEAR0` is `0xB81610` and the trigger is `0xB81640`. A range test with the
    // wrong bound would swallow the trigger into the enable family and the boot would fail
    // with no vector and no unclaimed access to point at.
    assert_eq!(
        cpuintr::decode(TRIGGER),
        Some(cpuintr::CpuIntrReg::LeafTrigger)
    );
    assert!(cpuintr::decode(VF + 0x1618).is_none());
    // An unaligned access inside a declared row is not a register.
    assert!(cpuintr::decode(leaf(0) + 1).is_none());
    // And the arithmetic the whole block hangs off.
    assert_eq!(cpuintr::vector_to_leaf_reg(cpuintr::DOORBELL_VECTOR), 4);
    assert_eq!(cpuintr::vector_to_leaf_bit(cpuintr::DOORBELL_VECTOR), 1);
    assert_eq!(cpuintr::vector_to_subtree(cpuintr::DOORBELL_VECTOR), 2);
}

/// ⊘ The tree does not answer through the wrong aperture.
///
/// The same offsets in BAR2's window are framebuffer addresses, and a decoder that ignored
/// the BAR would serve device memory as an interrupt register. `kayfabe_device::plane`'s
/// own docs call this the silent misattribution its classification exists to prevent.
#[test]
fn the_interrupt_tree_is_a_register_aperture_fact_not_an_offset_fact() {
    let p = plane();
    let w = p.write(2, TRIGGER, 4, DOORBELL);
    assert!(
        !w.raise_cpu_intr,
        "a write at the same offset in another aperture raised a vector"
    );
    assert_eq!(p.counters().cpu_intr_raises, 0);
}
