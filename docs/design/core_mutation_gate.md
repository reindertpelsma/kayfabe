# Core mutation-testing gate — does the suite NOTICE when the core lies? (decision #28)

**Status:** empirical gate, 2026-07-24, run against head `0ba300b` (post-#27 whole-core
determinism differential). Tool: `cargo-mutants 27.1.0`, stable toolchain, `-j 2`,
`--baseline skip` (suite confirmed green first), `--timeout 240`, full-workspace test
harness per mutant. Survivor-killing tests added on top; the workspace suite is green at
**132 tests** (was 113) with `cargo clippy --workspace --all-targets` clean and the core
still `#![forbid(unsafe_code)]` with no new runtime deps.

**2026-07-25 addendum (triage audit):** a re-audit of the residual survivors found the two
`handle_doorbell` guard mutants previously called *equivalent* are in fact **real gaps** —
the state their equivalence proof assumed unreachable (`chan.vas == None` with a live host
channel) IS reachable by freeing the VASpace handle after materialization. Both are now
killed (§fwd below). Every gap-killing test in this doc was **verified per-mutant** by
hand-applying the exact `cargo-mutants` mutation to the source and confirming the named
test flips from pass → fail (and the fwd/mmu numbers come from a full scoped re-run of
those two crates *with the new tests in place*). Final score: **245/247 = 99.2% killed**,
residual = 1 proven-equivalent + 1 documented-acceptable.

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

## Reproduce

```
export PATH="$HOME/.cargo/bin:$PATH"
cd /workspace/nvkvm-rs
cargo mutants -p kayfabe-core -p kayfabe-mmu -p kayfabe-fwd -p kayfabe-completion -p kayfabe-arch \
  --test-workspace true -j 2 --baseline skip --timeout 240 --build-timeout 900
```

The new survivor-killing tests live in `crates/kayfabe-completion/src/lib.rs` (unit),
`crates/kayfabe-core/src/gpa.rs` (unit), and the workspace suites
`tests/tests/c_bug_regressions.rs`, `tests/tests/object_model.rs`,
`tests/tests/rmgraph_order_independence.rs`, `tests/tests/security_boundary.rs`. Each is
tagged `★ Mutation-gate kill` in its doc comment and names the mutant it kills.
