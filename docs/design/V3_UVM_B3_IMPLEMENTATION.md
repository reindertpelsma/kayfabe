# b3: opt-in external fault service in host nvidia-uvm

**STATUS: IMPLEMENTATION PREFLIGHT, 2026-09-28.** The owner selected b3 and requires full ordinary
host CUDA coexistence. No kernel patch has been built or loaded in this resumption. N4 takeover
is no longer the next experiment. This narrows the implementation and verification order; it does
not claim managed-memory support is implemented.

Reference: NVIDIA open-gpu-kernel-modules 580.159.04, commit
`b81d58ee0224d1d290bef1c080592b619e184042`. Paths below are relative to that source. Kayfabe's
shared reference checkout was read only. Latest historical research is `v3-uvm-e6pp` at `c6765f5c`.

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

Finish the precise RM sharing/credential/FD-lifetime trace, then implement and adversarially test
the smallest authenticated opt-in path. Only after that add diversion/replay. No further product
choice is needed from the owner; host CUDA coexistence and the limited privileged exception are
settled. Implementation feasibility and safety remain obligations of this work.
