# gsp.spec — GUEST driver axis (Dg) only: the GSP firmware interface: what a stock guest driver expects of the GSP it talks
# to. Every entry is measured per ogkm tag by `dm.py` (gcc + DWARF); see V3_DRIVER_MATRIX.md §3.
#
# kind      name                      env  headers (comma list; =X #define, !X #undef)                        arg
#
# ── GSP boot, message queues, init arguments ──────────────────────────────────────────────────
struct    GSP_MSG_QUEUE_ELEMENT       rm   nvtypes.h,vgpu/vgpu_version.h,vgpu/rpc.h,=SDK_ALL_CLASSES_INCLUDE_FULL_HEADER,g_allclasses.h,!SDK_ALL_CLASSES_INCLUDE_FULL_HEADER,nverror.h,=RPC_STRUCTURES,=RPC_GENERIC_UNION,g_rpc-structures.h,!RPC_STRUCTURES,!RPC_GENERIC_UNION,=RPC_MESSAGE_STRUCTURES,=RPC_MESSAGE_GENERIC_UNION,g_rpc-message-header.h,gpu/gsp/message_queue_priv.h                                         GSP_MSG_QUEUE_ELEMENT
struct    msgqTxHeader                rm   msgq/msgq_priv.h                                                     msgqTxHeader
struct    msgqRxHeader                rm   msgq/msgq_priv.h                                                     msgqRxHeader
struct    MESSAGE_QUEUE_INIT_ARGUMENTS rm  gpu/gsp/gsp_init_args.h                                              MESSAGE_QUEUE_INIT_ARGUMENTS
struct    GSP_SR_INIT_ARGUMENTS       rm   gpu/gsp/gsp_init_args.h                                              GSP_SR_INIT_ARGUMENTS
struct    GSP_ARGUMENTS_CACHED        rm   gpu/gsp/gsp_init_args.h                                              GSP_ARGUMENTS_CACHED
struct    GspFwWprMeta                rm   nvtypes.h,gsp/gsp_fw_wpr_meta.h                                                GspFwWprMeta
struct    GspFwSRMeta                 rm   nvtypes.h,gsp/gsp_fw_sr_meta.h                                                 GspFwSRMeta
struct    LibosMemoryRegionInitArgument rm nvtypes.h,libos_init_args.h                                                  LibosMemoryRegionInitArgument
struct    GspStaticConfigInfo         rm   gpu/gsp/gsp_static_config.h                                          GspStaticConfigInfo
struct    GspSystemInfo               rm   gpu/gsp/gsp_static_config.h                                          GspSystemInfo
macros    gsp_fw_meta_consts          rm   nvtypes.h,gsp/gsp_fw_wpr_meta.h,gsp/gsp_fw_sr_meta.h                           (GSP_FW_WPR_META|GSP_FW_SR_META)_[A-Z0-9_]+
macros    gsp_msgq_consts             rm   nvtypes.h,vgpu/vgpu_version.h,vgpu/rpc.h,=SDK_ALL_CLASSES_INCLUDE_FULL_HEADER,g_allclasses.h,!SDK_ALL_CLASSES_INCLUDE_FULL_HEADER,nverror.h,=RPC_STRUCTURES,=RPC_GENERIC_UNION,g_rpc-structures.h,!RPC_STRUCTURES,!RPC_GENERIC_UNION,=RPC_MESSAGE_STRUCTURES,=RPC_MESSAGE_GENERIC_UNION,g_rpc-message-header.h,gpu/gsp/message_queue_priv.h,msgq/msgq_priv.h                        (GSP_MSG_QUEUE|MSGQ)_[A-Z0-9_]+
macros    libos_consts                rm   nvtypes.h,libos_init_args.h                                                    LIBOS_[A-Z0-9_]+
#
# ── RPC numbering and the vGPU protocol version ───────────────────────────────────────────────
enum      rpc_functions               rm   vgpu/rpc_global_enums.h                                              NV_VGPU_MSG_FUNCTION_NOP
enum      rpc_events                  rm   vgpu/rpc_global_enums.h                                              NV_VGPU_MSG_EVENT_FIRST_EVENT
macros    vgx_version                 rm   vgpu/vgpu_version.h                                                  VGX_(MAJOR|MINOR)_VERSION_NUMBER
macros    rpc_header_consts           rm   vgpu/rpc_headers.h                                                   (NV_VGPU_MSG_(HEADER_VERSION|SIGNATURE|RESULT)|MAX_GPC_COUNT|VGPU_MAX_REGOPS_PER_RPC|MAX_[A-Z0-9_]+)[A-Z0-9_]*
#
# ── RPC payload structures: every unversioned alias `rpc_*_v`, as rpc.c includes them ───────
typedefs  rpc_structs                 rm   nvtypes.h,vgpu/vgpu_version.h,vgpu/rpc.h,=SDK_ALL_CLASSES_INCLUDE_FULL_HEADER,g_allclasses.h,!SDK_ALL_CLASSES_INCLUDE_FULL_HEADER,nverror.h,=RPC_STRUCTURES,=RPC_GENERIC_UNION,g_rpc-structures.h,!RPC_STRUCTURES,!RPC_GENERIC_UNION,=RPC_MESSAGE_STRUCTURES,=RPC_MESSAGE_GENERIC_UNION,g_rpc-message-header.h   rpc_[a-z0-9_]+_v
#
#

#
# ── engine numbering the GSP-RM and the guest's CPU-RM must agree on ──────────────────────────
# RM_ENGINE_TYPE_* / MC_ENGINE_IDX_* are RM-internal numberings the guest reads back out of our
# init tables (FIFO_GET_DEVICE_INFO_TABLE, INTR_GET_KERNEL_TABLE, engineCaps bit positions).
macros    rm_engine_type              rm   nvtypes.h,gpu/gpu_engine_type.h                                      RM_ENGINE_TYPE_[A-Z0-9_]+
macros    mc_engine_idx               rm   nvtypes.h,gpu/intr/engine_idx.h                                      MC_ENGINE_IDX_[A-Z0-9_]+
