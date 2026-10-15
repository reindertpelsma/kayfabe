# V3 Blender test suite on the Windows guest - plan

**STATUS: DESIGN-ONLY, 2026-10-15.** Nothing here has run in the guest or on any GPU. The only runs were a CPU-only
proof of the runner and the data fetch on the host (section 9). Every claim is tagged **measured** (a command I ran or a
size I read, with the revision) or **inferred** (reasoned, not observed). Upstream means
`https://projects.blender.org/blender/blender`, tag `v5.2.0` = commit `fbe6228777e7d9afefcd61a413844e790ae75db7`
unless another tag is named. Upstream text is not copied here; only paths and short command lines are quoted.

## 0. Summary

- Blender's own tests that touch the GPU are **render regression tests**: a Python runner renders every `.blend` in a
  directory with a *release-style* `blender` binary and compares the PNG with a reference using `oiiotool`. They need
  no build tree. They cover Cycles (CPU, CUDA, OptiX), EEVEE and Workbench (OpenGL, optionally Vulkan), the GPU
  compositor and the GPU sequencer. About 2,200 cases for Cycles CUDA/OptiX, EEVEE and Workbench on OpenGL, plus about 400 GPU compositor and sequencer cases, over about 350 MB of data.
- Two families cannot run against a release binary: the **GPU-module C++ gtests** (`source/blender/gpu/tests`, about
  190 `GPU_TEST` cases) and the draw-manager gtests. They need a source build with `WITH_GTESTS` +
  `WITH_GPU_BACKEND_TESTS`. The release-runnable stand-in for the GPU module is `bl_pyapi_gpu.py`: **2** cases.
- Upstream keeps **skip lists and per-vendor thresholds**, not recorded expected failures. The EEVEE references are
  stated to be generated on OpenGL + NVIDIA, and Cycles GPU renders are compared with the *CPU* references under looser
  thresholds. So "no more failures than native" needs a native (or VFIO) baseline run of the same commands, and a pass
  proves less than it sounds (section 8).
- What the guest needs: the matching release zip, a 3.1.x `oiiotool.exe` (8 MB, from the PyPI wheel, no Python
  needed), the `tests/python` + `tests/files/render` tree for the tag (about 255 MB), and the Python inside the Blender
  zip. No git, svn or compiler.

## 1. Hypotheses and outcomes

| id | hypothesis | falsifier | result |
|---|---|---|---|
| H1 | the render-test runners run against a release binary, no build tree | runner imports something that only exists in a build tree | **holds (measured).** The runners import only `modules/render_report.py` (standard library). `cycles_render_tests.py` ran on the official `blender-5.2.0-linux-x64` with `--blender <exe> --oiiotool <exe> --testdir --outdir` and no build tree (section 9). Note the flags are `--blender/--oiiotool` (double dash) and `--oiiotool` is mandatory. |
| H2 | the GPU-relevant data is under 2 GB | size listing | **holds (measured).** All of `tests/files` is 797 MB (6,885 files); the render directories the GPU families use are 253 MB (3,423 files). |
| H3 | the suite already carries per-device failure lists, so a native baseline is needed | no such lists | **partly holds.** There are per-device/per-vendor *skip* lists and thresholds (section 6) but no list of expected failures with results, and the lists are written for upstream's build workers (NVIDIA/AMD/Intel, Linux/Windows/macOS). Unlisted flakiness is not recorded, so the baseline is still needed, and it should be run twice to measure its own spread. |

## 2. Version pairing

- Tags (measured, `git ls-remote`): `v5.2.0` `fbe6228777e7`, `v5.2.1` `9e2066aef7ef`, `v5.2.2` `d13f752e3b9c`.
  Branch `blender-v5.2-release` is `32ede9dba158` (it moves). The official 5.2.0 Linux build prints `Blender 5.2.0 LTS`,
  `build hash: fbe6228777e7` (measured), i.e. the hash is the tag commit.
- **The guest's exact patch level is not known to me** (the task says "Blender 5.2"). First action in the guest:
  `blender.exe --version`, take `build hash`, pick the tag whose commit starts with it. `fetch_data.py --tag` takes any tag.
- Tests change between patch releases (measured, `git diff --stat v5.2.0 v5.2.2 -- tests`: 43 files, including
  `eevee_render_tests.py` (+22 lines, new blocklist/threshold entries), `tests/python/CMakeLists.txt`, 16 bpy-API
  tests and reference images). A 5.2.0 binary with 5.2.2 tests would show failures that are version skew, not GPU
  faults. Pair binary and tests by build hash.
- Never use `main` or `blender-v5.3-release` tests with this binary: they exercise features and references that the
  5.2 binary does not have (inferred; the failure mode is mass reference mismatches and "No engine" style errors).
- Test data is pinned by the same tag (it lives in the same repository, see section 5), so there is no separate
  data/binary skew to manage.

## 3. Test families

Counts are **measured** by running the upstream runner's own `blend_list()` with the runner's blocklists over the
`v5.2.0` tree (`permutations.txt` files expanded), for NVIDIA + OpenGL. They assume the release build has Freestyle,
fluid, OpenVDB, OpenSubdiv and OIDN (inferred from the official build; CMake adds those families only when the
feature is on, `tests/python/CMakeLists.txt` L887-960). Times are **inferred** except the three CPU rows.
"Release?" = can run against the unmodified release zip.

| family | script (under `tests/python`) | device / API | cases | release? | data (MB, tests/files/...) | est. time on the guest |
|---|---|---|---|---|---|---|
| Cycles CPU | `cycles_render_tests.py --device CPU` | none (control) | 662 | yes | render 253 (shared) | measured on a 24-core host: 81 cases in 14 s; guest unknown, likely 5-10 min |
| Cycles CUDA | same, `--device CUDA` | CUDA | 647 | yes | shared | 10-40 min |
| Cycles OptiX | same, `--device OPTIX` | OptiX (RT cores, driver `nvoptix`) | 645 | yes | shared | 15-60 min; first OptiX use compiles kernels |
| EEVEE OpenGL | `eevee_render_tests.py --gpu-backend opengl` | OpenGL 4.6 | 567 | yes | shared | 20-60 min (shader compile bound) |
| EEVEE Vulkan | same, `--gpu-backend vulkan` | Vulkan | 564 | yes (CTest gates it behind `WITH_GPU_RENDER_TESTS_VULKAN`; the runner just takes the flag) | shared | 20-60 min |
| Workbench OpenGL / Vulkan | `workbench_render_tests.py --gpu-backend ...` | OpenGL / Vulkan | 321 / 320 | yes | shared | 5-15 min each |
| Compositor GPU (+CPU control) | `compositor_render_tests.py --gpu-backend opengl`; `compositor_file_output_tests.py --device GPU` | OpenGL compute | about 268 (+ file_output) | yes | 67 | 5-20 min |
| Sequencer GPU | `sequencer_render_tests.py --gpu-backend opengl` | OpenGL | about 137 | yes | 29 | 3-10 min |
| Overlay | `overlay_render_tests.py` | OpenGL, needs a real window (headed) | 1 | yes, but upstream gates it behind `WITH_GPU_RENDER_TESTS_HEADED` | in render | minutes |
| Sculpt/paint brush render | `sculpt_brush_render_tests.py` | GPU mesh paint | 5 `.blend` | yes | 13 | minutes |
| GPU module, Python | `bl_pyapi_gpu.py --gpu-backend opengl` | OpenGL | **2** | yes | none | seconds |
| UI event-simulation tests | `ui_simulate/run_blender_setup.py` via `tests/utils/blender_headless.py` | real window, OpenGL (Vulkan optional) | 43 | yes (`PASS_THROUGH=1`) | none | 15-40 min |
| GPU module gtests | C++ `source/blender/gpu/tests/*.cc` | OpenGL/Vulkan | about 190 `GPU_TEST` (14 files, grep count) | **no, build tree** | none | n/a |
| Draw-manager gtests | `source/blender/draw/tests/*.cc` | GPU | about 25 (grep count) | **no, build tree** | none | n/a |
| Hydra/Storm | `storm_render_tests.py` | GL/Vulkan via Hydra | n/a | only if the release ships Hydra (not checked) | in render | not planned |
| Other bpy tests (non-GPU) | the remaining entries of `tests/python/CMakeLists.txt` | none | n/a | mostly yes; some read source-tree files (`bl_rst_completeness`, `bl_load_addons`; inferred) | small | not part of this goal |

Sources: render families `tests/python/CMakeLists.txt` L884-1091 (family lists, `RUN_SERIAL` for non-CPU Cycles,
`gpu_render_tests` filter `light_group|shadow_catcher|denoise|guiding|reports`); compositor L1103-1223; sequencer
L1481-1575; sculpt L1664-1677; GPU module L270-294; UI tests L1684-1743; options in root `CMakeLists.txt` L778-862
(`WITH_GTESTS`, `WITH_GPU_RENDER_TESTS`, `WITH_GPU_BACKEND_TESTS`, `WITH_GPU_DRAW_TESTS`, `WITH_UI_TESTS`,
`CYCLES_TEST_DEVICES`, all `OFF` by default; `WITH_TESTS_BATCHED` is `ON`).

**What the CUDA/OptiX count includes (measured from the runner).** Cycles GPU runs the same families as CPU, minus
`BLOCKLIST_GPU` and, for OptiX, `BLOCKLIST_OPTIX`. Cycles OSL tests are CPU-only unless `WITH_CYCLES_TEST_OSL`.
Cycles GPU cases are compared with `cycles_renders` (the **CPU** references); the HTML also builds a "compare" page.

## 4. Command lines

The CMake `add_render_test()` (L178-186) calls `python <script> <args> --blender <exe> --oiiotool <oiiotool> --batch`
(`--batch` because `WITH_TESTS_BATCHED` defaults ON). Blender-script tests use `add_blender_test()`
(`tests/CMakeLists.txt`, `TEST_BLENDER_EXE_PARAMS`). For the guest, with `$B` = `blender.exe`, `$O` = `oiiotool.exe`,
`$T` = the tests root, `$R` = output root, `$PY` = `<blender>\5.2\python\bin\python.exe`:

```
# Cycles (CMake passes --osl limited for CPU when OSL is built, otherwise none; GPU devices: none)
$PY $T\python\cycles_render_tests.py --testdir $T\files\render\<fam> --outdir $R\cycles --device CUDA --osl none --blender $B --oiiotool $O --batch
# EEVEE / Workbench (the GPU backend selects the output subdirectory)
$PY $T\python\eevee_render_tests.py     --testdir $T\files\render\<fam> --outdir $R\eevee_opengl     --gpu-backend opengl --blender $B --oiiotool $O --batch
$PY $T\python\workbench_render_tests.py --testdir $T\files\render\<fam> --outdir $R\workbench_opengl --gpu-backend opengl --blender $B --oiiotool $O --batch
# GPU compositor / sequencer
$PY $T\python\compositor_render_tests.py --testdir $T\files\compositor\<fam> --outdir $R\compositor_opengl --gpu-backend opengl ...
$PY $T\python\sequencer_render_tests.py  --testdir $T\files\sequence_editing\<fam> --outdir $R\sequencer_opengl --gpu-backend opengl ...
# GPU module (Python)
$B --background --console-crash-handler --factory-startup --debug-memory --debug-exit-on-error --python-exit-code 1 --python-expr "import bpy;bpy.context.preferences.filepaths.file_preview_type='NONE'" --python $T\python\bl_pyapi_gpu.py --gpu-backend opengl
# UI tests (env BLENDER_BIN=$B, PASS_THROUGH=1)
$PY $T\utils\blender_headless.py --factory-startup --no-native-pixels --no-window-frame -p 0 0 800 600 --gpu-backend opengl --log operator --log-level debug --console-crash-handler --debug-memory --debug-exit-on-error --python-exit-code 1 --enable-event-simulate --python $T\python\ui_simulate\run_blender_setup.py -- --tests test_render.interactive_rendering_cycles
```

The family lists to loop over are the lists in section 3's source lines. Each runner exits non-zero if any case
failed (`sys.exit(not ok)`). Per-case Blender arguments the runner builds (measured in `cycles_render_tests.py`
`get_arguments`): `--background --factory-startup --enable-autoexec --debug-memory --debug-exit-on-error
--console-crash-handler <file.blend> -E CYCLES -o <out> -F PNG ... -f 1`, with `--cycles-device <DEV>` appended last.

Useful environment variables read by `modules/render_report.py`: `BLENDER_TEST_FILTER` (glob list, `a:b-c` form, to
bisect), `BLENDER_TEST_IGNORE_BLOCKLIST`, `BLENDER_TEST_IGNORE_VENDOR_BLOCKLIST`, `BLENDER_VERBOSE`,
`CYCLESTEST_SPP_MULTIPLIER`, and `BLENDER_TEST_UPDATE` (rewrites references: never set it).

Windows detail (measured in `render_report._run_tests`): the runner splits a batch at 32,766 command-line
characters on `win32`, so long family lists run as several Blender processes automatically.

## 5. Test data

- **Where (measured).** At `v5.2.0` the data is **in the main repository**, `tests/files/**`, stored as git-LFS
  (`.gitattributes` lists `*.blend`, `*.png`, `*.exr`... as `filter=lfs`; `.gitmodules` only lists the `lib/*` library
  submodules). There is no `lib/tests` SVN and no separate `blender-test-data` repository at this tag. A `git archive`
  or the web zip of the tag therefore contains only LFS pointer stubs (130-byte files).
- **Size (measured from the pointer `size` fields).** `tests/files` 797 MB / 6,885 files. By top directory:
  render 269 MB, modeling 212, compositor 67, ffmpeg 46, io_tests 38, sequence_editing 29, imbuf_io 19, tracking 14,
  sculpting 13, rest below 12 each. GPU render subset (all render families in section 3 plus `util`, `overlay`,
  `grease_pencil`, `workbench`) **253 MB / 3,423 files**; with the GPU compositor and sequencer sets about **350 MB**.
  The Cycles/EEVEE/Workbench families share one directory per family, so fetching a family costs one copy of the data.
- **Licence.** The repository `COPYING` states Blender is available under the GPL only (`doc/license/GPL-license.txt`);
  there is no separate licence file under `tests/files` (measured by listing). Runner scripts carry SPDX
  `Apache-2.0` (`cycles_render_tests.py`, `render_report.py`) or `GPL-2.0-or-later` (`bl_pyapi_gpu.py`). Inferred: the
  data is under the repository's GPL unless a file says otherwise; I did not audit individual third-party assets.
  We fetch from upstream at run time and commit none of it.
- **How to fetch without installing much.** Three routes, in order of preference:
  1. **Stage on the host and ship it as a disk or zip** (like the app disk in `V3_WINDOWS_APP_MATRIX.md`): no network
     in the guest, the same bytes for the VFIO baseline and the kayfabe run. Recommended.
  2. `scripts/bench/blender_suite/fetch_data.py` (added with this document, standard library only; runs with the
     Python inside the Blender zip): lists the tag's tree through the Gitea API and downloads each file through the
     API media endpoint, which resolves LFS pointers. **Measured:** 340 files / 4.5 MB in 84 s at 3 parallel
     requests; the 253 MB render set is about 3,400 files, so about 15 minutes.
  3. Git for Windows (bundles Git LFS): `git clone --depth 1 --filter=blob:none --sparse --branch <tag> <url>` then
     `git sparse-checkout set tests/python tests/utils tests/files/render ...`; LFS content comes with the checkout.
     This is the supported path, but it installs Git (about 60 MB) in the guest.
- **Trap (measured).** The web `/media/tag/...` route of projects.blender.org sits behind a Cloudflare bot challenge.
  A burst of 340 requests at 8 parallel connections tripped it (HTTP 403 with `cf-mitigated: challenge`), while git
  smart-HTTP, `/api/v1/...` and the LFS batch endpoint kept working. The script therefore uses the API route, 3
  workers and retries with back-off, and must not be turned up. Do not try to get around a challenge; stage on the host.

## 6. `oiiotool` and the upstream skip lists

- The runners require `oiiotool` (`tests/python/CMakeLists.txt` L17-29: render tests are disabled without it). The Blender release does not ship it
  (**measured** on the extracted Linux 5.2.0 tarball: `find -iname '*oiio*'` finds only `lib/usd/hioOiio`; the Windows zip was not opened).
  **Measured:** the PyPI wheel `OpenImageIO 3.1.18.1` (`cp314-win_amd64`, 8.0 MB) contains a self-contained
  `OpenImageIO/bin/oiiotool.exe` with its DLLs (47 entries); a wheel is a zip, so `python -m zipfile -e` or 7-Zip
  extracts it with no pip. The Linux wheel's `oiiotool` ran (version `3.1.18.1`) and produced the proof run's verdicts.
  The exact tool upstream uses is in `lib-windows_x64` (`OpenImageIO/bin/oiiotool.exe`, LFS, 0.95 MB plus DLLs in
  several library directories); it is the more faithful choice but needs about ten library directories (inferred),
  so use the wheel first. **Risk:** an OIIO version different from Blender's bundled one could shift `--diff` results
  slightly; run the CPU control family on both and compare before trusting the GPU numbers.
- **Skip lists (measured, `tests/python/*.py`, v5.2.0).** For this goal the relevant ones are:
  - `cycles_render_tests.py`: `BLOCKLIST_ALL` (14 files), `BLOCKLIST_GPU` (16 patterns: hair families, `glass_mix_40964`,
    `filter_glossy_refraction_45609`, `bevel_mblur`, `instance_types`, `sobol_uniform_41143`, `visibility_particles`,
    `guiding*`), `BLOCKLIST_OPTIX` (`big_triangles_50164`, `big_plane_43865`: ray-intersection precision). No
    CUDA-only list. OSL lists only apply with `WITH_CYCLES_TEST_OSL`.
  - `eevee_render_tests.py`: common `BLOCKLIST` (21 patterns), `BLOCKLIST_NVIDIA_GL` (`shadow_min_pool_size`:
    "non-deterministic"), `BLOCKLIST_VULKAN` (`lightprobe_planar`, `principled_blackbody`, `sss_concave_clamp`,
    `image`), no NVIDIA-Windows-specific list. Tighter thresholds (2/255, 0.049 %) are applied for NVIDIA + OpenGL
    "because references are generated there", with loosened ones for `image_mapping`, `displacement`, `texture`.
  - `workbench_render_tests.py`: four `raycast_*` files always, `image_log` on Vulkan.
  - `storm_render_tests.py`: `BLOCKLIST_VULKAN_NVIDIA` (sporadic black images, non-deterministic cases).
  - `ui_simulate/run_blender_setup.py`: Windows blocklists only for Intel/OpenGL and AMD; none for NVIDIA.
  - Upstream's text records Windows-specific differences only in comments (for example `cycles_render_tests.py`
    "underwater_caustics gives different results on Windows" next to the 0.032 threshold).
- Inferred: there are no recorded expected failures, so any NVIDIA-Windows flake that upstream's workers did not hit
  is invisible in the tree. That is the reason for the baseline.

## 7. Cases that need care (inferred unless stated)

- Upstream's own text marks as "alternating", "non-deterministic" or "sporadic" a number of EEVEE/Storm cases
  (section 6). Those can fail on the baseline too; count a case as a kayfabe regression only if it passed in every
  baseline run.
- The `get_gpu_device_vendor()` helper (`modules/render_report.py`) launches one extra Blender per EEVEE/Workbench run
  and picks the vendor-specific lists from `gpu.platform.device_type_get()`. In the guest this is the real NVIDIA
  driver, so the NVIDIA lists apply; confirm by printing `GPU_DEVICE_TYPE` once (runs `modules/gpu_info.py`).
- A crash is handled by the runner: it marks the case `CRASH` and re-runs the rest. After a dead channel or a TDR
  every later case fails with `CRASH` or `NO OUTPUT`, which would read as hundreds of failures. The wrapper
  (section 10) must stop a family after N consecutive crashes and check `nvidia-smi` before the next family.
- Known guest TDR: OptiX denoise at 5760x3240 with 8192 tiles. The suite's Cycles scenes are small, but I did not
  measure their resolutions (binary `.blend` files). The `denoise` and `denoise_animation` families use the
  denoiser, so they run last among Cycles families.
- First OptiX use compiles/loads kernels through the driver and can take minutes; give the first OptiX family a long
  timeout and run it alone once before the full pass.

## 8. What a pass would mean

- It would show: the guest NVIDIA driver on kayfabe runs CUDA and OptiX path tracing, the OpenGL 4.6 renderer of
  EEVEE and Workbench (shaders, compute, SSBO, image load/store, many texture formats, framebuffers), the GPU
  compositor and, if included, Vulkan, with image output inside upstream's tolerances. It is an independent,
  externally maintained correctness oracle, unlike the proprietary apps.
- It would not show: bit-exactness (thresholds are loose by design; Cycles GPU is compared with CPU images);
  presentation and display paths (all tests are background renders except the UI set); long-run stability, preemption
  or multi-process fairness; performance; TDR recovery; video encode. Test scenes are small, so memory pressure and
  large-allocation paths are lightly exercised.
- **A stronger check that costs little (inferred to work):** both runs write the PNGs. On a fixed GPU and driver
  the render should be deterministic, so also compare the output PNG digests between the VFIO run and the kayfabe
  run (`diff -r`, or hash). The earlier app matrix saw Cycles means equal to the host (`V3_APP_MATRIX.md`). Whether
  every case is bit-stable run to run on one machine is not measured; the baseline-twice step measures it, and the
  digest comparison is then reported only for cases that are stable on the baseline.
- Target (owner's): kayfabe failures subset of the union of baseline failures. Report also the cases that fail on
  only one baseline run (flaky) and cases that pass on kayfabe but fail on the baseline.

## 9. Measured proof run (host, CPU only, no GPU)

Host `root@172.22.1.20`, work under `/data/claude-blender-suite/` (2.3 GB used in all: Blender tarball 384 MB plus
its extraction, 16 MB of wheels, about 25 MB of data, the 28 MB sparse checkout). Sparse blobless fetch of tag `v5.2.0`: `.git` 28 MB. Official
`blender-5.2.0-linux-x64.tar.xz` (sha256 `96f6c181...c48` matched upstream's `.sha256`). The host has an RTX 4070, so
`CUDA_VISIBLE_DEVICES=""` and `--device CPU` were used, EEVEE/OpenGL was not run, and nothing touched a GPU.

| family | command shape | result | time |
|---|---|---|---|
| attributes | `cycles_render_tests.py --device CPU --osl limited --batch` | 5/5 passed | 2.0 s |
| shadow | same | 1/1 passed | 0.7 s |
| bsdf | same | 31/31 passed | 5.1 s |
| light | same | 44/44 passed | 6.4 s |

Negative control (measured): in a scratch copy, the reference `attribute_uniform.png` was replaced by another case's
image; the same command reported `4 passed, 1 FAILED attribute_uniform` and the line `Render result is different
from reference image`. So the verdict path (render, `oiiotool --diff`, threshold) is live, not trivially green.
Output is a gtest-style log plus `report.html`/`compare.html` under `<outdir>/<variation>/` (for example
`cycles/cpu/`). There is no JUnit file; the log lines are the machine-readable result:
`[       OK ] name`, `[ FAILED   ] name`, `[ PASSED   ] N tests.`, `[ FAILED   ] M tests, listed below:`.

## 10. Run plan for the guest

Nothing here is to be run until the guest is stable and the owner agrees. Strictly one family at a time, serial
(upstream marks GPU Cycles `RUN_SERIAL`). Cheapest and safest first, the heavy and risky ones last.

Install in the guest (no compiler, no git required): the matching `blender-<ver>-windows-x64.zip` (405 MB for 5.2.0;
upstream publishes `.sha256`); the OpenImageIO wheel extracted to a folder (8 MB); the test tree from section 5 route 1
or 2 (`tests/python`, `tests/utils`, `tests/files/render`, `tests/files/compositor`, `tests/files/sequence_editing`,
about 350 MB); `vc_redist` (already in the app disk); about 3 GB free for outputs. Python comes from the Blender zip (inferred path, see section 11).

| step | what | command shape | est. | stop rule |
|---|---|---|---|---|
| 0 | identify | `blender.exe --version`; `oiiotool.exe --version`; print `gpu_info.py` output | 1 min | if hash matches no tag, stop |
| 1 | tooling control, no GPU | Cycles CPU `attributes`, `shadow`, then all CPU families | 5-15 min | any failure here is tooling/OIIO, not GPU: fix first |
| 2 | GPU module smoke | `bl_pyapi_gpu.py --gpu-backend opengl` | 1 min | fail = stop |
| 3 | Workbench OpenGL | 16 families, 321 cases | 5-15 min | 5 consecutive CRASH = stop |
| 4 | EEVEE OpenGL | 30 families, 567 cases; small families first | 20-60 min | same |
| 5 | GPU compositor + sequencer OpenGL | compositor `input,color,filter,...`, sequencer `filter,transform,blend_modes_*,colorspace,effects` | 10-30 min | same |
| 6 | Cycles CUDA, all but denoise | 38 families (two have zero cases), about 625 cases | 10-40 min | same, check `nvidia-smi` between families |
| 7 | Cycles OptiX, all but denoise | first run `attributes` alone (kernel compile) | 15-60 min | same |
| 8 | Vulkan variants (if the guest Vulkan lane is healthy) | EEVEE, Workbench, compositor, sequencer with `--gpu-backend vulkan` | 30-90 min | same |
| 9 | Denoise families | Cycles CUDA and OptiX `denoise`, `denoise_animation` (22 cases each) | 5-20 min | the known TDR class; one family per boot window |
| 10 | UI event-simulation tests | 43 tests, one Blender window each, OpenGL | 15-40 min | needs the guest display up; run last |
| 11 | optional | GPU-module gtests need a Windows source build; not planned | n/a | decide only if 1-10 are clean |

Wrapper (to be written, not part of this document): a small Python driver that loops the table, runs each command
with a timeout, tees stdout to `<run>/<family>.log`, records start marker and exit code (the repository rule that a
killed job and a running job look alike), parses the log lines of section 9 into one JSON per family
(`{case: ok|failed|crash|no-output}`), and writes a summary plus the `report.html` trees. JUnit is optional and can be
generated from the JSON.

Pass/fail counts: count `[ OK ]` and `[ FAILED ]` per family from the logs; the totals must equal the case counts
in section 3 within the blocklist differences for the actual box (the log's `Running N tests` line is the truth).

Baseline: same wrapper, same tree, same zip, same oiiotool, same guest image and driver, on a VFIO-passthrough
Windows VM on the host's RTX 4070 (the same AD104 die kayfabe emulates), twice. A native Windows rental with the
same GPU is the alternative but changes the OS image. Compare: `failed(kayfabe) - union(failed(baseline))` must be
empty; list `flaky(baseline)` separately; compare PNG digests per section 8.

## 11. Open points and risks

- Guest patch level unknown (section 2). Guest OIIO version differs from Blender's (section 6). EEVEE Vulkan and UI
  tests depend on guest Vulkan and display state that I could not inspect.
- Step durations are estimates, not measurements; only CPU timings exist. The kayfabe slowdown factor is unknown.
- Whether `tests/files/render` scene resolutions are all small is not measured.
- Cloudflare challenge on the upstream web routes (section 5): stage data on the host rather than fetching from the
  guest if the challenge appears.
- Not checked: the Windows zip's file list (including the path of its bundled Python, assumed `<blender>\5.2\python\bin\python.exe` as in the Linux layout), Hydra/Storm
  availability in the release, resolution of each test scene, and how the run behaves on the guest at all.

## 12. Sources and reproduction

- Upstream at `v5.2.0` (`fbe6228777e7`): `tests/CMakeLists.txt`, `tests/python/CMakeLists.txt`, `CMakeLists.txt`
  (L628-640, L778-862), `.gitattributes`, `.gitmodules`, `tests/python/{cycles,eevee,workbench,compositor,sequencer,overlay,storm,sculpt_brush}_render_tests.py`,
  `tests/python/modules/{render_report,gpu_info}.py`, `tests/python/bl_pyapi_gpu.py`, `tests/python/ui_simulate/run_blender_setup.py`,
  `tests/utils/blender_headless.py`, `source/blender/gpu/tests/`, `source/blender/draw/tests/`,
  `source/blender/gpu/intern/gpu_context.cc` (default backend `GPU_BACKEND_OPENGL`).
- Host work directory: `/data/claude-blender-suite/` (`src/` sparse tag checkout, `proof/`, `dl/`).
- Fetch tool `scripts/bench/blender_suite/fetch_data.py` and case counter `scripts/bench/blender_suite/count_cases.py`
  (section 3 counts; `python3 -B count_cases.py <tests dir>` run from `<tests dir>/python`), both on this branch.
