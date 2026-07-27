# Testing doctrine — the rules this suite earned by being wrong

**What this file is.** The handful of rules about *how to write a test here* that this project
did not know at the start and paid to learn. Each one is a generalisation of a specific
incident, and each incident is cited so the rule can be attacked rather than obeyed.

**What it is not.** It is not the test *inventory* (that is `core_completeness_gate.md`), not
the mutation gate's method or numbers (`core_mutation_gate.md`), and not the C-quirk
regression list (`c_bug_regression_matrix.md`). The green-gate discipline — iterate until
green, no merge on red, tests must be mean — is in `../../CLAUDE.md` and is unchanged.

The unifying stance, from `l1_concurrency.md` §8.4: **pass = the design survived contact;
fail = the doc changes, not the assert.**

---

## 1. ★★ A green instrument on an unexercised path is worse than no instrument

It reads as *evidence*. Nobody re-checks a zero.

**The incident.** The conservation ledger's first census over the mean run reported
**0 leaked objects** — and that was a **true negative**, not a pass. `l1_os_shell.md` §7.6 T0
claimed `ctl_workload` "exercises T0 thousands of times per run"; the churn it actually drives
adds and removes *RPC* bindings (`Binding::host == None`), which own nothing host-side, and
the script's two real subset-frees targeted channels phase 0 deliberately leaves **virgin**.
**The path had never been reached.** Reaching it took a new `t0_churn` phase. Once reached,
the honest baseline was 24 objects / 6 mappings / 24 576 GPA bytes leaking — linear in frees.
(`l1_concurrency.md` §12.32, `l1_os_shell.md` §14.1.)

**The rules.**

1. **A reclamation test that does not first allocate proves nothing.** State what the
   instrument must have *seen*, and assert that too.
2. **Every instrument needs a non-vacuity assertion** — a case where it is known to read
   non-zero. A ledger that can only ever report `0` is a constant function.
3. **Bite-check the instrument, not only the fix.** Reverting the fix must move the number;
   if it does not, the number is not measuring the fix.

---

## 2. ★★ Assert the exact thing — never `is_err()`, never an absence

**Two canaries have now passed for the wrong reason**, which is the whole argument:

- The retired-proc R5 canary passed on `Rm(Other)` — an RM error from a verb held across a
  retire — while the property under test was `Stale::Proc`. The re-validation it existed to
  guard did not exist, and the canary was green (`l1_concurrency.md` §12.10).
- `the_system_proc_has_no_data_plane` passed via `UnknownPdb` because the system proc owned
  nothing at all; the `SystemDataPlane` refusal it named was **vacuous** until the grouping
  rule gave the system component a `Vas` (`l1_concurrency.md` §12.26 → §12.27).

**The rules.**

1. **Name the variant, including its payload** where the payload is the claim (`Stale::Proc(A)`
   with the *right* `A`).
2. **A refusal test needs a non-vacuity arm**: the same call, with the guarded condition
   removed, must **succeed**. Otherwise the test cannot distinguish "refused for my reason"
   from "never got there".
3. Corollary for near-neighbour faults: when two variants could plausibly report the same
   situation, **assert which, with a comment saying why** — so the day they start reporting as
   each other, a test changes. (`UnknownPdb` vs `Condemned` on a clean last-reference retire is
   pinned this way, `l1_concurrency.md` §12.33.)
4. **★ Assert a BOUND, never an ABSENCE.** An absence is not observable, so a rule phrased as
   one is untestable *and* invisible to the mutation gate. The rule "never busy-poll" was
   wrong on the merits (a short spin beats a syscall; `std::sync` mutexes do it) and
   unfalsifiable — and the gate had already proved it: three spin-versus-park mutants stay
   green precisely because "no polling" is an absence. Restated as **"every poll must be
   provably bounded"**, it has a testable consequence, and it sharpens the reasons for
   forbidding things: a periodic sweep's iteration count is a function of uptime, so it has no
   bound at all. (`l1_concurrency.md` §12.31.)

---

## 3. ★ Isolated cases test what you thought of; composed runs test what you didn't

**The incident.** `worker_death_retires_the_proc_loudly_and_never_resurrects` was green — and
green **only because it never issued an `apply` after the HUP**. The composed mean run put a
worker HUP and an alloc/map-heavy `apply` workload *in the same run*, and found that an
out-of-band retire is undone by the next refresh: a guest able to crash its isolate worker got
a **clean new isolate** on its next RM event (`l1_concurrency.md` §12.13).

The same shape recurred: the leak class in §1 above was found by a composed script within one
run, and the first version of that fix was a **use-after-free of its own making** — visible
only because the run had a publisher parked inside a mapping verb at the moment the drain
fired.

**The rules.**

1. **The composed run is the arbiter**, and both are needed: isolated cases localise, composed
   runs discover. Neither replaces the other.
2. When a composed run finds a defect, **add the isolated case too** — and check whether the
   isolated case would have been green. If it would, say so in the test's doc comment; that
   sentence is the reason the composed run exists.
3. **Assert progress as an edge, never a clock.** A wall-clock parallelism assertion passes
   because the box was fast (`l1_concurrency.md` §8.3, §8.4).

### 3.1 ★★ OWNER DIRECTIVE (2026-07-27) — the bar for "done" on every milestone

> *"we need enough testing and integration, especially with new milestones added, that try to
> simulate real behavior using mocks. and also mean not happy path. Iterate until green."*

This is a **standing requirement per milestone**, not a one-off. Four obligations:

1. **Integration, not unit.** Drive a realistic RM event stream through the mocks **end to end**
   and assert the observable end-state — the conservation ledger, the projection, the routing. A
   test that pokes one function directly proves far less than one that replays realistic traffic
   and then checks what the guest would actually observe. The mocks exist to make realistic
   multi-process × multi-thread × multi-GPU traffic cheap; use them that way.
2. **★ MEAN, not happy path.** The happy path is the least interesting thing the code does.
   Compose the nasty cases deliberately: interleave the case under test with other live procs
   doing real work; free mid-flight; race a re-declaration against a still-publishing generation;
   drive two GPUs at once; kill a worker partway; recycle handles under concurrent traffic; run
   **both** lock modes.
3. **Wire it into the mean test** (`tests/tests/l1_mean.rs`), not only a fresh isolated file.
   That test exists precisely because §3's incident showed isolated cases go green for the wrong
   reason.
4. **★★ Iterate until green — and NEVER narrow a test to make it pass.** A failing mean test
   **is** the finding; that is the whole point of writing it mean. Fix the code. If the test's
   expectation was genuinely wrong, say so explicitly and justify it with a **citation**, rather
   than quietly relaxing an assertion. Silently weakening a test converts a discovery into a
   permanent blind spot, and it is indistinguishable from progress in the diff.

**Why this bar, measured rather than asserted.** The 2026-07-26 identity round
(`l1_concurrency.md` §12.41) landed 7 bite-checks. **None of the 363 pre-existing tests caught
any of them.** Every real defect that day — the undeclared-namespace squat, the live-path
component split, the orphan misclassification, the oldest-wins scans — came from a composed run
or a newly-sharpened criterion. The happy path found nothing, all day.

---

## 4. ★★ Per-object reclamation must never race an in-flight verb — and no lock can prevent it

Stated here rather than only in the design docs because it is a rule about what a *reclamation
test* must establish before it can claim anything.

> **Verbs run lock-free by construction (R1), so no lock can exclude one. Only the isolate's
> own quiesce predicate can.**

**The incident.** The first G2 fix drained the pending-release queue on **every** checkout. It
freed a host VAS underneath a publisher parked *inside* its mapping verb → `RmError::BadHandle`
surfaced to the guest as an anonymous host error rather than as staleness. The guard is
`Isolate::is_quiesced`, read before our own checkout, indivisibly — the same predicate the reap
uses for the same reason (`l1_concurrency.md` §12.32, §12.16 G3).

**The rules.** Every reclamation trigger inherits this, so every reclamation *test* must
(a) compose the release against a **parked** verb on the same isolate, and (b) assert the
absence of the disposal verbs by name, not merely the absence of a crash. The consequence for
the design — the drain is *lazier* than the doc described, and forced to be —
is `l1_os_shell.md` §14.1 residue 2.

---

## 5. The mutation gate is a measurement, and it can be wrong upward

Full method, numbers and thresholds: **`core_mutation_gate.md`**. Two operational rules are
repeated here only because dropping either produces a *plausible-looking* wrong number, which
is the failure mode this doctrine is about:

- **`CARGO_INCREMENTAL=0`, always.** rustc ICEs on the churned incremental cache; cargo-mutants
  cannot tell a compiler crash from a type error and files the mutant **unviable**, silently
  removing it from the denominator. **136 of 293 builds ICE'd**, turning a real **88.95%** into
  a reported **67%**. Sanity check any campaign with
  `grep -rl "thread 'rustc'" mutants.out/log/ | wc -l` → must be 0.
- **`--test-workspace true`, always.** The crates under test have no unit tests of their own;
  without it cargo-mutants runs an empty suite and reports everything MISSED.

Generalised: **a gate that can be wrong upward is worse than no gate.** CI now fails on any ICE
in a mutant log and on a zero viable count, *before* the threshold is read — the score is not
trusted until the measurement is proven to have happened.

Two related rules from the same campaign:

- **A hollow test to move a number is worse than an honest gap.** Survivors that only affect
  panic-message text or have no production caller are documented as such, not chased
  (decision #15).
- **A threshold is set from a measurement and never lowered to clear a red night.** It is set
  *below* the measurement by the observed run-to-run churn, because a gate that cries wolf gets
  muted — and being muted is exactly how ~9 100 lines of L1 code reached `master` with no
  mutation run at all.

---

## 6. ★ Documentation drifts optimistic — treat it as a named risk class

Across three independent passes over this repo, roughly **twenty** instances were found of
documentation asserting more than the code did, **several stating the literal opposite of
their own code**. Examples, all real and all fixed in place:

- `RmGraph::gpu_of`'s rustdoc still described a default-to-GPU-0 guess *inside the one resolver
  whose entire discipline is MISS = no-guess* — false since §12.21.
- `sync_rpc_mappings` claimed *"MISS=FAULT is preserved — never a silent skip"* while its body
  correctly `continue`d on both arms. The body was right; the sentence was wrong about the
  most-cited exception in the codebase.
- `HostHandle`'s doc said it was "scoped to ONE isolate's handle namespace" — and nothing read
  that sentence, because it was prose (`l1_concurrency.md` §12.26).
- §6.2 of `l1_os_shell.md` justified a correct conclusion with a cost model wrong by orders of
  magnitude. **A wrong premise supporting a right conclusion is the hardest kind to catch.**

**Every one of them was true when written.** That is the point: this is not carelessness, it is
entropy, and it is why the project's answer is *gates* rather than *intentions* — the CI
boundary grep, the unsafe-surface `ls` gate, the mutation threshold, the conservation ledger.

**The rules.**

1. **Prefer a mechanism to a sentence.** A rule stated only in prose has no reader. `HostHandle`
   carrying its `IsolateId` is the same rule with a compiler behind it.
2. **When a doc claim is load-bearing for a mechanism, it needs a behavioural check** — not a
   citation to someone else's comment. Two claims cited from the C's own comments were false
   (`../reference/mode2_bench_lifecycle.md` §2, §3); citing a comment is still citing a belief.
3. **Correct in place, strike visibly.** The house style is `~~struck~~` plus the correction and
   its evidence, not a silent rewrite — so a reader who remembers the old claim learns that it
   was wrong rather than doubting their memory.

### 6.1 ★★ CITATION CONVENTION — cite OUR tree by symbol, PINNED trees by `file:line`

A staleness audit (`d379cc0`, 2026-07-27) resolved every `file:line` in the design docs against
the tree and found **~64 drifted**. Its verdict is the whole reason this section exists:

> *"Essentially none of them had a wrong FACT attached. **The pins were honest and useless,
> which is the worst combination, because they read as verified.** Symbols are stable; line
> numbers are not."*

Scale, so the failure is not read as bad luck: `l1_architecture_summary.md` had **24 of 30**
sampled citations drifted, and `crates/kayfabe-core/src/gpu.rs` grew 1,100 → 2,790 lines in
about fifty commits, which is enough to move every anchor in a file on its own.

**The rule, in two halves.**

1. **Our own `crates/` and `tests/` — cite by SYMBOL.**
   `crates/kayfabe-core/src/gpu.rs::Spine::plan_refresh`,
   `tests/tests/l1_mean.rs::a_published_gpa_is_provably_its_own_procs_ram_under_mean_arena_churn`.
   A symbol name is `grep`-able and survives every edit that does not delete the thing you meant;
   a line number survives nothing. Include the crate-relative path so the symbol is unambiguous.
   For a `//!` module doc or a free-standing macro invocation with no enclosing item, name the
   file plus the heading or the sentence — those are greppable too.

2. **Pinned external trees — cite by `file:line`, and do NOT "fix" them.** `ogkm`,
   `research_clones/`, `gvisor/`, the QEMU / cloud-hypervisor / rust-vmm sources, the Linux
   kernel, and the C artifact (the `C:` prefix). We do not edit those trees, so their line
   numbers are **stable and are the correct citation form** — the same audit re-resolved **>100**
   of them and they still landed. Converting them to symbols would lose the precision (a line
   inside a 900-line C function) and gain nothing.

**Exceptions, both narrow.**

- **A line inside a long function that a symbol cannot name** — keep it, but *anchor it to the
  symbol as well*, in this shape: `crates/kayfabe-fwd/src/lib.rs::some_fn` (the catch-all `_ =>`
  arm, ~`:1914`). The symbol is the durable half; the line is the hint, and a reader who finds
  the line has moved can still find the arm.
- **A verbatim runtime artifact** — a captured panic message, a log line — is quoted as it was
  emitted, line number included, because editing it would falsify the quote. Add the symbol
  beside it.

**What NOT to do about a stale pin.** Do not add a banner saying the pins have expired: a banner
rots exactly like the pin it warns about, and the reader who follows the citation is not the
reader who read the banner. **Re-resolve and re-pin by symbol.** Keep the original pinned line in
a trailing parenthetical wherever it is archaeology worth preserving — that costs nothing and
destroys no record.

★ **And re-resolving is itself an audit.** Every conversion forces you to open the code the claim
rests on. That is where the value is: the 2026-07-27 pass found four claims that had gone false
(two "still-open" findings that were fixed, one superseded type decision, one count) *only*
because resolving the symbol meant reading what the symbol now says. A drifted pin is cheap; the
claim hanging off it is not.

---

## 7. ★★ An optimisation with a correct-but-slow fallback must have the fallback tested as a FIRST-CLASS mode

From the owner, about the passthrough-versus-trap decision, and it generalises far past that
feature:

> **Any optimisation that has a correct-but-slow fallback must have that fallback tested as a
> FIRST-CLASS mode, not as a fallback nobody exercises.**

The failure this prevents is specific and it is not "the slow path has a bug". It is that the
slow path is the path the system takes **when something has already gone wrong** — the fast
path bailed, a capability was unavailable, a precondition did not hold — and that is the worst
possible moment to be running code whose only coverage was the day it was written. A fallback
exercised solely by the fast path declining is a fallback whose test coverage is a function of
how often the fast path fails, which is exactly the quantity the optimisation exists to drive
to zero.

**★ And the sharper half: randomised, irregular toggling tests the TRANSITIONS.** Running the
two modes as two fixed configurations proves each is self-consistent. It does not touch the
handoff, and the handoff is where slip-through bugs live: a party that acquired its guarantee
under one mode and acts on it after the switch. §6.8.1 of `l1_os_shell.md` is the worked
example — the RW-lock variant's reader/writer slip-through happens *only* at the disarm edge,
and no amount of steady-state running in either mode can produce it. So the stronger test is a
seeded, irregular flip between modes **while work is in flight**, with the same end-state
assertions as either fixed run. Irregular rather than periodic, for §2's reason: a regular
period is a clock, and a test that passes because of a timing coincidence passes for the wrong
reason.

**The precedent is this repo's own, and citing it is the argument.** `LockMode::{Degenerate,
Sharded}` ships and is tested in **both** configurations from day one, specifically so that a
late granularity flip is never the untested mode (review item P5; `l1_architecture_summary.md`,
*"Both lock modes ship, and are tested, from day one"*; `l1_os_shell.md` §5.2 item 3, which
argues the page-size axis **from** that precedent). The differential test asserts a
bit-identical end state and an operation-by-operation identical log across the two. The host
page size (`[4 KiB, 16 KiB, 64 KiB]`) is the same move on a second axis, and was argued from
the `LockMode` precedent rather than from first principles.

**The rules.**

1. **Name the fallback as a mode**, with a way to select it that the suite uses — not a code
   path reachable only by inducing the failure that triggers it.
2. **Run both, and assert they agree** on everything the optimisation is not allowed to
   change (end state, refusal variants, the operation log where one exists).
3. **Then toggle between them, irregularly, under load**, and assert the same things. This is
   the arm that finds transition bugs, and it is the arm that a two-fixed-modes suite reads as
   already covering.

---

## See also

- `core_mutation_gate.md` — does the suite notice when the core lies? Method, score, thresholds.
- `core_completeness_gate.md` — is every claimed behaviour actually exercised?
- `c_bug_regression_matrix.md` — every C-era bug, classified impossible / tested / deferred.
- `l1_concurrency.md` §12 and `l1_os_shell.md` §14 — the contact logs these rules were mined from.
- `../reference/` — measured NVIDIA/RM and bench-lifecycle facts, the sources these tests model.

---

## 8. ★★ The C's OMISSIONS are evidence, not gaps

**The incident (2026-07-27).** The region-lock design spent a full study evaluating 13 mechanisms
and recommending one, and the owner asked three separate times whether userfaultfd was really the
right answer. Nobody had asked the cheaper question first: **what did the C do?**

The answer, when finally measured (`../reference/uffd_isolate_kvm_study.md` Q3): **the C never
blocked a write at all.** `userfaultfd` has *zero* implementation there — planned four times,
never built, with `REFACTOR_PLAN.md:303` citing a proof-of-concept file that does not exist. Its
"demand-fault" path is **dead code** (`src/stub/nvkvm_stub.c:668-686` calls `stub_exit(139)` and
never returns, so the fault address has been permanently `0` since `3c23db9`). What it actually
ships is a **copy-once snapshot**, audited twice as item **P2-2**, with the reasoning written out
in the source (`src/qemu/virtio_nvgpu.c:626-663`):

> *"a second vCPU can flip an allowed value to a denied one in the window between… Snapshot the
> slot into a worker-private buffer ONCE up front."*

**There is no lock of any kind between QEMU and an isolate** in a system that runs 22 real GPU
apps at host parity.

### The rule

> **We already treat the C as authoritative for what it DOES. Treat what it DOESN'T do as a
> signal of equal weight — and find out WHY before building the thing it skipped.**

A working implementation on real hardware that *lacks* a mechanism we are about to build is
telling us one of three things, and they are all worth knowing **before** the build:

1. **It didn't need it** — the problem dissolves under a different decomposition (here: copy-once
   makes exclusion unnecessary, because a race the guest wins by supplying different bytes is not
   a security problem — only re-reading a validated value is).
2. **It tried and it didn't work** — the most valuable case, and the one most likely to be
   invisible, because failed attempts leave dead code and stale plans rather than documentation.
   Both were present here and both read as "planned, presumably done" until someone grepped.
3. **It has the bug and got away with it** — in which case we know exactly what to fix, and what
   the exposure looked like in practice.

### Why this is not hindsight

The cost asymmetry is stark: answering "did the C do this, and if not why" is **one grep and one
afternoon**. Building uffd would have been a mechanism, a runtime probe, a deployment requirement
(a sysctl or a udev rule), an unresolved arm64 question, and a collision with NVIDIA's own UVM
(`uvm_hmm.c:577-588` rejects any `userfaultfd_armed(vma)`) — all before discovering the incumbent
implementation had routed around the whole problem.

**Corollary for reviewers:** when a design doc proposes a mechanism, the first question is not
"is this the best mechanism?" but **"does the working implementation have one, and what happened
when it tried?"**
