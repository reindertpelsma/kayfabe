# V3_UVM_DEMAND_PAGING — managed memory and HMM in a kf3 guest

**STATUS: RESEARCH, 2026-09-26.** No production code. Answers *"how can kayfabe v3 support CUDA
managed memory / UVM demand paging (and HMM pageable access) in the guest?"*, ranks the options,
and names the first experiment. Owner direction recorded the same day (§0.1): **option (b), fault
delivery to the guest, everything else stock**; implementation preference root helper > kernel
module > nvidia-uvm patch. Branch `v3-uvm-research`; evidence in `traces/v3_uvm_research/`.

Epistemic tags, used on every claim that carries weight:
`[src]` read in ogkm-580.159.04 (`research_clones/ogkm-580.159.04` in the nvkvm tree; paths below
are relative to it) or in this tree at `origin/master` 8e562b9a; `[meas]` measured on hardware,
with the trace named; `[inf]` inferred — reasoned, not read and not run. An `[inf]` is a
hypothesis for an experiment, never a premise for code.

---

## 0. The answer

1. **The guest must see the fault. There is no design in which it does not.** Guest UVM chooses the
   backing page for a managed or pageable VA *while servicing the fault* (§2). Before that fault
   the page does not exist (managed) or exists only in a guest process page table kayfabe cannot
   read (HMM). Anything the host does without the guest's answer maps the wrong page or no page.
   `[src]`
2. **Option (a) in the owner's precise form is refuted** (§3): the GPU dereferences the guest's VA
   X through the twin, a UVM managed range is reachable by the GPU only at the owning process's
   CPU VA, so a host range registered at a host-chosen VA H is never touched by the GPU and host
   UVM never migrates anything. Guest RAM is a shared memfd, which host UVM cannot migrate
   anyway. `[src]`
3. **Option (b) is the only correct shape, and of the three implementations only one is
   feasible** (§4):
   - **(b1) root userspace helper: infeasible.** The fault buffer class needs RM *kernel*
     privilege; root is `USER_ROOT`, a level below. There is one buffer per GPU and nvidia-uvm
     already holds it. `[src]`
   - **(b2) standalone kernel module: infeasible.** Same single buffer, a single
     callback registrant, a single interrupt owner. Hooking nvidia-uvm's fault dispatch with
     kprobes or livepatch is the nvidia-uvm patch in disguise, and harder to maintain. `[src]`
   - **(b3) a patch to the host's open nvidia-uvm: feasible, and the recommendation.** UVM
     already attributes every fault to a va_space by instance pointer, already scopes its
     cancels by instance pointer or PDB, and already replays. The patch adds one per-va_space
     mode: *divert, don't service*. Faults in a va_space that opted in are queued to that
     va_space's own fd and parked. Its owner, kayfabe, answers *replay* or *cancel*.
4. **What b3 costs kayfabe**: the passthrough twin VAS must become **UVM-owned** on the host,
   because RM refuses a fault-capable VAS unless it is externally owned, and it then allocates no
   page tables in it. The publish executor therefore moves from `NV_ESC_RM_MAP_MEMORY_DMA` to
   UVM's external-mapping ioctls. Those ioctls are stock and unprivileged; they are what every
   CUDA process uses. Guest side, the replayable fault buffer, the non-replayable shadow buffer
   and the replay/cancel/clear-faulted decode are the unbuilt steps 5b–5d of the archived
   `resume_from_fault.md`, and every register, RPC and interrupt they need is already trapped
   (§5). `[src]`
5. **Read duplication (item 3, replaced per owner) is broken today, silently** (§6). The walker
   decodes a guest PTE's READ_ONLY bit and then drops it. It is not in the diff key and not in the
   host map flags, so a guest read-only duplicate is mapped **read-write on the host**, and an
   in-place RW→RO downgrade produces no diff at all. Predicted consequence: a GPU write to a
   `cudaMemAdviseSetReadMostly` page lands in a stale duplicate. The guest CPU then reads the old
   value, with no fault and no error. The stock fix is to propagate RO. It turns a silent wrong
   answer into a loud 719, and the same fault delivery (b3) makes it correct. `[src]` + `[inf]`
   for the consequence.
6. **First experiment** (§9): a host-only prototype of b3 on a rented GA10x with **no guest and no
   kayfabe**. A patched nvidia-uvm, plus a ~150-line CUDA program that reserves a VA, launches a
   kernel that touches it, receives the parked fault on its own uvm fd, maps the page with
   `cuMemMap`, and replays. It passes when the kernel completes with correct data and zero Xid. It
   tests the entire privileged half (divert, park, map, replay, cancel, timeout) before a line of
   kf3 changes. Cheap stock checks run alongside it: `readmostly_probe` in a kf3 guest, and a
   guest with `uvm_disable_hmm=1` against the five C-class apps.

### 0.1 Owner direction on this question (2026-09-26, recorded in place)

*"UVM with unprivileged likely is impossible. CPU VA == GPU VA, and we can not let a guest use VMM
VA. So a privileged helper or host kernel module is needed, for UVM only … this module shouldn't
trust the VMM."* Two candidate shapes, (a) VA identity through a stub and (b) fault-buffer access.
Later the same day: **(b) is the direction**, with preference root helper > kernel module >
nvidia-uvm patch, *"pick the highest one that is feasible"*. Item 3 (dual residency) was
**replaced**: stock UVM already does read duplication, and the question is whether kayfabe
handles it (§6). ⇒ §4 evaluates (b) in the owner's order. Only (b3) survives, and §4.4 says why
the two above it cannot be made to work rather than merely being harder.

---

## 1. What is established before this note

From `origin/v3-appfix` (`docs/design/V3_BUILD.md` "App-matrix fixes" C; `traces/v3_appfix/`) and
`origin/v3-apps2` (`docs/design/V3_APP_MATRIX.md`), box vast 52689820, RTX 3060, 580.159.04:

| shape (`um_probe.cu`) | bare metal | kf3 guest |
|---|---|---|
| `cudaMalloc` | ok | ok |
| managed + CPU init + `cudaMemPrefetchAsync` | ok | ok |
| managed + CPU init + `SetAccessedBy` | ok | ok |
| `cudaHostAlloc`, D2H into pageable | ok | ok |
| managed, **CPU first touch**, then GPU | ok | **719**, host Xid 31 `FAULT_PDE` |
| managed, **GPU first touch** | ok | **719** |
| kernel writes plain `malloc` memory (HMM, `pageableMemoryAccess=1`) | ok | **719** |

`[meas]` `traces/v3_appfix/c_um_shapes_{host,guest}.out`, `c_pageable_guest.out`.
⇒ Everything the guest UVM **maps** is published correctly. What fails is **demand paging**: the
guest UVM expects a replayable fault to populate the page. kf3 delivers none, and the host twin's
fault is non-replayable, so the host RCs the TSG.

The app-level family (`V3_APP_MATRIX.md` §R2 row 1): conjugateGradientUM (**silent wrong answer**:
it checks no status), attach_verify, UnifiedMemoryPerf, torch_ai_bench, clpeak. All show host
`Xid 31 … FAULT_PDE` at a user VA. ⊘ UnifiedMemoryStreams (C′, a refused host map `Other(31)`
before a `FAULT_PTE`) is a **separate** defect and not this note's.

### 1.1 clpeak

The matrix records clpeak (OpenCL) hitting `FAULT_PDE VIRT_WRITE @0x772b_de422000` after its
float/half/double groups passed. The first failing group is *integer*, and everything after it
fails too (a sticky context error). `[meas]` The VA is a UVA-range user address, and that alone
cannot separate managed, HMM and device memory. The bare-metal baseline in §1.2 answers the one
question that separates them: **does clpeak take replayable faults on bare metal at all?**

### 1.2 Bare-metal baseline — which workloads need demand paging

*(filled from `traces/v3_uvm_research/bm_*.out`; see §1.2 below the table for the reading)*

---

## 2. Why the guest has to see the fault — the information argument

- **Managed memory: the backing does not exist before the fault.** Guest UVM's fault service
  selects residency (`block_select_processor_residency`,
  `kernel-open/nvidia-uvm/uvm_va_block.c:11610-11690`). It then *populates* the page on the
  chosen processor (`block_populate_pages` → `block_populate_pages_cpu` / 
  `uvm_va_block_populate_pages_gpu`, `uvm_va_block.c:3044-3127`, CPU chunks from
  `uvm_cpu_chunk_alloc`, `:1570`). A GPU-first-touch page has no guest-physical address until
  that code runs. `[src]`
- **HMM: the backing exists, but only in a guest process's CPU page tables.** kayfabe reads no
  guest CPU state. The CR3 design is dead (`kayfabe-arch/src/lib.rs:191`: *"no CPU-state (CR3)
  read exists anywhere in the design"*). ⊘ Reading it anyway would still be wrong. The guest
  kernel's mmu-notifier invalidations (munmap, reclaim, THP collapse, KSM, NUMA migration) go
  only to the guest UVM, so a host mapping derived from a guest CPU PTE outlives the page.
  Another guest process then reads freed memory. `[src]` + `[inf]`
- **Residency policy is guest-internal.** Nothing kayfabe answers in an RM control is an input to
  `block_select_processor_residency`. The inputs are policy (preferred location, read
  duplication, accessed-by), access type and thrashing state, all guest UVM state. The one knob
  that pins residency to sysmem, `uvm_fault_force_sysmem` (`uvm_va_block.c:64`), is a guest
  module parameter, and a fault is still needed to create the mapping. `[src]`
- **Guest UVM cannot be told "no replayable faults".** `replayable_faults_supported = true` is
  hard-coded per architecture (`uvm_ampere.c:81`; also Volta `:81`, Turing `:76`, Pascal `:89`).
  No module parameter overrides it. Legacy virtualization mode refuses UVM outright
  (`uvm_gpu.c:1452-1456`), and the SR-IOV modes change page-table placement, not fault support
  (`uvm_gpu.c:3211`, `uvm_mmu.c:1093`). Refusing the fault-buffer registration kills `cuInit`
  (`[meas]` boot `pu1448`, `kf-rm/src/faultbuffer.rs` header). `[src]`

⇒ The archived `resume_from_fault.md` reached the same verdict in the pre-v3 architecture:
*"the host can own residency; we still have to own the fault"* (§1.4). In v3 the stall unit is
worse than it was there. The guest's pushbuffer runs **directly** on the host twin, so kayfabe
cannot pre-flight a working set at all, and the only stall that exists is the hardware's own
replayable-fault stall on the host. ⇒ **Option (b) needs a host replayable fault on the twin.**

---

## 3. Option (a) — VA identity through a stub, "the real driver migrates underneath"

### 3.1 The owner's precise version, point by point

> Guest UVM keeps managed memory in guest sysmem; kayfabe reports residency as if UVM never
> migrated; the pinned guest-RAM range backing guest UVM is registered on the host as UVM-managed
> at a host-chosen VA; GPU touch ⇒ host UVM migrates to vidmem, guest-CPU touch ⇒ host UVM migrates
> back; fails if the guest demands vidmem first or has no pinned GPA yet.

**(i) GPU VA == owning process CPU VA — REFUTED as a way around it.** A managed range is
created **only** by `mmap` of `/dev/nvidia-uvm`, and its GPU VA is the VMA's start. nvkvm-pv
measured this (`nvkvm-pv docs/internal/uvm-va-decoupling.md` §2a–2b, *"The GPU VA equals the CPU
VA — measured, not inferred"*). UVM keeps one `vma_wrapper` per range and disables any copy in
another mm (`kernel-open/nvidia-uvm/uvm.c:445-449`: *"On fork or move we want to simply disable
the new vma"*, `VM_DONTCOPY` at `:835`). No ioctl places a managed range at a GPU VA other than
its CPU VA. The only decoupled mapping UVM offers is an **external range**
(`UVM_CREATE_EXTERNAL_RANGE` + `UVM_MAP_EXTERNAL_ALLOCATION`), which maps an RM allocation and
never migrates. `[src]`
⇒ The GPU dereferences the guest's pointer X, embedded in kernel parameters, through the twin. A
range registered at a host VA H ≠ X is **never touched by the GPU**, so host UVM takes no fault
at H and **migrates nothing**. "GPU touch ⇒ host migrates to vidmem" has no trigger. To be
touchable, the range must sit at X in the owning process. In the VMM that makes the VMM's CPU VA
guest-chosen, which the owner's strict rule forbids (memory: *the VMM address is never
guest-chosen or guest-visible*, 2026-09-10). In a stub, the pages are the stub's (next point).

**(ii) Page ownership — REFUTED for guest RAM.** Guest RAM is a shared memfd held by the VMM and
KVM. UVM migrates only memory it owns: managed-range CPU pages are UVM's own chunks
(`block_populate_pages_cpu`, `uvm_va_block.c:1843`), and HMM migrates private anonymous memory
only. `uvm_hmm_must_use_sysmem` returns true for `!vma_is_anonymous(vma)`, `VM_SPECIAL`, DAX and
hugetlb (`uvm_hmm.c:3957-3977`: *"TODO: Bug 3660968: add support for file-backed migrations"*).
GPU atomics on a `VM_SHARED`/`VM_HUGETLB` VMA are a **fatal fault** (`uvm_hmm.c:2634-2640`), and
most VM hosts back guest RAM with hugetlb. For the memory to be migratable, guest RAM would have
to *be* a UVM managed VMA of the VMM, used as KVM memslot backing. `[src]`
⇒ Mechanically KVM could plausibly fault such a VMA (`VM_MIXEDMAP`, `uvm.c:835`; `vm_insert_page`
pages are GUP-able; UVM's CPU fault path migrates back and KVM's mmu-notifier zaps EPT on
migrate-out) `[inf]`. But that VMA would have to sit at the GPU VA X (point i), would be
unshareable (`VM_DONTCOPY`, one mm, so no vhost-user and no memfd live migration), and would put
**all** of guest RAM, including guest kernel memory, under host UVM residency. It fails on (i)
before any of that matters.

**(iii) Setup-only memslots vs dynamic backing — REFUTED.** Guest UVM allocates managed pages
from the guest kernel's page allocator at fault time (point 2 of §2). Any guest page can become
managed backing, so "the pinned range" is *all of guest RAM*. It cannot be known at setup.
`[src]`

**(iv) Which GPA backs X at the first GPU touch — the information problem, unsolvable host-side.**
For managed memory the answer is "none yet" (§2). For CPU-first-touched managed memory, the chunk
is recorded only in guest UVM's `block->cpu.chunks` and the guest process's page tables. Neither
is visible to kayfabe, and the guest GPU page tables hold no entry, because none is created until
the fault. `[src]`

**Verdict: (a) is refuted in this version.** It needs an input (the GPA behind X) that exists only
after the guest services a fault, and a mechanism (host UVM migrating a page the GPU touches at
X) that UVM does not have for a range registered at H. The part that survives is the one the
archived note already kept: *residency* could in principle be host-owned, but the *binding* is
the guest's.

### 3.2 The other (a) variants, briefly

| variant | why it fails |
|---|---|
| stub process whose mm mirrors the guest's GPU VA layout, guest RAM memfd mapped at X, host HMM services faults | needs X→GPA (§3.1 iv); memfd forces sysmem, atomics fatal (§3.1 ii); guest invalidations never reach the stub (§2) ⇒ stale-page reads across guest processes |
| stub owns UVM managed ranges at X, KVM maps them | KVM runs in the VMM mm; a managed VMA cannot be mapped in a second mm (`uvm.c:445-449`); `MULTI_PROCESS_SHARING_MODE` turns off `va_space_mm` (`uvm_va_space_mm.c:172-180`) and with it HMM (`uvm_hmm.c:160-165`) |
| steer guest UVM to "always sysmem" through what kayfabe reports | residency is guest policy (§2); a zero-FB GPU would keep UVM in sysmem (`uvm_gpu.c:3211-3219`) but also breaks `cudaMalloc`; faults are still needed to map |
| **nvkvm-pv's "all UVM as DMA"** (Tier 1, `known-limitations.md:962-1000`) | it works there because Mode 1 **intercepts the guest's `/dev/nvidia-uvm` mmap** and republishes a pinned RM sysmem object as an external range. In kf3 the guest runs a stock nvidia-uvm and no ioctl crosses to us. The equivalent needs a **guest-side** hook (§7.2), not a host one |

---

## 4. Option (b) — deliver the fault to the guest

### 4.1 The facts every implementation must live with

| fact | source |
|---|---|
| **One replayable fault buffer per GPU function.** `mmuFaultBuffer[64]` is indexed by GFID; the PF uses `[GPU_GFID_PF]`, and a second allocation while one is live returns `NV_ERR_NOT_SUPPORTED` | `src/nvidia/generated/g_kern_gmmu_nvoc.h:744`; `src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1212-1216` |
| The class `MMU_FAULT_BUFFER` is **`RS_FLAGS_ALLOC_KERNEL_PRIVILEGED`**, `Multi-Instance NV_FALSE` | `src/nvidia/src/kernel/rmapi/resource_list.h:975-982` |
| Kernel privilege is enforced as `privLevel < RS_PRIV_LEVEL_KERNEL ⇒ NV_ERR_INSUFFICIENT_PERMISSIONS`; root is `RS_PRIV_LEVEL_USER_ROOT`, below it | `src/nvidia/src/kernel/rmapi/alloc_free.c:647-667` |
| **nvidia-uvm takes the buffer at the first GPU registration**: `uvm_parent_gpu_init_isr` → `uvm_parent_gpu_fault_buffer_init` → `nvUvmInterfaceInitFaultInfo` | `kernel-open/nvidia-uvm/uvm_gpu_isr.c:352-365`, `uvm_gpu_replayable_faults.c:247-274` |
| **kayfabe itself registers the GPU**: the walker runs through `dlopen`ed `libcuda` (`cuInit` ⇒ `UVM_REGISTER_GPU`) | `crates/kf-cuda/src/driver_unsafe.rs:1-24` |
| RM callbacks: **one registrant**, otherwise `NV_ERR_IN_USE` | `kernel-open/common/inc/nv_uvm_interface.h:1088-1093`; `kernel-open/nvidia/nv_uvm_interface.c:1051-1066` |
| RM's replayable-fault notify goes to the **fault-buffer object's own event list** (the owner's client) | `src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:262-281` |
| A **fault-capable VAS must be externally owned**. The comment says why: an unowned one "*will cause what looks like a hang*" | `src/nvidia/src/kernel/gpu/mem_mgr/vaspace_api.c:678-690` |
| UVM refuses to adopt a VAS that is not externally owned | `src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:2689-2693` |
| In an externally owned VAS, RM reserves **no VA and allocates no page tables** | `src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:1419-1424`, `:3161-3168` |
| A GR channel in an externally owned VAS cannot be scheduled until its context is bound (UVM does this at `UVM_REGISTER_CHANNEL`) | `src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2200-2206` |
| UVM attributes each fault to a va_space by **instance pointer → registered user channel**; an unknown instance pointer is `NV_ERR_INVALID_CHANNEL` | `kernel-open/nvidia-uvm/uvm_gpu.c:3534-3570` |
| UVM's cancels are **scoped**: targeted by instance pointer + GPC/client, or by VA against the va_space's own PDB | `uvm_gpu_replayable_faults.c:352-431`, `:438` (`cancel_fault_precise_va`), `:1047-1080` |
| Replay `START`/`START_ACK_ALL` is **GPU-wide**: it re-issues every pending faulting access | `kernel-open/nvidia-uvm/uvm_hal_types.h:496-506` (quoted in `docs/archive/resume_from_fault.md` §2.1) |
| Host RM passes **non-replayable** faults to CPU-RM with `MMU_FAULT_QUEUED`, which wakes UVM's non-replayable servicer | `src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:1048-1056`, `:1498-1500` |

### 4.2 (b1) Privileged host userspace helper (root) — INFEASIBLE

- It cannot allocate the buffer: kernel-privileged class, and root < kernel (table above). `[src]`
- Even with kernel privilege it would be the **second** owner: nvidia-uvm holds the one PF
  buffer from kayfabe's own `cuInit` onward, and the allocator refuses a second. `[src]`
- ⊘ The shadow buffers do not help. The *replayable* client shadow buffer registers only under
  Confidential Compute (`NV_ASSERT_OR_RETURN(gpuIsCCFeatureEnabled(pGpu), NV_ERR_NOT_SUPPORTED)`,
  `src/nvidia/src/kernel/gpu/mmu/mmu_fault_buffer_ctrl.c:148`). Both shadow buffers are controls
  **on the `MMU_FAULT_BUFFER` object**, which the helper cannot own
  (`mmu_fault_buffer_ctrl.c:40-126`). `[src]`
- ⊘ `NV2080_CTRL_CMD_MC_CHANGE_REPLAYABLE_FAULT_OWNERSHIP` (`RMCTRL_FLAGS_PRIVILEGED`, so
  admin-callable) only flips which driver takes the interrupt
  (`src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:84-105`). Taking it from UVM stops
  **every** host CUDA process's faults being serviced, the helper still cannot read the buffer,
  and the notify it would get is delivered to the owner's object. `[src]`
- ⊘ UVM's tools event queues (`UvmEventTypeGpuFault`/`FatalFault`, `uvm_types.h:288-324`) are
  observational: the fault has already been serviced or cancelled when the event is read.
  `UVM_TEST_*` ioctls (`uvm_test_ioctl.h:514, 1194`) are test hooks gated on
  `uvm_enable_builtin_tests`, and they flush or drain; they do not park. `[src]`
- ⊘ There is **no RM or UVM control that delivers faults for one VAS to a privileged client.** I
  found none in `ctrl2080*`, `ctrlc369.h` or the UVM ioctl set, and the paths above are the only
  consumers of the buffer. `[src]` (absence claim: searched, not proven exhaustive).

### 4.3 (b2) A standalone host kernel module — INFEASIBLE as a module; a patch in disguise as a hook

- The `nvUvmInterface*` entry points **are** `EXPORT_SYMBOL` (`nv_uvm_interface.c:910`,
  `:1075`), so a third-party module can call them. But `nvUvmInterfaceInitFaultInfo` allocates the
  same single-instance buffer (refused while nvidia-uvm holds it),
  `nvUvmInterfaceRegisterUvmCallbacks` has one registrant (`NV_ERR_IN_USE`), and
  `nvUvmInterfaceOwnPageFaultIntr` names one owner. **Two users cannot hold fault info.** `[src]`
- The one way a module becomes the owner is to **replace nvidia-uvm**. kayfabe's walker needs
  host CUDA, and host CUDA needs nvidia-uvm. A replacement is therefore a fork of nvidia-uvm:
  (b3) under another name, with more code to carry. `[src]` + `[inf]`
- **Hooking** nvidia-uvm's dispatch (kprobe or ftrace on `service_fault_batch` /
  `uvm_parent_gpu_fault_entry_to_va_space`, or a livepatch module over nvidia-uvm functions):
  changing *behaviour* (divert instead of service) is not something a kprobe can do. Return
  overrides need `ALLOW_ERROR_INJECTION`, which nvidia-uvm does not declare. A livepatch
  replaces whole functions and must match nvidia-uvm's private struct layouts
  (`uvm_fault_buffer_entry_t`, the batch context, locking) for each release. It is the (b3)
  diff with no compiler checking it against the source it patches. ⇒ **Say it plainly: it is
  the patch in disguise with worse maintenance.** `[src]` for the facts, `[inf]` for the verdict.

### 4.4 (b3) Patch the host's open nvidia-uvm — FEASIBLE; the recommendation

**Shape.** Add one per-va_space mode, **EXTERNAL FAULT SERVICE (EFS)**, which a va_space enters
at `UVM_INITIALIZE`. Everything else stays stock.

| piece | what | where it hooks |
|---|---|---|
| opt-in | new `UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE`; **requires** `UVM_INIT_FLAGS_DISABLE_HMM` (or `MULTI_PROCESS_SHARING_MODE`) and refuses managed-range `mmap` on that fd. Gated by a module param, default **off** | `uvm_api_initialize` / `uvm_mmap` (`uvm.c`) |
| divert (replayable) | in the batch path, after `uvm_parent_gpu_fault_entry_to_va_space` resolves an entry to an EFS va_space: **copy** `{va page, access type, fault type, client type/id, gpc, utlb, ve_id, registered-channel id, timestamp}` into the va_space's queue, **dedupe** against its parked set, and do not service, mark fatal or cancel | `uvm_gpu_replayable_faults.c` (`preprocess_fault_batch` / `service_fault_batch` dispatch, ~`:1047`, `:2231-2373`) |
| divert (non-replayable: CE, host) | same for the non-replayable servicer. The channel stays **faulted** until cleared | `uvm_gpu_non_replayable_faults.c` |
| read | `poll`/`read` on the uvm fd (or an eventfd bound to it) returns fault records. **Never** an instance pointer or PDB address: records name the registered channel by an id the fd issued | new |
| `UVM_EFS_REPLAY` | issue a UVM replay (`push_replay_on_gpu`, `uvm_gpu_replayable_faults.c:503-543`). Rate-limited per va_space | new ioctl → existing function |
| `UVM_EFS_CANCEL(record, mode)` | `cancel_fault_precise_va` / `push_cancel_on_gpu_targeted` with the **kernel's stored** instance pointer and PDB for that record, never caller-supplied addresses | new ioctl → existing functions |
| `UVM_EFS_CLEAR_FAULTED(channel id)` | `clear_faulted_method_on_gpu` for a non-replayable fault (`uvm_gpu_non_replayable_faults.c:262-296`) | new ioctl → existing function |
| timeout | parked faults older than T (module param, e.g. seconds) are cancelled **by the kernel**. The channel RCs, which is today's behaviour, and the VMM cannot hold the GPU forever | new |

**Publishing into the UVM-owned twin — stock and unprivileged.** The twin VAS is allocated
externally owned with `ENABLE_FAULTING` and adopted with `UVM_REGISTER_GPU_VASPACE`. The
diff executor then maps with `UVM_CREATE_EXTERNAL_RANGE` + `UVM_MAP_EXTERNAL_ALLOCATION` and
unmaps with `UVM_UNMAP_EXTERNAL` (`uvm_ioctl.h:491, 935, 1042`), with per-GPU
`gpuMappingType` READ_ONLY/READ_WRITE/ATOMIC (`uvm_types.h:87-94`). Twin channels register with
`UVM_REGISTER_CHANNEL` (`:425`). ⊘ This **refutes the coordinator hypothesis** that host UVM
could tolerate a registered VAS whose PTEs kayfabe writes through RM. RM will not map into an
externally owned VAS at all (§4.1). One executor serves both cases: map or unmap on a diff line,
or map at a fault.

⚠ What the swap costs: `[inf]` unless marked
- **Kind.** kf3 maps with `NVOS46_FLAGS_PAGE_KIND_OVERRIDE_YES` for graphics
  (`kf-host/src/channel.rs:285-286`). UVM external mappings take kind from the allocation plus
  format/element/compression attributes. Whether every guest kind is expressible that way is
  **open**, and it bears on v3-gfx surfaces.
- **Per-call TLB cost.** kf3 batches maps with `DEFER_TLB_INVALIDATION` and one invalidate per
  batch (`THE_ARCHITECTURE_v3.md` §4.2). UVM external maps invalidate per call. This needs
  measuring, and a batched variant may belong in the patch.
- **One UVM va_space per guest VAS.** A va_space holds one GPU VA space per GPU, so N guest
  contexts take N uvm fds in the VMM, all with HMM disabled. `MULTI_PROCESS_SHARING_MODE`
  detaches them from the VMM mm entirely (`uvm_va_space_mm.c:172-180`), which is the stronger
  guarantee that no VMM VA can ever back a GPU access.
- Only **passthrough** twins (guest userspace channels) need this. Translated and emulated
  channels run in the GPGA VAS and stay RM-owned: guest UVM's own migration copies use physical
  or its own internal addresses, never a user VA that could demand-fault.

**Races.** `[inf]`, each tied to a test in §9
1. **Replay storms.** Host UVM replays after its own batches, and replay is GPU-wide, so parked
   faults re-fault whenever any tenant makes progress. Mitigations: the parked-set dedupe, never
   replaying for a batch that was all-diverted, and relying on UVM's existing duplicate
   accounting.
2. **Buffer pressure.** Repeated re-faults of parked accesses consume the shared buffer. The
   kernel-side timeout bounds how long a VMM can do this, and the rate limit on
   `UVM_EFS_REPLAY` bounds how fast.
3. **Map-before-replay.** Replay must follow the host map *and its TLB invalidate*, otherwise the
   replay re-faults. kf3's commit-on-ack already orders "host confirmed" before the guest's next
   method runs. The replay is issued after the ack.
4. **Channel teardown with parked faults.** On unregister, UVM detaches the channel and
   instance-pointer lookups fail (`uvm_gpu.c:3548-3552`). The patch must drop that channel's
   parked records under the same lock, or a late cancel names a freed instance.
5. **Guest cancels** (`REPLAY_CANCEL_*` on the guest's MEMOPS channel) become `UVM_EFS_CANCEL` →
   host RC → kf3 forwards `RC_TRIGGERED` (existing path). That is correct: the guest UVM decided
   the fault was fatal.

**Latency path.** `[inf]`: stage estimates to be replaced by the §9 measurements

host GPU fault → host fault IRQ → UVM bottom half fetches the batch (tens of µs) → divert, wake
fd → kf3 worker writes the guest packet(s) and advances emulated `PUT` → MSI-X into the guest
(KVM irqfd, µs) → guest UVM bottom half services: allocate, maybe CE-zero or copy (forwarded
channels) and write PTEs → guest `MEM_OP` invalidate + `REPLAY_START` → kf3 split → walk (the
full-VAS walk is ~0.2 ms, `the_walk_kernel_is_462ms_not_67us`) → diff → UVM external map
(+ TLB) → ack → `UVM_EFS_REPLAY` → the host GPU re-issues. Bare-metal UVM services a batch in
tens of µs. This path is plausibly **0.3–1 ms per batch**, 10–20× slower, and correct. The
biggest lever is ours: the fault records name the exact VAs, so the replay's reconcile can walk
**only those pages**. ⚠ Scope hints were removed from the walker on 2026-09-25
(`cuda/walk/kf_walk.cu:488-490`), because a scoped walk carried stale runs across skipped
regions. A *fault-scoped* walk diffed against committed placements does not carry anything
across, but it is a design decision for the owner, not a free optimisation.

**Maintenance.** The patch touches the fault-batch path, which churns between releases. That is
nvkvm-pv's warning, *"one touching fault handling is not [sustainable]"*
(`nvkvm-pv docs/internal/uvm-roadmap.md:143-171`). Keep it small: new code in its own file, and
three hook sites (batch dispatch, non-replayable dispatch, `uvm_api_initialize`). Package it as
DKMS. Fail loudly and checkably: a runtime probe, `UVM_EFS_QUERY`, that kayfabe calls at startup,
so a stock host refuses **by name** at the first managed allocation rather than wedging. Rebase
per `Dh` release. `[inf]`

**Blast radius.** The patch is host-kernel code. A bug in it is a host kernel bug, which is the
cost of (b) in every form. What limits it: the new code only copies records and calls existing
UVM functions, it validates every request against state the kernel stored itself, and it is off
unless the admin enables it.

### 4.5 SR-IOV and vGPU-capable parts

RM keeps per-GFID fault buffers (`mmuFaultBuffer[64]`, above). On SR-IOV datacenter parts each VF
has its own. They are programmed through the host's **vGPU manager stack**, and the VF is handed
to a VM by VFIO, where a guest driver owns it. That is a different product model (the guest drives
a real VF, and kayfabe is not the GPU), so it does not change this answer. A VF-backed kayfabe
mode is out of scope. `[src]` for the array, `[inf]` for the model.

---

## 5. What the guest must see (kf3 side of (b))

All of it is between the guest and kayfabe. Nothing here is privileged on the host (archived
`resume_from_fault.md` §2.2, re-checked):

| guest mechanism | kf3 today | needed |
|---|---|---|
| replayable buffer location: `0x20800a9b` `REGISTER_FAULT_BUFFER` (PTE list) | **answered + recorded** (`kf-abi/src/faultbuffer.rs`, `kf-rm/src/faultbuffer.rs`) | write 32-byte `clc369` packets into those guest pages: all dwords before `VALID`, a fence, then `PUT` |
| `MMU_FAULT_BUFFER_GET/PUT(1)` over BAR0 | trapped, `PUT` never moves (`DELIVERY_UNBUILT`) | serve honestly; `PUT` is ours, `GET` is the guest's |
| replayable interrupt (vector 64 ⇒ `CPU_INTR_LEAF(2)` bit 0) | interrupt tree exists | a **level** re-derived from `GET != PUT`, never an edge (`simulated_gpu_fault.md`) |
| non-replayable: `0x20800a9d` client shadow buffer + `MMU_FAULT_QUEUED` (`0x1005`) event | shadow buffer **answered** (`kf-abi/src/faultbuffer.rs:246-331`); event never sent | write the shadow entry and send the event (`kernel_gsp.c:1048-1056`) |
| packet identity: **guest** instance-block address + VEID | kf3 maps host twin ↔ guest chid (RC forwarding: `RC host twin 0x24 (guest chid 0xd …)`) | carry the guest instance pointer; a mismatch means *silence* (`kfifoConvertInstToKernelChannel` linear scan) |
| replay: `MEM_OP_C.TLB_INVALIDATE_REPLAY = START/START_ACK_ALL` on the guest UVM MEMOPS channel | the translated rewriter **drops** every TLB invalidate and splits there (`kf-chan/src/translated.rs:12-14, 337-351`), so the replay field is discarded | at that split: reconcile, ack, then `UVM_EFS_REPLAY` |
| cancel: `REPLAY_CANCEL_TARGETED/GLOBAL/VA_GLOBAL` | dropped the same way | `UVM_EFS_CANCEL`, scoped to that twin |
| CE resume: `C076 CLEAR_FAULTED_A/B` SW method (`uvm_ampere_host.c:190-210`, `has_clear_faulted_channel_sw_method` on Ampere) | unknown: needs a check that kf3 routes the `GP100_UVM_SW` subchannel at all | translate the guest instance to the twin and call `UVM_EFS_CLEAR_FAULTED` |
| prefetch-fault toggle (`kgmmuToggleFaultOnPrefetch`, BAR0) | trapped register | record it; mirror it to the host only if measured to matter |
| access counters | advertised, never written (deliberate fiction) | unchanged: no replay dependency |

⊘ **Security note on the rewriter, and it is good news:** because the translated rewriter drops
every TLB invalidate (a privileged host method, `alloc_channel.h:207-214`), a guest can never
issue a host replay or cancel through a forwarded pushbuffer. Host replay and cancel happen only
through the scoped ioctls above.

**How much of kf3 changes.** `[inf]`: the publish executor (RM map → UVM external map) for
passthrough twins, the twin VAS and channel allocation (externally owned, UVM-registered), the
guest fault-buffer plane (new: packet writer, `PUT`/`GET`, the interrupt level, the shadow
buffer and its event), the rewriter's invalidate split learning replay and cancel, the
`CLEAR_FAULTED` route, and the EFS fd in the worker's poll set. The walker, diff model,
commit-on-ack, doorbell plane and RC forwarding are unchanged.

---

## 6. Read duplication (item 3 as replaced) — does kf3 handle what stock UVM does?

**What guest UVM does** `[src]`:
- A read fault in a `ReadMostly` range copies the page to the faulting processor and maps it
  **read-only** on every holder (`block_select_processor_residency`, `uvm_va_block.c:11639-11650`;
  `uvm_va_block_make_resident_read_duplicate`, `:5185-5340`).
- A write **collapses** it: the other copies are **unmapped** (`uvm_va_block.c:4930-4946`, *"Also
  unmap read-duplicated pages excluding dest_id"*).
- A CPU read of a GPU-resident page in a ReadMostly range duplicates it and **revokes GPU write
  in place**: a permission downgrade that keeps the same physical page (`block_revoke_prot`,
  `uvm_va_block.c:9010-9060`).
- The copies are CE work on UVM's own channels, and the PTE changes are published with a TLB
  invalidate.

**What kf3 does with it** `[src]`:

| step | kf3 | verdict |
|---|---|---|
| CE copies for the duplicate / collapse | translated channel; PHYSICAL operands rewritten into the identity / guest-RAM windows (`kf-chan/src/translated.rs:6-11`) | ✔ by design |
| collapse = **unmap** + invalidate | `MEM_OP` invalidate ⇒ split ⇒ walk ⇒ UNMAP diff ⇒ host unmap, committed before the rest of the segment runs (`translated.rs:12-14`, `kf-chan/src/ring.rs:8`) | ✔ by design (the `reprefetch` test checks the ordering) |
| duplicate mapped **read-only** | walker decodes `KFWR_RF_READ_ONLY` (`cuda/walk/kf_walk.cu:257`), but `DiffRun` has no permission field (`crates/kf-mem/src/apply.rs:20-36`; built at `crates/kf-mem/src/vasmgr.rs:414-422`) and the host map never sets `NVOS46_FLAGS_ACCESS_READ_ONLY` (`crates/kf-host/src/channel.rs:285-292`; the flag exists, `src/common/sdk/nvidia/inc/nvos.h:1974-1977`) | ⊘ **host PTE is READ-WRITE** |
| in-place downgrade RW→RO | the diff key is `kf_hkey` = aperture class + kind, **no permission bit** (`kf_walk.cu:780-790`, mirrored by `crates/kf-cuda/src/diffmodel.rs:55-62`); *kept(p)* ⇔ same host ground truth and offset | ⊘ **no diff at all**; the host stays RW |
| `ATOMIC_DISABLE`, `PRIVILEGE`, `VOLATILE` | decoded (`kf_walk.cu:255-258`), dropped the same way | ⊘ same class |

**Consequence** `[inf]`, stated as a prediction for `readmostly_probe`:
- `gpuwrite` (ReadMostly, CPU init, prefetch to GPU, GPU **write**, CPU read): on bare metal the
  GPU write faults and UVM collapses to the GPU, so the CPU sees the new value. **On kf3 today
  the host PTE is RW**, the write lands in the GPU duplicate, guest UVM still believes the CPU
  copy is valid, and **the CPU reads the old value**. No fault, no error. The next CPU write
  collapses to the CPU and discards the GPU's write for good.
- `downgrade` reaches the same state through the in-place RW→RO revoke.
- `reprefetch` needs no demand fault and should pass if collapse ordering is right.
- `fault` needs a replayable fault after the collapse: 719 today.

**Fix, stock and unprivileged:** carry READ_ONLY (and ATOMIC_DISABLE) into run identity *and*
into `kf_hkey` / `host_key`, so a permission change becomes an UNMAP+MAP. Pass it to the host as
`NVOS46_FLAGS_ACCESS_READ_ONLY` today, or as `UvmGpuMappingTypeReadOnly` under b3. Before (b3)
lands, this turns the silent wrong answer into a loud host write fault (Xid 31 → 719). After (b3),
the write fault is delivered and the guest collapses correctly. ⊘ `PRIVILEGE` also deserves an
owner look. If the guest driver ever marks a user-VAS PTE privileged, the host twin currently
drops that restriction, which weakens a guest-internal boundary. I did not find such a use.
`[inf]`

---

## 7. Host and guest tweaks that change the picture without a new host module (item 4)

### 7.1 Host

| tweak | effect |
|---|---|
| `uvm_disable_hmm`, `uvm_perf_*`, `uvm_fault_force_sysmem`, thrashing and prefetch params (`module_param` list, `kernel-open/nvidia-uvm/*.c`) | none of them makes a non-UVM VAS fault-capable or delivers a fault to userspace. ⊘ No parameter parks faults. `[src]` |
| running the twin under a **UVM-owned VAS created by kayfabe** (stock) | ✔ possible, unprivileged, and it is exactly what CUDA does. It makes twin faults **replayable** and routes them to host UVM, which **cancels** them (no range, HMM off). ⇒ Still 719, but it is **the prerequisite of (b3)**, testable today with no patch (§9 E2) |
| same, with host HMM **on** in the VMM's va_space | ⊘ **forbidden**: host HMM would service a guest GPU access from **whatever the VMM has mapped at that VA** (`uvm_hmm_vma_is_valid`, `uvm_hmm.c:585-600`). That is exactly the "guest uses VMM VA" hole. `DISABLE_HMM` or `MULTI_PROCESS_SHARING_MODE` is mandatory |
| RM regkeys | none found that relaxes `vaspace_api.c:678-690`. It is compiled into GSP-RM too, and kernel-open cannot patch firmware. `[src]` + `[inf]` |
| MMU debug mode (`NV83DE_CTRL_CMD_DEBUG_SET_MODE_MMU_DEBUG`, NON_PRIVILEGED) | suppresses RC on a non-replayable fault (`kern_gmmu_gv100.c:2059-2073`), but the faulting access is not replayed. The warp's access is lost. ⊘ Not a resume mechanism. `[src]` + `[inf]` |

### 7.2 Guest (not stock-guest, but no host privilege), stopgaps only

- **Guest `uvm_disable_hmm=1`** makes `pageableMemoryAccess` report 0 (`uvm_gpu.c:3861`, via
  `uvm_va_space_pageable_mem_access_enabled`). Well-behaved runtimes then take their non-HMM
  paths, and an app that dereferences malloc memory on the GPU fails as it would on a non-HMM
  host. Worth one matrix run (§9 E4), because if a runtime (OpenCL?) keys a code path on that
  attribute, the fix is a guest modprobe option. `[inf]`
- **An nvkvm-pv-style fallback in the guest**: a guest-side hook turns `cuMemAllocManaged` into a
  pinned sysmem allocation published as an external range. That gives correct, coherent,
  non-migrating "managed" memory with no demand faults. It needs a guest component: an LD_PRELOAD
  that also covers `cuGetProcAddress`, or a guest kernel hook. `cudaMemPrefetchAsync` and
  `cudaMemAdvise` on the result must be made no-ops. It is Tier 1 of nvkvm-pv, with its
  limitation (no VRAM residency). `[inf]`

---

## 8. Security — threat model per option

| | hostile guest (root inside) | compromised VMM | bug in the privileged piece |
|---|---|---|---|
| **(a)** any variant | stale host mappings of guest pages after guest invalidations ⇒ guest cross-process leak (§2); host HMM in the VMM mm ⇒ **VMM memory reachable by guest GPU work** | — | — (refuted before this matters) |
| **(b1)** root helper | — | — | infeasible |
| **(b2)** hook module | same as b3 | same as b3 | worse than b3: the hook depends on private layouts, so a mismatched struct is a host kernel memory corruption |
| **(b3)** nvidia-uvm EFS | can only make **its own** twin's faults park; they are cancelled at the timeout. Guest-authored replay and cancel never reach the host GPU raw (§5). Every address the guest influences reaches the host as a UVM external map of kayfabe's **own** RM objects (store, guest-RAM descriptor), and RM and UVM validate those handles against the fd's client | can park its own faults (bounded by the kernel timeout), replay GPU-wide (rate-limited), cancel **only** through records the kernel stored for its va_space (no raw instance pointer or PDB in the API), map only RM objects its own client holds. It cannot read another va_space's records: attribution is UVM's own (`uvm_gpu.c:3534-3570`) | host kernel code. Kept to copy-and-call-existing-functions, default off, admin opt-in |

⊘ **The rule b3 must enforce itself, because it cannot trust the VMM:** every ioctl argument is
a *handle into state the kernel created for that fd* (record id, channel id). None is an address,
an instance pointer or a PDB. HMM and managed ranges are off on an EFS va_space, so **no VMM
virtual address can ever back a GPU access**.

---

## 9. Ranking, recommendation, experiments

| rank | option | feasibility | effort `[inf]` | security | verdict |
|---|---|---|---|---|---|
| **1** | **(b3) nvidia-uvm EFS patch + UVM-owned passthrough twins + guest fault plane** | ✔ every mechanism exists in UVM | host patch ~0.5–1 kLoC; kf3 publish-executor swap; guest fault plane (5b–5d) | good: scoped by UVM's own attribution; small new kernel surface | **recommended**; the only feasible (b) |
| 2 | stock hygiene, independent of the decision: propagate READ_ONLY/ATOMIC_DISABLE (§6); refuse by name when EFS is absent; run the twin UVM-owned (the b3 prerequisite) | ✔ | days each | strictly better | **do regardless** |
| 3 | guest stopgaps (§7.2): `uvm_disable_hmm=1`; guest managed→pinned shim | ✔ | small | neutral | stopgap only; not stock guest |
| 4 | (b2) standalone module / kprobe / livepatch | ✗ single owner; hooks are a patch in disguise | — | worse than b3 | rejected |
| 5 | (b1) root helper | ✗ kernel privilege and single owner | — | — | rejected |
| 6 | (a) all variants | ✗ information and VA identity | — | leaks | refuted |

**Recommendation.** Build (b3). The owner's preference order was checked in order. (b1) and (b2)
fail on facts that no amount of engineering moves: one kernel-privileged fault buffer per GPU,
owned by the nvidia-uvm that kayfabe's own `cuInit` loads. Do rank 2 first, whatever is decided.
The READ_ONLY fix removes a silent-corruption class today, and making twins UVM-owned is where
most of the kf3-side risk lives.

**Experiments, in order** (none needs `vh`):

- **E1: the privileged half, host-only (the first experiment).** Rented GA10x, open
  580.159.04 built from source with the EFS patch. Test program, plain CUDA driver API:
  `cuMemAddressReserve` a VA X, enable EFS on the process's own uvm fd, and launch a kernel that
  reads and writes X. A helper thread `poll`s the fd, gets the record, `cuMemCreate` +
  `cuMemMap`s X (libcuda publishes it through `UVM_MAP_EXTERNAL_ALLOCATION`), then calls
  `UVM_EFS_REPLAY`. **Pass**: the kernel completes with the right data, zero Xid, and the fault
  was seen exactly once after dedupe. **Also measure**: fault-to-record latency, record-to-replay
  latency, replay storms with a second CUDA tenant faulting concurrently, the timeout cancel (do
  not answer and expect an RC after T), and a targeted cancel (expect 719 in this process only,
  with the second tenant unaffected).
- **E2: UVM-owned twin, stock, no patch.** kf3 raw client (rmladder): an externally owned VAS
  with `ENABLE_FAULTING`, `UVM_INITIALIZE(MULTI_PROCESS_SHARING_MODE)`,
  `UVM_REGISTER_GPU_VASPACE`, the store and guest-RAM objects mapped through
  `UVM_CREATE_EXTERNAL_RANGE`/`MAP_EXTERNAL_ALLOCATION`, and a compute channel through
  `UVM_REGISTER_CHANNEL`, then cup2/cup8 on it. **Pass**: bit-exact. An unmapped touch becomes a
  *replayable* fault that UVM cancels (`fault_stats` moves), never the RM Xid-31 path. It
  measures map cost against the batched RM path and answers the kind-override question.
- **E3: `readmostly_probe` in a kf3 guest** (`traces/v3_uvm_research/readmostly_probe.cu`).
  Prediction today: `reprefetch` ok; `fault` 719; `gpuwrite` and `downgrade` **FAIL with a wrong
  value and no error** (the silent case). After the §6 fix: `gpuwrite` and `downgrade` → 719.
  After b3: all four ok.
- **E4: guest `uvm_disable_hmm=1`** against the five C-class apps and `um_probe pageable`. It
  separates the HMM-keyed failures from the managed ones (clpeak in particular, §1.1).

---

## 10. What I could not determine

1. Whether RM's non-replayable handling for a UVM-registered channel defers to UVM or RCs first
   on GSP hosts. That decision is inside GSP-RM firmware, and only the `MMU_FAULT_QUEUED`
   plumbing is in ogkm. E1 with a CE-faulting variant settles it.
2. Whether a context with parked replayable faults can still be time-sliced off the GPU, or holds
   it until the fault is answered. This bounds how long a slow guest may keep a fault parked
   before other tenants notice. E1 with a second tenant measures it.
3. Whether every guest PTE kind is expressible through UVM external-mapping attributes (§4.4). E2.
4. Whether the guest ever routes `GP100_UVM_SW` (`C076`) methods through a channel kf3 forwards,
   and how kf3 handles that subchannel today.
5. What clpeak's faulting buffer is. §1.2 gives the bare-metal fault counts, which say whether it
   needs demand paging at all. The allocation type itself is inside closed OpenCL.
