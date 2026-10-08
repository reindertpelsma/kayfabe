# Display reply diff — real RTX 4070 vs kf3, before Windows' first window programming

**STATUS: LIVE, 2026-10-09 (analysis of 2026-10-08 captures; GPU-free — no boot, no GPU lock, no QEMU).** Branch
`claude/display-reply-diff-20261008` from `claude/windows-flip-vsync-20261008` `a113f160`. Question: Windows under kf3 never
writes a window PUT after the modeset (display PUT count stays 39, runs 88-96), while on the real GPU the primary-surface DDI
writes window 0's PUT at 12.596 s (boot3) with no display read before it inside the DDI. So the KMD's decision is formed
EARLIER, from RPC answers and register read-backs. This record diffs both, field by field, ranks the differences, and adds five
default-off experiment flags (three new answers, two new read-backs) for one batched hardware boot.

Sources. Hardware: `traces/vfio_dvi_reference_20261008/boot3-gsp.jsonl.gz` (x-gsp-observer: every GSP RPC request AND reply
body) and `boot3-qemu-trace.log.gz` (every trapped BAR0 read/write with its value), on branch `claude/vfio-dvi-reference-20261008`
`15e80847`; boots 1-2 for cross-checks. kayfabe: the run 88/89/90/93/95/96 kf3 logs (`KF3_RPC_TRACE` gives statuses only, no
bodies; `KF3_DISPLAY_METHOD_TRACE` + `KF3_DISPLAY_WRITE_TRACE` give the guest's display methods/writes in run 96). kf3's reply
BODIES come from the code, replayed: `cargo run -p kf-rm --example display_replay -- 580.65.06` feeds every hardware
display request of boot3 (2359 requests up to 12.62 s, [input](replay-requests-boot3.txt.gz)) through kf's display link, the
`KF3_DISPLAY_CTRL_PROBE` echo and the `0x20808159` identity, with the runs' flags (`KF3_DISPLAY_CTRL_PROBE=1
KF3_DISPLAY_IMP_ENABLE=1`) → [kf replies](kf-replies-boot3-default-flags.txt.gz), [diff](reply-diff-boot3-default-flags.txt.gz).
kf3 never traps a BAR0 read, so kf3's read-back values below come from the display model code (`crates/kf-qemu/src/display.rs`,
`crates/kf-disp/src/engine.rs`), labelled `[code]`. Times are host UTC seconds of 20:10 (boot3). Tools: [tools/](tools/).
Field names: ogkm-595.84 headers (the 580.65.06 layouts kf3 answers with agree on every struct used here).

Labels: `[measured, <capture>]` = read from a capture (the captures of 2026-10-08); `[code]` = kf3's value derived by reading its code (or the GPU-free
replay of that code); `[inferred]` = reasoning, never evidence.

## 0. Ordered stream alignment (2026-10-09, owner direction: traces first, no ETW)

⊘ **Corrects §1 below** (written before this alignment): the KMD's display decisions do NOT match before
the modeset. The first display WRITE that differs is at **11.0746 s, at driver start** (the window and core
channel init pushes, row P16-P17), 1.4 s before the modeset; and on the hardware the SPD-infoframe block in
`NV_PDISP_SF_USER_0` is rewritten 5 times before the first window PUT, under kf3 twice (§1 said "the same").

**Method.** [align.py](tools/align.py) builds both streams and aligns them with `difflib` on item keys:
hardware = x-gsp-observer RPCs (function + control id + object handle, request AND reply) plus the
`vfio_region_write` display-range writes (`0x610000-0x6fffff`), in host-time order; kf3 = run 93's
`rpc-trace` lines (status; a `result=none` takes the status of the `GSP REFUSED` line that follows) plus its
`WTRACE` display writes, in log order. Object handles match on both sides (the guest picks them); the CPU-RM's
internal `0xabcdXXXX` handles are logged as 0 by kf-rm and are normalized. Window: hardware from the first
display-class RPC (9.800088, the `0x0073` alloc) to the first window-0 PUT (12.596419); kf3 from the same
alloc to the `0x90f10106` that follows the post-modeset `0x20808159`. The reply BODIES on the kf3 side are the
GPU-free replay of the hardware request through kf's display link (`[code]`); statuses are measured (boot3 and
run 93, RTX 4070, 2026-10-08).
Run 96 aligns identically (same condensed diff); run 88 has no `WTRACE`, and its first 769 RPCs equal run 93's.
Outputs: [aligned-boot3-vs-run93.txt.gz](aligned-boot3-vs-run93.txt.gz) (every item, `DIFF`/`HWONLY`/`KFONLY`),
[ordered-diffs-boot3-vs-run93.txt](ordered-diffs-boot3-vs-run93.txt) (the 355 non-equal groups, named from
ogkm-595.84), [hw-only-reads-events-msi-boot3.txt.gz](hw-only-reads-events-msi-boot3.txt.gz),
[the DDI window](hw-boot3-ddi-window-12.55-12.5965.txt), [the caps page](caps-page-boot3-vs-kf.txt).

**What cannot be aligned, and why** (boot3 and run 93, RTX 4070, 2026-10-08). (a) kf3 display READS never exit (`read_exits=0` on every status line), so
no kf3 read value is in any trace: every read-back row below is *hardware measured vs kf3 model code*. (b) kf3
logs no per-item MSI or GSP event; the hardware window holds 20 MSIs (all decoded as CPU_DOORBELL, GSP, CE2 or
GR0 non-stall by `boot3-msi-timeline`) and 1 event (`0x1020` at 11.3574, the NOCAT record); kf3's only
display-interrupt record is `WTRACE VSYNC`. (c) Non-display trapped BAR0 traffic on the hardware (GSP queue
`0x110000`, `0xb81000`, `0x702000-0x715000`; 23k-53k accesses) has no kf3 per-item log. (d) Push-buffer
CONTENTS are not in the hardware trace (only the PUT writes); kf3's `METHOD` trace has them.

**First positions** `[measured, boot3 + run 93, RTX 4070, 2026-10-08]`:
- first kayfabe-visible difference of any kind: **P1, 9.802520** (hardware sends one more
  `INTERNAL_FIFO_GET_NUM_CHANNELS`), then **P2, 9.806929** `INTERNAL_INIT_USER_SHARED_DATA` NV_OK vs
  `NOT_SUPPORTED`; 36 distinct non-display controls (45 calls) answered NV_OK on hardware are refused by kf3 between 9.80 and 11.46
  (a repeat refusal logs `UNSERVICED` without a second `GSP REFUSED` line; the alignment counts it as refused);
- first display-class difference: **P3, 10.025289** `INTERNAL_DISPLAY_GET_STATIC_INFO` reply;
- first display WRITE difference: **P16, 11.074621** (window 0 PUT 0x30 vs 0x0);
- first difference inside the primary-set DDI window (12.5707-12.596): **P38, 12.570316** `GET_HDCP_STATE`
  (NV_OK vs refused; falsified ALONE as the gate, run 95), then the read-backs P39 and the first
  kayfabe-visible DDI write that differs, **P40, 12.570799** `RM_INTR_EN_HEAD_TIMING(0)` 0x3f0062 vs 0x2.

| P | hw time / h-index | hardware item | kf3 item (run 93) | field-level difference (ogkm-595.84 names) | label |
|---|---|---|---|---|---|
| 1 | 9.802520 h3 | `INTERNAL_FIFO_GET_NUM_CHANNELS` ×5 | ×4 | one call fewer | measured |
| 2 | 9.806929 h8 … 11.454282 | 36 non-display controls (45 calls) NV_OK (`INIT_USER_SHARED_DATA`, `GET_CLASSLIST_V2`, `BUS_GET_INFO_V2`, `STATIC_KGR_*`, `PERF_*`, `CE_*`, …) | `0x56` | status (and 49 hardware-only non-display calls before 11.5, e.g. 22 more `BUS_GET_INFO_V2`) | measured |
| 3 | 10.025289 h23 | `INTERNAL_DISPLAY_GET_STATIC_INFO` | same, NV_OK | `feHwSysCap` 0xf0f vs 0xf; `bInternalMuxSupported` 1 vs 0; `numDispChannels` 0x54 vs 0x51 | hw measured, kf code |
| 4 | 10.150907 h152 | `SYSTEM_GET_SUPPORTED` | same | `displayMask`/`displayMaskDDC` 0x7f00 vs 0x100 (§2 topology) | hw measured, kf code |
| 5 | 10.151266 h153 | `SPECIFIC_OR_GET_INFO(0x100)` (+6 for 0x200-0x4000, hw only) | same (1) | `ditherType` 1 vs 0, `ditherAlgo` 4 vs 0, `dcbIndex` 7 vs 0 | hw measured, kf code |
| 6 | 10.177283 h184 | `SYSTEM_GET_CAPS_V2` ×2 | same | `capsTbl` 0x2f81 vs 0 (§2 #6: CROSS_BAR, GLITCHLESS_MODESET, …) | hw measured, kf code |
| 7 | 10.177634 h185 | `DP_GET_CAPS` | probe echo | 12 words (DP caps vs zero) | hw measured, kf code |
| 8 | 10.182559-10.203254 h198-255 | per-display `GET_TYPE`, `GET_CONNECTOR_DATA`, `DFP_GET_INFO`, `GET_PCLK_LIMIT`, `DP_SET_MANUAL_DISPLAYPORT`, … for 0x200-0x4000 | absent | hw-only (topology) | measured |
| 9 | 10.185028 h205 | `GET_CONNECTOR_DATA(0x100)` | same | §2 #5: `PRESENT_YES`/index 3/`HDMI_A` 0x61/location 3 vs `PRESENT_NO`/0/`DVI_D` 0x31/0 | hw measured, kf code |
| 10 | 10.186099 h208 | `DFP_GET_INFO(0x100)` | same | `flags` 0x105300 vs 0x100000 (§2 #5) | hw measured, kf code |
| 11 | 10.186437 h209 | `GET_PCLK_LIMIT(0x100)` | same | `vbPclkLimit` 0 vs 165000 | hw measured, kf code |
| 12 | 10.204353-10.206811 h257-263 | `GET_ACPI_DOD…` 0x1f ×4; `GET_CONNECTOR_TABLE` NV_OK; `GET_EDID_V2` 384 B | refused / refused / 128 B | statuses; another monitor's EDID | measured / kf code |
| 13 | 10.224177-10.924169 h269-2224 | `GET_HEAD_ROUTING_MAP` ×1956 | absent | hw-only (7 displays) | measured |
| 14 | 10.924525 h2225 | `GET_HOTPLUG_CONFIG` | probe echo | 0x7f00 vs 0 | hw measured, kf code |
| 15 | 10.948185 h2251 | NV5070 `SYSTEM_GET_CAPS_V2` | same | 0x24 vs 0 | hw measured, kf code |
| 16r | **10.948826-10.953350** | READ the caps page `NV_PDISP_FE_SW` 0x640000-0x640fff (8-byte reads) | no read visible | **101 of 1024 words differ** ([list](caps-page-boot3-vs-kf.txt)): `PRECOMP_WIN_PIPE_HDR_CAPA(w)` even windows `0x01df21d0` (SCLR, TMO, CSC*, ALPHA/FULL/UNIT_WIDTH) / odd windows `0x010021d0` (CSC11 only) vs kf `0x01d70000` (CSC*, TMO) for all 8; CAPB-F (scaler, precisions; kf: ILUT/TMO sizes only) differ; `POSTCOMP_HEAD_HDR_CAPA/C-F(h)` vs 0; `SOR_CAP` DP_A/DP_B/DP_8_LANES vs DUAL_TMDS; `SOR_CLK_CAP` DP_MAX 81 vs 0; `HEAD_CLK_CAP` 0x86 ×8 vs 0x77 ×4; words 0x8-0x18, 0x48-0x78, 0xc0-0xf0 vs 0 | hw measured vs kf code (`kf_disp::caps::sdr_page` — the broker sets `KF3_DISPLAY_SDR_COLOR` in every Windows run — Ada, 580.65.06, dumped) |
| 17r | 11.052579-11.074487 | READ the core ARMED area 0x688000-0x68bfff (8-byte) and every window's ARMED/state words | no read visible | hw: the firmware's lit head 0 + windows (most words `0xbadf5040`); window PUTs read 0x974/0x950/0x950/0x95c; kf: shadow, 0 | hw measured vs kf code (§2 #7) |
| **16** | **11.074621 h5103** | W window 0 PUT `0x690000` ← 0x30; windows 1-7 ← 0x10 | ← 0x0 for all 8 | **first display write difference**: the KMD's channel-init pushes differ | measured |
| 17 | 11.075438-11.076918 h5168-5219 | core PUT 0x10,0xe30,0,0x6b0,0x9c0,0xcd0,0,0x310,0x620; windows **0,2,4,6 only**: 0x340/0x320, 0x910/0x8f0, 0xa20/0xa00, 0, 0x7f0 | core 0x10,0x130,0x150,0x460,0x770,0xa80,0xd90; **all 8 windows**: 0x5d0, 0xed0 | push sizes and the window set differ (hardware programs only the full-caps windows of row 16r) | measured; the caps link is inferred |
| 18r | 11.076930 | READ `SET_GET_BLANKING_CTRL(0-3)` | — | 2,1,1,1 vs 0 (§2 #8) | hw measured vs kf code |
| 19 | 11.114331 h5224 | `GET_LOCKPINS_CAPS` | same | numScan/Flip/StereoPins 3/2/4 vs 0 | hw measured, kf code |
| 20 | 11.130915 h5261-5280 | `SET_OD_PACKET` ×8 status 0x1f | 0x56 | status | measured |
| 21 | 11.260721 h5547 | `SYSTEM_GET_ACTIVE` head 0 | same | `displayId` 0x100 vs 0 (§2 #7) | hw measured, kf code |
| 22 | 11.261039-11.273337 h5548-5592 | W BLANK head 0; 4× `DFP_ASSIGN_SOR`, 3× `DP_AUXCH_CTRL`, `HDCP_CTRL`, `GET_CONNECT_STATE`; W core PUT 0x630, window 0 PUT 0x850 ×2, core PUT 0x670 | absent | hw-only: the KMD takes over the firmware's head 0 | measured |
| 23 | 11.319810 h5593 | `GET_EDID_V2` | same | another monitor | hw measured, kf code |
| 24 | 11.353192-11.353931 | `SET_HDMI_SINK_CAPS` (hw only); `GET_HDMI_GPU_CAPS` NV_OK; `GET_HDMI_SCDC_DATA` 0x14 | — / 0x56 / 0x56 | HDMI path (§2 #5) | measured |
| 25 | 11.355714, 11.356655 | `0x00730128` NV_OK | refused | §2 #1 | measured |
| 26 | 11.357092-11.370557 | 28× `DFP_SET_ELD_AUDIO_CAPS`, 4× `DFP_ASSIGN_SOR` | absent | hw-only (§2 #5/#6) | measured |
| 27 | 11.371810-11.419634 | 56× hw-only + 27 shared `IS_MODE_POSSIBLE`; READ `HEAD_SET_MIN_FRAME_IDLE(h)` ARMED ×335 | — | `minImpVPState` 6 vs 0, bandwidths, `dispClkKHz`…; 0x10002 vs 0 (§2 #9/#10) | hw measured vs kf code |
| 28 | 12.332296 h6126 | `SYSTEM_GET_HOTPLUG_STATE` | same | `hotplugAfterEdidMask` 0x7e00 vs 0x100 (§2 #4) | hw measured, kf code |
| 28b | 12.333710 | 5× `GET_EDID_V2` | **6×** | kf-only extra EDID read right after the mask lists 0x100 — the guest acts on the mask | measured |
| 29 | 12.457970-12.498542 | `IS_MODE_POSSIBLE` ×4 | same | as P27 | hw measured, kf code |
| — | **modeset (commit) 12.4910-12.5526** | | | | |
| 30 | 12.493684 h6192 | `DFP_ASSIGN_SOR` | absent | hw-only (§2 #6) | measured |
| 31 | 12.494635-12.495735 | `GET_HDCP_STATE`, `0x00730122`, `0x00730128` NV_OK | refused ×3 | §2 #3/#2/#1 | measured |
| 32 | 12.496493 h6198 | `SET_HDMI_ENABLE` (enable=0) | absent | hw-only (§2 #5) | measured |
| 33 | 12.496839, 12.497132 | W SPD-infoframe blocks `0x6f0228-0x6f024c` ×2 | absent | hw-only, right after `SET_HDMI_ENABLE` | measured |
| 34 | 12.498341-12.552647 | core PUT 0x980,0xba0,0xbc0,0xc60,0xc80,0xca0 | 0x0(kf only),0x310,0x530,0x530,0x550,0x5f0,0x610,0x630 | values differ by the init base (P17); deltas equal | measured |
| 35r | 12.498437 | READ ARMED `HEAD_SET_MIN_FRAME_IDLE(0)` | — | 0x10002 vs 0 | hw measured vs kf code |
| — | 12.519843 / 12.570005 | SPD block; `HDCP_CTRL` 0x56; SPD block | same | equal | measured |
| — | **DDI window 12.5703-12.5964** | | | | |
| **38** | **12.570316 h6306** | `GET_HDCP_STATE` NV_OK | `0x56` | status (§2 #3) | measured |
| 39r | 12.570713-12.570776 | READ `RM_INTR_EN_HEAD_TIMING(0)` 0x3f0060 ×3; `CORE_HEAD_STATE(0)` 0x100 ×2; `EVT_STAT_HEAD_TIMING(0)` 0x7 | — | kf model: 0; 0x200; 0x7 (run 93 had `KF3_DISPLAY_LOADV`, else 0x6) | hw measured vs kf code |
| — | 12.570787 | W `EVT_STAT_HEAD_TIMING(0)` ← 0x2 | same | equal | measured |
| **40** | **12.570799 h6314** | W `RM_INTR_EN_HEAD_TIMING(0)` ← 0x3f0062 | ← 0x2 | the guest's read-modify-write of P39 (LAST_DATA set on both) | measured |
| — | 12.570941 | `SPECIFIC_DISPLAY_CHANGE` 0x007302a4 | same | equal | measured |
| **41** | **12.571274-12.571504** | W SPD-infoframe block (5th) | **absent** | hw-only write block after `DISPLAY_CHANGE` | measured |
| — | 12.580784 | `0x20808159` | same | equal | measured |
| 42 | 12.582748-12.587827 | 6 MSIs (CE2 non-stall decode); display ISR at 12.586250 reads `0x611c30`=0, `0x611ec0`=1, `0x611c00`=2, `0x611800`=0x7; W `0x611800` ← 0x2 once | `VSYNC h0 frame=1 evt=0x7 en=0x2 rm=0x2`; W `0x611800` ← 0x2 **twice** | MSIs and ISR reads not visible on kf3; one write more | measured (kf reads: code) |
| **43** | (none on hw) | no write; LAST_DATA stays enabled to 14.786 | W `RM_INTR_EN_HEAD_TIMING(0)` ← **0x0** | kf-only: the guest disables the frame-edge interrupt after the first VSync | measured |
| — | 12.594318 | `0x90f10106` | same | equal | measured |
| 45 | 12.595845-12.596419 | MSI (GR0 non-stall); W core PUT 0xcf0; **W window 0 PUT 0x870** | absent | the window programming | measured |

Read-backs on the hardware in the window with no kayfabe-visible counterpart (all of them): the caps page
(1024 words), the core ARMED area (4096 words), the window channel/ARMED words (`0x690000-0x697fff`, 314 reads),
core PUT read-backs `0x680000` (18), `SET_GET_BLANKING_CTRL` (4), the SF-user words `0x6f0000-0x6f0e00` (11.1308-
11.1344: `0x6f0100`/`0x6f0500`/`0x6f0900`/`0x6f0d00` = 0x10200), `HEAD_SET_MIN_FRAME_IDLE` ARMED (335), the
SPD-infoframe read-before-write words (`0x6f0228-0x6f024c`, 5 blocks), and the DDI/ISR reads of P39/P42.

**The 11 differences of §2 (ten rows + topology), by position.** Before the modeset only: #4 (12.3323),
#7 (11.05-11.27), #8 (11.0769), topology (10.15-10.92), and from #10 STATIC_INFO, NV5070 caps, LOCKPINS, PCLK,
HOTPLUG_CONFIG, DP caps. Before AND inside: #1 (11.3557/11.3567 and 12.4957), #5 (10.185 and 11.35-11.37; inside:
the missing `SET_HDMI_ENABLE` + 2 SPD blocks at 12.4965-12.4971), #6 (10.177, the ASSIGN_SOR calls 11.26-11.37;
inside: 12.4937), #9 (11.37-11.40 and 12.4984), #10's `IS_MODE_POSSIBLE` (11.37-11.42 and 12.49). Inside only:
#2 (12.4951). Inside and after (DDI window): #3 (12.4946 and 12.5703). After only: from #10, `CORE_HEAD_STATE`,
`RM_INTR_EN`, `EVT_STAT` (12.5707). New from this alignment: the caps page (P16r, before), the init pushes (P16-
P17, before), the 5th SPD block (P41, after) and the LAST_DATA disable (P43, after).

`[inferred]` The order changes the ranking of §5: the earliest display difference with a measured behavioural
consequence is the caps page / firmware state at driver start (P16r-P17: the hardware KMD programs only windows
0, 2, 4, 6 — the windows the page gives a scaler and TMO — and kf3's guest all eight, with different push sizes).
A KMD that picked a different window set or head state at init can reach the DDI without a usable primary window,
which fits "no window PUT" better than a status refused later. Its falsifier, before any boot: a kf3 caps page
with the hardware's even/odd window caps and head/SOR caps (P16r) still leaves all eight windows initialised and
no window PUT after the modeset. The commit-time and HDMI rows (P31-P33, P41) remain the second candidate.

## 1. What is the same (narrows the search)

- `[measured, VFIO boot3 + run96, RTX 4070, 2026-10-08]` From the modeset to the DDI the guest's RPC STREAMS match in order: `c3720101` ×2, `0x007302a4`,
  `0x00730280`, `0x00730122`, `0x00730128`, `0x0073012c`, `0x20800af1`, `0x0073028b`, `c3720101`, `c3700104`, `0x20800af2`,
  `0x00730282` (0x56 on BOTH), `0x00730280`, `0x007302a4`, `0x20808159`, `0x90f10106`. Only two hardware calls are absent
  under kf3: `DFP_ASSIGN_SOR 0x00731152` (12.4937) and `SPECIFIC_SET_HDMI_ENABLE 0x00730273` (12.4965, enable=0).
- `[measured, boot3 + run96]` The modeset's core pushbuffer is the same size step by step: hardware PUT 0x670→0x980→0xba0→
  0xbc0→0xc60→0xc80→0xca0, kf3 0x0→0x310→0x530→0x550→0x5f0→0x610→0x630 (deltas 0x310, 0x220, 0x20, 0xa0, 0x20, 0x20 on both),
  each with BLANK (`0x680240`←1) after the first and the SPD-infoframe rewrite in `NV_PDISP_SF_USER_0` (`0x6f0228-0x6f024c`)
  before the 4th and 7th. So the KMD's modeset decisions are the same; only the window programming after it is missing.
- `[measured, boot3]` No GSP event is posted between 11.36 and 14.74 s (only a NOCAT record at 11.357): the gate is not a
  missing event.
- `[measured, boot3; code]` Read-backs that are EQUAL: core GET `0x680004` (= PUT on both), ARMED `HEAD_SET_CONTEXT_DMA_OLUT(0)`
  `0x68a288` (hardware 0 at 12.519355/12.520103, before the push that sets it; kf3 also 0 then — run 96 shows the guest's
  `0x2288 = 0xff1fe144` only in the PUT-0x5f0 push AFTER that point; the hardware reads 0xff1fe144 from 12.602867), the ISR
  summaries `0x611ec0`/`0x611c00`/`0x611c30`, the SF_USER infoframe words (a plain shadow on kf3 returns what the guest wrote, as
  the hardware does). ⊘ This falsifies, by reading, the `0x68a288` half of the review's hypothesis (2): it is not a difference
  before the DDI.
- `[code]` `0x20808159` (12.5808): identity on both (332 bytes; hardware reply == request).

## 2. The differences (top 10, ranked by how plausibly they gate the window programming)

| # | where / when (boot3) | hardware `[measured, boot3; boots 1-2 agree; RTX 4070, 2026-10-08]` | kf3 `[measured run 88-96 status, 2026-10-07/08; code body]` | does the KMD plausibly branch on it? |
|---|---|---|---|---|
| 1 | `0x00730128` (unnamed in ogkm-595.84), 20 B, at 11.3557 and 11.3567 and in the commit (12.4957) | NV_OK, reply words `{0, 1, 0xff, 0, 0}` for the all-zero request | `NV_ERR_NOT_SUPPORTED` (3/3 in every run) | **yes, plausibly**: refused inside the commit, with an output the KMD reads; semantics unknown `[inferred]` |
| 2 | `0x00730122` (unnamed), 8 B, in the commit (12.4951) | NV_OK, reply == request `{0, 0x100}` | `NOT_SUPPORTED` (1/1 every run) | plausibly (status only) `[inferred]` |
| 3 | `GET_HDCP_STATE 0x00730280` (12.4946, 12.5703) | NV_OK, flags 0 | `NOT_SUPPORTED` | falsified ALONE (run 95); kept in the batch so the whole commit cluster answers as on hardware |
| 4 | `SYSTEM_GET_HOTPLUG_STATE 0x0073010a` (12.3323, right before the modeset's EDID re-reads) | `hotplugAfterEdidMask = 0x7e00`: the connected `0x100` NOT listed | `0x100`: the connected display listed | plausibly: ogkm defines the mask as "displays that have seen a hotplug after the last valid EDID read" — kf3 says the active monitor is stale `[inferred]` |
| 5 | connector identity of `0x100`: `GET_CONNECTOR_DATA 0x00730250`, `DFP_GET_INFO 0x00731140` (10.185) | flags `PRESENT_YES`, index 3, type `HDMI_A` (0x61), location 3; DFP flags `0x00105300` = HDMI_CAPABLE, RANGE_LIMITED_CAPABLE, YCBCR444_CAPABLE, HDMI_ALLOWED, LINK_SINGLE | `PRESENT_NO`, 0, `DVI_D` (0x31), 0; DFP flags `0x00100000` (LINK_SINGLE only) | **yes, measured consequence**: the kf3 guest never sends `SET_HDMI_ENABLE`, `SET_HDMI_SINK_CAPS`, 28× `DFP_SET_ELD_AUDIO_CAPS`; the real driver runs "DVI mode on an HDMI connector", kf3 a pure DVI-D connector no Ada board has `[inferred]` |
| 6 | `SYSTEM_GET_CAPS_V2 0x00730101` (10.1773) | `capsTbl {0x81, 0x2f}`: AA_FOS_GAMMA_COMP, KSV_SRM_VALIDATION; SINGLE_HEAD_MST, SINGLE_HEAD_DUAL_SST, HDMI_2_0, CROSS_BAR, GLITCHLESS_MODESET | `{0, 0}` | yes, measured consequence: no crossbar → the kf3 guest never sends `DFP_ASSIGN_SOR` (hardware ×8, one inside the commit); GLITCHLESS_MODESET changes the modeset protocol `[inferred]` |
| 7 | `SYSTEM_GET_ACTIVE 0x0073010c` head 0 (11.2607) and the init read-backs | `displayId 0x100` (the firmware lit head 0); the KMD then blanks head 0 (11.261039) | 0 (no head lit until the guest's modeset); no such blank in run 96 | possibly: the KMD inherits a firmware mode on hardware, starts from nothing under kf3 `[inferred]` |
| 8 | core `SET_GET_BLANKING_CTRL(h)` `0x680240+4h`, read once at 11.076930 | `0x2, 0x1, 0x1, 0x1` (UNBLANK for the lit head, BLANK for unlit) | `0, 0, 0, 0` `[code: plain shadow word]` — a value the hardware never returned | possibly (an undefined blank state) `[inferred]` |
| 9 | core ARMED `HEAD_SET_MIN_FRAME_IDLE(h)` `0x68a218+0x400h`: 335 reads 11.37-11.40, one at 12.498437 inside the modeset | `0x10002` every head (leading 2, trailing 1 — NVKMS's default, `nvkms-evo3.c:1433-1438`) | 0 `[code: the ARMED mirror starts at 0; run 96 method trace: the guest never writes 0x2218]` | weakly: the modeset push that follows the 12.498437 read is the same size on both `[measured]` |
| 10 | the rest (each cheap, each `[inferred]` weak): NV5070 `GET_CAPS_V2` `0x24` (DEEP_COLOR_SUPPORT, MULTIWAY_AFR_WAR) vs 0; `OR_GET_INFO(0x100)` ditherType 8_BITS / ditherAlgo 4 / dcbIndex 7 vs 0/0/0; `GET_LOCKPINS_CAPS` numScan/Flip/StereoPins 3/2/4 vs 0; `IS_MODE_POSSIBLE` minImpVPState 6 vs 0, min/floor bandwidth 0 vs 1e6, dispClkKHz 1350000 vs 1000000, worstCaseMargin/Domain set vs 0; `GET_STATIC_INFO` feHwSysCap 0xf0f vs 0xf, bInternalMuxSupported 1 vs 0, numDispChannels 84 vs 81; `GET_PCLK_LIMIT` vbPclkLimit 0 vs 165000; `GET_HOTPLUG_CONFIG` 0x7f00 vs 0 (probe echo); `DP_GET_CAPS` caps vs 0 (probe echo); `CORE_HEAD_STATE(0)` `0x612078` read at 12.5707: SNOOZE (0x100) vs AWAKE (0x200) `[code]`; `RM_INTR_EN_HEAD_TIMING(0)` initial 0x3f0060 vs 0 `[code]`; `EVT_STAT_HEAD_TIMING` 0x7 vs 0x6 (LOADV, falsified run 93) | | | `CORE_HEAD_STATE`: the only ogkm reader (`kernel_head_0400.c:27-40`, `kheadGetDisplayInitialized`) tests `!= 0`, exactly the 0x611d80/0x612078 read pattern at 12.5707 — SNOOZE vs AWAKE cannot branch there `[inferred, source]`. `RM_INTR_EN`: read-modify-written, no branch. |

Topology differences that follow from kf3's ONE connector (7 displays 0x100-0x4000 on hardware vs one): `GET_SUPPORTED` 0x7f00
vs 0x100, `GET_HEAD_ROUTING_MAP` (1960 calls vs 8), the per-display controls for 0x200-0x4000, `GET_CONNECTOR_TABLE` (refused),
`GET_EDID_V2` (another monitor's EDID: Philips 243V5, 384 bytes vs kf3's authored 128-byte 1080p). Recorded, not ranked:
replaying hardware requests for displays kf3 does not have produces refusals no kf3 guest ever sees. The replay's `0x40` for
`EVENT_SET_NOTIFICATION` is a replay artifact (the event allocs are not replayed); the runs answer it `0x0`.

## 3. Register reads on the hardware before the first window PUT `[measured, boot3, RTX 4070, 2026-10-08, 11.274-12.5964 s]`

[full list](hw-boot3-display-access-11.0-12.62.txt.gz) (display range, `0x611ec0` omitted). Every distinct read, with kf3's value:

| register | hardware value (reads) | kf3 | same? |
|---|---|---|---|
| `0x68a218`/`a618`/`aa18`/`ae18` ARMED `HEAD_SET_MIN_FRAME_IDLE(0-3)` | 0x10002 (215+40+40+40; 12.498437 the last) | 0 `[code]` | **no** (#9) |
| `0x68a288` ARMED `HEAD_SET_CONTEXT_DMA_OLUT(0)` | 0 (×2, 12.519) | 0 at that point `[code + run96 order]` | yes |
| `0x680004` core GET | = PUT (0x670, 0xba0) | = PUT `[code]` | yes |
| `0x611d80` `RM_INTR_EN_HEAD_TIMING(0)` | 0x3f0060 (×3) | 0 before the guest's first write `[code]` | no (#10, no branch) |
| `0x612078` `CORE_HEAD_STATE(0)` | 0x100 SNOOZE (×2) | 0x200 AWAKE `[code: Effect::Heads]` | no (#10, no branch per ogkm) |
| `0x611800` `EVT_STAT_HEAD_TIMING(0)` | 0x7 (×2) | 0x6 (0x7 under `KF3_DISPLAY_LOADV`) | no (falsified alone, run 93) |
| `0x611c00` `RM_INTR_STAT_HEAD_TIMING(0)` | 0x2 | 0x2 (`rm=0x2`, run 96 WTRACE) | yes |
| `0x611c30` `RM_INTR_STAT_CTRL_DISP` | 0 | 0 | yes |
| `0x6f0228-0x6f024c` `NV_PDISP_SF_USER_0` (HDMI SPD infoframe "NVIDIA" / "GeForce RTX 4070") | 0, then the words written | the guest's own writes (plain shadow) | yes |

Before 11.274 (driver start, 11.05-11.08): the KMD reads the whole core ARMED area with 8-byte reads (`0x688000-0x68ffff`; most
`0xbadf5040`, the window rows `0x689000+` hold the firmware's window state), every window's ARMED words (`0x690500 = 1`,
`0x690a2c = 0xcf`, …), `GET_RG_SCAN_LINE(0)` = 0x10011 and `SET_GET_BLANKING_CTRL(0-3)` = 2/1/1/1 (#8). kf3 shows no
firmware-lit head in any of these (#7). After the window programming the hardware reads `GET_RG_SCAN_LINE(0)` (12.619553) and
UNBLANKs head 0 (12.619777, `0x680240`←2) — after, so neither can gate the first window PUT.

## 4. Experiments added (all default off; GPU-free tests; hostile-guest bounded)

| flag | crate | what changes | derived from | test |
|---|---|---|---|---|
| `KF3_DISPLAY_PRIVATE_PROBE=1` (H-commit, #1-2) | kf-rm `display_ctrl_probe.rs` | `0x00730122` {0, one display bit} echoed NV_OK; `0x00730128` all-zero 20-byte request → `{0,1,0xff,0,0}` NV_OK; any other shape/size stays refused | the hardware's reply (boots 1-3, identical; RTX 4070, 2026-10-08); unnamed in ogkm, so only the measured shape | `the_private_probe_answers_only_the_measured_shapes` |
| `KF3_DISPLAY_HOTPLUG_EDID_SEEN=1` (H-edidseen, #4) | kf-disp `model.rs`, kf-rm `display.rs` | `hotplugAfterEdidMask` lists a display only until its EDID was answered; a monitor change lists it again | ogkm-595.84 `ctrl0073system.h:503-507` (the field's definition); hardware 0x7e00 | `hotplug_after_edid_lists_a_display_until_its_edid_is_read_only_under_the_experiment` |
| `KF3_DISPLAY_BLANK_STATE=1` (H-blankstate, #8) | kf-qemu `display.rs` | a new core life publishes BLANK (0x1) in every head's `SET_GET_BLANKING_CTRL` | class table (`NVC77D_SET_GET_BLANKING_CTRL`, `_BLANK`, `_BLANK_ENABLE`, newly derived) | `the_core_birth_words_are_the_hardware_read_backs_only_under_the_experiments` |
| `KF3_DISPLAY_ARMED_DEFAULTS=1` (H-armeddefault, #9) | kf-qemu `display.rs` | a new core life's ARMED `HEAD_SET_MIN_FRAME_IDLE(h)` = leading 2 / trailing 1 (0x10002) | class table (`NVC77D_HEAD_SET_MIN_FRAME_IDLE` + fields, newly derived); values `nvkms-evo3.c:1433-1438` | same test |
| (existing) `KF3_DISPLAY_HDCP_STATE=1` (#3) | kf-rm / kf-disp | unchanged | | |

`tools/derive_display_classes.sh` now also derives `SET_GET_BLANKING_CTRL` and `HEAD_SET_MIN_FRAME_IDLE` (both class TSVs
regenerated from the ogkm 580.65.06 / 580.159.04 tags; the diff is purely additive, 40 rows each) and pins `LC_ALL=C` (the
committed TSVs are in C-locale sort order; a UTF-8 locale reorders rows). With the batch flags the replay
([kf replies](kf-replies-boot3-batch-flags.txt.gz), [diff](reply-diff-boot3-batch-flags.txt.gz)) answers `0x00730122`,
`0x00730128` and `0x00730280` byte-for-byte as the hardware did, and `0x0073010a` no longer lists `0x100`.

Not implemented (cost, next if the batch is falsified): **H-hdmi** (#5: present the connector as HDMI_A in DVI mode — connector
data, DFP flags, and answers for `SET_HDMI_ENABLE`, `SET_HDMI_SINK_CAPS`, `GET_HDMI_GPU_CAPS`, `DFP_SET_ELD_AUDIO_CAPS`); **H-xbar**
(#6: the CROSS_BAR cap plus a `DFP_ASSIGN_SOR` answer); **H-inherit** (#7: a firmware-lit head 0 in `GET_ACTIVE`, the core/window
ARMED state and the blanking read-back). The 3-4 ms DDI-internal wait (review hypothesis 3) is not addressed here.

## 5. Ranking with falsifiers (stated before any boot)

1. **H-commit** (#1-3): a refusal inside the modeset commit leaves the KMD's path state incomplete, so it never programs the
   primary. *Falsifier:* with `KF3_DISPLAY_PRIVATE_PROBE` + `KF3_DISPLAY_HDCP_STATE` (all three commit-time controls answered as
   on hardware) the display PUT count stays 39 through the TDR.
2. **H-hdmi** (#5) — not implemented. *Falsifier:* with the connector presented as HDMI_A in DVI mode (and the guest sending
   `SET_HDMI_ENABLE`), no window PUT.
3. **H-edidseen** (#4). *Falsifier:* `0x0073010a` answered without `0x100` (log line + replay) and no window PUT.
4. **H-xbar** (#6) — not implemented. *Falsifier:* `DFP_ASSIGN_SOR` sent and answered, no window PUT.
5. **H-blankstate** (#8). *Falsifier:* BLANK published at core birth (log line), no window PUT.
6. **H-inherit** (#7) — not implemented (high cost).
7. **H-armeddefault** (#9). *Falsifier:* 0x10002 published at core birth, no window PUT.
8. The #10 group — weak; `CORE_HEAD_STATE` and `RM_INTR_EN` are argued away by ogkm source (not by a run).

## 6. The next hardware boot (one boot, batched per CLAUDE.md)

Binary: this branch's revision through `build_kf3.sh`. Flags: run 96's set (incl. `KF3_DISPLAY_WRITE_TRACE=1`,
`KF3_DISPLAY_METHOD_TRACE=1`, `KF3_RPC_TRACE=1`, `KF3_DISPLAY_CTRL_PROBE=1`, `KF3_DISPLAY_IMP_ENABLE=1`,
`KF3_DISPLAY_CORE_AT_VBLANK=1`) **plus** `KF3_DISPLAY_HDCP_STATE=1 KF3_DISPLAY_PRIVATE_PROBE=1 KF3_DISPLAY_HOTPLUG_EDID_SEEN=1
KF3_DISPLAY_BLANK_STATE=1 KF3_DISPLAY_ARMED_DEFAULTS=1 KF3_DISPLAY_LOADV=1` (LOADV and CORE_AT_VBLANK were falsified alone; on
together they make every listed read-back hardware-like at once).

Check first that every flag took effect (the variable reached the guest — run 94's lesson): the log lines `EXPERIMENT
KF3_DISPLAY_HOTPLUG_EDID_SEEN=1`, `KF3_DISPLAY_BLANK_STATE=1 … (4 words)`, `KF3_DISPLAY_ARMED_DEFAULTS=1 … (4 words)`, `PROBE
KF3_DISPLAY_PRIVATE_PROBE … 0x00730122` / `0x00730128` (`result=0x0` in the rpc-trace for both, `0x00730280` too).

Expected observation if one of them is the gate: after the modeset's last core PUT, a WTRACE `WRITE 0x690000 <- … Put(1)`
(window 0's PUT) followed by `TRACE window 0 latched`, `disp[… puts=N …]` with N > 39 at the stall marker (or no stall marker at
all), and LAST_DATA staying enabled past the first VSync (hardware: on 12.5708 → 14.786). Falsifier for the whole batch: puts
stays 39 with every flag confirmed; then H-hdmi and H-xbar (§4, not implemented) are next, before the read-trap owner question
of the flip-vsync record §9. If puts > 39: bisect in two boots (commit cluster {HDCP, PRIVATE} vs {EDID_SEEN, BLANK_STATE,
ARMED_DEFAULTS, LOADV}).

## 7. Run 97 (binary `kf3-bins/34a63c90`, the §6 batch): the batch is FALSIFIED

`[measured, run97 at 34a63c90, RTX 4070, 2026-10-09]` files `run97-*` (harness [drd-run.sh](drd-run.sh) = the flip record's
`flip-run.sh` with this checkout; `flock -o /tmp/kayfabe-fastguest.lock` held from the IOMMU switch 00:23:05 to the DMA-FQ
restore 00:24:02; `0000:01:00.0` back on `nvidia`, no new Xid). Every flag took effect: the log carries the `EXPERIMENT` /
`PROBE` line of each of the eight (`KF3_DISPLAY_WRITE_TRACE`: 178 `WTRACE` lines), and the rpc-trace shows `0x00730280`,
`0x00730122`, `0x00730128` `result=0x0` (were refused in runs 88-96). One flag changed the guest's behaviour where §0
predicted it would: after `SYSTEM_GET_HOTPLUG_STATE` the guest now reads the EDID **5** times (run 96: 6; hardware: 5, row
P28b) — the guest acts on `hotplugAfterEdidMask`. Everything else is as in run 96: the driver-start window init is unchanged
(all eight windows, PUTs 0x5d0 / 0xed0 — row P17), the modeset's core PUTs are the same, the guest disables LAST_DATA 0.26 ms
after the first VSync, and **`puts=39` at the stall marker** (15.5 s), 73 after the TDR. Falsified: H-commit (all three
commit-time controls answered as on hardware), H-edidseen, H-blankstate, H-armeddefault, together with LOADV and
CORE_AT_VBLANK.

## 8. The next boot (run 98): H-hdmi + H-xbar (coordinator's next two) + H-caps (§0's earliest candidate)

Three more default-off probes, each answering with the real GPU's measured reply (VFIO DVI reference boot3, RTX 4070,
2026-10-08) for the measured shape only (GPU-free tests:
`display_ctrl_probe::tests::the_topology_probes_answer_the_measured_requests_byte_for_byte`,
`display::tests::the_caps_probe_page_is_the_measured_one_for_its_class_only`):
- `KF3_DISPLAY_HDMI_PROBE` (H-hdmi, rows P9-P10, P24-P26, P32-P33, P41): `GET_CONNECTOR_DATA(0x100)` present / `HDMI_A`,
  `DFP_GET_INFO(0x100)` flags `0x00105300`, and the HDMI path (`SET_HDMI_ENABLE`, `SET_HDMI_SINK_CAPS`, `GET_HDMI_GPU_CAPS`,
  `GET_HDMI_SCDC_DATA` with its status `0x14`, `DFP_SET_ELD_AUDIO_CAPS`).
- `KF3_DISPLAY_XBAR_PROBE` (H-xbar, rows P6, P22, P26, P30): `SYSTEM_GET_CAPS_V2` `{0x81, 0x2f}`; `DFP_ASSIGN_SOR` answered
  from kf3's own topology (display `0x100 << i` on SOR `i`, SINGLE — byte-identical to the hardware's answer for `0x100`).
- `KF3_DISPLAY_CAPS_PROBE` (H-caps, row P16r): the real GPU's caps page (99 non-zero words) for Ada's C773 page only. A
  captured table — a probe, never a design.

Flags: run 97's set plus the three (binary: this branch's tip, 2026-10-09). Falsifiers, stated before the boot: **H-caps** — with the measured page the guest still
initialises all eight windows at driver start (row P17 unchanged); **H-hdmi** — the guest sends `SET_HDMI_ENABLE` /
`DFP_SET_ELD_AUDIO_CAPS` (the path is taken) and still writes no window PUT after the modeset; **H-xbar** — `DFP_ASSIGN_SOR`
is sent and answered and still no window PUT. Whole batch: `puts=39` at the stall marker with all three confirmed. If
`puts > 39`: one bisect boot (time-box: 3 boots), H-caps alone vs H-hdmi + H-xbar.

## 9. Run 98 (binary `kf3-bins/1048afc7`, §8's batch): the window programming happens — `puts=63` at the marker

`[measured, run98 at 1048afc7, RTX 4070, 2026-10-09]` files `run98-*`. All eleven flags confirmed (log lines; the HDMI/xbar
probe answered `0x00730101` ×2, `0x00730250` ×5, `0x00731140` ×4, `0x00731144` ×4, `0x00730273`, `0x00730293`, `0x007302a2`,
`0x007302a6`, `0x00731152`; `CAPS_PROBE … 99 words`). Against run 97, measured in the WTRACE:
- **driver start (row P17) now matches the hardware's shape**: windows 0/2/4/6 only (PUTs 0x310 → 0x8e0 → 0x9f0 → 0 → 0x7f0;
  hardware 0x320 → 0x8f0 → 0xa00 → 0 → 0x7f0), the odd windows untouched — H-caps' falsifier is not met;
- after the modeset (core PUT 0x630) and the first VSync, `0x90f10106` is followed by **core PUT 0x680 + window 0 PUT 0x800**
  (271314.546 s) — the hardware's order at 12.594-12.596 (`0x90f10106`, core PUT 0xcf0, window 0 PUT 0x870) — then more
  window 0 flips (0x8f0-0x950, 0x9b0-0xa00, 0xa50-0xa80, 0xae0-0xaf0), core updates every 2 s, and 921 VSyncs (frame 1072);
- the harness's stall marker (`0x00730108` after the first Passthrough birth) fired at 15.5 s with `puts=63`, but here it is
  a client's GET_CONNECT_STATE during normal display activity; the harness then quit the guest (30 s later, `puts=100`), so
  no QGA probe ran. ~20 s after driver start (271333.08) the guest's RM re-initialised the display from scratch (core and
  window init sequence, Translated kernel channels reborn) — a driver reset, cause not identified in this log. No host Xid
  during the boot (the Xid 31 lines at 00:25 belong to another agent's boot between runs 97 and 98).

`[inferred]` H-hdmi + H-xbar + H-caps together are sufficient for the primary-window programming; which one is necessary is
open. The init-window change is attributable to the caps page alone (nothing else in the batch touches what the KMD reads
before 11.07 s on hardware), which makes H-caps the first candidate.

## 10. Boot 3 of the time-box (run 99): bisect — H-caps alone — plus the D3D probes

Flags: run 97's set + `KF3_DISPLAY_CAPS_PROBE` (no HDMI, no xbar probe), same binary (`1048afc7`; later commits on this
branch change no code). Harness [drd-run2.sh](drd-run2.sh): the marker no longer ends the boot; 40 s after it a screendump,
then the QGA probes (D3D11 clear + read-back, D3D12 fence, monitor info + `nvidia-smi`, the user-session probe), then a
clean stop. Prediction (H-caps sufficient): driver-start init on windows 0/2/4/6 and a window 0 PUT after the modeset.
Falsifier: no window PUT after the modeset (`puts=39` at the marker) — then H-hdmi and/or H-xbar is necessary.
