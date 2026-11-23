# Open lanes: nvbandwidth, llama.cpp on Vulkan, LibreHardwareMonitor (NVAPI)

**STATUS: LIVE, 2026-11-23 for the code and the GPU-free tests of all three lanes; the two Linux rows have a native run on one RTX 4090 Vast box (section 8); the Windows row has never run anywhere and is `DESIGN-ONLY` until a Windows run is recorded. No row has run in a kayfabe guest.** Parent documents: [`V3_OPEN_ROWS_FFMPEG_DCGM.md`](V3_OPEN_ROWS_FFMPEG_DCGM.md) (the Linux discovery rows these copy), [`V3_GPU_USE_ASSERTION.md`](V3_GPU_USE_ASSERTION.md) (the judge and the negative controls), [`V3_WINDOWS_OPEN_ROWS.md`](V3_WINDOWS_OPEN_ROWS.md) (the Windows open rows `kfopen.py` runs).

Labels: **[run: <directory>]** a logged run, **[read]** a file or page read, **[inferred]** reasoning, not tested, **[design]** intent.

## 1. What was asked, and the rules that decide the shape

Three open candidates from the 2026-11-23 candidate list (a private research note, not in the repository; its ranking is [inferred] effort and value, not a measurement) had no lane: `nvbandwidth` (rank 1: the copy-engine paths, seen so far only as a DCGM diagnostic that skips on GeForce), LibreHardwareMonitor (rank 3: the first real NVAPI consumer, no NVAPI row existed) and llama.cpp's Vulkan backend on Linux (rank 4: cooperative-matrix compute and a pipeline-creation burst, only the Windows Vulkan bench existed).

Owner rules that bind them (project memory, paraphrased; binding): open first and standalone (build or install at run time from the repository, trust git only, no private object); the only accepted anchor of a downloaded binary is an official-origin checksum or a GitHub attestation, a sha256 of a binary pinned in git is none, and others bring their own configuration; a row that can fall back to the CPU asserts GPU use (the host census first, the application's own proof second, and a negative control, the same row with the GPU hidden, that must fail); no licence time checker; no environment variable as user configuration; shell is glue and logic is Python; every download is routed through `kf_trust` or listed with a reason in the unrouted manifest.

## 2. The three rows

| | `nvbandwidth` (Linux) | `llama_vulkan` (Linux) | `hwmon_lhm_nvapi` (Windows) |
|---|---|---|---|
| runs | the 35 test cases of nvbandwidth v0.9, each in its own process (`-j -t ID -b 512 -i 3`) | `llama-bench` of the pinned llama.cpp commit built with `-DGGML_VULKAN=ON`, the CUDA rows' model, `-ngl 99 -p 512 -n 64 -r 2` | `lhm_dump.ps1` loads `LibreHardwareMonitorLib.dll` from the release zip and prints the sensors as JSON; `hwmon_row.py` judges it |
| timeout | 1800 s for the row, 300 s per case | 1200 s | 300 s for the reader (row definition in `open_rows.json`) |
| pass | no case FAILed or MISMATCHed; every case that needs one GPU is `Passed` with a positive figure at or above 0.05 GB/s (any positive latency); every case that needs two GPUs is `Waived` on a machine with one (and the table is the machine's own answer: a peer case that passes on one GPU is a MISMATCH); a copy-engine, an SM-copy, the latency and the local-copy case passed | ggml_vulkan lists a device and it is an NVIDIA GPU (not llvmpipe, lavapipe, SwiftShader); every table row is on the Vulkan backend with ngl at least 1; `pp512` and `tg64` both have a positive t/s; exit status 0 | the library found hardware of type `GpuNvidia` named like an NVIDIA GPU; temperature, clock, load and memory-size sensors each have a numeric value; temperatures within 1 to 125 C, loads within 0 to 100 percent, clocks and memory total above 0; `nvidia-smi` temperature within 15 C and memory within 5 percent where it answers numbers; the reader raised nothing |
| GPU-use assertion | `gpu_use.proof_nvbandwidth` reads the row's `NVBANDWIDTH_PROOF` line (device name, GPU count, passed cases per class, smallest figure); class `a+proof` (a copy has no CPU form); native runs also sample the machine's nvidia-smi (memory.used rise) and print `gpu_use=`; in a guest the host census is the authority (`gpu_use_rows.ROWS`) | `gpu_use.proof_llama_vulkan` reads the log (device line and table rows); class `b` (a CPU Vulkan driver, `-ngl 0` and no driver all still print a table); host census and host engines required of a guest, memory rise of a bare-metal run | guest self-report only: the proof is `KFOPEN HWMON gpu=NVIDIA...`, the NVAPI hardware record's own name (no Windows row has a host-authored assertion, [V3_GPU_USE_ASSERTION.md](V3_GPU_USE_ASSERTION.md) section 2.2); a sensor reader does no GPU work, so the assertion is that the answer came from NVAPI and not from a fallback |
| negative control | the same row with `CUDA_VISIBLE_DEVICES` empty (`KF_NEGCTL=1` of `run_apps.sh`, through `KF_GPU_ENV`): no device is listed, nothing passes, the row and the judge both say FAIL | the same row with `VK_DRIVER_FILES` and `VK_ICD_FILENAMES` pointing at nothing and `-ngl 0` (`KF_VK_ENV`, `KF_NGL`): ggml finds no device, the row says `no_vulkan_device`, the judge FAIL | `kfopen.py run hwmon_lhm_nvapi --negctl`: the same reader with `IsGpuEnabled = false`; exit 0 only when the judge fails it (`CONTROL_OK`) |
| expected artifacts | `results.tsv` (case, class, result, reason, status, best figure, seconds), `summary.txt`, `hostsmi.csv`, `logs/<case>.log`; `--emit` prints the first two between `@@FILE` markers | `bench.log`, `summary.txt`, `hostsmi.csv` | `KFOPEN HWMON`, `CHECK` and `KFOPEN VERDICT` lines; the build receipt `C:\kfopen\out\build-receipt.json` |
| matrix status | discovery row of `run_apps.sh`, in no gate list | discovery row of `run_apps.sh`, in no gate list | **deferred** as an `apps.json` row (section 6) |

The row definition of the Windows row is data: `scripts/bench/windows/open_rows/open_rows.json`, row `hwmon_lhm_nvapi`, including the `matrix.row` block (id, category, session, tier, timeouts, success regex, proof) that one call of `gen_apps.open_row(...)` will wire.

## 3. The GPU workload classes these rows add

The classes are named from the candidate list and are [inferred]; the runs of section 8 show which of them a native GPU served.

- **nvbandwidth**: host-to-device and device-to-host copies by the **copy engine** (`cuMemcpyAsync`), the same by **copy kernels on the SMs**, both directions at once, device-local copy, and a **pointer-chase latency kernel**. These are CE-versus-SM channel classes: a VMM that forwards only the compute channel class would pass `vectorAdd` and fail these.
- **llama.cpp on Vulkan**: **Vulkan compute** with **cooperative-matrix** shaders (the `matrix cores:` field: `KHR_coopmat`, `NV_coopmat2` or `none` is recorded, never required), device-local buffers of the model's size, and the **pipeline-creation burst** at load (many compute pipelines created in a short time through the Vulkan driver, which the CUDA rows and `vkpeak` do not make). The burst is not counted by the row; its wall time is part of `secs=`.
- **LibreHardwareMonitor**: the first **NVAPI** consumer (`nvapi64.dll`, the driver's private query interface: thermals, clocks, loads, memory sizes, fans) among the Windows rows, which until now exercised NVML only through `nvidia-smi`.

## 4. Install recipes and the trust anchor of each input

| input | how it arrives | anchor |
|---|---|---|
| nvbandwidth source (Apache-2.0) | `provision_lanes.sh` step `nvbandwidth`: `pin_git_checkout` of `https://github.com/NVIDIA/nvbandwidth.git`, tag `v0.9`, commit `4a49bdae82d69e69330a88746cb12b449796ee8b` (`pins.env`), then `cmake` with `-DCMAKE_CUDA_ARCHITECTURES="75;80;86;89;90"` (the last with PTX) | pinned upstream commit, HEAD checked by `pin_lib.sh` |
| Boost program_options (nvbandwidth links it statically), cmake, g++ | the distribution's apt archive | the distribution's signed archive; nothing is downloaded by us |
| CUDA toolkit (nvcc, nvml.h) | the toolkit the machine has (the bundle build installs the pinned one first), else `pin_install_cuda_toolkit` of `pin_lib.sh`: NVIDIA's repository, keyring `.deb` pinned by sha256 per Ubuntu release, `cuda-toolkit-12-6=12.6.3-1` | the existing sanctioned path (not a new apt bypass); a keyring digest is a signing key, not a binary of ours |
| llama.cpp source (MIT) | `provision_lanes.sh` step `llama_vulkan`: `pin_git_checkout` of `PIN_LLAMA_URL` at `PIN_LLAMA_COMMIT` of `scripts/apps/pins.sh`, the CUDA rows' commit (one pin; a test fails on a second one), built into `build-vk` with `-DGGML_VULKAN=ON -DGGML_CUDA=OFF`, target `llama-bench` | pinned upstream commit |
| Vulkan headers, loader development files, `glslc` | `apt` packages `libvulkan-dev glslc` (`pins.env` `OPENROWS_VULKAN_BUILD_PACKAGES`) | the distribution's signed archive |
| the GGUF model | the CUDA rows' model: Hugging Face at an immutable revision, sha256 in `pins.sh`; step `llama_model`, a non-default step run by `run_standalone.sh` for a machine without `/opt/apps/data` | data, pinned by revision and digest as the CUDA rows do; not a binary of ours |
| LibreHardwareMonitor v0.9.6 release zip (MPL-2.0) | `kfopen_build.ensure_lhm`: `https://api.github.com/repos/LibreHardwareMonitor/LibreHardwareMonitor/releases/tags/v0.9.6` gives the asset's `digest`, the asset URL must lie under the pinned release, the download must hash to it, then it is unpacked under `<root>/tools/lhm` | **an official-origin checksum**: the release's own sha256 read at run time, the anchor class of the MinGit download of the existing Windows open rows (same origin as the asset: integrity and transport, not authenticity against a compromised upstream, as `V3_WINDOWS_OPEN_ROWS.md` section 4 says); no digest of the zip is in git (`validate_pins` refuses one); `--accept-unanchored-downloads REASON` lifts it, recorded in the receipt, and `--tool lhm=DIR` brings your own |

Not chosen: building LibreHardwareMonitor from source (needs the .NET SDK and its restore, a second toolchain for a sensor reader), and the nuget.org package `LibreHardwareMonitorLib` (repository-signed, but its .NET Framework build depends on six further packages whose assembly versions PowerShell 5.1 would have to bind by hand; the release zip carries the exact set the project's own executable runs with). The anchor the zip lacks, an attestation, is not available: the GitHub attestations API has none for the zip's sha256 [read: `gh api repos/LibreHardwareMonitor/LibreHardwareMonitor/attestations/sha256:<digest>` answered 404, 2026-11-23]. nvbandwidth publishes no prebuilt binary at all (its releases v0.9 to v0.11.0 have no assets [read: the releases API, 2026-11-23]), which is why it is built from a pinned commit. A guest that cannot reach GitHub reports `KFNOTRUN ... offline-guest-without-lhm`.

Under the Vast payload `apt-get` is the cache-only shim (V3_OPEN_ROWS_FFMPEG_DCGM.md section 3): `libboost-program-options-dev`, `libvulkan-dev` and `glslc` are not in the reviewed package set, and widening that set would need a cache rebuild and its own tests. The two steps therefore end `OPENROWS_<step>=FAIL` under the payload and the rows report `NOTRUN` with `binary_missing`; a standalone machine builds them. [design] this is the same decision as DCGM's, taken here and not argued again.

## 5. Files

| file | role |
|---|---|
| `scripts/apps/open_rows/nvbandwidth_row.py`, `nvbandwidth_cases.tsv` | the runner and the pure judges; the table of the 35 cases (class, GPUs needed) |
| `scripts/apps/open_rows/llama_vulkan_row.py` | the runner and the pure judge of the Vulkan row |
| `scripts/apps/open_rows/provision_lanes.sh`, `pins.env` | the build steps (glue, under 100 lines, every command of a step ends in `|| return 1` because `set -e` is off inside `if`) and the nvbandwidth pins |
| `scripts/bench/gpu_use.py`, `scripts/apps/gpu_use_rows.py` | `proof_nvbandwidth`, `proof_llama_vulkan`; the two rows' classes, engines and `app_missing` |
| `scripts/apps/run_apps.sh`, `apps_hook.sh`, `build_bundle.sh`, `matrix_table.py`, `open_rows/compare_logs.py`, `open_rows/run_standalone.sh` | the rows in the DISCOVERY block, the negative-control variables (`KF_VK_ENV`), the push of `llama_vk/`, the bundle build call, the compare front end and the standalone run |
| `scripts/bench/windows/open_rows/hwmon_row.py`, `lhm_dump.ps1`, `open_rows.json`, `kfopen*.py` | the Windows row: judge and runner, the 30-line PowerShell reader, the pins and the `release-zip` tool kind, the `--negctl` flag |

## 6. The Windows row and why it is not an `apps.json` row yet

`appmatrix/apps.json` is a hash-covered input of the published public Windows apps bundle (`appmatrix/bundle.pin`, `package_bundle.input_hash`, the `RECIPE_INPUT_DRIFT` guard). A new row there changes the input hash and turns `test_bundle_pin.RepoPin` red until the coordinator republishes the bundle to the public bucket; that is not this change's to do. So the row lives in `open_rows.json` (with its matrix definition as data and tests that hold that definition against what the code prints) and runs through `kfopen.py` on any Windows machine, which is also how the other open Windows rows are driven by hand. [design] When the bundle is next republished, wiring it is one `open_row("hwmon_lhm_nvapi", ...)` call from `matrix.row`, the `online_source` list and `app_requires` entry of `gate_profiles.json`, and `gen_doc.py`.

`lhm_dump.ps1` loads the library with `Assembly.LoadFrom` and resolves every dependency from the zip's directory with an `AssemblyResolve` handler, because the zip's `LibreHardwareMonitor.exe.config` redirects assembly versions for the executable and the reader is `powershell.exe`. That handler, `Computer.Open()` under PowerShell 5.1 and the NVAPI sensor names are [inferred] from the library's structure; nothing of this ran on Windows.

## 7. Tests and mutants (GPU-free)

- `scripts/ci/test_open_lanes.py` (117 tests): the case table against the 35 testcases of the pinned source; the parsing of nvbandwidth's JSON, listing and device list; the case judge (waived, mismatch, error, timeout, crash, no figure, floor, hidden GPU); the row summary and its class requirements; the proof line and its judge; the Vulkan judge on the one real Vulkan log of the repository (a native Windows RTX 4060 Ti, `scripts/ci/fixtures/open_lanes/`) and on constructed fallback logs (a CPU driver, no device, `-ngl 0`, a CPU backend row, no table); both runners end to end against fake tools that print what the real ones print; the real `run_apps.sh` with those fakes, default and `KF_NEGCTL=1`; text pins for the pins, the provision script and the wiring (marked as pins by their names: they prove a line is there).
- `scripts/ci/open_lanes_mutants.py` (62 mutants, two named killers each, in the `open rows mutants` job of `ci.yml`): a waived case that passes, a case with no figure that passes, a hidden GPU, a software Vulkan driver, a CPU backend, `-ngl 0`, a negative control that does not hide the GPU, a missing proof line that passes, a pin that is not a commit, a digest in `pins.env`, the bundle build able to fail the gate. `gpu_use_mutants.py` and `open_rows_mutants.py` of the existing rows stay green with the new rows added.
- `scripts/bench/windows/open_rows/tests/test_hwmon.py` (63 tests) and 22 new mutants in `open_rows_mutants.py` (98 in all, two named killers each): the judge on a constructed dump (the format of `lhm_dump.ps1`), every check against its failure, the negative control in all three outcomes, the release-zip anchor (hash mismatch, foreign URL, missing digest, lifted, user tool, no library in the zip), the pins policy, the preflight, the dispatch and the deferred matrix definition.

Not covered, said plainly: nothing runs PowerShell, .NET, NVAPI or Windows; the real output of nvbandwidth is covered by constructed JSON written to the format of its source (and by the native outputs of section 8 once they are fixtures); no test exercises the CMake or Vulkan builds.

## 8. Native results

(filled from the box run; see `traces/open_lanes_20261123/`)

## 9. Hypotheses and falsifiers (stated before the box run)

1. **H1** nvbandwidth v0.9 builds with CUDA 12.6 for sm_75 to sm_90 and, natively on an RTX 4090, all 18 single-GPU cases are `Passed` and the 17 peer cases `Waived`. Falsifier: a build error, or any single-GPU case not `Passed`.
2. **H2** With `CUDA_VISIBLE_DEVICES` empty the row FAILs and `gpu_use` judges the log `CONTROL_OK`. Falsifier: the binary lists the GPU anyway, or the row or the judge passes.
3. **H3** llama.cpp at the CUDA rows' commit builds with `-DGGML_VULKAN=ON` against the distribution's `libvulkan-dev` and `glslc` of the box's Ubuntu. Falsifier: configure or compile fails (Vulkan headers too old, no `glslc`).
4. **H4** On the RTX 4090 the Vulkan row passes and ggml reports a cooperative-matrix extension (`coopmat` other than `none`). Falsifier: `coopmat=none` or a failed row.
5. **H5** With the NVIDIA ICD hidden from the Vulkan loader (`VK_DRIVER_FILES` set to a missing file) ggml finds no device and the row says `no_vulkan_device`. Falsifier: ggml finds a device anyway, or the process crashes.
6. **H6** In both rows the native census records a memory rise of at least 16 MiB (`gpu_use=PROVEN`). Falsifier: `UNPROVEN` or `FAILED`.

## 10. Not measured

A kayfabe guest run of any row; the host census of these rows (kf3 submissions); the ratio thresholds of `compare` (reported, not judged); the copy-engine figures on any other die; the Windows row on Windows (the reader, the library, the sensor names and the tolerances against `nvidia-smi`); the cooperative-matrix shaders on a die without the extension; a data-center die.
