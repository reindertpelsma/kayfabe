//! The **version-independent** views: what the logic crates are allowed to see.
//!
//! The quarantine rule (decision #2) says `#[repr(C)]` NVIDIA layouts live only
//! in this crate. That is necessary but not sufficient — if the core took a
//! `Nvos46Parameters` it would still be pinned to one driver's field set. So the
//! crate's *public product* is this module: small owned structs carrying the
//! fields the core actually consumes, produced by [`crate::versions`] from
//! whichever wire layout the detected driver version uses.
//!
//! This is nvproxy's own decomposition. `NVOS21_PARAMETERS` and
//! `NVOS64_PARAMETERS` are two wire shapes for one verb, and nvproxy normalises
//! them through `ToOS64`/`FromOS64`
//! (`gvisor/pkg/abi/nvgpu/frontend.go:335-357, :824-827`) so its handlers see one
//! shape. [`AllocReq`] is that, generalised: one view per verb, N wire layouts
//! behind it.
//!
//! Field names deliberately mirror `kayfabe_core::rmgraph::RmEvent`'s payloads,
//! so the wire→event mapping is a rename and nothing else. That mapping is NOT
//! in this milestone (see the crate docs) — this crate does not depend on
//! `kayfabe-core`.

use crate::wire::AbiError;

/// `NV_ESC_RM_FREE` — the `RmEvent::Free` facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FreeReq {
    /// `hRoot` — the owning client namespace.
    pub client: u32,
    /// `hObjectParent`.
    pub parent: u32,
    /// `hObjectOld` — the handle to free.
    pub handle: u32,
}

/// `NV_ESC_RM_ALLOC` — the `RmEvent::Alloc` facts, normalised across the two
/// wire shapes (`NVOS21` v1 and `NVOS64` v2).
///
/// `rights_requested` is `NVOS64`-only and reads as `0` from an `NVOS21`, which
/// is what nvproxy's `NVOS21_PARAMETERS::GetPRightsRequested` also returns
/// (`gvisor/pkg/abi/nvgpu/frontend.go:322-324`) — a *declared* absence, not a
/// lost field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AllocReq {
    /// `hRoot` — the owning client namespace.
    pub client: u32,
    /// `hObjectParent`.
    pub parent: u32,
    /// `hObjectNew` — `0` asks RM to pick a handle.
    pub handle: u32,
    /// `hClass`.
    pub class: u32,
    /// `pAllocParms` — a guest pointer. **Never dereferenced here**; the caller
    /// resolves it through the VMM's guest-memory seam.
    pub params_ptr: u64,
    /// `pRightsRequested`, or `0` on the v1 shape.
    pub rights_requested: u64,
    /// `paramsSize` — guest-declared, so it is an *assertion by the guest*, not
    /// a fact. Validate it against the class's own size before trusting it.
    pub params_size: u32,
    /// The wire shape this came off, kept because the reply must be written back
    /// in the same shape.
    pub wire: AllocWire,
}

/// Which `NV_ESC_RM_ALLOC` wire shape a request arrived in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AllocWire {
    /// `NVOS21_PARAMETERS` — 32 bytes, no rights mask, no flags.
    #[default]
    V1,
    /// `NVOS64_PARAMETERS` — 48 bytes.
    V2,
}

/// `rpc_gsp_rm_alloc_v03_00`'s **fixed header** — the GSP-RPC alloc shape.
///
/// # ★ This is NOT [`AllocReq`], and confusing the two mis-decodes every field
///
/// [`AllocReq`] is the **ioctl** shape (`NVOS21`/`NVOS64`): it carries a
/// `params_ptr`, a guest pointer the caller resolves through the VMM's
/// guest-memory seam. The GSP-RPC shape is a *different struct* whose params are
/// an **inline flexible array** — the guest already copied them into the command
/// queue, so nothing is ever dereferenced (`docs/design/gsp_core_bridge.md`
/// §1.3). Running `decode_alloc` on an RPC body would read `hClass` out of
/// `status` and a pointer out of `paramsSize`.
///
/// Layout `[src]` `ogkm-580: src/nvidia/generated/g_rpc-structures.h:1491-1502` /
/// `ogkm-610: src/nvidia/generated/g_rpc-structures.h:1408-1419` — the same nine
/// members in the same order at both tags, only the line numbers move:
/// `hClient@0, hParent@4, hObject@8, hClass@12, status@16, paramsSize@20,
/// flags@24, reserved[4]@28, params[]@32`.
///
/// ★ Independently confirmed: the C artifact transcribed the same offsets by hand
/// from a live trace — *"fn=103 (GSP_RM_ALLOC) body: hClient@80, hParent@84,
/// hObject@88, hClass@92, paramsSize@100, params@112"*
/// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2132-2135`, repeated
/// `:6464-6465`), which are element-relative with a 48-byte element header and a
/// 32-byte envelope in front, i.e. **minus 80** they are `0/4/8/12/20/32`. Two
/// humans, two trees, one answer; `crates/kayfabe-abi/tests/mean_wire.rs` asserts
/// the subtraction rather than trusting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RpcAllocReq {
    /// `hClient` @ +0 — **the namespace this alloc is issued in**, and the only
    /// place that fact may be read from (`gsp_core_bridge.md` §3.2).
    pub client: u32,
    /// `hParent` @ +4. `0` on a client-root alloc — see the bridge's §2.2a
    /// normalisation, which is a bridge rule and deliberately not applied here.
    pub parent: u32,
    /// `hObject` @ +8. Also `0` on a client-root alloc.
    pub handle: u32,
    /// `hClass` @ +12.
    pub class: u32,
    /// `paramsSize` @ +20 — guest-declared, so an *assertion by the guest* and
    /// never a fact. Validate it against the payload before slicing with it.
    pub params_size: u32,
    /// `flags` @ +24 — carries `RMAPI_RPC_FLAGS_SERIALIZED`; decode it with
    /// [`crate::rpc_params_are_serialized`].
    pub params_flags: u32,
    /// Byte offset of `params[]` within the RPC payload. A *derived* constant
    /// ([`RpcAllocReq::HEADER`]), carried on the view so a caller never has to
    /// write `32` at a call site.
    pub params_at: usize,
}

impl RpcAllocReq {
    /// The C typedef name, for [`AbiError::Truncated`].
    pub const C_NAME: &'static str = "rpc_gsp_rm_alloc_v03_00";
    /// Bytes of fixed header before `params[]`.
    pub const HEADER: usize = 32;
}

/// `rpc_gsp_rm_control_v03_00`'s **fixed header** — the GSP-RPC control shape.
///
/// # ★ Not [`ControlReq`], and the confusion is silent
///
/// [`ControlReq`] is the **ioctl** shape (`NVOS54`), whose `params` is a *guest
/// pointer*. This one's `params[]` is an inline flexible array the guest already
/// copied into the command queue, so nothing here is ever dereferenced
/// (`docs/design/gsp_core_bridge.md` §1.3). The two also disagree about where
/// `paramsSize` lives, so running the wrong decoder yields a plausible struct
/// full of wrong numbers rather than an error.
///
/// Layout `[src]` `ogkm-610: src/nvidia/generated/g_rpc-structures.h:1423-1435`, and
/// `ogkm-580: src/nvidia/generated/g_rpc-structures.h:1506-1518` is **character for
/// character the same list** — this struct does not move across the supported range:
/// `hClient@0, hObject@4, cmd@8, status@12, paramsSize@16, rmapiRpcFlags@20,
/// rmctrlFlags@24, rmctrlAccessRight@28, reserved0(NvU64, 8-aligned)@32,
/// params[]@40`.
///
/// ★ Independently confirmed: the C artifact transcribed the same offsets by hand
/// from a live trace — *"fn=76 (GSP_RM_CONTROL) body: hClient@80, hObject@84,
/// cmd@88, status@92, paramsSize@96, params@120"*
/// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2134-2135`, repeated
/// `:2732-2733`), element-relative with a 48-byte element header and a 32-byte
/// envelope in front, i.e. **minus 80** they are `0/4/8/12/16/40`. Two humans,
/// two trees, one answer; `crates/kayfabe-abi/tests/mean_wire.rs` asserts the
/// subtraction rather than trusting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RpcControlReq {
    /// `hClient` @ +0 — **the namespace this control is issued in**, and the only
    /// place that fact may be read from. The C's `GPU_PROMOTE_CTX` handler is the
    /// counter-example: it reads a client out of a *params* field and never looks
    /// at this one (`gsp_core_bridge.md` §3.2).
    pub client: u32,
    /// `hObject` @ +4 — the object the command is issued against.
    pub object: u32,
    /// `cmd` @ +8 — e.g.
    /// [`crate::generated::ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`].
    /// Decide what it *means* with [`crate::versions::DriverAbiTable::control_params`].
    pub cmd: u32,
    /// `paramsSize` @ +16 — guest-declared, so an *assertion by the guest* and
    /// never a fact. Validate it against the payload before slicing with it.
    pub params_size: u32,
    /// `rmapiRpcFlags` @ +20.
    ///
    /// ★ Two bits live here, not one: `RMAPI_RPC_FLAGS_COPYOUT_ON_ERROR` =
    /// `NVBIT(0)` and `RMAPI_RPC_FLAGS_SERIALIZED` = `NVBIT(1)`
    /// (`ogkm-580: src/nvidia/inc/kernel/rmapi/rmapi.h:161-163` /
    /// `ogkm-610: src/nvidia/inc/kernel/rmapi/rmapi.h:161-163` — same lines at both), and
    /// `rpcRmApiControl_GSP` sets them independently
    /// (`ogkm-580: rpc.c:10997-11001` / `ogkm-610: rpc.c:10802-10806`).
    /// So the serialization question is [`crate::rpc_params_are_serialized`], a
    /// **bit test** — a `!= 0` on the whole word would refuse every control that
    /// merely asked for copy-out-on-error.
    pub rmapi_rpc_flags: u32,
    /// Byte offset of `params[]` within the RPC payload — the derived constant
    /// [`RpcControlReq::HEADER`], carried on the view so a caller never has to
    /// write `40` at a call site.
    pub params_at: usize,
}

impl RpcControlReq {
    /// The C typedef name, for [`AbiError::Truncated`].
    pub const C_NAME: &'static str = "rpc_gsp_rm_control_v03_00";
    /// Bytes of fixed header before `params[]`.
    ///
    /// 40, not 36: `reserved0` is `NvU64 NV_ALIGN_BYTES(8)` at +32.
    pub const HEADER: usize = 40;
}

/// `NV_ESC_RM_CONTROL` — the envelope an RM control command arrives in.
///
/// The *payload* at `params_ptr` is command-specific; the only one this
/// milestone decodes is [`SetPageDir`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ControlReq {
    /// `hClient`.
    pub client: u32,
    /// `hObject` — the object the command is issued against.
    pub object: u32,
    /// `cmd` — e.g. [`crate::generated::ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`].
    pub cmd: u32,
    /// `flags`.
    pub flags: u32,
    /// `params` — a guest pointer, never dereferenced here.
    pub params_ptr: u64,
    /// `paramsSize` — guest-declared.
    pub params_size: u32,
}

/// `NV_ESC_RM_DUP_OBJECT` — the `RmEvent::Dup` facts.
///
/// The only cross-client transfer edge in the RM object model, and therefore the
/// protocol-correct source of process grouping (`l1_concurrency.md` §12.27).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DupReq {
    /// `hClient` — the **destination** client.
    pub dst_client: u32,
    /// `hParent` — parent of the new alias.
    pub dst_parent: u32,
    /// `hObject` — the new alias handle.
    pub dst_handle: u32,
    /// `hClientSrc`.
    pub src_client: u32,
    /// `hObjectSrc`.
    pub src_handle: u32,
    /// `flags`.
    pub flags: u32,
}

/// `NV_ESC_RM_MAP_MEMORY_DMA` — the `RmEvent::MapMemoryDma` facts.
///
/// ★ This is the versioned one. `flags2` and `kindOverride` exist only from
/// driver 580.65.06 and are deliberately **not** on this view: the core has no
/// use for them, and a field that is present on some versions and absent on
/// others is exactly the kind of thing that leaks a version name upward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapMemoryDma {
    /// `hClient`.
    pub client: u32,
    /// `hDevice`.
    pub device: u32,
    /// `hDma` — the VASpace (or context-DMA) handle the mapping lands in.
    pub dma: u32,
    /// `hMemory` — the memory resource being mapped.
    pub memory: u32,
    /// `offset` into the memory resource.
    pub offset: u64,
    /// `length` of the mapping.
    pub length: u64,
    /// `flags`.
    pub flags: u32,
    /// `dmaOffset` — `[OUT]`, and `[IN]` when the fixed-offset flag is set. This
    /// is the guest VA the mapping is at.
    pub dma_offset: u64,
}

/// `NV_ESC_RM_UNMAP_MEMORY_DMA` — the `RmEvent::Unmap` facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnmapMemoryDma {
    /// `hClient`.
    pub client: u32,
    /// `hDevice`.
    pub device: u32,
    /// `hDma` — the VASpace handle.
    pub dma: u32,
    /// `hMemory`.
    pub memory: u32,
    /// `flags`.
    pub flags: u32,
    /// `dmaOffset` — the VA to unmap, as returned by the matching map.
    pub dma_offset: u64,
    /// `size` — `0` means "the whole mapping".
    pub size: u64,
}

/// `NV0000_ALLOC_PARAMETERS` — the two fields the core reads off a client root.
///
/// # The prefix contract (deliberate, and narrower than the struct)
///
/// This view is decoded from the **first 8 bytes only**, not from the whole
/// struct. That is not laziness, it is the honest bound on what we know:
///
/// - `hClient` @ +0 and `processID` @ +4 are the first two members in every
///   ogkm tree, and RM's own writer sets exactly them
///   (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc.h:55,70,75` /
///   `ogkm-610: src/nvidia/inc/kernel/vgpu/rpc.h:55,70,75` — same lines at both).
/// - The **rest** of the struct — `processName[100]` and `pOsPidInfo` — is
///   spelled identically by both vendored tags
///   (`ogkm-580: src/common/sdk/nvidia/inc/class/cl0000.h:47-52` /
///   `ogkm-610: src/common/sdk/nvidia/inc/class/cl0000.h:47-52`, with
///   `NV_PROC_NAME_MAX_LENGTH = 100U` at `nvlimits.h:47` in both), but has **no
///   oracle outside ogkm at all**: gVisor's `nvproxy` does not model
///   `NV0000_ALLOC_PARAMETERS`, and neither does the C artifact
///   (`grep NV0000_ALLOC_PARAMETERS` finds nothing in either). Both vendored
///   tags are ≥ 580 while [`crate::versions::TABLES`] admits versions down to
///   550.54.04, and `pOsPidInfo` has the shape of a recent addition — so
///   `sizeof` at 550/575 is still **unverified**.
///
/// Requiring 120 bytes here would therefore refuse a legitimate older client
/// alloc on a guess. Requiring 8 asserts only what every available reading
/// agrees on. The remaining gap is *below* 580, so settling the tail needs a
/// vendored 550/575 tag — the two we have cannot do it, and neither can faith.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClientAllocFacts {
    /// `hClient` — the client handle the guest asks for.
    pub h_client: u32,
    /// `processID` — the decision-#14 grouping discriminator. Decode it with
    /// [`crate::GuestOs::client_kind_from_process_id`].
    pub process_id: u32,
}

/// `NV0080_ALLOC_PARAMETERS` — the multi-GPU routing fact a Device declares.
///
/// Unlike [`ClientAllocFacts`] this is decoded from the **whole** struct, because
/// its 56-byte size is confirmed by three independent oracles: ogkm 610.43.02,
/// `gvisor/pkg/abi/nvgpu/classes.go:198-211`, and the C artifact's
/// `tests/abi_parity/abi_parity_test.go:120` (`nv0080_alloc_parameters … 56`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceAllocFacts {
    /// `deviceId` — the physical-GPU index this Device routes to.
    ///
    /// ★ NVIDIA spells this field `deviceId`; the project's prose (and
    /// `AllocFacts::device_instance`) calls it the *device instance*. Same field:
    /// ogkm's own MIG shim assigns `ws->nv0080Params.deviceId = migDev->deviceInstance`
    /// (`ogkm-580: src/common/src/nv_smg.c:503` / `ogkm-610: src/common/src/nv_smg.c:517`
    /// — same statement, moved), which is the two names meeting.
    ///
    /// Guest-declared and therefore attacker-controlled — see
    /// `docs/reference/mode2_bench_lifecycle.md`.
    pub device_id: u32,
    /// `hClientShare`.
    pub h_client_share: u32,
    /// `hTargetClient`.
    pub h_target_client: u32,
    /// `hTargetDevice`.
    pub h_target_device: u32,
    /// `flags`.
    pub flags: u32,
    /// `vaSpaceSize`.
    pub va_space_size: u64,
    /// `vaMode`.
    pub va_mode: u32,
}

/// `NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS` — the one fact a TSG declares that
/// the core models.
///
/// Decoded from the **whole** 20-byte struct, because unlike the client root its
/// tail has a second oracle: the field list is byte-identical in both vendored
/// trees (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:2903-2911` and
/// `ogkm-610: src/common/sdk/nvidia/inc/nvos.h:2899-2906`), so there is
/// no version fork to be tolerant about.
///
/// ★ `engineType` is deliberately **not** carried. It is a declared fact, but
/// `kayfabe_core::rmgraph::AllocFacts` has nowhere to put it and nothing above
/// would read it — and this crate's rule is that a field needs a consumer first.
/// See [`ChannelAllocFacts`] for why the same absence is *load-bearing* one level
/// down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TsgAllocFacts {
    /// `hVASpace` — the VASpace every channel in the group inherits.
    /// `NV01_NULL_OBJECT` (0) means the group declares none.
    pub h_vaspace: u32,
}

/// `NV_CTXSHARE_ALLOCATION_PARAMETERS` — the VASpace a subcontext declares.
///
/// Whole-struct (12 bytes) for the same reason as [`TsgAllocFacts`]: identical at
/// both vendored tags (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:3232-3237`,
/// `ogkm-610: src/common/sdk/nvidia/inc/nvos.h:3223-3228`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CtxShareAllocFacts {
    /// `hVASpace` — the VASpace this context share is bound to. 0 = none.
    pub h_vaspace: u32,
}

/// `NV_CHANNEL_ALLOC_PARAMS` — the three fields a channel declares that the core
/// models, decoded under a **prefix contract** ([`crate::versions::CHANNEL_ALLOC_PREFIX`]).
///
/// # ★★ Why the prefix stops at 32, with a measurement rather than a caveat
///
/// This is the one struct in the slice whose tail is **known to have moved**
/// inside the supported version range:
///
/// ```text
///                              610.43.02          580.159.04 (the bench)
///   +20  flags                 flags              flags
///   +24  hContextShare         hContextShare      hContextShare
///   +28  hVASpace              hVASpace           hVASpace
///   +32  ————————————————————  hHandleVASpace     hUserdMemory[0]   ★ diverges
/// ```
///
/// `ogkm-610: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-347` inserts
/// `hHandleVASpace` after `hVASpace`;
/// `ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-342` has no
/// such field. (The `typedef` opens at `:296` in **both** trees; only the closing
/// line differs, because 610 carries one member more.) A generated 610 mirror
/// would mis-read every field from +32 onward for the driver this bench runs
/// ([`crate::versions::BENCH_DRIVER`] = 580.159.04), so the struct is not
/// mirrored at all and only the agreeing prefix is read.
///
/// # ★ What is NOT here, and why that costs something
///
/// `engineType` (`+128` at 580, `+136` at 610 — i.e. past the prefix, in the
/// divergent region; the 8-byte skew is `hHandleVASpace`'s 4 bytes plus the
/// 4 bytes of re-alignment it forces on the 8-aligned `userdOffset[]`. The C
/// artifact, measured on 580, independently records `engineType@128`:
/// `nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:6760`, `:9232`). It is the
/// ONLY thing that distinguishes a GR channel from a
/// CE channel on the wire: both are `AMPERE_CHANNEL_GPFIFO_A`. The core learns a
/// channel's engine from `Arch::classify(class)` refined by its engine object
/// (`kayfabe_core::project`), never from this field — so dropping it costs
/// nothing *today* and is recorded because it is a declared fact we discard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelAllocFacts {
    /// `flags` @ +20 — the `NVOS04_FLAGS_*` word. Opaque here: the arch recovers
    /// the channel's `VChid` from it (`kayfabe_arch::Arch::vchid_from_userd_flags`),
    /// and this crate does not interpret a single bit of it.
    pub flags: u32,
    /// `hContextShare` @ +24. 0 = the channel declares no context share.
    pub h_ctx_share: u32,
    /// `hVASpace` @ +28. 0 = the channel declares no VASpace of its own, in which
    /// case its VAS is resolved through the context share or the parent TSG.
    pub h_vaspace: u32,
}

/// Where a page directory lives, from `NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_FLAGS_APERTURE`
/// (`flags[1:0]`, `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl0080/ctrl0080dma.h:842-845` /
/// `ogkm-610: src/common/sdk/nvidia/inc/ctrl/ctrl0080/ctrl0080dma.h:812-815` — identical
/// values at both).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdbAperture {
    /// `_VIDMEM` (0) — the root is in framebuffer.
    Vidmem,
    /// `_SYSMEM_COH` (1) — coherent system memory, i.e. guest RAM.
    SysmemCoherent,
    /// `_SYSMEM_NONCOH` (2) — non-coherent system memory, i.e. guest RAM.
    SysmemNoncoherent,
    /// A value NVIDIA has not defined. Kept as a distinct variant rather than
    /// folded into a default, because folding is how a new aperture silently
    /// becomes "vidmem" and every walk from that PDB reads the wrong memory.
    Undefined(u32),
}

impl PdbAperture {
    /// Decode from the two-bit aperture field.
    #[must_use]
    pub fn from_flags(flags: u32) -> Self {
        match flags & 0x3 {
            0 => Self::Vidmem,
            1 => Self::SysmemCoherent,
            2 => Self::SysmemNoncoherent,
            other => Self::Undefined(other),
        }
    }

    /// `true` for everything that is **not** the VIDMEM aperture — i.e. exactly
    /// the C emulator's own predicate, `(flags & 0x3) != 0`
    /// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2534`).
    ///
    /// ★ Note the `Undefined` arm is included, and deliberately so. It was
    /// written as `matches!(self, SysmemCoherent | SysmemNoncoherent)` first and
    /// the differential test against the C predicate caught the difference at
    /// `flags == 3`. Two reasons the C is right here:
    ///
    /// - the C emulator is this project's differential oracle, and a predicate
    ///   that silently disagrees with it at one input is worse than one that
    ///   agrees everywhere;
    /// - the two failures are not symmetric. Treating an unknown aperture as
    ///   *sysmem* walks guest RAM, where a wrong root misses and faults loudly;
    ///   treating it as *vidmem* walks emulated framebuffer, where a wrong root
    ///   can read whatever happens to be there.
    ///
    /// A caller that wants to refuse an unknown aperture outright still can —
    /// [`PdbAperture::Undefined`] survives as its own variant precisely so this
    /// `bool` is never the only thing anyone can ask.
    #[must_use]
    pub fn is_sysmem(self) -> bool {
        !matches!(self, Self::Vidmem)
    }
}

/// `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` payload — the `RmEvent::SetPageDir`
/// facts. Where a VAS's data-plane identity is born.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetPageDir {
    /// `physAddress` — the page-directory base.
    pub phys_address: u64,
    /// `numEntries`.
    pub num_entries: u32,
    /// The decoded aperture (`flags[1:0]`).
    pub aperture: PdbAperture,
    /// The raw `flags`, kept because bits above [1:0] carry `PRESERVE_PDES` and
    /// friends that this milestone does not interpret.
    pub flags: u32,
    /// `hVASpace` — the VASpace this page directory belongs to.
    pub h_vaspace: u32,
}

/// The GSP-RPC envelope (`rpc_message_header_v03_00`), validated.
///
/// `length` is **guest-written** and is used to find the payload, so it is
/// checked against the buffer before anything else touches it. An RPC whose
/// declared length exceeds its buffer is [`AbiError::RpcLength`], never a
/// clamped read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcEnvelope {
    /// `header_version` — `0x0300_0000` is MAJOR 3 / MINOR 0.
    pub header_version: u32,
    /// `signature` — see [`RpcEnvelope::SIGNATURE_VALID`].
    pub signature: u32,
    /// `length` — total message length **including** this 32-byte header.
    pub length: u32,
    /// `function` — an `NV_VGPU_MSG_FUNCTION_*` or `NV_VGPU_MSG_EVENT_*` id.
    pub function: u32,
    /// `rpc_result`.
    pub rpc_result: u32,
    /// `rpc_result_private`.
    pub rpc_result_private: u32,
    /// `sequence`.
    pub sequence: u32,
    /// Payload length, i.e. `length - 32`. Derived, and derived *safely*.
    pub payload_len: usize,
}

impl RpcEnvelope {
    /// `NV_VGPU_MSG_SIGNATURE_VALID` — ASCII `"VRPC"` little-endian.
    ///
    /// Cross-checked against the C emulator, which writes the same word at the
    /// same offset (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:1585`,
    /// `stl_le_p(el + 52, 0x43505256u)` where `el + 48` is the start of the RPC
    /// header, so this is `signature` at +4).
    pub const SIGNATURE_VALID: u32 = 0x4350_5256;

    /// The envelope's own size — `sizeof(rpc_message_header_v03_00)`.
    ///
    /// ★ This is **32**, and it is worth stating loudly: the C emulator's
    /// `nvkvm_m3_post_status` writes `rpc.length = 36` for a bare header with the
    /// comment `/* length = sizeof(rpc_message_header) */`
    /// (`src/qemu/nvkvm_gpu_emul.c:1586`), while the same file's own offset
    /// arithmetic uses 32 (`:1637` "32-byte rpc_message_header … el+48+32 =
    /// el+80", and `:1657` `rpc.length = hdr(32) + body(32)`). 32 is right; the
    /// 36 is a stale constant that only survives because the message is
    /// zero-padded and both sides checksum the *declared* length.
    pub const SIZE: usize = 32;
}

/// A helper for [`RpcEnvelope`] decoding shared by [`crate::versions`].
///
/// # Errors
///
/// [`AbiError::RpcLength`] if `declared` is smaller than the envelope or larger
/// than `available`.
pub(crate) fn rpc_payload_len(declared: u32, available: usize) -> Result<usize, AbiError> {
    let declared_usize = declared as usize;
    if declared_usize < RpcEnvelope::SIZE || declared_usize > available {
        return Err(AbiError::RpcLength {
            declared,
            available,
        });
    }
    Ok(declared_usize - RpcEnvelope::SIZE)
}

// ─────────────────────────── NV2080_CTRL_CMD_GPU_PROMOTE_CTX ───────────────────────────

/// ★★ **One `promoteEntry[i]`, classified into the three states the protocol can
/// actually produce.**
///
/// # Why this is an enum and not the wire struct with `Option`s
///
/// `kgrobjPromoteContext` zeroes the whole params struct and then runs **two independent
/// preparers into the same entry slot**
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c:90-124`):
///
/// - the *initialize* preparer writes `gpuPhysAddr`, `size`, `physAttr`, `bufferId`,
///   `bInitialize = 1`, `bNonmapped = 1` — and **never touches `gpuVirtAddr`**
///   (`kernel_graphics_context.c:1843-1849`);
/// - the *promote* preparer writes `bufferId`, `gpuVirtAddr`, `bNonmapped = 0` — and
///   **never touches `gpuPhysAddr`, `size`, `physAttr` or `bInitialize`**
///   (`kernel_graphics_context.c:1949-1955`).
///
/// Either may decline and write nothing. So an entry on the wire is exactly one of three
/// things, and the zeroes in it are **absence of a fact, not a fact**:
///
/// | state | phys | size | va | variant |
/// |---|---|---|---|---|
/// | initialize-only | set | set | 0 | [`Self::InitializeOnly`] |
/// | promote-only | **0** | **0** | set | [`Self::PromoteOnly`] |
/// | both | set | set | set | [`Self::Promotable`] |
///
/// ★★★ **`gpuPhysAddr == 0 && size == 0` in a promote-only entry means "not supplied".**
/// Binding `va → phys 0` would be manufacturing an address, which is exactly what
/// MISS = FAULT forbids; and refusing the entry as malformed would reject legitimate
/// guest traffic (it is the ordinary multi-channel-TSG case). It is therefore *named and
/// counted*, never bound and never silently skipped. The C artifact's
/// `if (!va || !sz || bNonmapped) continue;`
/// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2468`) discards it without a name
/// or a count — 4 of the 9 entries in the repo's own captured blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromoteEntry {
    /// Both preparers ran: a complete VA → phys mapping. The **only** bindable state.
    Promotable {
        /// `gpuVirtAddr`. Never 0 in this variant.
        va: u64,
        /// `size`. Never 0 in this variant.
        len: u64,
        /// `gpuPhysAddr`, in the aperture named by [`Self::Promotable::aperture`]. For
        /// `Vidmem` this is a **guest** framebuffer offset.
        phys: u64,
        /// `physAttr[1:0]`, decoded.
        aperture: kayfabe_arch::Aperture,
        /// `bufferId` — `NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_*`. **Two bytes wide**;
        /// carried rather than dropped, so MAIN/PATCH/PRIV_ACCESS_MAP stay
        /// distinguishable.
        buffer_id: u16,
    },
    /// The initialize preparer ran and the promote preparer declined (or was not
    /// attempted): a physical buffer that declares **no** VA and says so via
    /// `bNonmapped`. Not bindable — there is no address to bind.
    InitializeOnly {
        /// `gpuPhysAddr`.
        phys: u64,
        /// `size`.
        len: u64,
        /// `physAttr[1:0]`, decoded.
        aperture: kayfabe_arch::Aperture,
        /// `bufferId`.
        buffer_id: u16,
    },
    /// The promote preparer ran and the initialize preparer declined: a VA for a buffer
    /// initialized against some *other* channel/VAS. Not bindable — `phys` and `size`
    /// were never written, so there is nothing to point it at.
    PromoteOnly {
        /// `gpuVirtAddr`.
        va: u64,
        /// `bufferId`.
        buffer_id: u16,
    },
}

impl PromoteEntry {
    /// `bufferId`, whichever state this is.
    #[must_use]
    pub const fn buffer_id(self) -> u16 {
        match self {
            Self::Promotable { buffer_id, .. }
            | Self::InitializeOnly { buffer_id, .. }
            | Self::PromoteOnly { buffer_id, .. } => buffer_id,
        }
    }
}

/// ★ **The classifier** — one wire entry to one [`PromoteEntry`] state. Pure, total, and
/// the single place §2.3's *"zero means not-supplied"* reading is applied.
///
/// # The rules, in order, and why the order is the order
///
/// 1. **`bNonmapped != 0` ⇒ [`PromoteEntry::InitializeOnly`], whatever `gpuVirtAddr`
///    says.** The flag dominates the value: NVIDIA's own comment is *"the virtual
///    address is not to be promoted with this call"*, and a hostile guest can set the
///    flag **and** a plausible VA. Value-first would bind it.
/// 2. `va != 0 && size != 0` ⇒ [`PromoteEntry::Promotable`] — both preparers ran.
/// 3. `va != 0` (so `size == 0`) ⇒ [`PromoteEntry::PromoteOnly`] — the promote preparer
///    ran alone and never wrote `gpuPhysAddr`/`size`. **`size == 0` is not malformed
///    input here**; it is a field this pass does not write.
/// 4. otherwise ⇒ [`PromoteEntry::InitializeOnly`] — the entry declares no VA at all.
///
/// # ★ `physAttr` is decoded only where the protocol wrote it
///
/// The promote preparer never touches `physAttr`, so on a [`PromoteEntry::PromoteOnly`]
/// entry the field is the struct's pre-zeroed initial value. Refusing an undefined
/// aperture *there* would be reading an absence as a fact — the same mistake §2.3 forbids
/// one field over. The [`crate::wire::AbiError::PromoteAperture`] refusal therefore fires
/// on exactly the two states that carry an aperture.
///
/// # Errors
///
/// [`crate::wire::AbiError::PromoteAperture`] when `physAttr[1:0] == 3` on an entry whose
/// state carries an aperture.
pub fn classify_promote_entry(
    index: usize,
    e: &crate::generated::ctrl::Nv2080CtrlGpuPromoteCtxBufferEntry,
) -> Result<PromoteEntry, AbiError> {
    let aperture = |phys_attr: u32| -> Result<kayfabe_arch::Aperture, AbiError> {
        match phys_attr & 0x3 {
            0 => Ok(kayfabe_arch::Aperture::Vidmem),
            1 => Ok(kayfabe_arch::Aperture::SysmemCoherent),
            2 => Ok(kayfabe_arch::Aperture::SysmemNonCoherent),
            _ => Err(AbiError::PromoteAperture {
                entry: index,
                phys_attr,
            }),
        }
    };
    if e.b_nonmapped != 0 {
        return Ok(PromoteEntry::InitializeOnly {
            phys: e.gpu_phys_addr,
            len: e.size,
            aperture: aperture(e.phys_attr)?,
            buffer_id: e.buffer_id,
        });
    }
    if e.gpu_virt_addr != 0 && e.size != 0 {
        return Ok(PromoteEntry::Promotable {
            va: e.gpu_virt_addr,
            len: e.size,
            phys: e.gpu_phys_addr,
            aperture: aperture(e.phys_attr)?,
            buffer_id: e.buffer_id,
        });
    }
    if e.gpu_virt_addr != 0 {
        return Ok(PromoteEntry::PromoteOnly {
            va: e.gpu_virt_addr,
            buffer_id: e.buffer_id,
        });
    }
    Ok(PromoteEntry::InitializeOnly {
        phys: e.gpu_phys_addr,
        len: e.size,
        aperture: aperture(e.phys_attr)?,
        buffer_id: e.buffer_id,
    })
}

/// A decoded `NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS`.
///
/// ★ The two client handles are BOTH carried, because they do two different jobs
/// (`gpu_promote_ctx.md` §6.2):
///
/// | job | field |
/// |---|---|
/// | **namespace attribution** — which client is acting | the RPC **envelope**'s `hClient`, which this struct does not hold and the bridge must not replace |
/// | **object resolution** — whose handle table `hObject` is a handle in | [`Self::h_chan_client`] |
///
/// The C artifact reads `hChanClient` and never looks at the envelope at all
/// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2447`), so it cannot notice a
/// disagreement between them, let alone refuse one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromoteCtx {
    /// `engineType` — which engine's virtual context this is.
    pub engine_type: u32,
    /// `hChanClient` — the namespace [`Self::h_object`] is a handle in.
    pub h_chan_client: u32,
    /// `hObject` — a channel **or** a channel group (TSG).
    pub h_object: u32,
    /// The decoded entries, in wire order. Sparse-safe: iterate with
    /// [`Self::entries`], which never depends on a prefix invariant.
    entries: [Option<PromoteEntry>; MAX_PROMOTE_ENTRIES],
}

/// `NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES`, re-exported at the view layer so a
/// caller sizing a buffer does not have to name the generated module.
pub const MAX_PROMOTE_ENTRIES: usize =
    crate::generated::ctrl::NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES;

impl PromoteCtx {
    /// Build from a decoded prefix and the entries that survived decoding.
    pub(crate) fn new(
        engine_type: u32,
        h_chan_client: u32,
        h_object: u32,
        entries: [Option<PromoteEntry>; MAX_PROMOTE_ENTRIES],
    ) -> Self {
        Self {
            engine_type,
            h_chan_client,
            h_object,
            entries,
        }
    }

    /// The decoded entries, in wire order.
    pub fn entries(&self) -> impl Iterator<Item = PromoteEntry> + '_ {
        self.entries.iter().flatten().copied()
    }

    /// How many entries decoded — i.e. the guest's `entryCount`, after the
    /// [`crate::wire::AbiError::PromoteEntryCount`] bound has already refused anything
    /// above [`MAX_PROMOTE_ENTRIES`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    /// `true` when the control declared no entries at all.
    ///
    /// A legal, if unusual, message — `entryCount == 0` with the legacy fields also zero
    /// is a well-formed no-op, and it is decoded rather than refused.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// ★ **The classification, counted** — the three-way split of `entries()` reduced to
    /// numbers, so a caller that binds only [`PromoteEntry::Promotable`] can still report
    /// what it dropped and why.
    ///
    /// This exists because the C's defect D3 was not the *behaviour* (a promote-only
    /// entry is structurally unbindable — `AddressTable::bind` refuses a zero length)
    /// but the **silence**: a forced outcome that is never named reads as an intentional
    /// decision it is not.
    #[must_use]
    pub fn census(&self) -> PromoteCensus {
        let mut c = PromoteCensus::default();
        for e in self.entries() {
            match e {
                PromoteEntry::Promotable { .. } => c.promotable += 1,
                PromoteEntry::InitializeOnly { .. } => c.initialize_only += 1,
                PromoteEntry::PromoteOnly { .. } => c.promote_only += 1,
            }
        }
        c
    }
}

/// How many entries of each state one `PROMOTE_CTX` carried. See
/// [`PromoteCtx::census`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PromoteCensus {
    /// Complete mappings — the bindable ones.
    pub promotable: u32,
    /// Physical-only declarations (`bNonmapped`), which name no VA.
    pub initialize_only: u32,
    /// VA-only declarations, whose phys/size were never written by the producer.
    pub promote_only: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every defined aperture decodes to its own variant, and an undefined one
    /// stays undefined rather than becoming vidmem. Kills the `&`→`|`,
    /// `0x3`→`0x1`, and match-arm-collapse mutants in one predicate.
    #[test]
    fn every_aperture_encoding_decodes_to_its_own_variant() {
        assert_eq!(PdbAperture::from_flags(0), PdbAperture::Vidmem);
        assert_eq!(PdbAperture::from_flags(1), PdbAperture::SysmemCoherent);
        assert_eq!(PdbAperture::from_flags(2), PdbAperture::SysmemNoncoherent);
        assert_eq!(PdbAperture::from_flags(3), PdbAperture::Undefined(3));
        // Bits above [1:0] must not leak into the decision: PRESERVE_PDES is
        // bit 2 and must leave the aperture alone.
        assert_eq!(PdbAperture::from_flags(0x0000_0004), PdbAperture::Vidmem);
        assert_eq!(PdbAperture::from_flags(0xFFFF_FFFC), PdbAperture::Vidmem);
        assert_eq!(
            PdbAperture::from_flags(0xFFFF_FFFD),
            PdbAperture::SysmemCoherent
        );
        assert_eq!(
            PdbAperture::from_flags(0xFFFF_FFFF),
            PdbAperture::Undefined(3)
        );
    }

    /// `is_sysmem` agrees with the C emulator's `(flags & 0x3) != 0` predicate on
    /// every one of the four encodings — including the undefined one, where the C
    /// would say "sysmem" and we say so too, deliberately, because a walk of an
    /// unknown-aperture root through guest RAM at least faults loudly.
    #[test]
    fn is_sysmem_matches_the_c_emulators_predicate_on_all_four_encodings() {
        for flags in 0u32..4 {
            let c_says = (flags & 0x3) != 0;
            assert_eq!(
                PdbAperture::from_flags(flags).is_sysmem(),
                c_says,
                "flags={flags} disagrees with nvkvm_gpu_emul.c:2534"
            );
        }
        assert!(!PdbAperture::Vidmem.is_sysmem());
        assert!(PdbAperture::Undefined(3).is_sysmem());
    }

    /// The payload length is derived by subtraction, and every way that
    /// subtraction could go wrong is refused first.
    #[test]
    fn rpc_payload_len_refuses_every_impossible_declaration() {
        assert_eq!(
            rpc_payload_len(32, 32),
            Ok(0),
            "a bare header has no payload"
        );
        assert_eq!(rpc_payload_len(64, 4096), Ok(32));
        assert_eq!(rpc_payload_len(4096, 4096), Ok(4064));
        // Shorter than the header itself — the underflow case.
        assert_eq!(
            rpc_payload_len(31, 4096),
            Err(AbiError::RpcLength {
                declared: 31,
                available: 4096
            })
        );
        assert_eq!(
            rpc_payload_len(0, 4096),
            Err(AbiError::RpcLength {
                declared: 0,
                available: 4096
            })
        );
        // Longer than the buffer — the over-read case.
        assert_eq!(
            rpc_payload_len(4097, 4096),
            Err(AbiError::RpcLength {
                declared: 4097,
                available: 4096
            })
        );
        assert_eq!(
            rpc_payload_len(u32::MAX, 4096),
            Err(AbiError::RpcLength {
                declared: u32::MAX,
                available: 4096
            })
        );
    }
}
