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
| Default the main Windows launcher's `KF3_RPC_TRACE`, `KF3_BAR0_TRACE`, `KF3_MAPLOG`, `KF3_DISPLAY_METHOD_TRACE` off | H5, FALSIFIER before tests: any switch controls a decision, or the executable production copy enables it. CODE READ result: logging/read-trap diagnostics only; production copy omits all four. Remove default assignments, retain explicit `WIN_FLAGS=NAME=1` research/diagnostic arms. Two independent launcher-contract tests must kill each reintroduced default. Remove its dead `KF3_TIMER_MAP` token too. |
| Keep other measurement/debug/control flags optional and default off | §AB requires retaining them. Do not rename diagnostic flags, hardwire fault injection, or add a global registry in this lane. |
| Keep QOM property defaults pending individual omission checks | Resource size, guest ABI, OS and broker socket are user inputs. A Windows production value does not prove a Linux/default display property change. |

INFERRED: a completely flag-free Windows installation cannot be honestly claimed while the production launch still uses the blocked probe/runlist group. Options are (1) document a research launch and its limits (recommended for D1), or (2) derive and validate that group in a separate authorized change. D1 will not silently promote it.

## Full inventory

CODE READ: the default column records **base master before D1**; candidate changes are named explicitly and the result section records what landed. Class describes semantics, independent of a misleading PROBE/DIAG name. No current-revision hardware ablation was performed: “historical gate” is INFERRED current necessity from [the dated inventory](V3_FLAG_INVENTORY.md) and [ablation evidence](../../traces/windows_flag_ablation_20261010/README.md), not a new REQUIRED-TO-BOOT measurement. No live flag is asserted REQUIRED-TO-BOOT on this revision. “yes 1” means the executable perf6 call chain sets it, not that it is safe to promote. “no” means no assignment after clearing inherited KF3 variables.

| Environment name | Base default / parsing | Class | Effect | Production sets | Production needs it? | Reader (base revision) |
|---|---|---|---|---|---|---|
| `KF3_ASYNC_PREEMPT` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Real async disable/preempt and host completion relay | yes 1 | unknown today; real-preempt path in production | `crates/kf-disp/src/model.rs:558`, `crates/kf-rm/src/chanlink.rs:262`, `crates/kf-rm/src/display.rs:751` |
| `KF3_ASYNC_PREEMPT_REENABLE_NOACT` | off; only 1 enables | DEBUG | Answer re-enable OK without a host act (diagnostic control) | no | no; keep off | `crates/kf-qemu/src/chan.rs:1812` |
| `KF3_BAR0_READ_TRACE` | off; 1/on enables | MEASUREMENT | Explicit BAR0 read-exit tracing exception; malformed enabled config refuses | no | no | `crates/kf-qemu/src/device.rs:338`, `crates/kf-qemu/src/ffi_unsafe.rs:614`, `crates/kf-qemu/src/readtrace.rs:1` |
| `KF3_BAR0_TRACE` | off; only 1 enables | MEASUREMENT | Bounded BAR0 window read/write trace (read-trap exception) | no | no | `crates/kf-qemu/src/raw_unsafe.rs:176`, `crates/kf-qemu/src/device.rs:1431`, `crates/kf-qemu/src/ffi_unsafe.rs:48` |
| `KF3_CARRY_ATOMIC_DISABLE` | off; only 1 enables | DEBUG | Carry guest ATOMIC_DISABLE bit into mappings for fault research | no | no; needs fault service | `crates/kf-mem/src/apply.rs:312`, `crates/kf-harness/src/bin/kf-gate9.rs:50`, `crates/kf-cuda/src/abi.rs:131` |
| `KF3_COMPLETION_PROBE` | unset; set number ms, junk 1000 | MEASUREMENT | Own thread reads completion/release state after threshold | no | no | `crates/kf-chan/src/translated.rs:321`, `crates/kf-chan/src/host.rs:1075`, `crates/kf-chan/src/census.rs:12` |
| `KF3_DBFAST_PROBE` | unset; hex slot value | MEASUREMENT | Register fast doorbell probe only for an unused slot | no | no | `crates/kf-qemu/src/device.rs:2720` |
| `KF3_DBFAST_SPIN_US` | 0; parsed microseconds | DEBUG | Existing post-delivery doorbell spin experiment | no | no; no timing change here | `crates/kf-qemu/src/device.rs:2545` |
| `KF3_DEBUG` | off; only 1 enables | DEBUG | Required acknowledgement for force-migratable lab switch | no | no | `crates/kf-harness/src/bin/kf-syncwait-stress.rs:4`, `crates/kf-rm/src/chanlink.rs:322` |
| `KF3_DEBUG_CURSOR` | unset; 1 logs or value is dump directory | MEASUREMENT | Distinct cursor images/text and optional image dumps | no | no | `crates/kf-disp/src/scanout.rs:1400`, `crates/kf-qemu/src/display.rs:4204` |
| `KF3_DEBUG_FORCE_MIGRATABLE` | off; only 1 enables and DEBUG=1 required | DEBUG | Route USER channels to migratable relay for Linux validation | no | no | `crates/kf-harness/src/bin/kf-syncwait-stress.rs:5`, `crates/kf-rm/src/chanlink.rs:320`, `crates/kf-qemu/src/userrelay.rs:161` |
| `KF3_DEFERRED_API` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Authored deferred-API trigger methods on Translated rings | yes 1 | unknown today; historical Windows gate | `crates/kf-chan/src/tmode.rs:381`, `crates/kf-chan/src/tmode_gr_tests.rs:398`, `crates/kf-qemu/src/chan.rs:1778` |
| `KF3_DIAG_EVENT_RING` | off; only 1 enables | MEASUREMENT | Bounded lock-step relay event ring | no | no | `crates/kf-broker/src/ticks.rs:309` |
| `KF3_DIAG_PROGRESS` | unset/0/junk off; positive ms | MEASUREMENT | Nonblocking progress-strand watcher and timestamps | no | no | `crates/kf-qemu/src/userstrand.rs:22`, `crates/kf-qemu/src/userrelay.rs:55`, `crates/kf-qemu/src/chan.rs:3137` |
| `KF3_DIAG_STEP_METERS` | off; only 1 enables | MEASUREMENT | Display-worker step timings | no | no | `crates/kf-qemu/src/display.rs:2325` |
| `KF3_DIAG_UNMAP_DELAY_MS` | unset/0/junk off; positive ms | DEBUG | Deliberately hold unmaps for stale-mapping A/B control | no | no | `crates/kf-mem/src/vasmgr.rs:1496`, `crates/kf-mem/src/maplog.rs:50` |
| `KF3_DISPLAY_ARMED_DEFAULTS` | off; only 1 enables | DEBUG | Seed ARMED MIN_FRAME_IDLE defaults at a new core life | no | unknown; not in production | `crates/kf-qemu/src/display.rs:112` |
| `KF3_DISPLAY_BLANK_STATE` | off; only 1 enables | DEBUG | Seed BLANK readback on a new core life | no | unknown; not in production | `crates/kf-qemu/src/display.rs:98` |
| `KF3_DISPLAY_CAPS_PROBE` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Replace authored caps with measured AD104 page at matching class/base | yes 1 | historical gate as a group; NOT justified for promotion | `crates/kf-qemu/src/display.rs:157` |
| `KF3_DISPLAY_CORE_AT_VBLANK` | off; only 1 enables | DEBUG | Latch/notify active-head core updates at vblank | no | unknown; not in production | `crates/kf-disp/src/engine.rs:588`, `crates/kf-qemu/src/display.rs:2814` |
| `KF3_DISPLAY_CTRL_PROBE` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Six display controls answer OK with echoed request | yes 1 | historical Windows boot gate; owner/derivation gap | `crates/kf-rm/examples/display_replay.rs:9`, `crates/kf-rm/src/display_ctrl_probe.rs:26` |
| `KF3_DISPLAY_HDCP_STATE` | off; only 1 enables | DEBUG | Answer HDCP state OK with no HDCP flags | no | unknown; not in production | `crates/kf-disp/src/model.rs:73`, `crates/kf-rm/src/display.rs:406` |
| `KF3_DISPLAY_HDMI_PROBE` | off; only 1 enables | DEBUG | Measured connector/HDMI topology replies | no | unknown; not in production | `crates/kf-rm/src/display_ctrl_probe.rs:215` |
| `KF3_DISPLAY_HEAD_TIMING_EN_BASE` | unset; parsed hexadecimal base | DEBUG | Override initial head-timing enable readback | no | no; per-die state is not derived | `crates/kf-qemu/src/display.rs:2252` |
| `KF3_DISPLAY_HOTPLUG_EDID_SEEN` | off; only 1 enables | DEBUG | Defer hotplug until EDID has been read | no | unknown; not in production | `crates/kf-disp/src/model.rs:542`, `crates/kf-rm/src/display.rs:248` |
| `KF3_DISPLAY_ILUT_CONSTRUCTOR_PROBE` | off; only 1 enables | DEBUG | Advertise ILUT/TMO construction, refuse normal methods | no | no; constructor lab only | `crates/kf-qemu/src/display.rs:2150` |
| `KF3_DISPLAY_ILUT_OFFSET_256` | off; only 1 enables (D1 candidate on, 0 opt-out) | PRODUCTION-BEHAVIOUR | Decode input LUT offset in 256-byte units (nouveau source) | yes 1 | historical Windows colour gate as a group | `crates/kf-disp/src/color.rs:69`, `crates/kf-qemu/src/display.rs:116` |
| `KF3_DISPLAY_IMP_ENABLE` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Answer IMP_ENABLE GET true when display model is present | yes 1 | historical Windows gate jointly with control probe | `crates/kf-disp/src/model.rs:532`, `crates/kf-rm/src/display.rs:397` |
| `KF3_DISPLAY_LOADV` | off; only 1 enables | DEBUG | Add LOADV event bit and republish at frame edges | no | unknown; not in production | `crates/kf-qemu/src/display.rs:76` |
| `KF3_DISPLAY_LUT_MIRROR` | off; only 1 enables (D1 candidate on, 0 opt-out) | PRODUCTION-BEHAVIOUR | Accept MIRROR only where negative inputs cannot reach LUT | yes 1 | historical Windows colour gate as a group | `crates/kf-disp/src/color.rs:59`, `crates/kf-qemu/src/display.rs:115` |
| `KF3_DISPLAY_METHOD_TRACE` | off; only 1 enables | MEASUREMENT | Display method/engine diagnostic trace | no | no; in stock repo launcher, absent from production copy | `crates/kf-qemu/src/display.rs:2850` |
| `KF3_DISPLAY_OLUT_CONSTRUCTOR_PROBE` | off; only 1 enables | DEBUG | Constructor-only OLUT plus TMO/ILUT probe chain | no | no | `crates/kf-qemu/src/display.rs:2146` |
| `KF3_DISPLAY_PRIVATE_PROBE` | off; only 1 enables | DEBUG | Two measured commit-time private-control replies | no | unknown; not in production | `crates/kf-rm/examples/display_replay.rs:10`, `crates/kf-rm/src/display_ctrl_probe.rs:111` |
| `KF3_DISPLAY_SDR_COLOR` | off; only 1 enables (D1 candidate on, 0 opt-out) | PRODUCTION-BEHAVIOUR | Advertise source-derived SDR caps and run existing GPU colour pipeline | yes 1 | not required for idle desktop in older ablation; real colour is production | `crates/kf-qemu/src/display.rs:2166` |
| `KF3_DISPLAY_TMO_CONSTRUCTOR_PROBE` | off; only 1 enables | DEBUG | TMO constructor caps, refuse all display methods | no | no | `crates/kf-qemu/src/display.rs:2154` |
| `KF3_DISPLAY_TMO_SURFACE_CONSTRUCTOR_PROBE` | off; only 1 enables | DEBUG | TMO/ILUT surface-construction-only probe | no | no | `crates/kf-qemu/src/display.rs:2148` |
| `KF3_DISPLAY_TRACE` | off; only 1 enables | MEASUREMENT | Scanout source/copy digest trace | no | no | `crates/kf-qemu/src/display.rs:2809` |
| `KF3_DISPLAY_TRACE_CAP` | 4096 lines per kind | MEASUREMENT | Bound diagnostic trace output, parsed u32 | no | no | `crates/kf-qemu/src/display.rs:283` |
| `KF3_DISPLAY_WRITE_TRACE` | off; only 1 enables | MEASUREMENT | Trace guest display-register writes | no | no | `crates/kf-qemu/src/display.rs:85` |
| `KF3_DISPLAY_XBAR_PROBE` | off; only 1 enables | DEBUG | Measured crossbar caps/DFP_ASSIGN_SOR replies | no | unknown; not in production | `crates/kf-rm/src/display_ctrl_probe.rs:225` |
| `KF3_DOORBELL_CPL` | off; first character 1 enables | MEASUREMENT | Log guest privilege level of doorbell MMIO writes | no | no | `qemu/hw/misc/kf3/kf3.c:294` |
| `KF3_FAILCLOSED_CHILD` | unset; unit-test child mode | DEBUG | Subprocess panic-abort fixtures (cfg(test), no production read) | no | no | `crates/kf-qemu/src/failclosed.rs:104` |
| `KF3_GFX_POOL_PROBE` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Fabricated virtual GfxP pool sizing, not lifecycle | yes 1 | historical Windows boot gate; owner/derivation gap | `crates/kf-rm/src/gfxpool_probe.rs:6`, `crates/kf-rm/src/lib.rs:597` |
| `KF3_GSS_NATIVE` | off; only 1 enables | DEBUG | Nonprivileged subdevice GSS-legacy experiment | no | unknown; not in production | `crates/kf-gsp/src/boot.rs:419`, `crates/kf-rm/src/gssnative.rs:40`, `crates/kf-rm/src/rmrpc/policy.rs:270` |
| `KF3_INJECT_FWSEC_FAIL` | 0; parsed initial failure count | DEBUG | Drop initial FWSEC-FRTS STARTCPU acts | no | no | `crates/kf-qemu/src/device.rs:4232` |
| `KF3_INJECT_STALE_USERD` | unset; parsed u32 | DEBUG | Seed stale GP_PUT/GP_GET on Translated birth | no | no | `crates/kf-chan/src/host.rs:958`, `crates/kf-qemu/src/chan.rs:1895` |
| `KF3_INVALIDATE_CLEAR_OVER_ABSENT` | on; exact 0 disables | PRODUCTION-BEHAVIOUR | Complete invalidates over refused/absent host mappings | no | built-in on; research opt-out stays | `crates/kf-mem/src/vasmgr.rs:63` |
| `KF3_KERNEL_GR_CE` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Guest-kernel GR ring in private T-space with host GR context | yes 1 | historical Windows gate; Linux default scope unresolved | `crates/kf-qemu/src/chan.rs:1750` |
| `KF3_KERNEL_GR_WORK` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Re-author allowlisted guest-kernel GR methods | yes 1 | paired with KERNEL_GR_CE; default-off ruling until proven | `crates/kf-qemu/src/chan.rs:1760` |
| `KF3_LAZY_OBSERVE` | on; trimmed 0 disables; interval 16, disabled 1 | PRODUCTION-BEHAVIOUR | Conservative private USERD observation in Pointer mode | no | already default on; not a diagnostic timestamp flag | `crates/kf-chan/src/user_lazy.rs:125`, `crates/kf-qemu/src/userpump.rs:215` |
| `KF3_LIFECYCLE_MODE` | default; empty/unknown default (unknown warns) | PRODUCTION-BEHAVIOUR | Scan/Pointer/Window lifecycle; always_scan/no_pointer_markers/no_anchored_windows arms | no | already default; old all-scan arm remains selectable | `crates/kf-chan/src/user_lifecycle.rs:53`, `crates/kf-qemu/src/userpump.rs:171` |
| `KF3_MAPLOG` | off; ANY presence enables, including 0 | MEASUREMENT | VA-plane timeline, including hot-path logging | no | no; unset to disable | `crates/kf-mem/src/vasmgr.rs:605`, `crates/kf-mem/src/maplog.rs:22`, `crates/kf-qemu/src/device.rs:2232` |
| `KF3_MEMORY_LIST_PROBE` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Checked contiguous SYSRAM descriptor support | yes 1 | unknown today; in production | `crates/kf-rm/src/lib.rs:584` |
| `KF3_NEGCTL_CARVE` | off; any value other than 0 enables | DEBUG | Positive control counts every leaf as carve-out, refuses none | no | no | `crates/kf-mem/src/apply.rs:83`, `crates/kf-qemu/src/device.rs:1088`, `crates/kf-qemu/src/tspace.rs:86` |
| `KF3_NEGCTL_HEAP` | off; any value other than 0 enables | DEBUG | Count every channel birth as heap-out, refuse none | no | no | `crates/kf-qemu/src/chan.rs:1829` |
| `KF3_NEGCTL_NO_BAR1_DOORBELL` | off; ANY presence enables | DEBUG | Suppress Hopper+ BAR1 doorbell classification | no | no | `crates/kf-qemu/src/device.rs:1064` |
| `KF3_NEGCTL_NO_MICRO_RESERVE` | off; only 1 enables | DEBUG | Disable small host VA reservations for negative control | no | no; real reservations built-in on | `crates/kf-mem/src/batch.rs:1081`, `crates/kf-mem/src/sim/tests.rs:265`, `crates/kf-qemu/src/mem.rs:1297` |
| `KF3_NEGCTL_SKIP_CAP_BRACKET` | off; only 1 enables | DEBUG | Skip channel-alloc capability bracket for control | no | no | `crates/kf-host/src/channel.rs:497`, `crates/kf-host/src/birth.rs:62` |
| `KF3_NEGCTL_STALE_BIND` | off; any value other than 0 enables | DEBUG | Perturb stale-bind counter positive control | no | no | `crates/kf-chan/src/tmode.rs:2018`, `crates/kf-chan/src/host.rs:1276`, `crates/kf-qemu/src/chan.rs:1821` |
| `KF3_NEGCTL_TSPACE_OVERSIZE` | off; any value other than 0 enables | DEBUG | Oversize translated ring region to test refusal | no | no | `crates/kf-qemu/src/tspace.rs:107` |
| `KF3_NEGCTL_TWIN` | off; any value other than 0 enables | DEBUG | Make twin refusal counter move for control | no | no | `crates/kf-qemu/src/chan.rs:1838` |
| `KF3_NEGCTL_TWIN_WINDOW` | off; any value other than 0 enables | DEBUG | Map store window in twin as forbidden-window positive control | no | no | `crates/kf-qemu/src/mem.rs:2932` |
| `KF3_NO_BATCHED_MAP` | off; ANY presence disables batching | PRODUCTION-BEHAVIOUR | Select per-run mapping research arm | no | batching already built-in on | `crates/kf-mem/src/sim.rs:874`, `crates/kf-mem/src/batch.rs:1054`, `crates/kf-mem/src/sim/tests.rs:1368` |
| `KF3_NO_GUEST_VA_RESERVE` | off; ANY presence disables reservation | DEBUG | Disable guest low-VA reservation for host experiment | no | no | `crates/kf-host/src/channel.rs:677`, `crates/kf-qemu/src/chan.rs:242` |
| `KF3_PREEMPT_BIND_PROBE` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Answer preemption bind OK without host bind | yes 1 | historical Windows boot gate; owner/native gap | `crates/kf-rm/src/preempt_bind_probe.rs:26` |
| `KF3_PROF` | off; 1/on enables | MEASUREMENT | Trap/publish attribution timers and counters | no | no | `crates/kf-qemu/src/prof.rs:35`, `crates/kf-qemu/src/device.rs:305` |
| `KF3_PT_NSI_MIN_INTERVAL_US` | 0; invalid/0 off; positive us capped at 1000000 | PRODUCTION-BEHAVIOUR | Optional existing loss-free interrupt pacing (§X) | no | no delay by default; do not tune here | `crates/kf-chan/src/ptnsi.rs:62`, `crates/kf-qemu/src/chan.rs:2747` |
| `KF3_PT_NSI_RELAY` | on; trimmed 0 disables | PRODUCTION-BEHAVIOUR | Relay armed FIFO host events; 0 counts only | no | already built-in on | `crates/kf-chan/src/ptnsi.rs:334`, `crates/kf-qemu/src/chan.rs:2749` |
| `KF3_PT_STALL_SNAPSHOT` | off; only 1 enables | MEASUREMENT | Bounded snapshots of Passthrough stall/free state | no | no | `crates/kf-qemu/src/device.rs:2568`, `crates/kf-qemu/src/chan.rs:956` |
| `KF3_PT_STALL_SNAPSHOT_MS` | 1000; clamp 200..10000 | MEASUREMENT | Snapshot silence threshold when snapshot enabled | no | no | `crates/kf-qemu/src/chan.rs:959` |
| `KF3_PUT_CACHE` | on; trimmed 0 disables | PRODUCTION-BEHAVIOUR | Cache published guest GP_PUT until exhausted, per generation | no | already built-in on | `crates/kf-chan/src/user_lazy.rs:122`, `crates/kf-qemu/src/userpump.rs:215` |
| `KF3_READ_TRACE_MAX_BYTES` | 512 MiB; 1..16 GiB allowed | MEASUREMENT | BAR0 read-trace byte cap, parsed only if enabled | no | no | `crates/kf-qemu/src/readtrace.rs:16` |
| `KF3_READ_TRACE_MAX_RECORDS` | 8000000; positive only | MEASUREMENT | BAR0 read-trace record cap | no | no | `crates/kf-qemu/src/readtrace.rs:15` |
| `KF3_READ_TRACE_RANGES` | display range plus any supplied ranges | MEASUREMENT | Select read ranges; display range always included | no | no | `crates/kf-qemu/src/readtrace.rs:13` |
| `KF3_RELAY_GET_REFRESH` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Refresh relayed GP_GET from host event/park tick (never GP_PUT) | no | unknown; not in production | `crates/kf-chan/src/userd_relay.rs:24`, `crates/kf-chan/src/worker.rs:61`, `crates/kf-qemu/src/device.rs:3610` |
| `KF3_RELAY_PB_PEEK` | unset; 1 or after-fifo-update enables | MEASUREMENT | Bounded relayed pushbuffer/semaphore diagnostic peek | no | no | `crates/kf-qemu/src/chan.rs:860` |
| `KF3_RPC_TRACE` | off; only 1 enables | MEASUREMENT | GSP/RM command census trace | no | no | `crates/kf-rm/examples/display_replay.rs:3`, `crates/kf-rm/src/census.rs:334` |
| `KF3_STRESS_SECS` | 3 userstore, 2 lifecycle stress/props | DEBUG | GPU-free stress-test duration only (not product config) | no | no | `crates/kf-qemu/src/userstore.rs:863` |
| `KF3_STRESS_SEED` | clock-derived userstore seed | DEBUG | GPU-free userstore test seed | no | no | `crates/kf-qemu/src/userstore.rs:867` |
| `KF3_STRESS_THREADS` | 8 | DEBUG | GPU-free userstore test worker count | no | no | `crates/kf-qemu/src/userstore.rs:877` |
| `KF3_SW_RUNLIST_HOST_OWNED` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Ignore guest software-runlist submit and schedule kernel channels at birth | yes 1 | historical Windows gate; §W promotion STOPPED | `crates/kf-rm/src/sw_runlist_host.rs:36`, `crates/kf-qemu/src/chan.rs:1847` |
| `KF3_SW_RUNLIST_PROBE` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Software-runlist allocation metadata experiment | yes 1 | unknown today; no real scheduling by itself | `crates/kf-rm/src/lib.rs:501` |
| `KF3_SW_SUBCH_INERT` | off; only 1 enables | DEBUG | Accept unknown software-subchannel binds, refuse later methods | no | shadowed by DEFERRED_API in older production group | `crates/kf-qemu/src/chan.rs:1768` |
| `KF3_TCENSUS` | off; any value other than 0 enables | MEASUREMENT | Translated channel address/binding census | no | no | `crates/kf-qemu/src/chan.rs:1841` |
| `KF3_TRACE_NAME` | host BDF; validated name override | MEASUREMENT | Device name in BAR0 trace records | no | no | `crates/kf-qemu/src/ffi_unsafe.rs:641`, `crates/kf-qemu/src/readtrace.rs:182` |
| `KF3_TRANSLATED_CE_RELAY` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Relay real retired CE completions to guest engine vector | yes 1 | unknown current ablation; dependent channel group | `crates/kf-harness/src/bin/kf-gr-tier.rs:23`, `crates/kf-qemu/src/device.rs:4027`, `crates/kf-qemu/src/chan.rs:1788` |
| `KF3_TSHADOW` | off; any value other than 0 enables | MEASUREMENT | Alias enabling census only (old shadow algorithm deleted) | no | no | `crates/kf-chan/src/tmode.rs:22`, `crates/kf-qemu/src/chan.rs:1842` |
| `KF3_USERD_RELAY_OFF` | off; only 1 enables | DEBUG | Old Windows USERD adoption route without relay | no | research arm only | `crates/kf-qemu/src/chan.rs:1804` |
| `KF3_USER_RING_TRACE` | unset; nonempty/nonzero directory enables | MEASUREMENT | Bounded scan-refusal dumps and deferred rereads | no | no | `crates/kf-chan/src/user_storage.rs:675`, `crates/kf-qemu/src/device.rs:3985`, `crates/kf-qemu/src/chan.rs:2355` |
| `KF3_VRAM_ATTRS` | s1-nvkms-offscreen; s0/s1/s2 overrides | DEBUG | Display-copy slot allocation attribute experiment | no | no; s1 already default | `crates/kf-qemu/src/gpucopy.rs:367` |
| `KF3_WALK_REFUSAL_DUMP` | off; nonempty/nonzero enables | MEASUREMENT | Sample refused walker entries and levels | no | no | `crates/kf-mem/src/vasmgr.rs:407`, `crates/kf-mem/src/refusal_dump.rs:51` |
| `KF3_WIN_REFUSE_PT_FIFO_UPDATE` | off; only 1 enables | DEBUG | Explicit refusal of Windows PT FIFO migration registration | no | no | `crates/kf-rm/src/defapi.rs:324`, `crates/kf-qemu/src/chan.rs:1173` |
| `KF3_WIN_TWIN_DEFAPI_OBJECT` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Author empty 5080 host object on Windows user-work twin | yes 1 | historical Windows gate; dependent channel group | `crates/kf-rm/src/chanlink.rs:1067`, `crates/kf-host/src/channel.rs:1636`, `crates/kf-qemu/src/chan.rs:1156` |
| `KF3_WIN_USER_CHANNELS_PASSTHROUGH` | off; only 1 enables (two readers) | PRODUCTION-BEHAVIOUR | Classify Windows per-process USER work; also lower mirror VA start globally | yes 1 | historical Windows gate; Linux VA-floor scope unresolved | `crates/kf-harness/src/bin/kf-micro-reserve-probe.rs:15`, `crates/kf-rm/src/chanlink.rs:914`, `crates/kf-host/src/channel.rs:275` |
| `KF3_WIN_USER_RING_RELAY` | on; exact 0 disables (shadow can force on) | PRODUCTION-BEHAVIOUR | Restricted Windows USER ring relay at birth | no | already built-in on; explicit old arm retained | `crates/kf-qemu/src/userrelay.rs:16` |
| `KF3_WIN_USER_RING_SHADOW` | off; only 1 enables | DEBUG | Opaque original-GP shadow experiment, forces relay route | no | no | `crates/kf-qemu/src/userrelay.rs:26` |
| `KF3_WORKER_PARK_INFINITE` | off; only 1 enables | MEASUREMENT | Disable existing worker park safety timeout for missing-wake proof | no | no | `crates/kf-chan/src/worker.rs:113` |
| `KF3_WRITE_TRACE_RANGES` | all | MEASUREMENT | Select BAR0 read-trace companion write ranges | no | no | `crates/kf-qemu/src/readtrace.rs:13` |
| `KF3_ZCULL_BIND_PROBE` | off; only 1 enables | PRODUCTION-BEHAVIOUR | Answer cross-client ZCULL bind OK without host bind | yes 1 | unknown current ablation; owner/native gap | `crates/kf-rm/src/preempt_bind_probe.rs:147` |

CODE READ: measurement enable switches default off. Their numerical limits/thresholds are inert without the enable. Research opt-outs whose **production behavior** defaults on (lifecycle, put cache, lazy observation, interrupt relay) are class PRODUCTION-BEHAVIOUR, even where older comments called the switch “measurement class”. `KF3_MAPLOG=0`, `KF3_NO_BATCHED_MAP=0`, `KF3_NO_GUEST_VA_RESERVE=0` and `KF3_NEGCTL_NO_BAR1_DOORBELL=0` are **not** off switches; unset them. No parser normalization is made by D1.

### Dead names and launcher-only controls

CODE READ: DEAD means no product reader, not permission to delete historical evidence. Names in old scripts remain indexed for reproducibility. Configured identity values are not copied into this document.

| Name | Default/effect today | Class | Where referenced | Production sets / needs |
|---|---|---|---|---|
| `KF3_TIMER_MAP` | No reader; validated native timer mapping is automatic | DEAD | `scripts/bench/hitch/launch_h.sh:27` | explicitly dropped / no |
| `KF3_TSPACE` | No reader; T-space hardwired by §AB | DEAD | `crates/kf-mem/src/apply.rs:82`, `crates/kf-chan/src/translated.rs:167`, `crates/kf-qemu/src/mem.rs:1684` | no / no |
| `KF3_INCA_REFUSE` | No reader; strict carve/refusal behavior hardwired by §AB | DEAD | `crates/kf-chan/src/translated.rs:167`, `crates/kf-qemu/src/chan.rs:9135`, `crates/kf-qemu/src/tspace.rs:47` | no / no |
| `KF3_KERNEL_NVDEC_CTX` | No reader; hardwired kernel video context | DEAD | `crates/kf-qemu/src/chan.rs:6651` | no / no |
| `KF3_KERNEL_NVENC_CTX` | No reader; hardwired kernel video context | DEAD | `scripts/bench/windows/pc_sdr_experiment.py:46` | no / no |
| `KF3_KERNEL_OFA_CTX` | No reader; hardwired kernel video context | DEAD | `scripts/bench/windows/pc_sdr_experiment.py:46` | no / no |
| `KF3_BATCH_MICRO_RESERVE` | No reader; micro-reservations hardwired, negative control retained | DEAD | `crates/kf-qemu/src/mem.rs:1287` | no / no |
| `KF3_SCHEDULE_LATE_JOINERS` | No reader; late-join scheduling hardwired | DEAD | `crates/kf-qemu/src/latejoin.rs:6` | no / no |
| `KF3_WIN_KERNEL_PID4` | No reader; system-PID classification hardwired | DEAD | `crates/kf-rm/src/chanlink.rs:301`, `crates/kf-qemu/src/device.rs:3034`, `crates/kf-qemu/src/twin.rs:292` | no / no |
| `KF3_NEGCTL_SHADOW` | No reader; old T-mode shadow algorithm deleted | DEAD | `crates/kf-chan/src/tmode.rs:1559`, `crates/kf-chan/src/host.rs:1331`, `crates/kf-qemu/src/chan.rs:1846` | no / no |
| `KF3_TEAR_GATE` | No reader; obsolete held-flip rate gate removed by §AK | DEAD | `crates/kf-qemu/src/display.rs` regression guard; historical launcher comments | no / no |
| `KF3_LIFECYCLE_COUNT` | No reader; does not enable lifecycle or counters in master | DEAD | `scripts/bench/hitch/launch_h.sh:27` | yes 1 / no effect |
| `KF3_LIFECYCLE_PERIOD_S` | No reader; does not set a reporting period in master | DEAD | `scripts/bench/hitch/launch_h.sh:27` | yes 5 / no effect |
| `KF3_TSG_MIRROR` | Absent from master; host launcher comment belongs to another revision | DEAD | No source occurrence | no / no |
| `KF3_TWIN_VA_BASE` | No reader; replaced by MirrorVaStart | DEAD | `crates/kf-host/src/channel.rs:253` | no / no |
| `KF3_DIAG_ZERO_OK` | No reader; replaced by host-owned stub policy | DEAD | `crates/kf-rm/src/hoststub.rs:16` | no / no |

CODE READ: the following shell controls are not read by the device. They are DEBUG/reproducibility inputs; a user invoking QEMU directly needs none of them.

| Name | Script default / action | Class | Location | Production use |
|---|---|---|---|---|
| `KF3_BIN` | Per-revision binary in gfx/refusal harnesses; paths differ by harness | DEBUG | `scripts/bench/gfxset/suite.sh:14` | no |
| `KF3_BINS` | Per-revision binary parent in plane oracle | DEBUG | `scripts/bench/display/planes_oracle.sh:13` | no |
| `KF3_DEV_EXTRA` | Empty; append QOM property string to fast guest (every GPU) | DEBUG | `scripts/bench/dbfast_exit_hook.sh:5` | no |
| `KF3_EXTRA` | Empty; append QOM property string to interactive display launcher | DEBUG | `scripts/bench/display/interactive.sh:131` | no |
| `KF3_FB_MB` | Auto 8192 MiB, reduced below 10 GiB host VRAM with 2 GiB reserve in fastguest/boot_nvkvm | DEBUG | `scripts/bench/boot_nvkvm.sh:35` | no |
| `KF3_GPUS` | Unset = one default host minor; comma-separated minors select multi-GPU | DEBUG | `scripts/fastguest/build_fast_guest.sh:433` | no |
| `KF3_REV` | Normally checkout short HEAD (+dirty); Windows broker requires explicit selection | DEBUG | `scripts/bench/boot_nvkvm.sh:22` | 0d7ba743 (not master) |
| `KF3_REV_BIN` | Legacy glcrash harness binary revision (459da55d default) | DEBUG | `scripts/bench/windows/glcrash/tdr-run28.sh:5` | no |

### Tokens deliberately excluded from environment inventory

CODE READ: a token sweep alone overcounts macros, include guards, record labels and fragments. These have no environment default, launcher assignment or production need:
`KF3_ABI`, `KF3_BAR1_OVERLAYS_DEFAULT`, `KF3_BROKER_`, `KF3_BROKER_ON`, `KF3_BROKER_SURFACE`, `KF3_BROKER_VRAM_`, `KF3_BROKER_VRAM_AUTO`, `KF3_BROKER_VRAM_OFF`, `KF3_BROKER_VRAM_ON`, `KF3_BROKER_VRAM_SHIFT`, `KF3_BTN_`, `KF3_BTN_EXTRA`, `KF3_BTN_LEFT`, `KF3_BTN_MIDDLE`, `KF3_BTN_RIGHT`, `KF3_BTN_SIDE`, `KF3_BUILT`, `KF3_CURSOR_DEFINE`, `KF3_CURSOR_MAX_DIM`, `KF3_DISPLAY_`, `KF3_DISPLAY_FLAGS`, `KF3_DUMMY_OFF`, `KF3_GOP_H`, `KF3_GSP_OBSERVER`, `KF3_GUEST_RAM_H`, `KF3_H`, `KF3_KERNEL_`, `KF3_MAX_PIECES`, `KF3_MAX_VECTORS`, `KF3_MSIX_BAR`, `KF3_PT_NSI_`, `KF3_RC`, `KF3_REFRESH_BACKSTOP_MS`, `KF3_REPO`, `KF3_X`.
`KF3_GSP_OBSERVER` is a compile-time define enabling optional QOM observer support, not an environment switch. `KF3_BIN`, `KF3_BINS`, `KF3_REV` and friends choose test tools, not boot behavior.


## QOM device properties and omission plan

CODE READ: all 24 custom properties are defined by `qemu/hw/misc/kf3/kf3.c::kf3_properties`; realize reads them directly or passes them into `kf_qemu::device::Config`, the broker setup, GOP builder or ioeventfd setup. Values below are **base defaults**, unchanged by D1. A socket, VM identity, driver ABI and resource budget are configuration inputs, not measurement requirements. UNVERIFIED (hardware): none of the omissions below is a fresh VM boot test; QOM introspection and pure validators can establish defaults/validation only.

| Property | Base default | Class / effect; read site | Production perf6 value | Omit from minimal launch? |
|---|---|---|---|---|
| `gpu-minor` | 0 | PRODUCTION-BEHAVIOUR; host NVIDIA device minor; C realize → Config | omitted | yes for minor 0; not host GPU UUID selection |
| `fb-mb` | 8192 MiB | PRODUCTION-BEHAVIOUR; one real VRAM store; `device.rs`, `cardbudget.rs` | 8192 | yes if card budget permits; keep smaller explicit value on small/shared cards |
| `bar1-size` | 256 MiB | PRODUCTION-BEHAVIOUR; guest CPU framebuffer aperture and host BAR1 budget; C BAR builder / `cardbudget.rs` | 8589934592 | candidate omit for Linux; Windows 256 MiB replacing production 8 GiB is hardware-UNVERIFIED |
| `bar1-overlays` | 64 | PRODUCTION-BEHAVIOUR; Hopper+ BAR1 doorbell view limit; C view builder | omitted | yes |
| `bar2-size` | 32 MiB | PRODUCTION-BEHAVIOUR; instance aperture; C BAR builder | 33554432 | yes, exact default |
| `msix-vectors` | 32 | PRODUCTION-BEHAVIOUR; interrupt vectors; C MSI-X init | omitted | yes; kernel irqchip is required |
| `guest-driver` | host driver's version | PRODUCTION-BEHAVIOUR; guest GSP/RM ABI; `device.rs`, `guestsysinfo.rs` | 580.65.06 for Windows 580.88 | omit only when Linux guest uses same version as host; retain explicit Windows ABI |
| `dummy-bar` | off | DEBUG; synthetic exit/doorbell measurement pages; C MSI-X BAR | omitted | yes |
| `display` | off | PRODUCTION-BEHAVIOUR; emulate physical display engine; Config/display | on | **retain on for a desktop**; headless CUDA can omit |
| `gop` | off | PRODUCTION-BEHAVIOUR; embedded UEFI boot-display ROM; `gop.rs`, C ROM builder | on | **retain on for firmware/installer image in broker**; no VGA companion needed |
| `x11-dispsw` | off | PRODUCTION-BEHAVIOUR; optional GF100_DISP_SW object; Config / RM display link | omitted | retain on only for Linux X11 desktop; production Windows omits |
| `gop-efi` | unset | PRODUCTION-BEHAVIOUR; signed matching GOP image; `gop.rs` | omitted | yes with Secure Boot off; stock embedded ROM is unsigned |
| `gpu-uuid` | auto | PRODUCTION-BEHAVIOUR; guest-visible UUID; `gpuuid.rs` | explicit per-VM UUID | auto with persistent `-uuid` is code-supported; omitting explicit value is hardware-UNVERIFIED |
| `vm-id` | QEMU `-uuid`, else none | PRODUCTION-BEHAVIOUR; auto UUID identity input; C realize / `gpuuid.rs` | omitted, `-uuid` given | yes when `-uuid` supplied; no identity means random fallback |
| `channel-budget` | 0 → host-derived | PRODUCTION-BEHAVIOUR; per-runlist advertised/enforced cap; `chanbudget` | omitted | yes; explicit limit must satisfy host/minimum bounds |
| `user-view-keep` | 0 → 16 views/channel | PRODUCTION-BEHAVIOUR; bounded CPU store-view cache; `userstore.rs` | omitted | yes; cap 256 |
| `doorbell-ioeventfd` | off | PRODUCTION-BEHAVIOUR; optional KVM fast doorbell; C realize / Rust drainer | omitted | yes; regular trapped forwarding remains |
| `doorbell-ioeventfd-max` | 256 | PRODUCTION-BEHAVIOUR; registration cap; C | omitted | yes |
| `display-broker` | unset | PRODUCTION-BEHAVIOUR; absolute Unix socket; C / broker adapter | socket path | **retain for broker window**; without it console only |
| `display-broker-uid` | -1 | PRODUCTION-BEHAVIOUR; additional accepted broker uid; broker adapter | desktop user's uid | omit when QEMU and broker share euid (root also accepted); retain across users |
| `display-broker-vram` | auto | PRODUCTION-BEHAVIOUR; negotiate own VRAM copy slots; C / gpucopy | omitted | yes; on requires realize-time success, off is explicit host-memory research arm |
| `display-max-fps` | 0 → 75 Hz cap/current EDID | PRODUCTION-BEHAVIOUR; vblank/monitor cap; `pace.rs`, model | omitted | yes; explicit 24..75 only and requires display |
| `x-gsp-observer` | unset | MEASUREMENT; optional fresh capture path; C observer | unset (WIN_TRACE=0) | yes; never needed for boot |
| `x-gsp-observer-seconds` | 300 | MEASUREMENT; 1..7200 s when observer enabled; C | omitted | yes |

CODE READ: there are **24** custom properties in the table (the source count is checked in the evidence), plus inherited PCI properties. The inherited ones read explicitly are `romfile`, `rombar` and `romsize` (GOP conflict/ROM sizing). Omit all three for embedded GOP; an incompatible override is refused. The production launcher uses `bus=pci.0,addr=0x6,id=kf0` for placement/control. They are QEMU's inherited device inputs; omit them for one automatically placed device on `pc`, retaining an `id` only for QMP workflows that address the device.

CODE READ: resource inputs (`fb-mb`, BAR apertures, host minor) cannot be replaced with universally valid constants: the store must fit available host VRAM, and the guest apertures plus PRAMIN/headroom must fit host BAR1. `gpu-uuid` names the **guest** identity, not the selected host GPU. `guest-driver` is a declared ABI, not the package filename; Windows 580.88 uses the measured 580.65.06 layout in this production launcher.

### Inherited QEMU inputs

CODE READ (QEMU 10.2.4 `hw/pci/pci.c`, official release source used for the build): `romfile` defaults unset, `rombar` to -1 (automatic) and `romsize` to UINT32_MAX (automatic). They are PRODUCTION-BEHAVIOUR inputs read by kf3's GOP conflict/ROM builder, omitted by production, and not needed with the embedded GOP. `addr` defaults -1 (automatic), `id` unset, and `bus` to QEMU's available default bus; production explicitly names slot 6, id kf0 and pci.0. These optional placement identifiers are read by QEMU and by kf3 for logging/auto-GPU-UUID slot identity, not measurement flags. Omitting them has not been hardware-tested by D1.

## D1 implementation result (GPU-free)

CODE READ: the existing SDR pipeline, inert-MIRROR admission and 256-byte ILUT decoding are now selected by default. Only an exact `=0` selects each prior arm. The decoder, colour kernels, capability derivation and captured-page probe are unchanged. Constructor-only probes remain opt-in and need `KF3_DISPLAY_SDR_COLOR=0` to select their old no-processing arm.

CODE READ: the main `scripts/bench/windows/windows_broker.sh` no longer inserts RPC trace, BAR0 window trace, map logging or display-method trace into the default environment. Explicit `WIN_FLAGS=NAME=1` still enables each diagnostic. It also drops dead `KF3_TIMER_MAP`; the hitch launcher's extra list defaults empty instead of passing dead lifecycle counter/period names. Historical launchers and run records are retained. The read-only host launcher was not modified.

MEASURED: test-only EXPECTED RED commits and mutation results are in [D1 evidence](../../traces/install_flags_20261018/README.md). These checks establish the selected configuration and GPU-free decode/launcher behavior; all new default combinations and reduced guest commands remain hardware-UNVERIFIED. No relay/VA/lock/timer/poll implementation changed. All parked dependent-channel and captured/fabricated-reply decisions above remain unresolved; this is not a flag-free Windows boot claim.
