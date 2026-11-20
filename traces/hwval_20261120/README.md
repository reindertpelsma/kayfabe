# Hardware validation of `next` 32409d29b (guest-influence caps #540, ogkm-generated guest-ingress gate #545, driver refusal message #546), RTX 4090, 2026-11-20

STATUS: LIVE, 2026-11-20 (lane hwval). One Vast rental, no GPU run existed for these changes before it. Labels: MEASURED = a line in a file of
this directory (named); INFERRED = reading of code or of a table, no run.

## Run

| item | value |
|---|---|
| revision under test | `32409d29b9ee833afbaf9b4267a3583800e1a178` (`origin/next` at 2026-11-20 21:26 UTC, merge of #533). It contains #526 #527 #533 #534 #539 #540 #541 #545 #546. Binaries `kf3-bins/32409d29/` |
| baseline revision | `6524df051ffe594499a91af227c194ff8f866093` (`next` just before #545, with #540); built on the same box, `kf3-bins/6524df05/` |
| box | Vast instance 61703284 (offer 61212478, label `kf-accept-hwval-211208`, Denmark, host 57929), 1x RTX 4090 (AD102, Ada), 30 vCPU, 49 GB RAM, `df -h /` 117G at boot, `/dev/kvm` present, KVM image `vastai/kvm:ubuntu_cli_22.04-2025-05-16` |
| host driver | stock 575.51.03 closed at boot, swapped by `provision_host_driver.sh` to **580.159.04 (open module)**; kernel 6.8.0-59-generic (`merge_bar/prov.log`, `merge_bar/driver.log`) |
| guest drivers | fat and thin guest 580.159.04 (bench image); 595.84 (staged for the cross-branch rows) |
| window | rented 21:12 UTC, destroyed about 23:07 UTC (6747 s at 0.567 USD/h, about 1.07 USD) |
| teardown | `vastai destroy instance 61703284 -y`; `vastai show instances-v1` afterwards listed none of this lane's ids (another session's `kf-accept-newb-213514` was there and was not touched) |
| source | the box cloned the public repo (`KAYFABE_BRANCH=next`), no secret on it; only text came back (logs gzip-compressed). GPU board UUIDs are replaced by `GPU-REDACTED` in every file here |

Bare metal first: the bench suite on the bare host ran 30/30 before any guest (`merge_bar/hwval1_host.log`); the VFIO reference does not exist on Vast.

## Verdicts

| gate | verdict | evidence |
|---|---|---|
| `v3_gates.sh` | **PASS** `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS`, births 12/12 USER | `merge_bar/hwval1_gates.log` |
| `build_kf3.sh` | **PASS** `KF3_RC=0` (stripped tree of `.workerignore`, `KF_REQUIRE_ORACLES=1`) | the `KF3_RC=0` and `KF3_BUILT .../kf3-bins/32409d29/...` lines of `merge_bar/hwval1.log` |
| raw client + bare-metal suite | **PASS** `BARE_SUITE_PASS=30 FAIL=0 CRASH=0 ARMS=30` | `merge_bar/hwval1_host.log` |
| fast guest rebuild + `fast_suite.sh` 180 s (`KF_DEVICE=kf3`) | **PASS** `FAST_SUITE_PASS=30 FAIL=0 CRASH=0 NOTRUN=0 ARMS=30` | `merge_bar/hwval1_suite.out`, per-arm QEMU logs `merge_bar/fast_qemu/` |
| birth census | **PASS** `BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=12 all PRIVILEGED_CHANNEL=0 privilege=USER` | `merge_bar/hwval1_births.log` |
| `cuda_ladder.sh host` (cup2 cup3 cup8 cup8bench, 2 reps) | **PASS** 8/8 | `cuda_ladder/cl_hwval_cl_host.out` |
| `cuda_ladder.sh guest` (same rungs, fat guest, 2 reps) | **PASS** 8/8 (`CE rv=0xabcd1234`, `CUP3_VAL=43`, `CUP8_BAD=0`, `GUEST_BENCH_TOTAL_BAD=0`) | `cuda_ladder/cl_hwval_cl_guest.out`, `cuda_ladder/qemu/` |
| app matrix, Linux, **subset**: 16 rows of the CUDA-samples part (nvidia_smi deviceQuery vectorAdd vectorAddDrv matrixMul matrixMulDrv bandwidthTest simpleStreams asyncAPI simpleCallback simpleCooperativeGroups cudaTensorCoreGemm graphMemoryNodes simpleCudaGraphs simpleCUBLAS simpleCUFFT) | **PASS** bare metal 16/16, guest 16/16 (one app per boot, `apps_matrix.sh` unchanged) | `apps/host.res`, `apps/guest.res`, `apps/qemu/` |
| app matrix, the full 66 rows | **NOT RUN**: the full bundle (torch, blender, llama.cpp, models, open rows) and its guest provisioning were estimated above the time-box; this cut needed its own bundle script (see Deviations) | n/a |
| refusal census diff, #545 | **PASS, no regression** (see below) | `census_diff.py`, `merge_bar/`, `cuda_ladder/`, `apps/census.tsv`, `baseline_6524df05/` |
| #540 caps never trip | **PASS** 0 hits of `DEBUGGER_*BUDGET`, `ENGINE_OBJECT_TWIN_BUDGET`, `HOST HANDLES EXHAUSTED` in 30 + 8 + 16 + 7 + 4 QEMU logs (the fast suite, the ladder, the app cut, the 595.84 rows, the baseline) | grep over the `qemu/` directories |
| #540 positive tests (a guest loop of `GT200_DEBUGGER` allocs; ZCULL_BIND at a read-only page; the log-and-count step on 580.95.05) | **NOT RUN**: the audit's hardware items 2-4 need purpose-written guest programs, outside this lane | n/a |
| #546 message on hardware | **PASS** (see below) | `xbranch/nopin_refusal_message.txt` |
| 595.84 guest on the 580.159.04 host, pinned with `guest-driver=595.84` | **PASS** thin `--timer` arm 1/1, fat ladder 4/4 | `xbranch/hwval_x595_suite.out`, `xbranch/cl_hwval_x595_guest.out` |
| 550.x / 545.x guest | **NOT RUN**: not attempted (550.54.14 has a trust pin, 545.23.08 has none); the box was retired when the other rows were done | n/a |
| host 595.91.07 (a host swap) | **NOT RUN**: outside the lane's list | n/a |
| Windows gates (580.88 guest, tag 580.65.06), merge bar's Windows stage | **NOT RUN**: not requested for this lane (needs the 20-45 min Windows install). INFERRED only: at tag 580.65.06 the generated table admits `NV50_DEFERRED_API_CLASS` 0x5080 (Mode2Rpc) and `NV01_EVENT_KERNEL_CALLBACK` 0x78 (Mode2Rpc), read with `ingress_policy.compute()` | n/a |

## The refusal census (#545)

Method (`census_diff.py`, GPU-free): collect every `kf3: GSP REFUSED fn76/0x<ctrl>` / `fn103/0x<class>` line and every `kf-rm: RPC-REFUSED`
line of a set of QEMU logs; for tag 580.159.04 give each id its verdict at the **old** gate (`crates/kf-rm/tests/fixtures/pre_i7_gate_verdicts.tsv`
plus its two rule passes: bit 15 and class 0x2081) and at the **new** one (`ingress_policy.compute()`). A refused id that the old gate admitted
and the new one refuses is NEWLY_REFUSED and is checked against the lane.

- **Fast suite (30 logs, `merge_bar/`): 25 distinct ids refused, every verdict `same`** (old and new agree); 0 NEWLY_REFUSED.
- **Fat-guest ladder (8 logs), app cut (16 logs): 36 distinct ids, 8 NEWLY_REFUSED**: class `0x208f`, controls `0x20808513`, `0x2080852f`,
  `0x20809064`, `0x2080a612`, `0x20810108`, `0x20810110` (`cuda_ladder/`, `apps/census.tsv`). Names: class `0x208f` is `NV20_SUBDEVICE_DIAG`
  (non-privileged in ogkm, but kayfabe serves no such object, so the new gate refuses it by default; the old gate admitted it by row); control
  `0x20808513` is `NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2_PHYSICAL`; the other five have no ogkm name at 580.159.04 (GSP-only ids kayfabe's
  code names, `ingress_report.tsv` `unknown-to-ogkm`) and `0x2081xxxx` is the binary-API class the old gate passed by rule. Every CUDA
  rung and all 16 apps PASS in the guest with them refused with `NV_ERR_NOT_SUPPORTED` (status 0x56).
- **The decisive control, MEASURED:** the same four ladder rungs (cup2 cup3 cup8 cup8bench, 1 rep) on the baseline binary `6524df05`, whose gate is
  the old one (`baseline_6524df05/`): 4/4 PASS, and the set of `GSP REFUSED` (fn, id, status) triples is **33 and identical** to the new revision's
  rep-1 logs (nothing only on one side). So the eight "newly refused" ids were already answered 0x56 by the chain after the old gate; the new
  gate refuses them earlier and by name (`ControlNotPermitted`/`AllocClassNotPermitted`), the guest sees the same status, and no lane needed them.
- **Regression of #545 by this census: none.** No id a lane needed is refused. (Only the first 24 `RPC-REFUSED` lines per QEMU process are
  printed; the `GSP REFUSED` lines are per id, which is why the census reads those.)
- 595.84 guest rows (`xbranch/`, tag 595.84): 9 NEWLY_REFUSED, the same ids plus `0x2080852e` and `0x2080a618`, all four rungs PASS. NOT compared
  against a baseline at 595.84.

Expected per the design (section 7 item 2): the sets are identical apart from the tag; nothing contradicted it. The census cannot show an id
that a lane would need but that none of these lanes exercises.

## #546 on hardware (cross-branch `fn 1`)

Guest 595.84 (thin guest built from ogkm at the tag, `stage_guest_driver.sh`; fat guest `stage_fat_guest.sh`) on the 580.159.04 host:

- Pinned (the harness derives `guest-driver=595.84` from the guest's receipt): QEMU log `kf3: guest driver 595.84 (guest-driver=); measured ABI: ...`;
  thin `--timer` PASS, ladder 4/4 PASS.
- **Falsifier stated before the run:** with no `guest-driver=` property the guest must be refused at `fn 1` with a message that names the exact pin.
  Result, MEASURED (`xbranch/nopin_refusal_message.txt`): `kf-rm: the guest says it is driver 595.84; cannot re-select from 580.159.04 (the version
  the device defaulted to): MESSAGE_QUEUE_INIT_ARGUMENTS differs and the guest already consumed it. To serve this guest add guest-driver=595.84 to the
  kf3-gpu device`, and the arm ends `FAST_VERDICT=FAIL (raw client rc=1)` (an expected refusal, not a defect).
- The first "no pin" run (`xbranch/xbranch_nopin.log`) was NOT a control: the harness pins from the receipt. See `xbranch/NOTE_nopin_control.txt` for how
  the real control was made (a temporary, reverted edit of the box's checkout; no repository file changed).

## Deviations from the written procedure (all on the box, none in the repository)

- `merge_check.sh next-sha hwval1` ran gates, kf3 build, bare-metal suite, fast suite and census as one bar (that is the repository's own bar);
  `cuda_ladder.sh` ran afterwards from the provisioning checkout (same revision).
- The app cut used `apps/mini_bundle.sh` (a CUDA-samples-only cut of `build_bundle.sh`: same pinned toolkit, same pinned cuda-samples, same make line),
  `apps/inject_guest.sh` (the first half of `provision_guest_apps.sh`, into a qcow2 overlay `guest-apps.qcow2`, base untouched) and
  `apps/run_apps_mini.sh` (`apps_matrix.sh host` then `guest`, unchanged). `gpu_use=NA` in the rows is expected: none of these samples can fall back to the CPU.
- The ladder ran 2 reps instead of the script's default 3; the baseline ran 1 rep and the guest rungs only.
- One `fast_suite.sh` run (`xbranch/` control attempt, tag `hwval_x595np2`) reported `NOTRUN` ("a precondition refused before the arm ran"; run directly,
  `run_fast_guest.sh` said: no kf3 binary for `32409d29-dirty`, because the checkout was dirty during the temporary edit). It is not a result and is
  not listed above; the run with `QEMU_BIN` set is the control.

## Files

`merge_bar/` (provisioning, gates, kf3, host suite, fast suite output, census, per-arm QEMU logs) | `cuda_ladder/` | `apps/` | `xbranch/` |
`baseline_6524df05/` | `census_diff.py` (python3 -I census_diff.py <repo> <tag> <out.tsv> <logs...>).
