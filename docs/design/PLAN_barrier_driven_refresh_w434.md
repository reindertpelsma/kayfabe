# PLAN — barrier-driven refresh, and the deletion it enables

**STATUS: LIVE, 2026-09-11. THIS IS THE ACTIVE PLAN.** Owner-approved. Read this first after a
compaction; it supersedes the "GPGA is the fix" reading in `RESUME_HERE_w417.md` §w426.

## The ruling this is built on (owner, 2026-09-11, verbatim)

> *"va publish + the pt\*/pd\* + promote/depromote can only happen at synchronization points
> like tlb invalidate, rm call, uvm kernel channel, and they give very precise whats refreshed
> so you only look there, as thats only safe with locks in a honest guest, still remember that
> a malicious guest can write data underneath so for them protect a breakout/illegal action
> not a self corruption of the guest."*
>
> *"I think in real hardware all these 3 entrypoints eventually end up in a hardware tlb
> invalidate anyways, since I think gsp can call tlb invalidate as well."*

### What that fixes, in one sentence

Today we **sweep** to discover rows and race the engine. Tomorrow we **refresh exactly what a
barrier names, when it names it**, and there is nothing to race.

### The three rules it imposes

1. **Page-table memory is read ONLY inside a barrier's handler.** Reading `PD*`/`PT*` outside
   one is forbidden — that is the only window where an honest guest is holding its own locks.
2. **A barrier names its scope. Refresh exactly that.** No whole-VAS sweep, no sampling, no
   convergence, no cap.
3. **Threat model — and it is narrower than it looks.** A malicious guest CAN write page-table
   memory underneath us. We defend against **breakout and illegal action**, never against the
   guest **corrupting itself**. ⇒ Validate that what we read cannot make us touch memory we do
   not own or act for another proc; do NOT try to make a lying guest's own mappings coherent.

## ⊘⊘⊘ CORRECTED 2026-09-11 — **DO NOT UNIFY. THERE ARE THREE ENTRY POINTS AND WE OWN ALL THREE.**

> **Owner, correcting both me and their own earlier speculation:** *"no the driver doesn't
> unif[y] them in one call, thats why we have 3 entrypoints. dont take that assumption. this is
> what I thought. my suspicion is that a real bare metal gsp does that in microcode, but since
> we impersonate that we must implement all 3 entrypoints"*

I verified that RM's `vaspaceInvalidateTlb` lowers to `NV_PFB_PRI_MMU_INVALIDATE` and concluded
*"one barrier, two transports"*. **That conclusion is wrong, and the error is specific:** what
I traced is the path the guest's **kernel GMMU** owns. On a GSP part the mapping work itself
happens **inside the GSP** — which is us — and a real GSP does its invalidate in **microcode**.
Nothing crosses the guest boundary for it, so no barrier ever arrives.

⇒ **For the RM path there is nobody to hear from. We serviced the map, so we are the one who
knows it happened.** That is exactly why the count is three and not one, and why the table
below is a table of OUR obligations rather than of the guest's transports.

| # | entry point | who tells us | our obligation |
|---|---|---|---|
| 1 | **TLB invalidate** — `NV_PFB_PRI_MMU_INVALIDATE` | the guest, by writing a BAR0 register | handled; needs SCOPING to what the register names |
| 2 | **RM call** — a map/unmap RPC we service | **nobody — we are the GSP** | refresh inside the handler, before the reply |
| 3 | **UVM kernel channel** — `MEM_OP MMU_TLB_INVALIDATE` | the guest, as a pushbuffer method | consume `out.invalidates`; today it is DROPPED |

⚠ **The trap in the wrong reading:** it would have had us *waiting for a barrier* on path 2.
None is coming. A boot would look correct on the client (whose P1 is `rm-invalidate` via path 1)
and silently never refresh for anything the GSP mapped on the guest's behalf.

## ⊘ Superseded: the unification hypothesis

On real hardware the GPU re-reads page tables only after a **TLB invalidate**. So a correct
guest MUST emit one after any change it wants the GPU to see — which makes the invalidate the
**universal barrier**, and the "three entry points" merely three transports that carry it:

| transport | where it arrives | what it names |
|---|---|---|
| `MEM_OP_A`/`_D` `MMU_TLB_INVALIDATE` | a channel pushbuffer (UVM kernel channel included) | `pdb`, `PDB_ALL` vs targeted, `membar` |
| the BAR0 invalidate trigger | an MMIO write we already trap | the trigger's own fields |
| the GSP RPC `INVALIDATE_TLB` (fn 200) | the command queue | its params |

⚠ **Verify before building on it.** If some map path changes tables and emits no invalidate,
this collapses to "discover late" again for that path and the sweep must survive as a
backstop for it — scoped to that path, not as the primary mechanism.

## The work, in order

1. **[VERIFY]** Do all three transports converge on a TLB invalidate? Census each on a real
   boot: `MEMOP-CENSUS seen=`, the MMIO trigger count, RPC fn 200 count. ⊘ A transport that
   measures ZERO on a workload that clearly changed mappings is the counter-example.
2. **[CONSUME]** Make `out.invalidates` drive a refresh. It is decoded at
   `kayfabe-fwd/src/lib.rs:8167` and read by nothing. This is the single highest-value edit in
   the tree.
3. **[SCOPE]** Refresh only what the barrier names — `pdb`, targeted VA when present.
4. **[ORDER]** Refresh BEFORE the forward. We control when the host sees the work
   (`forward_ce` is two lines after the parse), so ordering our own forward after our own
   refresh needs no guarantee from the guest. This is the C's `nvkvm_gpu_emul.c:582`
   invariant: *"a mapping is always backed before the engine that uses it runs."*
5. **[DELETE]** Once 2–4 hold: the pin-rate sampling path and its 634-line function, most of
   `VasPublishArm`, the watermark/rescan convergence machinery, `VAS_PINRATE_ROWS`,
   `VAS_DRAIN_ROW_CAP`, and the arms that select between them.
6. **[DELETE]** `dbtable.rs` + `promotion.rs` — 648 lines, zero production referrers. Wire or
   delete; they read as capability and are not.
7. **[COLLAPSE]** The 46 environment arms. Each was an experiment; winners become the
   behaviour, losers go.
8. **[GATE]** Every mechanism prints a did-I-fire counter into ONE census line, and a gate
   asserts the expected set is non-zero on a real boot. This is what turns *"green CI over dead
   code"* — the actual failure mode, seven instances this session — into one grep.

## Falsifiers (do not declare any step done without these)

- The mean client at **threads:8 rounds:8**, P1/P2/P3 VERIFIED, **THREADS 8 of 8**.
- `inline_exceptions` falling from **166** toward 0; `worst_trap` from **1.79 s**.
- `MEMOP-CENSUS seen=` non-zero on a boot where P2 passes.
- The LLM graded on **text matching the same-boot CPU oracle**, never a token count.
- ⚠ n=1 is not a grade.

## Already landed toward this (this session)

- `w431` BAR1/BAR2 default to untrapped.
- `w432` GSP command queue serviced on the worker; the 1.79 s trap wired off the vCPU;
  `LANE-CENSUS` prints four counters that were previously incremented and never read.
- `w433` the written-down case for the deletion, and the withdrawal of my own GPGA proposal
  for this race.
