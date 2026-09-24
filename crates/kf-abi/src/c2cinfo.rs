//! `NV2080_CTRL_CMD_BUS_GET_C2C_INFO` (`0x2080182b`) — ★★ **the one served row whose value is
//! right by ARGUMENT rather than by capture**, and the distinction is the reason it has its own
//! module.
//!
//! # Routing
//!
//! Flags `0x50048` (`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:6826`) carry
//! `NON_PRIVILEGED(0x8) | ROUTE_TO_PHYSICAL(0x40) | …(0x10000) |
//! PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST(0x40000)`, so `rmresControl_Prologue_IMPL` RPCs it to
//! physical RM unchanged (`resource.c:255-291`) — ours to serve. `[measured 2026-08-09, boot
//! `gf1436` at `ec434b8`]` `cuInit` reaches it as row 86 of 87 and gets `0x56`; a real GA106
//! answers `NV_OK` (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:86`).
//!
//! # ★★★ Why an all-zero reply is a STATEMENT here and would be a defect elsewhere
//!
//! `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:86` shows all 28 bytes zero in **both**
//! directions, and this repository's standing rule is that such a row is evidence of nothing —
//! it is the `dlen = 0` shape, and possibly the `SKIP_COPYOUT` shape as well
//! (`crate::oracle`, and `crate::gsslegacy` for the one id where that objection is answerable).
//!
//! ⊘ **This row does not rest on that capture at all**, which is what makes it sound. `bIsLinkUp
//! = false` is the **true** answer for this part on first principles: C2C is NVIDIA's
//! chip-to-chip fabric, present on Grace-Hopper-class parts, and a GA106 is a consumer GeForce
//! die with no C2C links to bring up. Every remaining field is defined only when a link exists —
//! `nrLinks`, `linkMask`, `perLinkBwMBps` and the rest describe links this silicon does not have
//! — so zero is their meaning, not their absence.
//!
//! ★ The capture is therefore **corroboration**, not the source. That ordering matters: the
//! fifth-limit lesson is that a citation to an empty body is *"citing the oracle, not the oracle
//! being right"*, and an argument that would survive the capture being deleted is the only kind
//! that escapes it. This one would.
//!
//! ⚠ The residue, stated: this is a claim about **GA106**, not about Ampere and not about
//! GeForce. A part with C2C answers something else, and [`c2c_absent`] takes the fact as a
//! parameter rather than hard-coding it so that a future chip row states its own.

/// `NV2080_CTRL_CMD_BUS_GET_C2C_INFO`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080bus.h:1381`).
pub const NV2080_CTRL_CMD_BUS_GET_C2C_INFO: u32 = 0x2080_182b;

/// `sizeof(NV2080_CTRL_CMD_BUS_GET_C2C_INFO_PARAMS)` — `NvBool + NvBool` then six `NvU32`
/// (`ogkm-580: ctrl2080bus.h:1385-1394`), so 2 bytes of booleans, 2 of padding, 24 of words.
///
/// `[measured 2026-08-09]` the wire agrees: the real GA106 and our own boot `gf1436` both
/// declare `size=28`.
pub const C2C_INFO_PARAMS_SIZE: usize = 28;

/// Byte offset of `bIsLinkUp` — the field every other one is conditioned on.
pub const B_IS_LINK_UP_OFF: usize = 0;

/// The `[OUT]` reply for a part with no chip-to-chip fabric.
///
/// ★ Takes `has_c2c` rather than assuming it, so that the day a chip row describes a part with
/// C2C this function refuses to answer for it instead of quietly reporting "no links" about
/// silicon that has them.
///
/// # Errors
///
/// [`C2cError::NotModelled`] when asked about a part that *does* have C2C — this port has never
/// seen one and has no measurement to offer, and a confident zero would be the
/// `accuracy_is_fatal_when_a_fallback_was_keyed_on_ignorance` shape.
pub fn c2c_absent(has_c2c: bool) -> Result<Vec<u8>, C2cError> {
    if has_c2c {
        return Err(C2cError::NotModelled);
    }
    // Every field is zero, and `bIsLinkUp = 0` is the one that gives the rest their meaning.
    Ok(vec![0u8; C2C_INFO_PARAMS_SIZE])
}

/// Why a C2C reply could not be stated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum C2cError {
    /// The part has C2C links and this port has no measurement of a populated reply.
    NotModelled,
}

impl core::fmt::Display for C2cError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotModelled => write!(
                f,
                "this part has C2C links and no populated NV2080_CTRL_CMD_BUS_GET_C2C_INFO \
                 reply has ever been measured; reporting 'no links' would be a confident \
                 wrong answer about real silicon"
            ),
        }
    }
}

impl core::error::Error for C2cError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_without_c2c_reports_every_link_field_zero() {
        let body = c2c_absent(false).expect("GA106 has no C2C");
        assert_eq!(body.len(), C2C_INFO_PARAMS_SIZE);
        assert!(body.iter().all(|&b| b == 0));
        assert_eq!(body[B_IS_LINK_UP_OFF], 0);
    }

    #[test]
    fn a_part_with_c2c_is_refused_rather_than_told_it_has_none() {
        // ⊘ The falsifier for the module's central claim. Zero is only the right answer
        // *because* the part has no links; a function that returned it either way would be
        // stating a fact about silicon that no run of ours covers — this port has never seen
        // a C2C part, so the honest output is a refusal.
        assert_eq!(c2c_absent(true), Err(C2cError::NotModelled));
    }

    #[test]
    fn the_size_is_the_structs_arithmetic_and_not_a_captured_literal() {
        // 2 booleans + 2 padding + 6 words. Checked as arithmetic so a struct change that
        // kept the old literal is red.
        assert_eq!(C2C_INFO_PARAMS_SIZE, 2 + 2 + 6 * 4);
        assert_eq!(C2C_INFO_PARAMS_SIZE, 28);
    }
}
