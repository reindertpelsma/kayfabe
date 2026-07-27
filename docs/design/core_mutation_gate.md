# Core mutation-testing gate — does the suite NOTICE when the core lies? (decision #28)

**Status:** empirical gate, 2026-07-24, run against head `0ba300b` (post-#27 whole-core
determinism differential). Tool: `cargo-mutants 27.1.0`, stable toolchain, `-j 2`,
`--baseline skip` (suite confirmed green first), `--timeout 240`, full-workspace test
harness per mutant. Survivor-killing tests added on top; the workspace suite is green at
**132 tests** (was 113) with `cargo clippy --workspace --all-targets` clean and the core
still `#![forbid(unsafe_code)]` with no new runtime deps.

> ### ★★★ 2026-07-27 — THE SCOPE CHANGED, AND CI NOW SAYS **DO NOT QUOTE THE SCORE**
>
> Everything below describes campaigns run over a **four-hand-picked-path** scope. That scope
> was replaced in `0b78102 ci: mutation scope = EVERY production crate — the gate was measuring
> 41% of the code`. `.github/workflows/ci.yml:532-534` now runs
> `cargo mutants … -f 'crates/*/src/**/*.rs'`, and `:522-527` gives the reason in CI's own words:
>
> > *"★★ SCOPE = EVERY PRODUCTION CRATE. It used to be four hand-picked paths, which by
> > 2026-07-27 covered only ~17k of 41.6k implementation lines — so the threshold described the
> > OLD core while every real defect that week lived in the NEW OS layer … A gate that measures
> > a shrinking minority reports a number about code nobody is changing."*
>
> **★★ And the threshold carries an explicit embargo** (`ci.yml:579-584`):
>
> > *"⚠ PENDING RE-DERIVATION (2026-07-27). 91 was measured against the OLD four-path scope … so
> > this number describes a different population and MUST be re-derived from the first full run
> > under the new scope before it is trusted as a bar. … **but do not quote the score until it
> > is re-derived.**"*
>
> **What this means for every number in this file, and for the docs that cite them:**
>
> | figure | standing as of 2026-07-27 |
> |---|---|
> | **L1 92.44%** and the **91% CI floor** | **embargoed by CI itself.** Still cited as settled by `README.md`, `ARCHITECTURE.md`, `l1_architecture_summary.md` §3.9 and §7, and by `core_state_and_consolidation.md`. Those are outside this audit's edit scope and are **reported, not fixed**. |
> | **L0 99.2% (245/247)** | **not embargoed, but measured at `0ba300b` against a `kayfabe-core` of ~2,500 lines. That crate is now 8,411 lines.** The score is true of the population it was run on and says nothing about the 70% of the crate that did not exist yet. |
> | the **Scope** and **Reproduce** blocks in §L1 below | describe the retired four-path invocation. Left standing as the record of what was measured; **they are no longer what CI runs.** |
>
> ★ **The failure shape here is worth naming, because it is not drift.** No number below was
> ever wrong and no citation broke. The *population* moved out from under them — and a score is
> only meaningful relative to a scope, which is exactly the thing docs quote without. This is
> `testing_doctrine.md` §1's *"a green instrument on an unexercised path"* in its numeric form:
> a 91% floor over 41% of the code is a green instrument, and it stayed green while the defects
> that week landed in the other 59%.

**2026-07-25 addendum (standing gate, not a manual campaign):** this gate is now CI's,
not a human's memory — the weekly `mutants` job in `.github/workflows/ci.yml`
(`schedule:` + `workflow_dispatch:`) runs `cargo mutants` over the ~~**L1 surface**:
`kayfabe-rt`, `kayfabe-core/src/reactor.rs`, and the plan/execute/commit + isolate
pool/condemnation code in `kayfabe-fwd`/`kayfabe-isolate`~~ **— superseded 2026-07-27, see
the banner above: the scope is now every production crate**. The prior framing — "run a
campaign when it feels due" — is exactly what let ~9,100 lines of L1 land on `master`
with no mutation run at all; a gate that is remembered is not standing. **The L1
baseline is now measured and the job carries a hard threshold** — see
[§L1 baseline](#l1-baseline-2026-07-25) below, which also records the two harness
defects that first campaign's number was hiding. The L0 numbers below are the settled
baseline for the pure core.

**2026-07-25 addendum (triage audit):** a re-audit of the residual survivors found the two
`handle_doorbell` guard mutants previously called *equivalent* are in fact **real gaps** —
the state their equivalence proof assumed unreachable (`chan.vas == None` with a live host
channel) IS reachable by freeing the VASpace handle after materialization. Both are now
killed (§fwd below). Every gap-killing test in this doc was **verified per-mutant** by
hand-applying the exact `cargo-mutants` mutation to the source and confirming the named
test flips from pass → fail (and the fwd/mmu numbers come from a full scoped re-run of
those two crates *with the new tests in place*). Final score: **245/247 = 99.2% killed**,
residual = 1 proven-equivalent + 1 documented-acceptable.

**See also:** `testing_doctrine.md` — the cross-cutting rules for writing a test that means
something here (non-vacuity, exact-variant assertions, composed-vs-isolated runs, and why a
gate that can be wrong *upward* is worse than no gate). The two operational rules this file
proves the hard way — `CARGO_INCREMENTAL=0` and `--test-workspace true` — are summarised there
with their consequences; the method and the numbers stay here.

## Why a number, not an opinion

The core already had 113 tests — order-independence, proptest fuzz (A1–A4), the security
boundary/invariant suites (I1–I4 + confused-deputy), coverage-guided libfuzzer on the
decoder, the whole-core determinism differential, concurrency stress, and the C-bug
regression matrix. **Breadth is not meaningfulness.** `cargo-mutants` mutates the core's
own logic and measures whether the suite catches the change. A *surviving* mutant is a
change to core logic that ALL tests still pass through = a genuine, objective test gap.
This gate answers the owner's standing "do we have enough tests?" with a measured score
and a triage of every survivor.

## Scope & method (and the honest caveat)

Mutated the pure-core crates only: `kayfabe-core`, `kayfabe-mmu`, `kayfabe-fwd`,
`kayfabe-completion`, `kayfabe-arch` (mocks / `fuzz/` / `tests/` harness excluded — mutating
test/mock code is noise). Each mutant is scored by running the **entire workspace test
suite** against it (no subset — a mutant killed only by a heavy proptest still counts).

**Sharding (honest):** the workspace suite is ~100s/run and the `pushbuffer_parser`
proptest alone is ~77s, so mutants × full-suite is long. It was run in two passes:
**Pass 1** ran all five crates and fully completed `kayfabe-arch`, `kayfabe-completion`, and
`kayfabe-core` (its `kayfabe-fwd`/`kayfabe-mmu` slice was stopped mid-tail and is superseded).
**Pass 2** then ran `kayfabe-fwd` + `kayfabe-mmu` to completion **WITH the new
survivor-killing tests already in place**. So every crate is covered end-to-end, and the
fwd/mmu numbers already reflect the fixes. Every survivor was **triaged**, and every real
gap was killed and **independently verified** by hand-applying the exact mutation and
observing a targeted test fail (recorded per row). Only the *ordering* was sharded, not
the rigor or the coverage.

## The score

Viable = caught + survived (unviable = won't compile, e.g. `-> Default::default()` on a
non-`Default` return, or `Ok((Default, N))` on a non-`Default` tuple — these are tool
artifacts, not gaps; timeout = detected via a hang, counted as killed). **Mutation score =
killed / viable.** Numbers below are the authoritative per-crate finals (Pass 1 for
arch/completion/core, Pass 2 for fwd/mmu).

| Crate | caught | timeout (killed) | survived | unviable | **viable** | **killed** | **score** |
|---|---:|---:|---:|---:|---:|---:|---:|
| `kayfabe-arch` | 2 | 0 | 0 | 1 | 2 | 2 | **100%** |
| `kayfabe-completion` | 41 | 0 | 8→**0**¹ | 5 | 49 | 49 | **100%** |
| `kayfabe-core` | 124 | 2 | 21→**1**² | 32 | 147 | 146 | **99.3%** |
| `kayfabe-mmu` | 6 | 0 | 0 | 8 | 6 | 6 | **100%** |
| `kayfabe-fwd` | 40 | 0 | 3→**1**³ | 16 | 43 | 42 | **97.7%** |
| **total** | **213** | **2** | **2** | **62** | **247** | **245** | **99.2%** |

¹ All 8 completion survivors were **real gaps**, now killed (verified per-row). ² Of 21
core survivors, **20 were real gaps** (now killed); the 1 residual is the **equivalent**
`ClientUnion::union` `< → <=` (proved below). ³ Of 3 fwd survivors, **2 were real gaps**
(the `handle_doorbell` VAS-freed-channel guard pair — an initial "equivalent" call was
disproved by the free-after-materialize sequence and killed); the 1 residual is the
**acceptable** `parse_pushbuffer` cap-tightness mutant whose load-bearing sibling holds the
boundary (explained below).

**Bottom line:** across the mutated core, **both residual survivors are
equivalent-or-acceptable** — every one of the **24 real gaps** the gate found is closed
with an observable-behavior test. The 113-test suite was already strong (213 mutants caught
outright, first try); the gate's value was the **24 gaps breadth alone missed** (and one
model correction: a guard first mis-judged equivalent was proved load-bearing). So: the
tests are meaningful to a mutation score of **99.2% killed / 100% of real gaps closed**;
the residual 2 survivors (`union` `<→<=`, `parse_pushbuffer` cap-tightness) are
equivalent/acceptable for the documented reasons, not untested logic.

## The interesting finding

The richest gap cluster was **`RmGraph`'s parked-fact + free-subtree bookkeeping** — the
order-tolerance machinery (`Dup`/`MapMemoryDma`/`SetPageDir` before their targets) and RM
refcount teardown. Despite an existing fuzz suite over `RmGraph`, the mutation gate found
that **eight distinct predicates** in the parked-map cleanup, subtree cascade, and
dead-VAS mapping teardown were never exercised in a way that pinned their *exact* boolean:
e.g. a parked `Unmap` that must drop **only** the named map, a subtree free that must stay
**namespace-confined** (identical handle values in another client must survive), a dead-VAS
mapping teardown that must fire on `touched && dead` — never `touched || dead` (which would
tear down a live VAS's mappings when a dup keeps it alive), and the `MapMemoryDma` backing
`base + offset` (every prior map used offset 0, so `+` vs `-` was invisible). These are
exactly the confused-deputy / refcount hazards the C paid bench-days for, and the fuzz
suite's *structural* invariants did not localize them — the mutation gate did.

## Survivor triage — every survivor, its class, and how killed / why not

**Legend:** *Real gap* → new test asserts the observable behavior the mutant broke (each
verified by hand-applying the mutation and seeing the test fail). *Equivalent* → no input
distinguishes it from the original; documented, not chased (decision #15: don't add brittle
internal-state pins).

### `kayfabe-completion` (8 survivors → all real gaps, killed)

| Mutant | Class | Killed by |
|---|---|---|
| `outstanding_len` `+ → *` (140) | real | `outstanding_accounting_sums_all_three_queues_and_predicate_is_any` — drives pending+in_flight+awaiting_ack all co-populated; `*` (0-absorbing) diverges from `+`. |
| `has_outstanding` `\|\| → &&` (146) | real | same test — asserts TRUE when only `awaiting_ack` is non-empty (the `&&` mutant reports false). |
| `ack` in_flight retain `!= → ==` (174) | real | `ack_removes_an_in_flight_event_and_spares_its_siblings` — acks an event still in-flight; `==` keeps the acked one and drops its siblings. |
| `FenceArms::observe` `> → >=` (289) | real | `fence_jump_guard_accepts_a_legitimate_large_step` — a step of EXACTLY `MAX_FENCE_JUMP` must be accepted (`>=` rejects at the bound). |
| `DeliveryPlane::batch_outstanding → true`/`false` (335) | real | `batch_outstanding_tracks_the_drain_gate_state` — asserts false on a fresh/drained plane and true after a post. |
| `try_post` `next_batch += 1 → *= 1` (356) | real | `successive_batches_carry_distinct_monotonic_ids` — `*= 1` pins the drain-key BatchId at 0 forever; two posts must get distinct ids. |
| `MAX_FENCE_JUMP` `2 * 1024 → 2 + 1024` (67) | real | `fence_jump_guard_accepts_a_legitimate_large_step` — a legitimate 1500-step must not fault; `2+1024=1026` wrongly rejects it. |

### `kayfabe-core::gpa` (3 survivors → all real gaps, killed)

| Mutant | Class | Killed by |
|---|---|---|
| `GpaArena::alloc` `> → ==` (93) | real | `arena_alloc_boundary_exact_fill_ok_overrun_loud` — a one-shot over-run must fault (`==` only rejects the exact fill). |
| `GpaArena::alloc` `> → >=` (93) | real | same — an EXACT arena fill must SUCCEED (`>=` rejects the perfect fill). |
| `GpaArena::is_untouched → true` (105) | real | `is_untouched_flips_on_first_allocation` — after one alloc it must be false (early-arm merge discipline, L9). |

### `kayfabe-core::gpu` (3 survivors → all real gaps, killed)

| Mutant | Class | Killed by |
|---|---|---|
| `Proc::is_untouched` `&& → \|\|` (247: arena∧channels) | real | `cb14_arena_touch_alone_blocks_a_late_merge` — touches ONLY the arena; a late merge must still be a loud `LateMerge`. |
| `Proc::is_untouched` `&& → \|\|` (248: channels∧vases) | real | `cb14_host_channel_touch_alone_blocks_a_late_merge` — touches ONLY a host channel. |
| `Proc::is_untouched` `&& → \|\|` (251: host_vas∧binding) | real | `cb14_host_vas_touch_alone_blocks_a_late_merge` — touches ONLY a host VAS. |

(The C folded `is_untouched` all-at-once via `publish_backing`, which is why the existing
`cb14` couldn't distinguish the `&&`s — each clause needed isolating. This is the
disagreement-sensitivity a mutation gate exposes that an end-to-end test does not.)

### `kayfabe-core::project` (4 survivors → 3 real gaps killed, 1 equivalent)

| Mutant | Class | Killed by / why equivalent |
|---|---|---|
| `ClientUnion::union` `< → ==` (123) | real | `proc_anchor_is_the_minimum_client_in_its_component` — the anchor is the MINIMUM client handle; `==` (equality returns early) picks the other root → non-min anchor. |
| `ClientUnion::union` `< → >` (123) | real | same test — `>` inverts the min-selection. |
| `ClientUnion::union` `< → <=` (123) | **equivalent** | The `ra == rb` case returns early one line above (line 120), so at the comparison `ra != rb` always holds; `<` and `<=` are identical for every reachable input. Verified: the anchor test still passes under `<=`. Not chased. |
| `project` vChid dedup `!= → ==` (274) | real | `b1_vchid_collision_is_a_loud_contained_projection_fault` — two distinct channels decoding to one vChid must be a loud `VchidCollision` (the exec-plane twin of the existing PDB-collision test); `==` silently accepts the collision. |

### `kayfabe-core::rmgraph` (12 survivors → all real gaps, killed)

| Mutant | Class | Killed by |
|---|---|---|
| `apply` Unmap parked-map cleanup `delete !` (480:29) | real | `parked_map_unmap_drops_only_the_named_map` — an `Unmap` of an unresolved VAS must drop EXACTLY the named parked map, leaving the sibling (same VAS, other VA) intact. |
| `apply` … `&& → \|\|` (480:76) | real | same test — `\|\|` drops every parked map sharing the vaspace. |
| `apply` … `== → !=` (480:65 vaspace) | real | same test. |
| `apply` … `== → !=` (480:84 va) | real | same test. |
| `free_subtree` namespace filter `!= → ==` (519:36) | real | `free_subtree_cascade_is_namespace_confined` — freeing a non-root parent cascades to its OWN namespace's children while an identically-handled object in another client survives. |
| `free_subtree` parent-edge `!= → ==` (528:35) | real | same test — the child must be reached via the parent edge. |
| `free_subtree` dead-VAS mapping `&& → \|\|` (552:60) | real | `free_subtree_keeps_mappings_of_a_dup_kept_alive_vaspace` — a mapping whose VAS is kept alive by a dup (touched-but-not-dead) must survive; `\|\|` tears it down. |
| `free_subtree` dead-VAS mapping `delete !` (552:63) | real | same test — `delete !` selects live (not dead) VASes → drops the live mapping. |
| `free_subtree` parked-map prune `&& → \|\|` (567:17) | real | `free_subtree_prunes_a_parked_map_when_its_memory_is_freed` — a parked map whose memory was freed must be pruned, not resurrected against fresh memory. |
| `apply_map` backing `base + offset → base - offset` (674:64) | real | `map_at_offset_forward_populates_base_plus_offset` — a non-zero map offset resolves to base + offset (every prior map used offset 0). |
| `apply_map` idempotency guard `*existing == mapping → true` (691:31) | real | `conflicting_map_at_same_va_is_loud_identical_is_idempotent` — an identical re-send is idempotent, but a different map at a live (vas, va) is a loud `ConflictingMap`. |

(Two more `free_subtree` mutants — `519:50 \|\| → &&` and the `520`-region — were caught as
**timeouts**: the mutation makes the fixpoint loop non-terminating, which the per-mutant
`--timeout 240` detects = killed.)

### `kayfabe-fwd` (3 survivors → 2 real gaps killed, 1 acceptable)

| Mutant | Class | Killed by / why not |
|---|---|---|
| `handle_doorbell` guard `working_set.is_empty() → true` (274) | real | `cb14_ring_gate_on_vas_freed_channel_refuses_nonempty_allows_empty` — the no-VAS ring-gate arm IS reachable with a **live** host channel: a guest can `Free` its VASpace handle AFTER the channel materialized (re-projection nulls `chan.vas`; the host channel persists, so lazy materialization is skipped and `chan.vas.ok_or(NoVas)` is never reached). In that state a NON-empty working set must be a loud `NoVas` with ZERO host ops — the `→true` mutant skips the gate and rings the host doorbell UNGATED (the #14 cross-VAS class). |
| `handle_doorbell` guard `working_set.is_empty() → false` (274) | real | same test — an EMPTY working set on the VAS-freed channel must still ring (nothing to gate); the `→false` mutant refuses it. (An earlier triage wrongly called these equivalent by assuming `vas == None` ⇒ `host_channel == None`; the free-after-materialize sequence disproves that — a good example of the gate forcing a sharper model.) |
| `parse_pushbuffer` `MAX_PUSH_TOTAL_BYTES - total → + total` (541) | **acceptable** | This term trims the FINAL range so `total` lands exactly on the 8 MB budget instead of overshooting. Its mutation loosens that trim — but the **load-bearing** aggregate bound is the independent `if total >= MAX_PUSH_TOTAL_BYTES { break }` one line above (537), which is untouched. Under the mutant the total read is still bounded (overshoot ≤ one `MAX_PUSH_RANGE_BYTES` = 1 MB, so ≤ ~9 MB, never unbounded) — the boundary-1 *guarantee* (no unbounded read from hostile input) holds. Killing it would need an ~8 MB, 12+-range guest ring to observe a ~400 KB overshoot: brittle theater against a preserved invariant. Accepted; the security property is asserted structurally by the `break`. |

## L1 baseline (2026-07-25) — and the two harness defects it exposed
<a id="l1-baseline-2026-07-25"></a>

**Scope:** `crates/kayfabe-rt/**`, `kayfabe-core/src/reactor.rs`,
`kayfabe-fwd/src/lib.rs`, `kayfabe-isolate/src/lib.rs` — the L1 surface (the threaded
shell's logic, the completion-source reactor, plan/execute/commit, the isolate pool /
condemnation path). 292 mutants generated.

### ★ The first campaign's number was not a number

The first L1 campaign reported **24 caught / 36 viable = 67%**, with 256 "unviable".
Both halves of that were wrong, for two independent reasons — and this section exists
because a mutation score that can be wrong *upward* is worse than no score at all.

**Defect 1 — `--test-workspace`.** `kayfabe-fwd` and `kayfabe-rt` have no unit tests of
their own; every test that covers them lives in the `kayfabe-tests` workspace crate.
Without `--test-workspace true`, cargo-mutants runs only the mutated package's own
tests, i.e. an empty suite, and reports **everything** MISSED. (Already known and
already in the CI invocation; the scoring step now also fails on a zero viable count,
so the failure mode is loud rather than a plausible-looking bad score.)

**Defect 2 — rustc ICEs scored as "unviable".** ★ This is the one that inflated the
score. cargo-mutants rewrites one source file per mutant inside a copied tree that
reuses a single `target/`, and therefore a single **incremental** dep-graph cache. That
cache does not survive the churn: rustc aborts with
`rustc_middle/src/query/on_disk_cache.rs:479: assertion left == right failed`.
cargo-mutants cannot tell a compiler crash from a type error, so it files the mutant
**unviable** — which removes it from the denominator *silently*.

Measured: **136 of 293 mutant builds ICE'd.** The tell was visible in the output all
along — `SourceRegistry::is_empty -> false` was reported CAUGHT while its sibling
`-> true` was reported UNVIABLE, and `-> true` obviously typechecks. Setting
`CARGO_INCREMENTAL=0` removed every ICE (0 of 292 in the re-run) and moved 88 mutants
from "unviable" into the real denominator.

*The reported anomaly resolves the same way:* `lib.rs:1606:39 replace - with +` was
MISSED in the full run and UNVIABLE in a scoped re-run of the same file. The **full
run was right**; the scoped run's "unviable" was an ICE in the `pushbuffer_parser`
test-crate compile. Viability is a property of the mutant, never of the run scope — so
whenever the two disagree, the ICE-free run is the true one. CI now fails outright if
any mutant's build log contains an ICE, before the threshold is even read.

### The measured baseline

Viable = caught + timeout + missed (unviable excluded, as in the L0 table).

| Run | caught | timeout (killed) | missed | unviable | **viable** | **killed** | **score** |
|---|---:|---:|---:|---:|---:|---:|---:|
| First campaign (ICE-contaminated, unusable) | 24 | 0 | 12 | 256 | 36 | 24 | *67%* |
| **Before** — same tree, `CARGO_INCREMENTAL=0` | 152 | 1 | 19 | 120 | 172 | 153 | **88.95%** |
| **After** — with the tests below | 157 | 2 | 13 | 120 | 172 | 159 | **92.44%** |

The honest "before" is **88.95%, not 67%** — the first campaign was simply scoring a
fifth of the surface. Workspace suite: **203 tests** (was 192), fast path still ~18 s.

### Real gaps closed (each verified per-mutant by hand-applying the mutation)

★ marks the R5 re-validation guards — `l1_concurrency.md` §11 B5's "a forgotten
re-validation is *quieter* than the deadlock it replaced", so every assertion here
names the **exact fault variant** (§12.10: a canary in this codebase has already
passed for the wrong reason once).

| Mutant | Killed by |
|---|---|
| `reactor.rs:153:9 owner -> None` | `owner_names_the_routing_end_and_notify_owns_no_proc` (unit) — `owner` is a projection nothing else asserts; `deregister_proc` goes through the deliberately-different `belongs_to`. |
| `reactor.rs:169:59 == -> !=` (`from == proc`) | `deregister_proc_spans_all_gpus_and_spares_other_procs` — the test had only a `to`-direction seam, so the `from` half was never observed. Now one seam per direction **plus** a third proc's bystander seam. |
| `reactor.rs:169:67 \|\| -> &&`, `169:73 == -> !=` | same test (previously hidden as ICE-unviable). |
| `reactor.rs:387:9 iter -> empty()` | same test — every prior `iter` assertion was an `all(…)` predicate, vacuously true on empty. Now a count + content assertion. |
| `reactor.rs:399:9 is_empty -> true` | same test — `assert!(!reg.is_empty())` after a *partial* retire. |
| ★ `lib.rs:485:40 \|\| -> &&` (`plan_publish` target) | `plan_publish_refuses_when_either_half_of_the_target_is_missing` — removes the arena half alone, then the isolate half alone; each must be `NoTarget` **before any host verb**, and with both present the identical call plans (non-vacuity). |
| ★ `lib.rs:1285:26`, `1468:26 \|\| -> &&` | `commit_engine_object_proc_guard_refuses_on_either_term_alone`, `commit_control_proc_guard_refuses_on_either_term_alone` — commits proc A's plan against a **live** proc B (the term no whole-device canary reaches, since the shell always re-locks the plan's own `ProcId`), and separately retires A under its own plan. Exact `Stale::Proc(A)`; the control case also asserts the guest buffer is **untouched**. |
| ★ `lib.rs:531:26`, `918:26 \|\| -> &&` | `commit_publish_and_doorbell_proc_guards_refuse_on_either_term_alone` — the other two textually identical guards, plus "the refusal hands back its orphans": refusing *and* leaking is not refusing. |
| `lib.rs:403:16 delete !` (`round_trip` orphan release) | `a_refused_commit_releases_its_orphans_on_the_single_threaded_path` — a re-publication of an already-bound VA refuses in the commit with host memory already allocated; exactly that object is unmapped then freed, and the first publication's is left alone. |
| `lib.rs:295:9 Orphans::is_empty -> true` | `r5_canary_channel_torn_down_in_the_gap_refuses_loudly`, extended: the refusal frees **exactly** the objects it orphaned, child before parent. (Was ICE-hidden; the `-> false` sibling is equivalent — see below.) |
| `lib.rs:1606:39 - -> +` (total read budget) | `total_read_budget_clamps_a_straddling_range_to_what_is_left` — a 12-range GPFIFO whose budget edge lands **inside** a range, so the remaining-budget clamp is observable at all. ★ This supersedes the L0 verdict on the same term (rated *acceptable* at line ~184 above, on the grounds that killing it needed "an ~8 MB, 12+-range guest ring"): it does, and it costs 0.35 s. |
| `lib.rs:1676:59 ^ -> \|` and `^ -> &` | `sem_release_completion_identities_mix_both_operands_and_never_collide` — every prior test asserted only *that* a completion was observed, never *which*. A lossy fold is a completion **collision** = a lost completion (the F2 species), arriving through the untrusted pushbuffer. |
| `device.rs:868:9 kill_worker_slot -> ()` | `worker_death_kills_its_own_pool_slot_not_merely_the_proc` — `retire_proc` parks the proc in `spine.retired` rather than dropping it, so the dead slot is observable: pool size unchanged, idle count down by exactly one. The kill and the retire are separate critical sections by design (§7.3), and a sibling thread can reach the pool in that gap. |
| `device.rs:541:9 return_worker -> ()` | `a_refused_op_returns_its_worker_to_the_pool` — pool of ONE, so a leaked slot is a permanent wedge rather than a statistic; the watchdog turns it into a bounded failure. |
| ★ `device.rs:814:9 gate_working_set -> Ok(())` | `the_shell_ring_gate_refuses_an_unpublished_working_set` — the **#14 ring gate** as the shell exposes it. `pushbuffer_parser.rs` pinned the core function; the shell's route+lock wrapper was replaceable with "everything is published". Asserted in both lock modes, with a published-VA non-vacuity check. |

### Residual survivors — equivalent, or documented and not chased

**Proven equivalent** (no input distinguishes them; verified by applying the mutation
and observing the whole 203-test suite still green, plus the argument):

- `lib.rs:295:9 Orphans::is_empty -> false`. Both call sites read it as
  `if !orphans.is_empty() { worker.execute(&orphans.release_plan()) }`, with the worker
  already in hand and the result discarded. Under the mutant an **empty** `Release`
  runs: `for &_ in []` twice, `Ok(VerbReply::Released)`, dropped. Zero backend calls,
  zero state change — the only difference is a wasted match arm. (The dangerous
  direction, `-> true`, is a real leak and is killed above.)
- `lib.rs:1580:23 i = end.max(i + 1)` → `i * 1`. At that point `end =
  start.saturating_add(nargs).min(words.len())` with `start = i + 1`, and the loop
  condition guarantees `i < words.len()`, so `start + nargs ≥ i + 1` and
  `words.len() ≥ i + 1` ⇒ `end ≥ i + 1`. Therefore `end.max(i + 1) == end == end.max(i)`
  for **every** reachable state. The `.max(i + 1)` is defensive redundancy that the
  `end` computation already provides; it is unreachable-by-construction, not untested.

**Documented, not chased** (a test would pin diagnostics or need machinery that does
not exist — decision #15: no brittle internal-state pins):

- `lock.rs:111:5 held_ranks_in -> vec![]`, `113:26 & -> \|`, `& -> ^`, `113:36 != -> ==`
  (4 mutants). `held_ranks_in` has exactly one caller: interpolating a rank list into
  the **text of the R3 panic** that is already firing. Whether R3 panics — the
  invariant — is pinned by the lock suite; these mutants change only what a human reads
  afterwards. Pinning that string would couple the invariant suite to a diagnostic.
- `isolate/src/lib.rs:364:9 <Worker as Debug>::fmt -> Ok(())`. Same class: `Worker`'s
  `Debug` exists so a panic message can name the slot. No production behavior.
- `rt/src/inbox.rs:109:9 Inbox::is_empty -> true`. `Inbox::is_empty` has **no production
  caller** — it exists to satisfy `clippy::len_without_is_empty` beside `len()`, and
  `len()` *is* pinned (its own mutants are caught). An assertion here would restate the
  `len()` assertion one line above: a hollow test to move a number, which is precisely
  what this gate is supposed to make unnecessary.
- `device.rs:245:9 PoolGate::sample -> 0` / `-> 1`, and `258:9 wait_for_return -> ()`
  (3 mutants). All three degrade the pool-full backpressure gate from **parking** to
  **spinning**: the waiter returns immediately and re-enters from the top, which is
  still correct and still terminates (`pool_full_is_backpressure_not_a_hang` passes
  either way). What changes is CPU burn, and the suite asserts backpressure as
  **progress, never wall-clock** (§8.4) — deliberately, because timing assertions are
  how concurrency suites become flaky. Accepted as performance-shaped, not correctness.
  ★ Their neighbours (`251:9 signal_return -> ()`, `261:22 == -> !=`) ARE killed, as
  per-mutant timeouts — i.e. this cluster is scored by *wall clock*, so it is the one
  place the L1 score is not fully deterministic: `sample -> 1` came out CAUGHT in one
  ICE-free run and MISSED in the next. That measured churn (≈2 mutants) is what sets
  the threshold's headroom below.

**Known real gap, left open with its fix named** (the honest one):

- `device.rs:633:29 retries += 1` → `*= 1`, and `634:49 retries < MAX_COMMIT_RETRIES`
  → `<=`. These bound the converging-staleness **re-plan loop**; `*= 1` pins `retries`
  at 0 and makes the bound vacuous, so a pathological `Stale::Rebound` cycle becomes a
  spin instead of a surfaced fault. Killing them needs a mock that can force a *repeated*
  Rebound (today's recorder can inject a one-shot host error, not a persistent commit
  race), so the test would be new **machinery**, not a new assertion. Recorded here
  rather than papered over; the sibling counter mutant `251:53 += -> *=` on `PoolGate`
  IS caught, as a per-mutant timeout, which is the shape this one would take too.

### The threshold, and why it is where it is

CI's `mutants` job now fails below **91%** (see the scoring step in
`.github/workflows/ci.yml`). The reasoning:

- Measured after this pass: **92.44%** (159/172), up from 88.95% (153/172). The 13
  survivors are: **2 proven equivalent**, **6 diagnostics-only** (`held_ranks_in` ×4,
  `Worker`'s `Debug`, `Inbox::is_empty`), **3 spin-vs-park** on the pool gate, and
  **2 real** — the retry-bound pair, which needs new mock machinery. So the untested
  *logic* on this surface is 2 mutants wide, and it is named.
- The bar is **91%**, not 92.4%. One mutant on this surface is ≈0.6 points, and the
  pool-gate cluster is scored by timeout, which measurably moves by ~2 mutants between
  identical runs (see above). 91% means 157/172 still passes and 156/172 fails: it
  absorbs the known timing churn and trips on the third lost mutant — a real regression
  cluster — rather than on jitter. Setting it at the measurement would have produced a
  red night with nothing wrong, and a gate that cries wolf gets muted.
- It is **not** set at L0's 99.2%. That is a pure, single-threaded core; this surface is
  the threaded shell, and its residual is diagnostics plus one missing fault-injection
  knob, not untested logic. A gate set above reality is theatre — and being muted is
  exactly how this surface reached `master` with no mutation run in the first place.
- Raise the bar when a campaign measures higher **and** the number holds across a
  second run; never lower it to clear a red night. A survivor is a test gap, and this
  file is where that argument gets made.
- CI runs the campaign with **`KAYFABE_SLOW=1`**, which only ever *raises* the score
  (the 73 s pushbuffer fuzz is the sole coverage of the untrusted decoder; measured on
  the decoder scope, it took that slice from 2 missed to 1). The number above was
  measured with the flag **off**, so it is a conservative floor for what CI will see.
  Cost of the flag: ~90 s per mutant, roughly doubling a weekly job. Gating a slow test
  had silently hollowed out a security gate; that trade is not worth 90 s.

## Reproduce

L0 (the pure core):

```
export PATH="$HOME/.cargo/bin:$PATH"
cd /workspace/nvkvm-rs
cargo mutants -p kayfabe-core -p kayfabe-mmu -p kayfabe-fwd -p kayfabe-completion -p kayfabe-arch \
  --test-workspace true -j 2 --baseline skip --timeout 240 --build-timeout 900
```

L1 (the threaded shell). **`--test-workspace true` and `CARGO_INCREMENTAL=0` are both
load-bearing** — see §L1 baseline for what each one silently does to the score if
dropped. `-j 2` because each job copies the tree.

```
CARGO_INCREMENTAL=0 cargo mutants -p kayfabe-rt -p kayfabe-core -p kayfabe-fwd -p kayfabe-isolate \
  -f 'crates/kayfabe-rt/**' -f 'crates/kayfabe-core/src/reactor.rs' \
  -f 'crates/kayfabe-fwd/src/lib.rs' -f 'crates/kayfabe-isolate/src/lib.rs' \
  --test-workspace true -j 2 --timeout 240 --build-timeout 900
```

Sanity-check any campaign before trusting its number:
`grep -rl "thread 'rustc'" mutants.out/log/ | wc -l` must be **0**.

The new survivor-killing tests live in `crates/kayfabe-completion/src/lib.rs` (unit),
`crates/kayfabe-core/src/gpa.rs` (unit), and the workspace suites
`tests/tests/c_bug_regressions.rs`, `tests/tests/object_model.rs`,
`tests/tests/rmgraph_order_independence.rs`, `tests/tests/security_boundary.rs`. Each is
tagged `★ Mutation-gate kill` in its doc comment and names the mutant it kills.
