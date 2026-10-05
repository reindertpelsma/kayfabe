# Source associations only, compiled by tools/drivermatrix/dm.py; never a product policy.
enum rpc_functions rm vgpu/rpc_global_enums.h NV_VGPU_MSG_FUNCTION_NOP
enum rpc_events rm vgpu/rpc_global_enums.h NV_VGPU_MSG_EVENT_FIRST_EVENT
macros ctrl_cmds sdk nvtypes.h,ctrl/*.h,ctrl/*/*.h (NV[0-9A-F]+)_CTRL_CMD_[A-Z0-9_]+
