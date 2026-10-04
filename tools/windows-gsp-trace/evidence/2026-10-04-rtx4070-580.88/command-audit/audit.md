# Captured Windows commands versus v3-windows

**STATUS: RESEARCH, 2026-10-04. Static inventory, not a successful Kayfabe replay.**

Audited revision: `c50fad9ac485f53d45d4ea77a21cb7206267c65a`. Windows 580.88 uses ABI 580.65.06 via the source-derived Windows twin mapping; display row ADA. Names use local OGKM `ogkm-580.159.04`.

Generic allowlist/GSS/BinAPI rules are not handlers. A specific handler still has payload, object-state, layout and host-fact gates. No generic passthrough is counted.

## Counts

| Measure | Count |
| --- | --- |
| control_ids | 129 |
| class_ids | 20 |
| control_ids_with_sdk_name | 64 |
| control_ids_without_sdk_name | 65 |
| native_control_error_ids | 12 |
| native_ok_ids_without_handler | 92 |
| gfx_pool_query_records | 0 |

| Control implementation category | Distinct IDs |
| --- | --- |
| capture-derived constant reply | 1 |
| channel handler | 6 |
| conditional authored host-fact query | 3 |
| display handler | 10 |
| empirical identity reply | 1 |
| init-table handler | 6 |
| no specific handler | 102 |

## RPC envelopes

| Function | Name | Directions | Reply status counts |
| --- | --- | --- | --- |
| 10 | FREE | {'reply': 23, 'request': 23} | {'0x0': 23} |
| 71 | CONTINUATION_RECORD | {'reply': 7, 'request': 7} | {'0x0': 7} |
| 76 | GSP_RM_CONTROL | {'reply': 1999, 'request': 1999} | {'0x0': 1999} |
| 103 | GSP_RM_ALLOC | {'reply': 236, 'request': 236} | {'0x0': 236} |
| 4099 | POST_EVENT | {'reply': 5} | {'0x0': 5} |

## All directly observed controls

Sizes are declared parameter bytes. Status counts are independent reply observations, not inferred request/reply pairs. An SDK name says nothing about implementation coverage.

| ID | SDK name | Requests / replies | Request sizes | Native reply statuses | Kayfabe path |
| --- | --- | --- | --- | --- | --- |
| 0x00730102 | NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS | 7 / 7 | {12: 7} | {'0x0': 7} | display handler |
| 0x00730108 | NV0073_CTRL_CMD_SYSTEM_GET_CONNECT_STATE | 2 / 2 | {16: 2} | {'0x0': 2} | display handler |
| 0x0073010a | NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_STATE | 1 / 1 | {12: 1} | {'0x0': 1} | no specific handler |
| 0x0073010c | NV0073_CTRL_CMD_SYSTEM_GET_ACTIVE | 25 / 25 | {16: 25} | {'0x0': 25} | display handler |
| 0x00730122 | unresolved in this SDK | 1 / 1 | {8: 1} | {'0x0': 1} | no specific handler |
| 0x00730128 | unresolved in this SDK | 1 / 1 | {20: 1} | {'0x0': 1} | no specific handler |
| 0x0073012c | NV0073_CTRL_CMD_SYSTEM_VRR_DISPLAY_INFO | 1 / 1 | {12: 1} | {'0x0': 1} | no specific handler |
| 0x00730250 | NV0073_CTRL_CMD_SPECIFIC_GET_CONNECTOR_DATA | 15 / 15 | {72: 15} | {'0x0': 15} | display handler |
| 0x00730280 | unresolved in this SDK | 8 / 8 | {12: 8} | {'0x0': 8} | no specific handler |
| 0x00730282 | unresolved in this SDK | 4 / 4 | {2600: 4} | {'0x56': 4} | no specific handler |
| 0x0073028b | NV0073_CTRL_CMD_SPECIFIC_OR_GET_INFO | 29 / 29 | {56: 29} | {'0x0': 29} | display handler |
| 0x007302a4 | NV0073_CTRL_CMD_SPECIFIC_DISPLAY_CHANGE | 2 / 2 | {16: 2} | {'0x0': 2} | display handler |
| 0x007302a5 | unresolved in this SDK | 12 / 12 | {16: 12} | {'0x0': 12} | no specific handler |
| 0x00731140 | NV0073_CTRL_CMD_DFP_GET_INFO | 8 / 8 | {16: 8} | {'0x0': 8} | display handler |
| 0x00731142 | NV0073_CTRL_CMD_DFP_GET_DISPLAYPORT_DONGLE_INFO | 7 / 7 | {16: 7} | {'0x0': 7} | display handler |
| 0x00731144 | NV0073_CTRL_CMD_DFP_SET_ELD_AUDIO_CAPS | 16 / 17 | {120: 16} | {'0x0': 17} | no specific handler |
| 0x0073114e | NV0073_CTRL_CMD_DFP_UPDATE_DYNAMIC_DFP_CACHE | 1 / 1 | {24: 1} | {'0x0': 1} | no specific handler |
| 0x00731150 | NV0073_CTRL_CMD_DFP_SET_AUDIO_ENABLE | 1 / 1 | {12: 1} | {'0x0': 1} | no specific handler |
| 0x00731152 | NV0073_CTRL_CMD_DFP_ASSIGN_SOR | 3 / 3 | {80: 3} | {'0x0': 3} | no specific handler |
| 0x00731341 | NV0073_CTRL_CMD_DP_AUXCH_CTRL | 5 / 5 | {48: 5} | {'0x0': 5} | no specific handler |
| 0x00731343 | NV0073_CTRL_CMD_DP_CTRL | 1 / 1 | {28: 1} | {'0x0': 1} | no specific handler |
| 0x00731359 | NV0073_CTRL_CMD_DP_SET_AUDIO_MUTESTREAM | 1 / 1 | {12: 1} | {'0x0': 1} | no specific handler |
| 0x00731360 | NV0073_CTRL_CMD_DP_GET_LINK_CONFIG | 2 / 2 | {24: 2} | {'0x0': 2} | no specific handler |
| 0x00731362 | NV0073_CTRL_CMD_DP_CONFIG_STREAM | 1 / 1 | {84: 1} | {'0x0': 1} | no specific handler |
| 0x00731368 | unresolved in this SDK | 1 / 1 | {12: 1} | {'0x0': 1} | no specific handler |
| 0x00731378 | NV0073_CTRL_CMD_DP_SET_STEREO_MSA_PROPERTIES | 2 / 2 | {52: 2} | {'0x0': 2} | no specific handler |
| 0x00731381 | NV0073_CTRL_CMD_DP_SET_MSA_PROPERTIES_V2 | 1 / 1 | {80: 1} | {'0x0': 1} | no specific handler |
| 0x00800294 | NV0080_CTRL_CMD_GPU_GET_BRAND_CAPS | 1 / 1 | {4: 1} | {'0x0': 1} | no specific handler |
| 0x0080170e | NV0080_CTRL_CMD_FIFO_GET_LATENCY_BUFFER_SIZE | 12 / 12 | {12: 12} | {'0x0': 12} | no specific handler |
| 0x0080170f | NV0080_CTRL_CMD_FIFO_SET_CHANNEL_PROPERTIES | 2 / 2 | {16: 2} | {'0x0': 2} | no specific handler |
| 0x00801812 | unresolved in this SDK | 6 / 6 | {4: 6} | {'0x0': 6} | no specific handler |
| 0x20800102 | NV2080_CTRL_CMD_GPU_GET_INFO_V2 | 1 / 1 | {564: 1} | {'0x0': 1} | init-table handler |
| 0x2080012b | NV2080_CTRL_CMD_GPU_PROMOTE_CTX | 11 / 11 | {560: 11} | {'0x0': 11} | channel handler |
| 0x2080012f | NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS | 1 / 1 | {1464: 1} | {'0x56': 1} | no specific handler |
| 0x2080014b | NV2080_CTRL_CMD_GPU_GET_INFOROM_OBJECT_VERSION | 5 / 5 | {5: 5} | {'0x0': 2, '0x57': 3} | no specific handler |
| 0x20800156 | NV2080_CTRL_CMD_GPU_GET_INFOROM_IMAGE_VERSION | 1 / 1 | {16: 1} | {'0x0': 1} | no specific handler |
| 0x20800157 | NV2080_CTRL_CMD_GPU_QUERY_INFOROM_ECC_SUPPORT | 1 / 1 | {0: 1} | {'0x56': 1} | no specific handler |
| 0x208001a4 | NV2080_CTRL_CMD_GPU_GET_CHIP_DETAILS | 1 / 1 | {16: 1} | {'0x0': 1} | no specific handler |
| 0x20800a38 | NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE | 24 / 24 | {24: 24} | {'0x0': 24} | no specific handler |
| 0x20800a9a | NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_SET_2X | 4 / 4 | {8: 4} | {'0x0': 4} | no specific handler |
| 0x20800af0 | NV2080_CTRL_CMD_INTERNAL_DISPLAY_ACPI_SUBSYSTEM_ACTIVATED | 1 / 1 | {0: 1} | {'0x0': 1} | no specific handler |
| 0x20800af1 | NV2080_CTRL_CMD_INTERNAL_DISPLAY_PRE_MODESET | 1 / 1 | {0: 1} | {'0x0': 1} | no specific handler |
| 0x20800af2 | NV2080_CTRL_CMD_INTERNAL_DISPLAY_POST_MODESET | 1 / 1 | {0: 1} | {'0x0': 1} | no specific handler |
| 0x20800aff | NV2080_CTRL_CMD_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL | 2 / 2 | {16: 2} | {'0x0': 2} | no specific handler |
| 0x2080110b | NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS | 4 / 4 | {536: 4} | {'0x0': 4} | channel handler |
| 0x20801111 | unresolved in this SDK | 12 / 12 | {40: 12} | {'0x0': 12} | no specific handler |
| 0x20801208 | NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND | 1 / 1 | {24: 1} | {'0x0': 1} | channel handler |
| 0x20801211 | NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND | 8 / 8 | {112: 8} | {'0x0': 8} | no specific handler |
| 0x20801303 | NV2080_CTRL_CMD_FB_GET_INFO_V2 | 1 / 1 | {1028: 1} | {'0x0': 1} | init-table handler |
| 0x20801322 | NV2080_CTRL_CMD_FB_GET_OFFLINED_PAGES | 1 / 1 | {2056: 1} | {'0x56': 1} | no specific handler |
| 0x20801344 | NV2080_CTRL_CMD_FB_GET_REMAPPED_ROWS | 1 / 1 | {6152: 1} | {'0x0': 1} | no specific handler |
| 0x20801347 | NV2080_CTRL_CMD_FB_GET_ROW_REMAPPER_HISTOGRAM | 1 / 1 | {20: 1} | {'0x0': 1} | no specific handler |
| 0x20801357 | NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT | 1 / 1 | {4: 1} | {'0x56': 1} | no specific handler |
| 0x20801813 | NV2080_CTRL_CMD_BUS_GET_PEX_COUNTERS | 2 / 2 | {76: 2} | {'0x0': 2} | no specific handler |
| 0x20801819 | NV2080_CTRL_CMD_BUS_GET_PEX_UTIL_COUNTERS | 4 / 4 | {32: 4} | {'0x0': 4} | no specific handler |
| 0x20801823 | NV2080_CTRL_CMD_BUS_GET_INFO_V2 | 8 / 8 | {420: 8} | {'0x0': 8} | init-table handler |
| 0x20801829 | NV2080_CTRL_CMD_BUS_GET_PCIE_REQ_ATOMICS_CAPS | 1 / 1 | {12: 1} | {'0x0': 1} | no specific handler |
| 0x20801830 | NV2080_CTRL_CMD_BUS_GET_PCIE_CPL_ATOMICS_CAPS | 1 / 1 | {4: 1} | {'0x0': 1} | no specific handler |
| 0x20802a08 | NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE | 12 / 12 | {4: 12} | {'0x0': 12} | init-table handler |
| 0x20803083 | NV2080_CTRL_CMD_NVLINK_GET_PLATFORM_INFO | 1 / 1 | {39: 1} | {'0x0': 1} | no specific handler |
| 0x20803400 | NV2080_CTRL_CMD_ECC_GET_CLIENT_EXPOSED_COUNTERS | 1 / 1 | {96: 1} | {'0x0': 1} | no specific handler |
| 0x20803401 | NV2080_CTRL_CMD_ECC_GET_VOLATILE_COUNTS | 1 / 1 | {40: 1} | {'0x0': 1} | no specific handler |
| 0x20803404 | NV2080_CTRL_CMD_ECC_GET_REPAIR_STATUS | 1 / 1 | {2: 1} | {'0x0': 1} | no specific handler |
| 0x20808159 | unresolved in this SDK | 10 / 10 | {332: 10} | {'0x0': 10} | empirical identity reply |
| 0x2080852a | unresolved in this SDK | 5 / 5 | {1784: 5} | {'0x0': 5} | no specific handler |
| 0x2080852b | unresolved in this SDK | 2 / 2 | {2836: 2} | {'0x0': 2} | no specific handler |
| 0x2080852c | unresolved in this SDK | 2 / 2 | {1440: 2} | {'0x0': 2} | no specific handler |
| 0x2080852e | unresolved in this SDK | 4 / 4 | {528: 4} | {'0x0': 4} | no specific handler |
| 0x2080852f | unresolved in this SDK | 1 / 1 | {776: 1} | {'0x0': 1} | no specific handler |
| 0x20808530 | unresolved in this SDK | 1 / 1 | {912: 1} | {'0x0': 1} | no specific handler |
| 0x20808536 | unresolved in this SDK | 1 / 1 | {1812: 1} | {'0x0': 1} | no specific handler |
| 0x20808537 | unresolved in this SDK | 1 / 1 | {42424: 1} | {'0x0': 1} | no specific handler |
| 0x20808539 | unresolved in this SDK | 332 / 332 | {6824: 332} | {'0x0': 332} | no specific handler |
| 0x2080853a | unresolved in this SDK | 1 / 1 | {1456: 1} | {'0x0': 1} | no specific handler |
| 0x20808542 | unresolved in this SDK | 1 / 1 | {504: 1} | {'0x0': 1} | no specific handler |
| 0x20808546 | unresolved in this SDK | 35 / 35 | {24: 35} | {'0x0': 11, '0x56': 24} | no specific handler |
| 0x2080880f | unresolved in this SDK | 1 / 1 | {49: 1} | {'0x56': 1} | no specific handler |
| 0x20809001 | unresolved in this SDK | 2 / 2 | {8: 2} | {'0x0': 2} | capture-derived constant reply |
| 0x20809004 | unresolved in this SDK | 24 / 24 | {1544: 24} | {'0x0': 24} | no specific handler |
| 0x20809019 | unresolved in this SDK | 3 / 3 | {10800: 3} | {'0x0': 3} | no specific handler |
| 0x2080901b | unresolved in this SDK | 1 / 1 | {1980: 1} | {'0x0': 1} | no specific handler |
| 0x20809029 | unresolved in this SDK | 17 / 17 | {904: 17} | {'0x0': 17} | no specific handler |
| 0x2080902a | unresolved in this SDK | 12 / 12 | {804: 12} | {'0x0': 12} | no specific handler |
| 0x2080902b | unresolved in this SDK | 6 / 6 | {6160: 6} | {'0x0': 6} | no specific handler |
| 0x2080902c | unresolved in this SDK | 5 / 5 | {4116: 5} | {'0x0': 5} | no specific handler |
| 0x20809037 | unresolved in this SDK | 332 / 332 | {6024: 332} | {'0x0': 332} | no specific handler |
| 0x20809038 | unresolved in this SDK | 1 / 1 | {4: 1} | {'0x56': 1} | no specific handler |
| 0x20809063 | unresolved in this SDK | 1 / 1 | {520: 1} | {'0x0': 1} | no specific handler |
| 0x2080a026 | unresolved in this SDK | 1 / 1 | {532: 1} | {'0x0': 1} | conditional authored host-fact query |
| 0x2080a028 | unresolved in this SDK | 10 / 10 | {2192: 10} | {'0x0': 10} | conditional authored host-fact query |
| 0x2080a079 | unresolved in this SDK | 1 / 1 | {83972: 1} | {'0x0': 1} | no specific handler |
| 0x2080a080 | unresolved in this SDK | 4 / 4 | {52: 4} | {'0x0': 4} | no specific handler |
| 0x2080a081 | unresolved in this SDK | 1 / 1 | {1872: 1} | {'0x0': 1} | no specific handler |
| 0x2080a084 | unresolved in this SDK | 1 / 1 | {4: 1} | {'0x0': 1} | conditional authored host-fact query |
| 0x2080a088 | unresolved in this SDK | 2 / 2 | {12: 2} | {'0x0': 2} | no specific handler |
| 0x2080a095 | unresolved in this SDK | 1 / 1 | {94024: 1} | {'0x0': 1} | no specific handler |
| 0x2080a0a4 | unresolved in this SDK | 1 / 1 | {67396: 1} | {'0x0': 1} | no specific handler |
| 0x2080a0a7 | unresolved in this SDK | 4 / 4 | {14988: 4} | {'0x0': 4} | no specific handler |
| 0x2080a0a8 | unresolved in this SDK | 331 / 331 | {19724: 331} | {'0x0': 331} | no specific handler |
| 0x2080a0c4 | unresolved in this SDK | 3 / 3 | {332: 3} | {'0x0': 3} | no specific handler |
| 0x2080a0c5 | unresolved in this SDK | 1 / 1 | {1296: 1} | {'0x0': 1} | no specific handler |
| 0x2080a0c8 | unresolved in this SDK | 2 / 2 | {22924: 2} | {'0x0': 2} | no specific handler |
| 0x2080a0cc | unresolved in this SDK | 1 / 1 | {548: 1} | {'0x0': 1} | no specific handler |
| 0x2080a0d1 | unresolved in this SDK | 331 / 331 | {2024: 331} | {'0x0': 331} | no specific handler |
| 0x2080a0f2 | unresolved in this SDK | 1 / 1 | {96: 1} | {'0x56': 1} | no specific handler |
| 0x2080a612 | unresolved in this SDK | 4 / 4 | {11680: 4} | {'0x0': 4} | no specific handler |
| 0x2080a618 | unresolved in this SDK | 5 / 5 | {8620: 5} | {'0x0': 5} | no specific handler |
| 0x2080a630 | unresolved in this SDK | 1 / 1 | {1160: 1} | {'0x0': 1} | no specific handler |
| 0x2080a637 | unresolved in this SDK | 3 / 2 | {96024: 3} | {'0x0': 2} | no specific handler |
| 0x2080a63c | unresolved in this SDK | 1 / 1 | {16: 1} | {'0x56': 1} | no specific handler |
| 0x2080a801 | unresolved in this SDK | 1 / 1 | {1028: 1} | {'0x0': 1} | no specific handler |
| 0x2080b201 | unresolved in this SDK | 2 / 2 | {2188: 2} | {'0x0': 2} | no specific handler |
| 0x2080b202 | unresolved in this SDK | 11 / 11 | {3232: 11} | {'0x0': 11} | no specific handler |
| 0x2080b209 | unresolved in this SDK | 1 / 1 | {524: 1} | {'0x0': 1} | no specific handler |
| 0x2080b210 | unresolved in this SDK | 1 / 1 | {2568: 1} | {'0x0': 1} | no specific handler |
| 0x2080b216 | unresolved in this SDK | 1 / 1 | {824: 1} | {'0x0': 1} | no specific handler |
| 0x2080d02d | unresolved in this SDK | 2 / 2 | {4116: 2} | {'0x0': 2} | no specific handler |
| 0x2080e0af | unresolved in this SDK | 1 / 1 | {80904: 1} | {'0x0': 1} | no specific handler |
| 0x20810108 | unresolved in this SDK | 3 / 3 | {992: 3} | {'0x0': 3} | no specific handler |
| 0x2081010d | unresolved in this SDK | 1 / 1 | {0: 1} | {'0x0': 1} | no specific handler |
| 0x50800101 | NV5080_CTRL_CMD_DEFERRED_API | 8 / 8 | {584: 8} | {'0x0': 8} | no specific handler |
| 0x90e70113 | NV90E7_CTRL_CMD_BBX_GET_LAST_FLUSH_TIME | 1 / 1 | {16: 1} | {'0x56': 1} | no specific handler |
| 0x90f10106 | NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES | 10 / 10 | {184: 10} | {'0x0': 10} | init-table handler |
| 0xa06c0101 | NVA06C_CTRL_CMD_GPFIFO_SCHEDULE | 10 / 10 | {3: 10} | {'0x0': 10} | channel handler |
| 0xa06c0103 | NVA06C_CTRL_CMD_SET_TIMESLICE | 12 / 12 | {8: 12} | {'0x0': 12} | channel handler |
| 0xa06c010a | NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS | 11 / 11 | {88: 11} | {'0x0': 11} | init-table handler |
| 0xa06f0103 | NVA06F_CTRL_CMD_GPFIFO_SCHEDULE | 1 / 1 | {3: 1} | {'0x0': 1} | channel handler |
| 0xc3700104 | NVC370_CTRL_CMD_GET_CHANNEL_INFO | 1 / 1 | {20: 1} | {'0x0': 1} | display handler |
| 0xc3720101 | NVC372_CTRL_CMD_IS_MODE_POSSIBLE | 115 / 115 | {2048: 115} | {'0x0': 115} | display handler |

## Conditional host-fact query input gates

These results check size and every authored input word, plus serialization flags. A match still needs an available host answer.

| ID | Observed request gate results |
| --- | --- |
| 0x2080a026 | {'does_not_match': 1} |
| 0x2080a028 | {'does_not_match': 10} |
| 0x2080a084 | {'matches': 1} |

## Allocation classes

A decoder is not proof of working class semantics. Native allocation status is the inner RM_ALLOC status.

| Class | Names | Requests / replies | Native statuses | Kayfabe |
| --- | --- | --- | --- | --- |
| 0x00000000 | NV01_NULL_OBJECT, NV01_ROOT, NV1_NULL_OBJECT, NV1_ROOT | 30 / 30 | {'0x0': 30} | allocation decoder exists |
| 0x00000070 | NV01_MEMORY_SYSTEM_DYNAMIC, NV01_MEMORY_VIRTUAL, NV1_MEMORY_SYSTEM_DYNAMIC | 36 / 36 | {'0x0': 36} | allocation decoder exists |
| 0x00000073 | NV04_DISPLAY_COMMON | 11 / 11 | {'0x0': 11} | allocation decoder exists |
| 0x0000007e | NV01_EVENT_KERNEL_CALLBACK_EX, NV1_EVENT_KERNEL_CALLBACK_EX | 2 / 2 | {'0x0': 2} | allocation decoder exists |
| 0x00000080 | NV01_DEVICE_0 | 30 / 30 | {'0x0': 30} | allocation decoder exists |
| 0x00002080 | NV20_SUBDEVICE_0 | 30 / 30 | {'0x0': 30} | allocation decoder exists |
| 0x00002081 | NV2081_BINAPI | 5 / 5 | {'0x0': 5} | allocation decoder exists |
| 0x0000402c | NV40_I2C | 3 / 3 | {'0x0': 3} | denied by capability policy |
| 0x00005080 | NV50_DEFERRED_API_CLASS | 12 / 12 | {'0x0': 12} | denied by capability policy |
| 0x0000902d | FERMI_TWOD_A | 6 / 6 | {'0x0': 6} | allocation decoder exists |
| 0x00009067 | FERMI_CONTEXT_SHARE_A | 5 / 5 | {'0x0': 5} | allocation decoder exists |
| 0x00009096 | GF100_ZBC_CLEAR | 6 / 6 | {'0x0': 6} | allocation decoder exists |
| 0x000090e7 | GF100_SUBDEVICE_INFOROM | 1 / 1 | {'0x0': 1} | no allocation decoder |
| 0x000090f1 | FERMI_VASPACE_A | 6 / 6 | {'0x0': 6} | allocation decoder exists |
| 0x0000a06c | KEPLER_CHANNEL_GROUP_A | 11 / 11 | {'0x0': 11} | allocation decoder exists |
| 0x0000a140 | KEPLER_INLINE_TO_MEMORY_B | 6 / 6 | {'0x0': 6} | allocation decoder exists |
| 0x0000c56f | AMPERE_CHANNEL_GPFIFO_A | 12 / 12 | {'0x0': 12} | allocation decoder exists |
| 0x0000c7b5 | AMPERE_DMA_COPY_B | 12 / 12 | {'0x0': 12} | allocation decoder exists |
| 0x0000c997 | ADA_A | 6 / 6 | {'0x0': 6} | allocation decoder exists |
| 0x0000c9c0 | ADA_COMPUTE_A | 6 / 6 | {'0x0': 6} | allocation decoder exists |

## Deferred wrapper contents

NV5080_CTRL_CMD_DEFERRED_API contains another control ID. The wrapper and allocation class are unsupported; having a direct handler does not implement deferred execution.

| Nested ID | Name | Directions | Direct handler only |
| --- | --- | --- | --- |
| 0x2080012b | NV2080_CTRL_CMD_GPU_PROMOTE_CTX | {'reply': 4, 'request': 4} | channel handler |
| 0x2080012d | NV2080_CTRL_CMD_GPU_INITIALIZE_CTX | {'reply': 4, 'request': 4} | no specific handler |

## Limits

- This is one RTX 4070 / Windows 580.88 reference capture. Hardware VFIO success is not Kayfabe success.
- Unknown prefixes and 1 later observed missing record remain. No GR_GFX_POOL_QUERY_SIZE was retained; its output values are still unknown.
- All captured RPC sequence fields are zero; independent request/reply totals do not establish individual pairings.
- Continuation records are validated but not reassembled here. Fragmented command IDs and declared sizes remain identifiable; complete body semantics do not.
- Names absent from this SDK may exist in other sources. Nested commands in opaque protocols are not inferred.
- Display/channel/object links are assumed seated. Handler inventories do not prove Windows submission, paging, interrupts or TDR behavior.
- The empirical identity and capture-derived constant replies are counted separately: neither establishes full semantics or cross-GPU correctness.
- Existing FB_GET_INFO_V2 and BUS_GET_INFO_V2 handlers already rejected Windows info indices 1 and 24 in the original Kayfabe run. Matching ID/size is insufficient.

See `audit.json` for every gate, header source location, provenance hash, and fragmented-head count.
