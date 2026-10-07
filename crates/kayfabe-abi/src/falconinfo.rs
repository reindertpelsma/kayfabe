//! ★★★ **The engine inventory** — `NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO`
//! (`0x208001b0`), the sixth rung, and the first control this port answers by naming
//! *nothing*.
//!
//! ## The guest cannot start without it
//!
//! `[measured]` run `t133a`, a stock 580.159.04 guest at `c88f803`, the boot after the
//! register access map was served:
//!
//! ```text
//! NVRM: nvAssertOkFailedNoLog: Assertion failed: Call not supported
//!       [NV_ERR_NOT_SUPPORTED] (0x56) returned from pRmApi->Control(…,
//!       NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO, …) @ gpu.c:5368
//! NVRM: … returned from gpuBuildGenericKernelFalconList(pGpu) @ gpu.c:2126
//! NVRM: RmInitNvDevice: *** Cannot pre-initialize the device
//! NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x23:0x56:1206)
//! ```
//!
//! ## ★★ It is asked TWICE, by two different statements, and the second is weaker
//!
//! `gpuPreInit` calls two list builders back to back, and **each issues its own
//! independent control**:
//!
//! - `gpu.c:2126` → `gpuBuildGenericKernelFalconList` (`ogkm-580: gpu.c:5344-5422`)
//! - `gpu.c:2128` → `gpuBuildKernelVideoEngineList`  (`ogkm-580: gpu.c:5435-5503`)
//!
//! `[measured]` the oracle shows exactly that: `cap1b` records `0x208001b0` at
//! `rpc.sequence` **5** and **6**, back to back, with byte-identical 1284-byte replies
//! (records 142009 and 142016 of `traces/cap1b_coldboot_hermetic_d6.rec`). So one row here
//! answers two adjacent statements of `gpuPreInit`, and nothing forces a GSP to answer the
//! two consistently — see [`FalconInfoError::TooManyFalcons`].
//!
//! ## ★★★ Why this device names NO falcon
//!
//! An entry in this table is not a description; it is an **instruction to construct**.
//! Each one RM accepts becomes a `GenericKernelFalcon` whose every register access is
//!
//! ```c
//! return REG_INST_DEVIDX_RD32_EX(pGpu, DEVICE_INDEX_GPU, 0,
//!            pKernelFlcn->registerBase + offset, NULL);
//! ```
//!
//! (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/arch/turing/kernel_falcon_tu102.c:45-51`)
//! — `registerBase` straight into BAR0, unvalidated, and the object is registered as an
//! `IntrService` (`ogkm-580: gpu.c:5559-5568`) so those reads are reachable from the
//! interrupt path.
//!
//! `[measured]`, in this tree, by
//! `kayfabe-device/tests/constructed_falcon_info.rs`: of the eight bases the oracle's GA106
//! reports, **seven have no model at all** — `kayfabe_arch::GspModel::decode_reg` answers
//! `None` for every dword of the published falcon block — and the eighth, `NV_PSEC`,
//! decodes exactly **three**: `+0x040` mailbox0, `+0x100` cpuctl, `+0x118` dmatrfcmd, the
//! FWSEC secure-booter handshake. `_HWCFG2`, `_IRQSTAT`, `_IRQMASK` and `_IRQDEST` — the
//! registers an `IntrService`-registered falcon reads first — are modelled **nowhere**.
//! `RegPlane` answers an unclaimed register with a **defaulted zero**, which its own docs
//! call *"a wrong value rather than a fabricated mapping"*
//! (`kayfabe-device/src/plane.rs:49-56`). So naming these eight would hand RM eight
//! microcontrollers reporting an all-zero state.
//!
//! The eight, resolved from `engDesc = (classId << 8) | inst`
//! (`ogkm-580: g_eng_desc_nvoc.h:56-60`) against the NVOC class ids:
//!
//! | `engDesc` | class | engine | `registerBase` | `ctxBufferSize` |
//! |---|---|---|---|---|
//! | `0x8a20bf00` | `OBJFBFLCN` | FB falcon | `0x009a4000` | `0x1000` |
//! | `0x5ee8dc00` | `FECS`      | GR front-end ctxsw | `0x00409000` | `0x1000` |
//! | `0x4781e800` | `GPCCS`     | GPC ctxsw | `0x0041a000` | `0x1000` |
//! | `0x8f99e100` | `OBJBSP`    | NVDEC0 | `0x00848000` | `0x1000` |
//! | `0xf3d72200` | `Pmu`       | PMU | `0x0010a000` | `0x1000` |
//! | `0xe97b6c00` | `OBJMSENC`  | NVENC0 | `0x001c8000` | `0x1000` |
//! | `0x28c40800` | `OBJSEC2`   | SEC2 | `0x00840000` | `0x10000` |
//! | `0xdd7bab00` | `OBJOFA`    | OFA | `0x00844000` | `0x1000` |
//!
//! ⚠ The three-register exception is why this argument is *"none is modelled as a
//! falcon"* rather than *"none is modelled"*: an earlier draft of it claimed the stronger
//! thing and its own test went red on `NV_PSEC`. And RM *skips* the `ENG_SEC2` row outright
//! when a `KernelSec2` exists (`ogkm-580: gpu.c:5384-5390`), so even the oracle's own list
//! is conditional.
//!
//! ## ★★ An empty inventory is a supported answer, not a degraded one
//!
//! `[inferred]` from the guest's own source, and the reason this is a *statement* rather
//! than a shrug: `gpuBuildGenericKernelFalconList` with `numConstructedFalcons == 0` runs
//! its loop zero times, sets `numGenericKernelFalcons = 0` and returns `NV_OK`. Both
//! consumers — `gpuGetGenericKernelFalconForEngine_IMPL` (`ogkm-580: gpu.c:5542-5557`) and
//! `gpuRegisterGenericKernelFalconIntrService_IMPL` (`:5559-5568`) — iterate to that count,
//! so both become no-ops. **No boot-time step requires any particular falcon.**
//!
//! The cost is real, deferred, and *named at the right place*: a later guest allocating a
//! SEC2/NVDEC/NVENC/NVJPG/OFA engine-class channel object gets `NV_ERR_INVALID_CLASS` from
//! `channel_descendant.c:221-234`, which logs `"engine is missing for class 0x%x"`. That is
//! a refusal a client can see, at the call that wanted the engine — which is the trade this
//! port takes over a wrong register value now.
//!
//! ## ⊘ Not [`inert`](../../kayfabe_device/inert/index.html)-eligible, and the bytes cannot tell you that
//!
//! Inert's rule is *"the guest reads nothing but the status"*. RM reads
//! `numConstructedFalcons` and **branches on it twice** — once through
//! `NV_ASSERT_TRUE_OR_GOTO` (`ogkm-580: gpu.c:5375-5377`) and once as the loop bound.
//!
//! ★ And the caller zeroes its params first — `portMemSet(pParams, 0, sizeof(*pParams))`
//! (`ogkm-580: gpu.c:5366`) — so an inert reply and this row's served reply reach RM as
//! **the same 1284 bytes**. They would be the same *statement* only by accident. What makes
//! the served zero a statement is that [`FalconInventoryRow`] can express a non-empty
//! inventory and this chip's row chooses not to; an inert reply cannot express anything.
//!
//! ## Provenance — what the capture settles, and what it does not
//!
//! `[measured]` the C's `ctl_208001b0` (`C: src/qemu/mode2_initctrl_ga106.h:3878`) is
//! byte-identical to the reply at record **142009** of
//! `traces/cap1b_coldboot_hermetic_d6.rec` (and again at 142016), at payload offset 120 —
//! the C's own `resp + 120`. The C's array is 1280 bytes against a declared
//! `paramsSize = 1284`; `[measured]` the four bytes it truncates are zero in the trace, so
//! the C's `memset`-then-`memcpy` replay is faithful.
//!
//! That settles **layout**: `rpc.length = 1356 = 32 + 40 + 1284`, a 4-byte count then 64
//! 20-byte entries. ⊘ It settles nothing about **which** falcons to name — see above, where
//! this port declines the oracle's eight.
//!
//! ## ★★★ The boot, measured
//!
//! `[measured]` run `t134a`, a stock 580.159.04 guest at `1c79474`, one `nvidia-smi` on a
//! fresh boot. The empty inventory is **accepted**: `gpuBuildGenericKernelFalconList @
//! gpu.c:5368, 2126` is gone from the guest's log, `RmInitAdapter failed! (0x23:0x56:1206)`
//! with it, and the boot leaves `gpuPreInit` entirely — its dmesg reaches
//! `gpuStatePreInit_IMPL @ gpu.c:2204` and then `gpuStateInit_IMPL`. Host-side, `commands`
//! went 8 → 27 and distinct unserviced 1 → 6.
//!
//! ★★ So a guest that constructs **no** generic kernel falcon proceeds: nothing between
//! `:2126` and `gpuStateInit` asked for one, which is the boot-time half of the argument
//! above turned from `[inferred]` into a measurement. ⊘ The *runtime* half — that a client
//! allocating a falcon-backed engine class gets `NV_ERR_INVALID_CLASS` — is still
//! `[inferred]`: no boot has reached a channel allocation.

/// `NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h:4472`).
pub const NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO: u32 = 0x2080_01b0;

/// `NV2080_CTRL_GPU_MAX_CONSTRUCTED_FALCONS = 0x40` (`ogkm-580: ctrl2080gpu.h:4470`) — the
/// extent of `constructedFalconsTable[]`, and therefore the largest count that can be read
/// back without leaving the struct.
///
/// ⚠ This is **not** the bound RM checks. See [`FalconInfoError::TooManyFalcons`].
pub const MAX_CONSTRUCTED_FALCONS: usize = 0x40;

/// `NV_ARRAY_ELEMENTS(pGpu->genericKernelFalcons)` — the **destination** array's extent
/// (`ogkm-580: src/nvidia/generated/g_gpu_nvoc.h:1378`, `GenericKernelFalcon *[71]`).
///
/// ⚠ Present only so [`FalconInfoError::TooManyFalcons`] can name the gap it stands in
/// front of. Nothing encodes against it.
pub const RM_DESTINATION_FALCON_SLOTS: usize = 71;

/// Byte offset of `numConstructedFalcons`.
pub const NUM_CONSTRUCTED_FALCONS_OFF: usize = 0;

/// Byte offset of `constructedFalconsTable[0]`.
pub const CONSTRUCTED_FALCONS_TABLE_OFF: usize = 4;

/// `sizeof(NV2080_CTRL_GPU_CONSTRUCTED_FALCON_INFO)` — five `NvU32`, no padding
/// (`ogkm-580: ctrl2080gpu.h:4463-4469`).
pub const CONSTRUCTED_FALCON_STRIDE: usize = 20;

/// Byte offset of `engDesc` within one entry.
pub const ENG_DESC_OFF: usize = 0;
/// Byte offset of `ctxAttr` within one entry.
pub const CTX_ATTR_OFF: usize = 4;
/// Byte offset of `ctxBufferSize` within one entry.
pub const CTX_BUFFER_SIZE_OFF: usize = 8;
/// Byte offset of `addrSpaceList` within one entry.
pub const ADDR_SPACE_LIST_OFF: usize = 12;
/// Byte offset of `registerBase` within one entry.
pub const REGISTER_BASE_OFF: usize = 16;

/// `sizeof(NV2080_CTRL_GPU_GET_CONSTRUCTED_FALCON_INFO_PARAMS)` — the `[OUT]` struct RM
/// allocates (`ogkm-580: gpu.c:5362`) and therefore the `paramsSize` it declares.
///
/// `[measured]` in `cap1b` (record 142009) the oracle declares exactly this:
/// `psize = 1284`, `rpc.length = 1356 = 32 + 40 + 1284`.
pub const FALCON_INFO_PARAMS_SIZE: usize =
    CONSTRUCTED_FALCONS_TABLE_OFF + MAX_CONSTRUCTED_FALCONS * CONSTRUCTED_FALCON_STRIDE;

/// The BAR0 extent one falcon's register block occupies.
///
/// The published `NV_PFALCON_FALCON_*` registers span `0x000..=0x130`
/// (`ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_falcon_v4.h`), but the block
/// is placed on a 4 KiB boundary — `[measured]`, every one of the eight bases the oracle
/// reports is `0x1000`-aligned. So this is the **granularity** a base is checked at, not a
/// claim about how many registers exist.
pub const FALCON_REGISTER_BLOCK_LEN: u64 = 0x1000;

/// The largest engine-context buffer this port is willing to ask a guest kernel to
/// allocate.
///
/// ⚠ **A policy bound, not a driver limit.** RM enforces none: `ctxBufferSize` reaches
/// `memdescCreate(…, pKernelFalcon->ctxBufferSize, …)` unvalidated
/// (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/kernel_falcon.c:144-155`), so the value a
/// GSP puts here becomes a guest-kernel allocation size at the moment a client allocates a
/// matching engine-class object. `[measured]` in `cap1b` the largest the oracle's silicon
/// reports is `0x10000` (SEC2); this is sixteen times that.
pub const MAX_CTX_BUFFER_SIZE: u32 = 0x10_0000;

/// One row of `constructedFalconsTable[]` — an engine this device is prepared to have RM
/// construct a `GenericKernelFalcon` for.
///
/// ⚠ Every field is an instruction, not a description. See this module's docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstructedFalcon {
    /// `engDesc` — `(classId << 8) | inst` (`ogkm-580: g_eng_desc_nvoc.h:56-60`). Becomes
    /// `physEngDesc`, the key `gpuGetGenericKernelFalconForEngine` looks up.
    pub eng_desc: u32,
    /// `ctxAttr` — the `memdescCreate` attribute word for the engine context buffer.
    pub ctx_attr: u32,
    /// `ctxBufferSize` — the engine context buffer's size in bytes. Bounded by
    /// [`MAX_CTX_BUFFER_SIZE`].
    pub ctx_buffer_size: u32,
    /// `addrSpaceList` — index into RM's `ADDRLIST` array, deciding which address spaces
    /// the context buffer may land in.
    pub addr_space_list: u32,
    /// `registerBase` — the falcon's BAR0 register block base. Unvalidated by RM; bounded
    /// here by [`FalconInfoError::RegisterBaseOutsideAperture`].
    pub register_base: u32,
}

/// ★★ **Which engines this device tells the guest to construct falcons for.**
///
/// A chip row, because the inventory is a property of silicon — and, on this port, of how
/// much of that silicon's register map is actually modelled.
///
/// ★ There is deliberately **no count field**. `numConstructedFalcons` is derived from
/// [`Self::falcons`]'s length at encode time, so a reply whose declared count disagrees
/// with its table is not merely rejected — it cannot be *expressed*. That disagreement is
/// the whole of [`FalconInfoError::TooManyFalcons`]'s hazard, and half of it is closed
/// structurally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FalconInventoryRow {
    /// The engines to advertise, in table order.
    pub falcons: &'static [ConstructedFalcon],
}

impl FalconInventoryRow {
    /// ★★★ **This device constructs no generic kernel falcons.**
    ///
    /// `numConstructedFalcons = 0`, which `gpuBuildGenericKernelFalconList` accepts with
    /// `NV_OK` and an empty list. See this module's docs for why the oracle's eight are
    /// declined and what it costs.
    pub const NONE: FalconInventoryRow = FalconInventoryRow { falcons: &[] };

    /// How many falcons this row advertises.
    #[must_use]
    pub fn count(&self) -> usize {
        self.falcons.len()
    }
}

/// Why the reply could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FalconInfoError {
    /// ★★★ **A count RM's own bounds check lets through and its own loop then reads past
    /// the end of.**
    ///
    /// `gpuBuildGenericKernelFalconList` guards the loop with
    ///
    /// ```c
    /// NV_ASSERT_TRUE_OR_GOTO(status,
    ///     pParams->numConstructedFalcons <= NV_ARRAY_ELEMENTS(pGpu->genericKernelFalcons),
    ///     NV_ERR_BUFFER_TOO_SMALL, done);
    /// ```
    ///
    /// (`ogkm-580: gpu.c:5375-5377`) — the **destination** array, which is
    /// [`RM_DESTINATION_FALCON_SLOTS`] = 71. The **source** it then indexes,
    /// `constructedFalconsTable[]`, holds [`MAX_CONSTRUCTED_FALCONS`] = 64. So a count of
    /// **65..=71 passes the assert** and the loop reads up to `7 * 20 = 140` bytes past the
    /// end of the 1284-byte `portMemAllocNonPaged` buffer, constructing falcons — with
    /// their unvalidated `registerBase` — out of adjacent guest kernel heap. Counts ≥ 72
    /// are rejected; counts ≤ 64 are entirely inside the struct.
    ///
    /// ⚠ The sibling consumer is **worse**: `gpuBuildKernelVideoEngineList`
    /// (`ogkm-580: gpu.c:5435-5503`, reached from `gpu.c:2128`) has no upfront bound at
    /// all — it loops straight to `numConstructedFalcons` — so a count delivered to *its*
    /// call is an unbounded read. `[measured]` in `cap1b` the two are separate RPCs at
    /// `rpc.sequence` 5 and 6, and nothing makes a GSP answer them alike.
    ///
    /// This port therefore refuses above 64 rather than above 71: the encodable set is the
    /// set that stays inside the struct on **both** paths.
    TooManyFalcons {
        /// How many the row carried.
        count: usize,
        /// [`MAX_CONSTRUCTED_FALCONS`].
        max: usize,
    },
    /// A `registerBase` whose falcon register block does not lie inside this device's BAR0.
    ///
    /// RM validates nothing: `registerBase + offset` goes straight to
    /// `REG_INST_DEVIDX_RD32_EX` / `WR32_EX`
    /// (`ogkm-580: kernel_falcon_tu102.c:45-76`), reachable from the interrupt path. A base
    /// past the aperture is a read or write this device was never asked about.
    RegisterBaseOutsideAperture {
        /// The base the row carried.
        register_base: u32,
        /// This device's register aperture length.
        aperture_len: u64,
    },
    /// A `registerBase` that is not [`FALCON_REGISTER_BLOCK_LEN`]-aligned.
    RegisterBaseUnaligned {
        /// The base the row carried.
        register_base: u32,
    },
    /// A `ctxBufferSize` above [`MAX_CTX_BUFFER_SIZE`] — a guest-kernel allocation this
    /// port declines to request. See [`MAX_CTX_BUFFER_SIZE`]: it is a policy bound.
    ContextBufferTooLarge {
        /// The size the row carried.
        ctx_buffer_size: u32,
        /// [`MAX_CTX_BUFFER_SIZE`].
        max: u32,
    },
}

impl core::fmt::Display for FalconInfoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooManyFalcons { count, max } => write!(
                f,
                "{count} constructed falcons; constructedFalconsTable[] holds {max}, and RM \
                 bounds the count against its {RM_DESTINATION_FALCON_SLOTS}-slot destination \
                 array instead — 65..=71 reads past the params buffer"
            ),
            Self::RegisterBaseOutsideAperture {
                register_base,
                aperture_len,
            } => write!(
                f,
                "falcon registerBase {register_base:#x} + {FALCON_REGISTER_BLOCK_LEN:#x} runs \
                 past this device's {aperture_len:#x}-byte register aperture; RM bounds it \
                 nowhere"
            ),
            Self::RegisterBaseUnaligned { register_base } => write!(
                f,
                "falcon registerBase {register_base:#x} is not \
                 {FALCON_REGISTER_BLOCK_LEN:#x}-aligned"
            ),
            Self::ContextBufferTooLarge {
                ctx_buffer_size,
                max,
            } => write!(
                f,
                "falcon ctxBufferSize {ctx_buffer_size:#x} exceeds this port's {max:#x} bound; \
                 RM passes it to memdescCreate unvalidated"
            ),
        }
    }
}

impl core::error::Error for FalconInfoError {}

/// Encode `NV2080_CTRL_GPU_GET_CONSTRUCTED_FALCON_INFO_PARAMS` from a chip's row.
///
/// The whole [`FALCON_INFO_PARAMS_SIZE`]-byte struct is returned, zero past the declared
/// count — which for [`FalconInventoryRow::NONE`] is every byte of it, and is that row's
/// whole statement.
///
/// `regs_aperture_len` is this device's BAR0 extent, so a `registerBase` is checked against
/// the aperture actually being served rather than against a constant in this crate.
///
/// # Errors
///
/// Every variant of [`FalconInfoError`]; see each for the guest-side consequence it stands
/// in front of.
pub fn encode_constructed_falcon_info(
    row: &FalconInventoryRow,
    regs_aperture_len: u64,
) -> Result<Vec<u8>, FalconInfoError> {
    if row.count() > MAX_CONSTRUCTED_FALCONS {
        return Err(FalconInfoError::TooManyFalcons {
            count: row.count(),
            max: MAX_CONSTRUCTED_FALCONS,
        });
    }
    for f in row.falcons {
        if !u64::from(f.register_base).is_multiple_of(FALCON_REGISTER_BLOCK_LEN) {
            return Err(FalconInfoError::RegisterBaseUnaligned {
                register_base: f.register_base,
            });
        }
        if u64::from(f.register_base) + FALCON_REGISTER_BLOCK_LEN > regs_aperture_len {
            return Err(FalconInfoError::RegisterBaseOutsideAperture {
                register_base: f.register_base,
                aperture_len: regs_aperture_len,
            });
        }
        if f.ctx_buffer_size > MAX_CTX_BUFFER_SIZE {
            return Err(FalconInfoError::ContextBufferTooLarge {
                ctx_buffer_size: f.ctx_buffer_size,
                max: MAX_CTX_BUFFER_SIZE,
            });
        }
    }

    let mut params = vec![0u8; FALCON_INFO_PARAMS_SIZE];
    // ★ Derived from the table, never carried alongside it: see [`FalconInventoryRow`].
    let count = u32::try_from(row.count()).unwrap_or(u32::MAX);
    params[NUM_CONSTRUCTED_FALCONS_OFF..NUM_CONSTRUCTED_FALCONS_OFF + 4]
        .copy_from_slice(&count.to_le_bytes());
    for (i, f) in row.falcons.iter().enumerate() {
        let at = CONSTRUCTED_FALCONS_TABLE_OFF + i * CONSTRUCTED_FALCON_STRIDE;
        let put = |p: &mut [u8], off: usize, v: u32| {
            p[at + off..at + off + 4].copy_from_slice(&v.to_le_bytes());
        };
        put(&mut params, ENG_DESC_OFF, f.eng_desc);
        put(&mut params, CTX_ATTR_OFF, f.ctx_attr);
        put(&mut params, CTX_BUFFER_SIZE_OFF, f.ctx_buffer_size);
        put(&mut params, ADDR_SPACE_LIST_OFF, f.addr_space_list);
        put(&mut params, REGISTER_BASE_OFF, f.register_base);
    }
    Ok(params)
}
