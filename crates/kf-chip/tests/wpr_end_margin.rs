//! ★★★ v3-initrace — **WPR2 after a failed GSP boot** (`V3_HW_BOUNDARY_INVENTORY.md` §5.2 L1).
//!
//! RM places the FRTS region at `gspFwWprEnd - frtsSize`, with
//! `gspFwWprEnd = NV_ALIGN_DOWN64(vbiosReservedOffset - kgspGetWprEndMargin(), 0x20000)`
//! (`kgspPopulateWprMeta_TU102`, `ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/
//! kernel_gsp_tu102.c:776-779`), commands FWSEC to put it there (`frtsRegionOffset4K`,
//! `kernel_gsp_frts_tu102.c:324`) and then requires `WPR2_ADDR_LO == frtsOffset >> 12` exactly
//! (`:514-524`). The margin is zero on a clean boot and non-zero on every boot after a failed one
//! (`kgspGetWprEndMargin_IMPL`, `kernel_gsp.c:5637-5697`: the previous attempt's
//! `gspFwWprEnd - nonWprHeapOffset`, or the estimated WPR size, times `bootAttempts` — persisted in
//! the registry across opens, `:3913-3921`). The falcon model served the zero-margin derivation
//! always, so one failed boot made every retry fail. These tests hold the fix: WPR2 is served at
//! the offset the guest's own FWSEC-FRTS command names, located through the GSP falcon's DMEM load.
use kf_arch::gsp::{ArchBootState, BootContext, BootPhase, BootStep, FalconDma, GspModel, GspObservation, GspReg, RegWrite};
use kf_chip::falcon_gsp::{FalconGspModel, frts_offset_for, frts_offset_served, gsp_fw_wpr_end_for};

const MIB: u64 = 1 << 20;

/// ★ The ORACLE — ogkm-580's arithmetic for where RM puts FRTS on our device (no VBIOS-locked MMU
/// range, the VGA workspace the top `DRF_SIZE(NV_PRAMIN)` of FB — `kernel_gsp_tu102.c:745-771`),
/// for a given WPR-end margin. Written from the source, never from our model.
fn ogkm_frts_offset(fb_mb: u64, margin: u64) -> u64 {
    let fb = fb_mb * MIB;
    let vbios_reserved = fb - MIB; // vgaWorkspaceOffset = fbSize - DRF_SIZE(NV_PRAMIN)
    let wpr_end = (vbios_reserved - margin) & !(0x2_0000 - 1); // NV_ALIGN_DOWN64(…, 0x20000)
    wpr_end - MIB // frtsOffset = gspFwWprEnd - kgspGetFrtsSize_TU102 (1 MiB)
}

fn wpr2(m: &FalconGspModel, commanded: Option<u64>) -> (u64, u64) {
    let obs = GspObservation { stage: BootPhase::ProtectedRegionUp, wpr2_up: true, frts_offset: commanded, ..GspObservation::default() };
    (m.encode(GspReg::Wpr2AddrLo, &obs).unwrap(), m.encode(GspReg::Wpr2AddrHi, &obs).unwrap())
}

/// `_VAL` 31:4 = addr >> 12 (`dev_fb.h:34-39`).
fn reg(addr: u64) -> u64 {
    (addr >> 12) << 4
}

#[test]
fn a_clean_boot_is_served_exactly_as_before() {
    for fb in [6144, 8192, 11_857, 22_528] {
        let m = FalconGspModel::with_fb_size_mb(fb);
        assert_eq!(ogkm_frts_offset(fb, 0), frts_offset_for(fb), "{fb}: the oracle and the old derivation agree at margin 0");
        let want = (reg(frts_offset_for(fb)), reg(gsp_fw_wpr_end_for(fb)));
        assert_eq!(wpr2(&m, None), want, "{fb}: no command read — the derivation");
        assert_eq!(wpr2(&m, Some(frts_offset_for(fb))), want, "{fb}: the clean command — identical");
    }
}

/// ★★★ The retry: whatever margin RM added, `WPR2_ADDR_LO` is RM's own `frtsOffset >> 12`, and the
/// region is the 1 MiB FWSEC builds.
#[test]
fn every_retry_margin_is_served_where_rm_checks_it() {
    // A WPR of ~ 90 MiB (heap + ELF + boot binary + FRTS) times bootAttempts 1..3, plus an
    // unaligned estimate (the pre-meta path adds sizes that are not 128 KiB multiples).
    for fb in [8192, 11_857, 22_528] {
        let m = FalconGspModel::with_fb_size_mb(fb);
        for margin in [90 * MIB, 180 * MIB, 270 * MIB, 0x5_7a3_4c0] {
            let frts = ogkm_frts_offset(fb, margin);
            let (lo, hi) = wpr2(&m, Some(frts));
            assert_eq!(lo, reg(frts), "{fb} MiB, margin {margin:#x}: the exact LO compare RM makes");
            assert_eq!(hi, reg(frts + MIB), "{fb} MiB, margin {margin:#x}: the FRTS region's end");
            assert_ne!(lo, wpr2(&m, None).0, "the old derivation would have failed this retry");
        }
    }
}

/// ⊘ A guest-written offset is accepted only where FWSEC could place the region: 4 KiB-aligned,
/// above FB offset 1 MiB, at or below the zero-margin WPR end. Anything else derives.
#[test]
fn a_command_fwsec_would_not_honour_derives() {
    let fb = 8192;
    let d = frts_offset_for(fb);
    for bad in [d + 0x1000, d + MIB, d - 0x800, 0, 0x800, !0xFFF_u64] {
        assert_eq!(frts_offset_served(fb, Some(bad)), d, "{bad:#x}");
    }
    assert_eq!(frts_offset_served(fb, Some(MIB)), MIB, "the lowest acceptable");
}

/// One guest register write through the falcon model's own boot sequence.
fn write(m: &FalconGspModel, state: &mut ArchBootState, off: u64, val: u64) -> Vec<BootStep> {
    let ctx = BootContext { obs: GspObservation { stage: BootPhase::Cold, ..GspObservation::default() }, boot_args_seen: (false, false) };
    let rw = RegWrite { bar: 0, off, val, reg: m.decode_reg(0, off) };
    m.boot_sequence().on_write(m, &rw, &ctx, state).into_iter().collect()
}

/// `s_dmaTransfer_GA102`: BASE/BASE1 from `src`, then per 256-byte block MOFFS, FBOFFS, CMD.
#[allow(clippy::too_many_arguments)]
fn dma(m: &FalconGspModel, state: &mut ArchBootState, src: u64, dest: u64, mem_off: u64, blocks: u64, cmd: u64) -> Vec<BootStep> {
    let mut v = write(m, state, 0x11_0110, (src >> 8) & 0xFFFF_FFFF);
    v.extend(write(m, state, 0x11_0128, (src >> 40) & 0x1FF));
    for k in 0..blocks {
        v.extend(write(m, state, 0x11_0114, dest + 256 * k));
        v.extend(write(m, state, 0x11_011c, mem_off + 256 * k));
        v.extend(write(m, state, 0x11_0118, cmd));
    }
    v
}

/// ★★ The sequence locates the DMEM image exactly as RM loads it (`kgspExecuteHsFalcon_GA102` +
/// `s_dmaTransfer_GA102`, `ogkm-580: kernel_gsp_falcon_ga102.c:98-258`): IMEM from one base, then
/// DMEM from `PA + dataOffset - dmemVa` with `FBOFFS = dmemVa + k·256` and `MOFFS = dmemPa + k·256`
/// — and the STARTCPU that runs it is preceded by `FwsecCommand(PA + dataOffset - dmemPa)`.
#[test]
fn the_fwsec_start_carries_its_dmem_image_and_consumes_it() {
    let m = FalconGspModel::with_fb_size_mb(8192);
    let mut st = ArchBootState::default();
    // The ucode memdesc at a 49-bit sysmem PA (BASE1 non-zero), data at +0x3000, dmemVa 0x100.
    let pa: u64 = 0x1_2345_6700_0000;
    let (data_offset, dmem_va, dmem_pa, imem_va, imem_pa) = (0x3000u64, 0x100u64, 0u64, 0x200u64, 0u64);
    // IMEM (IMEM 4:4 set, SEC 2:2): no image latched, no step.
    assert!(dma(&m, &mut st, pa + 0x800 - imem_va, imem_pa, imem_va, 4, 0x10 | 0x4 | (6 << 8)).is_empty());
    // DMEM (IMEM clear, SET_DMTAG 6:6): the image.
    assert!(dma(&m, &mut st, pa + data_offset - dmem_va, dmem_pa, dmem_va, 16, 0x40 | (6 << 8)).is_empty());
    assert_eq!(
        write(&m, &mut st, 0x11_0100, 0x2),
        vec![BootStep::FwsecCommand(pa + data_offset - dmem_pa), BootStep::StartProcessor],
        "CPUCTL STARTCPU"
    );
    assert_eq!(write(&m, &mut st, 0x11_0100, 0x2), vec![BootStep::StartProcessor], "a start with no DMEM load of its own carries no image");
}

/// ★ Turing's `BOOT_WITH_LOADER` (`s_prepareHsFalconWithLoader`, `ogkm-580:
/// kernel_gsp_falcon_tu102.c:204-285`): the generic bootloader's `RM_FLCN_BL_DMEM_DESC` is written to
/// DMEM offset 0 through the PIO port (`DMEMC(0)` with `AINCW`, then `DMEMD(0)` words), and its
/// `dataDmaBase` (+0x40) is the FWSEC data image — carried by the start that runs it, once.
#[test]
fn a_turing_start_carries_the_bootloader_descriptors_data_image() {
    use kf_chip::falcon_gsp::RiscvLayout;
    let m = FalconGspModel::with_layout(RiscvLayout::Tu102, 8192);
    let mut st = ArchBootState::default();
    let data: u64 = 0x1_2345_6700;
    // RM_FLCN_BL_DMEM_DESC: reserved[4], signature[4], ctxDma=4, codeDmaBase{lo,hi},
    // nonSecureCodeOff/Size, secureCodeOff/Size, codeEntryPoint, dataDmaBase{lo,hi}, dataSize, argc, argv.
    let mut desc = vec![0u32; 21];
    desc[8] = 4;
    desc[9] = 0x8900_0000;
    desc[16] = (data & 0xFFFF_FFFF) as u32;
    desc[17] = (data >> 32) as u32;
    desc[18] = 0x1000;
    assert!(write(&m, &mut st, 0x11_01c0, 1 << 24).is_empty(), "DMEMC(0): offset 0, AINCW");
    for w in desc {
        assert!(write(&m, &mut st, 0x11_01c4, u64::from(w)).is_empty());
    }
    assert_eq!(write(&m, &mut st, 0x11_0100, 0x2), vec![BootStep::FwsecCommand(data), BootStep::StartProcessor]);
    assert_eq!(write(&m, &mut st, 0x11_0100, 0x2), vec![BootStep::StartProcessor], "consumed by its start");
    // Only one half written (a DMEM write that is not the descriptor): no image.
    assert!(write(&m, &mut st, 0x11_01c0, 0x40).is_empty(), "DMEMC(0) at +0x40, no AINCW");
    assert!(write(&m, &mut st, 0x11_01c4, 0xdead_0000).is_empty());
    assert_eq!(write(&m, &mut st, 0x11_0100, 0x2), vec![BootStep::StartProcessor]);
}

#[test]
fn only_the_gsp_falcons_dma_is_decoded() {
    let m = FalconGspModel::with_fb_size_mb(8192);
    assert_eq!(m.falcon_dma(0, 0x11_0110, 0xabcd), Some(FalconDma::Base(0xabcd)));
    assert_eq!(m.falcon_dma(0, 0x11_0128, 0xffff), Some(FalconDma::Base1(0x1ff)), "BASE1 is 8:0");
    assert_eq!(m.falcon_dma(0, 0x11_0118, 0x10), Some(FalconDma::Transfer { dmem_load: false }), "IMEM");
    assert_eq!(m.falcon_dma(0, 0x11_0118, 0x20), Some(FalconDma::Transfer { dmem_load: false }), "a write-back");
    assert_eq!(m.falcon_dma(0, 0x11_0118, 0x600), Some(FalconDma::Transfer { dmem_load: true }));
    assert_eq!(m.falcon_dma(0, 0x11_01c0, (1 << 24) | 0x1234), Some(FalconDma::DmemPort { addr: 0x1234, inc: true }));
    assert_eq!(m.falcon_dma(0, 0x11_01c4, 7), Some(FalconDma::DmemWord(7)));
    assert_eq!(m.falcon_dma(0, 0x84_0110, 0x1), None, "SEC2's Booter load is not FWSEC");
    assert_eq!(m.falcon_dma(1, 0x11_0110, 0x1), None, "BAR0 only");
    for f in [kf_chip::Family::Hopper, kf_chip::Family::Blackwell] {
        let fsp = f.gsp_model(0x6, 8192).unwrap();
        assert_eq!(fsp.falcon_dma(0, 0x11_0110, 1), None, "{f:?}: FSP runs FWSEC itself");
    }
}
