# The claim ledger — what we MEASURED, what we INFERRED, and what we merely ASSUMED

> ### STATUS — 2026-08-11 (w258 doc-hygiene sweep) / **LIVE doctrine, STALE CENSUS — the gate still runs; the numbers below do not**
>
> ★ **The doctrine and the gate are intact.** `scripts/claim_ledger.py` is still run and still
> enforced; nothing below about *what the categories mean* has been superseded.
>
> ⊘ **Every count below is a 2026-08-01 snapshot and all three have moved since.** §2's census
> pins master `0ad4c95` at **UNATTRIBUTED 383 / CONFLATED 66 / BARE-HW 17**, and §4 sets the
> ceiling at **382**. Committed drift, none of it written back here:
>
> | date | commit | reported |
> |---|---|---|
> | 2026-08-07 | `63cc812` | `UNATTRIBUTED_BAR` moved **382 → 381** in `scripts/claim_ledger.py:477` — doc untouched |
> | 2026-08-08 | `b354f46` | *"claim_ledger.py at `c2e2946` reports **UNATTRIBUTED 384 / CONFLATED 67 / BARE-HW 17**"*, flagging `CONFLATED 67 > 66` as a standing red |
> | 2026-08-09 | `9ae1740` | *"claim_ledger unchanged at baseline **446 / 72 / 18**"* |
>
> ⇒ **Read the numbers from `scripts/claim_ledger.py`, never from this file.** ★ A ledger whose
> purpose is to stop unattributed claims is itself the sharpest place for an unattributed number
> to sit: the bars below still *look* like the enforced baseline and have not been for ten days.

> *"tests are one piece, not proof. things like self reflection `SET_OBJECT's data was not
> observed to matter, and the code now says so instead of claiming a measurement it does not
> have` is really important especially for this project. **the only real proof are live boots
> with real work**, the rest is for a) rigorness and b) to help you speed up iteration and
> c) to prevent drift."* — the owner

## 0. The failure mode, stated precisely

In this project, **reading the open kernel modules tells you what the driver *does*. Only a
live boot with real work tells you what *happens*.** A comment that says "measured" when it
means "inferred from source" converts an inference into a fact **with no experiment behind
it**, and everything downstream inherits it as settled.

It has already happened, expensively, at least three times:

1. **`read-at-invalidate` on the compute path.** `CLAUDE.md` repeated a claim *its own design
   doc had already refuted*, and it was restated confidently for weeks. ★ A summary is not a
   source.
2. **The bit-15 GSS-legacy "2^31 unreviewed commands" alarm.** It counted address space, not
   risk; the real question was what property the rule relied on.
3. **`host_execution_plane.md` §1.4's first bullet** — found by this sweep, and it is the
   purest instance. The C's own comment (`C: src/qemu/virtio_nvgpu_pci.c:30-33`) says a
   memslot collision was *"proven by the earlier probe"*; the probe writeup says only *"the
   **likely cause** is a memslot conflict"*. The doc had **already written the correction**
   sixty lines below — *"Correction to §1.4's first bullet — comment ≠ measurement … Third
   instance of the comment-vs-measurement trap"* — and the bullet itself still said "proven".
   A correction that lives downstream of the claim does not stop the claim being quoted.

That third one is why this ledger is a **gate** and not a memo.

## 1. The four classes

| class | what it means | what it must carry |
|---|---|---|
| **MEASURED** | an experiment was run | **name it**: which capture, which bench run, which test, which commit, on which hardware |
| **INFERRED** | derived from reading ogkm / nvproxy / the C artifact | **cite it**, tagged — legitimate and load-bearing, but it is a *reading* |
| **ASSUMED** | neither | **say so.** Not automatically wrong. It must simply not pretend |
| **UNATTRIBUTED** | says one of the three, names none of them | the defect |

★★ **CONFLATED** is a sub-class of INFERRED and it is the one this whole exercise is named
for: a block that says **MEASURED / VERIFIED / PROVEN** whose only evidence is a **citation
into source**. It has read the driver and claimed a boot.

★ **"Reproduced on hardware", not "the mock agreed".** A mock is a model of the thing; a green
test against a mock measures the model. The mock wall in this project is itself measured — 749
green tests and a 99.2 % mutation score, then one honest mock killed twelve of them.

## 2. The census (master `0ad4c95`, produced by `scripts/claim_ledger.py`)

```
MEASURED         236
INFERRED         354
ASSUMED           32
UNATTRIBUTED     383
TOTAL           1005
  of which
  CONFLATED       66   strong claim word, evidence is a READING only
  BARE-HW         17   says "on hardware" and names no run at all
```

★★ **The gate fired four times before it landed, three of them on other people's merges,
and two of those four were its own false positives.** That hit rate is stated because it is
the honest one, and because both misses were **pattern defects fixed in the pattern**, never
absorbed into a bar. The bars were pinned against `4c1bf29` at 383 / 65.

- Rebasing onto `5c4cb0d` (`gsp-boot-spec`, `a5d63f8`) turned both red: **five** new
  unattributed sites and **one** conflated, across 1 094 lines the gate was not written
  against — including a section headed *"the isolation verdict this file rests on —
  measured, not estimated"* that names no run. Itemised beside the bars in the script and
  left on that author's floor rather than absorbed.
- Rebasing onto `554c333` (arch axis) fired again — and that one was a **false positive**.
  `kayfabe-chips/src/ad10x.rs:104` cites ogkm for a number and then says *"No Ada card was
  measured for this number."*: the exact honesty this gate is for, scored as the defect it
  is for, because the pattern knew *"not measured"* and not *"no X was measured"*. ⊘ The
  form was added to the honest-downgrade pattern; the bar was **not** raised. Absorbing it
  would have been the cheaper edit and the wrong one — a gate that penalises the behaviour
  it exists to encourage teaches people to stop writing it down.
- Rebasing onto `0ad4c95` fired a fourth time, also a false positive:
  `kayfabe-qemu-raw/src/shim_unsafe.rs` writes `// SAFETY: both pointers were just proven
  non-null`. **A local proof is not an empirical claim** — it is a deduction about the
  three lines above it, with no machine and no source involved, and demanding a citation
  would put noise in the one place (`unsafe` blocks) where noise is most expensive. Those
  phrases are now stripped before a block is examined, which *tightened* the ceiling
  389 → 383 (fifteen blocks turned out to hold local proofs and nothing else) while
  CONFLATED and BARE-HW did not move.

Read that table with §5's limits in hand. It is a census of **how claims are written**, not an
audit of whether they are true — this instrument cannot tell a real capture from an invented
one, and the 20 phantom citations the ogkm gate turned up were found by a human reading, not
by a grep.

★ Note where MEASURED concentrates: **126 of 225 are in `docs/design/`**, against 19 in
`kayfabe-abi` and 10 in `kayfabe-isolate-host`. The evidence lives in prose and the code
points at it. That is a workable arrangement and it is also exactly the arrangement in which a
design doc's correction fails to reach the code that quotes it — instance 3 above.

## 3. The ranked misclassifications — claims that assert a measurement they do not have

Ranked by what it would cost to have been wrong. Fixed items are marked ✎; the rest are
recorded here and left for the file's current owner (§7).

1. ✎ **`kayfabe-rmrpc/src/lib.rs`** — *"the hole that is now **measured** rather than
   suspected"*. The hole is real and the finding is load-bearing: `SET_PAGE_DIRECTORY` reaches
   the wire **only** for a `SHARED_MANAGEMENT` / `IS_EXTERNALLY_OWNED` VASpace, so this arm is
   *necessary and not sufficient*. The evidence is **one assert in `gpu_vaspace.c:3109`**.
   Nothing was booted. "Not sufficient" is precisely the class of conclusion a live boot can
   refute and a source read cannot, which is what makes the word matter here.
   → now reads *"the hole that is now READ rather than suspected"*, citation untouched.
2. ✎ **`kayfabe-fwd/src/ptdecode.rs`** — *"`#102` stage C2 measured that there is no
   read-at-invalidate on this path, **in this port or in the C**"*. One sentence spanning two
   epistemic states: in the C it is a genuine hardware measurement (both invalidate transports
   counted **zero** on the GSP-emulated compute path); in this port it is that result carried
   across, never re-run. This is the same claim, in the same words, whose conflation cost weeks.
   → split into the two halves, with the design consequence left unchanged and unsoftened.
3. ✎ **`docs/design/host_execution_plane.md` §1.4** — the bullet described in §0.3. The
   correction existed; the claim did not carry it.
   → bullet downgraded **in place** and pointed at its own correction. Left, not deleted:
   the instruction ("read that fix before implementing") is still right — the borrowed word
   was "proven".
4. ✎ **`kayfabe-abi/src/bringup.rs`** — *"Measured against `C: src/abi/nvgpu.h:243-256`"*. You
   cannot measure against a header. Two independent readings agree on offset +48, which is
   worth stating and is not a run; the stdin consequence (`fd` of 0) is reasoned, not watched.
   → restated as two readings, with the unwatched consequence named as unwatched.
5. ✎ **`kayfabe-arch/src/lib.rs`** — *"the **measured** regime's big-page table holds 32
   entries"*. The 32 is read from the C's table beside its decoder. The over-read arithmetic
   holds regardless; what a reading does not establish is **that the hardware in front of us
   is in that regime** — and that is the load-bearing half.
6. **`kayfabe-isolate-host/src/rm.rs:1485`** — *"THE ROUTING RULE, **measured against the
   driver**"*, evidenced by `ogkm-580: escape.c:328`, then: *"which is exactly how this was
   found, on the first real-hardware run"*. This one probably **is** measured — the run is
   simply not named, so it is indistinguishable from one that is not. Deferred: naming the run
   needs the person who made it. This is the largest residual class in the ledger and the
   cheapest to fix at the keyboard where the run happened.
7. **The C-INHERITED class, ~25 of the 65** — *"the C's **proven** host channel"*, *"the C's
   **measured** exhaustion (`C: nvkvm_mmap_host.c:382-389`)"*, *"the C's **measured**
   `engineType = 0` bug"*. These cite the **C artifact's own comments** as the record of a
   measurement. The C is the standing oracle and it did boot, so this is the best evidence
   available for much of the port — but it is exactly the artifact whose epistemic hygiene
   §0.3 just found a hole in. `kayfabe-vmm-kvm/src/slotnum.rs` is the model of how to do it
   well: it says *"The C's exhaustion is a datum, not a prediction"* and argues why the datum
   transfers. Most of the other 24 assert the transfer silently.

★ **Nothing in this list was deleted or softened into vagueness.** Each is restated at the
epistemic level actually held, and kept just as specific. *"Not observed to matter"* is more
useful than a false *"measured"*, and much more useful than silence.

## 4. The gate

`scripts/claim_ledger.py`, run by CI as its own step.

★ **It extends the mechanism `#74` proved rather than inventing a second one.** The
ogkm-version-tag gate is *"every NVIDIA citation names its tree"*, adjudicated across 299
citations and found to discriminate — **20 phantoms**. This gate is the next question about
the same sentence: *the citation names its tree; does the VERB name what actually happened?*
It reuses that gate's shape (Gate A + a ratchet, with a failure message that tells you the
legitimate ways out), and it reuses **this repo's own marker vocabulary** — `[src]`,
`[src@580]`, `[src@610]`, `[unverified]` — rather than asking anyone to learn a second one.

★★ It preserves the distinction that matters: **cited-to-source is not measured-on-hardware.**
A citation is *attribution*, and attribution is what the ogkm gate enforces. This gate splits
attribution into its two kinds and refuses to let one stand in for the other. Concretely: a
pointer at another design doc can never promote a block to MEASURED, no matter how specific
the section number — because that is precisely the move `CLAUDE.md` made.

**Three rules, deliberately different strengths** (each argued at length in the script):

- **BARE-HW — exact, and the strongest.** 17. A block that says *measured / verified /
  proven **on hardware*** and names no run at all. Nothing else can be meant by the phrase —
  it asserts a machine was switched on — so a source citation elsewhere in the block buys
  **no exemption**. ★ This rule does not use the paragraph, for the reason in §6. It is the
  only bar here that can reach zero, and ~6 of the 17 are the *legend lines that define the
  notation*; the other 11 are one-line fixes by whoever made the run.
- **CONFLATED — exact.** 65, both directions pinned. Small enough that whoever moves it can
  read the whole list; a growth-only check would leave slack, which is the argument the ogkm
  doc ratchet makes about itself.
- **UNATTRIBUTED — a ceiling.** 382 (was 383; tightened on the reachability-on-transition
  branch, and the script's own comment names the site that left). This is the weakest of the
  three and it is weaker on purpose: it is a census of pre-existing prose across a tree
  several agents write into at once, and an exact pin would turn unrelated doc commits red,
  whose outcome is not better prose but people bumping the number without reading it. The
  gate prints the exact line to change whenever the true count drops below the bar, so
  tightening stays one edit — and the tightening is expected to happen in the commit that
  created the slack, not later.

## 5. ★★ What this gate CANNOT see

Asked before it was written, and answered honestly, because a gate whose blind spots are
undocumented is read as covering them.

1. **It checks that a claim NAMES evidence — never that the evidence exists, says what the
   claim says, or was ever run.** A block citing `cap9_imaginary` passes. Phantom-hunting is
   human work.
2. **`observed` is not in the claim vocabulary, and that is a finding rather than a
   convenience.** It was in the first draft: 284 Rust hits, and reading them showed the
   overwhelming majority are the **domain verb** — *"a completion was observed for this
   proc"*, *"last doorbell token observed"*. In a project whose entire mechanism is observing
   a guest, `observed` describes the running program far more often than it describes us.
   Keeping it would have buried the signal under several hundred untriageable sites. ⇒ **the
   `SET_OBJECT`-shaped phrase the owner quoted is itself outside what this gate can see**, and
   is left to review. Stating that is the point of this section.
3. **A claim made without a claim word is invisible.** *"The driver writes 0x4 here"* asserts
   just as much and matches nothing. This gate raises the cost of the confident-sounding
   claim; it does not reach the quiet one.
4. **Code is out of scope.** An `assert_eq!` encoding an unmeasured belief is invisible here
   and always will be. This is the largest hole and it has no cheap closure.
5. **MEASURED-vs-INFERRED is decided by which tokens a block carries**, so a block that cites
   ogkm *and* names a date scores MEASURED even if the date belongs to the reading. Precision
   over recall.
6. **`[measured]`, the repo's existing marker, is not treated as evidence** — a marker
   declares a class, it does not name a run. This is the one place the ledger disagrees with
   an existing convention, and it disagrees in the strict direction.

## 6. ★★★ Proving the gate fires — and the first bite that DIDN'T

A gate never seen to fail is not evidence, so a claim was planted: a rustdoc line in
`crates/kayfabe-abi/src/bringup.rs` reading

> *"PLANTED BITE — to be removed. Measured on real hardware: this offset is stable across
> every driver revision and the frontend was proven never to reject it."*

with nothing whatsoever behind it.

**It did not fire.** Both ratchets stayed at 383 and 65, and the runner exited **0**.

The cause is structural and it is the most useful thing this exercise produced. **The unit
of the first two rules is the BLOCK**, and the bite was planted into a paragraph that
already carried a `C:` citation — for a *different* sub-claim. The new claim inherited that
paragraph's attribution. The gate could not see it at all.

The block-level design is not a mistake: a line-level rule flags every rustdoc paragraph
that says "measured" on one line and names its capture three lines later, and gets ignored
within a day. But it means **a paragraph is a place a claim can hide**, and that had to be
answered rather than noted.

The answer is the BARE-HW rule, which does not use the paragraph: the phrase *"measured on
hardware"* asserts one thing only, so the block must carry a run token and no citation
elsewhere in it buys an exemption. Re-run with the same bite still planted, the gate
printed `bare hardware claims: 18 (bar: 17)` and exited **1**. The bite was then removed
and the gate returned to 17 / exit 0.

★ Two lessons, and the second is the one to keep: (1) ask what the gate can actually SEE,
before trusting a green; (2) **the bite that fails to fire is worth more than the bite that
fires** — it is the only thing that finds the blind spot.

## 7. Deferred, and why

Files under active concurrent authorship were **not** edited; their findings are recorded
above and in `--list CONFLATED` output instead:

- `crates/kayfabe-qemu-raw/`, `qemu/hw/misc/nvkvm/nvkvm.c` — register dispatch in flight.
- `crates/kayfabe-gsp/` — boot-check analysis in flight (`boot.rs:257` *"the signature the
  bench measured"* is unattributed and is on that agent's floor).
- `tests/**` — flake-family work in flight. 65 unattributed sites and several conflated ones
  (`tests/tests/cancellation.rs:1`, *"the two measured facts everything here is built on"*)
  are left alone.
- `docs/design/gsp_boot_gate_spec.md` + `crates/kayfabe-crec/tests/gsp_boot_gates.rs` — the
  six sites in §2, merged while this was being written. A spike **was** run; naming it is one
  line each, and only that author can name it.

## 8. How to use it

```
scripts/claim_ledger.py                  # the census
scripts/claim_ledger.py --list CONFLATED # the ranked-fix list of §3
scripts/claim_ledger.py --list BARE-HW   # the 17, the ones to drive to zero
scripts/claim_ledger.py --gate           # what CI runs
```

When it fires, there are always **four** legitimate answers and only one of them is "fix the
code": name the run, cite the source, **say that you have neither**, or lower the bar because
you removed one. The third is not a defeat. It is the `SET_OBJECT` line, and it is worth more
than a borrowed measurement.
