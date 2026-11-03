#!/bin/sh
# Create the blobless clone used by sweep.py and set its sparse patterns.
# usage: sweep_setup.sh <clone-dir>      (needs network once; afterwards blobs arrive per checkout)
set -eu
d=${1:?usage: sweep_setup.sh <clone-dir>}
[ -d "$d/.git" ] || git clone --filter=blob:none --no-checkout https://github.com/NVIDIA/open-gpu-kernel-modules "$d"
git -C "$d" fetch --tags -q
git -C "$d" sparse-checkout init --no-cone
cat > "$d/.git/info/sparse-checkout" <<PAT
/src/common/sdk/nvidia/inc/
/src/nvidia/generated/g_*_nvoc.c
/src/nvidia/generated/g_rpc*.h
/src/nvidia/generated/g_gpu_class_list.c
control.h
rpc_global_enums.h
rpc_headers.h
gsp_fw_wpr_meta.h
gsp_fw_sr_meta.h
gspifpub.h
gsp_proxy_reg.h
gsp_init_args.h
message_queue.h
message_queue_priv.h
gsp_static_config.h
gsp_fw_heap.h
msgq.h
msgq_priv.h
libos_init_args.h
rmgspseq.h
rmlsfm.h
rmifrif.h
rmflcnbl.h
rmRiscvUcode.h
flcnifcmn.h
nv-firmware.h
kernel_gsp_fwsec.c
kernel_gsp_booter.c
kernel_gsp_booter_tu102.c
kern_fsp.h
kern_fsp_cot_payload.h
kern_fsp_retval.h
fsp_emem_channels.h
fsp_tnvl_rpc.h
fsp_caps_query_rpc.h
fsp_clock_boost_rpc.h
nvdm_format.h
mctp_format.h
PAT
