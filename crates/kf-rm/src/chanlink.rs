//! ★★★ **The channel plane's seat in the served chain** (`V3_P5_PORT_MAP.md` §2.1).
//!
//! The guest's channels are ALLOCATED, SCHEDULED and TOKENED through RPCs we answer. On the GSP
//! model the guest's CPU-RM does only bookkeeping and RPCs the rest — *"All real hardware
//! management is done in the host"* (`ogkm-580: kernel_channel.c:3105-3130`) — so the runlist
//! write, the RAMFC and the doorbell's meaning are OURS. This link carries each of those
//! statements to the device's channel plane (`kf-qemu`), which births the host twin, and answers
//! with what the plane did:
//!
//! | RPC | this link | the plane |
//! |---|---|---|
//! | `GSP_RM_ALLOC` of the GPFIFO channel class | carries the declaration ([`ChannelAlloc`]); **refuses the alloc** if the plane refused the birth, else declines (the object seat records it) | births the host twin **at allocation** (§7: never lazily) |
//! | `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`0xa06f0103`) | answers `NV_OK` + the `[IN]` echo **iff** the plane scheduled | `GPFIFO_SCHEDULE` on the host twin |
//! | `NVA06F_CTRL_CMD_BIND` (`0xa06f0104`) | answers `NV_OK` + the `[IN]` echo iff the plane owns the twin and the engine is a copy engine | nothing more: the twin's host TSG was bound at birth |
//! | `NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` (`0xc36f0108`) | answers the plane's GUEST token | a token-table slot routed to the twin |
//! | `GSP_RM_FREE` | observes | retires the token, frees the twin |
//! | `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` (`0x2080012b`) | answers `NV_OK` + the `[IN]` echo iff the plane owns a GR twin holding a host GR object | records the channel's context BOUND — **satisfied by the twin**, never forwarded, no guest byte touched |
//! | `NV2080_CTRL_CMD_GPU_EVICT_CTX` (`0x2080012c`) | answers `NV_OK` + echo iff the plane did it | takes the twin off the host runlist; records UNBOUND |
//!
//! ⊘ **Nothing here is answered without the plane having acted.** `0xa06f0103` stayed unserviced
//! for weeks precisely because an `NV_OK` with no host act behind it is a fabricated completion
//! (`sweep.rs` row `0xa06f_0103`); the answer now IS the act's result.
//!
//! ⊘ A channel this plane does not own (no statement accepted for it) gets `None` from every arm,
//! so the chain's other links and the unserviced ledger see it exactly as before.

use kf_abi::versions::{AllocParams, DriverAbiTable};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// `NV_OK`.
const NV_OK: u32 = 0;
/// `NV_ERR_INVALID_ARGUMENT`.
const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
/// `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`ogkm-580: ctrla06fgpfifo.h:69`).
pub const GPFIFO_SCHEDULE: u32 = 0xa06f_0103;
/// `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` — the TSG form (`ctrla06c.h`).
pub const TSG_GPFIFO_SCHEDULE: u32 = 0xa06c_0101;
/// `NVA06F_CTRL_CMD_BIND` (`ogkm-580: ctrla06fgpfifo.h:96`) — one `[IN]` `engineType`.
pub const BIND: u32 = 0xa06f_0104;
/// `NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` (`ogkm-580: ctrlc36f.h:79`).
pub const GET_WORK_SUBMIT_TOKEN: u32 = 0xc36f_0108;
/// `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` (`ogkm-580: ctrl2080gpu.h:984`).
pub const PROMOTE_CTX: u32 = 0x2080_012b;
/// `NV2080_CTRL_CMD_GPU_EVICT_CTX` (`ogkm-580: ctrl2080gpu.h:1003-1035`) — `{engineType, hClient,
/// ChID, hChanClient, hObject}`, all `[IN]`.
pub const EVICT_CTX: u32 = 0x2080_012c;
/// `sizeof(NV2080_CTRL_GPU_EVICT_CTX_PARAMS)`.
const EVICT_CTX_PARAMS_SIZE: usize = 20;
/// `NVA06F_CTRL_CMD_STOP_CHANNEL` (`ogkm-580: ctrla06fgpfifo.h:237`) — `{bImmediate}`.
pub const STOP_CHANNEL: u32 = kf_abi::submit::NVA06F_CTRL_CMD_STOP_CHANNEL;
/// `NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS` (`ogkm-580: ctrl2080fifo.h:339`).
pub const DISABLE_CHANNELS: u32 = kf_abi::submit::NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS;
/// `NVA06C_CTRL_CMD_PREEMPT` (`ogkm-580: ctrla06c.h:203`).
pub const TSG_PREEMPT: u32 = kf_abi::submit::NVA06C_CTRL_CMD_PREEMPT;
/// `NV_ERR_INSUFFICIENT_PERMISSIONS`.
const NV_ERR_INSUFFICIENT_PERMISSIONS: u32 = 0x1b;
/// `GT200_DEBUGGER` (`ogkm-580: resource_list.h:186-196`, parent `Device`,
/// `NV83DE_ALLOC_PARAMETERS` required).
pub const GT200_DEBUGGER: u32 = 0x83de;
/// `GF100_ZBC_CLEAR` (`ogkm-580: resource_list.h:820-830`, parent `Subdevice`, no params).
pub const GF100_ZBC_CLEAR: u32 = 0x9096;
/// `GF100_DISP_SW` (`ogkm-580: resource_list.h:1502-1511`, parent `KernelChannel`).
pub const GF100_DISP_SW: u32 = 0x9072;
/// `NV83DE_ALLOCATION_PARAMETERS` size: `{hDebuggerClient_Obsolete, hAppClient, hClass3dObject}`
/// (`ogkm-580: class/cl83de.h:51-55`).
const NV83DE_ALLOC_PARAMS_SIZE: usize = 12;
/// `NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK` — a 4-byte `exceptionMask`, an RM-internal event
/// filter that programs no hardware (`ctrl83dedebug.h:158-231`).
pub const DEBUG_SET_EXCEPTION_MASK: u32 = 0x83de_0309;
/// `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` (`ctrl2080gr.h:818-826`), 32 bytes, all `[IN]`.
pub const GR_SET_CTXSW_PREEMPTION_MODE: u32 = 0x2080_1210;
/// ★ v3-gfx: `NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND` (`ctrl2080gr.h:603-608`) —
/// `{hClient, hChannel, NvU64 vMemPtr, zcullMode}`, 24 bytes, all `[IN]`.
pub const GR_CTXSW_ZCULL_BIND: u32 = 0x2080_1208;
/// `NV2080_CTRL_CTXSW_ZCULL_MODE_SEPARATE_BUFFER` — the highest mode (`ctrl2080gr.h:453-455`).
const ZCULL_MODE_MAX: u32 = 2;
/// `NVA06C_CTRL_CMD_SET_TIMESLICE` (`ctrla06c.h:146-152`): `{NvU64 timesliceUs}`.
pub const TSG_SET_TIMESLICE: u32 = 0xa06c_0103;
/// `NV0080_CTRL_CMD_INTERNAL_PERF_CUDA_LIMIT_SET_CONTROL` (`ctrl0080internal.h:76`) — the guest
/// kernel's per-Device CUDA-limit edge (`kern_cuda_limit.c`: sent only when that Device's setting
/// CHANGES), `{NvBool bCudaLimit}`.
pub const PERF_CUDA_LIMIT_SET_CONTROL: u32 = 0x0080_2009;
/// `NV0080_CTRL_CMD_INTERNAL_PERF_CUDA_LIMIT_DISABLE` (`ctrl0080internal.h:82`) — no params, sent
/// on the guest's INTERNAL device at a Device's teardown (`kern_cuda_limit.c:47-75`).
pub const PERF_CUDA_LIMIT_DISABLE: u32 = 0x0080_2004;

/// ★ v3-chanctl: a `DISABLE_CHANNELS` list as a `Copy` value (`(hClient, hChannel)` × `n`, ≤ 64).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChanList {
    n: u8,
    items: [(u32, u32); kf_abi::submit::DISABLE_CHANNELS_MAX_ENTRIES],
}

impl ChanList {
    /// A list of at most 64 entries (`None` beyond).
    #[must_use]
    pub fn new(v: &[(u32, u32)]) -> Option<ChanList> {
        if v.len() > kf_abi::submit::DISABLE_CHANNELS_MAX_ENTRIES {
            return None;
        }
        let mut items = [(0, 0); kf_abi::submit::DISABLE_CHANNELS_MAX_ENTRIES];
        items[..v.len()].copy_from_slice(v);
        Some(ChanList { n: v.len() as u8, items })
    }

    /// The entries.
    #[must_use]
    pub fn as_slice(&self) -> &[(u32, u32)] {
        &self.items[..usize::from(self.n)]
    }
}

/// ★ What the guest declared when it allocated a GPFIFO channel — every field off the wire,
/// nothing inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelAlloc {
    /// `hClient`.
    pub client: u32,
    /// `hParent` (a device, or a TSG).
    pub parent: u32,
    /// The channel's handle.
    pub handle: u32,
    /// The class id.
    pub class: u32,
    /// `gpFifoOffset` — a GPU VA in the channel's VA space.
    pub gpfifo_va: u64,
    /// `gpFifoEntries`.
    pub entries: u32,
    /// `hVASpace` as declared (0 = the device's default / the TSG's).
    pub h_vaspace: u32,
    /// ★ The VA-space object the channel runs in, RESOLVED: `hVASpace` when non-zero, else the
    /// first `FERMI_VASPACE_A` allocated under the channel's parent device in this client (the
    /// device-default VAS RM creates lazily — the PMA scrubber's, `hVASpace = 0`). `None` when
    /// neither is known (refused by name at birth).
    pub vaspace: Option<u32>,
    /// ★ v3-gfx: the client whose namespace holds [`Self::vaspace`] — the channel's own, unless
    /// the VA space it names is a `DUP_OBJECT` alias, in which case the ORIGINAL's (the one the
    /// page-directory statement named). `[measured vgfx 2026-09-26]` the Vulkan UMD allocs the VA
    /// space in a probe client and dups it into its device; its context share names the dup.
    pub vaspace_client: u32,
    /// ★ The guest's own channel id, off `flags` `USERD_INDEX` ([`decode_userd_index_chid`]).
    pub chid: Option<u32>,
    /// `flags` (`NVOS04_FLAGS_*`).
    pub flags: u32,
    /// `engineType`, raw, when the layout is pinned.
    pub engine_type: Option<u32>,
    /// The guest kernel's resolved USERD descriptor (`userdMem`).
    pub userd: Option<kf_arch::UserdMem>,
    /// ★★★ A guest-KERNEL channel (§7's policy, and the Translated route) — **PROPOSED Q7 rule**
    /// (`V3_P5_PORT_MAP.md` Q7, pending the owner): the guest's CPU-RM stamped
    /// `internalFlags.PRIVILEGE = KERNEL` on the alloc ([`kf_abi::notifier::ChannelPrivilege`]:
    /// RM zeroes the caller's `internalFlags` and recomputes the level from the call's security
    /// context, which no userspace ioctl can make `KERNEL`), OR the client is one of the guest
    /// RM's own internal clients (`serverIsClientInternal`). ⊘ Never the pid sentinel
    /// ([`ChannelAlloc::declared_kernel_pid`]) and never `flags.PRIVILEGED_CHANNEL` — both reach
    /// the wire as guest userspace wrote them.
    pub kernel_client: bool,
    /// ★ Q7: the privilege level off the wire (`None`: no pinned layout / short params).
    pub privilege: Option<kf_abi::notifier::ChannelPrivilege>,
    /// ★ P5b: the client's root alloc declared `KERNEL_PID` — logged, NOT trusted for the route
    /// (guest userspace reached it, see [`ChannelPolicy`]'s alloc decode).
    pub declared_kernel_pid: bool,
    /// ★ P5b: the channel group it was allocated under (`hParent`), when that is a TSG this link
    /// saw allocated — the group `GPFIFO_SCHEDULE` names.
    pub tsg: Option<u32>,
    /// ★ v3-int: `hContextShare` as declared (0 = none: the group's legacy subcontext). The
    /// channel plane mirrors a guest TSG as one host group PER CONTEXT SHARE: CUDA's TSG is one
    /// ctxshare (all members share one GR context), a Vulkan TSG carries a graphics and an
    /// async-compute ctxshare (distinct subcontexts, which one legacy host subcontext cannot be).
    pub ctx_share: u32,
    /// ★ v3-promote: the DEVICE the channel hangs off (its parent, or its group's parent) — a
    /// guest free of the device takes the channel with it, so the plane must match it.
    pub device: u32,
    /// ★ P5c: the error notifier the guest kernel resolved for it (`errorNotifierMem`) — where a
    /// GSP writes the channel's robust-channel record (`kernel_channel.c:548-590`).
    pub error_notifier: Option<kf_arch::fault::ErrorNotifier>,
}

/// A statement for the channel plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChanStatement {
    /// A GPFIFO channel alloc.
    Alloc(ChannelAlloc),
    /// `GPFIFO_SCHEDULE` on a channel (`0xa06f0103`) or a TSG (`0xa06c0101`).
    Schedule {
        /// `hClient`.
        client: u32,
        /// The channel or TSG.
        object: u32,
        /// `bEnable`.
        enable: bool,
    },
    /// `BIND` a channel to an engine (`kchannelBindToRunlist`, `kernel_channel.c:2878-2886`).
    Bind {
        /// `hClient`.
        client: u32,
        /// The channel.
        object: u32,
        /// `engineType` (`NV2080_ENGINE_TYPE_*`).
        engine_type: u32,
    },
    /// ★ P5b: an ENGINE object (a copy, compute or 3D class — `kf_chip`'s generated sets)
    /// allocated under a channel.
    EngineObject {
        /// `hClient`.
        client: u32,
        /// The channel (`hParent`).
        parent: u32,
        /// The object's handle.
        handle: u32,
        /// Its class.
        class: u32,
        /// ★ w827: a COPY class's declared `NVB0B5_ALLOCATION_PARAMETERS` engine, as an
        /// `NV2080_ENGINE_TYPE_COPY(i)` ordinal (`CeAllocParams::declared_copy_engine_type`);
        /// `None` for compute/3D or an undeclarable one. CUDA allocates its copy class ON ITS GR
        /// CHANNEL naming a GRCE, and that GRCE is the only thing that says which engine runs it.
        copy_engine: Option<u32>,
    },
    /// ★ w827: a `GT200_DEBUGGER` session under a device, bound to the guest's GR object
    /// `obj3d` of client `app_client` — what `cuCtxCreate` allocates on every context
    /// (`[measured traces/host_reference_ga106 ctx_r1 i=401]`, then `SET_EXCEPTION_MASK` at i=425).
    Debugger {
        /// `hClient`.
        client: u32,
        /// The device (`hParent`).
        parent: u32,
        /// The session's handle.
        handle: u32,
        /// `hAppClient`.
        app_client: u32,
        /// `hClass3dObject` — a compute/3D object in `app_client`.
        obj3d: u32,
    },
    /// ★ w827: `DEBUG_SET_EXCEPTION_MASK` on a debugger session.
    DebuggerExceptionMask {
        /// `hClient`.
        client: u32,
        /// The session.
        object: u32,
        /// `exceptionMask` (`NV83DE_CTRL_DEBUG_SET_EXCEPTION_MASK_*`).
        mask: u32,
    },
    /// ★ w827: `GR_SET_CTXSW_PREEMPTION_MODE` for the guest's channel or TSG `channel` — what
    /// `cuCtxCreate` asks right after its debugger (`[measured host_reference_ga106 ctx_r1 i=426]`:
    /// `flags=CILP, cilpPreemptMode=CILP` on the context's TSG).
    CtxswPreemption {
        /// `hClient`.
        client: u32,
        /// `hChannel` — a channel or (on the `cuCtxCreate` path) a TSG handle.
        channel: u32,
        /// `flags` (`_FLAGS_CILP` bit 0, `_FLAGS_GFXP` bit 1).
        flags: u32,
        /// `gfxpPreemptMode`.
        gfxp: u32,
        /// `cilpPreemptMode`.
        cilp: u32,
    },
    /// ★ v3-gfx: `GR_CTXSW_ZCULL_BIND` for the guest's GR channel (or every GR channel of its
    /// TSG) — the zcull context-switch mode and, for `SEPARATE_BUFFER`, the zcull buffer's GPU VA
    /// in the channel's VA space, which the twin shares (VA identity). `[measured vgfx 2026-09-26]`
    /// the host's own Vulkan run binds `mode=2` on every 3D channel.
    ZcullBind {
        /// `hClient` (the envelope's; the params' must equal it).
        client: u32,
        /// `hChannel`.
        channel: u32,
        /// `vMemPtr`.
        va: u64,
        /// `zcullMode` (`0..=2`).
        mode: u32,
    },
    /// ★ w827: `SET_TIMESLICE` on a TSG (`[measured ctx_r1 i=427]`: 2048 µs).
    Timeslice {
        /// `hClient`.
        client: u32,
        /// The TSG.
        object: u32,
        /// `timesliceUs`.
        us: u64,
    },
    /// ★ w827: the guest Device `(client, device)` turned its CUDA limit on/off.
    CudaLimit {
        /// `hClient`.
        client: u32,
        /// The Device.
        device: u32,
        /// `bCudaLimit`.
        enable: bool,
    },
    /// ★ w827: `PERF_CUDA_LIMIT_DISABLE` — a Device is being torn down; which one is stated only
    /// by the free that follows.
    CudaLimitDisable,
    /// `GET_WORK_SUBMIT_TOKEN` on a channel.
    Token {
        /// `hClient`.
        client: u32,
        /// The channel.
        object: u32,
    },
    /// ★★★ `GPU_PROMOTE_CTX` (`0x2080012b`) — the guest's CPU-RM declaring a GR channel's context
    /// buffers to "physical RM" (us). Owner ruling 2026-09-25: **satisfied by the twin**. The
    /// guest channel runs as a host twin whose OWN context buffers host RM allocated and promoted
    /// when we created its engine object; nothing here is forwarded (the promote is a privileged
    /// GSP-internal control), the guest's buffers are never read or written, and the statement is
    /// recorded as "context bound" for that channel. See `V3_P5_PORT_MAP.md` item 22.
    PromoteCtx {
        /// `hChanClient` — the namespace of `object` (NOT the envelope's client: RM issues the
        /// control under the SUBDEVICE's client, `kernel_graphics_object.c:130-139`,
        /// `nv_gpu_ops.c:10886-10891`).
        chan_client: u32,
        /// `hObject` — the channel.
        object: u32,
        /// `engineType`.
        engine_type: u32,
        /// `bufferId`s of entries that ask physical RM to INITIALIZE a buffer (`bInitialize`).
        initialize: u32,
        /// `bufferId`s of entries that promote a VA (`bNonmapped == 0` with a VA) — the UVM bind.
        with_va: u32,
        /// Entries decoded.
        entries: u32,
        /// ★ v3-video: a video FALCON promote's `(virtAddress, size)` — where the guest's CPU-RM
        /// mapped its own (never-executed) falcon context buffer. `None` for every GR promote.
        falcon_ctx: Option<(u64, u64)>,
    },
    /// `GPU_EVICT_CTX` (`0x2080012c`) — the unbind counterpart (`nvGpuOpsStopChannel`).
    EvictCtx {
        /// `hChanClient`.
        chan_client: u32,
        /// `hObject` — the channel.
        object: u32,
        /// `engineType`.
        engine_type: u32,
    },
    /// ★★★ v3-chanctl: `NVA06F_CTRL_CMD_STOP_CHANNEL` (`0xa06f0112`) — "disabling and unbinding
    /// the channel and removing it from runlist … If we fail to preempt … we RC the channel"
    /// (`ctrla06fgpfifo.h:216-231`): a NOT-RUNNING promise. The guest's CPU-RM RPCs it verbatim and,
    /// only on `NV_OK`, writes the channel's OWN notifier (`kchannelNotifyRc_HAL`,
    /// `kernel_channel.c:1958-1979`).
    Stop {
        /// `hClient`.
        client: u32,
        /// The channel.
        object: u32,
        /// `bImmediate`.
        immediate: bool,
    },
    /// ★★★ v3-chanctl: `NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS` (`0x2080110b`) on the subdevice.
    DisableChannels {
        /// `hClient` of the CALL (the subdevice's client).
        client: u32,
        /// `bDisable`.
        disable: bool,
        /// `bOnlyDisableScheduling`.
        only_scheduling: bool,
        /// `bRewindGpPut`.
        rewind_gp_put: bool,
        /// `(hClient, hChannel)` entries.
        list: ChanList,
    },
    /// ★★★ v3-chanctl: `NVA06C_CTRL_CMD_PREEMPT` (`0xa06c0105`) on a channel group — ROUTE_TO_PHYSICAL
    /// with no CPU-RM body (`g_kernel_channel_group_api_nvoc.c:273`), so the whole verb is ours.
    Preempt {
        /// `hClient`.
        client: u32,
        /// The channel group.
        object: u32,
        /// `bWait` as the guest asked (the host verb always waits: the held reply IS the preempt).
        wait: bool,
    },
    /// ★ v3-video: acquire (`0x20808163`) / release (`0x20808164`) one GPU-wide NVENC session slot
    /// (`kf_abi::gssreplay::GSS_ENC_SESSION_ACQUIRE`) — carried to OUR host client, never answered
    /// from a table: the slot is host state.
    EncoderSession {
        /// `hClient` of the call (the guest process's client).
        client: u32,
        /// Acquire (`true`) or release.
        acquire: bool,
    },
    /// An object was freed (maybe one of ours).
    Free {
        /// `hClient`.
        client: u32,
        /// The object (`hObjectOld`).
        object: u32,
    },
}

/// What the plane did with a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChanAnswer {
    /// Not a channel the plane owns: decline (let the chain answer as it did before).
    NotOurs,
    /// Done.
    Done,
    /// Done; the guest's work-submit token.
    Token(u32),
    /// ★ P5b: accepted, and the host act runs OFF the drainer (never under the GSP lock); the
    /// reply is HELD until the act resolves the cell with its status (`kf_gsp::Deferred`).
    Deferred(kf_gsp::Deferred),
    /// Refused by name, with the NV status the guest reads.
    Refused {
        /// `NV_ERR_*`.
        status: u32,
        /// Why.
        why: String,
    },
}

/// ★ Where statements go. Called on the register drainer (never a vCPU) under the GSP lock, so it
/// must not block: a statement whose answer is a host act returns [`ChanAnswer::Deferred`] and the
/// plane performs the act on its own thread (P5b) — the reply waits, the drainer does not.
pub type ChanSink = std::sync::Arc<dyn Fn(ChanStatement) -> ChanAnswer + Send + Sync>;

/// ★★★ The link. Seated at the FRONT of the chain (ahead of the object seat, which terminates
/// `GSP_RM_ALLOC`/`GSP_RM_FREE`, and of the ledger, which would record the controls unserviced).
pub struct ChannelPolicy {
    abi: DriverAbiTable,
    guest_os: kf_abi::GuestOs,
    sink: ChanSink,
    /// Clients that declared the kernel sentinel pid.
    kernel_clients: std::collections::BTreeSet<u32>,
    /// `(hClient, parent)` → the first `FERMI_VASPACE_A` allocated under it.
    vas_under: std::collections::BTreeMap<(u32, u32), u32>,
    /// `hClient` → every VA-space object a page-directory statement named in it. ★ The fallback for
    /// `hVASpace = 0` when the VAS alloc itself never reached us: the device-default VAS of an RM
    /// internal client is constructed CPU-side (`vaspaceGetByHandleOrDeviceDefault`), and only its
    /// `COPY_SERVER_RESERVED_PDES` is RPC'd (`[measured p5b]`: no `FERMI_VASPACE_A` alloc seen).
    vas_stated: std::collections::BTreeMap<u32, std::collections::BTreeSet<u32>>,
    /// ★ P5b: `(hClient, hTsg)` → `(parent device, hVASpace, engineType)` of every channel group
    /// allocated — a member channel's VA space (`hVASpace = 0` ⇒ the group's) and engine
    /// (`ENGINE_TYPE_NULL` ⇒ the group's, libcuda's CE channels).
    tsgs: std::collections::BTreeMap<(u32, u32), (u32, u32, u32)>,
    /// ★ P5b: `(hClient, hCtxShare)` → its `hVASpace`.
    ctxshares: std::collections::BTreeMap<(u32, u32), u32>,
    /// ★ v3-gfx: every VA-space object seen (alloc'd, or named by a page-directory statement).
    vas_objects: std::collections::BTreeSet<(u32, u32)>,
    /// ★ v3-gfx: `DUP_OBJECT` aliases of a VA-space object, `(dst client, dst handle)` → the
    /// ORIGINAL — the same relation `barpde::PageDirPolicy` keeps, needed here because a channel
    /// (via its context share) may name the alias while the root was stated for the original.
    vas_aliases: std::collections::BTreeMap<(u32, u32), (u32, u32)>,
    /// The deferred outcome of the command last carried (`CommandPolicy::defers`).
    pending: Option<kf_gsp::Deferred>,
    /// Statements carried.
    pub carried: u64,
    /// Allocs the plane refused.
    pub refused: u64,
}

impl ChannelPolicy {
    /// A link for one guest driver's wire.
    #[must_use]
    pub fn new(abi: DriverAbiTable, guest_os: kf_abi::GuestOs, sink: ChanSink) -> ChannelPolicy {
        ChannelPolicy {
            abi,
            guest_os,
            sink,
            kernel_clients: Default::default(),
            vas_under: Default::default(),
            vas_stated: Default::default(),
            tsgs: Default::default(),
            ctxshares: Default::default(),
            vas_objects: Default::default(),
            vas_aliases: Default::default(),
            pending: None,
            carried: 0,
            refused: 0,
        }
    }

    fn refusal(status: u32, why: &str, cmd: &RpcCommand) -> Reply {
        eprintln!("kf-rm: channel plane REFUSED ({status:#x}): {why}");
        Reply { rpc_result: status, body: cmd.payload.clone() }
    }

    fn on_alloc(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let body = cmd.wire_body();
        let h = self.abi.decode_rpc_alloc(body).ok()?;
        if self.abi.is_client_root_class(kf_arch::ids::ClassId(h.class)) {
            // Learn the client's kind from the object model's own decode (the sentinel pid).
            if let Ok(crate::rmrpc::Translation::Event(crate::rmgraph::RmEvent::Alloc { facts, .. })) =
                crate::rmrpc::translate(&self.abi, self.guest_os, cmd)
                && matches!(facts.client_kind, Some(kf_arch::ClientKind::Kernel))
            {
                // ⊘ `hClient`, not `hObject`: a root alloc's wire `hObject` is 0
                // (`[measured p5bc]` every root logged `client 0x0`), so P5's set held only `0` and
                // no guest-kernel client outside RM's internal handle range was ever kernel — the
                // real cause of p5a's "internal clients are not marked by the pid sentinel".
                self.kernel_clients.insert(h.client);
            }
            eprintln!(
                "kf-rm: chanlink: client {:#x} root: kernel={} internal={}",
                h.client,
                self.kernel_clients.contains(&h.client),
                is_rm_internal_client(h.client)
            );
            return None;
        }
        // ⊘ P5b: a class the boundary refuses is never carried — the object seat refuses it next,
        // and a twin born for it would be a host channel for an object that does not exist
        // (`[measured kf3m2]` the RC watchdog's `VOLTA_CHANNEL_GPFIFO_A` is such a class).
        if !self.abi.capabilities().alloc_class(kf_arch::ids::ClassId(h.class)).is_permitted() {
            return None;
        }
        if h.class == GT200_DEBUGGER {
            let w = |p: &[u8], i: usize| p.get(4 * i..4 * i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            let Some(params) = crate::rmrpc::alloc_params_window(&self.abi, body).filter(|p| p.len() == NV83DE_ALLOC_PARAMS_SIZE) else {
                return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, "GT200_DEBUGGER params are not NV83DE_ALLOC_PARAMETERS (12 bytes)", cmd));
            };
            // `hDebuggerClient_Obsolete` "must be zero" (cl83de.h:52) — RM refuses it otherwise.
            if w(params, 0) != Some(0) {
                return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, "GT200_DEBUGGER hDebuggerClient_Obsolete is not zero", cmd));
            }
            let st = ChanStatement::Debugger {
                client: h.client,
                parent: h.parent,
                handle: h.handle,
                app_client: w(params, 1)?,
                obj3d: w(params, 2)?,
            };
            self.carried += 1;
            return self.carry_alloc(st, cmd, h.client, h.handle);
        }
        match alloc_shape(&self.abi, h.class) {
            Some(AllocParams::VaSpace) => {
                self.vas_objects.insert((h.client, h.handle));
                let first = *self.vas_under.entry((h.client, h.parent)).or_insert(h.handle);
                eprintln!(
                    "kf-rm: chanlink: FERMI_VASPACE_A {:#x}:{:#x} under {:#x} (device default for it: {first:#x})",
                    h.client, h.handle, h.parent
                );
                return None;
            }
            Some(AllocParams::Tsg) => {
                let params = crate::rmrpc::alloc_params_window(&self.abi, body)?;
                let t = self.abi.decode_tsg_alloc_facts(params).ok()?;
                self.tsgs.insert((h.client, h.handle), (h.parent, t.h_vaspace, t.engine_type));
                return None;
            }
            Some(AllocParams::CtxShare) => {
                let params = crate::rmrpc::alloc_params_window(&self.abi, body)?;
                let c = self.abi.decode_ctxshare_alloc_facts(params).ok()?;
                self.ctxshares.insert((h.client, h.handle), c.h_vaspace);
                return None;
            }
            Some(AllocParams::Channel) => {}
            Some(AllocParams::NoDeclaredFacts)
                if matches!(
                    engine_class_kind(h.class),
                    Some(
                        kf_chip::classes::Kind::Compute
                            | kf_chip::classes::Kind::DmaCopy
                            | kf_chip::classes::Kind::ThreeD
                            | kf_chip::classes::Kind::TwoD
                            | kf_chip::classes::Kind::InlineToMemory
                            | kf_chip::classes::Kind::VideoEncoder
                            | kf_chip::classes::Kind::VideoDecoder
                    )
                ) =>
            {
                self.carried += 1;
                let copy_engine = if engine_class_kind(h.class) == Some(kf_chip::classes::Kind::DmaCopy) {
                    crate::rmrpc::alloc_params_window(&self.abi, body)
                        .and_then(|p| kf_abi::submit::CeAllocParams::decode(p).ok())
                        .and_then(|c| c.declared_copy_engine_type())
                } else {
                    None
                };
                let st = ChanStatement::EngineObject { client: h.client, parent: h.parent, handle: h.handle, class: h.class, copy_engine };
                return self.carry_alloc(st, cmd, h.client, h.handle);
            }
            _ => return None,
        }
        let params = crate::rmrpc::alloc_params_window(&self.abi, body)?;
        let f = self.abi.decode_channel_alloc_facts(params).ok()?;
        let tsg = self.tsgs.get(&(h.client, h.parent)).copied();
        // The device the channel hangs off: its parent, or its group's parent.
        let device = tsg.map_or(h.parent, |t| t.0);
        let default_vas = |me: &Self| {
            // The device's default VAS: allocated under the parent device, or else the ONE VA
            // space this client ever stated a page directory for. Two candidates and no alloc to
            // decide between them is refused by name at birth (`None`).
            me.vas_under.get(&(h.client, device)).copied().or_else(|| {
                let set = me.vas_stated.get(&h.client)?;
                (set.len() == 1).then(|| set.iter().next().copied()).flatten()
            })
        };
        let ctx_vas = self.abi.decode_channel_alloc_facts(params).ok().and_then(|c| {
            (c.h_ctx_share != 0).then(|| self.ctxshares.get(&(h.client, c.h_ctx_share)).copied()).flatten()
        });
        let vaspace = if f.h_vaspace != 0 {
            Some(f.h_vaspace)
        } else if let Some(v) = ctx_vas.filter(|v| *v != 0) {
            Some(v)
        } else if let Some(v) = tsg.map(|t| t.1).filter(|v| *v != 0) {
            // ★ P5b: a group member's VA space is the GROUP's (`kernel_channel.c` takes it from
            // the TSG when the channel names none).
            Some(v)
        } else {
            default_vas(self)
        };
        // ★ P5b: `ENGINE_TYPE_NULL` names the group's engine (libcuda's CE channels).
        let engine_type = match self.abi.decode_channel_engine_type(params).ok().flatten() {
            Some(0) | None => tsg.map(|t| t.2).filter(|e| *e != 0),
            e => e,
        };
        let privilege = self.abi.decode_channel_privilege(params).ok().flatten();
        // ★ v3-gfx: a dup'd VA space is the ORIGINAL object (`DUP_OBJECT` aliases, it does not copy).
        let (vaspace_client, vaspace) = match vaspace {
            Some(v) => {
                let (c, v) = self.vas_canonical(h.client, v);
                (c, Some(v))
            }
            None => (h.client, None),
        };
        let st = ChannelAlloc {
            client: h.client,
            parent: h.parent,
            handle: h.handle,
            class: h.class,
            gpfifo_va: f.gp_fifo_offset,
            entries: f.gp_fifo_entries,
            h_vaspace: f.h_vaspace,
            vaspace,
            vaspace_client,
            chid: decode_userd_index_chid(f.flags),
            flags: f.flags,
            engine_type,
            userd: self.abi.decode_channel_userd_mem(params).ok().flatten(),
            // ⊘⊘ P5b: the pid sentinel is reachable from guest userspace (`[measured p5bd]` the
            // raw client's sandboxed-isolate client `0xc1d0000c` declared `KERNEL_PID`), so it is
            // recorded (`declared_kernel_pid`), never trusted. ★ P6 (Q7, PROPOSED): the route's
            // kernel test is `kernel_channel` — RM's own `internalFlags.PRIVILEGE = KERNEL` stamp
            // or RM's internal-handle range. A user channel classed kernel would be Translated,
            // and the Translated route REWRITES physical CE operands onto the store / guest-RAM
            // windows (guest userspace reaching guest-kernel memory); the converse fails safe.
            kernel_client: kernel_channel(h.client, privilege),
            privilege,
            declared_kernel_pid: self.kernel_clients.contains(&h.client),
            tsg: tsg.map(|_| h.parent),
            ctx_share: f.h_ctx_share,
            device,
            error_notifier: self.abi.decode_channel_error_notifier(params).ok().flatten(),
        };
        self.carried += 1;
        self.carry_alloc(ChanStatement::Alloc(st), cmd, h.client, h.handle)
    }

    /// An alloc statement's answer: a refusal is refused, a deferred act holds the reply, and
    /// anything else lets the object seat record the object and answer.
    fn carry_alloc(&mut self, st: ChanStatement, cmd: &RpcCommand, client: u32, handle: u32) -> Option<Reply> {
        match (self.sink)(st) {
            ChanAnswer::Refused { status, why } => {
                self.refused += 1;
                Some(Self::refusal(status, &format!("{client:#x}:{handle:#x} alloc: {why}"), cmd))
            }
            // ⊘ The object seat still records the object and builds the reply; the FSM holds it
            // until the act resolves. A failed act posts that reply as the refusal — and leaves a
            // node the guest never frees in the graph until its parent goes (named in the log).
            ChanAnswer::Deferred(d) => {
                self.pending = Some(d);
                None
            }
            _ => None,
        }
    }

    fn on_control(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let h = self.abi.decode_rpc_control(&cmd.payload).ok()?;
        if self.abi.control_params(kf_arch::ids::ControlCmd(h.cmd)).is_some()
            && let Ok(crate::rmrpc::Translation::PageDir(st)) = crate::rmrpc::translate(&self.abi, self.guest_os, cmd)
        {
            // Observed only: the memory plane's link answers these.
            self.vas_stated.entry(st.client.0).or_default().insert(st.vaspace.0);
            self.vas_objects.insert((st.client.0, st.vaspace.0));
            return None;
        }
        let Some(params) = h.params_at.checked_add(h.params_size as usize).and_then(|e| cmd.payload.get(h.params_at..e)) else {
            if matches!(h.cmd, PROMOTE_CTX | EVICT_CTX) {
                eprintln!(
                    "kf-rm: chanlink: control {:#010x} declares {} params bytes, {} arrived (rpc flags {:#x}) — not carried",
                    h.cmd,
                    h.params_size,
                    cmd.payload.len().saturating_sub(h.params_at),
                    h.rmapi_rpc_flags
                );
            }
            return None;
        };
        let st = match h.cmd {
            GPFIFO_SCHEDULE | TSG_GPFIFO_SCHEDULE => {
                // `{NvBool bEnable; NvBool bSkipSubmit; NvBool bSkipEnable}` — all [IN].
                ChanStatement::Schedule { client: h.client, object: h.object, enable: params.first().is_some_and(|&b| b != 0) }
            }
            GET_WORK_SUBMIT_TOKEN => ChanStatement::Token { client: h.client, object: h.object },
            // ★ v3-video: exactly the measured shape (4 zero bytes) is carried; anything else stays
            // unserviced, refused as before.
            kf_abi::gssreplay::GSS_ENC_SESSION_ACQUIRE | kf_abi::gssreplay::GSS_ENC_SESSION_RELEASE
                if params.len() == kf_abi::gssreplay::ENC_SESSION_PARAMS_SIZE && params.iter().all(|b| *b == 0) =>
            {
                ChanStatement::EncoderSession { client: h.client, acquire: h.cmd == kf_abi::gssreplay::GSS_ENC_SESSION_ACQUIRE }
            }
            GR_SET_CTXSW_PREEMPTION_MODE => {
                if !self.abi.capabilities().control(kf_arch::ids::ControlCmd(h.cmd)).is_permitted() {
                    return None;
                }
                let w = |i: usize| params.get(4 * i..4 * i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                if params.len() != 32 {
                    return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, &format!("SET_CTXSW_PREEMPTION_MODE params are {} bytes, not 32", params.len()), cmd));
                }
                // ⊘ grRouteInfo (+16 flags, +24 route) selects a GR engine under MIG; this device has
                // one GR and no MIG, so a route is refused by name rather than ignored.
                if w(4)? != 0 || w(6)? != 0 || w(7)? != 0 {
                    return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, "SET_CTXSW_PREEMPTION_MODE with a grRouteInfo (MIG routing) is not served", cmd));
                }
                ChanStatement::CtxswPreemption { client: h.client, channel: w(1)?, flags: w(0)?, gfxp: w(2)?, cilp: w(3)? }
            }
            GR_CTXSW_ZCULL_BIND => {
                if !self.abi.capabilities().control(kf_arch::ids::ControlCmd(h.cmd)).is_permitted() {
                    return None;
                }
                if params.len() != 24 {
                    return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, &format!("ZCULL_BIND params are {} bytes, not 24", params.len()), cmd));
                }
                let w = |i: usize| u32::from_le_bytes([params[4 * i], params[4 * i + 1], params[4 * i + 2], params[4 * i + 3]]);
                // ⊘ Another client's channel is refused by name (as DISABLE_CHANNELS refuses one):
                // the twin is looked up in THIS client's namespace only.
                if w(0) != h.client {
                    return Some(Self::refusal(NV_ERR_INSUFFICIENT_PERMISSIONS, &format!("ZCULL_BIND names client {:#x} from client {:#x}", w(0), h.client), cmd));
                }
                if w(4) > ZCULL_MODE_MAX {
                    return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, &format!("ZCULL_BIND mode {} is not a zcull mode", w(4)), cmd));
                }
                ChanStatement::ZcullBind { client: h.client, channel: w(1), va: u64::from(w(2)) | (u64::from(w(3)) << 32), mode: w(4) }
            }
            TSG_SET_TIMESLICE => {
                if !self.abi.capabilities().control(kf_arch::ids::ControlCmd(h.cmd)).is_permitted() {
                    return None;
                }
                let Some(us) = params.get(..8).filter(|_| params.len() == 8).map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])) else {
                    return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, &format!("SET_TIMESLICE params are {} bytes, not 8", params.len()), cmd));
                };
                ChanStatement::Timeslice { client: h.client, object: h.object, us }
            }
            PERF_CUDA_LIMIT_SET_CONTROL => {
                if !self.abi.capabilities().control(kf_arch::ids::ControlCmd(h.cmd)).is_permitted() {
                    return None;
                }
                if params.len() != 1 {
                    return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, &format!("PERF_CUDA_LIMIT_SET_CONTROL params are {} bytes, not 1", params.len()), cmd));
                }
                ChanStatement::CudaLimit { client: h.client, device: h.object, enable: params[0] != 0 }
            }
            PERF_CUDA_LIMIT_DISABLE => {
                if !self.abi.capabilities().control(kf_arch::ids::ControlCmd(h.cmd)).is_permitted() {
                    return None;
                }
                ChanStatement::CudaLimitDisable
            }
            DEBUG_SET_EXCEPTION_MASK => {
                if !self.abi.capabilities().control(kf_arch::ids::ControlCmd(h.cmd)).is_permitted() {
                    return None;
                }
                let Some(mask) = params.get(..4).filter(|_| params.len() == 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) else {
                    return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, &format!("SET_EXCEPTION_MASK params are {} bytes, not 4", params.len()), cmd));
                };
                ChanStatement::DebuggerExceptionMask { client: h.client, object: h.object, mask }
            }
            PROMOTE_CTX => {
                eprintln!(
                    "kf-rm: chanlink: GPU_PROMOTE_CTX on {:#x}:{:#x} params={} rpc_flags={:#x}",
                    h.client, h.object, h.params_size, h.rmapi_rpc_flags
                );
                // ★ The capability gate this control's decoder would have applied (it is admitted).
                if !self.abi.capabilities().control(kf_arch::ids::ControlCmd(h.cmd)).is_permitted() {
                    eprintln!("kf-rm: chanlink: GPU_PROMOTE_CTX not permitted by this boundary's allowlist");
                    return None;
                }
                let p = match self.abi.decode_promote_ctx(params) {
                    Ok(p) => p,
                    // ★ v3-video: a video falcon's context promote (`kernel_falcon.c:184-276`) —
                    // no entries, the buffer's VA only. Satisfied by the twin (host RM promoted its
                    // own falcon context with the engine object); carried with no entries.
                    Err(kf_abi::wire::AbiError::PromoteLegacyShape { .. }) if self.abi.decode_falcon_promote(params).is_ok() => {
                        let (engine_type, chan_client, object, va, size) = self.abi.decode_falcon_promote(params).ok()?;
                        eprintln!("kf-rm: chanlink: falcon ctx promote {chan_client:#x}:{object:#x} engine {engine_type:#x} guest ctx buffer VA {va:#x}+{size:#x}");
                        let st = ChanStatement::PromoteCtx { chan_client, object, engine_type, initialize: 0, with_va: 0, entries: 0, falcon_ctx: Some((va, size)) };
                        return self.carry_control_statement(st, cmd, &h);
                    }
                    Err(e) => return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, &format!("GPU_PROMOTE_CTX undecodable: {e:?}"), cmd)),
                };
                let (mut initialize, mut with_va, mut entries) = (0u32, 0u32, 0u32);
                for e in p.entries() {
                    entries += 1;
                    let bit = 1u32.checked_shl(u32::from(e.buffer_id())).unwrap_or(0);
                    match e {
                        kf_abi::view::PromoteEntry::InitializeOnly { .. } => initialize |= bit,
                        kf_abi::view::PromoteEntry::PromoteOnly { .. } => with_va |= bit,
                        kf_abi::view::PromoteEntry::Promotable { .. } => {
                            initialize |= bit;
                            with_va |= bit;
                        }
                    }
                }
                ChanStatement::PromoteCtx {
                    chan_client: p.h_chan_client,
                    object: p.h_object,
                    engine_type: p.engine_type,
                    initialize,
                    with_va,
                    entries,
                    falcon_ctx: None,
                }
            }
            EVICT_CTX => {
                if !self.abi.capabilities().control(kf_arch::ids::ControlCmd(h.cmd)).is_permitted() {
                    return None;
                }
                let w = |i: usize| params.get(4 * i..4 * i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                if params.len() != EVICT_CTX_PARAMS_SIZE {
                    return Some(Self::refusal(
                        NV_ERR_INVALID_ARGUMENT,
                        &format!("GPU_EVICT_CTX params are {} bytes, not {EVICT_CTX_PARAMS_SIZE}", params.len()),
                        cmd,
                    ));
                }
                ChanStatement::EvictCtx { engine_type: w(0)?, chan_client: w(3)?, object: w(4)? }
            }
            BIND => {
                let engine_type = params.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))?;
                ChanStatement::Bind { client: h.client, object: h.object, engine_type }
            }
            // ★★★ v3-chanctl — the three cancellation verbs, each carried to the plane, which
            // performs it on the host twin(s) and holds the reply until the host act completes.
            STOP_CHANNEL => {
                if params.len() != kf_abi::submit::STOP_CHANNEL_PARAMS_SIZE {
                    return Some(Self::refusal(
                        NV_ERR_INVALID_ARGUMENT,
                        &format!("STOP_CHANNEL params are {} bytes, not {}", params.len(), kf_abi::submit::STOP_CHANNEL_PARAMS_SIZE),
                        cmd,
                    ));
                }
                ChanStatement::Stop { client: h.client, object: h.object, immediate: params[0] != 0 }
            }
            TSG_PREEMPT => {
                let Some(p) = kf_abi::submit::Preempt::decode(params) else {
                    return Some(Self::refusal(
                        NV_ERR_INVALID_ARGUMENT,
                        &format!("PREEMPT params are {} bytes, not {}", params.len(), kf_abi::submit::PREEMPT_PARAMS_SIZE),
                        cmd,
                    ));
                };
                // The header's own bound (`ctrla06c.h:213`): a manual timeout past 1 s is invalid.
                if p.manual_timeout && p.timeout_us > kf_abi::submit::PREEMPT_MAX_MANUAL_TIMEOUT_US {
                    return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, &format!("PREEMPT timeoutUs {} > 1 s", p.timeout_us), cmd));
                }
                ChanStatement::Preempt { client: h.client, object: h.object, wait: p.wait }
            }
            DISABLE_CHANNELS => {
                let d = match kf_abi::submit::DisableChannels::decode(params) {
                    Ok(d) => d,
                    Err(e) => return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, &format!("DISABLE_CHANNELS: {e:?}"), cmd)),
                };
                // ⊘ `pRunlistPreemptEvent` is a KEVENT pointer in the GUEST kernel's address space
                // (kernel callers only, `kernel_fifo_ctrl.c:720-725`): nothing on the host can
                // signal it, so the asynchronous form is refused by name, never dropped.
                if d.runlist_preempt_event != 0 {
                    return Some(Self::refusal(NV_ERR_INVALID_ARGUMENT, "DISABLE_CHANNELS with a pRunlistPreemptEvent (async preempt) is not served", cmd));
                }
                // ⊘ Isolation: a list entry naming ANOTHER client's channel is refused — the only
                // in-tree caller lists channels of the calling client (`nv_gpu_ops.c:957-981`,
                // `RES_GET_CLIENT_HANDLE` of channels iterated from that very client), and one
                // guest process must never stop another's twin.
                if let Some((c, ch)) = d.list.iter().find(|(c, _)| *c != h.client) {
                    return Some(Self::refusal(
                        NV_ERR_INSUFFICIENT_PERMISSIONS,
                        &format!("DISABLE_CHANNELS from client {:#x} names channel {c:#x}:{ch:#x} of another client", h.client),
                        cmd,
                    ));
                }
                ChanStatement::DisableChannels {
                    client: h.client,
                    disable: d.disable,
                    only_scheduling: d.only_disable_scheduling,
                    rewind_gp_put: d.rewind_gp_put,
                    list: ChanList::new(&d.list)?,
                }
            }
            _ => return None,
        };
        self.carry_control_statement(st, cmd, &h)
    }

    /// Carry a control statement to the sink and build the guest's reply from its answer.
    fn carry_control_statement(&mut self, st: ChanStatement, cmd: &RpcCommand, h: &kf_abi::view::RpcControlReq) -> Option<Reply> {
        self.carried += 1;
        match (self.sink)(st) {
            ChanAnswer::NotOurs => None,
            // ★ The [IN] params echoed: the transport copies a non-empty reply over the caller's
            // struct (`ogkm-580: rpc.c:11085-11090`), so a zeroed body would rewrite bEnable.
            ChanAnswer::Done => Some(Reply { rpc_result: NV_OK, body: cmd.payload.clone() }),
            // ★ P5b: the same reply, HELD until the host act resolves it (a failure posts it as
            // that status — the envelope result, which a control's caller does read).
            ChanAnswer::Deferred(d) => {
                self.pending = Some(d);
                Some(Reply { rpc_result: NV_OK, body: cmd.payload.clone() })
            }
            ChanAnswer::Token(t) => {
                let mut body = cmd.payload.clone();
                let at = h.params_at;
                if body.len() < at + 4 || h.params_size < 4 {
                    return Some(Self::refusal(0x1F, "work-submit token params shorter than 4 bytes", cmd));
                }
                body[at..at + 4].copy_from_slice(&t.to_le_bytes());
                Some(Reply { rpc_result: NV_OK, body })
            }
            ChanAnswer::Refused { status, why } => {
                Some(Self::refusal(status, &format!("control {:#010x} on {:#x}:{:#x}: {why}", h.cmd, h.client, h.object), cmd))
            }
        }
    }

    /// ★ v3-gfx: the VA-space object `(client, handle)` names — itself, or the original a dup aliases.
    fn vas_canonical(&self, client: u32, handle: u32) -> (u32, u32) {
        self.vas_aliases.get(&(client, handle)).copied().unwrap_or((client, handle))
    }

    /// ★ v3-gfx: a `DUP_OBJECT` of a VA-space object we know becomes an alias of the original.
    fn on_dup(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let d = self.abi.decode_dup(&cmd.payload).ok()?;
        let src = self.vas_canonical(d.src_client, d.src_handle);
        if self.vas_objects.contains(&src) {
            self.vas_aliases.insert((d.dst_client, d.dst_handle), src);
        }
        None
    }

    fn on_free(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let f = self.abi.decode_free(&cmd.payload).ok()?;
        let (client, object) = (f.client, f.handle);
        // ★ v3-gfx: an alias's (or its client's) free drops the NAME. The original's own free
        // keeps its aliases: RM refcounts the object, and the dup still holds it.
        self.vas_aliases.retain(|&(c, h), _| !(c == client && (object == client || h == object)));
        if client == object {
            self.vas_objects.retain(|k| k.0 != client || self.vas_aliases.values().any(|o| o == k));
        } else if !self.vas_aliases.values().any(|o| *o == (client, object)) {
            self.vas_objects.remove(&(client, object));
        }
        if client == object {
            self.kernel_clients.remove(&client);
            self.vas_under.retain(|k, _| k.0 != client);
            self.vas_stated.remove(&client);
            self.tsgs.retain(|k, _| k.0 != client);
            self.ctxshares.retain(|k, _| k.0 != client);
        } else {
            self.tsgs.remove(&(client, object));
            self.ctxshares.remove(&(client, object));
            // ★ Only the DEVICE's free forgets its default VAS. The VASpace handle itself is a
            // transient NAME (`index = GPU_DEVICE`, "acquire reference to device vaspace",
            // `nvos.h:3187`): RM allocs it, publishes the PDEs, and FREES it — `[measured p5c]`
            // forgetting it there left the scrubber's channel with no VA space. The graph files it
            // the same way (`RmGraph::device_default_vas`, outliving the handle's own free).
            self.vas_under.retain(|k, _| !(k.0 == client && k.1 == object));
        }
        // ★ P5b: a twin's free is a host act too — its reply waits for it (the guest's next step
        // may unmap what the twin fetches from).
        if let ChanAnswer::Deferred(d) = (self.sink)(ChanStatement::Free { client, object }) {
            self.pending = Some(d);
        }
        None
    }
}

/// ★ P5b (Q6a) — **the alloc-params shape of a class, for EVERY family.** `kf_abi`'s table maps
/// the family-invariant classes and, of the engine classes, only the Ampere ids it was first
/// written for (`AMPERE_CHANNEL_GPFIFO_A`, `AMPERE_DMA_COPY_B`, …) — so a Turing (`0xc46f`),
/// Hopper (`0xc86f`) or Blackwell (`0xc96f`/`0xca6f`) channel alloc was `UnmappedAllocClass` and
/// never reached the channel plane. The engine classes come from `kf_chip`'s GENERATED per-family
/// sets (`classes::FAMILIES`, compiled from ogkm's `g_gpu_class_list.c`): every GPFIFO channel
/// class takes the one `NV_CHANNEL_ALLOC_PARAMS` (ogkm `resource_list.h`), every compute / copy /
/// 3D object is an edge whose params the object model does not read. ⊘ Class ids are unique
/// across families, so no family argument is needed (a class not listed for the guest's own
/// family is still refused earlier, by the capability allowlist).
#[must_use]
pub fn alloc_shape(abi: &DriverAbiTable, class: u32) -> Option<AllocParams> {
    // ★ w827: a debugger session is a graph node whose params the object model does not read —
    // the channel plane decodes them (`ChanStatement::Debugger`) and twins the session on the host.
    if class == GT200_DEBUGGER {
        return Some(AllocParams::NoDeclaredFacts);
    }
    // ★ v3-gfx: two graphics classes that are graph nodes with NO host counterpart, both
    // `RS_FLAGS_ALLOC_RPC_TO_ALL` (the guest's CPU-RM makes its own object, then asks "GSP"):
    //  - `GF100_ZBC_CLEAR` (`0x9096`, under a subdevice): its controls are answered from the per-VM
    //    table (`crate::zbc`); the host's GPU-global table is never touched.
    //  - `GF100_DISP_SW` (`0x9072`, under a channel): display software methods (flip/semaphore
    //    helpers). This device has no display engine; a headless UMD allocates it on every 3D
    //    channel and never methods it. ⊘ NOT twinned: host RM's dispsw acts on HOST display heads,
    //    so a guest's methods must never reach it. A guest that does method it faults its own
    //    twin (contained, `gpu_fault_is_contained`).
    //  `[measured vgfx 2026-09-26]` the host's own Vulkan/EGL run allocs 0x9072 ×12, 0x9096 ×3.
    if class == GF100_ZBC_CLEAR || class == GF100_DISP_SW {
        return Some(AllocParams::NoDeclaredFacts);
    }
    abi.alloc_params(kf_arch::ids::ClassId(class)).or_else(|| match engine_class_kind(class)? {
        kf_chip::classes::Kind::ChannelGpfifo => Some(AllocParams::Channel),
        kf_chip::classes::Kind::Compute
        | kf_chip::classes::Kind::DmaCopy
        | kf_chip::classes::Kind::ThreeD
        | kf_chip::classes::Kind::TwoD
        | kf_chip::classes::Kind::InlineToMemory
        | kf_chip::classes::Kind::VideoEncoder
        | kf_chip::classes::Kind::VideoDecoder => Some(AllocParams::NoDeclaredFacts),
        kf_chip::classes::Kind::Usermode => None,
    })
}

/// The engine-class kind of `class` on ANY family (generated sets). ⚠ An id may appear in several
/// families (`FERMI_TWOD_A` is in all five) — always with the SAME kind, which a test pins.
#[must_use]
pub fn engine_class_kind(class: u32) -> Option<kf_chip::classes::Kind> {
    kf_chip::classes::FAMILIES.iter().find_map(|f| f.kind_of(class))
}

/// `RS_CLIENT_INTERNAL_HANDLE_BASE` (`ogkm-580: inc/libraries/resserv/resserv.h:138`).
pub const RS_CLIENT_INTERNAL_HANDLE_BASE: u32 = 0xC1E0_0000;

/// ★ Is `h_client` one of the guest RM's OWN internal clients — the guest's own classification,
/// `serverIsClientInternal` (`ogkm-580: libraries/resserv/src/rs_server.c:2618-2623`).
///
/// `[measured p5a]` the PMA scrubber's client (`0xc1e00006` on that boot) was NOT marked kernel by
/// the root alloc's pid sentinel, so the pid rule alone under-reports RM's internal channels. The
/// handle is the guest RM's own statement: a client that asks for a fixed handle is RE-ENCODED
/// onto the user base `0xC1D00000` (`rs_server.c:3267-3271`), so no guest process can hold one in
/// this range — only the guest kernel's RM makes them.
#[must_use]
pub const fn is_rm_internal_client(h_client: u32) -> bool {
    h_client & RS_CLIENT_INTERNAL_HANDLE_BASE == RS_CLIENT_INTERNAL_HANDLE_BASE
}

/// ★★★ **The Q7 identity rule (PROPOSED, pending the owner's approval)** — a channel is the guest
/// KERNEL's iff a fact guest userspace cannot produce says so:
///
/// 1. the guest's CPU-RM stamped `internalFlags.PRIVILEGE = KERNEL` on its alloc
///    ([`kf_abi::notifier::ChannelPrivilege`] carries the ogkm proof), or
/// 2. its client is in the guest RM's internal-handle range ([`is_rm_internal_client`]).
///
/// ⊘ Everything else — including `ADMIN` (guest root userspace) — is a user channel: it is
/// Passthrough, where a physical operand faults on the unprivileged host twin (a failure, never an
/// escalation). Misclassing a user channel as kernel would put it on the Translated route, whose
/// `fbAliasVA` rewrite turns a physical operand into a store/guest-RAM window address.
#[must_use]
pub fn kernel_channel(h_client: u32, privilege: Option<kf_abi::notifier::ChannelPrivilege>) -> bool {
    is_rm_internal_client(h_client) || privilege.is_some_and(|p| p.is_kernel())
}

/// ★ The guest's own channel id, off `NV_CHANNEL_ALLOC_PARAMS.flags` — the guest kernel allocates
/// its `ChID` before it RPCs and states it as `USERD_INDEX` (`ogkm-580: kernel_channel.c:2786-2800`
/// writes `USERD_INDEX_PAGE_VALUE = chid / 8`, `USERD_INDEX_VALUE = chid % 8`, and sets
/// `USERD_INDEX_PAGE_FIXED`). Fields (`ogkm-580: alloc_channel.h:184-203`): `INDEX_VALUE 10:8`,
/// `INDEX_FIXED 11:11`, `PAGE_VALUE 20:12`, `PAGE_FIXED 21:21`. Copied from the old tree's
/// `kayfabe-chips/src/ga10x.rs:419` (pure; it decoded every boot of the §13 campaign).
///
/// `None` when the page is not fixed (the flags name no channel) or `INDEX_FIXED` is set (RM
/// itself refuses that combination with `NV_ERR_INVALID_STATE`). ★ A USERD page holds
/// `1 << 3` channels, so the widest chid expressible is `511 * 8 + 7 = 4095` — the 12-bit
/// doorbell `VECTOR` exactly.
#[must_use]
pub fn decode_userd_index_chid(flags: u32) -> Option<u32> {
    if (flags >> 21) & 1 == 0 || (flags >> 11) & 1 != 0 {
        return None;
    }
    let value = (flags >> 8) & 0x7;
    let page = (flags >> 12) & 0x1FF;
    Some(page * 8 + value)
}

impl core::fmt::Debug for ChannelPolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChannelPolicy").field("carried", &self.carried).field("refused", &self.refused).finish()
    }
}

impl CommandPolicy for ChannelPolicy {
    fn defers(&mut self, _cmd: &RpcCommand) -> Option<kf_gsp::Deferred> {
        self.pending.take()
    }

    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        // ⊘ Reset per command: a cell from a command whose reply was not ours must not ride on
        // the next one.
        self.pending = None;
        match cmd.function {
            RpcFunction::RmAlloc => self.on_alloc(cmd),
            RpcFunction::RmControl => self.on_control(cmd),
            RpcFunction::Free => self.on_free(cmd),
            RpcFunction::DupObject => self.on_dup(cmd),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[cap1b]` the PMA scrubber's channel declared `flags = 0x00a00120` (chid 1); the global
    /// CeUtils rang `0x10002` (chid 2).
    #[test]
    fn the_captured_flags_decode_to_the_guest_chid() {
        assert_eq!(decode_userd_index_chid(0x00a0_0120), Some(1));
        assert_eq!(decode_userd_index_chid(0x00a0_0220), Some(2));
        assert_eq!(decode_userd_index_chid(0x0020_0000 | (511 << 12) | (7 << 8)), Some(4095));
        assert_eq!(decode_userd_index_chid(0x0000_0120), None, "page not fixed: names no channel");
        assert_eq!(decode_userd_index_chid(0x0020_0920), None, "INDEX_FIXED: RM refuses it");
    }

    /// ★ Q7: only `PRIVILEGE = KERNEL` (or the internal range) is kernel; `ADMIN`, `USER`, a bare
    /// `UVM_OWNED` bit and an undecodable alloc are not.
    #[test]
    fn only_a_kernel_stamp_or_the_internal_range_is_kernel() {
        use kf_abi::notifier::ChannelPrivilege as P;
        let user = 0xc1d0_0001;
        assert!(kernel_channel(user, Some(P::from_internal_flags(0x82))), "UVM: KERNEL + UVM_OWNED");
        assert!(kernel_channel(user, Some(P::from_internal_flags(0x2))));
        assert!(!kernel_channel(user, Some(P::from_internal_flags(0x1))), "ADMIN is guest root userspace");
        assert!(!kernel_channel(user, Some(P::from_internal_flags(0x0))));
        assert!(!kernel_channel(user, Some(P::from_internal_flags(0x80))), "UVM_OWNED alone is not KERNEL");
        assert!(!kernel_channel(user, Some(P::from_internal_flags(0x3))), "undefined level");
        assert!(!kernel_channel(user, None), "undecodable is never kernel");
        assert!(kernel_channel(0xc1e0_0006, None), "RM's own internal client");
    }

    /// ★ v3-promote: `GPU_PROMOTE_CTX` reaches the plane as a statement naming the CHANNEL by
    /// `hChanClient`/`hObject` (not the envelope's subdevice), with the two protocol halves
    /// (initialize-only entries from `kgrobjPromoteContext`, VA-only entries from the UVM bind)
    /// kept apart; `NV_OK` echoes the `[IN]` params; a channel the plane does not own is declined.
    #[test]
    fn promote_ctx_is_carried_to_the_plane_and_answered_by_it() {
        use std::sync::{Arc, Mutex};
        let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench");
        let seen: Arc<Mutex<Vec<ChanStatement>>> = Arc::default();
        let s2 = seen.clone();
        let sink: ChanSink = Arc::new(move |st| {
            s2.lock().unwrap().push(st);
            match st {
                ChanStatement::PromoteCtx { object: 0xcafe_0013, .. } | ChanStatement::EvictCtx { object: 0xcafe_0013, .. } => ChanAnswer::Done,
                _ => ChanAnswer::NotOurs,
            }
        });
        let mut link = ChannelPolicy::new(abi, kf_abi::GuestOs::Linux, sink);
        // params: engineType GR, hChanClient, hObject, entryCount + entries.
        let mut p = vec![0u8; 560];
        let put = |p: &mut Vec<u8>, at: usize, v: u32| p[at..at + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut p, 0, 1);
        put(&mut p, 12, 0xc1d0_000b);
        put(&mut p, 16, 0xcafe_0013);
        put(&mut p, 40, 2);
        // entry 0: MAIN, initialize-only (PA + size, bNonmapped).
        p[48..56].copy_from_slice(&0x20_0000u64.to_le_bytes());
        p[48 + 16..48 + 24].copy_from_slice(&0x8600u64.to_le_bytes());
        p[48 + 30] = 1;
        p[48 + 31] = 1;
        // entry 1: PATCH (2), VA only (the UVM bind shape).
        p[80 + 8..80 + 16].copy_from_slice(&0x7000_0000u64.to_le_bytes());
        p[80 + 28..80 + 30].copy_from_slice(&2u16.to_le_bytes());
        let mut body = vec![0u8; 40];
        body[0..4].copy_from_slice(&0xc1d0_0001u32.to_le_bytes()); // the SUBDEVICE's client
        body[4..8].copy_from_slice(&0x5c00_0001u32.to_le_bytes());
        body[8..12].copy_from_slice(&PROMOTE_CTX.to_le_bytes());
        body[16..20].copy_from_slice(&560u32.to_le_bytes());
        body.extend_from_slice(&p);
        let cmd = |payload: Vec<u8>| RpcCommand {
            function: RpcFunction::RmControl,
            code: 76,
            sequence: 1,
            payload,
            elements: 1,
            delivered: Vec::new(),
        };
        let r = link.respond(&cmd(body.clone())).expect("the plane answered");
        assert_eq!(r.rpc_result, NV_OK);
        assert_eq!(r.body, body, "the [IN] params are echoed");
        assert_eq!(
            seen.lock().unwrap().last().copied(),
            Some(ChanStatement::PromoteCtx {
                chan_client: 0xc1d0_000b,
                object: 0xcafe_0013,
                engine_type: 1,
                initialize: 1 << 0,
                with_va: 1 << 2,
                entries: 2,
                falcon_ctx: None
            })
        );
        // A channel the plane does not own: declined (the FSM's named refusal answers it).
        let mut other = body.clone();
        other[40 + 16..40 + 20].copy_from_slice(&0xdead_0001u32.to_le_bytes());
        assert!(link.respond(&cmd(other)).is_none());
        // EVICT: 20 bytes, [IN].
        let mut e = vec![0u8; 40];
        e[8..12].copy_from_slice(&EVICT_CTX.to_le_bytes());
        e[16..20].copy_from_slice(&20u32.to_le_bytes());
        let mut ep = vec![0u8; 20];
        ep[0..4].copy_from_slice(&1u32.to_le_bytes());
        ep[12..16].copy_from_slice(&0xc1d0_000bu32.to_le_bytes());
        ep[16..20].copy_from_slice(&0xcafe_0013u32.to_le_bytes());
        e.extend_from_slice(&ep);
        let r = link.respond(&cmd(e)).expect("answered");
        assert_eq!(r.rpc_result, NV_OK);
        assert_eq!(
            seen.lock().unwrap().last().copied(),
            Some(ChanStatement::EvictCtx { chan_client: 0xc1d0_000b, object: 0xcafe_0013, engine_type: 1 })
        );
    }

    /// ★ v3-gfx: `GR_CTXSW_ZCULL_BIND` reaches the plane decoded (VA across two words, the mode);
    /// a bind naming ANOTHER client, a mode past `SEPARATE_BUFFER`, or a wrong size is refused by
    /// the link before the plane is asked; a channel the plane does not own is declined.
    #[test]
    fn zcull_bind_is_carried_and_a_foreign_client_or_bad_mode_is_refused() {
        use std::sync::{Arc, Mutex};
        let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench");
        let seen: Arc<Mutex<Vec<ChanStatement>>> = Arc::default();
        let s2 = seen.clone();
        let sink: ChanSink = Arc::new(move |st| {
            s2.lock().unwrap().push(st);
            match st {
                ChanStatement::ZcullBind { channel: 0xbeef_0100, .. } => ChanAnswer::Done,
                _ => ChanAnswer::NotOurs,
            }
        });
        let mut link = ChannelPolicy::new(abi, kf_abi::GuestOs::Linux, sink);
        let ctl = |client: u32, p: &[u8]| {
            let mut b = vec![0u8; 40];
            b[0..4].copy_from_slice(&client.to_le_bytes());
            b[4..8].copy_from_slice(&0xbeef_0004u32.to_le_bytes());
            b[8..12].copy_from_slice(&GR_CTXSW_ZCULL_BIND.to_le_bytes());
            b[16..20].copy_from_slice(&(p.len() as u32).to_le_bytes());
            b.extend_from_slice(p);
            RpcCommand { function: RpcFunction::RmControl, code: 76, sequence: 1, payload: b, elements: 1, delivered: Vec::new() }
        };
        // `[measured vgfx 2026-09-26]` the host UMD's own bytes: client, channel 0xbeef0100,
        // vMemPtr 0x4280000, mode 2.
        let bind = |client: u32, channel: u32, va: u64, mode: u32| {
            let mut p = vec![0u8; 24];
            p[0..4].copy_from_slice(&client.to_le_bytes());
            p[4..8].copy_from_slice(&channel.to_le_bytes());
            p[8..16].copy_from_slice(&va.to_le_bytes());
            p[16..20].copy_from_slice(&mode.to_le_bytes());
            p
        };
        let c = 0xc1d0_015c;
        let r = link.respond(&ctl(c, &bind(c, 0xbeef_0100, 0x1_0428_0000, 2))).expect("answered");
        assert_eq!(r.rpc_result, NV_OK);
        assert_eq!(
            seen.lock().unwrap().last().copied(),
            Some(ChanStatement::ZcullBind { client: c, channel: 0xbeef_0100, va: 0x1_0428_0000, mode: 2 })
        );
        let n = seen.lock().unwrap().len();
        assert_eq!(link.respond(&ctl(c, &bind(0xc1d0_0999, 0xbeef_0100, 0, 2))).expect("refused").rpc_result, NV_ERR_INSUFFICIENT_PERMISSIONS);
        assert_eq!(link.respond(&ctl(c, &bind(c, 0xbeef_0100, 0, 3))).expect("refused").rpc_result, NV_ERR_INVALID_ARGUMENT);
        assert_eq!(link.respond(&ctl(c, &bind(c, 0xbeef_0100, 0, 2)[..20])).expect("refused").rpc_result, NV_ERR_INVALID_ARGUMENT);
        assert_eq!(seen.lock().unwrap().len(), n, "a refused bind never reaches the plane");
        // A channel the plane does not own (a Translated/kernel one): declined.
        assert!(link.respond(&ctl(c, &bind(c, 0xdead_0001, 0, 2))).is_none());
    }

    /// ★ v3-gfx: a VA space `DUP_OBJECT`'d into another client resolves to its ORIGINAL (the
    /// Vulkan UMD's shape, `[measured vgfx 2026-09-26]`: probe client's VAS dup'd into its device);
    /// the original's own free keeps the alias alive (RM refcounts), the alias's free drops it.
    #[test]
    fn a_dupd_va_space_resolves_to_its_original_until_the_alias_goes() {
        let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench");
        let sink: ChanSink = std::sync::Arc::new(|_| ChanAnswer::NotOurs);
        let mut link = ChannelPolicy::new(abi, kf_abi::GuestOs::Linux, sink);
        let rpc = |function: RpcFunction, payload: Vec<u8>| RpcCommand { function, code: 0, sequence: 1, payload, elements: 1, delivered: Vec::new() };
        let words = |w: &[u32]| w.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        let (orig, alias) = ((0xc1d0_0016, 0xfade_0003), (0xc1d0_001a, 0xbeef_0300));
        link.vas_objects.insert(orig);
        // NVOS55: hClient hParent hObject hClientSrc hObjectSrc flags status
        assert!(link.respond(&rpc(RpcFunction::DupObject, words(&[alias.0, 0xbeef_0003, alias.1, orig.0, orig.1, 0, 0]))).is_none());
        assert_eq!(link.vas_canonical(alias.0, alias.1), orig);
        // A dup of something that is not a known VA space is not an alias.
        link.respond(&rpc(RpcFunction::DupObject, words(&[alias.0, 0xbeef_0003, 0xbeef_0400, orig.0, 0x1234, 0, 0])));
        assert_eq!(link.vas_canonical(alias.0, 0xbeef_0400), (alias.0, 0xbeef_0400));
        // The ORIGINAL's client goes (the UMD frees its probe client): the alias still resolves.
        link.respond(&rpc(RpcFunction::Free, words(&[orig.0, 0, orig.0, 0])));
        assert_eq!(link.vas_canonical(alias.0, alias.1), orig, "RM refcounts: the dup keeps the object");
        assert!(link.vas_objects.contains(&orig));
        // The alias's own free drops the name, and with it the last reference.
        link.respond(&rpc(RpcFunction::Free, words(&[alias.0, 0xbeef_0003, alias.1, 0])));
        assert_eq!(link.vas_canonical(alias.0, alias.1), alias);
    }

    /// ★ v3-gfx: `GF100_ZBC_CLEAR` and `GF100_DISP_SW` are graph nodes (no host twin); 2D and
    /// inline-to-memory are engine objects carried to the twin like 3D.
    #[test]
    fn graphics_classes_have_a_shape() {
        let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench");
        for c in [GF100_ZBC_CLEAR, GF100_DISP_SW, 0x902d, 0xa140, 0xcd40, 0xc797] {
            assert_eq!(alloc_shape(&abi, c), Some(AllocParams::NoDeclaredFacts), "{c:#x}");
        }
        assert_eq!(engine_class_kind(0x902d), Some(kf_chip::classes::Kind::TwoD));
        assert_eq!(engine_class_kind(0xa140), Some(kf_chip::classes::Kind::InlineToMemory));
        assert_eq!(engine_class_kind(GF100_DISP_SW), None, "DISP_SW is not an engine object: it is never twinned");
    }

    /// ★ v3-chanctl: STOP_CHANNEL / PREEMPT / DISABLE_CHANNELS reach the plane decoded; a
    /// DISABLE_CHANNELS naming another client's channel, or carrying an async preempt event, is
    /// refused by the link before the plane is asked.
    #[test]
    fn the_cancellation_verbs_are_carried_and_the_cross_client_list_is_refused() {
        use std::sync::{Arc, Mutex};
        let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench");
        let seen: Arc<Mutex<Vec<ChanStatement>>> = Arc::default();
        let s2 = seen.clone();
        let sink: ChanSink = Arc::new(move |st| {
            s2.lock().unwrap().push(st);
            ChanAnswer::Done
        });
        let mut link = ChannelPolicy::new(abi, kf_abi::GuestOs::Linux, sink);
        let ctl = |client: u32, object: u32, cmd: u32, p: &[u8]| {
            let mut b = vec![0u8; 40];
            b[0..4].copy_from_slice(&client.to_le_bytes());
            b[4..8].copy_from_slice(&object.to_le_bytes());
            b[8..12].copy_from_slice(&cmd.to_le_bytes());
            b[16..20].copy_from_slice(&(p.len() as u32).to_le_bytes());
            b.extend_from_slice(p);
            RpcCommand { function: RpcFunction::RmControl, code: 76, sequence: 1, payload: b, elements: 1, delivered: Vec::new() }
        };
        let (c, ch) = (0xc1d0_000b, 0xcafe_0013);
        let r = link.respond(&ctl(c, ch, STOP_CHANNEL, &[0])).expect("answered");
        assert_eq!(r.rpc_result, NV_OK);
        assert_eq!(seen.lock().unwrap().last().copied(), Some(ChanStatement::Stop { client: c, object: ch, immediate: false }));
        assert_eq!(link.respond(&ctl(c, ch, STOP_CHANNEL, &[0, 0])).expect("refused").rpc_result, NV_ERR_INVALID_ARGUMENT);
        let r = link.respond(&ctl(c, 0xcafe_0010, TSG_PREEMPT, &[1, 0, 0, 0, 0, 0, 0, 0])).expect("answered");
        assert_eq!(r.rpc_result, NV_OK);
        assert_eq!(seen.lock().unwrap().last().copied(), Some(ChanStatement::Preempt { client: c, object: 0xcafe_0010, wait: true }));
        let d = |list: Vec<(u32, u32)>, ev: u64| {
            let mut b = kf_abi::submit::DisableChannels { disable: true, only_disable_scheduling: false, rewind_gp_put: false, runlist_preempt_event: 0, list }
                .encode()
                .expect("encode");
            b[16..24].copy_from_slice(&ev.to_le_bytes());
            b
        };
        let n = seen.lock().unwrap().len();
        let r = link.respond(&ctl(c, 0x5c00_0002, DISABLE_CHANNELS, &d(vec![(c, ch)], 0))).expect("answered");
        assert_eq!(r.rpc_result, NV_OK);
        assert!(matches!(
            seen.lock().unwrap().last().copied(),
            Some(ChanStatement::DisableChannels { client, disable: true, only_scheduling: false, rewind_gp_put: false, list }) if client == c && list.as_slice() == [(c, ch)]
        ));
        assert_eq!(seen.lock().unwrap().len(), n + 1);
        let r = link.respond(&ctl(c, 0x5c00_0002, DISABLE_CHANNELS, &d(vec![(c, ch), (0xc1d0_000c, 0xcafe_0001)], 0))).expect("refused");
        assert_eq!(r.rpc_result, NV_ERR_INSUFFICIENT_PERMISSIONS, "another client's channel");
        let r = link.respond(&ctl(c, 0x5c00_0002, DISABLE_CHANNELS, &d(vec![(c, ch)], 0xffff_8000_0000_1000))).expect("refused");
        assert_eq!(r.rpc_result, NV_ERR_INVALID_ARGUMENT, "an async preempt event");
        assert_eq!(seen.lock().unwrap().len(), n + 1, "neither refusal reached the plane");
    }

    #[test]
    fn internal_clients_are_the_guest_rms_own() {
        assert!(is_rm_internal_client(0xc1e0_0006));
        assert!(!is_rm_internal_client(0xc1d0_0006), "the user base");
        assert!(!is_rm_internal_client(0xe000_0001), "a VF client");
    }
}
