# kayfabe, as it would be written from scratch

**STATUS: DESIGN-ONLY, 2026-09-11.** ⊘ This is **not** a migration plan and nothing here is
scheduled. It is the architecture we would build if the tree did not exist, written so the
present one can be judged against it — *"is the rewrite a so much better product to ship?"*
Where it disagrees with the code, the code is not thereby wrong; it is thereby **measured
against something**.

⊘ It inherits no constraint from the current tree on purpose. Every place the existing design
would object *"but we already have X"* is exactly a place worth examining.

---

## 1. What the product is

A stock, unpatched NVIDIA driver runs in a KVM guest and drives a real host GPU. The guest
sees an emulated GPU whose firmware processor (the GSP) we impersonate. We recover the guest's
*intent* from what it writes, and perform the equivalent work on the host through ordinary
unprivileged userspace interfaces.

Two things follow from that sentence and the whole architecture is downstream of them:

1. **We are the GSP.** Every structure the guest's RM builds for its firmware — page tables,
   instance blocks, message queues — is read by *us* and by nothing else. The host GPU never
   sees them.
2. **The host GPU is real.** Pushbuffers, operands and semaphores are executed by actual
   hardware. Those bytes must be in memory the hardware can reach, in the layout it expects.

⇒ **Every byte in the system has exactly one consumer: us, or the hardware.** That is the
organising principle of this design, and §4 is nothing but its consequences.

## 2. The invariants, stated first

These are not properties the system acquires. They are the shape it is built in. Each is
enforced by a *mechanism* named beside it, because an invariant with no enforcement is a
comment.

| # | Invariant | Enforced by |
|---|---|---|
| I1 | No blocking call on a vCPU thread. | An assert at every blocking door, fatal in CI |
| I2 | No blocking call while holding a lock. | The same door, rank witness |
| I2b | A vCPU can never *wait* on a blocked thread. | Lock partition (§3): the one lock a vCPU takes is never held across a blocking call |
| I3 | A vCPU MMIO write does bounded O(1) work and returns. | I1 + the write handler's shape (§5) |
| I4 | Page-table and mapping state changes only at a synchronization point. | Refresh owns the writes; nothing else has the handle |
| I5 | A completion is written only when the state it claims is true. | The completion is written *by* refresh, after it finishes |
| I6 | One guest process cannot observe another's memory. | One isolate per process; mappings installed only in the owner's |
| I7 | We never parse anything the hardware executes. | The two-store split (§4): executable bytes are never in a store we read |

★ I1 and I2 are one rule with two halves. Enforcing only I2 is the specific mistake the
current tree made, and it is invisible: a call moved out from under a lock and left on the vCPU
satisfies every lock assertion there is.

## 3. Threads

Exactly three kinds, and the rules are total.

**vCPU threads.** Run guest code, and our code only inside an MMIO trap. May: classify a
write against a table, update one word of shadow state, push to a bounded queue, wake a
coordinator, write one value to an already-mapped host doorbell, return. May not do anything
else — no syscall, no allocation that can fault, no host verb, no page walk, no process spawn.

★★★ **And it may take exactly ONE lock: the queue's.** That lock covers the queue and the
shadow words, and nothing else in the system. It is held for a push and released — a handful
of instructions, no call out of the critical section, ever.

⊘ Every other structure — the address table, the page-table shadows, the isolate registry, the
store maps — lives behind locks **a vCPU thread cannot take, because no code path it can reach
names them**.

⇒ **This is what makes the transitive-blocking rule structural instead of a discipline.** The
danger is never "a vCPU blocked"; it is "a vCPU waited on a lock held by a thread that
blocked". If the only lock a vCPU can acquire is one nobody ever holds across a blocking call,
that cannot happen, and it cannot be reintroduced by someone adding a call six frames down. An
earlier draft of this section said *"no ranked lock"*, which is both wrong and weaker: the vCPU
does take a lock. The point is **which one, and for how long**.

⊘⊘ **Do NOT make the shadow lock-free to avoid that lock.** `[measured w466/w469]` that is
exactly what was tried: the deferred-doorbell count was mirrored into an atomic so the hot path
need not take the plane lock, the mirror was refreshed on the write path only, and it went
stale the moment the coordinator serviced one — after which every MMIO write enqueued a
spurious token and slow traps went 169 → 6776. **The lock was not the problem; the second
source of truth was.** One short lock over queue-and-shadow together has no staleness question
to get wrong.

**The coordinator.** One thread. Owns everything that is not O(1) shadow. Drains the queues,
runs refresh, issues host RM calls, performs mmaps, spawns isolates, writes completions into
guest memory. It is allowed to block freely, because the only lock it shares with a vCPU is
the queue's, and it never holds that across anything.

**Host isolate processes.** One per guest process. Exist for **virtual-address identity**: the
host GPU must see the guest's addresses, and one address space per process is the only way to
give that without collisions. They are also the isolation boundary (I6), but identity is the
reason they exist.

⊘ **What a worker pool would buy and why it is not here:** refresh is serialised by design
(§6), and every other coordinator task is short. A pool adds concurrency where the contended
resource is the guest's own page tables. If measurement later shows the coordinator saturated,
the split is by *kind of work*, never by sharding refresh.

## 4. Memory: two stores, split by consumer

This is the heart of the design, and it is the thing the current tree does not have.

**Store A — the fake framebuffer.** A sparse `memfd` the size of the guest's VRAM. Guest
framebuffer-physical address *is* the offset into it; there is no translation and no allocator.
Holes cost nothing and the kernel materialises a page on first touch, so a 12 GiB store with
40 MiB live occupies 40 MiB.

It holds **everything RM CPU-touches**: GMMU page tables and directories, instance blocks,
GSP message queues and boot arguments, and every RM-internal descriptor it maps for itself.
**Only we ever read these bytes.** They reach the guest through BAR2 and PRAMIN, and nothing
else in the system looks at them.

**Store B — real video memory.** One host RM allocation covering the guest's whole VRAM. It
holds what **the hardware** executes and touches: pushbuffers, kernel operands, semaphores,
user surfaces. It is bound into channels and reachable through BAR1. **We never parse it** (I7).

**Guest RAM.** A `memfd` shared with the isolates, so a host engine can reach a guest-RAM
operand without a copy.

★★★ **Why the split is safe, and it is not an assumption.** BAR2 and PRAMIN carry structures
whose only reader is us; BAR1 and the channels carry bytes whose only reader is the hardware.
Two stores cannot diverge when no byte has two readers. This is §1's observation cashed in.

⊘ **The one case that would break it** is a single allocation both CPU-mapped internally by RM
and mapped by a client, which would want to live in both stores.

⊘⊘ **AND IT IS OBSERVED, NOT REFUSED.** An earlier draft of this document made an overlapping
physical page a named refusal. That was wrong twice over, and the second reason is the serious
one:

- **Overlap is not by itself a defect.** Free-then-reallocate legitimately puts a page in both
  images for the width of a window — the tables are guest-controlled and we read a snapshot of
  something mid-update. Nothing has leaked and nothing has diverged yet.
- **★★★ A fatal check on a guest-observable condition is a guest-triggerable DoS.** The guest
  writes both page tables. If overlap aborts the VMM, the guest can abort the VMM at will. A
  diagnostic that an attacker can fire is not a diagnostic; it is the bug.

⇒ the design **counts** it and surfaces a census line, exactly as the vCPU-blocking witness
does. **Transient overlap is noise; the same page in both images across consecutive refreshes
is the signal**, because that is what a genuinely shared allocation looks like and churn does
not.

★ **What still needs verifying, and it is a reading task, not a runtime one:** whether the
driver ever *depends* on seeing the same bytes through BAR1 and BAR2. If it never does, the
two stores holding different content at the same framebuffer offset is acceptable, and the
split is sound even when the images overlap. ⚠ Unverified as of this writing.

⇒ **What this deletes outright:** promotion, depromotion, the aperture-ownership decision, the
shadow/join/alias machinery, and every refusal predicate that exists to police them. Nothing
has to decide *which kind* of memory an address is; the question is answered by which table
maps it.

## 5. The MMIO contract

The guest's entire view is three regions, and each has exactly one rule.

**BAR0 — registers.** Backed by a **read-only** memory slot over a shadow page the coordinator
keeps current. Reads therefore never leave the guest: a driver spin-polling a status register
spins on memory at zero exits, and the coordinator's write is what releases it. That is the
completion contract (I5) made free rather than expensive. **Writes** take a VM exit and run the
handler of §3 — classify, one shadow word, enqueue, wake, return.

⊘ Registers whose *read* has a side effect cannot be shadow-backed. They are an **enumerated**
list with the argument written beside each, not a discovery.

**BAR1 — the framebuffer aperture.** Untrapped. One mapping over store B, whose layout is
rebuilt from the guest's own BAR1 page tables at refresh. A page the guest never mapped reads
as nothing, which is what hardware does; there is no fault to service and no CPU stall.

**BAR2 — the instance window.** Untrapped. Slices of store A mapped at the offsets the guest's
BAR2 page tables name, refreshed when those tables change. Since store A is a `memfd` and
framebuffer-physical is its offset, a slice is one fixed mapping and nothing else.

**PRAMIN — the bring-up window.** One fixed mapping of store A at the base the guest writes.
On a base change the window moves, synchronously, on the vCPU. ★ **This is the only permitted
vCPU stall in the system**, and it is permitted because there is no completion for the guest to
wait on and the driver uses the window only while bootstrapping BAR2. It is one mapping call.

**Doorbells.** A passthrough doorbell is a single store to a host mapping established earlier,
off the vCPU. No state, no queue, no completion. Every other doorbell is an ordinary BAR0 write.

⇒ Each of these is a handful of lines. The write handler for the whole device should be
readable on one screen, and if it is not, something has been let back in.

## 6. Refresh: the only place state changes

Everything that changes mapping or translation state happens in one function, called only from
the three points where the guest tells us it has changed something:

| # | Point | How the guest waits |
|---|---|---|
| S1 | TLB invalidate (`NV_PFB_PRI_MMU_INVALIDATE`) | Spins on the trigger register, which lives in the BAR0 shadow |
| S2 | RM call (`GPU_PROMOTE_CTX`) | Waits for the RPC reply, which refresh writes |
| S3 | UVM kernel channel (`MEM_OP_A`/`_D`) | Waits on the push's semaphore |

⊘ **There are three because the driver does not unify them**, and we must serve each. On real
hardware they converge inside firmware; we are that firmware.

Refresh holds **one lock, used by nothing else in the system**. It may block freely, because
no vCPU can ever wait on that lock — which is how I2 is satisfied without care being required.
A refresh arriving while one runs waits for it. That is contention we accept and can measure.

One pass does, in order: bring the BAR1 mapping up to date; bring store A's BAR2 slices up to
date; bring the address table up to date; **then** release whichever of S1/S2/S3 asked. The
release is last, always, so I5 holds by construction rather than by review.

★★★ **And none of those three is a full re-walk.** Every page table in the system — BAR1's,
BAR2's and the channels' — lives in **store A, which is ordinary RAM we own**. Ordinary RAM
has dirty tracking. ⇒ a refresh reads which pages were written since the last one and re-walks
only those subtrees, so its cost tracks **what changed**, not how large the tables are. A
guest that maps nothing new between two invalidates costs a bitmap read.

⊘ This is why the two-store split pays a second time. The reason the tables are cheap to
watch is the same reason they are safe to hold apart: they are RAM with exactly one reader.
⚠ Dirty tracking is not free — the mechanisms that provide it write-protect the pages, so the
first write after each read takes a fault. That is a cost per *modified page*, which is the
right shape, but it is a real cost and it is unmeasured.

## 7. Addresses

One table: guest channel virtual address to an offset in store B. Forward-populated at refresh
from the guest's own page tables, never reverse-resolved from a host pointer. A miss is a
fault, reported the way hardware reports one.

**Host virtual addresses are never guest-chosen and never guest-visible.** The isolate exists
so the host GPU sees the guest's own addresses without us handing the guest anything of ours.

## 8. Hostile guests

The guest is assumed hostile and the design owes it three things.

It may write garbage into any structure we read. ⇒ every walk is bounded, every decode is
total, and a malformed entry is a named refusal, never a panic and never a host action.

It may corrupt its own state underneath us. ⇒ that is permitted. We defend against a
**breakout or an illegal action**, not against a guest damaging itself. A guest that
invalidates its own mapping while racing its own access gets its own stale view, which is what
hardware gives it.

It may attempt to reach another process's memory. ⇒ I6. A mapping is installed only in the
isolate that owns it. Absence answers *not found*, never *denied*, so probing learns nothing.

⚠ The sparse store has a resource shape worth naming: holes are free for us and free for an
attacker, so a guest touching every page of it materialises real host memory. A cap on live
pages belongs in the design even if it is built later.

## 9. What does not exist in this design

Stated positively, because knowing what is absent is how the plan is judged.

- No promotion, depromotion, or shadow decision. §4 answers the question they existed to ask.
- No aperture-ownership resolution for a framebuffer address.
- No demand-faulting mirror, and no discovery of mappings by trapping. Refresh is authoritative.
- No heterogeneous framebuffer store in which any page might be a page table or might be data.
- No behaviour flags. An arm is an experiment; the loser is deleted when it is settled, and a
  setting that survives is a product decision with a name, not an environment variable.
- No deferral as a special case. Everything off the vCPU is deferred, so there is no second
  path to maintain and no arming to get wrong.

## 10. How it is verified

**The invariants are asserted at the mechanism, not enumerated in tests.** I1 and I2 live at
the one door every blocking operation already passes through, so a new blocking call in a new
place is caught the first time it runs, by name, without anyone having thought of it.

**A differential oracle beside a real card.** The same workload traced on hardware and in the
guest, diffed. It answers *where* we diverge, which no amount of internal testing can.

**Every claim carries the revision it was measured at**, and an empty artefact is never read as
an absent problem.

## 11. Honest limits of this document

- **It is not costed.** The claim that it is smaller is a claim about concepts removed, not a
  line estimate, and should not be read as one.
- **The BAR0 read-shadow is the least proven piece.** It rests on the set of read-side-effect
  registers being small and enumerable. That is checkable against the register model and has
  not been checked.
- **Refresh's cost is unmeasured.** Dirty tracking bounds it to what changed rather than to
  table size (§6), which is the right shape, but the write-protect fault it costs per modified
  page has not been timed. It is off the vCPU, so it cannot violate I1, but it can still be
  too slow.
- **It says nothing about display, multi-GPU, or driver-version breadth.** Those are real
  product requirements and this document does not address them.

---

## Appendix — the measured denominator, so the target can be judged

`[measured 2026-09-11, at `ea4bd4bf`]` counting `.rs` under `crates/`, excluding comment
lines, blank lines and `tests/`+`benches/` directories:

| | lines |
|---|---|
| **product code** | **118,989** |
| test code | 31,754 |
| comments (whole tree) | 117,469 |

⊘ The often-quoted *"200k"* is 281,186 total lines, of which **42% is comments**. Comments and
tests are not the target and cutting them would remove the two things that have most often
caught a live defect in this campaign.

A 50,000-line product-code target is therefore a **58% cut of 118,989**, not the 3x cut a naive
reading of the total suggests. Where it would plausibly come from, largest first:

| source | why it is real |
|---|---|
| the concepts §9 deletes | promotion, aperture ownership, shadow/join, the demand-fault mirror and their refusals span the device, core, rt and fwd crates |
| 51 environment arms | every arm keeps both paths alive; a settled arm's loser is pure subtraction |
| `kayfabe-abi` at 25,101 | transcribed ABI; the reachable subset is bounded by `RMCTRL_FLAGS_ROUTE_TO_PHYSICAL` and is a mechanical question, not a judgement |
| `kayfabe-isolate-host` at 21,269 | one 15.6k-line ladder binary in it is a harness, not product |

⚠ **This appendix is a measurement, not a promise.** Whether the architecture above actually
lands near 50,000 is unknown until it is built, and a line target is the wrong thing to steer
by in any case — steer by the concepts removed in §9 and let the count follow.
