# ENS10: scoped source admission and admission slice repairs

Source under test: `a577f065966f0d347b2fac0370e2b9165100f24b`, based on ENS9 `64c50ad472d4402a5bd7e7a3b07a628254a6565a`. The source manifest and SHA256 seal are in [source-meta.json](source-meta.json) and [source-sha256.json](source-sha256.json). This directory contains CPU evidence; no ENS10 hardware run occurred.

CODE READ: [the design](../../docs/design/V3_WALK_GATE_NARROWING.md) records the protections, invariants, conservative cases, protocol and remaining continuation/range work. Known target A's walk no longer closes target B's source admission. Own-space, unresolved/global, CPU-window, PDB-form split and overflow work remain conservative. Applied statement milestones are separate from the unchanged global RPC reply barrier. A release/register race continues or re-owes the exact guest token, and ready rings are awakened before entering unrelated synchronous apply. No timeout or poll interval changed; `tsgmirror.rs` is unchanged from ENS9.

The stronger goal remains PARTIAL: same-space disjoint ranges are held, multi-target batches retain their union, and unknown/global operations can still hold every ring. The soft admission allowance cannot bound total apply or continuous own-space hold to 100–200 ms. RM calls, mandatory rollback, recycle, retirement and ordinary mapping remain outside such a guarantee. A safe prefix continuation plus range certificate is still needed.

## Failing-first evidence and falsifiers

MEASURED in ENS10 CPU runs 2026-10-27, with raw-log hashes in [evidence-sha256.json](evidence-sha256.json):

| Baseline / committed red stage | Result |
|---|---|
| `309124774`, scoped unrelated-space contracts | 4 fail, 2 pass |
| Existing completion order, before narrowing | 2 pass; PUT-only completion-delay hypothesis falsified |
| `d6d556d9c`, applied-space independence from foreign RPC work | 2 fail, 2 pass |
| `8a720f118`, before-apply wake and late registration | 4 fail, 39 pass |
| `ca9fcbd57`, admission cleanup tail and slow replacement | 4 fail |

The original global gate already harvests host GET/reference before reaching its PUT gate. These baseline tests refute that gate as the sole explanation for delayed observation of already-published work. They do not establish native host publication or Windows completion-vector ordering from guest DMA submission.

## Adversarial model

MEASURED in ENS10 CPU run 2026-10-27, `handoff-green.log`: 62 focused tests pass. Both scope graphs (split requests and applied statements) explore 266,810 states and 1,076 terminal states each, with a bounded fair suffix from every reachable prefix. The four-million-state frontier cap fails as inconclusive. The graph imports the existing lifecycle harness and production Relay; native Inbox decisions, queued/running requests and ACKs participate in each scope decision.

MEASURED in ENS10 CPU run 2026-10-27, `handoff-green.log`: 256 seeds replay identically twice and cover Scan, Pointer, Window, migration and real exhausted credit. Separate fair schedules hold a foreign apply unfinished throughout another target's source reads, publications, migration and completion. Own held work cannot publish. A fresh-worker scenario observes a host-finished marker/GET within three worker steps while both holds remain active. This is a scheduler-step result under stated worker/GPU fairness, not an OS-time bound or a Windows interrupt test.

## Mutation results

MEASURED in ENS10 mutation campaign 2026-10-27: all 24 scope mutants and all 5 slice mutants are caught by at least two tests each; compilation errors count as zero kills and none occurred. [mutations.json](mutations.json) and [slice-mutations.json](slice-mutations.json) list every failing test. [mutation-attestation.json](mutation-attestation.json) binds the exact restored production/model/test inputs; no post-matrix normalization is used.

The publication/migration and pre-apply-wake mutants are caught by source-composition checks, which bind native callers to the tested adapter. Those are not simulations of actual GPU execution. Predicate, waiter, sequencing, manager-target and diagnostic mutants fail behavioral tests. The model itself runs separately against the unmutated native adapters; the entire model is not rerun for every mutant.

## Why the reservation peak exceeded the claimed slice

CODE READ in `sparse.rs` and `sparse/slice.rs`, reflected in [the design](../../docs/design/V3_WALK_GATE_NARROWING.md): the peak sums timed reservation/free calls, including space-reset work between renewals. It does not include every apply operation. Admission failure formerly converted every fresh sparse tile without a budget check, and replacement started every child after a slow release. ENS10 budgets optional complete-tile cleanup, queues deferred conversions, rechecks before each child and preserves mandatory repair and partial accepted coverage. Deferral does not fabricate an RM refusal.

MEASURED in ENS10 CPU run 2026-10-27, `slice-green2.log`: 317 host library tests pass with 3 existing ignored. The simulated 120-ms release case costs 240 ms after repair, compared with 360/480 ms before the fix. This explicitly disproves a universal 110-ms expectation. The coordinator's supplied boot-7391 aggregate peaks do not identify which reservation path consumed its particular 2.41 seconds; no causal host attribution is claimed here.

## Validation and limits

MEASURED in ENS10 CPU run 2026-10-27: the final serialized six-package regression passes 2,442 tests, with 0 failures and 14 existing ignores across 76 targets; all-target Clippy has new=0. Details are in `validation.json` and `gates.json`. The earlier full run failed one stale source-lookup guard; its log is preserved and that guard now checks the scoped call before retry. No assertion was removed and no ignore was added. Four historical scoped tests were activated.

Changed Rust files pass rustfmt with child-module traversal disabled. The workspace-wide formatting check reports three inherited differences: module ordering in `kf-chan/src/lib.rs` and `kf-mem/src/lib.rs`, and a tuple in the sparse cost harness. Those unrelated source files were preserved. All newly authored/changed Rust lines fit 100 columns. Existing ignored-test reasons are retained in the final regression ledger. The shared build cache is retained because it contains other users' artifacts.

The owner’s latency-decomposition addendum received 2026-10-27 is DESIGN-ONLY in this source. The design note specifies observation/publication/tagged-marker/actual-notification timestamps, a bounded slow history, gate causes, clock pairing and adversarial correlation tests. Per-packet stage counters/dumps and native hook tests are not implemented or claimed. GPU-wide vector edges can precede marker sampling; logical latching is distinct from the shared physical IRQ line’s successful notification.

MEASURED in ENS10 standalone CPU simulation 2026-10-27: [latency-prototype.rs](latency-prototype.rs) passes 13 tests, including exact decomposition and gate/scan delay confined to the first stage. This evidence artifact is not wired into production or included in the Cargo/source seal. It assumes caller-supplied packet/notification association. Native correlation, heartbeat clock binding, partial-stage aggregation, complete gate-interval retention and pending-first-slow dumps remain pending. These 13 checks are separate from the 2,442 Cargo tests.
