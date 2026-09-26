# UVM demand paging in a kf3 guest WITHOUT patching NVIDIA — mechanism survey

**STATUS: RESEARCH, 2026-09-26.** Companion to `docs/design/V3_UVM_DEMAND_PAGING.md` (§11 summarises
this file). Question: *is there any route to guest demand paging that leaves every NVIDIA
component stock — no nvidia-uvm patch, no nvidia.ko patch?* Every mechanism the host and guest
stacks expose was enumerated and checked against ogkm-580.159.04 source
(`research_clones/ogkm-580.159.04` in the nvkvm tree; paths relative to it). Evidence rows are in
`facts.tsv` beside this file. Tags as in the parent note: `[src]` read, `[meas]` measured,
`[inf]` inferred, `[ext]` external documentation.

## Answer in one paragraph

Two no-patch shapes exist and only one of them is buildable. **(N4) a kayfabe kernel module
that takes nvidia-uvm's place on nvidia.ko's exported `nvUvmInterface*` contract** is feasible:
it needs no NVIDIA source change, but it excludes host nvidia-uvm (one fault-buffer owner, one
callback registrant), so host CUDA disappears and the walker must launch through raw RM instead
of libcuda. **(N1) switching the guest to the pre-Pascal "basic" managed model** would remove
demand faults entirely, but the switch lives inside the proprietary libcuda (OS + architecture),
and nothing kayfabe answers reaches it. Everything else — root userspace owning the twin's page
tables, a blocking pager under host HMM, userfaultfd, UVM test/tools ioctls, module parameters,
fault-interrupt ownership, ATS — is closed by a specific line of stock code, listed below.

## The two walls every no-patch route hits

**Wall 1 — the fault record never leaves the kernel except through the buffer owner.** `[src]`
- One replayable fault buffer per GPU function, kernel-privileged class, nvidia-uvm holds it from
  kayfabe's own `cuInit` onward (parent note §4.1).
- A fault whose instance pointer nvidia-uvm does not know (a channel never registered with it)
  is neither serviced nor cancelled: UVM asserts `NV_ERR_INVALID_CHANNEL`, **flushes the whole
  buffer, issues a GPU-wide `REPLAY_START`, and restarts the batch**
  (`kernel-open/nvidia-uvm/uvm_gpu_replayable_faults.c:1088-1107`, replay type
  `UVM_FAULT_REPLAY_TYPE_START` at `:1100`). The stalled access re-issues, faults again, and the
  loop repeats until the page is mapped. That is the *"what looks like a hang on the GPU until
  the app is killed"* RM warns about (`src/nvidia/src/kernel/gpu/mem_mgr/vaspace_api.c:681-685`).
  No record is emitted: tools events are per-va_space, and the fault has none.
- RM's own replayable-fault path only notifies the buffer object's client
  (`kern_gmmu_tu102.c:262-281`), and `NV2080_CTRL_CMD_MC_CHANGE_REPLAYABLE_FAULT_OWNERSHIP` only
  moves an interrupt-mask bit (`kern_gmmu_gv100.c:84-105`).
- The exported callback registration is single-owner: `nvUvmInterfaceRegisterUvmCallbacks`
  returns `NV_ERR_IN_USE` if a registrant exists (`kernel-open/nvidia/nv_uvm_interface.c:1051-1063`).
⇒ Whoever wants the record must be *the* kernel registrant. That is nvidia-uvm (patched, = b3) or
a module in its place (N4). Userspace, root or not, never sees it.

**Wall 2 — a compute channel in a fault-capable VAS runs only if a kernel-interface caller
bound it.** `[src]`
- `ENABLE_PAGE_FAULTING` on a VAS requires `IS_EXTERNALLY_OWNED`
  (`vaspace_api.c:686-690`). Both are ordinary allocation flags; RM does not restrict them to
  kernel clients.
- Setting that VAS's page directory from userspace **is allowed for root**:
  `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` carries `RMCTRL_FLAGS_PRIVILEGED` (flags `0x14004`,
  `src/nvidia/generated/g_device_nvoc.c:886-890`; `PRIVILEGED` = admin user or kernel,
  `src/nvidia/inc/kernel/rmapi/control.h:196-202`). The implementation has no kernel-client
  check (`src/nvidia/src/kernel/gpu/mem_mgr/dma.c:427-520`). `UNSET_PAGE_DIRECTORY` is the same.
- A root client's channels are automatically privileged (`kernel_channel.c:281-287`), so kayfabe
  could push `MEM_OP` replay/cancel itself.
- **But** RM refuses to schedule a GR channel in an externally owned VAS until
  `bIsContextBound` (`kernel_channel.c:2200-2206`, enforced at TSG schedule
  `kernel_channel_group_api.c:1107` and `kernel_channel.c:3105`), and the **only** setter is the
  kernel-interface bind `nvGpuOpsBindChannelResources` (`src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:10903`).
  `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` from userspace is admin-callable (`0x10244`) and does the same
  promotion, but does not set the flag. `NV90F1_CTRL_CMD_VASPACE_RESERVE_ENTRIES` /
  `GET_PAGE_LEVEL_INFO` are kernel-privileged (`0x18000`, no PRIVILEGED bit).
⇒ A root-userspace-only design can own the twin's page tables but cannot run the guest's compute
channel in them. A kernel module is required regardless, which folds this into N4.

## Options, each closed or opened by a line of source

### N1. Make the guest use the basic (pre-Pascal) managed model — no faults at all

- Guest UVM marks a GPU faultable iff its ISR came up: `uvm_va_space.c:856-858` reads
  `gpu->parent->isr.replayable_faults.handling`. That is true whenever
  `replayable_faults_supported` (hard-coded per architecture: `uvm_pascal.c:89` … `uvm_blackwell.c:80`)
  and fault-buffer init succeeded; an init failure is fatal for the GPU
  (`uvm_gpu_isr.c:352-364`), which is the `cuInit` death already measured (parent §2). `[src]`
- Non-faultable GPUs use the UVM-Lite path (`uvm_va_range.h:253 uvm_lite_gpus`): full-range
  mapping at preferred location, migration driven by explicit calls. It exists and works; the
  guest cannot be steered into it from RM answers. `[src]`
- libcuda applies the basic model on Windows and macOS regardless of compute capability
  (NVIDIA staff, forum thread on Pascal `concurrentManagedAccess=0`), i.e. the decision is OS +
  architecture inside the proprietary library; no UVM ioctl reports fault capability
  (`uvm_ioctl.h`: `UVM_REGISTER_GPU_PARAMS` has none; `UVM_PAGEABLE_MEM_ACCESS` is HMM only).
  `[ext]` + `[src]`
- `CUDA_MANAGED_FORCE_DEVICE_ALLOC` changes *placement* (device-resident allocations), not the
  fault model; a CPU touch still migrates and the next GPU touch still faults. `[ext]`
- ⚠ Not measured: what `um_probe` prints for `concurrentManagedAccess` **inside a kf3 guest**
  (bare metal: 1). Add to E4. If a guest ever reported 0, the basic model would be in play.
**Verdict: no lever from kayfabe.** Only a guest-side component (parent §7.2) changes this.

### N2. Root userspace owns the twin's page tables (no kernel code at all)

Opened by Wall 2's first three bullets, closed by its fourth. Compute channels cannot be
scheduled. CE-only channels could, which is not the guest's case. **Dead** for GR.

### N3. Snoop or share the fault record without owning the buffer

Closed by Wall 1. Considered and rejected: tools events (post-service, per-va_space), `fault_stats`
(counts only), RM notify (owner's client), interrupt ownership (mask bit only). **Dead.**

### N4. A kayfabe module in nvidia-uvm's place on the exported interface — FEASIBLE, no patch

nvidia.ko exports 76 `nvUvmInterface*` symbols (`kernel-open/nvidia/nv_uvm_interface.c`,
`EXPORT_SYMBOL`) plus `nvidia_p2p_*`. They are the full RM contract nvidia-uvm itself consumes:
session/device/VAS create, `SetPageDirectory`, `GetExternalAllocPtes`, `GetChannelResourcePtes`,
`RetainChannel`/`BindChannelResources`/`StopChannel`, `ChannelAllocate` (kernel-privileged),
`InitFaultInfo`/`OwnPageFaultIntr`/`RegisterUvmCallbacks`/`FlushReplayableFaultBuffer`,
`GetNonReplayableFaults`, `MemoryAllocFB`/`AllocSys`/`CpuMap`, `PmaAllocPages`, `TogglePrefetchFaults`.
`[src]`

What the module does (kf-uvm.ko, `[inf]` for sizing):
| piece | exported call | notes |
|---|---|---|
| own the fault buffer + interrupt | `RegisterUvmCallbacks`, `InitFaultInfo`, `OwnPageFaultIntr` | must load **instead of** nvidia-uvm |
| twin VAS, PDB, page tables | `AddressSpaceCreate` (externally owned + faulting), `SetPageDirectory`, `MemoryAllocFB` | page tables written by kayfabe: the walker already decodes the guest PTE format, the host PTE is the same format with a translated PA; `GetExternalAllocPtes` gives the values for RM objects |
| bind the twin's GR channel | `RetainChannel` + `BindChannelResources` | sets `bIsContextBound` (Wall 2) |
| fault records to userspace | fetch entries, forward `{va, type, access, client, channel id}` over a char device | same record shape as b3's EFS |
| replay / cancel / clear-faulted | a kernel-privileged channel from `ChannelAllocate`, or kayfabe's own admin channel | `MEM_OP` methods |
| non-replayable path | `GetNonReplayableFaults`, `ReportNonReplayableFault` callback | CE faults |
| timeout | module-side, cancel after T | as b3 |

Costs, stated plainly:
1. **Host CUDA is gone.** nvidia-uvm cannot load beside the module (single registrant; module init
   fails), and `cuInit` needs `/dev/nvidia-uvm`. On a dedicated kayfabe host this may be
   acceptable; it is a deployment decision, not an engineering one.
2. **The walker leaves libcuda.** `crates/kf-cuda/src/driver_unsafe.rs` dlopens libcuda and JITs
   PTX. Raw-RM compute launch already exists in this tree (`rmladder`, `cup8` 2048² matmul
   byte-exact, `CLAUDE.md:116,156`), so the port is engineering: prebuilt cubin per family + QMD
   on a raw channel. `[src]` for the tooling, `[inf]` for the effort.
3. **Interface churn.** `nv_uvm_interface.h`/`nv_uvm_types.h` change per release. The module is
   rebuilt per driver version (DKMS), the same cadence as b3's rebase, but the compiler checks
   it against the header rather than against private nvidia-uvm structs. Smaller than a fork of
   nvidia-uvm because residency, va_blocks, HMM, migration and PMM policy are all absent: the
   guest UVM does those.
4. **Size.** Fault buffer fetch/forward and replay/cancel are a few hundred lines each in
   nvidia-uvm; VAS/PDB/bind glue similar. Estimate 2–4 kLoC. `[inf]`

Upside that b3 does not have: kayfabe writes the twin's PTEs directly, so the publish executor
stops being an RM map call. That removes the kind-override question, the per-call TLB cost of
UVM external maps, and the batched-map plumbing (`THE_ARCHITECTURE_v3.md` §4.2) in favour of PTE
writes + one invalidate. Security model is b3's (kernel code, validate every handle against
module-created state, no VMM VA can back a GPU access because the module maps only RM objects
and guest-RAM windows it was handed).

**Verdict: feasible and stock-NVIDIA.** It ranks below b3 on effort and on host-CUDA loss, above
it on "no NVIDIA fork". It is the answer to "without patching nvidia" if that phrase is a hard
requirement.

### N5. A stub process under host HMM with a blocking pager (userfaultfd / FUSE / custom vma)

- UVM refuses userfaultfd-armed VMAs and `VM_IO`/`VM_PFNMAP` outright
  (`uvm_hmm.c:591-598`, comment *"UVM doesn't support userfaultfd"*). `[src]`
- A `MAP_SHARED` (or hugetlb) VMA makes any GPU **atomic** a fatal fault (`uvm_hmm.c:2634-2640`).
  Managed-memory atomics are common. `[src]`
- File-backed and `VM_SPECIAL` VMAs are pinned to sysmem (`uvm_hmm.c:3969-3979`), so a guest
  answer of "resident in vidmem" (GPU-first touch, atomics) has no representation. `[src]`
- The parked service would block the GPU's single fault bottom half for every tenant. `[inf]`
**Dead.**

### N6. UVM test ioctls, tools, module parameters

`uvm_enable_builtin_tests=1` exposes ~100 `UVM_TEST_*` ioctls (full list in `facts.tsv`). The
fault-related ones — `FAULT_BUFFER_FLUSH`, `DRAIN_REPLAYABLE_FAULTS`, `GET_RM_PTES`,
`CHANGE_PTE_MAPPING`, `MAKE_CHANNEL_STOPS_IMMEDIATE`, `SKIP_MIGRATE_VMA`, `VA_BLOCK_INJECT_ERROR` —
flush, drain, inject errors or alter mappings of UVM-managed ranges; none parks or diverts.
Module parameters (`uvm_perf_fault_*`, `uvm_fault_force_sysmem`, `uvm_disable_hmm`,
`uvm_ats_mode`, …, full list in `facts.tsv`) tune servicing; none exports a fault. **Dead.** `[src]`

### N7. ATS / IOMMU SVA

Host UVM ATS needs `CONFIG_IOMMU_SVA` + notifier support (`uvm_ats_sva.h:49-57`) and an RM
`GPU_ATS_CAPABILITY_YES` answer (`nv_gpu_ops.c:7268`). It resolves GPU accesses against a **host
process mm**, which is option (a)'s VA-identity and page-ownership problem again, and HMM is
disabled when ATS is on (`uvm_hmm.c:148`). Consumer parts on x86 do not advertise it. **Dead** for
a stock guest. `[src]`

### N8. Upstream b3 instead of carrying it

nvidia-uvm is MIT/GPL dual-licensed open source; the EFS ioctl set could be proposed upstream to
`open-gpu-kernel-modules`. NVIDIA accepts contributions under CLA but merges few. A long shot that
removes the fork if it lands; not a plan to depend on. `[inf]`

### N9. Guest-side stopgaps

Unchanged from parent §7.2 (`uvm_disable_hmm=1`; managed→pinned shim). They are not stock-guest.

## Ranking for the "no NVIDIA patch" constraint

| rank | option | stock host? | stock guest? | verdict |
|---|---|---|---|---|
| 1 | b3 EFS patch to open nvidia-uvm | ✗ (patched open module) | ✔ | recommended when a maintained patch is acceptable |
| 2 | **N4 kf-uvm.ko replacing nvidia-uvm** | ✔ nvidia.ko stock; nvidia-uvm not loaded | ✔ | **the only no-patch route**; costs host CUDA + raw walker |
| 3 | N9 guest shims | ✔ | ✗ | stopgap |
| — | N1, N2, N3, N5, N6, N7 | | | closed by source (above) |

## Experiments this adds

- **E5 (cheap, guest):** print `concurrentManagedAccess` from `um_probe` inside a kf3 guest.
  Expected 1. Fold into E4.
- **E6 (N4 feasibility, host, no guest):** build a skeleton module that registers callbacks,
  creates an externally owned faulting VAS, sets a PDB it allocated, binds a GR channel via
  `BindChannelResources`, and runs `cup8` on it with nvidia-uvm unloaded. Pass: byte-exact
  result. Then touch an unmapped VA: expect a record on the char device, not an Xid. This is E1's
  twin for the no-patch branch; do E1 first unless host-CUDA loss is already accepted.
