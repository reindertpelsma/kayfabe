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
   ★ `[w740]` **The scrub in question is 8 bytes in two GPFIFO submissions**, measured three
   independent ways (the guest's own `memmgrMemSet(… sizeof vidmemData …)`, `ogkm-580`'s
   `NvU32 vidmemData`, and our own decode of the captured pushbuffer's
   `LINE_LENGTH_IN = 0x4`). ⊘ So *"a scrub"* here is **not** framebuffer-sized and does not
   need `device_reset`; the `scrubberConstruct` path that could have been scrubs **zero**
   bytes at init and registers for scrub-on-free instead. ⚠ And the **scope of "on
   scratchpad" is still open**: w740 executes the bytes with a CPU store **through a view
   the scratchpad isolate exported, into the scratchpad's own reserved object**, which meets
   *"not forged"* and the positive clause's *"uses the scratchpad"*, and does **not** meet
   w735's gloss of this constraint as *"the copy engine off the CPU"*. `CeExecutor::HostCe`
   is the arm that would, and `ce_copy` refuses a `CeSource::Constant` today
   (`kayfabe-isolate-host/src/rm.rs:8472`, `NOT_ON_THIS_RUNG`) — named here so the gap is a
   known one rather than a silent reading.
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
18. **Guest vidmem is vidmem** — no silent sysmem substitution. ✔ **Satisfied by construction**
    under the single store: there is no other memory to substitute (§18).
19. ⊘ ~~Classify per address, lease the classification.~~ **SUPERSEDED** — nothing is classified,
    because a from-root walk **recomputes** reachability rather than remembering it (§19).
20. **The GPU walker**: one kernel, **three structural invariants** (no loop terminates on guest
    data · every dereference bounds-checked · output capped with loud truncation), and a fallback
    at every layer. Runs in the **scratchpad isolate**, never the VMM (§20).
21. **ONE CUDA program, Turing through Blackwell, with the format as SETUP DATA** — no bit
    position in the kernel; a new die is a descriptor, a new format a descriptor plus one arm
    (§21).
22. **We do not lie about the aperture**, and it is **SYMMETRIC**: vidmem is vidmem **and sysmem
    is sysmem**. If the guest explicitly asks for DMA-mapped system memory it gets real host
    memory (§22, §w724c).

23. **★★★ THE MEMSLOTS FOR BAR0/BAR1/BAR2 ARE INSTALLED BEFORE THE GUEST DRIVER TOUCHES THE
    DEVICE — there is NO demand-fill path for BAR1/BAR2** (owner, 2026-09-15). A memslot exists
    so the guest's loads and stores go straight to memory **with no VM exit**; ⇒ in the intended
    steady state the BAR1/BAR2 **trap census is EMPTY BY CONSTRUCTION, not small**, and a trap
    on those windows **is itself the error**. Shape: BAR0 is 2–3 adjacent memslots (several
    **read-only with writes trapped**, plus one r/w for PRAMIN); BAR1 is one memslot whole;
    BAR2 is one memslot whole. ⇒ **In production only BAR0 write traps remain; the rest is
    discarded forever, with no fallback.**
    ⊘ **REFINEMENT, and it is load-bearing: the hook is the BAR *map* callback, not VM start.**
    BAR GPAs are not known before PCI enumeration — QEMU's PCI layer assigns them and the guest
    may **reassign** them (Linux does). So: installed in `pci_update_mappings`, before the guest
    driver touches the device, **and re-installed when a BAR moves**. With no trap, a guest that
    moves its BAR onto a stale memslot fails **invisibly**.
    ⊘ **And a read-only memslot converts "read trap" into "the shadow page must be correct at
    ALL times."** Registers with read side effects — clear-on-read, FIFOs, the free-running
    counter — cannot live in one; they would serve stale values with no way to notice. `[w590–
    w609]` took BAR0's read surface from 184 585 reads to **134**, *"one page from zero"*, with
    only the free-running counter left ⇒ the shape above is reachable, possibly with that one
    page still trapped.

24. **★★★ THE GPA RANGES FOR BAR0/1/2 ARE UNIQUELY RESERVED FOR OUR DEVICE, AND ARE NEVER
    HARDCODED** (owner, 2026-09-15). No other QEMU device may claim them. We **read back where
    the BARs landed** rather than assuming, and re-read when they move. ⇒ A hardcoded range is
    a collision waiting for a second device, and under constraint 23 that collision is
    **silent**.

25. **★★★ THE vCPU MMIO HANDLER HOLDS NO CAPABILITY THAT CAN BLOCK — blocking is UNSPELLABLE,
    not disallowed** (owner, 2026-09-15). *"Code in such a way you can't make the 44 ms worst
    trap ever again. If the vCPU MMIO handler is just one write on BAR0 without being able to
    add another functionality, on top of a simple queue, then getting it beyond that is just
    impossible."*
    ⇒ The handler receives **a queue handle and nothing else**: no `/dev/nvidia` fd, no isolate
    socket, no VM fd, no `ViewSpace`, no allocator. Then `mmap` is not *forbidden* — it is
    **uncallable**, because one cannot `mmap` without an fd; IPC is uncallable without the
    socket; `KVM_SET_USER_MEMORY_REGION` is uncallable without the VM fd. Every door in the
    w742 census becomes a **queued job by construction**.
    ⊘ **Reads:** a read must return a value, so it cannot merely queue — but under **23** BAR1/
    BAR2 reads are premapped and never trap, so a trapping read *is already* the error. The
    handler answers reads from the shadow page and holds nothing further.
    ★ **GATE, and it must GATE rather than report:** the vCPU allowlist goes to **EMPTY**, and a
    **non-empty allowlist is a BUILD FAILURE**. ⊘⊘⊘ **This is the correction of a measured
    failure, not a new idea.** `kayfabe-util/src/lock.rs`'s `the_vcpu_allowlist` already exists
    and its own test is named `blocking_inside_a_trap_is_recorded_and_says_whether_it_was_
    allowlisted` — **recorded**, with a list you may append to. ⊘ **CORRECTED w751: 25's GATE IS
    AIMED AT THE WRONG CENSUS.** `lock.rs`'s `BlockingSection` allowlist has **zero production
    call sites** and saw **none of the 197**; those are `lockwitness` doors, which have **no
    allowlist at all** (only `KAYFABE_VCPU_BLOCK_FATAL`). ⇒ emptying the allowlist would empty a
    list that is **already empty**; the gate must be built on `lockwitness`. ⚠ And
    `kayfabe_shim_regs_read` never calls `mark_vcpu_thread` (only the write path does), so **a
    door reached from a READ on a thread that has not yet written is INVISIBLE** — fix that
    first, or the census undercounts by construction. `deviceview.rs:1236`/`:1265` (was `:1197`) says it
    outright: *"blocked inside an MMIO exit, which `assert_not_on_vcpu` **only REPORTS**."*
    The allowlist grew **3 doors → 9** and **22 crossings → 197** and nothing failed. ★ This
    campaign's most-repeated lesson, applied to itself: **a check that reports is not a check
    that gates.**
    ⚠ And note WHY the lock discipline did not catch it: every increment satisfied the **lock
    rank** — `want` under the plane lock, `drain` only at **lock-free** entry points — and
    **LOCK-FREE IS NOT OFF-vCPU.** The lock-free points chosen are still on the vCPU thread, so
    constraint 9's discipline held perfectly while 4, 6 and 8 broke behind it.
    ⇒ **The remedy reuses built machinery:** the `want`/`drain` split already exists and is
    tested.
    ⊘⊘⊘ **CORRECTED w751 — "constraint 6 is exactly *move `drain` to a worker*" IS REFUTED, AND
    IT WAS MINE.** `drain` **already declines by name on a vCPU** (`deviceview.rs:915-922`,
    `plane.rs:3673-3682`; measured `declined_on_vcpu=14`) and **contributes ZERO of the 197**.
    The `want`/`drain` split is sound and was never the problem.
    ★★★ **All nine doors and all 197 crossings are ONE path**: `repoint_pramin`
    (`barmirror.rs:1630`) doing **release-and-re-arm** — `20 × 7 + 19 × 3 = 197`. And
    `cpu_of_that_trap=43390us` of 44440 ⇒ **the 44 ms is CPU, not an RM wait.**
    ⇒ The fix is **deletion, not relocation**: re-point the **existing** window in place with one
    `MAP_FIXED` (doors 4/6/7/8 vanish), park the release on the worker's reclaim tick (door 9),
    and doors 1–3 + 5 stay. Predicted **197/9 → 100/4 → 22/1, ~0.36 ms**.
    ⚠ **`l1_os_shell.md` §6.7 rule 3 and constraint 16 ALREADY FORBADE slot delete/recreate** —
    we were violating a rule we had written. The conflation that built the churn: *"a device view
    cannot be re-pointed"* is **true of the NODE, false of the WINDOW**.
    ⊘ **And this does NOT make `worst_trap` sub-millisecond.** The control arm's **22 ms at
    `NV_PGSP_QUEUE_HEAD`** is a SECOND constraint-4 violator that passes through **no
    `assert_lock_free` door at all** — the door census cannot see it.

> ### ⊘⊘⊘ **CORRECTED 2026-09-16 (w754) — `NV_PGSP_QUEUE_HEAD` WAS NOT THE VIOLATOR. IT WAS A
> ### BYSTANDER, AND IT HAD BEEN ONE SINCE w432.** Read this before acting on the line above.
> The paragraph above, `fable_off_vcpu_design.md` row 3 and `boot.rs:945`'s own comment all
> read `worst_trap … at=bar0+0x110c00` as *"the queue-head write drains and services the GSP
> command queue inline"*. **It has not done that since w432**, and `defer_commands` has
> survived a guest device reset since **w472b** (`boot.rs:828-846`, itself the correction of
> exactly that failure). Three refutations, all from w752's own logs, none needing a boot:
> `GSP-ASYNC ARMED` at realize on every arm · `LOCKCOST rank0 worst_wait=8790us
> worst_hold=748us` (a 25 ms trap cannot be that lock) · `cpu_of_that_trap` is
> `CLOCK_THREAD_CPUTIME_ID` **on the trapping thread**, so 96 % CPU means the vCPU *ran*.
> ★★★ **What it actually was:** `Regs::write` still called `adopt_pending_channel_rings()` on
> the vCPU, gated only on `pending_latch_epoch()` moving — and that function's FIRST act is
> three guest page-table settlement passes. It ran on **whichever register write noticed the
> latch**, and `0x110c00` is the register the guest writes most during driver init.
> ⇒ `[measured w754, GA106, vast 51217315]` with the settlement moved to the doorbell worker:
> `bar0+0x110c00` **24 999 µs → 1 579 µs**, and four more "slow registers"
> (`0xb81408/0410/1608/1610`) left `SLOW-SITES` entirely — they were the same code at a
> different address. Proved by one env var on the same binary (`KAYFABE_MATERIALIZE_INLINE=1`
> forces it back): 1 trap @1 579 µs vs 3 traps @15 336 µs, `doors 6` vs `doors 9`,
> `inline_exceptions 0` vs `5`.
> ★★★★★ **AND THE WORST TRAP WAS A THIRD THING AGAIN**, on all three arms:
> `bar0+0x110118` at **45–53 ms, 100 % CPU, zero context switches** — a `OnceLock<Vec<u64>>`
> initialising `(0..regs_aperture_len).step_by(4).filter(decode_reg)`, **4 194 304 evaluations
> over a 16 MiB aperture**, inside `publish_gsp_registers`, named by one stall-alarm boot.
> ⊘⊘⊘ **w573 had already fixed the identical sweep three functions away** (`dead_pages`, *"4.2
> million predicate evaluations under a halted guest"*) — **a fix applied to an instance leaves
> the class.** Warmed at realize: **control arm 25 077 µs → 920 µs, `slow_traps(>1000us)=0`.**
> ⊘ **Constraint 4 is STILL VIOLATED on the device arm, and the violator is a DIFFERENT KIND:**
> `worst_trap=9649us at=bar0+0xbb0090` (the usermode doorbell) with `cpu_of_that_trap=4621us`
> — **48 % CPU, 52 % WAIT** — `LOCKCOST rank0 worst_wait=9625us
> worst_wait_blocked_by=plane.rs:3286` (`window_page_backing`, which takes **both** `state` and
> `mem`). ⇒ **the next cut is LOCK SCOPE, not thread placement.** Moving more work off the vCPU
> cannot help a trap that is already waiting on a lock a worker holds.
> Full record: `w754_gsp_queue_off_vcpu.md`, `traces/w754_gsp_queue_off_vcpu/`.

> ### ✔✔✔ **MEASURED 2026-09-15 (w746) — THE HAND-OVER WORKS; RM REFUSES THE DUP `NV_ERR_INSUFFICIENT_PERMISSIONS`, AND THAT IS CONSTRAINT 30 ANSWERING FROM HARDWARE.**
> `[vast 51155860, GA106, 580.159.04 OPEN, TREE_REV 0c0dd2ce, three arms, one binary;
> `traces/w746_handover/`]` Control **`(P)` 8/8**; arm 2 reproduces 17/11/19/0; arm 3 **`(R)` 0/8**.
> ★★★ `HANDOVER-ASSERTS asked=4637 leaf_untabled=0 space_not_held=0 ⇒ ASKED AND PASSED`. The
> hand-over **ran 4637 times**, minting and committing a bare space each time, and every
> constraint-29/30 assert was exercised without firing (`ROUTE-DISAGREES=0`, `C30-REFUSED=0`,
> `C30-BIRTH-REFUSED=0`, `RING-HANDLE-REACHED-RM=0`). ⊘ These are *asserted-and-passed*; the
> `asked=` term is printed beside them precisely so they cannot be read as w745's vacuous zeros.
> ⇒ **THE NEW WALL IS `adopt_vaspace`**: `adopts=0 adopt_refused=4637
> first_refusal=[Rm("InsufficientPermissions")]` — `NV_ESC_RM_DUP_OBJECT` issued for the first
> time in this campaign and refused by RM.
> ★★★★★ **ogkm names the gate** `[ogkm-580.159.04 rs_client.c:537-551, clientCopyResource_IMPL]`:
> a **cross-client** dup by a client below `RS_PRIV_LEVEL_KERNEL` needs `RS_ACCESS_DUP_OBJECT`
> **granted on the source object**; `rsAccessCheckRights` ends `NV_ERR_INSUFFICIENT_PERMISSIONS`
> (`rs_access_map.c:540`). We grant nothing. ⇒ the missing piece is an **explicit share** by the
> per-proc isolate on its own VA space — `NV0000_CTRL_CMD_CLIENT_SHARE_OBJECT` (`0xd06`).
> ⊘⊘ **AND THIS ANSWERS CONSTRAINT 30 FOR THIS RESOURCE, FAVOURABLY.** 30 worried that euid 0
> makes `rmclientIsAdmin` hold for the scratchpad ⇒ effectively privileged. **Measured false**:
> `rmclientIsAdmin` yields `RS_PRIV_LEVEL_ADMIN`, the gate demands `>= RS_PRIV_LEVEL_KERNEL`,
> and **ADMIN < KERNEL** — RM treated our scratchpad as an ordinary user client. ⚠ It also
> SCOPES `[w744]`'s `NV_OK` on the same dup: that probe ran both clients in ONE process; this
> crosses two isolate processes. **A dup that succeeded once says nothing about a dup between
> different parties.**
> ⊘ Still unmeasured after two boots: `map_store_slice`, constraint 28 on hardware, leg B, and
> constraint 30 part 2's mapping question. `maps=0` for a second boot, now for a new reason.
>
> ### ⊘⊘⊘ **CORRECTED 2026-09-15 (w746) — THE CAUSE NAMED BELOW IS WRONG.** Read this first.
> The block below says the 4619 refusals were *"a routing key re-derived from a placeholder"*.
> **They were not, and `Pdb(0)` is not a placeholder.** `project.rs`'s `pdb_claims` makes two
> live VASpaces sharing a base a hard `ProjectionError::PdbCollision`, so `route_pdb` resolved
> it to proc 2 correctly — the same proc whose census produced the leaf three lines earlier in
> the same function. The failing conjunct is the next one, `Vas::host_vas == None`, and it was
> `None` because of a closed loop: `host_vas` is minted only by a commit that SUCCEEDED, the
> only verb that runs on the device arm is `EngineObject` (`VERBCOST … [EngineObject n=8 …
> 100.0%]`), it births a channel whose ring is `RingSource::Ours` and therefore **maps its own
> ring through the space**, and on the scratchpad arm that space is bare. Every engine object
> was refused (`forwarded=0 refused=10`, against `forwarded=8` on arm 2, same binary) and its
> fresh space unwound. ⇒ **a repair gated on the success it repairs.**
> ⊘⊘ **And `W745-BARE-SPACE-REFUSED=0`, the row w745 used to retire this very diagnosis, was
> VACUOUS**: `alloc_channel_in` reaches RM through `raw_map_dma`, which carried no bare-space
> refusal, so the counter could not increment whatever happened. RM answered `0x51`, which is
> indistinguishable from exhaustion. Fixed at w746 — the refusal now sits in
> `raw_map_dma_slice`, the one site that builds `NVOS46_PARAMETERS`.
> ⇒ w746 makes the hand-over **ensure** the space (`alloc_vaspace_bare` when `host_vas` is
> `None`, with a real commit phase), takes the caller's `ProcId` instead of re-deriving it, and
> attaches the three replacement asserts constraint 29 obliges plus constraint 30's.
> ⚠ **Still true below, and unchanged by this correction:** everything after *"What did hold on
> every arm"*, and the whole USERD paragraph — leg B is untouched and remains the owner's.
>
> ### ◐ **BUILT AND BOOTED — 2026-09-15 (w745). IT ARMS AND REFUSES AT THE HAND-OVER.**
> `[vast 51149807, GA106, 580.159.04 OPEN, three arms, one binary]` The control holds at
> **`(P)` 8 of 8**; the split arm is **`(R)` 0/8** with `handovers=0 handover_refused=4619`,
> every one `NoVas(ChanId(0))` under a placeholder **`pdb=Pdb(0)`**. ⇒ the hand-over
> **re-derives a routing key its caller already holds**, and the publish route's pdb is `None`
> until a `SetPageDir` arrives. ⊘ `adopts=0` — the dup was never issued, so `adopt_vaspace`,
> `map_store_slice`, the slice binding, the ring oracle and **constraint 28 on hardware** are
> all unmeasured. `RING-NOT-A-SLICE=0` is VACUOUS, not a pass.
> ★ What did hold on every arm: `TRAP_FILLS=0`, `misses=0`, `RmInitAdapter failed!=0`,
> `SMI_RC=0`, `HOST_DMESG_XID=0`, and the vCPU guard declined by name **14 times without
> panicking**. ⊘ **Constraint 25 gained nothing**: `VCPU-BLOCKING total=197 doors=9` on both
> the `isolate` and `scratchpad` arms, identical to w742.
> ⊘ Previously: **BUILT, NOT BOOTED — 2026-09-15 (w745), branch `w745-ownership-split`.** Bare per-proc
> `FERMI_VASPACE_A`s (`--bare-vaspaces`), the hand-over (`vaspace_handover`, tag 38), the dup
> (`adopt_vaspace`, tag 35), `map_store_slice` (tag 36) over `NVOS46::offset`,
> `FbLeafBacking::StoreSlice` binding a `HostBacking::slice`, and `StoreMapPort` as the VMM's
> one door. Behind `KAYFABE_VAS_OWNER`, whose off position is byte-identical to w743.
> ★ **F11 IS SCOPED BY A TYPE and the gate is green on its merits** — `mod handed_vaspace`
> (`ScratchpadRole` + `HandedVaSpace`, private fields, one constructor each), `APPROVED_RHS`
> **unchanged**, no verb moved out of `rm.rs`; what grew is the *universe* (a `*_src` client
> field is a different question from a destination one) with its own derived approved set and
> its own floor.
> ⊘⊘ **AND A SECOND RULING IS OPEN AND WAS NOT MADE: THE USERD.** `h_userd_memory_0` **is** a
> real RM operand, unlike the ring handle, so a per-proc client cannot name the scratchpad's
> object for it. This branch declines leg B for a store slice **by name**, which is the
> pre-leg-B channel — the guest advances `GP_PUT` in its own page and RM reads ours. ⇒ the raw
> client is **predicted not to pass** on this arm. The three ways out are tabled in
> `SINGLE_STORE_PLAN.md`'s w745 block; picking one is B1-shaped and is the owner's.
26. **★★★ THE OWNERSHIP SPLIT — the isolate borrows, the scratchpad holds** (owner,
    2026-09-15). *"All memory is held by the scratchpad, the userspace isolates only borrow
    from it."*

    | | **owns** | **sees** |
    |---|---|---|
    | per-proc **isolate** | channel, VA space, compute, control | **only its own VA space** |
    | **scratchpad** | the vidmem GPGA object, the tables, bounded kernel channels | all vidmem, all VA spaces |

    ⇒ **A per-proc isolate never holds an `hMemory` for vidmem and never receives the guest-RAM
    memfd.** Isolates remain keyed on VA spaces.
    ★ **Flow:** the isolate allocates a **bare** `FERMI_VASPACE_A` and does not touch it; it is
    handed through the VMM to the scratchpad, which dups it and builds **its own**
    `NV01_MEMORY_VIRTUAL` range inside, then maps slices of the one object into it.
    ⊘ **No dummy channel is needed** — `[w744]` route B mapped with the duped space plus the
    scratchpad's own range, `placed_as_asked=true`; `MapMemoryDma` takes an `hDma`, not a
    channel. ⚠ The space **must be bare**: a pre-built whole-space range makes the scratchpad's
    `0x19 INSERT_DUPLICATE_NAME` ⇒ **one range object per space**, so `alloc_vaspace_raw` splits.
    ★ **Hardware already enforces the half that matters:** `[w744]` a VA collision is refused
    **`0x51`** with the control passing at an unclaimed VA ⇒ **an isolate cannot map over a
    slice the scratchpad placed.**
    ★★ **AND IT DELETES PLANNED WORK, NOT JUST CODE:** `guestram.rs`'s deferred enforcement —
    fd pinning, the seccomp filter, **`SECCOMP_RET_USER_NOTIF` on `mmap`**, munmap confirmation
    — exists to make *"the isolate never `mmap`s guest RAM on its own"* checkable. Under 26 the
    per-proc isolate **has no guest-RAM descriptor at all**, so the property holds by absence
    and the notify machinery is never built. ⊘ Stated explicitly: the scratchpad **does** hold
    guest RAM, and is trusted to because **it executes no guest-derived work** — that is a trust
    statement, not an omission.
    ⊘ **F11 is SCOPED, not eliminated.** Someone still names a client they did not mint — the
    **scratchpad**, dup'ing the isolate's VA space. The rule is: *a per-proc isolate may never
    name a foreign client; the scratchpad may, and only for a VA space the VMM handed it.*
    Enforce as a **newtype**, the way `OwnClient` already does, so the approved set grows by a
    TYPE and not by a string on an allowlist.

> ### ✔ **BUILT AND BOOTED 2026-09-15 (w745)** — live on all three arms, and it never had to
> fire: `withheld_unmaps=0 worst_unmaps_outstanding=0 pending=false`, with every
> `MMUINVAL-REFRESH` carrying `unmaps_outstanding=0 drain_trips=0`. ⊘ `pending=false` at
> teardown is the half that matters: the barrier caused **no hang**. ⚠ A never-fired barrier is
> not a tested one — the test that fires it is offline, and it is the known-positive below.
> ### ✔ **BUILT 2026-09-15 (w745)**, with the known-positive this constraint demands by name.
> `MmuInvalidateLog::complete_through_unmaps` returns a **three-armed** `CompletionVerdict` —
> not a `bool`, because *"a newer trigger completes this"* and *"nobody will, come back"* are
> different obligations and the old `bool` made them one word. The outstanding count is asked of
> **the queue** (`SharedDevice::staged_release_len`) and never derived from
> `drain_pending_releases`, whose `0` means *"nothing owed"* and *"nothing could be issued"*
> alike — its own docs say it SKIPS. `RegPlane::revalidate_mirror_first` runs the memslot
> plane's unmaps **before** the publication, because `drain_mirror_revalidation` runs its fills
> first and could not be hoisted.
> ★ The known-positive **stalls a real unmap** rather than simulating one:
> `drain_pending_releases` walks live procs only, so a retired proc's queue is a stall the
> production control flow itself produces — asserted as such before anything is concluded from
> it, with the drained control proving the barrier is not simply a hang.
27. **★★★★★ A REFRESH MAY NOT COMPLETE UNTIL ITS UNMAPS HAVE LANDED** (owner, 2026-09-15).
    *"Before a refresh finishes, this kernel channel has unmapped slices the guest userspace no
    longer has access to. So the guest kernel knows: okay, invalidate done, I can reuse this
    phys for another userspace process safely after a scrub."*
    ⇒ **This is a GUEST-INTERNAL isolation invariant and we are the only thing that can break
    it.** If the refresh reports the TLB invalidate complete before the unmaps land, the guest
    kernel reuses a physical page that a guest **userspace** process can still reach through a
    stale slice — a cross-process leak **inside the guest**, caused by us, and **invisible to
    the guest**.
    ★ The hook is already right: the refresh is driven by the TLB invalidate, one of the three
    sanctioned sync points, so the barrier is where it belongs. What must be true:
    **unmaps applied AND acknowledged before the invalidate completes**, and **unmaps ordered
    before maps** within one refresh. `walkdiff` already emits `Unmap`; completion must wait.
    ⚠ **Needs a known-positive**: stall an unmap and assert the invalidate does **not** complete.
    A test that only checks unmaps happen cannot tell "before" from "eventually".

> ### ⚠ **BUILT 2026-09-15 (w745) AND NOT EXERCISED ON HARDWARE.** The w745 boot never issued
> a store map (`maps=0`), so the placement assertion and the page-size selection were reached
> only by the offline tests. The w744 trace remains the only hardware evidence for the RULE;
> there is none yet for this implementation of it.
> ### ✔ **BUILT 2026-09-15 (w745).** `RmConnection::raw_map_dma_flags` now refuses
> `RmError::PlacementRefused` when `dmaOffset != at`, **after tearing the mis-placed mapping
> down**, and selects the page-size flag from `kayfabe_abi::bringup::nvos46_page_size_flag`.
> ⊘ **The flag is keyed on `(at, len)`, not on `Run::class`, and that is a measurement not a
> shortcut:** `walkdiff::MapOp` has **zero production consumers**, so the class is not at the
> map site while `(at, len)` is at every one — and the two agree by construction, since the
> coalescer never emits a 64 KiB-class run at an unaligned VA. `NVOS46_BIG_PAGE_BYTES` is
> stated as an ARCHITECTURE-FAMILY fact (constraint 12's maintainable form) with the condition
> that would retire it.
> ★ There is exactly **one** `NVOS46` encode site in the crate and a test pins it at one, so
> the assertion is unavoidable rather than present at the sites that remembered.
28. **★★★ EVERY FIXED MAP ASSERTS ITS OWN PLACEMENT, AND THE PAGE-SIZE FLAG MATCHES THE RUN'S
    CLASS** (2026-09-15, from `[w744]`).

    > ### ⊘⊘⊘⊘ CORRECTED HOURS LATER, 2026-09-16 (w755c) — **THE CONGRUENCE RULE BELOW IS
    > ### TRUE AND CANNOT FIRE ON THIS PATH. I BUILT A PREDICTOR ON IT ANYWAY.** Read this
    > ### before the block under it.
    >
    > Owner, refuting it with arithmetic that was available the whole time: *"since the
    > addresses are guest chosen, don't all alignments and congruence already line up?"*
    >
    >     guest gives:   at ≡ gpga                    (mod guest page size)  ← a PTE MEANS this
    >     we map:        at ↦ reservation_base + gpga
    >     RM needs:      at ≡ reservation_base + gpga (mod P)
    >     subtract:      ⇒  reservation_base ≡ 0      (mod P)
    >
    > **The only term that can break harmony is `reservation_base mod P`**, and RM aligns
    > vidmem allocations. At 4 KiB it is always zero. ⇒ §28 essentially cannot fire on the
    > store-slice path, the congruence never explained `map_refused=2154`, and a predictor,
    > a calibration path, a probe and two rented boxes were built on a premise one page of
    > arithmetic refutes.
    >
    > ★★★★★ **AND THE DEEPER RULE, which is the part to keep: DO NOT MODEL RM'S INTERNALS.**
    > `rm_would_place` modelled `_dmaGetPageSize` so a placement could be refused *before*
    > asking. That is exactly the coupling `mode2_forwarding_model.md` forbids — *correctness
    > = observable end-states only*. The post-hoc assert (★★ below) is the right shape
    > **because it is purely observational**: ask for `at`, read what came back, refuse a
    > mismatch. It cannot go stale when the driver changes. A predictor can, silently.
    > ⇒ reverted. The wire form that carries `want`/`got` out of the child is kept, because
    > it is a fact about *our own* plumbing and depends on no model.
    >
    > ★★★ **WHERE THE ALIGNMENT DOES BELONG — in the RESERVATION, not in a predicate.**
    > Owner: *"ensure the gpga rm object is aligned with 1 GiB, that basically kills all these
    > issues with memory offset harmony."* Right, **on a contiguous object**: then
    > `phys = base + offset` with `base ≡ 0`, so `phys ≡ offset (mod P)` for every page size
    > and the guest's own congruence closes it. ⊘ On `ATTR_NONCONTIGUOUS_VIDMEM` it does not —
    > sub-descriptors walk a page list, so aligning the base constrains page 0 and nothing
    > else. ⚠ And contiguity is what that attribute's own docs refuse, because a multi-gigabyte
    > contiguous request *"can fail on a card whose free memory is merely fragmented — which
    > would refuse the boot for a reason that has nothing to do with capacity."*
    > ⇒ **BOTH, IN ORDER**: `reserve_gpga` tries contiguous + 1 GiB-aligned first and falls
    > back; which one happened is **recorded, never re-derived**, and the store-slice page-size
    > pin follows it (no pin when contiguous ⇒ the framebuffer keeps its TLB reach; 4 KiB when
    > not ⇒ the only size whose congruence holds however the pages fell).
    >
    > ⚠ **The harmony problem has never been OBSERVED.** This is hardening, not a fix, and
    > `map_refused=2154` is still unexplained.
    >
    > ### ⊘⊘⊘ THE SUPERSEDED REASONING — the RULE is right, its APPLICATION here was not
    > ### REFINED 2026-09-16 (w755) — **THE RULE BELOW IS "ALIGNMENT". RM'S RULE IS
    > ### "CONGRUENCE", AND THE DIFFERENCE IS A TERM *WE* INSERT.**
    >
    > Derived from `[ogkm-580.159.04]` source, not measured: `virtual_mem.c:1323` turns the
    > NVOS46 `offset` into a **sub-descriptor**; then `gm107.c:726/:922/:1081/:1532`
    >
    >     pageOffset = (reservation_base + offset) & (pageSize - 1)
    >     vaLo       = ALIGN_DOWN(at, pageSize)
    >     reject iff (at - vaLo) != 0 && (at - vaLo) != pageOffset
    >     got        = vaLo + pageOffset
    >
    > ⇒ `got == at` **iff** `at ≡ (reservation_base + offset) (mod pageSize)`.
    >
    > | request's intra-page offset | RM |
    > |---|---|
    > | `0` — page-aligned | **ACCEPTED and MOVED**, `NV_OK` |
    > | `== pageOffset` | honoured exactly |
    > | anything else | `NV_ERR_INVALID_OFFSET` |
    >
    > ⚠ **The dangerous row is the first.** A nicely aligned VA reads to RM as *"you did not
    > account for the page offset; let me."* That is why ★★ below — compare `dmaOffset` on
    > the way out, never trust the status — is the half that survives unchanged and is the
    > half that matters.
    >
    > ★★★★★ **AND THE MISALIGNING TERM IS OURS.** Owner, 2026-09-16: *"I suspect the guest
    > already enforces this… the guest driver is not going to violate its own hardware."* It
    > does, and that is why the fault is ours. A guest PTE gives `at ≡ guest_gpga`. The single
    > store's identity is *framebuffer address = file offset*, so RM sees
    > `reservation_base + guest_gpga` — and the two congruences differ by
    > **`reservation_base mod pageSize`**, a number the guest has never seen. **We added a term
    > to a congruence the guest had already satisfied.**
    >
    > ⇒ **Store slices pin `PAGE_SIZE_4KB` unconditionally**, and not out of conservatism: at
    > 4 KiB the inserted term vanishes, because any RM vidmem allocation is ≥4 KiB-aligned.
    > Above 4 KiB the congruence needs `reservation_base`, which `reserve_gpga` does not
    > report. ⚠ Costs TLB reach; the optimisation is **named** — have `reserve_gpga` report the
    > GPGA, pick the largest congruent size, and *measure* — not assumed away.
    > ⊘ Scoped to store slices: compressed kinds require big pages, so a blanket `_4KB`
    > everywhere could be refused outright.
    >
    > ⊘ **What this retires in the text below:** *"a large leaf is necessarily at a
    > large-aligned VA, so map it with the matching big-page flag at its own address"* is
    > sound for a **dedicated leaf** (base aligned by construction) and **false for a slice of
    > an object whose base we do not know**. The `[w744]` rows are real but were measured on
    > **ring VAs the isolate itself chose** — carrying them to walked guest leaves is a change
    > of provenance, which is the §29 failure applied to a measurement instead of an assert. `NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE` honours an
    arbitrary VA **only** with `NVOS46_FLAGS_PAGE_SIZE_4KB` — **0/3 without, 3/3 with** — and
    without it RM **relocates and returns `NV_OK`**:

        at=0x8000001000 status=0x0 dmaOffset=0x8000000000 honoured=false
        at=0x9000001000 status=0x0 dmaOffset=0x9000001000 honoured=true   (4K flag)

    ⇒ `_dmaGetPageSize` picks a big page, which cannot start on a 4 KiB boundary, so RM
    **aligns down instead of refusing** — the `Xid 31 FAULT_PDE` the flag's own doc describes,
    **reached through a success**. ⚠ Every production caller of `raw_map_dma_flags` passes
    `extra: 0`.
    ★ **The rule:** a large leaf is necessarily at a large-aligned VA, so map it with the
    matching big-page flag at its own address; only 4 KiB-class runs need `PAGE_SIZE_4KB`. The
    coalescer already carries `Run::class` and never coalesces across page sizes, so the class
    is in hand at the map site.
    ★★ **The enforcement:** every FIXED map **asserts `dmaOffset == requested` and refuses
    otherwise.** ⊘ A `Result<u64, RmError>` returns `Ok` here and tells you nothing — this was
    caught only because `MapOutcome` carries `dmaOffset` **beside** the status.

29. **★★★★★ AN ASSERT IS RETIRED ONLY BY RE-ASKING ITS QUESTION IN THE NEW SHAPE — never by
    deletion, and never on the strength of an argument alone** (owner, 2026-09-15).
    *"I would edit the asserts, since we know our design holds the constraints… but it's
    important to add new asserts, so that we still protect ourselves against dumb changes and
    especially the one where the reasoning is not entirely sound."*

    A gate encodes a failure someone actually met. When the design moves, the **mechanism** may
    be wrong while the **failure** is untouched. ⇒ Deleting the gate deletes the protection and
    leaves the failure; and a codebase where gates get deleted whenever they are inconvenient
    teaches the next reader that red means *"the design moved"* rather than *"stop"*.

    ## The rule, in three parts

    1. **Restate, do not remove.** Keep the question, change the shape, and make the replacement
       **fail-closed**. ★ Worked example, w745: `RING_NOT_A_JOINED_WINDOW` asked *"is this ring
       the guest's memory, not a blank twin?"* Under one reserved object the join **mechanism**
       is meaningless, so it became `StoreMapPort::is_slice_of_the_store` — a live query of the
       mapper's own ledger. **Mechanism deleted, question kept, still fail-closed.**
    2. ★★★ **THE REPLACEMENT MUST TEST THE ARGUMENT THAT RETIRED THE OLD ONE.** When a gate is
       removed because *"X never happens"*, **X is the thing most likely to be wrong**, and
       nothing else is now watching it. ⇒ the new assert fires **if X happens**.
       ★ Worked example, w745: `AdoptedGuestRing::memory` was deleted because *"the ring handle
       never reaches RM — it was an authorization token."* The replacement is therefore a test
       that goes **red if the ring handle ever does reach RM**, not a comment saying it doesn't.
    3. ⊘ **Never go green by shrinking the universe the gate quantifies over.** Moving the
       offending code out of the scanned file is the `gates_quantified_over_a_list` failure the
       F11 test's own docs name. ⇒ F11 was scoped by growing its approved set **by a TYPE**
       (`HandedVaSpace`), with `APPROVED_RHS` unchanged and no verb relocated — and three
       mutations prove it: forging the handle, adding a `From<u32>`, and neutering
       `ScratchpadRole::of` all go red.

    ⚠ **A retired assert is a commit-message obligation**: say which failure it guarded, why the
    mechanism no longer expresses it, and **name the assert that now does**. An assert deleted
    without a successor named is a regression, however green the suite is.

> ### ⊘⊘⊘ CORRECTED w748 — **30's PREMISE NAMES THE WRONG QUANTITY, AND ITS DEMANDED ASSERT
> ### ALREADY EXISTS IN DATA WE RECEIVE.** Read this before the text below.
>
> **(a) It blames the euid. The euid is not the quantity.** 30 reasons from *"our isolates'
> kernel-visible euid is 0 on a root VMM"*. `NV01_ROOT_NON_PRIV` is real in RM's core, but
> `escape.c:394-403` **rewrites it to `NV01_ROOT_CLIENT` before RM ever sees it**, so
> `bIsRootNonPriv` is permanently false and `rmclientIsAdmin` reduces to
> **`capable(CAP_SYS_ADMIN)`, evaluated PER IOCTL** — not to a uid, and not to a property of the
> process fixed at spawn. ⇒ a component that holds the capability it needs for one call makes
> `_PRIVILEGE_ADMIN` channels on *every* call, **with no knob**. That turns 30 into a
> **sequencing** problem (drop the capability between the mint and the birth) which collides
> with lazy channel birth.
>
> **(b) A `KernelChannel` CANNOT BE DUPED — by anyone, at any privilege.** `resCanCopy_IMPL` →
> `NV_FALSE` and `serverCopyResource` refuses at `rs_server.c:1719-1723` **before rights are
> consulted**. So 30's worry about *"a scratchpad-born channel handed to an isolate"* describes
> a transfer **that cannot happen**. Scratchpad birth means the scratchpad owns the channel for
> its whole life and the isolate becomes a doorbell store.
>
> **(c) ★ The assert 30 demands is already in the reply buffer, and nothing reads it.** RM
> writes its verdict into the caller's own params (`kernel_channel.c:281-287`), copied back on
> success (`alloc_free.c:195-218`): **`NVOS04_FLAGS_PRIVILEGED_CHANNEL`, bit 5**. ⇒ we can
> **assert the channel's actual privilege** after every birth instead of reasoning about it.
> **Do that.** It is free, it is exact, and it is the only form of 30 that cannot be argued with.
>
> ⊘ What survives unchanged: the *rule* — a shared resource needs its own measured answer and a
> fail-closed assert, and `DupObject` succeeding says nothing about what the duped object
> carries. Only the euid premise and the channel-transfer example are wrong.

30. **★★★★★ ANY RESOURCE THE SCRATCHPAD CREATES AND SHARES MUST BE PROVEN NOT TO CARRY THE
    SCRATCHPAD'S PRIVILEGE** (owner, 2026-09-15). *"The reason we also do isolates is to ensure
    the channel is created in an unprivileged process. If ogkm links the process that created
    the channel to the privileges of it… the cross guest process isolation is broken. So this
    has to be asserted."*

    ⊘⊘⊘ **CONFIRMED IN ogkm-580.159.04**, `src/kernel/gpu/fifo/kernel_channel.c:277-295`:

        RS_PRIV_LEVEL privLevel = pCallContext->secInfo.privLevel;
        if (privLevel >= RS_PRIV_LEVEL_KERNEL)            → _PRIVILEGE_KERNEL, _PRIVILEGED_CHANNEL_TRUE
        else if (rmclientIsAdmin(...) || hypervisorCheckForObjectAccess(hClient))
                                                          → _PRIVILEGE_ADMIN,  _PRIVILEGED_CHANNEL_TRUE
        else                                              → _PRIVILEGE_USER
        pKernelChannel->ProcessID    = pRmClient->ProcID;
        pKernelChannel->SubProcessID = pRmClient->SubProcessID;

    ⇒ **Privilege is stamped AT CREATION from the creating call context**, and **the channel
    records the creating process**. A `DupObject` does **not** re-run this, so **the stamp
    survives the hand-over.** ⚠ And F11 records that our isolates' kernel-visible euid is **0**
    on a root VMM, so `rmclientIsAdmin(...)` plausibly holds for the scratchpad — which would
    make every channel it births `_PRIVILEGED_CHANNEL_TRUE`.

    ⇒ **"The scratchpad births the channel and dups it to the isolate" is WITHDRAWN as a
    default.** It is admissible only if measured: **scratchpad-born channels come out
    `_PRIVILEGE_USER`**, asserted at birth and refusing otherwise.

    ★ **And it generalises past channels.** For **every** resource the scratchpad creates and
    shares — VA spaces, ranges, mappings, channels — the question is the same: *does sharing it
    convey the creator's privilege, its process identity, or a route to its CPU address space?*
    ⊘ **Do not answer from the API's shape.** `DupObject` succeeding says nothing about what the
    duped object carries; `[w744]` measured that a dup that succeeds can still be the wrong
    thing (the "one space vs two spaces" falsifier). ⇒ **each shared resource needs its own
    measured answer and its own fail-closed assert**, and until it has one the sharing is
    refused, not assumed.

31. **★★★★★ USERD PINNING IS A SECURITY ASSUMPTION ABOUT AN *HONEST* GUEST — and the window
    opens at TEARDOWN** (owner, 2026-09-15).

    ## The assumption, recorded because it is load-bearing and invisible

    ogkm programs the instance block from `memdescGetPhysAddr(pUserdSubDeviceMemDesc, AT_GPU, 0)`
    (`kernel_channel_gm107.c:328`) — a **physical** address, 4 KiB-attributed (`:689`), which
    **never touches PT\*/PD\***. ⇒ hardware holds a raw physical address for the channel's
    lifetime, so the page **cannot** be moved or freed while the channel lives **without
    hardware reading the wrong memory, silently**.
    ★ **We therefore depend on it being pinned — and that is an assumption about an HONEST
    guest.** A hostile guest that frees or re-points its own USERD hurts only itself *provided
    we hold nothing over that page*; the moment we do, its lifetime becomes our problem.

    ## ⚠ THE WINDOW IS AT TEARDOWN, AND IT IS A GUEST-INTERNAL CROSS-PROCESS READ

    When the channel dies or the VA space goes, the guest unpins the page and **its allocator may
    hand that 4 KiB to another guest process**. If any artefact of ours outlives that moment —
    a `LIST_OBJECT` slice handle over the page, a mapping, an armed CPU view — then a page now
    owned by **guest process B** is still reachable through machinery minted for **guest process
    A**. ⇒ **an unprivileged guest userspace process reading 4 KiB assigned to another process,
    caused by us, and invisible to the guest.**

    ⇒ **Constraint 27 extends from MAPPINGS to HANDLES.** A slice handle is not a mapping: it is
    an RM object with its own lifetime, and destroying a mapping does not destroy it.
    **Every artefact we mint over a guest page must be destroyed before the guest may reuse that
    page**, and the barrier must cover **channel teardown and VA-space destruction**, not only the
    TLB-invalidate refresh — those are different events and only one of them is currently a
    barrier.
    ⚠ **Known-positive required**, and it must be the racing one: tear a channel down with a live
    slice handle outstanding and assert the teardown **does not complete** until the handle is
    gone. A test that merely checks the handle is eventually destroyed cannot tell *"before the
    guest could reuse it"* from *"eventually"* — the same distinction 27 already turns on.

    ⊘ **And a `LIST_OBJECT` is a SNAPSHOT**: its page-number list is taken at creation and
    subscribes to nothing. If the guest moves the page, the handle silently names the old one.
    That is the same failure by a different route, and it is why the teardown barrier cannot be
    replaced by revalidation.

32. **★★★★★ LEG B IS RULED — ROUTE K: THE ISOLATE MINTS THE BIRTH CLIENT AND HANDS THE fd OVER**
    (owner, 2026-09-16: *"K is by far the cleanest, I am for it"*).

    ## The mechanism, and why it costs nothing the alternatives cost

    Two facts make it work, and neither was obvious: **`ProcessID` is stamped at CLIENT creation
    from the creating task** (`client.c:112`), and **a client is bound to the `struct file`, not
    to a pid** (STRICT default, `g_system_nvoc.c:104`) — which `SCM_RIGHTS` carries. Privilege
    comes from the **calling task's capability**, per ioctl (`escape.c:304`).
    ⇒ **Which task created the client** (identity) and **which process drives it** (the fd) are
    separable. Every earlier route conflated them.

    **The sequence.** I allocates client **B** on a second fd → passes it to S via `SCM_RIGHTS`
    and **closes its own copy** → S dups the store into B → S dups **I's VAS** into B **with no
    grant** (default same-PID `DUP` policy, `sharing.c:341-352`) → S births the channel in B →
    S **frees the dup** (the sub-memdesc holds its own parent ref, `mem_desc.c:2676`).
    ⇒ Stamps land as **`ProcessID = I`** and **`_PRIVILEGE_USER`**, and **I never holds a vidmem
    handle nor the guest-RAM memfd** — constraint 26 intact, and kayfabe's isolates stay
    unprivileged.

    ## ⊘ The cost, stated rather than discovered later

    The channel handle lives in **B**, reachable only by S, because **a `KernelChannel` cannot be
    duped by anyone at any privilege** (`resCanCopy_IMPL` → `NV_FALSE`; `serverCopyResource`
    refuses at `rs_server.c:1719-1723` **before rights are consulted**). ⇒ §26's row *"the
    isolate owns the channel"* is **amended**: the isolate owns the channel's **identity and
    address space**; S holds the **handle**. Ownership in the sense that matters — whose
    `ProcessID`, whose VAS, whose privilege — is the isolate's.

    ## ✔✔✔ MEASURED 2026-09-16 (w750) — **ALL FOUR DISCRIMINATORS HELD. ROUTE K IS REAL.**

    GA106, host **open** kernel module **580.159.04**, `kayfabe-rm-ladder --route-k`.
    Evidence: `traces/w750_route_k/route_k_run3.log`; pre-registration, predictions and the
    full reading: `docs/design/w750_route_k_prereg.md`. ⊘ Read the list below as **answered**,
    not as open.

    | # | measured | verdict |
    |---|---|---|
    | 1 | `K_BIT5=0` (readback `0x00000000`); known-positive `K_BIT5_KP=1` (`0x00000020`) | **HELD** |
    | 2 | `K_PIDS_CHAN` = **I's pid only**, S's absent; control `K_PIDS_DEV` = **both** | **HELD** |
    | 3 | dup freed ⇒ `0x57 OBJECT_NOT_FOUND` vs `0x23` while it exists; `K_CHANNEL_LIVE=1`, the engine's own semaphore lands, `K_CHAN_ALIVE_AFTER_FREE=1` | **HELD** |
    | 4 | `K_UVM_REG_CHAN_RC=0x0` **and `K_UVM_SCHED_RC=0`**; known-positive `0x0`; negative control `0x33` | **HELD** |

    ★ Row 4 is stronger than it asked for: the B-owned channel **scheduled**, and
    `kchannelIsSchedulable_IMPL` refuses an externally-owned channel with unbound
    allocations — the only userspace writer of `bIsContextBound` is UVM's register. So the
    registration did its work rather than merely returning `NV_OK`.

    ### ★★★ A SECOND GATE NOBODY NAMED — and it is a constraint on K, not a bug

    `osapi.c:2378`, in `rm_create_mmap_context`:
    `if (pRmClient->ProcID != osGetCurrentProcess()) rmStatus = NV_ERR_INVALID_CLIENT;`
    ⇒ **`NV_ESC_RM_MAP_MEMORY` is refused whenever the client's `ProcID` is not the CALLING
    task's**, so under K **S can never CPU-map any object in B**, for every object, forever.
    ⚠ Measured, with the instrument's own known-positive (`K_MAP_INSTRUMENT_KP=0` on the
    identical code path against a same-`ProcID` client), so `0x23` is RM's answer.
    ⇒ **It costs K nothing** — every page S must touch is a slice of **the store**, which S
    owns and reaches through its own handle (measured: S wrote the pushbuffer and read
    `GP_GET` on the very page the channel's USERD is in). ★★ And it is an **independent,
    RM-side enforcement of §26** that nobody put there. ⚠ But it is an **expiry condition**:
    anything K later wants S to CPU-map *inside B* is refused by name.

    ### ★★★★★ AND THE ZEROING QUESTION IS ANSWERED, THE OTHER WAY

    `fable_leg_b_solution_space.md` §7 reads `kernel_channel.c:2342-2356` as scrubbing a
    client USERD **only** for `ADDR_SYSMEM`, and predicted a **vidmem** USERD's poison would
    survive. **It does not.** Poison read back *before* the birth (`a5a50000 a5a50001 …`) and
    **all zeros after**, paired with `K_CHANNEL_LIVE=1` so neither *"the store never landed"*
    nor *"a wrong `userdOffset`"* explains it.
    ⇒ **Adoption must precede the guest's first `GP_PUT`, for every route.** K does by
    construction. ⚠ Something other than the CPU-RM arm §7 cites performs the scrub, and this
    probe does **not** say what — that is the next question, not a settled one.

    ### ⊘ AND THE CONTROL THAT SAVED THE RULING

    Runs 1 and 2 measured row 4 as **REFUTED** (`0x31 NV_ERR_INVALID_OBJECT`). Run 2's
    known-positive — a channel the per-proc role owns **itself**, same UVM session, same
    space — answered `0x31` **too**, so the refusal was about the harness, not the foreign
    client: on GA106 **CE0 shares runlist 0 with GR**, so a CE0 channel is graded by the
    graphics rule (`kernel_channel_gm107.c:722-727`). Both arms moved to **COPY(2)** and both
    went green. ⚠ **Without that control this lane would have told the owner that K cannot
    carry CUDA.**

    ## ⊘ SUPERSEDED BY THE BLOCK ABOVE — what had to be measured before it was believed (Fable §9)

    ⊘ **All four are now measured and all four HELD (w750, 2026-09-16).** Kept for the
    reasoning and for the refuting values each one named.

    1. **bit-5 readback** — `NVOS04_FLAGS_PRIVILEGED_CHANNEL` must come back **clear**. ★ Free,
       exact, and the only form of constraint 30 that cannot be argued with.
    2. **`ProcessID` lands as I's** — check the channel's pid-info reports under **I's** pid, not
       S's.
    3. **The freed dup is unmappable while `GP_GET` still advances** — proves the parent ref
       outlives the dup *and* that the channel is live.
    4. ⚠ **UVM registration of a B-owned channel from I's UVM fd is INFERRED viable**
       (`uvm_user_channel.c:945-948`, no `rmCtrlFd` validation) — **not measured**. If it fails,
       K needs an answer for UVM before it can carry CUDA.

    ⊘ **A, B and the sysmem-USERD family are DEAD — do not probe them.** USERD's aperture is a
    hardcoded `ADDR_FBMEM` (`kernel_fifo_gm107.c:82-87`) and libcuda never asks
    `GET_USERD_LOCATION` (zero `0x208011xx` controls in 197, real-GA106 trace); the `userdMem`
    descriptor reader is unreachable off GSP/VF builds.
    ⊘ And **"there is no SET for USERD" was FALSE** — `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO`
    (`0x20801116`) re-points a live channel's USERD+GPFIFO. It is privileged, so K is preferred,
    but the claim was a **name-shaped blind spot** in a grep for `SET_USERD`.

★ **THE PREFERRED MECHANISM for 23, and why (owner, 2026-09-15).** Rather than an anonymous
sparse `mmap`, allocate a **GPU-native sparse range** (`NVOS32_ALLOC_FLAGS_SPARSE = 0x04000000`,
confirmed present in RM's SDK) in the scratchpad and MMIO-map **that** for BAR1/BAR2. Three
reasons, the first being the owner's biggest: it **reserves the aperture on the host** up front,
like VRAM, so there are no mid-boot out-of-memory surprises; it makes a BAR1/BAR2 trap
**unimplementable by construction**; and it removes `munmap` from the BAR1/BAR2 path entirely —
pages are replaced **in the GMMU underneath a fixed host mapping**, so the memslot never moves
and KVM is untouched after boot.
⚠ **THE ONE OBJECTION, and it must be answered before this ships: SPARSE IS SILENT.**
*"writes ignored, reads 0"* makes *"nothing is mapped here"* indistinguishable from *"zero is
the value"*, forever, with no counter. Every expensive defect of 2026-09-15 was of exactly that
shape — `garbage 0x0`, `SUITE_RC=0` over 26 unmeasured arms, an empty leaf list published as
*"the guest has mapped nothing"*, `faults` pinned at 0 by its own plumbing. Concretely,
`kbusVerifyBar2` writes a pattern and reads it back: **on a sparse page that reproduces
`garbage 0x0` exactly, with no refusal to name.**
⇒ **Two PTE states behind the same memslot:** production **sparse** (reads 0, writes dropped, no
trap possible); bring-up leaves the PTE **INVALID** so the access faults into the GPU fault
buffer where it can be seen. Same memslot, same no-trap-in-prod guarantee, and the bring-up arm
stays **loud**. ★ The debug arm is the default until the raw client passes.

★ **And one measurement that changed a ruling, 2026-09-15 (w734):** §w724c's *"there is no
working intermediate — it does not boot"* is **refuted by measurement**; the intermediate costs
**5–10 s**, not minutes, and its aperture cost is **0.5 MiB of 256**. The correction is folded
directly above §w724c. ⇒ *a derivation cited as a measurement is the most expensive kind of
wrong*, and this file carried one for two weeks in the sentence that ordered the whole branch.

★ **Later additions that are not numbered constraints but bind the same way:**
**§w724g** gates expire — carry an expiry condition in the gate's own doc comment, and unwire and
delete in the **same** change · **§w727** BAR1/BAR2 are **sized options** with enforced minimums
(powers of two; refuse, never clamp) · **§w729** when stuck **ask Fable and give it this file**,
and **never buy a pass by relaxing a constraint** · **§w729b** a measured dead end **is a
deliverable**.

33. **★★★★★ THE RESERVATION'S SHAPE IS MEASURED AND RECORDED, NEVER ASSUMED — AND THE
    PAGE-SIZE DECISION FOLLOWS IT** (owner, 2026-09-16, w755c).
    *"Ensure the gpga rm object is aligned with 1 GiB, that basically kills all these issues
    with memory offset harmony."* — right, **on a contiguous object**, where
    `phys = base + offset` and `base ≡ 0 (mod 1 GiB)` make a FIXED map congruent at every
    page size. ⊘ On `ATTR_NONCONTIGUOUS_VIDMEM` it does not: `MapMemoryDma` sub-descriptors
    walk a page list, so aligning the base constrains page 0 and nothing else.
    ⚠ And contiguity is what that attribute's own docs refuse — a multi-gigabyte contiguous
    request *"can fail on a card whose free memory is merely fragmented, which would refuse
    the boot for a reason that has nothing to do with capacity."* **Both arguments are right.**
    ⇒ `reserve_gpga` tries **contiguous + 1 GiB-aligned first** and falls back to the
    documented noncontiguous form. One refused allocation is the entire cost, and the choice
    is made **by measurement rather than by anyone's prediction**.
    ★★ **Which one happened is RECORDED, never re-derived.** The store-slice page-size pin
    reads it: no pin when contiguous (the framebuffer keeps its TLB reach), `PAGE_SIZE_4KB`
    when not (the only size whose congruence holds however the pages fell). A map site that
    inferred the reservation's shape would be a second statement of one decision — the
    failure this file names at §28 and `a_second_source_of_truth_beside_a_complete_value`.
    ⊘ **Not a fix for anything observed.** See §28's w755c correction: harmony cannot fail at
    4 KiB, has never been measured failing, and `map_refused=2154` is still unexplained. This
    is hardening that also **buys back** page-size freedom the earlier unconditional pin took.

34. **★★★★★ DO NOT MODEL RM'S INTERNALS TO PRE-EMPT IT — ASSERT ON WHAT CAME BACK**
    (2026-09-16, w755c, from the owner's *"we shouldn't … infer rm placement logic"*).
    A predictor that reproduces `_dmaGetPageSize` so a call can be refused *before* it is
    made is the coupling `mode2_forwarding_model.md` forbids: **correctness = observable
    end-states only**. It buys a teardown we do not need, and it can go silently wrong on the
    next driver while reporting confidence. ⇒ the observational assert is the shape: ask,
    read what came back, refuse a mismatch **by name**, and carry the two numbers out
    (`WireError::PlacementRefused { want, got }`) so the refusal is readable.
    ⚠ This is not an argument against reading the driver's source — that is how `want`/`got`
    were understood at all. It is an argument against **depending** on the reading at runtime.

35. **★★★★★ NO BLOCKING CALL IN A WORKER THAT BLOCKS NEW INPUT TO IT** (owner, 2026-09-17).
    A worker may block — it must never block in a way that makes it deaf. ⊘ The distinction is
    the whole constraint: *"blocking"* is fine, *"blocking somewhere the next piece of work
    cannot reach me"* is not.
    ★★ **POLLING IS ALWAYS ALLOWED**: `epoll_wait(timeout=0)`, a queue check, a semaphore
    check. None of those is a blocking call and none needs permission.
    ★★ **AND NEITHER IS A CALL EXPECTED TO RETURN ALMOST INSTANTLY** (owner, 2026-09-17):
    logging, `mmap`/`munmap`, and their kind are **not** §35 blocking points. The rule is
    about a worker becoming **deaf**, not about latency in general, and a call that returns
    promptly leaves nothing unreachable.
    ⊘⊘ **THIS DOES NOT EXEMPT THEM FROM §25, AND THE TWO RULES HAVE DIFFERENT SUBJECTS.**
    §35 asks *"can new input still reach this worker?"*; §25 asks *"is a lock held across a
    call that may sleep?"* ⇒ an `mmap` is fine in a worker loop and is still **forbidden under
    a ranked lock**: `[measured 2026-08-13, boot w289j]` dropping a refused `FbJoined` inside
    the plane lock `munmap`ped under rank 0, fired `lockwitness`' R1 assert inside an
    `extern "C"` QEMU callback, and **aborted the whole VMM** on the guest's own MMIO write
    path — guest-reachable, because the guest chooses whether a join is refused.
    ⇒ *"returns almost instantly"* licenses it against §35 and says nothing about §25.
    ★★★ **AND A THIRD EXEMPTION WHOSE SET IS BELIEVED EMPTY** (owner, 2026-09-17):
    *"anything there is no alternative for with epoll fds — but I think that set is empty."*
    ⊘ That is the strong form and it is deliberate: the exemption exists so the rule is not
    a lie, and it is **presumed to have no members**. ⇒ **an entry is a FINDING, not a
    configuration.** Adding one requires showing that the call has no fd-backed alternative —
    not that none was convenient, and not that nobody looked.
    ⚠ Same shape as §25's own gate (*an empty allowlist is a build failure, not a pass*) and
    as the relaxation ratchets: a set whose growth is invisible stops being a rule. Whatever
    enforces §35 must therefore **print the set's size every run**, so an empty one is
    measured rather than assumed.
    ★★★ **There are exactly TWO blocking points, and both are lock-free waits:**
    (1) waiting on the MMIO notification — **the lock is not held while waiting**; or
    (2) waiting on `epoll` over several fds, always including the coordinator's new-work fd.
    ⊘ Choosing (1) is not about holding anything: it means a doorbell reaches this worker
    **directly** instead of via the coordinator's wake — one hop less.
    ⚠ **AND A PRECONDITION: a worker with semaphores it is still spin-checking MAY NOT BLOCK
    AT ALL**, on either point, until every one has been promoted to an epoll fd. It cycles
    with `timeout=0` and re-checks them instead.
    ⊘⊘ **The failure this precondition prevents has a name and we have just spent a day on
    its twin: a LOST WAKEUP.** A worker that blocks holding an un-promoted semaphore sleeps
    through its own completion — the work finishes, nothing writes to an fd, and the symptom
    is *"the copy never retired"*, indistinguishable from the wall w755h measured. ⇒ it is a
    **witness**, not a comment: refuse to enter either blocking point while `pending_spin > 0`,
    count it, and give the counter a known-positive (§ *a census zero needs a known-positive*).
    ★ It composes with §§4/6/25 for free: both blocking points being lock-free waits makes
    *"no blocking under a lock on any thread"* true **by construction** rather than by
    discipline.
    ⊘ One tunable, and it is measurable rather than guessed: how long to spin before promoting
    a semaphore to an fd. Sysmem semaphores are cheap to poll; **vidmem ones are not**, and a
    tight loop on one re-introduces at the completion end exactly the CPU-read cost the GPU
    execution removed. Condition the spin on the semaphore's aperture — the same
    `CpuPlane::{Fb, GuestRam}` split the operand read already makes.

36. **★★★★★ ONE WORKER RUNS MANY TASKS AT ONCE — kernel channels AND isolate work**
    (owner, 2026-09-17). A worker is not a task; it is a loop over several sources.
    ⊘ **ONE loop, with the `epoll` inside it** — there is no separate spin loop. Each cycle
    looks at the mutex, the epoll set and the semaphores, then decides what to do next.
    ★★★ **THE COORDINATOR IS EMERGENT, NOT APPOINTED.** If every worker is in `epoll` and one
    needs to block on the mutex for new work, that one **becomes** the coordinator by doing so.
    ⊘ And a coordinator **can never hold outstanding epoll fds, by construction** — it is
    blocked on the other point. There is no role to assign, no handoff to get wrong, and no
    state saying who is coordinating that could disagree with who actually is.
    ⚠ ⇒ *"which thread is the coordinator"* is not a fact to store. Storing it would be a
    second source of truth for something the blocking point already decides — the
    `a_second_source_of_truth_beside_a_complete_value` shape.

37. **★★★★★ EMULATED CHANNELS EXECUTE IN THE VMM WORKER, THROUGH A RAW CLIENT THE VMM OWNS —
    NOT BY A HOP INTO THE SCRATCHPAD** (owner, 2026-09-17). *"For emulated channels it executes
    in the worker in VMM, from there it can execute the raw client... This means the va space of
    scratchpad land in the VMM, which is also needed for the MMIO CPU maps to populate bar1/bar2
    anyways."*
    Two reasons, and **the second is the load-bearing one**:
    (a) it removes a process hop for work the worker already has in hand;
    (b) ★★★ **a raw client yields an `eventfd`**, so the worker polls its own semaphore inside
    its own `epoll` set — which is what makes §35 satisfiable *at all* for this work. Executing
    in the scratchpad gives the worker a reply it must wait for; executing here gives it an fd.
    ⇒ **The scratchpad's VA space lands in the VMM.** That is not a new cost: §23 already
    requires CPU maps in the VMM to populate BAR1/BAR2, so the VMM holds those views regardless.

    ⊘ **THIS DOES NOT REOPEN §20, AND THE LINE BETWEEN THEM IS THE LINE TO HOLD.** §20 puts the
    walker in the scratchpad because the walker **dereferences guest-authored pointers in a
    loop** — memory safety over hostile data, which must run at the privilege of that data. An
    emulated-channel submission is a different shape: a **bounded-arity descriptor**
    (`src, dst, len`) that is **bounds-checked against the store before it is built**, then
    handed to RM. It follows nothing. ⇒ The test for *"may this run in the VMM?"* is **not**
    *"is the input guest-authored?"* — nearly everything is — but **"does it chase guest-authored
    structure?"** Walker: yes, scratchpad. CE/scrub descriptor: no, VMM worker.
    ⚠ **So the boundary is enforceable and must be enforced by a gate, not by intent**: anything
    the VMM's raw client submits is built from a fixed number of already-validated fields. The
    day a loop over guest data appears on this path, it has become §20's problem and belongs in
    the scratchpad.

    ⊘ **AND THE DELTA IS THE RM CLIENT, NOT THE EXECUTION SITE.** Read carelessly this says
    *"guest-derived execution moves into the VMM"*, which would collide with §26's trust
    statement (*"the scratchpad … is trusted to because it executes no guest-derived work"*).
    It does not: `route_of_engine` already maps `EngineKind::Ce → DoorbellRoute::CpuCe` →
    `ShellDisposition::MayServeLocally`, and `cpu_ce::execute_ours` **already runs in the
    VMM**. What the VMM gains is an **RM client**, so the executor already there can hand a
    descriptor to hardware instead of moving bytes with the CPU.
    ⚠ **V is minted BY THE VMM.** RM stamps `ProcessID` from the calling task, which is why
    `mint_birth_client` refuses when the caller is the scratchpad. A V minted anywhere else
    carries the wrong stamp **while every ioctl succeeds** — route K's founding failure.
    ⚠ Duping the store into V extends **F11**'s approved set from *"a VA space the VMM handed
    the scratchpad"* to a **memory object**. That needs a second arm on the newtype, argued —
    not a string on an allowlist.

    ⊘ **It also retires the CPU-read execution path, which was never viable.** The emulated CE
    currently executes on the **CPU** (`cpu_ce.rs::execute_ours`, reading operands via
    `ce.fb().read()`), and under the single store a CPU read of the store is an **MMIO read at
    ~48 MiB/s**. ⇒ *"execute on the CPU"* and *"the store is the only memory"* were never
    compatible; this ruling is what makes the second one payable.

38. **★★★★★ THE CPU NEVER READS GUEST VIDMEM, AND THE PTX WALKS GPGA LIVE** (owner,
    2026-09-18). Four clauses, and they compose into one rule: *nothing copies the guest's
    framebuffer, and nothing reads it with the CPU.*

    ⊘ **(a) The CPU touches GPGA only for the VMM's BAR MMIO maps.** *"We ourself dont read
    from guest vidmem."* ⇒ CUT A — the device store's host-side `read`/`write` refusing by
    name — is **permanent and correct**, not a phase to be worked around. A CPU read of video
    memory is ~48 MiB/s *and* would be a second memory for one address; the refusal is the
    design asserting itself.
    ★ **The one exception, and it is not one:** the raw scratchpad client writes its **own**
    ring and USERD for kernel-channel work. Those are ours, resident in the scratchpad's VA,
    **not on GPGA** — so they are not guest vidmem and the rule is unbroken.

    ⊘ **(b) The PTX's mapping of GPGA is LIVE — the real RM object, natively mapped.** Not a
    copy, not a snapshot, not a staged image. `[measured w755x]` RM exports the reserved
    object to a control fd; `cuMemImportFromShareableHandle` + `cuMemMap` place **that object**
    in the CUDA context's VA. ⇒ the kernel dereferences `KfWin { base, len }` at GPGA offsets
    and reaches the guest's real tables where they live.

    ⊘ **(c) THERE IS NO IMAGE UPLOAD.** *"That code is not needed."* The staged path built the
    kernel's window by reading page tables through `FbRead`, which (a) forbids — so on the
    device arm it was **blind, and silently so**: it reported `bound=0 published=0` , which
    reads as *"there was nothing to publish"* rather than *"we could not look"*.
    ⚠ **It is DELETED, not kept as a fallback.** A fallback that answers wrong without saying
    so is worse than a refusal, and this tree spent a session reading that silence as data.

    ⊘ **(d) The prev/cur snapshot is resident in VIDMEM and is the KERNEL'S OWN** — `KfDev
    { tbl[2], cur_buf, have_prev, generation, acked }`. Its only job is comparing what is new
    against what is old. It was never something the host staged, and nothing about it crosses
    the bus.

    ⊘ **(e) A MALICIOUS GUEST RUNS NO KERNEL CHECKS, SO EVERY BOUND MUST BE OURS — AT EVERY
    STAGE, NOT ONLY IN THE WALK** (owner, 2026-09-18: *"a malicious guest doesn't follow any
    kernel check, so thats why we need to prepare it"*).
    The guest's own driver produces well-formed tables; a hostile one writes arbitrary bytes
    and we follow them. ⇒ every value derived from guest-authored memory is untrusted at each
    hop. Measured state of the chain:
    - **the walk** — `KF_GPGA_DEREF` is the single deref site and is guarded
      **overflow-safely** (`off > w.len || 8 > w.len - off`, never `off + 8 > w.len`); an
      out-of-range entry sets `KFWR_R_OOB` and bumps `refusals`, so a hostile attempt is
      **counted**, not silently skipped. ✔ `[w760]` 72/72 on hardware including hostile and
      racer streams.
    - **the map** — `offset.checked_add(len)` against the reserved object's length, with the
      comment that states the rule: *"RM maps whatever offset it is handed."* ✔
    - **the report** — truncation is refused before comparison (`walkshadow.rs:490`). ✔

    ⚠⚠⚠ **AND THE GAP THAT IS NOT YET CLOSED, named so it cannot be forgotten: a TRUNCATED
    report must never reach the differ.** §20's third invariant is *"output capped with loud
    truncation, forcing a full resync"*, and the cap is what a hostile guest will aim at — a
    table large enough to truncate the report. **A diff computed against a truncated CURRENT
    set emits `Unmap` for every mapping that fell off the end**, and those mappings are live.
    ⇒ the guest would be inducing us to unmap arbitrary ranges *through our own executor*.
    ⊘ The refusal exists today only on the **shadow-comparison** path. When the report is wired
    to `StoreMapPort::apply_ops`, it must pass through the same refusal — and the right shape
    is the one this tree already uses for unforgeable claims: make the ops constructible **only
    from a report that validated**, so *"nobody checked"* is unrepresentable rather than
    forbidden.

    ⇒ **The test for any future code touching the framebuffer is (a):** if it needs the CPU to
    read guest vidmem, it is wrong — the walk moves GPU-side instead. **And the test for any
    code consuming the walk is (e):** if a hostile table can make it do something, the bound
    is missing.

⚠ **This list stopped at 17 while §§18–22 were added as sections below it** — a reader hitting the
list would have concluded seventeen was all of them. ⇒ **Anything added below gets a row here in
the same change**, or the index becomes the most confidently wrong thing in the file.

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
| 2 — BAR0 write-only | HOLDS except the counter page | ~132 reads at `+0xbb0000`. ⊘ *"the device-view wire verb is unbuilt"* — **STALE, corrected 2026-09-15**: it is live on request tags **30/31**, reply **15**, and was exercised end to end in a boot (`DEVICE_VIEW=OK … released=true`) |
| 5 — DoorbellTable | wired | goal 4, w656–w660 |
| 15 — disjoint worlds | ⊘ **SUPERSEDED, not violated** | there is **one world**, not two (§15's supersession block). The row described the old design's failure to meet a constraint that no longer exists |
| 15 — one reserved object | ◐ **RESERVED AND ADVERTISED; the BACKING has not moved** *(2026-09-15)* | ⊘ *"`reserve_gpga` has no caller … no GPGA line in any boot"* is **STALE**: it has callers (`rm.rs:4595`, `:4651`) and three committed boots carry `reservation=HELD` / `RESERVED_MB`. ⚠ **But `page_backing` still returns memfd leaves**, so guest vidmem remains host RAM over PCIe and parity is unchanged at 0.20x. The object is **held and unused** until §3 |
| 4, 6, 7, 8 — sub-ms / off-vCPU | ◐ **PARTLY DISCHARGED w752, 2026-09-16** — `total=197 doors=9` ⇒ **`total=102 doors=6`**, doors 7/8/9 gone and 4/6 down to one crossing each; PRAMIN's move 44.4 ms ⇒ **1.12 ms** worst. ⚠ **4 IS STILL VIOLATED ON BOTH ARMS**: `worst_trap≈25 ms at bar0+0x110c00` (`NV_PGSP_QUEUE_HEAD`), which crosses **no `assert_lock_free` door** and is invisible to this census. See the w752 block below. ⊘ The w742 state it replaced: ⊘⊘⊘ **VIOLATED ON THE `device` ARM (w742, 2026-09-15)** — `VCPU-BLOCKING total=197 doors=9 worst_trap=44440us` (control: `total=22 doors=3 worst_trap=22311us`). ★ The old ruling covered **3 doors that WERE the 22 PRAMIN re-points**; the new doors are the single store's own machinery — `exporting a host device view`, `receiving a descriptor across the isolate boundary` (40×), `classifying a received descriptor`, `mmap`, `mmap MAP_FIXED`, `KVM_SET_USER_MEMORY_REGION` (20×) — **none of which is PRAMIN**, so the sanction does not reach them (*a ruling's date AND its architecture are part of the citation*). ⇒ **REMEDY IS CONSTRAINTS 6+7+8 THEMSELVES**: post to a queue, wake a worker, return; workers do the work and write completions asynchronously. Owner's queue/epoll/eventfd design of 2026-09-15 is the plan of record — see the block below. ⊘ Tracked as an OPEN VIOLATION with a named fix, **not** as hardening. |
| 7, 14 — epoll / threaded isolates | not built | |

### ✔✔✔ **PARTLY DISCHARGED 2026-09-16 (w752) — THE NINE DOORS ARE SIX, THE 197 CROSSINGS ARE 102, AND THE 44 ms IS 1.1 ms. Read this before the section below, which describes the state it replaced.**

`[vast 51210329, GA106, 580.159.04 OPEN, TREE_REV fcce2a11, three arms, one binary, control first]`

```
device arm w742:  VCPU-BLOCKING total=197 doors=9   move_ns[worst=44426000 mean=7248000]
device arm w752:  VCPU-BLOCKING total=102 doors=6   move_ns[worst=1115752  mean=496145]
  [40 × receiving a descriptor across the isolate boundary]   door 2 — STAYS (the arm)
  [20 × classifying a received descriptor]                    door 3 — STAYS (the arm)
  [20 × exporting a host device view to the VMM]              door 1 — STAYS (the arm)
  [20 × mmap MAP_FIXED (placing an armed device node)]        door 5 — STAYS, the SANCTIONED one
  [1  × KVM_SET_USER_MEMORY_REGION (installing a memslot)]    door 6 — the ONE-TIME install
  [1  × mmap (creating a guest-physical window)]              door 4 — the ONE-TIME install
```
⇒ **doors 7 (memslot drop), 8 (`munmap`) and 9 (releasing a view) are ABSENT**, and 4 and 6 are
down to **one crossing each** — the window + memslot the guest's *first* latch write creates.
★ **worst move 39.8x better, mean 14.6x better**; the device arm's mean move (0.50 ms) is now at
the arena control's mean on the same box (0.44 ms) and its **worst is 6.8x better than the
control's** (1.12 ms vs 7.55 ms).

**What did it:** *"a device view cannot be re-pointed"* is true of the **node** and false of the
**window**. `QemuMachine::repoint_device_window` places a newly armed node into the window that is
already there with one `MAP_FIXED` — which is what §6.7 rule 3 and **constraint 16** below already
required. ⊘ Compliance, not a new design. Door 9 moved to the worker's reclaim tick behind a
decline-by-name, with the w735 release barrier **restated** (constraint 29) as a happens-before on
the `MAP_FIXED` sequence: `releasable_through` releases only views whose replacement has landed.

⚠ **CONSTRAINT 4 IS STILL VIOLATED, ON BOTH ARMS, AND THE CENSUS CANNOT SEE WHY.**
`worst_trap` = **24 999 µs at `bar0+0x110c00`** (device) and **25 077 µs at the same address**
(control) — `NV_PGSP_QUEUE_HEAD`, 96 % CPU (`cpu_of_that_trap=23979us`). It passes through **no
`assert_lock_free` door at all**, so the door census is structurally blind to it (§0.3 of
`fable_off_vcpu_design.md`). ⇒ w752 removed the PRAMIN move as the device arm's worst trap and the
two arms now agree on what is left; **a reduced door count is not constraint 4 satisfied.**

⊘ **Still open:** P3(a)/(b) (take the arm itself off the vCPU: 102/6 → 20/1) and §2.4(c) (the
one-time install moves to the BAR-map callback, constraint 23: 102/6 → 100/4).

### ⊘⊘⊘ OPEN VIOLATION — 4, 6, 7, 8 on the `device` arm, and the remedy is already specified

`[measured w742, both arms, one binary]`

    device arm:   VCPU-BLOCKING total=197 doors=9   worst_trap=44440us
    control arm:  VCPU-BLOCKING total=22  doors=3   worst_trap=22311us

⚠ **Constraint 4 says ALL traps sub-millisecond, with PRAMIN the ONE sanctioned exception.**
44 ms is three orders out, and the control's 22 ms means this did not start with the single
store — but the single store **multiplied the doors from 3 to 9 and the crossings from 22 to
197**, and its new doors are not PRAMIN.

★★★ **THE REMEDY IS NOT NEW WORK — IT IS CONSTRAINTS 6, 7 AND 8, WHICH ARE UNBUILT:**
6 (*every MMIO trap only posts the write to a queue, wakes a worker, and returns*),
7 (*epoll in workers*), 8 (*workers do all the work; completions land asynchronously*).
⊘ `completion_wait_architecture.md` (2026-08-09) measured the shape: *"There is no
completion-wait architecture. There is one synchronous inline executor on the vCPU thread…
The op finishes before the MMIO write returns."* ★ And the escalation seam is **already
written and named** — `Registrar::arm_counter` (`sources.rs:398`) → the child's relay thread
`signal()`s the `Notifier` → the parent `Reactor` blocks in `Poller::wait` (`reactor.rs:307`).
**Every piece exists; none is connected.**

★ **Owner's design, 2026-09-15 — plan of record for constraint 10**, better specified than what
was in the docs: translate in the scratchpad → **batch** the channel's CE/scrub work → start →
wait on the semaphore, escalating to eventfd only when slow → load new work meanwhile → each
completion is a semaphore write **then**, only if armed, an interrupt (armed *after* the
semaphore ⇒ send immediately, closing the race) → idle last. Blocking rule: while any semaphore
still spins the loop may not block (`epoll` timeout 0); block only once **every** outstanding
semaphore has a registered eventfd; the epoll set always contains the global new-work eventfd.
Refresh: exactly one at a time, **queued, never lock-contended**, one completion marker per each
of the three entrypoints.
⊘ **Two amendments, argued from measurement:**
1. ⚠ **Keep the semaphore as source of truth, as a CONSTRAINT not a note.** `await_semaphore`'s
   own comment is the argument: *"a poll cannot mistake 'we were never woken' for 'it never
   landed'."* An eventfd can, and absence-read-as-health is this campaign's dominant failure
   class.
2. ⊘ **Drop the work hand-off on refresh entry.** `[w675]` RM's **device-global client lock
   serialises one isolate** — four sockets allow four in flight, RM does not. Hand-off is
   machinery buying concurrency RM will not honour; the simple rule (a worker entering refresh
   holds no other work) is enough.

⊘ **Sequencing:** behind the raw client. A doorbell refused at channel birth
(`PassthroughDoorbellBirth=19`, `dec=NONE`, `GET=0 PUT=1`) is not fixed by better waiting — the
semaphore holds `Ok(0)`, **never written**, not written late.
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
isolate loads CUDA. ⊘ **One walk isolate per `(vm, gpu)`** — see §w724f below; *"per VM"* was
wrong and the implementation was already right.

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

- **`KfSetup`** (once per **`(vm, gpu)`**, immutable): `abi_version`, `table_version`, `levels[]`,
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

## 22 — WE DO NOT LIE ABOUT THE APERTURE

**Owner, 2026-09-14:** *"if the guest says this is now in vidmem, its in vidmem. We don't lie about
aperture anymore, even PT\*/PD\* aperture in vidmem is correct now. With this, I think it has a
higher chance that windows boots as its different nvidia code we don't see. We are more close to
hardware."*

★★★ **Today the guest's own page tables are FALSE.** A PTE says `APERTURE = VIDEO_MEMORY` and the
bytes are in host RAM behind a DMA mapping. Every consumer of that field — bandwidth expectations,
coherence rules, atomics behaviour, compression/comptaglines — is being told something untrue.
§18 named the residence half of this; **22 names the wider one: the aperture field itself.**

⇒ Under one reserved device-local object the field is **true**, for user buffers and for PT\*/PD\*
pages alike.

### ★ Why this is a STRATEGY argument, not a tidiness one

**Hardware faithfulness is the only approach that does not require having anticipated each check.**
Every lie must be matched by a place where we noticed the guest would check, and the set of such
places is only knowable for drivers we have traced. ⇒ For **Windows** — NVIDIA code this project
has never seen — a lie is a bug we cannot enumerate in advance, while the truth needs no
enumeration at all.

⊘ It also turns a class of would-be special cases into non-events: a comptagline on real video
memory means what the hardware says it means, where on substituted system memory it meant nothing.

## ⚠ w721 — THREE THINGS TO SETTLE BEFORE THE SHAPE IS COMMITTED

1. **Deletions come LAST.** reserved object → BAR1/BAR2 device views → walker wired → *then*
   delete fake fb, join, arena, demand-fill mirror, host parsing. ⊘ If deletion rides along, the
   tree is broken across a long stretch with no working intermediate and no way to bisect which
   half broke the raw client.
2. ⊘ **CORRECTED — "no populate on fault" is FORCED BY THE MECHANISM, not a property to achieve.**
   **Owner, 2026-09-14:** *"we already have it for vidmem in channels (we can't even handle a
   fault/trap there at all), so I think bar1/bar2 follows from the same constraint."* ★ Correct.
   When an engine reads video memory through a GPU VA there **is no trap**: the GPU finds a valid
   host PTE or it faults with an Xid. ⇒ Every channel mapping must **already** be complete before
   the engine runs — the C's *"fault-safe: a mapping is always backed before the engine that uses
   it runs"*. A device-view memslot is the same shape: a guest CPU access goes **straight to
   hardware with no VM exit**, so there is no trap to service.

   ⇒ The open question is therefore **narrower than "is it possible"**: *is our BAR1/BAR2
   publication actually **complete**?* Same mechanism as channels, different input — channel
   mappings come from the guest's GPU VA spaces, BAR1/BAR2 from their own page tables. The
   demand-fill mirror exists because publication historically was **not** complete; `TRAP_FILLS=0`
   says it now is. ⇒ Cheap test, unchanged: make the trap path **refuse by name**, boot, and see
   whether the refusal ever fires — it now proves **coverage**, not feasibility.
3. ✔ **ANSWERED w722, and it is WORSE than stated below.** `[measured, GA106/RTX 3060, 580.159.03]`
   Full write-up: `bar1_simultaneous_view_ceiling.md`.

   - ⊘ **No Resizable BAR.** BAR0 16 MiB, **BAR1 256 MiB**, BAR3 32 MiB. The problem does not vanish.
   - **Ceiling ≈ 253 MiB**, reproducing exactly, always `status=0x51 NV_ERR_NO_MEMORY` — **with
     `ioctl()` returning 0 and `errno==0`**, i.e. `failed_zero_is_not_nothing_refused` live on this
     path. ⊘ Reservation is **not** the constraint: **8192 MiB reserved fine** in the same process
     that could not map 254 MiB.
   - ★★★★★ **THE APERTURE IS ONLY RELEASED BY `NV_ESC_RM_UNMAP_MEMORY` (0x4F), AND WE NEVER CALL
     IT.** `grep -rn NV_ESC_RM_UNMAP_MEMORY crates/ | grep -v _DMA` → **the constant and three doc
     comments, ZERO call sites**, while the `_DMA` variant is wired in 8 files. Clean A/B with
     **fresh** offsets each round: *with* the ioctl, 224 MiB every round, 5/5; *without* —
     `munmap` + `close`, **which is what this tree does today** — round 0 gets 224 MiB and rounds
     1–4 get **zero**. ⇒ **Dropping a `DeviceView` is not a release.** Recycling on today's
     `export_device_view` would leak the aperture and then refuse everything.
   - ⊘⊘⊘ **ONE GLOBAL POOL, AND IT STARVES US TOO.** With 253 MiB held, host **`cudaMalloc` fails
     with `initialization error` — CUDA context creation itself needs BAR1** (~3 MiB). ⇒ An
     all-resident BAR1 makes the GPU unusable for everything else **including our own walk
     kernel's context and our own channels**. That is a deadlock shape: the guest's views would
     starve the mechanism that publishes them.

   ### ★★★★★ CORRECTED AGAIN, w722c — THE WORKING SET IS ~3.6 MiB, NOT 256 MiB

   **Owner, 2026-09-14:** *"But why do you need 256MiB+ mapped in slow mmio if guest doesn't use
   it?"* ★ Right, and it makes the ceiling nearly moot.

   ⊘ **We never map the aperture. We map what the guest has actually mapped in its BAR1 page
   tables.** `[measured, the parity boots]` — **912 distinct BAR1 pages ≈ 3.6 MiB** across a
   162 000-doorbell LLM workload, all premapped, `TRAP_FILLS=0`.

   | | |
   |---|---|
   | usable pool | ~254 MiB |
   | measured working set | **3.6 MiB** |
   | utilisation | **1.4%** |

   ⇒ **All-resident is comfortable by ~70x.** No aperture resizing, no recycling design, no
   sizing gymnastics. ⚠ I reasoned from the **aperture** when the number that matters is the
   **working set** — the same error twice in one exchange: generalising from a bound instead of
   from a measurement.

   ### ⇒ What is actually left

   1. **Enforce a budget.** A guest *may* legally fill its aperture, and nothing stops it. Left
      unbounded it can starve our CUDA context and with it the walker. ⇒ A **checked** bound, not
      an assumed one — cheap, and it is the whole of the "pathological guest" story.
   2. ★★★ **CUMULATIVE CHURN is the real reason for the release verb — not capacity.** The
      instant working set is 3.6 MiB, but the guest **re-points** BAR1 entries over time, so what
      accumulates without release is **every distinct (BAR1 page, GPGA) pair over the whole boot**.
      Across 1178 refreshes that can exceed 254 MiB although no instant ever does. ⊘ And the
      failure is silent: rounds 1–4 got **zero** with `ioctl()` returning 0 and `errno==0`.

   ### ⊘⊘ SUPERSEDED — the sizing argument below, kept for its reasoning

   ### ⊘⊘ CORRECTED w722b — "IMPOSSIBLE" WAS AN OVERSTATEMENT, and 256 MiB MUST NOT BE HARDCODED

   **Owner, 2026-09-14:** *"Yeah 256MiB is vast limit. Not something to hardcode. Why is it then
   impossible?"* ★ Both halves are right.

   **(a) 256 MiB is ONE measured board, not a spec.** BAR1 size varies by board and by whether the
   host enabled **ReBAR**; datacenter parts ship with a large BAR1 natively. ⇒ It is a **queryable
   property** — `derive_per_die_maintain_per_family`. **Nothing may compare against a literal.**

   **(b) The real relation is a SIZING constraint, not an impossibility:**

       all-resident works  ⟺  advertised_guest_BAR1 + our_headroom  ≤  host_BAR1

   I collapsed that to *"impossible"* because on this box both sides are 256 MiB. ⊘ **But we choose
   the left-hand side.** The guest's BAR1 aperture is **advertised by us**, not inherited: advertise
   128 MiB and the same board leaves ~125 MiB of headroom. A 128 MiB-BAR1 GA106 is a real hardware
   configuration, so that is **a different truthful board, not a lie** — §22 intact.

   ⇒ **Query host BAR1 at startup, size the guest's aperture to fit inside it minus a reserved
   budget, and all-resident holds by construction.** Recycling is the **fallback** for hosts too
   small for the aperture we want — not the mandatory design.

   ### ⇒ What survives the correction, unchanged

   1. ★★★ **The release verb is required either way.** Even fully resident, the guest **re-points
      its BAR1 page tables over time** — the same BAR1 offset names different GPGA as its own RM
      manages the aperture. So views must be torn down and re-established, and `munmap` + `close`
      demonstrably returns **nothing** to the pool (rounds 1–4 got **zero**). Without
      `NV_ESC_RM_UNMAP_MEMORY` we leak until we refuse, **however we size things**.
   2. ✔ **DONE 2026-09-14/15 — ~~Build the release verb FIRST.~~** ~~`NV_ESC_RM_UNMAP_MEMORY` has
      no caller~~ — it now has **nine** references across the tree (`release_device_view`,
      `release_cpu_view`, the wire verb on tag **31**). `Nvos34Parameters` is in `kayfabe-abi`
      with every byte offset pinned, and the crossing was **exercised end to end in a boot**:
      `DEVICE_VIEW=OK … released=true`.

      ⚠ **This row said "no caller" for a day after it stopped being true**, and it is the second
      stale claim found in this file in one session (the other: the index stopped at 17). ⇒ **A
      "not built yet" row is a claim with an expiry, and nothing expires it automatically.** When
      the thing gets built, the row that said it was missing is the first place to look — not the
      last.
   3. **Reserve headroom for ourselves** — our CUDA context and channels come out of the same
      254 MiB. A budget the guest cannot consume, enforced, not hoped for.

   ⚠ Two instrument failures were caught en route, both of which would have shipped wrong numbers:
   a bisect reporting *"largest single map = 128 MiB"* was an artefact of the leak in the previous
   probe; and the first reclamation test printed **"APERTURE IS FULLY RECLAIMED"** as a **false
   positive**, because it remapped the **same** offsets it released — which succeeds even when
   nothing was returned. ⇒ **Only fresh offsets test reclamation.**

## ⊘ SUPERSEDED — the original statement of item 3, kept for its reasoning

3. ★★★ **HOST BAR1 IS THE BINDING LIMIT, AND IT IS SMALL.** `[measured]` a **256 MiB CPU view
   refused with `NoMemory` while a 6144 MiB reservation succeeded in the same process.** Host BAR1
   on a GA106 is **256 MiB total, shared with the host driver**; our advertised guest BAR1 is
   **also 256 MiB**.
   ⇒ **The reservation is bounded by video memory; simultaneous CPU views are bounded by BAR1.**
   Those are different numbers by a factor of ~24, and the design currently assumes only the
   first. This may force BAR1 views to be **recycled** rather than all-resident — which is what
   PRAMIN does, and why the hardware has one. **Settle before the reserved object lands.**

## ★★★★★ w724c — 22 IS SYMMETRIC: sysmem must be sysmem too

> **Owner, 2026-09-14:** *"vidmem is vidmem when guest asks it, DMA must still work when the guest
> asks it explicitly, no lying."*

§22 as written names only one direction. The rule is **symmetric**:

| the guest asks for | it must be |
|---|---|
| video memory | the **reserved device-local object** |
| **system memory, DMA-mapped** | **real host memory, reached by DMA** |

⊘ Having a large reservation to hand makes the second lie **easy and tempting** — "it's faster in
vidmem" — and it is the direction **nobody is watching**, which is exactly why it must be written
down. A guest that explicitly asks for a DMA-mapped sysmem buffer and silently gets device memory
has been lied to just as much as one whose vidmem is sysmem, and the consequences are the same
class: every property it derives from the aperture (coherence, host visibility, bandwidth,
lifetime) is wrong.

★ **No lying, either way.** The aperture the guest names is the aperture it gets.

## ⊘⊘ REINSTATED w735 — THE RULE HOLDS AFTER ALL; ONLY ITS REASON WAS WRONG. Read this before the refutation below.

The sequence, because the middle state is the misleading one:

| | |
|---|---|
| **w724c (mine)** | *"§6 must precede §3 — the intermediate does not boot"*, on a **derived** byte cost |
| **w734i** | ⊘ the byte cost is **refuted by measurement** — the intermediate costs **5–10 s**, it boots |
| **w735** | ✔ **the rule is REINSTATED on a different, structural ground: a LOCK RANK** |

★★★ **Arming a device view is an IPC round trip that asserts lock-free — and every host-side reader
of the store holds `LockRank::PlaneMem`, two of them on a vCPU inside an MMIO exit.** So the store
**cannot arm**, and every host-side consumer must arm *before* the lock. Of the four, only
`PlanePtBytes` can; `FbStoreReader`'s callers need a demand set; and the **CPU CE executor runs
inside `ce_session_with_root` and DROPS a refused submission** — the CeUtils-wedge class.

⇒ **§6 step 3 and constraint 9 are prerequisites of a clean §3.** Full argument: §w735 below.

### ⊘⊘⊘ AND THE LESSON IS ABOUT THE REFUTATION, NOT THE RULE

w734's census was **pre-registered honestly and measured the wrong dimension**: it measured
**seconds**, and **no number of seconds can reach a lock rank.** ⇒ **A measurement can refute a
stated reason while leaving the rule standing** — and a refutation is only as wide as the dimension
it measured.

⚠ I recorded *"the order is no longer forced"* on the strength of w734i. That was wrong, and it was
wrong in the most expensive direction: had anyone acted on it, they would have reordered the branch
around a bandwidth number and hit a lock-rank wall that no timing could have predicted.

## ⊘⊘⊘ REFUTED w734i — the byte cost below is wrong, and both terms are now measured

`[measured, vast 51082161, RTX 3060 GA106, 580.159.04, traces/w734_fbio_census/]` — raw client
`(P)`, 8/8 threads.

| term | what the section below assumed | **measured** |
|---|---|---|
| read rate through a device view of the reserved object | 48 MiB/s *(uncited)* | **52.5 MiB/s** — ★ the rate was **right** |
| **write** rate | assumed the same | **4987.5 MiB/s** — **95× faster**, and nobody had ever measured it |
| walk traffic per boot | **8.6 GiB** (`7.3 MiB × 1178`, derived) | **275.5 MiB** |
| ⇒ cost of the "impossible" intermediate | *"~3 min … **it does not boot**"* | **5–10 s** |

⇒ ★★★ **The RATE was right; the VOLUME was wrong by ~30×**, because the derivation assumed the
whole resident table set is re-read **every** refresh, and it is not. **The intermediate boots.**

⇒ **§w724c's conclusion is withdrawn, and with it the claim that §6 was FORCED to precede §3.**
⊘ The aperture cost — the one nobody had named, and the only candidate for a second reason — fits
with **500× margin**.

★ Honest about its own precision: the 5–10 s range is a range because `walk-bar`'s **3 454 311**
reads average **~41 bytes**. Those are 8-byte point-walk entry reads from
`bar1_translate`/`bar2_translate` — **latency-bound, not bandwidth-bound** (~1 µs per uncached BAR
round trip ⇒ ~3.5 s on top of the 2.6 s the bytes alone predict). A pure byte model understates
them.

### ⚠ What this does NOT overturn

§6 is **done and sound**, and the walker is wanted on its own merits: `[w726]` 205.7 µs against a
walk the host does far more slowly. ⇒ **The ordering was not wrong, its stated REASON was** — and a
rule whose recorded reason is false is one nobody can re-derive when it matters. ⊘ That is the
whole cost of a derivation cited as a measurement, and it was mine.

## ⊘ SUPERSEDED — the original section, kept for the reasoning it got right

## ⊘⊘⊘ CORRECTED w734 — THE SECTION BELOW CITES A DERIVATION AS A MEASUREMENT

`[surveyed w734]` Both *"§6 must precede §3"* and the section below's *"there is no working
intermediate — it does not boot"* turn on **one quantity**: how many bytes the host reads out of
the framebuffer store per boot. Both state it as **7.3 MiB × 1178 refreshes @ 48 MiB/s ⇒ ~3 min**.

⊘ **No byte counter for store I/O exists anywhere in this tree.** The only instrument is
`pages_swept` — a **page** count, at six page sizes of which three are **not** 4 KiB
(`ga10x.rs:842`: PD3 = **32 B**, PT_BIG = **256 B**). ⇒ Both documents turned pages into MiB **by
assumption**, and the ordering rule of this whole branch is therefore **a derivation cited as a
measurement** — this tree's own most expensive recurring failure, committed by me, in the file that
exists to prevent it.

⚠ **What this does and does not overturn:**

- ⊘ The **number** is unmeasured. 7.3 MiB assumed 1872 × 4 KiB; with 32 B and 256 B levels in the
  mix the true figure could be **materially lower**, and the intermediate correspondingly cheaper.
- ★ The **ordering may still hold on a second, structural ground that does not depend on bytes**:
  after the switch, `window_leaves` reads the guest's tables out of **video memory through a CPU
  aperture**, and host BAR1 is a **single global pool** — so the cost is bounded by a scarce
  *resource*, not merely by a rate. ⊘ That argument was never the one written down.

⇒ **w734 is measuring it properly**, by role (`trap` · `walk-bar` · `walk-guest-pt` · `out-of-band`
· `cpu-ce`), at the six entry points that reach `FbStore`. **Do not treat the section below as
settled until that census lands** — and if the number comes back small, the forced ordering is an
open question again rather than a rule.

### ★★★ TWO THINGS THE TABLE ABOVE DOES NOT CARRY, AND THE FIRST IS A WHOLE COST NOBODY HAD NAMED

⊘ **The aperture term.** A device view is mapped from **file offset 0 only**
(`nvidia_mmap_helper` refuses `vm_pgoff != 0`, `window_unsafe.rs:251-253`), so a device-backed
store needs **one armed node per contiguous run**, and an armed node costs host BAR1 — §22 item
3's 256 MiB, shared with the host driver. ⇒ **bytes and frames are bounded by completely
different things**: bytes by the bus (*a slow boot*), frames by the aperture (*does not fit, at
any speed*), and they can point opposite ways. Measured across four boots:

| | measured |
|---|---|
| distinct frames the walk touched, whole boot | **107 – 131** ⇒ **0.4 – 0.5 MiB** of 256 |
| cost to arm one view / give it back | **`arm_us` 241–273** / **`rel_us` 409–446** ⇒ ~0.7 ms a round |

⇒ the term that could have made this *"does not fit, at any speed"* fits with ~500× margin, and
the per-view tax — which nothing had costed — is under a millisecond.

⚠ **And two caveats the numbers do not carry themselves.** `[w734]` This is **ONE WORKLOAD**,
the raw client: a boot that ran the 30-arm guest suite cascade-failed on its **pre-existing**
`--concurrency`/`--engines` wedge and cascade-skipped 26 arms, so its census is a **narrower**
workload (146 MiB), never a wider one. ⊘ The same caveat §w727 attaches to `BAR1_MIN` applies
here with equal force. And `HOST_DMESG_XID=1` on those boots is the pre-existing
`Xid 31 … ENGINE CE0 … FAULT_PDE @ 0xa0_00000000` (w555/w711) — neither caused nor fixed by any
of this.


## ★★★★★ w724c — WHY THE SWITCH CANNOT BE INCREMENTAL: THERE IS NO WORKING INTERMEDIATE

> **Owner, 2026-09-14:** *"at 47MB/s it might not even boot in the timeout without dirty bit during
> refresh. So thats also why its basically not worth trying."*

⊘⊘⊘ The intermediate — **backing on real video memory, host walker still reading the tables** —
does not merely cost machinery we would delete. **It does not boot.**

| | per refresh | × 1178 refreshes |
|---|---|---|
| 7.3 MiB resident tables @ **48 MiB/s** | 152 ms | **~3 minutes** |
| 24 MiB worst case @ 48 MiB/s | 500 ms | **~10 minutes** |

⇒ Every refresh, because **the dirty tracking lived in the fake fb** — a memslot over host memory.
Removing it takes away the dirty signal **and** puts a 77x slower bus in the same step. The guest's
own boot timeouts fire long before that finishes.

### ★★★ AND THIS REFRAMES WHAT THE FAKE FB IS FOR

Not *"an optimisation for slow CPU reads"*. Its real function is to make page-table reads **cheap
enough that no dirty tracking is needed at all**: 7.3 MiB at **3674 MiB/s** is ~2 ms, so ~2.3 s a
boot — you can afford to re-read **everything, every time, forever**.

⇒ **The fake fb can only be removed once something else makes refresh affordable.** The PTX walker
is not an accelerator bolted on afterwards; it is the **precondition** for deleting the fake fb.
Every other ordering produces a tree that does not boot.

⇒ **Order is forced:** the walk kernel works and passes its tests → it runs inside kayfabe → the
backing switches → the fake fb goes. `SINGLE_STORE_PLAN.md`'s increments 4 and 6 are therefore
**gates**, not steps.

## ★★★★★ w724d — THE SCRATCHPAD ISOLATE IS SPECIAL: dynamically linked, sandboxed LATER

> **Owner, 2026-09-14:** *"the scratchpad isolate is special: (1) before entering mount namespace
> and chroot, it first opens libcuda and inits and loads the entire channel and PTX ensure its
> running; (2) then it drops privileges as usual; (3) during this time it actually links to libc on
> the host; (4) isolates NOT for scratchpad are unaffected."*

> #### ⊘⊘⊘ CORRECTED 2026-09-14 (increment 4, BUILT AND BOOTED) — **THE ORDERING BELOW IS
> #### NECESSARY AND NOT SUFFICIENT.** Two things it does not say, both about this process's
> #### NAMESPACES rather than about its link mode.
>
> **1. `cuInit` refuses inside the isolate's PID namespace, at ANY ordering.**
> `[measured, RTX 3060, 580.159.04, in a real boot]` the glibc image exec'd, `dlopen` of
> `libcuda.so.1` **succeeded**, every symbol resolved — and `cuInit` returned
> **`CUDA_ERROR_OPERATING_SYSTEM` (304)**. Bisected one namespace at a time:
>
> | namespace | `cuInit` |
> |---|---|
> | user / mount / net / ipc / uts, each alone | **0** |
> | **pid** | **304** |
> | pid + a REMOUNTED `/proc` | **0** |
> | user + pid + mount + a remounted `/proc` | **0** |
>
> ⇒ **It is not the PID namespace; it is a PID namespace whose `/proc` is still the
> PARENT'S**, so `/proc/self` resolves through host PIDs while the process sees its own.
> ⚠ The ordering below cannot address this: an isolate is **born namespaced** — the
> namespaces come from the `clone` that CREATES it, which is the only way `CLONE_NEWPID` can
> be had at all — so there is no moment in its life at which CUDA could have initialised.
> ★ The fix is a **mount, not a weakened boundary**
> (`kayfabe_linux_raw::sandbox::remount_proc`): dropping `CLONE_NEWPID` would hand one
> isolate visibility of every process on the host, permanently, to fix what four syscalls fix
> completely — and the mount is transient, because `sandbox::enter` puts a tmpfs over `/proc`
> a moment later.
>
> **2. ⚠ THE PRIVILEGE DROP IS PER-THREAD ON LINUX, AND CUDA'S THREADS EXIST BY THEN.**
> `capset()` with `pid == 0` changes the capabilities of the **calling thread**;
> `PR_SET_NO_NEW_PRIVS` and `PR_CAPBSET_DROP` are per-process, but the effective and
> permitted sets are not. Step 2 spawns NVIDIA driver threads and step 3 then drops privilege
> on the thread that calls it. ⊘ `surrender_privilege`'s read-back reads `/proc/self/status`,
> which reports the **calling thread**, so it would pass while other threads still held
> capabilities — *"we dropped them"* stays a checked outcome for one thread and becomes an
> assumption for the rest.
> ⊘ **NOT MEASURED, and said so rather than assumed either way** — a per-thread census needs
> `/proc/self/task/*/status` *after* the drop, and `/proc` is a tmpfs by then. It is reachable
> through a dirfd opened before the sandbox; that is the instrument this needs and does not
> have. ⇒ constraint 20's security argument needs a third qualification beyond *"the process
> ends with the same reach"*: **the thread that dropped does; its siblings are unverified.**

### The blocker this answers

★★ **CONFIRMED, MORE GENERALLY, AND WITHOUT A GPU** `[measured 2026-09-14, locally]`: a musl
**static-pie** binary's `dlopen` returns `NULL` with `dlerror()` = *"Dynamic loading not
supported"* for `libcuda.so.1`, `libc.so.6` and `libm.so.6` **alike**. The refusal is
**musl's**, and it arrives before any question about CUDA is asked — so the blocker is not
*"libcuda is the wrong kind of shared object"* but *"there is no dynamic linker in this
process"*. ⊘ It was expected to need a CUDA container; it did not, because the mechanism is
one layer below the thing the claim names.

⊘ Every other isolate is built `<arch>-unknown-linux-musl`, **static**, and `exec`'d from a memfd
inside a mount namespace with **no path to a dynamic loader** (`kayfabe-isolate-host/build.rs:1-51`;
the sandbox is documented as *"gated on a static binary"*). **`libcuda` is a glibc shared object,
and musl static binaries do not support `dlopen` at all.** ⇒ The scratchpad isolate as built could
not have loaded CUDA, and *"init before dropping privilege"* would not rescue it — there would be
nothing to init with.

### ★★★ The ordering is the whole design

1. `exec` **while the loader is still reachable** — the binary is glibc-linked, not musl-static.
2. **Full CUDA bring-up**: `cuInit` → `cuDeviceGet` → `cuCtxCreate` → `cuModuleLoadData` (**the PTX
   JIT runs here**) → channel up → **a real warm-up launch**.
3. **Then** enter the mount namespace, chroot, and drop privilege as every other isolate does.
4. ⊘ **No other isolate is affected** — they keep the static-musl, memfd-exec shape and its
   sandbox-first guarantee.

★ Why it holds: **open fds, existing mappings, the CUDA context and the loaded module all survive a
namespace change.** Only **path lookups** do not. ⇒ Step 2 exists to walk every lazy path *before*
there are no paths — CUDA is aggressively lazy, and each lazy path is one that would otherwise fail
after the drop, looking like a GPU fault rather than a sandbox effect.

### ⚠ The empirical risk, and it is cheaply testable

**CUDA must touch nothing by path after the drop.** A warm-up launch covers the normal path; it
does **not** exercise **error and recovery** paths, which may reopen a device node or read `/proc`.
⇒ Probe specifically: after namespace + drop, (a) can it launch again, (b) can it survive and
report a deliberately failed launch without reopening anything.

### ⊘⊘ CORRECTED w724e — THE WINDOW IS NOT A COST, IT IS STRUCTURALLY EMPTY

> **Owner, 2026-09-14:** *"the scratchpad channel is started before VM boots (the full init), in
> that frame before sandboxing it only has run trusted code so it cannot be tainted. Plus, the
> scratchpad doesn't even run guest ptx code at all, it only runs our trusted ptx program and CE
> utils like scrub and copy."*

★ I recorded the later sandboxing as a *"bounded but real"* cost. That was over-cautious. The
isolate is spawned at **PCI realize**, during VM construction, **before the guest's first
instruction** — so during the unsandboxed frame there is **no guest, no guest data, and nothing
untrusted has executed.** ⇒ A window is an exposure only if something can exploit it, and the thing
that would **does not yet exist**.

★★★ **And the running state is tighter still: the scratchpad channel never executes guest code.**
Our PTX, CE scrub, CE copy — all ours. The guest's kernels run on the guest's **own** channels.

⇒ The threat model collapses to:

| | |
|---|---|
| **code** | ours, established when nothing untrusted existed |
| **data** | the guest's page tables, and only that |
| **surface** | **memory safety over data** — ~200 auditable lines, 58 hostile cases, three structural invariants |

### ⚠ The one caveat, and it lands where the other risk already does

This holds **as long as CUDA init genuinely precedes the guest**. Anything that re-initialises
**later** — context recreation after an error, a hot-added second GPU — happens **with a guest
live**, and neither half of the argument covers it: the frame is no longer untainted, and the
process is no longer pre-sandbox.

⇒ Same **error/recovery path** already named above for post-drop path lookups. **One probe covers
both**, and both are reasons to treat CUDA re-initialisation as a refusal rather than a fallback.

⊘ §20's *"`libcuda` is initialised … before the isolate drops privilege"* stands; what changes is
that this isolate is a **different build** from the others, and that is now stated rather than
assumed.


## ★★★★★ w724f — MULTI-GPU IS STILL AN AXIS: one scratchpad, and one PTX launch, PER GPU

> **Owner, 2026-09-14:** *"multiple gpu still remains an axis, so multiple scratchpads if needed
> each with a ptx for tables in that gpu."*

⊘ **My docs said "one walk isolate per VM" — wrong.** The implementation was already right:
`Spine::install_isolate(proc, gpu, iso)` is keyed by **`(ProcId, GpuId)`**, with
`SCRATCHPAD_PROC = u32::MAX` standing for *"the VM"* (`scratchpad.rs:518-520`). ⇒ The scratchpad is
**already per `(vm, gpu)`**, and the correction is to the prose only.

### What is per-GPU, and none of it is optional

| | why |
|---|---|
| **GPGA space** | GPGA `0x1000` on GPU 0 and on GPU 1 name **different memory** (`gpgaview.rs`) |
| **the reserved object** | one per card, and the reservable size **differs per card** |
| **the scratchpad channel + its CE** | a channel belongs to one GPU |
| **the CUDA context** | bound to a device |
| **the walk launch** | reads *that* GPU's tables, through *that* GPU's GPGA mapping |
| **`KfSetup`, including the format descriptor** | ★ see below |
| **the BAR1 budget** | an aperture is a property of a card |

### ★★★ AND THE FORMAT DESCRIPTOR BEING PER-GPU VALIDATES §21 RATHER THAN MERELY OBEYING IT

A mixed-generation box — say **Turing alongside Blackwell** — needs **VER2 and VER3 live
simultaneously, in one kayfabe process**. ⊘ That is unserviceable by a kernel with bit positions
compiled in, and it is *not* solved by shipping two CUDA programs either: the two GPUs are driven
from the same process at the same time.

⇒ **One PTX, two descriptors, launched per GPU** is the only shape that works — which is exactly
what §21 requires for a reason that was about maintenance and turns out to be about **capability**.

### ⚠ The open multi-GPU question: PEER apertures

A VER2 PTE's aperture nibble is `0 vid, 1 peer, 2 sys-coh, 3 sys-noncoh` — so a guest PTE can name
**another GPU's memory**. The kernel already *decodes* that nibble; what we **serve** for it is
undecided, and it is the one place where a per-GPU walk meets a cross-GPU mapping.

⊘ Related and unresolved: `the_viewspace_citation_nobody_audited` records peer memory being deferred
while citing a type that did not model the GPU axis. ⇒ **Do not let the single-store work assume
peer never appears**; decide it explicitly, even if the decision is a named refusal.

## ⊘⊘⊘ w726 — §w724d's ORDERING IS NECESSARY BUT NOT SUFFICIENT, and §20 needs a third qualification

`[measured w726, increment 4]` Both corrections come from building it.

### 1. The blocker was one layer below where I named it

A static-pie **musl** binary's `dlopen` returns `NULL` with `dlerror()` = **`"Dynamic loading not
supported"`** — for **`libc.so.6` and `libm.so.6` as much as for `libcuda.so.1`**. ⇒ The refusal is
**musl's**, and arrives **before CUDA is asked about**. The blocker is *"there is no dynamic linker
in that process"*, not *"libcuda is the wrong kind of shared object"*. ★ No hardware was needed;
I proposed a container-hour to establish something a local run settled.

### 2. ★★★ Ordering alone does not work — an isolate is BORN namespaced

`cuInit` refused **`CUDA_ERROR_OPERATING_SYSTEM` (304)** *after* `dlopen` had succeeded. Bisected
one namespace at a time:

| namespace | alone |
|---|---|
| user / mount / net / ipc / uts | **pass** |
| **pid** | ⊘ **fail** |
| **pid + a remounted `/proc`** | ✔ **pass** |

⇒ **It is not the PID namespace — it is a PID namespace whose `/proc` is still the parent's.**
⊘ **No ordering could have fixed this**, because the isolate does not *enter* namespaces after
running: it is **born** in them. §w724d's *"init before you sandbox"* is necessary and insufficient.

★ The fix is **a mount, not a weakened boundary** — dropping `CLONE_NEWPID` would give one isolate
permanent visibility of every host process to solve what four syscalls solve — and it is
**transient**: `sandbox::enter` puts a tmpfs over `/proc` moments later.

### 3. ⚠ §20's PRIVILEGE CLAIM NEEDS A THIRD QUALIFICATION — and it is unmeasured

**`capset` is per-thread on Linux.** CUDA's driver threads exist by the time privilege is dropped,
and `surrender_privilege`'s read-back reads **`/proc/self/status`** — *the calling thread only*.

⇒ §20 may say *"the thread that dropped did"*. It may **not** say *"the process is unprivileged"*
without reaching the siblings, which needs a dirfd opened **before** the sandbox. ⊘ **That
instrument was not built and the sibling threads are unverified.** Recorded as an open gap rather
than assumed away — this is exactly the shape where a check that covers one thread reads as a check
that covers the process.

## ★★★★★ w727 — BAR1/BAR2 ARE SIZED OPTIONS, like VRAM, with enforced minimums

> **Owner, 2026-09-14:** *"just like VRAM size, where you can select how much vram to give to the
> guest, so can you select how much bar1/bar2 to give as option with also minimums set to
> function."*

### Why it is needed, measured

`[measured w726/e36]` on the bench GA106:

    BAR1-BUDGET host_bar1=256 MiB  advertised_guest_bar1=256 MiB  headroom=16 MiB
      ⇒ ⊘⊘ DOES NOT FIT — 256 + 16 > 256

⇒ **All-resident BAR1 cannot work while we advertise the host's entire aperture**, because our own
CUDA context and channels come out of the same pool. §22(b) already established that **we choose
the guest's side**, so this is the knob that makes the relation satisfiable rather than a refusal.

### The rule

BAR1 and BAR2 join the framebuffer as **operator-selectable sizes**, derived and checked at
startup, **never a literal**:

    guest_bar1 + our_headroom ≤ host_bar1        (queried, §22 item 3)
    guest_bar1 ≥ BAR1_MIN                        (enough to function)
    guest_bar2 ≥ BAR2_MIN

★ A 128 MiB-BAR1 GA106 is **a real hardware configuration**, so advertising one is a *different
truthful board*, not a lie — **§22 stays intact**. That is the same argument that licensed deriving
the framebuffer size from the reservation rather than asserting 12288.

### ⚠ Three things the knob must respect

1. **PCI BAR sizes are powers of two.** The choice is 64 / 128 / 256 MiB, not an arbitrary number —
   a "select any size" knob that accepts 100 MiB is a bug the guest's enumeration finds, not us.
2. **The minimum is a measurement, not a guess.** `[measured]` the LLM workload's BAR1 working set
   is **912 pages ≈ 3.6 MiB**, so 64 MiB has ~18x margin — but that is **one workload**. ⇒ `BAR1_MIN`
   must be justified by the census across the **guest suite**, and until then set conservatively and
   say it is provisional.
3. **Refuse at startup, loudly, never silently clamp.** A guest booted with a BAR too small for its
   driver fails somewhere unrecognisable. ⊘ And do not gate on it before anything consumes BAR1
   views — a refusal that fires every boot for a design not yet switched on is noise that teaches
   people to ignore the check.

⊘ **Not yet decided: what `our_headroom` must contain.** Known members: the CUDA context (~3 MiB
measured) and our own channels. Unknown: whether a second GPU, a second VM, or the host driver's own
growth share the pool — §22 item 3 measured **one global pool**, so they may.

## ★★★★★ w729 — WHEN STUCK: ASK FABLE. AND NEVER PASS BY RELAXING A CONSTRAINT

> **Owner, 2026-09-15:** *"stop when you are blocked/stuck. I would recommend at that point first
> ask fable to fix it for you or find a fix, then use that, and if fable has the same conclusion
> then also stop. Do not forget to tell fable the same constraint document. You and fable may both
> come up with something better, then you may try it, but its very important to mention it when the
> constraints are relaxed. What I would not recommend is to get something to pass just by relaxing
> constraints."*

### The escalation, in order

1. **Stuck ⇒ ask Fable** (Agent tool, `model: "fable"`), and **give it this document**. A fix that
   does not know the constraints is not a fix.
2. **Fable finds a way ⇒ use it.**
3. **Fable reaches the same conclusion ⇒ stop and discuss.** Two independent dead ends is
   information, not a reason to improvise.

### ⊘⊘⊘ THE RULE THAT MATTERS MOST — a pass bought by relaxing a constraint is not a pass

★ **A green obtained by loosening a constraint is a green for a different product.** The constraints
are not obstacles the work must survive; they **are** the product — an unprivileged host, a hostile
guest contained, no lying about apertures, no blocking a vCPU. Relax one and the number that comes
back is measuring something nobody asked for.

⇒ **If a constraint is relaxed, even temporarily, it must be:**

| | |
|---|---|
| **said in the report** | first line, not a footnote |
| **said in the commit message** | so `git log` carries it |
| **written into this file** | with the date, so the next reader inherits it |

⇒ And **two questions answered explicitly**, because relaxing is sometimes right:

1. **Why is this relaxation not needed to reach parity or good performance?** If the relaxation is
   what produces the number, the number is not evidence for the design.
2. **Why is it not needed for a multi-tenant VM product?** ⚠ The constraints exist *because* of that
   goal. Relaxing one means either the goal changed or the constraint was wrong — **name which**.

⊘ *"It passes with X turned off"* is a measurement of X, not a result. ⊘ *"Temporarily"* is a claim
about the future; this file already records that gates become permanent unless born with an expiry
condition (§w724g).

★ **Something genuinely better is allowed and wanted** — several constraints in this file were
*improved* by exactly that (15 and 19 superseded, 18 satisfied by construction, 22 widened to be
symmetric). The rule is not "never change a constraint". It is **never change one silently, and
never to make a test go green.**

### ★★★ A NEGATIVE RESULT IS A DELIVERABLE

> **Owner, 2026-09-15:** *"so stopping saying I cannot satisfy these constraints but my alternative
> that doesn't use it is not better either, has bad performance or is unstable, thats a conclusion
> also worth tomorrow."*

⇒ **"I could not satisfy the constraints, and the alternative that abandons them is no better —
here is the measurement"** is a **result**, not a failure to report one. It maps the design space,
which is the thing that was actually unknown.

★ Stated because the pressure at the end of a long autonomous run points exactly the other way:
toward arriving with *something* green. ⊘ A green bought by relaxing a constraint (above) is worth
**less than nothing** — it costs the session *and* leaves a false record. A measured dead end costs
only the session.

**What makes a negative result good rather than a shrug** — all four, or it is a shrug:

1. **What was tried**, concretely enough to not be retried by accident.
2. **Where it stopped**, by name — a refusal, a number, a wall — never *"it didn't work"*.
3. **What the alternative cost**, measured: *"bad performance"* is a claim; **`N ms` against a
   budget of `M`** is a result.
4. **What would change the answer** — the measurement, ruling or capability that would reopen it.

⊘ And the alternative's numbers are held to the **same** standard as the constraint-honouring path:
a comparison against an unmeasured alternative is not a comparison. ⚠ This tree already records the
shape — *"the CPU path costs ~3 min"* was only meaningful once someone measured the GPU path at
462 ms and then again at 205 µs.

## ★★★★★ w735 — THE SINGLE STORE'S WALL IS A LOCK RANK, NOT A BANDWIDTH

`[established from the source, 2026-09-15, building §3]` No constraint was relaxed to reach
this; it is what building the switch found.

§w724c said the intermediate *"does not boot"* and costed it in `bytes ÷ 48 MiB/s`. w734i
measured both terms and refuted the arithmetic — 275.5 MiB of walk traffic, 5–10 s, 0.5 MiB of
a 256 MiB aperture. **Both of those readings are about throughput, and the wall is not
throughput.**

**Arming a CPU view of the reserved object is an IPC round trip that asserts lock-free
(`kayfabe-isolate/src/lib.rs:3477`); every host-side reader of the framebuffer store holds
`LockRank::PlaneMem` when it reads (`plane.rs:2992`, `:5632`, `:5734`), and two of them run on
a vCPU inside an MMIO exit.** ⇒ **the store cannot arm**, so every host-side consumer of
framebuffer bytes must arm *before* it takes the lock. Full table, and which of the four can:
`SINGLE_STORE_PLAN.md`'s w735 block.

⇒ **§6 step 3 and constraint 9 are prerequisites of a clean §3** — they are what delete three
of those four consumers. This does not overturn §22, §18 or *"no populate on fault"*; it says
where the work is.

### ⚠ THREE THINGS THIS CHANGES ABOUT HOW THE REST OF §3 IS COSTED

1. ⊘ **A pre-registered threshold is only as good as its AXIS.** w734's Q4 was written down in
   advance, honestly, and in **seconds** — and the wall is a lock rank, which no number of
   seconds could have reached. Choosing the axis is the part nobody reviews.
2. ⊘⊘ **`FbTrapPolicy::Serve` becomes IMPOSSIBLE on the device arm, and that is a scoping, not
   a relaxation.** Under the arena an unslotted guest access was *served* by the trap. Under
   one object there is nothing to serve it from, so **publication completeness stops being a
   performance property and becomes a correctness one**. `TRAP_FILLS=0` is one workload
   (§w727's own rule), and the 30-arm suite is wedged on a pre-existing `--concurrency` bug.
3. ⊘ **§18 is NOT satisfied by §3 finishing.** `KAYFABE_FB_JOIN` defaults to `off` and `Off`
   materialises nothing, so the guest's **engines** see no guest vidmem today; after §3 the
   guest's **CPU** sees the object and the engines still do not. Engine-side slices need a
   cross-isolate handle, which the foreign-handle gate (`isolate/lib.rs:3477`) currently
   refuses and for which `DUP_OBJECT` exists only as a doc mention. ⇒ *"satisfied by
   construction"* is a claim about the end of a longer road than §3.

### ⊘⊘⊘ AND ONE DEFECT THAT WOULD HAVE SHIPPED — release ordering, silent and cross-tenant

*"Evict the slot, then release the view"* is wrong. `QemuMachine::remove_window` **parks** the
mapping rather than unmapping it (an accessor may still hold the `Arc`), and RM's
`osUnmapPciMemoryUser` is an **empty function** (`ogkm os.c:1275-1282`) — so
`NV_ESC_RM_UNMAP_MEMORY` returns the aperture **without touching the VMA**. Releasing early
leaves live PTEs pointing at BAR1 space RM has re-handed to the next mapping: ours, the host's
CUDA context, or another VM's. No fault, no counter.

✔ Closed in w735: `reclaim_released_windows` names the regions whose mapping actually went, and
a view is released only then. ⚠ Leaking is the safe direction — it refuses later arms loudly —
and `parked=` in the census makes it visible.

### ⊘ AND AN INSTRUMENT GAP, since the commit gate is what would have caught the above

`cargo check --workspace --all-targets` **does not compile `barmirror.rs`**: it is behind
`kayfabe-qemu-raw/host-isolates`. ⇒ w734a's *"the next `FbPageBacking` arm is now a compile
error in **both** matches"* is true only with that feature on, and the default gate proves
nothing about it. `[measured w735]` adding the arm produced **zero** errors under the default
gate and **exactly the two** expected errors with `--features kayfabe-qemu-raw/host-isolates`.
⇒ **the commit gate must carry the feature**, or the tree's most load-bearing match arms are
unchecked.

---

## §39 — THE MAPPING IS LIVE, SO EVERY READ IS A RACE WITH THE GUEST

**STATUS: LIVE, 2026-09-18 (w760c/w760d). Owner ruling, verbatim:**

> *"the guest changing the value underneath you is a real problem. very simple solution to
> forbid outright in your cuda code: you may never bound check on the value of a C pointer
> pointing to live gpga. so always copy, then check, then use"*
> *"note that if the vendor checks are bypassed, a guest corrupting itself is a non issue for
> us as it doesn't happen on honest ones. only breakout/escalation has to be prevented"*
> *"the ptx must keep the live mapping, its just hardening you need to do in ptx, for guest
> writes underneath"*

§38 put the walker on the **live** RM object — the real GPGA, in place, not a copy. That is
deliberate and stays. The consequence is that **every load races the guest**, and the guest is
under no obligation to hold still. §39 is what that costs.

### (a) COPY, THEN CHECK, THEN USE — never check through a live pointer

A bounds test on `*p` where `p` points into GPGA proves nothing: the guest may write between
the test and the use, and the value the check approved is not the value that gets used. The
load is `volatile`, so this is not theoretical and the compiler will not save you.

⇒ **One dereference site.** `kf_win_load` bounds the offset, dereferences **once** into a
caller-owned local, and everything downstream reasons about the copy. `KF_GPGA_DEREF` appears
exactly twice in `kf_walk.cu` (its definition and that one use) and there is exactly one raw
`const volatile uint64_t *` cast. Both are gated in `make check-invariants`.

⇒ **And no offset may be fetched twice.** One textual deref site does **not** prevent a double
fetch: two `kf_load64` calls on the same offset are two real reads of a volatile location, and
they can differ. Gated by `check_s39.py` (§39(a)), which requires every live-GPGA load site to
name a distinct offset expression. Watched to fire before being wired in.

### (b) THE THREAT MODEL — self-corruption is not our problem; escalation is

A guest that points a leaf at its own wrong page has corrupted **itself**. We do not defend
against that: an honest guest never does it, and a malicious one is only hurting its own VM.

What must never happen is the guest obtaining **memory that is not its own**. That is the only
class worth spending refusals on, and it is what every bound in the walker is for.

⚠ The corollary that matters when reading ogkm: **a malicious guest runs none of the vendor's
checks.** Every condition RM validates before writing a table is a condition our walker must
assume is violated, because RM is not in the loop — the guest writes the bytes directly. ⇒
ogkm's own rejection conditions are a **guaranteed-off-the-happy-path fuzz corpus**, and are
being mined as one (owner, 2026-09-18).

### (c) A LEAF THAT LEAVES THE STORE IS THE ESCALATION — and it was unbounded

★★★ Found by the gate above, w760c. `KFWR_R_OOB` bounds **a table we would READ**. Nothing
bounded **a leaf we would EMIT**: both emit chokepoints checked a leaf's *alignment* and then
reported the guest's PTE payload as a GPGA, and `kf_validate_report` could not catch it either
— it was never given the window. A table read out of bounds reads memory that is not ours; a
**leaf** out of bounds becomes a **mapping**, and hands the guest memory that is not its own.
The host's `map()` bound in Rust was the only thing standing there — a defence-in-depth layer
serving as a single point of failure.

⇒ `KFWR_R_LEAF_OOB`, refused at both chokepoints, **before the coalesce branch** — a check
placed after it lets a refused leaf extend a legal run past the end of the store with every
refusal flag clear, which is a silent escalation rather than a loud refusal. Gated (§39(c)).

### (d) THE WINDOW AND THE SPAN ARE TWO NUMBERS — w760d, and w760c got it wrong

`gpga_len` is **how many bytes of the store are mapped for me to READ**. The containment bound
is **how large the guest's GPGA space IS**. In production they coincide, because the single
store is the whole of guest vidmem and all of it is mapped. **Nowhere else do they coincide**:
a captured corpus image holds the guest's *table pages* and no framebuffer, so its leaves point
legitimately far outside the bytes it contains.

Collapsing them refused **263 legitimate leaves on real GA106 tables**. They are now
`KfWin { base, len, span }`, and production passes the store length for both *while saying in
a comment that the coincidence is a property of this deployment, not of the walker*.

⇒ **The general rule this is an instance of:** when a number is correct for two different
reasons, give it two names. The one place they diverge is the place that finds the bug — and
here the divergence was a real hardware capture, which is why the suite caught it and no amount
of local reasoning would have.

### (e) REFUSAL PROPAGATION MUST DOMINATE EVERY RETURN — w760e

`hostile/leaf_past_end` refused its one leaf correctly and reported `run=0 ref=0 mask=0x0`. The
refusal happened and vanished: the propagation sat at the **bottom** of the parallel leaf
function, below two early returns, and a warp that emits no runs has `total == 0` and takes
`if (!total) { … return; }`. The report then says **this address space is empty** when what
happened is **every mapping in it was rejected**. Opposite facts, identical bytes.

⚠ Pre-existing and not specific to the new flag — `MISALIGNED_LEAF` was lost the same way, and a
warp that is entirely misaligned is not exotic. Note which inputs it was invisible to: only a
walk whose *every* leaf is refused takes the losing path, i.e. exactly the hostile case.

⇒ Gated (`check_s39.py` §39(e)), and the gate immediately found a second site nobody had looked
at. Returns that precede the accumulator's birth are exempt — a return before there is a refusal
to lose is not a defect.

### (f) THERE ARE THREE VALIDATORS, AND ONLY ONE IS ON THE PATH — w760l/w760m

A containment check was added to `kayfabe_cuda::walk::Report::validate`, whose own doc comment
calls it *"what production consults"*. It is not: `walkshadow.rs` parses into
`kayfabe_mmu::walkreport::Report` and calls **that** type's `validate`. Three implementations
exist — the `.cu`'s `kf_validate_report` (the CUDA suite's oracle), the `kayfabe-cuda` one
(`selftest` only), and the `kayfabe-mmu` one (production) — and a doc comment asserting which is
which was **wrong**.

⇒ The check now lives in **`StoreMapPort::apply_ops`**, the sole mutator, which already holds the
store's length. Two reasons, and the second is the general one:

1. It bounds the **whole batch before anything moves**. `map()` refused out-of-range slices one
   at a time, but that failure arrives mid-apply — some unmapped, some mapped, the caller left
   holding a half-applied diff. A report naming memory outside the store is not one to partially
   honour.
2. **A check that must be handed its bound from elsewhere is a check someone can forget to hand.**
   Put the check where the bound already lives.

⊘ `walkreport::Report::validate_within` + `ParseError::RunOutsideGpga` are kept and are
**BUILT AND NOT YET WIRED** — named as such here rather than left looking wired, which is this
tree's own most-repeated failure shape.

### (g) EVERY RPC IS ANSWERED — EVENTUALLY, AND NEVER ON THE vCPU

**Owner, 2026-09-18:** *"how can we leave an RPC unanswered? all RPCs must be answered, even if
unknown with an error."* … *"I mean eventually answered, not on vcpu thread"*.

★ Already the implemented invariant, and worth recording as one. `boot.rs` — *"THE DEFAULT IS A
NAMED REFUSAL"* (task #127): a policy with no answer gets one posted for it, carrying a non-zero
envelope `rpc_result` with a **zeroed body** — never the request reflected, never a fabricated
`NV_OK`. Unknown functions fall through `other => RpcFunction::Other(other)` to
`Disposition::Reply` and get that same named refusal.

⊘ **Three measured exceptions, where answering is itself the bug:** `GSP_SET_SYSTEM_INFO` (72),
`SET_REGISTRY` (73), `ECC_NOTIFIER_WRITE_ACK` (202) are all `_issueRpcAsync` and echoing one
surfaces in the driver as an unexpected event and **desyncs the seqNum**. That set is *derived
from `rpc.c`* by `rpc_async_set_oracle.rs`, never hand-listed — a hand-list cannot detect a
shared misreading, and deriving it is how the third member was found.

★ And the owner's *"eventually"* is a first-class split, not an implementation detail:
`CommandPolicy::respond` says **what** the answer is; `CommandPolicy::may_deliver_yet` says
**when** it may be delivered — the latter existing so a reply can be held until the page-table
refresh reaches the host without blocking a vCPU (§ the three blocking invariants).

---

## §40 — TWO LIFETIMES: THE RESERVATION IS THE VM'S, THE DEVICE STATE IS THE DRIVER'S

**STATUS: LIVE, 2026-09-18 (w760). Owner ruling.**

> *"we load scratchpad process and the raw client when the driver loads, its also our persistent
> thing immediately. if it unloads, all GPU processes are killed incl the scratchpad, so we
> ourself also hold no references. if the host then also holds nothing, the host GPU driver can
> unload (unless the host holds work). … one thing that I want to avoid is to loose our GPU
> memory reservation when the guest is running but GPU driver unloaded it temporarily."*

The owner names the tension in the same breath as the design, and it resolves into **two
lifetimes that must not be collapsed into one**:

### Tier A — the VM's lifetime. **Must survive a guest driver unload.**

The scratchpad isolate and the single store reservation. `[measured w760]` this tier already
exists and is already VM-scoped (`scratchpad.rs:509`, *"The VM-lifetime scratchpad isolate. One
per `(vm, gpu)`, spawned at device realize"*).

⊘ **Why it must not be driver-scoped:** a guest that `rmmod`s its driver is still running, and
its vidmem allocation is still its own. Releasing the reservation there hands the guest's memory
back to the host allocator, where another tenant — or our own host CUDA context — can take it.
The guest then reloads and cannot get its own framebuffer back. **A temporarily driverless guest
is not a departed guest.**

### Tier B — the guest driver's lifetime. **Must reset on unload.**

GSP boot phase (and therefore WPR2), channels, VAS, mappings, BAR views. `[measured w760]` this
tier **does not exist**: there is no "guest driver unloaded" notion anywhere in the device layer,
which is precisely why the device-open wall is permanent (see §39 and
`the_device_open_wall_is_wpr2_not_ceutils`). The FSM *can* cycle — `Cold | Halted` both accept a
restart — but nothing drives it back.

### ⇒ What follows, in priority order

1. **Fix the root cause.** `kbusInitBar2` fails because `memmgrGetDeviceSuballocator` returns the
   memory manager's heap and that heap is **NULL** (`mem_desc.c:116-160`: the only
   `NV_ERR_INVALID_STATE` on that path is `NV_ASSERT_OR_RETURN(pHeap != NULL, …)`). A boot that
   does not fail needs no recovery, and this is the only item that makes the wall *not happen*.
2. **Reset tier B on guest driver unload**, clean or unclean. The clean path already exists
   (Booter Unload → `BootStep::Teardown` → `enter_halted()`); the unclean one — the guest process
   `kill -9`'d, or an init that failed before `NV_INIT_FLAG_GPU_STATE_LOAD` was set — has no path
   at all, and that is the one that actually fires.
3. **Model FLR.** `[measured w760]` kayfabe has no function-reset handling, so the guest's own
   documented recovery (*"the GPU is likely in a bad state and may need to be reset"*) is
   unavailable. A real device clears WPR2 on reset; ours must, or a wedged guest can only be
   fixed by destroying the VM.

⚠ **The ordering of tier-A teardown matters for the owner's last clause.** *"If the host then
also holds nothing, the host GPU driver can unload"* is only true if tier A is released at **VM
shutdown** — so the rule is: tier B resets on driver unload, tier A releases on VM exit, and
neither event may trigger the other's teardown.

---

## §41 — WHAT A vCPU MMIO WRITE MAY DO, STATED AS AN ALLOWLIST

**STATUS: LIVE, 2026-09-18 (w760). Owner ruling.**

> *"adopt_pending_channel_rings is not something to run on the vcpu thread either, it directly
> violates a constraint that keeps vcpu simple: mmio write is only updating queue and wake,
> nothing else. for doorbell it uses doorbell table, passthrough is one dword write, the other
> is putting the token in a queue. Same is for remaining registers, a synchronous write to a
> read register is fine to prevent a race."*

★ This **supersedes nothing and sharpens everything**. `no_blocking_work_in_any_mmio_trap` and
the three blocking invariants are PROHIBITIONS, and a prohibition needs a judgement call at
every new site: *"is this blocking?"* is answerable only after you know what the callee does,
which is exactly how `adopt_pending_channel_rings` — three page-table settlement passes —
lived on a vCPU for four rungs behind an innocent-looking register write.

### The allowlist. A vCPU MMIO write may do these and nothing else:

1. **Update a queue** — append a token/descriptor to a lock-free or briefly-locked structure.
2. **Wake** — signal the worker that owns the work.
3. **Doorbell specifically**: consult the doorbell table; a **passthrough** doorbell is ONE
   DWORD WRITE to the real register; an **emulated** doorbell puts the token in a queue.
4. **A synchronous write to a read register** — permitted, and permitted *because* it prevents
   a race: the value must be visible to the next read of that register.

⊘ **Everything else is a defect, including work that is fast today.** The rule is not "keep it
under N microseconds": a cheap callee acquires expensive callees over time, and the cost is
discovered as a latency spike attributed to whichever register happened to notice a latch.

### Why the positive form is enforceable and the negative one is not

`[measured w752→w754]` `worst_trap=24999us at=bar0+0x110c00` named `NV_PGSP_QUEUE_HEAD` for
four rungs. The register was a **bystander** — simply the one the guest writes most during
driver init, so it noticed the latch first. Under a prohibition, every brief written from that
number looked compliant because the *register's own* servicing had been deferred since w432.
Under this allowlist the question is not *"is this slow?"* but *"is this one of the four?"*,
and `adopt_pending_channel_rings` fails it on sight, at review time, with no boot.

⇒ **A new call reachable from `Regs::write` must be justified against the four items above by
name.** `worst_trap` NAMES THE SITE, NEVER THE CAUSE — so a profile that attributes cost to the
trapping register will reproduce the same four-rung error; attribution must be to the callee.

### Status

✔ `adopt_pending_channel_rings` moved off the vCPU at w754; the doorbell worker calls
`adopt_pending_channel_rings(false)` and `tests/ring_adopt_is_off_the_vcpu.rs` pins it.
⚠ The `Regs::write` entry point survives behind `inline_because_no_worker`, for a build with no
worker at all — that arm still violates §41 and is a degraded configuration, not a shipping
one. ⊘ `ring_adopt_census()`'s `on=` is the number that says which arm actually ran; it prints
at teardown, so read it there rather than inferring it from the gate.

## §42 — A DEFAULT IS THE DESIGN, AND SO ARE ITS PRECONDITIONS

> **Owner, 2026-09-18:** *"make the single store the default or forced"* · *"make route k the
> default or remove env vars. remove env options which now crash by construction."*

**`[measured w763]`** `KAYFABE_FB_STORE=device` and `KAYFABE_VAS_OWNER=k` were already the
defaults. Every knob they **depend on** still defaulted off — `SCRATCHPAD`,
`SCRATCHPAD_CUDA`, `DEVICE_VIEW`, `ISOLATES` — so a boot that named **nothing** died at
`enforce_device_store`:

    KAYFABE_FB_STORE=device and there is NO DEVICE-VIEW PORT

⇒ **The shipped default was a configuration that cannot boot.** A top-level flip whose
preconditions are not flipped with it is decoration.

### (a) Flip the whole dependency chain in one commit, or do not flip

A default names an architecture, and an architecture has parts. If `A`'s default requires
`B`, `C` and `D`, then `B`, `C` and `D` are not separate decisions — they are the same one,
spelled four times. ⊘ Leaving them off does not make the configuration conservative; it makes
it **unreachable**, which is strictly worse than the old arm because it fails at realize
instead of running something honest.

### (b) One statement of a default. An auditor asks the parser, never the environment

⊘⊘⊘ The `CONSTRAINT-VERDICT` block added at w760 to catch *"this boot is not measuring the
deliverable"* did its own `std::env::var` reads with the pre-flip defaults written into the
comparison. One commit after route K became the default it printed
`⊘ §26/§32 KAYFABE_VAS_OWNER != k` **on a boot that was running route K** — the variable was
merely absent, and absence had changed meaning.

⇒ A verdict block that restates a default is a **second statement** of it, and it fails in the
most misleading direction available: it accuses the correct configuration. Every auditor calls
`selected_vas_owner()`, `selected_fb_store()`, `selected_isolate_plane()` — the same parsers
the device calls. ⚠ `env::var(X) == "literal"` in an audit is the defect, not the check.

### (c) A census names symptoms; act on the FIRST LINK, not the count

That boot printed four violations — `DEVICE_VIEW=DISARMED`, `CUDA_WALK=DISARMED`,
`VAS_OWNER != k`, `ISOLATES != real`. They are **one** cause: no plane ⇒ no worker ⇒ no
reservation ⇒ no port ⇒ refuse. Reading four symptoms as four problems is how this survived.

### (d) An opt-in may MOVE; it may not silently disappear

`isolate_plane_from(None)` was `Stillborn`, and its stated reason was real: *"a default that
spawned anything would put a host process behind every guest without a single line of
configuration."* ⊘ That objection is **answered, not overridden**: the `host-isolates` cargo
feature stays **off by default**, so an archive that did not opt in cannot name
`HostIsolateFactory` at all and `real` there refuses at realize, by name. The property is held
**by linkage**, not by an `if`. ⇒ When you move a default, say where the old guarantee now
lives; if you cannot point at it, you removed it.

### (e) A test that reddens on a default flip is reporting a HIDDEN ARGUMENT

`[measured w763]` the flip reddened two whole suites and six more files — **not one of them
about what guest video memory is**. They ask about BAR ownership, refusal wording, a counter's
rate. They reached the benign arm only because it was the ambient default: an **undeclared
dependency on a process global**, revealed.

⊘ Both reflexes are wrong. Setting the variable inside the tests puts a process-global write
in a multi-threaded test binary — the shape that already cost this campaign a flake. A
`cfg(test)` default means the tested configuration is never the shipped one.

⇒ Move the ambient read **out of the composition root's contract**:
`Regs::create_probed_on(id, probe, Some(FbStoreArm::Arena))`. Production still calls
`Regs::create`, which still reads the environment, and there is still exactly one statement of
the default. ★ The precondition now lives in the test, which is where a precondition belongs.
