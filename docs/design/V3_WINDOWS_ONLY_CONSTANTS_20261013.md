# V3: constants no ogkm tag defines (the Windows-capture list)

**STATUS: LIVE, 2026-10-13 (derived mechanically from `tier_checks/merged_classification.tsv`; 40 of 140 inventory rows; review the grouping).** Related: `V3_CONSTANT_STRATEGY.md` §2 (tiers), §4 step 1-3, `V3_CONSTANT_LISTS_20261011.md`.

> **⊘ CORRECTION 2026-10-13 (owner; verified in the host's own module source).** RM's administrator check is `capable(CAP_SYS_ADMIN)`, not uid 0:
> `os_is_administrator()` returns `NV_IS_SUSER()` (`/usr/src/nvidia-595.91.07/nvidia/os-interface.c:382`), defined as `capable(CAP_SYS_ADMIN)`
> (`common/inc/nv-linux.h:502`); the two other access rights map to `CAP_PERFMON` and `CAP_SYS_NICE` (`os-interface.c:385-400`). A default Docker container
> (root without `CAP_SYS_ADMIN`) is therefore **unprivileged** to RM, exactly like kayfabe at runtime. So wherever this page says a measurement is "feasible
> on a headless rental" as a *privileged* measurement, that holds only on a KVM rental or bare metal with real `CAP_SYS_ADMIN` (read each rental's `CapEff`
> first); on a container rental it is a tier-2 measurement and kayfabe can obtain the same value itself at runtime (no table, no sweep). The earlier
> wording came from a desk check and wrongly assumed container root was RM admin. Rows marked `PRIVILEGED`/`KERNEL_PRIVILEGED`/`INTERNAL` stay tier 3 (or tier 4),
> because kayfabe cannot obtain them either way.

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


## How each row is obtainable (the constants ruling, §AF/§AH, tiers in `V3_CONSTANT_STRATEGY.md` §2)

Tier order: **(1)** derive or compute (ogkm, nouveau/nova, class tables, kayfabe's own state); **(2)** unprivileged host ioctl at startup; **(3)** privileged measurement once per die and driver (sweep, kernel module, hardware access); **(4)** reverse engineering, private and last. The inventory classes (`V3_CONSTANT_LISTS_20261011.md`) map onto them:

| class | meaning | route | rows in this list |
|---|---|---|---|
| ALLOWED-FAMILY | hand-kept family rows (policy) | none needed | 0 |
| 1-OBTAINABLE | layout in open source or trivial, value derived, computed or answered by the host | tiers 1-2 (delegate to the host where safe) | 12 |
| 2-MEASURE-LAYOUT | the value looks host-served, but the layout or id is in no open source, known from one observation | tier 3: capture the request/reply layout once, then one host probe may move it to class 1 | 17 |
| 3-MEASURE-SWEEP | not derivable, not host-served; a privileged probe is feasible on a headless rental and the reply is not display-state dependent | tier 3 per die and driver | 7 |
| 4-MEASURE-NEEDS-DISPLAY | feasible only with a lit head, or authored by kayfabe from its own state | display machine or forced connector; kayfabe authors it | 1 |
| 5-RE-CANDIDATE | fails every tier above (timing/behaviour or never observed) | first the display reference machine, reverse engineering last | 3 |

Caveat on the evidence: the notes below come from the inventory (run 641, one die, one Windows driver, five VFIO captures); `M:` = measured there, `I:` = inferred. "Used for" states what the inventory records; where it records nothing, the purpose is **not known** (that is why the row is on the list).

## What each row is for

### A. NV2080 / NV2081 / NV0080 controls the Windows driver issues
Most are "GSS-legacy" controls (command bit 15 set): they have no header in any open source. The real GPU answers them; kayfabe answers some from rows captured earlier on a GA106 (RTX 3060, 580.159.04, 2026-08-20) and refuses the rest with status `0x56`.

| id | class | what it is, and what the guest uses it for | measured facts |
|---|---|---|---|
| `0x2080852a` | 1 | GSS-legacy, purpose not known | real: 24 calls, OK, 1784 B reply |
| `0x2080852e` | 1 | GSS-legacy, purpose not known | real: 19 calls, OK, 528 B |
| `0x2080852f` | 1 | GSS-legacy, purpose not known | real: 4 calls, OK, 776 B |
| `0x2080853a` | 1 | GSS-legacy, purpose not known | real: 5 calls, OK, 1456 B |
| `0x20809009` | 1 | GSS-legacy, CUDA-runtime init row; kayfabe serves a captured 8 B row | served OK once |
| `0x20809064` | 1 | clock-domain listing (520 B); the captured row names the wrong clock domain for the encoder library | served OK 86 times from the captured row |
| `0x2080a060` | 1 | GSS-legacy, purpose not known | real: 5 calls, OK, 28 B |
| `0x2080a618` | 1 | GSS-legacy, purpose not known | real: 24 calls, OK, 8620 B (the largest) |
| `0x20810108` | 1 | NV2081 (binary-API class) control, purpose not known | real: 16 calls, OK, 992 B |
| `0x2081010d` | 1 | needed for Windows `StartDevice`: bisect runs 38-41 found this control alone moves it; kayfabe answers OK with zero-filled params | layout assumed from a closed-binary export row (owner ruling S, to confirm) |
| `0x00809908` | 2 | NV0080 object control (class 0x9908 assumed), purpose not known | real: 70 calls, OK, 16 B |
| `0x20801111` | 2 | software-runlist submit (40 B), tied to the Windows software-runlist object (class 0xb297); kayfabe echoes it and schedules nothing | 152 calls OK in run 641 |
| `0x20808524` | 2 | GSS-legacy, purpose not known | real: 5 calls, OK, 4 B |
| `0x2080853b` | 2 | GSS-legacy, purpose not known | never issued in the 5 real captures, 11 times in run 641 |
| `0x20809001` | 2 | GSS-legacy, CUDA-runtime init row; captured 8 B row | served OK once |
| `0x20809004` | 2 | GSS-legacy, purpose not known; by far the most frequent | real: 640 calls, OK, 1544 B; 86 calls refused in run 641 |
| `0x2080a02f` | 2 | GSS-legacy, purpose not known | the real GPU itself answers `0x56` (5 calls, 1 B) |
| `0x2080a088` | 2 | GSS-legacy, purpose not known | real: 10 calls, OK, 12 B |
| `0x2080a619` | 2 | GSS-legacy, purpose not known | never issued in the 5 real captures, 11 times in run 641 |
| `0x2080a630` | 2 | GSS-legacy, purpose not known | real: 5 calls, OK, 1160 B |
| `0x2080a70a` | 2 | GSS-legacy, purpose not known | real: 5 calls, OK, 26 B |
| `0x2080a801` | 2 | GSS-legacy, purpose not known | real: 10 calls, OK, 1028 B |
| `0x2080b201` | 2 | GSS-legacy, purpose not known | real: 10 calls, OK, 2188 B; 41 calls refused in run 641 |
| `0x20802806` | 3 | purpose not known | real: 5 calls, OK, 4 B |

### B. NV0073 display-common controls
| id | class | what it is, and what the guest uses it for | measured facts |
|---|---|---|---|
| `0x00730122` | 3 | per-display query; the reply is an 8 B echo of the request `{0, displayId}` | 7 real calls; replies byte-identical to the request in 3 boots; unknown what the firmware writes for other requests |
| `0x00730128` | 3 | display query; reply `{0, 1, 0xff, 0, 0}` for an all-zero request; the `0xff` word may be a head or display mask (inferred) | 17 real calls, 20 B; a captured constant |
| `0x007302a3` | 3 | display query (12 B); kayfabe echoes the request | equal to the real reply in 5 captures; an echo may hide a field |
| `0x007302a5` | 3 | display query (16 B reply); issued very often | 157 real calls; 45 calls refused in run 641 vs 28 in one real boot |
| `0x0073011a` | 2 | GET_DISPLAY_MODE: 28 B `{displayId, active w/h, total w/h, 0, refresh Hz}` (field names inferred); used for the current mode of a head | 5 of 5 calls OK; the app/ICD decides whether it is issued |

### C. kayfabe's own host-side asks (legacy GSS clock queries)
kayfabe asks the real host GPU these at start-up so it can answer the guest's CUDA/encoder clock queries truthfully. It currently refuses its own asks at host driver 595.91.07 because the layouts were only measured on 580.159.04 hosts (`HOST-ABI REFUSED` lines).
| id | class | purpose |
|---|---|---|
| `host 0x20809064` | 1 | clock-domain listing |
| `host 0x2080a084` | 1 | CUDA-runtime clock query |
| `host 0x2080a026` | 2 | CUDA-runtime max-clock query |
| `host 0x2080a028` | 2 | per-domain clock query |
| `GSS replay rows` | 2 | the four request layouts above plus the encoder-session pair `0x20808163/0x20808164` (4 B), hand-typed from a host probe on 2026-09-26 |

### D. Behaviour and timing rows (not RM controls)
| id | class | what it is | measured facts |
|---|---|---|---|
| `NV_PDISP_FE_EVT_STAT_HEAD_TIMING` bit 0 (LOADV) | 4 | a display-engine status bit the driver polls in its head-timing interrupt | the real GPU reads `0x7` every time; kayfabe publishes `0x6` |
| core update latch timing | 5 | when a core display update takes effect | hardware: three updates one frame apart; kayfabe: within 3 ms (run 93) |
| head-timing frame edge | 5 | when the head-timing events fire each frame | first interrupt 0.58-16.87 ms after enable |
| software-runlist object admission (class `0xb297`) | 5 | how kayfabe admits the Windows software-runlist object (graph-only, `maxTSGs=0`) | constants quoted from the flag inventory, not re-read |
