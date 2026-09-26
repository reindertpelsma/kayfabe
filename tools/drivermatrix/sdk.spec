# sdk.spec — BOTH driver axes: the public SDK structures a guest's CPU-RM sends to our GSP
# (GSP_RM_CONTROL / GSP_RM_ALLOC bodies) and the very same structures kf-host sends to the host
# RM through ioctls. Measured per ogkm tag by `dm.py` (gcc + DWARF); V3_DRIVER_MATRIX.md §3.
#
# kind      name                      env  headers                                                              arg
macros    ctrl_cmds                   sdk  nvtypes.h,ctrl/*.h,ctrl/*/*.h   (NV0000|NV0080|NV2080|NV0073|NVA06F|NVA06C|NVC36F|NV90F1|NV83DE|NVC86F|NVA06E|NV0090|NVB0CC|NV00F8|NV00DE|NV0041|NV503C|NVA0BC|NVC56F|NVC96F)_CTRL_CMD_[A-Z0-9_]+
typedefs  ctrl_params                 sdk  nvtypes.h,ctrl/*.h,ctrl/*/*.h   (NV0000|NV0080|NV2080|NV0073|NVA06F|NVA06C|NVC36F|NV90F1|NV83DE|NVC86F|NVA06E|NV0090|NVB0CC|NV00F8|NV00DE|NV0041|NV503C|NVA0BC)_CTRL_[A-Z0-9_]*PARAMS[A-Z0-9_]*
macros    class_ids                   rm   g_allclasses.h                                                       [A-Z0-9_]+
typedefs  alloc_params                rm   nvos.h,=SDK_ALL_CLASSES_INCLUDE_FULL_HEADER,g_allclasses.h,alloc/alloc_channel.h|nvos.h,=SDK_ALL_CLASSES_INCLUDE_FULL_HEADER,g_allclasses.h   (NV_[A-Z0-9_]*ALLOC[A-Z0-9_]*PARAM[A-Z0-9_]*|NV[0-9A-F]{4}_ALLOC(ATION)?_PARAM[A-Z0-9_]*|NV_[A-Z0-9_]*_ALLOCATION_PARAMETERS|NV_CHANNEL_ALLOC_PARAMS|NV_CHANNELGPFIFO_ALLOCATION_PARAMETERS)
typedefs  nvos_params                 sdk  nvtypes.h,nvos.h                                                     NVOS[0-9A-F]+_PARAMETERS|NV_[A-Z0-9_]*_PARAMS
