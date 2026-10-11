# V3 constant inventory: guest-visible values answered from something other than ogkm, kayfabe state or a host query

**STATUS: LIVE, 2026-10-11.** Read-only inventory; no crate was edited and no answer is proposed. Tree: `integration/windows-20261010` at `c1aa06b0` (worktree branch `claude/constant-inventory-20261011`). Machine-readable twin: `docs/design/v3_constant_inventory.tsv` (columns: id, name, kind, axes, kayfabe_source, evidence, monitor_attached, notes; same ids as this file).

Convention: **M:** = measured (a log line, a capture, a source line read); **I:** = inferred; **unknown** = no evidence. The two are kept apart in every cell. "ogkm" = ogkm 595.84 headers on the controller (`/workspace/ogkm-tars/open-gpu-kernel-modules-595.84`), used for control names; "undefined" = no header in 595.84 defines the id; **other ogkm versions (older tags, newer releases) are not yet checked**, so an "undefined" id may still be defined elsewhere (owner decision: derivations may use any ogkm version).

## 1. Summary

| section | rows | what it holds |
|---|---:|---|
| 1a Controls the Windows guest sent in run 641 that kayfabe refused (result=none) or could not ask the host for | 68 | 61 controls with `result=none` (62 distinct `GSP REFUSED fn76` rows in the log = those 61 plus `0x20801823`, which was answered `0x56` once), 1 refused class alloc (`0x402c`), 5 refused host-side asks (`HOST-ABI REFUSED`) |
| 1b Controls kayfabe answered NV_OK from a hand-typed, echoed, captured or invented source (or whose reply differs from the real GPU's for a hand-chosen model constant) | 26 | echo-probe answers, stubs, captured GA106 rows, authored display-model constants |
| 2 Hand-typed / observed / captured constants and probe tables in `kf-*` (not already a control row) | 35 | display register experiments, the captured 99-word caps page, Advertised values, BAR0 advertisements, host-ABI and GSS row sets |
| 3 Family rows (ALLOWED hand-maintained; do not re-list as open) | 11 | per-family dispatch, per-family allowlists |
| total | 140 | |

### 1.0 Shape of the answer (coordinator addition, 2026-10-11)

SHAPE describes the reply the guest needs for the control or value, judged from ogkm 595.84 and the captures; for a refused control it is the shape of the answer kayfabe does NOT give. (a) status-only, no meaningful body; (b) scalar or bitset; (c) layout defined by ogkm (595.84 unless the row says otherwise), only VALUES open; (d) layout NOT in ogkm 595.84 (other versions not yet checked): the struct or its field meanings come from captured replies or a host probe, and `shape_basis` says how they were decided; (e) opaque block of known size that kayfabe does not interpret (replayed or zero-filled, or no layout decoded). `n/a` = family rows. STATE_DEPENDENT: yes/no/unknown plus the field; for refused controls it is only what the real captures show (a payload that varies between calls is `unknown`, a single payload is `no` for the requests seen). FIELDS_SET: does kayfabe set individual fields (yes) or replay/echo bytes or set nothing (no).

| shape | rows |
|---|---:|
| a | 4 |
| b | 14 |
| c | 72 |
| d | 11 |
| e | 28 |
| n/a | 11 |

### 1.1 What this inventory rests on, and its limits (read before using a row)

- **Run 641** (`/var/lib/kf-windows-20261005/boundary-kayfabe-641/`, host only): kf3 `85f2217d`, host driver 595.91.07, Windows 11 guest (driver 580.88, branch key 580.65.06), `KF3_RPC_TRACE=1`, 4 min 12 s (marker.txt), 1944 `RmControl` lines. Its profile (command.json `flags`, M): `WIN_USER_CHANNELS_PASSTHROUGH, WIN_TWIN_DEFAPI_OBJECT, TIMER_MAP, KERNEL_GR_CE, KERNEL_GR_WORK, TRANSLATED_CE_RELAY, DEFERRED_API, PREEMPT_BIND_PROBE, DISPLAY_IMP_ENABLE, DISPLAY_CTRL_PROBE, ZCULL_BIND_PROBE, SW_RUNLIST_HOST_OWNED, ASYNC_PREEMPT, GFX_POOL_PROBE, SW_RUNLIST_PROBE, MEMORY_LIST_PROBE, DISPLAY_SDR_COLOR, DISPLAY_CAPS_PROBE, DISPLAY_LUT_MIRROR, DISPLAY_ILUT_OFFSET_256`. Off in it: `DISPLAY_HDMI_PROBE, DISPLAY_XBAR_PROBE, DISPLAY_PRIVATE_PROBE, DISPLAY_HDCP_STATE, DISPLAY_HOTPLUG_EDID_SEEN, DISPLAY_LOADV, DISPLAY_BLANK_STATE, DISPLAY_ARMED_DEFAULTS, DISPLAY_CORE_AT_VBLANK, GSS_NATIVE, DISPLAY_PRIVATE_PROBE`.
- **Revision skew (M):** the trace is from `85f2217d`; the code read here is `c1aa06b0`, 11 commits later. Display commits `a590f10a` and `b88f2f60` changed the `0x730101` caps table and the SOR flags after the run: the run served `capsTbl = 81 2f` (equal to the real GPU), HEAD derives `00 0c`. Rows say which revision a statement is about. Controls outside the display model were not changed by those commits (`git diff --stat 85f2217d..c1aa06b0 -- crates` touches only `kf-disp`).
- **Real-GPU answers (M):** five observer captures of the SAME Windows 580.88 guest on the SAME RTX 4070 (AD104), through vfio-pci with the patched observer (64 MiB source cap; `capture_complete` is `false` in at least `vfio-dvi-20261008/boot1`, so counts are lower bounds): `vfio-dvi-20261008/boot1..3` (DVI reference) and `vfio-gl-20261011/boot1..2` (OpenGL reference; `summary.md` calls the monitor a Philips 243V5 on HDMI-A-1). Both sessions read the same EDID (M: the `0x730245` reply is identical in all five captures). **So the die, the display IP version, the guest driver branch and the monitor are each sampled at exactly ONE point. No row can state an axis variation as measured; every `axes` cell says so.**
- **Joins done for this file (M, my scripts on the host, read-only):** `result=none` set of run 641 vs the 221 distinct `RmControl` ids in the five captures (status histogram, reply params size, number of distinct reply payloads per session); for the `0x0073xxxx` controls and `0x20801315`/`0x2080012f`, the first 96 reply bytes in the run-641 trace vs every real reply (`=` equal to some real reply, `!` equal to none). Real reply bytes are **not reproduced** here: `vfio-gl-20261011/summary.md` marks the raw captures HOST ONLY.
- **Not in the trace (M):** the params size the guest sent to kayfabe, and the bytes of kayfabe's reply for any control outside the `0x0073xxxx` set plus `0x20801315` / `0x2080012f`. Where a row compares replies it says so; otherwise it compares status and reply size only.
- **Where kayfabe's refusal comes from (M/I):** `result=none` (M) means no chain link produced a reply; `kf_gsp::GspFsm::answer` then posts `NV_ERR_NOT_SUPPORTED` with an empty body (`crates/kf-gsp/src/boot.rs:2207-2276`; M: the log shows 62 `GSP REFUSED ... =0x56` rows, one per distinct id). *Why* a given id has no link is inferred from code in the rows that cite a file; the others say "not claimed".

### 1.2 Flags that enable a hand-typed table (item 2 of the brief)

| flag | read at | what it enables | in run 641 | rows |
|---|---|---|---|---|
| `KF3_DISPLAY_CAPS_PROBE` | kf-qemu/src/display.rs:151 | replaces the authored display caps page with the RTX 4070's 99 captured non-zero words; only for (class 0xC773, base 0x640000), refuses elsewhere | ON | const-display-caps-page-probe |
| `KF3_GFX_POOL_PROBE` | kf-rm/src/lib.rs:583 | answers GR_GFX_POOL_QUERY_SIZE from invented sizes (4096 B/slot, <= 4096 slots) | ON | ctl-2080121f, const-gfxpool-sizes, ctl-20801220, ctl-20801222 |
| `KF3_DISPLAY_CTRL_PROBE` | kf-rm/src/lib.rs:533 | NV_OK + request echoed for 6 NV0073 controls (73013d, 73014b, 73012c, 7302a3, 730109, 731369) | ON | ctl-0073013d, ctl-0073014b, ctl-0073012c, ctl-007302a3, ctl-00730109, ctl-00731369 |
| `KF3_DISPLAY_PRIVATE_PROBE` | kf-rm/src/lib.rs:542 | 0x730122 echo; 0x730128 reply {0,1,0xff,0,0} | off | const-echo-730122, const-reply-730128, ctl-00730122, ctl-00730128 |
| `KF3_DISPLAY_HDMI_PROBE` | kf-rm/src/lib.rs:554 | HDMI path rows: connector data, DFP info flags, HDMI GPU caps, SCDC status, ELD echoes | off | const-hdmi-path-probe, ctl-00730293, ctl-007302a2, ctl-007302a6 |
| `KF3_DISPLAY_XBAR_PROBE` | kf-rm/src/lib.rs:554 | capsTbl {0x81,0x2f} and DFP_ASSIGN_SOR for display 0x100<<i on SOR i | off | const-caps-v2-observed-rm-features, const-crossbar-assign-sor |
| `KF3_DISPLAY_HDCP_STATE` | kf-rm/src/display.rs:406 | GET_HDCP_STATE answered NV_OK, flags 0 | off | const-get-hdcp-state-730280, ctl-00730280 |
| `KF3_DISPLAY_IMP_ENABLE` | kf-rm/src/display.rs:397 | IMP_ENABLE GET answered TRUE | ON | ctl-50700118 |
| `KF3_DISPLAY_HOTPLUG_EDID_SEEN` | kf-rm/src/display.rs:248,414 | hotplugAfterEdidMask only until the EDID is read | off | ctl-0073010a |
| `KF3_DISPLAY_SDR_COLOR / _LUT_MIRROR / _ILUT_OFFSET_256` | kf-qemu/src/display.rs:121-122,1785 | authored SDR colour page; decoder acceptance of MIRROR; 256-byte ILUT offset unit | ON (all three) | const-sdr-page, const-lut-mirror-offset256 |
| `KF3_DISPLAY_LOADV / _BLANK_STATE / _ARMED_DEFAULTS / _HEAD_TIMING_EN_BASE / _CORE_AT_VBLANK` | kf-qemu/src/display.rs:59-139,1871; kf-disp/src/engine.rs:589 | register read-back values and latch timing observed on the real GPU | off | const-evt-stat-loadv, const-blank-state, const-armed-min-frame-idle, const-head-timing-en-base, const-core-latch-vblank |
| `KF3_PREEMPT_BIND_PROBE / KF3_ZCULL_BIND_PROBE / KF3_SW_RUNLIST_HOST_OWNED` | kf-rm/src/lib.rs:518,525,507 | NV_OK + echoed request, nothing bound or scheduled | ON (all three) | ctl-20801211, ctl-20801208, ctl-20801111 |
| `KF3_SW_RUNLIST_PROBE / KF3_MEMORY_LIST_PROBE` | kf-rm/src/lib.rs:487,570 | object admission rules | ON | const-sw-runlist-object, const-memory-list-probe |
| `KF3_GSS_NATIVE` | kf-qemu/src/device.rs:331 | forwards GSS-legacy (bit 15) subdevice controls to the host RM | off | all GSS-legacy rows in 1a |

Searched markers not found as served constants: `OBSERVED_RM_FEATURES` exists only on `origin/claude/gl-730101-20261011` (`dispcaps.rs:50`); commit `b88f2f60` removed it from the integration branch (see `const-caps-v2-observed-rm-features`). `hand-typed` appears in `kf-disp/src/model.rs:79` (GET_HDCP_STATE) and as "never hand-typed" in `layout.rs`/`class.rs` headers. `hand-kept` appears in `kf-abi/src/lib.rs:65` (capability allowlist) and the comment-audit docs. `[observed` appears in `model.rs:39` (0x73011a). `ORACLE`/`oracle` is a test-fixture convention (kf-abi/src/oracle.rs, tests/); no served value was found to rest on it except where a row says so (cudartinit rows).

## 2. Row tables

Columns here are abbreviated; every cell is expanded in the TSV. "real" = status; reply params size; calls over the five captures.

### 2.1 Section 1a: refused controls and asks (68)

| id | name | run 641 | real GPU | shape | state-dep. | fields set by kayfabe | why refused (code read) |
|---|---|---:|---|---|---|---|---|
| `ctl-0073011d` | 0x0073011d NV0073_CTRL_CMD_SYSTEM_GET_CONNECTOR_TABLE (ctrl0073/ctrl0... | 1 | 0x0 x5; 272 B; 5 calls | c | unknown | no | refused: display link does not claim it (kf-rm/src/display.rs:1302-1316 test); no echo probe row for it |
| `ctl-00730122` | 0x00730122 undefined in ogkm 595.84 (other versions not yet checked) | 1 | 0x0 x7; 8 B; 7 calls | d | yes | no | refused unless KF3_DISPLAY_PRIVATE_PROBE=1 (off in run 641): kf-rm/src/display_ctrl_probe.rs:126 private_answer / :166... |
| `ctl-00730128` | 0x00730128 undefined in ogkm 595.84 (other versions not yet checked) | 3 | 0x0 x17; 20 B; 17 calls | d | unknown | no | refused unless KF3_DISPLAY_PRIVATE_PROBE=1 (off in run 641): kf-rm/src/display_ctrl_probe.rs:116 PRIVATE_0128_REPLY = 5... |
| `ctl-00730280` | 0x00730280 NV0073_CTRL_CMD_SPECIFIC_GET_HDCP_STATE (ctrl0073/ctrl0073... | 8 | 0x0 x49; 12 B; 49 calls | c | yes | yes | refused unless KF3_DISPLAY_HDCP_STATE=1 (off in run 641): kf-disp/src/model.rs:79 (id) and :1106 (answer) |
| `ctl-00730282` | 0x00730282 NV0073_CTRL_CMD_SPECIFIC_HDCP_CTRL (ctrl0073/ctrl0073speci... | 1 | 0x56 x20; 2600 B; 20 calls | c | unknown | no | refused: not claimed |
| `ctl-00730285` | 0x00730285 NV0073_CTRL_CMD_SPECIFIC_GET_ACPI_DOD_DISPLAY_PORT_ATTACHM... | 5 | 0x1f x20 0x56 x35; 12 B; 55 calls | c | unknown | no | refused: not claimed (display.rs:1302-1316 test) |
| `ctl-00730288` | 0x00730288 NV0073_CTRL_CMD_SPECIFIC_SET_OD_PACKET (ctrl0073/ctrl0073s... | 8 | 0x1f x40; 60 B; 40 calls | c | unknown | no | refused: not claimed |
| `ctl-00730293` | 0x00730293 NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_SINK_CAPS (ctrl0073/ctrl... | 1 | 0x0 x5; 12 B; 5 calls | c | no | no | refused unless KF3_DISPLAY_HDMI_PROBE=1 (off in run 641): kf-rm/src/display_ctrl_probe.rs:276 topology_answer, echo of... |
| `ctl-007302a2` | 0x007302a2 NV0073_CTRL_CMD_SPECIFIC_GET_HDMI_GPU_CAPS (ctrl0073/ctrl0... | 1 | 0x0 x5; 8 B; 5 calls | c | no | no | refused unless KF3_DISPLAY_HDMI_PROBE=1 (off): display_ctrl_probe.rs:246,263 HDMI_GPU_CAPS = 6 hand-typed (reply word 1) |
| `ctl-007302a5` | 0x007302a5 undefined in ogkm 595.84 (other versions not yet checked) | 45 | 0x0 x157; 16 B; 157 calls | e | unknown | no | refused: not claimed (no id in ogkm 595.84; other versions not yet checked) |
| `ctl-007302a6` | 0x007302a6 NV0073_CTRL_CMD_SPECIFIC_GET_HDMI_SCDC_DATA (ctrl0073/ctrl... | 1 | 0x14 x5; 12 B; 5 calls | c | no | no | refused unless KF3_DISPLAY_HDMI_PROBE=1 (off): display_ctrl_probe.rs:265 SCDC_STATUS = 0x14 hand-typed |
| `ctl-00800106` | 0x00800106 NV0080_CTRL_CMD_BIF_GET_PCIE_POWER_CONTROL_MASK (ctrl0080/... | 1 | 0x0 x5; 12 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-00800292` | 0x00800292 NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2 (ctrl0080/ctrl0080gpu... | 1 | 0x0 x5; 804 B; 5 calls | c | no | no | refused: not claimed (the guest class list is authored from the family class table, not through this control) |
| `ctl-00801306` | 0x00801306 NV0080_CTRL_CMD_FB_GET_COMPBIT_STORE_INFO (ctrl0080/ctrl00... | 1 | 0x0 x5; 80 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-00809908` | 0x00809908 undefined in ogkm 595.84 (other versions not yet checked) | 3 | 0x0 x70; 16 B; 70 calls | e | unknown | no | refused: not claimed |
| `ctl-2080012f` | 0x2080012f NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS (ctrl2080/ctrl2080gpu... | 3 | 0x56 x20; 1464 B; 20 calls | c | no | no | refused: not claimed |
| `ctl-2080013f` | 0x2080013f NV2080_CTRL_CMD_GPU_GET_OEM_BOARD_INFO (ctrl2080/ctrl2080g... | 1 | 0x0 x5; 136 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-20800160` | 0x20800160 NV2080_CTRL_CMD_GPU_GET_VPR_CAPS (ctrl2080/ctrl2080gpu.h:2... | 1 | 0x0 x10; 16 B; 10 calls | c | no | no | refused: not claimed |
| `ctl-20800173` | 0x20800173 NV2080_CTRL_CMD_GPU_QUERY_FUNCTION_STATUS (ctrl2080/ctrl20... | 1 | 0x56 x5; 12 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-2080017e` | 0x2080017e NV2080_CTRL_CMD_GPU_GET_VMMU_SEGMENT_SIZE (ctrl2080/ctrl20... | 1 | 0x0 x5; 8 B; 5 calls | c | no | no | refused: sweep triage AmputationIntended (kf-rm/src/sweep.rs, OBJGPU gpuInitVmmuInfo) |
| `ctl-2080080b` | 0x2080080b NV2080_CTRL_CMD_BIOS_GET_UEFI_SUPPORT (ctrl2080/ctrl2080bi... | 1 | 0x0 x5; 8 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-20800a2e` | 0x20800a2e NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_ROP_INFO (ctrl2080... | 1 | 0x0 x5; 96 B; 5 calls | c | no | no | refused: sweep triage AmputationIntended (KernelGraphics) |
| `ctl-20800a30` | 0x20800a30 NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PPC_MASKS (ctrl208... | 1 | 0x0 x5; 512 B; 5 calls | c | no | no | refused: sweep triage AmputationIntended (KernelGraphics) |
| `ctl-20800a3a` | 0x20800a3a NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_WR_OFFSET (ctrl... | 113 | 0x0 x5; 24 B; 5 calls | c | no | no | refused on purpose: SET_FECS_TRACE_WR_OFFSET stays unserviced (kf-rm/src/fecstrace.rs header "refuse the rest") |
| `ctl-20800a3f` | 0x20800a3f NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_TRACE_DEFINES... | 1 | 0x0 x5; 256 B; 5 calls | c | no | no | refused: sweep triage AmputationIntended (KernelGraphics) |
| `ctl-20800a80` | 0x20800a80 NV2080_CTRL_CMD_INTERNAL_PERF_GPU_BOOST_SYNC_GET_INFO (ctr... | 2 | 0x0 x10; 16 B; 10 calls | c | no | no | refused: sweep triage RefusalIsInvisible (KernelPerf) |
| `ctl-20800a87` | 0x20800a87 NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO (ct... | 1 | 0x56 x5; 1056 B; 5 calls | c | no | no | refused: sweep triage AmputationIntended (KernelNvlink) |
| `ctl-20800a9a` | 0x20800a9a NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_SET_2X (ctrl2080/ctrl2... | 25 | 0x0 x99; 8 B; 99 calls | c | unknown | no | refused: not claimed |
| `ctl-20800aaf` | 0x20800aaf NV2080_CTRL_CMD_INTERNAL_GET_ENABLED_SEC2_CLASSES (ctrl208... | 1 | 0x0 x5; 3 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-20800ab8` | 0x20800ab8 NV2080_CTRL_CMD_INTERNAL_GET_PCIE_P2P_CAPS (ctrl2080/ctrl2... | 1 | 0x0 x5; 3 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-20800afe` | 0x20800afe NV2080_CTRL_CMD_INTERNAL_INIT_USER_SHARED_DATA (ctrl2080/c... | 1 | 0x0 x5; 16 B; 5 calls | c | unknown | no | refused: not claimed |
| `ctl-20800aff` | 0x20800aff NV2080_CTRL_CMD_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL (c... | 7 | 0x0 x10; 16 B; 10 calls | c | unknown | no | refused: sweep triage RefusalHalts (GpuUserSharedData) |
| `ctl-20800b03` | 0x20800b03 NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODI... | 1 | 0x0 x5; 16352 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-20800b05` | 0x20800b05 NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_THROTTLE_... | 1 | 0x56 x5; 16352 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-20801110` | 0x20801110 NV2080_CTRL_CMD_FIFO_CONFIG_CTXSW_TIMEOUT (ctrl2080/ctrl20... | 1 | 0x0 x5; 8 B; 5 calls | c | no | no | refused: not claimed (kf-rm/src/sw_runlist_host.rs touches 0x20801111, not this one) |
| `ctl-20801220` | 0x20801220 NV2080_CTRL_CMD_GR_GFX_POOL_INITIALIZE (ctrl2080/ctrl2080g... | 1 | 0x0 x5; 16 B; 5 calls | c | no | no | refused: GfxPoolProbe serves only QUERY_SIZE 0x2080121f (kf-rm/src/gfxpool_probe.rs header) |
| `ctl-20801222` | 0x20801222 NV2080_CTRL_CMD_GR_GFX_POOL_REMOVE_SLOTS (ctrl2080/ctrl208... | 1 | not issued | c | unknown | no | refused: same (pool remove) |
| `ctl-2080205b` | 0x2080205b NV2080_CTRL_CMD_PERF_SET_POWERSTATE (ctrl2080/ctrl2080perf... | 1 | 0x0 x5; 4 B; 5 calls | c | no | no | refused: vfguest answers GET_POWERSTATE (0x2080205a) only (kf-rm/src/vfguest.rs:233) |
| `ctl-20802068` | 0x20802068 NV2080_CTRL_CMD_PERF_GET_CURRENT_PSTATE (ctrl2080/ctrl2080... | 1 | 0x0 x5; 4 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-20802801` | 0x20802801 NV2080_CTRL_CMD_LPWR_DIFR_CTRL (ctrl2080/ctrl2080lpwr.h:42) | 1 | 0x0 x5; 4 B; 5 calls | c | no | no | refused: not claimed |
| `ctl-20802806` | 0x20802806 undefined in ogkm 595.84 (other versions not yet checked) | 1 | 0x0 x5; 4 B; 5 calls | e | no | no | refused: not claimed |
| `ctl-20802a0f` | 0x20802a0f NV2080_CTRL_CMD_INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE (c... | 1 | 0x0 x5; 24 B; 5 calls | c | no | no | refused: sweep triage RefusalHalts (KernelCE) |
| `ctl-20802a12` | 0x20802a12 NV2080_CTRL_CMD_CE_IS_DECOMP_LCE_ENABLED (ctrl2080/ctrl208... | 8 | 0x0 x40; 8 B; 40 calls | c | unknown | no | refused: not claimed |
| `ctl-208081ea` | 0x208081ea undefined in ogkm 595.84 (other versions not yet checked) | 1 | 0x0 x5; 4 B; 5 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-20808524` | 0x20808524 undefined in ogkm 595.84 (other versions not yet checked) | 1 | 0x0 x5; 4 B; 5 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080852a` | 0x2080852a undefined in ogkm 595.84 (other versions not yet checked) | 2 | 0x0 x24; 1784 B; 24 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080852e` | 0x2080852e undefined in ogkm 595.84 (other versions not yet checked) | 12 | 0x0 x19; 528 B; 19 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080852f` | 0x2080852f undefined in ogkm 595.84 (other versions not yet checked) | 11 | 0x0 x4; 776 B; 4 calls | e | unknown | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080853a` | 0x2080853a undefined in ogkm 595.84 (other versions not yet checked) | 11 | 0x0 x5; 1456 B; 5 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080853b` | 0x2080853b undefined in ogkm 595.84 (other versions not yet checked) | 11 | not issued | e | unknown | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-20809004` | 0x20809004 undefined in ogkm 595.84 (other versions not yet checked) | 86 | 0x0 x640; 1544 B; 640 calls | e | unknown | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080a02f` | 0x2080a02f undefined in ogkm 595.84 (other versions not yet checked) | 1 | 0x56 x5; 1 B; 5 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080a060` | 0x2080a060 undefined in ogkm 595.84 (other versions not yet checked) | 1 | 0x0 x5; 28 B; 5 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080a088` | 0x2080a088 undefined in ogkm 595.84 (other versions not yet checked) | 2 | 0x0 x10; 12 B; 10 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080a618` | 0x2080a618 undefined in ogkm 595.84 (other versions not yet checked) | 11 | 0x0 x24; 8620 B; 24 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080a619` | 0x2080a619 undefined in ogkm 595.84 (other versions not yet checked) | 11 | not issued | e | unknown | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080a630` | 0x2080a630 undefined in ogkm 595.84 (other versions not yet checked) | 1 | 0x0 x5; 1160 B; 5 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080a70a` | 0x2080a70a undefined in ogkm 595.84 (other versions not yet checked) | 1 | 0x0 x5; 26 B; 5 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080a801` | 0x2080a801 undefined in ogkm 595.84 (other versions not yet checked) | 2 | 0x0 x10; 1028 B; 10 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-2080b201` | 0x2080b201 undefined in ogkm 595.84 (other versions not yet checked) | 41 | 0x0 x10; 2188 B; 10 calls | e | no | no | refused: GSS-legacy (cmd bit 15), not answered unless KF3_GSS_NATIVE=1 (off in run 641) forwards it to the host RM (doc... |
| `ctl-20810108` | 0x20810108 undefined in ogkm 595.84 (other versions not yet checked) | 2 | 0x0 x16; 992 B; 16 calls | e | no | no | refused: not claimed (NV2081 binary-API class control); default 0x56 |
| `alloc-0000402c` | class 0x402c NV40_I2C (RmAlloc, fn 103) |  | see notes | a | no | no | refused 0x56 (1 time, run 641): kayfabe presents no I2C daughterboard (kf-disp/src/model.rs:71 NO_I2C_PORT = 16); the r... |
| `hostask-2080014a` | host-side ask 0x2080014a NV2080_CTRL_CMD_GPU_GET_GID_INFO (named in h... |  | see notes | c | unknown | no | kayfabe asks the HOST at realize (c: runtime host query) but refuses its own ask: no row in kf_abi::hostabi::HOST_CONTR... |
| `hostask-20809064` | host-side ask 0x20809064 GSS-legacy clock-domain listing (undefined i... |  | see notes | d | yes | no | kayfabe asks the HOST at realize (c: runtime host query) but refuses its own ask: no public header; gssreplay ROWS meas... |
| `hostask-2080a028` | host-side ask 0x2080a028 GSS-legacy per-domain clock query (undefined... |  | see notes | d | yes | no | kayfabe asks the HOST at realize (c: runtime host query) but refuses its own ask: same |
| `hostask-2080a084` | host-side ask 0x2080a084 GSS-legacy cudart clock query (undefined in... |  | see notes | d | yes | no | kayfabe asks the HOST at realize (c: runtime host query) but refuses its own ask: same |
| `hostask-2080a026` | host-side ask 0x2080a026 GSS-legacy cudart max-clock query (undefined... |  | see notes | d | yes | no | kayfabe asks the HOST at realize (c: runtime host query) but refuses its own ask: same |
| `ctl-20801823-err` | 0x20801823 NV2080_CTRL_CMD_BUS_GET_INFO_V2 (ctrl2080bus.h) | 1 (0x56) | 0x0 x160; 420 B (all NV_OK) | c | yes | yes | served NV_OK 5 times from hostfacts pcie_max_gen (BUS_GET_INFO_V2 PCIE_GEN_INFO, host query) but answered 0x56 once (tr... |

### 2.2 Section 1b: served from a non-derived source (26)

| id | name | kayfabe source | shape | state-dep. | fields set by kayfabe |
|---|---|---|---|---|---|
| `ctl-0073013d` | 0x0073013d NV0073_CTRL_CMD_SYSTEM_QUERY_DISPLAY_IDS_WITH_MUX (8 B) | request echoed NV_OK (KF3_DISPLAY_CTRL_PROBE=1, ON in run 641): kf-rm/src/display_ctrl_probe.rs:29-36 ECHOED, link :53-96; the mo... | c | yes | no |
| `ctl-0073014b` | 0x0073014b NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_I2C_SUPPORT (8 B) | request echoed NV_OK (KF3_DISPLAY_CTRL_PROBE=1): display_ctrl_probe.rs:29-36 | c | yes | no |
| `ctl-0073012c` | 0x0073012c NV0073_CTRL_CMD_SYSTEM_VRR_DISPLAY_INFO (12 B) | request echoed NV_OK (KF3_DISPLAY_CTRL_PROBE=1) | c | yes | no |
| `ctl-007302a3` | 0x007302a3 undefined in ogkm 595.84 (other versions not yet checked)... | request echoed NV_OK (KF3_DISPLAY_CTRL_PROBE=1) | d | yes | no |
| `ctl-00730109` | 0x00730109 NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_CONFIG (24 B) | request echoed NV_OK (KF3_DISPLAY_CTRL_PROBE=1): ECHOED row, display_ctrl_probe.rs:29-36 | c | yes | no |
| `ctl-00731369` | 0x00731369 NV0073_CTRL_CMD_DP_GET_CAPS (60 B) | request echoed NV_OK (KF3_DISPLAY_CTRL_PROBE=1) | c | yes | no |
| `ctl-2081010d` | 0x2081010d undefined in ogkm 595.84 (other versions not yet checked) | NV_OK, zero-filled params, only the status is ours: kf-rm/src/hoststub.rs:53 over the generated cell kf-abi/src/hoststub_generate... | a | no | no |
| `ctl-20800a38` | 0x20800a38 NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE | bEnable = FALSE ("feature absent"), default route only, exact params size: kf-rm/src/fecstrace.rs:49 | c | no | yes |
| `ctl-2080220d` | 0x2080220d/0x2080220e NV2080_CTRL_CMD_SET/GET_RC_RECOVERY | per-VM recorded setting that STARTS ENABLED (vfguest.rs header; supersedes the earlier DISABLED answer) | c | yes | yes |
| `ctl-2080205a` | 0x2080205a NV2080_CTRL_CMD_PERF_GET_POWERSTATE | powerState = AC, per VM, never forwarded: kf-rm/src/vfguest.rs:233 | c | no | yes |
| `ctl-20801111` | 0x20801111 undefined in ogkm 595.84 (other versions not yet checked)... | NV_OK, request echoed, contents ignored, only under KF3_SW_RUNLIST_HOST_OWNED=1 (ON in run 641): kf-rm/src/sw_runlist_host.rs:36-... | e | yes | no |
| `ctl-20801211` | 0x20801211 NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND (112 B at 580.x) | NV_OK, request echoed, NOTHING bound, under KF3_PREEMPT_BIND_PROBE=1 (ON in run 641): kf-rm/src/preempt_bind_probe.rs:31,120 | c | yes | no |
| `ctl-20801208` | 0x20801208 NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND (24 B), cross-client c... | NV_OK, request echoed, nothing bound, under KF3_ZCULL_BIND_PROBE=1 (ON in run 641): kf-rm/src/preempt_bind_probe.rs:151,211 | c | yes | no |
| `ctl-2080121f` | 0x2080121f NV2080_CTRL_CMD_GR_GFX_POOL_QUERY_SIZE | reply from INVENTED virtual dimensions: 4 KiB per slot, up to 4096 slots (kf-abi/src/gfxpool.rs:13-15,58-62), under KF3_GFX_POOL_... | c | yes | yes |
| `ctl-20809009` | 0x20809009 undefined in ogkm 595.84 (other versions not yet checked)... | captured row {0, 0xd}, 8 B: kf-abi/src/cudartinit.rs:83,112 (SERVED :134), measured GA106 RTX 3060 580.159.04 on 2026-08-20 | e | no | no |
| `ctl-20809001` | 0x20809001 undefined in ogkm 595.84 (other versions not yet checked)... | captured row {0x03fc007f, 0}, 8 B: cudartinit.rs:86,113 | e | no | no |
| `ctl-20809064` | 0x20809064 undefined in ogkm 595.84 (other versions not yet checked)... | captured cudart row (10 leading words, 120 zero words): cudartinit.rs:90,114; served NV_OK 86 times in run 641 after the host ask... | e | no | no |
| `ctl-50700709` | 0x50700709 NV5070_CTRL_CMD_SYSTEM_GET_CAPS_V2 | empty capsTbl, never BUG_644815_DNISO_VIDMEM_ONLY: kf-disp/src/model.rs:1264 ("caps5070") | c | no | yes |
| `ctl-50700118` | 0x50700118 NV5070_CTRL_CMD_IMP_SET_GET_PARAMETER (GET IMP_ENABLE) | TRUE only under KF3_DISPLAY_IMP_ENABLE=1 (ON in run 641); the model default is FALSE: kf-disp/src/model.rs:1093 + kf-rm/src/displ... | c | yes | yes |
| `ctl-0073028a` | 0x0073028a NV0073_CTRL_CMD_SPECIFIC_GET_PCLK_LIMIT | pclkLimit/orPclkLimit/vbPclkLimit = the connector's monitor.max_pixel_khz = 165000 kHz default (hand-typed DVI single-link limit)... | c | yes | yes |
| `ctl-00730250` | 0x00730250 NV0073_CTRL_CMD_SPECIFIC_GET_CONNECTOR_DATA (72 B) | type DVI_D, location 0, platform DEFAULT_ADD_IN_CARD, count 1 (authored): kf-disp/src/model.rs:1369 | c | yes | yes |
| `ctl-00731140` | 0x00731140 NV0073_CTRL_CMD_DFP_GET_INFO | flags = SIGNAL_TMDS / LINK_SINGLE<<20 (authored; DVI, not HDMI capable): kf-disp/src/model.rs:1456-1466 | c | yes | yes |
| `ctl-0073028b` | 0x0073028b NV0073_CTRL_CMD_SPECIFIC_OR_GET_INFO | type SOR, protocol SINGLE_TMDS_A, location CHIP, bIsLitByVbios 0 (authored constants): kf-disp/src/model.rs:1354-1367 | c | yes | yes |
| `ctl-00730107` | 0x00730107 NV0073_CTRL_CMD_SYSTEM_GET_SUPPORTED | displayMask = all connectors of kayfabe's monitor model (4 SORs worth): model.rs "supported" | c | yes | yes |
| `ctl-0073010a` | 0x0073010a NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_STATE | hotplugAfterEdidMask = every display of this device; LID_OPEN (kf-disp/src/model.rs:1124); real 0x7e00 only under KF3_DISPLAY_HOT... | c | yes | yes |
| `ctl-00730245` | 0x00730245 NV0073_CTRL_CMD_SPECIFIC_GET_EDID_V2 | the monitor model's EDID (authored from the window size / the configured mode; kf-disp/src/edid.rs) not the physical monitor's | c | yes | yes |

### 2.3 Section 2: constants and probe tables (35)

| id | what | kayfabe source | shape | state-dep. | fields set by kayfabe |
|---|---|---|---|---|---|
| `const-get-display-mode-73011a` | NV0073 0x0073011a GET_DISPLAY_MODE: id, 28 B layout {displayId, active w, active h, total w, t... | kf-disp/src/model.rs:50-55 (id, branch list, size), answer at :1208-1262 (values from the armed hea... | d | yes | yes |
| `const-get-hdcp-state-730280` | NV0073 0x00730280 GET_HDCP_STATE: id (ogkm 595.84 only) and 12 B layout; reply flags & ENCRYPT... | kf-disp/src/model.rs:79 (id, "Hand-typed, not derived: owner review before it may become default")... | c | yes | yes |
| `const-echo-730122` | NV0073 0x00730122 (undefined in ogkm 595.84 (other versions not yet checked)): echo of request... | kf-rm/src/display_ctrl_probe.rs:126 (private_answer), KF3_DISPLAY_PRIVATE_PROBE=1 only (off in run... | d | yes | no |
| `const-reply-730128` | NV0073 0x00730128 (undefined in ogkm 595.84 (other versions not yet checked)): reply words {0,... | kf-rm/src/display_ctrl_probe.rs:116 PRIVATE_0128_REPLY; KF3_DISPLAY_PRIVATE_PROBE=1 only (off in ru... | d | unknown | no |
| `const-hdmi-path-probe` | HDMI path rows: GET_CONNECTOR_DATA words {flags 1, count 1, index 3, type 0x61, location 3}; D... | kf-rm/src/display_ctrl_probe.rs:246-389 (consts :259-267, topology_answer :276), KF3_DISPLAY_HDMI_P... | c | yes | yes |
| `const-crossbar-assign-sor` | NV0073 DFP_ASSIGN_SOR 0x00731152 (80 B): fixed crossbar, connector i on SOR i, type SINGLE; re... | kf-disp/src/model.rs:915-999 (assign_sor, derived layout names); also hand-typed copy in display_ct... | c | yes | yes |
| `const-caps-v2-observed-rm-features` | NV0073 0x00730101 GET_CAPS_V2 capsTbl: the three non-derivable bits AA_FOS_GAMMA_COMP, KSV_SRM... | AT HEAD c1aa06b0: removed. kf-disp/src/dispcaps.rs:14-60 derives only HDMI_2_0 (TMDS clock > 340 MH... | b | no | yes |
| `const-num-heads-client` | NV0073 0x00730102 GET_NUM_HEADS flags=CLIENT: count = heads with a lit SOR (armed state) | kf-disp/src/model.rs:872-880 heads_in_use ("[inferred] rule"), :1269-1307 | b | yes | yes |
| `const-get-active-heads` | NV0073 0x0073010c GET_ACTIVE: displayId on the lit SOR | kf-disp/src/model.rs:1323-1335 (from armed state, test model.rs:1965) | b | yes | yes |
| `const-display-model-authored` | kf-disp authored virtual-display facts in other controls: static_info (numDispChannels 81, i2c... | kf-disp/src/model.rs:1149-1170 (static_info), :1619-1634 (mode_possible), (get_type :1396, directmo... | c | yes for channel_info | yes |
| `const-display-caps-page-probe` | Display capabilities page (NV_PDISP_FE_SW at BAR0 0x640000): the real RTX 4070's 99 non-zero w... | kf-qemu/src/display.rs:141-259 (CAPS_PROBE_AT :155, CAPS_PROBE_WORDS :159), enabled by KF3_DISPLAY_... | e | no | no |
| `const-sdr-page` | Authored SDR colour capabilities (ILUT/OLUT DIRECT10, LOGSZ 10, SFCLOAD, TMO with a 1024-entry... | kf-disp/src/caps.rs:80 sdr_page; kf-qemu/src/display.rs:1785 | c | no | yes |
| `const-lut-mirror-offset256` | Colour-program acceptance rules: LUT CONTROL MIRROR accepted on DIRECT8/DIRECT10 ILUT/OLUT (OL... | kf-disp/src/color.rs:63,260,270,482 + kf-qemu/src/display.rs:121-122; KF3_DISPLAY_LUT_MIRROR / KF3_... | b | yes | yes |
| `const-evt-stat-loadv` | NV_PDISP_FE_EVT_STAT_HEAD_TIMING bit 0 (LOADV), hand-named; real reads 0x7 at every head-timin... | kf-qemu/src/display.rs:59-70 EVT_STAT_HEAD_TIMING_LOADV (hand-named, nouveau gv100.c cited); KF3_DI... | b | yes | yes |
| `const-blank-state` | Core channel SET_GET_BLANKING_CTRL(h) read-back: 0x2 (UNBLANK) for the lit head, 0x1 (BLANK) f... | kf-qemu/src/display.rs:84-95 blank_state_on, KF3_DISPLAY_BLANK_STATE=1 (off in run 641) | b | yes | yes |
| `const-armed-min-frame-idle` | ARMED HEAD_SET_MIN_FRAME_IDLE(h) default 0x10002 (leading 2, trailing 1) | kf-qemu/src/display.rs:96-139 (MIN_FRAME_IDLE_DEFAULT :139), KF3_DISPLAY_ARMED_DEFAULTS=1 (off in r... | b | yes | yes |
| `const-head-timing-en-base` | NV_PDISP_FE_RM_INTR_EN_HEAD_TIMING(h) (0x611D80) read-back base 0x3f0060 (firmware-enabled RG_... | kf-qemu/src/display.rs:1863-1885, KF3_DISPLAY_HEAD_TIMING_EN_BASE=<hex> (off, no value) | b | yes | no |
| `const-core-latch-vblank` | Core update latch timing: an update group including the CORE latches at the next vblank of an... | kf-disp/src/engine.rs:589-595 core_latch_at_vblank, KF3_DISPLAY_CORE_AT_VBLANK=1 (off) | a | yes | n/a |
| `const-head-timing-frame-edge` | Head-timing events LAST_DATA/VBLANK pending each frame edge whether enabled or not; first inte... | kf-qemu/src/display.rs:285-300 frame_edge (default behaviour) | a | yes | n/a |
| `const-monitor-pclk-165000` | Monitor model max pixel clock 165,000 kHz (DVI single-link) and the authored monitor facts (co... | kf-disp/src/edid.rs:155-170 (max_pixel_khz), model.rs:1369-1380 (DVI_D) | b | no | yes |
| `const-ce-fault-method-buffer` | NV2080 0x20802a08 CE_GET_FAULT_METHOD_BUFFER_SIZE: 0x5000 (20 KiB), every family | kf-rm/src/authored.rs:52-70 (Why::Advertised) | b | no | yes |
| `const-gmmu-static` | NV2080 INTERNAL_GMMU_GET_STATIC_INFO fault buffer sizes: replayable 0x31000, non-replayable 0x... | kf-rm/src/authored.rs:71-93 GMMU_STATIC | c | no | yes |
| `const-fecs-record-size` | NV2080 INTERNAL_STATIC_KGR_GET_FECS_RECORD_SIZE: 128 | kf-rm/src/authored.rs:94-110 | b | no | yes |
| `const-gsp-disp-vectors` | Interrupt-tree vectors: GSP stall 0x9b (MC_ENGINE_IDX_GSP 50) and DISP stall 0x9a (MC_ENGINE_I... | kf-rm/src/authored.rs:135-157 | b | no | yes |
| `const-engine-layout` | FIFO device-info table slot layout (runlists, PBDMA, RESET bits, CHRAM/RUNLIST PRI bases, INTR... | kf-rm/src/authored.rs:535-600 (ENGINE_LAYOUT_WHY :535, engine_table :560) | c | yes | yes |
| `const-user-register-access-map` | User register access map (gpuConstructUserRegisterAccessMap) served from accessmap.rs | kf-rm/src/hostfacts.rs PROVENANCE row user_register_access_map = Source::Authored("accessmap.rs");... | e | unknown | unknown |
| `const-conf-compute-bif-static` | CONF_COMPUTE_GET_STATIC_INFO ("CC off, fabricated so ogkm accepts it") and BIF_GET_STATIC_INFO... | kf-rm/src/hostfacts.rs PROVENANCE rows conf_compute, bif_static (Source::Authored) | c | no | yes |
| `const-bar0-boot-advertised` | BAR0 boot registers served as Advertised: NV_VIRTUAL_FUNCTION_PRIV_ACCESS_COUNTER_NOTIFY_BUFFE... | kf-chip/src/bar0.rs:104-123,176-180,244-252 (Provenance::Advertised) | b | no | yes |
| `const-fecs-trace-absent` | FECS trace answered "disabled" (bEnable=FALSE) and its SETs refused | kf-rm/src/fecstrace.rs:49 | b | no | yes |
| `const-gfxpool-sizes` | GfxP pool sizing from invented virtual dimensions (4096 B per slot, <= 4096 slots) | kf-abi/src/gfxpool.rs:13-62; kf-rm/src/gfxpool_probe.rs; KF3_GFX_POOL_PROBE=1 | c | yes | yes |
| `const-cudartinit-rows` | Captured GA106 rows for 0x20802209 (RC_GET_WATCHDOG_INFO {5}), 0x20809009, 0x20809001, 0x20809... | kf-abi/src/cudartinit.rs:80-146 | e | no | no |
| `const-gssreplay-rows` | GSS-legacy host-replay rows: request layouts for 0x20809064, 0x2080a028, 0x2080a026, 0x2080a08... | kf-abi/src/gssreplay.rs:56-104 (ROWS) | d | yes | no |
| `const-hostabi-rows` | Host-side RM struct layouts per host driver version (HOST_CONTROLS) and the pinned host interv... | kf-abi/src/hostabi.rs:503-848; host_driver.rs | c | n/a | n/a |
| `const-sw-runlist-object` | Admission of the Windows software-runlist object (class 0xb297) as a graph-only object: maxTSG... | kf-rm/src/sw_runlist_probe.rs (KF3_SW_RUNLIST_PROBE=1, ON in run 641); kf-rm/src/lib.rs:487 | c | unknown | unknown |
| `const-memory-list-probe` | ALLOC_MEMORY contiguous SYSRAM / FBMEM (class 0x82) MemoryList descriptor accepted as a typed... | kf-rm/src/lib.rs:570 (KF3_MEMORY_LIST_PROBE=1, ON in run 641); kf-rm/src/rmrpc/policy.rs:522-538 | c | unknown | unknown |

### 2.4 Section 3: family rows, ALLOWED (11)

Policy: `AGENTS.md` Rules ("Only family rows are maintained by hand"); `kf-abi/src/lib.rs:65-66` names the capability allowlist as the one hand-kept row set; `kf-chan/src/grtables.rs:144` says "a family row, by hand (allowed)". These are listed so nobody re-lists them as open. Where a family function carries an Advertised value no source states, that part is an open row in section 2 (`const-engine-layout`, `const-bar0-boot-advertised`).

| id | what | where |
|---|---|---|
| `fam-kf-chip-family` | kf_chip::Family answers: mmu_format, boot_style, engine_writes_userd_gp_get, chids_per_runlist, gsp_heartbeat_mailboxes... | kf-chip/src/lib.rs:181-290 |
| `fam-intr-subtree-map` | authored_intr_subtree_map per family (Turing/Ampere/Ada, Hopper; Blackwell None) | kf-chip/src/lib.rs:380-398 |
| `fam-display-row` | kf-chip display rows per die group: display classes, IP version, heads=4 and windows per head | kf-chip/src/display.rs:55-150 (TURING, AMPERE, ADA, BLACKWELL_GB20X) |
| `fam-capability-allowlist` | Capability allowlist (default-deny control/class tables, nvproxy-derived, newest row <= version) | kf-abi/src/capability.rs:1906-2100 (SHARED_CAPS :1906, CAPS_<version> statics from :1917) |
| `fam-report-semaphore` | Report-semaphore method offsets family set (Turing, Ampere, Ada, Hopper) | kf-chan/src/grtables.rs:144-153 REPORT_SEMAPHORE_FAMILIES (:149) |
| `fam-ptekind` | Uncompressed PTE kind table (TU102 style) per family | kf-chip/src/ptekind.rs:24-60 |
| `fam-engine-table` | Engine table layout rules per family (authored::engine_table, ofa_fault_id, fault ids) | kf-rm/src/authored.rs:300-620 |
| `fam-gsp-model` | GSP model per family (BootStyle, Family::gsp_model, falcon/FSP boot structures) | kf-chip/src/lib.rs:277-346; kf-chip/src/falcon_gsp.rs; fsp_gsp.rs |
| `fam-bar0-boot-regs` | BAR0 boot registers per family composed from host facts or ogkm rules | kf-chip/src/bar0.rs:140-262 |
| `fam-class-sets` | Class sets generated from ogkm (kf-chip classes) and the host classes derived from the host class list | kf-chip/src/classes.rs; host_classes.rs |
| `fam-sked` | SKED method table: per die group, held to each header; no family row | kf-chip/src/sked.rs:29 |

## 3. Notes

### 3.1 Controls whose real status equals kayfabe's (do not count as divergences) (M)

`0x2080012f` (real 0x56, 1464 B), `0x20800a87` (0x56, 1056 B), `0x20800173` (0x56, 12 B), `0x20800b05` (0x56, 16352 B), `0x2080a02f` (0x56, 1 B), `0x00730282` (0x56, 2600 B). Partly: `0x00730285` (real 0x56 x7 and 0x1f x4 in gl boot1; kayfabe 0x56 always). The brief's `0x2080a02f` and `0x20800a87` figures come from `vfio-gl-20261011/cmp.txt`.

### 3.2 Controls whose real status is NOT 0 or 0x56 (M)

`0x00730288` real 0x1f in every capture (60 B); `0x007302a6` real 0x14; `0x00730285` mixed 0x56 / 0x1f; `0x00731341` real 0xffff (not in the kayfabe trace); `0x00730282` 0x56. Each is a case where "serve NV_OK" would be wrong too; the rows say so.

### 3.3 The 0x730101 caps table across revisions (M)

Real GPU: `81 2f` in every capture. Run 641 (`85f2217d`): `81 2f`, from `OBSERVED_RM_FEATURES` (3 bits) plus derived bits. HEAD (`c1aa06b0`): derived `[0x00, 0x0c]` (HDMI_2_0 and CROSS_BAR only; `kf-disp/src/dispcaps.rs` test at line 123). Missing at HEAD relative to the real GPU: byte 0 `0x81` (AA_FOS_GAMMA_COMP, KSV_SRM_VALIDATION) and byte 1 `0x23` (SINGLE_HEAD_MST, SINGLE_HEAD_DUAL_SST, GLITCHLESS_MODESET). Whether the Windows driver needs them at HEAD is **unknown**; the run-641 GL crash occurred with them present (`traces/windows_gl_crash_20261011/README.md`).

### 3.4 Axes: what the evidence supports (M) and what it does not (I)

- **die / display IP version / guest-driver branch:** no row has measured variation. Real answers exist for AD104 / DISPv0404 / Windows 580.88 only. Older GA106 (RTX 3060) captures exist for the Linux 580.159.04 oracle (`traces/real_ga106/`, cited by `kf-abi/src/oracle.rs`, `cudartinit.rs`, `cepce.rs`) and underlie the `cudartinit` rows; they are a second die but a different OS and driver, so they do not isolate an axis either.
- **monitor state:** two real sessions on the same monitor and EDID differ in connector path (DVI vs HDMI-A per the session names) but give identical replies for every `0x0073xxxx` control except `0x0073011d` (connector table; 0 of 3 vs 2 distinct payloads intersect) and the volatile payloads; `0x0073011a` was issued only in the GL sessions. A monitor-dependent axis (EDID, HDCP-capable sink, multiple monitors, lid) was never varied: **unknown**.
- **host driver:** M for exactly one family of rows: the GSS host-replay asks and `0x2080014a` are refused at host 595.91.07 because the layouts were measured at 580.159.04 (`hostabi.rs`, `gssreplay.rs`). Everything else the host answers at realize was not checked at 595.91.07 in this inventory.
- **Linux vs Windows guest:** the sweep triage rows (`kf-rm/src/sweep.rs`) were derived from a Linux 580.159.04 GA106 boot. For six refused controls (`2080017e`, `20800a2e`, `20800a30`, `20800a3f`, `20800a80`, `20800a87`, plus `20800aff`, `20802a0f`) the triage decision is therefore **not** evidence for the Windows/AD104 guest, where the real GPU answers OK for all but `20800a87`.

### 3.5 Not determined (see also the final report)

- kayfabe-side params sizes and reply bytes for refused or served controls outside the trace's display set; the real request sizes are not in `cmp.txt` either.
- Whether any refused control is on a path Windows depends on (no ablation was run; none is claimed). The GL crash investigation names candidates in `traces/windows_gl_crash_20261011/README.md`.
- Real answers for `0x2080853b`, `0x2080a619` and `0x20801222` (never issued in the real captures) and for `0x50700709` (not compared).
- Request/response layouts of all unnamed `0x2080[89ab]xxx` ids: no header found in ogkm 595.84 (other versions not yet checked); V3_GSS_NATIVE.md section 1 says none in any open header it checked.
- Vast-fleet or other-die measurements: none exist in this repo; the owner ruling AF fleet table is not built.
- Whether other served controls (host-derived `0x2080xxxx`) match the real GPU byte for byte: the run-641 trace does not log their reply bytes.
- Display register values not seen by this grep: only constants carrying a marker (`[measured, VFIO`, `PROBE`, `EXPERIMENT`, `Advertised`, `captured`) were collected; an unmarked literal in `kf-disp/src/engine/` or `kf-qemu/src/display.rs` would be missed.
