# ENS5 prospective hardware registration — unrun

Registered 2026-10-27 before any ENS5 host/VM/GPU access. Source branch `codex/ens5-20261026` starts at TS1 `0dbdefd99113c075b1d66c081d82490aa69922d7` and merges breaker head `81abb80db`. The controller must pin final source, executable hashes, driver, flags, guest snapshot and workload before running. Sparse twin remains default OFF. No limit is raised by this task.

## Native RM cost arm (owner approval required)

Use the prepared `kf-sparse-object-cost` and `scripts/bench/ens5-object-cost.sh` under the documented exclusive host lock, an unprivileged UID with zero effective capabilities, no VM and an own RM client. Owner instruction is explicit: ask first. The launcher authorization flag does not grant permission on its own.

Run separate normal2MiB and sparse64KiB reservation arms at decimal counts1000,8000,32000,128000, ascending only while the preceding arm and desktop watchdog remain healthy. A first RM refusal or watchdog failure stops that arm and the ascending series. Fixed timeout300s, TERM then10s kill deadline. Every successfully created handle is freed; cleanup failures remain failures, and native client close is the final ownership boundary.

Capture per-object alloc/free wall samples, created/requested count, first refusal index/status, before/peak/after process RSS, host meminfo, readable RM client accounting and NvidiaSMI process-memory accounting. Unavailable accounting is unavailable, never zero. RSS includes user-space bookkeeping; NvidiaSMI is not an exact kernel page-table-memory meter. Do not label a noisy whole-host MemAvailable delta as exact per-object cost. The operator must supply a desktop watchdog program with before/during/after observations; its failure is a stop, not a passing allocation result. Keep the desktop's packet/frame health separately from aggregate GPU utilization.

## Linux validation

Prospective plan, registered 2026-10-27: after source/binary sealing and approval, run the existing Linux fast/gate suite with twin OFF, then ON. Include the saturated admission timing/scaling gate: small failed/declined admission below5ms, local undo work independent of unrelated blocks,4000maps+100saturatedbinds below100ms in the CPU model. Record actual native RM times separately. Run the ENS4 steady-completion/churn arm only with its matching sealed diagnostic build and verified USER-gate route; an ordinary Linux bypass route is a separately labeled control.

Collect admit_calls, admit_hard_err, admit_decline, total/max_ns, admit_blocks, cache coverage, live/peak blocks, slow_apply, dominant pipeline timer, walks_per_beat and census skipped/truncated coverage. Capture the first slow-apply snapshot and recent diagnostic tails. No green result can substitute for missing route/exposure or receipt evidence.

## Windows validation

Fresh Windows SPEC nine-workload suite with twin ON, three fresh boots, fixed production flags and TS1 mirror/replay behavior. Target: all nine workloads PASS on every boot, no heartbeats with a single apply at least200ms, admit_hard_err0, no unexpected Xid or guest153/141. Report all trials, first failures, unhonoured/capped counts and workload/subtest endpoints. A host Xid or guard refusal follows the controller's stop rules and is not silently retried.

Then run a fresh twin OFF control, followed by the requested performance A/B with the same source and configuration. No performance claim or default-ON change is authorized by CPU tests. Keep private/proprietary workload results out of public evidence; only allowed counters and appropriately labeled approved scores may be published.

## High-water and budget decision

Record the capture configuration’s exact viewset names in the parser manifest, with all expected workload intervals even when their samples are missing. Record workload boundaries and per-space live/peak census, including handle reuse and incomplete rows, for all nine SPEC workloads plus Heaven and Steel. Existing local excerpts are session aggregates, not per-space high-water. Session made-minus-freed is not a current-live gauge. A print-capped or incomplete workload remains censored; a missing workload remains unavailable.

Only after native object cost, refusal limits, watchdog observations and workload demand are known should the owner select another budget. Any proposal must keep a finite per-VM/session hard allocation bound and fairness between spaces. This registration changes neither8192 nor32768.
