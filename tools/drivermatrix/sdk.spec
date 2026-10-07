# sdk.spec — BOTH driver axes: the public SDK structures a guest's CPU-RM sends to our GSP
# (GSP_RM_CONTROL / GSP_RM_ALLOC bodies) and the very same structures kf-host sends to the host
# RM through ioctls. Measured per ogkm tag by `dm.py` (gcc + DWARF); V3_DRIVER_MATRIX.md §3.
#
# kind      name                      env  headers                                                              arg
macros    ctrl_cmds                   sdk  nvtypes.h,ctrl/*.h,ctrl/*/*.h   (NV0000|NV0080|NV2080|NV0073|NVA06F|NVA06C|NVC36F|NV90F1|NV83DE|NVC86F|NVA06E|NV0090|NVB0CC|NV00F8|NV00DE|NV0041|NV503C|NVA0BC|NVC56F|NVC96F|NV5080)_CTRL_CMD_[A-Z0-9_]+
typedefs  ctrl_params                 sdk  nvtypes.h,ctrl/*.h,ctrl/*/*.h   (NV0000|NV0080|NV2080|NV0073|NVA06F|NVA06C|NVC36F|NV90F1|NV83DE|NVC86F|NVA06E|NV0090|NVB0CC|NV00F8|NV00DE|NV0041|NV503C|NVA0BC|NV5080)_CTRL_[A-Z0-9_]*PARAMS[A-Z0-9_]*
macros    class_ids                   rm   g_allclasses.h                                                       [A-Z0-9_]+
typedefs  alloc_params                rm   nvos.h,=SDK_ALL_CLASSES_INCLUDE_FULL_HEADER,g_allclasses.h,alloc/alloc_channel.h|nvos.h,=SDK_ALL_CLASSES_INCLUDE_FULL_HEADER,g_allclasses.h   (NV_[A-Z0-9_]*ALLOC[A-Z0-9_]*PARAM[A-Z0-9_]*|NV[0-9A-F]{4}_ALLOC(ATION)?_PARAM[A-Z0-9_]*|NV_[A-Z0-9_]*_ALLOCATION_PARAMETERS|NV_CHANNEL_ALLOC_PARAMS|NV_CHANNELGPFIFO_ALLOCATION_PARAMETERS)
typedefs  nvos_params                 sdk  nvtypes.h,nvos.h                                                     NVOS[0-9A-F]+_PARAMETERS|NV_[A-Z0-9_]*_PARAMS
#
# ── the SDK's own sizes and numberings behind the served controls ────────────────────────────
macros    nv2080_engine_type          sdk  nvtypes.h,class/cl2080_notification.h                               NV2080_ENGINE_TYPE_[A-Z0-9_]+
macros    nv2080_notifiers            sdk  nvtypes.h,class/cl2080_notification.h                               NV2080_NOTIFIERS_[A-Z0-9_]+
macros    ctrl_limits                 sdk  nvtypes.h,ctrl/*.h,ctrl/*/*.h   (NV0000|NV0080|NV2080|NVA06F|NVA06C|NVC36F|NV90F1)_CTRL_[A-Z0-9_]*(MAX|SIZE|COUNT|INDEX)[A-Z0-9_]*
# 2026-10-07 (Windows Code43): control ids spelled without `_CMD_`, and the enumerated values the
# served FIFO/PERF controls compare against (`kf_rm::chanlink`, `kf_rm::vfguest`).
macros    ctrl_values                 sdk  nvtypes.h,ctrl/*.h,ctrl/*/*.h   (NV0080_CTRL_DMA_SET_DEFAULT_VASPACE|NV0080_CTRL_FIFO_SET_CHANNEL_PROPERTIES_[A-Z0-9_]+|NV2080_CTRL_PERF_POWER_SOURCE_[A-Z0-9_]+)
macros    intr_consts                 sdk  nvtypes.h,ctrl/ctrl2080/ctrl2080mc.h,?ctrl/ctrl2080/ctrl2080internal.h   NV2080_INTR_[A-Z0-9_]+
# 2026-10-07 (deferred API, OWNER_RULINGS §U): the bit positions of the two DRF fields of
# `NV5080_CTRL_DEFERRED_API_PARAMS.flags` (`ctrl5080.h`: `_FLAGS_DELETE 0:0`,
# `_FLAGS_WAIT_FOR_TLB_FLUSH 1:1`), evaluated by the compiler the way NVIDIA's own `DRF_*` macros
# do (`(0?X)` is the low bit, `(1?X)` the high bit) — never typed by hand.
const     nv5080_flags_delete_lo     sdk  nvtypes.h,ctrl/ctrl5080.h   (0?NV5080_CTRL_CMD_DEFERRED_API_FLAGS_DELETE)
const     nv5080_flags_delete_hi     sdk  nvtypes.h,ctrl/ctrl5080.h   (1?NV5080_CTRL_CMD_DEFERRED_API_FLAGS_DELETE)
const     nv5080_flags_wait_tlb_lo   sdk  nvtypes.h,ctrl/ctrl5080.h   (0?NV5080_CTRL_CMD_DEFERRED_API_FLAGS_WAIT_FOR_TLB_FLUSH)
const     nv5080_flags_wait_tlb_hi   sdk  nvtypes.h,ctrl/ctrl5080.h   (1?NV5080_CTRL_CMD_DEFERRED_API_FLAGS_WAIT_FOR_TLB_FLUSH)
