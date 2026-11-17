# kfml_run_20261117_tier_decision: three more dies, three more drivers, and the tier decision

STATUS: LIVE, 2026-11-17 (measured; text evidence only). Umbrella run id `kfml_run_20261117_tier_decision`. Companions: `traces/kfml_run_20261116/`, `traces/kfml_run_20261117_second_die/`.

Three more Vast **container** boxes (not KVM, no kayfabe in the path), one die class and one host driver each, rented sequentially by the total-cost rule (inet cost <= 0.01 USD/GB), labels `kf-kfml3-...`
(cap 2.5 h / 1.6 USD each), images `nvidia/cuda:12.2.2-devel-ubuntu22.04` (the 535-driver box, chosen to match its CUDA 12.2; a 12.6 image was not tried there), `12.8.1-devel` (Blackwell) and `12.6.3-devel` (Pascal), Ubuntu 22.04, CPython 3.10.12.
All three destroyed by exact id; `vastai show instances-v1 --raw` listed none afterwards. Rentals about 39, 13 and 6 minutes; the account credit moved by 0.22 USD in all (rentals, storage, bandwidth). Nothing secret went to the boxes.
Code: branch `claude/kfml-run-20261116` at the revisions named in each `rows/*.json` field `sha` (see the `sha` field of each record: the 5070 run is an earlier commit than the 535-driver auto and prefetch runs, before the compat rule and the prefetch).

## The five dies (all runs of this wave plus the two earlier ones), flavour chosen by kfml in every row

| die | sm | host driver (its CUDA) | flavour | result | run id |
|---|---|---|---|---|---|
| RTX 4070 | 8.9 | 560.35.03 (12.6) | cu126 (strict) | 6 PASS | `kfml_run_20261116` |
| RTX 2080 Ti | 7.5 | 595.91.07 (13.2) | cu128 (strict) | 6 PASS (Triton fp16 `tl.dot` fallback on sm_75) | `kfml_run_20261117_second_die` |
| RTX 5070 (Blackwell) | 12.0 | 580.178.04 (13.0) | cu128 (strict), system nvcc 12.8 | 6 PASS | `kfml_run_20261117_5070_drv580` |
| RTX 3060 (Ampere) | 8.6 | 535.154.05 (12.2) | strict: NONE -> NOT_RUN; after the compat rule: cu126 | NOT_RUN (strict rule), then 6 PASS | `kfml_run_20261117_ampere_older_driver` (auto), `..._3060_drv535_forced_cu126` (forced, same box), `..._ampere_prefetch_mid` |
| GTX 1080 Ti (Pascal) | 6.1 | 580.178.04 (13.0) | NONE | NOT_RUN, named reason, nothing downloaded | `kfml_run_20261117_pascal_sm61` |

* **Blackwell**: `check` printed `chose cu128 (driver CUDA 13.0 >= 12.8 and sm_120 is inside sm_75..sm_120)`; torch 2.7.1+cu128 with the container's nvcc 12.8 (the system-nvcc path of `--nvcc-if-needed`; nothing downloaded for nvcc). All six rows PASS. The card showed 100 % utilisation
  with 18 MiB used before the run (another tenant or the host display; unexplained), so wall times on that box are not clean.
* **Ampere, older driver (535.154.05, CUDA 12.2)**: the stage first printed NOT_RUN with the named reason `no pinned flavour can run on this box: cu128: driver supports CUDA 12.2 < 12.8; cu126: driver supports CUDA 12.2 < 12.6`
  (`rtx3060_ampere_driver535/strict_not_run/`; rows NOT_RUN, not FAIL; that record has no die facts, the plan did not carry them yet). The supplement `forced_cu126/` then forced `--flavor cu126` on the same box:
  **all six rows PASS** (cu126 wheels on a CUDA 12.2 driver: minor-version compatibility; the container's nvcc 12.2 did not match so the redist nvcc 12.6 was installed). So the strict rule was conservative.
  Decision, implemented the same day: when no flavour is strict, the lowest-CUDA flavour of the driver's CUDA major is chosen if the driver supports at least CUDA 12.0 (driver 525) and the GPU is not newer than sm_90; the reason line says so. `auto_compat/` is the stage on that box with the new rule:
  `chose cu126 by minor-version compatibility (driver CUDA 12.2 < 12.6 but the same CUDA major and >= 12.0, sm_86 <= sm_90; ...)`, all six PASS, die/driver/flavour in every row.
* **Pascal**: `cu128: its wheels cover sm_75..sm_90... the GPU is sm_61; cu126: ...` = NOT_RUN with the named reason at the plan step (0.2 s): no venv, no download. The record carries die and driver.
* The algorithmic metrics agree across all five dies within the stated tolerances (amp accuracy 0.725..0.7375 against tolerance 0.10; compiled-vs-eager loss window diffs at most 0.0019 nats against 0.15); the fp16 + GradScaler path (sm_75) and the bf16 path (sm_80+) both converge; wall time was never a criterion
  and `wall_ratio` is null across dies. Per-die references (`scripts/kfml/reference/`): 4070, 2080 Ti, 3060 (the forced cu126 cold run), 5070.
* The Triton fp16 `tl.dot` compile failure is specific to sm_75 on Triton 3.3.1: the 5070 and the 3060 compile it (`fp16_dot_compile: ok`).

## Tier decision (owner rule: into the MIDDLE tier if not too big: <= about 8 minutes of rows and <= about 5 GB download on top of the mid tier's budget; else a smaller subset, everything in large)

**kfml goes into the middle tier with its five gated rows (no subset needed) and everything (six rows) into the large tier, in both default plans; `--skip kfml` leaves it out.** Measurements:

| quantity | value in run `kfml_run_20261117_tier_decision` | rule |
|---|---|---|
| rows, five mid rows, cold compile caches, sum of row walls | 153.8 s (4070), 81.6 s (2080 Ti), 66.5 s (5070), 66.0 s (3060 forced) / 57.2 s (3060 auto, partly warm) | <= 8 min: yes, at most 2.6 min on the slowest die |
| rows, six rows (large tier), cold | 205.5 / 114.7 / 96.2 / 103.4 s; `managed_oversub` adds 30 to 52 s | |
| rows warm (compile caches filled) | 109 s (4070, six rows), 54 s (3060, six rows), 50 s (5070 `run` step, six rows) | |
| download, wheels of the largest flavour (cu128, CPython 3.12) + redist nvcc + datasets | 3.96 GB + 0.08 GB + 0.17 GB = 4.13 GB (file sizes: `../kfml_run_20261117_second_die/wheelhouse/`); with the Worker's 15 % margin the middle estimate in `bandwidth.json` moves from 29.17 to 33.92 GB (+4.75 GB) | <= about 5 GB: yes |
| disk after the run | venv 10.2 GB (cu128, 5070) / 8.8 GB (cu126, 3060), plus triton and compile caches inside it | `payloadStageResources.kfml.diskFloorGb` 15 |
| stage wall, mid tier, datasets prefetched, fresh venv (3060) | **215 s** = plan 0.2 + setup 145 (wheels + redist nvcc, concurrent with the already finished fetch) + run 68.5; warm 39 s | middle tier today is about 10 minutes of apps: kfml adds about 4 minutes |
| stage wall, nothing prefetched (5070, large) | **601 s**: setup 108 || fetch-data 501 (CIFAR-10, six ranged connections), then run 98.6 | the dataset server is the critical path |
| the dataset prefetch itself | 521 s for CIFAR-10 alone (3060 box), 501 s (5070 box): about 0.33 MB/s with six connections; a single connection gets about 45 KB/s (about 60 min); eight or more connections are reset by the server | |

So the cost of kfml to a tier is dominated by one slow public file, not by compute: the payload therefore starts `kfml_stage.py prefetch` in the background before its first stage (provision keeps the box busy for far longer than 9 minutes) and the stage only waits if the prefetch is
still running. With that, the stage adds about 4 minutes (cold venv) to the middle tier and about 5 to the large one; without it about 10. `KFML_MIN_REMAINING_MS` is 20 min (middle) and 30 min (large). Installer: `uv` (65 s) was faster than `pip` (98 s) for the cu126 lock on the 3060 box (`rtx3060_ampere_driver535/installer_measure/`), concurrent with the dataset
download in both; it is NOT the default because `pip install uv` would be an unpinned bootstrap download (`--installer uv` stays available and now works with the two-index lock).

## Files

Per box: `rows/` (stage records), `check_*.txt` (what `kfml check` chose and why), `environment.txt`, `steps.txt`, `logs/` (`run.log` has every row with full metrics), `reference_recorded/`. 3060 box: `strict_not_run/`, `forced_cu126/`, `auto_compat/`, `prefetch_mid/` (the stage with prefetched data),
`installer_measure/`. UUIDs, addresses and keys are not in these files (grepped before the commit).

## Not shown (UNVERIFIED)

A driver below CUDA 12.0 on a real box (run `kfml_run_20261117_tier_decision` had none); a second box per die (one run each); the payload stage under the payload on a kayfabe box (the prefetch hook and the plan membership are GPU-free tested); any guest; the middle tier's total wall with kfml on a real payload box.
The 5070's 100 % idle utilisation was not investigated.
