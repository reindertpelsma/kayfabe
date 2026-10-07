//! ★ OWNER_RULINGS §U (2026-10-07) — **`NV50_DEFERRED_API_CLASS` (`0x5080`): the registration
//! controls and the bundles they carry, read at the GUEST's measured layout.**
//!
//! What the real GSP receives (measured, Windows 580.88, VFIO boots 8/9/10, `gsp.jsonl`): only
//! `NV5080_CTRL_CMD_DEFERRED_API` (`0x50800101`) as `GSP_RM_CONTROL` (fn 76), `paramsSize` 584,
//! answered `NV_OK` with the params echoed. Why that and not `_V2` (`ogkm-580.95.05:
//! src/nvidia/generated/g_deferred_api_nvoc.c:183-230`, flags decoded against
//! `inc/kernel/rmapi/control.h:170-347`):
//!
//! | control | flags | reaches the GSP |
//! |---|---|---|
//! | `_DEFERRED_API` `0x50800101` | `0x50048` NON_PRIVILEGED, ROUTE_TO_PHYSICAL | yes, as itself |
//! | `_REMOVE_API` `0x50800102` | `0x50048` NON_PRIVILEGED, ROUTE_TO_PHYSICAL | yes, as itself |
//! | `_DEFERRED_API_V2` `0x50800103` | `0x10008` NON_PRIVILEGED | no: the guest's CPU-RM forwards it as `_INTERNAL` (`deferred_api.c:388-394`) |
//! | `_DEFERRED_API_INTERNAL` `0x50800104` | `0x500c8` NON_PRIVILEGED, ROUTE_TO_PHYSICAL, INTERNAL | yes |
//!
//! Every command id, flag value, flag bit position and layout here comes from the driver matrix
//! (`crate::generated::matrix`, gcc + DWARF per ogkm tag; the flag bit positions are the
//! compiler's own `(0?X)`/`(1?X)` of the header's DRF ranges). A version where one is absent gets
//! `None`, and the caller refuses the control by name. ⊘ Nothing is typed by hand except the
//! handle-namespace constants of [`HandleRules`], which live in a `.c` file the matrix cannot
//! compile (cited there).
//!
//! The bundle is decoded only into the fields kayfabe acts on; the rest of the guest's bytes are
//! kept as opaque, bounded bytes (the real RM copies the whole struct too, `deferred_api.c:101`).

use crate::DriverVersion;
use crate::generated::matrix as m;
use crate::matrix::Resolved;

/// `NV_OK`.
pub const NV_OK: u32 = 0;
/// `NV_ERR_INSUFFICIENT_RESOURCES` (`nvstatuscodes.h:55`).
pub const NV_ERR_INSUFFICIENT_RESOURCES: u32 = 0x1a;
/// `NV_ERR_INVALID_ARGUMENT` (`nvstatuscodes.h:60`) — the trigger's answer for a command it does
/// not know (`deferred_api.c:544-551`).
pub const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
/// `NV_ERR_INVALID_DATA` (`nvstatuscodes.h:66`) — the trigger's answer for an unknown handle
/// (`_Class5080GetDeferredApiInfo`, `deferred_api.c:135`).
pub const NV_ERR_INVALID_DATA: u32 = 0x25;
/// `NV_ERR_INVALID_OBJECT_HANDLE` (`nvstatuscodes.h:80`) — a registration whose handle is not a
/// valid new handle of the object's client, or already registered on the object
/// (`deferred_api.c:76-87`).
pub const NV_ERR_INVALID_OBJECT_HANDLE: u32 = 0x33;
/// `NV_ERR_INVALID_PARAM_STRUCT` (`nvstatuscodes.h:87`) — params of the wrong size.
pub const NV_ERR_INVALID_PARAM_STRUCT: u32 = 0x3a;
/// `NV_ERR_NO_MEMORY` (`nvstatuscodes.h:110`) — what `_Class5080AddDeferredApi` answers when its
/// allocation fails (`deferred_api.c:112-113`); kayfabe answers it at its own per-object,
/// per-client and per-VM bounds ([`Bounds`]).
pub const NV_ERR_NO_MEMORY: u32 = 0x51;
/// `NV_ERR_NOT_SUPPORTED` (`nvstatuscodes.h:115`).
pub const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
/// `NV_ERR_GENERIC` (`nvstatuscodes.h:28`) — `_REMOVE_API` of a handle the object does not hold
/// (`_Class5080DelDeferredApi`, `deferred_api.c:181`).
pub const NV_ERR_GENERIC: u32 = 0xffff;

/// Which registration control, by the id it arrived as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// `NV5080_CTRL_CMD_DEFERRED_API` — `NV5080_CTRL_DEFERRED_API_PARAMS`.
    V1,
    /// `NV5080_CTRL_CMD_DEFERRED_API_V2` — `NV5080_CTRL_DEFERRED_API_V2_PARAMS` (never routed to
    /// the GSP by a guest RM; served as the GSP's own V2 body would: register, `deferred_api.c:388`).
    V2,
    /// `NV5080_CTRL_CMD_DEFERRED_API_INTERNAL` — the V2 params (`typedef`, `ctrl5080.h`).
    Internal,
}

/// What one 5080 control id is at a version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// A registration in `Form`.
    Register(Form),
    /// `NV5080_CTRL_CMD_REMOVE_API`.
    Remove,
}

/// Where one params struct puts the five header fields and the bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamsLayout {
    /// `sizeof`.
    pub size: usize,
    /// `hApiHandle`.
    pub handle_off: usize,
    /// `cmd`.
    pub cmd_off: usize,
    /// `flags`.
    pub flags_off: usize,
    /// `hClientVA`.
    pub client_va_off: usize,
    /// `hDeviceVA`.
    pub device_va_off: usize,
    /// `api_bundle` (the union).
    pub bundle_off: usize,
    /// Its bytes.
    pub bundle_len: usize,
}

impl ParamsLayout {
    fn of(runs: &'static crate::matrix::StructRuns, v: DriverVersion) -> Option<Self> {
        let l = Resolved::of(runs, v).ok()?;
        let b = l.need("api_bundle").ok()?;
        Some(Self {
            size: l.size(),
            handle_off: l.need("hApiHandle").ok()?.off(),
            cmd_off: l.need("cmd").ok()?.off(),
            flags_off: l.need("flags").ok()?.off(),
            client_va_off: l.need("hClientVA").ok()?.off(),
            device_va_off: l.need("hDeviceVA").ok()?.off(),
            bundle_off: b.off(),
            bundle_len: b.bytes()?,
        })
    }
}

/// The two DRF fields of `flags`: bit positions and the values that matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagBits {
    /// `_FLAGS_DELETE` low bit (a one-bit field).
    pub delete_bit: u32,
    /// `_FLAGS_DELETE_EXPLICIT`.
    pub delete_explicit: u32,
    /// `_FLAGS_WAIT_FOR_TLB_FLUSH` low bit.
    pub wait_bit: u32,
    /// `_FLAGS_WAIT_FOR_TLB_FLUSH_TRUE`.
    pub wait_true: u32,
}

/// One registration's two flags, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    /// `DELETE_EXPLICIT`: the trigger does not delete the entry; only `_REMOVE_API` does.
    pub explicit_delete: bool,
    /// `WAIT_FOR_TLB_FLUSH_TRUE`: an implicit delete waits for a later deferred TLB invalidate
    /// on the same object (`deferred_api.c:657-670`, `:185-227`).
    pub wait_tlb_flush: bool,
    /// The raw word as the guest wrote it (no other bit has a meaning; kept for the log).
    pub raw: u32,
}

/// The eight commands a trigger may run (`deferred_api.c:465-551`), by their driver-matrix ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InnerCmds {
    /// `NV2080_CTRL_CMD_GPU_INITIALIZE_CTX`.
    pub initialize_ctx: u32,
    /// `NV2080_CTRL_CMD_GPU_PROMOTE_CTX`.
    pub promote_ctx: u32,
    /// `NV2080_CTRL_CMD_GPU_EVICT_CTX`.
    pub evict_ctx: u32,
    /// `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO`.
    pub update_channel_info: u32,
    /// `NV2080_CTRL_CMD_DMA_INVALIDATE_TLB`.
    pub invalidate_tlb: u32,
    /// `NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND`.
    pub zcull_bind: u32,
    /// `NV2080_CTRL_CMD_GR_CTXSW_PM_BIND`.
    pub pm_bind: u32,
    /// `NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND`.
    pub preemption_bind: u32,
}

/// The fields of the bundle members kayfabe reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BundleOffsets {
    // NV2080_CTRL_GPU_INITIALIZE_CTX_PARAMS
    init_size: usize,
    init_engine: usize,
    init_h_client: usize,
    init_chid: usize,
    init_chan_client: usize,
    init_object: usize,
    init_virt_mem: usize,
    init_phys: usize,
    init_phys_attr: usize,
    init_dma: usize,
    init_index: usize,
    init_bytes: usize,
    // NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS (the scalar prefix; entries go to the direct decoder)
    promote_size: usize,
    promote_engine: usize,
    promote_h_client: usize,
    promote_chid: usize,
    promote_chan_client: usize,
    promote_object: usize,
    promote_virt_mem: usize,
    promote_va: usize,
    promote_bytes: usize,
    promote_entry_count: usize,
    // NV2080_CTRL_GPU_EVICT_CTX_PARAMS
    evict_size: usize,
    evict_engine: usize,
    evict_h_client: usize,
    evict_chid: usize,
    evict_chan_client: usize,
    evict_object: usize,
    // NV2080_CTRL_DMA_INVALIDATE_TLB_PARAMS
    tlb_size: usize,
    tlb_vaspace: usize,
    // NV2080_CTRL_GR_CTXSW_ZCULL_BIND_PARAMS
    zcull_size: usize,
    zcull_channel: usize,
    zcull_va: usize,
    zcull_mode: usize,
}

impl BundleOffsets {
    fn at(v: DriverVersion) -> Option<Self> {
        let i = Resolved::of(&m::NV2080_CTRL_GPU_INITIALIZE_CTX_PARAMS, v).ok()?;
        let p = Resolved::of(&m::NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS, v).ok()?;
        let e = Resolved::of(&m::NV2080_CTRL_GPU_EVICT_CTX_PARAMS, v).ok()?;
        let t = Resolved::of(&m::NV2080_CTRL_DMA_INVALIDATE_TLB_PARAMS, v).ok()?;
        let z = Resolved::of(&m::NV2080_CTRL_GR_CTXSW_ZCULL_BIND_PARAMS, v).ok()?;
        let o = |r: &Resolved, f: &'static str| r.need(f).ok().map(crate::matrix::FieldAt::off);
        Some(Self {
            init_size: i.size(),
            init_engine: o(&i, "engineType")?,
            init_h_client: o(&i, "hClient")?,
            init_chid: o(&i, "ChID")?,
            init_chan_client: o(&i, "hChanClient")?,
            init_object: o(&i, "hObject")?,
            init_virt_mem: o(&i, "hVirtMemory")?,
            init_phys: o(&i, "physAddress")?,
            init_phys_attr: o(&i, "physAttr")?,
            init_dma: o(&i, "hDmaHandle")?,
            init_index: o(&i, "index")?,
            init_bytes: o(&i, "size")?,
            promote_size: p.size(),
            promote_engine: o(&p, "engineType")?,
            promote_h_client: o(&p, "hClient")?,
            promote_chid: o(&p, "ChID")?,
            promote_chan_client: o(&p, "hChanClient")?,
            promote_object: o(&p, "hObject")?,
            promote_virt_mem: o(&p, "hVirtMemory")?,
            promote_va: o(&p, "virtAddress")?,
            promote_bytes: o(&p, "size")?,
            promote_entry_count: o(&p, "entryCount")?,
            evict_size: e.size(),
            evict_engine: o(&e, "engineType")?,
            evict_h_client: o(&e, "hClient")?,
            evict_chid: o(&e, "ChID")?,
            evict_chan_client: o(&e, "hChanClient")?,
            evict_object: o(&e, "hObject")?,
            tlb_size: t.size(),
            tlb_vaspace: o(&t, "hVASpace")?,
            zcull_size: z.size(),
            zcull_channel: o(&z, "hChannel")?,
            zcull_va: o(&z, "vMemPtr")?,
            zcull_mode: o(&z, "zcullMode")?,
        })
    }
}

/// ★ Everything kayfabe reads of the deferred-API surface at ONE guest driver version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefApiAbi {
    /// The version it was resolved at.
    pub version: DriverVersion,
    /// `NV5080_CTRL_CMD_DEFERRED_API`.
    pub cmd_deferred: u32,
    /// `NV5080_CTRL_CMD_REMOVE_API`.
    pub cmd_remove: u32,
    /// `NV5080_CTRL_CMD_DEFERRED_API_V2`.
    pub cmd_v2: u32,
    /// `NV5080_CTRL_CMD_DEFERRED_API_INTERNAL` (absent before 555.42.02: measured `ABSENT`).
    pub cmd_internal: Option<u32>,
    /// `NV5080_CTRL_DEFERRED_API_PARAMS`.
    pub v1: ParamsLayout,
    /// `NV5080_CTRL_DEFERRED_API_V2_PARAMS` (= `_INTERNAL_PARAMS`).
    pub v2: ParamsLayout,
    /// `sizeof(NV5080_CTRL_REMOVE_API_PARAMS)`.
    pub remove_size: usize,
    /// `NV5080_CTRL_REMOVE_API_PARAMS.hApiHandle`.
    pub remove_handle_off: usize,
    /// The flags' fields.
    pub flags: FlagBits,
    /// The trigger's command ids.
    pub cmds: InnerCmds,
    bundle: BundleOffsets,
}

/// A registration as the guest sent it — the header decoded, the bundle bounded and opaque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    /// Which control carried it.
    pub form: Form,
    /// `hApiHandle`: the key the trigger's `0x200` data names.
    pub handle: u32,
    /// `cmd`: NOT checked at registration (`deferred_api.c:60-116`); the trigger checks it.
    pub cmd: u32,
    /// `flags`.
    pub flags: Flags,
    /// `hClientVA` (read by the real RM only for `DMA_INVALIDATE_TLB`; kayfabe IGNORES it, §U.2).
    pub client_va: u32,
    /// `hDeviceVA` (likewise ignored).
    pub device_va: u32,
    /// `api_bundle`, exactly the union's bytes.
    pub bundle: Vec<u8>,
}

/// Why a 5080 control's params were refused before any table was touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The params are not the version's struct.
    Size {
        /// What arrived.
        got: usize,
        /// The struct's size.
        want: usize,
    },
}

/// The bundle of a registration, decoded for the command it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bundle {
    /// `NV2080_CTRL_GPU_INITIALIZE_CTX_PARAMS` (`ctrl2080gpu.h:1039-1103`).
    InitializeCtx {
        /// `engineType`.
        engine_type: u32,
        /// `hClient` (of `hVirtMemory`).
        h_client: u32,
        /// `ChID` (deprecated).
        chid: u32,
        /// `hChanClient`.
        chan_client: u32,
        /// `hObject`: a channel or a TSG of `chan_client`.
        object: u32,
        /// `hVirtMemory`.
        virt_memory: u32,
        /// `physAddress`.
        phys_address: u64,
        /// `physAttr` (`APERTURE 1:0`, `GPU_CACHEABLE 2:2`, `PRESERVE_CTX 3:3`).
        phys_attr: u32,
        /// `hDmaHandle`.
        dma_handle: u32,
        /// `index`.
        index: u32,
        /// `size`.
        size: u64,
    },
    /// The scalar prefix of `NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS` (`ctrl2080gpu.h`); the entries
    /// stay in the bundle for the direct path's own decoder.
    PromoteCtx {
        /// `engineType`.
        engine_type: u32,
        /// `hClient`.
        h_client: u32,
        /// `ChID`.
        chid: u32,
        /// `hChanClient`.
        chan_client: u32,
        /// `hObject`.
        object: u32,
        /// `hVirtMemory`.
        virt_memory: u32,
        /// `virtAddress`.
        virt_address: u64,
        /// `size`.
        size: u64,
        /// `entryCount`.
        entry_count: u32,
    },
    /// `NV2080_CTRL_GPU_EVICT_CTX_PARAMS`.
    EvictCtx {
        /// `engineType`.
        engine_type: u32,
        /// `hClient`.
        h_client: u32,
        /// `ChID`.
        chid: u32,
        /// `hChanClient`.
        chan_client: u32,
        /// `hObject`.
        object: u32,
    },
    /// `NV2080_CTRL_DMA_INVALIDATE_TLB_PARAMS` — `hVASpace` only, and it is IGNORED (§U.2: the
    /// host invalidates the triggering channel's OWN space).
    InvalidateTlb {
        /// `hVASpace` as the guest wrote it.
        vaspace: u32,
    },
    /// `NV2080_CTRL_GR_CTXSW_ZCULL_BIND_PARAMS`.
    ZcullBind {
        /// `hChannel`.
        channel: u32,
        /// `vMemPtr`.
        va: u64,
        /// `zcullMode`.
        mode: u32,
    },
    /// `GR_CTXSW_PM_BIND`, `GR_CTXSW_PREEMPTION_BIND`, `FIFO_UPDATE_CHANNEL_INFO`: known, never
    /// served (see the trigger's refusals) — so not decoded.
    Unserved {
        /// The command.
        cmd: u32,
        /// Its name.
        what: &'static str,
    },
    /// A command the trigger does not run (`deferred_api.c:544-551`: `NV_ERR_INVALID_ARGUMENT`).
    Unknown {
        /// The command.
        cmd: u32,
    },
}

/// The rules `serverutilValidateNewResourceHandle` applies (`rs_utils.c:227-236` →
/// `clientValidateNewResourceHandle(.., bRestrict = TRUE)`, `rs_client.c:1442-1467`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandleRules;

impl HandleRules {
    /// `RS_FW_UNIQUE_HANDLE_BASE` — `ogkm-580.95.05: src/nvidia/src/kernel/rmapi/client.c:45`. A
    /// `#define` in a `.c` file, so the matrix cannot compile it; `[measured
    /// traces/deferred_api_falsify_20261007 F7a, host 595.91.07]` `0xc9f00000` is refused.
    pub const FW_HANDLE_BASE: u32 = 0xc9f0_0000;
    /// `RS_UNIQUE_HANDLE_RANGE` (`inc/libraries/resserv/rs_client.h:38`): the restricted range's
    /// size (`client.c:149-150`).
    pub const HANDLE_RANGE: u32 = 0x0008_0000;

    /// A handle a client's deferred entry may NOT use, before the client's live handles are
    /// asked: `0`, the client's own handle, the firmware generator's range.
    #[must_use]
    pub const fn structurally_refused(client: u32, handle: u32) -> bool {
        handle == 0
            || handle == client
            || (handle >= Self::FW_HANDLE_BASE
                && handle - Self::FW_HANDLE_BASE < Self::HANDLE_RANGE)
    }
}

fn get_u32(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn get_u64(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from(get_u32(b, off)?) | (u64::from(get_u32(b, off.checked_add(4)?)?) << 32))
}

impl DefApiAbi {
    /// The surface at `version`; `None` where any id, flag or layout is not measured there.
    #[must_use]
    pub fn at(version: DriverVersion) -> Option<DefApiAbi> {
        let val = |r: &'static crate::matrix::ValueRuns| r.at_u32(version).ok().flatten();
        let one_bit = |lo: &'static crate::matrix::ValueRuns,
                       hi: &'static crate::matrix::ValueRuns|
         -> Option<u32> {
            let (l, h) = (val(lo)?, val(hi)?);
            (l == h && l < 32).then_some(l)
        };
        let remove = Resolved::of(&m::NV5080_CTRL_REMOVE_API_PARAMS, version).ok()?;
        Some(DefApiAbi {
            version,
            cmd_deferred: val(&m::CTRL_CMDS_NV5080_CTRL_CMD_DEFERRED_API)?,
            cmd_remove: val(&m::CTRL_CMDS_NV5080_CTRL_CMD_REMOVE_API)?,
            cmd_v2: val(&m::CTRL_CMDS_NV5080_CTRL_CMD_DEFERRED_API_V2)?,
            cmd_internal: val(&m::CTRL_CMDS_NV5080_CTRL_CMD_DEFERRED_API_INTERNAL),
            v1: ParamsLayout::of(&m::NV5080_CTRL_DEFERRED_API_PARAMS, version)?,
            v2: ParamsLayout::of(&m::NV5080_CTRL_DEFERRED_API_V2_PARAMS, version)?,
            remove_size: remove.size(),
            remove_handle_off: remove.need("hApiHandle").ok()?.off(),
            flags: FlagBits {
                delete_bit: one_bit(
                    &m::NV5080_FLAGS_DELETE_LO_NV5080_FLAGS_DELETE_LO,
                    &m::NV5080_FLAGS_DELETE_HI_NV5080_FLAGS_DELETE_HI,
                )?,
                delete_explicit: val(
                    &m::CTRL_CMDS_NV5080_CTRL_CMD_DEFERRED_API_FLAGS_DELETE_EXPLICIT,
                )?,
                wait_bit: one_bit(
                    &m::NV5080_FLAGS_WAIT_TLB_LO_NV5080_FLAGS_WAIT_TLB_LO,
                    &m::NV5080_FLAGS_WAIT_TLB_HI_NV5080_FLAGS_WAIT_TLB_HI,
                )?,
                wait_true: val(
                    &m::CTRL_CMDS_NV5080_CTRL_CMD_DEFERRED_API_FLAGS_WAIT_FOR_TLB_FLUSH_TRUE,
                )?,
            },
            cmds: InnerCmds {
                initialize_ctx: val(&m::CTRL_CMDS_NV2080_CTRL_CMD_GPU_INITIALIZE_CTX)?,
                promote_ctx: val(&m::CTRL_CMDS_NV2080_CTRL_CMD_GPU_PROMOTE_CTX)?,
                evict_ctx: val(&m::CTRL_CMDS_NV2080_CTRL_CMD_GPU_EVICT_CTX)?,
                update_channel_info: val(&m::CTRL_CMDS_NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO)?,
                invalidate_tlb: val(&m::CTRL_CMDS_NV2080_CTRL_CMD_DMA_INVALIDATE_TLB)?,
                zcull_bind: val(&m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND)?,
                pm_bind: val(&m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_PM_BIND)?,
                preemption_bind: val(&m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND)?,
            },
            bundle: BundleOffsets::at(version)?,
        })
    }

    /// Which 5080 control `cmd` is, or `None` (not one of the four).
    #[must_use]
    pub fn control(&self, cmd: u32) -> Option<Control> {
        if cmd == self.cmd_deferred {
            Some(Control::Register(Form::V1))
        } else if cmd == self.cmd_v2 {
            Some(Control::Register(Form::V2))
        } else if Some(cmd) == self.cmd_internal {
            Some(Control::Register(Form::Internal))
        } else if cmd == self.cmd_remove {
            Some(Control::Remove)
        } else {
            None
        }
    }

    /// The params layout of a registration form.
    #[must_use]
    pub const fn layout(&self, form: Form) -> &ParamsLayout {
        match form {
            Form::V1 => &self.v1,
            Form::V2 | Form::Internal => &self.v2,
        }
    }

    /// ★ Decode a registration's params: EXACTLY the form's struct, nothing longer or shorter.
    ///
    /// # Errors
    /// [`DecodeError::Size`].
    pub fn decode_registration(
        &self,
        form: Form,
        params: &[u8],
    ) -> Result<Registration, DecodeError> {
        let l = self.layout(form);
        let size_err = DecodeError::Size {
            got: params.len(),
            want: l.size,
        };
        if params.len() != l.size {
            return Err(size_err);
        }
        let w = |off| get_u32(params, off).ok_or(size_err);
        let raw = w(l.flags_off)?;
        let field = |bit: u32| (raw >> bit) & 1;
        Ok(Registration {
            form,
            handle: w(l.handle_off)?,
            cmd: w(l.cmd_off)?,
            flags: Flags {
                explicit_delete: field(self.flags.delete_bit) == self.flags.delete_explicit,
                wait_tlb_flush: field(self.flags.wait_bit) == self.flags.wait_true,
                raw,
            },
            client_va: w(l.client_va_off)?,
            device_va: w(l.device_va_off)?,
            bundle: params
                .get(l.bundle_off..l.bundle_off + l.bundle_len)
                .ok_or(size_err)?
                .to_vec(),
        })
    }

    /// ★ Decode a `_REMOVE_API`'s `hApiHandle`.
    ///
    /// # Errors
    /// [`DecodeError::Size`].
    pub fn decode_remove(&self, params: &[u8]) -> Result<u32, DecodeError> {
        let e = DecodeError::Size {
            got: params.len(),
            want: self.remove_size,
        };
        if params.len() != self.remove_size {
            return Err(e);
        }
        get_u32(params, self.remove_handle_off).ok_or(e)
    }

    /// ★ The bundle of a registered `cmd`, decoded at the member's own driver-matrix layout (a union
    /// member starts at the union's first byte). A member larger than the form's union, or a
    /// bundle shorter than the member, is `Unknown` (never read past the bytes).
    #[must_use]
    pub fn decode_bundle(&self, cmd: u32, bundle: &[u8]) -> Bundle {
        let b = &self.bundle;
        let c = &self.cmds;
        let fits = |size: usize| size <= bundle.len();
        let w = |off: usize| get_u32(bundle, off).unwrap_or(0);
        let q = |off: usize| get_u64(bundle, off).unwrap_or(0);
        if cmd == c.initialize_ctx && fits(b.init_size) {
            Bundle::InitializeCtx {
                engine_type: w(b.init_engine),
                h_client: w(b.init_h_client),
                chid: w(b.init_chid),
                chan_client: w(b.init_chan_client),
                object: w(b.init_object),
                virt_memory: w(b.init_virt_mem),
                phys_address: q(b.init_phys),
                phys_attr: w(b.init_phys_attr),
                dma_handle: w(b.init_dma),
                index: w(b.init_index),
                size: q(b.init_bytes),
            }
        } else if cmd == c.promote_ctx && fits(b.promote_size) {
            Bundle::PromoteCtx {
                engine_type: w(b.promote_engine),
                h_client: w(b.promote_h_client),
                chid: w(b.promote_chid),
                chan_client: w(b.promote_chan_client),
                object: w(b.promote_object),
                virt_memory: w(b.promote_virt_mem),
                virt_address: q(b.promote_va),
                size: q(b.promote_bytes),
                entry_count: w(b.promote_entry_count),
            }
        } else if cmd == c.evict_ctx && fits(b.evict_size) {
            Bundle::EvictCtx {
                engine_type: w(b.evict_engine),
                h_client: w(b.evict_h_client),
                chid: w(b.evict_chid),
                chan_client: w(b.evict_chan_client),
                object: w(b.evict_object),
            }
        } else if cmd == c.invalidate_tlb && fits(b.tlb_size) {
            Bundle::InvalidateTlb {
                vaspace: w(b.tlb_vaspace),
            }
        } else if cmd == c.zcull_bind && fits(b.zcull_size) {
            Bundle::ZcullBind {
                channel: w(b.zcull_channel),
                va: q(b.zcull_va),
                mode: w(b.zcull_mode),
            }
        } else if cmd == c.pm_bind {
            Bundle::Unserved {
                cmd,
                what: "GR_CTXSW_PM_BIND",
            }
        } else if cmd == c.preemption_bind {
            Bundle::Unserved {
                cmd,
                what: "GR_CTXSW_PREEMPTION_BIND",
            }
        } else if cmd == c.update_channel_info {
            Bundle::Unserved {
                cmd,
                what: "FIFO_UPDATE_CHANNEL_INFO",
            }
        } else {
            Bundle::Unknown { cmd }
        }
    }

    /// `sizeof(NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS)` at this version — the bytes of a deferred
    /// promote bundle the direct path's decoder reads.
    #[must_use]
    pub const fn promote_params_size(&self) -> usize {
        self.bundle.promote_size
    }
}

/// `NV2080_CTRL_GPU_INITIALIZE_CTX_PRESERVE_CTX` 3:3 — `ctrl2080gpu.h:1111`. ⊘ A field of
/// `physAttr` the matrix does not measure (a DRF range); read through this ONE named bit.
/// `PRESERVE_CTX_YES` (= 1) asks the RM to adopt an already-initialised context promoted onto a
/// different channel (`ctrl2080gpu.h:1105-1109`) — a context migration a host twin cannot do.
pub const INITIALIZE_CTX_PRESERVE_CTX_BIT: u32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    /// The Windows 580.88 guest's twin tag (`guestsysinfo.rs`: its `nvBldVer.h` Windows block).
    const W: DriverVersion = DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    };

    /// ★ The matrix layout equals what the 2026-10-05 VFIO boots 8/9/10 carried (`gsp.jsonl`):
    /// `paramsSize` 584, the bundle at 24.
    #[test]
    fn the_v1_params_are_the_584_bytes_the_real_gsp_received() {
        let a = DefApiAbi::at(W).expect("580.65.06 measured");
        assert_eq!(a.v1.size, 584);
        assert_eq!(
            (
                a.v1.handle_off,
                a.v1.cmd_off,
                a.v1.flags_off,
                a.v1.client_va_off,
                a.v1.device_va_off
            ),
            (0, 4, 8, 12, 16)
        );
        assert_eq!((a.v1.bundle_off, a.v1.bundle_len), (24, 560));
        assert_eq!(a.cmd_deferred, 0x5080_0101);
        assert_eq!(a.cmd_remove, 0x5080_0102);
        assert_eq!(a.cmd_v2, 0x5080_0103);
        assert_eq!(a.cmd_internal, Some(0x5080_0104));
        assert_eq!(
            a.flags,
            FlagBits {
                delete_bit: 0,
                delete_explicit: 1,
                wait_bit: 1,
                wait_true: 1
            }
        );
        assert_eq!(a.cmds.initialize_ctx, 0x2080_012d);
        assert_eq!(a.cmds.promote_ctx, 0x2080_012b);
        assert_eq!(a.cmds.evict_ctx, 0x2080_012c);
        assert_eq!(a.cmds.update_channel_info, 0x2080_1116);
        assert_eq!(a.cmds.invalidate_tlb, 0x2080_2502);
        assert_eq!(a.cmds.zcull_bind, 0x2080_1208);
        assert_eq!(a.cmds.pm_bind, 0x2080_1209);
        assert_eq!(a.cmds.preemption_bind, 0x2080_1211);
        // `_INTERNAL` is measured ABSENT before 555.42.02.
        let old = DefApiAbi::at(DriverVersion {
            major: 550,
            minor: 90,
            patch: 7,
        })
        .expect("550 measured");
        assert_eq!(old.cmd_internal, None);
        assert_eq!(old.control(0x5080_0104), None);
    }

    /// The exact VFIO-10 registrations (idx 3066/3067, `ctl-10.txt`): the bundle words decode to
    /// the deferred INITIALIZE/PROMOTE of client c1d0002b's TSG ff0e0000.
    #[test]
    fn the_vfio_registrations_decode_to_the_virtual_context_init_and_promote() {
        let a = DefApiAbi::at(W).unwrap();
        let reg = |h: u32, cmd: u32, words: &[u32]| {
            let mut p = vec![0u8; 584];
            p[0..4].copy_from_slice(&h.to_le_bytes());
            p[4..8].copy_from_slice(&cmd.to_le_bytes());
            for (i, w) in words.iter().enumerate() {
                p[24 + 4 * i..28 + 4 * i].copy_from_slice(&w.to_le_bytes());
            }
            a.decode_registration(Form::V1, &p).unwrap()
        };
        let init = reg(
            0x4000_0002,
            0x2080_012d,
            &[
                1,
                0xc1d0_0029,
                0x10,
                0xc1d0_002b,
                0xff0e_0000,
                0,
                0x0360_a000,
                0,
                0,
                0,
                0x10,
                0,
            ],
        );
        assert_eq!(init.handle, 0x4000_0002);
        assert_eq!(init.flags, Flags::default());
        assert_eq!(
            a.decode_bundle(init.cmd, &init.bundle),
            Bundle::InitializeCtx {
                engine_type: 1,
                h_client: 0xc1d0_0029,
                chid: 0x10,
                chan_client: 0xc1d0_002b,
                object: 0xff0e_0000,
                virt_memory: 0,
                phys_address: 0x0360_a000,
                phys_attr: 0,
                dma_handle: 0,
                index: 0x10,
                size: 0
            }
        );
        let promote = reg(
            0x4000_0003,
            0x2080_012b,
            &[
                1,
                0xc1d0_0029,
                0x10,
                0xc1d0_002b,
                0xff0e_0000,
                0,
                0x11000,
                0,
                0xdc300,
                0,
                0,
            ],
        );
        assert_eq!(
            a.decode_bundle(promote.cmd, &promote.bundle),
            Bundle::PromoteCtx {
                engine_type: 1,
                h_client: 0xc1d0_0029,
                chid: 0x10,
                chan_client: 0xc1d0_002b,
                object: 0xff0e_0000,
                virt_memory: 0,
                virt_address: 0x11000,
                size: 0xdc300,
                entry_count: 0
            }
        );
        assert_eq!(a.promote_params_size(), 560);
    }

    #[test]
    fn flags_sizes_and_hostile_shapes() {
        let a = DefApiAbi::at(W).unwrap();
        let mut p = vec![0u8; 584];
        p[8..12].copy_from_slice(&0xffff_fffcu32.to_le_bytes());
        let r = a.decode_registration(Form::V1, &p).unwrap();
        assert!(
            !r.flags.explicit_delete && !r.flags.wait_tlb_flush,
            "only bits 0 and 1 mean anything"
        );
        p[8..12].copy_from_slice(&3u32.to_le_bytes());
        let r = a.decode_registration(Form::V1, &p).unwrap();
        assert!(r.flags.explicit_delete && r.flags.wait_tlb_flush);
        assert_eq!(r.bundle.len(), 560);
        for n in [0, 583, 585, 4096] {
            assert_eq!(
                a.decode_registration(Form::V1, &vec![0; n]),
                Err(DecodeError::Size { got: n, want: 584 })
            );
        }
        assert_eq!(a.decode_remove(&[1, 0, 0, 0x40]), Ok(0x4000_0001));
        assert!(a.decode_remove(&[0; 8]).is_err());
        // An unknown command, and a bundle too short for its member, never read past the bytes.
        assert_eq!(
            a.decode_bundle(0x2080_0101, &[0; 560]),
            Bundle::Unknown { cmd: 0x2080_0101 }
        );
        assert_eq!(
            a.decode_bundle(0x2080_012d, &[0; 8]),
            Bundle::Unknown { cmd: 0x2080_012d }
        );
        assert_eq!(
            a.decode_bundle(0x2080_1209, &[0; 560]),
            Bundle::Unserved {
                cmd: 0x2080_1209,
                what: "GR_CTXSW_PM_BIND"
            }
        );
        assert_eq!(a.control(0x5080_0102), Some(Control::Remove));
        assert_eq!(a.control(0x5080_0105), None);
    }

    #[test]
    fn the_handle_namespace_rules_match_the_measured_f7a_refusals() {
        let c = 0xcafe_0001;
        for h in [0, c, 0xc9f0_0000, 0xc9f7_ffff] {
            assert!(HandleRules::structurally_refused(c, h), "{h:#x}");
        }
        for h in [
            1,
            0x4000_0000,
            0xffff_ffff,
            0xcaf0_0000,
            0xc9ef_ffff,
            0xc9f8_0000,
        ] {
            assert!(!HandleRules::structurally_refused(c, h), "{h:#x}");
        }
    }
}
