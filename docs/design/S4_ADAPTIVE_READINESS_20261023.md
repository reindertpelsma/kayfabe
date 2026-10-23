# S4 adaptive Windows readiness

STATUS: LIVE, 2026-10-23; GPU-free proof green, new hardware run pending.

CODE READ (2026-10-23, owner02:20 instruction): the earlier GA102 bind banner timeout is an inconclusive readiness result, not driver evidence. The owner authorizes measured4–10x bench deadlines, absolute ceilings, stage budgets and one retry for the first transient read-only SSH/banner/query failure. Product timers, guest TDR policy, GPU fault admission, single-enable ordering and app oracles are unchanged.

CODE READ (2026-10-23, SW2 commit4ede56ac7): `scripts/bench/blackwell/readiness.py` and its tests are reused byte-for-byte from SW2. The common Windows CLI in `scripts/bench/windows/readiness.py` delegates factor selection, budget clipping and first-query retry to those primitives. It adds the S4 lease/export reserve and authored-command adapter. It does not copy or change Blackwell's Code22→Code0 observer or its fault guard. SW2.md was reread when that implementation appeared; no other worktree was changed and no cross-session message was sent.

CODE READ (2026-10-23, shared readiness code): one standard-VGA install-check boot supplies elapsed real shell time, followed by one successful read-only OS CIM roundtrip. S4 expresses their slowdown as `max(boot_s/60,query_s/2)*0.5` probe seconds for the shared half-second reference policy; its factor is clamped4–10. A timed-out first calibration query contributes its timeout lower bound if its retry succeeds. The box/source-bound receipt is exclusively created and never overwritten.

CODE READ (2026-10-23, `readiness.sh`, `desktop/bind.sh`, `desktop/arm.sh`): per-step requested duration is the nominal duration times the factor, clipped by its absolute ceiling and the persisted remaining stage. Bind/ON preparation stages have3600s absolute ceilings and a1200s lease export reserve; SSH readiness has2700s absolute ceiling. The unchanged170-minute auto-destroy remains the hard outer guard. No retry creates a new stage end.

CODE READ (2026-10-23, `win_vm.sh`): an optional calibration export configures this lane only. The actual SSH ConnectTimeout receives the scaled step budget; its unconfigured default remains8s. Generic baseline calibration completes before any NVIDIA enable. Read-only queries retry one transient failure; unknown failures remain fatal, and failed/partial output cannot establish state. Install, cold preparation and enable commands use the single-attempt path. A readiness deadline returns75/READINESS_INCONCLUSIVE.

MEASURED (2026-10-23, `traces/sweep_dies_20261018/adaptive/expected_red.log`, `inner_ssh_expected_red.log`): tests-only EXPECTED RED commits20db1f7bb and1a33267d1 precede the implementation; missing policy and nested fixed SSH bounds are exposed.

MEASURED (2026-10-23, `adaptive/green.log`, `adaptive/sw2_shared_green.log`, `adaptive/desktop_regression.log`): S4 adaptive57/57, shared SW2 policy21/21 and existing desktop70/70 pass; two CLI fixtures exercise real subprocess output and retry counts. No new Rust code or lint sites; fresh clippy not run.

MEASURED (2026-10-23, `adaptive/mutants.json`, `adaptive/bundle_proof.json`): ten mutants killed by15/6/10/6/9/2/2/3/3/2 tests; source restored. Package checks verify calibration exports precede install, source-relative bind binary selection, canonical wrappers and shell syntax.

INFERRED hypothesis (2026-10-23): measured readiness prevents this lane from treating a slow nested-host banner/query as driver failure. FALSIFIER before the new rental: the frozen calibrated bind still fails to reach a complete actual reply within its stage. That result remains inconclusive about the driver; only actual Windows/RPC/event observations can establish a driver outcome.
