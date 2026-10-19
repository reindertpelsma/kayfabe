# M1 TSG mirror correctness gates

STATUS: NOT GREEN / STOPPED, 2026-10-19 00:10 UTC (02:10 CEST). Lane M1.
Branch: `codex/tsg-mirror-gates-20261019`. Source: author head `c2685412e159534b88097ea824d05fe963be83b1`; no product code changed. Last origin-verified stage before this report: `00ea3054739fe2aa4754b3bdf4dffb30576d664a`. The final evidence head is the commit containing this report; its exact origin verification is recorded in the owner report after push.

## Result (nine lines)

- MEASURED DONE: read the complete common/M1 lane instructions; preregistration pushed before hardware work (`PREREG.md`).
- MEASURED DONE: inspected 62 Linux image candidates read-only; inventory preserved (`inventory-summary.json`, `PART_A.md`).
- MEASURED DONE: both supplied binary MD5s match; candidate-5 cursor7 broker located (`PART_A.md`).
- MEASURED PARTIAL: M1-owned guest overlay/key prepared, but the overlay is unbooted and not fully provisioned.
- MEASURED DONE: targeted release tests **31 passed / 0 failed** (`unit-results.txt`), including 64/65-subcontext boundary and kill-switch models.
- CODE READ NOT DONE: documented provisioning requires a VM boot before Part A can be ready; the owner retains the required shared lock until Part B. This conflicts with the required ordering; options are in `PART_A.md`.
- MEASURED NOT DONE: no hardware gate was executed. All Linux hardware gates and optional Windows extras are **UNRUN**, not a measured mirror failure.
- MEASURED DONE: owner run 3009 remains alive on production 0452613b, READY marker retained; no stop/restart was attempted. GPU lock was never acquired by M1; all M1 mounts/NBD processes are released.
- MEASURED CLEANUP EXCEPTION: `/workspace/m1-target` remains **177 MiB**. Automatic approval review rejected its deletion because “rm -f style commands are not permitted”; the STOP/no-workaround rule was followed.

## Gate and arm record

CODE READ revisions: B = candidate-5 `f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e`, binary directory `f8f9d1a5`, MD5 `c2ff1a02ed33a39e4b26d2dd002e62f6`; M = mirror `0d7ba74389cd85ac65826417ad53f5c0bcd1ef2f`, MD5 `48ab1b70c2cf2c26d2c1287bf7311a9a`.
CODE READ: command/flag cells for unrun rows are the intended commands, **not executed commands**. ON = planned `KF3_TSG_MIRROR=1`; OFF = planned `KF3_TSG_MIRROR=0`; base uses its default flags. Other measurement flags were to remain unset. No Xid zero is inferred from absent runs.

| Stage / command | Arm / rev | Flags | n | Result / verdict | Host Xid delta | Evidence |
|---|---|---|---:|---|---|---|
| Offline disk inventory, QEMU-NBD + mount | existing Linux candidates | read-only, ro,noload | 62 | MEASURED inventory complete; prerequisite hypothesis falsified | N/A, no GPU job | `inventory-summary.json`, `PART_A.md` |
| `cargo test -j2 --release -p kf-qemu tsgmirror` | c2685412e code, docs-only lane changes | release; nice 19; own target; shared dev build lock | 31 tests, 1 invocation | MEASURED PASS 31/31 | N/A, GPU-free fake host | `unit-results.txt` |
| G2 `cuda_ladder.sh host/guest` all four rungs | host / B / M | default / default / ON | 0 each | **UNRUN**: provisioning ordering conflict | UNMEASURED | `PART_A.md` Stop/options |
| G2 `scripts/apps/apps_matrix.sh host/guest ... all` | host / B / M | default / default / ON | 0 each | **UNRUN**: same conflict; app bundle absent in all inspected guests | UNMEASURED | `PART_A.md`, inventory |
| G3 `gfx_suite.sh`; Vulkan/GL applications | host / M / B | default / ON / default | 0 each | **UNRUN**: same conflict; gfx sources/build tools absent | UNMEASURED | `PART_A.md`, inventory |
| G4 `video_lane.sh`, `video_hook.sh` | host / B / M | default / default / ON | 0 each | **UNRUN**: same conflict; default identical-static-ffmpeg prerequisite absent | UNMEASURED | `PART_A.md` |
| UVM / 8+ guest-channel stress | B / M | default / ON | 0 each | **UNRUN** hardware; 65th-cap fake-host tests only are PASS | UNMEASURED | `unit-results.txt`, `PART_A.md` |
| `display/broker_lane.sh run` with candidate-5 broker | M / B | ON / default | 0 each | **UNRUN**: no guest boot | UNMEASURED | `PART_A.md` |
| `display/interactive.sh run` with candidate-5 broker | M / B | ON / default | 0 each | **UNRUN**: no guest boot | UNMEASURED | `PART_A.md` |
| Complete CUDA stage, kill-switch equivalence | M / B | OFF / default | 0 each | **UNRUN** hardware; fake-host kill-switch models PASS | UNMEASURED | `unit-results.txt` |
| Optional Windows browser / Heaven / driver cycle / 15-minute hold | M (throwaway disk) / B where applicable | ON / default | 0 each | **UNRUN**: optional, Linux gates not reached | UNMEASURED | this report |

## Counts, falsifiers, mutations

MEASURED: before/after product change counts are N/A because no product behavior changed. One targeted source validation: **31 passed, 0 failed, 0 ignored; 356 library tests filtered out**. Integration-test executables selected zero tests and are not counted as coverage. No full release suite, v3_gates, fast_suite, clippy, app-matrix, hardware or A/B run occurred in M1. Inherited compiler warnings were present; this run did not assess clippy.

| Mutation | Killing tests | Status |
|---|---:|---|
| None: documentation/evidence only, no product change | N/A | No new mutation claim; author's historical mutation results were not rerun |

MEASURED: the prepared-fat-guest hypothesis was falsified by missing prerequisites in every inspected candidate. Hardware correctness and mirror-OFF equivalence hypotheses remain **untested**. No FPS/latency, sample spread, guest result-set equality, TDR or Xid result is claimed. The mount error was identified as a missing partition-device event, then resolved by waiting for that event; no product timing constant changed.

## Owner decisions

1. INFERRED recommendation: **do not promote mirror to the default launcher** from this lane's evidence; G2/G3/G4 and the Linux broker regression gates remain unrun.
2. CODE READ recommendation: authorize stopping the owner before stock-QEMU provisioning under the shared lock, then rerun the preregistered base/mirror gates; keep the original hard GPU deadline.
3. INFERRED recommendation: retain the mirror diagnostic kill switch and existing rollout state until those gates pass; M1 makes no flag/default change.
4. MEASURED action required: arrange permitted deletion of the 177 MiB M1 target directory; automatic approval rejected M1's attempted deletion and no alternate deletion method was used.

## Remaining verification and morning state

CODE READ / MEASURED: all hardware properties named in the table remain unverified, including the old Vulkan Xid-69 shape, CUDA ASYNC subcontexts with real kernels, video results, many-channel execution, real hardware cap refusal, broker/interactive behavior, and mirror-OFF equivalence. Offline presence of modules/libraries does not prove a boot or a successful workload. Filesystem inventory checked documented locations, not every possible user-defined installation path.

MEASURED: at 00:09:50 UTC, owner QEMU PID 3591325 still executes production 0452613b; launcher READY is 01:40:35 CEST for run 3009. Owner flock PID 3591199 still holds the shared lock. M1 did not stop the owner, so restoring it again would launch a second VM. No M1 GPU lock, VM, CUDA workload, build, mount or NBD attachment remains running. The M1 overlay/key and small inventory logs remain in `/data/codex-M1/` for an authorized resume. The target-directory exception is recorded above. No other lane was messaged, no integration/master merge or force-push was performed, and no owner launcher or owner disk was modified.
