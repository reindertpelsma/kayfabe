//! The **GSS-legacy** control plane — commands with [`crate::capability::RM_GSS_LEGACY_MASK`]
//! set, which reach physical RM under their own id without ever entering resserv.
//!
//! ★★★ This module holds exactly **one** served id, and the fact that it is one rather than a
//! rule is the whole design. `kayfabe-rmrpc`'s default for a rule-permitted GSS-legacy control
//! is a **named refusal** and must stay one: the C research artifact's default was to echo the
//! request back under `NV_OK`, and for a control whose params are `[OUT]` that is a body of
//! zeros which the CUDA runtime read as real data and died on —
//! `cudaErrorInitializationError(3)` with **no errno and no log line**
//! (`C: src/qemu/nvkvm_gpu_emul.c:3335-3360`, pinned by
//! `kayfabe-rmrpc/tests/gss_legacy_answer.rs`). ⊘ Nothing here relaxes that. A rule permits a
//! command to be *named*; only a measurement permits it to be *answered*.
//!
//! # ★★★ Why `0x20808159` can be answered by identity, and why that is a MEASUREMENT
//!
//! `[measured 2026-08-09, real GA106 on 580.159.04]` a real part answers it `NV_OK` with the
//! 332-byte buffer **byte-unchanged** (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:80`:
//! `in` and `out` are equal for all 332 bytes; the request's head is `{2, 0x0c, 0, 0x25}` and
//! every data word is zero).
//!
//! ⊘⊘ **The obvious objection is that `in == out` proves nothing**, because it is also what a
//! caller sees when RM copies *nothing* back — the `RMAPI_PARAM_COPY_FLAGS_SKIP_COPYOUT` shape,
//! which makes the interposer re-read the caller's own buffer. That objection is correct for
//! `0x0000013a` and `0x20803002`, and it **cannot apply to this id**, for the same structural
//! reason that makes the command ours in the first place:
//!
//! `_nv04ControlWithSecInfo` tests `IsGssLegacyCall(cmd)` **before** resserv
//! (`ogkm-580: src/nvidia/interface/deprecated/rmapi_deprecated_control.c:97`), so this command
//! never reaches `rmapiParamsCopyOut` and **no `RMAPI_PARAM_COPY` flag is ever consulted**. Its
//! transfers are a bare, unconditional pair in `RmGssLegacyRpcCmd`
//! (`ogkm-580: rmapi_gss_legacy_control.c`):
//!
//! ```text
//! :72-75   portMemExCopyFromUser(pArgs->params, pKernelParams, pArgs->paramsSize)   — always
//! :145-151 if (status == NV_OK) portMemExCopyToUser(pKernelParams, pArgs->params, …) — always
//! ```
//!
//! ⇒ On the `NV_OK` that hardware returned, the copy-out **did** run. So `in == out` is a
//! positive measurement — physical RM was handed 332 bytes and gave 332 identical bytes back —
//! and reproducing it is an **identity on the guest's own buffer**, not a body this port
//! invented. ★ That is the precise difference from the C's echo: the C echoed where it had
//! measured *nothing*; this echoes where the echo *is* the measurement.
//!
//! ⚠ The honest residue, stated: this says the buffer is unchanged, **not** that the command
//! is a no-op inside GSP. Whatever it does, it does not do it through this buffer.
//!
//! # ⚠ The sticky question, which is the one that could make this permanent
//!
//! A GSS-legacy reply is the one shape whose caching *this port* controls. `rpcRmApiControl_GSP`
//! populates the guest's control cache from branch (b) (`ogkm-580: rpc.c:11098-11103`):
//!
//! ```c
//! else if (IsGssLegacyCall(cmd) && !FINN_SERIALIZED &&
//!          rmapiControlIsCacheable(rpc_params->rmctrlFlags, rpc_params->rmctrlAccessRight, NV_TRUE) &&
//!          !(rpc_params->rmctrlFlags & RMCTRL_FLAGS_CACHEABLE_BY_INPUT))
//!     rmapiControlCacheSetUnchecked(…);
//! ```
//!
//! — and every flag it reads is a word **the reply** carries, i.e. a word we write.
//! `kayfabe_device::sticky::StickyAnswerGuard` zeroes both unconditionally on every accepted
//! control reply, and `rmapi_control_is_cacheable(0, …)` is `false` on its first conjunct. ⇒ the
//! guest cannot keep this answer, so every ask reaches us and a later correction is possible.
//! ⊘ That is a property of the **chain**, not of this module, which is why it is checked by
//! `sticky_answer.rs`'s `Guarded` row for `InitTablePolicy` rather than asserted here.
//!
//! # ⊘ What this module does NOT claim
//!
//! Serving it removes a wall. It does not make `cuInit` correct, and it says nothing about the
//! eight calls after it — rows 81-88 of the reference trace have **never** been exercised
//! against this port, because every boot so far has been tearing down by then.

/// The one GSS-legacy control this port answers.
///
/// ⊘ Deliberately **not** named after a symbol: it appears in no NVOC export table, in no open
/// SDK header, and in neither `nvproxy` nor the C artifact's tables — searched at
/// `ogkm-580`/`ogkm-610` and `gvisor/pkg/abi/nvgpu` on 2026-08-09. Its body lives in GSP
/// firmware. A plausible invented name would be the worst thing this constant could carry.
pub const GSS_LEGACY_0X8159: u32 = 0x2080_8159;

/// Its `paramsSize`, as the guest declares it on the wire.
///
/// `[measured 2026-08-09, real GA106 on 580.159.04]`
/// `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:80` declares `size=332` and carries 332
/// bytes in each direction, and our own boot `gf1435` at `d24ad77` declares the same.
///
/// ⚠ There is **no second source** for this number and there cannot be one: the GSS-legacy path
/// bypasses resserv, so `resControlLookup`'s param-size check never runs and no table declares
/// it. The guest's declaration is the only statement of it in existence, which is why
/// [`answer_gss_legacy`] refuses a buffer that does not match rather than trusting either side
/// alone.
pub const GSS_LEGACY_0X8159_PARAMS_SIZE: usize = 332;

/// The second GSS-legacy control this port answers.
///
/// ⊘ Unnamed for the same reason as [`GSS_LEGACY_0X8159`]: absent from every NVOC table, SDK
/// header, `nvproxy` table and C artifact table searched on 2026-08-09.
pub const GSS_LEGACY_0X8162: u32 = 0x2080_8162;

/// Its `paramsSize` — a single byte.
///
/// `[measured 2026-08-09, real GA106 on 580.159.04]`
/// `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:85` declares `size=1`, and our own boot
/// `gf1436` at `ec434b8` declares the same.
pub const GSS_LEGACY_0X8162_PARAMS_SIZE: usize = 1;

/// The byte a real GA106 returns for [`GSS_LEGACY_0X8162`].
///
/// ★★★ **Double-sourced, and — unlike [`GSS_LEGACY_0X8159`] — NOT an identity.** The request
/// carries `00` and the reply carries `01`
/// (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:85`: `in=00 out=01`), and the C research
/// artifact independently records *"host returns bool=1"* for the same id.
///
/// ⚠⚠ **The consequence is that [`GSS_LEGACY_0X8159`]'s cache argument does NOT extend to this
/// one, and copying it across would be the mistake.** That id is safe under branch (b) because
/// its answer is the identity on the guest's own buffer, so a replayed cache entry is
/// indistinguishable from re-executing. This id **writes a byte the guest did not send**, so a
/// cached entry is a real answer persisting. Its safety rests entirely on
/// `kayfabe_device::sticky::StickyAnswerGuard` zeroing the reply's flag words — a property of
/// the chain, checked by `sticky_answer.rs`, and not a property of the value.
///
/// ⊘ What the byte MEANS is unmeasured. It is `1` on hardware and this port reproduces `1`; that
/// `out != in` proves RM wrote it is all this claims.
pub const GSS_LEGACY_0X8162_REPLY: u8 = 0x01;

/// Why a GSS-legacy identity answer could not be given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GssLegacyError {
    /// The command is not one this module answers.
    ///
    /// ⊘ Named rather than defaulted. This is the boundary that keeps the *rule* from becoming
    /// an *answer*: a GSS-legacy id this port has not measured gets a refusal, which is the C's
    /// lesson made mechanical.
    NotServed {
        /// The id asked for.
        cmd: u32,
    },
    /// The buffer is not the size this command's only measurement declares
    /// (`[measured 2026-08-09, real GA106 on 580.159.04]`, and see
    /// [`GSS_LEGACY_0X8159_PARAMS_SIZE`] for why there is no second source).
    WrongSize {
        /// The length `[measured 2026-08-09]` on a real GA106.
        want: usize,
        /// What arrived.
        got: usize,
    },
}

impl core::fmt::Display for GssLegacyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotServed { cmd } => write!(
                f,
                "{cmd:#010x} is GSS-legacy, which lets it be NAMED but not ANSWERED; this port \
                 answers only ids whose end-state it has measured"
            ),
            Self::WrongSize { want, got } => write!(
                f,
                "a GSS-legacy identity needs the measured {want}-byte buffer and {got} arrived"
            ),
        }
    }
}

impl core::error::Error for GssLegacyError {}

/// Answer a GSS-legacy control by returning the caller's own bytes.
///
/// ★ The reply is the request, which is what a real GA106 does — see the module docs for why
/// that is a measurement here and an invention everywhere else.
///
/// # Errors
///
/// [`GssLegacyError::NotServed`] for any id but [`GSS_LEGACY_0X8159`];
/// [`GssLegacyError::WrongSize`] for a buffer that is not the measured length.
pub fn answer_gss_legacy(cmd: u32, params: &[u8]) -> Result<Vec<u8>, GssLegacyError> {
    let want = params_size(cmd).ok_or(GssLegacyError::NotServed { cmd })?;
    if params.len() != want {
        return Err(GssLegacyError::WrongSize {
            want,
            got: params.len(),
        });
    }
    // ⊘ The two served ids are answered by DIFFERENT rules, and the match is written out
    // rather than generalised because the difference is the whole safety argument: `…8159`
    // is the identity on the guest's buffer, `…8162` writes a byte the guest did not send.
    match cmd {
        GSS_LEGACY_0X8159 => Ok(params.to_vec()),
        GSS_LEGACY_0X8162 => Ok(vec![GSS_LEGACY_0X8162_REPLY]),
        _ => Err(GssLegacyError::NotServed { cmd }),
    }
}

/// Every GSS-legacy id this port answers, paired with the `paramsSize`
/// `[measured 2026-08-09, real GA106 on 580.159.04]` for it.
///
/// ★ Stated as data so a gate can quantify over it instead of over a sample — the
/// `gates_quantified_over_a_list` rule applied to the one list whose growth is dangerous.
pub const SERVED: [(u32, usize); 2] = [
    (GSS_LEGACY_0X8159, GSS_LEGACY_0X8159_PARAMS_SIZE),
    (GSS_LEGACY_0X8162, GSS_LEGACY_0X8162_PARAMS_SIZE),
];

/// The `paramsSize` `[measured 2026-08-09]` for a served GSS-legacy id, or `None` if it is
/// not served.
#[must_use]
pub fn params_size(cmd: u32) -> Option<usize> {
    SERVED.iter().find(|(c, _)| *c == cmd).map(|(_, n)| *n)
}

/// Does this GSS-legacy control id carry a WRITTEN argument for why the guest may cache our
/// answer forever?
///
/// ★★★★★ **This exists because the same question was asked in two places and only one of
/// them was updated — w349b, measured on hardware.** The serve-site tripwire in
/// `kayfabe_device::inittables` carried a hardcoded pair of ids, while
/// `init_tables.rs`'s sticky-answer gate quantified over a DERIVED set. Adding three served
/// ids satisfied the derived gate, left the hardcoded tripwire untouched, and the boot
/// refused all three anyway (`0x20809009 served=0 refused=1`) with the table rows present
/// and correct. **A list that answers a question in one place and a predicate that answers
/// it in another will drift, and the drift is invisible to the test that only knows one.**
///
/// ⇒ Both sides now call this. `gates_quantified_over_a_list` in the other direction: the
/// remedy is not a longer list, it is ONE list.
///
/// The argument itself differs per id and lives with the id:
/// - [`GSS_LEGACY_0X8159`] — the reply is the IDENTITY on the guest's own buffer.
/// - [`GSS_LEGACY_0X8162`] — no such property; safe only via `StickyAnswerGuard`.
/// - [`crate::cudartinit`]'s three — the answers are CONSTANTS, so a cached answer and a
///   fresh one are the same bytes by construction.
pub fn carries_cache_argument(cmd: u32) -> bool {
    SERVED.iter().any(|(c, _)| *c == cmd)
        || crate::cudartinit::SERVED.iter().any(|(c, _, _)| *c == cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real GA106's request head, transcribed from the raw `in=` hex of
    /// `cuinit_ioctl_trace_real_ga106.txt:80` — `{2, 0x0c, 0, 0x25}` as four LE words.
    fn real_ga106_request() -> Vec<u8> {
        let mut b = vec![0u8; GSS_LEGACY_0X8159_PARAMS_SIZE];
        b[0] = 0x02;
        b[4] = 0x0c;
        b[12] = 0x25;
        b
    }

    #[test]
    fn the_answer_is_the_request_which_is_what_hardware_returned() {
        let req = real_ga106_request();
        let out = answer_gss_legacy(GSS_LEGACY_0X8159, &req).expect("served");
        assert_eq!(out, req, "hardware returned the buffer byte-unchanged");
        assert_eq!(out.len(), GSS_LEGACY_0X8159_PARAMS_SIZE);
    }

    #[test]
    fn the_identity_is_not_the_same_as_returning_zeros() {
        // ⊘⊘ The falsifier that separates this from the C's defect. The C answered an
        // unmodelled control with a ZERO body; an identity answer preserves whatever the
        // caller sent. For the real request those differ in three bytes, so a regression to
        // "reply with zeros" cannot pass this test.
        let req = real_ga106_request();
        let out = answer_gss_legacy(GSS_LEGACY_0X8159, &req).expect("served");
        assert_ne!(
            out,
            vec![0u8; GSS_LEGACY_0X8159_PARAMS_SIZE],
            "an all-zero reply is the C's cudart-killing shape, not this one"
        );
        assert_eq!(&out[..16], &req[..16]);
    }

    #[test]
    fn the_rule_does_not_become_an_answer() {
        // ★★★ The property the whole module exists for: bit 15 permits a command to be NAMED,
        // never to be ANSWERED. The three ids the C observed GSS-legacy traffic on are the
        // concrete case — the cudart init gate at `C: src/qemu/nvkvm_gpu_emul.c:3328-3395`,
        // pinned in `kayfabe-rmrpc/tests/gss_legacy_answer.rs` — and this port does NOT
        // serve them.
        for cmd in [0x2080_9009u32, 0x2080_9001, 0x2080_9064, 0x2080_8513] {
            assert_eq!(
                answer_gss_legacy(cmd, &real_ga106_request()),
                Err(GssLegacyError::NotServed { cmd }),
                "{cmd:#010x} is GSS-legacy and unmeasured — it must refuse"
            );
        }
    }

    #[test]
    fn a_buffer_that_is_not_the_measured_length_is_refused() {
        // ⚠ Load-bearing because the guest's declaration is the ONLY statement of this size
        // in existence — resserv's param-size check never runs on this path.
        for n in [0usize, 331, 333, 4096] {
            assert_eq!(
                answer_gss_legacy(GSS_LEGACY_0X8159, &vec![0u8; n]),
                Err(GssLegacyError::WrongSize {
                    want: GSS_LEGACY_0X8159_PARAMS_SIZE,
                    got: n
                })
            );
        }
    }

    #[test]
    fn the_id_really_is_gss_legacy_and_not_privileged() {
        // The two facts that put it on this path at all: bit 15 set (so `IsGssLegacyCall` is
        // true and resserv is bypassed), and NOT the 0xC000 privileged pattern, which would
        // demand root (`ogkm-580: rmapi_gss_legacy_control.c:56-60`).
        assert_ne!(GSS_LEGACY_0X8159 & crate::capability::RM_GSS_LEGACY_MASK, 0);
        assert_ne!(
            GSS_LEGACY_0X8159 & 0x0000_C000,
            0x0000_C000,
            "a privileged legacy command would need root and libcuda is unprivileged"
        );
    }
}

#[cfg(test)]
mod tests_8162 {
    use super::*;

    #[test]
    fn the_second_id_returns_the_byte_hardware_wrote_and_it_is_not_an_echo() {
        // ★★★ `in=00 out=01` — so unlike `…8159` this one is NOT the identity, and the test
        // says so in the strongest available form: feeding it the guest's own `00` must NOT
        // give `00` back. An echo regression is therefore red here and green there, which is
        // the whole point of writing the two rules out separately.
        let out = answer_gss_legacy(GSS_LEGACY_0X8162, &[0x00]).expect("served");
        assert_eq!(out, vec![0x01]);
        assert_ne!(out, vec![0x00], "an echo would return the guest's own zero");
        assert_eq!(out.len(), GSS_LEGACY_0X8162_PARAMS_SIZE);
    }

    #[test]
    fn the_second_id_ignores_what_the_guest_sent() {
        // ⊘ The reply is a stated value, not a function of the request — so a guest cannot
        // steer it. `[measured 2026-08-09, real GA106 on 580.159.04]` the reply is `01` for
        // the `00` libcuda sends (`cuinit_ioctl_trace_real_ga106.txt:85`); what a non-zero
        // request would do has never been observed, and answering `01` regardless is this
        // port declining to invent a second behaviour.
        for probe in [0x00u8, 0x01, 0xCD, 0xFF] {
            assert_eq!(
                answer_gss_legacy(GSS_LEGACY_0X8162, &[probe]),
                Ok(vec![0x01])
            );
        }
    }

    #[test]
    fn the_served_set_is_exactly_two_and_each_has_its_own_rule() {
        assert_eq!(SERVED.len(), 2);
        assert_eq!(params_size(GSS_LEGACY_0X8159), Some(332));
        assert_eq!(params_size(GSS_LEGACY_0X8162), Some(1));
        assert_eq!(params_size(0x2080_8513), None, "unmeasured stays unserved");
        // ⚠ Every served id must really be GSS-legacy, or it is in the wrong module.
        for (cmd, _) in SERVED {
            assert_ne!(cmd & crate::capability::RM_GSS_LEGACY_MASK, 0);
        }
        // ★★ And the two rules genuinely differ, which is the fact that stops `…8159`'s
        // cache argument from being copied onto `…8162`.
        let ident = answer_gss_legacy(GSS_LEGACY_0X8159, &vec![0xABu8; 332]).expect("served");
        assert_eq!(ident, vec![0xABu8; 332], "…8159 is the identity");
        assert_eq!(
            answer_gss_legacy(GSS_LEGACY_0X8162, &[0xAB]),
            Ok(vec![0x01]),
            "…8162 is not"
        );
    }
}
