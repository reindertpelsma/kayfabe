# P1 stop report

STATUS: PARTIAL, 2026-10-19. Branch `codex/perf-parity-20261019`; candidate source `78c6d54b2`. No promotion.

## Result

- CODE READ — DONE: full lane instructions; pre-registration; W2/W3/W4 source findings, including the shared completion-fd provenance ambiguity.
- MEASURED — DONE: expected-red tests committed separately; default/diagnostic release tests green; four mutants each killed by two tests.
- CODE READ — PARTIAL: compile-time-excluded, runtime-opt-in NSI recorder and adapted probe/analysis files. Host runner and hardware validation remain unfinished.
- MEASURED — STOP: Clippy failed with 89 inherited denied sites; all primary site texts match production. No changed-line diagnostic was found, but compilation failed, so the gate is not green.
- MEASURED — NOT DONE: host binary, W1 check, performance experiments, correctness hardware gates and morning-time verification.
- MEASURED — CLEANUP BLOCKED: automatic approval review rejected target-directory deletion. The 418 MiB target remains; no alternative deletion was attempted.

## Validation

MEASURED: before implementation, kf-chan + kf-qemu: 833 passed / 0 failed / 3 pre-existing ignored; touched kf-qemu: 312/0/0. All 15 existing pacer tests passed. New wiring tests: EXPECTED RED 1/3/0. Candidate kf-qemu: default 316/0/0; diagnostic feature 322/0/0. Four mutants: default-on, unbounded-tail, collapsed-source and duplicate-snapshot, each 2 kills. See `TEST_RESULTS.md` and `clippy_sites.tsv` for scope and limits.

MEASURED: green suites preceded final formatting-only edits and the Clippy stop. No further cargo command followed that stop. No hardware sample or performance gain is claimed.

| Planned arm | Revision | Flags | P1 n | Median | Spread |
|---|---|---|---:|---|---|
| Production | 0452613b5 | default | 0 | unmeasured | unmeasured |
| TSG mirror | unavailable to P1 | not selected | 0 | unmeasured | unmeasured |
| Pacer | production | 0 / 20 / 50 us, separate arms | 0 each | unmeasured | unmeasured |
| Lazy observe | production | default / 0, separate arms | 0 each | unmeasured | unmeasured |
| PUT cache | production | default / 0, separate arms | 0 each | unmeasured | unmeasured |
| GET suppression | unimplemented | default-off candidate required | 0 | unmeasured | unmeasured |

## Walls and falsifiers

| Wall | Hypothesis and falsifier | Result / gain |
|---|---|---|
| W1 | TSG switching; falsify if mirror DIRECT/COMPUTE legs stay >=300/600 us or Steel Nomad stays within spread | MEASURED: untested; gain unknown |
| W2 | Extra/late edges; falsify if proven redundancy removal leaves ISR/fps unchanged or loses a wake | CODE READ: source ambiguity documented; attribution incomplete; gain unknown |
| W3 | Cheaper synchronous mask writes; falsify if a following shadow read can see stale state | CODE READ: ioeventfd/irqfd do not apply the required device state; no equivalent shortcut established |
| W4 | Lazy observe/GET sync costs; falsify by unchanged performance beyond spread | MEASURED: untested; GET experiment unimplemented; gain unknown |
| W5 | Localize residual GPU leg after W1; falsify if timestamps cannot separate stages | MEASURED: untested; gain unknown |

## Owner decisions and next steps

1. CODE READ — Recommend repairing the inherited denied Clippy sites before resuming; alternatively clarify the changed-line gate when crate Clippy cannot complete. No lint relaxation was presumed.
2. CODE READ — Keep production. After the gate is resolved: host build and required correctness gates, W1 mirror readiness/result, W2 attribution, then the requested balanced experiments.
3. MEASURED — Target cleanup needs an allowed deletion operation; the automatic rejection remains unresolved.

## Host state at stop

MEASURED at 00:07 CEST: the existing owner's run 3008 was still up on production 0452613b; its launcher had recorded OWNER_SESSION_READY. GPU driver binding was nvidia. P1 never acquired the GPU lock, stopped the VM, ran a guest command, or changed a host binary. The existing owner's launcher still held its lock. No P1 process or P1 lock remains. GPU health/Xid counts and the later 06:30 morning state were not verified; no nvidia-smi command was run while Part B was gated.

MEASURED: only CPU source preparation occurred on the host. The required morning note records this early stop and production restart instructions; it does not promise a future host state.
