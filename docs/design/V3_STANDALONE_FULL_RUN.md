STATUS: LIVE, 2026-11-20 (code and GPU-free tests: `scripts/ci/standalone_full.py`, `scripts/ci/apps_discovery_stage.py`, the `--full` flag and two stages in `scripts/ci/vast_payload.py`; no box has run `--full`, so every duration and cost below is an ESTIMATE unless it is marked M; the stage `apps_discovery` alone ran once on a box on 2026-11-20 and was red, section 4; usage: `docs/howto/full-standalone-run.md`).

# The standalone full run: what it covers, what each part needs, what cannot run

The owner's goal of 2026-11-17: the standalone run passes with everything, no skips: the private images, the open rows, kfml, the probes and the open software that was planned. This document makes "everything" a list that a test can read. The list is `FULL_STAGES` in `scripts/ci/standalone_full.py`; `vast_payload.py --full` plans exactly that list, refuses by name before the first stage any stage whose input is missing, and records every `--skip`.

Labels: READ (read from a file of this tree), M (a number from committed evidence, named beside it), ESTIMATE (a planning number), INFERRED (reasoning). Private objects are called by neutral names (private image A, B, C; the SPEC bundle); what is in them and how they are held is not part of this repository's documents.

## 1. The strict contract (READ: `scripts/ci/standalone_full.py`, `scripts/ci/vast_payload.py`)

1. **One invocation.** `python3 -I scripts/ci/vast_payload.py --full --instruction FILE [--matlab-image PATH|URL] [--skip STAGE]... SHA NONCE`. `--full` needs an instruction file (it is the only place the strict switch lives) and refuses `--only`.
2. **A stage cannot silently disappear.** The plan is the 16 stages of section 2, in that order. A stage whose input is missing is a named refusal (`KF_PAYLOAD_FULL_REFUSED <stage>:<input>`, then `KF_PAYLOAD_FAIL suite=preflight`) before any stage runs: `tier:large`, `propimage_apps:propimages`, `propimage_apps:strict`, `apps:linux_apps`, `windows_gate:windows`, `windows_apps:winapps`, `windows_gate_2:windows_runs_2`, `windows_open_rows:online_host`, `matlab:matlab_image`, `specviewperf:spec_both`.
3. **A skip is explicit and printed.** `--skip STAGE` is the only exit. `KF_PAYLOAD_FULL manifest=1 stages=N skipped=a,b complete=0|1 by_design=...` is printed and the same record is the `full` key of the result file (`vast_report.validate_full`). `provision` and `merge_bar` cannot be skipped; `windows_apps` and `windows_open_rows` need `windows_gate`, `apps_discovery` needs `apps`.
4. **No skipped row inside a stage.** `propimages_strict: true` (every listed image and app runs; a skip fails by name `STRICT_SKIP:<image>:<app>:<reason>`), kfml's strict rule (`kfml_not_run`, in the same instruction field), the MATLAB stage's `--strict-rows` (added here: the zero-skip rule of the Worker's shape without its pins; the standalone shape was permissive and graded a run with skipped rows as PASS), and the new stages below.
5. **Discovery lane: report, never gate.** A discovery row that RAN and failed, crashed or hung is a listed breakage; a row that did NOT run fails its stage by name (`apps_discovery_not_run`, `windows_open_rows_not_run`).
6. **By-design NOT_RUN rows are named, not hidden** (table in section 3). Everything else that does not run is a failure.
7. **Trust.** A standalone run trusts git only: what the repository downloads is checked against an upstream-authored anchor (official checksum, pinned upstream commit, vendor signature); a path or URL the user passes is trusted and its computed sha256 and size are recorded; lifting a check is explicit (`docs/design/V3_STANDALONE_TRUST.md`). A GitHub attestation anchor is NOT BUILT.

## 2. The manifest

Column key. Lane: gate (must pass) or discovery (reports; a row that did not run fails by name). Time and cost are ESTIMATE unless marked M (a number from the item's own design document or traces); a box costs about 1 USD per hour and downloads about 0.01 USD per GB (ESTIMATE, the acceptance run plan of 2026-11-17). "Alone" is how a user runs the item without the full run.

### 2.1 Base stages (merged)

| Item | Stage | Lane | Alone | Needs | Trust anchor | Time (E / M) | Not run is |
|---|---|---|---|---|---|---|---|
| provision a box: drivers, kf3 build, guest image | `provision` | gate | `scripts/bench/box/provision_full.sh` | KVM box, GPU, root, 700 GB disk | repo; NVIDIA installer by NVIDIA's checksum; ogkm by pinned commit | 8 min (M) | `provision` stage fails; cannot be skipped |
| CUDA ladder on the host, then in the guest | `cuda_host`, `cuda_guest` | gate | `scripts/bench/cuda_ladder.sh host\|guest` | provision | repo | minutes | stage fails by name |
| merge bar: 9 gates, 30 raw arms, 30 thin arms, birth census | `merge_bar` | gate | `scripts/bench/box/merge_check.sh SHA TAG` | provision | repo | about 1 h | stage fails; cannot be skipped |
| Linux apps, 66 gate rows (CUDA samples, torch, llama.cpp, blender, ffmpeg, vulkan, opencl, ...) | `apps` | gate | `scripts/bench/box/apps_gate.sh TAG PROV middle` | provision | upstream pins in `scripts/apps/pins.sh` | with the base leg 3.3 h (M) | `apps_not_run`; 4 rows are policy NOT_RUN (UVM, documented) |
| GPU-use assertion of the app rows (#475) | inside `apps` | gate | `KF_NEGCTL=1` runs the CPU control of each row | a kf3 guest | repo | none extra | a PASS that cannot be judged becomes NOTRUN and fails the row |
| llama.cpp built without the prebuilt web UI download (#469) | inside `apps` | gate | build of the apps bundle | network | pinned upstream commit | 4 min (M) | build step fails by name |
| documented install flow | `install_flow` | gate | `scripts/ci/install_flow.py` | provision | repo | 315 s (M) | `install_flow` fails |
| kfml: six ML research workloads on the box's own GPU (#455, #472) | `kfml` | gate | `python3 -m kfml run --tier large` (`scripts/kfml/README.md`) | a die with a pinned wheel flavour; 1.25 x VRAM <= 60 percent of free RAM for `managed_oversub` | hashed locks, datasets by sha256 in `pins.json` | 5 min warm (M) | `kfml_not_run` (strict) |

### 2.2 Open Linux rows and probes

| Item | Stage | Lane | Alone | Needs | Trust anchor | Time (E) | Not run is |
|---|---|---|---|---|---|---|---|
| FFmpeg NVENC/NVDEC matrix, 47 cases (#471) | `apps_discovery` | discovery | `sudo bash scripts/apps/open_rows/run_standalone.sh DIR` | NVIDIA driver, network | distribution packages; headers by pinned commit | up to 50 min per side | `apps_discovery_not_run:nvcodec_matrix` |
| NVML field sweep and DCGM (#471) | `apps_discovery` | discovery | same script | same; DCGM from NVIDIA's apt repository | apt's signed index; key from NVIDIA's URL | up to 55 min per side | `apps_discovery_not_run:nvml_dcgm`; **the DCGM half is NOT_RUN by design under the payload** |
| Vulkan CTS subset, 291 cases (#474) | `apps_discovery` | discovery | `python3 scripts/apps/vkcts/vkcts.py build`, then `run --side host\|guest` | `git cmake g++ python3`, 25 GB | pinned upstream commit, refused otherwise | build 10 min (M), run 15 min in a guest (M) | `apps_discovery_not_run:vkcts` |
| vLLM, TensorRT, JAX rows (#478) | `apps_discovery` | discovery | `sudo bash scripts/apps/mlrows/run_standalone.sh DIR` | network once, 25 GB, Turing or newer; vLLM needs CUDA 13 (R580+) | hashed locks (`pip --require-hashes`), model sha256 | setup 3 to 8 min each (M), runs 3 s to 2 min (M) | `apps_discovery_not_run:<row>` (vLLM on a CUDA 12 driver is a named NOT_RUN reason, so it fails here) |
| host census, the unserviced-RPC count and the host GPU-use record (#471) | inside the rows above | discovery | `scripts/apps/open_rows/census.py` | kf3 log | repo | none extra | the row's GPU-use proof is NA, never a pass |
| GPU utilisation reported to the guest, default on (#459) | device property | gate | `report-utilization=on\|off` on the device | kf3 | repo | none | not a stage; the NVML rows read it |
| Headless-graphics probes: `pv_vk_rt_ext`, `pv_validate_vk`, `vk_ofa` and 35 more items, 38 in all (`scripts/bench/gfxset`, `docs/design/V3_GFX_TESTSET.md`) | **none** | discovery | `bash scripts/bench/gfxset/suite.sh RUN` | a second guest image (`provision_guest_gfx.sh`) | repo, upstream pins | hours | **not in any stage: GAP** |

### 2.3 Windows

| Item | Stage | Lane | Alone | Needs | Trust anchor | Time (E / M) | Not run is |
|---|---|---|---|---|---|---|---|
| Windows gate (LTSC guest, offline, recorded clock) | `windows_gate` | gate | `scripts/bench/box/windows_gate.sh TAG PROV` | provision | pinned Windows media and driver (`docs/DOWNLOADS.md`) | 75 min (M) | `windows_retry_*` / `windows_gate` |
| Windows apps, profile `mid` (10 apps) or `full` (78) | `windows_apps` | gate | `KF_WINAPPS_PROFILE=mid bash scripts/bench/box/windows_apps.sh TAG PROV` | the gate's overlay | public bundle by its manifest hashes; official installers by checksum | 56 min (M) for mid | `windows_apps_retry_*` |
| second Windows run on the same box | `windows_gate_2` | gate | `windows_gate.sh TAGw2 PROV` | provision | as the gate | 35 min (M) | `windows_gate_2` |
| VK-GL-CTS (OpenGL 4.6, Vulkan, a two-context soak) and DirectX 12 samples, nine rows (#470) | `windows_open_rows` (new) | discovery | `python scripts\bench\windows\open_rows\kfopen.py run ROW` in an online Windows guest | the gate's overlay, an ONLINE guest with the real clock, 30 GB, network to the upstream hosts | pinned upstream commits; official checksum or vendor signature; lift explicit (`--accept-unanchored-downloads REASON`) | up to 6 h | `windows_open_rows_not_run`; **by design: `dxsamples_multiadapter` (two GPUs), `vkglcts_gl46` and `vkglcts_vk` (need a control run on native Windows)** |
| a new Windows 11 base image with a second layer of applications | none | being built separately | none | n/a | n/a | n/a | NOT BUILT: not in `FULL_STAGES` |

The stage `windows_open_rows` runs `windows_apps.sh` with the profile `open_source` and `WINVM_NET=online`, in a child of the gate's overlay: the clock rule (`docs/design/V3_CLOCK_RULE.md`) gives an online guest the real clock by itself; the proprietary images stay offline. The stage is graded by `vast_payload.windows_open_rows_rows` (the discovery rule), not by the apps-stage grader, which counts a failing open row as a guest failure.

### 2.4 Private images and the SPEC lanes

| Item | Stage | Lane | Alone | Needs | Trust anchor | Time (E) | Not run is |
|---|---|---|---|---|---|---|---|
| private image A, gate rows (solidworks, edrawings, and edge until #460 drops it from the gate lane) | `propimage_apps` | gate | `scripts/ci/propimage_apps.py run --image prebuilt:PATH --strict` | the image (file or presigned URL), AMD CPU profile, a swtpm 0.10 build | what the user passes: trusted, sha256 and size recorded; swtpm by the commit upstream's tag peels to (#466, #476) | 0.75 h | `STRICT_SKIP:imga:<app>:<reason>` |
| private image A, discovery rows (3dmark, superposition, resolve) | `propimage_apps` | discovery | same, `--app NAME=present,lane=discovery` | same | same | 1.5 h | `STRICT_SKIP` (the row must run; its verdict does not gate) |
| the Wukong benchmark | none | excluded | none | no verdict driver | n/a | n/a | **by design: left out of a strict list** |
| private image B, 8 gate rows (the GPU step and launch rows of its applications) (#446, #452, #458) | `propimage_apps_b` | gate | `scripts/ci/propimage_apps_b_stage.py run ...` | the image, the boot GOP, the open-asset data volume, 16 GB guest RAM | user objects; swtpm as above | 2.5 h | the stage's own named refusal |
| private image B, 26 discovery rows | `propimage_apps_b` | discovery | same, `--apps` | same | same | 4.2 h | the stage's own named refusal |
| Linux image C, 8 MATLAB GPU rows (#464, #465) | `matlab` | discovery (every row, standalone) | `vast_payload.py --only matlab --matlab-image PATH --matlab-driver V` | the image, a driver version or file | user object; driver by NVIDIA's checksum | up to 2.5 h | `STRICT_SKIP:<row>:<reason>` under `--full` |
| SPECviewperf lane A and lane B on the open LTSC guest, online with the real clock (#461) | `specviewperf` | gate | `KF_STAGES=specviewperf KF_SPEC=both` through the payload; `scripts/bench/box/specviewperf_stage.sh` | the SPEC bundle (presigned URL file), a disk that holds a lane's tree (`WINVM_DISK_GB`) | user object, recorded | 4.1 h | `specviewperf` lane rows; a guest that shut itself down is a named row reason, not a bare failure |

### 2.5 Planned and not built

| Item | PR | State | Note |
|---|---|---|---|
| Unreal installed build as a cached artifact for the open Linux guest | #462 | tooling and tests only, nothing built | built on a trusted machine, never on a rented box; no source and no cache is `UNREAL_NO_SOURCE_OR_CACHE`. Not in `FULL_STAGES` |
| a generic-runner entry for image B (`--propimage` with image B) | none | not built | `STRICT_UNKNOWN_APP:imgb:*`; the stage `propimage_apps_b` runs the rows meanwhile |
| a guest mode for kfml (`kfml_stage.py --where guest`) | #455 | designed and tested with a fake transport, never run | the payload's `kfml` stage runs on the host GPU only: it does not test the guest. **GAP** |
| a further private image | none | none specified | add a row to the instruction's `propimages` and a definition under `scripts/bench/windows/app_defs/` |

## 3. Known NOT_RUN by design (the only rows a complete run may report as not run)

| Stage | Rows | Why | Bucket |
|---|---|---|---|
| `apps` | `UnifiedMemoryStreams`, `UnifiedMemoryPerf`, `conjugateGradientUM`, `attach_verify` | unified memory is a documented limitation (owner ruling 2026-10-02) | documented limitation |
| `apps_discovery` | the DCGM half of `nvml_dcgm` | the payload's package installs hold no NVIDIA package (decided 2026-11-17); `run_standalone.sh` runs it | decision |
| `windows_open_rows` | `dxsamples_multiadapter` | needs two GPUs; a guest has one | hardware |
| `windows_open_rows` | `vkglcts_gl46`, `vkglcts_vk` | pass only against a control from native Windows on the same GPU model, a second machine | needs a second machine |
| `propimage_apps` | `wukong` | no verdict driver | known gap |
| (a die) | kfml rows whose RAM rule fails on a small box | `managed_oversub` needs 1.25 x VRAM of host RAM | box configuration; `kfml_not_run` in strict |

## 4. What this change wires (READ)

- `standalone_full.py`: the manifest, `refusals`, `check_flags`, `record`, `line`, `online_capable` (one TCP connect, nothing sent).
- **2026-11-20, first run of `apps_discovery` on a box (M = read from the box logs of one Vast RTX 4090 box on a stock Ubuntu 22.04 host; I = inferred; the fixes are GPU-free tested and mutated, and have NOT run on a box).** All six rows were red or NOT_RUN: `discovery_not_run:nvml_dcgm,vllm,tensorrt,jax`, and `nvcodec_matrix` and `vkcts` were wrongly counted as rows that ran. Causes and what this change does about each:
  1. **The stage booted the base guest image, not the apps image (M).** `run()` built its environment without `KF_GUEST_IMG`, so the boot helper used its default `guest.qcow2` (the boot log says `KF_GUEST_PROFILE image=/workspace/bench/guest.qcow2`; the `apps` stage's says `guest_apps.qcow2`), which has no `/opt/apps`; the boot helper opens that image writable and its mtime changed, so the six boots also dirtied the base guest the other legs use. Effects (M): `nvcodec_matrix`: `/opt/apps/bundle/run_apps.sh` missing, ssh rc 127, no result line, graded HANG (and HANG counted as a row that ran); `nvml_dcgm`: `/opt/apps/venv/bin/python` missing, rc 127, and its note was `-` because `run_apps.sh` took the header line of its own log (`--diag-timeout`) as the first match and then dropped it; `vkcts`: `VKCTS_CONFIG device is not a real GPU (name=- type=-)`, rc 3 (I: no libvulkan1 in the base image), graded KNOWN_BREAKAGE. Now: the stage boots only `guest_apps.qcow2` of `BENCH_DIR`, through a throwaway `qemu-img` overlay (the overlay carries a copy of the image's `.STAGED` driver receipt and is removed at the end); it refuses before any boot with FAIL `not_apps_image` (`KF_GUEST_IMG` names another file, or the apps image is a link to the base image) and RETRY `no_apps_image` (absent) or `overlay_failed`. The image's name is one constant in `scripts/kfci/apps_gate.py`, read by the gate and by the stage.
  2. **The wedge probe lied (M).** After any non-PASS row `apps_hook.sh` runs `/opt/apps/bundle/samples/vectorAdd`; with the binary absent every boot said `boot wedged after <app>`. Now: `no_samples_in_guest` and the boot goes on. A guest shell rc 126 or 127 with no result line is NOTRUN naming the missing command (`not-run:missing_command:<path>`), never HANG.
  3. **`vllm`: no `Python.h` (M: `MLROWS_FACTS ... "python_h": false`).** `python3-dev` was in neither `scripts/apps/image/recipe.json` nor `scripts/apps/setup_side.sh` (I: the apps image lacks it too). Now both list it; the recipe's pin of `run_apps.sh` was recomputed with the recipe tool's own hash (`build_image.sha256_file`, checked by `test_apps_image`). The image is not rebuilt here.
  4. **`tensorrt` and `jax`: the host-side setup refused `python_too_old: 3.10 < 3.12` (M)** (the locks are cp312, the guest is noble with 3.12.3, the box host is 22.04). Setup returns rc 0 on a refusal by design and the stage ignored it, so both rows were booted (about 26 minutes) before failing by name. Now: the stage reads `MLROWS_SETUP row=R status=NOT_RUN reason=WHY`, does not boot the refused rows, reports them NOT_RUN with the real reason (`python_too_old:3.10<3.12`) at once, runs the others, and still fails by name at the end. A setup `FAIL` is not a refusal (the guest run names it).
  5. **Two holes in the fail-by-name rule.** A `CONFIG` note on a FAIL (vkcts: not a real GPU, or GPU use not shown) and a HANG with no result line were graded as rows that ran. Now both are NOT_RUN; a GUEST_DEAD stays a listed breakage.
  **Not decided here (OPEN, design decision):** where Python 3.12 comes from on a 22.04 host. Options: build the ML venvs inside the provisioning guest (it has network and noble's 3.12); a noble chroot; a PPA is out (it breaks the trust-anchor rule of section 1 item 7). This change only makes the refusal early and named; until the decision, `tensorrt` and `jax` are NOT_RUN on a 22.04 host and the stage fails by name.
- `apps_discovery_stage.py` and the stage `apps_discovery`: builds the Vulkan CTS binary and the three ML environments, runs the host control when the host side of the matrix is installed (it runs first: the guest run's comparison reads the host log), then the six rows in the guest; verdict rule of section 1 item 5.
- the stage `windows_open_rows`: section 2.3.
- `matlab_stage.py --strict-rows`, passed by `--full`.
- `vast_report.py`: `validate_apps_discovery`, `validate_windows_open_rows`, `validate_full`; the bound on result rows is 32 (it was 19 in the validator and 20 in the Worker's `results.js`; one full run reports up to 25 rows, section 5).
- `ci/vast-broker/bandwidth.json` and `src/bandwidth.js`: the two stages have table entries and the plan twin knows them.

## 5. Gaps this audit found that are NOT wired (named, with the reason)

1. **kfml gates nothing about the guest**: the stage runs on the host GPU. `--where guest` is designed and never run (needs a booted guest and a transport).
2. **The headless-graphics probes are in no stage**: the gfxset suite needs its own guest image.
3. **Image B is not a generic-runner image**: `--propimage` with image B is refused by name; the stage is manual-only.
4. **`apps_matrix.sh ... all` includes two discovery rows and leaves out four**: `nvcodec_matrix` and `nvml_dcgm` are in the first list of `run_apps.sh`, the others are in `DISCOVERY`.
5. **The Windows open rows need an online guest and a second machine**: the stage runs, but a single-box run cannot give the conformance rows their control.
6. **The row-count bound** of the result file was below what one full run reports (section 4); raised to 32 in both readers.
7. **Pinned lists that name each other's text** (the manual-only stage table, the plan twin, the gate's job list, the recipe hashes of `run_apps.sh`) broke when branches that were each green were merged; two of them now read the source instead of spelling it.
