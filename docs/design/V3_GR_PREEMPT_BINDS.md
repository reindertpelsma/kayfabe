# GR preemption and zcull binds on the USER relay

**STATUS: DESIGN-ONLY, 2026-10-27.** GFXP1, base commit `d2ea6d26be6f746e579e6bfc8dbfe39847ddb990`. GPU-free design and tests-first implementation; hardware validation is required before default enablement. `KF3_GR_BINDS` is OFF unless explicitly `1`; `0` disables it. No host/GPU experiment is authorized by this task.

## Hypothesis and falsifiers

**INFERRED H-PREEMPT:** a guest-requested but unapplied GR bind could leave an effective preemption/buffer configuration different from native, extending service delays and contributing to guest engine timeouts. A long walker wall timer does not identify which engine/context prevented progress. Before any hardware test, reject this explanation if the affected guest never triggers the missing bind, if the host already has equivalent effective modes/buffers, or if correctly applied binds/readbacks leave the symptom unchanged. Graphics preemption on a guest channel also does not establish compute preemption on kayfabe's own walker context.

Microsoft describes scheduler preemption followed by a default two-second TDR wait, but this does not identify a particular NVIDIA request or missing fence in our boots. See [WDDM TDR](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/timeout-detection-and-recovery) and [TdrDelay](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/tdr-registry-keys). No timeout/registry constant is changed.

**CODE READ, OGKM580.159.04/595.84 `ctrl2080gr.h`:** zcull NO_CTXSW means resetting tables at a context switch without saving/restoring the buffer; it does not mean disabling all depth culling. SEPARATE_BUFFER saves/restores that state. Neither missing deferred adapter proves a WFI default nor missing guest zcull bind proves the entire active-time gap. Effective state must be read from the actual host channel with `KF3_TSG_FACTS=1`.

## Existing behavior and demand

**CODE READ, base commit `d2ea6d26`:** `kf-qemu/userpump.rs::software` runs a bounded, registered class5080 action after its prefix witness, then keeps the suffix parked until completion. Initialize/promote/evict use an asynchronous production adapter. Deferred zcull/PM/preemption binds have no USER adapter. The legacy Translated/T-space classifier explicitly refuses their guest VAs because T-space does not map those addresses; it does **not** already serve these binds. Direct same-client zcull and ctxsw setters do reach owned host objects. Preserve that distinction rather than extending a supposed working T-space bind path.

**MEASURED, retained census C at commit `6564dd597` and WC tables at commit `1eedda417`, PF1 parity evidence on branch `codex/pf1-tsgparity-20261026`:** successful direct preemption-bind counts C/B202/B205/B206 are9/12/12/12; zcull-bind3/5/5/5; PM-bind direct0 in those tables. They omit parameter values, ordering and nested deferred bundle execution; they are startup populations, not proof that the failing workload fires a particular deferred request. No current host log is read for this GPU-free task.

## Access and source boundary

**CODE READ, NVIDIA generated `g_subdevice_nvoc.c` in both inspected versions:**

| Control | ID | Export flags / access mask | Host authority |
|---|---|---|---|
| GR_CTXSW_PREEMPTION_BIND | `20801211` | `10248` /0 | NON_PRIVILEGED |
| GR_CTXSW_ZCULL_BIND | `20801208` | `90348` /0 | NON_PRIVILEGED; hClient must be caller for ordinary clients |
| GR_CTXSW_PM_BIND | `20801209` | `10048` /0 | NON_PRIVILEGED |
| GR_SET_CTXSW_PREEMPTION_MODE | `20801210` | `10348` /0 | NON_PRIVILEGED |
| GR_GET_CTXSW_MODES | `20801235` | `48` /0 | NON_PRIVILEGED |
| GR_CTXSW_PM_MODE | `20801207` | `118` /0 | NON_PRIVILEGED |
| GR_GET_ENGINE_CONTEXT_PROPERTIES | `2080122d` | `8` /0 | NON_PRIVILEGED; per-buffer size/alignment from host RM |

No CAP_SYS_ADMIN, CAP_SYS_NICE, CAP_PERFMON, kernel module or patched driver is needed for these ordinary owned-object operations. Host RM's supported modes, limits and exact refusal statuses remain authoritative. Scheduling fairness is not an extra security prohibition. Guest handles, route information, physical addresses and raw registration bytes never become host control bytes.

**CODE READ, OGKM `kernel_graphics_context.c`:** `kgrctxMapCtxBuffers` and related allocation paths provide main/patch/global buffers and may preallocate PM state; allocation depends on skip-buffer flags, context/subcontext, platform and driver policy. Unmap/free paths include zcull and preemption buffers. The public source does not contain the actual `_IMPL` bodies of GR preemption bind/set and zcull bind on the GSP path. Consequently it does not establish whether a particular deployed mode setter automatically allocates all buffers, refuses, or selects another effective mode. A successful setter is not a buffer/default-mode proof.

**CODE READ, OGKM595 `kernel_graphics.c::subdeviceCtrlCmdKGrGetEngineContextProperties_IMPL`:** route0 selects this device's GR, validates engine index, reads host static context-buffer size/alignment, rejects unavailable size `NV_U32_MAX`, and fills `bInfoPopulated`. Inputs must be zeroed; do not let a guest use the optional prepopulated values to enlarge the answer. The 580 source has the same ordinary query contract. Layouts come from the generated version matrix; engine IDs come from the public SDK enumeration, not a captured size or die table.

## Chosen path: guest VA, authored bind, VA worker

**CODE READ, base commit `d2ea6d26`:** path **(a)** is feasible without new mappings: bind only guest VAs already confirmed in the triggering USER twin's own host VAS. Use the host's query for each nonzero buffer pointer's extent/alignment. Cover the whole interval with confirmed writable placement rows; reject holes, read-only rows, overflow, unknown/zero/oversized size, bad alignment and unsupported pointer domains. Keep the exact guest VA unchanged in the authored control. No allocation, map, remap, contents copy or new GPU authority is introduced.

The first adapter supports the triggering GR channel or its own TSG in the same logical guest client. A foreign target is refused; extending to another live channel requires a target-specific mirror/lifetime/stream-order adapter, not passing its guest handle to RM. This is the concrete supported target contract of this first adapter, not an assertion that ordinary NVIDIA clients cannot bind their other owned objects. Legacy T-space remains refused.

Guest decoding uses the registering driver's generated layout, including the8/9-pointer preemption-array change. The bounded typed request retains scalar intent and pointers, not opaque bytes. Host encoding resolves the host driver's layout independently; zero all padding/routes, replace client/channel with owned handles. Unsupported host layouts refuse rather than truncating pointers. The CPU pool-control pointer has a distinct address domain that this GPU-leaf adapter cannot validate: a nonzero entry in that slot refuses. Other pointer slots use the corresponding host buffer-property engine ID, including SETUP where defined. For the main preemption buffer, query requested graphics and compute requirements and validate the maximum if both are selected. Flags/modes outside the SDK domains refuse; host RM still decides support for legitimate modes.

Resource policy is a trusted bound, not a hardware fact: at most9 pointers,64MiB per buffer,128MiB total validated extent and32768 placement pieces, with a bounded queue. Native RM refusals are preserved. Zero pointers confer no memory authority; mode-required buffer presence is checked where specified, and unsupported combinations can still be refused by RM. A query failure cannot silently become a zero-sized success.

Only the VA worker performs query/bind. It already serializes placement/reconciliation; serve one queued action only once older mapping work is settled, between VA turns. Copy/validate bounded placement metadata, then release row/registry/graph locks before native calls. No row guard crosses RM. The private USER publication/lifetime guard excludes native Free for that channel and is never taken by a vCPU. A busy guard parks the request; no spin or new polling timer. A pending reply retains the original allocation identity and private lease, so recycled handles cannot redirect an action. Guest Free/reset cancels queued old allocations; old requests fail without calling RM. A host operation already in progress finishes on the still-owned old channel before physical Free can run.

Ordering uses the existing USER prefix witness and asynchronous pending-action cell. No suffix is published until the real query/bind result reaches the worker. Completion wakes the original worker through the existing plane/notifier seam. A bind error is a real refusal, not an ACK probe; registry execution/deletion semantics follow the existing action result handling. `KF3_TSG_FACTS=1` takes an owned-channel mode readback after the bind, with its exact status.

## Scope, sharing and lifetime

**CODE READ:** the TSG mirror owns one host TSG per guest TSG and an explicit host ctxshare per used guest share; several members can share context state. A native bind names an owned channel, not an arbitrary token or every channel in the TSG. Let host RM resolve its context/subcontext scope; do not issue repeated per-member binds with incompatible VASes or collapse ASYNC shares into VEID0. Existing standalone ctxsw setter semantics are preserved.

The adapter allocates no host buffers. The VM's guest store and mapped RAM objects already own the backing; bound native context lifetime is the native RM channel/TSG lifetime. Guest-controlled later invalidation remains an ordinary modification of that same VM's mapping, processed by the existing VA manager; this adapter never silently moves a VA to keep a bind working. A malicious guest can fault its own ordinary channel, as an unprivileged host process can. No cross-VM object/address is available through this path.

Teardown cancels pending requests before native Free. Hold the existing record/private-storage lifetime through a running bind; only successful native Free releases channel ownership/private storage under the existing production teardown. Then release pending metadata/cells/Mirror references. Do not free a guest-owned memory allocation because it appeared in a bind. Native Free failure retains/quarantines the existing resources. Tests cover canceled/stale requests, refusal and metadata release; the adapter has no independent buffer allocator to roll back.

Alternative **(b)**, host-owned device-local buffers, remains designed but unimplemented: use the same NP host query, allocate/map each owned buffer in a private reserved range with the existing VA-manager exclusion protocol, bind only after all resources are ready, retain all mappings on an uncertain native outcome, and unmap/free only after the last sharing context has really been freed. Key ownership by actual shared context/ctxshare/TSG generation, not just a guest handle. Pool-control CPU mappings and any required buffer initialization need their own contract. This alternative avoids guest mapping churn but adds private writable mappings and cross-member lifetime work; the current bounded VA-preserving path does not require those changes. The task's owner authorizes host-only objects, but that is not a substitute for this lifetime/initialization proof.

## Test-first acceptance and counters

Before implementation, publish a design/interface-only refusal seam (no runtime caller), then commit failing tests alone as EXPECTED RED. Tests drive the production host seam with a fake: GFXP/preemption, zcull and PM requests must query and bind an authored owned channel; malformed/unknown sizes/ranges must not bind; exact status must propagate; addresses and placement rows remain unchanged; cancellation/Free prevents stale actions and drops request references; no new work occurs with the flag off. Real decoder/layout tests cover both guest and host versions. The USER branch/VA-loop integration is checked separately so a correct isolated helper cannot conceal missing wiring.

Heartbeat counters when enabled: binds served, binds refused and successfully validated/bound bytes. They count host outcomes, not registration ACKs or allocated bytes, and must not change decisions. At least two independent tests must kill each behavior mutant, including no bind, validation bypass, wrong owned handle/VA, swallowed RM error, early suffix/completion, canceled target and wrong counters/default gate. No hardware result is claimed by a fake test.

## Hardware plan, after review and GPU-free green

1. Same binary, same host/guest drivers and device; preregister OFF/ON arms with `KF3_TSG_FACTS=1`. First ordinary NP query/bind/get on owned objects, preserving exact statuses. Stop on instability/new host Xid and preserve evidence; never enable globally from this design alone.
2. Linux guest A/B: use an explicitly authorized USER-relay diagnostic route, not a new Linux Passthrough mapping. Run a long graphics/compute workload with concurrent host CUDA work; test contention in both directions. Record guest bind request/trigger, native results, effective gfx/compute/zcull modes, workload and host-kernel GPU timestamps plus enqueue/finish wall times. Check the host walker context's own compute preemption independently. A lack of GR bind triggers makes that arm non-discriminating.
3. Windows Enscape OFF/ON boots using the exact green binary and fixed sparse configuration, same recipe; capture all binds/readbacks, late-subtest progress, guest153/141, host Xids, walker GPU versus wall latency and layout refresh. A passing boot alone is not causal proof; repeated matched arms and a corresponding effective-state/latency change are required.
4. If effective state already matches OFF, stop calling this a missing-preemption explanation. If supported bind succeeds but effective state or latency does not change, investigate queue/engine identity and memory work rather than changing TDR timers. Keep `KF3_GR_BINDS` default OFF until owner-reviewed hardware validation.

Source provenance: NVIDIA official580.159.04 commit `b81d58ee0224d1d290bef1c080592b619e184042`,595.84 tag commit `722ae84526a09ed672fbe75448e2909834ba4cce`; `ctrl2080gr.h`, `ctrl0080fifo.h`, `nvos.h`, `kernel_channel.c`, `kernel_graphics_context.c`, `kernel_graphics.c` and generated export tables. Prior parity audit source/evidence is on `codex/pf1-tsgparity-20261026` commit `5fa2bc567`. No closed-driver disassembly or private benchmark scores are included here.
