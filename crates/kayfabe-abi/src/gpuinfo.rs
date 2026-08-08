//! `NV2080_CTRL_CMD_GPU_GET_INFO_V2` (`0x20800102`) — the first control this port serves
//! whose reply is a **function of the request**, and the first one where the guest kernel
//! answers most of the question before we ever see it.
//!
//! ## ⊘⊘ The refutation this module exists to record
//!
//! `execution_plane_increments.md` §14.27 measured, on a real GA106 driving a real libcuda,
//! that refusing this control makes `cuInit` return `100`, and it published the eleven
//! `(index, value)` rows libcuda asks for. That table is **correct and at the wrong
//! boundary by one layer**, and the error is not conservative:
//!
//! ★★★ **Ten of those eleven indices never reach a GSP.** `getGpuInfos`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c:88-580`) is a
//! thirty-two-arm `switch` that answers `GEMINI_BOARD`, `GPU_SMC_MODE`,
//! `GPU_DEBUGGING_CAPABILITY`, `DMABUF_CAPABILITY` and twenty-eight others **from kernel
//! state**, and forwards only the `default:` arm — marking each forwarded entry by OR-ing
//! `INDEX_FORWARD_TO_PHYSICAL` (`0x8000_0000`, `:83`, `ct_assert`ed equal to
//! `NV2080_CTRL_GPU_INFO_INDEX_RESERVED` = bit 31) into the index word before one
//! `NV_RM_RPC_CONTROL` carries the **whole** params struct across (`:570-577`).
//!
//! ⇒ A port that answered all eleven from the ioctl table would be **overwriting ten values
//! the guest's own kernel had just computed**, with numbers that happen to agree today
//! because they were read off the same machine. `a_table_does_not_decide_behaviour`, in a
//! new place: the table was right, and the dispatch above it is what decides.
//!
//! Of the eleven, exactly **one** — index `0x11`, which the open header does not name — hits
//! `default:` and is forwarded. That is the row this port serves.
//!
//! ## `[measured]` The GSP-level wire, which is the only boundary that counts
//!
//! `traces/rpctrace_ga106_boot1.bin` is an RPC-level capture from a real GA106 boot, and it
//! contains **three** `0x20800102` calls, all `status=0x0 psize=564`:
//!
//! ```text
//! seq303  REQ 01000000 11000080 00000000                    (listSize 1; index 0x11 | FORWARD)
//!         REP 01000000 11000000 00000000                    (bit 31 CLEARED, data 0)
//! seq780  REQ/REP identical to seq303
//! seq806  REQ 02000000 23000080 00000000 24000080 00000000
//!         REP 02000000 23000000 58e0ec19 24000000 32251eb9
//! ```
//!
//! Three facts fall straight out of those bytes and none of them was guessable:
//!
//! 1. **The forward bit really is set on the wire**, and the reply **clears it**. A port
//!    that matched the request's index word against a table keyed on `0x11` would match
//!    nothing at all, and a port that echoed the index unchanged would hand the guest back
//!    an index word with bit 31 still set. The C artifact hit exactly this and fixed it by
//!    hand: *"the guest request currently arrives as `0x80000011`; leaving that bit set is
//!    the remaining non-gpuId control divergence in the cuCtxCreate trace"*
//!    (`C: src/qemu/nvkvm_gpu_emul.c:3220-3224`).
//! 2. **The untouched tail of the 564-byte struct comes back verbatim.** `[measured]` by
//!    seeding it `0xCD` through `rmladder --gpu-info-sweep` (R21) and reading it back
//!    unchanged. So the reply is the **request, edited** — not a fresh buffer — and this
//!    module encodes it that way.
//! 3. ★★★ **Indices `0x23` and `0x24` are PER-CHIP IDENTITY VALUES, and no table may hold
//!    them.** See the section below; this is the reason this module refuses rather than
//!    completes.
//!
//! ## ★★★ `0x23` / `0x24` — measured on two different physical GA106s, and they DISAGREE
//!
//! | source | GPU | `0x23` | `0x24` |
//! |---|---|---|---|
//! | `traces/rpctrace_ga106_boot1.bin` seq806 (GSP RPC reply) | `GPU-e28d7776-e4f9-704b-d392-d46f187343f8` | `0x19ece058` | `0xb91e2532` |
//! | `rmladder --gpu-info-sweep`, 2026-08-08, run 1 | `GPU-d0913685-1ec0-805a-e319-43a901a0e1ff` | `0x4324d4e9` | `0x8708a4a8` |
//! | the same sweep, run 2, same box | same | `0x4324d4e9` | `0x8708a4a8` |
//!
//! ⇒ **Stable across runs on one part, different between two parts.** Both are unnamed in
//! both vendored open headers (`ogkm-580` and `ogkm-610` leave `0x23`, `0x24` and `0x26`
//! blank between `GEMINI_BOARD` `0x22` and `SURPRISE_REMOVAL_POSSIBLE` `0x25`), and the
//! physical handler is GSP firmware, so no source in this repository says what they mean.
//!
//! ⊘ **This is exactly the shape `derive_what_you_cannot_query_then_oracle_it` forbids a
//! table for**, and it is why this module ships **one** row rather than seventy. A constant
//! transcribed from whichever GA106 happened to be rented would be a per-chip fact wearing a
//! chip-family label — right on one box, silently wrong on the next, and the wrongness is a
//! 32-bit identifier nobody would notice.
//!
//! ⊘ And it is **not** the `dlen = 0` mistake in reverse. Answering them `0` is not
//! "decoding an absence to zeros" — it is contradicting three positive measurements that all
//! say non-zero. The C artifact did answer `0` (its map has no row and its default is zero,
//! `C: nvkvm_gpu_emul.c:3226-3231`) and still reached `bad=0 maxerr=0`, so `0` is *probably*
//! survivable — and *probably* is not a reason to write a wrong number into a reply the
//! guest may cache forever (`RMCTRL_FLAGS_CACHEABLE_BY_INPUT`,
//! `docs/reference/gsp_control_classification.tsv:65`).
//!
//! ⇒ [`GpuInfoError::UnmeasuredForwardedIndex`], **by name**, and the whole call refused —
//! which is also what RM itself does, breaking its loop and returning the error for the
//! entire request on the first index it cannot answer (`:566-569`).
//!
//! ## ⊘ Why refusing those two cannot regress anything
//!
//! Today **the entire control is unserved** — seven committed bench boots log
//! `unserviced fn 76 cmd 0x20800102` — and those boots still reach `cuInit`. So serving the
//! `0x11` rows and refusing the `0x23`/`0x24` row is **strictly more than the status quo on
//! every recorded call**, and the one call that stays refused stays refused exactly as it is
//! today. There is no configuration in which this module makes a boot worse.

/// `NV2080_CTRL_CMD_GPU_GET_INFO_V2`.
///
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h:285`.
pub const NV2080_CTRL_CMD_GPU_GET_INFO_V2: u32 = 0x2080_0102;

/// `NV2080_CTRL_GPU_INFO_MAX_LIST_SIZE` — 70, and it is **both** the array length and the
/// exclusive upper bound on a legal index.
///
/// `ogkm-580: ctrl2080gpu.h:122` (`0x00000046U`), and `getGpuInfos` uses the same constant
/// for both roles: `gpuInfoListSize > MAX` and `index >= MAX` are each
/// `NV_ERR_INVALID_ARGUMENT` (`subdevice_ctrl_gpu_kernel.c:97-111`).
pub const GPU_INFO_MAX_LIST_SIZE: usize = 0x46;

/// `sizeof(NV2080_CTRL_GPU_GET_INFO_V2_PARAMS)` = `4 + 8 * 70` = 564.
///
/// `ogkm-580: ctrl2080gpu.h:289-292` — `{ NvU32 gpuInfoListSize; NV2080_CTRL_GPU_INFO
/// gpuInfoList[70]; }`, each element two `NvU32`s. Confirmed on the wire as `psize=564` in
/// all three recorded GSP calls and in the interposed `cuInit` ioctl.
pub const GPU_GET_INFO_V2_PARAMS_SIZE: usize = 4 + 8 * GPU_INFO_MAX_LIST_SIZE;

/// ★★★ Bit 31 of the index word: *"the guest kernel could not answer this one — you answer
/// it."*
///
/// `INDEX_FORWARD_TO_PHYSICAL` (`subdevice_ctrl_gpu_kernel.c:83`), `ct_assert`ed at `:84` to
/// equal `DRF_NUM(2080, _CTRL_GPU_INFO_INDEX, _RESERVED, 1)` — i.e. the SDK's own
/// `_RESERVED 31:31` field (`ctrl2080gpu.h:125`) is what the kernel repurposes as the
/// forward marker. It is set on the request and **must be cleared in the reply**.
pub const GPU_INFO_INDEX_FORWARD_TO_PHYSICAL: u32 = 0x8000_0000;

/// `NV2080_CTRL_GPU_INFO_INDEX_INDEX 23:0` (`ogkm-580: ctrl2080gpu.h:64`).
pub const GPU_INFO_INDEX_MASK: u32 = 0x00ff_ffff;

/// `NV2080_CTRL_GPU_INFO_INDEX_GROUP_ID 30:24` (`ogkm-580: ctrl2080gpu.h:124`).
///
/// RM validates it against `pGpu->gpuGroupCount` (`subdevice_ctrl_gpu_kernel.c:113-118`).
/// This port realizes one GPU group, so any non-zero group is a question about a device
/// that does not exist here.
pub const GPU_INFO_GROUP_ID_SHIFT: u32 = 24;

/// ★★★ The **forwarded** rows a GA106 answers, and the whole of what this port can say
/// truthfully. One row.
///
/// ⊘ This is deliberately **not** the eleven-row table in `execution_plane_increments.md`
/// §14.27 and **not** the seventy rows `rmladder --gpu-info-sweep` measured. Both of those
/// are ioctl-boundary readings, and every index in them except `0x11` is answered by the
/// guest's own kernel before the RPC is built — see the module docs. A row here is a claim
/// about what **GSP-RM** returns, and there are exactly two ways to earn one: read it out of
/// a GSP-level capture's reply, or measure an index the open switch is known to forward.
///
/// `0x11` has it three ways over two different physical parts:
/// `traces/rpctrace_ga106_boot1.bin` seq303/seq780 (`REP … 11000000 00000000`),
/// `rmladder --gpu-info-sweep` (`0x11 NV_OK data=0`), and libcuda's own eleven-index ioctl
/// (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:42`, `out=` pair `0x11 → 0`).
///
/// ⚠ `0` here is a **positive reading**, not an unfilled field. That distinction is the
/// whole of `c_oracle_empty_rows_are_wrong`, and it is the reason this row is admissible
/// while `0x23`/`0x24` are not.
pub const GA106_FORWARDED_GPU_INFO: &[(u32, u32)] = &[(0x11, 0)];

/// Why a `GPU_GET_INFO_V2` request could not be answered. Each variant names the offending
/// value **and** the bound, because the alternative is a guest-side failure that mentions
/// neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuInfoError {
    /// The guest's own `gpuInfoListSize` is zero or larger than the array it indexes.
    ///
    /// ⊘ It is a **guest-supplied count used as a loop bound over a buffer**, so it is
    /// checked before it is used, never after and never trusted. RM applies the identical
    /// bound and calls it `NV_ERR_INVALID_ARGUMENT`
    /// (`ogkm-580: subdevice_ctrl_gpu_kernel.c:97-101`) — including the `== 0` half, which
    /// is easy to leave out and which makes an empty request a legal no-op instead of the
    /// error the driver expects.
    ListSize {
        /// What the guest declared.
        asked: u32,
        /// [`GPU_INFO_MAX_LIST_SIZE`].
        max: usize,
    },
    /// The params buffer is shorter than [`GPU_GET_INFO_V2_PARAMS_SIZE`].
    ShortParams {
        /// What arrived.
        len: usize,
        /// What the struct is.
        need: usize,
    },
    /// The index field is `>= NV2080_CTRL_GPU_INFO_MAX_LIST_SIZE`, which RM rejects at
    /// `subdevice_ctrl_gpu_kernel.c:107-111` before it reaches any handler.
    IndexOutOfRange {
        /// The index the guest asked for, forward bit and group already stripped.
        index: u32,
        /// [`GPU_INFO_MAX_LIST_SIZE`].
        max: usize,
    },
    /// A non-zero `GROUP_ID`. This port realizes one GPU group, so the answer is about a
    /// device that does not exist rather than a value that is merely unknown.
    ForeignGroup {
        /// The group the guest asked about.
        group: u32,
    },
    /// ★★★ The guest kernel forwarded an index this port has **no measured GSP answer
    /// for**, and it is refused by name rather than filled in.
    ///
    /// This is the variant `0x23` and `0x24` land in, and the module docs carry the
    /// measurement (`[measured 2026-08-08]`, two different RTX 3060 parts): they are
    /// per-chip identity values that differ between the two.
    /// ⊘ A plausible number here is worse than a refusal — the control is
    /// `CACHEABLE_BY_INPUT`, so the guest may keep a fabricated identity for the life of the
    /// boot, and nothing would log.
    UnmeasuredForwardedIndex {
        /// The forwarded index, forward bit already stripped.
        index: u32,
    },
}

impl core::fmt::Display for GpuInfoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ListSize { asked, max } => write!(
                f,
                "gpuInfoListSize {asked} is not in 1..={max} — the guest's own count is not \
                 a bound this port may take on trust"
            ),
            Self::ShortParams { len, need } => {
                write!(f, "{len}-byte params, {need} needed for the struct")
            }
            Self::IndexOutOfRange { index, max } => {
                write!(f, "gpu info index {index:#x} is not below {max:#x}")
            }
            Self::ForeignGroup { group } => write!(
                f,
                "gpu info GROUP_ID {group} — this port realizes one GPU group (0)"
            ),
            Self::UnmeasuredForwardedIndex { index } => write!(
                f,
                "index {index:#x} was forwarded to physical RM and this port has no MEASURED \
                 GSP answer for it; refused by name rather than invented"
            ),
        }
    }
}

impl core::error::Error for GpuInfoError {}

/// ★★★ Answer a `GPU_GET_INFO_V2` request: **the request, edited**.
///
/// The reply is a byte-for-byte copy of `request` with one edit per entry the guest kernel
/// marked [`GPU_INFO_INDEX_FORWARD_TO_PHYSICAL`] — bit 31 cleared, `data` filled from
/// `forwarded`. Everything else is left exactly as it arrived, and that is a **correctness
/// requirement, not an economy**:
///
/// - ⊘ **An entry without the forward bit already holds the guest kernel's own answer.**
///   `getGpuInfos` writes `pParams->gpuInfoList[i].data = data;` at `:566` for every index it
///   resolves, and only then RPCs the struct. Overwriting one of those replaces a value the
///   kernel computed from its own state with a value we transcribed from someone else's
///   machine.
/// - ⊘ **The tail past `gpuInfoListSize` is the guest's**, and
///   `[measured 2026-08-08, RTX 3060, driver 580.159.04]` — real GSP returns it unchanged
///   (`rmladder --gpu-info-sweep` R21, `tail=untouched` with a `0xCD` seed). Zeroing it
///   would be a 500-byte divergence nobody asked for.
///
/// # Errors
///
/// Every variant of [`GpuInfoError`]. In particular
/// [`GpuInfoError::UnmeasuredForwardedIndex`] refuses the **whole** call rather than the one
/// entry, which is RM's own all-or-nothing shape: its loop `break`s on the first non-`NV_OK`
/// status and returns it for the entire request (`:566-569`).
pub fn answer_gpu_get_info_v2(
    request: &[u8],
    forwarded: &[(u32, u32)],
) -> Result<Vec<u8>, GpuInfoError> {
    let Some(body) = request.get(..GPU_GET_INFO_V2_PARAMS_SIZE) else {
        return Err(GpuInfoError::ShortParams {
            len: request.len(),
            need: GPU_GET_INFO_V2_PARAMS_SIZE,
        });
    };
    let mut out = body.to_vec();

    let count = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
    // ⊘ The bound comes FIRST and both halves of it are RM's: zero is as illegal as 71.
    if count == 0 || count as usize > GPU_INFO_MAX_LIST_SIZE {
        return Err(GpuInfoError::ListSize {
            asked: count,
            max: GPU_INFO_MAX_LIST_SIZE,
        });
    }

    for i in 0..count as usize {
        // In range by construction: `count <= 70` and the buffer is `4 + 8 * 70` long.
        let at = 4 + 8 * i;
        let word = u32::from_le_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]]);
        let index = word & GPU_INFO_INDEX_MASK;
        let group = (word & !GPU_INFO_INDEX_FORWARD_TO_PHYSICAL) >> GPU_INFO_GROUP_ID_SHIFT;
        if index as usize >= GPU_INFO_MAX_LIST_SIZE {
            return Err(GpuInfoError::IndexOutOfRange {
                index,
                max: GPU_INFO_MAX_LIST_SIZE,
            });
        }
        if group != 0 {
            return Err(GpuInfoError::ForeignGroup { group });
        }
        if word & GPU_INFO_INDEX_FORWARD_TO_PHYSICAL == 0 {
            // ★ The guest kernel already answered this one. Leave both words alone.
            continue;
        }
        let Some(&(_, value)) = forwarded.iter().find(|&&(idx, _)| idx == index) else {
            return Err(GpuInfoError::UnmeasuredForwardedIndex { index });
        };
        out[at..at + 4]
            .copy_from_slice(&(word & !GPU_INFO_INDEX_FORWARD_TO_PHYSICAL).to_le_bytes());
        out[at + 4..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    Ok(out)
}

/// Read the `(index, data)` pairs back out of a params buffer — the inverse of
/// [`answer_gpu_get_info_v2`]'s edit, for tests and for the trace differential.
///
/// ⚠ Index words are returned **raw**, forward bit and group included, because "did bit 31
/// survive?" is the single most load-bearing question a differential can ask of this control
/// and a decoder that helpfully stripped it would make that question unaskable.
///
/// # Errors
///
/// [`GpuInfoError::ShortParams`] or [`GpuInfoError::ListSize`].
pub fn decode_gpu_info_pairs(params: &[u8]) -> Result<Vec<(u32, u32)>, GpuInfoError> {
    let Some(body) = params.get(..GPU_GET_INFO_V2_PARAMS_SIZE) else {
        return Err(GpuInfoError::ShortParams {
            len: params.len(),
            need: GPU_GET_INFO_V2_PARAMS_SIZE,
        });
    };
    let count = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    if count == 0 || count as usize > GPU_INFO_MAX_LIST_SIZE {
        return Err(GpuInfoError::ListSize {
            asked: count,
            max: GPU_INFO_MAX_LIST_SIZE,
        });
    }
    Ok((0..count as usize)
        .map(|i| {
            let at = 4 + 8 * i;
            (
                u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]),
                u32::from_le_bytes([body[at + 4], body[at + 5], body[at + 6], body[at + 7]]),
            )
        })
        .collect())
}

/// Build a request the way the guest kernel builds one — for tests only.
///
/// `entries` are `(index_word, data)` pairs written verbatim, so a test can set the forward
/// bit, a group id or a pre-filled kernel answer explicitly.
#[must_use]
pub fn build_request(entries: &[(u32, u32)]) -> Vec<u8> {
    let mut p = vec![0u8; GPU_GET_INFO_V2_PARAMS_SIZE];
    p[0..4].copy_from_slice(&(u32::try_from(entries.len()).unwrap_or(u32::MAX)).to_le_bytes());
    for (i, &(word, data)) in entries.iter().enumerate().take(GPU_INFO_MAX_LIST_SIZE) {
        let at = 4 + 8 * i;
        p[at..at + 4].copy_from_slice(&word.to_le_bytes());
        p[at + 4..at + 8].copy_from_slice(&data.to_le_bytes());
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The forward bit, stated three ways so a mistyped constant is caught here.
    #[test]
    fn the_forward_marker_is_the_sdk_s_own_reserved_bit() {
        assert_eq!(GPU_INFO_INDEX_FORWARD_TO_PHYSICAL, 1 << 31);
        assert_eq!(GPU_INFO_INDEX_FORWARD_TO_PHYSICAL, 0x8000_0000);
        // `INDEX 23:0` and `GROUP_ID 30:24` and `RESERVED 31:31` must tile a u32 exactly.
        assert_eq!(
            GPU_INFO_INDEX_MASK
                | (0x7f << GPU_INFO_GROUP_ID_SHIFT)
                | GPU_INFO_INDEX_FORWARD_TO_PHYSICAL,
            u32::MAX
        );
        assert_eq!(GPU_GET_INFO_V2_PARAMS_SIZE, 564);
        assert_eq!(GPU_INFO_MAX_LIST_SIZE, 70);
    }

    /// ★★★ The bytes of `seq303` out of `traces/rpctrace_ga106_boot1.bin`, verbatim. This is
    /// the GSP-level request/reply pair a real GA106 produced, and it is the whole
    /// specification for the forwarded path.
    #[test]
    fn the_recorded_ga106_rpc_pair_is_reproduced_byte_for_byte() {
        let req = build_request(&[(0x8000_0011, 0)]);
        // The capture's REQ, transcribed: `01000000 11000080 00000000` then zeros.
        assert_eq!(&req[0..12], &[1, 0, 0, 0, 0x11, 0, 0, 0x80, 0, 0, 0, 0]);

        let rep = answer_gpu_get_info_v2(&req, GA106_FORWARDED_GPU_INFO).expect("0x11 is measured");
        // The capture's REP: `01000000 11000000 00000000` then zeros.
        assert_eq!(&rep[0..12], &[1, 0, 0, 0, 0x11, 0, 0, 0x00, 0, 0, 0, 0]);
        assert_eq!(rep.len(), GPU_GET_INFO_V2_PARAMS_SIZE);
        assert_eq!(
            decode_gpu_info_pairs(&rep).expect("well formed"),
            [(0x11, 0)]
        );
    }

    /// ⊘ The bit MUST be cleared. A reply that echoed `0x80000011` is the divergence the C
    /// artifact found by hand in its `cuCtxCreate` trace.
    #[test]
    fn the_forward_bit_is_cleared_in_the_reply_and_the_test_would_see_it_if_not() {
        let rep = answer_gpu_get_info_v2(&build_request(&[(0x8000_0011, 0)]), &[(0x11, 0)])
            .expect("served");
        let (word, _) = decode_gpu_info_pairs(&rep).expect("well formed")[0];
        assert_eq!(word & GPU_INFO_INDEX_FORWARD_TO_PHYSICAL, 0);
        assert_eq!(word, 0x11);
    }

    /// ★★★ The property the eleven-row ioctl table would have broken: an entry the guest
    /// kernel already answered is returned **untouched**, value and all.
    #[test]
    fn an_entry_the_guest_kernel_already_answered_is_not_overwritten() {
        // `0x37` (GPU_DEBUGGING_CAPABILITY) is kernel-resolved — it never carries the
        // forward bit — and the kernel had already written 1 into it. A port keyed on the
        // §14.27 ioctl table would "helpfully" rewrite it; one keyed on the *value* being
        // different would not notice. So the kernel's answer here is deliberately a value
        // NO table in this port holds.
        let req = build_request(&[(0x37, 0xdead_beef), (0x8000_0011, 0)]);
        let rep = answer_gpu_get_info_v2(&req, GA106_FORWARDED_GPU_INFO).expect("served");
        let pairs = decode_gpu_info_pairs(&rep).expect("well formed");
        assert_eq!(
            pairs[0],
            (0x37, 0xdead_beef),
            "the kernel's own answer stands"
        );
        assert_eq!(pairs[1], (0x11, 0));
    }

    /// ★★★ `0x23` / `0x24` — the `seq806` request — is refused BY NAME, and the refusal
    /// names the index.
    #[test]
    fn the_per_chip_identity_indices_are_refused_by_name_and_never_invented() {
        let req = build_request(&[(0x8000_0023, 0), (0x8000_0024, 0)]);
        assert_eq!(
            answer_gpu_get_info_v2(&req, GA106_FORWARDED_GPU_INFO),
            Err(GpuInfoError::UnmeasuredForwardedIndex { index: 0x23 })
        );
        // ⊘ And the falsifier for the whole argument: `[measured 2026-08-08]`, the two
        // real RTX 3060 parts this project has asked DISAGREE on these, so no constant can
        // be right on both.
        assert_ne!(0x19ec_e058u32, 0x4324_d4e9u32);
        assert_ne!(0xb91e_2532u32, 0x8708_a4a8u32);
        // ⊘ ...and none of the four is zero, so answering zero contradicts four positive
        // readings (`[measured 2026-08-08]` and `traces/rpctrace_ga106_boot1.bin`) rather
        // than filling an absence. That is the whole difference from the `dlen = 0` class.
        for measured in [0x19ec_e058u32, 0xb91e_2532, 0x4324_d4e9, 0x8708_a4a8] {
            assert_ne!(measured, 0);
        }
        assert!(
            !GA106_FORWARDED_GPU_INFO
                .iter()
                .any(|&(i, _)| i == 0x23 || i == 0x24),
            "a row for a per-chip identity value must never appear in this table"
        );
    }

    /// ⊘ The guest's count is a loop bound over a buffer and is never taken on trust.
    #[test]
    fn a_hostile_or_absurd_list_size_is_refused_before_it_indexes_anything() {
        for bad in [0u32, 71, 0xffff, u32::MAX] {
            let mut p = vec![0u8; GPU_GET_INFO_V2_PARAMS_SIZE];
            p[0..4].copy_from_slice(&bad.to_le_bytes());
            assert_eq!(
                answer_gpu_get_info_v2(&p, GA106_FORWARDED_GPU_INFO),
                Err(GpuInfoError::ListSize {
                    asked: bad,
                    max: GPU_INFO_MAX_LIST_SIZE
                }),
                "gpuInfoListSize {bad} must not be honoured"
            );
            assert!(decode_gpu_info_pairs(&p).is_err());
        }
        // …and the largest LEGAL count really is accepted, so the bound is not off by one
        // in the direction that refuses a legitimate request.
        let all: Vec<(u32, u32)> = (0..GPU_INFO_MAX_LIST_SIZE as u32).map(|i| (i, 0)).collect();
        let rep = answer_gpu_get_info_v2(&build_request(&all), GA106_FORWARDED_GPU_INFO)
            .expect("70 kernel-answered entries are legal");
        assert_eq!(decode_gpu_info_pairs(&rep).expect("well formed").len(), 70);
    }

    /// A truncated params buffer is refused rather than read short.
    #[test]
    fn a_short_params_buffer_never_decodes() {
        let full = build_request(&[(0x8000_0011, 0)]);
        for n in [0usize, 4, 11, GPU_GET_INFO_V2_PARAMS_SIZE - 1] {
            assert_eq!(
                answer_gpu_get_info_v2(&full[..n], GA106_FORWARDED_GPU_INFO),
                Err(GpuInfoError::ShortParams {
                    len: n,
                    need: GPU_GET_INFO_V2_PARAMS_SIZE
                })
            );
        }
    }

    /// An index past the array, and a group that names a GPU this port does not realize.
    #[test]
    fn an_out_of_range_index_and_a_foreign_group_are_each_named() {
        assert_eq!(
            answer_gpu_get_info_v2(
                &build_request(&[(0x8000_0046, 0)]),
                GA106_FORWARDED_GPU_INFO
            ),
            Err(GpuInfoError::IndexOutOfRange {
                index: 0x46,
                max: GPU_INFO_MAX_LIST_SIZE
            })
        );
        // group 1, index 0x11, forward bit set.
        assert_eq!(
            answer_gpu_get_info_v2(
                &build_request(&[(0x8000_0011 | (1 << GPU_INFO_GROUP_ID_SHIFT), 0)]),
                GA106_FORWARDED_GPU_INFO
            ),
            Err(GpuInfoError::ForeignGroup { group: 1 })
        );
    }

    /// ★ `[measured 2026-08-08, RTX 3060, driver 580.159.04]` The tail past
    /// `gpuInfoListSize` comes back verbatim, because real GSP returns it verbatim
    /// (`rmladder --gpu-info-sweep` R21's `0xCD` seed came back untouched).
    #[test]
    fn the_tail_past_the_declared_count_is_returned_verbatim() {
        let mut req = build_request(&[(0x8000_0011, 0)]);
        for b in &mut req[12..] {
            *b = 0xCD;
        }
        let rep = answer_gpu_get_info_v2(&req, GA106_FORWARDED_GPU_INFO).expect("served");
        assert!(
            rep[12..].iter().all(|&b| b == 0xCD),
            "the reply must be the request EDITED, not a fresh buffer"
        );
    }
}
