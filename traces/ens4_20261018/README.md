# ENS4 GPU-free evidence

Base: TS1 `0dbdefd99113c075b1d66c081d82490aa69922d7`; branch `codex/ens4-20261026`. No host, VM, lock or GPU use. The new Linux client was compiled and only its explicit virtual-time simulation was executed.

MEASURED in the 2026-10-26 GPU-free run, `validation.json`: EXPECTED RED `6636fbfe8` had13 failures/2 passes for bounded injection and the latency endpoint; EXPECTED RED `e621d20c8` had4 failures/8 passes for scorer route/exposure/receipt contracts. The complete post-change suite, final targeted tests, Clippy and hand-mutant results are pinned in that file and `mutations.json`. Compile failures are not killing tests.

MEASURED in the same 2026-10-26 run, `deliberate-gate.json`: virtual-time no-hold control is PASS; the77ms conditional global apply is deliberately FAIL. Larger synthetic intervention doses also FAIL the calibrated latency endpoint. This is not a Windows TDR, actual Linux timing, a cause established from retained workload logs, or a production repair. No latency limit, walk-gate policy, sparse default or GPU work was changed outside the explicit default-off diagnostic.

CODE READ: use [the design note](../../docs/design/V3_ENS4_DELAY_GATE.md) for flag bounds, actual sleep placement, real CE/WFI receipts, ordinary-Linux route caveat, strict offline scoring and the completed SET_REF metric and its proxy/coverage limits. The prospective [registration](PREREG.md) precedes any authorized hardware boot. Its source/executable/guest image/flags must be sealed by the controller before scheduling; the old ENS3 QEMU seal is not this experiment's seal.
