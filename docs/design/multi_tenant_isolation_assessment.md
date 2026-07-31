# Is the security model sufficient for AGGRESSIVE multi-tenant?

> **Status: assessment, written 2026-07-31 against this tree at `63915da` and
> `ogkm-580.159.04`. No GPU was switched on for this document.** Companion to
> `guest_blast_radius.md` (task #129) and `core_security_threat_model.md` (decision #18C);
> it asks a question neither of them asks.

## 0. The question, the scope, and the epistemic frame

**The question, as put:** *is the security model sufficient for aggressive multi-tenant —
mutually distrusting tenants sharing one host GPU?*

**Scope, as set by the owner:** **confidentiality and integrity between tenants, and
escalation to the host.** ⊘ **Denial of service is explicitly out of scope for this
document.** A wedged engine is an availability problem; it is covered by
`guest_blast_radius.md` §5 and `compute_limiting_and_priority.md`, and nothing here should
be read as an opinion on it. Where a finding touches DoS it is marked and dropped.

⊘ **I ran no hardware.** Every statement below is a reading. Labels follow the scheme
`guest_blast_radius.md` §0 and `compute_limiting_and_priority.md` §0 established, used
strictly:

| label | meaning |
|---|---|
| **[src@580]** | read out of `/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04/`, cited file:line. Nothing ran. |
| **[src-rust]** | read out of this tree at `63915da`, cited file:line. Nothing ran. |
| **[src-C]** | read out of the C research artifact. Nothing ran. |
| **[run: …]** | somebody else's experiment, named — commit, date, box |
| **[inferred]** | a conclusion drawn from the above |
| **[unknown]** | nobody here knows, and this file says so instead of guessing |

★★★ **The honesty constraint that governs the whole document.** The per-tenant isolation
model has **never run on hardware**. `docs/design/open_questions_for_the_owner.md:88-91`
records it in the tree's own words: *"`#16` records that **P0–P6 was never complete — only
P0+P1 landed**, and `#95` records that **the bench never compiled past `862c7c2`**, so ★★★
**#14's P0/P1 has never run on hardware at all**"* **[src-rust]**, and that the C cannot
oracle it either — it runs exactly one CUDA process per QEMU lifetime. ⊘ Nothing below
that is *designed* is described as *validated*, and the distinction is repeated at every
item rather than stated once.

---

## 1. ★★★ The direct answer

> ## **No.**
>
> Not because a specific control is missing, but because **there is no tenant axis in this
> system at all.** Every isolation property this tree proves — I1–I4, `#14`, §12.44's
> namespace scoping — is **intra-VM, between guest processes**. Cross-tenant separation is
> not implemented, not tested, and not named: it is the incidental consequence of two VMs
> being two host processes. And at the one place two tenants genuinely do meet — the host
> NVIDIA driver — the driver's own cross-client boundary is **defeated by construction**,
> because every tenant's isolate presents the same euid.

Three sentences, each of which the rest of the document supports:

1. **The core cannot alias two tenants because it never sees two tenants.** `Gpu` is one
   VM's state; `Gpu::procs` is keyed by guest *process*
   (`crates/kayfabe-core/src/gpu.rs:1127-1134`) **[src-rust]**. There is no `VmId`,
   `TenantId` or `GuestId` anywhere in `crates/` **[src-rust]**. This makes T1's answer
   *good* — see §3 — and it also means the multi-tenant claim in `README.md:8-11` and
   `docs/design/l1_architecture_summary.md:128` rests on something outside the code.
2. **The host RM layer is where tenants meet, and there our separation is one line.**
   Every isolate hard-codes its own client handle in every ioctl (§5.1), and that is the
   *only* thing standing between tenant A and tenant B's RM objects, because RM's own
   `euid`-or-`pid` client check passes between same-uid isolates (§5.2). ★ That line is
   not a stated invariant and no test pins it **[src-rust]**.
3. **The sandbox's own threat model assumes the isolate can be compromised, and the
   post-compromise blast radius is currently every tenant on the box.** That is the
   sharpest form of the answer and it is developed in §5.3.

★ **And one thing that came back CLEAN, which is worth as much as the alarms.** The
expected break — residual tenant data on GPU memory reuse — **does not exist on the paths we
drive**: RM scrubs VRAM on the *free* path with a CE memset and makes the frame
un-allocatable until the scrub retires (§4.1) **[src@580]**. Three conditions could turn
that off and one of them is unprivileged (§4.2); our own recycling paths are clean today for
reasons nobody wrote down (§4.4); and the design already contains the change that would
break it (§4.5). But "no" above is **not** driven by T2.

**What "yes" would look like** is §8: a ranked list of what must be true, and §9 the
smallest change set that would get there. Two items in it are cheap; one is a hardware
campaign that has never been attempted.

---

## 2. What a "tenant" IS in this system, as built — the fact that decides everything else

Establish this first, because every later finding is a statement about one of these
boundaries and they are routinely conflated.

```
tenant A                                        tenant B
┌──────────────────────────────┐               ┌──────────────────────────────┐
│ guest VM A (stock nv driver) │               │ guest VM B                   │
│   proc a1     proc a2        │               │   proc b1                    │
└──────────────┬───────────────┘               └──────────────┬───────────────┘
   VMM process A │  ← one `Gpu`, one `RmGraph`     VMM process B │  ← its own
   ┌─────────────┴──────────┐                     ┌─────────────┴──────────┐
   │ isolate(a1)  isolate(a2)│                     │ isolate(b1)            │
   └─────────────┬──────────┘                     └─────────────┬──────────┘
                 └────────────────┬─────────────────────────────┘
                        ONE host NVIDIA driver, ONE host GPU
```

**As built** — `[src-rust]` throughout:

- **One VMM process = one VM.** `KvmVmm::realize` opens `/dev/kvm` and does one
  `create_vm()` (`crates/kayfabe-vmm-kvm/src/lib.rs:700-703`); nothing constructs two.
- **One core instance per VMM.** `pub struct Gpu { spine, system: Proc, procs:
  BTreeMap<ProcId, Proc> }` (`crates/kayfabe-core/src/gpu.rs:1127-1134`). `system` is *the*
  guest kernel, singular; `procs` is *the* guest's userspace processes.
- **One isolate child process per (guest process × GPU target).** `IsolateId::new(proc,
  gpu)` is the only constructor (`crates/kayfabe-isolate/src/lib.rs:515-536`), and
  `HostIsolateFactory::spawn` forks one sandboxed child per id
  (`crates/kayfabe-isolate-host/src/isolate.rs:914` → `:800` → `:856`).
- **One RM client per isolate process.** `RmConnection::open` is called once per child
  (`crates/kayfabe-isolate-host/src/child.rs:212`); the resulting `Arc<RmConnection>` is
  cloned only across that child's own worker threads (`:212-215`). There is **no** shared
  RM client, device fd or host VA table between isolates.
- **The multi-axis that IS modelled is multi-GPU, not multi-VM.** `GpuTarget`
  (`gpu.rs:1139`), `GpuId` in every routing key, `Proc::isolates: BTreeMap<GpuId,
  IsolateBox>` (`gpu.rs:354`) — several host GPUs for **one** guest.

⇒ **The tenant boundary is entirely below our code**: two host processes, and the host
NVIDIA driver they both call. Nothing in `crates/` implements it, and — the part that
matters — nothing in `crates/` *depends* on it holding, so no test can notice if it stops
holding. `README.md:8-11`'s parenthetical *"(and several guests, and several GPUs)"* has
structure behind the second half and none behind the first **[src-rust]**.

★ This is not a criticism of the design. Per-VMM-process separation is a *good* boundary —
it is the C's declared model too (`C: docs/SECURITY_MODEL.md:22-27`: *"QEMU = the cross-VM
/ host boundary"*) **[src-C]**. The problem is that it is currently **assumed rather than
stated**, so §5's finding — that the host driver does not reinforce it — has nowhere to be
noticed.

---

## 3. T1 — CLIENT ALIASING: the fix is **BUILT**, and tenants cannot alias in the core

**The finding under test** — somebody else's experiment, named as theirs:
`[run: kprobes on RM's dup funnel, RTX 3060 / 580.159.04 — two concurrent processes issued
82 dups each, every one to the same destination 0xc1d00069; dups with a user client as
destination: 0]`, quoted with its numbers at `crates/kayfabe-core/src/project.rs:20-47`
**[src-rust]**: *two concurrent
CUDA processes share one dup-DST client, aliasing both `Proc`s together, and UVM's gpu-ops
client is GLOBAL; the fix is to key on anchor client / `(client, PDB)` / `(client,
vChid)` rather than on the client alone.*

### 3.1 Built, and further than the finding asked

All `[src-rust]`:

| the fix | where it is |
|---|---|
| **anchor client** | `ProcAnchor(ClientKey)` = *"the smallest client **declaration** in its dup-connected component"* (`crates/kayfabe-core/src/lib.rs:258`); computed by `anchor_of` (`crates/kayfabe-core/src/project.rs:656-662`), stored as `Proc::anchor` (`gpu.rs:300`) |
| **address keyed on `(GpuId, Pdb)`** | projection `project.rs:215`; routing `gpu.rs:879`; per-VAS table `Proc::vases: BTreeMap<(GpuId, Pdb), Vas>` (`gpu.rs:331`); demux `route_pdb` (`crates/kayfabe-fwd/src/lib.rs:1094`) |
| **execution keyed on `(GpuId, VChid)`** | projection `project.rs:218`; routing `gpu.rs:881`; demux `route_doorbell` (`kayfabe-fwd/src/lib.rs:1251`) |
| **the aliasing edge itself, refused** | `project.rs:628` — `if !(is_user(dst_decl) && is_user(src_decl)) { continue; }`. A dup whose destination is a **kernel** client (UVM's global gpu-ops session) is **not** a grouping edge |

Neither `route_pdb` nor `route_doorbell` takes an `HClient` **[src-rust]**. The module that
states the fix most directly is `crates/kayfabe-core/src/promote.rs:18-26`: *"The C
artifact keyed its table on the `hChanClient` … two concurrent CUDA processes **share one
duplicated client**, and UVM's gpu-ops client is global, so a client handle does not
identify a process. Here `hObject` is resolved to a live resource, the resource names a
`(GpuId, Pdb)`, and the `(GpuId, Pdb)` names the `Vas` — the memory boundary itself."*

★ Beyond the named fix, `ClientKey { client: HClient, incarnation: u32 }`
(`crates/kayfabe-core/src/rmgraph.rs:251`) adds a **generation counter**, so a *recycled*
`hClient` value cannot let a ghost's declared facts resolve into the next holder's
namespace — `resolve_declared_handle` (`project.rs:406-419`) refuses a superseded
declaration **[src-rust]**. That is §12.44, and it is a real second axis the original
finding did not ask for.

### 3.2 ⚠ I was told to be suspicious of the tests, and the suspicion is answered — for the shape the C found

The warning was that *the acceptance signal for the earlier work was itself measuring the
aliased mapping*. The key separation file in this tree is written **as a correction of
exactly that**, and says so in its own header
(`tests/tests/rmgraph_order_independence.rs:11-32`): *"What this file used to assert, and
why it was fiction … The scenario gave process A and process B **one UVM client each** …
**That shape cannot occur.**"* **[src-rust]** It uses the measured handle values verbatim
(`:56-66`, with the UVM session `0xc1d00069` numerically **between** proc A's and proc B's
clients, so an ordering bug cannot hide), constructs distinct PDBs explicitly (`:69-74`),
and never derives an assertion from `ProcId`. `i1_no_proc_va_resolves_to_another_procs_
backing` (`tests/tests/security_invariants.rs:183`) checks a K-process world against an
**injective PDB→phys oracle** in both directions (`:224-241`) — keyed on the PDB, which is
the correct key. ⇒ **Not circular** `[inferred]`.

### 3.3 ★ The residual, stated as a residual

A `DUP_OBJECT` whose destination is a **live, declared, user** namespace belonging to
another proc **still merges the two `Proc`s** (`project.rs:648` `uf.union(dst_decl,
src_decl)`) **[src-rust]** — deliberately, because that is what a genuine CUDA-IPC-style
share *is*. Acceptance refuses only the **undeclared**-destination squat
(`rmgraph.rs:1238-1254`, `:1324`), which is the hostile shape
`tests/tests/l1_mean.rs:3607` closes.

The tree argues the live-destination shape out of the attacker model at
`docs/design/l1_concurrency.md:4069-4075`: *"in Mode-2 these events come from the guest's
stock NVIDIA kernel driver, whose RM validates `hClientDst` before emitting the dup RPC,
so a hostile user process in the guest cannot produce them — only a compromised guest
kernel can."* **[src-rust]** That is the A3 tier of `core_security_threat_model.md` §2, and
A3 already owns its whole VM. ⚠ Two things follow that should be said out loud:

- Both fuzz properties **structurally exclude** the merging dup — `i1_junk_event`'s
  generator keeps its dup lanes provably disjoint (`security_invariants.rs:132-135`), and
  `b1_hostile_a_cannot_influence_b` draws both ends of every `Dup` from A's own client
  range (`security_boundary.rs:197-202`) **[src-rust]**. So the residual is not merely
  accepted; it is **outside the search space of the properties that would find its
  consequences.**
- The named gap, if one more test is wanted: *A live and publishing, B live and publishing,
  `Dup{src:(A,vas), dst:(B,alias)}` → what?* Today the answer is `GpuError::LateMerge`
  (`gpu.rs:72`, raised `:1814`) — a loud refusal of the **event**, not a preserved
  separation — and nothing asserts it **[src-rust]**.

### 3.4 ★★★ "If two guest processes can alias, can two TENANTS?" — answered

**No, and for a reason that is stronger than the fix and weaker than it sounds.** Every
aliasing vector in §3.1–§3.3 requires **one shared `RmGraph`**, and two tenants have two
(§2) `[inferred]`. So T1's headline threat does not cross the tenant boundary at all.

⊘ But do not read that as "tenants are isolated". It says only that *this* mechanism does
not reach them. The tenant boundary is §2's process separation, and the question of
whether **it** holds is §4 and §5 — which is where the real answers are.

---

## 4. T2 — RESIDUAL DATA ON MEMORY REUSE

**The question:** when tenant A's GPU memory is freed and reallocated to tenant B, does
anything zero it?

### 4.1 ★ The answer for VRAM: **yes — RM scrubs, on FREE, and reuse is blocked until it lands**

This is the strongest single piece of good news in the document, and it is worth naming the
function and the moment as asked, rather than saying "the driver scrubs" **[src@580]**:

- The engine is `OBJMEMSCRUB`, an **asynchronous CE-driven memset**
  (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/mem_scrub.c:96` `scrubberConstruct`, registered into
  PMA at `:175`). Under Confidential Computing it swaps CE for SEC2 rather than turning off
  (`mem_scrub.c:171-186`).
- ★ **It runs on the FREE path, not the allocate path.** `heapFree_IMPL` → `vidmemPmaFree`
  (`ogkm-580: src/nvidia/src/kernel/mem_mgr/video_mem.c:378-406`) → `pmaFreePages`
  (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/phys_mem_allocator/phys_mem_allocator.c:1391`) →
  `scrubSubmitPages` (`:1576` → `mem_scrub.c:404`) → `_scrubMemory` (`mem_scrub.c:1047`),
  which builds an `ADDR_FBMEM` memdesc (`:1059-1062`) and issues the memset (`:1076`,
  `:1085-1088`). ★ `heap.c` — 4 277 lines — contains **zero** occurrences of "scrub"; the
  legacy non-PMA heap does not scrub, and RM closes that hole by *refusing the allocation*
  rather than serving it dirty (`video_mem.c:663-666`).
- ★★ **The async race is closed structurally, not by a wait.** A freed frame is marked
  `ATTRIB_SCRUBBING` instead of `STATE_FREE` (`phys_mem_allocator.c:1467`), and the
  allocator's free-frame scan requires **all eight bitplanes zero**
  (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/phys_mem_allocator/regmap.c:839-880`)
  with `MAP_IDX_SCRUBBING` never among the
  skipped planes. ⇒ **a frame with a pending scrub is not allocatable at all.** The bit is
  cleared only after hardware completion, via the scrubber's work-id/semaphore progress
  (`mem_scrub.c:999-1034`, `:323-373`) and `_pmaClearScrubBit`
  (`phys_mem_allocator_util.c:792-812`).
- Clients are told: `NV0080_CTRL_FB_CAPS_VIDMEM_ALLOCS_ARE_CLEARED` is set iff
  `memmgrIsScrubOnFreeEnabled()`
  (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/mem_mgr_ctrl.c:118-120`).

⇒ **On our target configuration — Turing-or-later dGPU, open modules, Linux host,
`RMCFG_FEATURE_PLATFORM_GSP == 0` (`ogkm-580: src/nvidia/generated/rmconfig.h:260,275`) — VRAM freed
by tenant A through PMA is scrubbed before tenant B can receive it** `[inferred]`. The
default is `NV_TRUE` for TU102…GB20C (`ogkm-580: src/nvidia/generated/g_mem_mgr_nvoc.c:381-391`).

### 4.2 ⚠ But it is conditional, and three of the conditions matter to us

Ranked by how plausibly they fire on a real deployment **[src@580]**:

1. ★★ **`NV_VASPACE_ALLOCATION_FLAGS_SKIP_SCRUB_MEMPOOL` = `BIT(10)`
   (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:3177`) is settable by ANY unprivileged
   client, with no privilege gate.**
   `ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/vaspace_api.c:634-637` translates it
   straight from the client's
   allocation params into `VASPACE_FLAGS_SKIP_SCRUB_MEMPOOL` (the privilege checks in that
   function are at `:124` and `:174`, for other flags). It reaches the free path —
   `ogkm-580: src/nvidia/src/kernel/mem_mgr/pool_alloc.c:353-360` adds
   `PMA_FREE_SKIP_SCRUB`, and `rmMemPoolTrim`/`rmMemPoolRelease`
   set `bSkipScrub` around their frees (`:809-813`, `:933-937`), reached with the client's
   own flags at `ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:1264`.
   ⇒ **PMA frames that held that client's GPU page tables
   return to the global free list dirty**, and the next allocation receives them with
   `PMA_ALLOCATE_RESULT_IS_ZERO` set (`phys_mem_allocator.c:682`) because PMA believes its
   own invariant. The disclosed bytes are PDE/PTE contents — physical addresses, aperture,
   kind and privilege bits — not arbitrary payload; that is a **structural** disclosure
   about another tenant's memory layout rather than its data `[inferred]`.
   ★ **Not reachable through this port today**: `alloc_vaspace` builds **all-zero**
   parameters of its own (`crates/kayfabe-isolate-host/src/rm.rs:1447-1452`) and no verb
   carries a guest VASpace alloc blob **[src-rust]**. But `FERMI_VASPACE_A` is on the
   alloc-class allowlist (`crates/kayfabe-abi/src/capability.rs:871`) and `RmBackend::alloc`
   forwards a guest params blob verbatim (`rm.rs:1441`), so this is the same *pre-emptive*
   shape as §5.5: it becomes live the moment a guest-driven VASpace alloc exists.
2. ⚠ **The vGPU gates.** Scrub-on-free is disabled outright under
   `PDB_PROP_GPU_IS_VIRTUALIZATION_MODE_HOST_VGPU` or `IS_VIRTUAL_WITHOUT_SRIOV`
   (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/mem_mgr_gm107.c:1474-1483`,
   `ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/ampere/mem_mgr_ga100.c:124-133`). We are
   a system that makes NVIDIA drivers believe things about virtualization, and the repo
   carries a live vGPU-posture decision. ⇒ **if our host-side configuration ever puts the
   host RM into a vGPU mode, this protection turns off silently.** Nothing in our tree
   reads `NV0080_CTRL_FB_CAPS_VIDMEM_ALLOCS_ARE_CLEARED` to notice **[src-rust]**.
3. **The scrubber fails OPEN.** If the scrubber object is invalid, `pmaFreePages` proceeds
   with no scrub and marks the frame `STATE_FREE` immediately
   (`phys_mem_allocator.c:1443-1448`, comment: *"We allow free with invalid scrubber
   object"*).

Plus two that are less likely to bite us: the regkey `RMDisableScrubOnFree`
(`mem_mgr.c:204-208`), and pre-Turing silicon.

★ **The one thing a guest cannot do**: `NVOS32_ALLOC_INTERNAL_FLAGS_SKIP_SCRUB`
(`nvos.h:1485`) is properly gated — `standard_mem.c:50-57` refuses any non-zero
`internalflags` below `RS_PRIV_LEVEL_KERNEL`, and every client entry point hard-sets
`CLIENTALLOC` (`video_mem.c:647`, `system_mem.c:208`, `virtual_mem.c:368`) **[src@580]**.
I found no `NVOS02`/`NV_ESC_RM_ALLOC_MEMORY` flag that skips the scrub.

### 4.3 ★ Sysmem is a DIFFERENT mechanism, and a weaker one

**[src@580]** On a discrete GPU there is **no sysmem scrubber at all**: `SysmemScrubber`
(`sysmem_scrub.c:44`) asserts `bFastScrubberSupportsSysmem` (`:56`), which is `NV_TRUE` only
for `GB10B | GB20B | GB20C` (`g_mem_mgr_nvoc.c:405-415`) — Tegra/SoC parts. Instead:

- Sysmem is zeroed **at allocation** by the Linux page allocator: `os.c:1013-1023` passes
  `PDB_PROP_SYS_INITIALIZE_SYSTEM_MEMORY_ALLOCATIONS` as `nv_alloc_pages`' `zeroed`
  argument → `ogkm-580: kernel-open/nvidia/nv.c:3846-3847` → `__GFP_ZERO`
  (`ogkm-580: kernel-open/nvidia/nv-vm.c:263-266`), with explicit `memset` fallbacks (`:413-416`,
  `:538-541`).
- `osFreePagesInternal` (`os.c:1058-1080`) does **no** zeroing on free.
- ⚠ **The whole thing is a module parameter.** `InitializeSystemMemoryAllocations` —
  *"1 = zero out system memory allocations (default), 0 = do not perform memory clears"*
  (`ogkm-580: src/nvidia/arch/nvalloc/unix/include/nv-reg.h:180-198`), applied at `osinit.c:211-218`.

⇒ **Sysmem residual safety between tenants rests on a host driver module parameter that
lives outside this system, is not read by anything in this tree, and would fail silently if
an operator turned it off for throughput** `[inferred]` **[src-rust]** for the "not read"
half. That is a smaller hazard than §4.2 item 1 but a wider one, because it needs no bug —
only a `modprobe` line.

### 4.4 Our own side: GPA arenas and memslot recycling — correct today, **by accident**

**[src-rust]** throughout.

- **`crates/kayfabe-core/src/gpa.rs` has no memory to zero.** It is a pure address-space
  allocator over `u64` ranges: not one occurrence of `zero`/`memset`/`fill(0)`/`MADV`/
  `PUNCH_HOLE` in 1 428 lines. Both levels **recycle globally**, not per-`Proc`:
  `GpaSpace::free: Vec<Range<u64>>` (`:49`) is popped by `carve()` (`:94-128`) before
  bumping, and `release()` (`:204-220`) pushes a range back after checking owner and
  containment and **nothing else**; `GpaArena::free: BTreeMap<u64,u64>` (`:309`) is
  first-fit-reused by `alloc()` (`:335-358`). ⇒ **a GPA range vacated by one `Proc` is
  handed verbatim to the next.** What the design enforces is *identity*, not content:
  `ArenaId` carries a monotonic `generation` (`:228-236`) so a dead proc's `GpaBlock` cannot
  be freed into the arena that inherited its address (`:401-406`).
  ★ `core_security_threat_model.md` §3 I1's *"arenas are per-`Proc`, disjoint by
  construction"* is **true and says nothing about this**: it is a statement about
  simultaneously-live procs, not about reuse across a dead one.
- **The coarse memslot tier is safe because backing is minted fresh, not because anything
  clears it.** Every `install_window` creates a new `mmap` and a new `memfd`
  (`crates/kayfabe-vmm-kvm/src/lib.rs:868,873`; same at
  `crates/kayfabe-vmm-qemu/src/lib.rs:1113,1118`); `remove_window` (`:999-1067`) drops the
  memslots, drops the `SharedRam` and retires the mapping — **no `MADV_REMOVE`, no
  `fallocate(PUNCH_HOLE)`, no `memset`**. The kernel supplies zero pages to the next one.
- **The fine tier is incidentally re-zeroed**: `unmap_guest` calls `window.restore(...)`
  (`kayfabe-vmm-kvm/src/lib.rs:1544-1546`), a fresh `MAP_PRIVATE|MAP_ANONYMOUS` over the
  range (`crates/kayfabe-linux-raw/src/mapping_unsafe.rs:330`) — justified in its own docs
  for a *completely different* reason (`:1511-1513`: a hole in the window's VMA would leave
  the memslot pointing at a gap).
- **A crate-wide grep for zeroing intent on teardown paths returns nothing.** Six hits
  total across `crates/`, all on serialisation buffers, ioctl-arg blanking, a test fixture,
  a comment about the C, and QEMU's balloon punch-hole *which we deliberately disable*
  (`crates/kayfabe-vmm-qemu/src/host.rs:204-231`).

⇒ **No residual-data exposure found on the recycling paths as built** `[inferred]` — and
every one of the three reasons is a side effect of something else, asserted nowhere and
tested nowhere. A future change that pools backing instead of minting it would break this
with no red test.

### 4.5 ★★★ The break is already designed, already written down, and not yet built

`docs/design/gpga_address_space.md` §9.2 (`:349-372`) states the hazard exactly, and it is
the owner's own observation **[src-rust]**:

> *"With arenas, a freed slice **stays inside our own reservation** and is handed to the
> next requester **directly**. **Nothing zeroes it.** If that next requester is a different
> guest process, it reads the previous one's data."* ⇒ *"Under reservation, the scrubber
> must perform a REAL clear… **Zero on FREE, not on allocate.**"*

That is precisely right, and §4.1 is the reason: RM's protection is a **free-path** property
of PMA, so the moment memory stops being returned to PMA — which is exactly what a
reservation arena is — the protection stops applying. The `HostSlice`/`HostExtent` shape
that arenas need **is built** (`crates/kayfabe-mmu/src/lib.rs`, recorded at
`gpga_address_space.md` §8.4); the reservation allocator that would use it is **not**, and
**there is no zeroing primitive anywhere in `kayfabe-vmm*`, `kayfabe-qemu-raw` or `gpa.rs`
waiting for it** **[src-rust]**.

★ **One correction to that document, in its favour.** Its §5 item 1 says the load-bearing
assumption is *"a newly allocated host object is already zero-initialised… NVIDIA scrubs
VRAM **on allocation** for security"* and asks for a citation. The citation is §4.1 above,
and **the direction is the other way round**: RM scrubs on **free** and makes the frame
un-allocatable until the scrub retires. The conclusion the document draws is unchanged and
its §9.2 inversion is unchanged — but the mechanism matters, because "scrubs on allocation"
would survive arenas and "scrubs on free into PMA" does not. ⇒ `gpga_address_space.md` §5
item 1 can now be closed as **cited**, with the direction corrected in place.

### 4.6 The T2 verdict

| path | scrubbed? | by what | status |
|---|---|---|---|
| VRAM via PMA, our target config | **yes** | RM's CE scrubber, on free; reuse blocked by `ATTRIB_SCRUBBING` | **relied on, unverified by us** |
| VRAM where `SKIP_SCRUB_MEMPOOL` was set | **no** | — | not reachable through this port **today** (§4.2 item 1) |
| VRAM under a vGPU-mode host RM | **no** | — | `[unknown]` whether our deployment ever triggers it |
| sysmem | zero-on-**alloc** | Linux `__GFP_ZERO`, defeatable by a module parameter | outside this system, unchecked |
| our GPA ranges | n/a | no memory in `gpa.rs` | recycled across procs, content unaddressed |
| our coarse memslots | **yes, incidentally** | fresh `mmap` + fresh `memfd` per install | not asserted, not tested |
| our fine-tier slots | **yes, incidentally** | anonymous re-`mmap` in `restore` | justified for another reason |
| **future reservation arenas** | ⊘ **no** | — | the designed break; **not built**; no primitive exists |

⊘ **Nothing in this section was measured.** It is a reading of `ogkm-580.159.04` and of this
tree. The experiment that would settle it is **HW-2** in §9.

---

## 5. T3 — ESCALATION, and what aggressive multi-tenant does to F11 and F14

### 5.1 F11's premise re-read against the code, and its citations corrected

`guest_blast_radius.md` F11 says the isolate's kernel-visible euid is the VMM's, and that
P holds only because `hClient` is never guest-derived. Both halves check out **[src-rust]**:

- The user-namespace map is written as the single line `0 {uid} 1` from `outer_ids()`
  (`crates/kayfabe-linux-raw/src/sandbox_unsafe.rs:609-613`; the gid map at `:614-618`).
  ⚠ F11 cites `:596-617`, which spans the rustdoc into the gid map — the write block is
  `:609-613`.
- **No uid change exists anywhere in the tree**: `setuid`/`setresuid`/`seteuid`/`setgid`/
  `setresgid` return **zero hits** across `crates/`. `surrender_privilege`
  (`sandbox_unsafe.rs:531`, called at `:828`) drops capabilities only.
- **Every host ioctl hard-codes our own client.** `crates/kayfabe-isolate-host/src/rm.rs`
  is the sole host-ioctl issuer in the workspace. `h_client: self.client` at `:858`
  (⚠ F11 cites `:857`), `:906`, `:938`, `:1001`; `h_root: self.conn.client` at `:1513`,
  `:2569`; every `raw_alloc` caller passes `client` / `self.client` / `self.conn.client`
  (`:554`, `:571`, `:582`, `:633`, `:1066`, `:1441`, `:1629`, `:1911`, `:1957`).
  Guest-derived values reach `h_object_parent` only, via `narrow()` (`:1420-1422`), and
  `alloc_engine_object` additionally refuses a parent this connection did not mint as a
  channel (`:1622-1624`).
- ★ **No test pins it.** Searching `crates/*/tests`, `tests/tests` and `fuzz`, nothing
  asserts that the host-facing `h_client` field is connection-owned; every `hClient` test
  hit is about the *guest*-facing namespace model, so F11's follow-up (2) is **unbuilt**
  **[src-rust]**.

### 5.2 ★★★ What F11 becomes under aggressive multi-tenant — and it is not what F11 says

F11 frames the euid widening as reaching *"every root GPU client on the host
(`nvidia-persistenced`, a display server, another root CUDA process)"*. **Under aggressive
multi-tenant the set it reaches includes every other tenant's isolate**, and that is a
categorically different claim: it is not a widening against host bystanders, it is the
tenant/tenant boundary itself.

The mechanism, re-derived here rather than carried across **[src@580]**:

1. `RmIoctl` computes `secInfo.privLevel` from `osIsAdministrator()` on **every** escape
   (`escape.c:304`), and `rmclientValidate_IMPL` then runs
   `_rmclientUserClientSecurityCheck` for any sub-kernel caller
   (`ogkm-580: src/nvidia/src/kernel/rmapi/client.c:748-762`).
2. That check ends in `osValidateClientTokens`, and the comparison is an **OR**:
   `if ((pClientTokenUser->euid != pCurrentTokenUser->euid) && (pClientTokenUser->pid !=
   pCurrentTokenUser->pid)) return NV_ERR_INVALID_CLIENT;`
   (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/os.c:3856-3858`). **A matching euid alone passes.**
   The token is `{euid, pid}` from `osGetSecurityToken` (`os.c:3787-3807`), and `euid` is
   the **initial-namespace** value (`ogkm-580: kernel-open/common/inc/nv-linux.h:156`).
3. The gate is on by default: `PDB_PROP_SYS_VALIDATE_CLIENT_HANDLE` initialises true
   (`ogkm-580: src/nvidia/generated/g_system_nvoc.c:103`), overridable only by a registry key
   (`ogkm-580: src/nvidia/src/kernel/core/system.c:715-724`).
4. **NVIDIA's own comment says what the check is for**, and it is exactly our threat:
   *"Validate the client handle to make sure that the user who created the handle is the
   one that uses it. Otherwise **a malicious user can guess the client handle created by
   another user and access information that its not privy to**."*
   (`ogkm-580: src/nvidia/src/kernel/rmapi/client.c:483-488`).

⇒ **Two tenants' isolates running under the same host uid pass RM's cross-client check
against each other** `[inferred]`. And naming a foreign client is not a partial
capability: RM resolves the object **inside that client**, and
`rsAccessGetAvailableRights` takes its *owner* arm — the resource's own access mask,
copied directly, rather than the share-policy path a non-owner would go through
(`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_access_map.c:140-165`, driven from
`rs_client.c:399-422`) **[src@580]**. ⇒ the caller is adjudicated **as the victim**
`[inferred]`. `NV_ESC_RM_MAP_MEMORY` on a foreign memory object is then a read of another
tenant's VRAM; `NV_ESC_RM_FREE` on a foreign root destroys it.

★ **And the audit's open question about guessability is answerable, in the unfavourable
direction.** `guest_blast_radius.md` §7 lists *"whether such a handle is guessable in
practice"* as not established. It is: client handles are
`CLIENT_ENCODEHANDLE(handleBase, clientCurrentHandleIndex)` — a **fixed base
`0xC1D00000`** (`ogkm-580: src/nvidia/inc/libraries/resserv/resserv.h:135`) **OR**'d with a
**monotonically increasing** index that the server carries across allocations
(`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_server.c:3241-3262`), over a range of `0x100000`
(`resserv.h:152`) **[src@580]**. Concurrent isolates on one host receive **adjacent**
values `[inferred]`. There is nothing to guess.

★★ **One genuinely reassuring narrowing, found in the same sweep and worth as much as the
alarm.** RM's *default* cross-client dup policy is **PID**-scoped, not euid-scoped:
`serverInitGlobalSharePolicies` installs `RS_ACCESS_DUP_OBJECT` with
`sharePolicy.type = RS_SHARE_TYPE_PID` on the default inherited list
(`ogkm-580: src/nvidia/src/kernel/rmapi/sharing.c:346-352`), and the callback compares
`pSrcClient->ProcID == pDstClient->ProcID`
(`ogkm-580: src/nvidia/src/kernel/rmapi/client_resource.c:217-228`) **[src@580]**. Separate isolates
are separate processes, so **`NV_ESC_RM_DUP_OBJECT` across tenants is refused by RM even at
a matching euid** `[inferred]`. The exposure in this section is the *direct-naming* path,
not the dup path — a distinction worth keeping, because it means the fix in §9 has a
smaller job than it first appears.

⚠ Note also that `serverCopyResource` validates **only the destination client** against the
caller's security info (`rs_server.c:1695`) — the source client is never validated. The
share-policy check inside `clientCopyResource_IMPL` (`rs_client.c:542-552`) is what carries
that boundary, which is why the previous paragraph's default matters so much **[src@580]**.

### 5.3 ★★★ The sharpest form: the sandbox exists because isolate compromise is IN MODEL

This is the finding I would lead with if only one survived.

The isolate is *deliberately* the component that faces hostile input, and the tree says
compromise is in-model rather than excluded: `guest_blast_radius.md` F9 argues seccomp's
value is *"defence in depth for the host **kernel** boundary if the isolate is ever
compromised through the driver"*, and F10 states that P being true means a
malformed-payload bug in the host driver is *equally* reachable **[src-rust]**. The whole
architecture of a cap-dropped, userns-confined, `pivot_root`ed child is an admission that
the code inside it may fall.

**What happens the moment one does, on a multi-tenant box:** the compromised isolate has
the VMM's euid (§5.1), therefore passes RM's client check against every other tenant's
isolate (§5.2), therefore can name, map and free their RM objects — and there is no
seccomp filter to stop the ioctls (**absent tree-wide**: no `seccomp` syscall, no
`SECCOMP_*`, no `libseccomp`; `crates/kayfabe-isolate-host/src/lib.rs:57` says so in
prose) **[src-rust]**.

⇒ **Post-compromise containment across tenants is currently zero** `[inferred]`. The
capability drop bounds what a compromised isolate can do *to the host*; it does nothing to
bound what it can do to *co-tenants*, because RM's own boundary is keyed on a credential
the sandbox does not change. ★ And this is exactly why §9's first item is a uid change and
not a filter: a filter constrains our code, and our code is what has already been assumed
to fail.

### 5.4 F14 — the fd crossing, re-read and re-scoped

`guest_blast_radius.md` F14's structural claim is correct and I re-checked the reachability
half **[src-rust]**: `CrossedFd` has **no production call site** — it appears only in
`crates/kayfabe-isolate-host/src/fdcross.rs` (definition `:81`, methods `:97`–`:162`), its
re-export (`lib.rs:71,77`) and `crates/kayfabe-isolate-host/tests/fd_crossing.rs`. Neither
`child.rs`, `isolate.rs`, `proto.rs` nor any `bin/` uses `write_frame_with_fds` /
`read_frame_with_fds`. F14's *"latent because the consumer is unbuilt"* is exact.

**What aggressive multi-tenant adds to it** is not the escalation F14 names (a root VMM
getting the 265 `PRIVILEGED` controls — that is a host-boundary claim) but a **cross-tenant
composition**, and it is worth writing down before the consumer is built:

- `FdOrigin::Isolate(a)` may be lent only to `a`; `FdOrigin::Vmm` may be lent to **any**
  isolate (`fdcross.rs:138` `lend_to`, documented `isolate_vmm_fd_crossing.md` §7)
  **[src-rust]**. That asymmetry is correct for a VMM-minted `memfd`. It is *not* correct
  for a descriptor the VMM obtained from an isolate and re-minted, or for one the VMM
  created itself on a crossed GPU fd — both would carry `FdOrigin::Vmm` and be lendable
  everywhere `[inferred]`.
- `isolate_vmm_fd_crossing.md` §9 already records that `lend_to` checks **identity, not
  liveness**, so a descriptor held across an isolate's death is accepted by a reincarnated
  same-`(proc, gpu)` isolate; `does_not_observe_isolate_lifetime`
  (`tests/fd_crossing.rs:735`) pins the gap **[src-rust]**. Within one VM that is the
  known hole. ⚠ **Across VMs, `IsolateId` is `(proc: u32, gpu: GpuId)`
  (`crates/kayfabe-isolate/src/lib.rs:515-536`) and carries no VM discriminator** — so if a
  single VMM ever served two VMs, or if a broker ever spanned two, two tenants' isolates
  would compare **equal**. Not reachable today (§2); recorded because the identity type is
  the thing that would have to change first, and changing it later is the expensive
  version.
- ★ F14's own recommended mitigation — *"a `seccomp` filter on the VMM refusing `ioctl` on
  descriptors of this class"* — **does not exist in any form** **[src-rust]**.

### 5.5 ★ A finding the audit missed: three allowlisted controls are cross-client primitives, and one carries a HOST FILE DESCRIPTOR

`guest_blast_radius.md` F10 says guest-chosen classes and payloads reach the driver and
that this is inside P, because an unprivileged local process may send any payload too. That
is true for P. It is **not** the whole question for cross-tenant, because it does not
distinguish a payload that is **opaque data** from a payload that is **a reference into one
of our own namespaces**.

Three entries on the ingress allowlist (`crates/kayfabe-abi/src/capability.rs`) are
cross-client sharing primitives **[src-rust]**:

| cmd | name | line | RM flags **[src@580]** |
|---|---|---|---|
| `0x00000d04` | `NV0000_CTRL_CMD_CLIENT_SET_INHERITED_SHARE_POLICY` | `:568` | `0x9` = `NON_PRIVILEGED`, `accessRight=0x0` |
| `0x00003d05` | `NV0000_CTRL_CMD_OS_UNIX_EXPORT_OBJECT_TO_FD` | `:569` | `0x9` = `NON_PRIVILEGED`, `accessRight=0x0` |
| `0x00003d06` | `NV0000_CTRL_CMD_OS_UNIX_IMPORT_OBJECT_FROM_FD` | `:570` | `0x9` = `NON_PRIVILEGED`, `accessRight=0x0` |

(flags read from `ogkm-580: src/nvidia/generated/g_client_resource_nvoc.c:1542-1556`, `:1632-1646`,
`:1647-1661`).

Two things make them different in kind from the rest of the allowlist:

1. ★★ **`IMPORT_OBJECT_FROM_FD` bypasses RM's PID share policy entirely.** The
   implementation resolves the caller-supplied `fd` with `nv_get_file_private(pParams->fd,
   …)` — **in the calling process's descriptor table** — and calls `RmImportObject`
   (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/os.c:2505-2529`), which dups from the RM-internal
   client `hObjExportRmClient` through `RMAPI_API_LOCK_INTERNAL`
   (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/rmobjexportimport.c:639-641`) **[src@580]**. At
   kernel privilege with no `REJECT_KERNEL_DUP_PRIVILEGE` flag, `clientCopyResource_IMPL`
   takes its `else` arm and **skips the access-rights check** (`rs_client.c:542-552`)
   **[src@580]**. ⇒ **possession of the exported descriptor IS the capability**, and §5.2's
   PID-scoped default does not apply to it `[inferred]`.
2. ⚠ **The payload carries a host fd number.** `RM_CONTROL` payloads are forwarded
   verbatim (`crates/kayfabe-fwd/src/lib.rs:1925-1933`'s `else` arm → `route_control`,
   `crates/kayfabe-rt/src/device.rs:1444-1487` → `plan_control` `kayfabe-fwd:1968` →
   `crates/kayfabe-isolate/src/lib.rs:1667-1670` → `rm.rs:872`) **[src-rust]**, so a
   guest-chosen integer would be resolved by the host kernel **as an index into the
   isolate's own descriptor table** — which holds `/dev/nvidiactl`, `/dev/nvidia<N>` and
   the VMM socket. That is a confused-deputy shape, and it is a different animal from
   "arbitrary bytes in a params blob".
3. `SET_INHERITED_SHARE_POLICY` lets a client install `RS_SHARE_TYPE_ALL` grants on its own
   objects, dissolving RM's cross-client boundary for that client. As an attack it is
   self-harm (a hostile tenant can only open **itself**) `[inferred]` — but it is a live
   lever for a *collusion* or *coerced-victim* shape, and it should not sit on a
   default-forward allowlist unexamined.

★ **Why this is latent today, precisely.** The guest-facing bridge does **not** yet emit a
forward for the control long tail: `kayfabe-rmrpc` refuses unmodelled controls
(`UnknownControl` / `GspRuleControlUnserviced`), and its own doc says *"`Translation::
Forward` … needs `kayfabe_fwd::classify_control` … when a forward arm lands, **this** is the
site it replaces"* (`crates/kayfabe-rmrpc/src/lib.rs:508-575`) **[src-rust]**. So the guard
is *"the forward arm is unwritten"*, which is a schedule, not a control. ⇒ **These three
entries should be removed from the allowlist, or explicitly denied, before that arm
lands** — the six existing `ControlPermit::Denied` entries (`capability.rs:1197-1230`) are
the mechanism and it already exists.

### 5.6 A second thing the audit missed, smaller: `apply_promote_ctx` still compares client VALUES

`crates/kayfabe-core/src/promote.rs:349` gates on
`proc.client_values().contains(&p.client)`, and `Proc::client_values()` (`gpu.rs:400`) is
documented as the **lossy by-value view** of `Proc::clients: BTreeSet<ClientKey>`
**[src-rust]**. It is defence-in-depth over a route that is already `(GpuId, Pdb)`-keyed
(`promote.rs:296-311`), so it cannot cause a mis-route — but it is the one site where a
`ClientKey`-scoped comparison was available and a value comparison was used, and it is the
exact shape §12.44 exists to remove. No test was found that bites it **[src-rust]**.

---

## 6. Side channels — does `#128` change the picture?

`docs/design/register_plane_read_native.md` proposes that on the GPU's register/doorbell
BAR pages **reads become native passthrough** (memslot-backed, no vmexit) while writes stay
trapped; its own status line reads *"ruled, not yet built"* (`:5`) **[src-rust]**. The two
timer registers it names are `NV_PTIMER_TIME_0/1` at `0x9400`/`0x9410` (`RW-4R`, the pair
`NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET` exists so clients may map directly) and the VF
mirror at `0xBB0080`/`0xBB0084` (`:66-76`).

**Built or not — checked** **[src-rust]**:

- **Not built.** `0x9400` / `0x9410` / `PTIMER_TIME` appear **nowhere** in code as
  constants. `Vmm::map_read_native` exists (`crates/kayfabe-vmm/src/lib.rs:830-836`, KVM
  impl `crates/kayfabe-vmm-kvm/src/lib.rs:1699-1728`) but it installs a read-only memslot
  over an **ordinary host memory backing**, never a host GPU BAR, and it has **zero
  production call sites** — every reference is a test or a mock. No `mmap` of
  `/sys/bus/pci/.../resource0` or of `/dev/nvidia*` into a memslot exists anywhere.
- What *is* built is full emulation: `VIRTUAL_FUNCTION_TIME_0/1` served from the emulated
  register plane (`crates/kayfabe-device/src/ga10x.rs:466`, `:524`;
  `crates/kayfabe-device/src/plane.rs:499`, `:517`) off a host monotonic clock
  (`crates/kayfabe-qemu-raw/src/shim.rs:799-844`).

**Does it change the picture?** For confidentiality between tenants — ★ **yes, and the
doc's own security paragraph understates it in a specific way.** §7 (`:87-90`) says, in
full: *"A read-only free-running host counter is a **low-risk exposure** but is a
**high-resolution timing side channel**, and it leaks host GPU uptime. Say so in the
security model when this lands."* Three observations:

1. It names **one** leak — host GPU uptime — and self-classifies as low risk with no
   argument. The cross-tenant direction is absent: a nanosecond-resolution counter that is
   *the same counter* for every guest on the box is a **shared, high-resolution timebase**,
   which is the standard precondition for a contention-based covert channel and for
   timing-oracle attacks between co-tenants `[inferred]`.
2. ⚠ §3 of that note (`:41-52`) *argues for* the shared timebase on the grounds that
   correlation between guest and host traces is then correct by construction. That is a
   good engineering reason and it is the same property that makes cross-tenant correlation
   possible; the note does not connect the two **[src-rust]**.
3. The instruction *"say so in the security model when this lands"* has **not** been
   executed: neither `guest_blast_radius.md` nor `core_security_threat_model.md` contains
   any timing-side-channel finding **[src-rust]**. This document is that paragraph.

⊘ **What I did not do and will not pretend to:** I did not estimate a channel bandwidth,
and I have no measurement of whether a practical cross-tenant channel exists on a GA106
through the PTIMER. `[unknown]`. The honest statement is that `#128` converts a *modelled,
per-VM* clock (today: `HostMonotonicClock` behind an emulated register, which we could
perturb or quantise at will) into a **shared physical one we no longer mediate**, and that
losing the mediation point is the security-relevant change — not the resolution per se.

⚠ Separately, `register_plane_read_native.md:73-76` flags that the PF timer pair is
**writable** (`tmrSetCurrentTime_GV100`), that a guest write must not reach the host GPU,
and that *"the policy there must be **decided explicitly**"* — currently undecided
**[src-rust]**. That is an integrity item on the same page, and on a multi-tenant box a
guest write that reached the host timer would be a cross-tenant integrity effect, not a
self-inflicted one.

---

## 7. What I could not determine

Stated as gaps, not padded into guesses.

1. **Whether §5.2's euid path is exploitable end-to-end.** I have the mechanism from a
   reading and no run on either side of it. What is missing is not a further reading but an
   experiment: two isolates at the same uid, one naming the other's `hClient`. Named as
   **HW-3** in §9. `[unknown]`
2. **Whether the deployment actually runs VMMs as root, or as one shared service uid, or
   as one uid per tenant.** This decides whether §5.2 is live or moot, and it is an
   *operator* fact that appears nowhere in this tree. `guest_blast_radius.md` §7 also
   records that P was never analysed for a non-root VMM. `[unknown]`
3. **The bandwidth or practicality of a PTIMER-based cross-tenant channel** (§6).
   `[unknown]`
4. **Whether anything besides `_rmclientUserClientSecurityCheck` stands between an isolate
   and a foreign RM client.** I traced the direct-naming path and the dup path; I did not
   exhaustively enumerate every escape's own validation. `[unknown]`
5. **What the host driver does with a bit-15 command absent from its tables** — carried
   forward unchanged from `guest_blast_radius.md` F6; the space is GSP-serviced and GSP is
   a signed binary. `[unknown]`
6. **Whether `NV_VASPACE_ALLOCATION_FLAGS_SKIP_SCRUB_MEMPOOL` being unprivileged is
   intentional** (§4.2 item 1). It is not documented as privileged and no gate was found on
   its path, but RM's `RS_ACCESS`/capability layer was not enumerated exhaustively for it.
   `[unknown]`
7. **Whether non-root GMMU page-table levels are fully overwritten before use.** Only the
   *root* level is explicitly scrubbed (`gmmu_walk.c:351-357`, *"in case GMMU prefetches
   some uninitialized entries"*) **[src@580]**; whether `mmu_walk`'s entry fill covers every
   byte of a lower level was not traced. This is the residue that item 6's flag would
   expose. `[unknown]`
8. **Whether any of §4's gate values are actually set on a running host.** Every default in
   §4.2 is read from a source or generated initialiser. Nothing was executed. `[unknown]`
9. ⊘ **Nothing in this document was reproduced on hardware by me, and the per-tenant
   isolation model has never been reproduced on hardware by anyone** (§0).

---

## 8. The ranked list — what must be true for aggressive multi-tenant

Ranked by what it would cost to be wrong. Each marked **built** / **designed** / **unknown**
— and, separately, whether anything *tests* it, because the two are not the same.

| # | Must be true | Status | Tested? |
|---|---|---|---|
| **1** | **A compromised isolate cannot reach another tenant's RM objects.** | ⊘ **FALSE as built** — §5.3. Shared euid defeats RM's client check; no seccomp exists | no |
| **2** | **No guest-supplied value ever reaches a host `hClient` field, or a host fd number.** | **built** for `hClient` (§5.1); ⚠ *not* structurally — call discipline in one file, and §5.5's fd-carrying controls are allowlisted | ⊘ **no test at all** |
| **3** | **Tenant A's freed GPU memory is unreadable by tenant B.** | **built — by the HOST DRIVER, not by us** (§4.1). Three ways off, one of them unprivileged (§4.2); sysmem rests on a host module parameter (§4.3); the reservation-arena design removes it (§4.5) | ⊘ no — nothing in this tree asserts, checks or measures it |
| **4** | **Two tenants never share one core instance / one `RmGraph`.** | **built** (§2) — but as a consequence of deployment, not as an invariant; `IsolateId` carries no VM discriminator (§5.4) | no — nothing asserts it, so nothing notices if it changes |
| **5** | **The VMM does not hold an `ioctl`-capable GPU descriptor.** | **built** today only because `CrossedFd` has no consumer (§5.4); the transport is built and the mitigation (seccomp) does not exist | the *transport's* refusals are tested (18 tests); the *property* is not |
| **6** | **Cross-tenant address/execution isolation inside one VM.** | **built** and property-proven — I1–I4, §3.1 | ★ yes, and non-circularly (§3.2) — the strongest item on this list |
| **7** | **The isolate holds no capability and opens the device only after surrendering.** | **built**, and measured by its author `[run: commit 2575177, 2026-07-30, RTX 3060 / 580.159.04, root VMM]` | yes — `sandbox_escape.rs:412`, `:473`, `:515` |
| **8** | **No cross-tenant high-resolution timing channel.** | **designed away today** (emulated PTIMER); `#128` would remove the mediation point (§6) | no |
| **9** | **The live-destination `DUP_OBJECT` merge is A3-only.** | **designed** — argued in prose (`l1_concurrency.md:4069-4075`), excluded from both fuzz generators (§3.3) | ⊘ the shape is outside the properties' search space |
| **10** | **Any of the above on real hardware, with two VMs.** | **unknown** — never attempted (§0) | — |

⊘ **Item 9's DoS sibling is out of scope by directive** and is not ranked here: a hostile
tenant can hang a host engine and nothing in the construction prevents it
(`guest_blast_radius.md` §5). It is the repo's own standing multi-tenant blocker and this
document does not revisit it.

---

## 9. The smallest change set that would make aggressive multi-tenant defensible

Ordered by ratio of boundary bought to work required. ⊘ None of these is a substitute for
§8 item 10.

### C1 — ★★★ Run each isolate at a distinct, non-zero uid *(the single highest-value change)*

Today the userns map is `0 {outer_uid} 1` (`sandbox_unsafe.rs:609-613`), so every isolate on
the host presents the VMM's euid. Mapping each isolate to a **distinct** uid — a per-isolate
subuid, or at minimum `nobody`, but distinct **per tenant** — makes RM's own
`osValidateClientTokens` check (§5.2) refuse cross-tenant client naming **in the host
kernel**, independently of our code. That is the property §5.3 needs: it survives the
compromise of our code, which is the case the sandbox exists for.

⚠ It is not free. `guest_blast_radius.md` F11 already notes it needs its own analysis of
what in the sandbox assumes uid 0, and the rootless arm of `acquire_mount_namespace`
(`:586-604`) versus the privileged fallback (`:624`) behave differently here. But it is the
only item on this list that moves the boundary from *our* code to the *kernel's*.

### C2 — Make "no guest-derived client handle, no guest-derived descriptor" structural, not a convention

Two halves, both cheap:

- A newtype only `RmConnection` can mint, required by every ioctl builder's client field, so
  F11's counterexample becomes unrepresentable rather than merely not-currently-written.
  Same shape as F7's recommendation for `DevDir` and as §4's confused-deputy primitive.
- **Deny `0x0d04`, `0x3d05`, `0x3d06`** in `CapabilityTable` (§5.5) — the
  `ControlPermit::Denied` mechanism already exists at `capability.rs:1197-1230`. Do it
  **before** the control-forward arm lands, not after.

Plus the test F11 asked for and nobody wrote (§5.1).

### C3 — Do not inherit the host driver's scrub silently; pin it, and build the arena clear before the arena

Three parts, none large, ordered by when they are cheap:

1. **Read the driver's own answer at bring-up.** RM publishes
   `NV0080_CTRL_FB_CAPS_VIDMEM_ALLOCS_ARE_CLEARED` iff scrub-on-free is enabled
   (`mem_mgr_ctrl.c:118-120`) **[src@580]**. Query it once per isolate and **refuse to
   serve a second tenant** if it comes back false. That turns §4.2's three silent
   disable-paths — the vGPU-mode gate, the regkey, pre-Turing silicon — into one loud
   refusal, and it costs one allowlisted control.
2. **Zero on free in the arena, before the arena exists.** `gpga_address_space.md` §9.2
   already rules it (*"Zero on FREE, not on allocate"*, a CE memset). Today there is **no**
   zeroing primitive in `kayfabe-vmm*`, `kayfabe-qemu-raw` or `gpa.rs` **[src-rust]**; the
   `HostExtent::Slice` type that will carry the requirement is already built. Building the
   clear at the same time as the reservation allocator is the difference between a
   requirement and a retrofit.
3. **Correct `gpga_address_space.md` §5 item 1 in place** (§4.5): the assumption it asks to
   verify is now cited, and the direction is the opposite of what it says. Leaving a
   "verify this" open next to a satisfied citation is how the correction-downstream failure
   in `claim_ledger.md` §0 happens.

⊘ Explicitly **not** proposed: zeroing our GPA/memslot recycling paths. §4.4 found no
exposure there, and adding a clear to a path that does not need one buys nothing and hides
the day it starts needing one. What those paths need is an **assertion**, not a memset.

### C4 — Seccomp on the VMM for GPU-class descriptors, before `#128`'s consumer exists

`isolate_vmm_fd_crossing.md` §11 item 2 already names it and marks it *stated, not
mitigated*. F14 makes it load-bearing for P; §5.4 makes it load-bearing for the tenant
boundary. Doing it while `CrossedFd` still has no consumer is the cheap moment.

### C5 — Write the deployment invariant down, and give `IsolateId` a VM discriminator

Not a mechanism, a statement: *"one VM per VMM process; two tenants never share a core
instance; isolates of different tenants never share a uid."* Today §2's separation is
real and invisible, so nothing can regress *loudly*. Adding a VM/tenant field to
`IsolateId` (§5.4) is the one structural piece, and it is far cheaper now than after the
fd crossing has consumers.

### The evidence that would be needed to BELIEVE any of it

⊘ Every item below is a **hardware run that has never been performed**. Named so they can
be scheduled, not so they can be cited.

| id | the run | what it decides |
|---|---|---|
| **HW-1** | **Two VMs, one host GPU, concurrently, both doing real compute.** Has never been done — not once, in either the C or this tree (§0). | Whether "aggressive multi-tenant" is a configuration that boots at all before it is one that is secure |
| **HW-2** | **The residual-data experiment**: isolate A allocates VRAM on the exact path we drive, writes a pattern, frees; isolate B allocates and reads. Repeat for sysmem, and for a recycled GPA/memslot. Control: the same with an explicit zeroing step. | §4 / C3 — the only way to convert §4's reading into a fact |
| **HW-3** | **The euid experiment**: two isolates at the same uid; one issues `NV_ESC_RM_CONTROL` and `NV_ESC_RM_MAP_MEMORY` naming the other's `hClient`. **Control arm: the same two isolates at different uids** — the control is what makes it evidence for C1 rather than just alarming. | §5.2 / §5.3 / C1 |
| **HW-4** | **`#14` P0/P1 at all**: two concurrent CUDA processes in one guest, on hardware, on a binary that is actually HEAD (`#95`). | Item 6 of §8 is property-proven in logic and has never been observed |
| **HW-5** | **Post-compromise**: a deliberately-subverted isolate (a debug build that issues a foreign-`hClient` ioctl on command) on a two-tenant box, before and after C1. | The one that would actually falsify §5.3 |

★ HW-3 and HW-5 are the two that would change the answer in §1. HW-1 is the prerequisite for
every other row on the list.

---

## 10. Where this is cross-referenced

- `guest_blast_radius.md` — P, the blast-radius property, and findings F1–F14. This
  document **corrects two citations** (F11's `:596-617` → `:609-613`, and `rm.rs:857` →
  `:858`), **answers one of its six open questions** (handle guessability, §5.2,
  unfavourably), and **re-scopes F11 and F14** from host-bystander widenings to the
  tenant/tenant boundary. P itself is unchallenged: nothing here is a counterexample to P,
  because every finding is inside what a local unprivileged process could already do.
- `core_security_threat_model.md` — I1–I4. §3 confirms its §4/§12.44 claims against the
  code; §5.6 names one site where the value-vs-`ClientKey` distinction was not applied.
- `isolate_vmm_fd_crossing.md` — §5.4 extends its §7 and §9 to the cross-VM case.
- `gpga_address_space.md` — §4.5 **closes its §5 item 1** (the scrub assumption is now
  cited) and corrects its direction; its §9.2 is the source of C3 item 2 and is confirmed
  correct against `ogkm-580`.
- `register_plane_read_native.md` §7 — §6 is the security-model paragraph that note asks
  for.
- `compute_limiting_and_priority.md` — the wedge, deliberately out of scope here.
