# V3: constants no ogkm tag defines (the Windows-capture list)

**STATUS: LIVE, 2026-10-13 (derived mechanically from `tier_checks/merged_classification.tsv`; 40 of 140 inventory rows; review the grouping).** Related: `V3_CONSTANT_STRATEGY.md` §2 (tiers), §4 step 1-3, `V3_CONSTANT_LISTS_20261011.md`.

## What this is

Owner, 2026-10-13: whatever ogkm defines is already known, so measurement only makes sense for GSP/RM traffic ogkm does not expose but that is reachable. These are the inventory rows with `ogkm = NOT_IN_ANY_VERSION`: no layout and no semantics in any of the 221 ogkm tags checked (515.43.04..615.78.08). Measured: the classification TSV. Inferred, not measured: that the open Linux driver never issues them (it cannot, they are not in its source) and that the closed Linux driver might; so "Windows-only" here means "observed only from the Windows 580.88 driver, absent from the open source".

All rows were observed on ONE die (RTX 4070 / AD104), ONE Windows driver (580.88) and ONE monitor. No axis was varied, so nothing below is a per-die or per-driver fact yet.

## The 40 rows


### A. NV2080 / NV2081 / NV0080 controls (GPU, GSS-legacy and perf families)

| id | what (inventory name) | kind | tier class |
|---|---|---|---|
| `const-echo-730122` | NV0073 0x00730122 (undefined in ogkm 595.84 (other versions not yet checked)): echo of request {0, displayId} | control-id+layout | 3-MEASURE-SWEEP |
| `const-get-display-mode-73011a` | NV0073 0x0073011a GET_DISPLAY_MODE: id, 28 B layout {displayId, active w, active h, total w, total h, 0, refresh Hz} | control-id+layout | 2-MEASURE-LAYOUT |
| `const-gssreplay-rows` | GSS-legacy host-replay rows: request layouts for 0x20809064, 0x2080a028, 0x2080a026, 0x2080a084 (single-domain, input words named) and the encoder-ses | control-id+layout | 2-MEASURE-LAYOUT |
| `const-reply-730128` | NV0073 0x00730128 (undefined in ogkm 595.84 (other versions not yet checked)): reply words {0, 1, 0xff, 0, 0} for an all-zero 20 B request | reply-values | 3-MEASURE-SWEEP |
| `ctl-00809908` | 0x00809908 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-20801111` | 0x20801111 undefined in ogkm 595.84 (other versions not yet checked) (software-runlist submit, 40 B) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-20802806` | 0x20802806 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 3-MEASURE-SWEEP |
| `ctl-20808524` | 0x20808524 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-2080852a` | 0x2080852a undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 1-OBTAINABLE |
| `ctl-2080852e` | 0x2080852e undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 1-OBTAINABLE |
| `ctl-2080852f` | 0x2080852f undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 1-OBTAINABLE |
| `ctl-2080853a` | 0x2080853a undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 1-OBTAINABLE |
| `ctl-2080853b` | 0x2080853b undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-20809001` | 0x20809001 undefined in ogkm 595.84 (other versions not yet checked) (GSS-legacy) | reply-values | 2-MEASURE-LAYOUT |
| `ctl-20809004` | 0x20809004 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-20809009` | 0x20809009 undefined in ogkm 595.84 (other versions not yet checked) (GSS-legacy) | reply-values | 1-OBTAINABLE |
| `ctl-20809064` | 0x20809064 undefined in ogkm 595.84 (other versions not yet checked) (GSS-legacy clock-domain listing, 520 B) | reply-values | 1-OBTAINABLE |
| `ctl-2080a02f` | 0x2080a02f undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-2080a060` | 0x2080a060 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 1-OBTAINABLE |
| `ctl-2080a088` | 0x2080a088 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-2080a618` | 0x2080a618 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 1-OBTAINABLE |
| `ctl-2080a619` | 0x2080a619 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-2080a630` | 0x2080a630 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-2080a70a` | 0x2080a70a undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-2080a801` | 0x2080a801 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-2080b201` | 0x2080b201 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 2-MEASURE-LAYOUT |
| `ctl-20810108` | 0x20810108 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 1-OBTAINABLE |
| `ctl-2081010d` | 0x2081010d undefined in ogkm 595.84 (other versions not yet checked) | reply-values | 1-OBTAINABLE |

### B. NV0073 (display common) controls

| id | what (inventory name) | kind | tier class |
|---|---|---|---|
| `ctl-00730122` | 0x00730122 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 3-MEASURE-SWEEP |
| `ctl-00730128` | 0x00730128 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 3-MEASURE-SWEEP |
| `ctl-007302a3` | 0x007302a3 undefined in ogkm 595.84 (other versions not yet checked) (12 B) | reply-values | 3-MEASURE-SWEEP |
| `ctl-007302a5` | 0x007302a5 undefined in ogkm 595.84 (other versions not yet checked) | control-id+layout | 3-MEASURE-SWEEP |

### C. kayfabe's host-side asks (legacy GSS clock queries): the host-side layout at 595.91.07 is unmeasured

| id | what (inventory name) | kind | tier class |
|---|---|---|---|
| `hostask-20809064` | host-side ask 0x20809064 GSS-legacy clock-domain listing (undefined in ogkm 595.84 (other versions not yet checked)) | control-id+layout | 1-OBTAINABLE |
| `hostask-2080a026` | host-side ask 0x2080a026 GSS-legacy cudart max-clock query (undefined in ogkm 595.84 (other versions not yet checked)) | control-id+layout | 2-MEASURE-LAYOUT |
| `hostask-2080a028` | host-side ask 0x2080a028 GSS-legacy per-domain clock query (undefined in ogkm 595.84 (other versions not yet checked)) | control-id+layout | 2-MEASURE-LAYOUT |
| `hostask-2080a084` | host-side ask 0x2080a084 GSS-legacy cudart clock query (undefined in ogkm 595.84 (other versions not yet checked)) | control-id+layout | 1-OBTAINABLE |

### D. Behaviour and timing rows (not an RM control: a queue capture module cannot measure them)

| id | what (inventory name) | kind | tier class |
|---|---|---|---|
| `const-core-latch-vblank` | Core update latch timing: an update group including the CORE latches at the next vblank of an active head (one PUT per frame) | timing | 5-RE-CANDIDATE |
| `const-evt-stat-loadv` | NV_PDISP_FE_EVT_STAT_HEAD_TIMING bit 0 (LOADV), hand-named; real reads 0x7 at every head-timing ISR, kf3 publishes 0x6 | bit-set | 4-MEASURE-NEEDS-DISPLAY |
| `const-head-timing-frame-edge` | Head-timing events LAST_DATA/VBLANK pending each frame edge whether enabled or not; first interrupt 0.58-16.87 ms after enable | timing | 5-RE-CANDIDATE |
| `const-sw-runlist-object` | Admission of the Windows software-runlist object (class 0xb297) as a graph-only object: maxTSGs=0, QoS=0, advertised engine, fn-1 Windows 580.88 ident | other | 5-RE-CANDIDATE |

## What a Windows capture module would collect

- **Requests** (layout, size, parameter values the driver chooses) and **replies** (status plus bytes) for exactly these ids, per (die, Windows driver branch). A request census may also find ids not on this list: add them with the same fields.
- Request layouts learned on one die can be **replayed** by a Linux tier-3 module on other dies, but only if the Linux raw-GSP spike works (`V3_CONSTANT_STRATEGY.md` §4 step 1, still unrun). Without it the Windows capture runs on every rented die.
- **Group C** (host-side asks): kayfabe refuses them at host 595.91.07 because no layout is measured (`HOST-ABI REFUSED` lines). A captured guest-side layout can be tried against the host's unprivileged ioctl on the home host (tier 2) before any rental.
- **Group D** is behaviour and timing of the display engine, not RM traffic. It needs a head (home display machines, VFIO timing captures, or the forced-connector experiment), not this module.
- Never collected: UUIDs, serials, VBIOS, MACs or other board data.

## Falsifiers

- The module's decode of the GSP message queues must match a VFIO observer capture of the same boot byte for byte on the home 4070, or it is not used.
- A row stays "unmeasured" until two measurements on the same die and driver agree; a row that differs between dies is recorded per die (no "same family" rule without a mechanism, §3).

