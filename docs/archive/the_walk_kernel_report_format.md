> ⊘⊘⊘ **ARCHIVED — THIS DESCRIBES A SUPERSEDED ARCHITECTURE. IT IS REFERENCE, NOT CURRENT.**
> The live design is `docs/design/THE_DESIGN.md`. This file is kept because its *measurements*,
> its *ogkm findings* and its *reasoning* remain useful — its **architecture does not**. Do not
> implement from it, and do not cite it as current. Archived 2026-09-21 (w822).

# The walk kernel's report format

**STATUS: DESIGN-ONLY (2026-09-14, w720f).** ⊘ Nothing is built. Owner's request: *"just think of
a format how the ptx responds the allocations to the host. I would do the entire tracking there,
including holding an internal table (of your own format) of the va mappings per channel so you
know whats changed."*

Context: `dirty_tracking_without_uffd.md` §w720e — a CUDA kernel walks the guest's page tables
from the root each refresh, in the scratchpad's contiguous GPGA mapping, and tells the host what
to map. Liveness is **derived** from reachability, never tracked.

## The one decision that sizes everything: RUNS, NOT ENTRIES

The kernel coalesces as it walks. Consecutive PTEs with consecutive GPGAs and identical flags
become **one run**. ⇒ A 2 GiB buffer mapped at 4 KiB is **512K entries but ONE run**, and the
report's size tracks the number of distinct *mappings*, not the amount of memory mapped.

★ This is what keeps the PCIe traffic at kilobytes and makes a per-refresh delta viable at all.
The walk visits entries in ascending VA, so runs are **produced already sorted** — which is also
what makes the diff below a linear merge.

## Two structures, and only one of them crosses the link

| | lives | crosses PCIe | contents |
|---|---|---|---|
| **the kernel's table** | vidmem, persistent | **never** | the full current mapping set, per VAS, sorted runs |
| **the report** | a small output buffer | yes, per refresh | the **delta** only |

The kernel's own table is its business and its format; nothing here constrains it beyond *sorted
runs per PDB*, which makes each refresh's diff a **merge join** against the freshly walked set —
linear, no hashing, no allocation.

## ⊘⊘⊘ CORRECTED BY THE IMPLEMENTATION, w720k — read this before the structures below

`cuda/walk/` implements this contract and **50/50 tests pass**. Six things in this document were
wrong or underspecified; the implementation's README §*"What the format doc got wrong"* is
authoritative where they differ.

1. ⊘ **`ReportHeader` is declared 64 B and its fields sum to 56.** The implementation adds
   `refuse_mask` (this doc's dead `reserved`, given a job — it is what lets a hostile test assert
   **which** refusal fired) plus 8 bytes of pad.
2. ⊘ **Typo:** *"every `first_run + run_count <= run_count`"* should read **`<= header.run_count`**.
3. ★★★ **Validation CANNOT be extended to `gpga` page-alignment, and assuming it is a
   VULNERABILITY.** VER2 carries a **4 KiB-granular address field at every leaf level**, so a
   hostile guest can spell a **512 MiB page whose base is only 4 KiB-aligned**. ⇒ It must be a
   **named refusal** (`KFWR_R_MISALIGNED_LEAF`), never an assumption. `len` page-alignment *is*
   enforceable; `gpga` is not.
4. ⊘ *"An unaligned pointer"* is **inexpressible below the root**: every VER2 table pointer is a
   bit-field shifted by exactly the bits its target's size needs, so the check can only fire on
   the root, which arrives from outside the format. Kept as a checked claim
   (`hostile/unaligned_inexpressible`).
5. ★★ **This doc never ordered a dual PDE's two halves**, yet *"produced already sorted"* requires
   a total order — both sub-tables cover the **same 2 MiB**. The implementation chooses **big
   before small within each 64 KiB chunk**, i.e. **(VA ascending, page size descending)**, and the
   diff's comparator must be the same one.
6. ★ **`TOO_DEEP` is unreachable by construction** and every hostile case asserts its *absence* —
   confirming the design intent that **a cycle is harmless, not detected**.

### ★★★★★ And the finding that makes the bounds check non-negotiable — MEASURED

A negative control rebuilt the kernel with `-DKF_BREAK_BOUNDS`, deleting **only** the two bounds
checks:

| hostile case | with the check deleted |
|---|---|
| `pointer_past_end` | *"an illegal memory access was encountered"* — loud; container recovered |
| the other | ⊘⊘⊘ **silently returned data from beyond the declared window**, reporting a mapping at `gpga=0xdead000` |

⇒ **AN OUT-OF-BOUNDS READ DOES NOT RELIABLY FAULT.** The hardware cannot be relied on to catch
what the check catches. This is the measured argument for the structural invariant, and it is why
the check may never be treated as belt-and-braces.

## The report

Three fixed-size arrays, no nesting, no pointers, no variable-length records — so the Rust side
parses it with bounds checks and nothing else. Little-endian, naturally aligned, **no bitfields**
(their layout is not stable across compilers).

```c
struct ReportHeader {          // 64 B
    uint32_t magic;            // 'KFWR'
    uint16_t version;          // format version; host refuses an unknown one
    uint16_t flags;            // see ReportFlags
    uint64_t generation;       // +1 per completed walk
    uint64_t acked_generation; // what the kernel believes the host last consumed
    uint32_t pdb_count;
    uint32_t pdb_capacity;
    uint32_t run_count;
    uint32_t run_capacity;     // ⊘ truncation is detectable from the buffer alone
    uint64_t entries_visited;  // walk census -- cost, not correctness
    uint32_t refusals;         // out-of-range / too-deep / malformed, by the kernel
    uint32_t reserved;
};

struct PdbEntry {              // 32 B -- one per address space touched
    uint64_t pdb;              // the page-directory base: the VAS identity
    uint32_t first_run;        // index into the run array
    uint32_t run_count;
    uint32_t vas_flags;        // NEW_VAS, GONE, RESYNC
    uint32_t reserved;
    uint64_t reserved2;
};

struct MapRun {                // 32 B
    uint64_t va;               // start, page-aligned
    uint64_t gpga;             // start, page-aligned; meaningless when op == UNMAP
    uint64_t len;              // bytes; a multiple of page_size
    uint32_t flags;            // aperture | read_only | atomic_disable | volatile | page_size
    uint16_t op;               // MAP | UNMAP | REMAP
    uint16_t pdb_index;        // back-reference, so a run is self-describing
};
```

**Ops.** `MAP` — a range the guest now has and did not before. `UNMAP` — had, and no longer.
`REMAP` — same VA range, different GPGA or flags. ⊘ `REMAP` is kept distinct rather than expressed
as `UNMAP`+`MAP` because the host can then change a binding **without a window in which the VA is
unmapped**, which is the difference between a re-point and a transient fault.

**Flags** carry the *decoded* fields, never the raw entry: aperture, read-only, atomic-disable,
volatile, and page size. ⇒ The format-version knowledge (VER2 vs VER3) stays in the kernel and
does not leak into the host's parser or into this format.

## ⊘⊘⊘ THE THREE SAFETY PROPERTIES, in the format rather than in discipline

**1. Truncation is loud and fails safe.** `ReportFlags::TRUNCATED` set, and `run_count ==
run_capacity` independently visible. ⚠ A truncated report is **never** applied as a delta — it
forces a **full resync** (fallback path, batched CE). The dangerous outcome is a short answer that
reads as whole: the host would silently drop mappings and the guest would fault somewhere we could
not explain. ⇒ The host must refuse to apply any report carrying `TRUNCATED`.

★ This is the third hazard class neither bounds-checking nor the depth cap catches: a **legal but
enormous** table tree — every pointer valid, no cycles, 12 GiB mapped at 4 KiB ≈ 3M entries. A
well-formed hostile guest uses exactly this, because it passes every other check.

**2. The generation handshake makes a lost delta impossible.** The kernel's table advances when it
walks; if the host never consumes that report, the next delta would be computed against state the
host does not have. ⇒ The host **acks** a generation; the kernel compares `acked_generation`
against its own and, on any mismatch, emits a **full resync** (`RESYNC`, every mapping as `MAP`)
instead of a delta. ⚠ Any error on the host's consume path simply withholds the ack — so the
failure mode is *"we resync"*, never *"we silently lost a mapping"*.

**3. The host validates the report even though we wrote the kernel.** `magic`, `version`,
`run_count <= run_capacity`, `pdb_count <= pdb_capacity`, every `first_run + run_count <=
run_count`, every `pdb_index < pdb_count`, every `len` non-zero and page-aligned. ⊘ Not because the
kernel is untrusted, but because its **input** is guest-authored: a kernel bug reachable only by a
hostile table is exactly the bug that will exist. Validation is ~10 comparisons over a buffer we
already have in cache.

## Sizing

32 B per run. A refresh that changes a handful of mappings is **hundreds of bytes**. Even a
thousand changed runs is **32 KB** — against the 24 MB the no-tracking design would have moved.
⇒ Cap suggestion: **64K runs (2 MB)**, far above any plausible refresh, so `TRUNCATED` means
*"something pathological"* rather than *"a busy moment"*.

## ⚠ What this does NOT decide

- The kernel's internal table layout — deliberately unconstrained beyond *sorted runs per PDB*.
- Whether the kernel walks **all** PDBs or only those an invalidate named
  (`dirty_tracking_without_uffd.md` layer 1 scoping). The format supports both: `pdb_count` is
  whatever was walked.
- Whether a `UNMAP` for a whole VAS is one run or a `vas_flags::GONE` with none. ⇒ Both are
  expressible; pick when the consumer exists.

## ★★★★★ w720g — THE SCOPE HINT, and the constant it silently depends on

> **Owner, 2026-09-14:** *"tlb invalidate, the rm map call that implies invalidate, or the uvm
> channel, all three of them, can sometimes put which VA table is refreshed. Its maybe only safe to
> parse them, so maybe communicated to the cuda program."*

Parse all three; hand them to the kernel as **scope**, never as truth.

| source | yields |
|---|---|
| RM's BAR0 `0xB830B0` write | `{pdb}` — *"something in this address space"*. **Already trapped** (`mmuinval.rs`) |
| UVM's `MEM_OP_D[31:27] == 0xa` | `{pdb, va_base, va_len}` — a real range; ≤4/batch, power-of-2 rounded **superset** |
| an RM map call implying an invalidate | `{pdb}`, and often the mapped range from the call itself |

The kernel takes an array of `{pdb, va_base, va_len}` with a sentinel meaning **walk this whole
PDB**. Anything ambiguous, absent, or overflowing the scope array degrades to a **full walk** —
cheap, because a full walk is ~67 µs.

⇒ ★★★ **The hint can only make the walk FASTER; it can never make it WRONG.** That asymmetry is
what makes it safe to act on a signal that is only *sometimes* precise.

### Why the hints are COMPLETE over what matters

**A mapping the guest has not invalidated for is one the guest itself cannot rely on** — on real
hardware its TLB would be stale. ⇒ Every *usable* mapping has been announced by one of the three,
and usable-completeness is the only completeness we need.

★ This also disposes of UVM's *"no invalidate needed"* cases (`uvm_va_block.c:6367`, `:6566`,
`:6682`, `:7063`): they occur under an **already-invalid parent**, so the mapping becomes usable
only when the parent is validated — which **does** invalidate.

### ⊘⊘⊘ THE HOLE — checked, and we are safe BY A CONSTANT NOBODY CONNECTED TO THIS

`kgmmuInvalidateTlb_GM107` emits **no invalidate at all** under three conditions
(`kern_gmmu_gm107.c:126-135`):

```c
    if (API_GPU_IN_RESET_SANITY_CHECK(pGpu) ||
        IS_VIRTUAL_WITHOUT_SRIOV(pGpu) ||
        (IS_VIRTUAL(pGpu) && gpuIsWarBug200577889SriovHeavyEnabled(pGpu)))
        return status;
```

⇒ If our emulated GPU made the guest believe it were a paravirt vGPU, **RM would stop invalidating
entirely** and every hint would vanish — with no fault, no refusal, just stale bindings.

★ **We are safe, and the reason is already in the tree** (`ga10x.rs:543`):

> `NV_PMC_BOOT_1`. Read back **zero**: `VGPU = REAL`, i.e. this device advertises no
> [virtualization]

⚠⚠ **But note what that makes this constant.** `PMC_BOOT_1 = 0` was chosen so the guest believes
it owns real hardware — mode 2's premise. It now **also** turns out to be the thing that keeps RM
emitting invalidates at all, i.e. the precondition of this entire scoping layer. A future change
that advertised vGPU to satisfy some unrelated check would break this silently.

⇒ **Guard it and say why**, before anything depends on it: a test asserting `PMC_BOOT_1 == 0`
whose message names *the invalidate skip*, not just *"we look like real hardware"*.

## ★★★★★ w720i — WHERE THE KERNEL RUNS, AND WHY IT MUST NOT BE THE VMM

> **Owner, 2026-09-14:** *"a hang on the scratchpad is only harm for the guest itself. I am more
> worried that we execute the CUDA from a trusted VMM process, and that if the kernel is
> compromised that it can get host access, since cuda has no boundary specifically between a host
> process and who launched the cuda kernel."*

★ Correct, and it **outranks** the hang. It also decides the placement question left open in
`dirty_tracking_without_uffd.md`.

### Scoping it precisely — this is NOT code injection

The PTX is **ours**, compiled at build time; nothing guest-controlled selects or modifies it. What
is guest-authored is the **data** the kernel walks. ⇒ The bug class is **memory safety in ~200
lines of CUDA we write and can audit**, not *"the guest runs code in our context"*.

⊘ And note the guest **already** executes arbitrary GPU code — that is the product. GPU MMU
context isolation is already load-bearing. What this adds is not that dependency; it is **a
context worth attacking**.

### The attack path

hostile tables → our kernel computes a wild address → it writes wherever that lands **inside its
own CUDA context**. ★ The owner's point is exactly right: the GPU MMU separates contexts from each
other, but **within** a context a kernel reaches everything the context mapped — including any
host memory pinned or mapped into it. If that context belongs to the VMM, a wild write reaches
VMM state.

### ⇒ The fix is to make the context WORTHLESS. Four layers, strongest first

1. ★★★ **Never the VMM process.** A **dedicated isolate** whose entire address space is the GPGA
   buffer, the shadow and the output. Full compromise yields **nothing**: no guest RAM, no
   descriptors, no VMM state, no other guest's anything. ⇒ This answers *"worker or isolate"* with
   a reason: **a dedicated walk isolate — not the worker, and never the VMM.**
   ⊘ Note this is the isolate pattern used **for security**, where
   `isolate_exists_for_VA_IDENTITY_not_security.md` records it normally is not.
2. ⊘⊘⊘ **SUPERSEDED w720j — DO NOT map the GPGA buffer read-only.** ~~Map it READ-ONLY in that
   context.~~
   **Owner, 2026-09-14:** *"I would not map GPGA as read-only, map it as write, its not a problem.
   Then you do not need a separate maintained channel, just use the same scratchpad one."* …
   *"because you need write in GPGA to do CE copies, and thats on scratchpad."*

   ★ Two reasons, and the second is mechanical rather than a judgement call:
   - ⊘ **It is not a boundary.** The same isolate holds a **CE that can already write anywhere in
     GPGA**, so a read-only CUDA mapping stops nothing an attacker owning that process could not
     do by another door. It would guard only against a bug in **our own** kernel — for which the
     hostile-table suite is a better instrument than an Xid in production.
   - ★★★ **The scratchpad's GPGA mapping MUST be writable, because that is the mapping its CE
     copies use.** A read-only second mapping would be a duplicate VA range over the same memory,
     with different permissions, kept in sync — exactly the bookkeeping this redesign exists to
     delete.

   ⇒ **One mapping, read-write, the scratchpad's existing one.** See the address-space invariant
   below for what *does* have to be unreachable.
3. ★ **No host memory mapped into the context at all.** Output lands in device memory and is
   copied out afterwards, so there is **no host page the kernel could name** even with a
   completely wrong address.
4. **Validate the report anyway** (already in this document's §3). Not because the kernel is
   untrusted, but because *a kernel bug reachable only by a hostile table is exactly the bug that
   will exist*.

⇒ With all four, the worst outcome of a memory-safety bug is: corrupt the guest's own video memory
(**impossible** under 2), corrupt the shadow (**forces a resync**, self-healing), or emit a
malformed report (**rejected** by 4). **Nothing reaches the host.**

### ⚠ The residual, stated honestly

A hung kernel in a dedicated context **still occupies GPU resources**, and recovering it may need
a channel or context teardown that briefly disturbs the guest sharing the card. ⇒ Contained, not
free — but a **liveness** cost, never a confidentiality or integrity one.

## ★★★★★ w720j — THE ISOLATE'S ADDRESS-SPACE INVARIANT, and the one region that must be hidden

GPGA is mapped **read-write** (above). What must be unreachable by the guest is **the shadow**.

⊘⊘⊘ **If the shadow lives anywhere inside GPGA, a hostile guest writes to it directly through its
own page tables and forges *"unchanged"* for a table it just edited.** ★ That is the **collision
attack that killed hashing**, arriving by a different route — and strictly worse, because the
guest does not even have to search for a collision, it just writes the answer it wants.

### The invariant

| region | guest-addressable? | kernel | 
|---|---|---|
| **GPGA** | **yes**, by construction | reads |
| **the shadow** | ⊘ **never** | reads + writes |
| **the report buffer** | ⊘ **never** | writes |
| **host memory** | ⊘ **none mapped at all** | — |

The shadow lives in the scratchpad isolate's own VA space — a separate RM allocation, or a region
of the reserved object deliberately **not** exposed as guest GPGA. Either works; what matters is
that **no guest page table can name it**.

⚠ **Check it rather than assume it.** A test that walks the guest's own page tables and asserts
none resolves into the shadow or the report is a direct instrument; the alternative is a
structural assumption that a future layout change breaks silently — and silently is exactly how
this one would break, since a forged *"unchanged"* produces no fault and no refusal.

## ★★★★★ w723 — SETTLED: THE KERNEL RETURNS CURRENT STATE, NOT DELTAS

The choice left open at w720f/w720j is decided. **The kernel walks and emits the current run set.
kayfabe diffs it against the Rust model it already maintains.**

### Why

1. ★★★ **The diff had a real closure bug, in C.** `[w722]` the whole-run merge join emitted, for
   one new run covering several old ones, a `REMAP` followed by `UNMAP`s of the runs it had just
   replaced — **applying that in order deleted the mapping just made.** All nine single-step
   `delta/*` cases passed against it. ⇒ **Diff logic is subtle enough to get wrong**, and it
   belongs in safe Rust beside the model and the existing tests, not in the place where a missed
   check returns silent garbage.
2. **No shadow ⇒ no forge-unchanged attack.** The last descendant of the hashing-collision problem
   disappears: the guest cannot write a shadow that does not exist.
3. **No generation handshake for shadow coherence**, and no *"the shadow advanced but the host
   never consumed it"* hazard.
4. **The kernel becomes stateless**, composing with the from-root walk already being stateless:
   in goes a root, a scope and setup data; out comes *what the tables currently say*. Nothing
   persists on the GPU between launches.
5. **The cost is nothing.** ~1000 runs × 32 B ≈ **32 KB** a refresh, ~38 MB a boot — against 28 GB
   for raw tables. The delta is cheaper and the difference does not matter.

### What this changes in the format above

- `MapRun::op` is **not filled by the kernel**. It is either dropped, or filled by the host as the
  product of its own diff. ⊘ Pick one; do not leave a field the kernel ignores and the host
  believes.
- `PdbEntry::vas_flags` keeps `NEW_VAS`/`GONE` only if the host cannot derive them — it can, so
  prefer deriving.
- **`acked_generation` and the resync handshake survive**, for a different reason: a report that
  arrives torn or truncated must still be refused as a whole. ⇒ The handshake now protects the
  *report*, not a shadow.
- ⊘ **`TRUNCATED` still forces a full refresh** and is still never applied — unchanged.

### ⊘ The w722 diff work is NOT discarded

The per-page-size-class, segment-granularity algorithm — `UNMAP` where old covers and new does
not, `MAP` where new covers and old does not, `REMAP` where both differ, with the two sets
**disjoint in `(va, class)` by construction** so apply order cannot matter — is **exactly what the
host side must implement**. It moves languages, not designs. ★ And `cuda/walk`'s round-trip
harness was deliberately built uncoupled from the shadow (*"a fresh full walk is a second walker
that is never acked"*), so the same 2000-step closure property tests the **host** diff unchanged.

## ★★★★★ w725b — RUN IDENTITY MUST INCLUDE `KIND`: do not coalesce across a field you do not propagate

`[found w725, from real driver tables]` Replaying a stock 580.159.04 guest's own page tables shows
**99.7% of leaf entries carry `KIND` and `COMPTAGLINE` values our synthetic builder can never
produce** — `KIND=9` ×6017, `KIND=6` ×959, `KIND=0` on only **22**.

⊘ Decode is unaffected (VER2's vidmem address is 32:8, below both fields) and that is now checked.
**The coalescer is affected, and nothing had decided it.** Run identity is *(contiguous VA,
contiguous GPGA, equal decoded flags)*, and the flags carry **no KIND** — so the real 12 GiB
address space emits **1 run where KIND-aware identity emits 3**, at two boundaries where the guest
changed memory kind across contiguous VA *and* contiguous physical address under identical
permissions.

### ⇒ The decision: KIND joins run identity

★ **A run is the unit we publish as ONE mapping, and one mapping can carry only one kind.** On real
hardware a PTE's `KIND` tells the GPU how to *interpret* the memory — tiling, compression. A host
mapping built with the wrong kind makes an engine misread a surface the guest wrote correctly: no
fault, no refusal, wrong pixels or wrong tensors.

⊘ **Even though we may not propagate `KIND` yet.** The principle is the point:

> **Never coalesce across a field you do not propagate.** Merging destroys the boundary
> irreversibly; keeping it costs two extra runs. You can always merge later — you cannot unmerge.

⇒ Cost on the real address space: **3 runs instead of 1.** Against a report cap of 64K runs, that
is nothing, and the report's size tracks distinct mappings rather than memory mapped.

### ⚠ What this does NOT settle

Whether our publish path can *express* a kind on the host mapping. If it cannot, the boundary is
preserved and unused — which is the correct order: **preserve first, propagate when the publish
path can carry it.** ⊘ The opposite order silently loses the information before anyone notices it
was needed.

★ `COMPTAGLINE` is deliberately **not** added: it is a compression-tag *index*, allocated by RM
per surface, and two runs differing only in comptagline are the same mapping shape. ⚠ Revisit if we
ever propagate compression state — and note the two `dev_mmu.h` copies in the reference tree
**disagree about its width** (53:36 vs 55:36), which is its own reason not to key anything on it.
