# b3: opt-in external fault service in host nvidia-uvm

**STATUS: HOST-ONLY PROOF DONE — PASSING ON HARDWARE, 2026-09-30.** The owner selected b3 and
requires full ordinary host CUDA coexistence. The opt-in nvidia-uvm patch is **built and measured**
(`tools/uvm_efs/`, evidence `traces/v3_uvm_b3/`): a real compute kernel faults in an EFS VA space,
the fault is delivered to the registering process instead of being cancelled, the process maps the
page, a replay completes the kernel with correct data; a scoped cancel fails only that VA space;
native host CUDA (matmul + real managed-memory demand paging) is unaffected throughout. This is the
**privileged half only** — no guest, no kayfabe VMM. Per the owner's standing rule this does **not**
claim guest managed-memory support; that needs the guest fault plane (`V3_UVM_DEMAND_PAGING.md`
§5, "What the guest must see") wired on top. The preflight below (2026-09-28) is preserved; where the experiment settled one
of its open questions the answer is folded in **above** it, in "§0 Result".

Reference: NVIDIA open-gpu-kernel-modules 580.159.04, commit
`b81d58ee0224d1d290bef1c080592b619e184042`. Paths below are relative to that source. Kayfabe's
shared reference checkout was read only. Latest historical research is `v3-uvm-e6pp` at `c6765f5c`.

## §0 Result — the built patch and what the host-only experiment measured (2026-09-30)

Built from source on a rented GA106 box, host **open 580.159.04** (`nvidia-uvm.ko` sha256
`2ca52cad872382e6…`, patch sha256 `c4b06fb9…`), Linux 6.8.0-59, CUDA 12.6. Full matrix, kernel
shutdown accounting, Xid census and latency in `traces/v3_uvm_b3/`.

### 0.1 The patch (`tools/uvm_efs/patch/`, ~1.3 kLoC: one new source + ~90 lines of hooks)

A per-`va_space` mode, **EXTERNAL FAULT SERVICE (EFS)**, entirely in `uvm_efs.c` plus a handful of
hook lines in stock files. Off unless the admin loads the module with `uvm_efs_enable=1` **and** a
UVM file opts in at `UVM_INITIALIZE` (`UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE`, which then **requires**
HMM off — `DISABLE_HMM` or `MULTI_PROCESS_SHARING_MODE`). In such a VA space, a replayable fault
that stock UVM would mark fatal because no UVM range can service it
(`service_fault_batch_dispatch`, `NV_ERR_INVALID_ADDRESS`) is diverted: its `clc369` packet is
copied into a bounded per-file table, the file's owner is woken, and the access stays pending in
hardware. The owner reads records (`UVM_EFS_WAIT`) and resolves them (`UVM_EFS_RESOLVE`, REPLAY or
CANCEL). Everything else — managed faults, fatal fault *types*, prefetch faults, non-replayable
faults, every other VA space, every other process — is untouched stock code. The three hook sites
are the batch dispatch's `else` branch, `uvm_api_initialize`, and the teardown paths.

★ **The publish executor is the stock external-mapping ioctls, and it was exercised two ways.** The
owner's process maps a faulted page with `cuMemCreate` + `cuMemMap` (libcuda ⇒
`UVM_MAP_EXTERNAL_ALLOCATION`), *or* — the kayfabe shape — with kayfabe's **own** RM vidmem placed
through `UVM_CREATE_EXTERNAL_RANGE` + `UVM_MAP_EXTERNAL_ALLOCATION` on the EFS file directly
(`efs_fault … raw`, `RESULT PASS`, `DATA bad=0`). Both replay to correct data. This is the
§4 conclusion — the twin VAS is UVM-owned and the publish executor moves to the external-mapping
ioctls — shown end to end on hardware.

### 0.2 The registration / lifetime proof (task item 1, now checked and measured 2026-09-30)

*Which object keeps what alive.* The records live in `uvm_efs_va_space_t`, allocated **before**
`uvm_va_space_create` and attached **before** the file is published, so no fault can ever be
attributed to an EFS VA space that lacks EFS state. It is freed at the very end of
`uvm_va_space_destroy`, after the fault bottom-halves were flushed — i.e. it lives exactly as long
as the `uvm_va_space_t`, which lives as long as the file. A `UVM_EFS_WAIT`/`RESOLVE` ioctl holds a
file reference (it runs on the fd), so no resolve can race the destroy. A record holds **no** pointer
into a UVM or RM object: it names its GPU by `uvm_gpu_id_t` + UUID and remembers the GPU VA space
*generation*. `remove_gpu_va_space` bumps that generation under the VA-space write lock and drops
the GPU's records; a later resolve of a stale id finds the generation moved and does nothing. So an
EFS record never keeps hardware channel state alive and never outlives the page directory it would
act on — the preflight's "a UVM channel-memory reference alone does not keep hardware channel state
alive" concern is respected by holding **no** such reference.

*On VMM exit / crash.* Measured 2026-09-30: a process SIGKILLed with a fault parked
(`efs_fault crash`) tears down through `uvm_release → uvm_va_space_destroy`. The **first** step of
that path, before stock UVM stops any channel, is `uvm_efs_va_space_shutdown`, which stops accepting
records, cancels the timeout work synchronously, and cancels every parked fault in hardware. The
kernel logged `parked_at_shutdown=1 hw_cancelled=1` for that tgid and `nvidia-smi` was healthy
afterward (`dmesg_efs_shutdown.txt`). So a dead VMM leaves the GPU in exactly the state stock UVM
would: the faulting channel RCs (Xid 31), nothing is wedged, no record survives.

*On channel teardown while a fault is pending.* Measured (`efs_fault ctxdestroy`): destroying the
CUDA context while a fault is parked completes cleanly (`CTXDESTROY_COMPLETED CUDA_SUCCESS`),
**bounded by the EFS timeout** (~4 s here) — the context holds the GPU until the parked fault is
resolved or the kernel's deadline cancels it, then teardown proceeds. This answers
`V3_UVM_DEMAND_PAGING.md` §10 Q2 ("can a context with parked faults be timed-sliced off, or does it
hold the GPU"): it holds until answered, and the kernel timeout guarantees a bound. In the
integrated product the guest services its own fault in tens of µs, so this bound is a safety net,
not the common path.

*With a second unrelated host CUDA process running.* Measured 2026-09-30 (`coexist` = tiled sgemm +
real `cudaMallocManaged` demand paging, EFS-unaware): **203.6 vs 211.9 iters/s idle (96%)**,
`managed_bad=0`, run concurrently with five back-to-back `efs_fault service 128` (all `RESULT PASS`,
`DATA bad=0`). The ordinary process's managed-memory faults are serviced by stock UVM the whole
time — EFS diverts **only** the opted-in VA space's faults (its records are in a different file's
`efs` struct; attribution is stock UVM's own instance-pointer→channel→va_space map). A batch whose
every fault was parked is not replayed (a replay would only re-raise them); while records are parked
a periodic safety replay keeps any overflow-dropped fault of another tenant moving
(`g_safety_replays` > 0 in the timeout run).

### 0.3 Authorization boundary — what was enforced, and the one item still open

Enforced and measured 2026-09-30 (`efs_auth RESULT PASS`): EFS is refused unless the module is
enabled (`NV_ERR_NOT_SUPPORTED`) and the file asked for HMM-off (`NV_ERR_INVALID_ARGUMENT` otherwise);
`WAIT`/`RESOLVE` on a non-EFS file are `NV_ERR_NOT_SUPPORTED`; **only the initializing thread group**
may `WAIT`/`RESOLVE` — a forked child sharing the inherited fd is refused
(`NV_ERR_INSUFFICIENT_PERMISSIONS`), so a passed EFS fd does not delegate fault service; a
fabricated/never-issued record id is `stale`, no action. The `RESOLVE` ABI carries **only**
kernel-issued opaque ids — never an instance pointer, PDB or host address — and every hardware
action uses the packet and PDB the kernel saved itself. A compromised VMM can therefore only park
its own faults (bounded by the timeout), replay GPU-wide (which re-raises only its own parked
accesses), and cancel via ids the kernel issued for its own VA space; it cannot read or act on
another VA space's records.

⊘ **Not closed, and it is the same gap the preflight and `V3_UVM_DEMAND_PAGING.md` §12.5 already
name.** EFS reuses stock `UVM_REGISTER_GPU_VASPACE`/`UVM_REGISTER_CHANNEL`, which carry the Bug
1624521 TODO: RM's `Dup`/`Retain` path does **not** verify that the caller owns the supplied
`hClient`/handle. EFS does not *worsen* this — a fault is still attributed by stock UVM to whichever
va_space the channel was registered in, and delivered only to that file, only to the initializing
tgid — but it does not *close* it either. For the single-VMM product path (the VMM is the RM client
that created the objects) this is the intended flow. For **multiple mutually-untrusting VMMs sharing
one GPU**, an object-ownership check at registration (compare the registering fd's process, or a
capability token, against the RM client that owns the handle) is still required before b3 is relied
on for VM-to-VM isolation. That is an RM/UVM-interface question for the owner, unchanged by this
experiment, and it is **not** a blocker for the host-only proof or for a single guest.

### 0.4 Latency (GPU PTIMER-calibrated, `service 256`, quiet box)

`hw access→packet` p50 1.8 µs · `packet→parked` (UVM bottom half + divert) p50 94.5 µs ·
`parked→user` (wake + `WAIT`) p50 36.8 µs · **`DELIVERY` packet→user p50 131.2 µs, p99 206.1 µs** ·
`map (cuMemMap)` p50 143 µs · `user→replay` p50 153 µs · `total access→done` p50 288.8 µs,
p99 370.7 µs. A 32-page run measured delivery p50 ≈ 83 µs (`efs_fault service 32`, 2026-09-30,
`run_full.log`); under concurrent coexistence pressure delivery held p50 ≈ 90–117 µs
(`clean_reruns.txt`, the five `service 128` re-runs at `c94601d9`). The negative control
(pre-mapped, `efs_fault negative`) took **zero** faults. The whole run produced 6 Xid-31 events,
all from the six deliberately **unserviced** cases (2× stock, cancel, timeout, crash, ctxdestroy);
**zero** on any serviced run.

⚠ **Of the `service 256` figures above, only `hw access→packet` is in a committed log** (found
2026-09-30, attributing this paragraph for the claim ledger). `run_full.log` section E — the
`service 256` "quiet box, larger sample" run, 2026-09-30, PTIMER-calibrated — reads `packet→parked`
p50 58.5 µs, `parked→user` p50 15.1 µs, DELIVERY p50 73.7 µs / p99 98.8 µs, map p50 125.0 µs,
`user→replay` p50 135.2 µs, total p50 208.9 µs / p99 244.7 µs. The run behind the other figures
above is not committed.

### 0.5 What this does and does not establish

Establishes, on hardware: the divert/park/deliver/map/replay/cancel/timeout/teardown mechanism; the
opt-in and per-tgid authority; the external-mapping publish executor (libcuda's and kayfabe's own RM
objects); and full native host-CUDA coexistence including managed-memory demand paging. Does **not**
establish: anything guest-side (no guest, no VMM ran); multi-VMM VA-space-ownership authentication
(§0.3); the graphics page-kind and per-call TLB-cost questions of §4.4 (compute pages only here);
non-replayable/CE-fault handling (the shader path only). Next is the guest fault plane
(`V3_UVM_DEMAND_PAGING.md` §5) injected into a stock guest, which is where a guest-managed-memory
claim would first be earned.

## First boundary to establish: authority, not fault delivery

`UVM_REGISTER_GPU_VASPACE` accepts user RM handles. In `kernel-open/nvidia-uvm/uvm_va_space.c`
around 1528, the supplied `rm_control_fd` is explicitly discarded beside Bug 1624521 before
`nvUvmInterfaceDupAddressSpace`. `uvm_user_channel.c` around 132 similarly discards the fd before
`nvUvmInterfaceRetainChannel`. These helpers cannot be cited as proof that a new EFS caller owns
the supplied objects.

**Important nuance from this source review:** this is not proof that RM performs no access checks.
`src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:nvGpuOpsDupAddressSpace` passes
`NV04_DUP_HANDLE_FLAGS_REJECT_KERNEL_DUP_PRIVILEGE` to `DupObject`. In
`src/nvidia/src/libraries/resserv/src/rs_client.c:clientCopyResource_IMPL`, that flag forces an
`RS_ACCESS_DUP_OBJECT` rights check even for a kernel caller. The destination client, sharing
policy, credentials and retained-object lifetime still need tracing to establish the exact scope.
`nvGpuOpsRetainChannel` takes a different path through `_nvGpuOpsLocksAcquireAll` and
`nvGpuOpsVerifyChannel`; do not infer its authorization from the duplication path.

Also, `osValidateClientTokens` in `src/nvidia/arch/nvalloc/unix/src/os.c` accepts matching EUID
**or** matching PID. That source fact is not automatically the effective check for every helper.
It does show why "a credentials check exists" is not sufficient evidence for strict per-process
or per-VM isolation, especially when multiple VMMs share a UID.

Before exposing EFS to an untrusted VMM, establish an authenticated, lifetime-pinned relationship
between its UVM file, RM address space and registered channels. An FD's integer value or a supplied
`hClient` is not that relationship. Reject mismatched, forged and stale objects; define explicitly
whether intentional FD transfer delegates authority. If this cannot be implemented within UVM's
available interface, document the smallest required RM-interface change before widening the patch.
Do not hide an unauthenticated registration behind a default-off switch and call it secure.

### Follow-up source trace: checks and lifetime are distinct

The next source pass narrows the questions above (same pinned 580.159.04 source):

- `rmapi/sharing.c:serverInitGlobalSharePolicies` installs a default `RS_SHARE_TYPE_PID`
  duplication policy. `rmapi/client_resource.c:cliresShareCallback_IMPL` checks the current
  process against the source client's PID when the destination is a kernel client and a parent
  context is present. `resserv/rs_client.c:clientCopyResource_IMPL` supplies that parent context;
  `resserv/rs_access_map.c` carries it into the sharing callback. This is materially stronger
  than assuming the same-EUID token helper is the only check. Policies can be overridden or
  explicitly shared, so test the actual duplication path, including deliberate delegation.
- The channel-retention path's `serverAcquireClient` takes a client lock; it is not itself a
  caller-credential check. However, `nvGpuOpsVerifyChannel` compares the channel's `pVAS` with
  the previously duplicated address-space object and rejects mismatches. Thus channel retention
  is not an unrestricted handle lookup either. Any b3 design must preserve that binding, not
  introduce a second raw-handle cancellation interface.
- Retaining a `uvm_user_channel_t` reference is **not sufficient to pin usable RM channel state**.
  `uvm_user_channel.h` explicitly permits detach while that memory object remains retained.
  `uvm_user_channel_detach` removes fault lookup, defers hardware-resource release until fault
  buffers are flushed, and clears `gpu_va_space`; `uvm_user_channel_destroy_detached` waits for
  the clear-faulted tracker, releases the RM-retained channel, then drops the memory reference.
  An EFS record must become invalid during detach, under the appropriate locks, and userspace
  replies must revalidate generation/liveness before accessing channel or VA-space state.

**Implementation implication, not yet a demonstrated authorization result:** prefer reusing the
existing checked UVM registration and retained-object lifecycle. Run same-process, foreign
same-UID, foreign different-UID and explicit-sharing negative/positive controls before deciding
that a new RM export is necessary. Do not defer PID-sensitive registration to a kernel worker
without tracing how the originating authority is carried. Separately define EFS file transfer
semantics and prove pending replies cannot outlive the channel's hardware resources.

## Incremental patch shape

1. **Opt-in state and authenticated registration.** Default stock behavior. Admin enables the
   facility; a separately opened UVM file opts in before registrations. Keep native CUDA's own UVM
   file out of this mode. Disable HMM/managed-range interpretation for the guest twin address space,
   so a guest GPU VA can never become an implicit host VMM pointer.
2. **Bounded fault records after existing attribution.** In the replayable fault batch path, after
   `uvm_parent_gpu_fault_entry_to_va_space` resolves ownership, divert only opted-in faults to that
   file's bounded queue. Keep kernel-owned channel/VA-space references and a generation with each
   opaque record ID. Do not return instance pointers, page-directory addresses or host pointers.
   Do not block the shared fault batch/interrupt path while userspace services the guest.
3. **Repair, replay and cancellation.** Use existing UVM external mappings for the twin and existing
   hardware helpers. Replay may have GPU-wide effects: rate-limit it and test neighboring native
   CUDA, rather than claiming it is automatically per-VM. Cancellation uses only the kernel's saved
   target. Queue exhaustion, process death and deadlines have explicit bounded outcomes.
4. **Non-replayable fault handling.** Copy-engine faults are not a substitute for the replayable
   shader proof. Handle their reporting/clear-faulted path separately, with correct channel scope.
5. **Guest integration only after the host proof.** Inject the guest's expected fault packet and
   interrupt, let stock guest UVM resolve it, mirror the resulting mapping, then replay. Preserve
   the GPU data plane and CUDA-based walker; do not introduce an N4-style replacement launcher.

The external-map executor also needs independent checks for graphics page-kind representation and
per-call TLB cost; see `V3_UVM_DEMAND_PAGING.md` §4.4. Do not silently replace all existing mapping
behavior before those constraints are understood.

## First experiment: disposable host-only proof

Use a dedicated KVM-capable rental, pinned matching driver source/userspace and kernel headers.
Never unload/replace a driver on the shared physical desktop or Paguro Windows host. Bring back
source patches and text results to trusted storage/GitHub after each run; never rely on the rental.

| Case | Required evidence |
|---|---|
| Stock/default-off | Native CUDA allocation, compute, streams/graphs, managed first-touch and pageable/HMM control retain correct results; patch-disabled baseline recorded |
| Authenticated EFS registration | Valid own objects accepted; foreign same-UID and different-UID test objects, stale handles, mismatched GPU/VA space and deliberate races rejected or explicitly authorized by a reviewed capability rule |
| Real shader replayable fault | Missing mapping produces a queued record for the correct EFS file; no fabricated completion and no use of a CE fault as proof |
| Repair/replay | Real mapping repair, replay and synchronized correct data; record median/p99 fault delivery/service/completion separately |
| Scoped cancel | Only the intended faulting channel is cancelled; another EFS channel and a native host CUDA workload complete correct data |
| Native host coexistence | Native CUDA and managed/pageable-memory tests run concurrently with EFS fault pressure; no module takeover, global HMM disable or ordinary-host feature removal |
| Abuse/lifecycle | Queue saturation, replay storm, EFS fd/process close, channel unregister/reuse, timeout and module teardown are bounded; no stale records or host use-after-free |

A host-only CUDA allocator experiment must establish which UVM file owns the CUDA-created VA space;
opening a second EFS fd does not magically opt libcuda's existing address space in. The research's
small `cuMemMap` example is a proposed proof shape, not an already-working registration recipe.
Write the ownership and launch setup down before using its success/failure to judge b3.

## Current next action

⊘ **SUPERSEDED 2026-09-30 by §0 Result.** The opt-in path *and* diversion/replay/cancel/timeout/
teardown were implemented and adversarially tested together on hardware; the case table above is
satisfied for the compute-shader path except the two items §0.3/§0.5 keep open. The registration
row's "foreign same-UID/different-UID object" authentication is the one deliberately **not** closed:
§0.3 explains it is the pre-existing stock Bug-1624521 gap, needed only for multiple mutually-
untrusting VMMs, and it is an owner/RM-interface question, not host-only-proof work.

Next, in order: (1) the **guest fault plane** (`V3_UVM_DEMAND_PAGING.md` §5) — inject the guest's
`clc369` packet + interrupt, let stock guest UVM service and replay, mirror the mapping — wired onto
this EFS host source; only when a guest `cudaMallocManaged` first touch completes correctly is a
guest-managed-memory claim earned. (2) The graphics **page-kind** and per-call **TLB-cost** checks
of §4.4 (compute pages only were mapped here). (3) **Non-replayable/CE** fault handling (§4/step 4),
a separate path from this shader proof. (4) For multi-VMM, the object-ownership check of §0.3.
