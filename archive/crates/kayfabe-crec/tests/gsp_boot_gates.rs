//! The GSP boot gates, as predicates over our own register model.
//!
//! `docs/design/gsp_boot_gate_spec.md` enumerates every place the stock NVIDIA driver can
//! decide our emulated GPU is not a GPU, from `RmInitAdapter` to `GSP_INIT_DONE`. Each gate
//! is a `NV_PRINTF(LEVEL_ERROR, …)` guarded by an exact condition, so the list is a test
//! suite: **re-express the driver's own C condition here, and assert our model satisfies
//! it.** No GPU, no VM, no C harness.
//!
//! ## What this suite can and cannot see
//!
//! ⚠ It is a *conformance* check against a specification read out of `ogkm-580.159.04`, not
//! a differential against a running driver. It catches "our answer violates the condition
//! NVIDIA's source states"; it cannot catch "we misread the source". Each assertion
//! therefore quotes the C verbatim and cites `file:line`, so a misreading is reviewable
//! rather than merely reproducible.
//!
//! ⚠ It deliberately re-derives the driver's arithmetic **independently** of
//! `ga10x.rs`'s `const fn`s rather than calling them. A test that calls the same helper the
//! production code calls asserts only that a function equals itself.

use kayfabe_arch::gsp::{GspModel, GspObservation, GspReg};
use kayfabe_crec::ga10x::{FB_SIZE_MB, Ga10xGspModel, USABLE_FB_SIZE_IN_MB_ADDR};

// ── the driver's own bit-field vocabulary, transcribed ────────────────────────────
//
// `FLD_TEST_DRF(d, r, f, c, v)` is "the field `f` of `v` equals the named constant `c`".
// `DRF_VAL(d, r, f, v)` extracts it. `hi:lo` field ranges are from the published headers.

/// `DRF_VAL` over an inclusive `hi:lo` bit range.
fn drf_val(v: u64, hi: u32, lo: u32) -> u64 {
    let width = hi - lo + 1;
    let mask = if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    (v >> lo) & mask
}

/// `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT_PROGRESS` — `7:0`
/// (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_gc6_island_addendum.h:30-32`).
const GFW_BOOT_PROGRESS_FIELD: (u32, u32) = (7, 0);
/// `..._PROGRESS_COMPLETED`.
const GFW_BOOT_PROGRESS_COMPLETED: u64 = 0xFF;
/// `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK_READ_PROTECTION_LEVEL0` — bit 0,
/// `_ENABLE = 1`.
const GFW_BOOT_PLM_READ_PL0: (u32, u32) = (0, 0);
/// `NV_PFALCON_FALCON_CPUCTL_HALTED` — bit 4, `_TRUE = 1`
/// (`ogkm-580: .../turing/tu102/dev_falcon_v4.h`).
const CPUCTL_HALTED_FIELD: (u32, u32) = (4, 4);
/// `NV_PFALCON_FALCON_HWCFG2_RISCV` — bit 10, `_ENABLE = 1`.
const HWCFG2_RISCV_FIELD: (u32, u32) = (10, 10);
/// `NV_PRISCV_RISCV_CPUCTL_ACTIVE_STAT` — bit 7, `_ACTIVE = 1`
/// (`ogkm-580: .../ampere/ga102/dev_riscv_pri.h`).
const RISCV_CPUCTL_ACTIVE_FIELD: (u32, u32) = (7, 7);
/// `NV_PFB_PRI_MMU_WPR2_ADDR_LO_VAL` / `_HI_VAL` — `31:4`
/// (`ogkm-580: .../turing/tu102/dev_fb.h:35, 38`).
const WPR2_ADDR_VAL_FIELD: (u32, u32) = (31, 4);
/// `NV_PFB_PRI_MMU_WPR2_ADDR_LO_ALIGNMENT` = `0xc` (`dev_fb.h:36`).
const WPR2_ADDR_ALIGNMENT: u32 = 0xc;
/// `NV_USABLE_FB_SIZE_IN_MB_VALUE` — `31:0`
/// (`ogkm-580: .../ampere/ga102/dev_gc6_island_addendum.h:34`).
const USABLE_FB_SIZE_VALUE_FIELD: (u32, u32) = (31, 0);

/// The state the model is in once FWSEC and the Booter have run and GSP is alive.
fn booted() -> GspObservation {
    GspObservation {
        wpr2_up: true,
        riscv_active: true,
        ..GspObservation::default()
    }
}

/// The state at cold boot, before anything has run.
fn cold() -> GspObservation {
    GspObservation::default()
}

fn enc(reg: GspReg, obs: &GspObservation) -> u64 {
    Ga10xGspModel::new()
        .encode(reg, obs)
        .unwrap_or_else(|| panic!("model cannot serve {reg:?} — the driver reads it"))
}

// ══ Stage 1 — GFW boot ════════════════════════════════════════════════════════════

/// **G1.1** — `kflcnWaitForHalt_TU102`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/arch/turing/kernel_falcon_tu102.c:345`):
///
/// ```c
/// while (!FLD_TEST_DRF(_PFALCON, _FALCON, _CPUCTL_HALTED, _TRUE,
///                      kflcnRegRead_HAL(pGpu, pKernelFlcn, NV_PFALCON_FALCON_CPUCTL)))
/// ```
///
/// If this is ever false the driver spins for 2.05 s and gives up with
/// `"Timeout waiting for Falcon to halt"`. It must hold in **every** observation, because
/// the guest reads it before the boot FSM has any state.
#[test]
fn g1_1_gsp_falcon_reports_halted_in_every_state() {
    for obs in [cold(), booted()] {
        let (hi, lo) = CPUCTL_HALTED_FIELD;
        let v = enc(GspReg::GspFalconCpuctl, &obs);
        assert_eq!(
            drf_val(v, hi, lo),
            1,
            "G1.1: CPUCTL.HALTED must read _TRUE (obs={obs:?}, raw=0x{v:x})"
        );
    }
}

/// **G1.2** — `_gpuIsGfwBootCompleted_TU102`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/arch/turing/kern_gpu_tu102.c:406`):
///
/// ```c
/// if (!FLD_TEST_DRF(_PGC6, _AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK,
///                   _READ_PROTECTION_LEVEL0, _ENABLE, regVal))
/// {
///     *gfwBootProgressVal = 0x0;
///     return NV_FALSE;
/// }
/// ```
///
/// ★★ **This is read FIRST and it short-circuits.** An emulator that serves a perfect
/// `PROGRESS` but leaves the PLM at reset gets `"failed to wait for GFW_BOOT: (progress
/// 0x0)"` — an error naming the register it answered correctly. Measured: the throwaway C
/// spike written for the feasibility study failed exactly this way before the PLM was set.
#[test]
fn g1_2_gfw_boot_plm_read_protection_level0_is_lowered() {
    let (hi, lo) = GFW_BOOT_PLM_READ_PL0;
    let v = enc(GspReg::GfwBootPlm, &cold());
    assert_eq!(
        drf_val(v, hi, lo),
        1,
        "G1.2: PLM READ_PROTECTION_LEVEL0 must read _ENABLE before the progress word is \
         even consulted (raw=0x{v:x})"
    );
}

/// **G1.3** — same function, `kern_gpu_tu102.c:427`:
///
/// ```c
/// return FLD_TEST_DRF(_PGC6, _AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT,
///                     _PROGRESS, _COMPLETED, regVal);
/// ```
#[test]
fn g1_3_gfw_boot_progress_reads_completed() {
    let (hi, lo) = GFW_BOOT_PROGRESS_FIELD;
    let v = enc(GspReg::GfwBootProgress, &cold());
    assert_eq!(
        drf_val(v, hi, lo),
        GFW_BOOT_PROGRESS_COMPLETED,
        "G1.3: GFW_BOOT.PROGRESS must read _COMPLETED (raw=0x{v:x})"
    );
}

// ══ Stage 5 — WPR2 precheck, FB layout ════════════════════════════════════════════

/// **G5.1** — `_kgspBootGspRm` (`ogkm-580: kernel_gsp.c:3870`) via `kgspIsWpr2Up_TU102`
/// (`kernel_gsp_tu102.c:1249`):
///
/// ```c
/// NvU32 data = GPU_REG_RD32(pGpu, NV_PFB_PRI_MMU_WPR2_ADDR_HI);
/// wpr2HiVal = DRF_VAL(_PFB, _PRI_MMU_WPR2_ADDR_HI, _VAL, data);
/// return (wpr2HiVal != 0);
/// ```
///
/// At cold boot this must be **zero**, or the driver refuses with `"unexpected WPR2
/// already up, cannot proceed with booting GSP"`. This is the classic passthrough failure:
/// a prior guest left WPR2 up and Booter Unload never ran.
#[test]
fn g5_1_wpr2_is_down_at_cold_boot_and_up_after_fwsec() {
    let (hi, lo) = WPR2_ADDR_VAL_FIELD;
    let down = enc(GspReg::Wpr2AddrHi, &cold());
    assert_eq!(
        drf_val(down, hi, lo),
        0,
        "G5.1: WPR2_ADDR_HI._VAL must be 0 at cold boot (raw=0x{down:x})"
    );

    // G6.3a is the inverse of the same read, after FWSEC:
    // `if (wpr2HiVal == 0) "failed to execute FWSEC for FRTS: no initialized WPR2 found"`
    // (`ogkm-580: kernel_gsp_frts_tu102.c:505-512`).
    let up = enc(GspReg::Wpr2AddrHi, &booted());
    assert_ne!(
        drf_val(up, hi, lo),
        0,
        "G6.3a: WPR2_ADDR_HI._VAL must be non-zero once FWSEC has run (raw=0x{up:x})"
    );
}

/// **G5.2** — `kmemsysReadUsableFbSize_GA102`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_sys/arch/ampere/kern_mem_sys_ga102.c:48`):
///
/// ```c
/// NvU32 regValue = GPU_REG_RD32(pGpu, NV_USABLE_FB_SIZE_IN_MB);
/// *pFbSize = ((NvU64) DRF_VAL(_USABLE, _FB_SIZE_IN_MB, _VALUE, regValue) << 20);
/// ```
///
/// ★★ **`NV_USABLE_FB_SIZE_IN_MB` is deliberately NOT a [`GspReg`], and nothing in this
/// repo serves it yet.** `GspModel`'s stated rule is that a register belongs to the GSP
/// plane only if its served value is a function of the boot FSM's state; this one is a
/// devinit constant, so it sits with PTIMER and the fuses on the open side of the seam
/// (`mode2_gsp_port_plan.md` §11-O1). Adding it to `GspReg` was tried and reverted: it
/// violates the module's own invariant, and it moves every positional golden in
/// `cap1_differential.rs` by +3, because the C's capture reads that address exactly three
/// times.
///
/// What this test pins is the part that *is* ours: the constant the WPR2 layout is sized
/// from must be usable by whatever plane ends up answering the register, and must be
/// large enough that the driver's arithmetic does not underflow.
#[test]
fn g5_2_the_fb_size_the_wpr2_layout_is_sized_from_is_shared_and_sane() {
    let fb_bytes = FB_SIZE_MB << 20;
    assert_ne!(
        fb_bytes, 0,
        "G5.2: a zero usable FB size underflows the whole WPR2 layout"
    );
    // Magnitude, not the exact number, so a board-size table row is not a test edit: the
    // driver reserves 1 MiB of PRAMIN plus a 1 MiB FRTS off the top.
    assert!(
        fb_bytes > 0x0020_0000,
        "G5.2: FB must exceed PRAMIN + FRTS or `frtsOffset` goes negative (fb=0x{fb_bytes:x})"
    );
    // The value must survive the register's own field width, or the plane that eventually
    // serves it cannot express what the layout was sized from.
    let (hi, lo) = USABLE_FB_SIZE_VALUE_FIELD;
    assert_eq!(
        drf_val(FB_SIZE_MB, hi, lo),
        FB_SIZE_MB,
        "G5.2: FB_SIZE_MB must fit NV_USABLE_FB_SIZE_IN_MB_VALUE ({hi}:{lo})"
    );
    // And the address is recorded so the two planes cannot drift apart silently.
    assert_eq!(
        USABLE_FB_SIZE_IN_MB_ADDR, 0x0011_83A4,
        "NV_PGC6_AON_SECURE_SCRATCH_GROUP_42"
    );
}

/// **G6.3b** — `kgspExecuteFwsec_TU102`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_frts_tu102.c:514-524`):
///
/// ```c
/// data = GPU_REG_RD32(pGpu, NV_PFB_PRI_MMU_WPR2_ADDR_LO);
/// wpr2LoVal = DRF_VAL(_PFB, _PRI_MMU_WPR2_ADDR_LO, _VAL, data);
/// expectedLoVal = (NvU32) (pPreparedCmd->frtsOffset >> NV_PFB_PRI_MMU_WPR2_ADDR_LO_ALIGNMENT);
/// if (wpr2LoVal != expectedLoVal)
///     "failed to execute FWSEC for FRTS: WPR2 initialized at an unexpected location: ..."
/// ```
///
/// ★★★ **An exact compare against the driver's own arithmetic over the FB size we
/// advertise.** The chain, all in `kgspPopulateWprMeta_TU102`
/// (`ogkm-580: kernel_gsp_tu102.c:742, 761, 776, 779`):
///
/// ```text
/// fbSize             = NV_USABLE_FB_SIZE_IN_MB._VALUE << 20
/// vgaWorkspaceOffset = fbSize - DRF_SIZE(NV_PRAMIN)        // 0x100000, dev_ram.h:26
/// gspFwWprEnd        = NV_ALIGN_DOWN64(vgaWorkspaceOffset - wprEndMargin, 0x20000)
/// frtsOffset         = gspFwWprEnd - kgspGetFrtsSize()     // 1 MiB on TU/GA/AD
/// ```
///
/// `wprEndMargin` is 0 unless a regkey sets it (`kgspGetWprEndMargin_IMPL`,
/// `kernel_gsp.c:5637`), and we advertise no MMU lock, so `vbiosReservedOffset` is the VGA
/// workspace offset.
///
/// The arithmetic below is written from the ogkm sources, **not** by calling `ga10x.rs`'s
/// `const fn`s — a test that calls the same helper as production asserts a tautology.
#[test]
fn g6_3b_wpr2_addr_lo_equals_the_drivers_computed_frts_offset() {
    // The driver's chain, re-derived here.
    const PRAMIN_SIZE: u64 = 0x0010_0000;
    const FRTS_SIZE: u64 = 0x0010_0000;
    const WPR_ALIGN: u64 = 0x2_0000;

    let (fb_hi, fb_lo) = USABLE_FB_SIZE_VALUE_FIELD;
    // What the driver would read out of NV_USABLE_FB_SIZE_IN_MB, given what we sized the
    // layout from.
    let fb_size = drf_val(FB_SIZE_MB, fb_hi, fb_lo) << 20;

    let vga_workspace_offset = fb_size - PRAMIN_SIZE;
    let gsp_fw_wpr_end = vga_workspace_offset & !(WPR_ALIGN - 1);
    let frts_offset = gsp_fw_wpr_end - FRTS_SIZE;
    let expected_lo_val = frts_offset >> WPR2_ADDR_ALIGNMENT;

    let (hi, lo) = WPR2_ADDR_VAL_FIELD;
    let served = drf_val(enc(GspReg::Wpr2AddrLo, &booted()), hi, lo);

    assert_eq!(
        served, expected_lo_val,
        "G6.3b: WPR2_ADDR_LO._VAL = 0x{served:x} but the driver computes \
         frtsOffset(0x{frts_offset:x}) >> {WPR2_ADDR_ALIGNMENT} = 0x{expected_lo_val:x} from \
         the FB size we advertise ({FB_SIZE_MB} MiB). It would print \"WPR2 initialized at \
         an unexpected location\" and refuse to boot."
    );
}

/// The same chain, checked against the **C oracle's** measured constants rather than
/// against our own arithmetic.
///
/// `C: src/qemu/mode2_regs_ga10x.h:57-62` — `NVKVM_FB_SIZE_MB 12288`,
/// `NVKVM_WPR2_LO_VAL 0x02FFE000`, `NVKVM_WPR2_HI_VAL 0x02FFF000`. That artifact got a real
/// 580.159.04 driver through this gate on real hardware, so these three numbers are an
/// *existence proof* and not a second opinion. If our derivation disagrees with them at the
/// oracle's FB size, our derivation is wrong.
#[test]
fn g6_3b_derivation_reproduces_the_c_oracles_measured_constants() {
    assert_eq!(
        FB_SIZE_MB, 12288,
        "the oracle's constants below are only comparable at its own FB size"
    );
    let (hi, lo) = WPR2_ADDR_VAL_FIELD;
    assert_eq!(
        drf_val(enc(GspReg::Wpr2AddrLo, &booted()), hi, lo),
        drf_val(0x02FF_E000, hi, lo),
        "must reproduce the C oracle's NVKVM_WPR2_LO_VAL"
    );
    assert_eq!(
        drf_val(enc(GspReg::Wpr2AddrHi, &booted()), hi, lo),
        drf_val(0x02FF_F000, hi, lo),
        "must reproduce the C oracle's NVKVM_WPR2_HI_VAL"
    );
}

/// **G5.7** — `kflcnIsRiscvCpuEnabled_TU102`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/arch/turing/kernel_falcon_tu102.c:124-132`),
/// reached from `kgspPrepareForBootstrap_TU102` (`kernel_gsp_tu102.c:413`):
///
/// ```c
/// if (!kflcnIsRiscvCpuEnabled_HAL(pGpu, pKernelFalcon))
///     "RISC-V core is not enabled.\n"   -> NV_ERR_NOT_SUPPORTED
/// ```
#[test]
fn g5_7_hwcfg2_reports_riscv_enabled() {
    let (hi, lo) = HWCFG2_RISCV_FIELD;
    let v = enc(GspReg::GspFalconHwcfg2, &cold());
    assert_eq!(
        drf_val(v, hi, lo),
        1,
        "G5.7: HWCFG2.RISCV must read _ENABLE (raw=0x{v:x})"
    );
}

// ══ Stage 8 — Booter Load ═════════════════════════════════════════════════════════

/// **G8.2** — `s_executeBooterUcode_TU102`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_booter_tu102.c:76-80`):
///
/// ```c
/// if (mailbox0 != 0)
///     "Booter failed with non-zero error code: 0x%x\n"   -> NV_ERR_GENERIC
/// ```
///
/// `mailbox0` is read back from SEC2's `NV_PFALCON_FALCON_MAILBOX0` **after** the driver
/// wrote the physical address of its `GspFwWprMeta` there. A real Booter replaces it with
/// a status; we must not echo the address back.
#[test]
fn g8_2_sec2_mailbox0_reads_zero_after_booter() {
    for obs in [cold(), booted()] {
        let v = enc(GspReg::Sec2FalconMailbox0, &obs);
        assert_eq!(
            v, 0,
            "G8.2: SEC2 MAILBOX0 must read 0 or the driver reads its own WPR-meta address \
             back as a Booter error code (obs={obs:?})"
        );
    }
}

// ══ Stage 9 — RISC-V liveness and the teardown sentinel ═══════════════════════════

/// **G9.1** — `kgspBootstrap_TU102` (`ogkm-580: kernel_gsp_tu102.c:551`) via
/// `kflcnIsRiscvActive_GA102`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/arch/ampere/kernel_falcon_ga102.c:47-56`):
///
/// ```c
/// if (kflcnIsRiscvActive_HAL(pGpu, pKernelFalcon) || _kgspIsProcessorSuspended(...)) { }
/// else { "Failed to boot GSP.\n"  -> NV_ERR_NOT_READY }
/// ```
///
/// ★ Both directions are load-bearing. Reporting ACTIVE before the Booter has run would
/// tell the driver a core is running that never started.
#[test]
fn g9_1_riscv_active_tracks_the_boot_state_in_both_directions() {
    let (hi, lo) = RISCV_CPUCTL_ACTIVE_FIELD;
    assert_eq!(
        drf_val(enc(GspReg::GspRiscvCpuctl, &cold()), hi, lo),
        0,
        "G9.1: RISCV CPUCTL.ACTIVE_STAT must be 0 before the core has been started"
    );
    assert_eq!(
        drf_val(enc(GspReg::GspRiscvCpuctl, &booted()), hi, lo),
        1,
        "G9.1: RISCV CPUCTL.ACTIVE_STAT must read _ACTIVE once GSP is up"
    );
}

/// **G9.2** — `_kgspIsProcessorSuspended`
/// (`ogkm-580: kernel_gsp_tu102.c:1224-1238`):
///
/// ```c
/// mailbox = kflcnRegRead_HAL(pGpu, pKernelFlcn, NV_PFALCON_FALCON_MAILBOX0);
/// return (mailbox == 0x80000000);
/// ```
///
/// ★★ **580 tests EXACT EQUALITY with the constant inlined** — the symbol
/// `INTERRUPT_PROCESSOR_SUSPENDED_VALUE` does not exist at that tag; 610 masks instead
/// (`ogkm-610: kernel_gsp_tu102.c:333, 348`). So the sentinel must **replace** the mailbox
/// value, never be OR-ed onto it: a shadow still holding a boot-args low half with bit 31
/// set reads as suspended at 610 and hangs `gpuTimeoutCondWait` forever at 580 — on the
/// teardown path, where a hang is a wedged GPU.
#[test]
fn g9_2_suspend_sentinel_replaces_the_mailbox_it_does_not_or_into_it() {
    // A boot-args low half whose bit 31 is set: the exact value that makes OR-vs-replace
    // observable. A zero shadow could not tell the two implementations apart.
    let poisoned = GspObservation {
        boot_args_lo: 0x8123_4560,
        suspended: true,
        ..GspObservation::default()
    };
    assert_eq!(
        enc(GspReg::GspFalconMailbox0, &poisoned),
        0x8000_0000,
        "G9.2: 580 compares MAILBOX0 == 0x80000000 for exact equality; OR-ing the sentinel \
         onto a boot-args shadow hangs the teardown poll"
    );

    // And the echo must be intact while not suspended, or G7.4's write is lost.
    let echoing = GspObservation {
        boot_args_lo: 0x8123_4560,
        boot_args_hi: 0x0000_0007,
        ..GspObservation::default()
    };
    assert_eq!(enc(GspReg::GspFalconMailbox0, &echoing), 0x8123_4560);
    assert_eq!(enc(GspReg::GspFalconMailbox1, &echoing), 0x0000_0007);
}

// ══ The seam itself ═══════════════════════════════════════════════════════════════

/// Every gate register named in `gsp_boot_gate_spec.md` §5 as representable must actually
/// round-trip through `decode_reg`.
///
/// ★★ **Quantified over a LIST, and the list is pinned.** `gates_quantified_over_a_list`:
/// shortening the list weakens the gate with zero red tests, so the count is asserted
/// separately. Adding a gate register means adding a row *and* bumping the count.
#[test]
fn every_gate_register_decodes_at_the_address_it_is_served_from() {
    let model = Ga10xGspModel::new();
    let gate_regs = [
        ("G1.1 GSP CPUCTL", GspReg::GspFalconCpuctl),
        ("G1.2 GFW PLM", GspReg::GfwBootPlm),
        ("G1.3 GFW progress", GspReg::GfwBootProgress),
        ("G5.1/G6.3a WPR2 hi", GspReg::Wpr2AddrHi),
        ("G5.7 HWCFG2", GspReg::GspFalconHwcfg2),
        ("G6.3b WPR2 lo", GspReg::Wpr2AddrLo),
        ("G7.4 boot-args lo", GspReg::GspFalconMailbox0),
        ("G7.4 boot-args hi", GspReg::GspFalconMailbox1),
        ("G8.2 SEC2 mailbox0", GspReg::Sec2FalconMailbox0),
        ("G8.1 SEC2 CPUCTL", GspReg::Sec2FalconCpuctl),
        ("G9.1 RISCV CPUCTL", GspReg::GspRiscvCpuctl),
    ];
    assert_eq!(
        gate_regs.len(),
        11,
        "the universe this test quantifies over is pinned: a shorter list is a smaller \
         true statement"
    );

    for (gate, reg) in gate_regs {
        let (bar, off) = Ga10xGspModel::at(reg)
            .unwrap_or_else(|| panic!("{gate}: {reg:?} has no address on this generation"));
        assert_eq!(
            model.decode_reg(bar, off),
            Some(reg),
            "{gate}: a guest access at bar{bar} +0x{off:x} must decode back to {reg:?}"
        );
        assert!(
            model.encode(reg, &booted()).is_some(),
            "{gate}: {reg:?} decodes but cannot be served — the driver would fault"
        );
    }
}

/// The gaps §5 names are still gaps, stated as a test so they cannot close by accident and
/// go unnoticed, and cannot silently *widen*.
///
/// These are registers the driver genuinely reads on the boot path that this model cannot
/// name. Four of them (`NV_PBUS_VBIOS_SCRATCH(0x0E)`, `(0x15)`, the SEC2 ucode fuse
/// version, the GSP debug fuse) currently read as zero through whatever default the
/// register plane applies, and zero happens to pass — **by luck, not by design**. Two
/// (`HWCFG2.RESET_READY`, the memory-scrubbing-done bits) are polls.
#[test]
fn the_named_gaps_are_still_gaps() {
    // `HWCFG2` is representable, so RESET_READY and MEM_SCRUBBING are *reachable* — they
    // share the register. Assert the encoding does NOT claim them, so nobody reads this
    // suite as saying stage 7 is covered.
    let hwcfg2 = enc(GspReg::GspFalconHwcfg2, &cold());
    // NV_PFALCON_FALCON_HWCFG2_RESET_READY and _MEM_SCRUBBING (`dev_falcon_v4.h`,
    // Ampere: bits 3 and 12 respectively on GA102). Neither is set by our encoding.
    const RESET_READY_BIT: u32 = 3;
    const MEM_SCRUBBING_BIT: u32 = 12;
    assert_eq!(
        drf_val(hwcfg2, RESET_READY_BIT, RESET_READY_BIT),
        0,
        "G7.1 is an UNCOVERED gap; if this bit is now set, stage 7 grew coverage and \
         gsp_boot_gate_spec.md §5 must be updated"
    );
    assert_eq!(
        drf_val(hwcfg2, MEM_SCRUBBING_BIT, MEM_SCRUBBING_BIT),
        0,
        "G7.2 is an UNCOVERED gap; see G7.1"
    );
}
