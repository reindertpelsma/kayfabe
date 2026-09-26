//! E10a — the GA10x CE codec decodes the per-operand **phys-mode target**, the residency
//! signal a physical copy-engine operand carries and nothing else does.
//!
//! `memmgrTestCeUtils` (the wall at `mem_mgr.c:463`) issues a **physical** CE copy
//! `sys ← vid`: the source is `_TARGET_LOCAL_FB` (the emulated framebuffer) and the
//! destination is `_TARGET_COHERENT_SYSMEM` (guest RAM)
//! (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:1069-1082`). Without the
//! target decoded, those two operands are indistinguishable and their addresses can even
//! collide numerically, so the residency question the CPU branch turns on is unanswerable.
//! This is the decode; the residency split that consumes it is E10b.

use kayfabe_abi::submit;
use kayfabe_arch::{Arch, PhysTarget, PushMethod};
use kayfabe_chips::Ga10xArch;

/// Build the five method runs of one physical `AMPERE_DMA_COPY_B` copy, with the given
/// `SET_SRC_PHYS_MODE`/`SET_DST_PHYS_MODE` target values, and decode them through the
/// shipped GA10x codec against one engine-method state — the engine's own accumulation.
fn decode_physical_copy(src_target: u32, dst_target: u32) -> Vec<PushMethod> {
    let a = Ga10xArch::new();
    let hdr = |m, n| submit::method_header_inc(0, m, n).expect("encodable");
    // SET_SRC_PHYS_MODE (0x260) and SET_DST_PHYS_MODE (0x264) are consecutive, so one
    // incrementing run of count 2 writes both — exactly as `channel_utils.c` emits them.
    let phys = (
        hdr(submit::ce::SET_SRC_PHYS_MODE, 2),
        vec![src_target, dst_target],
    );
    let bind = (
        hdr(submit::SET_OBJECT, 1),
        vec![kayfabe_abi::generated::classes::AMPERE_DMA_COPY_B],
    );
    let offs = (
        hdr(submit::ce::OFFSET_IN_UPPER, 4),
        vec![0x1, 0x1000, 0x1, 0x2000],
    );
    let line = (hdr(submit::ce::LINE_LENGTH_IN, 2), vec![0x40, 1]);
    // A PHYSICAL copy: both `_TYPE_PHYSICAL` bits set, a real transfer type, no multi-line,
    // no remap.
    let fire = (
        hdr(submit::ce::LAUNCH_DMA, 1),
        vec![
            submit::ce::LAUNCH_TRANSFER_NON_PIPELINED
                | submit::ce::LAUNCH_SRC_PHYSICAL
                | submit::ce::LAUNCH_DST_PHYSICAL,
        ],
    );
    let mut st = kayfabe_arch::MethodState::new();
    a.pushbuffer()
        .decode_run(&mut st, &[bind, phys, offs, line, fire])
}

fn only_launch(methods: &[PushMethod]) -> PushMethod {
    let launches: Vec<&PushMethod> = methods
        .iter()
        .filter(|m| matches!(m, PushMethod::CeLaunchDma { .. }))
        .collect();
    assert_eq!(
        launches.len(),
        1,
        "exactly one launch fires per run, got {methods:?}"
    );
    *launches[0]
}

/// ★★★ The residency of each physical operand is recovered from its phys-mode target —
/// `LOCAL_FB` on the source, `COHERENT_SYSMEM` on the destination — mirroring
/// `memmgrTestCeUtils`' `sys ← vid` copy.
#[test]
fn ce_phys_mode_decodes_the_operand_residency() {
    let got = decode_physical_copy(
        0, // SET_SRC_PHYS_MODE_TARGET_LOCAL_FB
        1, // SET_DST_PHYS_MODE_TARGET_COHERENT_SYSMEM
    );
    let PushMethod::CeLaunchDma {
        dst_is_virtual,
        src_is_virtual,
        dst_target,
        src_target,
        ..
    } = only_launch(&got)
    else {
        unreachable!("filtered")
    };
    assert!(!src_is_virtual && !dst_is_virtual, "a physical copy");
    assert_eq!(src_target, PhysTarget::LocalFb, "vid source is LOCAL_FB");
    assert_eq!(
        dst_target,
        PhysTarget::CoherentSysmem,
        "sys destination is COHERENT_SYSMEM — the guest-RAM residency the CPU branch needs"
    );
}

/// The other two enumerated targets decode distinctly — non-vacuity that the decode reads
/// the field rather than answering a constant.
#[test]
fn ce_phys_mode_decodes_every_enumerated_target() {
    for (raw, want) in [
        (0u32, PhysTarget::LocalFb),
        (1, PhysTarget::CoherentSysmem),
        (2, PhysTarget::NonCoherentSysmem),
        (3, PhysTarget::Peer),
    ] {
        let got = decode_physical_copy(raw, raw);
        let PushMethod::CeLaunchDma {
            src_target,
            dst_target,
            ..
        } = only_launch(&got)
        else {
            unreachable!("filtered")
        };
        assert_eq!(src_target, want, "SRC target for raw {raw}");
        assert_eq!(dst_target, want, "DST target for raw {raw}");
    }
}

/// ⊘ A physical operand that never wrote its phys-mode register is `LocalFb` — the engine's
/// own reset value (`ogkm-580: clc7b5.h:68`), faithful rather than a guess. This is the
/// arm that must NOT be a refusal: hardware performs such a copy.
#[test]
fn ce_phys_mode_unlatched_is_local_fb_the_reset_value() {
    let a = Ga10xArch::new();
    let hdr = |m, n| submit::method_header_inc(0, m, n).expect("encodable");
    // No SET_*_PHYS_MODE run at all, but both operands physical.
    let bind = (
        hdr(submit::SET_OBJECT, 1),
        vec![kayfabe_abi::generated::classes::AMPERE_DMA_COPY_B],
    );
    let offs = (
        hdr(submit::ce::OFFSET_IN_UPPER, 4),
        vec![0x1, 0x1000, 0x1, 0x2000],
    );
    let line = (hdr(submit::ce::LINE_LENGTH_IN, 2), vec![0x40, 1]);
    let fire = (
        hdr(submit::ce::LAUNCH_DMA, 1),
        vec![
            submit::ce::LAUNCH_TRANSFER_NON_PIPELINED
                | submit::ce::LAUNCH_SRC_PHYSICAL
                | submit::ce::LAUNCH_DST_PHYSICAL,
        ],
    );
    let mut st = kayfabe_arch::MethodState::new();
    let got = a
        .pushbuffer()
        .decode_run(&mut st, &[bind, offs, line, fire]);
    let PushMethod::CeLaunchDma {
        src_target,
        dst_target,
        ..
    } = only_launch(&got)
    else {
        unreachable!("filtered")
    };
    assert_eq!(src_target, PhysTarget::LocalFb);
    assert_eq!(dst_target, PhysTarget::LocalFb);
}
