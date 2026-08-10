//! ★★★★★ **§16.77 — the RISC-V interrupt-status AND, pinned at the register model.**
//!
//! # What this file is about, in one sentence
//!
//! `kgspService_TU102` decides whether it has anything to do by ANDing three registers,
//! and on a GSP-offload adapter **two of the three are RISC-V registers**, not falcon
//! ones — so a port that models only the falcon pair reports "no interrupt pending" for
//! every interrupt it will ever raise.
//!
//! # The chain, every link cited
//!
//! 1. `kgspService_TU102` opens with `intrStatus = kflcnGetPendingHostInterrupts(...)` and
//!    returns immediately on zero, with
//!    `NV_ASSERT_FAILED("KGSP service called when no KGSP interrupt pending")` and
//!    **without writing `IRQSCLR`**
//!    (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:1155-1162`).
//! 2. `kflcnGetPendingHostInterrupts` branches on `kflcnIsRiscvMode`
//!    (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/kernel_falcon.c:84-90`).
//! 3. In RISC-V mode it is
//!    `IRQSTAT & NV_PRISCV_RISCV_IRQMASK & NV_PRISCV_RISCV_IRQDEST`
//!    (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/arch/ampere/kernel_falcon_ga102.c:311-321`;
//!    `_TU102` at `.../turing/kernel_falcon_tu102.c:384-392` is the same body over
//!    different offsets).
//! 4. ⚠ **The mode is LATCHED, not probed.** `kflcnResetIntoRiscv_GA102` calls
//!    `kflcnSetRiscvMode(pKernelFlcn, NV_TRUE)` unconditionally
//!    (`ogkm-580: kernel_falcon_ga102.c:84-95`), reached from the GSP boot sequencer's
//!    `GSP_SEQ_BUF_OPCODE_CORE_RESUME`
//!    (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/ampere/kernel_gsp_ga102.c:161`), and
//!    `kflcnIsRiscvMode` caches the tristate
//!    (`ogkm-580: src/nvidia/generated/g_kernel_falcon_nvoc.h:959-965`). So `BCR_CTRL` is
//!    never consulted after bootstrap, and modelling it would not have helped.
//!
//! `[measured 2026-08-10, boot w212 at e309a85]` before this was modelled: the guest's ISR
//! reached `kgspService` — proving the interrupt tree scan *did* attribute the vector — and
//! printed the assert at step 1. The same boot's device report says `0 IRQSCLR cleared` and
//! `348 gated`, which is that early return, seen from the other side.
//!
//! # ⊘ Why the assertion is on the AND and not on the two new decode arms
//!
//! Two arms in a `match` are a fact about this file; the AND is the fact about the guest.
//! A test that only checked `decode_reg(0x111528) == Some(GspRiscvIrqmask)` would stay
//! green if a later edit made the *encoding* zero — which is precisely the shape of the
//! bug, since an undecoded register and a decoded-but-zero one are the same value to RM.

use kayfabe_arch::gsp::{BootPhase, GspModel, GspObservation, GspReg};
use kayfabe_device::ga10x::Ga10xGspModel;

/// `NV_PFALCON_FALCON_IRQSTAT_SWGEN0` — bit 6.
const SWGEN0: u64 = 1 << 6;

/// `NV_FALCON2_GSP_BASE` + `NV_PRISCV_RISCV_IRQMASK`
/// (`ogkm-580: ampere/ga102/dev_falcon_second_pri.h:26`, `.../dev_riscv_pri.h:28`), which is
/// also the offset `C: src/qemu/nvkvm_gpu_emul.c:1572` answers.
const RISCV_IRQMASK: u64 = 0x0011_1528;
/// `NV_FALCON2_GSP_BASE` + `NV_PRISCV_RISCV_IRQDEST` (`.../dev_riscv_pri.h:29`);
/// `C: nvkvm_gpu_emul.c:1573`.
const RISCV_IRQDEST: u64 = 0x0011_152c;

/// `NV_PFALCON_FALCON_IRQSTAT` / `IRQMASK` / `IRQDEST` over `NV_PGSP` = `0x0011_0000`.
const FALCON_IRQSTAT: u64 = 0x0011_0008;
const FALCON_IRQMASK: u64 = 0x0011_0018;
const FALCON_IRQDEST: u64 = 0x0011_001c;

fn observation(swgen0_pending: bool) -> GspObservation {
    GspObservation {
        stage: BootPhase::Running,
        wpr2_up: true,
        riscv_active: true,
        suspended: false,
        swgen0_pending,
        boot_args_lo: 0,
        boot_args_hi: 0,
    }
}

/// Read one BAR0 offset the way the plane does: decode, then encode. `None` decode is the
/// **unclaimed** arm, which the plane answers with a defaulted zero
/// (`kayfabe_device::plane`'s header, *"An unclaimed register reads ZERO"*) — so it is
/// folded to `0` here rather than panicking. That is the whole point: the defect under test
/// is invisible unless the model is read through the same defaulting the guest sees.
fn read(model: &Ga10xGspModel, off: u64, obs: &GspObservation) -> u64 {
    model
        .decode_reg(0, off)
        .and_then(|reg| model.encode(reg, obs))
        .unwrap_or(0)
}

/// ★★★★★ `kflcnRiscvReadIntrStatus_GA102`, reproduced against the model — the one
/// assertion that would have gone red on every boot from `w208` through `w212`.
#[test]
fn riscv_read_intr_status_is_non_zero_while_swgen0_is_latched() {
    let model = Ga10xGspModel::default();
    let obs = observation(true);

    let intr_status = read(&model, FALCON_IRQSTAT, &obs)
        & read(&model, RISCV_IRQMASK, &obs)
        & read(&model, RISCV_IRQDEST, &obs);

    assert_eq!(
        intr_status, SWGEN0,
        "kflcnRiscvReadIntrStatus = IRQSTAT & RISCV_IRQMASK & RISCV_IRQDEST. A zero here is \
         the guest taking kernel_gsp_tu102.c:1158's early return — no IRQSCLR, no drain, and \
         the os-event flow-control gate latched shut for the rest of the boot."
    );
}

/// The falcon pair must keep working too: `kflcnIsRiscvMode` is false on a falcon-mode
/// bootstrap, and the same `IRQSTAT` then has to survive a *different* AND
/// (`kflcnReadIntrStatus_TU102`). Both pairs, one latch.
#[test]
fn falcon_read_intr_status_is_non_zero_while_swgen0_is_latched() {
    let model = Ga10xGspModel::default();
    let obs = observation(true);

    let intr_status = read(&model, FALCON_IRQSTAT, &obs)
        & read(&model, FALCON_IRQMASK, &obs)
        & read(&model, FALCON_IRQDEST, &obs);

    assert_eq!(intr_status, SWGEN0, "kflcnReadIntrStatus_TU102");
}

/// ⊘ **The negative half, and it is not decoration.** If both ANDs were non-zero
/// unconditionally the guest would service a phantom interrupt on every message, drain an
/// empty queue and — because `kgspService` clears the edge *before* servicing — could clear
/// a latch that was about to be set. The mask registers are constants; only `IRQSTAT` is
/// state, and this is what pins that.
#[test]
fn both_ands_are_zero_with_nothing_latched() {
    let model = Ga10xGspModel::default();
    let obs = observation(false);

    assert_eq!(
        read(&model, FALCON_IRQSTAT, &obs),
        0,
        "IRQSTAT is the state"
    );
    assert_eq!(
        read(&model, FALCON_IRQSTAT, &obs)
            & read(&model, RISCV_IRQMASK, &obs)
            & read(&model, RISCV_IRQDEST, &obs),
        0,
    );
    assert_eq!(
        read(&model, FALCON_IRQSTAT, &obs)
            & read(&model, FALCON_IRQMASK, &obs)
            & read(&model, FALCON_IRQDEST, &obs),
        0,
    );
}

/// ★ The two offsets round-trip through [`Ga10xGspModel::at`], so the model cannot name a
/// register it would not answer at that address — `assert_disjoint` at realize walks that
/// direction, and a decode/offset disagreement would put the register inside another
/// source's window without anything going red.
#[test]
fn the_riscv_pair_round_trips_through_reg_offset() {
    let model = Ga10xGspModel::default();
    for (off, reg) in [
        (RISCV_IRQMASK, GspReg::GspRiscvIrqmask),
        (RISCV_IRQDEST, GspReg::GspRiscvIrqdest),
    ] {
        assert_eq!(model.decode_reg(0, off), Some(reg), "decode 0x{off:x}");
        assert_eq!(Ga10xGspModel::at(reg), Some((0, off)), "at({reg:?})");
    }
}
