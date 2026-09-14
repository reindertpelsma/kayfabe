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
