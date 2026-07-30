# The questions that cost months if answered wrong

> Written 2026-07-30, at the point where **stage C3 landed and the buildable-without-a-decision
> work ran out**. Ranked by *blast radius*, not by effort. Each entry states what breaks if we guess
> wrong, and — where it matters — **whether we could even detect the mistake ourselves**.
>
> ⊘ This is not a backlog. Small tracked items (`#60`, `#68`, `#78`, `#85`, `#88`, `#89`, `#97`)
> are deliberately excluded: getting them wrong costs hours, not months.

---

## TIER 1 — wrong answers are UNDETECTABLE by us

These are the dangerous ones. Not because they are hard, but because **our own tests cannot tell us
we got them wrong.** A green suite is not evidence in any of the three.

### Q1. How will we know the completion / interrupt plane is correct?

**The state.** `c_rust_trace_differential.md` §5a, limit **L1**: *"the completion plane has **NO C
oracle at all**"* — the C **forges** completions, so a byte-perfect diff against it proves nothing
about ours. And `cap1` constrains the interrupt plane **not at all**: its single `IrqRaise` is the
driver's own `INTR_LEAF_TRIGGER` self-test, `nvkvm_gsp_raise_swgen0` is reachable only from
`deliver_events` which returns immediately with **no os-event registered**, the C posts **202**
status elements and announces **none**, and **zero** `IRQSCLR` writes follow.

**Why it compounds.** Completion is how the guest learns work finished. Get it wrong and the guest
either **hangs forever** waiting for a completion that never arrives, or **proceeds on one that
should not have fired** — and both are silent. There is no assertion we can write today that
distinguishes a correct completion plane from a broken one, because we have no ground truth for it.
This is the single most likely place to lose weeks to blind debugging.

★★★ **What I would want decided:** *how we will obtain ground truth, before we build it.*
- **(a)** One new C capture with an **os-event registered**, so `deliver_events` actually fires and
  the interrupt path executes at all. Cheap, bench-only, and it converts a blind plane into an
  oracled one. **My recommendation** — and it is the single highest-value bench task available.
- **(b)** Validate only against guest behaviour (it hangs or it does not). Honest, but the feedback
  loop is a whole boot per bit of information.
- **(c)** Accept blindness and debug empirically. This is what "months of slop" looks like.

**Blocks:** everything after a doorbell rings. **Costs if wrong:** weeks, silently.

### Q2. What *is* the fabricated aperture — a range, or a predicate?

**The state.** **Where the fabricated aperture begins and how large it is is written down nowhere
in this tree.** The core does not need it (it asks "is this representable?" and is told yes/no),
which is why C3 shipped without it — but `HostRmBackend::fb_read` cannot be built without it, and
it refuses rather than guessing.

**Why it compounds.** *Everything* in the data plane is classified by it. §12's split sends
representable operands to real hardware and unrepresentable ones to us. If the boundary is wrong in
one direction we hand **fabricated addresses to a real engine** (corruption); in the other we
intercept **real memory** (hangs, or a throughput cliff).

★ **The question under the question**, and I think it is the real one: §12's criterion is *"is this
**address** representable"* — which reads like a **predicate**, not an extent. If it is a predicate
(e.g. "has a host backing"), there may be no aperture *range* to define at all, and the real
backend needs something different from what its refusal currently implies. Candidates: a config
value; derived from `NV_USABLE_FB_SIZE_IN_MB`; fixed by the GSP boot / WPR2 layout; or genuinely
per-page.

**Blocks:** C3's real backend, rung 4's `ce_copy`, the GPGA allocator. **Costs if wrong:** silent
corruption.

### Q3. Are we allowed to build a software CE — and can we afford one?

**The state.** `eight_blockers_resolved.md` §11.6 says it plainly: *"a software CE must exist to
perform intercepted copies — which the **execution-plane doctrine says never to build**, and which
the C nonetheless does for kernel channels."* §12 then ruled that we **perform** unrepresentable
copies. **That is a software CE.** The conflict was noted and never resolved.

**Why it compounds.** Two independent risks in one question:
- **Doctrine:** either we build a thing the design forbids, or we refuse a thing the ruling
  requires. Both are structural, and discovering it late means unwinding the data plane.
- **Performance:** a CPU copy on every fabricated write is a potential throughput cliff. The C
  reached ~host parity bare-metal (`49.9` vs `47.5` t/s) — ★ **but I do not know whether that was
  measured with its software CE on the path**, and that matters. If it was, the cost is bounded and
  known. If it was not, we have no data.

**Blocks:** rung 4 and any performance claim. **Costs if wrong:** a data-plane rewrite, or a product
that is correct and too slow.

---

## TIER 2 — wrong answers are detectable, but only late

### Q4. Is v1 single-process — and is that safe to assume?

**The state, and it is uncomfortable.** `#14` (two concurrent CUDA apps) is the **founding problem
of this rewrite**. Yet: `#16` records that **P0–P6 was never complete — only P0+P1 landed**, and
`#95` records that **the bench never compiled past `862c7c2`**, so ★★★ **#14's P0/P1 has never run
on hardware at all.** Meanwhile the C **cannot oracle this**: it runs exactly **one CUDA process per
QEMU lifetime** (measured), so Mode-2 multi-process has no reference implementation anywhere.

**Why it compounds.** The per-process isolate model is the architecture. If it does not hold, the
rework is not a patch. Deferring is defensible — but deferring *without deciding* means we may
build the last mile on an assumption that was never tested and cannot be oracled.

★★ **What I would want:** either *"v1 is explicitly single-process, and the isolate model is
re-validated before v2"*, or *"prove multi-process before first compute."* Not silence.

### Q5. What is v1, exactly?

Single GPU or multi? Compute only, or does graphics/NVENC come along (the C did all three in
**Mode 1**)? Bare-metal, or multi-tenant cloud? **Must the guest be stock?**

★★ That last one is load-bearing and worth separating: the C reproduced on a **stock unpatched
guest**, which is the strongest claim the project has. But if a small guest-side shim were
permitted, several hard problems (completion delivery, aperture identification) get dramatically
easier. Keeping "stock" is a *choice with a price*, and it should be made deliberately rather than
inherited.

### Q6. Is there a performance target for v1, or is correctness the whole bar?

Nobody has stated one for the Rust. It decides Q3, and it decides whether the simple design wins.
"Correct first, fast later" is a fine answer — it just has to be *said*, because some designs are
hard to make fast afterwards.

---

## TIER 3 — known unknowns that will bite on the next hardware

### Q7. `#13` has no root cause and will return

1/3 one day, **9/9 the next on the bit-identical binary** (md5-verified). Root cause **unknown**,
environmental. What survived: the faulting VA was **not in our table** 5/5 ⇒ a **capture** gap, not
a propagation gap. On new hardware this comes back and we will not know what changed. Worth
deciding whether to chase it *before* it costs a debugging session, or accept it as a known ghost.

### Q8. `#95` is a measurement-validity hole, not just a stale build

The bench silently served a binary built from `862c7c2` for weeks. Anything "known from hardware"
about `#14` P0/P1 is **from a binary predating the fix**. This is not one stale result — it is a
period during which our hardware knowledge was not about the code we thought.

### Q9. Hardware strategy

Vast is now (correctly) treated as **ephemeral**: nothing persistent, destroy on crash, rebuild from
the recipe. But the last mile needs a GPU repeatedly. Accept rebuild-per-session, or find one stable
box? This is cost/ergonomics, not architecture — but it sets the cadence of everything in Tier 1.

---

## What I would answer first, if I could only have three

1. **Q1** — because it is the only one where being wrong is *invisible*, and because the fix
   (one capture with an os-event registered) is cheap and available the moment a bench exists.
2. **Q2** — because it blocks two built-but-refusing code paths *today*, and because the
   range-vs-predicate distinction may mean the question is smaller than it looks.
3. **Q4** — not to build multi-process, but to **decide out loud** whether v1 assumes it away, so
   the last mile is not built on an untested and un-oracled premise.

Q3 is nearly as urgent but is partly answerable from the C: measuring whether its ~host-parity
number included the software CE would resolve half of it without a decision from anyone.
