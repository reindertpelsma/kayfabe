# Native Windows boundary cohort

**STATUS: RESEARCH, 2026-10-05.**

| Run | Records | Attachments | Gaps / drops / unstable |
|---|---:|---|---|
| boundary-vfio-8 | 8226 | 1:2, 2:8224 | 0 / 0 / 15 |
| boundary-vfio-9 | 8195 | 1:2, 2:8193 | 0 / 0 / 8 |
| boundary-vfio-10 | 8208 | 1:2, 2:8206 | 0 / 0 / 7 |
| boundary-kayfabe-2 | 365 | n/a | unavailable / unavailable / unavailable |
| boundary-kayfabe-3 | 365 | n/a | unavailable / unavailable / unavailable |
| boundary-kayfabe-4 | 365 | n/a | unavailable / unavailable / unavailable |
| boundary-kayfabe-5 | 365 | n/a | unavailable / unavailable / unavailable |

## Observed declined or nonzero Kayfabe controls

Native columns list observed **inner control** statuses and counts; these are not paired replies or matched inputs.

| ID / public source name | Kayfabe results | Native status observations |
|---|---|---|
| 0x00730109 NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_CONFIG | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x0073011d NV0073_CTRL_CMD_SYSTEM_GET_CONNECTOR_TABLE | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x0073012c NV0073_CTRL_CMD_SYSTEM_VRR_DISPLAY_INFO | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×3; boundary-vfio-9: 0x00000000 ×3; boundary-vfio-10: 0x00000000 ×3 |
| 0x0073013d NV0073_CTRL_CMD_SYSTEM_QUERY_DISPLAY_IDS_WITH_MUX | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x0073014b NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_I2C_SUPPORT | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x00730285 NV0073_CTRL_CMD_SPECIFIC_GET_ACPI_DOD_DISPLAY_PORT_ATTACHMENT | boundary-kayfabe-2: none ×5; boundary-kayfabe-3: none ×5; boundary-kayfabe-4: none ×5; boundary-kayfabe-5: none ×5 | boundary-vfio-8: 0x0000001f ×4, 0x00000056 ×7; boundary-vfio-9: 0x0000001f ×4, 0x00000056 ×7; boundary-vfio-10: 0x0000001f ×4, 0x00000056 ×7 |
| 0x00731369 NV0073_CTRL_CMD_DP_GET_CAPS | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×3; boundary-vfio-9: 0x00000000 ×3; boundary-vfio-10: 0x00000000 ×3 |
| 0x00800106 NV0080_CTRL_CMD_BIF_GET_PCIE_POWER_CONTROL_MASK | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x00800292 NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2 | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x00801306 NV0080_CTRL_CMD_FB_GET_COMPBIT_STORE_INFO | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x2080012b NV2080_CTRL_CMD_GPU_PROMOTE_CTX | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×19; boundary-vfio-9: 0x00000000 ×19; boundary-vfio-10: 0x00000000 ×19 |
| 0x2080012f NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000056 ×1; boundary-vfio-9: 0x00000056 ×1; boundary-vfio-10: 0x00000056 ×1 |
| 0x2080013f NV2080_CTRL_CMD_GPU_GET_OEM_BOARD_INFO | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20800160 NV2080_CTRL_CMD_GPU_GET_VPR_CAPS | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×2; boundary-vfio-9: 0x00000000 ×2; boundary-vfio-10: 0x00000000 ×2 |
| 0x20800173 NV2080_CTRL_CMD_GPU_QUERY_FUNCTION_STATUS | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000056 ×1; boundary-vfio-9: 0x00000056 ×1; boundary-vfio-10: 0x00000056 ×1 |
| 0x2080017e NV2080_CTRL_CMD_GPU_GET_VMMU_SEGMENT_SIZE | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x2080080b NV2080_CTRL_CMD_BIOS_GET_UEFI_SUPPORT | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20800a2e NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_ROP_INFO | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20800a30 NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PPC_MASKS | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20800a38 NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE | boundary-kayfabe-2: none ×3; boundary-kayfabe-3: none ×3; boundary-kayfabe-4: none ×3; boundary-kayfabe-5: none ×3 | boundary-vfio-8: 0x00000000 ×29; boundary-vfio-9: 0x00000000 ×29; boundary-vfio-10: 0x00000000 ×29 |
| 0x20800a3f NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_TRACE_DEFINES | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20800a80 NV2080_CTRL_CMD_INTERNAL_PERF_GPU_BOOST_SYNC_GET_INFO | boundary-kayfabe-2: none ×2; boundary-kayfabe-3: none ×2; boundary-kayfabe-4: none ×2; boundary-kayfabe-5: none ×2 | boundary-vfio-8: 0x00000000 ×2; boundary-vfio-9: 0x00000000 ×2; boundary-vfio-10: 0x00000000 ×2 |
| 0x20800a87 NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000056 ×1; boundary-vfio-9: 0x00000056 ×1; boundary-vfio-10: 0x00000056 ×1 |
| 0x20800aaf NV2080_CTRL_CMD_INTERNAL_GET_ENABLED_SEC2_CLASSES | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20800ab8 NV2080_CTRL_CMD_INTERNAL_GET_PCIE_P2P_CAPS | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20800afe NV2080_CTRL_CMD_INTERNAL_INIT_USER_SHARED_DATA | boundary-kayfabe-2: none ×2; boundary-kayfabe-3: none ×2; boundary-kayfabe-4: none ×2; boundary-kayfabe-5: none ×2 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20800aff NV2080_CTRL_CMD_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×2; boundary-vfio-9: 0x00000000 ×2; boundary-vfio-10: 0x00000000 ×2 |
| 0x20800b03 NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER_V2 | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20800b05 NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_THROTTLE_CTRL | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000056 ×1; boundary-vfio-9: 0x00000056 ×1; boundary-vfio-10: 0x00000056 ×1 |
| 0x20801110 not named by this source vocabulary | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20801111 not named by this source vocabulary | boundary-kayfabe-2: none ×2; boundary-kayfabe-3: none ×2; boundary-kayfabe-4: none ×2; boundary-kayfabe-5: none ×2 | boundary-vfio-8: 0x00000000 ×13; boundary-vfio-9: 0x00000000 ×13; boundary-vfio-10: 0x00000000 ×13 |
| 0x20801303 NV2080_CTRL_CMD_FB_GET_INFO_V2 | boundary-kayfabe-2: 0x00000056 ×1; boundary-kayfabe-3: 0x00000056 ×1; boundary-kayfabe-4: 0x00000056 ×1; boundary-kayfabe-5: 0x00000056 ×1 | boundary-vfio-8: 0x00000000 ×2; boundary-vfio-9: 0x00000000 ×2; boundary-vfio-10: 0x00000000 ×2 |
| 0x20801823 NV2080_CTRL_CMD_BUS_GET_INFO_V2 | boundary-kayfabe-2: 0x00000000 ×1, 0x00000056 ×1; boundary-kayfabe-3: 0x00000000 ×1, 0x00000056 ×1; boundary-kayfabe-4: 0x00000000 ×1, 0x00000056 ×1; boundary-kayfabe-5: 0x00000000 ×1, 0x00000056 ×1 | boundary-vfio-8: 0x00000000 ×32; boundary-vfio-9: 0x00000000 ×32; boundary-vfio-10: 0x00000000 ×32 |
| 0x2080205b NV2080_CTRL_CMD_PERF_SET_POWERSTATE | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20802068 NV2080_CTRL_CMD_PERF_GET_CURRENT_PSTATE | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×2 |
| 0x20802801 NV2080_CTRL_CMD_LPWR_DIFR_CTRL | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20802806 not named by this source vocabulary | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20802a0f NV2080_CTRL_CMD_INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20802a12 NV2080_CTRL_CMD_CE_IS_DECOMP_LCE_ENABLED | boundary-kayfabe-2: none ×8; boundary-kayfabe-3: none ×8; boundary-kayfabe-4: none ×8; boundary-kayfabe-5: none ×8 | boundary-vfio-8: 0x00000000 ×8; boundary-vfio-9: 0x00000000 ×8; boundary-vfio-10: 0x00000000 ×8 |
| 0x208081ea NV2080_CTRL_CMD_RUSD_GET_SUPPORTED_FEATURES | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20808524 not named by this source vocabulary | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x20809004 not named by this source vocabulary | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×34; boundary-vfio-9: 0x00000000 ×34; boundary-vfio-10: 0x00000000 ×34 |
| 0x2080a02f not named by this source vocabulary | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000056 ×1; boundary-vfio-9: 0x00000056 ×1; boundary-vfio-10: 0x00000056 ×1 |
| 0x2080a060 not named by this source vocabulary | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x2080a70a not named by this source vocabulary | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×1; boundary-vfio-9: 0x00000000 ×1; boundary-vfio-10: 0x00000000 ×1 |
| 0x2080a801 not named by this source vocabulary | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×2; boundary-vfio-9: 0x00000000 ×2; boundary-vfio-10: 0x00000000 ×2 |
| 0x20810108 not named by this source vocabulary | boundary-kayfabe-2: none ×1; boundary-kayfabe-3: none ×1; boundary-kayfabe-4: none ×1; boundary-kayfabe-5: none ×1 | boundary-vfio-8: 0x00000000 ×2; boundary-vfio-9: 0x00000000 ×2; boundary-vfio-10: 0x00000000 ×2 |

## Interpretation and limits

- All native coverage remains partial, even with queue sequence zero and zero observed gaps/drops.
- Attachment generations are observer attachment epochs, not proven separate boots. No prefix is deduplicated.
- Physical status-queue direction is called reply by the decoder; source-defined unsolicited events are separated here.
- No request/reply pairing or cross-arm phase alignment is attempted. First-observation indices refer to different streams.
- Native success for the same numeric control ID does not establish matching object, parameters, phase, or required virtual-device behavior.
- Only controls observed by Kayfabe are compared. Other native traffic may include DWM/userspace activity; it is not a kernel implementation backlog.
- Native outer RPC and inner control status differ. Kayfabe result=none stays null; no numeric status is inferred.
- invalid_elements counts sampled empty/stale slots too, not malformed submitted RPCs. Unstable snapshots remain explicit coverage limits.
- Compiler names associate exact source numbers, not runtime policy. No name means unresolved in this source vocabulary.
- Metadata/identity declarations are supplied assertions; compare QEMU executable hashes in addition to revision labels.
- Output excludes raw messages, addresses, handles, QPCs and capability words. Review supplied metadata before publication.
