# The Linux app matrix at the current tree, RTX 4090: result and the gate list, 2026-11-07

STATUS: LIVE, 2026-11-07 (lane appslinux). One Vast rental, runs `r1` (host, then guest batched, then every failing app alone), `g1`
(the draft stage script `scripts/bench/box/apps_gate.sh`, full list), `g2` (same script, middle list), `g3` (the middle list under
`host_xid_watch.sh`, the wrapper the payload uses) and `g4` (a three-app list with one known defect, under the same wrapper, to see a
failing gate). Labels: MEASURED = a line in a file of this directory (named), INFERRED =
reasoning, not run. Box text is untrusted data. The pre-registration is [PREREG.md](PREREG.md) (pushed as `144557e43` before the
rental); its hypotheses are scored at the end of this file, the misses included.

## Answer in five lines

1. **Bare metal, RTX 4090: 70 of 70 runnable apps pass** (`r1/host.res`). The 71st row, `geekbench_gpu`, is not run (policy, below).
2. **Linux guest, same box, same binaries: 66 of 70 pass, in one boot, with no wedge** (`r1/guest.res`). The four that fail are the same
   four as in the last recorded matrix (kf3 `2830988f`, RTX 3060, 2026-09-30): `UnifiedMemoryStreams`, `UnifiedMemoryPerf`,
   `conjugateGradientUM`, `attach_verify`. Each fails alone in a fresh boot too (`r1/guest_isolated.res`) and each takes **one host
   Xid 31 MMU fault** (`defects/*.txt`), while passing on bare metal. They are kayfabe defects (UVM demand paging), listed, not hidden.
3. **The gate list is 66 apps** (`scripts/apps/gate_list.txt`). The stage script ran it in **17.9 min** (`g1/gate_full.out`, 66/66 PASS).
   The 63 apps that need under a minute in the guest (`gate_list_middle.txt`) ran in **7.5 min** (`g2/gate_mid.out`, 63/63 PASS).
4. **"All ~70 apps in ~10 minutes" holds for the guest only without `vkpeak` (7.3 min alone), `clpeak` (2 min) and `gpu_burn` (1.6 min).**
   With them: host 14.1 min, guest batched 19.3 min.
5. Provisioning the apps costs **29 min of wall time on a fresh box** (bundle 18.8, host runtime 2.1, guest image 9.1, the last two
   overlapped), on top of 9.7 min for `provision_full.sh`. A CI box cannot do the bundle step (below), so the guest image has to arrive
   prebuilt: 20.2 GiB on disk, 13.1 GB with zstd -3.

## Setup (MEASURED, run r1, 2026-11-07)

- Tree: origin/next `a9dee7b2c` plus only `PREREG.md` (branch head `144557e43`). `git diff --stat a9dee7b2c 144557e43 -- crates qemu cuda`
  is empty. The kf3 binary is `kf3-bins/144557e4` (sha256 `73a2c184eb6b1902...`). `origin/next` has moved since (`b11655b70`, site and
  CI changes); nothing here was re-run on it.
- Box: Vast instance 59497053 (label `kf-appslinux-20261107`), machine 55263, host 33035, RTX 4090 (AD102, 24564 MiB), AMD EPYC 7B13,
  30 vCPUs, 49 GiB RAM, nested KVM, 214 GB root disk, 0.7685 USD/h with that disk. Host driver 580.159.04 (open module, installed by
  `provision_full.sh`; the box shipped 575 and 595), guest driver 580.159.04, guest Ubuntu 24.04, kernel 6.8.0-147.
- Guest: `kf3-gpu`, 16 GiB, 6 vCPUs, no persistence mode. Bundle `eee45adc54213fec` (CUDA 12.6 toolkit, cuda-samples v12.5, llama.cpp
  `4b1a27f`, clpeak 1.1.2, vkpeak 20250531). The bundle is built `-arch=sm_86` and ran on this `sm_89` card unchanged.
- Timeline (UTC, `logs/`): rent 01:38:36, `provision_full.sh` 01:41:27-01:51:09, `build_bundle.sh` 01:52:34-02:11:20, host runtime
  02:12:26-02:14:31, guest image 02:12:48-02:21:53, host matrix 02:22:04-02:36:08, guest batched 02:39:00-02:58:20, four isolated boots
  to 03:03:58, `g1` 03:05:01-03:22:54, `g2` 03:23:05-03:30:37.
- `g4` (03:43:05-03:45:44, `g4/gate_fail.out`): `vectorAdd`, `UnifiedMemoryPerf`, `deviceQuery` under the wrapper. The stage printed
  `APP_ROW ... result=FAIL`, `UnifiedMemoryPerf --- FAILED`, `APPS_STAGE_FAIL`, `APPS_STAGE_EXIT rc=1`; the wrapper saw two new host
  Xid 31 and exited 6 (`NEW HOST Xid ACROSS THE COMMAND`). Through `stage_grade` that is the failing row `host_xid` with `apps_passed`
  beside it, the same as any other stage that faults the host GPU (replayed in `test_a_real_failing_gate_app_under_the_wrapper...`).
- Spend: instance 59497053, rented 02:38:36 CET (01:38:36 UTC), destroyed 04:46:26 CET (03:46:26 UTC), absent from `vastai show
  instances-v1` afterwards: 2 h 08 min at 0.7685 USD/h = 1.64 USD.
- Two harness faults, both mine or the harness's, neither a verdict: (1) the first guest attempt (02:36:41) failed 14 boots in 7 s each
  because `guest_apps.qcow2` had no `.STAGED` receipt (the guest profile now refuses an image without a verified driver version);
  `apps_gate.sh` now copies the receipt with the image (`logs/guest_driver_attempt0_nostaged.log`). (2) I replaced `setup_side.sh`
  under a running bash and restarted that step once; the host-runtime time above is the second start.

## Per-app table (MEASURED, run r1, RTX 4090, 2026-11-07; `apps_table.tsv`, reproduce: `python3 scripts/apps/matrix_table.py traces/apps_linux_20261107/r1`)

Columns: app, class, host, guest batched, guest alone, host secs, guest secs, guest Xid, kf3 refusals, the key output line on each
side, reason. The class is decided mechanically by `scripts/apps/matrix_table.py` (rules in PREREG.md; GPU-free tests:
`scripts/ci/test_apps_matrix_table.py`).

| class | apps | meaning |
|---|---|---|
| GATE | 66 | host PASS and guest PASS: required |
| KAYFABE_DEFECT | 4 | host PASS, guest FAIL alone in a fresh boot |
| HOST_LIMIT | 0 | host failed: none |
| ORDER_SENSITIVE | 0 | batched FAIL, alone PASS: none |
| POLICY_SKIP | 1 | `geekbench_gpu` |

The four defects, each with the evidence the classification rule asks for (`defects/<app>.txt`; the guest Xid count is 0 for all four):

| app | guest result | host Xid in the boot (MEASURED, RTX 4090, 2026-11-07) | kf3 |
|---|---|---|---|
| UnifiedMemoryStreams | `CUBLAS_STATUS_EXECUTION_FAILED` at `UnifiedMemoryStreams.cu:221`, rc 134 | Xid 31, GPC0, `FAULT_PTE` VIRT_READ | `RC host twin ... except_type=0x1f (Xid 31)` x8 |
| UnifiedMemoryPerf | code 719 at `matrixMultiplyPerf.cu:435` | Xid 31, GPC2, `FAULT_PDE` VIRT_READ | same, x8 |
| attach_verify | `cudaDeviceSynchronize() -> 719` | Xid 31, GPC0, `FAULT_PDE` VIRT_READ | same, x8 |
| conjugateGradientUM | **silent wrong answer**: `Error amount = 1.000000, result = SUCCESS`, rc 0 | Xid 31, GPC5, `FAULT_PDE` VIRT_READ | same, x8 |

`conjugateGradientUM` is the dangerous one: it exits 0. The harness catches it (its predicate wants `Error amount = 0`), which is
why this matrix stays in the gate list's design: a passing exit code is never the predicate. The same four failed with the same Xid
class on RTX 3060 (R3) and RTX 3070 (R4), `docs/design/V3_APP_MATRIX.md`. Every app, passing or not, shows 4-30 kf3 refusal lines
(`CLEARED OVER ABSENCE`, `kf3_refusals` column): a baseline of this tree, not a differentiator.

Other facts of the table:

- Digests equal bare metal for all three apps that print one (`r1/host.res` and `r1/guest.dig`): `torch_correct` CNN train step
  `763c693a5a53f948`, `hf_generate` `0d973108a6251e14`, `llama_cpp_gen` `30451301af8e011d`; the Blender CUDA render has `mean=0.6405` on
  both sides.
- One host NVRM line, `Out of memory [NV_ERR_NO_MEMORY]` in `_memdescAllocInternal`, at 02:20:23, before the host matrix started
  (during the guest-image build, no app running): not attributable to any app, noted because it is unexplained.
- Guest vs host figures, one sample each, perf never gates (key lines in the table): `sgemm_cublas` 58.78 vs 59.07 TFLOPS,
  `vkpeak` fp32 62107 vs 62058 GFLOPS, `stream_triad` 928.9 vs 932.0 GB/s, `nbody` 12715 vs 12766 GFLOPS, `cudaTensorCoreGemm`
  15.53 vs 15.43 TFLOPS, `llama_bench` tg64 417 vs 483 tok/s; lower in the guest: `bf16TensorCoreGemm` 85.6 vs 116.6 TFLOPS,
  `memcpy2d` 6.2 vs 9.2 GB/s, the sub-30-microsecond kernels (`reduction`, `transpose`). `hashcat`'s speed line is not comparable (the
  mask attack ends in 0.1 ms; the guest picked Thr:32 against Thr:64).

## Time (MEASURED, runs r1, g1, g2, RTX 4090, 2026-11-07)

| what | wall | apps | note |
|---|---|---|---|
| host matrix, all rows | 14 min 4 s | 70 + 1 not run | `vkpeak` 445 s, `gpu_burn` 95 s, `clpeak` 80 s; the other 67 apps: 222 s |
| guest, one boot, 70 rows | 19 min 20 s | 66 pass | sum of app times 15.6 min; the other 3.7 min is the boot and the per-app log capture (about 3 s each) |
| guest, 4 failing apps alone | 5 min 38 s | 4 boots | 82-86 s per boot of one app |
| **`apps_gate.sh full`** (`g1`) | **17 min 51 s** | 66/66 PASS | the stage script, 66 apps in one boot |
| **`apps_gate.sh middle`** (`g2`) | **7 min 31 s** | 63/63 PASS | the apps under 60 s in the guest |
| `apps_gate.sh middle` under `host_xid_watch.sh` (`g3`) | 7 min 41 s (stage 456 s) | 63/63 PASS, `HOST Xid CLEAN`, rc 0 | the wrapper `vast_payload.py` puts around each stage; 8 Xid before and 8 after (the defects of `r1`) |

- **One app per boot costs 82-86 s a boot, so 70 apps would take about 97 minutes** (INFERRED from the four isolated boots). That is
  why the harness batches. In this run one boot held 70 app processes and 4 wedge probes with no `APPS_WEDGE` line, no `NoMemory` and no
  `slot ... is full` in its kf3 log (`r1/guest.res`, `boot_ap_r1_b1.qemu.log`): the per-boot process budget of R2 (about 40 CUDA
  processes) did not bite.
- **Minimal batching that fits a CI box (INFERRED from the runs above):** one boot for the whole list, in run order, the wedge probe on
  after any non-PASS row (harness default), no isolation phase in the gate (a gate app that fails is already a verdict; isolation is for
  the people who read it). Order does not matter for the apps that pass: `r1`, `g1`, `g2`, `g3` ran them in the same order and 63 apps
  passed in every run; the three slow ones sit mid-list and do not shorten anything by moving.
- **Does "all ~70 apps in ~10 min" hold? No on the full list** (guest 17.9-19.3 min; host 14.1 min). **Yes for the middle list in the
  guest** (7.5 min for 63 apps). The owner's middle-tier sentence is therefore met by `gate_list_middle.txt`; the full list is the
  release tier's.

## Downloads, sizes, secrets (MEASURED where a size is given, run r1 on the RTX 4090 box, 2026-11-07; otherwise INFERRED)

Everything is official or public; **nothing needs a secret and none is put on a box.** `geekbench_gpu` is the exception: `docs/DOWNLOADS.md`
marks Geekbench "not allowed (scripts need Pro), manual" and the free mode uploads the result, so `setup_side.sh` no longer fetches it
(`KF_APPS_GEEKBENCH=1` does, for a manual run) and the table calls the row POLICY_SKIP. If the owner wants it, that is a terms decision.

| item | source | size | seen |
|---|---|---|---|
| CUDA toolkit 12.6 (apt, toolkit only) | developer.download.nvidia.com | the box's apt cache ended at 4.4 GB (toolkit plus the driver build) | `build_bundle.log`: 8 min with install |
| cuda-samples v12.5, gpu-burn, llama.cpp `4b1a27f`, clpeak 1.1.2 | github.com | 1.3 GB, 1 MB, 1.2 GB, 4 MB checked out | `logs/prefetch.log`: 2-10 s each |
| vkpeak 20250531 | github.com release | a few MB | in the bundle |
| torch 2.6.0+cu124, torchvision, transformers, accelerate, cupy, numpy | download.pytorch.org, pypi | pip cache 1.7 GB, venv 5.4 GB | host 2.1 min, guest part of 9.1 min |
| Qwen2-0.5B-Instruct (HF) | huggingface.co | 954 MB | same |
| qwen2.5-1.5b-instruct-q4_k_m.gguf | huggingface.co | 1,117,320,736 B | 11 s |
| Blender 4.5.0 | download.blender.org | 376,170,648 B (1.2 GB unpacked) | 6 s |
| ffmpeg, hashcat, vulkan-tools, clinfo | Ubuntu apt | small | in `setup_side.sh` |
| the bundle (`bundle.tgz`) | built on the box | 1,349,699,594 B (2.1 GB) | `eee45adc54213fec` |
| the apps guest image | built on the box | 20.2 GiB on disk, 73.5 GiB virtual; 13,082,387,956 B with `zstd -T0 -3` (27 s) | |

## What a CI box can and cannot do (INFERRED from the code, nothing here ran under the Worker)

- The Worker box installs packages through a cache-only apt shim over a GitHub release asset (`scripts/ci/setup-cache.lock.json`:
  `debs.tar.gz`, 285 MB). The CUDA toolkit is not in it, so `build_bundle.sh` cannot run there; the guest image build also needs
  pip, Hugging Face and Blender downloads. So: **build the apps guest image once per revision of the app set, publish it, fetch it on the
  box** (13 GB compressed). The Worker's presigned `/bench-data/` route is the existing channel for such a private object
  (`scripts/bench/box/fetch_bench_data.sh`); adding an object name is a Worker change, which this lane was told not to make.
  Until that exists the stage ends `APPS_STAGE_RETRY class=NOT_BUILT`, never a pass.
- The host side needs only `bin/` and `cuda/` of the bundle (49 MB: the hook pushes them into the guest at every boot) and the repo's
  `scripts/apps/`; it does not need the 2.1 GB bundle or the host Python venv.
- Time: the apps stage adds 7.5 min (middle) or 17.9 min (large) of guest time plus one guest boot, to a payload whose Windows stage
  alone needs 75 min left (`WINDOWS_MIN_REMAINING_MS`). The stage refuses to start with under 60 min left (`APPS_MIN_REMAINING_MS`,
  INFERRED margin: 2x the full-list run plus a boot).

## The draft stage (DISABLED BY DEFAULT)

- `scripts/ci/vast_payload.py`: `APPS_STAGE_ENABLED = False`; when the owner sets it, tiers `middle` and `large` run stage `apps` after
  `cuda_guest` and before `windows_gate` (`large`: the full list, `middle`: `gate_list_middle.txt`); `small`, `adhoc` and unknown never.
  `KF_STAGES=apps` stays refused. No Worker change.
- `scripts/bench/box/apps_gate.sh <tag> <prov> [full|middle]`: the lane. Reads the list, refuses without the prebuilt image
  (`RETRY class=NOT_BUILT`), runs `apps_matrix.sh guest` in its own session under `KF_APPS_BUDGET_S` (default 3000 s) so the budget
  kills QEMU too, prints `APP_TRY` lines as results arrive and one `APP_ROW` per gate app at the end, then exactly one
  `APPS_STAGE_PASS|FAIL|RETRY class=...` and `APPS_STAGE_EXIT rc=N`. A failing app is also printed as `<app> --- FAILED`, so the
  payload's FINAL SUMMARY lists it as a `failed_test`.
- Grading (`apps_rows`): PASS only with rc 0, exit line 0, the list size, and exactly one PASS row for every name of the list. FAIL is
  the suite row `apps` (+ `apps_passed`). RETRY is `apps_retry_<class>` (the Worker already reads `<x>_retry_<y>` as a retry row,
  `ci/vast-broker/src/release.js retryRow`). Below `APPS_MIN_REMAINING_MS` the stage does not start: `apps_retry_time`.
- Real output replays: `g1/gate_full.out` grades as `[{'name': 'apps', 'pass': 66}]` against the committed full list
  (`test_the_real_g1_stage_output_grades_as_a_pass`); `g2/gate_mid.out` likewise for the middle list.
- Report rows: `vast_run_report.py` is PR #282's file and is not in `next` yet, so the extension is a patch against #282's head
  (`6e9e2fcb2`): `pr282_apps_rows.patch` (applies cleanly there; its 92 tests and its mutant harness pass, the new `apps-*`
  mutants each have two killers). It turns `APP_ROW` lines into `apps[]` rows `{sub, name, os, result, note}` and `tests[]` rows
  (suite `apps_linux`) of the Linux guest subrun, adds the stage's rows to that subrun's counts, and maps an `apps_retry_*` row to a
  RETRY subrun. On the real `g1` lines it yields 66 apps rows, Linux subrun PASS with 101 passing rows, a 15.7 KB report. Apply it
  after #282 merges (`git apply`), or the owner can ask for it as a commit on #282.
- Which files reach the box, and the Worker bundle hash: the Worker bundle is wrangler's build of `ci/vast-broker/src` with its config
  (hash in `ci/vast-broker/deployment.json`); `scripts/**` is read on the box from the git checkout of the tested SHA. This change
  touches only `scripts/`, `traces/` and docs, so the bundle hash is unchanged. Two Worker-side follow-ups are NOT in this change: a
  `requirements` rule that makes the Worker require an `apps` PASS row when the stage is on, and the object name for the image.
- Failure semantics follow every other stage: the payload stops at the first failing stage, so a failing `apps` stage means
  `windows_gate` does not run on that box (the same as a failing `cuda_guest`). If the owner wants Windows to run regardless, that is a
  one-line change in `execute_stages` and is not made here.

## Hypotheses scored (PREREG.md)

| id | verdict | evidence |
|---|---|---|
| H1 host >= 65 of 70 | **held**: 70/70 | `r1/host.res` |
| H2 guest within +-3 of host | **falsified by one app**: 66/70, four fail alone | `r1/guest_isolated.res`, `defects/` |
| H3 the four UVM rows still fail | **held** | same |
| H4 one boot holds all rows | **held**: no `APPS_WEDGE`, no `NoMemory`, 70 apps and 4 probes in one boot | `r1/guest.res` |
| H5 "70 apps in ~10 min" does not hold; host 12-35 min, guest 25-70 | **held for the claim, missed the range on the low side**: host 14.1 min (in range), guest 19.3 min (below 25) | table above |
| H6 apps provisioning 45-120 min | **falsified**: 29 min wall (30 min serial) for the apps, 39 min with `provision_full.sh` | timeline |

## Not done

- No second die: the gate list is measured on one RTX 4090 (AD102) and one driver. The 3090 fallback and the other families were not
  rented. The earlier matrices (3060, 3070) agree on which four fail. A list per family is INFERRED to be the same list.
- The stage has not run under `vast_payload.py` or the Worker; `g3` checks only its output under the Xid-watch wrapper.
- Windows apps: out of scope here.
- No kayfabe code was changed. The four failures are the existing UVM demand-paging item; nothing was opened for them.
