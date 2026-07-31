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

---

# ANSWERED — the owner's rulings, 2026-07-30

All six were answered the same day they were raised. Recorded here because several **changed shape**
from the question as I posed it, and two dissolved rather than being decided.

## Q1 — the completion / interrupt plane → **staged, not blind**

**Ruling:** the interrupt path is the **slow path**; correctness rests on **semaphore polling**.
Treat interrupts as an **optimisation with a fallback**, and test that disabling the optimisation
keeps everything working.

⚠ **My correction, which the owner accepted:** *we do not choose which path the guest takes.* If its
driver blocks on an os-event and we never deliver, it **hangs** — polling is not ours to fall back
to. Mode 1 hit exactly this as `#127`, whose fix (an os-event poll relay) is recorded as
load-bearing for Mode-2 interrupt delivery. **An interrupt capability must exist even if delivery is
staged.**

★★★ **What rescues it is Q5:** since the guest may be patched for bring-up, we can **force the guest
to poll** during development, isolating the interrupt plane into a separately testable step. That
converts an unbounded, un-oracled risk into a staged one — and is strictly better than my proposal
of chasing another capture.

## Q2 — the fabricated aperture → **the question was wrong**

**Ruling:** *"you ask it wrong. the address or size parameter itself never was sufficient alone…
better is, if I have a region containing this block of memory, what is: gpu object backed / faked
ram / null pages / unallocated — and then decide per such region."*

★★ **This is already built.** Stage C2 landed exactly that: `Representability {HostBacked,
Fabricated, PhysicalOperand, Untracked}`, `AddressTable::spans()` as the region query, and
`partition_ce()` intersecting both operands. The "aperture extent" is **not needed as a global
fact**.

★★★ **And the sharper half:** *a guest-userspace CE can only address GPU VA and guest-process VA,
both valid in the isolate, so it forwards **without inspection**; only privileged CE must be
handled.* Independently the C's own `is_user_ce` predicate — derived from first principles rather
than copied.

★★ **The map-time argument that retired my objection.** I worried a naive `origin == User` fast path
would make the rare case (fabricated VRAM in a userspace GPU VA) *invisible*. The owner: *"you should
not have an incomplete guest gpu va that has some things not mapped in the first place… if its gpu
va anywhere we have mapped it."* **Every VA the guest can name is one we published**, so its backing
was decided at **map time**, not CE time. The rare case is not invisible — it is **impossible to be
unhandled**. My "fast-path + assert the premise" would have asserted a state the invariant already
excludes. C2 had in fact implemented this already: the fabricated-VRAM case is *"the normal path —
ordinary publication, after which the classifier answers `HostBacked` by itself"*.

## Q3 — the software CE → **allowed, and bounded**

**Ruling:** a software CE is permitted **only for privileged/kernel CE that directly addresses
GPGA**. Everything VA-addressed forwards.

⇒ The §11.6 doctrine conflict is resolved: the doctrine's *"never build a software CE"* governs the
**fast path**; the bounded privileged case is what the C does too. **Never on the 99% path**, so the
performance risk is bounded by construction.

★ The crisp statement: **the only genuine requirement to emulate is a CE whose operands are GPGA.
Everything VA-addressed can be forwarded, because we control the mapping — emulating it is an
optimisation, not a requirement.**

## Q4 — multi-process → **assert it in mocks now, test on hardware later**

**Ruling:** *"your mock tests need to assert #14 isn't an issue, this already fixes 95% of an
architecture flaw. then yes I would focus on getting #14 to test as soon as possible, but if gsp boot
sequence didn't even complete yet with rust… I think the test is futile now on real hardware."*

★★ And it named a **better intermediate rung** than the one I was aiming at: **guest driver loads and
`nvidia-smi` enumerates**, between "RM ioctl works" and "CUDA kernel runs". That reframe is what
produced the first end-to-end boot attempt the same day.

## Q5 — must the guest be stock? → **split by purpose**

1. **Debugging and testing: NO.** Patch the guest's ogkm, kernel or userspace however is useful.
2. **The shipped product: YES** — *"thats the whole selling point. its what allows linux isos to work
   out of the box and windows eventually."*

★★★ The single most useful ruling of the day: it unblocks bring-up instrumentation **without**
weakening the claim that matters. (And the first boot needed **no** guest patches anyway.)

## Q6 — performance target → **by path frequency, with one addition**

1. **The common 99% path:** performance **and** correctness.
2. **The uncommon <1% path:** correctness only.
3. ⊘ **But it must not become a bolt-on** — *"you need to have the code written against the spec so
   the uncommon is already covered by construction."*

★★ **My addition, accepted:** the trap on the <1% path is not performance, it is that **an uncommon
path with no perf pressure is exactly where a plausible-but-wrong implementation survives**, because
nothing stresses it. So the <1% path needs *correctness plus a falsification story*, not correctness
alone. (Same shape as Q1's oracle-less completion plane.)

## Q-VBIOS — generate, never dump

Raised after the first guest boot stopped at `kgspExtractVbiosFromRom`. **Ruling:** a dumped ROM is
*"an unstable hack that requires one time root and is difficult with shipping"*; generate a **fake
VBIOS that passes ogkm's parsers**, as a **generator for any driver/arch**, with **no pinned version
or arch** — part of the auto-gen, *"it shouldn't become a bolt on"*.

★★★ **The stronger reason, which does not decay:** a dumped ROM describes the **host's** card while
we emulate a **different** device, so it can silently disagree with the registers we answer. A
generated one, derived from the same config, **cannot disagree by construction.** The dumped ROM was
always the *wrong source of truth*, not merely the inconvenient one.

**Verified before building** (`ogkm-580.159.04`): the driver performs **no cryptographic
verification** of FWSEC — `kernel_gsp_fwsec.c:993` copies signatures out, `frts_tu102.c:355`
null-checks them, `:397` hands one to the **falcon**. In Mode 2 **we are the falcon**. ⇒ **structure,
not secrets.**

**Outcome (`7825926`):** the driver parses the generated ROM, echoes `VBIOS version 94.18.00.00.00`
— our own profile, from a field initialised to `"unknown"` — and is **past the ROM gate**. Generating
against 580.159.04 and 610.43.02 produces byte-identical output, and the diff has teeth (over the
same tags `nvos.rs` moves 122 lines). New stop: `BAR0+0x110100`, the GSP falcon's `CPUCTL` halt bit.

---

# NEW — raised by the overnight run of 2026-07-31

> ⚠ **Numbered Q10+ deliberately.** `Q7`/`Q8`/`Q9` were already taken by the Tier-3 entries above
> (the `#13` ghost, the `#95` stale-bench entry, hardware strategy). An earlier draft of this
> section reused those numbers, which would have read as answering them.

Three questions that **measurement produced**, not analysis. Each is a design decision rather than a
patch, each is written down as a live defect with a pinning test, and none is blocking today's north
star (GA10x, 580). They are here because answering them wrong is the kind of thing that costs months.

## Q10 — the boot FSM is an unhooked IMPLEMENTATION → **ANSWERED 2026-07-31** (task #121)

> ★★★ **OWNER RULING, and it reframes the question I asked.** *"Logic that's arch independent remains
> arch independent. What's not true is that for a series of archs you can't have code that's only for
> those archs — but then it's no longer in core logic, it's an implementation. The implementation
> should be constructed so that it has enough methods, data and flow to override or hook on, so any
> arch can be implemented."*
>
> **Three consequences.** (1) The arch-*independent* half of the claim **holds** and is not in
> question; the CI grep that keeps generation names out of generation-free code is right and stays.
> (2) Arch-*specific* code is **legitimate and must exist somewhere** — a Turing/Ampere GSP boot
> sequence is a real thing that has to be written down, and the defect was never that it exists.
> (3) ★★★ **The defect is that it is UNHOOKED**: `boot.rs:544-598` encodes the Turing ordering in
> `match` arms with no extension point, so a second architecture cannot supply its own sequence
> without **editing the first one's**.
>
> ⇒ **The acceptance criterion changes, and becomes directly measurable:**
> ⊘ *not* "adding an arch costs zero logic-crate edits"
> ★ *but* **"a new arch can be implemented by ADDING ALONGSIDE, without MODIFYING the existing
> arch's implementation."**
> Test it by implementing GH100's boot sequence and counting lines **changed** (not added) in the
> Turing path. Zero changed = the seam works.
>
> The work is to give the boot implementation enough **methods, data and flow** to hook or override:
> which registers exist on a generation, which transitions they drive, what the stages are and their
> order. GH100 is the forcing case; its cost is enumerated in `kayfabe_chips::gh100::MISSING_TRANSITIONS`.

### What was run (task #118, `554c333`, both vendored ogkm tags)

**What was measured** — run named: task #118, commit `554c333`, both vendored ogkm tags
(580.159.04 and 610.43.02), pinned by `tests/tests/arch_axis_second_generation.rs`. The claim
*"adding a GPU generation is an adapter-crate impl with zero logic-crate edits"* was asserted in
three places and had never been executed. It now has been, on two
generations, and they answer in opposite directions:

- **Ada holds** — one struct, one `VBIOS_PROFILES` row, boots the unmodified `GspFsm` to `Booted`.
  ⚠ But it is the **easiest member of the universe**, and provably so: NVIDIA's own generated HAL
  binds the Turing/Ampere implementations across `TU102…AD107`, so for the registers `GspReg` models,
  Ada and GA10x are **byte-identical**. An experiment that selects its easiest case produces a green
  with no red available to it.
- **Hopper fails, inside a logic crate.** Four of eighteen `GspReg` variants have **no register** on
  GH100 and three are ones `GspFsm::mmio_write` fires transitions on. Not offsets — Hopper's offsets
  are mostly Ampere's.

★★★ **The root cause, and it is the question:** `kayfabe-gsp/src/boot.rs:544-598` encodes **Turing's
boot ordering itself** in `match` arms over `GspReg`. `Sec2FalconMailbox0` latches the Booter arg;
`Sec2FalconCpuctl` decides Load-vs-Unload; `GspQueueHead` advances the ring. **The seam transports a
generation's VALUES but not its SEQUENCE.** Hopper does not have that sequence at all — it boots a
partitioned GSP-FMC through FSP's EMEM queue, and boot stages 1, 2, 6 and 8 do not exist there.

**Why it is a decision, not a task.** Making a boot *sequence* data means choosing a representation
for it — a per-generation state table, a trait with a default Turing impl, or an accepted per-arch
module. Each has a different cost when a generation disagrees *structurally* rather than numerically.
⚠ Note the honest distinction we are relying on: a **one-time seam widening** is not a bolt-on (the
register plane got one this session and it is fine); a **per-generation `match` arm** is.

**Not urgent** — GA10x is the target and Hopper is not on the roadmap. But this is the **first
measured failure** of an architecture claim the project has repeated for months, and the fix is a
shape, not a patch. Was pinned by `the_boot_fsm_cannot_be_driven_past_fwsec_on_that_generation`,
which went **red if the seam was widened** without answering this.

### ★★★ What was built, and the number the ruling asked for (task #121, 2026-07-31)

**The seam.** `kayfabe_arch::BootSequence` — an implementation of a generation's boot *ordering*,
sibling to `GspModel`, which always carried its *values*. Three parts, matching the ruling's three
words:

| the ruling's word | what it became |
|---|---|
| **methods** | `BootSequence::on_write` / `::on_read`. `on_write` takes a `RegWrite` carrying **both** the raw `(bar, off, val)` **and** `decode_reg`'s opinion of it, because a generation driven by registers no `GspReg` names has only the former. |
| **data** | `BootStageDesc` — the named stages of a cold boot, in order, as a table. Plus `ArchBootState`: latches and a byte window the FSM stores and never interprets, so a `&self` sequence can still remember something across writes. |
| **flow** | `BootStep` — eight arch-independent effects (`StartProcessor`, `FirmwareLoaded`, `Teardown`, `BootArgsLo/Hi`, `PublishBootArgs`, `CommandDoorbell`, `ClearStatusIrq`), applied by `GspFsm::apply`. `mmio_write` no longer names a register. |

`GspModel::boot_sequence()` is **not defaulted**: a default would have to be some generation's
ordering, which is exactly how one becomes "the" shape. `NoBootSequence` is the explicit
*not-implemented-yet* answer.

**⊘ Turing did not become the shape.** `FalconSecureBooterBoot` (`crates/kayfabe-gsp/src/seq.rs`) is
the old `match`, body for body, now *an implementation a generation selects*. It carries no offsets
and no chip facts — it is written in `GspReg` and in `GspModel`'s predicates — which is why
`kayfabe-device` (GA10x) and `kayfabe-chips` (Ada) both select it and neither inherits the other's
ordering. `BootPhase::FwsecRan` was renamed `ProtectedRegionUp` in the same move: FWSEC is one
firmware's name for one generation's way of reaching that stage.

**★★★ THE NUMBER.** `git diff --numstat e5c0f45..1292f80` — the seam commit to the commit that
implements GH100's boot ordering — names exactly two files: `crates/kayfabe-chips/src/gh100.rs`
(+407/−67) and `tests/tests/arch_axis_second_generation.rs` (+499/−95). Lines **changed** in the
falcon path (`kayfabe-gsp/src/seq.rs`, `kayfabe-device/src/ga10x.rs`, `kayfabe-chips/src/ad10x.rs`)
and in the logic crates `kayfabe-arch` / `kayfabe-gsp`: **ZERO**. GH100 declares **three** boot
stages against the falcon regime's **five**, because one FSP command does the work of four falcon
writes, and a test drives both generations' registers and compares the emitted steps to the declared
stages.

**★★ Per-GPU, not per-build** (the follow-up ruling: *"multi gpu with each gpu its own arch shouldn't
become bolt on"*). A boot ordering is reached by a method on a per-instance `Box<dyn GspModel>` value
— `RegPlane` already holds one — and its mutable state (`ArchBootState`) is a field inside the
per-instance `GspFsm`. No generics, no compile-time selection, no process-global. So **yes**: two
`GpuId`s could carry different boot implementations without changing the shape, and `MG-6`
`HeterogeneousArch` stays a *policy* refusal rather than becoming a structural one.
`boot_orderings_are_per_instance_values_not_a_build_time_choice` holds three models from two crates
alive at once and asserts each answers with its own ordering.

**★ A #118 reading corrected.** `GspReg::GspQueueHead` was recorded *absent* on GH100 because
`hopper/gh100/dev_gsp.h` defines no `NV_PGSP_QUEUE_HEAD` and "the only writer in the tree is the
Turing HAL". Both halves are true; the inference is wrong. `kgspSetCmdQueueHead` is halified and the
**Turing binding is the fallback for every chip** that is not a VF or a Tegra part
(`ogkm-580: src/nvidia/generated/g_kernel_gsp_nvoc.c:664-679`), so GH100 writes `0x110c00+(i)*8` on
every RPC send (`ogkm-580: kernel_gsp.c:425`). *"This chip's header does not define the symbol"* is a
fact about headers, not about which registers the driver writes.

**What the seam does NOT solve, said out loud.** `MISSING_TRANSITIONS` is renamed
`ARCH_LOCAL_BOOT_EVENTS`; three of its four entries are now served through the seam, and the fourth —
Confidential-Compute WPR2 suppression, `kgspIsWpr2Up_GH100` returning `NV_FALSE` unconditionally
(`ogkm-580: kernel_gsp_gh100.c:220-236`) — is a **missing observable**, not a missing ordering, and is
left in the list marked `STILL UNMODELLED`. A test asserts the list still names something unsolved.

⊘ **Still not a Hopper port.** Nothing has touched Hopper silicon; `Gh100FspBoot` enumerates its own
limits (no FSP reply, no multi-packet NVDM reassembly, no `AINCR`, no teardown).

**What was measured** — run named: task #121, this bench (38 cores), commits `e5c0f45` (seam) and
`1292f80` (GH100), rebased onto `c97b640`. `cargo test --workspace --no-fail-fast` → **1329 passed /
0 failed** at `e5c0f45` — the seam adds no net test, it rewrites one in place, so that is its
parent's count — and **1335 / 0** at `1292f80`, the +6 being the tests named above. clippy
`-D warnings` clean; `scripts/ci_gates.sh --all` → **ALL GATES CLEAN (19 steps, floor 19)**;
`claim_ledger.py --gate` unchanged at 383 / 66 / 17. ★ The same two trees measured **1305 / 0** and
**1311 / 0** before the rebase, against a `b1d3672` baseline of 1305 / 0 — identical deltas; the
absolute totals moved by +24 because of the four upstream commits. Six induced defects were each watched to fire: dropping
`FirmwareLoaded` from the FSP command (2 tests red), declaring a stage nothing emits (1), reverting
the queue-head correction (1), letting GH100 inherit the falcon ordering (6), freezing GH100's
`HWCFG2` (1), and breaking the falcon regime's own `FirmwareLoaded` guard (**14 failures across 6
suites**, 1297 passed / 14 failed — which is what proves the moved body is live and that GA10x
still runs through it).

## Q11 — how does a driver version express **REMOVAL**? → **ANSWERED 2026-07-31** (task #122)

> ★★ **OWNER RULING:** make **subtract** possible as well as add. That expresses *replace*
> (subtract the old, add the new), which is exactly the defect recorded at task #118 (`554c333`) and
> read from `gvisor nvproxy: version.go:1036-1053` — nvproxy replaces two
> DRAM-encryption commands at 575.
>
> ★ **One caveat carried into the task, not a disagreement.** `inherit-then-{add,subtract}` is a
> **delta chain**, and this repo has already been bitten by that shape: the *gates quantified over a
> list* finding, where shortening a list weakened a gate with **zero red tests**. A subtract in an
> early version silently shrinks every later version's set, and you cannot see what 575 actually
> allows without replaying the chain. ⇒ **Add subtract, and materialize each version's RESOLVED set
> and gate on it**, so a subtract's effect is visible per-version rather than implied, and a mistake
> stays local instead of propagating forward in silence.

### What was run (task #118, `554c333`, both vendored ogkm tags)

**What was measured** — run named: task #118, commit `554c333`, pinned by the characterisation test
added to `crates/kayfabe-abi/src/capability.rs`; the replacement itself is read from
`gvisor nvproxy: version.go:1036-1053`. 575.51.02 was added as a second driver version. Its
*additive* half is one `TABLES` row, exactly as designed. Its **subtractive half cannot be expressed at all**: nvproxy
*replaces* two DRAM-encryption commands at 575, while `CapabilityTable` is **inherit-then-ADD** — its
own module doc says so.

**Live consequences today**, now pinned by a characterisation test rather than fixed:
- `0x20801359` is **refused at every version**, although a 550/570 guest legitimately issues it.
- `0x20801358` is **permitted under the 575-era name**, while pre-575 guests mean a *different
  command* by that number.

★★ **Why it matters beyond two commands.** The standing requirement is 575/570/etc *"out of the box
and no bolt-on"*. A version model that can only ever ADD will keep accumulating wrong answers as
versions diverge: every command a newer driver **removes or repurposes** becomes either a false
permit or a false refuse, silently. The question is what a version *is* — an increment, or a full
description that can differ in both directions.

★ Related and deferred — same run (task #118, `554c333`), and the version delta itself is read from
the generator's own record over `ogkm-580` vs `ogkm-610`: `kayfabe-abi/src/submit.rs`'s
`ChannelAllocParams` is
documented *"abstract by construction, unbuilt"* but is concretely **pinned to 580** by a const-offset
encoder used unconditionally in `kayfabe-isolate-host`. 610 inserts `hHandleVASpace` at +32 and shifts
every later field. Adding a version parameter has blast radius outside `kayfabe-abi`.

### What was BUILT (task #122, on top of `b1d3672`)

**Shape (b), not (a)** — the owner's second phrasing, which supersedes the first: `CapabilityTable`
is now **one shared base + per-boundary blocks**, `resolved(boundary) = SHARED_CAPS ∪
own_blocks(boundary)`, depth exactly two, **no `inherits` pointer**. Add *and* subtract are both
expressible, but there is no operation for either: a boundary that must not have a row simply does
not name the block carrying it. That dissolves the delta chain the caveat above warns about instead
of managing it — an early subtract cannot shrink a later boundary's set, because no boundary reads
another's. Cost, paid deliberately: a block four boundaries share is named four times.

**One axis: driver version.** The owner's phrasing said *arch*; the only source for this data —
nvproxy's registry — is a chain of driver **versions**, and a `CapabilityTable` is reachable only
through `DriverAbiTable`. The shape is variant-agnostic, so an arch axis later is more variants over
the same struct; building it now would be rows no traffic can reach.

**The two live consequences changed**, and the characterisation test that pinned them was rewritten
to assert the right answer rather than deleted
(`the_575_boundary_replaces_two_dram_encryption_commands`):
- `0x20801359` was refused at every version; it is now **permitted at 570.86.15** — the only
  boundary whose vendor map has it — and refused at 550/555/560 (never existed) and 575+ (deleted).
- `0x20801358` was permitted at every version under the 575-era name; it is now
  `..._INFOROM_SUPPORT` at 570, `..._STATUS_V575` at 575+, and **refused** below 570.

**A third, independent removal was carried too**: `NVC36F_CTRL_GET_CLASS_ENGINEID` is in nvproxy's
base map (`gvisor nvproxy: version.go:360`) and deleted at 555.42.02 (`:933`). The port refused it
at every version, including the two where a 550 guest legitimately issues it. Two `TABLES` rows were
added (550.90.07, 555.42.02) so the boundary exists to say it at. One worked example is not a
mechanism; this is the second, at a different boundary and in the other direction.

★ `ChannelAllocParams` was **left alone**, and the redesign did not make it cheaper. It is a *wire
layout* problem, not a capability-set one: the fix is a `ChannelAllocWire` field on `DriverAbiTable`
(the `MapDmaWire` precedent, which already existed) plus a version-taking `encode_into` threaded
through `kayfabe-isolate-host/src/rm.rs`. Nothing in the capability rebuild touches that path. If
anything it is marginally *dearer*: `TABLES` now has eight rows rather than six, so the new field
costs two more lines.

## Q12 — a cross-process **"verb parked"** edge (the fifth flake)

`abandon_releases_a_wedged_requester_with_wedged` flakes at ~0.5% (2/400 before *and* after the flake
campaign — untouched by it). Its precondition is `recv_timeout(25ms).is_err()`: **a sleep used as a
progress edge** for a park that happens in a **child process**, where *"no reply yet"* is not the same
as *"past the VAS allocation"*.

**Deliberately not fixed**, and the reasoning is the ask: lengthening the sleep moves the rate without
killing the class, and loosening the `== 1` assertion would delete the §7.5 contract it exists to
protect. A correct fix needs a **real cross-process parked-verb edge** — a protocol change. That is a
decision about the isolate protocol, so it is yours.

---

★ **Method note, since it is the reusable part.** All three came from *executing* a claim the repo had
only asserted. The pattern that produced them: pick the claim that is cheapest to falsify, run it
against the case most likely to break it — **not** the easiest case — and report refutation as the
valuable outcome. Two of the three were invisible while only one generation and one driver version
existed, and no amount of testing the existing one could have surfaced them.
