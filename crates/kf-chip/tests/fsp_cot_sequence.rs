//! ★ 2026-09-26 (`V3_FAMILY_PORT_BLACKWELL.md`) — the FSP regime's boot sequence over the packets
//! RM actually sends on a GB20x, in order: the clock-boost `CAPS_QUERY` and `CLOCK_BOOST`
//! (`kfspCheckForClockBoostCapability_GB100`, `kfspSendClockBoostRpc_GB100`) and then the COT
//! (`kfspSendBootCommands_GH100`). Only the COT boots the GSP, and the boot-args address it carries
//! names `GSP_FMC_BOOT_PARAMS`, which points at the LibOS array (`boot_args_indirection`).
use kf_arch::gsp::{ArchBootState, BootContext, BootStep, RegWrite};
use kf_chip::fsp_gsp::FMC_BOOT_PARAMS_BOOT_ARGS_OFFSET;

const EMEMC: u64 = 0x008F_2AC0;
const EMEMD: u64 = 0x008F_2AC4;
const QUEUE_HEAD: u64 = 0x008F_2C00;
const QUEUE_TAIL: u64 = 0x008F_2C04;

/// One packet the way `kfspSendPacket_GH100` sends it; returns the steps its HEAD write meant.
fn send(m: &dyn kf_arch::gsp::GspModel, st: &mut ArchBootState, words: &[u32]) -> Vec<BootStep> {
    let seq = m.boot_sequence();
    let ctx = BootContext::default();
    let w = |off: u64, val: u64| RegWrite { bar: 0, off, val, reg: m.decode_reg(0, off) };
    let _ = seq.on_write(m, &w(EMEMC, 1 << 24), &ctx, st);
    for v in words {
        let _ = seq.on_write(m, &w(EMEMD, u64::from(*v)), &ctx, st);
    }
    let _ = seq.on_write(m, &w(QUEUE_TAIL, (words.len() as u64) * 4 - 4), &ctx, st);
    seq.on_write(m, &w(QUEUE_HEAD, 0), &ctx, st).iter().collect()
}

fn packet(nvdm: u32, dwords: usize, boot_args: Option<u64>) -> Vec<u32> {
    let mut p = vec![0u32; dwords];
    p[0] = 0xC000_0000;
    p[1] = 0x7e | (0x10de << 8) | (nvdm << 24);
    if let Some(a) = boot_args {
        // `NVDM_PAYLOAD_COT.gspBootArgsSysmemOffset` at payload byte 852 (packed) = packet 860.
        p[860 / 4] = a as u32;
        p[860 / 4 + 1] = (a >> 32) as u32;
    }
    p
}

#[test]
fn only_the_cot_boots_the_gsp_and_its_address_is_the_fmc_params() {
    let m = kf_chip::Family::Blackwell.gsp_model(0x3, 8192).unwrap();
    assert_eq!(m.boot_sequence().boot_args_indirection(), Some(FMC_BOOT_PARAMS_BOOT_ARGS_OFFSET));
    assert_eq!(FMC_BOOT_PARAMS_BOOT_ARGS_OFFSET, 48);
    let mut st = ArchBootState::default();
    // NVDM_TYPE_CAPS_QUERY / NVDM_TYPE_CLOCK_BOOST — short packets: FSP answers, no boot.
    assert!(send(&*m, &mut st, &packet(0x18, 3, None)).is_empty());
    assert!(send(&*m, &mut st, &packet(0x1a, 3, None)).is_empty());
    // The COT (868 bytes): starts the FMC, loads GSP-RM, publishes the FMC params' address.
    let fmc = 0x1_2345_6000u64;
    let steps = send(&*m, &mut st, &packet(0x14, 217, Some(fmc)));
    assert_eq!(steps, vec![BootStep::StartProcessor, BootStep::FirmwareLoaded, BootStep::PublishBootArgs(fmc)]);
    // ⊘ A later short packet does NOT re-publish the COT the window still holds (a second open's
    // CAPS_QUERY before its own COT).
    assert!(send(&*m, &mut st, &packet(0x18, 3, None)).is_empty());
}

#[test]
fn a_cot_too_short_to_carry_the_field_publishes_nothing() {
    let m = kf_chip::Family::Hopper.gsp_model(0x0, 8192).unwrap();
    let mut st = ArchBootState::default();
    assert!(send(&*m, &mut st, &packet(0x14, 100, None)).is_empty());
}
