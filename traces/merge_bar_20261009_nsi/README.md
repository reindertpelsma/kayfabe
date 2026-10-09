# Merge bar for claude/passthrough-nsi-nogate-20261008, 2026-10-09 (run nsi2) - INCOMPLETE

**STATUS: RESEARCH, 2026-10-09 - the merge bar on `4b8399d195f2ff471066afa2c0ecee10f53c84a0` is INCOMPLETE. The box (vast 54920121) was destroyed at the owner's request while the guest app matrix was running. The `merge_check.sh` stage and the CUDA ladder finished and passed; the host app matrix finished; the guest app matrix did NOT finish and has no result. This is not a merge-bar pass.**

Exact revision tested: `4b8399d195f2ff471066afa2c0ecee10f53c84a0` (branch `claude/passthrough-nsi-nogate-20261008`; code identical to `d00c9f2b`, the last commit is evidence and docs only). On the box, `git -C /root/kayfabe rev-parse HEAD` printed that hash, `merge_check.sh` logged `HEAD=4b8399d195f2ff471066afa2c0ecee10f53c84a0`, and the lanes used `/workspace/bench/kf3-bins/4b8399d1/qemu-system-x86_64` (the lanes script printed `KF3_BIN=` that path; the ladder and apps picked the binary of the checkout revision).

Box: vast 54920121 (KVM template image, RTX 3080 Ti, driver 580.159.04, 15 vCPU, about 23 GB RAM, 146 GB root disk, offer 54852949 at 0.216 USD/h). Box 54805775 named in the task no longer existed. A first provisioning at `7519cd89` was stopped by the coordinator before any stage ran (it was not the final commit); the same box was reused for `4b8399d1`.

## What ran on 4b8399d1 (all `[measured, nsi2 at 4b8399d1, 2026-10-08/09 UTC]`)

| stage | result |
|---|---|
| kf-* crate tests (`cargo test --no-fail-fast`) | `TESTS passed 2448 failed 0` |
| v3 gates | `V3_GATES_SUMMARY pass=9 fail=0` |
| kf3 build of this revision | `KF3_RC=0` |
| bare-metal suite | `BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30` |
| fast guest rebuild | `FG_RC=0` |
| fast guest suite | `FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30` |
| birth census | `BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=11 all PRIVILEGED_CHANNEL=0 privilege=USER` |
| `merge_check.sh` exit line | `EXIT rc=0 2026-10-08T22:43:34+00:00` (started 22:29:46, 14 minutes) |
| CUDA ladder, host, 1 rep | cup2, cup3, cup8, cup8bench all PASS (4/4) |
| CUDA ladder, guest (fat guest, kf3), 1 rep | cup2, cup3, cup8, cup8bench all PASS (4/4) |
| app bundle build, host runtime, guest image provisioning | each `RC=0` (bundle: every `BUILD_*` line `ok`) |
| app matrix, host (bare metal) | `host.res` 71 rows with `verdict=PASS`, none other (3 extra `APPDIG` digest lines) |
| app matrix, guest | NOT FINISHED: killed with the box. Last progress seen: one app per boot, boot 46 of the first pass (`memcpy2d`) at 2026-10-09T00:01 UTC, every boot before it `rc=0`; no per-app verdicts were collected and the isolated re-run phase never started |

## What is missing, and why it matters

- No guest app-matrix result at `4b8399d1`: the baseline at `3f4904ca` was 67/71 (61/65 apps + 6/6 probes; four UVM failures). Nothing here confirms or refutes that for this revision.
- No log files were brought back before the box went. The numbers above are the verdict lines read from the box during the run (copied from the session's command output), not archived logs. Per-arm ladder timings and the ledger columns are not archived. [inferred] The lines are accurate because they were read with `grep` from the run's own logs, but they cannot be re-checked from this directory.

## GitHub CI for 4b8399d1 (run 37853710325, checked 2026-10-09)

Conclusion `success`: `stable` (build, test, clippy, fmt, boundary) success, `aarch64` success, `firmware` success, `slow` (soaks) skipped. No pull request exists for the branch.

## Cost

Box 54920121 ran from about 2026-10-08T22:14 UTC (rented, boot about 20 minutes) until the owner's destruction after about 00:05 UTC on 2026-10-09: roughly 2 hours at 0.216 USD/h, about 0.45 USD in total (estimate from the offer price, not a vast invoice).

## Rerun notes

- The VM needs about 20 minutes to boot; ssh uses `public_ipaddr` and the `22/tcp` HostPort, not the `ssh_host`/`ssh_port` the CLI shows.
- Stage times on this box: provision 8.8 min; `merge_check.sh` 14 min; ladder host 0.2 min, guest 3.4 min; `build_bundle` 9.8 min; `setup_side` 4.1 min; guest app provisioning 9.6 min; host apps 12.5 min; guest apps at least 40 min and still going at boot 46 (roughly one app per minute in the first pass, then the isolated re-runs).
