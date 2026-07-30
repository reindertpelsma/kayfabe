//! Axis A, assembled: build a [`GspAbi`] for one guest driver version.
//!
//! # ★★ This existed only in the test harness, and that was the finding
//!
//! `kayfabe_gsp::GspFsm::new` takes a [`GspAbi`] — a bundle of the `msgq` constants, the
//! element layout, the RPC envelope and the init-args offsets. Every one of those is
//! already version-keyed data in `kayfabe-abi`. But the code that *assembled* them into a
//! `GspAbi` lived in `tests/src/gspworld.rs`, i.e. nothing that ships could construct the
//! value the FSM needs. Stage Q4 needs one, so the assembly moved here and the harness
//! delegates — which means the ~3 000 lines of existing GSP conformance tests now drive
//! **this** function, rather than a second assembly that could drift from it.
//!
//! # ★ What is genuinely a constant here, and what is not
//!
//! Two things are stated inline rather than read from a table, and both are marked. The
//! `msgq` library constants are compile-time in the driver at both vendored tags and their
//! citation is on the field. `RPC_HEADER_VERSION` likewise. Everything else — the element
//! layout, `queueElementSizeMax`, the init-args shape — comes from
//! [`kayfabe_abi::versions::table_for`], which refuses below its floor rather than
//! nearest-neighbouring.

use kayfabe_abi::DriverVersion;
use kayfabe_abi::generated::rpc as rpcids;
use kayfabe_gsp::{
    ElementLayout, FunctionCodes, GspAbi, GspFault, InitArgsLayout, MsgqAbi, RpcAbi, TransportHdr,
};

/// The RPC function ids, **from the generated table** rather than transcribed.
///
/// ★ Every one of these is `NV_VGPU_MSG_FUNCTION_*` / `NV_VGPU_MSG_EVENT_*` as the
/// generator lifted it out of `ogkm`'s `rpc_global_enums.h` X-macro. Writing them out by
/// hand — which is what the test harness did — puts a second copy of an NVIDIA constant
/// outside the Axis-A quarantine, which decision #2 exists to prevent.
pub const FUNCTIONS: FunctionCodes = FunctionCodes {
    set_guest_system_info: rpcids::NV_VGPU_MSG_FUNCTION_SET_GUEST_SYSTEM_INFO,
    free: rpcids::NV_VGPU_MSG_FUNCTION_FREE,
    dup_object: rpcids::NV_VGPU_MSG_FUNCTION_DUP_OBJECT,
    unloading_guest_driver: rpcids::NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER,
    get_gsp_static_info: rpcids::NV_VGPU_MSG_FUNCTION_GET_GSP_STATIC_INFO,
    continuation_record: rpcids::NV_VGPU_MSG_FUNCTION_CONTINUATION_RECORD,
    gsp_set_system_info: rpcids::NV_VGPU_MSG_FUNCTION_GSP_SET_SYSTEM_INFO,
    set_registry: rpcids::NV_VGPU_MSG_FUNCTION_SET_REGISTRY,
    gsp_rm_control: rpcids::NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
    gsp_rm_alloc: rpcids::NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
    gsp_init_done: rpcids::NV_VGPU_MSG_EVENT_GSP_INIT_DONE,
    post_event: rpcids::NV_VGPU_MSG_EVENT_POST_EVENT,
};

/// `MSGQ_VERSION` — byte-identical and on the same line at both vendored tags, only the
/// path moves (`ogkm-610: src/nvidia/inc/libraries/msgq/msgq_priv.h:37-38`,
/// `ogkm-580: src/common/shared/msgq/inc/msgq/msgq_priv.h:37-38`).
const MSGQ_VERSION: u32 = 0;
/// `MSGQ_MSG_SIZE_MIN` (same citation).
const MSGQ_MSG_SIZE_MIN: u32 = 16;
/// `MSGQ_FLAGS_SWAP_RX` (`ogkm-610:`/`ogkm-580: msgq.h:30-39`).
const MSGQ_FLAGS_SWAP_RX: u32 = 1;
/// `RM_PAGE_SIZE` — the **driver's** page size, never the host's
/// (`ogkm-610:`/`ogkm-580: src/nvidia/inc/kernel/gpu/mem_mgr/rm_page_size.h:38`).
const RM_PAGE_SIZE: u32 = 4096;
/// The RPC envelope's `header_version`.
const RPC_HEADER_VERSION: u32 = 0x0300_0000;

/// Assemble the whole Axis-A bundle for a guest driver version.
///
/// # Errors
///
/// [`kayfabe_abi::wire::AbiError::NoTableForVersion`] below the supported floor, or
/// whatever [`ElementLayout::new`] refuses for a table row that does not describe a real
/// element. Both arrive as a [`GspFault`], which already has a `From` for each — the GSP
/// crate's own fault type is the vocabulary the only consumer speaks.
/// ★ There is no nearest-neighbour fallback: answering a driver with another version's
/// element layout is a wire disagreement that surfaces as a checksum failure inside the
/// guest, with nothing on this side saying why.
pub fn gsp_abi_for(version: DriverVersion) -> Result<GspAbi, GspFault> {
    let table = kayfabe_abi::versions::table_for(version)?;
    let wire = table.gsp_element_wire();
    let transport = match wire.transport() {
        None => TransportHdr::None,
        Some(t) => TransportHdr::Mctp {
            header_off: t.header_off,
            header_word: t.header_word,
            nvdm_off: t.nvdm_off,
            nvdm_word: t.nvdm_word,
        },
    };
    let element = ElementLayout::new(
        wire.hdr_size(),
        wire.checksum_off(),
        wire.seqnum_off(),
        wire.elem_count_off(),
        transport,
    )?;
    let init = table.gsp_init_args_wire();
    Ok(GspAbi {
        msgq: MsgqAbi {
            version: MSGQ_VERSION,
            msg_size_min: MSGQ_MSG_SIZE_MIN,
            swap_rx_flag: MSGQ_FLAGS_SWAP_RX,
            region_page_size: RM_PAGE_SIZE,
        },
        element,
        rpc: RpcAbi {
            header_version: RPC_HEADER_VERSION,
            codes: FUNCTIONS,
        },
        element_size_max: table.gsp_element_size_max(),
        init_args: InitArgsLayout {
            // `NvLength` is `size_t`, so there are 4 pad bytes after the `u32` at +8 — the
            // C's `+0/+8/+16/+24` are right, and right because they were hard-coded rather
            // than derived (`C: src/qemu/nvkvm_gpu_emul.c:3411-3425`).
            shared_mem_pa_off: 0,
            pte_count_off: 8,
            cmd_queue_off_off: 16,
            stat_queue_off_off: 24,
            min_size: init.min_size(),
            element_hdr_size_off: init.element_hdr_size_off(),
        },
        driver: *table,
    })
}
