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
