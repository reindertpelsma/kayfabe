# L5 GPU-free validation

STATUS: ANSWERED (GPU-free scope), 2026-10-18. Hardware is NOT RUN.
Base `0dbcc8a1463f0ae3ae58ed867373a603b69e6ba8`; final Rust code
`c81bd42d42c6a7c94fce46a2787e9246d4a09168` (only docs/scripts follow).

MEASURED: both revisions used the same command, on the dev box, with a separate
lane-owned target directory and a disk-space check before every cargo invocation:

```sh
CARGO_TARGET_DIR="$L5_TARGET" nice -n 19 cargo test -j2 --release -p kf-qemu -p kf-mem -p kf-disp
```

| Crate | Base library pass | Final library pass | Integration pass, both | Ignored, both |
|---|---:|---:|---:|---:|
| kf-qemu | 337 | 339 | 21 | 0 |
| kf-mem | 181 | 183 | 5 | 3 |
| kf-disp | 229 | 229 | 18 | 3 |
| Total | 747 | 751 | 44 | 6 |

MEASURED: full totals **791 → 795 passed**, zero failed at both endpoints;
6 ignored at both; doc tests run, zero cases. No hardware-dependent ignored
case was represented as passing.

## Failing-first commits and mutations

MEASURED:

- S1: `cpuwin::tests::sink_all_removes_the_boot_seed_before_the_first_bar1_change`.
- S2: `cpuwin::tests::sink_all_removes_a_reseeded_boot_view_before_a_later_reboot`.
- U1: `bar1phys::tests::a_stale_unload_does_not_reseed_a_new_lifes_scratch`.
- U2: `bar1phys::tests::a_stale_unload_preserves_a_new_life_with_a_mapping_above_the_boot_range`.

| Change / stage | Result |
|---|---|
| Seed tests alone, EXPECTED RED `1b05fdbb4` | S1/S2 fail; the two prior `sink_all_` cases pass |
| Seed fix `8eb188452` | S1/S2 pass; full final kf-mem library 183 pass / 3 ignored |
| Manually remove seed retirement from `sink_all` | S1 and S2 both fail (2 kills) |
| Stale tests alone, EXPECTED RED `db0ab961b` | U1/U2 both fail; both wrongly show a placed seed |
| Typed stale refusal fix `c81bd42d4` | all 18 bar1phys tests pass, including ordinary host refusal |
| Manually treat StaleLife as an ordinary host refusal again | U1 and U2 both fail (2 kills) |
| Restore both fixes; complete three-crate run | 795 pass / 0 fail / 6 ignored |

MEASURED: every mutant was restored before the final complete run. The ordinary
host-refusal case still re-seeds and logs its refusal; the stale case cancels the
restore. Existing tests for unload/display life reset, device reset/reboot/S3,
seed reuse, re-init models, scrub decisions and ordering remain green. This does
not establish real-driver/hardware behavior.

## The one non-endpoint failure

MEASURED: the first full post-seed-fix kf-mem library run was 182 pass / 1 fail /
3 ignored. The failing, unchanged test was
`batch::tests::the_act_thread_waits_for_a_chunk_not_for_the_operation`:

```text
longest lock WAIT 100001 us; longest whole hand-over 100008 us
longest hold 86505 us (2048 entries)
the act thread waited 100001 us for a ledger lock
```

MEASURED: the unchanged isolated rerun passed (1/1), with lock WAIT 53333 us.
The next full three-crate run passed, and the final run after the stale fix also
passed (795 total). INFERRED: shared-host scheduling can affect that wall-clock
oracle; the isolated pass does not prove the cause. No timeout, bound, interval,
priority, test oracle or ignored annotation was changed. The complete failure
and build logs remain in L5's local lane logs; only this small excerpt is public.

## Static checks and scope

- MEASURED: `bash -n` and ShellCheck passed for both new shell scripts.
- MEASURED: Python AST parsing passed for all four Python scripts; none was imported
  or executed. PowerShell was reviewed as text; no PowerShell parser was installed.
- MEASURED: staged diffs passed whitespace and credential/prohibited-path/global-IPv4
  pattern scans before every commit. This is a pattern scan, not a proof of absence.
- CODE READ / MEASURED lexical inventory: boot-layer base delta and L5 Rust delta
  add zero `unsafe` syntax and zero environment-variable reads. Worker/lock paths
  were read; no added vCPU blocking path was identified. No runtime latency claim.
- NOT MEASURED: hardware, store-adapter integration, broker presentation, Linux/Windows
  actual unload/reload/reboot, all-store pattern readback, NEW OS timing. **n=0,
  spread unavailable.** The requested run-3081 log was retrieved read-only; no host
  VM/GPU action or host file write was performed by L5.
