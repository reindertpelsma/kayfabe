# os.spec — HOST driver axis (Dh) only: the OS-facing ioctl surface kf-host and kf-linux-raw use
# on the host's /dev/nvidiactl, /dev/nvidia<N> and /dev/nvidia-uvm — escape numbers, the
# nv-ioctl.h frontend structs, and the UVM ioctl parameter blocks. The RM parameter blocks those
# escapes carry (NVOS*, controls, alloc params) are in sdk.spec. Measured per ogkm tag by `dm.py`.
#
# kind      name                      env  headers                                                              arg
macros    nv_escapes                  rm   nv_escape.h                                                          NV_ESC_[A-Z0-9_]+
macros    nv_ioctl_consts             uvm  nvtypes.h,nv-ioctl-numbers.h,nv-ioctl.h                              (NV_IOCTL_[A-Z0-9_]+|NV_ESC_[A-Z0-9_]+|NV_RM_API_VERSION_[A-Z0-9_]+)
typedefs  nv_ioctl_structs            uvm  nvtypes.h,nv-ioctl-numbers.h,nv-ioctl.h                              nv_ioctl_[a-z0-9_]+_t|nv_ioctl_[a-z0-9_]+
typedefs  uvm_params                  uvm  nvtypes.h,uvm_types.h,uvm_ioctl.h                                    UVM_[A-Z0-9_]+_PARAMS
macros    uvm_ioctls                  uvm  nvtypes.h,uvm_types.h,uvm_ioctl.h                                    UVM_[A-Z0-9_]+
