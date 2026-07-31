# Resume-from-fault — what NVIDIA does with it, and what it locks out of this design

**Question this answers (the owner's words):** *"I want to know first what NVIDIA can do with
resume-from-fault, and if it's emulatable at all, to see if some features aren't permanently
locked out for us … whether this limitation — since it will probably not fire properly in the
guest — is ever a problem (e.g. trying to resume as guest after we got a fault error); and for
each scenario where it does sound like an issue, whether it's solvable using our design."*

**Epistemic classes** (per `claim_ledger.md`): `[src@580]` = read out of `ogkm-580.159.04`,
nothing ran. `[meas]` = a scan of a committed capture in `traces/mode2_c_reference/`, which is a
recording of a real stock guest driver — not a live run. `[C:]` = read out of the C artifact.
⊘ **No hardware run was available for this note.** Every scan below was performed 2026-08-01.

---

## 0. The answer in five lines

1. **"We cannot resume a GPU fault" is false as a blanket statement.** With Confidential Compute
   off — our target — the entire replayable-fault loop is **open, guest-side, and inside
   interfaces we already own**: the buffer is guest RAM whose address is RPC'd *to us*, `GET`/`PUT`
   are BAR0 registers we already trap, the interrupt is an MSI-X we already raise, and the replay
   is a pushbuffer method on a channel whose doorbell we already gate. `[src@580]`
2. **It is not hypothetical.** In `cap3_matmul_forwarding` the guest's UVM **armed the replayable
   fault buffer and polled it**, and the first BAR0 register it read after each of our four MSI-X
   interrupts was `MMU_FAULT_BUFFER_PUT(1)`. `[meas]`
3. **The real limit is not replay, it is the STALL UNIT.** Silicon stalls one warp's memory access
   and resumes it. We stall a whole *submission* by not ringing the doorbell. A coarser stall is
   legal and invisible to the guest — but only for an address we knew **before** submitting. Once
   work is on the host GPU we have no stall at all, and nothing fixes that.
4. **The host-replays hypothesis does not hold as written**, for three independent reasons (§1).
   A weaker version does: the host can own *residency*; it cannot own the guest's *VA binding*.
5. **Trap-on-transition is not complete on its own** (§6) — and the backstop the holes need is
   exactly the fault mechanism in (1). The two halves of this note answer each other.

---

## 1. ★★★ The hypothesis tested first: does the host replay for us?

`C: docs/design/mode2_uvm_residency.md:31-35` records the C's model:

> *"The guest's managed range is backed by a **host** `cudaMallocManaged` allocation. **Host UVM
> owns residency**; the **guest UVM is an inert fiction.**"*

and `:96-98`: *"the emulated GPU never delivers UVM faults, so the guest UVM stays put on its
own."*

**Verdict: it does not hold as written.** Three reasons, independent.

### 1.1 It was never built, and it says so

`cudaMallocManaged` / `cuMemAllocManaged` appear **zero times** in the C's `src/`; no host CUDA
runtime is linked anywhere (the host stub issues raw RM ioctls only). There is no residency
reporting, no migrate-suppression, no `MEMORY_MANAGED` handling. The doc's own de-risking step is
still open: *"**The one spike to de-risk the fast path** … Spike: run a `cudaMallocManaged`
workload through a Mode-1-style GPA window"* (`C: docs/design/mode2_uvm_residency.md:75-84`), and
`C: docs/design/mode2_compute_forwarding.md:270` still lists it as future work. Every rung of the
proven ladder uses explicit `cuMemAlloc` — `C: tests/mode2/cup2.c:19`, `cupctx2.c:64`,
`cup8.c:65`, `cup8_iter.c:46`. **The `cup8` result at `bad=0 maxerr=0` touches no managed
memory.**

### 1.2 The adjacent mode where the host *did* own residency: broken, and measured broken by the Mode-1 microbench at `C: docs/perf/forwarding_latency_decomposition.md:136`

Mode 1 forwards the guest's UVM ioctls to the **host** UVM on the **real** GPU — the closest
realisation of "the host replays for us" that has ever existed in this project. Its measured
result: *"**UVM managed memory is broken** — `cuMemAllocManaged` returns null in the guest
(CPU-touch then segfaults); the managed/demand-paged path isn't supported"*
(`C: docs/perf/forwarding_latency_decomposition.md:136`, `:147-148`). ★ The one place the
hypothesis was actually testable, it failed.

### 1.3 The structural break: the guest UVM does not map until it faults

The model needs the guest's GPU-side mapping for the managed range to *exist and be static*
("held *static* as 'resident in sysmem, GPU is DMA-ing it'", `:39-41`). But UVM creates GPU
mappings for a managed range **from the fault-servicing path** — `service_fault_batch` →
`uvm_va_block_service_locked` (`ogkm-580: kernel-open/nvidia-uvm/uvm_gpu_replayable_faults.c:2231-2373`,
dispatch at `:2951`). With no faults delivered, no mapping is published; with no mapping
published, the address table never learns the VA (`mode2_address_table.md` §4); and the launch
that dereferences it is a **miss**.

★ So the mechanism the doc relies on to keep the guest UVM quiescent — *we never fault it* — is
the same mechanism that prevents the binding existing. The two halves of the model are in
tension, and §1.2 is what that tension looks like when it is run.

⊘ **What I could not determine:** whether libcuda or UVM eagerly maps a managed range under some
default policy (`UvmSetAccessedBy`, a `cudaMemAdvise` default, or a "GPU is the only accessor"
heuristic) that would sidestep the first fault. The policy plumbing is in closed userspace and I
found no in-tree answer. If such a default exists, §1.3 weakens and the locked-out set in §5
shrinks. **This is the single most valuable thing a live run could settle**, and it is one
`cudaMallocManaged` + `cudaMemPrefetchAsync` program away.

### 1.4 What survives, and it is worth keeping

The *residency* half is sound and unaffected: where the bytes physically live, and who migrates
them, genuinely can be host-owned, because host UVM runs on the host GPU under the host driver.
The **accepted limitation** at `:87-90` (the guest cannot oversubscribe to its own backing store)
remains correct and remains cheap.

**Restated correctly:** *the host can own residency; we still have to own the fault.*

---

## 2. What NVIDIA actually does with resume-from-fault — the readings

### 2.1 Replay is a pushbuffer method, not a register poke

The replay types are `START` and `START_ACK_ALL` — *"Completes when all fault replays are
in-flight"* / *"Completes when all faulting accesses have been correctly translated or faulted
again"* (`ogkm-580: kernel-open/nvidia-uvm/uvm_hal_types.h:496-506`). UVM uses **only** those two;
`CANCEL_TARGETED` / `CANCEL_GLOBAL` are separate HAL entry points
(`ogkm-580: kernel-open/nvidia-uvm/uvm_hal.h:827-830`). `[src@580]`

The issue point is a host method on UVM's own channel — `uvm_hal_volta_replay_faults`,
`ogkm-580: kernel-open/nvidia-uvm/uvm_volta_host.c:234-264`:

```c
    if (type == UVM_FAULT_REPLAY_TYPE_START)
        replay_value = HWCONST(C36F, MEM_OP_C, TLB_INVALIDATE_REPLAY, START);
    ...
    NV_PUSH_4U(C36F, MEM_OP_A, ..., MEM_OP_D, HWCONST(C36F, MEM_OP_D, OPERATION,
                                                      MMU_TLB_INVALIDATE_TARGETED));
```

pushed by `push_replay_on_gpu` on `UVM_CHANNEL_TYPE_MEMOPS`
(`ogkm-580: kernel-open/nvidia-uvm/uvm_gpu_replayable_faults.c:503-543`). Encodings:
`NVC36F_MEM_OP_C_TLB_INVALIDATE_REPLAY_START = 0x1`, `_START_ACK_ALL = 0x2`
(`ogkm-580: kernel-open/nvidia-uvm/clc36f.h:140-145`, `clc56f.h:145-150`). `[src@580]`

A **register**-based replay does exist, in the `REPLAY` field at bits `5:3` of the same
`NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE` register the guest already hammers
(`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_vm.h:141-147`) — but CPU-RM only ever
writes `CANCEL_*` there, and only on the vGPU-guest path
(`kgmmuFaultCancelIssueInvalidate_GP100`,
`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/pascal/kern_gmmu_gp100.c:236-295`, sole caller
`kgmmuFaultCancelTargeted_VF`, `ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:2941-2960`).
**CPU-RM never issues `REPLAY_START`.** `[src@580]`

### 2.2 ★★ With Confidential Compute off, UVM owns the whole loop — and nothing is privileged to us

This is the finding that makes the feature reachable at all.

| step | who does it | where | reaches us as |
|---|---|---|---|
| allocate the buffer | guest CPU-RM | `kgmmuFaultBufferReplayableAllocate_IMPL`, `ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1192-1290` | — |
| tell the GPU where it is | guest CPU-RM → GSP | `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER` (`0x20800a9b`), sent at `kern_gmmu.c:1257-1262`; kernel-side receiver is a no-op printf at `kern_gmmu.c:3132-3150` | ★ **a GSP control RPC — we are the GSP, so we are told the PTE list** |
| map the buffer into UVM | guest RM | `nvGpuOpsInitFaultInfo` → `MapToCpu` on the `0xB069` object, `ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:9229-9237` | — |
| hand UVM the register pointers | guest RM | `kgmmuGetFaultRegisterMappings_TU102`, `ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:186-231` — **raw kernel-mapped BAR0 addresses** | — |
| read `PUT`, write `GET` | **guest UVM, directly over BAR0** | `ogkm-580: kernel-open/nvidia-uvm/uvm_volta_fault_buffer.c:39-85`; *"Slow path: read the put pointer from the GPU register via BAR0 over PCIe"* at `uvm_gpu_replayable_faults.c:332-333` | ★ **BAR0 MMIO we already trap** |
| the interrupt | GPU | vector 64 ⇒ `CPU_INTR_LEAF(2)` bit 0 at BAR0 `0xB81008` (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_fb.h:31-32`) | ★ **an MSI-X we already raise** |
| the replay | guest UVM | §2.1's pushbuffer method | ★ **a channel we already gate and decode** |

The CC divergence is explicit: `kgmmuGetFaultRegisterMappings_GH100` falls straight through to the
Turing HAL unless `gpuIsCCFeatureEnabled && gpuIsGspOwnedFaultBuffersEnabled`
(`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/hopper/kern_gmmu_gh100.c:352-400`), and
`bIsGspOwnedFaultBuffersEnabled` is only ever set inside the CC block
(`ogkm-580: src/nvidia/src/kernel/gpu/conf_compute/conf_compute.c:304-321`). A *replayable* shadow
buffer can only be registered under CC — `NV_ASSERT_OR_RETURN(gpuIsCCFeatureEnabled(pGpu),
NV_ERR_NOT_SUPPORTED)` at
`ogkm-580: src/nvidia/src/kernel/gpu/mmu/mmu_fault_buffer_ctrl.c:148`. **CC is off in our target**
(`mode2_rewrite_design_decisions`). `[src@580]`

### 2.3 The two controls, checked for privilege as instructed

| control | flags | class | who can issue it | does it reach us? |
|---|---|---|---|---|
| `NV2080_CTRL_CMD_MC_CHANGE_REPLAYABLE_FAULT_OWNERSHIP` (`0x2080170c`) | `0x4` = `RMCTRL_FLAGS_PRIVILEGED` (`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:6415-6428`) | admin or kernel | UVM, from the guest kernel | ★ **No.** Its whole implementation is a guest-local flag flip: `pKernelGmmu->uvmSharedIntrRmOwnsMask &= ~RM_UVM_SHARED_INTR_MASK_MMU_REPLAYABLE_FAULT_NOTIFY` (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:84-105`). It never touches hardware and never RPCs. |
| `NV2080_CTRL_CMD_GPU_REPORT_NON_REPLAYABLE_FAULT` (`0x20800177`) | `0x40040` = `ROUTE_TO_PHYSICAL \| PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST`, no PRIV/NON_PRIV/INTERNAL bit ⇒ **KERNEL_PRIVILEGED** (`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:940-953`) | kernel only | UVM | **Yes** — `ROUTE_TO_PHYSICAL` means it arrives at us as a GSP control RPC. |

Enforcement: `rmControlValidateClientPrivilegeAccess`,
`ogkm-580: src/nvidia/src/kernel/rmapi/control.c:675-712`; flag values
`ogkm-580: src/nvidia/inc/kernel/rmapi/control.h:170-239`. `[src@580]`

★★★ **The isolate's dropped capabilities are irrelevant to all of this.** Nothing on the
replayable-fault path asks us to perform a privileged operation *on the host*. Every step is
between the guest and us, and the privilege that matters is the guest's own, checked inside the
guest kernel. The concern that our capability-surrender (`2575177`) forecloses replay is
**refuted**.

---

## 3. ★★★ The reframe: our stall unit is the submission, not the access

The premise in the brief — *"On real silicon the GPU's MMU is the reader and can stall. In Mode 2
nothing walks the guest's page tables … We are a reader that cannot wait"* — is **half right, and
the half it gets wrong is the load-bearing one.**

We *can* wait. The `#14` ring-gate already does exactly that: an unmapped VA in a submission's
working set is refused, the doorbell is never rung, and the work is held
(`kayfabe_fwd::plan_doorbell` → `VerbPlan::gated_doorbell` → `FwdFault::Address(Miss)`,
`simulated_gpu_fault.md:24-28`). That **is** a fault-and-stall. What we cannot do is wait *after*
we have rung the host doorbell.

So the two capabilities separate cleanly:

| | silicon | us |
|---|---|---|
| **stall granularity** | one warp's memory access, mid-instruction | one submission, at the doorbell |
| **resume** | `REPLAY_START` → uTLBs re-issue the stalled accesses | re-resolve the working set, ring the doorbell |
| **what must be known to stall** | nothing — the MMU discovers it | ★ **the address, before we submit** |

A coarser stall is legal and invisible to the guest: the 32-byte fault packet carries a VA, an
instance-block address, an engine and a client — **not a program counter**
(`simulated_gpu_fault.md:69-80`, from
`ogkm-580: src/common/sdk/nvidia/inc/class/clc369.h:31-71`). There is nothing in it that lets the
guest tell "one warp stalled" from "the submission never started".

⇒ **The permanent limitation is not replay. It is pre-flight working-set determination.** Every
entry in §5's locked-out set traces to that one row of the table, and nothing else does.

---

## 4. Measurements

### 4.1 M1 — the "torn entry" is an artifact of our instrument

**Scan:** `cap1_coldboot_hermetic` (359 062 records), `cap3_matmul_forwarding` (532 824),
`cap2b_stalequeue_nofn47` (862 940), 2026-08-01, decoder record format per
`scripts/mode2_diag/rec_dump.py`. `[meas]`

| | cap1 | cap3 |
|---|---|---|
| MMIO accesses of width **8** (read or write), anywhere | **0** | **0** |
| MMIO accesses of width 4 | all 358 090 | all 405 235 |
| BAR2 `lo`→`hi` 4-byte pairs on one 8-byte unit | 88 920 | 107 268 |
| …of which the two records are **sequence-adjacent** | **88 920 (100 %)** | **107 268 (100 %)** |
| …with *anything* intervening | 0 | 0 |
| …with an `MMU_INVALIDATE` intervening | 0 | 0 |

**The cause is the observation path, and it is provable from the source.** `nvkvm_bar2_ops` and
`nvkvm_aperture_ops` declare **no `.impl`** (`C: src/qemu/nvkvm_gpu_emul.c:6621-6627` and
`:4758-4763`), so QEMU applies its default: `if (!access_size_max) { access_size_max = 4; }`
(`/workspace/bench/qemu-10.2.4/system/memory.c:540-541`), and for a `DEVICE_LITTLE_ENDIAN` region
the split loop runs `for (i = 0; i < size; i += access_size)` — **low dword first**
(`:551-555`). `nvkvm_bar0_ops` declares `impl.max_access_size = 4` explicitly
(`C: src/qemu/nvkvm_gpu_emul.c:4571`). QEMU's re-entrancy guard (`memory.c:545-554`) additionally makes the
two halves un-interleavable, which is why adjacency is 100 %.

⇒ **The trace cannot distinguish an 8-byte guest store from two 4-byte guest stores.** 0 of
196 188 pairs interleaved is itself evidence for the split hypothesis — a genuinely
two-instruction publication on a multi-vCPU guest would be expected to interleave at least once.

★ **The caveat this attaches to `gmmu_publication_discipline.md`:** §1's *"on the wire it is
definitively not atomic"* and §6.2(1)'s tearing hazard must be restated as — *our observer
definitely tears; whether the guest CPU or the silicon does is not established by this trace.*
That doc's §9 item 4 already flagged the silicon half as open; this adds that the **guest-CPU
half is open too**, which it did not. The security argument in §6.2 survives unchanged, because it
is an argument about **our** walker, which definitely can tear.

**Non-aperture path:** all pairs are BAR2. But there is a second write path into the same
framebuffer backing — **PRAMIN**: 33 978 4-byte BAR0 writes in the `0x700000`–`0x7fffff` window,
**identical in both captures**, 16 460 of them after the first invalidate. `[meas]` They land in
the same `fb_pages` as BAR2 (`C: src/qemu/nvkvm_gpu_emul.c:1474-1480`), and the C's own comments name it as
a page-table path — *"PRAMIN window (BAR2/page-table setup hammers it)"* (`C: :1657`) and
*"kernel's page tables (already in FB via PRAMIN)"* (`C: :3517`). PRAMIN is architecturally a
32-bit windowed register aperture, so a tear there would be real — but our instrument cannot
distinguish it either.

### 4.2 M2 — "counted is not complete": enumerate the transports, then check each

⊘ **131 was a count over one transport.** Here is the enumeration, and what the trace can and
cannot say about each. A GPU VA→physical binding can change by:

| # | transport | visible in this trace? | invalidate we can see? |
|---|---|---|---|
| 1 | CPU-RM writes PTE/PDE through **BAR2** | yes — 214 552 writes in cap3 `[meas]` | yes — 308 `0xB830B0` writes, each naming exactly one PDB `[meas]` |
| 2 | CPU-RM writes through **PRAMIN** | yes — 33 978 `[meas]` | not separable from (1)'s invalidates |
| 3 | **CE-written page tables** (the `#13` mechanism) | **no** — only as `GuestRead` of the copy source | the C's `MEM_OP` zero came from a separate run (`mode2_14_concurrent_apps` round-6, 2026-07-22), never from this trace |
| 4 | UVM's `MEM_OP` `MMU_TLB_INVALIDATE` pushbuffer method | **no** — it lives in a pushbuffer in guest RAM | ditto; `gmmu_publication_discipline.md` §9 item 3 already lists this as unre-measured |
| 5 | **PDB rebind** — writing the instance block's page-directory pointer | as an FB write, indistinguishable from data | ★ changes *every* mapping of a channel with **no PTE write at all** |
| 6 | RPC to us (we are the GSP): `UPDATE_BAR_PDE`, map/unmap controls | as msgq `GuestRead`/`GuestWrite` | none needed — it is a message, not a memory write |
| 7 | **PDE clear at teardown** | one BAR2 write | ★ `_mmuWalkPdeRelease` frees the child's backing store **before any invalidate** (`ogkm-580: src/nvidia/src/libraries/mmu/mmu_walk.c:1509-1552`) |
| 8 | **valid→valid remap** (`PTE_DOWNGRADE`) | a BAR2 write | ★ **zero downgrade invalidates in any capture** — all 786 across three captures are upgrades `[meas]` |
| 9 | physical page reuse under a still-valid mapping | nothing | no transaction exists |
| 10 | the guest CPU's own page tables under ATS | nothing | not applicable on our target (§5, S3) |

**Decoded invalidate stream**, all three captures, correct field positions from
`ogkm-580: src/common/inc/swref/published/ampere/ga100/dev_vm.h:63-118`: `[meas]`

| capture | writes | `HUBTLB_ONLY=1` | `=0` | distinct PDBs | `REPLAY` |
|---|---|---|---|---|---|
| `cap1_coldboot_hermetic` | 139 | 89 | 50 | 6 | **`NONE` ×139** |
| `cap3_matmul_forwarding` | 308 | 177 | **131** | 9 | **`NONE` ×308** |
| `cap2b_stalequeue_nofn47` | 139 | 89 | 50 | 6 | **`NONE` ×139** |

Every one carried `TRIGGER=1, ALL_VA=1, ALL_PDB=0, SYS_MEMBAR=0, ACK=NONE_REQUIRED,
INVAL_SCOPE=NON_LINK_TLBS, CACHE_LEVEL=ALL, USE_PASID=0`. The `HUBTLB_ONLY` counts now reproduce
`gmmu_publication_discipline.md` §0's table exactly, at the correct bit (`2:2`, not 20).

**Coverage on transport (1), the only one this instrument can measure:** taking the 4 KiB BAR2
pages written immediately before some invalidate as "page-table-like" (26 distinct pages in cap3),
45 328 of 214 552 BAR2 writes land on them; **43 662 have some later invalidate, 1 666 are still
uncommitted when the capture ends.** `[meas]`

⇒ **The honest answer to "is every mapping change followed by an invalidate we can see?" is
no** — high coverage on transport (1), and transports (3)–(10) are either invisible to this
instrument or carry no invalidate by construction. This is the same shape of error the brief
warned about, caught before it was repeated.

### 4.3 ★★★ M3 — the guest's replayable fault machinery is ALREADY LIVE against us

This was not asked for, and it is the most important thing this note measured — a scan of
`cap3_matmul_forwarding` and `cap1_coldboot_hermetic`, 2026-08-01. `[meas]`

**In `cap3_matmul_forwarding`, on the compute path** (`cup8`, `bad=0 maxerr=0`): `[meas]`

| BAR0 offset | register | accesses |
|---|---|---|
| `0xB83028` | `MMU_FAULT_BUFFER_GET(1)` — **replayable** | 1 read, **6 writes** |
| `0xB8302C` | `MMU_FAULT_BUFFER_PUT(1)` — **replayable** | **7 reads** |
| `0xB83070` | `MMU_PAGE_FAULT_CTRL` | 1 read, 1 write |
| `0xB83110` | `ACCESS_COUNTER_NOTIFY_BUFFER_SIZE` | 1 read → `0x100` |
| `0xB83008` / `0xB8300C` | the **non**-replayable buffer's GET/PUT | **0** |

`cap1_coldboot_hermetic` has **none** of this — the boot alone never touches the fault block. It
is CUDA, through UVM, that arms it.

**The six `GET` writes are three instances of one exact function.** They occur in pairs, `0x0`
then `0xC0000000`, and that is verbatim `uvm_hal_volta_fault_buffer_write_get`
(`ogkm-580: kernel-open/nvidia-uvm/uvm_volta_fault_buffer.c:57-85`): write the index, then a
second write OR-ing in `GETPTR_CORRUPTED_CLEAR | OVERFLOW_CLEAR` — with the comment that the
second write is skipped **only** `if (g_uvm_global.conf_computing_enabled)`. CC is off, so UVM
writes the real register, and it did.

**And the interrupt path is already wired to it.** `cap3` contains 5 `IrqRaise` records. One is
the driver's own `INTR_LEAF_TRIGGER` self-test at boot (`0xB81640 <- 0x81`, identical to cap1's
single one). **The other four are our GSP MSI-X (vector 0), and the first BAR0 register the guest
reads after each of them is `0xB8302C` — the replayable fault buffer's PUT pointer.** `[meas]`

```
   #420583   GWr a=0x10227b000 len=4096
   #420584   GWr a=0x102241010 len=4
   #420585   IRQ msix vec=0            <- our interrupt
   #420586   Rd  bar0 0x00b8302c = 0   <- MMU_FAULT_BUFFER_PUT(1)
```
…and identically at records 422856, 429197, 433577.

⇒ **The guest's replayable-fault top half runs, against us, four times, in the C's own matmul
capture.** It reads `PUT`, sees `0` (the C has no handler for the block — every offset except
`0xB83110` falls through `default: return 0`, `C: src/qemu/nvkvm_gpu_emul.c:1582`), and returns. The path
is inert **only because `PUT` never moves.** Every other link in the chain is already connected.

That is what makes §5's judgements "solvable" rather than "hopeful": the remaining work is to move
one pointer and write 32 bytes, not to invent a transport.

---

## 5. The scenarios, judged (1) plausible? (2) what happens today? (3) solvable?

### S1 — UVM / `cudaMallocManaged`, on-demand paging, oversubscription

**(1)** Yes. `pageableMemAccess` is advertised to userspace whenever
`replayable_faults_supported` (`ogkm-580: kernel-open/nvidia-uvm/uvm_gpu.c:3861`), true on
Pascal+ (`uvm_ampere.c:81`). Managed memory is the default recommendation in modern CUDA.

**(2)** The guest arms the buffer and polls it (§4.3, `[meas]`); `PUT` never moves; no fault is
serviced; **no GPU-side mapping is ever published for the managed range** (§1.3); our table
misses; the `#14` ring-gate refuses the doorbell and *nothing tells the guest*
(`simulated_gpu_fault.md:24-28`) ⇒ **the application hangs on a semaphore that will never be
released.** ⚠ With `#111` wired as it stands it would instead receive `RC_TRIGGERED` / Xid 31 —
which is **worse**, because the application did nothing wrong: it is precisely the case-B
misattribution `simulated_gpu_fault.md:162-178` forbids, and §S6 below shows the RC has a blast
radius no application handles.

**(3) Solvable — this is the one that pays for the fault buffer.** Every piece is ours (§2.2), and
`simulated_gpu_fault.md:259-269` already named this as the one path its own deferral does not
cover. The build is §7 step 5. ★ **But only for the first-touch case** — where the faulting
address is derivable from the submission (a pointer in the launch's parameter buffer, a copy
descriptor). A pointer the *kernel computes at runtime* is unreachable, per §3.

### S2 — Access counters and migration heuristics

**(1)** No application API; UVM's own heuristics only. The guest does read
`ACCESS_COUNTER_NOTIFY_BUFFER_SIZE` once, and we answer 256 (§4.3, `[meas]`).

**(2)** We advertise a 256-entry (8 KiB) notify buffer and never write one entry, never raise its
interrupt. Nothing breaks; the migration heuristics simply never fire. ⚠ That 256 is a **lie of
convenience** the C added deliberately, and it is load-bearing: *"0 => memdescCreate(0) =>
NV_ERR_INVALID_ARGUMENT (access_cntr_buffer.c:72) => UVM register fails => cuInit bails"*
(`C: src/qemu/nvkvm_gpu_emul.c:1575-1580`). Answering honestly breaks `cuInit`.

**(3) Not needed, and out of scope — permanently.** Access counters carry **no replay
dependency**: `uvm_gpu_access_counters.c` contains zero occurrences of `replay`; notifications are
retired by `access_counter_clear_all` / `_clear_targeted`
(`ogkm-580: kernel-open/nvidia-uvm/uvm_gpu_access_counters.c:233-295`). They are a pure
*optimisation*. The correct action is not to build them but to **write the lie down** as a
deliberate, reasoned answer with its citation, so nobody later mistakes it for a modelled feature.

### S3 — ATS / HMM, `cudaHostRegister`, peer / NVLink

**ATS.** **(1)** Requires platform PASID/ATS support; `USE_PASID` was `0` in all 786 invalidates
across three captures `[meas]`, and our target is a commodity PCIe GA10x. **(2)** ATS is serviced
*inside* the replayable batch (`uvm_ats_service_faults` at
`ogkm-580: kernel-open/nvidia-uvm/uvm_gpu_replayable_faults.c:1718`, reached at `:1985`), so with
no faults it never runs. **(3) Out of scope by hardware target**, not by design limitation.

**HMM.** Rides the same `service_fault_batch` dispatch as S1
(`uvm_gpu_replayable_faults.c:2247`, `:2331-2333`). **Solved-or-not with S1**; no separate work.

**`cudaHostRegister`.** **(1)** Yes, common. **(2)/(3)** **Not locked out.** No fault or replay
dependency exists — the only in-tree mentions are VA-range constraints
(`ogkm-580: kernel-open/nvidia-uvm/uvm.h:150, 262, 296`). It is an **eager** mapping, so the
binding *is* published and our table *does* learn it, by the ordinary route.

**Peer / NVLink.** **(1)** Not on a single GA106. **(2)/(3)** No replay dependency found; peer
setup is RM-control driven. If multi-GPU arrives it is a **mapping** problem
(`multi_gpu_and_mig.md`), not a fault problem.

### S4 — Debugger / profiler

★ **The distinction that decides this: SM exception resume is not MMU fault replay.** They share
no code.

**(1)** Reachable by an unprivileged guest application. `GT200_DEBUGGER` (`0x83DE`) allocates with
`RS_FLAGS_ALLOC_NON_PRIVILEGED` (`ogkm-580: src/nvidia/src/kernel/rmapi/resource_list.h:186-196`),
and `DEBUG_SUSPEND_CONTEXT` (`0x83de0317`) / `DEBUG_RESUME_CONTEXT` (`0x83de0318`) carry flags
`0x10248` — **NON_PRIVILEGED** (`ogkm-580: src/nvidia/generated/g_kernel_sm_debugger_session_nvoc.c:562, :577`).
That is the cuda-gdb path.

**(2)** Those controls are `ROUTE_TO_PHYSICAL`, so they arrive at us as GSP control RPCs — and the
C answers the generic fall-through with `body.status = NV_OK` and an unmodified params echo
(`C: src/qemu/nvkvm_gpu_emul.c:3057`, `:3435`). ⇒ **A debugger attaches successfully, sets breakpoints
successfully, and none of them ever fire.** That is a false green, this project's most-repeated
failure class.

**(3) Locked out — and it is *not* a replay problem.** It needs SM error state, warp trap
handling and single-step on a GR context whose golden image is the silicon boundary we forward
across (`mode2_fakeboot_complete`). The correct action is cheap and is **not** to build it:
answer the `0x83DE` class and its controls with an explicit `NV_ERR_NOT_SUPPORTED`, converting a
silent false green into an honest failure.

⚠ **One sub-case that is in scope and easy to miss.** *MMU debug mode*
(`NV83DE_CTRL_CMD_DEBUG_SET_MODE_MMU_DEBUG`, `0x83de0307`, also NON_PRIVILEGED) changes what RM
does on a fault: `kgmmuServiceMmuFault_GV100` queries it and **only if it is disabled** writes the
error notifier and resets the channel
(`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:2059-2073`, `:2207-2211`).
If we ever emit an RC we must honour that flag, or we kill a context a debugger explicitly asked
us to preserve.

### S5 — ★★ The guest gets a fault from us and tries to recover

This is the owner's specific worry, and the readings are sharp.

**(1) Reachable?** Not today: `#111` is built at the decision/encode/transport layers and
**nothing calls it in production** (`simulated_gpu_fault.md:353-356`). It becomes reachable the
moment the doorbell path is wired.

**(2) What happens when it fires.** Three findings, in increasing severity.

**(a) The error notifier is not written — confirmed verbatim, not inferred.**
`_kgspRpcRCTriggered`, `ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:657-668`:
```c
    bIsCcEnabled = gpuIsCCFeatureEnabled(pGpu);
    // With CC enabled, CPU-RM needs to write error notifiers
    if (bIsCcEnabled && pKernelChannel != NULL)
    {
        NV_ASSERT_OK_OR_RETURN(krcErrorSetNotifier(...));
    }
    return krcErrorSendEventNotifications_HAL(...);
```
CC off ⇒ **CPU-RM does not call it.** Corroborated in prose at
`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:85-90` (*"except in the
GSP_CLIENT path where GSP has already written to the notifiers"*) and `:352-372`. `#111`'s §8
listed this as `[inferred]`; it is now **read directly, with the conditional quoted**.

**(b) ★ The in-tree consumer of that notifier spins forever without it.**
`uvm_channel_get_status` returns `NV_OK` when `error_notifier->status == 0`
(`ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:2058-2082`), and the waiters are `while (1)`
loops whose only exits are forward progress or a non-zero notifier
(`channel_reserve_in_pool` `:603-627`, `uvm_channel_manager_wait` `:660-676`). `UVM_SPIN_LOOP`
never bails — on timeout it prints *"Warning: stuck waiting for %llus"* and returns, and these
callers **ignore its return value** (`ogkm-580: kernel-open/nvidia-uvm/uvm_common.h:288-298`).
⇒ **Sending `RC_TRIGGERED` without writing the notifier can cause exactly the hang `#111` exists
to replace.** ⊘ Scope this honestly: that is UVM's channel path. What *libcuda* does with an
application channel is closed and I could not determine it — but the consumer we can read hangs,
and that is the shape to design against.

**(c) ★★ The blast radius is not "sticky context". It is "reboot required".**
`krcErrorSetNotifier_IMPL` carries a WAR:
```c
    // WAR bug 4503046: mark reboot required when any UVM channels receive an error.
    if (pKernelChannel->bUvmOwned) { sysSetRecoveryRebootRequired(pSys, NV_TRUE); }
```
(`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:255-262`). And UVM's own fatal
error is **process-global and never cleared** outside test builds —
`atomic_cmpxchg(&g_uvm_global.fatal_error, NV_OK, error)`
(`ogkm-580: kernel-open/nvidia-uvm/uvm_global.c:420-445`), header contract *"Once that happens the
driver should refuse to do anything other than try and clean up as much as possible"*
(`uvm_global.h:262-266`) — reported onward by `nvGpuOpsReportFatalError`, which logs *"requiring
os reboot to recover"* and calls `sysSetRecoveryRebootRequired`
(`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:11538-11549`). NVIDIA even notes the
cross-GPU spread: *"UVM currently attributes all errors as global and fails operations on all
GPUs"* (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:702-707`).

**And what does the guest's "recovery attempt" actually do?** Not a replay. RM's post-RC action is
**kill and reset**: `NV906F_CTRL_CMD_RESET_CHANNEL`, whose CPU-side implementation only forwards
`bIsRcPending` and RPCs onward — *"All real hardware management is done in the host"*
(`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:3036-3082`). `bIsRcPending` is
**never set to true anywhere in the open tree**; it is only *cleared*, at `:3026-3028`. So the
guest's recovery arrives **at us**, as a GSP RPC, and today the C answers it `NV_OK` and does
nothing.

**(3) Solvable — yes, cheaply, in three parts.**
- **Write the error notifier ourselves.** We are the GSP; that is our job under this split. The
  shape is fully readable: `krcErrorSetNotifier_IMPL` →
  `krcErrorWriteNotifier_HAL(..., 0xffff /* notifierStatus */, ...)`
  (`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:234-350`, write at `:330-338`), landing in the client's
  ctxdma or `Memory` notifier via `krcErrorWriteNotifier_CPU` (`:92-232`).
- **Never emit for a `bUvmOwned` or system-component channel.** `#111`'s A/B rule already refuses
  case B by declared client kind (`simulated_gpu_fault.md:180-185`). (c) gives that rule a
  **second, independent reason** — one mis-attributed fault on a UVM channel marks the guest's GPU
  reboot-required — and that reason belongs in the rustdoc so nobody relaxes the rule later.
- **Honour `RESET_CHANNEL` for real** — "nothing" must mean "we really did make the channel
  resettable", not "we ignored the RPC".

### S6 — The sticky-fatal claim (`l1_concurrency.md:928`)

> *"A GPU that faults a channel does not silently hand back a fresh context; it makes the context
> **sticky-fatal** until the application tears it down and builds a new one. Every CUDA
> application already handles that path, because Xids exist."*

**Is it true?** As a justification for refusing to resurrect a dead isolate — **yes**, and the
mechanism holds, and it is a **reading**, not a run: there is no replay after an RC anywhere in
RM; recovery is kill + reset (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:3036-3082`);
`bIsRcPending` gates further use until an explicit reset (`:3026-3028`). Three corrections, though:

1. ⚠ **Understated.** For a **UVM-owned** channel it is not context-sticky, it is *process-global
   and system-global* (S5(c)). No application "already handles" *"the driver now says reboot"*.
2. ⚠ **Overstated in its premise.** "Because Xids exist" assumes the application *sees* one. With
   CC off and no notifier write from us, it may see nothing at all (S5(a)/(b)).
3. ★★★ **It does not survive a guest that expected a replayable fault, and that is the crisp
   answer to the owner's question.** A replayable fault is *by construction* non-fatal: it stalls,
   is serviced, is replayed, and the program continues. Answering one with an RC converts a
   **recoverable** event into a **fatal** one — and, for a UVM channel, into a reboot-required
   one. So the sticky-fatal argument is sound where `l1_concurrency.md` uses it (the isolate died;
   the data is gone; fail loudly) and is **not a licence to answer a demand-paging fault with an
   RC.** Those are different events and they must stay different.

---

## 6. Verdict on the owner's trap-on-transition design

> *"PDB writes are heavy. Only trap on the final mark — valid ⇒ our sync to GPGA: allocate if not
> already, do the mmaps. And trap on pages being set invalid ⇒ our sync: dealloc if no reference
> and not reserved, munmap."*

**The instinct is right and the economics are right.** Trapping edges rather than stores is the
correct shape, and it is what makes 100 % framebuffer passthrough thinkable at all.

**But as stated it is not complete.** Seven ways a mapping becomes reachable or unreachable
without crossing the edge as described — each with its citation, ordered by how likely it is to
bite:

1. ★★ **The edge is on the wrong object: validity ≠ reachability.** UVM publishes **children
   first, fenced, then parents bottom-up** — *"write entries bottom up, so that they are valid
   once they're inserted into the tree"*
   (`ogkm-580: kernel-open/nvidia-uvm/uvm_mmu.c:771-782`). So a leaf is written `VALID=1` while
   its parent PDE is still invalid: the leaf's invalid→valid edge is **not** the moment the
   mapping becomes reachable. The reachability edge is the **parent PDE publication**, and one
   such write can make up to 512 leaves reachable at once. ⇒ The design must track *reachability*,
   which means: on a PDE publication, enumerate the newly-reachable subtree.
2. ★★ **…but enumerating means walking, and walking is exactly what §6.2 forbids.**
   `mmuWalkReserveEntries(..., bInvalidate = NV_FALSE)` leaves a level reachable with
   **uninitialised backing store** (`ogkm-580: src/nvidia/src/libraries/mmu/mmu_walk_reserve.c:57-63`,
   `:85`), so a walker reads allocator residue as PTEs — *"wrong physical page, and a
   cross-context read"* (`gmmu_publication_discipline.md` §6.2(2)). **(1) and (2) are in direct
   tension.** The only resolution: walk to *enumerate candidates*, but bind only entries we also
   **witnessed being written**. Anything reachable-but-unwitnessed stays a miss. ⇒ which is a
   fault, which is §7 step 5.
3. ★★ **Teardown crosses no leaf edge at all.** `_mmuWalkPdeRelease` clears the parent PDE first
   and **frees the sub-level backing store second, with no TLB invalidate between the two**
   (`ogkm-580: src/nvidia/src/libraries/mmu/mmu_walk.c:1509-1552`; the invalidate happens later at
   the caller, `gpu_vaspace.c:1803-1811`). Hundreds of leaves are unmapped by one PDE write and
   are **never written invalid** — the memory is simply recycled. A design watching leaf entries
   misses the whole unmapping, and then misparses the recycled page's next contents as PTE writes.
   ⇒ a PDE clear must invalidate our shadow of the entire subtree **and** retire those FB pages
   from "this is a page table".
4. ★ **valid→valid is a mapping change.** A remap changes the physical address without ever
   passing through invalid; RM drives `update_type = PTE_DOWNGRADE` for it
   (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/virt_mem_allocator_gm107.c:2602-2606`).
   Neither proposed edge fires. ⚠ And we have **no observation of it**: 0 of 786 invalidates across
   three captures is a downgrade `[meas]`. Same for a **protection-only** change (RO→RW, privilege
   bit) — not an address change, but granting more access than the guest intended.
5. ★ **PDB rebind changes everything with zero entry writes.** Swapping the instance block's
   page-directory pointer re-points a channel's whole address space. The C already snoops
   `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` for exactly this (`C: src/qemu/nvkvm_gpu_emul.c:2736-2790`), so
   the mechanism exists — but it is not an entry edge and must be a first-class one.
6. **Three states, not two.** Sparse is a distinct fill state with its own templates
   (`MMU_WALK_FILL_SPARSE`, `ogkm-580: src/nvidia/src/kernel/gpu/mmu/gmmu_walk.c:904-935`).
   valid→sparse is an unmap; sparse→valid is a map; invalid→sparse is neither. Conflating sparse
   with valid and conflating it with invalid are *different* bugs.
7. **Level granularity is not uniform.** PDE0 is a **16-byte dual** entry (two sub-tables per
   entry) and on **GA10x PD1 is itself a 512 MiB leaf**
   (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/ampere/kern_gmmu_fmt_ga10x.c:46-53`). A design
   keyed on "leaves are PTEs" is wrong on our exact chip — and this is not hypothetical: it is
   `#13`, *"the GA10x 512M-leaf gap that silently dropped page-table writes for weeks"*
   (`simulated_gpu_fault.md:176-178`).

Plus, from §4.2: the edges must be watched on **PRAMIN** as well as BAR2 (33 978 writes into the
same FB backing `[meas]`), and transports (3), (4) and (6) do not appear as MMIO edges at all.

### 6.1 The verdict

**Adopt it — as *reachability*-on-transition, with holes 1–7 closed explicitly — and do not
believe it is complete.** With all seven closed, the residue is (a) valid→valid remaps and
protection changes, and (b) anything published by a transport we do not decode. Both are
*unobserved*, not *impossible*.

★★★ And that settles the brief's conditional. *"If replay is unavailable, this design must be
complete on its own"* — **it cannot be made complete on its own, so replay must not be
unavailable.** §2.2 and §4.3 say it need not be. The two halves of this note are the same answer.

---

## 7. Recommended strategy

Ordered by cost-to-value. Steps 1–3 are days, remove real defects, and are worth doing whether or
not managed memory is ever on the roadmap.

**0. Do not build the replayable fault buffer yet.** Nothing on the `cup8` ladder needs it. It is
step 5, and step 5 is gated on a decision (see below) that steps 1–4 inform.

**1. Stop answering `NV_OK` to things we do not implement.** Three specific sites, each a false
green today:
   - the `GT200_DEBUGGER` (`0x83DE`) class and its controls ⇒ `NV_ERR_NOT_SUPPORTED` (S4);
   - `NV2080_CTRL_CMD_GPU_REPORT_NON_REPLAYABLE_FAULT` (`0x20800177`) ⇒ refuse, not succeed;
   - `ACCESS_COUNTER_NOTIFY_BUFFER_SIZE = 256` ⇒ keep it (it is load-bearing for `cuInit`) but
     **write it down as a deliberate lie with its reason and citation**, so it is never mistaken
     for a modelled feature (S2).

**2. Write the error notifier. `#111` is a hang generator without it.** S5(a)/(b): CPU-RM does not
write it with CC off, and the in-tree consumer spins forever on a zero. This is the single
highest-value correction in the note, it is one struct write, and its shape is fully readable at
`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:234-350`.

**3. Record the reboot-required blast radius in `#111`'s A/B rule.** S5(c) gives the existing rule
a second, independent justification. A rule with two reasons survives a refactor that a rule with
one does not.

**4. Build trap-on-transition as *reachability*-on-transition, closing holes 1–7** (§6). This is
the data-plane work that has to happen regardless. Close hole 2 with the witness rule — *bind only
what you saw written* — which means accepting that misses will exist, which is the input to step 5.

**5. Emulate the replayable fault buffer — when, and only when, managed memory is on the
roadmap.** Sub-order, and 5a is free enough to do now:
   - **5a. Receive and record `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER` (`0x20800a9b`)**,
     which arrives as a GSP control RPC carrying the buffer's PTE list
     (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1257-1262`). One RPC; turns "where is the buffer" from an unknown
     into a fact. **Do this now** — it costs nothing and it de-risks everything after it.
   - 5b. Serve `MMU_FAULT_BUFFER_GET(1)` / `PUT(1)` honestly (today they fall through to
     `default: return 0`, `C: src/qemu/nvkvm_gpu_emul.c:1582`).
   - 5c. Write a 32-byte packet — all eight dwords before `VALID`, a store fence, then advance
     `PUT`, then pulse `CPU_INTR_LEAF(2)` bit 0. ★ The interrupt is a **level re-derived from
     `GET != PUT`, not an edge** (`simulated_gpu_fault.md:122-128`); treating it as an edge drops
     faults invisibly. ★ The attribution key is the **instance-block physical address**, matched by
     linear scan (`kfifoConvertInstToKernelChannel`,
     `ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:572-656`); a
     mismatch produces silence, not an error.
   - 5d. Decode `MEM_OP_D.OPERATION == MMU_TLB_INVALIDATE_TARGETED` with
     `MEM_OP_C.TLB_INVALIDATE_REPLAY == START` on UVM's `MEMOPS` channel, re-resolve the held
     submission's working set, and ring the doorbell.
   - **The gate is `#111` §9.4's negative control and it is the important one:** a *correct*
     program must complete at `bad=0 maxerr=0` and emit **zero** faults. A fault emitter that
     fires on legitimate traffic turns a working forwarder into a broken one.

**6. Accept as out of scope, and say so in the compatibility matrix rather than discovering it:**
access-counter migration heuristics (S2 — no replay dependency, pure optimisation); ATS/PASID
(S3 — hardware target); SM-level debugger and profiler trapping (S4 — not a replay problem, and it
needs the GR silicon boundary); guest-owned oversubscription to guest backing store (already
accepted at `C: docs/design/mode2_uvm_residency.md:87-90`); multi-GPU peer/NVLink.

---

## 8. The locked-out set

**Permanently locked out — not solvable within this design:**

| # | feature | why, in one line |
|---|---|---|
| L1 | Faulting on an address not determinable **before** submission — computed pointers, pointer-chasing, device-side allocation, device-side graph launch | §3: our stall unit is the submission. No fault emulation reaches it. ⚠ It only *bites* when the address is unbound; the ordinary `cuMemAlloc` case never faults. |
| L2 | Debugger / profiler SM trapping, breakpoints, single-step | S4: a different mechanism entirely; needs SM state behind the GR silicon boundary |
| L3 | Guest-driven oversubscription / eviction to the **guest's own** backing store | residency is host-owned; already an accepted limitation |
| L4 | ATS with PASID | out by hardware target, not by design |

**Not locked out — solvable, mechanism named:**

| # | feature | mechanism |
|---|---|---|
| N1 | UVM demand paging / managed memory, first-touch | §7 step 5 — every piece is already ours (§2.2, §4.3) |
| N2 | HMM | rides N1 |
| N3 | Access counters | not needed — no replay dependency; declined, not blocked |
| N4 | `cudaHostRegister`, peer mappings | eager; no fault dependency; the binding is published by the ordinary route |
| N5 | Guest recovery after a fault we emit | §7 steps 2 + 3 — notifier write, A/B refusal, honour `RESET_CHANNEL` |

★ **The set is small, and none of it is caused by an inability to replay.** L1 is caused by *when*
we can stall; L2 by *what* we model; L3 and L4 by scope. The thing the brief worried might be
permanently locked out — resume-from-fault itself — is **not**.

---

## 9. ⊘ What I could not determine

Stated plainly, because the owner is making an architecture decision on this. No plausible
mechanism has been substituted for any of them.

1. ★ **Whether libcuda/UVM ever maps a managed range eagerly**, under some default policy that
   would sidestep the first fault (§1.3). This is the highest-value open item: if such a default
   exists, N1 may already work and the case for step 5 weakens sharply. It is one
   `cudaMallocManaged` program away from being settled.
2. **What libcuda does with an application channel whose error notifier is never written.** S5(b)
   establishes the behaviour of the consumer we *can* read — UVM's — which hangs. The
   `CUDA_ERROR_ILLEGAL_ADDRESS` sticky-context mapping is closed userspace and appears nowhere in
   the vendored tree.
3. **Whether a real GA10x MMU, or the guest CPU, can produce a torn entry.** §4.1 establishes only
   that *our observer* tears, and that the trace cannot distinguish the two cases. Both halves are
   open; `gmmu_publication_discipline.md` §9 item 4 had flagged only one.
4. **Whether transports (3), (4) and (6) of §4.2 carry an invalidate on the compute path.** The C's
   zero for `MEM_OP`/`INVALIDATE_TLB` came from external instrumentation
   (`mode2_14_concurrent_apps` round-6), not from these captures, and I did not re-measure it. The
   method offsets for a re-measure are in `gmmu_publication_discipline.md` §9 item 3.
5. **What a `PTE_DOWNGRADE` invalidate looks like on the wire.** None appears in any of the three
   captures — all 786 are upgrades `[meas]`. Its predicted encoding is `SYS_MEMBAR=1,
   ACK=GLOBALLY` (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:166-179`),
   unverified.
6. **Whether a guest ever issued `0x2080170c` or `0x20800177` against the C.** The C has no handler
   and keeps no per-control census, and I did not decode the msgq RPC stream to check.
7. **What the four MSI-X in `cap3` were actually announcing.** §4.3 establishes that the guest read
   the replayable `PUT` first after each; it does not establish what the interrupt was *for*.
   Given `IrqRaise == 1` in the hermetic boot and the C's self-deadlock latch
   (`C: src/qemu/nvkvm_gpu_emul.c:1861-1863`), the four are probably GSP status-queue announcements — but I
   did not confirm it, and the ordering (fault-buffer check *before* the leaf scan) is worth a
   second look by whoever builds step 5.

---

## 10. Provenance

- Source read: `ogkm-580.159.04` — the bench's tag. The vendored 610.43.02 tree was not used
  (`ogkm_is_versioned`).
- Captures scanned 2026-08-01: `cap1_coldboot_hermetic` (359 062 records),
  `cap3_matmul_forwarding` (532 824), `cap2b_stalequeue_nofn47` (862 940). All three parsed dense
  and complete against `scripts/mode2_diag/rec_dump.py`'s record format; record counts matched
  their headers exactly.
- QEMU access-size behaviour read from `/workspace/bench/qemu-10.2.4/system/memory.c` — the tree
  the bench's binary is built from.
- ⊘ **No hardware run was available for this note.** Everything marked `[src@580]` or `[C:]` is a
  reading; everything marked `[meas]` is a scan of a committed recording of a real stock driver,
  not a live run.
