# Dirty tracking for one store, without userfaultfd

**STATUS: LIVE (2026-09-14, w720).** Design options and their arithmetic. ⊘ **Nothing here is
built**, and the measurement that arbitrates between them has not been taken.

## Why this file exists

The **single-store** simplification — everything in one reserved RM object, no fake fb, no two
worlds — is wanted (owner, 2026-09-14): *"no two worlds, no ram, everything vidmem, CE fast
enough, one RM object, one MMIO mmap, a lot of code deleted."* ⊘ Its one real cost is **dirty
tracking**: *"without dirty tracking we end up reading tens of megabytes, every single refresh"*
— 1178 refreshes a boot, so tens of GB of link traffic stolen from the workload.

⊘⊘ **`userfaultfd` is ruled out** (owner; `uvm_hmm.c:577` *"UVM doesn't support userfaultfd"*;
`nvkvm-pv/docs/internal/gpa-window-narrow-maxphyaddr.md:213`). Do not re-propose it.

## The options, and what each actually saves

| # | idea | saves the READ? | saves the DECODE? | new machinery |
|---|---|---|---|---|
| 1 | the invalidate names the edited range | **yes** | yes | small |
| 2 | a spare bit in the entry, cleared when RM rewrites it | ⊘ **no** | yes | small, + format risk |
| 2′ | **per-page hash, compare against last refresh** | ⊘ no | yes | trivial, **no format risk** |
| 3 | **GPU kernel diffs the tables in vidmem** | **yes** (link) | yes | ★ large |
| 4 | batch the CE copies into one push | n/a | n/a | small — **a precondition, not an option** |

### ⊘ Idea 2's structural flaw, and why 2′ dominates it

To see the marker was cleared you must **read the entry**. Scanning every entry to check its bit
pays the transfer you were avoiding. ⇒ It saves the *downstream* work, not the copy.

★ That may still be the larger half: reading 24 MB by CE at 10 GB/s is ~2.4 ms, while decoding
~960k entries at even 10 ns each is ~9.6 ms. **The decode likely dominates the copy.**

⊘ But **hashing (2′) gives the same answer with none of the risk**: no assumption about which bits
the MMU ignores, no risk that RM reads back an entry and trips on a stray bit, and no dependence
on hardware behaviour the open source cannot describe. ~0.2 ms for 1872 pages. ⇒ **Prefer 2′ over
2** unless the research finds a genuinely ignored bit *and* proves nothing validates entries it
did not write.

⚠ **Hierarchical detection would save the read** — read the small directory level, learn which
leaves changed — but only if a parent entry is rewritten when a child changes. Editing a PTE does
not normally touch its PDE. **If that holds, this route cannot save reads at any granularity.**

### ⊘ Polling a marker through the MMIO mapping does not help

`[measured, this tree]` word-at-a-time reads of video memory run at **13–15 MiB/s** (~0.3 µs a
word) against 48 MiB/s bulk. One marker per page is 1872 serialised reads ≈ **0.5 ms**, the same
order as CE-copying all 7.3 MiB (~0.8 ms) — and it **does not batch**, where the CE version is one
submission. ⇒ Same cost, batching given up.

### ★★★ Idea 3 — move the scan to the data

**Owner, 2026-09-14:** *"we upload a small ptx program that reads the vidmem with GPU speed to see
if there is an update and only reports back whats changed."*

| scanning 24 MB of tables | one pass | × 1178 refreshes |
|---|---|---|
| CE copy over PCIe @ ~10 GB/s | 2.4 ms | **2.8 s, ~28 GB of link traffic** |
| GPU kernel over vidmem @ ~360 GB/s | **67 µs** | **79 ms, ~0 link traffic** |

Shadow copy in vidmem; a kernel compares live against shadow and writes changed indices to a small
output buffer; CE back only that buffer. ★ **The link leaves the path entirely.** Composes with
idea 1 rather than competing: the invalidate says where to look, the kernel says what changed.

⚠ **Cost, stated honestly:** our own GR channel, our own module, our own launch path, and a
PTX/CUDA dependency on the host side — more new surface than ideas 1, 2′ and 4 combined. Not
foreign territory (guest compute forwarding works, `CUP3_VAL=43`), but *our* kernel on *our*
channel is not the same thing as forwarding the guest's.

### Idea 4 is a precondition of every design that CE-reads tables

Per-page submission is 1872 × 1178 ≈ **2.2M** doorbell/semaphore round trips; at even 20 µs each
that is **~45 s a boot — worse than the CPU path it replaces.** Batched it is 1178 submissions and
the cost vanishes into the bytes. ⇒ **Gather into one push, always.**

## ⚠ THE MEASUREMENT THAT ARBITRATES ALL OF THEM — and it has not been taken

**How many page-table pages actually change per refresh?**

★ Ideas 1, 2, 2′ and 3 all pay off **in proportion to sparsity**, and all are pointless if changes
are dense — then the bytes must cross regardless and batched CE (4) is the entire answer.

⊘ It is cheap and needs no new machinery: the refresh already re-reads the tables, so hashing each
page and counting changes is a small patch on the existing boot path. **One boot answers it.**
⇒ Take this number before building any of 1, 2′ or 3.

## ★★★ w720b — THE RESEARCH VERDICT: 2 IS DEAD, 2′ SURVIVES, 1 IS HALF-AVAILABLE

Read from `ogkm 610.43.02`. ⊘ **Idea 2 (a spare bit as an explicit dirty marker) must not be
built**, and the reason is not aesthetic — it is **two silent false negatives**, the dangerous
direction:

1. **The read-modify-write path preserves the bit verbatim.**
   `virt_mem_allocator_gm107.c:1797-1802` — when `bReadPtes` is set (any update that is not
   `UPDATE_ALL` and not an invalidation, `:2222`) RM `portMemCopy`s the existing entry out,
   edits fields, writes it back. No masking, no validation. ⇒ **RM changes a PTE meaningfully and
   our marker survives; the detector reports "unchanged".**
2. **Table relocation bulk-copies entries** (`_gmmuWalkCBCopyEntries`, `gmmu_walk.c:1009-1010`,
   raw `memmgrMemCopy`). The bit survives a table move.

And the bit was never safe to begin with:
- On the **Turing/Ampere** headers the VER2 PTE has **zero unnamed bits** — all 64 are claimed.
- ⊘ The two `dev_mmu.h` copies in this tree **disagree about bits 54:55**: `gp100/dev_mmu.h:150`
  gives `COMPTAGLINE (18+35):36` = 53:36, `tu102/dev_mmu.h:636` gives `(20+35):36` = 55:36 — and
  `uvm_turing_mmu.c:299` comments "53:36" while compiling against 55:36.
- ⚠ No `_RESERVED`/`_IGNORED`/`_SPARE` field exists in any `dev_mmu.h` here, and the source states
  the MMU reads fields one would expect it to ignore: *"When VALID == 0, MMU still reads the VOL
  and PRIV fields"* (`uvm_turing_mmu.c:233-235`). ⇒ **The source cannot certify any bit as
  hardware-ignored. Do not stake the design on it.**

★ **2′ (per-page hashing) is untouched by all of the above**, because it compares **content**: an
RMW edit shows up, a relocation shows up, and it assumes nothing about which bits hardware reads.
⇒ The research turns *"prefer 2′"* into *"2′ is the only safe one of the two."*

### Idea 1 — available for UVM, never for RM

| | targeted range? | detail |
|---|---|---|
| **UVM** | **yes** | `tlb_invalidate_va` carries `{base, size, page_size, depth, PDB}` (`uvm_hal.h:141-147`); on the wire `MEM_OP_D[31:27] == 0xa` (`MMU_TLB_INVALIDATE_TARGETED`), addr in `MEM_OP_A[31:12]`+`MEM_OP_B`, log2 size in `MEM_OP_A[5:0]` |
| **RM** | ⊘ **never** | `kern_gmmu_gm107.c:193-194` sets `_ALL_VA, _TRUE` **unconditionally**: *"Not using range-based invalidate."* Zero hits tree-wide for `MMU_TLB_INVALIDATE_TARGETED` under `src/nvidia/` |

⚠ Two limits on the UVM half: **≤4 ranges per batch** (`UVM_TLB_BATCH_MAX_ENTRIES = 4`,
`uvm_tlb_batch.h:36`; `max_ranges = 8` on Ampere, `uvm_ampere.c:33`) — the 5th collapses the batch
to invalidate-all — and the range is **rounded up to a power of two and aligned down**
(`uvm_ampere_host.c:288-303`). A **superset**, so safe, but not tight.

### ⊘⊘⊘ Q3 — HIERARCHICAL DETECTION IS DEAD

**A child PTE change never causes a parent PDE write.** No accessed bit, no dirty bit, no counter,
no valid-count anywhere in the VER2 PDE or DUAL_PDE. Both drivers keep that bookkeeping in **host
CPU structs** — `uvm_page_directory_struct.ref_count` (`uvm_mmu.h:144`), RM's
`numValid`/`numSparse`/… in `MMU_WALK_LEVEL_INST` (`mmu_walk_private.h:184-209`). A PDE is written
only on a **structural** change (`mmu_walk.c:1506-1510`: *"If we've changed any sublevel…"*).

⇒ Reading a small directory level tells you **nothing** about which leaves changed. ★ One partial
exception: at PD0 (and PD1 on Ampere) the 16-byte entry is *either* a dual PDE *or* a 2 MB /
512 MB PTE (`uvm_turing_mmu.c:47-53`, `gmmu_fmt.c:64-86`), so watching those levels **does** catch
every large-page mapping change — and nothing about 4 KB or 64 KB.

### ⚠ A correction of my own, recorded because the shape recurs

I briefed this research with *"both invalidate transports measured ZERO"* from the research repo's
`CLAUDE.md`. **This tree had already corrected that at w326/w476** — `mmuinval.rs` documents RM's
transport as a BAR0 store at `0xB830B0` and names the old zero as *"a census over transports is
only as complete as its list of transports"*. ⇒ The agent re-derived a correction we owned.
**A stale fact in a navigation file costs a subagent's whole run**, and `CLAUDE.md` is the most-read
file in the other tree.

★ One genuinely new detail from it: **RPC fn=200 is a compile-time STUB on GA106**
(`g_rpc_private.h:320`, `rpcInvalidateTlb_STUB`), returning `RPC_UNKNOWN_FUNCTION`. ⇒ That
particular zero was **structurally guaranteed** and could never have been anything else.

## ★★★★★ w720c — THE SETTLED DESIGN: three layers, each degrading into the one below

**Owner, 2026-09-14:** *"there is probably already cuda on the host, so we can just link against it.
and if the PTX fails, we fall back to CE copying everything batched."* … *"still using idea 1"*.

### ⊘ Hashing is OUT — and the reason is SECURITY, not correctness

**Owner:** *"I don't think any form of CRC is safe to handle va addresses guest userspace can select
to try crc to match itself."* ★ Correct, and it is the product's own threat model: guest userspace
picks VAs ⇒ picks PTE contents ⇒ can **search for a collision** that makes the detector report
*"unchanged"* about a table it just changed. A hash is **forgeable by the adversary this project
exists to contain** ([[hostile-guest-isolation-is-the-value-proposition]]).

⇒ **Byte-exact comparison against a shadow the guest cannot address.** Idea 2′ dies with idea 2,
for a different and better reason.

### The layers

| layer | answers | mechanism |
|---|---|---|
| **1 — the invalidate** | *when*, and *where to look* | RM: BAR0 `0xB830B0` ⇒ "this PDB" — **already trapped** (`mmuinval.rs`). UVM: `MEM_OP_D[31:27]==0xa` ⇒ a VA range (≤4/batch, power-of-2 rounded superset) |
| **3 — the PTX diff** | *what changed*, byte-exact, within that scope | compare live vs shadow in vidmem; emit a **page bitmap** |
| **4 — batched CE** | moves only the changed pages | **and is the fallback** |

★★★ **Every layer degrades into the one below it**, which is what makes the design safe to build
incrementally:

- PTX absent / JIT fails → batched CE over the scoped range.
- RM's invalidate (no range) → scope is the whole PDB; diff it.
- Everything missing → batched CE of all tables — **the baseline, which is required anyway.**

⇒ **This reorders the build.** The fallback is the floor, not a contingency: build **4** first
because it is needed under every branch, then **1** (scoping — the BAR0 trap already exists), then
**3** as pure acceleration. Sparsity stops gating the start; it only decides whether layer 3 earns
its place, and can be measured *while* layer 4 is built.

### `[measured w720c, bench host 51007398]` The CUDA dependency is DRIVER-ONLY

    libcuda.so.580.159.04                   present
    libnvidia-ptxjitcompiler.so.580.159.04  present
    nvcc                                    NOT installed

★ Both ship with the **driver**; the box has no CUDA toolkit at all. ⇒ Nothing new to install
anywhere we already run. And with no `nvcc`, the PTX is **hand-written or shipped as a string
constant** (a byte-compare kernel is ~30 lines), so there is **no build-time CUDA dependency
either**.

### ⚠ Why PTX and not a hand-built launch — the Turing+ requirement decides it

Launching compute at the RM level means constructing a **QMD**, and ogkm publishes **exactly one**
QMD header in the entire tree: `src/common/sdk/nvidia/inc/class/cla0c0qmd.h` — **Kepler**. Turing,
Ampere, Hopper and Blackwell QMD layouts are **not in the open driver**; they live in closed CUDA
userspace and differ per family. ⇒ A hand-built launch is **per-family reverse engineering by
construction**, which is the opposite of the owner's *"ensure that it works across all families
(Turing+) we target"*. PTX compiled for `sm_75` JITs forward onto every later architecture.

⊘ Note we have **never allocated a compute class**: we allocate CE (`AMPERE_DMA_COPY_B`) on our own
channels, and the compute class list exists only in a `kayfabe-chips` **test**, as the allowlist for
forwarding *the guest's* objects. Our own compute channel is new ground either way.

### ★ A possible sidestep for the interop, worth pricing

CUDA may not need to address our reserved object at all. **CE vidmem→vidmem is LOCAL bandwidth
(~300 GB/s), not PCIe** — so CE-copy the live tables into a CUDA-owned vidmem buffer (24 MB ≈
80 µs, no link traffic), diff against a CUDA-owned shadow, and return only the bitmap. That trades
~80 µs of local copy for deleting `cuImportExternalMemory` and the two-worlds-of-VA problem
entirely.

### Sizing

24 MB of tables = 6144 pages ⇒ a **768-byte bitmap**. The kernel reads live + shadow = 48 MB at
~360 GB/s ≈ **133 µs**. Only the bitmap crosses the link.

## ★★★★★ w720d — FOUR REFINEMENTS FROM THE OWNER, one of which changes the arithmetic

### 1. The kernel COMPARES AND UPDATES in one pass

> *"the compare PTX also directly writes the old so we don't have to do another slower CE copy."*

One launch: read live, compare against shadow, write changed bytes **into** the shadow, emit the
bitmap. ⇒ The second CE disappears.

⚠ **The hazard this introduces, and it must be designed in from the start.** The shadow advances
**whether or not we successfully consume the bitmap**. An error between the kernel completing and
the changed pages being acted on loses that change **permanently** — the next diff sees
`shadow == live` and reports clean. ★ This is the classic *dirty bit cleared before the work
committed*. ⇒ Carry a **generation counter**, and make **any** error on the consume path force a
full refresh rather than a diff.

### 2. ⊘ NO host-side shadow at all — the Rust model IS the "old"

> *"best if we just rely on our internal table that already contains all va mappings in Rust
> structures rather than duplicating an old version of the table on the host ram."*

★ We already maintain the decoded VA→GPGA model. Comparing **decoded model vs freshly-read table**
answers the same question as byte-diffing, without duplicating megabytes of host RAM.

⇒ **The shadow becomes purely a PTX-path artifact, living in vidmem.** The fallback path has no
shadow to keep coherent, so the two paths never have to agree about one — which removes the
worst class of bug a dual-path design would otherwise have.

### 3. ★★★ READ FROM A PRIVATE SNAPSHOT — a correctness fix independent of dirty tracking

> *"the CE copy batches it to our VM va (private) and then we use that copied version to act on for
> reading tables (since its private, the guest cannot change underneath us after copy)."*

Today a walk reads memory **the guest can mutate mid-walk**: two entries read a microsecond apart
can come from different generations, and the decoded result is a tree **that never existed**.
Copying into our private VMM VA first makes consistency **structural** rather than a timing
assumption.

⇒ Worth doing **on its own merits**, before and independently of any of the dirty-tracking layers.

### 4. ⚠ BATCHING IS PER-LEVEL, NOT PER-REFRESH — and this corrects §"Idea 4" above

> *"further copies can be later batched inside a single refresh, for example where leaves are is
> only possible if you decoded and read the base."*

A refresh **cannot** be one CE batch: the leaf addresses are unknown until the level above is
decoded. ⇒ **One batch per level, levels serial** — four to five round trips per refresh on Ampere
(PD3→PD2→PD1→PD0→PT), so ≈5 × 1178 ≈ **6k submissions a boot**, not 1178.

⊘ Still cheap, but it is **5× the figure quoted earlier in this file**, and it is a **hard floor**
for the CE-only path. Within a level, batch everything.

★★★ **This is where layer 3 earns its place a SECOND time, beyond bandwidth.** A GPU kernel can
**pointer-chase on the GPU** — root to leaf without returning to the host per level — collapsing
four or five serial round trips into **one launch**.

⚠ Cost: PTE/PDE field decode moves **into the kernel**. Only bit extraction, but it is real format
knowledge in a new place, and it is per-format (VER2 here, VER3 on Hopper+).
⊘ **A cheaper middle exists and should be chosen deliberately, not by default:** the kernel diffs
only the page set discovered by the **previous** refresh, and newly-appeared tables are picked up
on the next one — sound because the table set changes slowly, and it keeps all format knowledge on
the host.
