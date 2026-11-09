**STATUS: DESIGN-ONLY, 2026-11-08.** The migration policy and future step inventory are DESIGN.
CODE READ: the stdlib execution library and Linux apps port described below are implemented;
MEASURED: GPU-free validation is recorded below. No new GPU, VM, Worker, provider or deployment
run was performed for PM1. Worker deployment and execution of a complete unmigrated plan remain DESIGN.

CODE READ, policy update 2026-11-09: [middle real apps](V3_MIDDLE_REAL_APPS.md) adds a
Worker-authored middle opt-in and required evidence, separate from this original migration.
New missing-bundle outcomes are MEDIA; historical NOT_BUILT remains an accepted retry alias.
The older measurements below retain their original run scope; rollout is not performed here.

# Python CI steps and migration contract

## Scope and starting evidence

CODE READ at `50640c605` (`origin/next`, 2026-11-08): the three candidate stages had
154 lines (apps), 328 (Windows apps), and 619 (SPEC). PM1 migrates the smallest,
`apps_gate.sh`. Contrary to the lane brief's premise that all three were unproven,
`traces/apps_linux_20261107/` and `traces/winapps_20261107/` already contain hardware
records. Those are older measurements, not validation of this port. The existing apps
verdict rules, fake cases and recorded-output graders remain in force. SPEC's stage
still declares that it has had no box run (see [SPEC gate](V3_SPEC_GATE.md)); Windows
apps retains its own [gate design and measurements](V3_WINDOWS_APPS_GATE.md).

CODE READ: the nvkvm-pv reference sweep mixes provider selection, spend, provisioning,
remote command quoting, guest execution and grading in one 3,229-line shell file alongside
Python planning helpers. PM1 follows the owner decision of 2026-11-08: one execution core
in Python, with actions as functions/classes; planning belongs to the JavaScript Worker.
The reference lessons retained here are pinned ShellCheck 0.10.0, argument-list discipline
(remote shell boundaries require explicit quoting), and checking the actual source tree
before any paid run. PM1 has no paid-run path and does not acquire provider credentials.

## Migration policy (DESIGN, owner decision)

- New stages are Python. New Bash glue stays about 50 lines or less; longer orchestration
  belongs in a Python step, not a growing wrapper around a parallel Python implementation.
- Migrate release-path stages when touched. Keep the command surface and verdict grammar;
  replace the old shell body with a small `exec python3 -I` wrapper after equivalence checks.
- Keep ShellCheck pinned at 0.10.0 for remaining glue; Python migration does not disable it.
- Tests precede a new rule. A mutant of each verdict/guard rule needs at least two distinct
  killing tests. Preserve negative cases, isolated reruns, failure priority and RETRY classes.
- DESIGN: distinguish source reads, experiments, inferences and plans explicitly. Fake tests do not qualify hardware
  behavior; changing an implementation does not refresh the historical hardware records.

## Implemented step contract (CODE READ)

`scripts/kfci/step.py` defines `Step`: `id`, `needs`, `produces`, `estimated_minutes`,
`timeout_seconds`, and `run(ctx) -> StepResult`. Artifact names describe prerequisites
and successful outputs; they are not shell commands or implicit filesystem probes.
`StepResult` has a typed PASS/FAIL/RETRY verdict. RETRY requires a `RetryClass`; PASS
and FAIL cannot carry one. Codes are PASS 0, FAIL 1, RETRY 3, with USAGE 2 for the
legacy CLI. RETRY never counts as a successful artifact producer. No automatic retries
are performed by the executor.

`Context.emit` flushes one line immediately. Steps write existing stage rows such as
`APP_ROW` through this API. `Runner` adds `KFCI_START`, `KFCI_STEP_START id=...`, one
`KFCI_ROW id=... result=... [class=...]`, `KFCI_STEP_EXIT id=... rc=...`, a summary and
`KFCI_EXIT rc=...`. A budget guard emits a RETRY TIME row without a step start/exit pair,
because that step did not run. The returned summary dictionary records every attempted
or guarded step and its outcome; a stopped assignment never becomes PASS.

CODE READ: admission requires the step's estimated minutes to fit before the assignment's
budget minus its tail reserve. The context's monotonic deadline is the earlier of that
boundary and the step timeout. Every subprocess is bounded by the remaining context time;
a PASS returned after the deadline is converted to RETRY TIME. Exceptions become RETRY
OPERATOR with an exit line. Estimates are declarations, not measured runtimes. Python
step code must cooperate with the context; this is not a sandbox that can interrupt an
arbitrary infinite Python loop. Supervisor SIGTERM/SIGINT unwinds owned subprocesses. The cancellation exception
is deliberately distinct from `InterruptedError`, which Python selectors swallow;
the cancellation tests killed a mutant that reintroduced that bug.

`scripts/kfci/proc.py` accepts only argument lists, uses no `shell=True`, starts a new
session, applies a named environment allow-list, drains stdout/stderr in fixed chunks,
and captures at most 64 KiB by default (tail capture, with a truncation flag). Optional
file logging streams bytes without retaining them in memory. Timeouts send TERM then
KILL to the owned process group; callback exceptions also clean up that group. The
helper never uses process-name matching or kills an unrelated process. A child that
creates its own session escapes this group contract; legacy provisioning helpers are
not reimplemented by this port. Calls into remaining Bash helpers use fixed executable
paths and separate arguments; guest text is parsed, never executed as Python or shell.

`scripts/kfci/catalog.py` registers explicit steps, validates an assigned order against
needs/produces, and provides a tiny local topological resolver. Unknown steps, missing
products, cycles and ambiguous producers are errors. An explicitly requested producer
resolves ambiguity; otherwise the caller must choose. The Worker order is never
replanned. `scripts/kfci/testing.py` provides `FakeClock`, `FakeStep` and `FakeProcess`.

## Assignment contracts (CODE READ; deployment DESIGN)

CODE READ, reconciliation audit on 2026-11-08: `scripts/kfci/planner_assignment.py`
interprets the 13 assignment-reachable definitions copied unchanged from PR #310 at
commit `ca01ec8c9d5e7d2b8009d133d5d0858783502cff`, in
`ci/vast-broker/planner/schemas/contract.schema.json`. The full source contract SHA-256 is
`1027555309f8731cf2349f0550d6d3fa7d9150c72f61cc03c4cbcdf013c1359e`.
The local subset is `scripts/kfci/schemas/planner-assignment.schema.json`; it adds only
its root reference and provenance comment. This replaces PM1's earlier uncommitted J2
snapshot. It includes ABI metadata, port capability, seed-server and execution-sequence
fields. PM1 changes no Worker source. Validation uses stdlib and local references only;
input remains capped at 1 MiB with finite JSON and structural bounds. DESIGN: pin both
reviewed revisions together before deployment; later J2 protocol changes need another audit.

CODE READ: J2 assignments carry `runs`, with unique instance `id` values and registry
`run_id` values, driver pin references, scoped artifact `needs`/`produces`, per-run
minutes, and a box reservation in minutes. The executor preserves this order and repeated
run types. It validates every registered run and scoped dependency before any action.
Registered artifact names must match the bases of the scoped keys. Contexts expose
assignment/instance ids and the requested driver pin ids; the executor does not invent
driver swaps. The Worker retains pricing, coverage and admission decisions. Proposed
assignments, incomplete/unpinned drivers, unknown run ids, and seed operations without
migrated seed support are refused before work. Only registered implementations can run;
a complete current plan still contains unmigrated actions and is therefore refused.
The current wire contract is accepted only when `seed_server` is null and
`execution_steps` is exactly the ordered run-instance sequence. Seed-serving and wait
operations are unsupported and rejected before any step starts; they are never skipped.
ABI metadata is retained as data, without implementing driver installation.

CODE READ: the J2 reservation bounds the assignment, and admission uses the larger of
the registered implementation estimate and the assigned run minutes. The J2 Linux app
aliases `linux_apps_full` / `linux_apps_middle` match its `kf3_bins` / `open_image`
prerequisites. They use the same AppsGate implementation, deriving a stable safe tag
from assignment/instance ids when absent and using the configured evidence directory.
The supervisor must supply a reservation that reflects its remaining box time; this
library does not acquire a box or infer elapsed provider time.

CODE READ: `scripts/kfci/assignment.schema.json` and `scripts/kfci/assignment.py` also
retain the smaller local convenience format below. It has explicit preexisting inputs
and optional local dependency resolution. It is not the J2 wire format:

```json
{
  "version": 1,
  "id": "local_apps",
  "budget_seconds": 3600,
  "reserve_seconds": 120,
  "inputs": ["linux_apps_ready"],
  "steps": [
    {"id": "apps_gate", "parameters": {"tag": "local_apps", "prov": "/tmp/kfci-evidence", "which": "middle"}}
  ]
}
```

CODE READ, local format: required fields are version, id, budget_seconds and a nonempty ordered
steps list (up to 256). Reserve and inputs are optional. Unknown envelope/entry fields,
duplicate JSON keys, duplicate step ids, invalid artifact/step names and nonfinite or
nonpositive budgets are refused before work. Reserve must be less than the budget.
Step-specific parameters are data, interpreted by the registered step only; assignments
cannot supply executable code. Inputs such as `linux_apps_ready` are the supervisor's
statement that provisioning has already completed. This port does not manufacture a
build/provision step to satisfy it automatically.

## Three launch modes (CODE READ; deployment DESIGN)

All modes accept either validated assignment format and call the same `Runner`;
imports work with isolated Python and stdlib only. Local `--resolve` applies only to
the convenience format; a J2 plan is never resolved or reordered here:

```bash
# Direct: local assignment file; optional --resolve adds local dependencies.
python3 -I scripts/kfci/main.py direct assignment.json
# Worker on Vast: supervisor sends one assignment on stdin.
python3 -I scripts/kfci/main.py worker < assignment.json
# Non-Vast daemon: supervisor supplies one assignment JSON per line, serially.
python3 -I scripts/kfci/main.py daemon < assignments.jsonl
```

CODE READ: daemon mode persists until EOF, prints a `KFCI_RESULT` JSON summary per
assignment, and returns 0 on clean EOF even if an assignment returned RETRY or FAIL.
Consumers must grade the per-assignment rows/results. A malformed assignment stops the
stream with USAGE 2. The supervisor owns transport, authentication, assignment acquisition
and host lifecycle. No HTTP polling, rental, Worker network access or scheduling is
implemented here. DESIGN: the Worker builds the complete per-box plan; a non-Vast
supervisor may deliver the same plan over a pipe; a local operator may use the resolver.

## Linux apps migration (CODE READ)

The ten-line shell wrapper preserves `apps_gate.sh <tag> <prov_dir> [full|middle]`
and invokes the Python entry point's `apps-gate` command. This compatibility command
runs the same `AppsGate.run(ctx)` but omits the generic KFCI envelope so current payload
and report parsers receive the existing stage transcript. The production catalog also
exposes it as `apps_gate`, needing `linux_apps_ready` and producing `linux_apps_checked`.
Its estimate is 50 minutes; its maximum step timeout allows optional legacy provisioning
plus the matrix budget. The existing payload still controls whether the stage is selected;
the migration does not enable it or change tier mapping.

**CODE READ, DX1 integration, 2026-11-08:** `scripts/kfci/apps_gate.py` invokes the
trusted `app_policy.py linux` helper with isolated Python and only `BASE_ENV`, emits
its `APP_POLICY` rows before bundle preflight, and refuses policy ids in any gate
list as `RETRY class=BAD_LIST`. Failed, truncated, empty or malformed policy output
also refuses the list. The existing ASCII name/known-app checks, authoritative
isolated result, supported-app failure priority and credential-free supervisor
contract below remain in force. The policy helper gets no legacy namespace exceptions.

CODE READ: gate lists, the `KF_APPS_*` knobs, provisional bundle checks, optional build/copy/
provision chain, matrix argument order, per-boot and per-app timeouts, log locations, and
result-file precedence are retained. `APPS_STAGE_LIST`, `APP_TRY` (300-character bound),
`APP_ROW` (120-character note bound), failure names, counts, verdicts and exit lines keep
their grammar. The isolated file's last matching row wins over a batched result. FAIL
outranks NOT_RUN/time shortage; missing rows become TIME only on timeout exits 124/137,
otherwise BOX; complete passing rows with a nonzero matrix exit become OPERATOR.
The explicit legacy environment allow-list includes the payload's `BASH_ENV` and Xid
hook variables; the product configuration namespaces `KF3_` and `KAYFABE_` are
also explicitly allowed, so existing diagnostic/build flags continue to reach the
remaining helpers. Unrelated environment keys are not forwarded.

DESIGN — required supervisor contract: supply a credential-free execution environment.
Do not put secrets or bearer capabilities in `KF3_*`, `KAYFABE_*`, `BASH_ENV`, or files
made available to the box. These prefix exceptions preserve product configuration;
they do not recognize arbitrary secrets and are not a universal credential filter.
`BASH_ENV` and legacy helpers are trusted supervisor-authored code, never guest or
assignment input. Worker/daemon enablement requires the supervisor to strip provider,
Worker and Access capabilities before delivery. This library is not a sandbox for a
credential-bearing caller.

CODE READ: progress rendering is separate from authoritative grading. Result input is
read in chunks of at most 65,537 bytes; oversized lines are drained without retaining
their remainder. An oversized or NUL-bearing matching app row becomes invalid evidence
and cannot produce PASS; an unrelated progress row does not determine an app's verdict.
`APP_TRY` text is bounded to 300 UTF-8 bytes. Note content is bounded to 120 UTF-8 bytes
before the legacy failure prefix, without cutting a multibyte character. Control bytes
and Unicode line separators in display text become spaces. These display sanitizations
are deliberate exceptions to unrestricted byte equivalence with Bash. Bash strips NUL
in command substitution; Python deliberately refuses NUL-bearing authoritative evidence.
The list preflight splits only at LF and uses ASCII whitespace, retaining CR inside
comments and the legacy rejection of NBSP,
EM SPACE and NEL before comments. Known app FAIL retains priority over missing evidence
and malformed unrelated progress. Unknown/missing evidence cannot create a PASS.
Unreadable files, invalid process limits and other operational errors still fail closed;
this is not a guarantee against arbitrary host I/O failure after a process has started.
Legacy provisioning can create detached processes; its hardware lifecycle is unverified
here. The frozen old Bash body remains only in
`scripts/ci/fixtures/kfci/apps_gate_legacy.sh.txt` as a test oracle.

## RV2 follow-up validation

MEASURED, PM1 RV2 GPU-free run on 2026-11-08: `python3 -m unittest discover -s scripts/ci
-p 'test_kfci*.py'` passes 161 tests. The paired old/new controls include CR in failing,
passing and unrelated progress notes, oversized unrelated progress before/after the
failure, NUL verdict rejection, Unicode list preflight and byte-bounded UTF-8 display.
Two controls pass both transcripts through the unchanged payload grader and retain
its `apps` failure with `apps_passed=1`, rather than `apps_retry_operator`.
The 177 existing apps/report cases pass. `python3 scripts/ci/kfci_mutants.py` has a
139-test green baseline and kills all 56 mutants with at least two distinct methods
per mutant, including end-to-end failure priority. Evidence:
[`validation.json`](../../traces/kfci_pm1_rv2_20261108/validation.json) and
[`kfci_mutants.json`](../../traces/kfci_pm1_rv2_20261108/kfci_mutants.json).
These counts overlap and are not additive. No hardware qualification is implied.

## Initial GPU-free validation (before RV2)

MEASURED, 2026-11-08: the initial Linux apps module ran 64 tests, all passing;
the same 64 pass after the migration. The final new suite runs 134 tests:
75 core/process/image-copy cases, 39 J2 schema/execution cases, and 20 migration
cases (including the inherited stage cases). The report suite passes 113 tests.
The new mutant baseline passes 114 tests; the existing payload mutant baseline
passes its 496 tests. All 48 new mutants and 19 existing apps/Windows-apps/payload-hook
mutants are killed by at least two distinct test methods each. Counts overlap
between suites; they are not a combined unique-test total. Detailed evidence:
[`kfci_mutants.json`](../../traces/kfci_pm1_20261108/kfci_mutants.json),
[`payload_apps_mutants.json`](../../traces/kfci_pm1_20261108/payload_apps_mutants.json).

MEASURED, PM1 GPU-free run on 2026-11-08 at commit `22c98880d`:
`python3 -m unittest discover -s scripts/ci -p 'test_*.py'` ran 2,104 tests
with two skips and no failures. Within that run, the equivalence cases in
`scripts/ci/test_kfci_apps.py` feed the same fake APPRES transcript to old Bash
and new Python, comparing rows, order, verdict and exit status; timestamps and elapsed
seconds are the only normalized fields. Cases include isolated reruns, all verdict
kinds, missing/duplicate rows, notes, invalid lists, budget exhaustion and GNU timeout's
invalid-duration exit. Tests preceded implementation (expected missing-module/format
failures). The comparisons caught omitted environment forwarding and a timeout-parser
classification difference; both were corrected with the original verdicts retained.

MEASURED, PM1 GPU-free run on 2026-11-08 at commit `22c98880d`:
the same full Python discovery command ran `scripts/ci/test_kfci.py` and
`scripts/ci/test_kfci_planner.py`. Their cancellation tests cover SIGTERM/SIGINT with a fake matrix, require its
owned process group to stop, and check the last exit line. Image-copy tests cover
private/group modes and umask with fake files. J2 parity tests cover JavaScript ASCII
digits, strict end anchors and UTF-16 string-length bounds; the Python interpreter
matches those semantics rather than accepting a broader Python regex/length domain.

CODE READ: `scripts/ci/kfci_mutants.py` runs an unchanged-copy baseline and isolated
source mutations, checks exact-once anchors and syntax, clears private bytecode caches,
reports distinct failing test methods, and fails below two killers. The newline-admission
variant disables both the schema and independent marker guards: removing only a redundant
guard would leave the accepted input set unchanged. Existing payload mutants remain in
`scripts/ci/payload_mutants.py`; its private fixture now includes the package required by
the wrapper. CI discovery runs the new tests and its GPU-free helper job runs the new
mutants. No baseline or pass threshold was weakened. The inventory below is checked
against tracked Bash sources; ShellCheck 0.10.0 passes all box scripts after two small
preexisting lint findings were corrected (exit trap structure and an unused variable).

## Bash inventory and proposed order

CODE READ, 2026-11-08: the table includes every tracked `.sh` file or Bash-shebang fixture over 150 lines in the
PM1 tree (147 files, including the frozen Bash-shebang equivalence fixture), plus apps_gate, which shrank from 154 to 10 lines. Counts include
comments and blank lines. DESIGN: order 1 is this port; 2 media/apps support and Windows apps;
3 SPEC; 4 Windows lifecycle/gate; 5 shared workload stages; 6 provisioning/boot/build;
7 other active helpers when touched; 8 diagnostic/test-only scripts if revived;
9 frozen archive and recorded trace scripts, which remain immutable references unless
explicitly reactivated. Proposed names are scoped Python step/function names, not promises
that an archive will be executed or migrated. No historical source is edited for this table.

| Bash file | Lines | Proposed Python steps/functions (DESIGN unless marked implemented) | Order |
|---|---:|---|---:|
| `archive/nvkvm/scripts/build_qemu.sh` | 272 | `reference_only.archive.nvkvm.scripts.build_qemu` | 9 |
| `archive/nvkvm/scripts/stage_guest_libs.sh` | 346 | `reference_only.archive.nvkvm.scripts.stage_guest_libs` | 9 |
| `archive/nvkvm/src/qemu/build_qemu.sh` | 188 | `reference_only.archive.nvkvm.src.qemu.build_qemu` | 9 |
| `archive/nvkvm/tests/validate.sh` | 1848 | `reference_only.archive.nvkvm.tests.validate` | 9 |
| `archive/nvkvm/tools/abi_derive.sh` | 264 | `reference_only.archive.nvkvm.tools.abi_derive` | 9 |
| `archive/scripts/bench/e0_isolate_witness.sh` | 321 | `reference_only.archive.scripts.bench.e0_isolate_witness` | 9 |
| `archive/scripts/bench/e2_doorbell_witness.sh` | 240 | `reference_only.archive.scripts.bench.e2_doorbell_witness` | 9 |
| `archive/scripts/bench/relaxation_inert_gate.sh` | 172 | `reference_only.archive.scripts.bench.relaxation_inert_gate` | 9 |
| `archive/scripts/bench/single_store_e36_boot.sh` | 160 | `reference_only.archive.scripts.bench.single_store_e36_boot` | 9 |
| `archive/scripts/bench/single_store_e6_boot.sh` | 230 | `reference_only.archive.scripts.bench.single_store_e6_boot` | 9 |
| `archive/scripts/bench/w266_grade.sh` | 188 | `reference_only.archive.scripts.bench.w266_grade` | 9 |
| `archive/scripts/bench/w266_run.sh` | 153 | `reference_only.archive.scripts.bench.w266_run` | 9 |
| `archive/scripts/bench/w267_grade.sh` | 258 | `reference_only.archive.scripts.bench.w267_grade` | 9 |
| `archive/scripts/bench/w267_run.sh` | 180 | `reference_only.archive.scripts.bench.w267_run` | 9 |
| `archive/scripts/bench/w268_grade.sh` | 341 | `reference_only.archive.scripts.bench.w268_grade` | 9 |
| `archive/scripts/bench/w268_run.sh` | 217 | `reference_only.archive.scripts.bench.w268_run` | 9 |
| `archive/scripts/bench/w269_run.sh` | 183 | `reference_only.archive.scripts.bench.w269_run` | 9 |
| `archive/scripts/bench/w270_run.sh` | 240 | `reference_only.archive.scripts.bench.w270_run` | 9 |
| `archive/scripts/bench/w271_run.sh` | 315 | `reference_only.archive.scripts.bench.w271_run` | 9 |
| `archive/scripts/bench/w274_run.sh` | 159 | `reference_only.archive.scripts.bench.w274_run` | 9 |
| `archive/scripts/bench/w274b_run.sh` | 166 | `reference_only.archive.scripts.bench.w274b_run` | 9 |
| `archive/scripts/bench/w275_run.sh` | 183 | `reference_only.archive.scripts.bench.w275_run` | 9 |
| `archive/scripts/bench/w276_run.sh` | 320 | `reference_only.archive.scripts.bench.w276_run` | 9 |
| `archive/scripts/bench/w277_run.sh` | 407 | `reference_only.archive.scripts.bench.w277_run` | 9 |
| `archive/scripts/bench/w278_run.sh` | 182 | `reference_only.archive.scripts.bench.w278_run` | 9 |
| `archive/scripts/bench/w279_run.sh` | 233 | `reference_only.archive.scripts.bench.w279_run` | 9 |
| `archive/scripts/bench/w280_run.sh` | 334 | `reference_only.archive.scripts.bench.w280_run` | 9 |
| `archive/scripts/bench/w281_run.sh` | 374 | `reference_only.archive.scripts.bench.w281_run` | 9 |
| `archive/scripts/bench/w282_run.sh` | 488 | `reference_only.archive.scripts.bench.w282_run` | 9 |
| `archive/scripts/bench/w287_run.sh` | 284 | `reference_only.archive.scripts.bench.w287_run` | 9 |
| `archive/scripts/bench/w288n_notifier_run.sh` | 187 | `reference_only.archive.scripts.bench.w288n_notifier_run` | 9 |
| `archive/scripts/bench/w290_cup2.sh` | 181 | `reference_only.archive.scripts.bench.w290_cup2` | 9 |
| `archive/scripts/bench/w290p_run.sh` | 291 | `reference_only.archive.scripts.bench.w290p_run` | 9 |
| `archive/scripts/bench/w297_cup3.sh` | 163 | `reference_only.archive.scripts.bench.w297_cup3` | 9 |
| `archive/scripts/bench/w299_grade.sh` | 209 | `reference_only.archive.scripts.bench.w299_grade` | 9 |
| `archive/scripts/bench/w304_confirm.sh` | 151 | `reference_only.archive.scripts.bench.w304_confirm` | 9 |
| `archive/scripts/bench/w305a_preempt.sh` | 161 | `reference_only.archive.scripts.bench.w305a_preempt` | 9 |
| `archive/scripts/bench/w305b_crit1.sh` | 208 | `reference_only.archive.scripts.bench.w305b_crit1` | 9 |
| `archive/scripts/bench/w308_cup8.sh` | 323 | `reference_only.archive.scripts.bench.w308_cup8` | 9 |
| `archive/scripts/bench/w309_crit1.sh` | 292 | `reference_only.archive.scripts.bench.w309_crit1` | 9 |
| `archive/scripts/bench/w311_run.sh` | 195 | `reference_only.archive.scripts.bench.w311_run` | 9 |
| `archive/scripts/bench/w314_grade.sh` | 217 | `reference_only.archive.scripts.bench.w314_grade` | 9 |
| `archive/scripts/bench/w315_floor.sh` | 252 | `reference_only.archive.scripts.bench.w315_floor` | 9 |
| `archive/scripts/bench/w320_sync.sh` | 154 | `reference_only.archive.scripts.bench.w320_sync` | 9 |
| `archive/scripts/bench/w322_operands.sh` | 234 | `reference_only.archive.scripts.bench.w322_operands` | 9 |
| `archive/scripts/bench/w381_guest_servable.sh` | 205 | `reference_only.archive.scripts.bench.w381_guest_servable` | 9 |
| `archive/scripts/bench/w384_doorbell_latency.sh` | 215 | `reference_only.archive.scripts.bench.w384_doorbell_latency` | 9 |
| `archive/scripts/bench/w384_hook.sh` | 216 | `reference_only.archive.scripts.bench.w384_hook` | 9 |
| `archive/scripts/bench/w385_concurrent_fuzz.sh` | 199 | `reference_only.archive.scripts.bench.w385_concurrent_fuzz` | 9 |
| `archive/scripts/bench/w418_r34_hook.sh` | 253 | `reference_only.archive.scripts.bench.w418_r34_hook` | 9 |
| `archive/scripts/bench/w586_boot.sh` | 169 | `reference_only.archive.scripts.bench.w586_boot` | 9 |
| `archive/scripts/bench/w736_fbstore_run.sh` | 721 | `reference_only.archive.scripts.bench.w736_fbstore_run` | 9 |
| `archive/scripts/bench/w753_known_positives.sh` | 252 | `reference_only.archive.scripts.bench.w753_known_positives` | 9 |
| `archive/scripts/orphan_gate.sh` | 358 | `reference_only.archive.scripts.orphan_gate` | 9 |
| `archive/scripts/run_full_suite.sh` | 921 | `reference_only.archive.scripts.run_full_suite` | 9 |
| `archive/tests/guest/validate.sh` | 2770 | `reference_only.archive.tests.guest.validate` | 9 |
| `broker/selftest.sh` | 456 | `diagnostic.selftest` | 8 |
| `scripts/bench/blockage_hook.sh` | 191 | `helper.scripts.bench.blockage_hook` | 7 |
| `scripts/bench/boot_capture.sh` | 561 | `linux_boot_capture` | 6 |
| `scripts/bench/box/apps_gate.sh` | 10 | `apps_gate (implemented)` | 1 |
| `scripts/bench/box/fetch_bench_data.sh` | 276 | `fetch_bench_data` | 2 |
| `scripts/bench/box/specviewperf_stage.sh` | 619 | `spec_preflight, spec_copy, spec_run, spec_collect` | 3 |
| `scripts/bench/box/test_fetch_bench_data.sh` | 216 | `diagnostic.test_fetch_bench_data` | 8 |
| `scripts/bench/box/trio_stage.sh` | 479 | `trio_host_control, trio_two_guests, trio_grade` | 5 |
| `scripts/bench/box/windows_apps.sh` | 328 | `windows_apps_preflight, windows_apps_run, windows_apps_grade` | 2 |
| `scripts/bench/box/windows_gate.sh` | 907 | `windows_gate_preflight, windows_gate_boot, windows_gate_probes, windows_gate_grade` | 4 |
| `scripts/bench/cup2_hook_procspin.sh` | 225 | `workload.cup2_hook_procspin` | 5 |
| `scripts/bench/cup2_hook_rmtrace.sh` | 186 | `workload.cup2_hook_rmtrace` | 5 |
| `scripts/bench/cup3_hook.sh` | 405 | `workload.cup3_hook` | 5 |
| `scripts/bench/cup3x2_hook.sh` | 377 | `workload.cup3x2_hook` | 5 |
| `scripts/bench/cup8_hook.sh` | 305 | `workload.cup8_hook` | 5 |
| `scripts/bench/cup8bench_hook.sh` | 329 | `workload.cup8bench_hook` | 5 |
| `scripts/bench/display/broker_hook.sh` | 269 | `workload.broker_hook` | 5 |
| `scripts/bench/display/broker_lane.sh` | 226 | `workload.broker_lane` | 5 |
| `scripts/bench/display/hook.sh` | 349 | `workload.hook` | 5 |
| `scripts/bench/display/input_proof.sh` | 192 | `workload.input_proof` | 5 |
| `scripts/bench/display/interactive.sh` | 362 | `workload.interactive` | 5 |
| `scripts/bench/display/unload_hook.sh` | 247 | `workload.unload_hook` | 5 |
| `scripts/bench/gfxset/itemdefs.sh` | 505 | `workload.itemdefs` | 5 |
| `scripts/bench/guest_cuinit_wall.sh` | 205 | `helper.scripts.bench.guest_cuinit_wall` | 7 |
| `scripts/bench/host_xid_watch.sh` | 161 | `host_xid_watch` | 2 |
| `scripts/bench/llm_parity.sh` | 185 | `helper.scripts.bench.llm_parity` | 7 |
| `scripts/bench/perf_compare.sh` | 208 | `helper.scripts.bench.perf_compare` | 7 |
| `scripts/bench/provision_bench_tree.sh` | 456 | `provision_bench_tree` | 6 |
| `scripts/bench/provision_box.sh` | 312 | `provision_box` | 6 |
| `scripts/bench/provision_host_driver.sh` | 295 | `provision_host_driver` | 6 |
| `scripts/bench/rmladder_suite.sh` | 313 | `helper.scripts.bench.rmladder_suite` | 7 |
| `scripts/bench/w392_llm.sh` | 205 | `helper.scripts.bench.w392_llm` | 7 |
| `scripts/bench/windows/glcrash/tdr-run28.sh` | 353 | `diagnostic.tdr_run28` | 8 |
| `scripts/bench/windows/glcrash/tdr-run29.sh` | 354 | `diagnostic.tdr_run29` | 8 |
| `scripts/bench/windows/glcrash/vfio_gl_reference.sh` | 170 | `diagnostic.vfio_gl_reference` | 8 |
| `scripts/bench/windows/host_tools/runners/click/vfio_dvi_reference_click.sh` | 165 | `diagnostic.vfio_dvi_reference_click` | 8 |
| `scripts/bench/windows/host_tools/runners/winprod/windows_broker_prod.sh` | 313 | `diagnostic.windows_broker_prod` | 8 |
| `scripts/bench/windows/host_tools/runners/winprod/windows_broker_prod2.sh` | 313 | `diagnostic.windows_broker_prod2` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run10.sh` | 209 | `diagnostic.tdr_run10` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run11.sh` | 209 | `diagnostic.tdr_run11` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run12.sh` | 211 | `diagnostic.tdr_run12` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run13.sh` | 213 | `diagnostic.tdr_run13` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run14.sh` | 216 | `diagnostic.tdr_run14` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run15.sh` | 221 | `diagnostic.tdr_run15` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run16.sh` | 227 | `diagnostic.tdr_run16` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run17.sh` | 236 | `diagnostic.tdr_run17` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run18.sh` | 259 | `diagnostic.tdr_run18` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run19.sh` | 267 | `diagnostic.tdr_run19` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run20.sh` | 272 | `diagnostic.tdr_run20` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run21.sh` | 276 | `diagnostic.tdr_run21` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run22.sh` | 276 | `diagnostic.tdr_run22` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run23.sh` | 279 | `diagnostic.tdr_run23` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run24.sh` | 310 | `diagnostic.tdr_run24` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run5.sh` | 172 | `diagnostic.tdr_run5` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run6.sh` | 177 | `diagnostic.tdr_run6` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run7.sh` | 179 | `diagnostic.tdr_run7` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run8.sh` | 211 | `diagnostic.tdr_run8` | 8 |
| `scripts/bench/windows/host_tools/tdrhunt/tdr-run9.sh` | 210 | `diagnostic.tdr_run9` | 8 |
| `scripts/bench/windows/regress_matrix.sh` | 182 | `helper.scripts.bench.windows.regress_matrix` | 7 |
| `scripts/bench/windows/win_vm.sh` | 1082 | `windows_media, windows_install, windows_boot, windows_stop` | 4 |
| `scripts/bench/windows/windows_broker.sh` | 312 | `helper.scripts.bench.windows.windows_broker` | 7 |
| `scripts/ci/fixtures/kfci/apps_gate_legacy.sh.txt` | 154 | `test_oracle.apps_gate_legacy` (retained fixture) | 9 |
| `scripts/ci/vast_onstart.sh` | 160 | `helper.scripts.ci.vast_onstart` | 7 |
| `scripts/ci_gates.sh` | 341 | `helper.scripts.ci_gates` | 7 |
| `scripts/display/gop_standin.sh` | 697 | `helper.scripts.display.gop_standin` | 7 |
| `scripts/drivermatrix/sweep.sh` | 538 | `helper.scripts.drivermatrix.sweep` | 7 |
| `scripts/fastguest/build_fast_guest.sh` | 603 | `build_fast_guest` | 6 |
| `scripts/fastguest/run_fast_guest.sh` | 423 | `run_fast_guest` | 6 |
| `scripts/lifecycle_mutation_check.sh` | 296 | `helper.scripts.lifecycle_mutation_check` | 7 |
| `scripts/rpctrace/build_instrumented.sh` | 252 | `helper.scripts.rpctrace.build_instrumented` | 7 |
| `scripts/rpctrace/capture.sh` | 269 | `helper.scripts.rpctrace.capture` | 7 |
| `tools/derive_classes.sh` | 189 | `helper.tools.derive_classes` | 7 |
| `tools/derive_display_layouts.sh` | 264 | `helper.tools.derive_display_layouts` | 7 |
| `tools/derive_hwref.sh` | 244 | `helper.tools.derive_hwref` | 7 |
| `traces/blackwell_adaptive_tier3_20261023/run_kit/arm.sh` | 168 | `reference_only.traces.blackwell_adaptive_tier3_20261023.run_kit.arm` | 9 |
| `traces/blackwell_adaptive_tier3_20261023/run_kit/controller.sh` | 222 | `reference_only.traces.blackwell_adaptive_tier3_20261023.run_kit.controller` | 9 |
| `traces/blackwell_cached_validation_20261023/run_kit/arm.sh` | 171 | `reference_only.traces.blackwell_cached_validation_20261023.run_kit.arm` | 9 |
| `traces/blackwell_cached_validation_20261023/run_kit/controller.sh` | 200 | `reference_only.traces.blackwell_cached_validation_20261023.run_kit.controller` | 9 |
| `traces/blackwell_edge_validation_20261023/run_kit/arm.sh` | 171 | `reference_only.traces.blackwell_edge_validation_20261023.run_kit.arm` | 9 |
| `traces/blackwell_edge_validation_20261023/run_kit/controller.sh` | 199 | `reference_only.traces.blackwell_edge_validation_20261023.run_kit.controller` | 9 |
| `traces/blackwell_fields_phase_20261022/run_kit/arm.sh` | 171 | `reference_only.traces.blackwell_fields_phase_20261022.run_kit.arm` | 9 |
| `traces/blackwell_fields_phase_20261022/run_kit/controller.sh` | 182 | `reference_only.traces.blackwell_fields_phase_20261022.run_kit.controller` | 9 |
| `traces/blackwell_provider_validation_20261023/run_kit/arm.sh` | 168 | `reference_only.traces.blackwell_provider_validation_20261023.run_kit.arm` | 9 |
| `traces/blackwell_provider_validation_20261023/run_kit/controller.sh` | 225 | `reference_only.traces.blackwell_provider_validation_20261023.run_kit.controller` | 9 |
| `traces/blackwell_validation2_20261022/run_kit/arm.sh` | 156 | `reference_only.traces.blackwell_validation2_20261022.run_kit.arm` | 9 |
| `traces/blackwell_validation_20261022/run_kit/arm.sh` | 156 | `reference_only.traces.blackwell_validation_20261022.run_kit.arm` | 9 |
| `traces/blackwell_windows_resume_20261023/run_kit/arm.sh` | 168 | `reference_only.traces.blackwell_windows_resume_20261023.run_kit.arm` | 9 |
| `traces/blackwell_windows_resume_20261023/run_kit/controller.sh` | 231 | `reference_only.traces.blackwell_windows_resume_20261023.run_kit.controller` | 9 |
| `traces/windows_flag_ablation_20261010/host_tools/tdr-run27.sh` | 323 | `reference_only.traces.windows_flag_ablation_20261010.host_tools.tdr_run27` | 9 |
| `traces/windows_flag_ablation_20261010/host_tools/windows_broker_prod3.sh` | 315 | `reference_only.traces.windows_flag_ablation_20261010.host_tools.windows_broker_prod3` | 9 |
| `traces/windows_matrix_20261011/gravity-vulkan/vfio-ring-p0/vfio_launch.sh` | 170 | `reference_only.traces.windows_matrix_20261011.gravity_vulkan.vfio_ring_p0.vfio_launch` | 9 |
| `traces/windows_matrix_20261011/resume-20261011/host-scripts/windows_broker_prod3.sh` | 315 | `reference_only.traces.windows_matrix_20261011.resume_20261011.host_scripts.windows_broker_prod3` | 9 |
