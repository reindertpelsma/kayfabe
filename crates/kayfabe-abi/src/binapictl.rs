//! `0x20810108` — the one control on the **binary-API subdevice class**
//! ([`crate::capability::NV2081_BINAPI_CLASS`]), and the **first status divergence from real
//! hardware** in the whole of a CUDA program's ioctl stream.
//!
//! # Where it comes from, and why it is ours to answer
//!
//! `binapiControl_IMPL` interprets nothing. On a GSP client (`IS_FW_CLIENT`) it takes the
//! subdevice group lock and forwards the caller's buffer **whole**, under the **same command
//! id**, to physical RM — which on this port is the emulated GSP
//! (`ogkm-580: src/nvidia/src/kernel/rmapi/binary_api.c:104-121`; the `IS_VIRTUAL` arm two
//! branches up does the same thing through `NV_RM_RPC_API_CONTROL`). There is no kernel-side
//! implementation to shadow and no NVOC entry to read a `paramSize` off: the class exists
//! precisely so that a command the guest kernel does not understand reaches firmware intact.
//!
//! ⇒ It arrives at this port as an ordinary `fn 76 GSP_RM_CONTROL`, and
//! `[measured 2026-08-10, boot `w221` at `49dc3ec`]` it did:
//! `traces/guest_boots/run_w221_49dc3ec_grfwd_qemu.log:447` carries
//! `unserviced fn 76 cmd 0x20810108`. ⊘ That is the positive statement that **nothing**
//! answered it — the shape [`crate::gsslegacy`] documents for its own ids.
//!
//! ★ **Where the host reference cited below lives.** `nvdiff` is the host↔guest ioctl
//! differential; its captures are committed in the **research-artifact repo**, not this one —
//! `nvidia-gpu-passthrough` branch `status-divergence`, `traces/host_reference_ga106/` (host)
//! and `traces/guest_mode2_vh2/` (guest), each with a `MANIFEST.txt` carrying its provenance
//! and its measured noise floor. Paths below are relative to that repo.
//!
//! # ★★★ What hardware does with it — and the honest limit of that measurement
//!
//! `[measured 2026-08-10, real GA106, nvdiff host reference]`
//! `traces/host_reference_ga106/ctx_r1.jsonl.zst` record **77**: `paramsSize = 992`,
//! `rc = 0`, `NV status = NV_OK`, and the 992-byte body is **identical before and after the
//! ioctl**. `nvdiff.py replies` classes the id `pure-IN (RM wrote nothing)` over the whole
//! 608-record program — 1 call, 992 bytes declared, 992 read back, **0 changed**.
//!
//! ⚠⚠ **`in == out` is the weakest possible measurement here, and this module says so rather
//! than dressing it up.** [`crate::gsslegacy`] could rule the objection out structurally: a
//! GSS-legacy command bypasses resserv, so no `RMAPI_PARAM_COPY_FLAGS_SKIP_COPYOUT` can exist
//! on its path and the copy-out provably ran. **This id has no such argument** — it goes
//! through resserv like any other control — and, worse, *the measured request body is 992
//! **zero** bytes*, so on this capture "RM copied 992 unchanged bytes back", "RM skipped the
//! copy-out entirely" and "RM wrote 992 zeros" are **three hypotheses one measurement cannot
//! separate**. Our own guest sends the same all-zero body — this repo,
//! `traces/guest_boots/run_w210_8574466_ctl_probe.log:426` (boot `w210` at `8574466`),
//! `in=0000…`.
//!
//! ★ **So the echo here is chosen as a RULE, not read off the reply.** `[[an-echo-is-
//! unverifiable-by-its-reply]]`. The rule is justified by what the three hypotheses have in
//! common rather than by choosing between them:
//!
//! * if RM copied the buffer back unchanged, an echo reproduces it exactly;
//! * if RM skipped the copy-out, the caller keeps its own bytes and an echo is
//!   indistinguishable from that — the guest's RM writes our reply into *its* kernel params
//!   buffer, and whether that reaches userspace is the guest's decision, not ours;
//! * if RM wrote zeros, an echo of an all-zero request reproduces that too.
//!
//! ⊘ The one answer that is wrong under **all three** is the one this port gives today: an
//! **empty** reply body, which the transport zero-fills to full length
//! (`[[an-in-annotation-is-not-a-transport-fact]]` — an empty reply body is a full-length
//! zero-fill, and RM copies it back regardless of direction markings). For an all-zero request
//! that happens to coincide with hypothesis three; for any non-zero request it silently
//! overwrites the caller's own words. The echo is the strictly safer rule, and it is the only
//! one of the two that stays correct if a future `libcuda` sends a non-zero body.
//!
//! ⊘ **What serving it does NOT do.** `[measured 2026-08-08, real GA106,
//! `execution_plane_increments.md` §14.27]` forcing *this control* to `NV_ERR_NOT_SUPPORTED`
//! and nothing else leaves `cuInit(0) = 0`; it is the **class** [`crate::generated::classes::
//! NV2081_BINAPI`] that is load-bearing, not the command on it. So this closes a status
//! divergence and buys no rung. Anyone reading a boot that advances after this landed should
//! look elsewhere for the cause.

/// The one control on the binary-API subdevice class that a CUDA program issues.
///
/// ⊘ Deliberately **not** given an invented symbolic name, for [`crate::gsslegacy`]'s reason:
/// it appears in no SDK header, in no NVOC export table, in `nvproxy`'s maps or in the C
/// artifact's tables — the class exists to carry commands the kernel cannot name. A plausible
/// invented name would be the worst thing this constant could carry.
pub const BINAPI_CTRL_0X0108: u32 = 0x2081_0108;

/// Its `paramsSize`, as **both** sides declare it on the wire.
///
/// `[measured 2026-08-10]` real GA106 (nvdiff host reference, record 77) and our own guest
/// (`traces/guest_boots/run_w210_8574466_ctl_probe.log:426`) both declare `992`.
///
/// ⚠ There is **no** third source and there cannot be one: `binapiControl_IMPL` forwards
/// `pParams->paramsSize` unexamined, so no table in the driver states it. Two independent
/// callers agreeing is the whole of the evidence, which is why [`answer_binapi_0108`] refuses a
/// buffer of any other length instead of padding or truncating one.
pub const BINAPI_CTRL_0X0108_PARAMS_SIZE: usize = 992;

/// Why a binary-API control could not be answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinApiCtrlError {
    /// A command id this module does not serve reached it — the caller's dispatch is wrong,
    /// and answering would be claiming to have served an id nobody measured.
    NotThisCommand {
        /// The id that arrived.
        cmd: u32,
    },
    /// The buffer is not [`BINAPI_CTRL_0X0108_PARAMS_SIZE`] bytes.
    ///
    /// ⊘ Refused rather than resolved in either direction: the size is the guest's own
    /// assertion and the only statement of it that exists, so a mismatch means the caller is
    /// not the caller this was measured against.
    WrongSize {
        /// What arrived.
        got: usize,
        /// What both measured callers declare.
        want: usize,
    },
}

impl core::fmt::Display for BinApiCtrlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotThisCommand { cmd } => write!(
                f,
                "{cmd:#010x} is not the binary-API control this module measured; the class \
                 carries commands the kernel cannot name, so an unmeasured id is refused \
                 rather than echoed"
            ),
            Self::WrongSize { got, want } => write!(
                f,
                "the binary-API control's only two measured callers both declare {want} bytes \
                 and {got} arrived"
            ),
        }
    }
}

impl core::error::Error for BinApiCtrlError {}

/// Answer `0x20810108` with the request body, unchanged.
///
/// ★ The identity is written out as a copy rather than left to the serve site's
/// splice-the-params-back tail, for [`crate::gsslegacy::answer_gss_legacy`]'s reason: the echo
/// is then something the code **says**, and this is where the id and the length are checked.
///
/// # Errors
///
/// [`BinApiCtrlError::NotThisCommand`] for any id other than [`BINAPI_CTRL_0X0108`], and
/// [`BinApiCtrlError::WrongSize`] for a body that is not
/// [`BINAPI_CTRL_0X0108_PARAMS_SIZE`] bytes.
pub fn answer_binapi_0108(cmd: u32, params: &[u8]) -> Result<Vec<u8>, BinApiCtrlError> {
    if cmd != BINAPI_CTRL_0X0108 {
        return Err(BinApiCtrlError::NotThisCommand { cmd });
    }
    if params.len() != BINAPI_CTRL_0X0108_PARAMS_SIZE {
        return Err(BinApiCtrlError::WrongSize {
            got: params.len(),
            want: BINAPI_CTRL_0X0108_PARAMS_SIZE,
        });
    }
    Ok(params.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured shape: 992 bytes in, the same 992 bytes out.
    #[test]
    fn the_measured_body_is_echoed_verbatim() {
        let req = vec![0u8; BINAPI_CTRL_0X0108_PARAMS_SIZE];
        let out = answer_binapi_0108(BINAPI_CTRL_0X0108, &req).expect("the measured length");
        assert_eq!(out, req);
    }

    /// ★ The non-vacuity instrument for the echo. The captured request is **all zeros**, so a
    /// test built only from the capture cannot tell an echo from a zero-fill — which is
    /// exactly the ambiguity the module docs refuse to hide. This drives a body no capture
    /// contains, so the assertion has something to bite.
    #[test]
    fn a_non_zero_body_comes_back_unchanged_not_zeroed() {
        let mut req = vec![0u8; BINAPI_CTRL_0X0108_PARAMS_SIZE];
        for (i, b) in req.iter_mut().enumerate() {
            *b = u8::try_from(i % 251).expect("a byte");
        }
        let out = answer_binapi_0108(BINAPI_CTRL_0X0108, &req).expect("the measured length");
        assert_eq!(out, req, "the reply must be the request, not a zero-fill");
        assert!(
            out.iter().any(|b| *b != 0),
            "a zero-filled reply would pass an all-zero fixture and fail here"
        );
    }

    /// A length neither measured caller declares is refused, never padded or truncated.
    #[test]
    fn a_length_no_caller_declares_is_refused() {
        for len in [0usize, 991, 993] {
            let req = vec![0u8; len];
            assert_eq!(
                answer_binapi_0108(BINAPI_CTRL_0X0108, &req),
                Err(BinApiCtrlError::WrongSize {
                    got: len,
                    want: BINAPI_CTRL_0X0108_PARAMS_SIZE,
                })
            );
        }
    }

    /// The id is checked here, so a mis-wired dispatch is a refusal rather than an echo of
    /// somebody else's buffer under this module's name.
    #[test]
    fn another_command_id_is_refused() {
        let req = vec![0u8; BINAPI_CTRL_0X0108_PARAMS_SIZE];
        assert_eq!(
            answer_binapi_0108(0x2081_0107, &req),
            Err(BinApiCtrlError::NotThisCommand { cmd: 0x2081_0107 })
        );
    }
}
