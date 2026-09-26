# ★ HOST-AXIS structs the other specs do not reach (V3_DRIVER_MATRIX.md §2.2; inventory 2026-09-26):
# kind    name                       env  headers                                         pattern
typedefs  host_ce_alloc              sdk  nvtypes.h,class/clb0b5sw.h                      NVB0B5_ALLOCATION_PARAMETERS
typedefs  host_zbc_ctrl              sdk  nvtypes.h,ctrl/ctrl9096.h                       NV9096_CTRL_[A-Z0-9_]*PARAMS[A-Z0-9_]*
macros    host_zbc_cmds              sdk  nvtypes.h,ctrl/ctrl9096.h                       NV9096_CTRL_CMD_[A-Z0-9_]+
typedefs  host_nvos_wrappers         rm   nvos.h,nv-unix-nvos-params-wrappers.h           nv_ioctl_nvos[0-9]+_parameters_with_fd
