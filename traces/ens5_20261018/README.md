# ENS5 GPU-free evidence — validation checkpoint

Status: PARTIAL, 2026-10-27. Current standalone source checks and gates pass; the final serialized Cargo suite is pending additional disk capacity above the mandatory4GiB floor. No PR or hardware campaign is ready for approval.

Source branch `codex/ens5-20261026`, TS1 base `0dbdefd99113c075b1d66c081d82490aa69922d7`, independent breaker branch merged. No host, VM, GPU or host lock was used by this lane.

The [design note](../../docs/design/V3_ENS5_SPARSE_ADMISSION.md) describes the local admission/rollback algorithm, whole-block occupancy, bounded cache, empty-normal recycling, same-apply reconsideration and diagnostics. [PREREG.md](PREREG.md) specifies the still-unrun hardware matrix; [BUDGET.md](BUDGET.md) separates finite design bounds from measurements needed before changing them.

MEASURED in the ENS5 CPU run 2026-10-26/27, `validation.json` and `mutations.json`: failing-first regressions cover the saturated slow path and ownership defects; subsequent model and native-verb-fake checks exercise the fixes. Compile failures are not killing tests. Mutation results distinguish behavior from weaker source-composition checks.

MEASURED in ENS5 CPU run 2026-10-26/27, `timings.json`: the saturated admission and mixed-apply comparisons isolate CPU bookkeeping with a fake RM. They are not native allocation costs, Windows packet latencies, or proof that the hardware workload passes. `highwater-available.json` records the limited local counter evidence and explicitly unavailable per-workload/per-space coverage.

Four breaker bodies are retained but ignored with explicit reasons in the design note and validation ledger. No assertion body was weakened or deleted. A failed parallel wall-time ratio check is retained in private evidence; final timing-sensitive checks run serially without changing its bound.
