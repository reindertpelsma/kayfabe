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

## Results

_(appended after the run; see `results.txt` for the filtered raw logs.)_
