# Mode-2 address table — the one table of truth

Status: design, 2026-06-17 (user brainstorm + source verification against the
open kernel modules `research_clones/ogkm`, tag 580.159.04 / Ampere GA10x).
Supersedes the per-access "walk + heuristic cascade" in `nvkvm_chan_execute`.
Governs all Mode-2 data-plane address resolution; the #12 teardown hang is the
first bug it dissolves. See also `mode2_address_virtualization.md` (the two
translation chains and the GPU-physical page categories this builds on).

## 0. Core invariant (non-negotiable)

There is **one authoritative table** that resolves any guest GPU virtual address
to a GPU-physical (GPGA) range. It is:

- **Forward-populated only.** Data flows *in* at bind time and *out* at lookup.
  Never traced backwards from a VA at execution time.
- **Never resolved by walking the PDB at lookup.** Lookups are pure table reads.
- **No heuristic fallbacks.** Delete the `nvkvm_chan_execute` cascade
  (instblk → snooped `chan_vas[]` → `bar1_wpg` FB scan → "one VA aliases many GPAs"
  guessing). A lookup either hits the table or it is a **fault** (§6).

This is stricter than "a cache." The table *is* the truth for our resolution; the
guest's in-memory page tables (PDB) are treated as **a communication channel, not
storage** (§2).

## 1. Mental model: the table IS the GPU's TLB

The guest's contract with GPU hardware is: *write PTEs to memory → issue a TLB
invalidate → only then rely on the mapping.* A real TLB holds stale/absent
entries until an invalidate; it is refreshed from memory on invalidate (and, on
real HW, on a miss-walk — which we deliberately do NOT replicate, §6).

We make **our table play the role of that TLB.** The guest cannot distinguish
"real HW TLB, flushed on invalidate" from "our table, refreshed on invalidate":
the observable semantics (a write does not take effect until invalidate) are
identical. Consequences:

- Our table's "staleness" between invalidates is **not a bug** — it is
  architecturally identical to a real TLB. The guest's own invalidate discipline
  is exactly what keeps our table correct. We add no new requirement on the guest.
- We do **not** model the *host* GPU's TLB. We resolve guest-VA → host backing +
  host VA and keep the host VA space aligned; the host driver owns the host TLB.
  Two TLBs: the guest's (= our table) and the host's (= host driver's job).

## 2. Why the guest is the authoritative allocator (and PDB is communication)

Verified in ogkm:

- **PMA (vidmem heap allocator) is CPU-side**, even under GSP
  (`bPmaEnabled = NV_TRUE` set in the kernel RM, `mem_mgr.c`). The guest kernel RM
  picks vidmem physical offsets. GSP does **not** pick free GPU-phys for the heap.
- Every object carries a **memdesc**; `memdescGetPhysAddr(pMemDesc, AT_GPU, off)`
  is its authoritative GPU-physical address. The guest records the phys of
  everything it allocates.

So the binding (VA↔phys) is *decided by the guest* and *crosses our boundary* at
the bind-time RPC/ioctl (we are the device and the faked GSP). What GSP owns is
not the allocation but the **page-table management** for `bGspOwned` channels
(`kernel_channel.c: pKernelChannel->bGspOwned`) — which is exactly why their
CPU-side instance-block PDB reads empty and the old cascade failed on them.

→ We record the authoritative binding at bind time; we never need to reverse it.

## 3. Data structure

- **Keyed by VAS** (the PDB root / page-dir base), NOT a global VA space. The same
  GPU-VA legitimately maps to different GPAs in different VASes (the aliasing the
  cascade fell into — one VA → 4 GPAs across kernel VASes). Per-VAS keying makes
  RPC-populated and PDB-populated VASes disjoint, so the two sources never collide.
- Per VAS: a sorted set of `VA-range → { gpga_base, aperture, size }`
  (interval tree or sorted array + binary search; the existing "GPGA binary-search
  index" is precedent).
- The GPGA→host-object layer is the *second* table (already exists as
  `m2_fbback[]` / `m2_gpga`): an allocated GPGA range is an offset/slice into a
  real host GPU allocation (double-mmap). Lookup chains:
  `VA → (this table) → GPGA → (m2_fbback) → host slice`.
- **Locking:** one RW-lock per VAS (shard later if contention shows). Resolvers
  take read; populate/invalidate handlers take write. Soundness depends on §5.

## 4. Population — forward, from exactly these sources

> **Correction 2026-06-17 (ogkm-verified): §4.2 is the load-bearing source, not
> §4.1.** `FILL_PTE_MEM` is *not* the general map transport under GSP. The common
> map path is the CPU-side MMU walker `dmaUpdateVASpace` (`virt_mem_allocator_gm107.c`),
> which sets `bFillPteMem = flags & DMA_UPDATE_VASPACE_FLAGS_FILL_PTE_MEM` (bit 25)
> and **writes PTEs directly into the page-table memory CPU-side** — there is no
> per-map GSP-RPC carrying VA↔phys for it. The explicit `NV0080_CTRL_CMD_DMA_FILL_PTE_MEM`
> control (0x801802) exists but is not the path UVM/CeUtils maps take. So the table
> must be populated predominantly via **§4.2 (read the page tables at invalidate)**;
> §4.1 covers only the cases that genuinely cross as an RPC (PROMOTE_CTX, channel/
> object create). This makes §4.2 + §5 the spine of the design, not a fallback.

1. **Direct RPC/ioctl bindings** — `GPU_PROMOTE_CTX` (0x2080012b, already captured
   into `va_map[]`), channel/object create, and the rare explicit
   `NV0080_CTRL_CMD_DMA_FILL_PTE_MEM` (0x801802). These hand us the binding
   directly across the GSP boundary; record it.
2. **Invalidate → read-the-page-tables (LOAD-BEARING)** — for every VAS whose PTEs
   the guest writes CPU-side via the walker (the general case: UVM, CeUtils, device-
   default channels), on the invalidate event (§5) read that PDB's page tables from
   the FB/sysmem memslot and diff the changed ranges into the table.

PDB pages may live in a **fast RAM memslot — we do NOT trap individual PTE
writes.** We read the PDB only at the invalidate commit point (§5), which is the
only point the guest guarantees a consistent, committed view.

**Caveat surfaced 2026-06-17 (must hold for §4.2 to work):** the CPU-written PTEs
must actually land in a memslot we can read. The overnight trace shows this is
*partially* true today — e.g. gpfifo VA `0x120064000` walks cleanly to sysmem phys
`0x165664000` under PDB `0x3114000`/`0x3400000` — but the same PDB **faults** for
other VAs/contexts (e.g. `0x121010000` under client `0xc1d00001`). So our page-walk
/ FB-mirror has gaps that must be closed (or the walk made reliable) before "read at
invalidate" is trustworthy for *all* VAs. This is the #11-open item in §11.

## 5. The coherence event: TLB invalidate (two transports, both observed)

> ### ⊘⊘⊘ CORRECTED 2026-08-14 (w324) — **"TWO TRANSPORTS" IS WRONG, AND SO IS THE ZERO BELOW.
> ### ON GA106 THERE IS A THIRD, IT IS THE ONE RM ACTUALLY USES, AND WE HAVE NEVER DECODED IT.**
> Full derivation with quotes: **`guest_invalidate_discipline_and_the_publish_boundary.md`**.
> Headlines, each cited there:
> - ★★★★★ **RM's invalidate is a BAR0 MMIO register write** —
>   `GPU_VREG_WR32(pGpu, NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE, …)`
>   (`ogkm: kern_gmmu_tu102.c:117`) at **`0xB80000 + 0x30B0 = 0xB830B0`**. Not an RPC, not a
>   pushbuffer method. `grep -rn MMU_INVALIDATE src/qemu/ ../nvkvm-rs/crates/` = **zero hits in
>   both trees** — the signal has been arriving at our BAR0 handler all along.
> - ★★★★★ **The `INVALIDATE_TLB` RPC zero below is a known-negative, not a measurement.**
>   `rpcInvalidateTlb` is **`_STUB` on GA106** (`ogkm: g_rpc_private.h:320`) and is reached only
>   under vGPU with `VF_INVALIDATE_TLB_TRAP_ENABLED` (`kern_gmmu_gm107.c:151-159`). **It could
>   never have been non-zero.** ⇒ *a census over a path that cannot execute*.
> - ★★★★★ **MAP INVALIDATES TOO.** The GMMU negative-caches — *"the GPU TLBs may cache invalid
>   entries using any page size they decide"* (`ogkm: uvm_mmu.c:1534-1536`). Map and unmap are
>   symmetric in *whether* they invalidate; they differ only in the **membar**.
> - ⊘ **And it is CLIENT-SUPPRESSIBLE**: `NVOS46/47_FLAGS_DEFER_TLB_INVALIDATION`
>   (`ogkm: nvos.h:2150`, `:2191`) — *"can leave stale entries in the TLB, and allow access to
>   memory no longer owned by the RM client"* (`nvos.h:2146-2148`). ⇒ **an invalidate-triggered
>   UNPUBLISH is not safe against a hostile guest**; observe the write instead.
>
> ★ **WHAT SURVIVES BELOW, exactly:** the `MEM_OP` transport is real and is UVM's; the CE-write
> capture feed is real and is source (2). What must **not** be repeated is *"the guest issues no
> invalidate on the compute path"* — that reads the two-transport zero as a claim about all
> transports, and it is false.

> **★ CORRECTION (2026-07-22, audit S3 — #14 round-6, decisive).** The invalidate model in this
> section governs the **kernel/UVM/RM paths** (where the two transports DO appear). It does **NOT**
> fire on the **Mode-2 GSP-emulated compute path**: `mode2_14_concurrent_apps` round-6 measured
> **both** transports = **0 occurrences** there (`INVALIDATE_TLB` RPC fn=200 = 0; `MEM_OP`/
> `MMU_TLB_INVALIDATE` pushbuffer method = 0), *and* `DMA_FILL_PTE_MEM` (§4.1) = 0. On that path the
> compute working-set's leaf PTEs are published **exclusively through the CE page-table-write data
> plane** (kernel-RM CeUtils physical CE copies into PD pages — the #13 mechanism, commit `b83d0b4`).
> So the address table has **two co-equal populate sources**, not "RPC + read-at-invalidate":
> **(1)** bind-time RPC/ioctl bindings (§4.1); **(2)** the **observed CE-PT-write**, attributed by
> its destination-FB-address → owning PDB, latched and decoded **at the CE release semaphore** (the
> commit point that replaces the absent invalidate). This is the rewrite's `nvkvm-mmu` "CE-write
> capture feed," equal to the RPC source (`mode2_rust_rewrite_architecture.md` L3). §4.2's
> "read-at-invalidate is load-bearing" and §11's "always-invalidate is universal" (open item #1)
> are therefore **false for the GSP-emulated compute path** — resolved to the CE-write-hook there.

Both carry the **PDB address** (so we know which VAS to refresh) and **membar**
bits (the fence, §5.1). Both are observable to us.

- **RM / GSP-managed VASes → `INVALIDATE_TLB` RPC.**
  `NV_VGPU_MSG_FUNCTION_INVALIDATE_TLB`, `rpcInvalidateTlb_v23_03(pdbAddress, regVal)`
  (`vgpu/rpc.c`; issued via `NV_RM_RPC_INVALIDATE_TLB`, `kern_gmmu_gm107.c`). In GSP
  mode the privileged MMU register is owned by GSP, so the invalidate is RPC'd —
  it lands in our GSP-RPC queue (already decoded; today likely acked blindly).
- **UVM / privileged kernel channels → `MEM_OP` pushbuffer method.**
  `uvm_hal_ampere_host_tlb_invalidate_all(push, pdb, depth, membar)`
  (`uvm_ampere_host.c`) emits, on class C56F (AMPERE_CHANNEL_GPFIFO_A):
  ```
  NV_PUSH_4U(C56F, MEM_OP_A, sysmembar_value | INVAL_SCOPE=NON_LINK_TLBS,
                   MEM_OP_C, TLB_INVALIDATE_PDB=ONE | PDB_ADDR_LO(pdb_lo)
                            | PAGE_TABLE_LEVEL(level) | aperture | ack,
                   MEM_OP_D, OPERATION=MMU_TLB_INVALIDATE (0x9) | PDB_ADDR_HI(pdb_hi));
  ```
  We see this **in the pushbuffer parser** when draining the channel — the *same*
  parser the #12 fix needs (§7). Decode `MEM_OP_D.OPERATION == MMU_TLB_INVALIDATE`
  (0x9) / `..._TARGETED` (0xa); reconstruct `pdb` from `PDB_ADDR_LO|HI`; read
  `SYSMEMBAR` / `ACK_TYPE=GLOBALLY` for the fence.

On either event: take the VAS write-lock, refresh/diff that PDB's bindings into
the table, apply MMIO changes per §5.1–5.2, release.

### 5.1 Membar = hard barrier (the in-flight-DMA fence)

When the invalidate carries a membar/sysmembar bit, it is a **serialization
point**. In our managed pushbuffer interpreter we do **not advance to the next
method** until: (a) the table refresh for that PDB is applied, AND (b) the fenced
outstanding host-side work has drained. That is the literal meaning of the bit;
honoring it is how "atomic end-state after invalidate" is actually achieved.

### 5.2 MMIO materialization: unmap eager, map lazy, reclaim deferred

- **Unmap is eager (correctness + security).** A removed or re-pointed range must
  have its stale host backing dropped *before* the guest can reach it — else its
  next DMA hits stale memory (a cross-context leak). Bounded to the ranges the
  invalidate touched.
- **Map is lazy (perf).** Materialize new host mmaps on first touch, re-checking
  the table. (Eager-map is an option if invalidates prove rare/scoped and you want
  "after sync, MMIO == table, nothing deferred" — measure first; UVM migration can
  storm invalidates.)
- **Deferred reclamation for the fence.** Refcount the backing. Invalidate marks
  the old backing for-delete, drops its ref, and returns *without blocking*; the
  in-flight forwarded op, on completion, sees the flag and does the munmap. The
  *table* update is synchronous (new binding visible immediately); only the *old
  backing's reclamation* is deferred. While marked-for-delete, new accesses see
  the new binding (or fault), never the stale backing.

## 6. Miss handling — a miss is a fault, never a walk, never a guess

> ### ★★★★★ OWNER RULING 2026-08-12 — MIRROR THE WHOLE VAS. §6 stands; its SCOPE narrows.
>
> **What changed.** The Rust port (`kayfabe`, w276) now runs the C's whole-VAS sweep
> (`enum_gr_sysmem`, `C: nvkvm_gpu_emul.c:583-591`) at the doorbell: a walk from each address
> space's **own installed page-directory root**, whose every reached page is admitted and whose
> leaves are forward-populated. The owner was shown the objection below **in full** and ruled for
> the whole-VAS port anyway — this is *"port the C, don't redesign"* applied to the completeness
> invariant.
>
> **⊘ What did NOT change, and must not be misread.** §6 below is about **`resolve`**, and it is
> untouched: a lookup that misses is still a fault, still never walked, still never guessed. The
> sweep is a **populate source** (§4), not a resolver. It runs **before** the consumer that reads
> the table, never as a fallback **after** a miss. ⇒ *"we now walk on a miss"* is false; the port
> has no such path and adding one is still refused.
>
> **★ Why the timing hazard below does not apply to the submission's own set.** A **doorbell is
> the guest's own commit point** for the work it is submitting. The guest cannot be mid-update on
> a VA that this submission will touch without racing its own GPU. So for that reachable set, the
> *"uncommitted, possibly mid-update"* state §6 refuses to read is not the state we read.
>
> **⚠ THE ACCEPTED RESIDUAL, stated rather than papered over.** The argument above covers the
> submission's own set and **not the rest of the address space**. A whole-VAS walk also reads
> regions another guest thread may be rewriting right now, and there §6's hazard is intact: a torn
> multi-level walk can resolve to the wrong physical page. **This is a knowingly accepted risk, not
> a refuted one.** Two things bound it and neither is a proof of absence:
> - ★★ **The dirty-driven re-sweep, which is why the sweep is HALF a design.** A page that was
>   mid-update when it was swept was **by definition being written**, so it lands in the dirty set,
>   so the next doorbell re-sweeps it. The torn window is **bounded and self-healing**. ⇒ **A
>   one-shot sweep without dirty-driven re-sweep does NOT carry this argument.** Build both halves
>   or neither. (`kayfabe_fwd::plan_pt_sweep` triggers = never-swept / truncated / dirty — the C's
>   `chan_vas_n` / `m2_gr_pt_trunc` / `m2_gr_vas_dirty`.)
> - The window is a wrong *guest-owned* mapping inside **the guest's own address space**, not a
>   cross-VM one: every page still passes the aperture checks, the walk is depth- and
>   budget-bounded, an unreadable page is still a loud fault and never zeros, and a truncated walk
>   contributes **no** leaves rather than partial ones.
>
> **⊘ The residual is real and is not zero.** It is a fidelity/consistency risk taken to reach the
> C's completeness. Anyone re-opening this must re-open it as *"is the self-healing bound good
> enough"*, not as *"§6 was wrong."*
>
> ★ **Scope of the relaxation, exactly as implemented** (`kayfabe_mmu::reach::ReachShadow::witness_swept`):
> a page is admitted **iff** a descent starting at that address space's own installed PDB reached
> it. It is *not* "read whatever the guest points at" — that is the `cap2b` class this project
> keeps as a fixture. The cost is hole 2's guarantee: residue reachable from the root can now bind,
> where before it could only ever make an unwitnessed page reachable. `swept_binds` reports how
> much of the published set exists **only** because of this.
>
> ⚠ **A ruling's date is part of its citation.** This one is 2026-08-12 and its architecture is a
> doorbell-driven populate pass with a dirty-driven re-sweep. If either half goes, re-ask.

A lookup that finds no binding means the guest never committed (invalidated) that
VA → it is not relying on it yet → resolving it would mean reading **uncommitted,
possibly mid-update** page-table state. That is a security hole (torn multi-level
walk → wrong physical page → cross-context leak), not a recoverable case.

Therefore: **miss = a real GPU page fault**, surfaced loud and forwarded to the
guest as a fault (which is exactly what real HW does for a genuinely unmapped VA).

- We explicitly do **NOT** do an opportunistic "walk the PDB one last time" on a
  miss. Real HW's miss-walk is safe only because the driver never *acts* on an
  uncommitted VA — which, for us, means it is already in the table (no miss). A
  miss is, by definition, the unsafe-to-walk state.
- A miss is also the signal that we failed to capture a binding at its populate
  site (§4) — fix it there, not with a fallback.

## 7. Relationship to #12 (same plumbing)

The #12 teardown hang (`mode2_baremetal_32.md`) is: a `bGspOwned` CE scrub channel
(gpfifo 0x120064000, `picked_pdb=0`) whose finishPayload sema (vidmem 0x12006c004)
we never wrote, because we could not resolve its pushbuffer VAS. Under this design:
the channel-buffer binding is recorded at channel-create (§4.1); the sema resolves
by table hit; no PDB, no cascade, no special case. And UVM's invalidate being a
`MEM_OP` pushbuffer method (§5) means the table's invalidation hook and #12's
`SET_SEMAPHORE` parsing are the **same** channel-pushbuffer decoder. Build the
decoder once; both are served.

## 8. Allocation & host OOM

- **When backing is allocated:** at the guest's `RM_ALLOC` (vidmem) — forward to
  the host GPU there and record the GPGA→host-slice in `m2_fbback`. Distinct from
  *binding* (§4), which happens later at map/invalidate.
- **OOM reporting:** the host alloc must be **reserved at the alloc RPC** so its
  failure returns synchronously as `NV_ERR_NO_MEMORY` (`heap.c`) → guest RM →
  `cudaErrorMemoryAllocation`. Lazy "promote-on-touch" cannot report OOM cleanly
  (no RPC in flight at touch; only recourse is a fatal Xid). Compromise:
  reserve/account host capacity at alloc (cheap, synchronous error), materialize
  pages lazily, treat a post-reservation OOM as the genuinely-fatal exception.

## 9. Security properties (why determinism beats opportunism)

- No torn reads: the PDB is read only at the invalidate commit point.
- No stale backing reachable: unmap is eager for changed ranges (§5.2).
- No guessing: a miss is a fault, not a heuristic resolution to *some* page (§6).
- Per-VAS keying prevents cross-VAS aliasing from resolving one client's VA to
  another's backing.

## 10. Userspace rings stay opaque (verified safe)

Opaque passthrough of userspace (libcuda) gpfifo rings does **not** endanger the
table:

- Userspace channels are **non-privileged** (`uvm_channel_is_privileged` is the
  privilege gate; user channels lack it) → cannot issue `MMU_TLB_INVALIDATE`.
- Userspace memory maps are **kernel-mediated** (`DMA_FILL_PTE_MEM` ioctl, RM
  control) → observed at §4.1, not in the user ring.
- User rings *may* carry `MEM_OP` **membars** (data ordering) — irrelevant to the
  table; only `OPERATION=MMU_TLB_INVALIDATE` matters, and that is privileged.

So the table changes only via kernel-observable events; the `m2opaque` fast path
remains sound.

## 11. Open items to verify before/while implementing

1. **Always-invalidate is universal.** Strongly evidenced (UVM pushes the
   invalidate inline after PTE writes; RM carries it via RPC), but not proven for
   every map path. The §6 "miss = fault" rule makes a missed-invalidate a loud,
   safe failure rather than silent corruption — but a *legitimate* path that
   maps-and-uses without invalidate would then false-fault. Confirm none exists.
2. **Ampere routes `INVALIDATE_TLB` via RPC in GSP mode** (saw Maxwell HAL +
   generic RPC; Ampere HAL almost certainly the same — verify the GA10x HAL).
3. **HW rejects `MMU_TLB_INVALIDATE` from a non-privileged channel** (strongly
   implied by the privilege model; not proven by a HW privilege-check line).
4. **UVM PTE-write transport** (CPU vs CE inline) — not load-bearing for the
   design (we read at invalidate regardless), but informs whether PDB reads ever
   touch sysmem vs vidmem.

## 12. Migration / refactor note

Implementing this means **deleting** `nvkvm_chan_execute`'s PDB-resolution cascade
and the per-doorbell re-sweep, replacing exec-time resolution with a table lookup.
Populate from §4 sites; invalidate from §5; fault on miss. This is also the clean
shape for the Rust rewrite ([[rewrite_horizon_target]]): an owned
`HashMap<PdbRoot, IntervalMap<VaRange, Binding>>` behind a lock, two populate
entry points, one lookup, no heuristics.

## 13. VAS identity = PDB, never the client handle (the #12 lesson)

The table is keyed **per-VAS by its PDB** (the page-directory-base physical address =
the GPU's CR3, stored in each channel's instance block at `RAMIN+0x200`). This is how
hardware identifies an address space, and it is **client-independent**: many channels —
and many RM clients — share one VAS by pointing their instance blocks at the same PDB.

A channel does **not** name its VAS directly. Open-RM `kernel_channel.c:1030`:

```c
pKernelChannel->hVASpace = pKernelChannel->pKernelCtxShareApi->hVASpace;
```

The effective VAS comes from the channel's **KernelCtxShare** (subcontext, under the
TSG). When the channel's own `hVASpace == NV01_NULL_OBJECT` (`0`) and there is no
explicit ctxshare, the implicit TSG binds the **device-default VAS**. So resolution
order for a channel's VAS is:

1. channel `hVASpace` if non-null, else
2. its ctxshare / TSG VAS, else
3. the **device-default VAS** (lazily created per device).

…then that VAS → its **PDB** = the table key.

**Anti-pattern this forbids (root cause of #12, see
[[mode2_2nd_context_hang]] cont. 7):** keying the per-VAS host state by the **client
handle** (`m2_devvas[client]`). A GSP-managed `_VIRTUAL_MODE` CeUtils scrubber channel
runs with `hVASpace=0` in a *shared/inherited* VAS owned by its ctxshare/device-default,
**not** by its client handle. A per-client lookup returns nothing → the host map bails
(`hDev=0`), the buffer "doesn't exist," and the vidmem finishPayload never resolves —
all one bug. Two channels with the **same** VA→GPA then collide on a FIXED-map (one
"MAPPED", the next "map-FAILED") purely because they were keyed by different clients
instead of the one PDB they share.

**Rule:** resolve every channel to its PDB via the chain above and key the table (and
the host isolate / VAS) on that PDB. Channels sharing a ctxshare/TSG share one table
entry and one isolate — matching silicon, and making "same VA in two clients" a
**reuse**, not a collision.

**Corollary (device-default VAS must be modeled):** because the device-default VAS is
established GSP-side, a faked GSP must itself **materialize its PDB** (assign it / build
its page directory, or capture the guest PMA's page-directory allocation and bind it),
or there is no root to key on for `hVASpace=0` channels. The Rust core's
`HashMap<PdbRoot, …>` (§12) is populated for the device-default VAS at device-alloc
time, not lazily on first channel use.

### 13.1 Concrete instance: the `hVASpace=0` system VAS

The device-default VAS for `hVASpace == NV01_NULL_OBJECT` kernel channels is the first
table to build, because it is **GSP-managed = ours to mint**. Implementation (full
detail + the #12 worked example in [[mode2_2nd_context_hang]] cont. 8):

- One **QEMU-owned VAS per kernel device**, with a **PDB we mint** (not GPU-wide — per
  device, to avoid cross-device VA aliasing).
- **Forward-populated by observation** (BAR1-written page → `VA→FB` interval); the
  finishPayload then resolves by contiguity within the buffer object — no GSP page
  tables, no reverse resolve.
- **Keyed by the minted PDB**, so sibling kernel channels share one entry and one host
  isolate (HW keys by instance-block PDB, client-independent).
- Resolution + host placement consult this VAS when the channel names none and the
  per-client lookup misses — fixing the "no dev/vas for client" class of failures.
- Separate, smaller **coherence** step on the *write* side: complete semas into the host
  page the guest actually reads (the overlay/memslot backing), never the emulated-FB
  copy, and never after a de-alias of a still-referenced shared buffer.
