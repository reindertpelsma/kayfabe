# V3 interface coverage meter

**STATUS: LIVE, 2026-11-18.**

Run from the repository root with Python 3 and Git. The meter reads committed evidence and the two pinned open GPU kernel module source checkouts. It requires no GPU or NVIDIA driver binaries.

```sh
python3 -m scripts.ci.coverage_meter --revision 55194e22e \
  --oracles third_party \
  --extra-evidence b77819baff81389d887b1c43c5606d7bd9878355:traces/open_rows_guest_20261117 \
  > coverage.json
python3 scripts/ci/test_coverage_meter.py
python3 scripts/ci/coverage_meter_mutants.py
```

The pin and URL definitions are in `scripts/ci/fetch_oracles.sh`. For read-only analysis, shallow-fetch the exact commits into a separate source directory and supply that directory via `--oracles`. The initial lane checkout lacked both oracle trees. The first run used separate exact-pin source checkouts.

## What the meter counts

The JSON rows join by interface namespace and public numeric ID. Methods additionally require the owning class. The control owner is the high word of the control ID. Aliases and the oracle versions that define them are retained. The inventory takes the union of both oracle versions. It reads the SDK control and class headers, the generated class declarations, the RM escape numbers, and Linux UVM, DRM and modeset ioctl numbers. Method inventory comes from recognizable published scalar offsets and indexed method bases. The macro pass reports unresolved nonliteral candidates and omits non-command fields. The method inventory is approximate because headers also contain fields, constants and layouts outside these conventions.

A trace row includes issued count, outcome counts, the earliest and latest evidence directories by first-parent addition date, and matching policy modules when available. Handler candidates come from named controls in RM policy modules, the generated ABI matrix and the RM row-use table. A candidate can be a version- or profile-gated link, an explicit refusal, or a source reference. Generated family membership comes from the class table in `crates/kf-chip/src/classes.rs`.

- Request traces and legacy teardown rows establish serviced, refused or unserviced outcomes. A refusal heartbeat is a cumulative count of non-OK replies. The FSM also posts non-OK replies for unserviced requests, so the heartbeat cannot identify the policy outcome on its own.
- Within an artifact the meter uses per-status heartbeat high-water marks and combines overlapping status and request evidence by maximum. Fabrication markers can describe the same successful receipt as a raw RPC trace. Unknown host RM ioctls, Xids and unattributed methods stay in their own namespaces. A passthrough birth with matching channel facts in the same artifact is attributed to its class. Other births retain a route-only tally. An aggregate count for RPC function 76 establishes control traffic without identifying any command.
- The scan covers committed text logs, selected text excerpts and the whole-boot JSON census. Byte-identical artifacts in a trace directory count once. Different excerpts can overlap and runs can lack trace logging, hit sample caps or live in archives. Counts are evidence tallies, and a `never_issued` group means no matching ID was found in the scanned evidence. It does not establish that a guest driver has never issued a command.

The lead list sorts each control, class, method or guest ioctl candidate by the recorded issue count times an inferred feature weight. The issue count uses the larger of the non-OK tally and the combined explicit refusal and unserviced counts. The JSON also lists up to 40 plausible unseen features separately since their issue count is zero. The probe descriptions and feature weights are **INFERRED**. A high count can indicate a frequently queried optional feature. Compare a native run with a guest run and check the application result before calling it a bug.

## Feeding a new run

Commit the log or supported text excerpt beneath its trace directory, then run the meter against that revision. For the differential and open-guest runners, commit a versioned JSON file with disjoint deltas and pass it as `--windows traces/RUN/census.json`. In this mode the meter reads that artifact instead of raw logs so the same observations do not count twice. Case labels are ordered as supplied. Use public decimal interface IDs and a class for controls and class-qualified methods. Repeat an outcome row for additional independent events, or give it a count. Keep run-local object handles, addresses and payloads out of the window.

```json
{
  "schema": 1,
  "windows": [{
    "case": "device-discovery",
    "rows": [{"kind": "control", "class": 8320, "id": 545260289, "outcome": "refused", "count": 1}]
  }]
}
```

## First result

**MEASURED.** The scan covered 262 committed trace directories at `55194e22e` plus the open-guest directory at `b77819baf` (included in that total). It found supported observations in 87 directories. The control rows contain 162,276 issued observations across 241 IDs. Explicit evidence shows 10 refused control IDs and 136 unserviced control IDs. The firmware ledger recorded non-OK statuses for 143 control IDs. These sets overlap.

| Interface | Defined in pinned sources | IDs observed | Defined IDs unseen |
| --- | ---: | ---: | ---: |
| RM control | 1,647 | 241 | 1,458 |
| Class | 286 | 36 | 252 |
| Class-qualified method | 3,774 | 0 | 3,774 |
| RM escape | 41 | 0 | 41 |
| UVM ioctl | 180 | 0 | 180 |
| DRM ioctl | 24 | 0 | 24 |
| Modeset ioctl | 1 | 0 | 1 |

The decoded evidence also includes 238 host unknown-escape lines for `0xf0`, 2,568 passthrough channel births (268 matched to class `0xc56f` within the same artifact), and two explicit fabrication markers for `0x2080121f`. These are artifact tallies. The extractor reported 37 unresolved macro candidates in each oracle tree.

### Ranked leads

| Rank | Interface ID and name | Issues | Score | Suggested probe (INFERRED) |
| ---: | --- | ---: | ---: | --- |
| 1 | control `0x20802a12` `NV2080_CTRL_CMD_CE_IS_DECOMP_LCE_ENABLED` | 37,355 | 186,775 | Compare managed-memory migration, then repeat with memory sharing enabled. |
| 2 | control `0x20800a80` `NV2080_CTRL_CMD_INTERNAL_PERF_GPU_BOOST_SYNC_GET_INFO` | 6,652 | 26,608 | Compare read-only management queries at idle and under load. |
| 3 | control `0x20800afe` `NV2080_CTRL_CMD_INTERNAL_INIT_USER_SHARED_DATA` | 6,570 | 26,280 | Compare read-only management queries at idle and under load. |
| 4 | control `0x2080012f` `NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS` | 6,043 | 24,172 | Compare read-only management queries at idle and under load. |
| 5 | control `0x20800a87` `NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO` | 3,398 | 16,990 | Compare managed-memory migration, then repeat with memory sharing enabled. |
| 6 | control `0x2080017e` `NV2080_CTRL_CMD_GPU_GET_VMMU_SEGMENT_SIZE` | 3,397 | 16,985 | Compare managed-memory migration, then repeat with memory sharing enabled. |
| 7 | control `0x20800aff` `NV2080_CTRL_CMD_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL` | 3,918 | 15,672 | Compare read-only management queries at idle and under load. |
| 8 | control `0x20800a1e` `NV2080_CTRL_CMD_INTERNAL_UVM_UNREGISTER_ACCESS_CNTR_BUFFER` | 2,859 | 14,295 | Compare managed-memory migration, then repeat with memory sharing enabled. |
| 9 | control `0x20800a9c` `NV2080_CTRL_CMD_INTERNAL_GMMU_UNREGISTER_FAULT_BUFFER` | 2,859 | 14,295 | Compare managed-memory migration, then repeat with memory sharing enabled. |
| 10 | control `0x20800a9e` `NV2080_CTRL_CMD_INTERNAL_GMMU_UNREGISTER_CLIENT_SHADOW_FAULT_BUFFER` | 2,859 | 14,295 | Compare managed-memory migration, then repeat with memory sharing enabled. |
| 11 | control `0x20800a2e` `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_ROP_INFO` | 3,397 | 13,588 | Compare Vulkan and CUDA device capabilities, then start a minimal workload. |
| 12 | control `0x20800a30` `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PPC_MASKS` | 3,397 | 13,588 | Compare Vulkan and CUDA device capabilities, then start a minimal workload. |
| 13 | control `0x20800a3f` `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_TRACE_DEFINES` | 3,397 | 13,588 | Compare Vulkan and CUDA device capabilities, then start a minimal workload. |
| 14 | control `0x20800b03` `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER_V2` | 3,397 | 13,588 | Compare Vulkan and CUDA device capabilities, then start a minimal workload. |
| 15 | control `0x20800b05` `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_THROTTLE_CTRL` | 3,397 | 13,588 | Compare Vulkan and CUDA device capabilities, then start a minimal workload. |
| 16 | control `0x2080012b` `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` | 3,353 | 13,412 | Compare context creation and alternating compute and graphics workloads. |
| 17 | class `0xc36f` `VOLTA_CHANNEL_GPFIFO_A` | 3,346 | 13,384 | Compare context creation and alternating compute and graphics workloads. |
| 18 | control `0x20800a38` `NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE` | 3,259 | 13,036 | Compare Vulkan and CUDA device capabilities, then start a minimal workload. |
| 19 | control `0x20800a9a` `NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_SET_2X` | 3,231 | 12,924 | Compare read-only management queries at idle and under load. |
| 20 | control `0x2080013f` `NV2080_CTRL_CMD_GPU_GET_OEM_BOARD_INFO` | 3,423 | 10,269 | Compare device enumeration and capability queries before launching work. |
| 21 | control `0x20802a0f` `NV2080_CTRL_CMD_INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE` | 3,397 | 3,397 | Compare boot or application-stage census windows on native and guest runs. |
| 22 | class `0x402c` `NV40_I2C` | 3,377 | 3,377 | Compare boot or application-stage census windows on native and guest runs. |
| 23 | control `0x2080014b` `NV2080_CTRL_CMD_GPU_GET_INFOROM_OBJECT_VERSION` | 1,113 | 3,339 | Compare device enumeration and capability queries before launching work. |
| 24 | control `0x20800a4b` `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_IP_VERSION` | 3,304 | 3,304 | Compare boot or application-stage census windows on native and guest runs. |
| 25 | control `0x20810108` | 3,214 | 3,214 | Compare boot or application-stage census windows on native and guest runs. |
| 26 | control `0x20802068` `NV2080_CTRL_CMD_PERF_GET_CURRENT_PSTATE` | 284 | 1,136 | Compare read-only management queries at idle and under load. |
| 27 | control `0x20808513` `NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2_PHYSICAL` | 281 | 1,124 | Compare read-only management queries at idle and under load. |
| 28 | control `0x20800157` `NV2080_CTRL_CMD_GPU_QUERY_INFOROM_ECC_SUPPORT` | 280 | 1,120 | Compare read-only management queries at idle and under load. |
| 29 | control `0x20800ab8` `NV2080_CTRL_CMD_INTERNAL_GET_PCIE_P2P_CAPS` | 219 | 1,095 | Compare managed-memory migration, then repeat with memory sharing enabled. |
| 30 | control `0x2080a0d1` | 1,073 | 1,073 | Compare boot or application-stage census windows on native and guest runs. |
| 31 | control `0x730291` `NV0073_CTRL_CMD_SPECIFIC_GET_BACKLIGHT_BRIGHTNESS` | 176 | 880 | Compare monitor enumeration and mode changes with a native run. |
| 32 | control `0x800294` `NV0080_CTRL_CMD_GPU_GET_BRAND_CAPS` | 281 | 843 | Compare device enumeration and capability queries before launching work. |
| 33 | control `0x73012c` `NV0073_CTRL_CMD_SYSTEM_VRR_DISPLAY_INFO` | 166 | 830 | Compare monitor enumeration and mode changes with a native run. |
| 34 | control `0x20800a2c` `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_ZCULL_INFO` | 173 | 692 | Compare context creation and alternating compute and graphics workloads. |
| 35 | control `0x20800a34` `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER` | 173 | 692 | Compare Vulkan and CUDA device capabilities, then start a minimal workload. |
| 36 | control `0x731144` `NV0073_CTRL_CMD_DFP_SET_ELD_AUDIO_CAPS` | 124 | 620 | Compare monitor enumeration and mode changes with a native run. |
| 37 | control `0x20800a3a` `NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_WR_OFFSET` | 116 | 464 | Compare Vulkan and CUDA device capabilities, then start a minimal workload. |
| 38 | control `0x20810110` | 387 | 387 | Compare boot or application-stage census windows on native and guest runs. |
| 39 | control `0xa06f0112` `NVA06F_CTRL_CMD_STOP_CHANNEL` | 91 | 364 | Compare context creation and alternating compute and graphics workloads. |
| 40 | control `0x20808546` | 345 | 345 | Compare boot or application-stage census windows on native and guest runs. |
