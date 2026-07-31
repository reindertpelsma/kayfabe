//! The `GSP_RM_CONTROL` reply body for
//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP` — the control that asks the
//! GSP **which of BAR0's registers unprivileged guest userspace is allowed to touch**.
//!
//! ## Why the guest cannot start without it
//!
//! `gpuConstructUserRegisterAccessMap_IMPL` allocates the params, zeroes them, and issues
//! this control under `NV_ASSERT_OK_OR_GOTO`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_register_access_map.c:238-254`). A refusal is
//! a `LEVEL_ERROR` at `:244`, propagates out of `gpuPreInit` at `gpu.c:2125` and ends
//! `RmInitNvDevice` — `[measured]` run `t132a`, a stock 580.159.04 guest at `f83ce31`:
//!
//! ```text
//! NVRM: [NV_ERR_NOT_SUPPORTED] (0x56) returned from pRmApi->Control(…,
//!       NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP, pParams, sizeof(*pParams))
//!       @ gpu_register_access_map.c:244
//! NVRM: … returned from gpuConstructUserRegisterAccessMap(pGpu) @ gpu.c:2125
//! NVRM: RmInitNvDevice: *** Cannot pre-initialize the device
//! NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x23:0x56:1206)
//! ```
//!
//! ★★ It is **served**, not inert. `kayfabe_device::inert`'s eligibility rule is *"the
//! guest reads nothing but the status"*, and this caller reads **four** `[OUT]` fields and
//! branches on every one of them (`:249-253`, then `:260-320`):
//! `userRegisterAccessMapSize` decides whether a map exists at all, `compressedSize`
//! decides *allow-everything* versus *inflate*, `compressedData` is inflated into a
//! 512 KiB bitmap, and `profilingRanges`/`profilingRangesSize` are then subtracted from it.
//!
//! ⚠ The distinction is not academic even though this port serves zeros. `pParams` is
//! `portMemSet` to zero **before** the call (`:242`), so an inert reply and the zeros below
//! would reach RM as the same bytes — and would be the same *statement* only by accident.
//! The zeros here are a claim this module names, guards and can be argued with; an inert
//! reply would be the absence of one.
//!
//! ## ★★★ What a bitmap actually is — a policy, not a description
//!
//! One bit per 32-bit register of BAR0, set = *userspace may `NV2080_CTRL_CMD_GPU_EXEC_REG_OPS`
//! this offset*. The single reader is `gpuValidateRegOffset_IMPL`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_access.c:1632-1638`), reached from
//! `subdevice_ctrl_gpu_regops.c:84`; a clear bit is `NV_ERR_INSUFFICIENT_PERMISSIONS` at
//! that client's own call.
//!
//! So whatever is served here, the guest **acts on**: a set bit is a promise that reading
//! that offset returns something meaningful. This device's register plane decodes a listed
//! set of offsets and answers **zero** everywhere else (`kayfabe_device::plane`), so a set
//! bit over an offset we do not model hands a client a plausible wrong answer with no
//! refusal anywhere — the same argument that kept `NV_REG_BASE_TIMER` out of
//! [`crate::chipinfo`]'s reg-base table, arrived at one layer down.
//!
//! ## ★★★ `userRegisterAccessMapSize == 0` is RM's own vocabulary for "no map"
//!
//! This is the property that makes it possible to answer honestly without publishing a
//! policy we cannot back. RM aligns the size up, and then:
//!
//! ```text
//! if (pGpu->userRegisterAccessMapSize == 0)
//! {
//!     NV_PRINTF(LEVEL_INFO, "User Register Access Map unsupported for this chip.\n");
//!     status = NV_OK;
//!     goto done;
//! }
//! ```
//! (`ogkm-580: gpu_register_access_map.c:260-267`)
//!
//! `done:` with `status == NV_OK` frees only the params, so the control **succeeds** and
//! `pGpu->pUserRegisterAccessMap` stays `NULL`. ⊘ That is not a zero-shaped lie of the
//! #125 kind: it is a first-class arm of the guest's own code with its own message, exactly
//! as [`crate::chipinfo::REG_BASE_UNSUPPORTED`] is for a register base.
//!
//! ★★ And it is **fail-closed**, which is the direction a permission answer should default:
//! with the map `NULL` and the GPU fully constructed,
//! `gpuGetUserRegisterAccessPermissions_IMPL` returns `NV_FALSE` (`:141-152`), so every
//! non-administrator regop is refused at its own call site rather than silently served a
//! zero.
//!
//! ## ⚠ The one combination that must never be encoded
//!
//! ```text
//! if (!pGpu->bUseRegisterAccessMap || compressedSize == 0)
//!     portMemSet(pGpu->pUserRegisterAccessMap, 0xFF, pGpu->userRegisterAccessMapSize);
//! ```
//! (`ogkm-580: gpu_register_access_map.c:286-291`; `bUseRegisterAccessMap` is
//! unconditionally `NV_TRUE` in the open tree's constructor,
//! `ogkm-580: src/nvidia/generated/g_gpu_nvoc.c:584`)
//!
//! A **non-zero size with an empty bitmap opens every register in BAR0 to unprivileged
//! guest userspace** — the exact opposite of the zero-size answer, and one constant away
//! from it. [`RegisterAccessMapError::MapPublishedWithoutBitmap`] is that combination made
//! unencodable.
//!
//! ## ★★ What the oracle settled, and what it did not
//!
//! `C: src/qemu/mode2_initctrl_ga106.h:3363` (`ctl_20800a41`, registered at `:6249` as
//! `{0x20800a41u, 0x0u, 8204u, 8200u, ctl_20800a41}` — `psize` 8204, captured `dlen` 8200,
//! the four trailing zeros trimmed) reconstructs **verbatim** out of
//! `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6`: records **142000-142002**,
//! three consecutive 4096-byte `GUEST_WR`s starting at gpa `0x1_2764_7000`, the params at
//! payload offset **120** of the first, under an RPC envelope reading
//! `header_version=0x03000000 signature="VRPC" length=8276 function=76 rpc_result=NV_OK
//! sequence=0` and a control header carrying `hClient=0xc2000006 hObject=0xabcd2080
//! cmd=0x20800a41 status=NV_OK paramsSize=8204 rmapiRpcFlags=0`.
//! `8276 = 32 (envelope) + 40 (`RpcControlReq::HEADER`) + 8204 (params)`, the same
//! arithmetic that made the layouts agree for [`crate::pcibars`] and [`crate::chipinfo`].
//!
//! ★ **It is also the first reply this port encodes that does not fit one message-queue
//! element.** `48 + 8276` is three of 580's 4096-byte elements, which is why the C had to
//! learn to skip continuation elements at all
//! (`C: src/qemu/nvkvm_gpu_emul.c:3545-3554`, which names this very command).
//!
//! What those bytes settle is the **layout**: `compressedData` starts at 8 and
//! `profilingRangesSize` at 4104, with no padding anywhere, and the whole struct is 8204.
//! `crates/kayfabe-device/tests/user_register_access_map.rs` pins this encoder against
//! them.
//!
//! ⊘ They settle **nothing about which values to serve**. The oracle's board reported
//! `userRegisterAccessMapSize = 0x80000` and a 1269-byte gzip stream that inflates to
//! 524 288 bytes naming **6809 permitted ranges, 37 743 registers**, `0x9400` among them —
//! the PTIMER block [`kayfabe_device::ga10x`] explicitly refuses to advertise because this
//! device does not serve it. Replaying that map would hand guest userspace a permitted read
//! of a stopped clock, 6809 times over.
//!
//! ## ⊘ What the zero answer costs, stated rather than discovered
//!
//! - `pGpu->bRmProfilingPrivileged` is left at its zero-initialised `NV_FALSE` instead of
//!   being computed at `:309`. `[inferred]` from `ogkm-580`: its **only** reader is
//!   `fecs_event_list.c:1197`, inside `if (IS_VIRTUAL(pGpu) || hypervisorIsVgxHyper())`
//!   (`:1194`). This guest is neither — it runs the bare-metal RM against an emulated
//!   device — so the field has no reader on this port's path. If a vGPU posture is ever
//!   taken, it acquires one.
//! - `mem_mgr_gm107.c:1932-1937` adds `pGpu->userRegisterAccessMapSize` to `rsvdSlowSize`,
//!   so the guest's framebuffer reservation estimate is 512 KiB smaller than the oracle's
//!   board's. It is an estimate that is subsequently capped anyway (`:1939`), and nothing
//!   indexes it.
//! - `[inferred]`, and the one to watch: **no closed userspace client is greppable.** The
//!   argument above that a refused regop surfaces as a named
//!   `NV_ERR_INSUFFICIENT_PERMISSIONS` at the client's own call is a read of the two open
//!   trees. `osIsAdministrator()` short-circuits the check entirely
//!   (`ogkm-580: gpu_access.c:1632`), so a root `nvidia-smi` cannot exercise it at all.

/// `NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:772`).
pub const NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP: u32 = 0x2080_0a41;

/// `NV2080_CTRL_INTERNAL_GPU_USER_REGISTER_ACCESS_MAP_MAX_COMPRESSED_SIZE`
/// (`ogkm-580: ctrl2080internal.h:774`) — the fixed extent of `compressedData[]`.
pub const MAX_COMPRESSED_SIZE: usize = 4096;

/// `NV2080_CTRL_INTERNAL_GPU_USER_REGISTER_ACCESS_MAP_MAX_PROFILING_RANGES`
/// (`ogkm-580: ctrl2080internal.h:775`) — the fixed extent of `profilingRanges[]`, in
/// **bytes**, not entries.
pub const MAX_PROFILING_RANGES: usize = 4096;

/// Byte offset of `userRegisterAccessMapSize`.
pub const MAP_SIZE_OFF: usize = 0;

/// Byte offset of `compressedSize`.
pub const COMPRESSED_SIZE_OFF: usize = 4;

/// Byte offset of `compressedData[0]`.
pub const COMPRESSED_DATA_OFF: usize = 8;

/// Byte offset of `profilingRangesSize`.
///
/// ★ `compressedData[]` is `NvU8[4096]` and what follows is an `NvU32`, and 4104 is already
/// 4-aligned, so **no padding is inserted** — the field sits immediately after the array.
/// The oracle's bytes are the witness: `profilingRangesSize` reads 0 there and every byte
/// of `compressedData` past the 1269-byte gzip stream is zero, so an off-by-padding
/// encoding would be indistinguishable at that one capture and wrong on any board that
/// published ranges.
pub const PROFILING_RANGES_SIZE_OFF: usize = 4104;

/// Byte offset of `profilingRanges[0]`.
pub const PROFILING_RANGES_OFF: usize = 4108;

/// `sizeof(NV2080_CTRL_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP_PARAMS)` — the `[OUT]`
/// struct RM allocates (`ogkm-580: gpu_register_access_map.c:240`) and therefore the
/// `paramsSize` it declares and the number of bytes it copies back.
pub const USER_REGISTER_ACCESS_MAP_PARAMS_SIZE: usize = 8204;

/// How many bytes of gzip framing `gpuInitRegisterAccessMap_IMPL` skips before handing the
/// buffer to RM's own inflate: `pComprData += 10`, with the comment *"Strip off gzlib
/// 10-byte header — XXX this really belongs in the RM GZ library"*
/// (`ogkm-580: gpu_register_access_map.c:359-365`).
///
/// ★ Unconditional. RM never checks the magic, so a bitmap that is not gzip-framed is not
/// rejected — it is inflated from ten bytes into itself. See
/// [`RegisterAccessMapError::BitmapNotGzipFramed`].
pub const GZIP_HEADER_SKIP: usize = 10;

/// The first three bytes of a gzip member: `ID1 ID2 CM`, with `CM = 8` (deflate).
const GZIP_MAGIC: [u8; 3] = [0x1f, 0x8b, 0x08];

/// One `(offset, size)` pair of `profilingRanges[]`, in the units
/// `gpuSetUserRegisterAccessPermissionsInBulk_IMPL` reads them in
/// (`ogkm-580: gpu_register_access_map.c:105-126`): a **byte** register offset and a
/// **byte** length, both of which that function requires to be 4-aligned.
///
/// ⚠ Every range here is *subtracted* from the published bitmap — it is the set of
/// registers that profiling tools may not reach without admin. A range is therefore only
/// meaningful alongside a published map, and [`encode_user_register_access_map`] refuses
/// one without.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfilingRange {
    /// Byte offset of the first register of the range, within BAR0.
    pub offset: u32,
    /// Length of the range in bytes.
    pub size: u32,
}

/// ★★ **What this device tells the guest unprivileged userspace may touch.**
///
/// A chip row, not a logic-crate constant, for the same reason every other geometry is one:
/// the bitmap is one bit per register of *this chip's* BAR0, and a second generation is a
/// second row.
///
/// See [`Self::NOT_PUBLISHED`] for the answer this port actually serves, and this module's
/// docs for why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterAccessMapRow {
    /// `userRegisterAccessMapSize` — the size of the **inflated** bitmap in bytes, i.e.
    /// one bit per 32-bit register of BAR0.
    ///
    /// Zero is RM's own *"unsupported for this chip"* (`ogkm-580: :261-267`), and is not a
    /// missing value: see this module's docs.
    pub map_size_bytes: u32,
    /// `compressedData` — the gzip-framed deflate stream that inflates to exactly
    /// [`Self::map_size_bytes`] bytes.
    ///
    /// ⊘ Empty is **not** "deny everything". Empty with a non-zero size is
    /// *allow everything*; see [`RegisterAccessMapError::MapPublishedWithoutBitmap`].
    pub compressed: &'static [u8],
    /// `profilingRanges` — ranges removed from the published bitmap for non-admin clients.
    pub profiling_ranges: &'static [ProfilingRange],
}

impl RegisterAccessMapRow {
    /// ★★★ **This device publishes no user register access map.**
    ///
    /// The answer RM spells `userRegisterAccessMapSize == 0` and logs as *"User Register
    /// Access Map unsupported for this chip"* (`ogkm-580: gpu_register_access_map.c:263`),
    /// returning `NV_OK`. See this module's docs: it advertises nothing, it is fail-closed
    /// for regops, and it is the only answer this port can back with the registers it
    /// actually serves.
    pub const NOT_PUBLISHED: RegisterAccessMapRow = RegisterAccessMapRow {
        map_size_bytes: 0,
        compressed: &[],
        profiling_ranges: &[],
    };

    /// Whether this row publishes a bitmap at all.
    #[must_use]
    pub fn publishes_map(&self) -> bool {
        self.map_size_bytes != 0
    }
}

/// Why the reply could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterAccessMapError {
    /// ★★★ **A map size with no bitmap behind it, which RM reads as *allow every
    /// register*.**
    ///
    /// `compressedSize == 0` sends `gpuConstructUserRegisterAccessMap_IMPL` down the
    /// `portMemSet(…, 0xFF, …)` arm (`ogkm-580: gpu_register_access_map.c:286-291`),
    /// opening the whole of BAR0 to unprivileged guest regops. It is one constant away from
    /// [`RegisterAccessMapRow::NOT_PUBLISHED`] and means the opposite of it, and nothing
    /// downstream would log.
    MapPublishedWithoutBitmap {
        /// The size the row declared.
        map_size: u32,
    },
    /// A bitmap with no map size. RM would take the `:261` arm and never inflate it, so the
    /// row states a policy that is silently not applied.
    BitmapWithoutMap {
        /// How many bytes of bitmap the row carried.
        compressed_len: usize,
    },
    /// A map size that is not a multiple of four.
    ///
    /// ★ RM `NV_ALIGN_UP`s it before allocating (`ogkm-580: :260`) but the inflate is
    /// asked for the *rounded* length (`:295-296`), so an unaligned row would describe a
    /// bitmap of one size and be inflated at another — and `gpuInitRegisterAccessMap_IMPL`
    /// fails the whole control when the inflated length disagrees (`:373-380`).
    MapSizeNotDwordAligned {
        /// The size the row declared.
        map_size: u32,
    },
    /// The bitmap does not fit `compressedData[]` ([`MAX_COMPRESSED_SIZE`]).
    BitmapTooLarge {
        /// How many bytes the row carried.
        len: usize,
        /// The array bound.
        max: usize,
    },
    /// ★ The bitmap is not gzip-framed.
    ///
    /// `gpuInitRegisterAccessMap_IMPL` skips [`GZIP_HEADER_SKIP`] bytes unconditionally and
    /// never checks the magic (`ogkm-580: :359-365`), so a raw deflate stream is not
    /// rejected — it is inflated from ten bytes into itself and produces a bitmap that is
    /// wrong rather than absent.
    BitmapNotGzipFramed,
    /// The profiling ranges do not fit `profilingRanges[]` ([`MAX_PROFILING_RANGES`]
    /// bytes, i.e. 512 pairs).
    ProfilingRangesTooLarge {
        /// How many bytes the pairs would occupy.
        bytes: usize,
        /// The array bound, in bytes.
        max: usize,
    },
    /// Profiling ranges without a published map. They are subtracted from a bitmap that
    /// does not exist, so RM never reaches them (`ogkm-580: :310-320` is past the `:261`
    /// early return) and the row overstates what this device restricts.
    ProfilingRangeWithoutMap {
        /// How many ranges the row carried.
        count: usize,
    },
    /// A range whose offset or size is not 4-aligned.
    ///
    /// ★★ Not cosmetic: `gpuSetUserRegisterAccessPermissions_IMPL` `NV_ASSERT_OR_RETURN`s
    /// both (`ogkm-580: :51-52`), which fails the bulk call, which fails
    /// `gpuConstructUserRegisterAccessMap` at `:313-319` — so one misaligned range does not
    /// lose its own range, it **ends `RmInitNvDevice`**, at this line, again.
    ProfilingRangeUnaligned {
        /// The offending offset.
        offset: u32,
        /// The offending size.
        size: u32,
    },
    /// A range that runs past the end of the published bitmap.
    ///
    /// Same consequence as [`Self::ProfilingRangeUnaligned`]: `bitOffset + bitSize` above
    /// `mapSize * 8` is `NV_ERR_INVALID_ARGUMENT` at `ogkm-580: :64-65`, and that failure
    /// reaches the caller of this control.
    ProfilingRangeOutsideMap {
        /// The offending offset.
        offset: u32,
        /// The offending size.
        size: u32,
        /// The published map's size in bytes.
        map_size: u32,
    },
}

impl core::fmt::Display for RegisterAccessMapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MapPublishedWithoutBitmap { map_size } => write!(
                f,
                "a {map_size}-byte register access map with no bitmap, which RM reads as \
                 \"allow every register in BAR0 to unprivileged userspace\""
            ),
            Self::BitmapWithoutMap { compressed_len } => write!(
                f,
                "{compressed_len} bytes of register access bitmap with a zero map size, \
                 which RM never inflates"
            ),
            Self::MapSizeNotDwordAligned { map_size } => write!(
                f,
                "register access map size {map_size} is not a multiple of 4; RM rounds it \
                 up and then inflates to the rounded length"
            ),
            Self::BitmapTooLarge { len, max } => write!(
                f,
                "register access bitmap is {len} bytes; compressedData[] holds {max}"
            ),
            Self::BitmapNotGzipFramed => write!(
                f,
                "register access bitmap is not gzip-framed; RM skips {GZIP_HEADER_SKIP} \
                 bytes of framing without checking for it"
            ),
            Self::ProfilingRangesTooLarge { bytes, max } => write!(
                f,
                "profiling ranges occupy {bytes} bytes; profilingRanges[] holds {max}"
            ),
            Self::ProfilingRangeWithoutMap { count } => write!(
                f,
                "{count} profiling ranges with no published register access map to subtract \
                 them from"
            ),
            Self::ProfilingRangeUnaligned { offset, size } => write!(
                f,
                "profiling range {offset:#x}+{size:#x} is not 4-aligned; RM fails the whole \
                 control on it"
            ),
            Self::ProfilingRangeOutsideMap {
                offset,
                size,
                map_size,
            } => write!(
                f,
                "profiling range {offset:#x}+{size:#x} runs past the {map_size}-byte \
                 register access map; RM fails the whole control on it"
            ),
        }
    }
}

impl core::error::Error for RegisterAccessMapError {}

/// Encode `NV2080_CTRL_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP_PARAMS` from a chip's row.
///
/// The whole [`USER_REGISTER_ACCESS_MAP_PARAMS_SIZE`]-byte struct is returned, zero-filled
/// past each declared extent — which for [`RegisterAccessMapRow::NOT_PUBLISHED`] is the
/// entire struct, and is that row's whole statement.
///
/// # Errors
///
/// Every variant of [`RegisterAccessMapError`]; see each for the guest-side consequence it
/// stands in front of.
pub fn encode_user_register_access_map(
    row: &RegisterAccessMapRow,
) -> Result<Vec<u8>, RegisterAccessMapError> {
    if !row.map_size_bytes.is_multiple_of(4) {
        return Err(RegisterAccessMapError::MapSizeNotDwordAligned {
            map_size: row.map_size_bytes,
        });
    }
    if row.publishes_map() && row.compressed.is_empty() {
        return Err(RegisterAccessMapError::MapPublishedWithoutBitmap {
            map_size: row.map_size_bytes,
        });
    }
    if !row.publishes_map() && !row.compressed.is_empty() {
        return Err(RegisterAccessMapError::BitmapWithoutMap {
            compressed_len: row.compressed.len(),
        });
    }
    if row.compressed.len() > MAX_COMPRESSED_SIZE {
        return Err(RegisterAccessMapError::BitmapTooLarge {
            len: row.compressed.len(),
            max: MAX_COMPRESSED_SIZE,
        });
    }
    if !row.compressed.is_empty()
        && (row.compressed.len() < GZIP_HEADER_SKIP || row.compressed[..3] != GZIP_MAGIC)
    {
        return Err(RegisterAccessMapError::BitmapNotGzipFramed);
    }

    if !row.profiling_ranges.is_empty() && !row.publishes_map() {
        return Err(RegisterAccessMapError::ProfilingRangeWithoutMap {
            count: row.profiling_ranges.len(),
        });
    }
    let ranges_bytes = row.profiling_ranges.len() * 8;
    if ranges_bytes > MAX_PROFILING_RANGES {
        return Err(RegisterAccessMapError::ProfilingRangesTooLarge {
            bytes: ranges_bytes,
            max: MAX_PROFILING_RANGES,
        });
    }
    // The bitmap holds one bit per dword, so its reach in bytes of register space is
    // `map_size_bytes * 8 * 4` — the same product `gpuSetUserRegisterAccessPermissions`
    // bounds against, expressed in bit units there (`ogkm-580: :46, :64-65`).
    let map_reach = u64::from(row.map_size_bytes) * 32;
    for r in row.profiling_ranges {
        if !r.offset.is_multiple_of(4) || !r.size.is_multiple_of(4) {
            return Err(RegisterAccessMapError::ProfilingRangeUnaligned {
                offset: r.offset,
                size: r.size,
            });
        }
        if u64::from(r.offset) + u64::from(r.size) > map_reach {
            return Err(RegisterAccessMapError::ProfilingRangeOutsideMap {
                offset: r.offset,
                size: r.size,
                map_size: row.map_size_bytes,
            });
        }
    }

    let mut params = vec![0u8; USER_REGISTER_ACCESS_MAP_PARAMS_SIZE];
    params[MAP_SIZE_OFF..MAP_SIZE_OFF + 4].copy_from_slice(&row.map_size_bytes.to_le_bytes());
    let compressed_size = u32::try_from(row.compressed.len()).unwrap_or(u32::MAX);
    params[COMPRESSED_SIZE_OFF..COMPRESSED_SIZE_OFF + 4]
        .copy_from_slice(&compressed_size.to_le_bytes());
    params[COMPRESSED_DATA_OFF..COMPRESSED_DATA_OFF + row.compressed.len()]
        .copy_from_slice(row.compressed);

    let ranges_size = u32::try_from(ranges_bytes).unwrap_or(u32::MAX);
    params[PROFILING_RANGES_SIZE_OFF..PROFILING_RANGES_SIZE_OFF + 4]
        .copy_from_slice(&ranges_size.to_le_bytes());
    for (i, r) in row.profiling_ranges.iter().enumerate() {
        let o = PROFILING_RANGES_OFF + i * 8;
        params[o..o + 4].copy_from_slice(&r.offset.to_le_bytes());
        params[o + 4..o + 8].copy_from_slice(&r.size.to_le_bytes());
    }

    Ok(params)
}
