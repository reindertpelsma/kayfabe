# THE CONSTRAINTS — what kayfabe must do, not just do

**STATUS: LIVE, 2026-09-13.** Owner's list, given in conversation this date and consolidated here
so it survives a context compaction. ⊘ This supersedes nothing; it COLLECTS what is scattered
across `THE_OVERNIGHT_DIRECTIVE.md`, the design docs and agent memory.

> **Why this file exists.** The owner, 2026-09-13: *"we already got the LLM passing under
> everything trapped on kayfabe a few days ago… If bar1/bar2 is now STILL trapped under your cuda
> runs then its worthless to continue, you are basically doing something that worked. The entire
> reason I am pursuing this is because I want it to get to work under the constraints."*
>
> ⇒ **Functionality is not the deliverable. Functionality UNDER THESE CONSTRAINTS is.** A green
> workload that meets none of them is a repeat of work already finished in early September.

## ★★★★★ ANSWERED `[measured w708-w710, 2026-09-14]` — all three pass under the constraints

> Owner: *"So is it possible to get raw, cuda and llms under the constraints?"*

| workload | result | BAR1/BAR2 | vCPU blocking |
|---|---|---|---|
| raw client (`--uvm-mean`) | **(P)**, `MEAN_FALSIFIER=PASS`, `THREADS 8 of 8` | `TRAP_FILLS=0` | 22 = PRAMIN moves |
| cup3 (CUDA) | **`CUP3_VAL=43`** — first compute | `TRAP_FILLS=0` | 22 = PRAMIN moves |
| LLM (Qwen2-0.5B) | **`LLM_OK=1 LLM_TOKENS=16`** | `TRAP_FILLS=0` | 32 = PRAMIN moves |

At LLM scale: **22 671 doorbells arrived, 22 517 served, 8 refused**; `slots peak=1032`;
`HOST_DMESG_XID=0`.

⇒ Constraints **1** (no BAR1/BAR2/PRAMIN traps), **2** (BAR0 write-only bar the counter page),
**4/8** (the only vCPU blocking is the boot-time PRAMIN re-point you ruled sufficient) and **5**
hold across all three workloads, including the one that generates twenty-two thousand doorbells.

★ **The premap-scaling fear was unfounded.** `distinct_pages=912` for BAR1 under the LLM — the
SAME as under cup3 — because the BAR1 working set is bounded by the 256 MiB aperture, not by model
size. "Zero traps is workload-limited" was a real risk and it is now measured false.

⊘ What unblocked all three was ONE bug (`the_uvm_map_external_allocation_wall.md`): the
per-doorbell cap stranding UVM's push burst. The raw client strands **zero**, which is why goal 7
was green for weeks over the same code path — and is the known-negative that validates the fix as
inert where it should be and decisive where it mattered.

### ★ Goal 8 too — and the reactor turns out NOT to be a prerequisite

`[measured w711]` `TWOCLIENT_OUTCOME=(P)`: two raw clients pass the mean test **concurrently** —
two independent `W392D_OUTCOME=(P)`, `THREADS 8 of 8` each, **28 overlapping pairs of graded
intervals**. Goal 8 states a mechanism (*"needs an epoll loop in workers"*) as well as an outcome;
the outcome is met **without** it. Consistent with the earlier survey: 1 worker / 4 workers /
4 isolates all measured **1.00x**, because RM holds a device-global API lock across the GSP RPC.
⇒ the reactor buys **liveness isolation**, not throughput.

⚠ **The pass is not clean, and the grade does not say so.** `HOST_DMESG_XID=2` — one
`Xid 31 MMU Fault: ENGINE CE0` **per client**, the same fault a single-client boot produces once.
The clients content-verify and grade `(P)` anyway, so it is contained; but a `(P)` here means
*"the client's own checks passed"*, not *"the host GPU was never faulted"*. ⊘ Concurrency is not
its cause — it is present with one client — so the reactor must not be built to fix it.

**Still open:** 7 and 14 (epoll workers, threaded isolates — now an improvement, not a blocker),
12 (any die), 15 (the two disjoint vidmem worlds / one reserved object), 16 (memslots as setup),
and the per-client host MMU fault above.

## The list

1. **No traps in BAR1, BAR2 or PRAMIN.**
2. **Only WRITE traps in BAR0**, PRAMIN excepted. (Read traps in BAR0 are already shown
   unnecessary.)
3. **Multiple concurrent workers in isolates, with multiple parallel transactions.**
4. **All traps sub-millisecond.** The PRAMIN base re-point may be longer — it is the one
   sanctioned expensive trap (`the_write_trap_contract.md`).
5. **The new DoorbellTable wired.**
6. **Every MMIO trap only posts the register write to a queue** (optionally clearing another
   register to close a race), wakes a worker, and returns.
7. **epoll in workers**, so more CUDA work runs in parallel than there are workers.
8. **Workers do all the work**, and completion writes land in VMM memory, asynchronously from the
   vCPU threads.
9. **Emulated channels actually use the scratchpad** to do work when work is needed.
   ⊘ Not forged: *"a scrub must be executed, on scratchpad, if its from an emulated channel."*
10. **Scratchpad work can go from polling to waiting on an eventfd** to cut CPU load (sets the
    eventfd IRQ).
11. **Interrupt forwarding actually works** — passthrough when libcuda falls back from semaphore
    to eventfd, and the emulated-channel wake-up IRQ too (`the_interrupt_arming_model.md`).
12. **No hardcoded single chip.** Any die must work, as in `nvkvm-pv`. A per-die fact must be one
    of: derived from ogkm source · obtained by an **unprivileged** host userspace ioctl ·
    computed · a stub that satisfies ogkm because guest userspace does not care · or defined per
    ARCHITECTURE FAMILY so it stays maintainable.
13. **No raw VMM pointers in safe code.** They belong in `unsafe` only, and safe code is always
    bounds-checked rather than trusted to have been written correctly.
14. **Isolates can have multiple threads** executing several CUDA operations in parallel, as
    `nvkvm-pv` does.
15. **The two vidmem worlds are disjoint** — see below.
16. **Memslots are a SETUP thing, not a runtime one.** Reserve VMM ranges for BAR0/1/2 **once**,
    at start. Anything unoccupied is a sparse region if one is needed (mappable from the GPU)
    unless ogkm genuinely allows it unmapped — and because the range is VMM-**reserved**, no
    anonymous heap allocation can land in it. At **runtime** you translate a BAR1/BAR2 offset
    (to GPA where that is not skippable) into a **VMM VA**, and use that VA in `mmap` or in
    ioctls. ⊘ Never one memslot per published page. Same model as `nvkvm-pv` and the Mode-2 C.
17. **Host userspace stays UNPRIVILEGED.** Standing, absolute, and it constrains every item above.

## ⊘⊘⊘ SUPERSEDED w721 — THERE IS ONE WORLD, NOT TWO. Read this before §15 below.

**Owner, 2026-09-14:** *"One GPGA store, one RM object, no more fake fb, no more bar1/bar2 traps,
no more populate on fault."*

§15 split framebuffer backing into two worlds because **CPU reads of video memory are 48 MiB/s**
and page tables had to be re-read every refresh. ⇒ **That premise is gone.** The copy engine reads
video memory at ~10 GB/s across the link and a GPU kernel reads it at ~360 GB/s *without crossing
the link at all*, so there is no longer a reason for a second memory.

⇒ **The aperture store (fake fb), the per-leaf join, the demand-fill mirror and the two-world
classifier are all deleted.** One reserved device-local RM object is all of guest video memory;
BAR1, BAR2, PRAMIN, channels and engines are all views of **it**.

★ §15's text below is kept because its *sub-findings* remain true and were expensive: that RM
writes page tables through BAR2 with the CPU, that UVM writes **its** tables with the copy engine
(w719b), and that a use-keyed rule would have corrupted. ⊘ **Its conclusion — two stores — does
not survive.** See `gpga_is_one_reserved_object.md` and `dirty_tracking_without_uffd.md`.

## 15, in full — the split that is easy to get wrong

> Owner: *"the clean split that pramin/bar2 is from fake fb and that bar1 is only mapping from the
> real guest vidmem… we discovered that **all fake fbs were only used in bar2 and pramin and not
> bar1/userspace channels, and none of the real userspace vidmem were used in bar2/pramin but only
> bar1/userspace channels**"*

| world | backing | serves |
|---|---|---|
| the reserved object | **ONE** device-local RM object, all guest vidmem | BAR1, userspace channels, engines |
| the aperture store | **ONE sparse memfd**, same advertised VRAM size, only touched pages resident (a few MiB) | BAR2, PRAMIN |

- Guest vidmem is **one reserved RM object**; channels map **slices** of it, chosen by the
  guest's own **PD*/PT*** entries.
- Those tables are **published and updated ONLY in the refresh function** — not on demand, not at
  a trap.
- The disjointness is an **empirical finding**, not an aspiration: it is what allowed the aperture
  backing to be stripped of GPGA positions entirely.

### ⊘ Naming — why not "GPGA real" / "GPGA control"

The owner's working terms are *GPGA real* and *GPGA control*. `control` is **already three things**
in this tree — the control **plane**, an experiment **control arm** (`KAYFABE_PREMAP_BAR1=0`), and
RM **control** commands (`NV2080_CTRL_*`) — so `GpgaControl` reads as at least two wrong things at
every call site. Proposed instead, and the split is then legible from the name alone:

- **`GpgaDevice`** — the reserved device-local object. What an ENGINE reads and writes.
- **`GpgaAperture`** — the sparse memfd. What the guest CPU pokes through an aperture, and
  nothing else. (`pramin_is_a_bringup_aperture_not_a_running_path` already calls PRAMIN exactly
  that.)

⊘ Avoid `GpgaShadow`: `gpga_is_one_reserved_object.md` uses "shadow" pejoratively for the
`install_join` mechanism it deletes, so the word already means "the thing that was wrong".

## Measured state, 2026-09-13 (rev `d8bc93bf`)

| constraint | state | evidence |
|---|---|---|
| 1 — no BAR1/BAR2/PRAMIN traps | **HOLDS**, both workloads | `TRAP_FILLS=0`; every fill was a premap install (w696: `1728+211 == premap 1939`; `5841+294 == premap 6135`) |
| 2 — BAR0 write-only | HOLDS except the counter page | ~132 reads at `+0xbb0000`; the device-view wire verb is unbuilt |
| 5 — DoorbellTable | wired | goal 4, w656–w660 |
| 15 — disjoint worlds | **NOT HONOURED** | one `BarMirror` + one store serves BAR1 **and** BAR2; the real-object join is per-LEAF, not per-BAR (`barmirror.rs:19-21`) |
| 15 — one reserved object | **designed, not wired** | `gpga_is_one_reserved_object.md` is STATUS: LIVE and says the per-leaf join *"is scheduled for deletion by it"*; `reserve_gpga` has no caller outside its own crate; no GPGA line in any boot |
| 4, 8 — sub-ms / off-vCPU | **HOLDS**, with the sanctioned PRAMIN exception | `VCPU-BLOCKING total=22 doors=3` and `PRAMIN-SLOT moves=22` — **the 22 doors ARE the 22 PRAMIN re-points**. `moves=22` is IDENTICAL under the raw client and CUDA (only `skipped` moves: 18439 vs 5448), so it is a BOOT-TIME set and the owner's ruling (*"297us for a thing that only happens at boot… thats fine for that mmap"*, `move_ns[worst=296558 mean=74698]`) is not expired |
| 7, 14 — epoll / threaded isolates | not built | |
| 12 — any die | not done | ~20 GA106 `ChipProfile` fields are per-die measurements |

## ★ Constraints 12 and 15 share ONE prerequisite (found 2026-09-13, w696d)

`ChipProfile` is a compile-time **`static`** (`ga10x.rs:1686`, `pub static GA106: ChipProfile`),
and the WPR2 addresses inside it are computed by `const fn` from a hardcoded `FB_SIZE_MB = 12288`
(`gsp_fw_wpr_end()`, `ga10x.rs:224`).

⇒ **Constraint 15** says *"the advertised framebuffer size is derived from the reservation that
succeeded, never asserted ahead of it"* — impossible against a `const fn`.
⇒ **Constraint 12** says no per-die constants — and `FB_SIZE_MB` is one, as are ~20 other
`ChipProfile` fields.

**Both need the same thing: `ChipProfile` constructed at START, not at compile time.** They are
one refactor, not two.

⚠ And the advertised number is already wrong by 2x: `[measured 2026-09-11, bare metal]` the
largest single vidmem reservation is **6144 MiB against 12288 advertised**. So today the guest is
told it has twice the video memory we can actually reserve for it.

### ★★★★★ `[measured w696f/w696g]` THE SIZE IS ALREADY A KNOB — one derivation, and it BOOTS

The owner, 2026-09-13: *"the reservation is giving actually as argument when kayfabe starts, how
much vidmem to give to the guest. manual from cli option is more important than auto detect."*

- Setting `FB_SIZE_MB = 6144` (the size this part can actually reserve) failed at **COMPILE**
  time, not boot: `assertion failed: GA106_BAR1_PDE_BASE < frts_offset()`. One captured absolute
  (`0x2_F1CA_C000`, a real RTX 3060's GSP value) did not scale, while the carve-out around it
  already did.
- Derived as a **relative** placement (`bar1_pde_base_for`, `0x20C_C000` above the carve-out
  base), reproducing the captured byte **exactly** at 12288. ⇒ After that single change the whole
  workspace builds at 6144 with **nothing else to fix**.
- `[measured w696g, branch `w696g-fb6144`, rev `a6274473`]` booting at **6144 MiB advertised**
  grades **`(P)`**, `MEAN_FALSIFIER=PASS`, `THREADS 8 of 8`, `TRAP_FILLS=0`.

⇒ **Rung 2 is plumbing, not risk.** What remains is a QEMU `DEFINE_PROP_UINT64` for the size,
through the shim ABI to a runtime `ChipProfile`, plus the reservation itself and *"if that fails,
the VM does not start."* ⊘ The `const` assertions did the enumerating for free — each one is a
per-die absolute declaring itself at compile time, which is also how constraint 12 should be
attacked.

### Proposed order (each rung independently green-able)

1. `ChipProfile` becomes a runtime-constructed value; `GA106` becomes a constructor call with
   today's constants as inputs. ⊘ **No behaviour change** — the same bytes, computed later. This
   is the load-bearing rung and the only risky one.
2. Reserve ONE object at start; refuse to boot if it fails; derive the advertised FB size from
   what succeeded and feed it into (1).
3. BAR1 served as SLICES of that object, published in the refresh function only.
4. BAR2/PRAMIN moved onto `GpgaAperture`; the two worlds disjoint BY TYPE.
5. Delete `install_join` and the per-leaf machinery.

⊘ Rung 1 is where constraint 12 starts too — the per-family derivation has somewhere to live only
once the profile is a value.

## ★★★★★ 16 — memslots are SETUP, and the tree already has the rule and breaks it

> Owner, 2026-09-14: *"memslots are largely a setup thing. you do it once reserve vmm ranges for
> bar0/1/2… so during runtime you actually translate bar1/2 (to gpa if not skippable) to vmm va,
> and use that in mmap or ioctls using va. same as nvkvm-pv and… mode 2 C."*

⊘ **This is already the tree's documented rule.** `window_unsafe.rs` quotes `l1_os_shell.md` §6.7
verbatim: **"One memslot per window (or per arena grant) — never one per published object."**

★ And PRAMIN already obeys it. The vCPU door census is that model working: `1 × mmap (creating a
guest-physical window)`, `1 × KVM_SET_USER_MEMORY_REGION`, then `20 × mmap MAP_FIXED (placing a
backing inside a window)` — **one** window, **one** memslot, and twenty re-points that touch KVM
not at all.

⊘⊘ **The BAR1/BAR2 mirror does the opposite.** `install_file_window(gpa, PAGE, …)` goes to
`install_window_inner`, i.e. **one window and one memslot per 4 KiB page** — each mirrored page is
exactly the "published object" §6.7 forbids giving its own slot.

`[measured w696ctl/w696h]` `slots peak=954` / `peak=920` for the **raw client**, and the premap
installs `6135` pages. ⇒ The memslot count scales with the touched working set, which is the
mechanism that would decide an LLM's fate — not `TRAP_FILLS`, which stays 0 either way.

⇒ **This supersedes the per-leaf coalescing idea below.** Coalescing 16 pages into one slot is a
16x improvement on a quantity that should be CONSTANT. Reserve the aperture once; place backings
inside it with `MAP_FIXED`; the slot count stops being a function of the workload at all.

## ⊘ Superseded: per-leaf coalescing (kept because the sub-findings still hold)

> Owner: *"can you not combine pages that are adjacent/consecutive to one single mmap range… that
> shouldn't be hard during refresh"*

Correct, and the groundwork is already there:

- **The install API already takes a length.** `install_file_window(gpa, PAGE, fd, offset, ro)` —
  a 64 KiB slot is the same call with `PAGE` replaced by the leaf length.
- **The backing is already contiguous.** The page arena is **address-indexed**: *"no free list, no
  bump cursor, no recycling. The address IS the offset."* Its docstring records the very problem
  this solves, already fixed once for PRAMIN: allocation-ordered offsets made *"a 1 MiB PRAMIN
  window 256 unrelated offsets, and could not be placed with one `mmap`."*
- **A leaf is contiguous in guest-phys by construction** — `DecodedLeaf{va, phys, size}`, which is
  what a 64 KiB large page means.

⇒ One memslot per 64 KiB leaf instead of **16**, with no dependency on the reserved object.
KVM memslots are a bounded resource with per-slot overhead, so one-slot-per-4-KiB is precisely
what would not survive an LLM-scale working set.

### ⚠ The one thing that makes it a semantics change, not an edit

`revalidate` iterates the mirror table **per page** (`s.page_off`). A coalesced slot therefore
needs 16 table entries sharing one slot id — dropping any page must drop the whole slot, and the
removal must happen once. That is contained, but it is a correctness-sensitive change to the
mechanism that currently delivers `TRAP_FILLS=0`.

⊘ **Order matters here**: do not touch the green mechanism before the measurement says it is
needed. The deciding number is `TRAP_FILLS` under an LLM-scale working set, which became
measurable only at w696 — before that the counter could not tell a trap from a premap install.

★ And when the reserved object lands, coalescing stops being per-leaf: slices of ONE object are
contiguous across leaves, so a run becomes one slot however long it is.

## ⊘ Three false violations in one session, all caught by opening the counter

Recorded because the pattern is the point, not the individual errors:

1. **BAR1/BAR2 "1728 traps"** — `fills` summed premap installs; `TRAP_FILLS` was 0.
2. **BAR1/BAR2 "5841 traps" on the raw client** — same counter, same error, second workload.
3. **"22 vCPU blocking doors"** — they are the 22 PRAMIN re-points, already measured at 297 µs
   worst and already ruled sufficient by the owner **on this same date**, with the ruling's expiry
   condition written down and NOT met.

★ Each one read as a regression of a goal the directive lists as complete. ⇒ **A violation claim
is a decision input and earns the same scrutiny as a green.** Two of the three had their answer
sitting in a doc comment or a dated ruling in the same file as the counter.

## ⚠ The instrument warning that governs this file

`[measured w696]` The BAR-mirror fill counter summed premap installs into a number whose own
docstring said *"every fill was ONE trapped access"*. It read as thousands of traps when the trap
count was **zero**, and it was reported as a goal-2 regression twice — the second time changing a
project decision. ⇒ **Before reporting any constraint as violated, open the line that CHANGES the
counter.** A name and a docstring are not substitutes, and this class has now cost w607, w627,
w695l and w696.

## ✔ 18 IS SATISFIED BY CONSTRUCTION UNDER THE SINGLE STORE (w721)

§18 forbade backing guest video memory with host system memory. Under one reserved device-local
object **there is no other memory to substitute**, so the constraint stops being a rule that can be
violated and becomes a property of the design. ⇒ Kept below as the statement of *why*, and as the
negative control (`the_all_dma_baseline.md`) that proves such a substitution is **correct in value
and wrong in residence** — now measured at **5.0x** (`the_llm_parity_ratio_is_0_20x`).

## 18 — GUEST VIDMEM IS VIDMEM: no silent sysmem substitution

> Owner, 2026-09-14: *"if the guest says this is in vidmem then it must be vidmem (our vidmem
> reserved RM object)"* … *"no secret DMA mappings as vidmem for now"*.

**The rule.** When the guest's own page tables place an allocation in video memory, the bytes live
in the **reserved device-local RM object**. We never satisfy a guest vidmem allocation with host
system memory that is DMA-mapped so the engine can reach it.

★ **Why this is a constraint and not an optimisation.** A DMA-mapped sysmem page presented as
vidmem is **correct in value and wrong in residence**. The guest computes the right answer, every
data-correctness test passes, and every engine access is a **PCIe round trip** instead of a local
vidmem access. ⇒ The defect is invisible to exactly the tests that would normally catch a backing
bug, which is why it survived: `--ce-client` passes either way.

⊘ **The one future exception, named so it is not confused with this.** **UVM managed memory** is
the single legitimate case for sysmem behind a vidmem-looking address: there the driver itself
migrates pages between host and device, and both residences are correct *by design, with the
driver's knowledge*. **We do not support UVM managed memory.** ⇒ Today there is **no** sanctioned
case, and nothing currently in the tree is one — what exists is not managed memory and was never
intended as it.

### ⚠ Measured state 2026-09-14 (w719): VIOLATED, and this is constraint 15's other half

`FbPageBacking::Joined` leaves are `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over a **memfd** ⇒ host
sysmem. `plane.rs:2543-2558` routes `FbWindow::FbAperture` (BAR1) through the same
`s.fb.page_backing(phys, …)` as PRAMIN and BAR2, so **every guest vidmem page is host RAM**.

★ The fix is the same fix as 15 and 16, and the code for it already exists on both sides:
`HostRmBackend::export_device_view` arms a node over an `alloc_vidmem` object (proved by
`rmladder --bar1-crossing`), and `QemuVmm::install_device_window` places it. What is missing is
named in that function's own doc comment: *"the mirror that walks the guest's BAR1 page table and
drives this verb is the remaining work."*

⊘ **`export_backing`'s `NotExportableAsMemory` refusal is not evidence against this.** It refuses
`ExportSource::HostDeviceMemory` because a memfd can be a KVM memslot and device memory cannot —
a constraint that only binds under the memslot-per-page design **constraint 16 retires**. Under 16
the runtime path is a translate + `mmap` into a pre-reserved VMM range, which device memory serves
fine.

### The falsifier this needs

⚠ **Do not grade this on data correctness.** A coherent shared-sysmem BAR1 returns the right
bytes, so `--ce-client` is green before and after. The test must measure **residence** —
bandwidth, or the aperture the engine actually reached — or it cannot go red today.

## ⊘⊘⊘ w719b — THE JUSTIFICATION FOR §15 WAS HALF WRONG; THE RULE SURVIVES AND IS STRONGER

Earlier the same day this file recorded, as settled, that *"ogkm does not use CE on PT\*/PD\*
pages"* — offered as the finding that makes the aperture store safe to keep out of the engine's
reach. ⊘ **That is true of RM's GMMU walker and FALSE of `nvidia-uvm`**, which has its own walker
and its own allocator and, on Ampere, writes its page tables **with the copy engine**:
`ce_hal->memcopy` (`uvm_mmu.c:432`), `ce_hal->memset_8` (`:462`), reachable because
`uvm_ampere.c:58` sets `ce_phys_vidmem_write_supported = true`, which makes `uvm_mmu_use_cpu()`
false (`uvm_mmu.c:269-276`).

### ★★★ Why §15 survives — and why a USE-keyed rule would have corrupted

| whose tables | written by | reached through | world |
|---|---|---|---|
| **RM**'s | the CPU | BAR2 | the **aperture store** ✓ |
| **UVM**'s | the **copy engine** | a GPU VA — never a CPU aperture | the **reserved object** ✓ |

⇒ A rule phrased as *"page tables live in the fake fb"* would put UVM's CE-written tables in a
memory **no engine can reach**. The rule that survives is the owner's, keyed on the **aperture**:
whatever is reached through BAR2/PRAMIN is the aperture store — because the aperture is
**observable**, and *"is it a page table"* is not a sufficient classification.

★ The fake fb still never needs engine visibility. That is now a **consequence of the aperture
rule**, not of a general claim about page tables.

⚠ Two corrections of record: `_gmmuWalkCBFillEntries` uses **`TRANSFER_FLAGS_SHADOW_ALLOC`**
(`gmmu_walk.c:804`), not `TRANSFER_FLAGS_NONE` — right conclusion, wrong pointer, this tree's
named recurring defect. And `research_clones/ogkm` is **`610.43.02`** (`version.mk:1`), not 580.

⇒ **The lesson: a negative over a codebase needs its SUBSYSTEMS enumerated first.** `grep` found
no CE in `gmmu_walk.c`, which was true; the question was never only about that file.

★ Sizing, trigger and exhaustion behaviour for the aperture store:
`the_aperture_store_lifetime.md`. ⊘ It **refutes the ~40 MiB bound**: BAR2's dynamic window is
**16 MiB** on a GA106 and is an **evicting LRU cache**, so residency is not liveness, and 12 GiB
mapped at 4 KiB needs **24 MiB of small page tables alone**.

## ⊘⊘⊘ SUPERSEDED w721 — NOTHING IS CLASSIFIED, BECAUSE LIVENESS IS DERIVED

§19 built a per-address classifier with revocable leases, because we had to decide which world a
page belonged to and when that decision expired. ⇒ **With one store there are no worlds to classify
into, and with a from-root walk each refresh, reachability is RECOMPUTED rather than remembered.**

| §19 needed | why it is gone |
|---|---|
| the parent-PDE-invalidate death signal | reachability is recomputed every refresh |
| the lease + revoke | nothing is remembered, so nothing must be revoked |
| the hole-punch trigger | *"reachable last refresh, not this one"* **is** the answer, observed |
| RM's recycle-changes-role hazard | the walk sees what the pointers say **now** |

★ §19's **safe-by-default** argument survives and generalises: the errors were asymmetric
(misfiled data ⇒ silent corruption; misfiled control ⇒ merely slower). That asymmetry is why the
design defaults to real video memory everywhere.

## 19 — CLASSIFY PER ADDRESS, DEFAULT TO VIDMEM, AND LEASE THE CLASSIFICATION

> Owner, 2026-09-14, on learning UVM CE-writes its page tables: *"if nvidia uses ce for page
> tables then we can still support. the driver never allocates page tables and data on same
> address. so the solution is simply to determine per address what the target needs. since its
> emulated channel only, means that passthrough remains untouched."*

★ Accepted, and it converges with `gpga_is_one_reserved_object.md`'s own rule — *"backing … is
decided by who reads it"*, learned when a range is mapped into a GPU address space. ⊘ Passthrough
is genuinely untouched: those channels operate on the reserved object, which is real video memory
either way.

Two refinements, both from measured facts rather than taste.

### ★★★ (a) The DEFAULT is the reserved object — because the errors are asymmetric

| misclassification | consequence |
|---|---|
| data → aperture store | an engine reads memory it cannot reach ⇒ **silent corruption**, no fault, no status |
| control → reserved object | **correct**, merely slower for CPU reads |

⇒ **Reserved object by default; the aperture store requires POSITIVE EVIDENCE.** The aperture
store is an optimisation applied where it is provably safe, never the bucket things fall into by
where they happened to be touched. Every classifier bug then costs milliseconds, not correctness.

⊘ It remains load-bearing as an optimisation, so this is not an argument for dropping it: CPU
reads of video memory are **48 MiB/s** against **3674 MiB/s** for host memory, and re-reading page
tables every refresh costs **~10 minutes a boot** versus ~150 ms promoted.

★ Evidence required for the aperture store: reached through BAR2/PRAMIN, **and** never mapped
into a GPU VAS, **and** never named as a decoded CE operand.

### ★★★ (b) The classification is a LEASE, not a label

The premise *"the driver never allocates page tables and data on same address"* holds **at any
instant**, which is what matters — but not **over time**. RM recycles page-table pages through a
packed sub-memdesc cache (`gmmu_walk.c:578`) and a memory pool (`:586-588`), and **non-root levels
are not scrubbed at allocation** (`gmmu_walk.c:341-343`). ⇒ An address that is a page table now
can be user data later **with no write in between**.

⇒ A classification must be **revocable at an observable event**, and the event is already known:
the **parent PDE going invalid**, which strictly precedes the free (`mmu_walk.c:1514-1552`).

★ Conveniently that is **one event doing three jobs** — it retires the classification, punches the
aperture store's backing (`PageArena::punch_range`), and releases the lease. Actioned at a
synchronisation point, never mid-walk.

### ⚠ What must be MEASURED before the migration path is designed

If a page is reclassified after it already has bytes in the wrong world, something must move them
— the **shadow** `gpga_is_one_reserved_object.md` exists to abolish. Whether that is a loud
refusal-and-reclassify at a synchronisation point or a real migration path depends entirely on how
often it happens. ⇒ `kayfabe_device::twoworlds` measures exactly that, and it is already wired to
print at teardown. **Boot first, design second.**

## ★★★★★ w720h — WHAT THE SINGLE STORE DELETES FROM THE THREAT MODEL

> **Owner, 2026-09-14:** *"one vidmem rm object was simply best idea. with fake fb deleted later,
> entire DoS bugs just disappear that were hard to patch. we only have to limit workers, isolates,
> va tables, all simple bounds."*

★ Recorded because it is the **security** argument for the design, and it is stronger than the
code-deletion one.

### The precise reason

**The fake fb is the only place where a cheap guest action causes UNBOUNDED host allocation.**
Everything else in the system is a countable thing with an obvious cap.

⇒ That asymmetry is why the quota question had no good answer: you cannot bound *"framebuffer
pages the guest touched"* without breaking legitimate use, because the guest **legitimately**
expects the whole advertised VRAM to work.

★★★ **The vulnerability is the GAP BETWEEN ADVERTISED AND BACKED.** We advertise 12 GiB and back
it lazily; the exploit lives in the laziness. **One pre-allocated reservation closes the gap by
construction** — the guest cannot consume more than was allocated before it booted. ⇒ The bound
stops being *enforced* and becomes *structural*, which is the only kind that cannot have a bug.

### The amplification shapes that stop existing

| shape | today | after |
|---|---|---|
| `JoinFbLeaf` — one guest touch ⇒ an RM allocation + mapping | ✔ live, `n=286`–1156 a boot | **gone**, no per-leaf allocation |
| arena page materialisation on demand | ✔ | **gone** |
| mirror slot install/churn | ✔ | **gone**, one static mapping |
| sparse-memfd residency growth | ✔ unbounded by design | **gone**, fixed reservation |

⇒ What remains is **counters**: workers, isolates, VA spaces. *"At most N"* is testable and
obviously correct.

### ⚠ TWO THINGS THAT DO NOT DISAPPEAR — so the win is not remembered as bigger than it is

1. ★★★ **The walk kernel is new attack surface, and the sharpest we have had**: guest-authored
   pointers dereferenced **on the GPU**, where a hang is a DoS on our own scratchpad and debugging
   is worst. The invariants (fixed trip count, bounds check, capped output) are **designed, not
   proven**. ⇒ That is exactly why the hostile suite is being built before the kernel goes near
   production.
2. ⊘ **A guest can still make refreshes EXPENSIVE without exhausting anything.** Mapping
   everything at 4 KiB makes the walk scale with table size (24 MB instead of ~7). Bounded and
   never fatal, but not free. ⇒ It changes **category** — exhaustion becomes **rate** — rather
   than disappearing.

⊘ And unaffected: the **50x bulk-placement defect** (`to_device`, 0.8 host cores for 28 s,
`the_llm_parity_ratio_is_0_20x`) is the same amplification shape — guest copies memory, we burn
host CPU — and the single store does not touch it.

## 20 — THE GPU WALKER: one kernel, three structural invariants, a fallback at every layer

**Owner, 2026-09-14:** *"why not let the PTX do the walk? and tell our program what to map?"*

A CUDA kernel in the **scratchpad isolate** walks the guest's page tables from the root each
refresh and returns the mappings. Built and tested: `cuda/walk/`, **50/50** hostile cases,
differential-agreeing with `kayfabe-mmu`'s walker on **1212 benign leaves**.

### ★★★ The three invariants — structural, never probable

1. **No loop terminates on guest data.** Depth is format-bounded ⇒ **fixed trip counts**, never a
   data-dependent `while`. ⇒ A cycle is **harmless, not detected** (`TOO_DEEP` is unreachable by
   construction, and every hostile case asserts its *absence*).
2. **Every dereference is preceded by a bounds check** against `gpga_len`. One compare, no
   exceptions.
   ⊘⊘⊘ **MEASURED, not assumed:** a negative control (`-DKF_BREAK_BOUNDS`, deleting only the two
   checks) made one hostile case raise *"an illegal memory access"* — and the other **silently
   return data from beyond the window**, reporting a mapping at `gpga=0xdead000`. ⇒ **An
   out-of-bounds read does not reliably fault.** The check is load-bearing, never belt-and-braces.
3. **Output is capped and truncation is LOUD**, forcing a full resync — never a short report that
   reads as whole. ★ This is the third hazard class the other two miss: a **legal but enormous**
   tree (12 GiB at 4 KiB ≈ 3M entries, every pointer valid, no cycles) — what a well-formed
   hostile guest actually uses.

### Placement, and why it is not the VMM

⊘ **CUDA draws no boundary between a host process and the kernel it launched** — the GPU MMU
separates *contexts*, not a kernel from its own context's mappings. ⇒ The kernel runs in the
**scratchpad isolate**, which is **root-in-the-guest, unprivileged-on-host**: exactly the privilege
of the data it processes, so it **cannot escalate**. `libcuda` is initialised (through
`cuModuleLoadData`, where the PTX JIT runs) **before** the isolate drops privilege; no other
isolate loads CUDA. **One walk isolate per VM.**

⊘ It is **not code injection**: the PTX is ours, built at build time. The bug class is **memory
safety over guest-authored data** in ~200 auditable lines.

### Every layer degrades into the one below

scope hint absent/ambiguous → **full walk** (~67 µs) · kernel unavailable → **batched CE** · no
range from the invalidate → **whole PDB**. ⇒ The hint can only make the walk **faster, never
wrong**.

## 21 — ONE CUDA PROGRAM, TURING THROUGH BLACKWELL, WITH THE FORMAT AS DATA

**Owner, 2026-09-14, three times:** *"our ptx must be Turing+ compatible"* … *"you need to support
both the turing/ada page tables as blackwell table, in same kayfabe, so also in the C walker. I
would avoid shipping two cuda program."*

| | status |
|---|---|
| compile target | `-gencode arch=compute_75,code=compute_75` — **PTX only, no cubin** ✔ |
| the claim is checked | `make check-ptx` fails if `cuobjdump -sass` finds any `code for sm_` ✔ |
| arch-specific intrinsics | **none** ✔ |
| forward-JIT demonstrated | on sm_86 from the `compute_75` target ✔ |
| **actual Turing silicon (sm_75)** | ⚠ **never run** — the floor we claim |
| **page-table format** | ⊘ **VER2 only = Turing→Ada. Hopper/Blackwell are VER3** |

⇒ ★★★ **The architecture limit is the FORMAT, not the PTX.** And it decides a sequencing rule:
the host walker is **format-polymorphic** (`fmt: &dyn GmmuFmt`), the kernel is not. **Add the
format seam to the kernel BEFORE deleting the host parsing**, or the deletion silently caps the
product at Ada — with Blackwell being goals 1 and 10 of the directive.

### The format is SETUP DATA, not code

**Owner:** *"that kind of config you already derived from ABI on host and also with runtime
userspace data is perfect to pass. I would call this setup data alongside table version."*

⇒ **No bit position lives in the kernel.** The host derives the layout from the `GmmuFmt` impls it
already maintains and uploads it; the kernel holds the **algorithm**, not the layout. One uniform
branch on `table_version` covers what field offsets cannot (VER3's PCF) — and it is **warp-uniform**,
so it costs nothing.

- **`KfSetup`** (once per VM, immutable): `abi_version`, `table_version`, `levels[]`,
  **`gpga_base` / `gpga_len`** (invariant 2's bound, as derived config), `page_sizes`,
  `max_entries`.
- **`KfLaunch`** (per refresh): `root_pdb`, `scope[]`, `out`, `run_capacity`, `generation`.

★ This directly serves `derive_per_die_maintain_per_family`: a new die is a new descriptor, **no
kernel change**; a new format is a descriptor plus one arm in the switch.

⚠ `abi_version` is refused if unknown — a Rust/PTX skew must fail **loudly at launch**, not decode
garbage field offsets and look like a page-table bug.

⊘ **Keep an independently-written table builder in the TESTS.** `cuda/walk/kf_tables.h` was
written from `dev_mmu.h` sharing no code with either decoder, which is what made the 1212-leaf
differential meaningful. Production reads one descriptor; the oracle stays independent.
