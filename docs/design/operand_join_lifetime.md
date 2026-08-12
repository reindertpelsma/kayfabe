# The joined leaf's LIFETIME — who owns it, what ends it, and what happens when nothing does

**STATUS: LIVE — 2026-08-13.** Design, written **before** any release path is wired, at the
owner's explicit instruction:

> *"make sure that the best solution doesn't become a retrofit. and that if we need some kind of
> pinning that we can unpin it or that we have a cleanup mechanism later."*

⊘ **Nothing here is implemented yet.** This document exists so that the join landing in `w282`
is *the fix's first half with a NAMED second half*, rather than the fix's first half plus an
unexamined leak. §5 states exactly what leaks today.

---

## 1. ★★★★★ THE OWNER'S QUESTION, ANSWERED IN `ogkm` — and the answer is the GOOD one

> *"if guest OS frees a page, so do we, then guest OS is safe to reuse, no unprivileged
> userspace has it then anymore. **Pls verify this in ogkm.**"*

**Verified, `research_clones/ogkm` @ 580.159.04. The premise holds, but NOT for the reason the
phrasing suggests — and the difference is the whole design.**

### 1.1 ⊘ The userspace free is BEST-EFFORT. Do not build on it.

The explicit path is `NV_ESC_RM_FREE` — `_IOWR('F', 0x29, NVOS00_PARAMETERS)`, declared at
`ogkm/src/nvidia/arch/nvalloc/unix/include/nv_escape.h:33`, dispatched at
`ogkm/src/nvidia/arch/nvalloc/unix/src/escape.c:503` — plus `NV_ESC_RM_UNMAP_MEMORY` (`0x4F`),
`NV_ESC_RM_UNMAP_MEMORY_DMA` (`0x58`), and UVM's `UVM_FREE` (**34**) / `UVM_UNMAP_EXTERNAL`
(**66**) (`ogkm/kernel-open/nvidia-uvm/uvm_ioctl.h:383`, `:735`).

⚠ **`UVM_UNMAP_EXTERNAL_ALLOCATION` and `UVM_MEM_UNMAP` do not exist in this driver version.**
⚠ And UVM's numbers are **raw integers**, not `_IOC`-encoded (`uvm_ioctl.h:40`) — the
`_IOC_SIZE` trap this project already banked, one plane over.

**On `SIGKILL` or OOM, userspace issues NONE of these.** A design edge-triggered on a guest
*userspace* ioctl would leak every joined leaf of every killed process. ⇒ **Refused as the
primary mechanism.**

### 1.2 ★★★ The guest KERNEL's teardown is GUARANTEED, and it is observable

Linux always calls `.release` on the last `fput`. There is no path that skips it.

```
nvidia_close                     ogkm/kernel-open/nvidia/nv.c:2231
  rm_cleanup_file_private         .../unix/src/osapi.c:3004
    RmFreeUnusedClients           .../unix/src/osapi.c:545
    serverFreeDisabledClients     .../libraries/resserv/src/rs_server.c:1046
      clientFreeResource_IMPL     .../libraries/resserv/src/rs_client.c:785
```

`RmFreeUnusedClients` carries the guarantee **in its own comment** (`osapi.c:545-577`):

> *"The `nvfp` pointer uniquely identifies an open instance in kernel space and the kernel
> interface layer **guarantees that we are not called before the associated nvfp descriptor is
> closed**. We can thus safely free abandoned clients with matching `nvfp` pointers."*

`clientFreeResource_IMPL` is **the single shared funnel** — the same function runs for the
ioctl path and the fd-close path — and it manufactures the unmaps userspace never issued:
`clientUnmapResourceRefMappings` (`rs_client.c:1145`) synthesizes a full `RS_CPU_UNMAP_PARAMS`
per live CPU mapping, and `_clientUnmapInterMappings` (`rs_client.c:1309`) does the same for
GPU-VA mappings → `gvaspaceUnmap` → **real PTE writes + a real TLB invalidate**.

UVM is the same shape and fires *earlier*, on `mm` teardown, which is exactly the SIGKILL case:
`uvm_va_space_mm_shutdown` (`uvm_va_space_mm.c:328`) stops channels, detaches them, flushes the
replayable fault buffer, and calls `uvm_gpu_va_space_unset_page_dir` (`:390`) — **a
page-directory write to hardware, issued by the kernel, with no userspace ioctl at all.**

### 1.3 ★★ What WE see, as the faked GSP

On the GSP boundary the free path is **one** RPC: **`NV_VGPU_MSG_FUNCTION_FREE` = 10**
(`ogkm/src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:20`), emitted by `rpcRmApiFree_GSP`
(`ogkm/src/nvidia/src/kernel/vgpu/rpc.c:11120`, header written at `:11142`), driven from
`serverFreeResourceRpcUnderLock` (`alloc_free.c:994`) and from each destructor
(`mem.c:177`, `device.c:246`/`:288`, `kernel_channel.c:1219`, `vaspace_api.c:572`, …).

⊘ **Page-table teardown is NOT an RPC and must not be waited for as one.**
`NV_RM_RPC_DMA_FILL_PTE_MEM` is a **compiled-out no-op** in the open modules
(`ogkm/src/nvidia/inc/kernel/vgpu/rpc_vgpu.h:42`), and `UNMAP_MEMORY_DMA` is
`rpcUnmapMemoryDma_STUB` on **every** GSP-era chip including GA10x
(`ogkm/src/nvidia/generated/g_rpc_private.h:634`). CPU-RM owns the GMMU and writes the PTEs
itself. ⇒ we see the unmap as **PTE stores + an MMU invalidate**, which is the transport
`kayfabe_fwd::ptdecode` already decodes, not as a message.

⚠ **`UNLOADING_GUEST_DRIVER` (47) is unload-only** — `kernel_gsp.c:5231`, reached from module
unload / suspend / GC6. It is **not** a per-free signal and must not be read as one.

### 1.4 ⇒ THE ANSWER, in one line

> **Event-driven unjoin IS viable — but the event is the guest KERNEL's teardown (a `FREE` RPC,
> or the PTE clear that unbinds the leaf), never a guest USERSPACE free.**
> ⚠ **And it can be arbitrarily late**, so the fallback is not optional.

Three independent deferral mechanisms can delay teardown past process death without bound:
the PM-lock-blocked `nvidia_close_deferred` kthread queue (`nv.c:2262`), UVM's
`deferred_release_q` (`uvm.c:212`), and `bUseDeferredClientListFree` batching
(`osapi.c:2984`). ⇒ **Budget for LATENCY, not for LOSS.** Anything time-triggered on process
exit is wrong; anything edge-triggered on the observed traffic is right.

---

## 2. THE OWNERSHIP TABLE — every join this device performs

| | |
|---|---|
| **owner** | the `(ProcId, GpuId, Pdb)` whose `AddressTable` carries the `JoinsGuestWindow` binding. ⊘ **Never the channel** — a channel may die while its VAS lives, and `mode2_address_table.md` §13 is explicit that many channels share one PDB |
| **unit** | one framebuffer **leaf** (`FB_LEAF_GRANULE`), keyed by `leaf.phys`. Two operands in one leaf are **one** join; the second replays |
| **the three things a join creates** | ① an `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` in the isolate (`join_fb_leaf`), mapped **into two host VASes** since `w282b` — the guest-facing range *and* the executor VAS; ② a `MappedRegion` held in the isolate's `FbJoinTable` and a second in the shell's `SparseFb`; ③ a `Binding` in the owner's `AddressTable` |
| **lifetime** | from `adopt_joined_fb_leaf` returning `Ok` until the owner's binding is dropped |
| **what ends it** | §3 |

★ **The unit is a leaf and not a page, and that is what makes a release a release of one
thing.** Leg 7 de-duplicates by `leaf.phys` *before* joining, so the release side never has to
reason about partial ownership of a leaf.

---

## 3. THE THREE TRIGGERS, in priority order — ⊘ NONE IS WIRED

### T1 — the binding goes away (the precise trigger)

The guest's own unmap reaches us as **PTE clears decoded by `kayfabe_fwd::ptdecode`**, which
already runs at every doorbell and already settles per-`(gpu, pdb)`. When the settlement removes
the leaf's binding, the join has lost its reason to exist.

⇒ Hook: `AddressTable::unbind` **already returns the dropped `Binding`**, and its doc already
says why — *"the returned `Binding::host` is what makes 'retire its host backing' an executable
sentence rather than an aspiration — it names both the mapping to undo and the object to
free."* The primitive is present; the caller is not.

### T2 — the address space dies (the backstop, and it is the one that covers `SIGKILL`)

When a `Vas` is retired — client free, `Proc` teardown, condemnation — **every** joined leaf in
its table is released, whether or not any T1 event was ever observed. This is what makes §1.4's
"can be arbitrarily late" survivable: the latest possible T1 is bounded above by T2.

⊘ **T2 must sweep, not trust a list.** A count of joins is a second source of truth beside the
table; the table *is* the record, and iterating it is the only reading that cannot drift.

### T3 — the device resets (already built, and it is the only one that exists today)

`SparseFb::device_reset` clears `joined` outright, with its own reason:

> *"★★★★★ THE JOINS GO TOO … A joined range that survived a device life would be the PREVIOUS
> guest's framebuffer content."*

⊘ It is a whole-device hammer and covers exactly one case. It is **not** a substitute for T1/T2.

### ⊘ REFCOUNT: not needed, and adding one would be a second source of truth

Overlap is **refused, not shared**: `SparseFb::install_join` answers `ALREADY_JOINED` if any
byte overlaps an existing join. So a leaf has exactly one join and one owner, and "how many
users does this join have" is a question with no consumers. ★ The idempotent-replay arm in
`plan_back_fb_leaf` is what makes repeat presentation free, and it is what a refcount would
otherwise be counting.

---

## 4. ★★★ THE ORDER A RELEASE MUST RUN IN — the join's own order, reversed

The join's order is load-bearing (`w260`); the release inherits the reverse of it, and the
reason is the same in both directions.

1. **Unbind** in the owner's `AddressTable` first. After this nothing can resolve the VA, so
   nothing new can be pointed at the object. ⊘ The reverse order leaves a window in which the
   table advertises memory that is being torn down.
2. **Un-install the view** — `SparseFb` must re-materialise its own local pages for the range
   *before* the mapping goes, because `install_join` **deleted** them
   (and note the measured consequence: a joined page reads as *never written*, which is what
   produced `FwdFault::RingFbNeverWritten`). ⚠ **This is the one genuinely new piece of code**:
   there is no `remove_join`, and un-installing is not symmetric with installing.
3. **Unmap BOTH host VASes and free the descriptor** — `release_unadopted_fb_leaf` already
   stages exactly this as `kayfabe_isolate::Orphans`. ⚠ Since `w282b` the join maps into two
   spaces, so the release must undo two mappings; a release that undoes one leaves the executor
   VAS pointing at freed pages, which is strictly worse than the leak it was fixing.
4. **Drop the isolate's `MappedRegion`** — `FbJoinTable` has `install` and **no removal method
   at all**, so this too is new code.

⊘ **RM pins the pages behind an `OS_DESCRIPTOR` until the object is freed**, so steps 3 and 4
may not be reordered: tearing the mapping out from under a live descriptor leaves the GPU MMU
pointed at pages this process no longer describes.

---

## 5. ⊘⊘ WHAT LEAKS TODAY — stated plainly, not softened

With `w282` landed and no release path wired:

- **Every leaf leg 7 joins stays joined for the life of the `Vas`.** If the guest frees the
  buffer and its allocator hands the same framebuffer offset to a *different* buffer in the
  *same* VAS, the new buffer inherits the old join. ⚠ **Within one address space that is a
  correctness question, not an isolation one** — the memory was and remains this guest
  process's own.
- **Across processes it is bounded by the `Vas` key, and that bound is structural.** A join
  lives in one `(ProcId, GpuId, Pdb)`'s table and is mapped in that isolate's host VASes. A
  second guest process is a different `ProcId` ⇒ a different isolate ⇒ a different host VAS,
  and since `w282b` `AddressTable::owns` refuses a foreign `Pdb` **by name** at both entrances.
  ⇒ **The owner's ruling 4 (*"cross process in one VM is still an important isolation"*) is not
  weakened by the missing release.** The leak is a *resource* leak, not a *boundary* leak.
- **Host memory grows with the number of distinct operand leaves a VAS ever touches**, not with
  the number live at any instant. For `cup2`'s shape that is small; for a long-lived process
  churning buffers it is unbounded.

★ **This is why the shape matters more than the wiring.** Every one of §3's triggers is a
*caller* for primitives that already exist (`AddressTable::unbind` returning the binding,
`release_unadopted_fb_leaf` staging orphans, the per-`(gpu, pdb)` settlement pass), plus exactly
two genuinely new pieces of code — `SparseFb::remove_join` and `FbJoinTable::remove`. Nothing
in §2's ownership model has to change for them to land, which is what *"not a retrofit"* means.

---

## 6. THE FALSIFIERS a release rung must pre-register

- **T1 fires at all.** A boot in which the guest frees a joined buffer must show the unbind, by
  `fb_phys`. ⊘ A count of releases is not it — `a_count_cannot_see_a_substitution`.
- **T2 covers what T1 missed.** Kill the guest process with `SIGKILL` (§1.1: it issues nothing)
  and assert the leaf is released anyway, naming it.
- **Step 2 actually re-materialises.** After a release, a guest read of the range must return
  the guest's own bytes and not zeros — the `RingFbNeverWritten` shape, which is what a missing
  re-materialisation looks like from the other side.
- ★ **A negative control on the release itself**: a joined leaf that is *not* freed by the guest
  must still be joined at the end of the boot. A release path that releases everything passes
  every test above and is a bug.
