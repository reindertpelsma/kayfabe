# ENS7 GPU-free evidence

Branch `codex/ens7-20261027`, base `21b17af213e0ff751797775a67624f82cc4de347` from the coordinator's ENS6 work. No ENS7 host/GPU access occurred. The source policy and operator-facing fields are described in [the design](../../docs/design/V3_ENS7_SPARSE_BUDGET.md) and [monitoring guide](../../docs/howto/sparse-twin-monitoring.md).

MEASURED in the ENS7 CPU campaign 2026-10-27: `red-to-green.json` names13 initial failing-first regressions and2 additional cooldown regressions. Compile errors are excluded. `mutations.json` records20 runtime mutants and4 source-composition mutants, each with at least2 killing tests and a green baseline. Source-composition tests are weaker than runtime proof and do not represent native GPU execution.

In the ENS7 CPU campaign 2026-10-27, `validation.json` records the final serialized host/memory/QEMU Cargo run and zero-new-debt gates. `source-sha256.json` seals the changed source; `evidence-sha256.json` seals retained private logs. The native RM cost data is the coordinator's pre-existing ENS6 campaign, linked in the design; it is not an ENS7 measurement. Windows validation and the budget/stall hypothesis remain untested by this lane.
