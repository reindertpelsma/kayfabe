//! # GB202 (consumer Blackwell) — the two places the family assumption is WRONG
//!
//! ## Why this file exists at all
//!
//! `crates/kayfabe-chips/tests/host_classes.rs` ends with a test whose name is
//! `every_generation_decodes_a_work_submit_token_the_same_way`. That was true of the three
//! generations that existed when it was written and **GB202 falsifies it** — the driver
//! binds a Blackwell-specific encoder that sets a third field. A test asserting sameness
//! over a list that silently stops short of the interesting member is the *"gate quantified
//! over a shortened list"* this crate's own docs name; so the counterexample is asserted
//! here, positively, rather than left to be noticed.
//!
//! ## ⊘ What this file is NOT
//!
//! It is a **transcription check**, and it says so. The GA10x decoder is pinned by two
//! instruments that reach past transcription — `tests/tests/doorbell_token.rs` replays
//! tokens a real GA106 handed real channels, and `worksubmit_token_oracle.rs` compiles
//! `kfifoGenerateWorkSubmitTokenHal_GA100` itself and differentials the whole field space.
//! **Neither exists for GB202**, because neither can be built without either a Blackwell
//! board or a second compiled-C oracle. Everything below can catch a wrong bit position, a
//! wrong mask and a missing refusal. None of it can catch a shared misreading of
//! `kernel_fifo_gb202.c`, and none of it establishes that a Blackwell board accepts
//! anything.

use kayfabe_arch::Arch;
use kayfabe_arch::gsp::{ArchBootState, BootContext, GspModel, GspReg};
use kayfabe_arch::ids::{RunlistId, VChid};
use kayfabe_chips::gb20x::{
    Gb20xArch, Gb20xGspModel, WPR2_ADDR_HI_580, WPR2_ADDR_LO_580, decode_work_submit_token_gb202,
    encode_work_submit_token_gb202,
};

// ── The oracle's vocabulary, transcribed from NVIDIA's headers, NOT imported ──────────
//
// ★ Written out here rather than read from `gb20x.rs` so the expectation does not come
// from the thing under test. `ogkm-580: src/common/inc/swref/published/blackwell/gb202/
// dev_vm.h:27-32`.

/// `NV_VIRTUAL_FUNCTION_DOORBELL` (`:27`) — the register the token is stored to.
const DOORBELL_REG: u64 = 0x3_0090;
/// `NV_VIRTUAL_FUNCTION_DOORBELL_VECTOR` is `11:0` (`:28`).
const VECTOR_HI_BIT: u32 = 11;
/// `NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_ID` is `22:16` (`:29`).
const RUNLIST_LO_BIT: u32 = 16;
/// …and `22` is its high bit (`:29`).
const RUNLIST_HI_BIT: u32 = 22;
/// `NV_VIRTUAL_FUNCTION_DOORBELL_RUNLIST_DOORBELL` is `30:30` (`:30`).
const RUNLIST_DOORBELL_BIT: u32 = 30;
/// `..._RUNLIST_DOORBELL_ENABLE` is `0x1` (`:32`) — **and `_DISABLE` is `0x0` (`:31`)**,
/// which is the opposite polarity to GB100's definition of the same-named field.
const RUNLIST_DOORBELL_ENABLE: u32 = 1;

/// The token this file's oracle builds, from the transcribed field positions alone. This
/// is `kfifoGenerateWorkSubmitTokenHal_GB202` (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/
/// arch/blackwell/kernel_fifo_gb202.c:73,75,76`) re-written from its three `FLD_SET_DRF*`
/// lines, starting — as RM does — from `val = 0`.
fn oracle_token(chid: u32, runlist: u32) -> u32 {
    let vector_mask = (1u32 << (VECTOR_HI_BIT + 1)) - 1;
    let runlist_mask = (1u32 << (RUNLIST_HI_BIT - RUNLIST_LO_BIT + 1)) - 1;
    let mut val = 0u32;
    val |= (runlist & runlist_mask) << RUNLIST_LO_BIT;
    val |= chid & vector_mask;
    val |= RUNLIST_DOORBELL_ENABLE << RUNLIST_DOORBELL_BIT;
    val
}

/// ★★★ **Every token RM's GB202 encoder produces round-trips through this decoder.**
///
/// Swept over the full width of both fields — 4096 chids × 128 runlists is 524 288 values,
/// which is cheap and is the only thing that pins the two **widths**. Spot checks cannot:
/// a decoder that masked the runlist to six bits would agree with this one on every small
/// value and lose runlist 64 and up.
#[test]
fn every_gb202_token_the_drivers_encoder_can_produce_round_trips() {
    let mut checked = 0u64;
    for runlist in 0u32..128 {
        for chid in 0u32..4096 {
            let token = oracle_token(chid, runlist);
            let got = decode_work_submit_token_gb202(u64::from(token)).unwrap_or_else(|| {
                panic!(
                    "★ token {token:#010x} (chid {chid}, runlist {runlist}) came out of \
                     NVIDIA's own encoder and this decoder REFUSED it"
                )
            });
            assert_eq!(
                (got.vchid, got.runlist),
                (
                    VChid(u16::try_from(chid).expect("chid < 4096")),
                    RunlistId(u16::try_from(runlist).expect("runlist < 128"))
                ),
                "★ token {token:#010x} decoded to the wrong (chid, runlist)"
            );
            // …and this crate's own encoder must agree with the transcribed one, or the
            // round-trip is one function talking to itself.
            assert_eq!(
                encode_work_submit_token_gb202(
                    u16::try_from(chid).expect("chid < 4096"),
                    u16::try_from(runlist).expect("runlist < 128")
                ),
                Some(u64::from(token)),
                "★ this crate's encoder disagrees with the transcribed one at \
                 (chid {chid}, runlist {runlist})"
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 524_288,
        "★ NON-VACUITY: the whole field space must have been swept, not {checked} values"
    );
}

/// ★★★★★ **THE FINDING: the Ampere decoder refuses EVERY real GB202 token.**
///
/// `ga10x::decode_work_submit_token` rejects any token with a bit outside `0x007F_0FFF`
/// (`crates/kayfabe-chips/src/ga10x.rs`), and every GB202 token sets bit 30. So a Blackwell
/// `Arch` that had delegated its doorbell decode to GA10x's — the delegation every other
/// generation in this crate legitimately makes — would answer `None` for **every** ring the
/// guest ever submits.
///
/// ⊘ The failure mode is the *good* one: refused, loudly, not silently mis-routed. That is
/// worth stating because the same is **not** true of datacenter Blackwell — GB100's encoder
/// writes no bit outside the Ampere mask, so the wrong decoder there is accepted quietly
/// (`ogkm-580: kernel_fifo_gb100.c:114-115`, fields at `blackwell/gb100/dev_vm.h:622,624`).
/// One Blackwell fails loudly and the other fails silently, which is why "Blackwell" is not
/// one answer.
#[test]
fn the_ampere_decoder_refuses_every_gb202_token_and_that_is_the_whole_point() {
    let mut refused = 0usize;
    let mut probes = 0usize;
    for runlist in [0u32, 1, 7, 63, 127] {
        for chid in [0u32, 1, 4, 255, 4095] {
            let token = u64::from(oracle_token(chid, runlist));
            assert!(
                kayfabe_chips::ga10x::decode_work_submit_token(token).is_none(),
                "★ the Ampere decoder ACCEPTED GB202 token {token:#010x} — if this ever \
                 becomes true the refusal is silent and the two encodings have merged"
            );
            refused += 1;
            assert!(
                decode_work_submit_token_gb202(token).is_some(),
                "★ …and the Blackwell decoder must accept it, or the assertion above is \
                 about a token nobody can produce"
            );
            probes += 1;
        }
    }
    assert_eq!(refused, 25);
    assert_eq!(probes, 25);

    // ★★ And the converse: an AMPERE token must be refused HERE. Bit 30 clear means the
    // token did not come from this generation's encoder, and serving it would route an
    // Ampere-shaped ring onto a Blackwell channel map.
    for amp in [0x0000_0000u64, 0x0000_0004, 0x007F_0FFF, 0x0003_0001] {
        assert!(
            decode_work_submit_token_gb202(amp).is_none(),
            "★ the Blackwell decoder accepted the Ampere token {amp:#010x}; \
             RUNLIST_DOORBELL_ENABLE is set in EVERY token its encoder writes"
        );
        assert!(
            kayfabe_chips::ga10x::decode_work_submit_token(amp).is_some(),
            "★ NON-VACUITY: {amp:#010x} must be a token the Ampere decoder ACCEPTS, or \
             the refusal above is about a malformed value rather than a real divergence"
        );
    }
}

/// ⊘ **Bits RM's encoder starts from zero and never sets must be refused** — the `15:12`
/// hole, `29:23`, bit 31, and anything wider than the `NvU32` RM writes.
#[test]
fn bits_the_gb202_encoder_cannot_write_are_refused() {
    let well_formed = u64::from(oracle_token(4, 1));
    assert!(decode_work_submit_token_gb202(well_formed).is_some());

    for bad_bit in [12u32, 13, 14, 15, 23, 24, 28, 29, 31] {
        let token = well_formed | (1u64 << bad_bit);
        assert!(
            decode_work_submit_token_gb202(token).is_none(),
            "★ bit {bad_bit} is outside every field kfifoGenerateWorkSubmitTokenHal_GB202 \
             writes (gb202/dev_vm.h:28-32), so {token:#010x} did not come from RM"
        );
    }
    assert!(
        decode_work_submit_token_gb202(well_formed | 0x1_0000_0000).is_none(),
        "★ RM writes an NvU32; anything above 32 bits was not generated by it"
    );
    // The one bit that must be REQUIRED rather than forbidden.
    assert!(
        decode_work_submit_token_gb202(well_formed & !(1u64 << RUNLIST_DOORBELL_BIT)).is_none(),
        "★ clearing RUNLIST_DOORBELL must refuse — it is set in every GB202 token"
    );
}

/// ⊘ Out-of-range inputs to this crate's encoder are refused rather than truncated.
///
/// RM's `FLD_SET_DRF_NUM` *would* silently truncate; a helper that copied that would make
/// an out-of-range test input look like a valid token and would quietly alias two channels.
#[test]
fn the_encoder_refuses_a_pair_the_hardware_fields_cannot_hold() {
    assert!(encode_work_submit_token_gb202(4095, 127).is_some());
    assert!(
        encode_work_submit_token_gb202(4096, 0).is_none(),
        "★ chid 4096 does not fit VECTOR 11:0"
    );
    assert!(
        encode_work_submit_token_gb202(0, 128).is_none(),
        "★ runlist 128 does not fit RUNLIST_ID 22:16"
    );
}

/// ★ The `Arch` really wires the Blackwell decoder, not the inherited one. The delegation
/// trap, watched: `Gb20xArch` composes a `MockArch` and forwards nine of its eleven
/// methods, so forwarding this one would compile and be wrong on every ring.
#[test]
fn the_blackwell_arch_uses_its_own_doorbell_decoder() {
    let arch = Gb20xArch::default();
    let token = u64::from(oracle_token(7, 3));
    assert_eq!(
        arch.decode_doorbell(token),
        decode_work_submit_token_gb202(token),
        "★ Gb20xArch::decode_doorbell is not this generation's decoder"
    );
    assert!(
        arch.decode_doorbell(0x0000_0004).is_none(),
        "★ Gb20xArch accepted an Ampere-shaped token — it is delegating"
    );
    // ⊘ …and the mock is not answering either.
    let mock = kayfabe_mocks::MockArch::default();
    assert_ne!(
        arch.decode_doorbell(token),
        mock.decode_doorbell(token),
        "★ Gb20xArch agrees with MockArch's INVENTED encoding on a real token"
    );
    // The register the token is stored to is unchanged Volta→Blackwell, which is the half
    // of the coordinator's brief that DOES hold: `0x30090` with `DRF_BASE(NV_VIRTUAL_
    // FUNCTION) = 0x30000` ⇒ `USERMODE_DOORBELL_OFF = 0x90`.
    assert_eq!(
        DOORBELL_REG & 0xFFFF,
        0x0090,
        "★ NV_VIRTUAL_FUNCTION_DOORBELL's offset within the usermode window is 0x90 on \
         GB202 (gb202/dev_vm.h:27) exactly as on Ampere — this is the part that did NOT \
         move, and it is asserted so the two halves are not confused"
    );
}

// ══════════════════════════════════════════════════════════════════════════════════════
// The registers
// ══════════════════════════════════════════════════════════════════════════════════════

/// ★★★★★ **Both driver generations' WPR2 addresses must decode**, because the register
/// moved *by driver version* rather than by chip: a 580 Blackwell guest polls
/// `NV_PFB_PRI_MMU_WPR2_ADDR_HI` at `0x1FA828` and a 610 one polls
/// `NV_HUBMMU0_PRI_BASE + NV_HUBMMU_PRI_MMU_WPR2_ADDR_HI` at `0x88A828`.
///
/// ⊘ Serving only one is the `dlen = 0` shape: the unserved address is *unclaimed*, an
/// unclaimed BAR0 read answers `0`, and the guest sits in `kgspWaitForGfwBootOk` forever
/// with nothing refused and no counter moved.
#[test]
fn both_driver_generations_wpr2_addresses_decode() {
    let m = Gb20xGspModel::new();
    for (off, want, era) in [
        (0x0088_A824u64, GspReg::Wpr2AddrLo, "610"),
        (0x0088_A828, GspReg::Wpr2AddrHi, "610"),
        (WPR2_ADDR_LO_580, GspReg::Wpr2AddrLo, "580"),
        (WPR2_ADDR_HI_580, GspReg::Wpr2AddrHi, "580"),
    ] {
        assert_eq!(
            m.decode_reg(0, off),
            Some(want),
            "★ the {era}-era WPR2 address {off:#010x} is UNCLAIMED — a guest on that \
             driver would read 0 and never see WPR2 come up"
        );
    }
    // NON-VACUITY: the two eras really are different addresses.
    assert_ne!(0x0088_A828u64, WPR2_ADDR_HI_580);
}

/// ★★★ **The GB202 FSP-boot-complete gate is claimed, and it answers `0xFF`.**
///
/// `_kfspWaitBootCond_GB202` polls `NV_THERM_I2CS_SCRATCH` for
/// `_FSP_BOOT_COMPLETE_STATUS_SUCCESS` before RM may send a single FSP packet
/// (`ogkm-580: src/nvidia/src/kernel/gpu/fsp/arch/blackwell/kern_fsp_gb202.c:45-57`,
/// values at `blackwell/gb202/dev_therm_addendum.h:27-30`). The register is at
/// **`0x00ad00bc`** on GB202 (`blackwell/gb202/dev_therm.h:27`) and at `0x000200bc` on
/// Hopper and GB100 — so reaching for the wrong Blackwell directory gets the wrong address.
#[test]
fn the_gb202_fsp_boot_complete_gate_is_served_at_the_moved_offset() {
    let m = Gb20xGspModel::new();
    let seq = m.boot_sequence();
    let state = ArchBootState::default();
    let ctx = BootContext::default();

    const GB202_THERM_I2CS_SCRATCH: u64 = 0x00AD_00BC;
    const HOPPER_THERM_I2CS_SCRATCH: u64 = 0x0002_00BC;
    const FSP_BOOT_COMPLETE_SUCCESS: u64 = 0xFF;

    assert!(
        seq.may_read(0, GB202_THERM_I2CS_SCRATCH),
        "★ the gate must be CLAIMED, or `RegPlane::read_inner` never asks for it and the \
         guest reads a defaulted 0 with no fault (w567)"
    );
    assert_eq!(
        seq.on_read(&m, 0, GB202_THERM_I2CS_SCRATCH, &ctx, &state),
        Some(FSP_BOOT_COMPLETE_SUCCESS),
        "★ the gate must read _STATUS_SUCCESS (0xFF), or the guest never sends a COT packet"
    );
    // ⊘ …and the HOPPER offset must NOT be claimed by this model. Serving it would hide a
    // wrong-directory mistake by answering both, which is the opposite of a refusal.
    assert!(
        !seq.may_read(0, HOPPER_THERM_I2CS_SCRATCH),
        "★ 0x000200bc is Hopper's/GB100's NV_THERM_I2CS_SCRATCH, not GB202's. Claiming it \
         here would make reaching for the wrong header invisible"
    );
    assert_ne!(GB202_THERM_I2CS_SCRATCH, HOPPER_THERM_I2CS_SCRATCH);
}

/// ⊘ **The registers this generation does NOT have are refused, not defaulted.**
///
/// `gpuWaitForGfwBootComplete` binds the not-supported stub for every chip past AD107
/// (`ogkm-580: g_gpu_nvoc.c:2378-2385`), so GB202 never polls
/// `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05`; and the FSP boot path has no SEC2 Booter
/// Load/Unload convention. Five `GspReg` variants therefore have no offset here, and
/// answering any of them with a zero would be inventing a register.
#[test]
fn the_five_absent_registers_refuse_in_both_directions() {
    let m = Gb20xGspModel::new();
    let obs = kayfabe_arch::gsp::GspObservation::default();
    let absent = [
        GspReg::GfwBootProgress,
        GspReg::GfwBootPlm,
        GspReg::Sec2FalconCpuctl,
        GspReg::Sec2FalconMailbox0,
        GspReg::Sec2FalconDmatrfcmd,
    ];
    for reg in absent {
        assert_eq!(
            Gb20xGspModel::at(reg),
            None,
            "★ {reg:?} has no offset on GB202 — see the module docs"
        );
        assert_eq!(
            m.encode(reg, &obs),
            None,
            "★ {reg:?} must raise RegisterUnserviceable, not decode to zero"
        );
    }
    // NON-VACUITY: the registers this generation DOES have must be answered, or the five
    // `None`s above are a model that refuses everything.
    for reg in [
        GspReg::GspFalconCpuctl,
        GspReg::GspFalconHwcfg2,
        GspReg::GspRiscvCpuctl,
        GspReg::Wpr2AddrHi,
        GspReg::GspQueueHead(0),
    ] {
        assert!(
            Gb20xGspModel::at(reg).is_some() && m.encode(reg, &obs).is_some(),
            "★ NON-VACUITY: {reg:?} is present on GB202 and must be served"
        );
    }
}

/// ★ Every offset this model claims must round-trip: `at(reg)` then `decode_reg` must come
/// back to the same variant. A transposed constant survives a spot check and not this.
#[test]
fn every_claimed_register_round_trips_through_its_own_offset() {
    let m = Gb20xGspModel::new();
    let mut checked = 0usize;
    let all = [
        GspReg::GspFalconCpuctl,
        GspReg::GspFalconHwcfg2,
        GspReg::GspFalconDmatrfcmd,
        GspReg::GspFalconMailbox0,
        GspReg::GspFalconMailbox1,
        GspReg::GspFalconIrqstat,
        GspReg::GspFalconIrqmask,
        GspReg::GspFalconIrqdest,
        GspReg::GspFalconIrqsclr,
        GspReg::GspRiscvCpuctl,
        GspReg::GspRiscvIrqmask,
        GspReg::GspRiscvIrqdest,
        GspReg::Wpr2AddrLo,
        GspReg::Wpr2AddrHi,
        GspReg::GspQueueHead(0),
        GspReg::GspQueueHead(7),
    ];
    for reg in all {
        let (bar, off) = Gb20xGspModel::at(reg).expect("present on GB202");
        assert_eq!(
            m.decode_reg(bar, off),
            Some(reg),
            "★ {reg:?} is placed at {off:#010x} and decodes to something else"
        );
        checked += 1;
    }
    assert_eq!(checked, 16, "★ NON-VACUITY: 16 registers, not {checked}");

    // `NV_PGSP_QUEUE_HEAD__SIZE_1` is 8 (`ogkm-580: blackwell/gb20b/dev_gsp.h:34`), so the
    // ninth queue does not exist and must not be placed.
    assert_eq!(Gb20xGspModel::at(GspReg::GspQueueHead(8)), None);
}
