# V3 — the stock tier and the opt-in cooperation stages

**STATUS: DESIGN-ONLY, 2026-10-03; §5.1 corrected the same day (pool backing, owner).** **Owner, 2026-10-03: post-release work**, except stage 1 (the guest doorbell helper), which is a release candidate once a per-token breakdown and a non-nested baseline exist (`OWNER_RULINGS.md` §I). The owner sketched this plan on 2026-10-02 and 2026-10-03,
and the review's corrections are folded in. Nothing here is decided or built, apart from what
`docs/OWNER_RULINGS.md` §H already rules (the stub rule, and the vGPU guest stack crossed off).

> **2026-10-10:** the host-side stages (3 and 4) and the other host-kernel candidates are consolidated, with probes, stock fallbacks and an implementation order, in `V3_HOST_PATCH_LIST.md`. Owner policy: the stock tier needs no patch; the patched host is an optional better tier.

The idea is one **stock tier** that needs no kernel change anywhere, plus **opt-in stages** that
each need one named piece of cooperation. The owner's stated goal is that, with these stages,
kayfabe replaces nvkvm-pv (§7).

## 1. The stages at a glance

| stage | needs | adds |
|---|---|---|
| stock | nothing: an unprivileged VMM process, stock host and guest drivers | everything that runs today: the app matrix, multi-GPU, the driver range |
| 1. guest doorbell helper | a guest kernel module; works with the distro's own NVIDIA driver, open or closed | doorbell writes without a VM exit (`V3_GUEST_DOORBELL_MODULE.md`) |
| 2. patched guest driver | a patched open-gpu-kernel-modules build in the guest | doorbell passthrough by token (§3.1), dynamic memory (§4), fault-free managed memory (§5.3), the guest half of host-owned UVM (§5.1) |
| 3. patched host driver | a patched host nvidia-uvm | host-owned UVM residency (§5.1), for a cooperating guest only |
| 4. host helper module | a small host kernel module | a software SR-IOV (mdev) device for unmodified hypervisors (`V3_VFIO_USER_FRONTEND.md` §2, option 0), and an in-kernel doorbell for stock guests (§3.2) |
| ~~5. Windows memory balloon~~ | — | dropped (§6) |

## 2. The stock tier

The owner accepts three limits here.

- **No GPU demand faults on managed memory.** Pinned host memory and `cudaMalloc` are unaffected.
  - Make it fail loudly. On a stock host a GPU fault on a host twin cannot be serviced. Check
    that the app gets a channel error, and that nothing is corrupted silently.
  - Managed memory that the app prefetches still works. Prefetch and advise calls never raise a
    GPU fault (`V3_UVM_STATE_MACHINE.md` §8.2).
- **Doorbell writes trap**, with ioeventfd as an option (`V3_DOORBELL_IOEVENTFD.md`).
- **All guest VRAM is reserved at VM start and cannot be resized** (the single store).

## 3. Doorbells

### 3.1 Passthrough by token (stage 2)

- **The patched guest driver gets each passthrough channel's work-submit token from kayfabe**,
  which hands it the host twin's token.
  - Stock RM computes the token itself from the runlist and channel ID
    (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/ampere/kernel_fifo_ga100.c:178`), so the
    patch is one function.
  - Channel IDs can stay guest-chosen, because userspace uses the token only for the doorbell
    write.
- **Kernel channels ring a separate page that stays trapped.** These are the Translated and
  Emulated channels: CeUtils, guest UVM and display.
  - The shared doorbell page then carries only passthrough channels. That is the condition under
    which it may carry no write trap at all (`THE_CONSTRAINTS.md` §47; `THE_ARCHITECTURE_v3.md:463`).
- **Security is as in `OWNER_RULINGS.md` §D.** Any token may be rung, but a ring only makes the
  GPU re-read that channel's own `GP_PUT`, and no kayfabe code runs on a passthrough ring.
- **Cost:** the patch must be rebased for every supported guest driver release, so keep it to a
  few hooks. Stage 1 keeps its value because it needs no patched driver.

### 3.2 An in-kernel doorbell for stock guests (stage 4)

Owner idea: move the doorbell helper to the host.

- **The binding.** Keep today's ioeventfd registrations: one per passthrough guest token and
  site, matched by value (`crates/kf-chan/src/dbfast.rs`). The host helper binds each
  registration's eventfd to an in-kernel write of the host twin's token to the host doorbell
  page. The write happens on the eventfd wakeup, so no drainer thread sits between the guest's
  write and the GPU.
- **What a module can use on the dev host's kernel 7.0** (read from `/proc/kallsyms`, 2026-10-03):
  - `kvm_io_bus_register_dev` is **not** exported, so a module cannot add its own MMIO handler.
  - Binding a wait-queue entry to an eventfd, as vfio's virqfd does, needs no KVM export.
  - `kvm_page_track_register_notifier` is exported, but it tracks writes to RAM pages only.
- **Expected gain** is on bare-metal hosts, where the exit is cheap and the drainer wakeup is a
  large share of the latency. On nested hosts the exit itself dominates, and only a guest-side
  stage (1 or 2) removes it. Unverified.
- **Open for the mdev path.** Under an unmodified QEMU, kayfabe's daemon holds no KVM VM fd to
  register ioeventfds with, and the helper cannot register an MMIO handler (above). A trapped
  doorbell then crosses QEMU, unless the guest runs stage 1 or 2.

## 4. Dynamic memory for Linux guests (stage 2)

### 4.1 The model

- Each VM has two numbers:
  - **reserved**, allocated at start, which holds every RM structure (page tables and other
    internal state);
  - **capacity**, the most the guest may hold.
- Dynamic memory is a sparse range of the vidmem aperture above the reserved size. Guest
  page-table entries keep their normal format, and kayfabe tells the two ranges apart by offset.
- Reserved stays the single store. Each dynamic chunk is its own host RM object of 2 MiB, so the
  single store needs no holes.

### 4.2 An allocation must be able to fail, before any page-table write

- **The allocation is the only place to refuse memory** (owner). A page-table write cannot
  express "no memory", and a refused TLB invalidate is a GPU-wide failure.
- **One synchronous request.** The patched guest driver asks kayfabe for memory. The reply is
  either a dynamic address range already backed by a new host RM object, or "no memory".
  - It travels as an RPC on the existing GSP queue, so the host allocation runs on kayfabe's RPC
    worker, never inside a vCPU trap.
- **Order** (review's proposal): reserved first, down to a floor kept for RM's own structures,
  then dynamic.
  - The guest reports an error (`NV_ERR_NO_MEMORY`, so `cudaErrorMemoryAllocation`) only if both
    fail.
  - Reserved is already paid for, so using it first leaves more of the shared pool to other VMs.
- **Release.** The host frees a chunk only when no published page-table leaf references it, and
  it counts leaves per chunk to know that. A release while the chunk is still mapped is refused.
  - Never trust the guest's free ordering. A chunk freed and handed to another VM must not stay
    reachable from this one.
- **Safety net.** The walker refuses any leaf that points into an unbacked dynamic address. The
  GPU then faults inside the guest's own context, which stays contained.

### 4.3 Free-memory reporting

- Container model: free = reserved free + min(capacity left, what the host could grant now).
- It is racy the same way containers are. That is acceptable because an allocation can fail
  cleanly (§4.2).
- The patched driver changes the framebuffer-info answers that `nvidia-smi` and `cudaMemGetInfo`
  read.

### 4.4 A stock driver on the same VM

- A stock driver sees only the reserved size and works within it.
- The owner's separate, larger reservation for stock guests needs the driver kind before the
  framebuffer size is first stated. Two facts make that hard:
  - the size is stated in several places the guest reads before any RPC;
  - the store is reserved at QEMU startup today (`crates/kf-qemu/src/device.rs:39`, `:281`).
- So it is either a per-VM setting, or the store reservation moves to after the driver's boot
  handshake.

## 5. UVM: host-owned residency (stages 2 and 3)

### 5.1 The design

- **The host does every migration, in both directions.** The guest never holds a GPU fault, and
  a fault freezes GR only for the host's own service time (the owner's core invariant).
- **The cooperating guest takes managed memory only from a dedicated, pinned pool.** It registers
  each managed range with the host: guest GPU VA to pool pages.
  - The patched host nvidia-uvm manages that range with CPU VA ≠ GPU VA. That rule is a UVM
    software restriction, not a hardware one (owner).
- **Migration destinations come from the VM's own VRAM** (reserved or dynamic). Managed memory
  never takes host VRAM outside the VM's accounting.
- **To the guest, the pool is ordinary RAM. Its host backing must not be memfd.**
  - ⊘ **Corrected 2026-10-03 (owner).** The first version said the pool "cannot be ordinary guest
    RAM". That holds only for memfd backing. Stage 3 already needs a host kernel module, and a
    module can own the backing pages itself.
  - **Why not memfd.** After a page's data moves to VRAM, a guest CPU touch must trap. shmem's
    fault path offers no hook short of userfaultfd, which is ruled out (owner, 2026-09-14). Stock
    KVM on kernel 7.0 has per-page access exits only for guest_memfd memory.
  - **How.** The host module provides a file whose pages it owns, and stock QEMU maps it as a
    second guest-RAM block (`memory-backend-file`, attached as its own NUMA node or a DIMM).
    - The guest sees ordinary RAM and keeps its other allocations off that block.
    - kayfabe's guest-RAM import leaves the block to host UVM.
  - **The module's fault handler is the trap.**
    - Migration to VRAM zaps every mapping of the page, and KVM's MMU notifier drops the guest's
      mapping with it.
    - A guest CPU touch then faults into the module, which migrates the page back before mapping it.
    - The handler must never fail: a fault KVM cannot resolve fails `KVM_RUN` and kills the guest.
  - That means no guest page-table reads or edits, no handover call and no race boundary.
- **What "we control the host kernel" covers.** A module can load through DKMS and use exported
  symbols; it cannot change KVM or mm. On kernel 7.0 that rules out a module's own MMIO handlers
  (§3.2), but not this design.
- **The walker skips registered managed ranges.** Host UVM owns those page-table entries.
- **Guest residency queries** read a small host-written table, or are stubbed (owner).

### 5.2 Why this answers option (a)'s refutations

`V3_UVM_DEMAND_PAGING.md` §3.1 refuted "the real driver migrates underneath" on four points.
Each is removed by one cooperation piece.

| refutation | removed by |
|---|---|
| (i) a managed range's GPU VA must equal its CPU VA | the stage-3 host patch |
| (ii) memfd guest RAM cannot migrate | the dedicated pool |
| (iii) any guest page can become managed backing | the dedicated pool |
| (iv) the host cannot know which page backs an address | the guest registers ranges up front |

### 5.3 Fault-free managed memory (stage 2 only, stock host)

- **The idea:** pre-populate each managed range in guest system RAM, and keep it mapped on the
  GPU with "accessed by" the GPU and preferred location CPU. UVM maps that immediately
  (`V3_UVM_STATE_MACHINE.md` §7, H4).
- **It runs on a stock host,** because the GPU never faults. Data stays in system RAM unless the
  app prefetches.
- **The trade-off:** correct everywhere, but slow for apps that depend on demand migration.
- **First steps:**
  - It can be prototyped with an LD_PRELOAD shim in a stock guest before anything is patched.
  - Check GPU atomics on remote system memory first (`V3_UVM_STATE_MACHINE.md` §8, item 4).

### 5.4 Open questions, in order

1. **Does KVM fault cleanly through the module's file?** That means a get_user_pages call, an MMU
   notifier zap, and a refault into the module. `V3_UVM_DEMAND_PAGING.md` §3.1(ii) found the
   closest case plausible but untested. This is the first experiment.
2. **Many guest processes reuse the same GPU VAs,** while UVM keeps one address layout per VA
   space.
   - That probably means one host UVM VA space per guest GPU VA space; how they share the one
     pool is open.
   - As far as the review can tell, decoupling CPU and GPU addresses is a real UVM change, not
     one removed check.
3. **How the guest keeps other allocations off the pool node**, so that only managed pages come
   from it. One option is onlining it for the driver alone.
4. **How long GR is held** while the host services a fault. The host bounds it, but the length on
   hardware is not yet known.
5. **GPU atomics** and the `ATOMIC_DISABLE` bit on pool pages.

## 6. Windows: no dynamic memory

The owner's balloon idea is dropped, for three reasons.

- **A balloon cannot pin addresses.** Windows' video memory manager can move or evict any
  allocation, so a balloon allocation does not hold fixed VRAM addresses. No API gives a
  third-party driver an allocation's physical address either.
- **A Windows guest copes with a smaller reservation by itself.** NVIDIA's Windows driver spills
  CUDA allocations into system RAM by default (the "CUDA – Sysmem Fallback Policy" setting), and
  D3D does the same through residency management. A smaller reservation makes it slower, not
  broken.
- **The only option without a guest driver doesn't pay off.** That option is backing reserved
  VRAM on first reference:
  - it cannot give memory back;
  - it saves nothing if the driver scrubs the whole framebuffer at boot;
  - it would split the single store into chunks.

## 7. Replacing nvkvm-pv

- **The goal.** With doorbell passthrough, dynamic memory and UVM in place, the owner's goal is
  that kayfabe replaces nvkvm-pv.
- **The structural advantage.** kayfabe's host RM surface is a fixed list it writes itself:
  50 host controls in `HOST_CONTROLS` (`crates/kf-abi/src/hostabi.rs`). Nothing is forwarded from
  the guest.
- **Caveats before calling it a full replacement:**
  - Better security is the architecture's promise until the audit, which is roadmap item 6.
  - kayfabe needs GSP, so Turing or newer. If nvkvm-pv serves pre-Turing GPUs, retiring it drops
    them until a non-GSP mode exists ("non-GSP later", 2026-09-25).

## 8. Suggested order

1. Doorbell passthrough by token (§3.1). Per `OWNER_RULINGS.md` §D, test a non-nested host first.
2. Dynamic memory for Linux (§4).
3. Fault-free managed memory (§5.3).
4. The host helper module: the software SR-IOV device and the in-kernel doorbell (§3.2).
5. Host-owned UVM (§5.1).

Windows dynamic memory is dropped (§6).
