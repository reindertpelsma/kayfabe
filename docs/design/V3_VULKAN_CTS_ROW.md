# V3 Vulkan CTS row (open Linux row, `deqp-vk`, discovery lane)

**STATUS: LIVE as tooling and discovery lane, 2026-11-17. MEASURED natively once (RTX 3060 container, `traces/vkcts_host_20261117/`): build, host run, GPU-use assertion and the host-versus-host control. NOT RUN: any kf3 guest, so the host-versus-guest diff has no guest data yet (section 9 lists what that run must confirm).** Owner rulings that bind this row (2026-11-17): open first; a standalone third party trusts git only, builds from the repository alone and needs no private object, account, secret or licence; integrity of repository-fetched third-party sources is an upstream commit or an upstream-published digest; no sha256 of a binary or artifact of ours is a pin; no remote cache of a built binary in this change.

Labels (the only run behind a MEASURED below is the one of 2026-11-17 in `traces/vkcts_host_20261117/`): **MEASURED** = ran, and the sentence names the evidence. **CODE READ** = read in the pinned upstream source or in this repository. **INFERRED** = reasoning, not a run. **DESIGN** = intent.

## 1. What the row is

The Khronos Vulkan conformance suite (`KhronosGroup/VK-GL-CTS`, Apache-2.0, target `deqp-vk`) run on the die from a committed list of 291 cases, on the host and in the kf3 guest, with the same binary. Today the only Vulkan ray-tracing check in the matrix is the presence of the extension (`pv_vk_rt_ext`, `docs/design/V3_GFX_TESTSET.md`); this row exercises whole API classes per case: ray tracing pipeline and ray query, acceleration-structure build, copy, compaction and serialization, mesh and task shaders, sparse binding and residency, video decode and encode queues, timeline semaphores and synchronization2, descriptor buffers, device-generated commands and cooperative matrices (ranking: `APP_RANKING_FINAL_20261117.md`, Linux open row 1, a night file outside the repository). Every failing case is a minimal reproduction, and the host run of the same binary is the control, so the owner's three-bucket classification is mechanical.

**The pass criterion is not "all pass".** It is the DIFF of the per-case class list between the host run and the guest run on the same die. A case that passes on the host and does not in the guest is a candidate kayfabe bug; a case that fails on both is not kayfabe's (bucket 2, shown by the control); a case whose extension or feature is absent is `NotSupported` in the CTS's own vocabulary and is never a failure. The row reports and does not gate (discovery lane, `matrix_table.py` class `DISCOVERY`, in no gate list).

## 2. Trust and pins

- **The one source pin is the upstream commit** `0d18b347943e5e843529a05c95220e27fe30c538` (tag `vulkan-cts-1.4.7.1`, resolved on 2026-11-17 with `git ls-remote`), in `scripts/apps/vkcts/pins.json`. The fetch is by commit, the checked-out `HEAD` must equal it or the build refuses (`PinRefused`; the same rule as `pin_git_checkout` in `scripts/apps/pin_lib.sh`, in Python).
- **Externals** are fetched by the pinned CTS's own `external/fetch_sources.py` (and the two `fetch_video_*_samples.py`, whose sha256 per clip is upstream's). Before it runs, `pins.json` is compared with that script's text: every git external there must be pinned here with the same URL and the same revision, an external we do not pin is refused, and the two source archives and the RenderDoc header must carry the same sha256 as in the script. After it runs, every external's `HEAD` must equal the pinned commit and every fetched archive must hash to the pin. `video_generator` is a tag upstream (`v0.1.1`); the commit it resolved to on 2026-11-17 is pinned here and checked after the fetch, so a moved tag is refused. A reviewer checks one thing against upstream: the CTS commit; the rest follows from it.
- **Nothing we build is pinned or cached remotely.** `build` writes the sha256 of the binary into its manifest as a record (key `binary_sha256_recorded_not_pinned`) and nothing compares against it. A user who passes `--artifact` to `run` supplies a trusted artifact and a trusted `--caselist` (its digest is recorded); the committed list is checked against `pins.json`. The local build is keyed by the content of the pins and the build configuration (`content_key`) so a second `build` on the same disk is a no-op; that is a local convenience, not a trust anchor.
- **Future attested cache (DESIGN, not in this change).** A cache of the built artifact would be produced by a GitHub workflow that builds from the pinned commit, publishes the tarball with a GitHub attestation, and be used only after `gh attestation verify` against the pinned repository, workflow and source commit (owner rule 2026-11-17: an official upstream signature or an attestation, never a digest pinned in git). It would plug in at one place: `build()` would try `fetch_attested(key)` before `fetch_all`, fall back to the source build when absent or unverifiable, and a user-supplied artifact stays trusted and recorded. Worker runs may use an R2 object for the same artifact (owner addendum 2026-11-17), the repository default stays build-from-source.

## 3. Build

`vkcts.py build`: shallow fetch of the pinned commit (about 1.8 GB checkout), the CTS's fetch scripts, `cmake` with `DEQP_TARGET=vulkan_headless` (CODE READ, `targets/vulkan_headless/vulkan_headless.cmake`: no window system, no X, no GL/EGL), `SELECTED_BUILD_TARGETS=deqp-vk` (CODE READ, top `CMakeLists.txt`: only that target and its dependencies are built), `Release` with `-O1 -g0` and static libstdc++, ninja when present (make otherwise), jobs from the CPU quota (cgroup aware) and memory (2 GB per job), a time-box of 5400 s, a refusal under 25 GB free. Install: stripped binary and `external/vulkancts/data/vulkan` (amber files, video clips) under `<root>/<key>/`, `<root>/current` points at it.

**MEASURED, 2026-11-17** (`traces/vkcts_host_20261117/build_manifest.json`): on a Vast CUDA container (13.44 cores by cgroup quota, 62 GB, g++ 11.4.0, cmake 3.22.1, ninja 1.10.1) the fetch took 237 s (the shallow clone and all externals) and the build 341 s for 1555 ninja steps, 13 jobs. The stripped `deqp-vk` is 77,488,128 bytes, the installed artifact 209 MB, the source tree 2.2 GB, the build tree 1.7 GB. The inferred 30-60 minutes of the ranking note was an overestimate by a factor of five or more for this configuration; a box with fewer cores scales roughly with `1555 steps / cores` (INFERRED). Network volume: INFERRED 1.5-3 GB (the disk size of the checkouts is MEASURED, the bytes on the wire were not).

Packages on the build machine: `git cmake python3 g++ make` (+ `ninja-build` optional). To run: `libvulkan1` and the NVIDIA Vulkan ICD. **MEASURED** on the container (same trace): the vendor's ICD JSON and `libGLX_nvidia.so.0` are mounted by the provider, but `vkCreateInstance` fails until `libxext6`, `libx11-6` and `libegl1` are installed (the ICD loads them); after that `vulkaninfo` shows the RTX 3060 as `DISCRETE_GPU`, API 1.4.312.

## 4. The case list

`scripts/apps/vkcts/caselist.tsv` (`area TAB case`, 291 cases, sha256 in `pins.json`), generated by `gen_caselist.py` from `caselist_spec.json` and the main mustpass of the pinned commit (`external/vulkancts/mustpass/main/vk-default.txt`, 3.2 million names). Per bucket the names are grouped by prefix plus `depth` components and a deterministic evenly spaced pick is taken from the groups (rule printed in the spec); no case is chosen by how it behaves on any machine. Areas and quotas: `rt_pipeline` 28, `accel_structure` 44 (build/compaction/copy/serialization, host threading, update, flags), `ray_query` 22, `mesh_task` 30 (10 of them task shader cases), `sparse` 30, `video_decode` 16, `video_encode` 15, `video_other` 12 (profiles, capabilities, synchronization), `timeline` 15, `sync2` 18, `descriptor_buffer` 18, `dgc` 20, `coop_matrix` 21, and the two `info` cases that open the list and give the device identity and the extension set. The list is part of the experiment: a change needs a new digest in `pins.json`, and `run` refuses a committed list whose digest differs.

## 5. Running

`vkcts.py run` writes the list in batches of 40 and starts one `deqp-vk` per batch (`--deqp-caselist-file`, `--deqp-log-filename`, `--deqp-archive-dir`, logs of images, shader sources and SPIR-V off, shader cache off, crash handler on, device 1, with the NVIDIA ICD forced through `VK_DRIVER_FILES` when it is found, so a software rasteriser on the machine cannot be device 1).

- **Parsing** (CODE READ: `framework/qphelper/qpTestLog.c`, `framework/common/tcuTestSessionExecutor.cpp`, `tcuApp.cpp`): the QPA log is read as a stream of `#beginTestCaseResult` / `<Result StatusCode="X">` / `#endTestCaseResult` / `#terminateTestCaseResult` markers; the console's `Test run totals` is a second witness and a disagreement is noted (`console_totals_disagree`). A fixture of a real log is `scripts/ci/fixtures/vkcts_real_qpa.txt`. The binary runs cases in tree order, not list order (MEASURED 2026-11-17, `scripts/ci/fixtures/vkcts_real_console.txt`); results are returned in list order.
- **Crash and hang attribution** (`vkcts_lib.execute`): a case that was open when the process ended is `Crash` (or `Timeout` when the harness killed it after `case_timeout_s` of QPA silence), the run continues after it in a new process; a clean exit that leaves a case without a result marks it `NotRun`; more than six consecutive batches without any result (the seventh) stop the run (a wedged device must not burn the budget); a 2100 s budget bounds everything; unreached cases are `NotRun`, never silently dropped. A case never counts as passed because it was not seen.
- **Classes**: `pass`, `warn` (QualityWarning, CompatibilityWarning, Waiver), `not_supported`, `fail` (Fail, ResourceError, InternalError, DeviceLost, and any status this table does not know), `crash`, `timeout`, `not_run`.
- **Device identity** (CODE READ + MEASURED on the real log): the properties and extensions of `dEQP-VK.info.device_properties` and `...device_extensions` give name, type, vendor, API, driver and the extension set. A device that is not `DISCRETE_GPU`, `INTEGRATED_GPU` or `VIRTUAL_GPU`, or whose name says llvmpipe, lavapipe, SwiftShader or softpipe, fails the row (exit 3): the row did not exercise the GPU.
- **GPU use** is asserted from the HOST counters, like the other rows (`scripts/bench/windows/gpu_assert.py`: `evaluate` over `nvidia-smi --query-gpu` and `dmon`, requirement `gpu` and `mem`): the sampler runs on the host around the run; for the guest run `apps_hook.sh` starts and stops it from the host around the `ssh` call (the guest's own utilisation counters are `[N/A]` under kf3, `V3_PROPIMAGE_LINUX.md` section 7). An idle GPU, an unreadable GPU and no `nvidia-smi` are three FAIL reasons (exit 4).
- **Output**: lines `VKCTS_SUMMARY`, `VKCTS_DEVICE`, `VKCTS_EXT`, `VKCTS_RUN`, `VKCTS_GPU`, up to 60 `VKCTS_DETAIL` lines (first lines of the log of each failing case), the per-case block `VKCTS_RESULTS_BEGIN v1 side=<s>` ... `VKCTS_RESULTS_END n=<N> digest=<16 hex>` (TSV: case, CTS status, class, area, short detail; about 45 KB), `DIGEST vkcts_cases <16 hex>` (the existing `APPDIG` channel: `summarize.py` already compares it host against guest) and `APPNOTE`. Everything that comes from the guest is sanitised to a short printable line and accepted only for case names of the list; `parse_block` refuses a missing end, a wrong count or digest, an unknown status or class and a duplicate.

## 6. The diff

`vkcts.py diff <results-dir>` (or `--host LOG --guest LOG`) reads the two blocks, matches cases by name (a case on one side only is `INCOMPLETE`), and prints the non-identical cases, one `VKCTS_DIFF` summary line, the extensions one side has and the other lacks (`VKCTS_EXT_DIFF`, the usual root cause of lost support), the host census line of each side and the counts per area. `--strict` exits 1 when there is a candidate (for a later gate lane; the discovery lane never uses it).

| host | guest | category | meaning |
|---|---|---|---|
| ok | ok | `SAME_OK` | the same, pass or warning |
| not supported | not supported | `SAME_NOT_SUPPORTED` | absent on both |
| bad | bad (same class) | `BOTH_FAIL` | not kayfabe (bucket 2: the control fails too) |
| bad | bad (other class) | `BOTH_FAIL_DIFFERENT` | information |
| ok | fail, crash, timeout | **`GUEST_ONLY_FAIL`** | candidate kayfabe bug |
| ok | not supported | **`GUEST_LOSES_SUPPORT`** | candidate: a feature or extension the guest hides (see `VKCTS_EXT_DIFF`) |
| not supported | ok | **`GUEST_GAINS_SUPPORT`** | candidate: an anomaly |
| not supported | bad | **`GUEST_FAIL_HOST_NS`** | candidate: the guest offers what it cannot do |
| bad | ok | `HOST_ONLY_FAIL` | host flake or host configuration |
| bad | not supported | `HOST_FAIL_GUEST_NOT_SUPPORTED` | information |
| not run | any | `INCOMPLETE` | no verdict |

Every candidate ends in one of the owner's three buckets: (1) a documented kayfabe limitation, (2) an app or harness bug shown by a control (the host run is the control; a second host run gives the noise floor, equal digests on 2026-11-17 in `traces/vkcts_host_20261117/diff_run1_vs_run2.txt`), (3) our own configuration error.

## 7. How it is wired into the matrix

`scripts/apps/run_apps.sh` has a second list, `DISCOVERY`, with the row `vkcts` (timeout 2700 s, command `python3 $B/vkcts_tool/vkcts.py run --side $SIDE --artifact $B/vkcts --out $O/vkcts`). `list` names it, `listall` and `all` do not, so `apps_matrix.sh ... all` and the gate stage never run it; an `APPNOTE <text>` line in an app's log is its row note. `apps_matrix.sh` copies the tool (`vkcts.py`, `vkcts_lib.py`, `pins.json`, `caselist.tsv`, `gpu_assert.py`) to `/opt/apps/bundle/vkcts_tool` on the host, links `vkcts/current` as `/opt/apps/bundle/vkcts` and runs the diff when both logs exist; `apps_hook.sh` pushes the same into the guest and brackets the row with the host sampler. `matrix_table.py` classes it `DISCOVERY` and keeps it out of `--gate-list`. The provisioning is not in `build_bundle.sh` (the 45-minute cap of the stage must not grow) and not in `expected_rows.txt`: it is a separate `vkcts.py build`. The image recipe pins the sha256 of `run_apps.sh` and was updated with it.

**Commands.** Standalone, native or in any machine that has the NVIDIA driver and Vulkan loader: `python3 scripts/apps/vkcts/vkcts.py check`, `build`, `run --side host --out out/host` (build about 10 minutes on 13 cores, run about 90-100 s; MEASURED). On a kf3 box, after `build --root /workspace/apps/vkcts`: `bash scripts/apps/apps_matrix.sh host RUN vkcts`, `bash scripts/apps/apps_matrix.sh guest RUN vkcts`, then `python3 scripts/apps/vkcts/vkcts.py diff /workspace/apps/results/RUN` (the matrix script already runs it when both logs exist). The guest run time is not measured (INFERRED: a few times the native 90 s; the row timeout is 2700 s).

## 8. Hypotheses and results

Each was stated with its falsifier before the run (2026-11-17, in the session that built the row); results are MEASURED in `traces/vkcts_host_20261117/`.

| # | Hypothesis | Falsifier | Result |
|---|---|---|---|
| H1 | the `deqp-vk` build finishes inside 90 minutes on a 13-core container | not finished, or fails | held: 341 s |
| H2 | after the CTS fetch scripts every pinned external is at its commit and the fetch script says what `pins.json` says | `PinRefused` | held (the build ran to the end; the check is in the code path) |
| H3 | the device identity is readable from `info.device_properties` of a real log | unknown device, exit 3 | held: `NVIDIA GeForce RTX 3060`, `DISCRETE_GPU`, vendor 4318, API 4211000, 250 extensions |
| H4 | the host counters show GPU use (at least 2 samples at or above 3 % and a memory rise of at least 64 MiB) | `verdict FAIL` | held: run 1 26 busy samples, peak 16 %, +494 MB; run 2 20 busy, peak 54 %, +494 MB |
| H5 | every case of the list exists in the binary | any `NotRun` | held: 0 `NotRun` |
| H6 | two host runs give the same class list (noise floor of the row is zero) | different digests | held: `70d5ed8f0ca07431` twice, and again through `run_apps.sh` |
| H7 | in a kf3 guest the same list has no `GUEST_ONLY_FAIL` and no `GUEST_LOSES_SUPPORT` | any candidate | **not run** |

## 9. Native first run (MEASURED, 2026-11-17, RTX 3060 GA106, driver 580.159.03, `traces/vkcts_host_20261117/host_run1_results.tsv`)

291 cases, 189 pass, 102 not supported, 0 fail, 0 crash, 0 timeout, 87-103 s.

| area | pass | not supported | main reasons (CTS text, counts over the run) |
|---|---|---|---|
| info | 2 | 0 | |
| rt_pipeline | 22 | 6 | `accelerationStructureHostCommands` absent (3), `VK_EXT_ray_tracing_invocation_reorder` absent (2) |
| accel_structure | 22 | 22 | `accelerationStructureHostCommands` absent (18), `VK_KHR_device_address_commands` (3) |
| ray_query | 20 | 2 | `accelerationStructureHostCommands` |
| mesh_task | 27 | 3 | `storageInputOutput16` |
| sparse | 17 | 13 | device-group sparse binding with one device (8), colour attachment format (2) |
| video_decode | 3 | 13 | separate reference images (6), `VK_KHR_unified_image_layouts` (6) |
| video_encode | 4 | 11 | `VK_KHR_video_encode_av1` (5), `VK_KHR_video_encode_feedback2` (2) |
| video_other | 7 | 5 | profile formats, AV1 encode |
| timeline | 15 | 0 | |
| sync2 | 10 | 8 | `VK_FUCHSIA_external_semaphore`, `VK_KHR_internally_synchronized_queues` |
| descriptor_buffer | 18 | 0 | |
| dgc | 15 | 5 | required DGC stages, `VK_EXT_descriptor_heap` |
| coop_matrix | 7 | 14 | combination not supported (6), `VK_EXT_cooperative_matrix_maintenance1` (4) |

Observation (MEASURED 2026-11-17, `traces/vkcts_host_20261117/host_run1_results.tsv`): a die with the host's driver reports these classes as supported, so a `GUEST_LOSES_SUPPORT` in the guest names a feature kayfabe hides; the 102 not-supported cases are the host's baseline, not a finding.

## 10. What the guest run still has to confirm

1. The guest run completes on a kf3 guest: the tool and the 209 MB artifact push over `ssh`, the row finishes inside its 2700 s, the diff runs.
2. `VKCTS_GPU side=guest source=host` PASS: the host counters show work while the guest runs the list (the sampler is bracketed by the hook; written and unit-tested, never run against a guest).
3. The guest's `info` cases give the same device name and a comparable extension set; the candidates of the diff, if any, are classified into the three buckets with a second host run as the control (the noise floor was zero in the one pair measured).
4. The CTS runs many cases per process; a guest wedge inside a batch is attributed to the open case and the next batch starts after it; whether the guest recovers after a crash is not known (the discovery lane lets the row fail without stopping later rows).
5. The class `video_*`: kf3's NVDEC/NVENC engines are "derived from source and not yet measured" (`V3_VIDEO_ENGINES.md`); these 43 cases are likely where the first candidates appear (INFERRED).

## 11. Files

`scripts/apps/vkcts/`: `pins.json`, `vkcts_lib.py` (pure logic), `vkcts.py` (fetch, build, run, census, diff), `caselist.tsv`, `caselist_spec.json`, `gen_caselist.py`. Tests `scripts/ci/test_vkcts.py` (fixtures in `scripts/ci/fixtures/vkcts_real_*.txt`); mutants `scripts/ci/vkcts_mutants.py` (two named killers each, in `ci.yml`). Usage: `docs/howto/vulkan-cts.md`.
