# Memory cacheability — the four deciders, the C's scars, and what `kayfabe-linux-raw` can and cannot enforce

> **STATUS: LIVE — 2026-08-14 (w312).** Research + design record for `CachePolicy` in
> `kayfabe-linux-raw` (`l1_os_shell.md` §4). Supersede it in place; do not write a successor
> beside it.
>
> **§§0–6 are unchanged and still current.** ★ **§7 and §8 are new in w312** and are the
> reason to re-read this file: §7 records the owner's *"map everything write-back in the VMM"*
> proposal and the verdict the rest of this document supports, so it is not re-proposed from
> scratch; §8 records the **guest-side instrument** that closes the gap §1.1 and
> `memtype.rs`'s header both name — *"nothing in userspace can read a guest's effective type"*
> — together with three things it measured that correct claims made elsewhere in this tree.
>
> Written before M2-c builds on the mapping API, because a cache attribute cannot be
> retrofitted into a mapping API — every call site would have to change.
>
> Sources: the C tree at `/workspace/nvidia-gpu-passthrough` (`src/`, `docs/`, `tools/`) and
> the NVIDIA open kernel modules at `research_clones/ogkm`. Every claim below carries a
> citation; where two sources in the C repo disagree, that is said out loud rather than
> smoothed over.

---

## 0. The one-paragraph version

`mmap(2)` has no cacheability argument. A userspace mapping's cache attribute is decided
**entirely by what is being mapped**: ordinary kernel pages (anonymous, `memfd`, `tmpfs`,
a regular file) are write-back and nothing can change that, while a device mapping gets
whatever the driver's own `mmap` handler put in `vma->vm_page_prot`. So a `CachePolicy`
parameter cannot *cause* an attribute. It exists to (1) refuse the provably unattainable,
(2) force every call site to state its requirement so the requirement exists in the source
at all, and (3) name, per variant, who owns the parts this layer cannot enforce.

---

## 1. The four independent deciders

A byte has **one** effective cache type. Four parties get a say and they do not negotiate.

| # | Decider | Set by | Overridden by |
|---|---|---|---|
| 1 | host userspace PTE | kernel page allocator (RAM) or the driver's `mmap` handler (device fd) | — |
| 2 | host EPT/NPT entry | KVM, from `kvm_is_mmio_pfn()` + arch rules | — |
| 3 | guest PTE | guest kernel | decider 2, **on Intel only** |
| 4 | what the device needs | NVIDIA RM, at allocation time | nothing — it is the requirement |

### 1.1 x86: how deciders 1–3 combine

- **PAT × MTRR.** The effective type is a combination, not an override. Linux's
  `pgprot_noncached` on x86-64 is **`UC-`** (weak uncached), *not* strict `UC`, and `UC-`
  combines with MTRRs: over a range with a write-combining MTRR it is effectively
  write-combining; without one it is uncached. NVIDIA relies on this deliberately —
  `ogkm/kernel-open/nvidia/nv-mmap.c:381-389`:

  > *"For frame buffer memory, callers are expected to use the UC- memory type if we report
  > WC as unsupported, which translates to the effective memory type WC if a WC MTRR exists
  > or else UC."*

  and warns about the spelling at `ogkm/kernel-open/common/inc/nv-pgprot.h:51-54`:

  > *"Note: the kernel's implementation of pgprot_noncached() on x86-64 evaluates to UC-
  > (noncached weak ordering) instead of strict UC."*

  The same combining rule is why NVIDIA refuses a *cached* framebuffer mapping outright —
  `ogkm/kernel-open/common/inc/nv-linux.h:864-880`:

  > *"If a WC MTRR is present, we can't satisfy the WB mapping attempt here, since the
  > achievable effective memory types in that case are WC and UC, if not it's typically UC
  > (MTRRdefType is UC); we could only satisfy WB mapping requests with a WB MTRR."*
  > → `#define NV_ALLOW_CACHING(mt) ((mt) == NV_MEMORY_TYPE_SYSTEM)`

- **`track_pfn_remap()` / `reserve_pfn_range()` silently downgrade.** A `remap_pfn_range`
  of a range `/proc/iomem` does not call *System RAM* has its requested write-back
  `vm_page_prot` **silently rewritten to `UC-`**. No error, no log. This is #111's proximate
  mechanism (`src/guest/nvkvm_mmap.c:44-66`).

- **Intel EPT vs AMD NPT — the vendor split that hid the bug for weeks.** For a
  struct-page-backed memslot, Intel EPT sets **`IPAT`** and forces write-back regardless of
  the guest PTE; **AMD NPT has no `IPAT` and honours the guest PTE.** Verbatim,
  `src/guest/nvkvm_mmap.c:52-57`:

  > *"On an Intel host this was masked: KVM's EPT sets the IPAT bit and forces WB for any
  > struct-page memslot regardless of the guest PTE. On an AMD host (NPT honors the guest
  > PTE memtype) the UC- sticks, and CPU access to these cache-coherent, memfd-backed window
  > pages runs ~100x slow (measured 0.12 GB/s vs 14 GB/s on an EPYC 7K62 / RTX 3060)."*

  There is **no runtime vendor detection** anywhere in the C — no `x86_vendor`, no CPUID
  check. The fix is unconditional and portability rests on the argument in that comment.

- **The other direction, and it matters more to us:** for a *real device-BAR* pfn,
  `kvm_is_mmio_pfn()` is true and KVM forces the EPT type **uncached regardless of the guest
  PTE**. The project measured this — `docs/design/async_event_delivery.md:37-43`:

  > *"matmul still PASSED under WB-all → on x86 KVM, true device-BAR pfns get EPT=UC forced
  > by `kvm_is_mmio_pfn()` regardless of guest PAT, so a WB doorbell does not hang. The
  > c5d5d8a 'would hang' fear was untested/over-cautious."*

  **This is the single most important line for the Rust design**, and §4 draws the
  consequence.

### 1.2 arm64: MAIR, not PAT — and the differences are load-bearing

| x86 concept | arm64 equivalent | Difference that bites |
|---|---|---|
| PAT index | MAIR index in the PTE (`PTE_ATTRINDX`) | no PAT MSR, no per-page combining with MTRRs |
| MTRR | **none** | so "cached framebuffer" is *permitted* on arm64 and refused on x86 (`nv-linux.h:879`: `NV_ALLOW_CACHING(mt) ((mt) != NV_MEMORY_TYPE_REGISTERS)`) |
| `UC-` (weak uncached) | **does not exist** | `NV_PGPROT_UNCACHED_WEAK` is defined only under `NVCPU_X86_64` (`nv-pgprot.h:63-64`); `nv_encode_caching`'s `UNCACHED_WEAK` case falls through to `UNCACHED` |
| WC (PAT `WC`) | `MT_NORMAL_NC` | on arm64, write-combining and sysmem-uncached are **the same attribute** — `nv-pgprot.h:59-60`: `#define NV_PGPROT_WRITE_COMBINED(old_prot) NV_PGPROT_UNCACHED(old_prot)` |
| UC (PAT `UC`) | `Device-nGnRE` **only for device memory** | Normal-NC vs Device is a difference *in kind*, not degree |

The distinction NVIDIA is most emphatic about, `nv-pgprot.h:44-49`:

> *"Don't rely on the kernel's definition of pgprot_noncached(), as on 64-bit ARM that's not
> for system memory, but device memory instead."*
> → `#define NV_PGPROT_UNCACHED(old_prot) __pgprot_modify((old_prot), PTE_ATTRINDX_MASK, PTE_ATTRINDX(MT_NORMAL_NC))`

with `NV_PGPROT_UNCACHED_DEVICE` kept as the escape hatch that *does* give Device memory,
used for every non-`SYSTEM` memory type (`nv-mmap.c:346-409`).

**Consequences for us, if arm64 is ever built:**

1. **`WriteCombining` and `Uncached` collapse for system memory.** Two of our three variants
   are the same MAIR index there. The enum does not need an arm64 branch, but a *test* that
   distinguishes them by behaviour would be testing nothing on that arch.
2. **Device memory forbids unaligned and multi-register access.** A bulk copy that is merely
   slow on x86 is architecturally invalid on `Device-nGnRE`. RM reports this to clients —
   `src/nvidia/src/kernel/rmapi/client_resource.c:1146-1157`: unaligned BAR1 access is
   advertised as unsupported exactly when `PDB_PROP_CL_DISABLE_IOMAP_WC` forces Device type.
   `MappedRegion`'s `copy_nonoverlapping` is therefore **not** a legal access surface over a
   Device-typed arm64 mapping; `VolatileRegion`'s aligned ≤8-byte atomics are.
3. **Coherence is not free.** `os_flush_cpu_cache_all` / `flush_cache_all()` exists and is
   compiled **aarch64-only** (`ogkm/kernel-open/nvidia/os-interface.c:1048-1097`), skipped
   when `nvos_is_chipset_io_coherent()`. The x86 "PCIe DMA snoops, so write-back is coherent"
   argument is an *x86* argument.
4. The write-combining drain is `dsb st`, not `sfence` (`os-interface.c:1099-1108`).
5. ppc64le has no write-combining at all (`nv-pgprot.h:74-77`) — out of scope, noted so the
   enum is not mistaken for universal.

---

## 2. What each region class actually needs — from the driver, not from intuition

NVIDIA's `nvidia_mmap_helper` (`ogkm/kernel-open/nvidia/nv-mmap.c:573-623`) is the single
point where a device-node mapping's attribute is chosen. It classifies by **address range
within the node**, not by node:

| Region | `nv-mmap.c` classifier | Cache type | x86 effective | Site |
|---|---|---|---|---|
| **BAR0 registers** | `IS_REG_OFFSET` | `NV_MEMORY_UNCACHED` + `TYPE_REGISTERS`, hardcoded | `UC-` | `:591-592` |
| **USERD / doorbell** (the `ud` sub-window of BAR1) | `IS_UD_OFFSET` | `NV_MEMORY_UNCACHED` + `TYPE_FRAMEBUFFER`, hardcoded | **`UC-`** | `:602-603` |
| **BAR1 aperture / framebuffer** (everything else) | `IS_FB_OFFSET` | `mmap_context->caching`, normally `WRITECOMBINED`; retried as `UNCACHED_WEAK` if WC unavailable | **WC** | `:611-621` |
| **peer IO** | `at->flags.peer_io` | hardcoded `NV_MEMORY_UNCACHED` | `UC-` | `:696-698`, `nv.c:3365-3371` |
| **system memory** (`/dev/nvidiactl`) | `NV_IS_CTL_DEVICE` | `at->cache_type` as RM requested — usually `CACHED` | WB | `:728-730` |

> **★ The counter-intuitive row is USERD/doorbell.** RM *allocates* USERD with a
> write-combining object attribute (`kernel_fifo_gm107.c:83-89`,
> `pUserdInfo->userdAttr = NV_MEMORY_WRITECOMBINED`) and then **maps it to userspace
> uncached**. The mapping is what governs the CPU. Anyone reasoning "a doorbell is a
> streaming write, so write-combining" gets this wrong, and this project's own design doc
> already had it right — `docs/design/mode2_compute_forwarding.md:1073`: *"GPU
> registers/USERD via BAR1 are **UC**"* — while the C's code did not implement it.

Two further facts worth carrying:

- **Userspace cannot choose the caching type through the RM ABI.**
  `src/nvidia/arch/nvalloc/unix/src/escape.c:594-595`:
  `// Don't allow userspace to override the caching type` — the flags are force-reset to
  `_DEFAULT`. So the attribute an isolate's mapping gets is decided by RM's own policy from
  the *object class*, not by anything we pass at map time. That is decider 4, and it is why
  our `CachePolicy` at the mapping site is a **declaration to be cross-checked**, not a request.
- **NVIDIA never leaves an attribute-aliased kernel linear map.** On x86 it re-types the
  pages with `set_memory_uc`/`set_pages_uc` on allocation (`nv-vm.c:1032-1035`) and restores
  write-back before free (`nv-vm.c:920-923`); on arm64, which has no `set_memory_*`, it marks
  the allocation `flags.aliased` and maps on demand (`nv.c:3310-3313` and three siblings).
  We never re-type pages, so we never create this hazard — but a future device backing must
  not assume the driver's page is safe to also map write-back elsewhere.

---

## 3. What the C's fix actually was — and which parts were bolt-ons

Four layers, in the order they were written. **Layers 2–4 are all after-the-fact corrections
to layer 1**, which is exactly the shape the owner flagged.

| # | Commit | What it did | Bolt-on? |
|---|---|---|---|
| 1 | `59bb98a` (initial) | **Blanket `pgprot_writecombine` for every device mmap.** | the original sin |
| 2 | `578662f` (#94) | Migrate-range VMA `pgprot_noncached` → `vm_get_page_prot()` (write-back). **One line.** DtoH 0.073 → 8.595 GB/s (118×); 7B decode 23.0 → 63.4 t/s (2.76×). | yes |
| 3 | `c5d5d8a` (#95) | Introduced the **`ctx->dev_id` branch**: `NVKVM_DEV_CTL`/`NVKVM_DEV_UVM` ⇒ write-back, everything else ⇒ write-combining. +17 lines. Empty `cuCtxSynchronize` 3.19 µs → 0.37 µs (host parity, was 8.9×). | yes |
| 4 | `d1247f7` (#111) | `nvkvm_force_range_wb()` — **a PGD→PTE walker that clears `_PAGE_PCD|_PAGE_PWT|_PAGE_PAT` on freshly-created leaf PTEs**, plus 2 call sites and 4 throwaway kernel modules in `tools/`. +86 lines. 0.12 → 14.5 GB/s. | yes, and the deepest one |

Layer 4 verbatim intent (`src/guest/nvkvm_mmap.c:44-66`) is quoted in §1.1. Its properties
are worth stating plainly because they are what "bolt-on" means concretely:

- it rewrites PTEs **behind `track_pfn_remap()`'s back**, so the kernel's `memtype`
  reservation for the range still says `UC-` while the PTEs say write-back —
  `/sys/kernel/debug/x86/pat_memtype_list` disagrees with reality (which is why
  `tests/mode2/cup6.c` exists to read it);
- there is **no matching fixup on teardown** and no `set_memory_*` anywhere in the tree;
- skipping the TLB flush is safe **only** under the stated precondition (*"PTEs that
  `remap_pfn_range()` has just created … before userspace can have touched — and thus
  TLB-cached — them"*), which is a comment, not a mechanism.

And the classification layer (3) is a **proxy**, not the fact: `dev_id` stands in for
"sysmem vs BAR", so a `/dev/nvidia0` sysmem mapping gets write-combining and a
`/dev/nvidiactl` BAR mapping would get write-back — both wrong, both silent. NVIDIA gets
three attributes out of one fd; a per-fd proxy cannot.

**The fix the C knew it needed and never built** is precisely this task. `docs/perf/
forwarding_latency_decomposition.md:359-362`:

> *"FIX (must be per-region — cannot blanket-WB): a WB mapping of the real doorbell BAR would
> leave the ring store in cache and never reach the device => decode HANG. So plumb a memtype
> (WB sysmem / WC BAR) from the host mmap classification to the guest, and have
> remap_pfn_range honor it (WB for sysmem, WC for BAR). Tracked #95."*

The wire protocol never gained the field: `src/common/nvkvm_proto.h:464-479`
(`struct nvkvm_req_mmap_on_isolate`) carries `prot`, `map_flags`, `session_id`, `reserved`
— and **no memtype**, in either request or response. Meanwhile
`src/qemu/nvkvm_mmap_host.c:14-29` documents a host-side memory-type classification and
`KVM_MEM_READONLY` usage **that does not exist in the file** — stale aspirational comments
describing the design that was filed and dropped.

### 3.1 Two live landmines found while reading (C repo, not ours to fix — flagged)

1. `src/guest/nvkvm_mmap.c:400` — the UVM "realize" path maps a **semaphore pool**
   `pgprot_writecombine` unconditionally, with no comment and no `nvkvm_force_range_wb()`.
   It still carries the pre-`c5d5d8a` blanket policy. Dead today (`:270-275` casts it away);
   wrong the day it is revived. This is the #95 bug, preserved in amber.
2. `src/guest/nvkvm_virtio.c:687` — `ioremap()` (⇒ `UC-`) over the virtio shm slot region,
   which is **real host RAM** (`src/qemu/virtio_nvgpu.c:1223-1226`,
   `memory_region_init_ram_ptr`). The ring next door uses `memremap(..., MEMREMAP_WB)`
   (`src/guest/nvkvm_main.c:512-514`). Never swept by the #94/#95/#111 series.

### 3.2 A documentation contradiction in the C repo, stated rather than smoothed

`docs/design/async_event_delivery.md:87-122` concludes #111 was **in the host EPT** (window
exposed as an MMIO BAR ⇒ KVM stamps `UC`), and proposes a QEMU-side fix, noting that guest
PTE write-back, guest PTE write-combining and a write-back MTRR were all measured as no-ops.
The fix that actually landed the same day (`d1247f7`) is **guest-side**, and the memory note
records the EPT conclusion as superseded.

Both are explicable, and the explanation is the most useful thing in this document:
**the "guest PTE write-back" experiment never produced a write-back PTE.** `remap_pfn_range`
downgraded it, so the experiment measured a `UC-` PTE twice and correctly concluded "the
guest PTE does not matter" from a premise that was false. Only `tools/pteinfo`, a throwaway
module that *decoded the live PTE*, settled it.

> **Requesting an attribute and observing no change is evidence about the request, not about
> the attribute.** Nothing in this space should be believed until the attribute has been read
> back — and from userspace it cannot be.

Recommendation for the C repo (not actioned here): `docs/design/async_event_delivery.md:87-122`
and `docs/PRE_PUBLIC_CHECKLIST.md:22,29,35-37` still carry the superseded EPT conclusion.

---

## 4. Is "prefer write-back, something below will downgrade it" confirmed?

**Half. And the wrong half is the dangerous one.**

**Confirmed** for anything backed by host RAM. Four independent measurements, one direction:

| Path | Before | After | Source |
|---|---|---|---|
| warm 16 MB pageable DtoH | 0.073 GB/s | 8.595 GB/s (118×, host parity) | `forwarding_latency_decomposition.md:258-285` |
| 7B decode | 23.0 t/s | 63.4 t/s (2.76×) | `:287-303` |
| empty `cuCtxSynchronize` | 3.19 µs | 0.37 µs (8.9× → parity) | `:335-373` |
| pinned CPU write / read | 0.22 / 0.12 GB/s | 14.50 / 13.99 GB/s | `async_event_delivery.md:87-122`, memory note |

**Not confirmed as a blanket**, and this is the correction: the C's *original* policy was
the mirror-image blanket (write-combining everywhere), and it was equally wrong. The reason
a write-back blanket nevertheless *appeared* safe is a guest-only accident —
`async_event_delivery.md:37-43`: mapping everything write-back in the guest did not break the
doorbell because **`kvm_is_mmio_pfn()` forces the EPT type uncached for real device-BAR pfns
regardless of what the guest asked**. The hypervisor was silently correcting the guest.

`kayfabe-linux-raw` runs in the **host** process. There is no backstop there. The safety
margin that made "prefer write-back" survivable in the guest **does not transfer to this
layer**, and a design rule derived from guest measurements would be derived from the wrong
system.

So the rule the API encodes is not "prefer write-back". It is:

> **The attribute belongs to the region, is decided where the region's class is known, and
> travels with it — never inferred at the mapping site from a proxy for the class.**

Write-back remains the right *answer* for every mapping this crate can make today, and that
too is a fact rather than a preference: page-cache memory has exactly one attribute.

---

## 5. What `kayfabe-linux-raw` does, and what it explicitly does not

**Does** (`crates/kayfabe-linux-raw/src/cache.rs`):
- `CachePolicy { WriteBack, WriteCombining, Uncached }` — three, matching the three the
  driver's `mmap` handler actually produces. No `WriteThrough` / `WriteProtected` (PAT can
  encode them; `nv_encode_caching` never selects them). No separate `UC-`: on x86 it is what
  `pgprot_noncached` already gives, and on arm64 it does not exist.
- Required at `MappedRegion::map`, `VolatileRegion::map`, `Reservation::map_fixed_in`.
  Not on `Reservation::new` — `PROT_NONE` address space is never accessed.
- No `Default`, never `Option`, not `#[non_exhaustive]`: a fourth attribute must be a
  compile error at every match site.
- `Backing::attainable_cache_policy()` — the one enforcement point. Both current backings
  are ordinary kernel pages ⇒ write-back, so anything else is
  `RawError::CachePolicyUnattainable`, **before** the syscall.
- `cache_policy()` on both region types, so the intended attribute survives into a bug report.

**Does not, and whose job it is:**

| Not enforceable here | Owner |
|---|---|
| the EPT/NPT type of a memslot | `kayfabe-vmm` (`map_guest`, `map_read_native`) — see §6 |
| the guest PTE | the guest kernel / the emulated device's BAR attributes. **#111 lived here.** |
| the `NV_MEMORY_*` attribute of the RM allocation | the isolate's forwarding path; RM force-resets client-supplied caching flags (`escape.c:594-595`), so this is RM's policy from the object class |
| whether the mapping *is* write-back | **nothing in userspace.** The PAT index lives in the PTE; the C shipped `tools/pteinfo` (a kernel module) to read it and `tools/fixwb` to change it. The only userspace-visible evidence is a bandwidth measurement (`tests/integration/pinned_write_bench.c`) — an L3 bench with a threshold, not a unit test. |
| the release fence before a doorbell store under write-combining | a future doorbell seam; named in `VolatileRegion::map`'s docs, deliberately not smeared into every access |

**Test honesty.** The suite proves: the refusal fires at every mapping door, for both
backings and both unattainable policies, by exact variant; the refusal consumes no
placement; a region reports its policy; every backing today attains write-back and only
write-back. It proves **nothing** about the actual PTE, and `mapping_unsafe.rs`'s test module
says so at the top rather than in a footnote.

---

## 6. Recommended follow-ups (not done here — different owners)

1. **`kayfabe-vmm::map_guest` / `map_read_native` should carry a guest-visible cacheability
   intent.** Decider 2 is the one that #111's *first* diagnosis blamed and the one that
   `kvm_is_mmio_pfn()` silently drives. Today the trait says nothing about it, so an adapter
   choosing "RAM memslot vs MMIO region" makes a cacheability decision with no vocabulary for
   it — which is precisely how the C's window ended up an MMIO BAR
   (`docs/design/gpa_window_pci_bar.md:100-111`, chosen for memslot-collision reasons, with no
   cacheability discussion in the document at all). It need not be `CachePolicy` — the honest
   shape is probably a two-valued "guest-RAM-like vs device-like" — but the *absence* should
   be a decision, not an oversight.
2. **The isolate's RM mapping path (M2-d) must cross-check.** When `Backing::DeviceFile`
   lands, the `CachePolicy` at the mapping site and the object class RM allocated must agree.
   Nothing can check that automatically; it is a two-call-site review, and §11's exit gate is
   where it belongs.
3. **arm64:** if it is ever built, `MappedRegion`'s bulk-copy accessors are **not** a legal
   surface over a Device-typed mapping (§1.2 consequence 2). That is a real constraint on the
   region-type/attribute pairing, and it is invisible from x86.

---

## 7. ★★★ "Map everything write-back in the VMM and let the host module impose UC where it needs it" — the owner's proposal, 2026-08-14, and the verdict

Recorded here in full so it is not re-proposed from scratch. The proposal, as put:

> *"Map everything write-back in the VMM, since the host NVIDIA module will already enforce
> uncached where it needs to and that wins; and the guest will set it correctly anyway since
> it thinks it is on real hardware."*

It has **two independent claims**, and they do not stand or fall together.

### 7.1 ★ Where it is RIGHT, and the code already agrees

- **For sysmem the host GPU reaches by DMA** — rings, pushbuffers, semaphores, notifiers,
  the RPC queue — **write-back is correct and it is the permissive choice.** On x86 PCIe DMA
  snoops, so a write-back mapping of a page the GPU also touches is coherent with no explicit
  flush (§1.1). The evidence in §4 is one-directional and overwhelming: 118×, 8.9×, ~100×,
  2.76×, all from moving a host-RAM-backed mapping *to* write-back. Every production
  `CachePolicy::WriteBack` site in `kayfabe-isolate-host` is of this class and every one of
  them is right.
- **The host NVIDIA module's own UC choice for a true register aperture should be inherited,
  not second-guessed** — and structurally it *cannot* be second-guessed, which is the
  stronger form of the same point: `mmap(2)` has no cacheability argument, so for a
  `Backing::DeviceFile` the attribute is whatever `nv_encode_caching()` put in
  `vma->vm_page_prot`, decided from the `NV_MEMORY_*` attribute fixed at allocation time.
  `Backing::attainable_cache_policy()` returns `None` for that backing precisely to say so.
  `rm.rs:2311-2320` states the same thing at the one call site where it bites:
  > *"a hardcoded write-combining is right for a framebuffer object and **wrong for the
  > doorbell**, which is a BAR0 register range NVIDIA maps uncached unconditionally"*

⇒ So for **our own host mappings** the proposal is very nearly a description of what the code
already does. That is not the half worth arguing about.

### 7.2 ⊘ Where it FAILS — and this is the load-bearing correction

The second claim — *"the guest will set it correctly since it thinks it is on real
hardware"* — is a claim about **decider 3**, and it silently assumes decider 3 is *consulted*.

**On Intel it is not.** For a normal-RAM-backed memslot KVM sets the EPT memory type with
**`IPAT`**, and `IPAT` means the guest PTE is **discarded, not combined**. On **AMD NPT there
is no `IPAT`** and the guest PTE is honoured (§1.1, quoting `src/guest/nvkvm_mmap.c:52-57`).

⇒ **"Map write-back and let the guest decide" becomes "map write-back and impose write-back"
on half the fleet.** The two readings are indistinguishable from the host, produce identical
logs, and differ only in whether a guest that asks for uncached gets it.

★★ **And that exact asymmetry already cost this project a full false green.** #111 was a
guest PTE silently downgraded to `UC-`; on the Intel bench `IPAT` overrode it and everything
passed, and on an AMD host the `UC-` stuck and the same code ran at **0.12 GB/s against 14
GB/s** — correct results, one to two orders of magnitude slower, nothing logged. A rule
justified by *"the guest will set it correctly"* is a rule whose failure mode is invisible on
the machine most likely to be used for the test.

> ### ⚠ AND A CORRECTION TO THE CORRECTION — the split may be KERNEL-VERSION dependent, not
> ### purely vendor dependent, and this document does not know
>
> §1.1's Intel/AMD statement is sourced from a **2026-era C-repo comment**, not from a reading
> of the KVM in the bench's kernel. Upstream KVM has changed when it sets `IPAT` for normal
> RAM (the self-snoop / "honor guest PAT" work), so *"Intel ⇒ `IPAT` ⇒ the guest is
> overridden"* is **believed here and not verified on this fleet**.
>
> ⊘ Do not treat it as measured. ★ **A ruling's DATE and its ARCHITECTURE are both part of the
> citation**, and this one has neither pinned. §8's **arm 2** exists to settle it *by
> measurement, per host, at run time* rather than by vendor lookup — and until it has run on
> an Intel bench, the honest statement is: **the guest's choice may or may not be consulted,
> we do not know which on any given host, and the design must not depend on the answer.**

### 7.3 ⚠ arm64 — the proposal is an x86-only bet, and our portability commitment forbids leaving that implicit

Everything in §7.1 rests on x86 facts. On arm64:

- **Normal vs Device is a difference in kind, not degree** (§1.2). `Device-nGnRE` forbids
  unaligned and multi-register access *architecturally* — a bulk copy that is merely slow on
  x86 can **fault** there.
- **Mismatched aliases are UNPREDICTABLE**, not merely slow. NVIDIA does not create them: on
  x86 it re-types pages with `set_memory_uc`/`set_pages_uc` and restores write-back before
  free; on arm64, which has no `set_memory_*`, it marks the allocation `flags.aliased` and
  maps on demand (§2).
- **DMA coherence is not guaranteed.** The snoop argument in §7.1 holds only on an IO-coherent
  chipset; NVIDIA carries an explicit `flush_cache_all()` path for the rest, compiled for
  aarch64 only.

⇒ A blanket write-back policy is **an x86 bet**. It may still be the right bet — we ship x86
first — but it must be *recorded as a bet*, because `support_matrix_asymmetry` commits us to
Turing+ across architectures and the cost of discovering this at arm64 bring-up is a redesign
of the mapping API, not a patch.

### 7.4 ⚠ Mode 1 — the seam, written down before it is rediscovered

The owner's own caveat, and it deserves more than a footnote. In **Mode 1** the guest runs a
paravirt module **we wrote**. So *"the guest will set it correctly since it thinks it is on
real hardware"* has no subject: the guest does not think anything we did not tell it, and the
attribute is **ours to choose and ours to get wrong**.

⇒ Record this as a **seam**, not a caveat: the Mode-1 guest module is a *fifth* decider in
practice even though it is architecturally decider 3, because it is inside our trust boundary
and inside our source tree. Any rule of the form *"the guest handles it"* is Mode-2-only and
must say so at the point it is stated.

### 7.5 Is a blanket write-back a SECURITY issue? — ⊘ UNVERIFIED ASSUMPTION, recorded as one

The owner asked directly. **My reading is that it is not a confidentiality or integrity
issue**, and the argument is short: *a caching attribute grants no access*. A write-back alias
of a page the guest already owns adds no reach — the guest could already read and write those
bytes; it now does so through a cache. There is no new object, no new mapping, no new
capability.

⊘ **But that is a reading, not a measurement, and it is stated here as an assumption so that
nobody later cites this document as having settled it.** Three specific ways it could be
wrong, each with the falsifier that would settle it:

| the worry | why it is not obviously fine | what would settle it |
|---|---|---|
| **Mismatched-alias behaviour** — the same physical page mapped write-back by us and uncached by someone else | Intel's SDM warns that aliasing a page with different memory types *"may lead to undefined operations that can result in a system failure"*. On **arm64 it is architecturally UNPREDICTABLE** (§1.2). A guest-reachable path to a host machine check is an **availability** failure, and `hostile_guest_isolation_is_the_value_proposition` puts host availability squarely inside the threat model. | An enumeration of every page this design maps twice, with the two attributes named. Today the answer is believed to be "none" (§2: *"we never re-type pages, so we never create this hazard"*), but that is a claim about the **current** backings and it has no test. |
| **Coherence, not caching** — a write-back host mapping of a page a non-IO-coherent GPU DMAs into | The host then parses **stale bytes** out of a page an attacker controls the timing of. That is an integrity question about our own parsing, not about the guest's access. x86 snooping makes it moot; arm64 does not. | An arm64 bring-up measurement, or an explicit refusal to support non-IO-coherent arm64. |
| **Timing observability** — a cached alias is a faster and more stable side channel than an uncached one | Marginal: the guest already has cached mappings of its own RAM. But *"marginal"* is a judgement and the threat model does not currently contain a cache-side-channel section at all. | A decision in `SECURITY_MODEL.md` about whether cross-guest cache side channels are in scope. They are almost certainly out of scope for v1, but that should be **written**, not assumed. |

⇒ **Status: my reading is "no C/I issue; an availability question that is unmeasured on both
architectures".** Do not launder it into a settled fact, and do not cite §7.5 as clearance.

---

## 8. ★★★★★ The guest-side instrument (w312) — closing the gap §1.1 names

§1.1 and `memtype.rs`'s header both end at the same wall:

> *"Nothing in userspace can read a guest's effective type. A consumer that needs that answer
> must measure it **in the guest**."*

- **`scripts/bench/memtype_probe.c`** — the probe. One C file, no dependencies, compiled
  **inside** the guest (`gcc`), the same delivery pattern as `cup3.c` and
  `e2_doorbell_poke.c`. ⊘ Deliberately not Rust: the bench guest has no toolchain, and a
  musl-static cross build would put a build system between the question and the answer.
- **`scripts/bench/memtype_probe_hook.sh`** — the `POST_CAPTURE_HOOK` arm, with the three
  pre-registered readings in its header.

### 8.1 ★★★ The finding that shaped it: in the guest the three instruments are NOT co-equal

`memtype.rs` has three instruments and says they are *"deliberately three, because each one
alone has a way of being green while wrong"*. **That framing does not port unchanged.**

- `/proc/iomem` and `pat_memtype_list`, read **inside the guest**, observe **decider 3 only** —
  the guest kernel's own request and its own bookkeeping. They are structurally blind to
  deciders 1 and 2 exactly as the host module is blind to 2 and 3. ⊘ **Two blind instruments
  do not add up to sight.**
- **The timing witness is the only instrument that observes the COMBINATION.** The CPU
  resolves all three deciders in hardware; a load's latency *is* the resolution.

⇒ The categorical half is not there to **corroborate** the timing half. It is there to
**attribute** it. The timing says *what the type is*; the disagreement between the two says
*which decider produced it*. **The pair is the measurement; neither alone is.**

| guest record (iomem / PAT) | timed verdict | reading |
|---|---|---|
| `System RAM` / write-back | cached | consistent — nothing overrode the guest |
| `System RAM` / write-back | uncached-class | ★★★ decider 1 or 2 forced UC. The guest believes it holds cached RAM and every access is a bus transaction. **A real problem** — this is the 0.12 GB/s shape. |
| uncached / uncached-minus | cached | ★★★ the guest asked for uncached and did not get it. **Decider 2 DISCARDED the guest's choice** — on x86 that is `IPAT`. A register poll here can hang. **This is §7.2, measured.** |
| uncached / uncached-minus | uncached-class | consistent — the guest's choice was honoured |

★★ Rows 3 and 4 are the whole point of the exercise. *"Let the guest decide"* is a proposal
about decider 3; **whether decider 3 is consulted at all** is what this table answers, per
host, at run time, instead of per vendor from a comment written in another repo.

### 8.2 ★★★★★ What it measured, and the correction it forces on `memtype.rs`'s own constants

Run 2026-08-14 on the **local development box**, which is itself a **KVM guest on an AMD EPYC
7543** — i.e. the "guest PTE is honoured" side of §7.2. Not the GA106 bench; no GPU involved.
Verbatim:

```
region                 role                       guest record          timed                   ratio
anon-wb                control:wb                 untracked/System RAM  cached                  1.0x
control-mismatch       control:footprint-mismatch untracked/System RAM  inconclusive(in-band)   8.1x
devmem-nonram          known-positive             uncached-minus        uncached-class        129.1x
★ MEMTYPE PROBE regions=3 subjects=0 known_positive=FIRED controls_ok=categorical-available
```

**The known-positive fired, watched: 129.1× and `uncached-class`, on a region whose only
difference from `anon-wb` is the mapping attribute** — same DRAM, same footprint, same
stride, `/dev/mem` at an `ACPI Non-volatile Storage` range that x86 maps `UC-`. The probe
reported two different answers, which is the whole of what a known-positive is for.

★ **A second subject, and it is the one this project should look at twice.** Pointing the
probe at an **emulated PCI BAR** in the same guest (`--pci 0000:00:03.0:1`, a virtio device):

```
pci:0000:00:03.0:bar1  subject  uncached-minus  uncached-class  8535.6x  0xc0082000
```

**8 535×** — two orders of magnitude beyond a real `UC` aperture, because every load is a VM
exit into the VMM rather than a bus cycle. ⚠ That is the cost class of **our own emulated
GPU's BAR0**, per 32-bit register read, and it is a number worth having in front of anyone
designing a guest-side polling loop against `nvkvm_gpu_emul.c`. ⊘ It is *not* a cacheability
finding — the record and the verdict agree, nothing is overridden, the guest asked for
uncached and got it. It is a **trap-cost** finding that this instrument happens to be able to
see, and it is recorded here rather than in a design doc only because this is where it was
measured.

> ### ★★★★★ AND THE THIRD ROW IS A CORRECTION TO `memtype.rs`
>
> `control-mismatch` is **ordinary write-back RAM** — 256 MiB of anonymous memory, one load
> per page — judged against a 4 KiB reference. It measured **8.1×** (and **9.1×** on a second
> run under a different uid). `memtype.rs` documents `Inconclusive(InTheBand)` as existing
> because **a real device aperture came in at 9.1×** (task #150, 2026-08-01).
>
> ⇒ **Ordinary DRAM and a real uncached aperture produce the SAME ratio.** The band is not a
> region of low confidence that a better floor would shrink — it is a region where two
> genuinely different memory types **overlap**, and no choice of `UNCACHED_RATIO_FLOOR` can
> separate them.
>
> ⇒ ⊘ **`UNCACHED_RATIO_FLOOR`, `CACHED_RATIO_CEILING` and `REFERENCE_SPREAD_CEILING` are
> documented in `memtype.rs` as properties of the MEMORY TYPE. They are properties of the
> COMPARISON**, and they hold only when the subject and the reference have the **same
> footprint and the same stride**. `tests/effective_memtype.rs` happens to satisfy that (its
> control and its BAR are the same shape), so the constants have never been wrong in practice —
> but nothing states the requirement, and the next caller to time a large region against a
> small reference gets a confident `UncachedClass` for write-back memory.
>
> ⇒ `memtype_probe.c` therefore measures a **fresh reference at the subject's own footprint**
> for every region, and runs the mismatched comparison as a **standing control** so the
> failure mode is exhibited on every run rather than trusted not to occur.

### 8.3 ⊘ Two more things the probe found — both by refusing itself

- **★★★★★ The first "known-positive" was not one, and the VOID gate caught it.** The probe
  originally opened `/dev/mem` `O_RDONLY` and asserted, from `/proc/iomem` not calling the
  range `System RAM`, that the mapping was `UC-` *by construction*. **It was write-back**, and
  timed at **1.0×**. `drivers/char/mem.c` decides with `uncached_access()`, whose entire x86
  test is `O_DSYNC` set, or the address above `high_memory` — the `/proc/iomem` label is not
  consulted at all. ⇒ **A "known" positive that was reasoned rather than watched is a
  hypothesis.** The probe exits **2 = VOID** when nothing fired, so the run was refused rather
  than reported as three tidy write-back readings.
- **The PAT list must be read WHILE THE MAPPING IS LIVE.** It records *reservations*, and the
  reservation being asked about is the one the probe itself just created. A copy slurped at
  startup answered `untracked` for the known-positive **while it was timing at 129×** — and
  `untracked` reads as *"ordinary memory, so write-back"*, the permissive answer, for exactly
  the region the probe exists to look at.

### 8.4 The pre-registered bench arms — what a sibling should run, and what would mean trouble

Run: `POST_CAPTURE_HOOK=scripts/bench/memtype_probe_hook.sh scripts/bench/boot_capture.sh <tag>`
(`GQ_TIMEOUT >= 120`). It needs no GPU state, disturbs nothing, and is safe beside another arm.

| arm | expected on the GA106 bench guest | what a different reading means |
|---|---|---|
| **1. controls** | `anon-wb` → `cached` ~1×; `devmem-nonram` → `uncached-class` ≥ 10×; `control-mismatch` → **not** `cached`; `known_positive=FIRED` | ⊘ `known_positive` not `FIRED` ⇒ **the run is VOID**, not green-with-a-caveat. ★★ `anon-wb` anything but `cached` is the **loud** result: guest RAM is where every ring, pushbuffer and semaphore we place lives, and `uncached-class` there is the 0.12-GB/s-with-correct-results shape. `control-mismatch` coming back `cached` would mean §8.2's correction has stopped being load-bearing. |
| **2. the vendor question (§7.2)** | **no row** saying *"THE GUEST ASKED FOR UNCACHED AND GOT CACHED"* — the bench host is AMD, so the guest's choice should be honoured everywhere | ★★★ Such a row **on AMD** falsifies §1.1 for this fleet and voids every *"let the guest decide"* argument here. Running the same arm on an **Intel** host is how §7.2's ⚠ correction gets settled; a *present* row there confirms `IPAT` is live on that kernel, an *absent* one confirms it is not. |
| **3. the NVIDIA BARs** (`KAYFABE_MEMTYPE_NVIDIA=1`) | the `mmap` arms report `EBUSY` while the module is loaded — a **refusal**, not an answer. The **categorical** half still answers and needs no `mmap`. | ★★★ **If the guest's PAT list records `uncached`/`uncached-minus` for the NVIDIA BAR0 range, the declared requirement at `rm.rs:1520` and `rm.rs:1731` is false** — see §8.5. |

⊘ Arm 3's timing half needs the NVIDIA module unloaded (`e2_doorbell_witness.sh` does this).
The hook deliberately does **not** unload it: doing so mid-capture changes what every other
arm is measuring.

### 8.5 ★★★ A contradiction this rung found and did NOT fix

Two call sites declare `CachePolicy::WriteBack` while their own rustdoc argues **uncached**:

- `crates/kayfabe-isolate-host/src/rm.rs:1500-1520` — `open_usermode`, the **BAR0 window the
  doorbell store lands in**. The bullet is headed *"★★ `CachePolicy::WriteBack`, not
  write-combining"* and its body then cites
  `nv_encode_caching(…, NV_MEMORY_UNCACHED, NV_MEMORY_TYPE_REGISTERS)` — i.e. the evidence
  says **uncached** and the conclusion says write-back. The heading rules out the wrong
  alternative (write-combining) and then picks the other wrong one.
- `crates/kayfabe-isolate-host/src/rm.rs:1708-1733` — **`map_object_uncached`**, whose first
  doc line is *"CPU-map an already-allocated object, **uncached**"* and whose body passes
  `CachePolicy::WriteBack`. ★ **The name is false**, which is the failure class
  `refuse_by_name_means_the_name_is_true` was banked for.

**Scope, honestly:** this is **not** a live runtime bug. `mmap(2)` cannot install a cache
attribute, so for a `Backing::DeviceFile` the policy argument is **purely declarative** — the
real attribute comes from `nv_encode_caching()` either way, and the doorbell path works
because the declaration is inert. It is an **oracle** bug: `require_attainable` cannot refuse
it (`DeviceFile` ⇒ `attainable == None`, and `an_unadjudicable_backing_accepts_every_policy`
is that behaviour under test), and a future `memtype::require_effective` over that mapping
would report `Downgraded { requested: WriteBack, effective: Uncached }` **for a correct
mapping** — the check inverted.

⊘ Not changed here: this rung is measurement, and `docs/design/mode2_doorbell_mapping.md:180-183`
already rules that *"the doorbell call site must pass `Uncached`"*. The fix is a separate rung
with its own falsifier, and **arm 3 above is that falsifier**: measure what the guest kernel
records for the BAR0 range before editing the declaration to match a comment.

⚠ Related and worth one line: **no production call site anywhere in the workspace requests
`CachePolicy::Uncached`.** For a three-variant enum with no `Default`, whose whole purpose is
to force each site to state its requirement, one variant being unreachable in production is
either a fact about our mappings or a symptom of the above. It is currently unexamined.
