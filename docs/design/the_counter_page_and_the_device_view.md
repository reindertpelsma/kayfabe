# The counter page, and the armed device node that serves it

**STATUS: LIVE but its SECURITY ARGUMENT IS REFUTED, 2026-09-13 (w596) — see §4a before
building anything.** The mechanism is measured and works; `ACCESS_READ_ONLY` does NOT make the
mmap read-only, so the containment argument that chose this design over the alternative is gone. Supersedes nothing; extends `the_bar0_read_surface.md` §3 piece 3.

## 1. Why this page is the last one

`[measured w591, three arms, one binary, all graded (P)]` BAR0 reads reaching our handler fell
**184 585 → 134**, and the census names where every one of them is:

    BAR0-READ-HOTSPOTS pages_touched=1 reads_from_live_pages=132
                       reads_from_BACKED_pages=0  top[+0xbb0000=132]

⇒ **One page. The free-running counter.** It cannot be shadowed — it changes continuously, so no
copy keeps up — and `the_bar0_read_surface.md` §3 already named the only answer: map the HOST's
own usermode page over it, read-only, so the counter resolves natively and the doorbell store at
`+0x90` still exits.

⊘ **And the count is not the argument.** 132 reads is milliseconds. Two other things are:

- **Reads are the only SYNCHRONOUS class.** A write is posted; a read cannot retire until we
  answer. *"Zero read traps"* is literally *"no vCPU ever waits on kayfabe code."* One remaining
  read keeps a read handler alive, and this tree has measured what a handler that exists does to
  a boot (w586: a census on the hot path cost four boots).
- **Today's answer is WRONG, not merely slow.** The guest's PTIMER reads are served from a host
  **CPU** clock, while GPU-written semaphore timestamps carry the real PTIMER — the 43 ppm
  divergence `native_dataplane_cup2_ga106.md` §4 already measured. Anything correlating the two
  (CUPTI, nsys, `cuEventElapsedTime` against host time) sees drift. **The mapping is the only
  implementation in which the guest reads the GPU's own clock.**

## 2. The two designs that do NOT work, and why — ruled 2026-09-13

**(A) Pass the isolate's RM client fd to the VMM and let it mmap.** ⊘ **Mechanically impossible.**
`rm_create_mmap_context` refuses with `NV_ERR_INVALID_CLIENT` unless
`pRmClient->ProcID == osGetCurrentProcess()` (`ogkm-580: osapi.c:2378`), so the VMM can never arm
anything on a client it did not create. And there is no "RM-assigned offset" to mmap at:
`NV_ESC_RM_MAP_MEMORY` names a **device node** (`escape.c:534`), the context is stored on that
node's `struct file`, and the mmap must be on that node at `vm_pgoff == 0` and exactly the
registered length (`nv-mmap.c:527-536`, `:562`).

**(B) The VMM opens its own `/dev/nvidiactl` and allocates its own usermode object.** Works, and
is the wrong trade. Under the owner's unprivileged rule the device nodes are world-openable
(measured: `crw-rw-rw-`), so a compromised VMM can open them in *every* design — B does not grant
an attacker a client, it pre-opens one. What B costs is the only boundary this product can ever
have: **a VMM sandbox (seccomp/Landlock) that denies `/dev/nvidia*` outright.** B also puts
`NV_ESC_RM_ALLOC` sequences and the RM API lock in the vCPU-owning process and doubles the
RM-ABI surface into a second process.

## 3. The design: an armed device node, read-only, over the wire

A **fresh `/dev/nvidiaN` node carrying a one-shot armed mapping context and no client behind it**
is what crosses — not a client fd. That type already exists in this tree as
`kayfabe_isolate::DeviceView`, produced by `arm_cpu_view` / `export_device_view`
(`kayfabe-isolate-host/src/rm.rs`) and consumed by `WindowBacking::DeviceView`
(`kayfabe-vmm-qemu/src/lib.rs`). Only the **wire verb** was deleted, on 2026-09-12, for having
zero senders (`ORPHANS_wire_or_discard.md`). ⇒ The cost is one verb, not a mechanism.

⚠ **Request tag 25 and reply tag 12 are RETIRED, not free** — a peer built from an older revision
may still send them. Mint new numbers.

Arm it `NVOS33_FLAGS_ACCESS_READ_ONLY`, so the **kernel driver** refuses `PROT_WRITE` on the
descriptor. That is not a nicety: `0xbb0090` is the PF mirror of the VF doorbell
(`kern_gpu_tu102.c:275-283` — bare metal drives `NV_VIRTUAL_FUNCTION` even on host), and its
token is `runlist << 16 | chid` for **any** host channel. A writable mapping of that page is a
cross-tenant ring. Read-only makes it structurally not one, even for a hostile VMM.

## 4. ★★★★★ MEASURED — the whole mechanism, unprivileged, on this hardware

`rmladder --bar1-crossing`, GA106, **euid 1002, non-root, `kvm` group only**:

    W393 LEG A = ONE MEMORY: all 64 words the child stored through node A read back through
                 node B in THIS process.  => an armed device node CROSSES a process boundary.
    W393 device slot = KVM_SET_USER_MEMORY_REGION accepted a VM_PFNMAP device view as memslot
    W393 guest exit  = the ONLY exit is the signal store; the data store and the data load
                       took NO exit
    W393 LEG B = a guest CPU store took no VM exit and landed in card memory another CPU
                 view reads.

⇒ **All three premises are measurements, not readings**: the node crosses, KVM accepts a
`VM_PFNMAP` device view as a memslot, and guest access through it takes no exit. ⊘ Run as root
this probe reports `euid 0`; the run that matters is the one above, because the owner's rule is
*"anything you want to obtain from host userspace must be able unprivileged (non root)"*.

## 4a. ⊘⊘⊘ **REFUTED 2026-09-13 (w596) — `ACCESS_READ_ONLY` DOES NOT MAKE THE mmap READ-ONLY.**

`[measured, GA106, unprivileged euid 1002]`, `rmladder --bar1-crossing` LEG R:

    FAIL W393 LEG R = PROT_WRITE was ACCEPTED on a node armed ACCESS_READ_ONLY.

★ §3's security primitive is **absent**, and the chain that predicted it is real but leads
somewhere else. `NVOS33_FLAGS_ACCESS_READ_ONLY` does lower to `NV_PROTECT_READABLE`
(`mapping_cpu.c:970-982`) — and that value governs **RM's own mapping bookkeeping**. The
`mmap` protection comes from a different source entirely: `nv-mmap.c:155` tests
`mmap_context->prot`, and that field is set from **`RmValidateMmapRequest`**
(`osapi.c:2494-2503`), which validates the BAR range and never reads the NVOS33 access flags.
⇒ Two plausible chains, one real, and they meet nowhere.

⚠ **This is why the probe ran before the wire verb.** Every step of the refuted argument was a
correct citation of real code; the conclusion was still false. *"Citing the oracle is not the
oracle being right"* — and a chain read across two files is exactly where that fails.

⇒ **The design as written is UNSAFE and must not be built.** A descriptor armed over the
usermode register block grants its holder WRITE access to page 0, which contains the doorbell at
`+0x90` alongside the counter at `+0x80`/`+0x84`. No sub-range excludes one and keeps the other:
they share a page. The exposure is not the VMM ringing its own guest's doorbells — it already
does that through the isolate — it is the token space: `runlist << 16 | chid` names **any host
channel on the GPU**, including other tenants'.

★ What survives: §4's three measurements stand — the node crosses, KVM accepts a `VM_PFNMAP`
device view as a memslot, and guest access through it takes no exit. The MECHANISM works. Only
the containment argument for handing that descriptor to the VMM is gone, and it is the part that
decided A′ over B.

⊘ OPEN, and not to be guessed at: whether a different arming makes the kernel refuse the write
map, or whether containment has to come from elsewhere (a VMM sandbox that cannot `open()` the
node is unaffected by what the descriptor permits, since the descriptor is already open).

## 5. Constraints the implementation will hit

- **mmap 64 KiB, slot 4 KiB.** The driver refuses any length but the registered one
  (`DRF_SIZE(NVC361)` = 64 KiB). Only page 0 holds registers — `TIME_0 0x30080`,
  `TIME_1 0x30084`, `DOORBELL 0x30090`. Slot page 0; leave the other 15 `Observe`.
- **Memory type is decided for you on Intel**: BAR pfns are `!pfn_valid`, so EPT is UC with
  IPAT. On AMD, NPT honours the guest's PAT and the guest maps BAR0 UC. `assert_effective_cacheability`
  cannot see the EPT side; say so rather than assert it.
- **No pinning.** `kfp->pin && !allow_unsafe_mappings` is `-EINVAL`. Nothing may take a
  `gfn_to_pfn_cache` over this gfn.
- **Revocation is real and needs an arm.** GPU reset / runtime suspend calls
  `unmap_mapping_range`, which zaps EPT. The next read re-faults into `nvidia_fault` and is
  re-inserted — or, if the GPU is gone, `KVM_RUN` returns a memory-fault exit. ⇒ On isolate or
  GPU death, re-tier the page to `Observe` **before** the guest can read it, and treat a memory
  fault on this gfn as *GPU lost*, not a crash. The BAR1/BAR2 mirror owes the same arm.
- **The fault handler's wakeup can block a vCPU** after a runtime suspend. It cannot occur while
  the isolate holds a client, but it is the one blocking path in this design and is named here
  so it is not discovered later.
- **Live migration is impossible**, and already was for this product. Stated once.
- **Install at adapter realize, never lazily** — the first PTIMER read must find the slot, and
  the slot ioctl asserts lock-free.
- **Derivation, not a constant.** Every architecture Turing→Blackwell uses
  `gpuGetRegBaseOffset_TU102` = `FULL_PHYS_OFFSET (0xb80000) + DRF_BASE(NV_VIRTUAL_FUNCTION)
  (0x30000)`; `tu102`, `ga100`, `gh100` and `gb100` `dev_vm.h` all agree. Carry the FORMULA in
  the chip profile. ⚠ Hopper+ `HOPPER_USERMODE_A` with `bBar1Mapping` hands the region out
  through **BAR1** (`usermode_api.c:66-98`): if the guest driver takes that variant, the GPA
  comes from the emulated GSP's placement, not from a fixed BAR0 offset. On the host always
  allocate the BAR0 variant.
- **The known-positive this census needs:** the isolate's `host_ptimer_via_usermode` and a read
  through the VMM's own mapping must agree over a bracketed sample. Without it, "the counter page
  is served natively" is a zero nobody drove.

## 6. ★ The thing worth more than the page itself

The read-only slot's write exit goes through `kvm_io_bus_write` **before** it reaches userspace.
⇒ An **ioeventfd with `DATAMATCH` on each channel's doorbell token** — known at channel creation
— turns the doorbell into an in-kernel eventfd signal with **no userspace exit at all**.
Unregistered tokens still fall through to the userspace exit, so the refusal path survives.

That satisfies *"no blocking work in any MMIO trap"* for the doorbell **by construction**, rather
than by keeping the handler fast. It is a bigger prize than the 132 reads, and it is reachable
only from this design.
