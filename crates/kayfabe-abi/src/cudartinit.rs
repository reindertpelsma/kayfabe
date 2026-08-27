//! ★★★★★ **The four controls the CUDA RUNTIME issues that the DRIVER API never does.**
//!
//! `[measured 2026-08-20, real GA106 (RTX 3060) on 580.159.04, bench 48097794]`
//!
//! # Why this module exists
//!
//! For six rungs the "LLM wall" was: `libcuda` answers every call correctly in the guest —
//! `cuInit`, `cuDeviceGetName`, `cuDevicePrimaryCtxRetain`, managed allocation, CPU touch,
//! `cuCtxSynchronize`, all `0` — while `libcudart` returns **3**
//! (`cudaErrorInitializationError`) from its very first call. Host `Xid` was `0` and every
//! `ioctl(2)` returned `0`, so `strace` could not see it: RM puts its verdict **inside the
//! parameter struct**.
//!
//! # How the set was found, and why it is exactly this set
//!
//! Both halves were traced on the SAME bare-metal GA106 through the `nvdiff` `LD_PRELOAD`
//! recorder, which captures the parameter buffer on **both sides** of every ioctl:
//!
//! - `cudaGetDeviceCount` alone → 105 ioctl records, 55 distinct RM controls.
//! - the driver-API probe the guest already passes → 397 records, 60 distinct controls.
//!
//! The runtime is not doing *more*; it is doing something *different*. Exactly **six**
//! controls appear in the runtime trace and in **no** driver-API record. All six answer
//! `NV_OK` on hardware with a **complete** body (`captured == paramsSize` for every one, so
//! none is in the `dlen = 0` / truncated-row hazard class), and none varied between its two
//! calls in the trace.
//!
//! # ★★★ The causality is MEASURED, not inferred
//!
//! An `LD_PRELOAD` interposer refused a chosen set **on the bare-metal host** in-band exactly
//! as this port refuses to the guest — `ioctl` returns `0`, `NVOS54_PARAMETERS.status` is set
//! to `NV_ERR_NOT_SUPPORTED` — and the host's own `cudaGetDeviceCount` was graded:
//!
//! | refused (alone) | host `cudaGetDeviceCount` |
//! |---|---|
//! | `0x20809009` | **3** |
//! | `0x20809001` | **3** |
//! | `0x20809064` | **3** |
//! | `0x20802209` | **3** |
//! | `0x2080a026` | 0 |
//! | `0x2080a084` | 0 |
//! | *(nothing — negative control, run FIRST and again LAST)* | 0 |
//!
//! ⇒ **Four controls, each one INDEPENDENTLY sufficient to reproduce the wall on real
//! hardware with no guest, no emulator and no QEMU in the picture.** The two innocent ones are
//! deliberately **not** served here: a set that is larger than the measurement is a claim
//! nobody made.
//!
//! ⚠ The negative control was run both first and last, because a sequential arm sweep
//! otherwise confounds the arm with session time (this tree has paid for that once already).
//!
//! # ⊘ A refutation of this port's own earlier reasoning, recorded so it is not repeated
//!
//! An earlier rung retired this hypothesis by arguing *"`nvproxy` answers unknown controls
//! with `NV_ERR_NOT_SUPPORTED` and CUDA works under gVisor, so refusing these is
//! survivable."* Checked **by value** against `gvisor/pkg/abi/nvgpu/*.go`, with a live
//! positive control to prove the grep was sighted: of these six, nvproxy's tables contain
//! **only** `0x20802209`. The other five are **absent from nvproxy entirely**, so gVisor
//! never demonstrated that refusing them is survivable, and the premise did not hold.
//!
//! # Why a measured table rather than a host forward
//!
//! Forwarding is the better long-term answer and is built (`VerbPlan::SubdeviceControl`), but
//! it is **gated off**: a synchronous host round-trip from `CommandPolicy::respond` trips the
//! R1 no-blocking-under-lock guard, and the off-BQL execution site does not exist yet.
//!
//! These four are also **not derivable from chip facts**. Three carry the GSS-legacy bit and
//! have no `#define` in any open header, SDK or NVOC table; they are opaque by construction.
//! So the honest options are "refuse" (measured fatal) or "answer what the hardware answers".
//!
//! ⚠ **This is a capture-derived table and therefore expires like one.** It is pinned to the
//! chip and driver in the header above. `nvkvm-pv` shipped a capture-derived table that
//! silently became wrong across a vendor bump and read as a vendor regression for weeks. The
//! rows carry their provenance so a future reader can re-measure rather than re-derive, and
//! `serve_len` refuses any request whose declared size is not the measured one.

/// `NV2080_CTRL_CMD_RC_GET_WATCHDOG_INFO` — the one id of the four that nvproxy also knows
/// (`gvisor/pkg/abi/nvgpu/ctrl.go:753`), and the only one of the four **without** the
/// GSS-legacy bit.
pub const RC_GET_WATCHDOG_INFO: u32 = 0x2080_2209;

/// GSS-legacy, unnamed in every open header. Hardware answers `{0, 0xd}`.
pub const CUDART_INIT_0X9009: u32 = 0x2080_9009;

/// GSS-legacy, unnamed in every open header. Hardware answers `{0x03fc_007f, 0}`.
pub const CUDART_INIT_0X9001: u32 = 0x2080_9001;

/// GSS-legacy, unnamed in every open header. 520 bytes; the leading ten `u32`s carry the
/// content and the remaining 120 words are zero **on hardware** — measured, not assumed.
pub const CUDART_INIT_0X9064: u32 = 0x2080_9064;

/// `0x2080a001` — ★★★★★ **THE FALLBACK, and the one control a host-side experiment could
/// NEVER have found.**
///
/// `[measured 2026-08-20, real GA106]` It appears **ZERO times** in an unmodified host trace,
/// because a healthy host never needs it. It is reached only *after* `0x2080a084` and
/// `0x2080a026` have failed — which is precisely the state our guest is in.
///
/// Proven by putting the host INTO that state: refusing `a084` + `a026` on bare metal under
/// the recorder makes the host take the identical branch, ask `0x2080a001` twice, get
/// `status = 0x0`, and return **`cudaGetDeviceCount = 0`**. Our guest asks the same control,
/// with a **byte-identical request**, and we answer `0x56`.
///
/// ⇒ `a084` and `a026` measured "innocent" on the host *because the host served the
/// fallback*. Refusing all three is what is fatal, and no refusal or substitution experiment
/// driven by the host trace could have covered a control the host trace does not contain.
pub const CUDART_INIT_0XA001: u32 = 0x2080_a001;

/// The measured reply words, in `u32` order. The body is these words little-endian, then
/// zero-padded to [`params_size`] — and the padding is itself measured, not a default.
const WORDS_2209: &[u32] = &[0x5];
const WORDS_9009: &[u32] = &[0x0, 0xd];
const WORDS_9001: &[u32] = &[0x03fc_007f, 0x0];
const WORDS_9064: &[u32] = &[0x0, 0x2, 0x1, 0x1, 0x1, 0x64, 0x4, 0x10, 0x1, 0x64];
/// ★ Stable across both host calls, and the guest's request is `{0,0,0,0}` exactly as the
/// host's is — so this row is not merely captured, it is captured against the same input.
const WORDS_A001: &[u32] = &[0x1, 0x5, 0x2, 0x11];

/// `(cmd, paramsSize, leading words)` — the whole served universe of this module.
///
/// ⊘ `0x2080a026` and `0x2080a084` are **deliberately absent**: both were measured
/// **innocent** (refusing either alone leaves the host at `0`), and this port does not answer
/// a control merely because it saw one.
pub const SERVED: &[(u32, usize, &[u32])] = &[
    (RC_GET_WATCHDOG_INFO, 4, WORDS_2209),
    (CUDART_INIT_0X9009, 8, WORDS_9009),
    (CUDART_INIT_0X9001, 8, WORDS_9001),
    (CUDART_INIT_0X9064, 520, WORDS_9064),
    (CUDART_INIT_0XA001, 16, WORDS_A001),
];

/// The measured `paramsSize` for a served id, or `None` for anything else.
///
/// ★ Callers must treat `None` as *unmeasured*, never as *empty*: an uncaptured reply body is
/// evidence of nothing, and decoding one to zeros is how this tree produced a NULL channel
/// table once already.
pub fn params_size(cmd: u32) -> Option<usize> {
    SERVED.iter().find(|(c, _, _)| *c == cmd).map(|(_, n, _)| *n)
}

/// Build the measured reply body for `cmd`.
///
/// `req_len` is the length the guest declared; it must equal the measured `paramsSize` or the
/// call is refused. A control whose declared size is not the one that was measured is a
/// **different** control as far as this table is concerned, and answering it from these bytes
/// would be an invention wearing a measurement's clothes.
pub fn answer_cudart_init(cmd: u32, req_len: usize) -> Result<Vec<u8>, CudartInitError> {
    let (_, size, words) = SERVED
        .iter()
        .find(|(c, _, _)| *c == cmd)
        .ok_or(CudartInitError::NotServed)?;
    if req_len != *size {
        return Err(CudartInitError::WrongSize {
            asked: req_len,
            measured: *size,
        });
    }
    let mut body = vec![0u8; *size];
    for (i, w) in words.iter().enumerate() {
        body[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    Ok(body)
}

/// Why a request was refused — named, because a refusal this port cannot name is one nobody
/// can debug from a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CudartInitError {
    /// The id is not in [`SERVED`] — including the two ids measured innocent.
    NotServed,
    /// The guest declared a size other than the measured one.
    WrongSize {
        /// The `paramsSize` the guest declared on this request.
        asked: usize,
        /// The `paramsSize` a real GA106 answered with, per [`SERVED`].
        measured: usize,
    },
}

/// ★★★★★ **SPLICED rows — `(cmd, paramsSize, &[(word, value)])`.**
///
/// A [`SERVED`] row replaces the whole body with a constant. That is right when the guest
/// sends all-zeros, and **wrong** the moment the request carries content: it would clobber
/// the guest's own words with our zeros — the `#203` defect pointing the other way.
///
/// `0x2080200b` `NV2080_CTRL_CMD_PERF_GET_LEVEL_INFO_V2` is the first row of that kind. Its
/// request carries `[0]=0x4 [3]=0x1 [9]=0x10 [194]=0x2`, and a real GA106 writes exactly
/// **nine** words and leaves the rest alone. So we do the same: keep the guest's buffer,
/// overwrite only what RM overwrites.
///
/// `[measured 2026-08-20, real GA106 580.159.04]` Captured from a host DRIVEN INTO the
/// guest's state (`0x2080a026` + `0x2080a084` refused), because a healthy host never asks
/// this control at all. Request byte-identical to the guest's; reply stable across both calls.
///
/// ⚠ The nine values decode as clock frequencies in kHz (≈465/930 MHz core, ≈7.5/7.3/9.0 GHz
/// memory). They are a property of the GA106 SKU, not of this machine — but they are still
/// **capture-derived and therefore expiring**, exactly like [`SERVED`].
pub const SPLICED: &[(u32, usize, &[(usize, u32)])] = &[(
    PERF_GET_LEVEL_INFO_V2,
    780,
    &[
        (1, 0x4),
        (4, 0x7_1868),
        (5, 0x7_1868),
        (6, 0x7_1868),
        (7, 0xe_30d0),
        (10, 0x72_74c8),
        (11, 0x72_74c8),
        (12, 0x6f_6788),
        (13, 0x89_58f0),
    ],
)];

/// `0x2080200b` — the SECOND member of the minimal fatal pair.
///
/// ★★★ `[measured 2026-08-20]` The full subset lattice over
/// `{0x2080a084, 0x2080a026, 0x2080200b}` on bare metal: **every singleton is innocent**, and
/// so are `{a084,a026}` and `{a084,200b}`. Only **`{a026, 200b}`** is fatal. ⇒ three
/// individually-harmless controls, one fatal PAIR — invisible to any one-at-a-time
/// experiment, and the lattice also proves serving EITHER member is sufficient.
pub const PERF_GET_LEVEL_INFO_V2: u32 = 0x2080_200b;

/// Apply a [`SPLICED`] row onto the guest's own request buffer, in place.
///
/// Returns `false` if the id has no spliced row or the buffer is not the measured length —
/// refusing rather than writing at a guessed offset.
pub fn splice_cudart_init(cmd: u32, buf: &mut [u8]) -> bool {
    let Some((_, size, words)) = SPLICED.iter().find(|(c, _, _)| *c == cmd) else {
        return false;
    };
    if buf.len() != *size {
        return false;
    }
    for (w, v) in words.iter() {
        buf[w * 4..w * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_served_row_is_exactly_what_the_hardware_answered() {
        // The bytes below are transcribed from the captured `ppost` of
        // `traces`-equivalent nvdiff records; keeping them as literals here makes the
        // transcription checkable rather than trusted.
        assert_eq!(answer_cudart_init(0x2080_2209, 4).unwrap(), vec![5, 0, 0, 0]);
        assert_eq!(
            answer_cudart_init(0x2080_9009, 8).unwrap(),
            vec![0, 0, 0, 0, 0x0d, 0, 0, 0]
        );
        assert_eq!(
            answer_cudart_init(0x2080_9001, 8).unwrap(),
            vec![0x7f, 0x00, 0xfc, 0x03, 0, 0, 0, 0]
        );
        let b = answer_cudart_init(0x2080_9064, 520).unwrap();
        assert_eq!(b.len(), 520);
        assert_eq!(&b[..40], &[
            0, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0,
            0x64, 0, 0, 0, 4, 0, 0, 0, 0x10, 0, 0, 0, 1, 0, 0, 0, 0x64, 0, 0, 0,
        ]);
        // ★ The tail is measured zero, not defaulted zero — assert it so a future edit that
        // changes the padding rule has to say so.
        assert!(b[40..].iter().all(|&x| x == 0), "measured tail is all zero");
    }

    #[test]
    fn the_fallback_row_is_what_the_host_answered_in_the_SAME_failing_state() {
        // ⊘ Not "what a healthy host answers" — a healthy host never asks this. The bytes
        // come from a host DRIVEN INTO the guest's state by refusing a084+a026.
        assert_eq!(
            answer_cudart_init(0x2080_a001, 16).unwrap(),
            vec![1, 0, 0, 0, 5, 0, 0, 0, 2, 0, 0, 0, 0x11, 0, 0, 0]
        );
    }

    #[test]
    fn a_spliced_row_preserves_every_word_rm_did_not_write() {
        // ★ The whole point: the guest's own content survives. A constant body would not.
        let mut buf = vec![0xABu8; 780];
        assert!(splice_cudart_init(PERF_GET_LEVEL_INFO_V2, &mut buf));
        let w = |i: usize| u32::from_le_bytes(buf[i * 4..i * 4 + 4].try_into().unwrap());
        assert_eq!(w(1), 0x4);
        assert_eq!(w(4), 0x7_1868);
        assert_eq!(w(13), 0x89_58f0);
        // untouched words still carry the caller's bytes, not zeros
        assert_eq!(w(0), 0xABAB_ABAB);
        assert_eq!(w(2), 0xABAB_ABAB);
        assert_eq!(w(194), 0xABAB_ABAB);
    }

    #[test]
    fn a_spliced_row_refuses_a_length_that_is_not_the_measured_one() {
        let mut short = vec![0u8; 776];
        assert!(!splice_cudart_init(PERF_GET_LEVEL_INFO_V2, &mut short));
        let mut wrong_id = vec![0u8; 780];
        assert!(!splice_cudart_init(0x2080_a026, &mut wrong_id));
    }

    #[test]
    fn the_two_innocent_ids_are_not_served() {
        // ⊘ Refusing either of these left the host at `cudaGetDeviceCount=0`, so they are
        // outside the measurement and must stay outside the table.
        assert_eq!(params_size(0x2080_a026), None);
        assert_eq!(params_size(0x2080_a084), None);
        assert_eq!(
            answer_cudart_init(0x2080_a026, 532),
            Err(CudartInitError::NotServed)
        );
    }

    #[test]
    fn a_size_that_is_not_the_measured_one_is_refused_rather_than_padded() {
        assert_eq!(
            answer_cudart_init(0x2080_9009, 12),
            Err(CudartInitError::WrongSize {
                asked: 12,
                measured: 8
            })
        );
        // ⚠ Including the shorter direction: truncating a measured body is the same defect
        // as zero-filling a short one, just pointing the other way.
        assert_eq!(
            answer_cudart_init(0x2080_9064, 4),
            Err(CudartInitError::WrongSize {
                asked: 4,
                measured: 520
            })
        );
    }

    #[test]
    fn three_of_the_four_carry_the_gss_legacy_bit_and_one_does_not() {
        // The watchdog id is a normal, nvproxy-known control; the other three are opaque.
        let m = crate::capability::RM_GSS_LEGACY_MASK;
        assert_eq!(RC_GET_WATCHDOG_INFO & m, 0);
        for c in [
            CUDART_INIT_0X9009,
            CUDART_INIT_0X9001,
            CUDART_INIT_0X9064,
            CUDART_INIT_0XA001,
        ] {
            assert_ne!(c & m, 0, "{c:#010x} should be GSS-legacy");
        }
        assert_eq!(SERVED.len(), 5);
    }

    #[test]
    fn each_rows_word_list_fits_inside_its_measured_size() {
        for (cmd, size, words) in SERVED {
            assert!(
                words.len() * 4 <= *size,
                "{cmd:#010x}: {} words do not fit in {size} bytes",
                words.len()
            );
        }
    }
}
