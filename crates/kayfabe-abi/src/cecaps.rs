//! `NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS` (`0x20802a0b`) — the copy engines' own
//! capability table, and the first control this port serves whose **reply is not the
//! request edited**: it is `[OUT]`-only, so the whole 136 bytes are constructed.
//!
//! ## ⊘⊘ First, the two refutations this module exists to record — and both are §14.32's
//!
//! §14.32 picked this rung correctly and then handed its successor a probe plan and a
//! reply table that are **both wrong**, for two different reasons. Recording them here
//! because the mistake is the interesting part: it named the trap it then fell into.
//!
//! ### 1. ★★★ "`rmladder --probe-ctrl 0x20802a0b:136` is sound on this struct" — it is
//! ### sound and it is UNRUNNABLE, and even the reachable sibling is blind by construction
//!
//! §14.32 reasoned that §14.31's `[IN]`-field trap does not apply here (true — this struct
//! has no `[IN]` field), and concluded the probe *"is the cheap instrument that turns the
//! `capsTbl[4..63]` zeros from ambiguous into positive"*. `[measured 2026-08-09, real GA106
//! `GPU-d0913685`, `traces/real_ga106/rmladder_r18_cecaps_real_ga106.txt`]`:
//!
//! ```text
//! info  R18 0x20802a0b    = refused Other(86) (no value measured)
//! ★     R18 0x20802a0a    = NV_OK, 136 bytes: e303e303e203e203 00…00 0f00000000000000
//! ```
//!
//! ⊘ **`0x20802a0b` is not reachable from usermode at all.** Its export flags are `0x101d0`
//! (`ogkm-580: g_subdevice_nvoc.c:7705-7718`) = `GPU_LOCK_DEVICE_ONLY(0x10)` |
//! `ROUTE_TO_PHYSICAL(0x40)` | `INTERNAL(0x80)` | `API_LOCK_READONLY(0x100)` |
//! `GSP_PLUGIN_FOR_VGPU_GSP(0x10000)`, and it carries **neither** `PRIVILEGED(0x4)` **nor**
//! `NON_PRIVILEGED(0x8)`, which is `RMCTRL_FLAGS_KERNEL_PRIVILEGED` — the default that
//! refuses every usermode client including root (`ogkm-580: control.h:170-247`). This is
//! `0x20802a08`'s shape exactly, the one `probe_ctrl`'s own doc comment already records as
//! refused. ⇒ ★★ **Checking that a probe is sound on the STRUCT is not checking that the
//! probe can reach the CONTROL.** §14.31's trap was disarmed and a *different*
//! precondition, one rung older and already written down, was never asked.
//!
//! ⊘⊘ And the sibling that *is* reachable cannot answer the question either.
//! `0x20802a0a` carries `NON_PRIVILEGED` and answers `NV_OK` — but
//! `subdeviceCtrlCmdCeGetAllCaps_IMPL` opens with
//! `portMemSet(pCeCapsParams, 0, sizeof(*pCeCapsParams))`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/ce/kernel_ce_shared.c:312`) **before** it forwards.
//! The `0xCD` seed is destroyed by the callee, so a "buffer was WRITTEN" verdict there is
//! guaranteed regardless of what the physical layer did — and indeed not one `0xCD` byte
//! came back. ★ The seed instrument is **blind by construction on this control**, and
//! `capsTbl[4..63]`'s zeros stay unmeasured-at-the-physical-boundary. There is no usermode
//! instrument that can measure them. See §3 for why it stops mattering.
//!
//! ### 2. ★★★ The bytes §14.32 published as "the real GA106's own reply" are the reply of
//! ### the control it correctly told you NOT to serve
//!
//! §14.32's table is headed *"The real GA106's own reply
//! (`cuinit_ioctl_trace_real_ga106.txt:62`)"* and line 62 is **`0x20802a0a`** — the kernel's
//! composed answer, one boundary above the id being served. That is the very trap the same
//! section named in bold two paragraphs earlier (*"a table read at one boundary does not
//! describe the boundary below"*), applied to the id and then not applied to the bytes.
//!
//! ★ The bytes turn out to be right anyway, and that is worth having **proved** rather than
//! inherited. `subdeviceCtrlCmdCeGetAllCaps_IMPL` (`kernel_ce_shared.c:282-336`) post-
//! processes the physical reply with exactly two operations, and both are **monotone ORs**:
//!
//! - `pCeCapsParams->present |= BIT64(kceInst)` for every non-stubbed `KernelCE` (`:329`);
//! - `kceAssignCeCaps_HAL(pGpu, pKCe, pCeCapsParams->capsTbl[kceInst])` (`:331`), which for
//!   every Turing/Ampere/Ada part resolves to `kceAssignCeCaps_GP100`
//!   (`g_kernel_ce_nvoc.c:413-427`) → a bare `if (pKernelNvlink != NULL) kceGetNvlinkCaps(…)`
//!   (`kernel_ce_gp100.c:311-323`) → at most three `RMCTRL_SET_CAP`s, and `RMCTRL_SET_CAP`
//!   is `|=` (`ogkm-580: control.h:99`).
//!
//! There is **no `portMemSet` after the RPC and no `RMCTRL_CLEAR_CAP` anywhere on this
//! path**, so the physical reply survives to the caller in full. And the three bits the
//! kernel could have added are [`cap::SYSMEM_READ`], [`cap::SYSMEM_WRITE`] and
//! [`cap::NVLINK_P2P`] — which are **precisely the three that measure clear** in both
//! observed entries. ⇒ ★★★ The kernel added nothing: `GPU_GET_KERNEL_NVLINK` is `NULL` on a
//! GeForce GA106, so `kceAssignCeCaps_GP100` returns without touching a byte. The
//! caller-visible bytes **are** the physical reply, established by a source argument and
//! confirmed by which bits are zero, not by assuming the two boundaries agree.
//!
//! ⚠ Note what makes serving those bytes safe even if that argument were wrong: the guest
//! runs this same kernel code, so whatever set `X` it ORs on top, it ORs onto our reply too.
//! Serving `V` = the caller-observed value gives the caller `V | X = V`, because
//! `V = V_physical | X` already. The construction is idempotent under the guest's own
//! post-processing. That is the property, and it is why an OR-only post-pass is benign
//! where an assignment would not be.
//!
//! ## ★★★ What is served, and every bit's source
//!
//! `NV2080_CTRL_CE_GET_ALL_CAPS_PARAMS` (`ogkm-580: ctrl2080ce.h:331-334`) is
//! `NvU8 capsTbl[64][2]` then an 8-aligned `NvU64 present` — 136 bytes, `present` at 128,
//! no padding anywhere. `0x20802a0b` shares the type by `typedef` (`:340`).
//!
//! **`present` is a projection of the engine list**, not a number this module states. It is
//! `BIT64(instance)` OR'd over every row of the chip's own `FifoDeviceEntry` slice whose
//! `DEV_TYPE_ENUM` is [`crate::deviceinfo::DEV_TYPE_ENUM_LCE`] — the identical slice, and
//! the identical test, that `FIFO_GET_DEVICE_INFO_TABLE` and
//! `INTERNAL_DEVICE_INFO` already serve. A device that advertises four copy engines to the
//! guest's FIFO and five to the guest's CE layer would be two descriptions of one silicon,
//! which is the drift [`crate::deviceinfo`] exists to forbid.
//!
//! ⚠ ★★★ **And that projection is why an ogkm constant must not be used here.** The GA10x
//! HAL declares `NV_CE_MAX_LCE_MASK = 0x1F` — five LCEs, `{0,1}` GRCE, `{2,3}` sysmem,
//! `{4}` even-async (`ogkm-580: kernel_ce_ga102.c:33-38`) — and a reading of that constant
//! predicts `present = 0x1f` with a fifth entry. `[measured]` a real GA106 answers
//! **`present = 0x0f`** and `capsTbl[4] = {0x00, 0x00}`, twice independently (libcuda's
//! `cuInit` and a bare-Subdevice ladder). ⇒ The mask is the **permitted universe**; what the
//! part exposes is the dispatch's, and they differ by one engine. Fourth sighting of
//! `a_table_does_not_decide_behaviour`, and the one that would have shipped a caps entry for
//! an engine this device does not advertise.
//!
//! **The caps bits.** Only one of the twelve varies across this chip's CEs:
//! [`cap::GRCE`] — *"Set if the CE is synchronous with GR"* (`ctrl2080ce.h:105-106`) — set
//! on CE0/CE1 (`0x03e3`) and clear on CE2/CE3 (`0x03e2`). ★ That is a **principled per-CE
//! hardware fact, not a copy-paste**: `NV_CE_GRCE_ALLOWED_LCE_MASK = 0x03`
//! (`kernel_ce_ga102.c:34`, the mask `kceGetGrceSupportedLceMask_GA102` returns at `:188-196`
//! for GA102/103/104/**106**/107) names exactly LCE0 and LCE1, backed by
//! `NV_CE_GRCE_CONFIG__SIZE_1 = 2` and `NV_CE_MAX_GRCE = 2`
//! (`swref/published/ampere/ga102/dev_ce.h:32`, `kernel_ce_ga102.c:38`). ⊘ Open source gives
//! the *allowed* mask; the measurement gives that this part realises it. Both are recorded,
//! and [`GA10X_GRCE_LCE_MASK`] carries the arch in its name so Hopper is a named line rather
//! than a retrofit.
//!
//! The other eleven bits are uniform across every present LCE and come from
//! [`GA10X_LCE_BASE_CAPS`]. ⊘ **They are measured, and they cannot be anything else**: the
//! body of `subdeviceCtrlCmdCeGetAllPhysicalCaps_IMPL` is not in the vendored tree at all —
//! only its prototype and export row — because `ROUTE_TO_PHYSICAL | INTERNAL` puts it inside
//! GSP-RM firmware. There is no `ceGetPhysicalCaps` symbol in `src/` to read. This is
//! `0x20810108`'s situation (§14.26): a real part is the only oracle, and unlike the C
//! artifact's `dlen = 0` rows this one **captured a body**, twice.
//!
//! ⚠ ★ One projection was considered and is **wrong**, recorded so it is not re-invented:
//! [`cap::CC_SECURE`] is *"Set if the CE is capable of encryption/decryption"*
//! (`ctrl2080ce.h:137-138`) — a property of the silicon, **not** of whether Confidential
//! Computing is switched on. Deriving it from `ChipProfile::conf_compute` (which this port
//! serves as both-bits-clear) would agree on GA106 by coincidence and be wrong on any
//! CC-capable part with CC disabled. It is an arch fact and stays one.
//!
//! ## What the guest does with it
//!
//! `nv_gpu_ops.c:8439-8446` maps these bits straight into UVM's `gpuCeCaps`
//! (`grce`, `shared`, `sysmemRead`, `sysmemWrite`, `nvlinkP2p`, `sysmem`, `p2p`, `secure`),
//! and `nvkms-difr.c:805` refuses a CE for DIFR prefetch iff [`cap::GRCE`] is set. So these
//! are consumed bits, not diagnostics — which is the reason to state the measured ones and
//! refuse to invent the rest.

use crate::deviceinfo::{engine_info_type, DEV_TYPE_ENUM_LCE};
use crate::inittables::FifoDeviceEntry;

/// `NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS` — `ogkm-580: ctrl2080ce.h:336`. ★ **This** is
/// the id this port serves; see the module header for why it is not `0x20802a0a`.
pub const NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS: u32 = 0x2080_2a0b;

/// `NV2080_CTRL_CMD_CE_GET_ALL_CAPS` — `ogkm-580: ctrl2080ce.h:325`. The id libcuda's
/// `cuInit` calls and the one that fails `0x56`; it is **never served here**, because
/// `subdeviceCtrlCmdCeGetAllCaps_IMPL` is the guest kernel's own and reaches us only as
/// its forward of [`NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS`]. Present so tests and the
/// trace differential can name the caller without spelling a bare number.
pub const NV2080_CTRL_CMD_CE_GET_ALL_CAPS: u32 = 0x2080_2a0a;

/// `NV2080_CTRL_MAX_CES` — `ogkm-580: ctrl2080ce.h:327`.
///
/// ⚠ Not `NV2080_CTRL_MAX_PCES` (32). `subdeviceCtrlCmdCeGetAllCaps_VF`
/// (`ogkm-580: kernel_ce_ctrl.c:176`) loops the wrong one and under-copies; this port
/// serves the bare-metal `_IMPL` path, and the array really is 64 entries long.
pub const MAX_CES: usize = 64;

/// `NV2080_CTRL_CE_CAPS_TBL_SIZE` — `ogkm-580: ctrl2080ce.h:68`. Two bytes per CE.
pub const CAPS_TBL_SIZE: usize = 2;

/// Byte offset of `present` — `capsTbl` is 128 bytes and `NvU64` is already 8-aligned there,
/// so there is no padding before it.
pub const PRESENT_OFF: usize = MAX_CES * CAPS_TBL_SIZE;

/// `sizeof(NV2080_CTRL_CE_GET_ALL_CAPS_PARAMS)` = 136, and the `paramSize` both export rows
/// advertise (`ogkm-580: g_subdevice_nvoc.c:7699`, `:7714`).
pub const CE_GET_ALL_CAPS_PARAMS_SIZE: usize = PRESENT_OFF + 8;

/// The twelve named `NV2080_CTRL_CE_CAPS_*` bits, as `(byte index, mask)` — the shape
/// NVIDIA's own `byte:mask` macros carry (`ogkm-580: ctrl2080ce.h:91-102`), kept rather than
/// flattened to a `u16` so a transcription error is a compile-time-visible pair and not a
/// silently-plausible number.
///
/// ⊘ Bits `1:0x10`..`1:0x80` are undefined in this driver version and are never set.
pub mod cap {
    /// `0:0x01` — set if the CE is synchronous with GR. ★ The only bit that varies per CE
    /// on GA10x.
    pub const GRCE: (usize, u8) = (0, 0x01);
    /// `0:0x02` — set if the CE shares physical CEs with any other CE.
    pub const SHARED: (usize, u8) = (0, 0x02);
    /// `0:0x04` — enhanced sysmem **read** performance. ⚠ One of the three bits the guest
    /// kernel may OR in from NVLink topology; measured clear on GA106.
    pub const SYSMEM_READ: (usize, u8) = (0, 0x04);
    /// `0:0x08` — enhanced sysmem **write** performance. ⚠ Kernel-OR-able; measured clear.
    pub const SYSMEM_WRITE: (usize, u8) = (0, 0x08);
    /// `0:0x10` — usable for P2P over NVLink. ⚠ Kernel-OR-able; measured clear.
    pub const NVLINK_P2P: (usize, u8) = (0, 0x10);
    /// `0:0x20` — usable for sysmem transactions.
    pub const SYSMEM: (usize, u8) = (0, 0x20);
    /// `0:0x40` — usable for P2P transactions.
    pub const P2P: (usize, u8) = (0, 0x40);
    /// `0:0x80` — supports a block-linear copy larger than 64 KiB.
    pub const BL_SIZE_GT_64K_SUPPORTED: (usize, u8) = (0, 0x80);
    /// `1:0x01` — supports non-pipelined block linear.
    pub const SUPPORTS_NONPIPELINED_BL: (usize, u8) = (1, 0x01);
    /// `1:0x02` — supports pipelined block linear.
    pub const SUPPORTS_PIPELINED_BL: (usize, u8) = (1, 0x02);
    /// `1:0x04` — the CE is capable of encryption/decryption. ⊘ A silicon capability, **not**
    /// a statement that Confidential Computing is enabled; see the module header.
    pub const CC_SECURE: (usize, u8) = (1, 0x04);
    /// `1:0x08` — the CE can handle decompression workloads. Hopper and later.
    pub const DECOMP_SUPPORTED: (usize, u8) = (1, 0x08);

    /// Every named bit, so a gate or a decoder can quantify over the set rather than
    /// re-listing it. ★ `gates_quantified_over_a_list`: the list lives next to the
    /// definitions it enumerates.
    pub const ALL: &[(&str, (usize, u8))] = &[
        ("GRCE", GRCE),
        ("SHARED", SHARED),
        ("SYSMEM_READ", SYSMEM_READ),
        ("SYSMEM_WRITE", SYSMEM_WRITE),
        ("NVLINK_P2P", NVLINK_P2P),
        ("SYSMEM", SYSMEM),
        ("P2P", P2P),
        ("BL_SIZE_GT_64K_SUPPORTED", BL_SIZE_GT_64K_SUPPORTED),
        ("SUPPORTS_NONPIPELINED_BL", SUPPORTS_NONPIPELINED_BL),
        ("SUPPORTS_PIPELINED_BL", SUPPORTS_PIPELINED_BL),
        ("CC_SECURE", CC_SECURE),
        ("DECOMP_SUPPORTED", DECOMP_SUPPORTED),
    ];
}

/// One CE's two caps bytes. A newtype rather than a `[u8; 2]` so the bit accessors are the
/// only way to read it and `capsTbl[i][0]` never gets confused with `capsTbl[0][i]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CeCaps(pub [u8; CAPS_TBL_SIZE]);

impl CeCaps {
    /// An all-clear entry — what a CE absent from `present` carries.
    pub const NONE: Self = Self([0; CAPS_TBL_SIZE]);

    /// Set one named cap. `RMCTRL_SET_CAP`'s `|=`, in Rust.
    #[must_use]
    pub const fn with(mut self, (byte, mask): (usize, u8)) -> Self {
        self.0[byte] |= mask;
        self
    }

    /// Test one named cap. `NV2080_CTRL_CE_GET_CAP`, in Rust.
    #[must_use]
    pub const fn has(self, (byte, mask): (usize, u8)) -> bool {
        self.0[byte] & mask != 0
    }

    /// The little-endian `u16` reading, for tests that want to say `0x03e3` in one token.
    /// ⊘ Not the wire form — the wire form is the two bytes, in order.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        u16::from_le_bytes(self.0)
    }
}

/// The caps every present LCE on a GA10x part carries, GRCE aside.
///
/// `[measured 2026-08-09, real GA106 `GPU-d0913685`,
/// `traces/real_ga106/rmladder_r18_cecaps_real_ga106.txt` and
/// `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:62`, byte-identical]` — `0x03e2`,
/// i.e. `SHARED | SYSMEM | P2P | BL_SIZE_GT_64K_SUPPORTED | SUPPORTS_NONPIPELINED_BL |
/// SUPPORTS_PIPELINED_BL`.
///
/// ⊘ **Clear, and deliberately:** `SYSMEM_READ`, `SYSMEM_WRITE` and `NVLINK_P2P` (this part
/// has no NVLink, and these are exactly the three the guest kernel would add if it had),
/// `CC_SECURE` (Ampere consumer CEs cannot encrypt) and `DECOMP_SUPPORTED` (no decompression
/// engine before Hopper).
///
/// ⚠ Named `GA10X_` because the value is an architecture's, and the day a Hopper profile
/// appears this constant must not silently answer for it.
pub const GA10X_LCE_BASE_CAPS: CeCaps = CeCaps::NONE
    .with(cap::SHARED)
    .with(cap::SYSMEM)
    .with(cap::P2P)
    .with(cap::BL_SIZE_GT_64K_SUPPORTED)
    .with(cap::SUPPORTS_NONPIPELINED_BL)
    .with(cap::SUPPORTS_PIPELINED_BL);

/// Which LCE indices are graphics copy engines on GA10x — `NV_CE_GRCE_ALLOWED_LCE_MASK`,
/// `ogkm-580: src/nvidia/src/kernel/gpu/ce/arch/ampere/kernel_ce_ga102.c:34`, returned by
/// `kceGetGrceSupportedLceMask_GA102` (`:188-196`) for GA102/103/104/106/107.
///
/// ⚠ This is the mask of LCEs *allowed* to be a GRCE. It is intersected with what the chip
/// actually exposes, never used on its own — see [`GA10X_EXPOSED_LCE_MASK_IS_NOT_A_SOURCE`].
pub const GA10X_GRCE_LCE_MASK: u64 = 0x03;

/// ⊘⊘ **A constant that exists to be refused.** `NV_CE_MAX_LCE_MASK = 0x1F`
/// (`ogkm-580: kernel_ce_ga102.c:37`) enumerates five GA10x LCEs, and reading it as the
/// exposed set predicts `present = 0x1f`. `[measured]` a real GA106 answers **`0x0f`**, from
/// two independent callers. The permitted universe is not the dispatch; `present` is
/// projected from the chip's own engine list and nothing else. Kept, with its value, so the
/// contradiction is pinned rather than rediscovered.
pub const GA10X_EXPOSED_LCE_MASK_IS_NOT_A_SOURCE: u64 = 0x1f;

/// The per-CE facts this reply is built from — all of them projections, none of them new
/// numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeGeometry {
    /// `present`: which LCE instances this device advertises. Built by
    /// [`CeGeometry::from_engines`] from the chip's `FifoDeviceEntry` slice.
    pub present: u64,
    /// Which of those are graphics copy engines. [`GA10X_GRCE_LCE_MASK`] ∩ `present`.
    pub grce: u64,
    /// The caps every present LCE carries before [`cap::GRCE`] is applied.
    pub base: CeCaps,
}

impl CeGeometry {
    /// Project a geometry out of the engine list this device already advertises.
    ///
    /// ★ The whole point: `present` is not stated here, it is **read off the same slice**
    /// `FIFO_GET_DEVICE_INFO_TABLE` and `INTERNAL_DEVICE_INFO` serve, using the same
    /// `DEV_TYPE_ENUM == `[`DEV_TYPE_ENUM_LCE`] test `encode_internal_device_info_table`
    /// uses to find the copy-engine fault-id range. One silicon, one description.
    ///
    /// # Errors
    ///
    /// [`CeCapsError::NoCopyEngines`] if the chip row advertises none — RM's own
    /// `kgmmuInitCeMmuFaultIdRange_GA100` already refuses to boot such a table, so a zero
    /// `present` here would be a second symptom of a fault the device-info encoder catches
    /// first; and [`CeCapsError::InstanceOutOfRange`] for an instance id past
    /// [`MAX_CES`], which has no slot in `capsTbl` and no bit in `present`.
    pub fn from_engines(engines: &[FifoDeviceEntry]) -> Result<Self, CeCapsError> {
        let mut present = 0u64;
        for e in engines {
            if e.engine_data[engine_info_type::DEV_TYPE_ENUM] != DEV_TYPE_ENUM_LCE {
                continue;
            }
            let instance = e.engine_data[engine_info_type::INSTANCE_ID];
            if instance as usize >= MAX_CES {
                return Err(CeCapsError::InstanceOutOfRange {
                    engine: e.name,
                    instance,
                    max: MAX_CES,
                });
            }
            present |= 1u64 << instance;
        }
        if present == 0 {
            return Err(CeCapsError::NoCopyEngines);
        }
        Ok(Self {
            present,
            grce: present & GA10X_GRCE_LCE_MASK,
            base: GA10X_LCE_BASE_CAPS,
        })
    }

    /// The caps entry for one CE index: [`CeCaps::NONE`] unless the index is present.
    ///
    /// ⊘ An absent CE gets all-zero rather than the base caps, because the header defines
    /// `present` as the qualifier — *"If a CE is not marked present, its caps bits should be
    /// ignored"* (`ogkm-580: ctrl2080ce.h:319-322`) — and a table whose ignored rows still
    /// claim `SYSMEM | P2P` is a table that lies to anything that stops honouring the
    /// qualifier.
    #[must_use]
    pub fn caps_for(&self, index: usize) -> CeCaps {
        if index >= MAX_CES || self.present & (1u64 << index) == 0 {
            return CeCaps::NONE;
        }
        if self.grce & (1u64 << index) == 0 {
            self.base
        } else {
            self.base.with(cap::GRCE)
        }
    }
}

/// Why a CE caps reply could not be built. ⊘ Every variant refuses the whole control: there
/// is no partial answer on an `[OUT]`-only table, and a zeroed `present` is a *declared*
/// value meaning "this GPU has no copy engines", never a blank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CeCapsError {
    /// The chip row advertises no `LCE` engine at all.
    NoCopyEngines,
    /// An engine row's `INSTANCE_ID` has no slot in `capsTbl` and no bit in `present`.
    InstanceOutOfRange {
        /// The offending row's name.
        engine: &'static str,
        /// Its `INSTANCE_ID`.
        instance: u32,
        /// [`MAX_CES`].
        max: usize,
    },
}

impl core::fmt::Display for CeCapsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoCopyEngines => write!(
                f,
                "the chip row advertises no DEV_TYPE_ENUM_LCE engine, so present would be 0 \
                 — a declared value meaning 'this GPU has no copy engines', not a blank"
            ),
            Self::InstanceOutOfRange {
                engine,
                instance,
                max,
            } => write!(
                f,
                "engine {engine} has INSTANCE_ID {instance}, past NV2080_CTRL_MAX_CES {max}: \
                 no capsTbl slot and no present bit exist for it"
            ),
        }
    }
}

impl core::error::Error for CeCapsError {}

/// Build the whole `[OUT]` reply — 136 bytes, constructed rather than edited.
///
/// ⊘ **Nothing of the request is preserved**, and that is the difference from every other
/// arm this port serves. `NV2080_CTRL_CE_GET_ALL_CAPS_PARAMS` has no `[IN]` field
/// (`ogkm-580: ctrl2080ce.h:315-322` documents both members `[out]`), and the caller hands
/// RM a buffer its own kernel has already `portMemSet` to zero
/// (`kernel_ce_shared.c:312`). There is nothing to echo.
///
/// ⊘ Infallible **because every way this reply can be wrong is caught upstream**, in
/// [`CeGeometry::from_engines`], where the chip row is still in scope and the refusal can
/// name the offending engine. An encoder that can fail here would be a second place to
/// decide, and the serve site would have two refusal paths for one fact.
#[must_use]
pub fn encode_ce_get_all_physical_caps(geometry: &CeGeometry) -> Vec<u8> {
    let mut out = vec![0u8; CE_GET_ALL_CAPS_PARAMS_SIZE];
    for i in 0..MAX_CES {
        let caps = geometry.caps_for(i);
        out[i * CAPS_TBL_SIZE..(i + 1) * CAPS_TBL_SIZE].copy_from_slice(&caps.0);
    }
    out[PRESENT_OFF..PRESENT_OFF + 8].copy_from_slice(&geometry.present.to_le_bytes());
    out
}

/// Read a 136-byte reply back into `(present, capsTbl)` — for tests and the trace
/// differential, so a comparison against a captured reply is done on decoded values rather
/// than on a hex string somebody regrouped by hand.
///
/// ★ `re-derive from the raw bytes, never from the paragraph` is easier to obey when the
/// derivation is a function.
///
/// # Errors
///
/// [`CeCapsDecodeError::ShortParams`] if the buffer is under [`CE_GET_ALL_CAPS_PARAMS_SIZE`].
pub fn decode_ce_get_all_physical_caps(
    params: &[u8],
) -> Result<(u64, Vec<CeCaps>), CeCapsDecodeError> {
    let Some(body) = params.get(..CE_GET_ALL_CAPS_PARAMS_SIZE) else {
        return Err(CeCapsDecodeError::ShortParams {
            len: params.len(),
            need: CE_GET_ALL_CAPS_PARAMS_SIZE,
        });
    };
    let mut tbl = Vec::with_capacity(MAX_CES);
    for i in 0..MAX_CES {
        tbl.push(CeCaps([body[i * CAPS_TBL_SIZE], body[i * CAPS_TBL_SIZE + 1]]));
    }
    let mut present = [0u8; 8];
    present.copy_from_slice(&body[PRESENT_OFF..PRESENT_OFF + 8]);
    Ok((u64::from_le_bytes(present), tbl))
}

/// Why a reply could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CeCapsDecodeError {
    /// The buffer is shorter than the struct.
    ShortParams {
        /// What arrived.
        len: usize,
        /// [`CE_GET_ALL_CAPS_PARAMS_SIZE`].
        need: usize,
    },
}

impl core::fmt::Display for CeCapsDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortParams { len, need } => {
                write!(f, "{len}-byte params, {need} needed for the struct")
            }
        }
    }
}

impl core::error::Error for CeCapsDecodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The GA106 engine rows this port advertises, reduced to what this module reads.
    /// ⊘ Deliberately built here rather than imported from `kayfabe-device`: this crate is
    /// below that one, and the real slice is exercised end-to-end by
    /// `kayfabe-device/tests/ce_get_all_physical_caps.rs`.
    fn lce_row(name: &'static str, instance: u32) -> FifoDeviceEntry {
        let mut engine_data = [0u32; crate::inittables::ENGINE_DATA_TYPES];
        engine_data[engine_info_type::DEV_TYPE_ENUM] = DEV_TYPE_ENUM_LCE;
        engine_data[engine_info_type::INSTANCE_ID] = instance;
        FifoDeviceEntry {
            name,
            engine_data,
            pbdma_ids: [0; crate::inittables::ENGINE_MAX_PBDMA],
            pbdma_fault_ids: [0; crate::inittables::ENGINE_MAX_PBDMA],
            num_pbdmas: 1,
        }
    }

    fn gr_row() -> FifoDeviceEntry {
        let mut e = lce_row("GR0", 0);
        e.engine_data[engine_info_type::DEV_TYPE_ENUM] = 0;
        e
    }

    fn ga106() -> CeGeometry {
        CeGeometry::from_engines(&[
            gr_row(),
            lce_row("CE0", 0),
            lce_row("CE1", 1),
            lce_row("CE2", 2),
            lce_row("CE3", 3),
        ])
        .expect("four LCE rows project")
    }

    /// ★★★ The pin. `[measured 2026-08-09, real GA106 `GPU-d0913685`]` — the whole 136-byte
    /// reply, from BOTH independent callers, byte for byte.
    ///
    /// `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:62` (libcuda's `cuInit`) and
    /// `traces/real_ga106/rmladder_r18_cecaps_real_ga106.txt` (a bare Subdevice, no channel)
    /// carry the identical `out=` string.
    ///
    /// ⊘ **Built structurally, not typed as a hex blob** — and that is not fastidiousness.
    /// The first draft of this constant *was* a 272-character literal, and it was
    /// **sixteen bytes short**; the length assertion caught it, which is the third
    /// hand-regrouped-hex defect in four rungs and the first one a test caught instead of a
    /// reader. A 120-byte run of zeros is not something a human should transcribe. ★ The
    /// authority is `kayfabe-device/tests/ce_get_all_physical_caps.rs`, which parses the
    /// `out=` field out of the committed trace file itself and compares against this;
    /// nothing here can drift from the artifact without that test going red.
    fn real_ga106_reply() -> Vec<u8> {
        let mut v = vec![0xe3, 0x03, 0xe3, 0x03, 0xe2, 0x03, 0xe2, 0x03];
        v.resize(PRESENT_OFF, 0x00);
        v.extend_from_slice(&0x0f_u64.to_le_bytes());
        assert_eq!(v.len(), CE_GET_ALL_CAPS_PARAMS_SIZE);
        v
    }

    #[test]
    fn ga106_geometry_reproduces_the_real_reply_byte_for_byte() {
        assert_eq!(encode_ce_get_all_physical_caps(&ga106()), real_ga106_reply());
    }

    /// ⊘ The measured `present` is `0x0f`, and `NV_CE_MAX_LCE_MASK` says `0x1f`. Pinned so
    /// nobody "corrects" the projection into the HAL constant — the mask is the permitted
    /// universe, the engine list is the dispatch.
    #[test]
    fn present_is_the_engine_list_and_not_the_allowed_lce_mask() {
        let g = ga106();
        assert_eq!(g.present, 0x0f);
        assert_ne!(g.present, GA10X_EXPOSED_LCE_MASK_IS_NOT_A_SOURCE);
        let (present, tbl) = decode_ce_get_all_physical_caps(&real_ga106_reply()).expect("decode");
        assert_eq!(present, 0x0f, "hardware answers four CEs, not five");
        assert_eq!(tbl[4], CeCaps::NONE, "the fifth entry is empty on hardware");
    }

    /// ★ The single differing bit between CE0/1 and CE2/3 is `GRCE`, and nothing else.
    #[test]
    fn grce_is_the_only_per_ce_difference() {
        let (_, tbl) = decode_ce_get_all_physical_caps(&real_ga106_reply()).expect("decode");
        for (a, b) in [(0usize, 2usize), (0, 3), (1, 2), (1, 3)] {
            let differing: Vec<&str> = cap::ALL
                .iter()
                .filter(|&&(_, bit)| tbl[a].has(bit) != tbl[b].has(bit))
                .map(|&(name, _)| name)
                .collect();
            assert_eq!(differing, ["GRCE"], "CE{a} vs CE{b}");
        }
        assert!(tbl[0].has(cap::GRCE) && tbl[1].has(cap::GRCE));
        assert!(!tbl[2].has(cap::GRCE) && !tbl[3].has(cap::GRCE));
        assert_eq!(tbl[0].as_u16(), 0x03e3);
        assert_eq!(tbl[2].as_u16(), 0x03e2);
    }

    /// ★★★ The proof that the caller-visible bytes are the physical reply: the only three
    /// bits `kceAssignCeCaps_GP100` could add are exactly the three that measure clear.
    #[test]
    fn the_three_kernel_or_able_bits_are_clear_on_every_present_ce() {
        let (present, tbl) = decode_ce_get_all_physical_caps(&real_ga106_reply()).expect("decode");
        for i in 0..MAX_CES {
            if present & (1u64 << i) == 0 {
                continue;
            }
            for bit in [cap::SYSMEM_READ, cap::SYSMEM_WRITE, cap::NVLINK_P2P] {
                assert!(
                    !tbl[i].has(bit),
                    "CE{i} carries a bit the guest kernel would have OR'd in from NVLink \
                     topology; the physical-reply argument in this module's header rests on \
                     all three being clear"
                );
            }
        }
    }

    /// ⊘ CC_SECURE and DECOMP are clear on this arch, and neither is a projection of
    /// anything this port serves elsewhere.
    #[test]
    fn cc_secure_and_decomp_are_clear_on_ga10x() {
        let caps = ga106().caps_for(0);
        assert!(!caps.has(cap::CC_SECURE));
        assert!(!caps.has(cap::DECOMP_SUPPORTED));
    }

    /// An absent CE is all-zero, not "base caps nobody should read".
    #[test]
    fn absent_ces_carry_no_caps() {
        let g = ga106();
        for i in 4..MAX_CES {
            assert_eq!(g.caps_for(i), CeCaps::NONE, "CE{i}");
        }
        assert_eq!(g.caps_for(MAX_CES), CeCaps::NONE, "past the array");
    }

    /// ⊘ A chip row with no copy engine refuses rather than answering `present = 0`.
    #[test]
    fn no_copy_engines_refuses() {
        assert_eq!(
            CeGeometry::from_engines(&[gr_row()]),
            Err(CeCapsError::NoCopyEngines)
        );
        assert_eq!(CeGeometry::from_engines(&[]), Err(CeCapsError::NoCopyEngines));
    }

    /// ⊘ An instance id with no slot refuses rather than wrapping into another CE's row.
    #[test]
    fn instance_past_the_array_refuses() {
        let e = CeGeometry::from_engines(&[lce_row("CE64", 64)]);
        assert!(matches!(
            e,
            Err(CeCapsError::InstanceOutOfRange {
                instance: 64,
                max: 64,
                ..
            })
        ));
        assert!(CeGeometry::from_engines(&[lce_row("CE63", 63)]).is_ok());
    }

    /// A sparse engine list projects a sparse `present`, and the GRCE intersection follows
    /// it rather than asserting `0x03` unconditionally.
    #[test]
    fn grce_is_intersected_with_what_is_actually_exposed() {
        let g = CeGeometry::from_engines(&[lce_row("CE1", 1), lce_row("CE5", 5)])
            .expect("two LCE rows project");
        assert_eq!(g.present, 0b10_0010);
        assert_eq!(g.grce, 0b10, "only CE1 is both allowed-GRCE and exposed");
        assert!(g.caps_for(1).has(cap::GRCE));
        assert!(!g.caps_for(5).has(cap::GRCE));
        assert_eq!(g.caps_for(0), CeCaps::NONE);
    }

    /// The twelve named bits are twelve distinct `(byte, mask)` pairs — a transcription
    /// collision would otherwise make two caps the same bit and nothing would notice.
    #[test]
    fn every_named_cap_is_a_distinct_bit() {
        for (i, &(na, a)) in cap::ALL.iter().enumerate() {
            assert!(a.0 < CAPS_TBL_SIZE, "{na} indexes byte {}", a.0);
            assert!(a.1.count_ones() == 1, "{na} is not a single bit");
            for &(nb, b) in &cap::ALL[i + 1..] {
                assert_ne!(a, b, "{na} and {nb} are the same bit");
            }
        }
        assert_eq!(cap::ALL.len(), 12);
    }

    #[test]
    fn a_short_buffer_is_refused_rather_than_padded() {
        let short = vec![0u8; CE_GET_ALL_CAPS_PARAMS_SIZE - 1];
        assert!(matches!(
            decode_ce_get_all_physical_caps(&short),
            Err(CeCapsDecodeError::ShortParams { need: 136, .. })
        ));
    }

    /// The layout, against the header rather than against itself.
    #[test]
    fn the_struct_layout_is_the_headers() {
        assert_eq!(MAX_CES, 64);
        assert_eq!(CAPS_TBL_SIZE, 2);
        assert_eq!(PRESENT_OFF, 128);
        assert_eq!(CE_GET_ALL_CAPS_PARAMS_SIZE, 136);
    }
}
