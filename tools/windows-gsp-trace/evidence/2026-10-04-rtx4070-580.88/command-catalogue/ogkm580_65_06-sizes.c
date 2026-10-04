/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#include <stdio.h>
#include <ctrl/ctrl0073/ctrl0073dfp.h>
#include <ctrl/ctrl0073/ctrl0073dp.h>
#include <ctrl/ctrl0073/ctrl0073specific.h>
#include <ctrl/ctrl0073/ctrl0073system.h>
#include <ctrl/ctrl0080/ctrl0080dma.h>
#include <ctrl/ctrl0080/ctrl0080fifo.h>
#include <ctrl/ctrl0080/ctrl0080gpu.h>
#include <ctrl/ctrl2080/ctrl2080bus.h>
#include <ctrl/ctrl2080/ctrl2080ce.h>
#include <ctrl/ctrl2080/ctrl2080ecc.h>
#include <ctrl/ctrl2080/ctrl2080fb.h>
#include <ctrl/ctrl2080/ctrl2080fifo.h>
#include <ctrl/ctrl2080/ctrl2080gpu.h>
#include <ctrl/ctrl2080/ctrl2080gr.h>
#include <ctrl/ctrl2080/ctrl2080internal.h>
#include <ctrl/ctrl2080/ctrl2080nvlink.h>
#include <ctrl/ctrl5080.h>
#include <ctrl/ctrl90e7/ctrl90e7bbx.h>
#include <ctrl/ctrl90f1.h>
#include <ctrl/ctrla06c.h>
#include <ctrl/ctrla06f/ctrla06fgpfifo.h>
#include <ctrl/ctrlc370/ctrlc370chnc.h>
#include <ctrl/ctrlc372/ctrlc372chnc.h>
int main(void) {
    printf("0x00730102 %zu\n", sizeof(NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS));
    printf("0x00730108 %zu\n", sizeof(NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS));
    printf("0x0073010a %zu\n", sizeof(NV0073_CTRL_SYSTEM_GET_HOTPLUG_STATE_PARAMS));
    printf("0x0073010c %zu\n", sizeof(NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS));
    printf("0x0073012c %zu\n", sizeof(NV0073_CTRL_SYSTEM_VRR_DISPLAY_INFO_PARAMS));
    printf("0x00730250 %zu\n", sizeof(NV0073_CTRL_SPECIFIC_GET_CONNECTOR_DATA_PARAMS));
    printf("0x0073028b %zu\n", sizeof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS));
    printf("0x007302a4 %zu\n", sizeof(NV0073_CTRL_SPECIFIC_DISPLAY_CHANGE_PARAMS));
    printf("0x00731140 %zu\n", sizeof(NV0073_CTRL_DFP_GET_INFO_PARAMS));
    printf("0x00731142 %zu\n", sizeof(NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS));
    printf("0x00731144 %zu\n", sizeof(NV0073_CTRL_DFP_SET_ELD_AUDIO_CAP_PARAMS));
    printf("0x0073114e %zu\n", sizeof(NV0073_CTRL_DFP_UPDATE_DYNAMIC_DFP_CACHE_PARAMS));
    printf("0x00731150 %zu\n", sizeof(NV0073_CTRL_DFP_SET_AUDIO_ENABLE_PARAMS));
    printf("0x00731152 %zu\n", sizeof(NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS));
    printf("0x00731341 %zu\n", sizeof(NV0073_CTRL_DP_AUXCH_CTRL_PARAMS));
    printf("0x00731343 %zu\n", sizeof(NV0073_CTRL_DP_CTRL_PARAMS));
    printf("0x00731359 %zu\n", sizeof(NV0073_CTRL_DP_SET_AUDIO_MUTESTREAM_PARAMS));
    printf("0x00731360 %zu\n", sizeof(NV0073_CTRL_DP_GET_LINK_CONFIG_PARAMS));
    printf("0x00731362 %zu\n", sizeof(NV0073_CTRL_CMD_DP_CONFIG_STREAM_PARAMS));
    printf("0x00731378 %zu\n", sizeof(NV0073_CTRL_CMD_DP_SET_STEREO_MSA_PROPERTIES_PARAMS));
    printf("0x00731381 %zu\n", sizeof(NV0073_CTRL_CMD_DP_SET_MSA_PROPERTIES_V2_PARAMS));
    printf("0x00800294 %zu\n", sizeof(NV0080_CTRL_GPU_GET_BRAND_CAPS_PARAMS));
    printf("0x0080170e %zu\n", sizeof(NV0080_CTRL_FIFO_GET_LATENCY_BUFFER_SIZE_PARAMS));
    printf("0x0080170f %zu\n", sizeof(NV0080_CTRL_FIFO_SET_CHANNEL_PROPERTIES_PARAMS));
    printf("0x00801812 %zu\n", sizeof(NV0080_CTRL_DMA_SET_DEFAULT_VASPACE_PARAMS));
    printf("0x20800102 %zu\n", sizeof(NV2080_CTRL_GPU_GET_INFO_V2_PARAMS));
    printf("0x2080012b %zu\n", sizeof(NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS));
    printf("0x2080012d %zu\n", sizeof(NV2080_CTRL_GPU_INITIALIZE_CTX_PARAMS));
    printf("0x2080012f %zu\n", sizeof(NV2080_CTRL_GPU_QUERY_ECC_STATUS_PARAMS));
    printf("0x2080014b %zu\n", sizeof(NV2080_CTRL_GPU_GET_INFOROM_OBJECT_VERSION_PARAMS));
    printf("0x20800156 %zu\n", sizeof(NV2080_CTRL_GPU_GET_INFOROM_IMAGE_VERSION_PARAMS));
    printf("0x20800157 0\n"); /* Public declaration has no parameters. */
    printf("0x208001a4 %zu\n", sizeof(NV2080_CTRL_GPU_GET_CHIP_DETAILS_PARAMS));
    printf("0x20800a38 %zu\n", sizeof(NV2080_CTRL_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE_PARAMS));
    printf("0x20800a9a %zu\n", sizeof(NV2080_CTRL_INTERNAL_PERF_BOOST_SET_PARAMS_2X));
    printf("0x20800af0 0\n"); /* Public declaration has no parameters. */
    printf("0x20800af1 0\n"); /* Public declaration has no parameters. */
    printf("0x20800af2 0\n"); /* Public declaration has no parameters. */
    printf("0x20800aff %zu\n", sizeof(NV2080_CTRL_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL_PARAMS));
    printf("0x2080110b %zu\n", sizeof(NV2080_CTRL_FIFO_DISABLE_CHANNELS_PARAMS));
    printf("0x20801208 %zu\n", sizeof(NV2080_CTRL_GR_CTXSW_ZCULL_BIND_PARAMS));
    printf("0x20801211 %zu\n", sizeof(NV2080_CTRL_GR_CTXSW_PREEMPTION_BIND_PARAMS));
    printf("0x2080121f %zu\n", sizeof(NV2080_CTRL_GR_GFX_POOL_QUERY_SIZE_PARAMS));
    printf("0x20801303 %zu\n", sizeof(NV2080_CTRL_FB_GET_INFO_V2_PARAMS));
    printf("0x20801322 %zu\n", sizeof(NV2080_CTRL_FB_GET_OFFLINED_PAGES_PARAMS));
    printf("0x20801344 %zu\n", sizeof(NV2080_CTRL_FB_GET_REMAPPED_ROWS_PARAMS));
    printf("0x20801347 %zu\n", sizeof(NV2080_CTRL_FB_GET_ROW_REMAPPER_HISTOGRAM_PARAMS));
    printf("0x20801357 %zu\n", sizeof(NV2080_CTRL_FB_DRAM_ENCRYPTION_INFOROM_SUPPORT_PARAMS));
    printf("0x20801813 %zu\n", sizeof(NV2080_CTRL_BUS_GET_PEX_COUNTERS_PARAMS));
    printf("0x20801819 %zu\n", sizeof(NV2080_CTRL_BUS_GET_PEX_UTIL_COUNTERS_PARAMS));
    printf("0x20801823 %zu\n", sizeof(NV2080_CTRL_BUS_GET_INFO_V2_PARAMS));
    printf("0x20801829 %zu\n", sizeof(NV2080_CTRL_CMD_BUS_GET_PCIE_REQ_ATOMICS_CAPS_PARAMS));
    printf("0x20801830 %zu\n", sizeof(NV2080_CTRL_CMD_BUS_GET_PCIE_CPL_ATOMICS_CAPS_PARAMS));
    printf("0x20802a08 %zu\n", sizeof(NV2080_CTRL_CE_GET_FAULT_METHOD_BUFFER_SIZE_PARAMS));
    printf("0x20803083 %zu\n", sizeof(NV2080_CTRL_NVLINK_GET_PLATFORM_INFO_PARAMS));
    printf("0x20803400 %zu\n", sizeof(NV2080_CTRL_ECC_GET_CLIENT_EXPOSED_COUNTERS_PARAMS));
    printf("0x20803401 %zu\n", sizeof(NV2080_CTRL_ECC_GET_VOLATILE_COUNTS_PARAMS));
    printf("0x20803404 %zu\n", sizeof(NV2080_CTRL_ECC_GET_REPAIR_STATUS_PARAMS));
    printf("0x50800101 %zu\n", sizeof(NV5080_CTRL_DEFERRED_API_PARAMS));
    printf("0x90e70113 %zu\n", sizeof(NV90E7_CTRL_BBX_GET_LAST_FLUSH_TIME_PARAMS));
    printf("0x90f10106 %zu\n", sizeof(NV90F1_CTRL_VASPACE_COPY_SERVER_RESERVED_PDES_PARAMS));
    printf("0xa06c0101 %zu\n", sizeof(NVA06C_CTRL_GPFIFO_SCHEDULE_PARAMS));
    printf("0xa06c0103 %zu\n", sizeof(NVA06C_CTRL_TIMESLICE_PARAMS));
    printf("0xa06c010a %zu\n", sizeof(NVA06C_CTRL_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS_PARAMS));
    printf("0xa06f0103 %zu\n", sizeof(NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS));
    printf("0xc3700104 %zu\n", sizeof(NVC370_CTRL_CMD_GET_CHANNEL_INFO_PARAMS));
    printf("0xc3720101 %zu\n", sizeof(NVC372_CTRL_IS_MODE_POSSIBLE_PARAMS));
    return 0;
}
