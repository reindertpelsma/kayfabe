//! `NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_SET_2X` (`0x20800a9a`) — the P-state boost request,
//! and ★★★ **the id the userspace-level worklist did not name.**
//!
//! # ⊘ THE REFUTATION THIS MODULE EXISTS TO RECORD
//!
//! The ioctl differential's `0x56` census named **`0x2080200a` `NV2080_CTRL_CMD_PERF_BOOST`**
//! as the second status divergence from hardware: guest `0x56`, hardware `NV_OK`, the second
//! ioctl of `cuCtxCreate` on both sides. That is a true statement about the **ioctl boundary**
//! and a false lead about **this port**, because `0x2080200a` never reaches this port at all.
//!
//! `[measured 2026-08-10, over every committed device log]`
//! `grep -l 0x2080200a traces/guest_boots/*_qemu.log` returns **0 files**;
//! `grep -l 0x20800a9a` returns **38**. The device's own unserviced census — the instrument
//! whose whole job is to say *"the guest asked and nobody answered"* — has recorded
//! `unserviced fn 76 cmd 0x20800a9a` in thirty-eight boots and `0x2080200a` in **none**.
//! ⇒ Serving `0x2080200a` would have been **dead code with a passing unit test**: an arm in
//! `WantedTable` for an id the wire never carries, indistinguishable at every gate except a
//! live boot from a fix that works.
//!
//! ★ The driver source says why, and it is not subtle. `0x2080200a` has a **kernel-side
//! implementation**: NVOC row 453 gives it `pFunc = subdeviceCtrlCmdKPerfBoost_IMPL`, flags
//! `0x10318` (`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:6940-6954`). That function
//! checks `pKernelPerf != NULL` and then calls `kperfBoostSet`, which **re-packages the same
//! two fields under a different command id** and sends *that* to physical RM —
//! i.e. to us:
//!
//! ```text
//! boostParams2x.flags    = pBoostParams->flags;      // ogkm-580: kern_perf_boost.c:85-86
//! boostParams2x.duration = pBoostParams->duration;
//! pRmApi->Control(pRmApi, …, NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_SET_2X, &boostParams2x, …);
//! ```
//!
//! (`ogkm-580: src/nvidia/src/kernel/gpu/perf/kern_perf_boost.c:44-108`). The status of *that*
//! call is returned unchanged up through `kperfBoostSet` and `subdeviceCtrlCmdKPerfBoost_IMPL`
//! to the guest's userspace. ⇒ The `0x56` the differential measured on `0x2080200a` **is this
//! port's refusal of `0x20800a9a`, one translation later.** The census and the differential
//! were looking at the same event through two boundaries and calling it two ids.
//!
//! ⊘ **`kperfGpuBoostSyncStateInit` is a different failure and is NOT fixed here.** The same
//! guest dmesg carries `kperfGpuBoostSyncStateInit_IMPL: Failed to read Sync Gpu Boost init
//! state, status=0x56` twice per `cuCtxCreate` attempt; that is a *different* internal control
//! and this module does not touch it. Naming it is the point — after this lands,
//! `KernelPerf` still logs, and a reader must not read that line as this fix failing.
//!
//! # What the reply can honestly say
//!
//! `NV2080_CTRL_INTERNAL_PERF_BOOST_SET_PARAMS_2X` is `{ NvBool flags; NvU32 duration; }`
//! (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:1706-1710`) — **both
//! fields are `[in]`**, the header documents no output, and the caller reads nothing back. So
//! the entire content of a correct answer is *"received, and here is what I read"*: the same
//! shape as [`crate::fmbpromote`], and the reply is re-encoded from the decoded view rather
//! than copied, so a value the validation rejected cannot travel around the check inside the
//! same buffer.
//!
//! ⚠⚠ **This is an ACKNOWLEDGEMENT, not a performance.** This port manages no clock domain and
//! no P-state table; it forwards compute to a host GPU whose own driver governs that GPU's
//! clocks. Answering `NV_OK` says the request was received and that nothing in this device
//! opposes it — it does **not** say a boost happened. That is defensible only because the
//! reply carries **no fact of ours**: every field is the guest's own, and there is no `[out]`
//! word for us to invent. ⊘ The moment this control gains an output — or the moment a guest
//! can read a clock through a path this port serves — the acknowledgement becomes a claim and
//! this argument expires.
//!
//! ★ And the alternative is not neutral. `0x56` here is a divergence from hardware on the
//! **second ioctl of every `cuCtxCreate`**, in a driver where `NV_ERR_NOT_SUPPORTED` is the
//! FSM's signature for *"nobody claimed this"* — so leaving it is leaving a permanent false
//! positive in the one instrument that says what this port has not built.
//!
//! ⊘ **What serving it does NOT do.** `libcuda` demonstrably survives the refusal: the
//! differential's guest reached 221 further calls of `cuCtxCreate` past this point before
//! parting company with hardware somewhere else entirely. This closes a status divergence. It
//! is not the wall.

/// The internal P-state boost command physical RM receives.
///
/// ⊘ **Not** `NV2080_CTRL_CMD_PERF_BOOST` (`0x2080200a`), which the guest kernel implements
/// itself and never forwards — see the module docs for the measurement.
pub const INTERNAL_PERF_BOOST_SET_2X: u32 = 0x2080_0a9a;

/// `sizeof(NV2080_CTRL_INTERNAL_PERF_BOOST_SET_PARAMS_2X)`.
///
/// `NvBool` is `NvU8`, so the `NvU32` behind it is 4-aligned: `flags` at 0, three pad bytes,
/// `duration` at 4, eight in total
/// (`ogkm-580: .../ctrl2080/ctrl2080internal.h:1706-1710`).
///
/// ⚠ The pad bytes are **not** read and **not** asserted about: `kperfBoostSet_IMPL` declares
/// its `boostParams2x` with `= {0}` (`kern_perf_boost.c:82`) so they are zero in practice, but
/// that is a property of one caller's initialiser, not of the wire, and refusing a boot over
/// three bytes nothing reads would be a gate with no subject.
pub const INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE: usize = 8;

/// Byte offset of `flags`.
const FLAGS_OFF: usize = 0;
/// Byte offset of `duration`.
const DURATION_OFF: usize = 4;

/// `NV2080_CTRL_PERF_BOOST_FLAGS_CMD` — `flags[1:0]`.
const FLAGS_CMD_MASK: u8 = 0b11;
/// `_CMD_BOOST_TO_MAX`, the largest defined value of the two-bit command field
/// (`ogkm-580: .../ctrl2080/ctrl2080perf.h:76-78`). `0b11` is undefined.
const FLAGS_CMD_MAX: u8 = 2;
/// Every bit the header defines: `CMD` at `1:0`, `CUDA` at `4:4`, `ASYNC` at `5:5`,
/// `CUDA_PRIORITY` at `6:6` (`ogkm-580: .../ctrl2080/ctrl2080perf.h:76-90`). Bits 3, 2 and 7
/// name nothing.
const FLAGS_DEFINED_BITS: u8 = 0b0111_0011;

/// `NV2080_CTRL_PERF_BOOST_DURATION_MAX` — one hour, in seconds
/// (`ogkm-580: .../ctrl2080/ctrl2080perf.h:92`).
const DURATION_MAX: u32 = 3600;
/// `NV2080_CTRL_PERF_BOOST_DURATION_INFINITE` — "until cleared"
/// (`ogkm-580: .../ctrl2080/ctrl2080perf.h:93`).
const DURATION_INFINITE: u32 = 0xffff_ffff;

/// A boost request, as this port read it.
///
/// ⊘ Exists so the reply can be re-encoded from what the decoder **accepted** rather than
/// copied from what arrived — [`crate::fmbpromote`]'s argument, and the reason a rejected
/// value cannot reach the guest by riding along in the same buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerfBoostRequest {
    /// `flags`, whole. Kept as the byte the guest sent rather than split into its fields:
    /// the reply must reproduce it exactly, and a struct of decoded sub-fields would have to
    /// be re-assembled to do that.
    pub flags: u8,
    /// `duration`, in seconds, or [`DURATION_INFINITE`].
    pub duration: u32,
}

/// Why a boost request could not be acknowledged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerfBoostError {
    /// The buffer is not [`INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE`] bytes.
    WrongSize {
        /// What arrived.
        got: usize,
        /// `sizeof` the struct the header declares.
        want: usize,
    },
    /// `flags` sets a bit the header defines no name for, or names command `0b11`.
    ///
    /// ⊘ Refused rather than masked. An acknowledgement is only honest about a request this
    /// port could **read**; silently dropping bits would answer `NV_OK` to a request nobody
    /// understood, which is the `A WALL THAT CAN CARRY NO NAME` shape.
    UndefinedFlags {
        /// The byte that arrived.
        flags: u8,
    },
    /// `duration` is neither `<= 3600` nor the infinite sentinel — RM's own documented bound
    /// (`ogkm-580: .../ctrl2080/ctrl2080perf.h:92-93`), applied here rather than assumed.
    DurationOutOfRange {
        /// The value that arrived.
        duration: u32,
    },
}

impl core::fmt::Display for PerfBoostError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongSize { got, want } => write!(
                f,
                "NV2080_CTRL_INTERNAL_PERF_BOOST_SET_PARAMS_2X is {want} bytes and {got} arrived"
            ),
            Self::UndefinedFlags { flags } => write!(
                f,
                "boost flags {flags:#04x} name a command or a bit the SDK header does not \
                 define; an acknowledgement of a request this port could not read would be an \
                 NV_OK with no meaning"
            ),
            Self::DurationOutOfRange { duration } => write!(
                f,
                "boost duration {duration} is past NV2080_CTRL_PERF_BOOST_DURATION_MAX ({DURATION_MAX}) \
                 and is not the infinite sentinel"
            ),
        }
    }
}

impl core::error::Error for PerfBoostError {}

/// Read a boost request, refusing every shape this port cannot name.
///
/// # Errors
///
/// [`PerfBoostError::WrongSize`], [`PerfBoostError::UndefinedFlags`] or
/// [`PerfBoostError::DurationOutOfRange`].
pub fn decode_perf_boost_set_2x(params: &[u8]) -> Result<PerfBoostRequest, PerfBoostError> {
    if params.len() != INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE {
        return Err(PerfBoostError::WrongSize {
            got: params.len(),
            want: INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE,
        });
    }
    let flags = params[FLAGS_OFF];
    if flags & !FLAGS_DEFINED_BITS != 0 || flags & FLAGS_CMD_MASK > FLAGS_CMD_MAX {
        return Err(PerfBoostError::UndefinedFlags { flags });
    }
    let duration = u32::from_le_bytes([
        params[DURATION_OFF],
        params[DURATION_OFF + 1],
        params[DURATION_OFF + 2],
        params[DURATION_OFF + 3],
    ]);
    if duration > DURATION_MAX && duration != DURATION_INFINITE {
        return Err(PerfBoostError::DurationOutOfRange { duration });
    }
    Ok(PerfBoostRequest { flags, duration })
}

/// Encode the acknowledgement: the request's own two fields, re-stated.
///
/// ⊘ The three pad bytes are written **zero** rather than preserved. They are unread by RM
/// and unset by its own caller's `= {0}`; writing zero makes the reply a function of the
/// decoded request alone, which is the property the decode-then-re-encode exists for.
#[must_use]
pub fn encode_perf_boost_set_2x(req: &PerfBoostRequest) -> Vec<u8> {
    let mut body = vec![0u8; INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE];
    body[FLAGS_OFF] = req.flags;
    body[DURATION_OFF..DURATION_OFF + 4].copy_from_slice(&req.duration.to_le_bytes());
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ The two requests the differential actually measured on the ioctl boundary, carried
    /// through the translation the guest kernel performs.
    ///
    /// `[measured 2026-08-10, real GA106]` `0x2080200a` records 130 and 478 of the host
    /// reference carry `flags=0x12 duration=0xffffffff` and `flags=0x10 duration=0`; our own
    /// guest's `traces/guest_boots/run_w210_8574466_ctl_probe.log:428` carries the first.
    /// `kperfBoostSet_IMPL` assigns that `NvU32 flags` into an `NvBool`, so the byte that
    /// reaches this module is the low one.
    #[test]
    fn the_two_measured_requests_are_acknowledged() {
        for (flags, duration) in [(0x12u8, 0xffff_ffffu32), (0x10, 0)] {
            let mut raw = vec![0u8; INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE];
            raw[FLAGS_OFF] = flags;
            raw[DURATION_OFF..DURATION_OFF + 4].copy_from_slice(&duration.to_le_bytes());
            let req = decode_perf_boost_set_2x(&raw).expect("a measured request");
            assert_eq!(req, PerfBoostRequest { flags, duration });
            assert_eq!(encode_perf_boost_set_2x(&req), raw);
        }
    }

    /// Every command the header names decodes; the fourth encoding of a two-bit field does not.
    #[test]
    fn the_undefined_command_encoding_is_refused() {
        for cmd in 0u8..=2 {
            let mut raw = vec![0u8; INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE];
            raw[FLAGS_OFF] = cmd;
            assert!(decode_perf_boost_set_2x(&raw).is_ok(), "cmd {cmd} is named");
        }
        let mut raw = vec![0u8; INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE];
        raw[FLAGS_OFF] = 0b11;
        assert_eq!(
            decode_perf_boost_set_2x(&raw),
            Err(PerfBoostError::UndefinedFlags { flags: 0b11 })
        );
    }

    /// A bit the header defines no name for is refused rather than masked away.
    #[test]
    fn an_unnamed_flag_bit_is_refused_not_dropped() {
        for bit in [2u8, 3, 7] {
            let mut raw = vec![0u8; INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE];
            raw[FLAGS_OFF] = 1 << bit;
            assert_eq!(
                decode_perf_boost_set_2x(&raw),
                Err(PerfBoostError::UndefinedFlags { flags: 1 << bit }),
                "bit {bit} names nothing in ctrl2080perf.h"
            );
        }
    }

    /// RM's own documented bound, both edges and the sentinel.
    #[test]
    fn the_duration_bound_is_rms_own() {
        for d in [0u32, 1, DURATION_MAX, DURATION_INFINITE] {
            let mut raw = vec![0u8; INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE];
            raw[DURATION_OFF..DURATION_OFF + 4].copy_from_slice(&d.to_le_bytes());
            assert!(decode_perf_boost_set_2x(&raw).is_ok(), "{d} is in range");
        }
        let mut raw = vec![0u8; INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE];
        raw[DURATION_OFF..DURATION_OFF + 4].copy_from_slice(&(DURATION_MAX + 1).to_le_bytes());
        assert_eq!(
            decode_perf_boost_set_2x(&raw),
            Err(PerfBoostError::DurationOutOfRange {
                duration: DURATION_MAX + 1
            })
        );
    }

    /// ★ The non-vacuity instrument for "re-encoded, not copied": pad bytes the guest set are
    /// NOT carried into the reply, which is what makes the reply a function of the decode.
    #[test]
    fn the_reply_is_the_decode_not_the_buffer() {
        let mut raw = vec![0u8; INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE];
        raw[FLAGS_OFF] = 0x12;
        raw[1] = 0xde;
        raw[2] = 0xad;
        raw[3] = 0xbe;
        raw[DURATION_OFF..DURATION_OFF + 4].copy_from_slice(&7u32.to_le_bytes());
        let req = decode_perf_boost_set_2x(&raw).expect("pad bytes are not read");
        let out = encode_perf_boost_set_2x(&req);
        assert_eq!(&out[1..4], &[0, 0, 0], "a copy would have carried 0xdeadbe");
        assert_eq!(out[FLAGS_OFF], 0x12);
        assert_eq!(&out[DURATION_OFF..], &7u32.to_le_bytes());
    }

    /// A length the header does not declare is refused.
    #[test]
    fn a_wrong_length_is_refused() {
        for len in [0usize, 4, 7, 9, 12] {
            assert_eq!(
                decode_perf_boost_set_2x(&vec![0u8; len]),
                Err(PerfBoostError::WrongSize {
                    got: len,
                    want: INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE,
                })
            );
        }
    }
}
