//! `NV2080_CTRL_CMD_FB_GET_INFO_V2` (`0x20801303`) — ★★★ the wall §14.31 named, and the
//! first one this port serves **without stating a single new number**.
//!
//! ## ⊘⊘ The two refutations this module exists to record, and both are §14.31's
//!
//! ### 1. ★★★ "It appears in the device ledger in NEITHER direction ⇒ it never reaches the
//! emulated GSP" — FALSE. **Both ledgers were FULL.**
//!
//! §14.31 grepped boot `gt1431_ff7a0ea` for `unserviced fn 76 cmd 0x20801303` and for
//! `control 0x20801303 result …`, found neither, and concluded the command never leaves the
//! guest kernel. `[measured 2026-08-09, re-reading that same boot's
//! `/workspace/bench/run_gt1431_ff7a0ea_qemu.log`]` the two summary lines it did not read
//! say:
//!
//! ```text
//! nvkvm: commands: 362 decoded, 67 UNSERVICED (…), 32 distinct
//! nvkvm: controls: 101 answered, 32 distinct cmd/result rows (…)
//! ```
//!
//! **Both `32`s are the caps** — `kayfabe_device::unserviced::UNSERVICED_SAMPLE_MAX` and
//! `kayfabe_qemu_raw::shim::SERVED_CONTROL_SLOTS`, each `= 32` — and both are
//! `if s.len() < MAX && !s.contains(&entry)`, so once the distinct set fills, **every
//! later first-seen command is dropped silently**. `0x2080012f` is the thirty-second and
//! last unserviced row printed; `0x20801303` is asked *after* it and had nowhere to go.
//!
//! ⇒ ★★ **An absence from a saturated list is not evidence of absence.** This is the same
//! species as `c_oracle_empty_rows_are_wrong` (an empty capture is evidence of nothing) and
//! of `pgrep_comm_truncation_trap` (a check that cannot fail), reached through a third
//! route: an instrument that silently stops recording at a bound nobody looked at. The
//! ledger's own module documents the cap in prose and the reader still concluded from a
//! miss — so a *documented* bound is not a bound anybody checks. ★ The caps are raised
//! and, more importantly, the printer now says when a list is **truncated**, so a future
//! reader cannot mistake a full list for a complete one.
//!
//! ### 2. ⚠ §14.31's own reply table mis-transcribed `0x08`, one byte-boundary off — again
//!
//! It published *"`0x08` | `0x0000c000` — ★ already agreed"*. `[measured]` the raw bytes at
//! `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:50` are `08000000 0000c000`, which
//! little-endian is index `0x08`, data **`0x00c00000`** — `12 582 912` KiB = **12 GiB**, the
//! RTX 3060 12 GB's whole framebuffer. `0x0000c000` would be 48 MiB. The same trace's
//! line 36 answers the same index the same way, and so does our own guest at
//! `cuinit_trace_guest_gt1431_ff7a0ea.txt`.
//!
//! ⇒ This is the second time in three rungs that a hand-regrouped word reached a published
//! table (§14.30's `0x03003020` for `0x00302000` was the first), and both times the wrong
//! word **decoded plausibly**. ★ It cost nothing here only because `0x08` turns out to be
//! answered by the guest's own kernel and is never forwarded — see below. The lesson stands
//! whichever way the luck fell: **re-derive from the hex, never from the paragraph.**
//!
//! ## ★★★ Which of the seven indices actually reach a GSP — three, and it is not seven
//!
//! `_kmemsysGetFbInfos` (`ogkm-580: src/nvidia/src/kernel/gpu/mem_sys/kern_mem_sys_ctrl.c:137-996`)
//! answers what it can from the guest's own kernel state, tracks the rest in
//! `fbInfoListIndicesUnset`, and only then RPCs. Every index with a `case` arm in that
//! function's second `switch` is **kernel-answered**; the `default:` arm is a bare
//! `continue`, which leaves the bit set and sends the index onward. For libcuda's failing
//! request (`cuinit_ioctl_trace_real_ga106.txt:50`, seven indices):
//!
//! | index | name | where it is answered |
//! |---|---|---|
//! | `0x08` | `TOTAL_RAM_SIZE` | **guest kernel** — `:335`, from `pMemoryManager->Ram.fbTotalMemSizeMb` |
//! | `0x17` | `RAM_LOCATION` | **guest kernel** — `:711`, a constant |
//! | `0x18` | `FB_IS_BROKEN` | **guest kernel** — `:716`, a PDB property |
//! | `0x0b` | `BUS_WIDTH` | ★ **forwarded** — no `case` |
//! | `0x19` | `FBP_COUNT` | ★ **forwarded** — no `case` |
//! | `0x1b` | `L2CACHE_SIZE` | ★ **forwarded** — no `case` |
//! | `0x0d` | `RAM_TYPE` | ★ **forwarded** — no `case` |
//!
//! ⊘ **So a port that transcribed all seven from the trace would overwrite three values the
//! guest had already computed correctly** — `GPU_GET_INFO_V2`'s ten-of-eleven and
//! `BUS_GET_INFO_V2`'s five-of-six for the third time. `[measured]` our own boot already
//! answers `0x08` **byte-identically to a real GA106** (`0x00c00000`) with no help from this
//! module, because our emulated part really does present 12 GiB.
//!
//! ## ⚠ The RPC's shape is NOT `BUS_GET_INFO_V2`'s, and the difference is load-bearing
//!
//! `kbusSendBusInfo_IMPL` forwards **one entry per RPC**. `_kmemsysGetFbInfos` does not: it
//! allocates **one fresh** `NV2080_CTRL_FB_GET_INFO_V2_PARAMS`, copies the still-unset
//! indices into it **compacted from slot 0**, sets `fbInfoListSize` to that count, and sends
//! a single RPC (`:952-990`). So the wire request this module answers is a **four**-entry
//! struct — `{0x0b, 0x19, 0x1b, 0x0d}` in the original relative order — never the guest's
//! seven-entry ioctl buffer, and its `data` words arrive zeroed rather than as libcuda left
//! them.
//!
//! ★ The property that matters is nonetheless the same one [`crate::businfo`] relies on:
//! **arriving here is the marker.** Only indices the guest kernel could not answer are put
//! in the RPC, so every declared entry is filled and an index with no derivation refuses the
//! **whole** call — which is RM's own shape, since `_kmemsysGetFbInfos` returns the RPC's
//! status for the entire request and one poisoned entry fails all seven.
//!
//! ## ★★★ What this port answers — and it restates NOTHING
//!
//! All four forwarded indices are **projections of
//! `kayfabe_device::ChipProfile::memory_system`**, the row this port already serves to
//! `NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG` (`0x20800a1c`, see
//! [`crate::memsysconfig`]). Two are the same field verbatim; two are derived from one field
//! by an arithmetic relation.
//!
//! | index | served | from | real GA106 |
//! |---|---|---|---|
//! | `0x1b` `L2CACHE_SIZE` | `0x0024_0000` | `memory_system.l2_cache_size`, verbatim | `0x00240000` ✓ |
//! | `0x0d` `RAM_TYPE` | `0x11` `GDDR6` | `memory_system.ram_type`, verbatim | `0x00000011` ✓ |
//! | `0x0b` `BUS_WIDTH` | `192` | `ltc_count × 32` | `0x000000c0` ✓ |
//! | `0x19` `FBP_COUNT` | `3` | `ltc_count ÷ 2` | `0x00000003` ✓ |
//!
//! ★★ **The projection is the design, not a shortcut.** `l2CacheSize` and `ramType` are
//! *the same two silicon facts* under two control ids. A port that pasted the trace's words
//! into a second table would hold two independently-written descriptions of one chip — the
//! drift `kayfabe_device::ChipProfile::device_info` already refuses to create — and would be
//! able to tell RM that its L2 is 2.25 MiB in one reply and something else in the next.
//! Here they cannot disagree, because there is only one of each.
//!
//! ### The two relations, with the arch they are true of stated
//!
//! - **`BUS_WIDTH = ltc_count × 32`.** One LTC sits in front of one 32-bit FBPA. RM's own
//!   `PARTITION_COUNT` doc says that index *"returns the number of FBPAs"* while `FBP_COUNT`
//!   returns FBPs (`ogkm-580: ctrl2080fb.h:68-74, 200-208`), and the Ampere PLC arms in
//!   `kmemsysIsPagePLCable_GA102` (`kern_mem_sys_ga102.c:66-120`) enumerate
//!   `ltsPerLtcCount × ltcCount ∈ {48, 40, 4×8, 3×8}` — i.e. `ltcCount ∈ {12, 10, 8}` for
//!   the 384-, 320- and 256-bit Ampere parts. `12×32 = 384`, `10×32 = 320`, `8×32 = 256`,
//!   and here `6×32 = 192`. Four parts, one relation.
//! - **`FBP_COUNT = ltc_count ÷ 2`.** An FBP aggregates two FBPAs on GA10x, so GA102's
//!   twelve LTCs are six FBPs and GA106's six are three. ⊘ An odd `ltc_count` is
//!   [`FbInfoError::LtcCountNotPairable`], never a rounded answer.
//!
//! ⊘ Both live here rather than on a chip row, for the reason §14.30 established for
//! `PCIE_GEN_INFO`: a `&'static [(u32, u32)]` is the shape that invites a measured word to
//! be pasted in. ⚠ And both are named `GA10X_*` rather than left anonymous, because the day
//! a Hopper row appears they are the two lines that have to be looked at — an arch seam at
//! the time the code is written, not a retrofit.
//!
//! ## ⊘⊘ The trap the OBVIOUS next step walks into — `LTS_COUNT` is NOT `ltc × ltsPerLtc`
//!
//! The very next `FB_GET_INFO_V2` in the same real trace (`:66`) asks `{0x1a FBP_MASK,
//! 0x22 LTC_COUNT, 0x23 LTS_COUNT}` and is answered `{0x07, 6, 18}`. `0x22` is
//! `memory_system.ltc_count` exactly. `0x23` is **not** `ltc_count × lts_per_ltc_count`:
//! that product is `6 × 4 = 24`, and the hardware says **18**.
//!
//! Both readings are real-hardware, and neither is wrong. The static config's
//! `ltsPerLtcCount = 4` is a captured GSP reply (`C: src/qemu/mode2_initctrl_ga106.h:5391`,
//! `dlen 40`, a row *with* a body); `FB_INFO_INDEX_LTS_COUNT` is documented as *"the **active**
//! LTS count across all active LTCs"* (`ogkm-580: ctrl2080fb.h:251-254`), and a floorswept
//! GA106 runs three of each LTC's four slices — `18 × 128 KiB = 2304 KiB`, which is
//! `l2_cache_size` and which matches GA102's `48 × 128 KiB = 6 MiB` from the `== 48` arm.
//!
//! ★★★ ⚠ **And `ga10x.rs`'s own comment on that field is arithmetic that self-justifies the
//! wrong reading**: *"2.25 MiB = 24 slices x 96 KiB … The capture agrees with itself."*
//! 24 × 96 KiB is 2304 KiB, so it *checks out* — and 96 KiB is not an Ampere L2 slice.
//! `two_encodings_agreeing_on_the_first_values`, in a doc comment: a consistency check that
//! passes for a value that is being read as the wrong quantity. ⊘ The field itself is
//! **correct and is not touched here**; only the justification is.
//!
//! ⇒ `0x1a`/`0x22`/`0x23` are deliberately **not served by this rung**. `0x23` has exactly
//! one supporting reading and a plausible-looking derivation that contradicts it, which is
//! the configuration that has produced this project's silent wrong answers. It is the next
//! rung, with the evidence written down.

/// `NV2080_CTRL_CMD_FB_GET_INFO_V2` (`ogkm-580: ctrl2080fb.h:459`).
pub const NV2080_CTRL_CMD_FB_GET_INFO_V2: u32 = 0x2080_1303;

/// `NV2080_CTRL_FB_INFO_MAX_LIST_SIZE` — 128 (`ogkm-580: ctrl2080fb.h:371`).
///
/// ⚠ **This is the array length and NOT the legal-index bound**, which is the one place
/// this control's shape differs from [`crate::businfo`]'s. There, `BUS_INFO_MAX_LIST_SIZE`
/// and `INDEX_MAX + 1` are the same number, so one check does both jobs. Here the header
/// says so out loud — *"Intentionally picking a value much bigger than
/// NV2080_CTRL_FB_INFO_INDEX_MAX to prevent VGPU plumbing updates"* — and a copy of
/// `businfo`'s single bound would accept 68 indices that name nothing. See
/// [`FB_INFO_INDEX_MAX`].
pub const FB_INFO_MAX_LIST_SIZE: usize = 0x80;

/// `NV2080_CTRL_FB_INFO_INDEX_MAX` = `..._NUMA_NODE_ID` = `0x3b`
/// (`ogkm-580: ctrl2080fb.h:366-368`) — the largest index the header defines, and the bound
/// this module rejects on. See [`FB_INFO_MAX_LIST_SIZE`] for why the two are different.
pub const FB_INFO_INDEX_MAX: u32 = 0x3b;

/// `sizeof(NV2080_CTRL_FB_GET_INFO_V2_PARAMS)` = `4 + 8 * 128` = 1028.
///
/// `[measured 2026-08-08, real GA106 `GPU-d0913685`, driver 580.159.04 Open]` on the wire as
/// `size=1028` on all five `FB_GET_INFO_V2` calls of
/// `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt` (lines 36, 37, 41, 50, 66), and
/// `[measured 2026-08-08, boot `gt1432_20e319b`]` on all four of our own guest's
/// (`cuinit_trace_guest_gt1432_20e319b.txt`). ⊘ Separately — a *reading*, not a measurement
/// — it is the size `_kmemsysGetFbInfos` hands `NV_RM_RPC_CONTROL` (`sizeof(*pRpcParams)`,
/// `ogkm-580: kern_mem_sys_ctrl.c:975`).
pub const FB_GET_INFO_V2_PARAMS_SIZE: usize = 4 + 8 * FB_INFO_MAX_LIST_SIZE;

/// `NV2080_CTRL_FB_INFO_INDEX_TOTAL_RAM_SIZE` — ⊘ **guest-kernel answered**, never
/// forwarded (`ogkm-580: kern_mem_sys_ctrl.c:335`). Named so a test can assert this module
/// refuses it rather than answering a framebuffer size it does not own.
pub const FB_INFO_INDEX_TOTAL_RAM_SIZE: u32 = 0x08;

/// `NV2080_CTRL_FB_INFO_INDEX_BUS_WIDTH` — ★ forwarded. The FB data bus width in bits.
pub const FB_INFO_INDEX_BUS_WIDTH: u32 = 0x0b;

/// `NV2080_CTRL_FB_INFO_INDEX_RAM_TYPE` — ★ forwarded. An `NV2080_CTRL_FB_INFO_RAM_TYPE_*`.
pub const FB_INFO_INDEX_RAM_TYPE: u32 = 0x0d;

/// `NV2080_CTRL_FB_INFO_INDEX_RAM_LOCATION` — ⊘ guest-kernel answered
/// (`ogkm-580: kern_mem_sys_ctrl.c:711`).
pub const FB_INFO_INDEX_RAM_LOCATION: u32 = 0x17;

/// `NV2080_CTRL_FB_INFO_INDEX_FB_IS_BROKEN` — ⊘ guest-kernel answered
/// (`ogkm-580: kern_mem_sys_ctrl.c:716`).
pub const FB_INFO_INDEX_FB_IS_BROKEN: u32 = 0x18;

/// `NV2080_CTRL_FB_INFO_INDEX_FBP_COUNT` — ★ forwarded. FBPs, **not** FBPAs.
pub const FB_INFO_INDEX_FBP_COUNT: u32 = 0x19;

/// `NV2080_CTRL_FB_INFO_INDEX_FBP_MASK` — ⊘ forwarded, and deliberately **not answered by
/// this rung**; see this module's header.
pub const FB_INFO_INDEX_FBP_MASK: u32 = 0x1a;

/// `NV2080_CTRL_FB_INFO_INDEX_L2CACHE_SIZE` — ★ forwarded. In **bytes**
/// (`ogkm-580: ctrl2080fb.h:210-213`), and ⊘ *"a value of zero indicates that the L2 cache
/// isn't supported"* — so zero is a positive claim of absence, never a blank.
pub const FB_INFO_INDEX_L2CACHE_SIZE: u32 = 0x1b;

/// `NV2080_CTRL_FB_INFO_INDEX_LTC_COUNT` — ⊘ forwarded, not answered by this rung.
pub const FB_INFO_INDEX_LTC_COUNT: u32 = 0x22;

/// `NV2080_CTRL_FB_INFO_INDEX_LTS_COUNT` — ⊘ forwarded, not answered by this rung, and the
/// one whose obvious derivation is contradicted by hardware:
/// `[measured 2026-08-08, real GA106 `GPU-d0913685`, driver 580.159.04 Open,
/// `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:66`]` this index answers **18**
/// while `ltcCount × ltsPerLtcCount` from this port's own `0x20800a1c` row is **24**. See
/// this module's header, and `kayfabe-device/tests/fb_get_info_v2.rs`, which fails if the
/// two ever stop disagreeing.
pub const FB_INFO_INDEX_LTS_COUNT: u32 = 0x23;

/// One LTC fronts one 32-bit FBPA on GA10x, so the FB data bus is this many bits per LTC.
///
/// ⚠ Named with its architecture. `[measured/derived]` against four Ampere parts through
/// `kmemsysIsPagePLCable_GA102`'s slice-count arms — see this module's header for the
/// arithmetic.
pub const GA10X_FBPA_DATA_BITS: u32 = 32;

/// An FBP aggregates this many FBPAs (hence LTCs) on GA10x.
///
/// ⚠ Named with its architecture, for the same reason [`GA10X_FBPA_DATA_BITS`] is.
pub const GA10X_LTC_PER_FBP: u32 = 2;

/// The memory-system facts this control's forwarded indices are projected from.
///
/// ★ Deliberately a *view* over `kayfabe_device::ChipProfile::memory_system` rather than a
/// table of its own: every field here is already stated once, for `0x20800a1c`, and this
/// type exists so the second control cannot state them a second time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbGeometry {
    /// `l2CacheSize` in bytes — `MemorySystemRow::l2_cache_size`.
    pub l2_cache_size: u64,
    /// `ramType` — `MemorySystemRow::ram_type`, an `NV2080_CTRL_FB_INFO_RAM_TYPE_*`.
    pub ram_type: u32,
    /// `ltcCount` — `MemorySystemRow::ltc_count`. The only field either derivation reads.
    pub ltc_count: u32,
}

impl FbGeometry {
    /// The FB data bus width in bits: one 32-bit FBPA per LTC.
    ///
    /// # Errors
    ///
    /// [`FbInfoError::LtcCountZero`] — a zero-width bus is not a true statement about any
    /// part this port emulates, and `0` is what an unstated row would produce.
    pub fn bus_width_bits(self) -> Result<u32, FbInfoError> {
        if self.ltc_count == 0 {
            return Err(FbInfoError::LtcCountZero);
        }
        Ok(self.ltc_count * GA10X_FBPA_DATA_BITS)
    }

    /// The number of FBPs: two LTCs each on GA10x.
    ///
    /// # Errors
    ///
    /// [`FbInfoError::LtcCountZero`], or [`FbInfoError::LtcCountNotPairable`] when the count
    /// is odd — which cannot describe a GA10x part and must not be rounded into one.
    pub fn fbp_count(self) -> Result<u32, FbInfoError> {
        if self.ltc_count == 0 {
            return Err(FbInfoError::LtcCountZero);
        }
        if !self.ltc_count.is_multiple_of(GA10X_LTC_PER_FBP) {
            return Err(FbInfoError::LtcCountNotPairable {
                ltc_count: self.ltc_count,
            });
        }
        Ok(self.ltc_count / GA10X_LTC_PER_FBP)
    }

    /// The L2 cache size in bytes, as the 32-bit field this control returns.
    ///
    /// # Errors
    ///
    /// [`FbInfoError::L2CacheSizeZero`] — the header makes `0` mean *"no L2 on this
    /// subdevice"* — or [`FbInfoError::L2CacheSizeTooWide`] when the row's `u64` does not
    /// fit the reply's `u32`, which would otherwise truncate a real size into a small one.
    pub fn l2_cache_size_u32(self) -> Result<u32, FbInfoError> {
        if self.l2_cache_size == 0 {
            return Err(FbInfoError::L2CacheSizeZero);
        }
        u32::try_from(self.l2_cache_size).map_err(|_| FbInfoError::L2CacheSizeTooWide {
            bytes: self.l2_cache_size,
        })
    }

    /// The RAM type.
    ///
    /// # Errors
    ///
    /// [`FbInfoError::RamTypeUnknown`]. ⊘ `NV2080_CTRL_FB_INFO_RAM_TYPE_UNKNOWN == 0`
    /// (`ogkm-580: ctrl2080fb.h:375`), so zero is a *declared* value meaning *"unknown"* and
    /// not an absence — the `two_encodings_agreeing_on_the_first_values` shape again. A row
    /// that has not stated its memory is refused rather than served as `UNKNOWN`.
    pub fn ram_type_checked(self) -> Result<u32, FbInfoError> {
        if self.ram_type == 0 {
            return Err(FbInfoError::RamTypeUnknown);
        }
        Ok(self.ram_type)
    }

    /// The `(index, data)` pairs this port answers, in ascending index order.
    ///
    /// Exactly the four indices `_kmemsysGetFbInfos` forwards out of libcuda's request. An
    /// index outside this set is [`FbInfoError::UnmeasuredIndex`] at
    /// [`answer_fb_get_info_v2`], never a filled-in zero.
    ///
    /// # Errors
    ///
    /// Whatever the four projections return; the whole set is refused rather than a
    /// partially-derived one served.
    pub fn forwarded_answers(self) -> Result<[(u32, u32); 4], FbInfoError> {
        Ok([
            (FB_INFO_INDEX_BUS_WIDTH, self.bus_width_bits()?),
            (FB_INFO_INDEX_RAM_TYPE, self.ram_type_checked()?),
            (FB_INFO_INDEX_FBP_COUNT, self.fbp_count()?),
            (FB_INFO_INDEX_L2CACHE_SIZE, self.l2_cache_size_u32()?),
        ])
    }
}

/// Why an `FB_GET_INFO_V2` request could not be answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbInfoError {
    /// `fbInfoListSize` is zero or larger than the array it indexes. RM applies exactly
    /// this bound, both halves of it, before it reads a single entry
    /// (`ogkm-580: kern_mem_sys_ctrl.c:1021-1025`).
    ListSize {
        /// What the guest declared.
        asked: u32,
        /// [`FB_INFO_MAX_LIST_SIZE`].
        max: usize,
    },
    /// The params buffer is shorter than [`FB_GET_INFO_V2_PARAMS_SIZE`].
    ShortParams {
        /// What arrived.
        len: usize,
        /// What the struct is.
        need: usize,
    },
    /// An index past [`FB_INFO_INDEX_MAX`] — a value the header names nothing for.
    ///
    /// ⚠ Bounded by `INDEX_MAX` and **not** by [`FB_INFO_MAX_LIST_SIZE`]; the two differ by
    /// 68 on this control and coincide on [`crate::businfo`]'s.
    IndexOutOfRange {
        /// The index the guest asked for.
        index: u32,
        /// [`FB_INFO_INDEX_MAX`].
        max: u32,
    },
    /// ★★★ The guest kernel forwarded an index this port has **no derivation for**, and it
    /// is refused by name rather than filled in.
    ///
    /// ⊘ Zero is not available as a fallback on this control either: `L2CACHE_SIZE = 0` is
    /// *"no L2 cache"*, `RAM_TYPE = 0` is *"unknown"*, and `FBP_COUNT = 0` is a part with no
    /// framebuffer partitions. Every one of them is a positive claim.
    UnmeasuredIndex {
        /// The forwarded index.
        index: u32,
    },
    /// The chip row states no LTCs, so neither the bus width nor the FBP count exists.
    LtcCountZero,
    /// An odd `ltcCount`, which no GA10x part has — two FBPAs make an FBP. Refused rather
    /// than rounded, because a rounded FBP count is a wrong answer nothing would notice.
    LtcCountNotPairable {
        /// The offending `ltcCount`.
        ltc_count: u32,
    },
    /// The chip row states a zero L2, which this control's header defines as *"the L2 cache
    /// isn't supported on the associated subdevice"*.
    L2CacheSizeZero,
    /// The chip row's 64-bit `l2CacheSize` does not fit this control's 32-bit `data` field.
    L2CacheSizeTooWide {
        /// The offending size in bytes.
        bytes: u64,
    },
    /// The chip row states `NV2080_CTRL_FB_INFO_RAM_TYPE_UNKNOWN`, which is a declared value
    /// meaning *unknown* rather than a blank.
    RamTypeUnknown,
}

impl core::fmt::Display for FbInfoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ListSize { asked, max } => write!(
                f,
                "fbInfoListSize {asked} is not in 1..={max} — the guest's own count is not a \
                 bound this port may take on trust"
            ),
            Self::ShortParams { len, need } => {
                write!(f, "{len}-byte params, {need} needed for the struct")
            }
            Self::IndexOutOfRange { index, max } => write!(
                f,
                "fb info index {index:#x} is above NV2080_CTRL_FB_INFO_INDEX_MAX {max:#x} \
                 (which is NOT the 0x80 array length)"
            ),
            Self::UnmeasuredIndex { index } => write!(
                f,
                "fb info index {index:#x} was forwarded to physical RM and this port has no \
                 derivation for it; refused by name rather than answered zero, which on this \
                 control reads as 'no L2', 'unknown RAM' or 'no FB partitions'"
            ),
            Self::LtcCountZero => write!(
                f,
                "the chip row states ltcCount = 0, so it states neither an FB bus width nor \
                 an FBP count"
            ),
            Self::LtcCountNotPairable { ltc_count } => write!(
                f,
                "ltcCount {ltc_count} is odd; an FBP aggregates two FBPAs on GA10x, and a \
                 rounded FBP count is a wrong answer with no symptom"
            ),
            Self::L2CacheSizeZero => write!(
                f,
                "the chip row states l2CacheSize = 0, which this control's header defines as \
                 'the L2 cache isn't supported on the associated subdevice'"
            ),
            Self::L2CacheSizeTooWide { bytes } => write!(
                f,
                "l2CacheSize {bytes:#x} does not fit the 32-bit data field; truncating it \
                 would report a smaller cache than the port serves elsewhere"
            ),
            Self::RamTypeUnknown => write!(
                f,
                "the chip row states ramType = NV2080_CTRL_FB_INFO_RAM_TYPE_UNKNOWN (0), a \
                 declared value meaning 'unknown' rather than an absence"
            ),
        }
    }
}

impl core::error::Error for FbInfoError {}

/// Answer an `FB_GET_INFO_V2` RPC: **the request, edited**.
///
/// Every entry the request declares is filled from `answers`; the tail past
/// `fbInfoListSize` is left exactly as it arrived. ⚠ Unlike [`crate::businfo`]'s tail, this
/// one is not *observed* untouched on hardware — no `rmladder` sweep of this control exists
/// yet — it is left alone because RM's own read-back loop only ever indexes
/// `0..fbInfoListSize` (`ogkm-580: kern_mem_sys_ctrl.c:977-988`), so the tail is bytes
/// nobody reads either way.
///
/// ⊘ **Every declared entry is filled**, with no forward-bit test, because
/// `_kmemsysGetFbInfos` only copies indices the guest kernel could not answer into the RPC
/// params (`:959-965`). Arriving here is the marker.
///
/// # Errors
///
/// Every variant of [`FbInfoError`]. [`FbInfoError::UnmeasuredIndex`] refuses the **whole**
/// call rather than the one entry, which is RM's shape: the caller propagates the RPC's
/// single status to the entire request and one refused index fails all of them.
pub fn answer_fb_get_info_v2(
    request: &[u8],
    answers: &[(u32, u32)],
) -> Result<Vec<u8>, FbInfoError> {
    let Some(body) = request.get(..FB_GET_INFO_V2_PARAMS_SIZE) else {
        return Err(FbInfoError::ShortParams {
            len: request.len(),
            need: FB_GET_INFO_V2_PARAMS_SIZE,
        });
    };
    let mut out = body.to_vec();

    let count = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
    if count == 0 || count as usize > FB_INFO_MAX_LIST_SIZE {
        return Err(FbInfoError::ListSize {
            asked: count,
            max: FB_INFO_MAX_LIST_SIZE,
        });
    }

    for i in 0..count as usize {
        // In range by construction: `count <= 128` and the buffer is `4 + 8 * 128` long.
        let at = 4 + 8 * i;
        let index = u32::from_le_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]]);
        if index > FB_INFO_INDEX_MAX {
            return Err(FbInfoError::IndexOutOfRange {
                index,
                max: FB_INFO_INDEX_MAX,
            });
        }
        let Some(&(_, value)) = answers.iter().find(|&&(idx, _)| idx == index) else {
            return Err(FbInfoError::UnmeasuredIndex { index });
        };
        out[at + 4..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    Ok(out)
}

/// Read the `(index, data)` pairs back out of a params buffer — for tests and for the trace
/// differential.
///
/// # Errors
///
/// [`FbInfoError::ShortParams`] or [`FbInfoError::ListSize`].
pub fn decode_fb_info_pairs(params: &[u8]) -> Result<Vec<(u32, u32)>, FbInfoError> {
    let Some(body) = params.get(..FB_GET_INFO_V2_PARAMS_SIZE) else {
        return Err(FbInfoError::ShortParams {
            len: params.len(),
            need: FB_GET_INFO_V2_PARAMS_SIZE,
        });
    };
    let count = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    if count == 0 || count as usize > FB_INFO_MAX_LIST_SIZE {
        return Err(FbInfoError::ListSize {
            asked: count,
            max: FB_INFO_MAX_LIST_SIZE,
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

/// Build a request the way `_kmemsysGetFbInfos` builds one — for tests only.
///
/// `entries` are `(index, data)` pairs written verbatim, so a test can replay both the
/// compacted RPC (whose `data` words are zero) and libcuda's own ioctl buffer.
#[must_use]
pub fn build_request(entries: &[(u32, u32)]) -> Vec<u8> {
    let mut p = vec![0u8; FB_GET_INFO_V2_PARAMS_SIZE];
    p[0..4].copy_from_slice(&(u32::try_from(entries.len()).unwrap_or(u32::MAX)).to_le_bytes());
    for (i, &(index, data)) in entries.iter().enumerate().take(FB_INFO_MAX_LIST_SIZE) {
        let at = 4 + 8 * i;
        p[at..at + 4].copy_from_slice(&index.to_le_bytes());
        p[at + 4..at + 8].copy_from_slice(&data.to_le_bytes());
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The GA106 row this port already serves to `0x20800a1c`, restated nowhere: these are
    /// `kayfabe_device::ga10x::GA106_MEMORY_SYSTEM`'s three relevant fields, and
    /// `kayfabe-device`'s own `fb_get_info_v2.rs` asserts the two are the same values.
    const GA106: FbGeometry = FbGeometry {
        l2_cache_size: 0x0024_0000,
        ram_type: 0x11,
        ltc_count: 6,
    };

    #[test]
    fn ga106_projections_match_real_hardware() {
        // `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:50`, re-derived from the raw
        // `out=` bytes rather than from any prose table.
        assert_eq!(GA106.bus_width_bits(), Ok(0x0000_00c0));
        assert_eq!(GA106.fbp_count(), Ok(0x0000_0003));
        assert_eq!(GA106.l2_cache_size_u32(), Ok(0x0024_0000));
        assert_eq!(GA106.ram_type_checked(), Ok(0x0000_0011));
    }

    #[test]
    fn the_wall_request_is_answered_byte_for_byte() {
        // The four indices `_kmemsysGetFbInfos` compacts out of libcuda's seven.
        let req = build_request(&[(0x0b, 0), (0x19, 0), (0x1b, 0), (0x0d, 0)]);
        let out = answer_fb_get_info_v2(&req, &GA106.forwarded_answers().unwrap()).unwrap();
        assert_eq!(
            decode_fb_info_pairs(&out).unwrap(),
            vec![
                (0x0b, 0x0000_00c0),
                (0x19, 0x0000_0003),
                (0x1b, 0x0024_0000),
                (0x0d, 0x0000_0011),
            ]
        );
    }

    /// ⊘ The three indices the guest kernel answers itself must be REFUSED, not served.
    /// Serving `0x08` would overwrite the guest's own correct 12 GiB with whatever a chip
    /// row happened to say.
    #[test]
    fn kernel_answered_indices_are_refused_by_name() {
        let answers = GA106.forwarded_answers().unwrap();
        for index in [
            FB_INFO_INDEX_TOTAL_RAM_SIZE,
            FB_INFO_INDEX_RAM_LOCATION,
            FB_INFO_INDEX_FB_IS_BROKEN,
        ] {
            let req = build_request(&[(index, 0)]);
            assert_eq!(
                answer_fb_get_info_v2(&req, &answers),
                Err(FbInfoError::UnmeasuredIndex { index })
            );
        }
    }

    /// ⊘ The next rung's three indices are refused too — `0x23` especially, whose obvious
    /// derivation `ltc_count * lts_per_ltc_count = 24` contradicts the hardware's 18.
    #[test]
    fn next_rungs_indices_are_refused_rather_than_guessed() {
        let answers = GA106.forwarded_answers().unwrap();
        for index in [
            FB_INFO_INDEX_FBP_MASK,
            FB_INFO_INDEX_LTC_COUNT,
            FB_INFO_INDEX_LTS_COUNT,
        ] {
            let req = build_request(&[(index, 0)]);
            assert_eq!(
                answer_fb_get_info_v2(&req, &answers),
                Err(FbInfoError::UnmeasuredIndex { index })
            );
        }
    }

    /// ⚠ One refused entry fails the WHOLE call, which is RM's own shape.
    #[test]
    fn one_unmeasured_entry_refuses_the_whole_request() {
        let req = build_request(&[(0x0b, 0), (FB_INFO_INDEX_LTS_COUNT, 0), (0x1b, 0)]);
        assert_eq!(
            answer_fb_get_info_v2(&req, &GA106.forwarded_answers().unwrap()),
            Err(FbInfoError::UnmeasuredIndex {
                index: FB_INFO_INDEX_LTS_COUNT
            })
        );
    }

    /// ★ The bound is `INDEX_MAX` (`0x3b`) and not the `0x80` array length — a copy of
    /// `businfo`'s single check would let 68 undefined indices through.
    #[test]
    fn the_index_bound_is_index_max_not_the_array_length() {
        assert!(FB_INFO_INDEX_MAX as usize + 1 < FB_INFO_MAX_LIST_SIZE);
        let req = build_request(&[(FB_INFO_INDEX_MAX + 1, 0)]);
        assert_eq!(
            answer_fb_get_info_v2(&req, &[]),
            Err(FbInfoError::IndexOutOfRange {
                index: FB_INFO_INDEX_MAX + 1,
                max: FB_INFO_INDEX_MAX,
            })
        );
        // …and an in-range-but-undefined index is a *different* refusal, so the two are
        // distinguishable rather than collapsed.
        let req = build_request(&[(FB_INFO_INDEX_MAX, 0)]);
        assert_eq!(
            answer_fb_get_info_v2(&req, &[]),
            Err(FbInfoError::UnmeasuredIndex {
                index: FB_INFO_INDEX_MAX
            })
        );
    }

    #[test]
    fn the_guests_own_count_is_not_taken_on_trust() {
        let mut req = build_request(&[(0x0b, 0)]);
        req[0..4].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            answer_fb_get_info_v2(&req, &[]),
            Err(FbInfoError::ListSize {
                asked: 0,
                max: FB_INFO_MAX_LIST_SIZE
            })
        );
        req[0..4].copy_from_slice(&(FB_INFO_MAX_LIST_SIZE as u32 + 1).to_le_bytes());
        assert_eq!(
            answer_fb_get_info_v2(&req, &[]),
            Err(FbInfoError::ListSize {
                asked: FB_INFO_MAX_LIST_SIZE as u32 + 1,
                max: FB_INFO_MAX_LIST_SIZE
            })
        );
        assert_eq!(
            answer_fb_get_info_v2(&req[..8], &[]),
            Err(FbInfoError::ShortParams {
                len: 8,
                need: FB_GET_INFO_V2_PARAMS_SIZE
            })
        );
    }

    /// ⊘ Every zero this control could answer is a positive claim, so an unstated row is a
    /// refusal rather than a default.
    #[test]
    fn an_unstated_row_refuses_rather_than_defaults() {
        let blank = FbGeometry {
            l2_cache_size: 0,
            ram_type: 0,
            ltc_count: 0,
        };
        assert_eq!(blank.bus_width_bits(), Err(FbInfoError::LtcCountZero));
        assert_eq!(blank.fbp_count(), Err(FbInfoError::LtcCountZero));
        assert_eq!(blank.l2_cache_size_u32(), Err(FbInfoError::L2CacheSizeZero));
        assert_eq!(blank.ram_type_checked(), Err(FbInfoError::RamTypeUnknown));
        assert!(blank.forwarded_answers().is_err());
    }

    #[test]
    fn an_odd_ltc_count_is_refused_rather_than_rounded() {
        let odd = FbGeometry {
            ltc_count: 5,
            ..GA106
        };
        assert_eq!(
            odd.fbp_count(),
            Err(FbInfoError::LtcCountNotPairable { ltc_count: 5 })
        );
        // …but the bus width it implies is still well defined, so the two derivations do
        // not share a failure by accident.
        assert_eq!(odd.bus_width_bits(), Ok(160));
    }

    #[test]
    fn an_l2_that_does_not_fit_the_reply_is_refused_rather_than_truncated() {
        let wide = FbGeometry {
            l2_cache_size: 0x1_0000_0000,
            ..GA106
        };
        assert_eq!(
            wide.l2_cache_size_u32(),
            Err(FbInfoError::L2CacheSizeTooWide {
                bytes: 0x1_0000_0000
            })
        );
    }

    /// ★ The bus-width relation against the three other Ampere parts RM's own PLC arms
    /// enumerate — 384-, 320- and 256-bit — so the relation is checked on more than the one
    /// part we own.
    #[test]
    fn the_bus_width_relation_holds_for_every_ampere_ltc_count_rm_enumerates() {
        for (ltc, bits, fbps) in [
            (12u32, 384u32, 6u32),
            (10, 320, 5),
            (8, 256, 4),
            (6, 192, 3),
        ] {
            let g = FbGeometry {
                ltc_count: ltc,
                ..GA106
            };
            assert_eq!(g.bus_width_bits(), Ok(bits));
            assert_eq!(g.fbp_count(), Ok(fbps));
        }
    }

    /// ⊘⊘ The measured contradiction, pinned so nobody "simplifies" `0x23` into the product
    /// later: a real GA106 answers `LTS_COUNT = 18` while the static config it also answers
    /// says `ltcCount(6) * ltsPerLtcCount(4) = 24`.
    #[test]
    fn lts_count_is_not_the_product_of_the_static_configs_two_fields() {
        const REAL_GA106_LTS_COUNT: u32 = 18; // cuinit_ioctl_trace_real_ga106.txt:66
        const STATIC_CONFIG_LTS_PER_LTC: u32 = 4; // C: mode2_initctrl_ga106.h:5391, dlen 40
        assert_ne!(
            REAL_GA106_LTS_COUNT,
            GA106.ltc_count * STATIC_CONFIG_LTS_PER_LTC
        );
        // …and 18 slices at the Ampere slice size that GA102's `== 48` arm implies is
        // exactly this part's L2, which is why 18 is the active count and 24 is not.
        assert_eq!(
            REAL_GA106_LTS_COUNT * 128 * 1024,
            GA106.l2_cache_size as u32
        );
    }

    #[test]
    fn the_tail_past_the_declared_entries_is_left_alone() {
        let mut req = vec![0xAAu8; FB_GET_INFO_V2_PARAMS_SIZE];
        req[0..4].copy_from_slice(&1u32.to_le_bytes());
        req[4..8].copy_from_slice(&FB_INFO_INDEX_BUS_WIDTH.to_le_bytes());
        req[8..12].copy_from_slice(&0u32.to_le_bytes());
        let out = answer_fb_get_info_v2(&req, &GA106.forwarded_answers().unwrap()).unwrap();
        assert_eq!(
            &out[12..],
            &req[12..],
            "the tail must arrive back untouched"
        );
        assert_eq!(&out[8..12], &0xc0u32.to_le_bytes());
    }
}
