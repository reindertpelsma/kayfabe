//! `NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK` (`0x20802a02`) — which **physical** copy engines
//! back each **logical** one, and the first control this port serves whose value was
//! measured by asking the real part *this control*, at *this boundary*, rather than being
//! derived or read off a neighbour.
//!
//! ## Why it exists: §14.42's wall is TWO controls, six lines apart
//!
//! `queryCopyEngines` (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:8449-8541`) is
//! one function and one loop. Per copy engine it issues, in order:
//!
//! ```text
//! Control(… NV2080_CTRL_CMD_CE_GET_CAPS …)          // 0x20802a01, :8503
//!     if (status != NV_OK) goto done;
//! setCeCaps(rmCeCaps, ceCaps);
//! Control(… NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK …)   // 0x20802a02, :8521
//!     if (status != NV_OK) goto done;
//! ceCaps->cePceMask = pceMaskParams.pceMask;
//! ceCaps->supported = NV_TRUE;
//! ```
//!
//! `0x20802a01` is the guest kernel's own and reaches this port as its forward of
//! `0x20802a07` ([`crate::cecaps::answer_ce_get_physical_caps`]). `0x20802a02` carries
//! `ROUTE_TO_PHYSICAL` and arrives **unmodified**. Both are checked with a hard `goto done`.
//! ⇒ ★ **Serving only the caps control moves the wall by six lines**, which is why this
//! module landed in the same rung rather than the next one.
//!
//! ## ★★★ The reason this one is MEASURED and its neighbour is DERIVED
//!
//! The two ids sit in opposite epistemic positions, and the difference is one flag word:
//!
//! | id | flags | `NON_PRIVILEGED(0x8)`? | reachable from usermode? |
//! |---|---|---|---|
//! | `0x20802a07` | `0x301d0` (`g_subdevice_nvoc.c:7645-7658`) | **no** | ⊘ no — `KERNEL_PRIVILEGED` |
//! | `0x20802a02` | `0x30349` (`g_subdevice_nvoc.c:7585-7598`) | **yes** | ★ **yes** |
//!
//! `RMCTRL_FLAGS_KERNEL_PRIVILEGED` is the *default* — a row carrying neither `PRIVILEGED`
//! nor `NON_PRIVILEGED` refuses every usermode client including root
//! (`ogkm-580: control.h:170-247`). That is why `0x20802a0b` could not be probed
//! (`crate::cecaps` §1) and why `0x20802a07` cannot be either.
//!
//! `0x20802a02` can. And its body is **nowhere in the vendored tree** — only the export row
//! references `subdeviceCtrlCmdCeGetCePceMask_IMPL`; `ROUTE_TO_PHYSICAL` puts the
//! implementation inside GSP-RM firmware. ⇒ **Reachable and unreadable**: a real part is the
//! only oracle, and it is one we can actually ask. `derive_what_you_cannot_query_then_oracle_it`
//! says the measurable one gets measured, so it was.
//!
//! ## The measurement
//!
//! `[measured 2026-08-09, real GA106 `GPU-d0913685` (RTX 3060), host driver 580.159.04,
//! `traces/real_ga106/rmladder_r24_pcemask_real_ga106.txt`]`:
//!
//! ```text
//! ★ R24 LCE0 (type 0x09) = NV_OK  pceMask=0x00000020 popcount=1 [WRITTEN]
//! ★ R24 LCE1 (type 0x0a) = NV_OK  pceMask=0x00000010 popcount=1 [WRITTEN]
//! ★ R24 LCE2 (type 0x0b) = NV_OK  pceMask=0x00000010 popcount=1 [WRITTEN]
//! ★ R24 LCE3 (type 0x0c) = NV_OK  pceMask=0x00000020 popcount=1 [WRITTEN]
//! info R24 LCE4 (type 0x0d) = refused Other(86) (no value measured)
//! ```
//!
//! ⊘ `[WRITTEN]` is a distinct fact from `NV_OK`, and only the seed can separate them: the
//! rung seeds `pceMask` with `0xCDCDCDCD` and reports whether RM touched it, exactly as R18
//! does and for R18's reason. ⚠ It seeds **only** that word — `ceEngineType` is `[IN]`
//! (`ogkm-580: ctrl2080ce.h:167-170`), so R18's whole-buffer blanket would have asked for
//! engine type `0xCDCDCDCD`, drawn a refusal, and measured nothing.
//! [`seed_only_the_OUT_region`], third sighting.
//!
//! ## ★★ Two things the measurement corroborates that it was not aimed at
//!
//! 1. **The engine count, from an independent control.** LCE4 refuses `0x56`, against a
//!    `NV_CE_MAX_LCE_MASK = 0x1f` that permits five LCEs
//!    (`ogkm-580: kernel_ce_ga102.c:33-38`). [`crate::cecaps`] already measured
//!    `present = 0x0f` from two callers of a *different* control; this is a third sighting
//!    through a third door. See [`crate::cecaps::GA10X_EXPOSED_LCE_MASK_IS_NOT_A_SOURCE`].
//! 2. **The `SHARED` caps bit.** Four LCEs map onto **two** distinct PCEs — `{PCE5: LCE0,
//!    LCE3}`, `{PCE4: LCE1, LCE2}` — so every one of them shares physical hardware with
//!    another. [`crate::cecaps::GA10X_LCE_BASE_CAPS`] sets
//!    [`crate::cecaps::cap::SHARED`] on every present LCE, measured independently. ★ Two
//!    unrelated measurements agreeing on one structural claim is worth more than either,
//!    and neither was chosen to make the other come out.
//!
//! ⊘ **What it does NOT corroborate:** `GRCE`. [`crate::cecaps::GA10X_GRCE_LCE_MASK`] is
//! `{LCE0, LCE1}`, which is *not* one of the two PCE groupings — LCE0 pairs with LCE3 and
//! LCE1 with LCE2. The two facts are orthogonal and the near-miss is recorded so nobody
//! later "notices the pattern" and derives one from the other.
//!
//! ## ⚠ Why the table is named per-PART, not per-ARCH
//!
//! [`crate::cecaps`] can name its caps constants `GA10X_` because a caps bit is an
//! architecture's. A PCE→LCE map is **not**: it is a function of how many PCEs a die has and
//! of the floorsweeping applied to the individual part, and `kceGetMappings_HAL` recomputes
//! it per chip. A GA102 with more PCEs answers different words to this same control. So the
//! table is [`GA106_LCE_PCE_MASKS`], the row lives on the chip profile, and a part with no
//! row gets a **named refusal** rather than GA106's answer under another die's name.
//!
//! ## What the guest does with it
//!
//! `queryCopyEngines` stores it as `ceCaps->cePceMask` and sets `ceCaps->supported = NV_TRUE`
//! (`nv_gpu_ops.c:8534-8537`). ⚠ ★ **The value is consumed, not merely carried** — and the
//! honest statement of what serving it does not buy is at the serve site, not here.

extern crate alloc;
use alloc::vec::Vec;

/// `NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK` — `ogkm-580: ctrl2080ce.h:163`.
pub const NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK: u32 = 0x2080_2a02;

/// `sizeof(NV2080_CTRL_CE_GET_CE_PCE_MASK_PARAMS)` — `NvU32 ceEngineType` then
/// `NvU32 pceMask` (`ogkm-580: ctrl2080ce.h:167-170`), no padding, = 8. The `paramSize` the
/// export row advertises (`ogkm-580: g_subdevice_nvoc.c:7594`).
pub const CE_GET_CE_PCE_MASK_PARAMS_SIZE: usize = 8;

/// Byte offset of the `[OUT]` `pceMask` word.
pub const PCE_MASK_OFF: usize = 4;

/// `NV2080_CTRL_MAX_PCES` — `ogkm-580: ctrl2080ce.h`. ⚠ Not `NV2080_CTRL_MAX_CES` (64); the
/// two are different axes and `subdeviceCtrlCmdCeGetAllCaps_VF` is upstream's own example of
/// confusing them (see [`crate::cecaps::MAX_CES`]).
pub const MAX_PCES: usize = 32;

/// ★★★ The measured PCE mask of each logical copy engine on a **GA106**, indexed by LCE
/// instance.
///
/// `[measured 2026-08-09, real GA106 `GPU-d0913685`, driver 580.159.04,
/// `traces/real_ga106/rmladder_r24_pcemask_real_ga106.txt`]` — see the module header for the
/// verbatim rung output and for why LCE4 is absent rather than zero.
///
/// ⊘ **Four entries, not five.** The fifth LCE the arch mask permits is refused by the real
/// part at this very control, so there is no fifth word to state. An array padded to `0`
/// would be a positive claim that LCE4 exists and is backed by no physical engine —
/// `the_C_oracle's_EMPTY_rows_are_WRONG`, whose lesson is that an unmeasured slot must be
/// *absent*, never decoded to zeros.
///
/// ⚠ Named `GA106_` and not `GA10X_`: a PCE→LCE map is a per-part fact, unlike the caps bits
/// next door. See the module header.
pub const GA106_LCE_PCE_MASKS: &[u32] = &[
    // LCE0 → PCE5. Shares its physical engine with LCE3.
    0x0000_0020,
    // LCE1 → PCE4. Shares its physical engine with LCE2.
    0x0000_0010,
    // LCE2 → PCE4.
    0x0000_0010,
    // LCE3 → PCE5.
    0x0000_0020,
];

/// Answer `NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK` (`0x20802a02`) for one engine.
///
/// `masks` is the chip row's own table — [`GA106_LCE_PCE_MASKS`] for a GA106 — indexed by
/// LCE instance. `present` is [`crate::cecaps::CeGeometry::present`], passed so the two
/// descriptions of *how many copy engines this device has* are checked against each other
/// here rather than allowed to drift: a chip whose table and whose engine list disagree is
/// refused by name instead of answering out of the longer one.
///
/// ⊘ The request is **edited, not replaced**: `ceEngineType` is `[IN]` and the caller's own
/// value is the only right thing in that word.
///
/// # Errors
///
/// [`CePceMaskError::ShortParams`]; [`CePceMaskError::NotACopyEngine`] and
/// [`CePceMaskError::EngineNotPresent`], both mirroring RM's own `NV_ERR_NOT_SUPPORTED` arms
/// and the second one measured directly at this boundary (LCE4 → `0x56`); and
/// [`CePceMaskError::NoMaskForEngine`] for a part whose chip row does not state a mask for an
/// engine it advertises. ⊘ Every variant refuses the whole control — `queryCopyEngines`
/// `goto done`s on any status but `NV_OK`, and there is no partial answer to a single word.
pub fn answer_ce_get_ce_pce_mask(
    request: &[u8],
    present: u64,
    masks: &[u32],
) -> Result<Vec<u8>, CePceMaskError> {
    let Some(body) = request.get(..CE_GET_CE_PCE_MASK_PARAMS_SIZE) else {
        return Err(CePceMaskError::ShortParams {
            len: request.len(),
            need: CE_GET_CE_PCE_MASK_PARAMS_SIZE,
        });
    };
    let engine_type = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    // ⊘ The two-branch inverse, never `engine_type - 0x09`. See `cecaps`.
    let Some(index) = crate::submit::copy_index_of_engine_type(engine_type) else {
        return Err(CePceMaskError::NotACopyEngine { engine_type });
    };
    let index = index as usize;
    if index >= crate::cecaps::MAX_CES || present & (1u64 << index) == 0 {
        return Err(CePceMaskError::EngineNotPresent {
            engine_type,
            index,
            present,
        });
    }
    // ★ The cross-check that makes drift a refusal instead of a wrong number: the engine is
    // advertised, so the chip row owes a mask for it. A missing one is the chip row's fault
    // and is named as such — ⊘ never backfilled with zero, which on this control would claim
    // a copy engine backed by no physical engine at all.
    let Some(&mask) = masks.get(index) else {
        return Err(CePceMaskError::NoMaskForEngine {
            engine_type,
            index,
            stated: masks.len(),
        });
    };
    let mut out = body.to_vec();
    out[PCE_MASK_OFF..PCE_MASK_OFF + 4].copy_from_slice(&mask.to_le_bytes());
    Ok(out)
}

/// Read the `[OUT]` word back out — for tests and the trace differential, so a comparison is
/// done on a decoded value rather than on a hex string regrouped by hand.
///
/// # Errors
///
/// [`CePceMaskError::ShortParams`].
pub fn decode_ce_pce_mask(params: &[u8]) -> Result<u32, CePceMaskError> {
    let Some(body) = params.get(..CE_GET_CE_PCE_MASK_PARAMS_SIZE) else {
        return Err(CePceMaskError::ShortParams {
            len: params.len(),
            need: CE_GET_CE_PCE_MASK_PARAMS_SIZE,
        });
    };
    Ok(u32::from_le_bytes([body[4], body[5], body[6], body[7]]))
}

/// Why a PCE-mask reply could not be built — the four arms
/// [`answer_ce_get_ce_pce_mask`] refuses by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CePceMaskError {
    /// The request is shorter than the struct.
    ShortParams {
        /// What arrived.
        len: usize,
        /// [`CE_GET_CE_PCE_MASK_PARAMS_SIZE`].
        need: usize,
    },
    /// `ceEngineType` is not any `NV2080_ENGINE_TYPE_COPY(i)`.
    NotACopyEngine {
        /// The `[IN]` value that arrived.
        engine_type: u32,
    },
    /// A copy engine this part does not advertise. ★ Measured directly at this boundary: a
    /// real GA106 refuses LCE4 with `0x56`.
    EngineNotPresent {
        /// The `[IN]` value that arrived.
        engine_type: u32,
        /// Its decoded LCE index.
        index: usize,
        /// What this device does advertise.
        present: u64,
    },
    /// The device advertises this engine and the chip row states no mask for it. ⊘ A chip
    /// row defect, refused rather than papered over with a zero.
    NoMaskForEngine {
        /// The `[IN]` value that arrived.
        engine_type: u32,
        /// Its decoded LCE index.
        index: usize,
        /// How many masks the chip row does state.
        stated: usize,
    },
}

impl core::fmt::Display for CePceMaskError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortParams { len, need } => {
                write!(f, "{len}-byte params, {need} needed for the struct")
            }
            Self::NotACopyEngine { engine_type } => write!(
                f,
                "ceEngineType {engine_type:#x} is not an NV2080_ENGINE_TYPE_COPY(i): not a \
                 copy engine"
            ),
            Self::EngineNotPresent {
                engine_type,
                index,
                present,
            } => write!(
                f,
                "ceEngineType {engine_type:#x} decodes to LCE{index}, which this device does \
                 not advertise (present={present:#x}): NV_ERR_NOT_SUPPORTED, which is what a \
                 real GA106 answers for LCE4"
            ),
            Self::NoMaskForEngine {
                engine_type,
                index,
                stated,
            } => write!(
                f,
                "ceEngineType {engine_type:#x} decodes to LCE{index}, which this device \
                 ADVERTISES, but the chip row states only {stated} PCE mask(s): the engine \
                 list and the PCE table disagree about how many copy engines exist"
            ),
        }
    }
}

impl core::error::Error for CePceMaskError {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// The GA106 `present` the engine list projects — `crate::cecaps` measured it twice.
    const GA106_PRESENT: u64 = 0x0f;

    fn request(engine_type: u32) -> Vec<u8> {
        // ⚠ Seeded `0xCD` in the `[OUT]` half ONLY, so a test can tell "we wrote the mask"
        // from "the mask happened to be zero" — R18's discipline, applied to our own encoder.
        let mut p = vec![0xCDu8; CE_GET_CE_PCE_MASK_PARAMS_SIZE];
        p[0..4].copy_from_slice(&engine_type.to_le_bytes());
        p
    }

    /// ★★★ The reply reproduces the real part's four words, byte for byte.
    ///
    /// This is the whole point of the module: the trace is the oracle, and the encoder is
    /// checked against the trace's numbers rather than against a paraphrase of them.
    #[test]
    fn the_four_measured_masks_come_back() {
        // `traces/real_ga106/rmladder_r24_pcemask_real_ga106.txt`, verbatim.
        for (index, engine_type, want) in
            [(0usize, 0x09u32, 0x20u32), (1, 0x0a, 0x10), (2, 0x0b, 0x10), (3, 0x0c, 0x20)]
        {
            let out =
                answer_ce_get_ce_pce_mask(&request(engine_type), GA106_PRESENT, GA106_LCE_PCE_MASKS)
                    .expect("an advertised engine is answered");
            assert_eq!(out.len(), CE_GET_CE_PCE_MASK_PARAMS_SIZE);
            // `ceEngineType` is `[IN]`: it must come back exactly as sent.
            assert_eq!(
                u32::from_le_bytes([out[0], out[1], out[2], out[3]]),
                engine_type,
                "LCE{index}: the [IN] word must be echoed unedited"
            );
            assert_eq!(
                decode_ce_pce_mask(&out).expect("full-length reply"),
                want,
                "LCE{index}: pceMask must be the measured word"
            );
        }
    }

    /// ★★ The edge the real part draws: LCE4 is refused, not answered zero.
    ///
    /// `0x0d` is `NV2080_ENGINE_TYPE_COPY(4)` and a real GA106 answers `0x56` to it. A zero
    /// here would be the C oracle's empty-row mistake in a new place.
    #[test]
    fn lce4_is_refused_the_way_hardware_refuses_it() {
        let err = answer_ce_get_ce_pce_mask(&request(0x0d), GA106_PRESENT, GA106_LCE_PCE_MASKS)
            .expect_err("LCE4 is not advertised by this part");
        assert!(matches!(err, CePceMaskError::EngineNotPresent { index: 4, .. }), "{err}");
    }

    /// A non-copy engine type is refused by name — RM's own `!RM_ENGINE_TYPE_IS_COPY` arm.
    /// `0x01` is `NV2080_ENGINE_TYPE_GRAPHICS`, comfortably below `COPY0 = 0x09`.
    #[test]
    fn a_non_copy_engine_is_refused() {
        let err = answer_ce_get_ce_pce_mask(&request(0x01), GA106_PRESENT, GA106_LCE_PCE_MASKS)
            .expect_err("graphics is not a copy engine");
        assert!(matches!(err, CePceMaskError::NotACopyEngine { .. }), "{err}");
    }

    /// ★★★ The two-branch encoding, at the discontinuity that a `- 0x09` shortcut gets wrong.
    ///
    /// `0x13` is one past `COPY9`. The shortcut would read it as copy engine 10 and — on a
    /// device advertising 11 engines — answer a real mask for an engine the caller never
    /// named. Here it must be refused as not-a-copy-engine at all.
    #[test]
    fn the_gap_between_copy9_and_copy10_is_not_a_copy_engine() {
        let err = answer_ce_get_ce_pce_mask(&request(0x13), u64::MAX, GA106_LCE_PCE_MASKS)
            .expect_err("0x13 is in the gap between COPY9 and COPY10");
        assert!(matches!(err, CePceMaskError::NotACopyEngine { .. }), "{err}");
    }

    /// ⊘ The drift arm: an engine the device advertises with no mask on the chip row is a
    /// **refusal**, never a zero. Advertise five, state four.
    #[test]
    fn an_advertised_engine_with_no_stated_mask_is_refused() {
        let err = answer_ce_get_ce_pce_mask(&request(0x0d), 0x1f, GA106_LCE_PCE_MASKS)
            .expect_err("LCE4 advertised but not stated");
        assert!(
            matches!(err, CePceMaskError::NoMaskForEngine { index: 4, stated: 4, .. }),
            "{err}"
        );
    }

    /// A short request is refused rather than read past its end.
    #[test]
    fn a_short_request_is_refused() {
        let err = answer_ce_get_ce_pce_mask(&[0u8; 7], GA106_PRESENT, GA106_LCE_PCE_MASKS)
            .expect_err("7 bytes is short of the struct");
        assert!(matches!(err, CePceMaskError::ShortParams { len: 7, need: 8 }), "{err}");
    }

    /// ★★ The structural cross-check the module header claims: four LCEs, **two** distinct
    /// PCEs, so every advertised engine shares its physical engine with another — which is
    /// what `cecaps` sets `SHARED` on every present LCE for, measured independently.
    ///
    /// ⊘ Guards the claim, not the numbers: if a future part's table makes every LCE
    /// exclusive, `SHARED` must stop being unconditional in `cecaps`, and this goes red.
    #[test]
    fn every_advertised_lce_shares_its_pce() {
        let mut seen = alloc::collections::BTreeMap::new();
        for (i, &m) in GA106_LCE_PCE_MASKS.iter().enumerate() {
            assert_eq!(m.count_ones(), 1, "LCE{i} is backed by exactly one PCE");
            *seen.entry(m).or_insert(0usize) += 1;
        }
        assert_eq!(seen.len(), 2, "four LCEs onto two PCEs");
        for (mask, count) in seen {
            assert!(
                count > 1,
                "PCE mask {mask:#x} backs {count} LCE — cecaps sets SHARED unconditionally, \
                 so an exclusive PCE would make that bit a lie"
            );
        }
    }

    /// ⊘ The near-miss recorded in the header, pinned so nobody derives one fact from the
    /// other: the GRCE pair `{LCE0, LCE1}` is **not** either PCE grouping.
    #[test]
    fn the_grce_pair_is_not_a_pce_grouping() {
        assert_ne!(
            GA106_LCE_PCE_MASKS[0], GA106_LCE_PCE_MASKS[1],
            "LCE0 and LCE1 are the GRCE pair and do NOT share a PCE — the two facts are \
             orthogonal and neither may be derived from the other"
        );
    }
}
