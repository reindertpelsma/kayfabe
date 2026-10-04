## Identity at the handshake

- Windows fn 72: `bGspNocatEnabled=1`
- Windows fn 1: `the guest says guestDriverVersion="580.88" guestVersion="r580_78-7" guestTitle="DVSReal r580_78 580.88 DVS-Applications" guestClNum=0 vgx=0x2b.0x13; this device answers as driver 580.65.06`
- Linux fn 72: `bGspNocatEnabled=0`
- Linux fn 1: `the guest says guestDriverVersion="580.65.06" guestVersion="rel/gpu_drv/r580/r580_78-179" guestTitle="Private r580_78 rel/gpu_drv/r580/r580_78-179 unknown" guestClNum=0 vgx=0x2b.0x13; this device answers as driver 580.65.06`

## RPC functions (count Windows / Linux)

| function | Windows | Linux | note |
|---|---|---|---|
| RmControl | 90 | 395 |  |
| UNSERVICED | 31 | 127 |  |
| Free | 23 | 67 |  |
| RmAlloc | 23 | 138 |  |
| UpdateBarPde | 2 | 3 |  |
| GetGspStaticInfo | 1 | 2 |  |
| GspSetSystemInfo | 1 | 2 |  |
| SetGuestSystemInfo | 1 | 2 |  |
| SetRegistry | 1 | 2 |  |
| UnloadingGuestDriver | 1 | 1 |  |

Windows first-seen order: GspSetSystemInfo → SetRegistry → SetGuestSystemInfo → GetGspStaticInfo → RmAlloc → RmControl → UNSERVICED → UpdateBarPde → Free → UnloadingGuestDriver

Linux first-seen order: GspSetSystemInfo → SetRegistry → SetGuestSystemInfo → GetGspStaticInfo → RmControl → UNSERVICED → RmAlloc → UpdateBarPde → Free → UnloadingGuestDriver

## UNSERVICED codes

| code | Windows | Linux |
|---|---|---|
| 76 | 31 | 127 |

## GSP_RM_CONTROL ids: Windows only (14), shared (58), Linux only (66)

| id | Windows count | kayfabe answered (Windows) |
|---|---|---|
| 0x00800292 `NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2` | 1 | unserved→0x56 x1 |
| 0x00801306 `NV0080_CTRL_CMD_FB_GET_COMPBIT_STORE_INFO` | 1 | unserved→0x56 x1 |
| 0x00801707 `NV0080_CTRL_CMD_FIFO_GET_ENGINE_CONTEXT_PROPERTIES` | 1 | 0x0x1 |
| 0x00801b02 `NV0080_CTRL_CMD_MSENC_GET_CAPS_V2` | 1 | 0x0x1 |
| 0x00801c02 `NV0080_CTRL_CMD_BSP_GET_CAPS_V2` | 1 | 0x0x1 |
| 0x20800160 `NV2080_CTRL_CMD_GPU_GET_VPR_CAPS` | 1 | unserved→0x56 x1 |
| 0x20800173 `NV2080_CTRL_CMD_GPU_QUERY_FUNCTION_STATUS` | 1 | unserved→0x56 x1 |
| 0x20800aaf `NV2080_CTRL_CMD_INTERNAL_GET_ENABLED_SEC2_CLASSES` | 1 | unserved→0x56 x1 |
| 0x20800ab8 `NV2080_CTRL_CMD_INTERNAL_GET_PCIE_P2P_CAPS` | 1 | unserved→0x56 x1 |
| 0x2080121f `NV2080_CTRL_CMD_GR_GFX_POOL_QUERY_SIZE` | 1 | unserved→0x56 x1 |
| 0x20801303 `NV2080_CTRL_CMD_FB_GET_INFO_V2` | 1 | 0x56x1 |
| 0x20801315 `NV2080_CTRL_CMD_FB_GET_GPU_CACHE_INFO` | 1 | 0x0x1 |
| 0x20808159 | 1 | 0x0x1 |
| 0x20809001 | 1 | 0x0x1 |

Shared ids answered differently (3):

| id | Windows | Linux |
|---|---|---|
| 0x20800102 `NV2080_CTRL_CMD_GPU_GET_INFO_V2` | 0x0x1 | 0x0x2, 0x56x1 |
| 0x20800301 `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` | 0x0x1 | 0x0x7, 0x56x2 |
| 0x20801823 `NV2080_CTRL_CMD_BUS_GET_INFO_V2` | 0x0x1, 0x56x1 | 0x0x3 |

## GSP_RM_ALLOC classes: Windows only (0), shared (8), Linux only (12)

| id | Windows count | kayfabe answered (Windows) |
|---|---|---|

## Refusal ledger (last heartbeat)

- Windows: total=33 distinct=28
- Linux: total=140 distinct=60
- refused for Windows and never for Linux (9): `fn76/0x20800aaf=0x56x1` `fn76/0x00800292=0x56x1` `fn76/0x20801303=0x56x1` `fn76/0x00801306=0x56x1` `fn76/0x20801823=0x56x1` `fn76/0x20800ab8=0x56x1` `fn76/0x20800173=0x56x1` `fn76/0x20800160=0x56x1` `fn76/0x2080121f=0x56x1`
- refused for Linux and never for Windows: 41

## Named refusals (RPC-REFUSED), Windows only

