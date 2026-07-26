# Memory cacheability — the four deciders, the C's scars, and what `kayfabe-linux-raw` can and cannot enforce

> Status: **research + design record** for `CachePolicy` in `kayfabe-linux-raw`
> (`l1_os_shell.md` §4). Written before M2-c builds on the mapping API, because a cache
> attribute cannot be retrofitted into a mapping API — every call site would have to change.
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
