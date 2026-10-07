# NV50_DEFERRED_API (class 0x5080) passthrough-safety falsification

**STATUS: RESEARCH, 2026-10-07.** Written BEFORE the first run (per working rule
"every hypothesis states its falsifier before the run"). Measured results are
appended below the falsifier list after the run; MEASURED and INFERRED are kept
apart.

This is a read-only security probe of the REAL host RM as an ordinary client. It
changes no kayfabe production code path. It exists to answer whether class 5080
can be forwarded in a Passthrough channel without a cross-object / cross-channel /
cross-client / privilege-escalation / DoS hazard.

## Hardware / software under test

- Host: `root@172.22.1.20` (owner hardware), builds under `/var/lib/kf-windows-20261005/`.
- GPU: NVIDIA GeForce RTX 4070 (Ada / GB-less AD104), bound to the `nvidia` driver.
- Host driver: **595.91.07** (NOTE: the source reference is ogkm 580.95.05; the
  cross-object dispatch from PBDMA→object is physical-RM/GSP and is NOT in open
  source, so it is MEASURED here, not derived).
- Test binary: `crates/kf-harness/src/bin/kf-deferred-falsify.rs`, an ordinary
  (non-QEMU, non-privileged-verb) RM client. F4 is additionally run as a non-root
  user.

## Background (MEASURED facts from ogkm 580.95.05, deferred_api.c)

- `NV50_DEFERRED_API_CLASS` 0x5080 is a software-channel descendant.
- Registration: control `NV5080_CTRL_CMD_DEFERRED_API_V2` (0x50800103,
  NON_PRIVILEGED) copies a bundle {hApiHandle, cmd, flags, hClientVA, hDeviceVA,
  api_bundle} into a **per-OBJECT** btree keyed by the client-chosen 32-bit
  `hApiHandle`, and records the caller's privLevel. The inner `cmd` is NOT checked
  at registration.
- Trigger: software method 0x200 (data = hApiHandle) in a pushbuffer on the
  subchannel bound to the 5080 object. Handler `_class5080DeferredApiV2` looks
  Data up in THAT object's list only, runs the stored control with the recorded
  privLevel, then deletes the entry (DELETE_IMPLICIT) unless flags say EXPLICIT /
  WAIT_FOR_TLB_FLUSH. Unknown handle returns `NV_ERR_INVALID_DATA`.
- Privilege of the 8 accepted commands:
  - NON_PRIVILEGED: DMA_INVALIDATE_TLB, GR_CTXSW_ZCULL_BIND, GR_CTXSW_PM_BIND,
    GR_CTXSW_PREEMPTION_BIND.
  - PRIVILEGED: GPU_PROMOTE_CTX, GPU_INITIALIZE_CTX, FIFO_UPDATE_CHANNEL_INFO.
  - kernel-only: GPU_EVICT_CTX.

## The observable (how we tell "executed" from "untouched")

`AddDeferredApi` returns `NV_ERR_INVALID_OBJECT_HANDLE` when the `hApiHandle`
already exists in the object's btree. With DELETE_IMPLICIT:

- **"entry executed"**  == re-registering the SAME hApiHandle on the SAME object
  now SUCCEEDS (the trigger deleted it).
- **"entry untouched"** == re-registering the same hApiHandle FAILS with
  `NV_ERR_INVALID_OBJECT_HANDLE` (it is still in the btree).

We fence/drain after firing 0x200 before re-registering, so an async handler has
run. We also test registering the SAME handle number on two different objects, to
see whether the client-namespace validation allows it (btree is per-object).

## The hypothesis to FALSIFY (the owner's worry)

> H: In a Passthrough channel, whose pushbuffer the host never inspects, a
> guest-chosen 0x200 + handle can make the host execute a deferred-API entry that
> the guest's own channel/object/client did not register, OR run something with
> more privilege than its registrant, OR cause cross-tenant harm. Unlike a
> doorbell token the handle cannot be selected/translated in a host call, and we
> cannot translate a ring.

A SINGLE counterexample proves H. H is **falsified** only if ALL of F1–F6 hold on
real hardware. Report the result EITHER WAY.

### Falsifier list (stated before the run)

- **F1 (cross-object, same channel/client).** An entry registered on object O_A is
  NOT executed by 0x200 fired on a different 5080 object O_B (same channel, same
  client). PASS = O_A's entry stays "untouched" after firing on O_B.
- **F2 (cross-channel, same client).** Same, but 0x200 is fired from another
  channel's pushbuffer (O_B lives on channel C2, O_A on C1). PASS = O_A untouched.
- **F3 (cross-client).** Same, but 0x200 is fired by a DIFFERENT RM client — a
  second process. PASS = O_A (in process 1) untouched after process 2 fires.
- **F4 (no escalation).** A non-root client that registers a PRIVILEGED / kernel
  cmd (GPU_PROMOTE_CTX, GPU_INITIALIZE_CTX, FIFO_UPDATE_CHANNEL_INFO,
  GPU_EVICT_CTX) and fires it gets a FAILURE at trigger and the operation has no
  effect. PASS = entry is consumed-with-failure (or rejected) AND no privileged
  state changed. (Inner cmd is not checked at registration, so registration may
  succeed; the privLevel gate is at trigger.)
- **F5 (garbage handles).** Unregistered handles (0, 1, 0xffffffff, 0x40000000+N
  for N up to a few hundred, and the values of this client's OTHER real RM object
  handles — channel / memory / VA-space) never execute anything. Record exactly
  what the firing channel does (error / Xid / RC / killed) and that OTHER
  channels and clients stay alive. PASS = nothing executes, blast radius is at
  most the firing channel.
- **F6 (rate / DoS).** Firing a valid EXPLICIT-delete entry (DMA_INVALIDATE_TLB on
  the client's own VA space) in a tight loop for a long time does NOT degrade a
  concurrent well-behaved channel's latency/throughput disproportionately versus
  an equivalent spam of ordinary methods (semaphore releases / plain flush).
  Measure both against a no-contention baseline. PASS = degradation is not
  disproportionate vs the ordinary-method spam.

### Verdict rule

- If ALL of F1–F6 PASS → **H is FALSE** (not a blocker for forwarding 5080 in
  Passthrough). State residual concerns separately.
- If ANY Fk FAILS → **H is TRUE / partially TRUE** (a blocker). State exactly
  which channel kinds (Passthrough / Translated) are affected and how.

## Results (run 2026-10-07, commit `756ba678`, driver 595.91.07, RTX 4070)

Raw evidence: `results.txt` (DF_* lines + host dmesg Xid/NVRM lines). The suite ran
as non-root user `vmmtest2` (euid 1001), sharing the GPU with the live host
desktop (nvidia-smi 340 MiB / 0 %, unchanged before and after).

### What blocked the trigger half (be explicit)

The native probe allocates a real 5080 object (MEASURED `class_id=0x5080`,
`engine_id=0x22` = the SW engine) under a **COPY0 (CE) channel** built by
`kf_chan::HostRing::new`. A **bare fence on that channel completes**
(`DF_P0_BAREFENCE bare_fence_completed=true`), so the ring works. But
`SET_OBJECT` naming the 5080 object's `classEngineID` (=0x1) on that CE channel
**halts the channel with Xid 32** (`DF_P0_SETOBJECT bind_fence_ok=false`): the SW
engine is not on the COPY0 runlist, so the method is rejected by the host PBDMA.
⇒ Every phase that must *trigger* a registered entry (P0, F1, F2, F3, the trigger
side of F5, F6) is **NOT MEASURED** here. Triggering needs the 5080 object on a
GR/compute-capable channel (a GR-context channel — heavier; the clear next step).
This is a harness limitation, **not** an RM refusal of registration.

### Per-falsifier outcome

| F | Status | Evidence |
|---|--------|----------|
| **F7a** arbitrary-handle registrability | **MEASURED — registers verbatim** | A host client registers almost any `hApiHandle`: 1, 0x40000000(+N), 0xffffffff, 0xcafe0000, even the auto-generator-range 0xcaf00000. REJECTED (all as `INVALID_OBJECT_HANDLE` 0x33): `0`, `0xcafe0001` (== this client's hClient), the FW-reserved range `0xc9f00000` ([0xc9f00000,0xc9f7ffff]), and the client's own live resource handles (channel/tsg/vaspace/5080 object). Matches ogkm `clientValidateNewResourceHandle` exactly. |
| **F7b** dedicated client ⇒ own namespace | **MEASURED — independent** | The same arbitrary handle `0x40000000` registers in two independent clients *concurrently* (`registered_in_client1=true registered_in_client2_concurrently=true`). (Artifact: kf-host mints identical *structural* handles per client, so a structural handle collides in both — reported, not a real negative.) |
| **F4** no escalation | **REGISTRATION MEASURED / dispatch INFERRED** | As non-root (euid 1001): `GPU_PROMOTE_CTX`, `GPU_INITIALIZE_CTX`, `GPU_EVICT_CTX` **register OK** (inner cmd not checked at registration — confirms ogkm). `FIFO_UPDATE_CHANNEL_INFO` is **rejected at registration** (0x33) because its V2 handler validates the bundle's `hClient`/`hUserdMemory` (ogkm `deferred_api.c:356`, seen in dmesg). The dispatch-time privilege gate (entry runs with the registrant's recorded user privLevel ⇒ privileged control fails) is **INFERRED** from ogkm (`deferred_api.c:575` + `serverControl_Prologue`), not measured (trigger blocked). |
| **F8** stray 0x200 / SET_OBJECT 0x5080, no object | **MEASURED — self-harm only** | Firing 0x200 on an unbound subchannel (no 5080 object) → **Xid 32 on the firing channel only**, channel RC'd/dead (`last_alive=false`); the firing *client* recovers (`firing_channel_recovered=true`); a concurrent bystander client's registered entry is **untouched** and the client **alive** (`victim_untouched=true victim_alive=true`). Directly supports the "refuse 5080 on Passthrough" fallback being safe. |
| **F1/F2/F3** cross-object/channel/client | **NOT MEASURED (trigger blocked) — INFERRED isolated** | ogkm: the entry btree is **per-OBJECT** (`deferred_api.c:104`); the 0x200 handler looks up Data only in *that* object's list (`:129`, unknown ⇒ `INVALID_DATA`); and it dispatches on the registrant object's **own** client/subdevice (`:556 RES_GET_CLIENT`) with the registrant's privLevel (`:575`). Cross-object/channel/client execution is structurally impossible — but this is INFERRED, not measured on hardware. |
| **F5** garbage handles | **Isolation MEASURED (via F8) / lookup INFERRED** | Bystander isolation is shown by F8. "Unknown handle ⇒ no execute, entry untouched, `INVALID_DATA`" is INFERRED from `deferred_api.c:129-135`. |
| **F6** rate/DoS | **NOT MEASURED (trigger blocked)** | Needs a working trigger on a GR/compute channel. |

### Host-kernel signals from the provoked failures (coordinator's question)

All MEASURED from host dmesg (`results.txt`) / nvidia-smi:

1. **Exact code + component.** Every malformed-pushbuffer failure → **Xid 32**
   ("Invalid or corrupted push buffer stream", PBDMA), logged by **kernel RM**
   (`NVRM: Xid (PCI:0000:01:00): 32, name=kf-deferred-fal, channel 0x…,
   intr1 80000000`). Control-level failures (bad bundle hClient, duplicate/unknown
   handle) are kernel-RM soft checks `nvCheckOkFailedNoLog … INVALID_OBJECT_HANDLE
   (0x33) @ deferred_api.c:356/:388`; `:388` is the V2 handler forwarding to GSP via
   `DEFERRED_API_INTERNAL`. No GSP-only crash.
2. **Channel fate.** The faulted channel is **RC'd / killed** (fence never completes;
   `last_alive=false`). The host RM **continues**. Re-enabling the *same* channel was
   not tested (INFERRED: stays in RC until freed); the client recovers by allocating a
   fresh channel/object (`firing_channel_recovered=true`).
3. **Blast radius.** Firing channel only. A concurrent bystander **client** (its
   registered entry and its liveness) is unaffected; the **host desktop** (gnome-shell
   et al. sharing the GPU) kept running; nvidia-smi 340 MiB/0 % before and after.
4. **Latency.** Xid is logged essentially when the PBDMA processes the bad method
   (paired lines ~1.5 ms apart); the 5 s cadence between events is the probe's own
   poll-timeout, not Xid latency.
5. **MMU fault / nvidia-uvm.** **None.** Only Xid 32 (PBDMA); no Xid 31 or any MMU
   fault, nothing replayable, nothing reaching nvidia-uvm.

So the host kernel module **does** get an actionable exception (the Xid 32 interrupt
+ RC) and **resumes from it** (RCs the one channel, keeps serving everyone else).

### Verdict on H

- **The owner's REFINED core worry — "the host cannot hold an entry under exactly the
  guest-chosen, unremappable handle H" — is FALSE (MEASURED).** A host RM client
  registers the guest's chosen H *verbatim* (F7a), registration is a control call to
  the emulated GSP so H is known before use (not pulled from an untranslated ring), and
  a dedicated host client per guest gives an independent handle namespace (F7b). The
  only values a client cannot register are `0`, its own `hClient`, the FW-reserved range
  `[0xc9f00000,0xc9f7ffff]`, and its own live resource handles — all of which kayfabe
  controls and which the guest's identical restrict range forbids it from choosing too.
  **No ring translation of the handle is needed.**
- **The ORIGINAL cross-object/channel/client/escalation worry (F1–F4) is INFERRED
  FALSE** on a strong ogkm basis (per-object btree, dispatch on the registrant's own
  client/subdevice with the registrant's privLevel), but is **NOT MEASURED** on this
  host: the SW-method trigger needs the 5080 object on a GR/compute-capable channel,
  and on a CE channel `SET_OBJECT` halts with Xid 32. Closing this gap (a GR-context
  channel) is the one remaining measurement.

### Channel kinds affected (if any)

On the measured axes, **neither Passthrough nor Translated is shown to be a blocker.**
Residual, self-harm-only risk (MEASURED via F8): a guest 0x200 that is malformed, or a
SET_OBJECT of a SW class on an incompatible channel, Xid-32-RCs **its own** channel —
consistent with owner ruling Q11 (self-harm, not inducible across isolation). This is
identical to any other bad pushbuffer method a Passthrough guest can already send.
