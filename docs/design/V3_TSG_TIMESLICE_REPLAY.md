# TSG timeslice intent and optional readback facts

**STATUS: LIVE, 2026-10-26; GPU-free validation only.** Hardware validation is outstanding.

## Contract and evidence limits

CODE READ, base `3bfb0965640f12c3971293b2b1b2d989d6e0cfef`, `ChanPlane::statement`: a known TSG's SET arriving before any host member could return NotOurs without retaining its value. Looking up live members before queueing could also miss a preceding birth still waiting on the act thread. The new tests exercise both early intent and production wiring; they do not establish workload frequency or performance impact.

MEASURED in PF1's retained run53/run55 and run19 windows: the identified timeslice calls follow host births; all 57 native WVF3 calls follow the first guest channel allocation, without observing host birth/value. See the [PF1 audit at its pinned publication](https://github.com/reindertpelsma/kayfabe/blob/5fa2bc567fa7065ffb48a8fefa6b69a22c89e8d4/docs/audits/TSG_PARITY_20261026.md). These bounded windows do not demonstrate an early-loss trigger on a current workload. INFERRED: expect no performance effect from the early path in those observed windows.

FALSIFIER for a future causal claim: a complete target birth/set/readback stream with zero early sets and matching requested/native bookkeeping in both replay arms falsifies early loss as that boot's cause. A timeslice getter reports RM's requested-value bookkeeping, not necessarily the rounded hardware quantum. Neither a source default nor a successful getter proves an effective scheduling quantum or preemption cost.

## Remember, replay and lifetime

CODE READ, `timeslice.rs`, `chan.rs` and `userrelay.rs`: known TSG SETs reserve bounded state at statement time, then execute through the existing deferred act queue. All host members are resolved on that queue after preceding births. Each live host group receives one SET, even with many members. Only successful live SETs replace saved intent. An early SET receives NV_OK after its unchanged u64 is remembered; the first native group validates it at birth. A native refusal fails that birth with its exact RM status, retains the request, and uses the existing native cleanup path. There is no invented CPU value limit or rounding rule. Transport failures map to INVALID_STATE rather than claiming a native RM refusal.

CODE READ, `chanlink.rs`: only an allocated TSG produces a TSG-timeslice statement. Unknown handles and other classes keep their existing fallback. The owning device follows the statement, so device/client/TSG Free invalidates saved and queued intent before handle reuse. A queued SET from the old lifetime cannot affect its successor. Reset clears intent. Failed queue admission cancels its reservation exactly once; freeing an object cannot hide still-queued acts from the global queue cap.

CODE READ, `tsgmirror.rs` is unchanged from the base. Replay runs after its plan/commit and before publication. Explicit and legacy joiners receive no redundant group SET; each new mirror-OFF ctxshare shard and each Translated ring's private group receives the saved request. Mode/ctxshare-cap/member refusals do not consume saved intent. Native cleanup retains channel, ctxshare, TSG ordering. Successful native group free removes only applied-value telemetry; desired intent lives until guest TSG/device/client Free or reset.

| State | Bound | Exhaustion |
|---|---:|---|
| Remembered/reserved TSGs per client | 256 | NO_MEMORY |
| Remembered/reserved TSGs per VM | 4096 | NO_MEMORY |
| Outstanding SET tickets, including freed lifetimes | 4096 per VM | NO_MEMORY |
| Applied-group telemetry | 65536 | Omit diagnostic entry; no behavior decision |
| Applied groups printed in one status line | 16 | Count remaining entries |
| Optional fact logical lifetimes / host groups | 65536 each | Omit facts, count drops |
| Optional fact events | 100000 per VM session | Stop GET/log work, count drops |

CODE READ, `Replay::summary`: the existing status line includes `timeslice_early_sets`, `timeslice_replayed`, `timeslice_dropped`, and `timeslice_last_us` per retained printed group. The last value is the authored request, not a hardware quantum reading. Healthy replay has zero drops. `KF3_TIMESLICE_REPLAY=0` suppresses only birth replay and counts each suppression; live setters and intent/free accounting still operate. Replay is ON with no environment setting.

## Default-off facts

CODE READ, `tsgfacts.rs` and `kf-host::tsgread`: `KF3_TSG_FACTS=1` enables bounded `TSG-FACT` lines at group birth, member binding, timeslice setters, ctxsw-mode setters and the existing direct zcull-bind adapter. Each has the client/logical group, observed logical generation, unique host birth, owned host TSG/channel, latest guest desire, request, setter result, and both readback results. A bare channel's implicit logical group uses its guest channel handle. Generations start on the first observed SET/birth, end on guest Free/reset, and are not an assertion that all guest allocations were observed. Different shards share one logical generation and have distinct host-birth identities. Failed setters remain visible beside the actual getter result. Existing unsupported PM/preemption-bind adapters are not added by this diagnostic.

CODE READ, official OGKM 595.91.07 headers/exports: GET_TIMESLICE `0xa06c0104` has an eight-byte output on the owned TSG; GET_CTXSW_MODES `0x20801235` has a 24-byte parameter block with our channel at offset zero on our subdevice. The latter returns zcull, PM, SMPC, compute-preemption and graphics-preemption fields. Both exports are NON_PRIVILEGED with access-right mask zero. Readback refusals are logged, never converted into guest success or failure. Neither GET executes when facts are disabled or the event budget is exhausted. No fact lock, intent lock or member/slot lock spans a host call. Interleave is untouched and remains subject to the existing capability-free refusal.

CODE READ, NVIDIA references: `ctrl/ctrla06c.h:129–180`, `ctrl/ctrl2080/ctrl2080gr.h:1817–1848`, `g_kernel_channel_group_api_nvoc.c:249–260`, and `g_subdevice_nvoc.c:5767–5780`. These are public-source ABI/access readings, not evidence of deployed firmware acceptance. No closed-binary implementation is used.

## Validation and future hardware

GPU-free results and the mutation table are in [the TS1 evidence index](../../traces/ts1_20261018/README.md). Source-composition guards supplement fake-RM behavior tests; they do not exercise an ioctl or a real birth. All original mirror tests remain unchanged.

Hardware remains unrun. Before a performance claim: gates 9/9, the 30-arm fast suite with replay explicitly 1 and 0, then Windows SPEC nine with the mirror ON. With correctness green, compare private Windows Steel Nomad window medians to the private baseline and run Enscape with the sealed ENS3 diagnostics. Enable facts in separate observation boots, capture birth/set/status/get streams and early counts, and keep diagnostic readback overhead out of performance arms. No score is published here.
