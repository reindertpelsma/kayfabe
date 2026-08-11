# Reachability-on-transition — the address plane's populate discipline

> ### STATUS — 2026-08-11 (w258 doc-hygiene sweep) / **LIVE — discipline stands; §7 RESIDUE ITEM 7 is STALE**
>
> ⊘ **STALE, one item only.** §7's residue item 7 says: *"None of this ran against a guest. Every
> `[test]` above is a mock-level test in this workspace."* **That has since run on real hardware,
> twice:**
> - `d864d86` (2026-08-11, w246) — *"ROUTE B FIRES … §16.97's 'unreachable' was true of a
>   CONFIGURATION, not the code"*. The A/C pair isolates `KAYFABE_PT_WITNESS_EXEC`, the flag that
>   binds the ring's VA: **`RING-VA-UNBOUND` 8 → 0**.
> - `0fff2ce` (2026-08-11, w247) — *"ALL THREE preconditions armed at once — the address plane is
>   COMPLETE"*, across four boot corners, recording that the witness populates the CE channels'
>   VAS (`pdb=0x201000`).
>
> ⇒ The populate discipline this doc specifies is no longer mock-only; it is **measured on a real
> GA106**. ★ Note *how* item 7 went stale: it was a correct statement about **coverage**, and
> coverage is exactly the kind of claim a later boot silently falsifies without touching the doc.
> Everything else here is unrefuted.

**What this is.** `resume_from_fault.md` §7 **step 4**, built. §6 of that note put the owner's
*trap-on-transition* instinct to seven ways a mapping becomes reachable or unreachable without
crossing the edge as stated, and §6.1 returned the verdict this document implements:

> **Adopt it — as *reachability*-on-transition, with holes 1–7 closed explicitly — and do not
> believe it is complete.**

**Epistemic classes** (per `claim_ledger.md`): `[src@580]` = read out of `ogkm-580.159.04`,
nothing ran. `[meas]` = a scan of a committed capture under
`../nvidia-gpu-passthrough/traces/mode2_c_reference/`, which is a recording of a real stock guest
driver, not a live run. `[C:]` = read out of the C artifact. `[test]` = a test in this workspace
that fails if the statement stops holding. ⊘ **No hardware run was available for this work.**

---

## 0. The model in six lines

1. **The unit of truth is a PAGE-TABLE PAGE'S CONTENT, not a store and not an edge.** The shadow
   holds, per address space, the decoded slot map of every page-table page we have read.
2. **Transitions are a DIFF, not a pattern match.** Each pass recomputes two things from that
   content — the set of pages *reachable* from the PDB root, and the set of leaves that should be
   bound — and emits the difference against what the table currently says. Nothing watches for
   "the store that flips the valid bit".
3. **A leaf binds iff it is REACHABLE and WITNESSED.** Reachable = a chain of page-directory
   entries from the root to its page, out of content we hold. Witnessed = the guest was seen to
   write that page.
4. **Reachable-but-unwitnessed stays a MISS**, which is a fault, which is `resume_from_fault.md`
   §7 step 5. That is the price §6.1 named and it is paid here, not argued away.
5. **Unreachability is retirement.** A page that *was* reachable and is not any more has its
   leaves unbound, its shadow entry deleted, and its level metadata dropped — so its next
   contents are not misparsed as page-table entries.
6. **It is not complete, and §7 says which parts are not.**

★ **Where the model departs from the brief, and why it is worth saying.** The owner's design and
§6 both reason in terms of *edges* — "trap the store that marks the entry valid", "trap the store
that marks it invalid". This one reasons in terms of *content*, and derives the edges. Two of the
seven holes (4, 7) exist **only** because an edge-watching design cannot see a change that does
not pass through the state it watches for; a content diff has no such blind spot, because it never
asks what a store did — only what the page now says. The cost is that the shadow is O(the guest's
page tables) rather than O(1), which is why §5 bounds it.

---

## 1. Why the edge is the wrong object (hole 1), in the driver's own words

UVM publishes **children first, fenced, then parents bottom-up**:

```c
    // write entries bottom up, so that they are valid once they're inserted
    // into the tree
```
`ogkm-580: kernel-open/nvidia-uvm/uvm_mmu.c:771-782` `[src@580]`

So a leaf carries `VALID=1` while its parent page-directory entry is still invalid. Its
invalid→valid edge is **not** the moment the mapping becomes reachable; the parent's publication
is, and one such 8-byte store can make up to 512 leaves reachable at once.

RM's walker takes the opposite order — child allocated and cleared to invalid, *then* the parent
published, *then* the leaves written (`ogkm-580: src/nvidia/src/libraries/mmu/mmu_walk.c:1179-1189`,
`:1230-1241`, `:1365-1406`; leaves at `mmu_walk_map.c:163-169`) `[src@580]`. **The two orders are
opposite and both are legal**, which is the argument against watching either one: a rule keyed on
"the leaf goes valid" is wrong for UVM, and a rule keyed on "the parent goes valid" is wrong for
RM. A content diff is right for both because it is not keyed on an order at all.

`gmmu_publication_discipline.md` §2.3 states the single invariant both satisfy — *a page-table
entry that is reachable from the root is never uninitialised* — and that invariant is what makes
the reachability closure meaningful rather than a walk through garbage.

`[test]` `tests/tests/reachability.rs::a_leaf_written_valid_before_its_parent_binds_only_when_the_link_is_published`

---

## 2. The two gates

### 2.1 REACHABLE

Closure from the root over page-directory edges held in the shadow. The root is a **declared**
fact — a PDB *is* its own root page — and is reachable by definition, whether or not anything has
been read out of it.

★ **Edges out of an unwitnessed page count toward reachability, and that is deliberate.** The
tempting stricter rule — *only witnessed pages contribute edges* — buys nothing and costs
correctness: the guest routinely links a page through a directory this port never saw written (the
`SET_PAGE_DIRECTORY` root, a directory published by a transport we do not decode), and refusing
those edges would make the tree unknowable rather than safe. The safety comes from the **bind**
gate, and the argument is closed:

> A page-directory entry read out of allocator residue can only ever make some *other* page
> reachable. That page is itself unwitnessed, so nothing in it binds. Residue therefore cannot
> produce a binding — it can only produce a reachable page with nothing in it.

That is hole 2's hazard (`mmuWalkReserveEntries(..., bInvalidate = NV_FALSE)` leaves a level
reachable with uninitialised backing store — `ogkm-580:
src/nvidia/src/libraries/mmu/mmu_walk_reserve.c:57-63`, `:85`) `[src@580]`, closed by the bind
gate alone. `[test]` `tests/tests/reachability.rs::residue_can_make_a_page_reachable_but_never_binds_a_leaf_out_of_it`

### 2.2 WITNESSED

A page is witnessed once the guest has been seen to write it — in this port, once it has entered
its `Vas`'s dirty set from the copy-engine page-table-write latch (`kayfabe_fwd::latch_pt_writes`).
The record is **cumulative and survives the drain**: `plan_pt_decode` consumes the dirty set (a
page written again must be dirty *again*), so the witness has to be taken at the drain or a page
witnessed in one pass and linked in the next would bind nothing.

★ This is §6.1's rule, honoured as written: *walk to enumerate candidates, but bind only entries
we also witnessed being written.* The granularity is the **page**, not the entry, and that is
stated rather than implied — a page whose first half we saw written and whose second half we did
not is treated as witnessed throughout. Closing that gap needs a byte-range witness the current
latch does not carry (`kayfabe_fwd::PtWrite` names a page and a byte count, not an offset), and it
is recorded in §7 as residue rather than described as closed.

---

## 3. The seven holes, one by one

| # | hole | verdict |
|---|---|---|
| 1 | validity ≠ reachability | **CLOSED** — §1, §2.1 |
| 2 | enumerating means walking, which §6.2 forbids | **CLOSED at page granularity** — §2.2; entry granularity is §7 residue |
| 3 | teardown crosses no leaf edge | **CLOSED** — §3.3 |
| 4 | valid→valid remap; protection-only change | **remap CLOSED; protection change DETECTED, not modelled** — §3.4 |
| 5 | PDB rebind with zero entry writes | **CLOSED for the VASpace rebind; the instance-block write is unobserved** — §3.5 |
| 6 | three states, not two (sparse) | **CLOSED as a distinct state; the guest-visible no-fault semantics is §7 residue** — §3.6 |
| 7 | level granularity is not uniform | **CLOSED** — §3.7 |

### 3.3 Teardown (hole 3) — the PDE clear, and the retirement it must cause

`_mmuWalkPdeRelease` clears the parent entry first and frees the sub-level backing store second,
**with no TLB invalidate between the two** — `ogkm-580:
src/nvidia/src/libraries/mmu/mmu_walk.c:1509-1552`; the invalidate happens later at the caller,
`ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:1803-1811`. `[src@580]` Hundreds of leaves
are unmapped by one directory store and are **never written invalid** — the memory is simply
recycled.

Under a content diff this is not a special case: the edge is gone from the parent's slot map, the
subtree leaves the reachability closure, and its leaves leave the desired set. Two things must
happen **beyond** unbinding, and they are the part a leaf-watching design cannot express at all:

1. **The pages are retired from the shadow.** A page that was reachable and is not any more is
   deleted outright.
2. **Their level metadata is dropped** (`Vas::pt_meta`). This is the half hole 3 names explicitly:
   *"or we misparse the recycled page's next contents as PTE writes."* Once the level is
   forgotten, a later write to that recycled page is **deferred** by
   `kayfabe_fwd::plan_pt_decode` — level unknown — instead of being decoded as a page table.
   `[test]` `tests/tests/reachability.rs::the_pass_drops_the_level_of_a_retired_page_so_its_next_write_is_deferred`

★ **Retirement is keyed on `was reachable and is not`, never on `is not reachable`.** The
distinction is load-bearing: a page-table page filled *before* anything points at it — §12.1(i)'s
orphan leaf, the guest's own build order, and the exact shape `#13` was about — is unreachable and
must be **kept**, because the link is coming. A page that has *fallen out* of the tree is a
different fact. Conflating the two deletes the orphan and rebuilds `#13`.
`[test]` `tests/tests/reachability.rs::a_pde_clear_retires_the_whole_subtree_and_an_orphan_is_not_retired_with_it`

### 3.4 valid→valid (hole 4) — the remap fires; the protection change is named

RM drives `update_type = PTE_DOWNGRADE` for a re-map — `ogkm-580:
src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/virt_mem_allocator_gm107.c:2602-2606` `[src@580]`
— and neither of the owner's two proposed edges fires on it, because the entry never passes
through invalid.

**Under a content diff it fires**, because the diff compares what the slot *says* rather than what
a store *did*. That is not a claim about hardware and it does not need one: it is a property of
the comparison.
`[test]` `tests/tests/reachability.rs::a_remap_that_never_passes_through_invalid_still_fires`
⚠ What remains open is not the mechanism but the **observation**: no downgrade
appears anywhere in the C oracle's captures — all 786 invalidates across `cap1_coldboot_hermetic`,
`cap3_matmul_forwarding` and `cap2b_stalequeue_nofn47` are upgrades `[meas]`
(`resume_from_fault.md` §4.2, scan of 2026-08-01) — so the transport by which a downgrade would
reach this port has **not been observed**, and this is unobserved rather than impossible.

**The protection-only change is a different answer and gets a different one.** A slot that changes
only its read-only bit is a mapping change that grants or withdraws access without moving an
address. The shadow **detects it** and reports it by name; the address table **cannot represent
it**, because `kayfabe_mmu::Binding` carries no rights. So the honest state is: *seen, named,
counted, not modelled* — a `ReachOutcome::protection_changes` entry, never a silent `unchanged`.
`[test]` `tests/tests/reachability.rs::a_protection_only_change_is_reported_and_never_silently_unchanged`

⊘ It is worth being exact about what this is and is not. It is a **fidelity** gap, not an
isolation one: the rights in question are the guest's own declarations about its own address
space, so failing to tighten them cannot reach another tenant. Modelling it means adding rights to
`Binding` and to the publication verb, which is forwarding-plane work.

### 3.5 PDB rebind (hole 5)

The C snoops `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` for exactly this
(`C: src/qemu/nvkvm_gpu_emul.c:2736-2790`) `[C:]`. In this port the control already arrives as
`RmEvent::SetPageDir` and *"last declaration wins"* is already implemented
(`kayfabe_core::rmgraph`), and the structural consequence is inherited rather than built: a `Vas`
is keyed by `(GpuId, Pdb)`, so a rebind produces a **different key** — the old `Vas` is staged for
release (`stage_dropped_vases`, fill-before-drop) and a fresh one is minted. Nothing carries over.

What step 4 adds is that the shadow is **part of that object** and therefore cannot outlive the
rebind, plus a standing audit that says so out loud:
`ReachShadow::audit_root(pdb)` → `ReachFault::RootMismatch`, run at every commit. The idiom is
`AddressTable::audit_identity`'s and so is the argument for it: `Vas::new` proves the law about
every shadow *it* constructs; the audit proves it about the shadow the commit is *holding*. Those
differ the moment anything reaches a `Vas` another way — a future bulk-restore, a merge, a
deserialize — and a law with only a constructor check is one refactor away from being a law about
nothing. `[test]` `tests/tests/reachability.rs::a_shadow_whose_root_is_not_the_vas_s_pdb_is_a_loud_refusal`

⊘ **What is NOT closed.** §6's hole 5 also names *"swapping the instance block's page-directory
pointer"* — a write into `RAMIN+0x200`, not a control. This port does not observe that write at
all, and no test here pretends otherwise. It is §7 residue.

### 3.6 Sparse (hole 6)

Sparse is a distinct fill state with its own templates — `MMU_WALK_FILL_SPARSE`, `ogkm-580:
src/nvidia/src/kernel/gpu/mmu/gmmu_walk.c:904-935` `[src@580]` — and
`gmmu_publication_discipline.md` §7 rule 4 states the walker's obligation: *treat sparse as "no
binding, but do not fault the guest"*; conflating it with valid and conflating it with invalid are
**different** bugs.

`kayfabe_arch::PteDecode` therefore gains a `Sparse` variant, and the shadow keeps a sparse slot
as a slot rather than as an absence. The three transitions §6 enumerates then fall out of the
desired-set diff without a rule of their own:

| transition | what the diff does | why |
|---|---|---|
| valid → sparse | **unbind** | the leaf leaves the desired set |
| sparse → valid | **bind** | it enters it |
| invalid → sparse | **nothing** | neither state contributes a leaf |

★ Conflation is caught in *both* directions and by construction: fold sparse into `Leaf` and the
first row binds a mapping the guest declared as backing-free; fold it into `Invalid` and the
sparse declaration disappears, taking `ReachShadow::sparse_at` with it.
`[test]` `tests/tests/reachability.rs::sparse_is_a_third_state_and_the_three_transitions_differ`

⊘ **The half that is not modelled**: what the *guest* should observe on a sparse VA. On real
silicon a sparse mapping drops writes and returns zeros rather than faulting; here a sparse VA is
a table miss like any other, so anything downstream that turns a miss into a fault will fault it.
`ReachShadow::sparse_at(va)` exists so that consumer can ask — it has no consumer yet, and saying
so is worth more than shipping the query and implying one.

### 3.7 Level granularity (hole 7)

Two facts, one chip, and this is `#13` — *"the 512M-leaf gap that silently dropped page-table
writes for weeks"*:

- **the deepest directory's slot is a 16-byte DUAL entry naming two sub-tables**, and
- **on this port's target generation the second directory level is itself a 512 MiB leaf** —
  `ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/ampere/kern_gmmu_fmt_ga10x.c:46-53` `[src@580]`.

Entry width was already per-level (`GmmuFmt::entry_size`) and the child's level already came from
the format rather than from `level + 1` (`PteDecode::Pde::child_level`). Step 4 closes the two
that were left:

1. **A dual entry names two sub-tables, so a decode returns two edges.** `PteDecode::Pde` carries
   `edge` and `also: Option<PdeEdge>`. A decoder that returns one and drops the other is `#13`'s
   exact shape at a different level, and the previous shape could not express the second edge at
   all. `[test]` `tests/tests/reachability.rs::a_dual_directory_slot_names_two_sub_tables_and_both_are_followed`
2. **A directory slot that becomes a leaf unlinks a whole subtree with no clear.** This is holes 3
   and 7 composed, it belongs to no single one of them, and it is exactly the change an
   edge-watching design cannot see: no entry went invalid, and yet everything under that slot
   stopped being reachable. The content diff handles it as one step — the edge is gone, the
   subtree retires, and the leaf enters the desired set (subject to
   `kayfabe_mmu::walker::leaf_disposition`).
   `[test]` `tests/tests/reachability.rs::a_directory_slot_that_becomes_a_leaf_retires_the_subtree_it_used_to_name`

---

## 4. Where it sits, and what runs when

The pass is unchanged in shape — plan under the owner's lock, execute with no lock (it blocks),
commit under the lock again (`kayfabe_fwd::ptdecode`, R1/R5). Step 4 adds one call at each end.

| phase | lock | step 4's addition |
|---|---|---|
| `plan_pt_decode` | owner's, rank 1 | **witness** every drained page — tasks *and* deferrals |
| `run_pt_decode` | none | `SubtreeDecode` now carries the per-page decodes, not only the flattened leaves |
| `commit_pt_decode` | owner's, rank 1 | audit the root; **observe** each decoded page; **settle** once; apply |

★ **`settle` runs once per pass, not once per page.** A closure and a desired-set recompute are
O(the shadow), so doing them per page would make a pass quadratic in a guest-influenced quantity.
Once per pass is also the semantically right point: the guest's own publication protocol is a
sequence of stores followed by a commit signal, and a shadow that emitted a transition halfway
through a pass would be answering with a tree the guest had not finished writing.

**What the shadow is NOT.** It holds no page *content* — decoded slots only — and it is not a
cache the resolve path may consult. `AddressTable::resolve` is unchanged and still MISS = FAULT;
the shadow's entire output is a set of `bind`/`unbind` calls into that table.

---

## 5. Bounds (boundary-1)

Everything in the shadow is guest-influenced, so everything in it is bounded and every refusal is
loud rather than absorbed:

| quantity | bound | refusal |
|---|---|---|
| pages in one shadow | `MAX_SHADOW_PAGES` | `ReachFault::TooManyPages`, and the page is not admitted |
| slots across one shadow | `MAX_SHADOW_SLOTS` | `ReachFault::TooManySlots`, and the page is not admitted |
| page-table pages whose level is remembered | `kayfabe_fwd::MAX_PT_META` | pre-existing; `meta_refused` |
| entries examined in one pass | `kayfabe_fwd::PT_DECODE_BUDGET` | pre-existing; `WalkFault::BudgetExhausted` |

★ **A refused page is refused whole**, not truncated. Admitting half a page's slots would make the
desired set a statement about a page-table page that never existed, and the diff would then unbind
the half it did not admit — a wrong unmap manufactured by our own bookkeeping.

⚠ **An orphan is kept forever, by design, and that is what the page bound is for.** §3.3 keeps
never-reachable pages so the link can arrive later; a guest that fabricates page-table writes to
distinct addresses without ever linking them therefore grows the shadow. It grows to the bound and
then refuses loudly. There is no timeout and no eviction heuristic, deliberately: an eviction rule
would silently delete the orphan the design exists to keep.

---

## 6. What an unbind may NOT do

The desired-set diff produces unbinds, and one of them is refused rather than performed: **a range
whose binding is host-published**. Dropping it from the table would leave the host object still
allocated and still mapped into that `Vas`'s host address space with no core state naming it —
worse than a leak, because hardware would keep resolving it. That is the `RepointsPublished` rule
(`kayfabe_mmu::walker::PopulateRefusal`) applied to the other direction, and it gets its own
variant so the two are not read as one: `PopulateRefusal::UnbindsPublished`. Unpublishing needs a
worker and an unmap verb — the forwarding plane — so the shadow says so and refuses.
`[test]` `tests/tests/reachability.rs::an_unbind_of_a_host_published_range_is_refused_not_performed`

---

## 7. Residue — what is NOT closed

Stated plainly, because the point of §6.1 was that the design must not be believed complete.

1. **Entry-granularity witness.** §2.2's witness is per page. A page one of whose entries the
   guest wrote under observation and another of which it did not is witnessed throughout, so an
   entry that arrived by an unobserved transport into an already-witnessed page will bind. Closing
   it needs a byte-range witness; `kayfabe_fwd::PtWrite` carries a page and a byte count and no
   offset, so it cannot be closed here.
2. **Transports that are not the copy-engine page-table write.** `resume_from_fault.md` §4.2
   enumerates ten ways a binding changes and this port observes one of them. In particular
   CPU-written entries through the instance/BAR2 window and through PRAMIN — 33 978 PRAMIN writes
   into the same framebuffer backing in each of two captures `[meas]` — reach no witness here.
   Anything they publish stays a miss.
3. **The instance-block page-directory write** (§3.5). Not observed at all.
4. **Sparse's guest-visible semantics** (§3.6). Detected and queryable; no consumer.
5. **Protection changes** (§3.4). Detected and reported; `Binding` has no rights to carry them.
6. **Whether a real MMU, or the guest CPU, can produce a torn entry.**
   `gmmu_publication_discipline.md` §9 item 4 and `resume_from_fault.md` §4.1 leave both halves
   open; the shadow reads whole entries at their natural width and re-reads on the next witness,
   which bounds the damage of a torn read to one pass without settling the question.
7. **None of this ran against a guest.** Every `[test]` above is a mock-level test in this
   workspace. The C artifact remains the only implementation a real NVIDIA driver has accepted,
   and it does not implement this model.

★ And the standing one, from §6.1: **the residue is the input to `resume_from_fault.md` §7 step
5.** Misses exist by construction here — that is what the witness gate *is* — and the backstop for
a miss is the replayable fault buffer, which is not built.

---

## 8. Relationship to the open owner decision

`mode2_address_table.md` §6 — *"a miss is a fault, never a walk"* — rests on a premise §5's own
★ CORRECTION weakened, and `gmmu_publication_discipline.md` §6.1/§6.3 argue that **walk-on-fault**
(as distinct from walk-ahead) would be safe. Whether a miss may walk is an **open owner decision**.

**Nothing in this document depends on which way it goes**, and that is deliberate. The witness
gate answers a different question from the walk question: an entry this port never saw written
stays unbound whether or not a future miss is allowed to walk, because a walk would find the same
unwitnessed entry. If walk-on-fault is later adopted, it becomes a *third* populate source feeding
the same shadow through the same `observe`, with the fault as its trigger condition — and §2.1's
residue argument is what would keep it honest.

---

## 8.5 The bite ledger — each fix removed, each test watched going red

A test that passes both with and without its fix is decoration. So every closure above was
un-done in the tree, compiled, and its named test run; the table records what happened, on
2026-08-01, on the 38-core build box, against this branch.

| the fix, removed | the test that went RED |
|---|---|
| the reachability gate in `settle` (bind an unreachable leaf) | `a_leaf_written_valid_before_its_parent_binds_only_when_the_link_is_published` |
| the witness gate in `settle` (bind an unwitnessed leaf) | `residue_can_make_a_page_reachable_but_never_binds_a_leaf_out_of_it` |
| retirement altogether | `a_pde_clear_retires_the_whole_subtree_and_an_orphan_is_not_retired_with_it` |
| `ever_reachable` in the retirement predicate (so an orphan retires too) | the same test, on its second half |
| `pt_meta.remove` for a retired page, in the commit | `the_pass_drops_the_level_of_a_retired_page_so_its_next_write_is_deferred` |
| the protection-only arm of the diff | `a_protection_only_change_is_reported_and_never_silently_unchanged` |
| the root comparison in `audit_root` | `a_shadow_whose_root_is_not_the_vas_s_pdb_is_a_loud_refusal` |
| the `audit_root` call in the commit | `the_pass_refuses_a_shadow_whose_root_is_not_the_address_spaces` |
| `PteDecode::Sparse` folded back into `Invalid` at the decoder | `sparse_is_a_third_state_and_the_three_transitions_differ` |
| the dual slot's second edge, dropped at the decoder | `a_dual_directory_slot_names_two_sub_tables_and_both_are_followed` |
| the host-published guard on an unbind | `an_unbind_of_a_host_published_range_is_refused_not_performed` |

Eleven planted, eleven fired, and the tree was re-run green afterwards. The harness is
`scripts/bite_reachability.py` and it is committed rather than run once: a bite ledger in a
commit message is a claim about a tree that has since moved, and a re-runnable one is the
difference between *eleven bites fired once* and *eleven bites fire*. It reports three
outcomes that look like success from a distance and are not — a pattern that no longer
matches (the bite was never applied), a removal the compiler rejected (the test never ran),
and a genuine non-biter. ⊘ What this does **not**
say: that the model is right. A bite ledger says each test depends on the code it names, which is
the weakest thing worth having and the one this project has been bitten by not having.

---

## 9. Provenance

- Source read: `ogkm-580.159.04` — the bench's tag (`ogkm_is_versioned`: the vendored 610.43.02
  tree disagrees and was not used here).
- Capture figures are quoted from `resume_from_fault.md` §4.1/§4.2 and
  `gmmu_publication_discipline.md` §0, whose scans were run on 2026-07-31 and 2026-08-01 against
  `cap1_coldboot_hermetic` (359 062 records), `cap3_matmul_forwarding` (532 824) and
  `cap2b_stalequeue_nofn47` (862 940). Nothing here re-ran them.
- ⊘ No hardware run was available. Everything marked `[src@580]` or `[C:]` is a reading;
  everything marked `[test]` is a mock-level test in this workspace.
