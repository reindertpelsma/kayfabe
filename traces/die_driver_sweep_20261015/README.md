# Die × driver campaign, 2026-10-15

**STATUS: LIVE, 2026-10-15.** Implement, test and fix the other-die cells under
`/root/codex_prompt_die_driver_sweep.md` (including its added implementation requirement).
Base: `origin/claude/gpu-types-20261015` plus cursor fix `2f99e673d`;
initial combined product revision: `23585a319`.

## Hypotheses stated before runs

| ID | hypothesis | falsifier | result |
|---|---|---|---|
| B0 | The combined product revision passes all GPU-free `kf-*` tests. | Any failed test or failed compilation, with all crates attempted (`--no-fail-fast`). | Falsified at `23585a319`: 3230 passed, one aggregate-RSS test failed. The exact test passed alone. Fix `59ea4dbaa` isolates both process-RSS oracles; full rerun 3231 passed, zero failed, 14 ignored. `gpu_free/{before,after,rss-isolated}.txt`. |
| A1 | GA104 (RTX 3070) on open host 595.91.07 runs the gates, raw bare-metal suite and fast guest on the combined revision. | A failed gate, non-USER birth, or any failed/crashed/not-run arm; distinguish provisioning failure from a product failure. | Pending. |
| A2 | The same GA104 host runs the native CUDA ladder and the stock Linux 580.159.04 guest ladder correctly. | Any rung's content oracle fails; a native failure prevents attributing that workload's guest failure to kayfabe. | Pending. |

Evidence is measured only when a completed command's text output is present here.
Every cell must name its actual QEMU revision, host/guest drivers, box and evidence.
Boot, CUDA, graphics and display are separate checks; partial checks do not make a passing cell.
The owner's running Windows VM is untouched. No GPU workload runs on the shared owner host.
GPU-free builds use `/data/codex/` with two build jobs.

## Next work

Linux first: GA104, then available Ada, Turing and Blackwell; driver endpoints selected
from the committed API intervals, with the GSP boundaries considered separately from controls.
Windows acceptance requires desktop, Code 0, DWM, a D3D app and zero TDR, under the production profile.
For each failure: first failing operation, stock oracle, falsifier, failing regression test,
derived fix, rebuilt cell and regression runs. One wall is bounded to about three hours.

Open owner decisions D-1/D-2/D-3 remain as recorded in
`docs/design/V3_GPU_TYPES_20261015.md` and `V3_CONSTANT_MATRIX_20261014.md`.
F-6 board-specific fallback removal is already in the base; it is not new campaign work.

Budget and instance ownership: [COST.md](COST.md).
