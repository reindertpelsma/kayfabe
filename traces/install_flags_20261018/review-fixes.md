# PR #72 review corrections

**STATUS: RESEARCH, 2026-10-22.** CODE READ: review input is
`origin/claude/review-pr72-20261022:docs/audits/2026-10-22-pr72-review.md`, read in full.
The earlier [submission evidence](README.md) is a historical snapshot; its SDR default-on
proposal and QEMU/command checks do not qualify the revised configuration on hardware.

## Scope and falsifiers

CODE READ: the [revised design](../../docs/design/V3_FLAG_INVENTORY_20261022.md) records
falsifiers before changes. B2 withdraws SDR default-on: production enabled SDR together with
CAPS_PROBE, which discards the authored SDR page. G05 remains open. Only the existing
MIRROR and 256-byte ILUT defaults remain on, retaining exact `=0` research arms.
SDR accepts exact `=1` only. No relay, VA, locking, timeout or polling policy is changed.

MEASURED (2026-10-22, `python3 scripts/ci/debt.py claims`): B1 reproduced exactly
`existing-debt=3758 new=12`; after adding source/command/date attribution, `new=0`.
No debt baseline was edited. The first docs-only repair was pushed as `fc487e743`.

## Tests before changes

MEASURED (2026-10-22, `python3 scripts/bench/windows/test_diagnostic_consumers.py`):
EXPECTED RED commit `9574264a3` reaches **2 passed / 6 failed** before harness fixes.
The failures are two each for winpass RPC, wr-run RPC and current decoder log summaries.
Fixed harnesses pass all 8; the 8 existing launcher tests also pass. CI's existing
`python3 -m unittest discover -s scripts/ci -p 'test_*.py'` now discovers these contracts.
Analysis-only `abort_point.py` / `rpc_diff.py` require logs collected explicitly with
`WIN_FLAGS=KF3_RPC_TRACE=1`; they cannot recover missing RPC output.

MEASURED (2026-10-22, `cargo test --release -j2 -p kf-qemu --lib`): the behavior-preserving
selection extraction passes all **378** pre-existing library tests. The initial 60-second
Cargo-lock wait expired without starting a build. After the prior holder exited, a fresh
bounded attempt acquired the lock normally; no held lock was broken or bypassed.

MEASURED (2026-10-22, `cargo test --release -j2 -p kf-qemu --lib production_color_defaults_tests`):
EXPECTED RED commit `b39532504` reaches **12 passed / 11 failed** before SDR rollback.
After rollback, 22 passed and one new test was overbroad: it expected the OLUT constructor
probe to succeed on GB20x. That step stopped. CODE READ: the existing
`caps.rs::olut_probe_changes_only_its_surface_load_bit_in_every_display_cell` instead
requires `NVCA73_POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD` refusal. The options were to preserve
that contract (chosen) or add new family behavior (outside scope). Test-only commit
`5383adb83` requires that exact refusal; rerunning before the rollback still gives
**12 passed / 11 failed**. No product capability or failure threshold was weakened.

MEASURED (2026-10-22, same colour command): with SDR restored to opt-in, **23 passed / 0 failed**.
The suite calls the real pre-GPU `display_color_config` used by build, covering both driver
versions and every derived family for plain defaults and the four constructor arms (including
the explicit GB20x OLUT refusal). It also covers explicit SDR/constructor conflicts, authored
SDR selection, default-plus-CAPS_PROBE, SDR-plus-CAPS_PROBE, constructor-plus-CAPS_PROBE and
unsupported-class fallback. All probe environment variables are cleared in isolated children.

CODE READ: `pc_pool_experiment.py` and `host_tools/runners/boundary-tools/pc_boundary_experiment.py`
retain their existing standalone-probe invocations. Restoring SDR opt-in makes those arms work
with SDR unset, without requiring `SDR_COLOR=0`. All four tool guides state the unset default,
explicit SDR conflict, CAPS_PROBE page replacement and blanket method refusal. Hardware
execution of these scripts is not claimed by these configuration tests.

CODE READ: two structural tests in `scripts/ci/test_display_color_wiring.py` supplement the
functional Rust tests: build must destructure the shared selection exactly once and use its
caps/constructor/processing arms without a later local override. They guard composition wiring;
they do not execute GPU bring-up or establish a guest boot.

## Final verification

MEASURED (2026-10-22, release Rust regressions and GPU-free Python contracts): each
mutation below is applied separately and restored in `finally`; Rust accounting uses the
final outer 23-test summary, never duplicate child output. The restored colour suite passes
23/23. The six original launcher mutants were rerun against the revised branch, each with
its actual exported environment checked before running the tests.

| Mutant | Distinct failing tests |
|---|---:|
| Restore SDR default-on | 11 |
| Replace selection call-site `sdr_color_on()` with false | 6 |
| Bypass CAPS_PROBE selection | 3 |
| Bypass constructor-probe refusal selection | 9 |
| Restore MIRROR default-off | 3 |
| Restore ILUT default-off | 3 |
| Shadow selected SDR at build call-site (structural guards) | 2 |
| Remove winpass RPC opt-in | 2 |
| Remove wr-run RPC opt-in | 2 |
| Revert wr-run MIRROR log matching | 2 |
| Revert wr-run ILUT log matching | 2 |
| Restore main launcher RPC default | 2 |
| Restore main launcher BAR0_TRACE default | 2 |
| Restore main launcher MAPLOG default | 2 |
| Restore main launcher DISPLAY_METHOD_TRACE default | 2 |
| Restore dead TIMER_MAP default | 2 |
| Restore hitch dead lifecycle extras | 2 |

MEASURED (2026-10-22, `python3 -m unittest discover -s scripts/ci -p 'test_*.py'`):
**57 passed**, including 16 launcher contracts and two build-wiring guards. All four
revised INSTALL Bash blocks and changed launchers pass `bash -n`; changed Rust files pass
rustfmt, and the edited local documentation links resolve. Provisioning/install commands
were not executed. Hardware n=0; no new patched-QEMU/guest-display result is claimed.

MEASURED (2026-10-22, `KAYFABE_NO_KVM=1 cargo test --release -j2` over the same 22
kf-* packages as the initial submission): **3772 passed / 0 failed / 20 ignored**, 212
result groups, explicit final exit 0. Initial submission: 3757/0/20; the 15 added Rust
tests account for the difference. Library scope: kf-qemu 378 -> 393 passing tests.
No ignored test was removed, enabled or relabelled. All builds used nice 19, two Cargo
jobs, a lane-owned target and the shared build lock; the disk guard passed before each build.

MEASURED (2026-10-22, `cargo clippy --release -j2 -p kf-qemu --all-targets --message-format=json`):
**348 -> 348 warnings, 0 added**, normalized by package/lint/primary file/message, excluding
line-number shifts. `python3 scripts/ci/debt.py clippy --log <that-json-log>` reports
`existing-debt=247 new=0`; the partial crate scope does not establish removal of workspace debt.
`python3 scripts/ci/debt.py claims` reports `existing-debt=3758 new=0`. No baseline file changed.
CODE READ: product/test/script/tool source is unchanged after commit `b49b245c0`; subsequent
commits contain documentation/evidence only and use `[skip ci]`. The code fix was pushed
separately with normal CI. The PR remains draft; hosted CI completion is not claimed here.

MEASURED (2026-10-22, process executable and build-holder audit): all lane build/test jobs
have ended and their shared Cargo-lock holders exited. No GPU/VM lock was taken, no GPU
was used, and no guest was booted. Cleanup is **NOT DONE**: automatic approval review rejected
the forced-removal command; under the lane's stop-on-refusal rule that step stopped without
substituting another deletion command. The lane-owned Cargo target (648 MiB) remains for
owner cleanup. The earlier QEMU source/build directories remain absent. Private text logs
and the report identify the retained target for the owner; no binaries are committed.
