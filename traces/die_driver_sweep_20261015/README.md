# Die × driver campaign, 2026-10-15

**STATUS: LIVE, 2026-10-15.** Implement, test and fix the other-die cells under
`/root/codex_prompt_die_driver_sweep.md` (including its added implementation requirement).
Base: `origin/claude/gpu-types-20261015` plus cursor fix `2f99e673d`;
initial combined product revision: `23585a319`.

## Hypotheses stated before runs

| ID | hypothesis | falsifier | result |
|---|---|---|---|
| B0 | The combined product revision passes all GPU-free `kf-*` tests. | Any failed test or failed compilation, with all crates attempted (`--no-fail-fast`). | Falsified at `23585a319`: 3230 passed, one aggregate-RSS test failed. The exact test passed alone. Fix `59ea4dbaa` isolates both process-RSS oracles; full rerun 3231 passed, zero failed, 14 ignored. `gpu_free/{before,after,rss-isolated}.txt`. |
| A1 | GA104 (RTX 3070) on open host 595.91.07 runs the gates, raw bare-metal suite and fast guest on the combined revision. | A failed gate, non-USER birth, or any failed/crashed/not-run arm; distinguish provisioning failure from a product failure. | The first suite falsified the all-arms-pass prediction (A3); a fresh-seed rerun passed: gates 9/9 + gate10 PASS, every birth USER, native raw 30/30 and fast guest 595.91.07 30/30 at `ba864a55`. The initial native 29/30 is retained (A3). |
| A2 | The same GA104 host runs the native CUDA ladder and the stock Linux 580.159.04 guest ladder correctly. | Any rung's content oracle fails; a native failure prevents attributing that workload's guest failure to kayfabe. | Four native rungs passed. Initial guest run failed before work due to the wrong profile (A2-profile); after the harness fix, all four guest rungs passed, same QEMU binary. `ga104_3070/cl_codex_ampere_580_receipt_guest.out`. |
| A2-profile | The fat guest's GSP boot failure comes from selecting host ABI 595 for guest driver 580.159.04. | With only the explicit guest-driver property corrected, cup2 still fails at the same queue-header check. | Not falsified: cup2 passed on the same binary `ba864a55`; full ladder rerun passed 4/4 after the receipt fix. The first four failed rungs are retained as a harness failure. |
| A3 | Native concurrent-fuzz's initial seed generated no engine work in its positive control, rather than detecting a GPU correctness failure. | Replaying seed 1792058516649537499 performs engine work and still fails its control, or shows a correctness violation. | Not falsified: the original seed replay again produced zero control engine operations and no violations. Seed 1 performed 17 control engine operations and passed. A complete native suite rerun (new generated seed) passed 30/30; no grader code changed. Both seeds and the first failed run are retained. |
| G1 | GA104 renders the graphics suite correctly with native and 580.159.04 guest userspace. | A native failure (environment/workload finding), or a guest step failure / render hash mismatch after the native oracle passes. | Native and guest steps passed; ten render hashes matched at `ba864a55`, box 56076314. |
| W1 | GA104 with the Windows 580.88 baseline reaches Code 0, desktop and DWM under the production profile. | Any nonzero adapter problem code, absent desktop/DWM, boot failure or TDR. A passing desktop additionally needs a D3D app before the cell passes. | Pending: two-boot native-image INF/GSP setup, then a fresh production-profile QEMU process. Unknown GA104/580 caps remain an explicit measurement gap. |

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

## GA104 wave, measured results

Box 56076314, RTX 3070 (10de:2484), stock open host 595.91.07.
All completed hardware runs used `kf3-bins/ba864a55/qemu-system-x86_64`.

| Guest | Raw | CUDA | Graphics | Desktop / broker | Cell |
|---|---|---|---|---|---|
| Linux 595.91.07, kernel 6.8.0-59 | 30/30 | not tried | not tried | not tried | incomplete |
| Linux 580.159.04, kernel 6.8.0-142 | 30/30 with automatic receipt | 4/4 | native + guest steps pass, 10/10 hashes match | Cinnamon + NVIDIA gears about 60 FPS; actual X11 SHM presentation on Xvfb | pass for these checks; diagnostics below |
| Windows 580.88 | not tried | not tried | not tried | realize refused before boot | fail; missing host GID ABI inventory row at 595.91.07 |

The original thin initrd also passes its timer arm with only an explicit
`guest-driver=580.159.04` added; no rebuilt-initrd confound in that comparison.
Before and after logs are retained. The full rebuilt thin suite passed 30/30.

The broker test measured zero guest Xid and zero flip timeouts, with 12 NV assertions.
The display drainer emitted idle-window/head STALL diagnostic dumps; the reported
36 is a count of diagnostic lines, not 36 separate stalls. Presentation used X11
SHM on Xvfb, not a measured DMA-buf path. Teardown ultimately forced the QEMU down
(exit 137) after the harness's ACPI shutdown wait; graceful shutdown was not proved.

Windows first refusal: host ABI rejects `GPU_GET_GID_INFO` (0x2080014a), then
`gpu-uuid=auto` correctly refuses without the host UUID. Hypothesis H-GID: the
published control is absent from the host ABI inventory, so a generated layout row
and control registration will make the default identity work. Falsifier: the
correctly carried unprivileged control still fails or produces no valid UUID.
Next: failing test, compile the public SDK layouts at the committed measured tags,
register the control, rebuild, retry the same default Windows profile. No random
UUID bypass is used to label this production cell passing.
