# KF3 flags and installation defaults, 2026-10-22

**STATUS: RESEARCH, 2026-10-22.** D1 audit of master `e64b8e5f6d0f64c41c9c4d40775e74be08de7164`.
CODE READ unless a paragraph says MEASURED or INFERRED. Hardware validation is outside this lane.
This dated inventory supplements the historical [inventory](V3_FLAG_INVENTORY.md); its older default/reader claims must be checked against this revision.

## Evidence and falsifiers (recorded before changes)

- Hypothesis H1: production needs launcher overrides that master does not default on. FALSIFIER: the reader defaults to the launcher value, or the launcher does not set the name. Result: mixed; lifecycle/Pointer, lazy observe 16 and put cache already default on; the SDR colour trio defaults off.
- Hypothesis H2: the lifecycle counter knobs still control master. FALSIFIER: no environment reader exists anywhere in the product. Result: falsified; `KF3_LIFECYCLE_COUNT` and `KF3_LIFECYCLE_PERIOD_S` occur only in a launcher.
- Hypothesis H3: the production mirror-off setting can be promoted. FALSIFIER: master has no reader, or executable launcher assignments differ from the comment. Result: falsified on both: master has no TSG mirror switch and the launcher comment is not an executed setting. No mirror code will be added.
- Hypothesis H4: the source-derived colour settings can use the production defaults without rewriting the display algorithm. FALSIFIER: enabling them requires a new algorithm, a captured page, a new driver/family assumption, or a missing dependency. Check the existing decoder and its GPU-free oracles before changing defaults; hardware compatibility of new default combinations remains unverified.

MEASURED (read-only file retrieval, not a VM run): the owner's perf6 launcher and the script it invokes were read on 2026-10-22. SHA-256, respectively:
`761013153cafd88b49971932471cbf58758dc13bd0deda2aa601cc33dbbdc703`,
`43bed8abda37a8116f8f6be83795bed57b70ca9d99b5b7e14d292650df305b64`.
Private filenames and machine paths are omitted from public evidence. D1's private report records them for the owner.

CODE READ: perf6 clears inherited `KF3_*`, selects revision `0d7ba743`, and calls a production copy of `windows_broker.sh`.
Its extra flags default to `KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5`; it drops `KF3_TIMER_MAP`.
The executable settings do **not** include the `KF3_TSG_MIRROR=0` mentioned in its header.
The invoked script sets these names to 1:

```
KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_WIN_TWIN_DEFAPI_OBJECT
KF3_KERNEL_GR_CE KF3_KERNEL_GR_WORK KF3_TRANSLATED_CE_RELAY KF3_DEFERRED_API
KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE
KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_ASYNC_PREEMPT
KF3_GFX_POOL_PROBE KF3_SW_RUNLIST_PROBE KF3_MEMORY_LIST_PROBE
KF3_DISPLAY_SDR_COLOR KF3_DISPLAY_CAPS_PROBE
KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256
```

CODE READ: the stock in-repo Windows launcher differs: it includes diagnostic tracing in its default `FLAGS`. It must not be advertised as the minimal user launcher.

## Design before implementation

Binding: [OWNER_RULINGS](../OWNER_RULINGS.md) §§A, S, U, W, AB, AI and [THE_CONSTRAINTS](THE_CONSTRAINTS.md) §42.
Only existing values proven by the executable production launcher are candidates. No relay, VA or locking algorithm changes; no timer/poll changes.

| Decision | Rationale and validation gate |
|---|---|
| Keep lifecycle, Pointer transition, lazy observe 16 and put cache as implemented | Already built-in; retain `KF3_LIFECYCLE_MODE=always_scan`, `KF3_PUT_CACHE=0`, `KF3_LAZY_OBSERVE=0` research arms. Counter-period knobs have no reader; do not invent one. |
| Candidate: default `KF3_DISPLAY_SDR_COLOR`, `KF3_DISPLAY_LUT_MIRROR`, `KF3_DISPLAY_ILUT_OFFSET_256` on | Executable production values; existing source-derived colour decoder only. Preserve each `=0` arm. Test real colour plans with absent environment; two independent mutation killers per change. Constructor-only probes must explicitly select `KF3_DISPLAY_SDR_COLOR=0`. |
| Stop promotion of `KF3_DISPLAY_CAPS_PROBE`, `KF3_GFX_POOL_PROBE`, reply/bind probes | Captured page, fabricated sizing or status-only replies are not justified by a default flip; derivation/owner decisions belong to another lane. These remain opt-in. |
| Stop promotion of `KF3_SW_RUNLIST_HOST_OWNED` | §W explicitly says off and undecided; executable launcher is not a replacement owner ruling. |
| Park kernel GR pair, Windows USER routing/VA floor, deferred-API object/trigger, completion relay and async-preempt defaults | These are a dependent channel configuration (§42), with unresolved runlist/probe dependencies and Linux scope. Partial promotion cannot establish a bootable default. Retain existing overrides, list the exact gap. |
| Remove only dead launcher assignments, if verified | Remove the dead lifecycle extras from `scripts/bench/hitch/launch_h.sh`; preserve caller-provided `WIN_FLAGS`. Do not alter historical evidence or host files. |
| Keep measurement/debug/control flags optional and default off | §AB requires retaining them. Do not rename diagnostic flags, hardwire fault injection, or add a global registry in this lane. |
| Keep QOM property defaults pending individual omission checks | Resource size, guest ABI, OS and broker socket are user inputs. A Windows production value does not prove a Linux/default display property change. |

INFERRED: a completely flag-free Windows installation cannot be honestly claimed while the production launch still uses the blocked probe/runlist group. Options are (1) document a research launch and its limits (recommended for D1), or (2) derive and validate that group in a separate authorized change. D1 will not silently promote it.

## Full inventory

Inventory and omission results are being reconciled against every literal reader and launcher reference; the final rows below will include defaults, class, source, launcher setting and necessity. This checkpoint preserves the design before implementation.
