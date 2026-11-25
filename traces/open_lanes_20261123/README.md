# Open lanes: nvbandwidth and llama.cpp on Vulkan, native run on one RTX 4090 Vast box, 2026-11-23

STATUS: LIVE, 2026-11-23 (evidence of a native run, no kayfabe guest, no Windows). Branch `claude/open-lanes-20261123` (PR against `next`); the design is
`docs/design/V3_OPEN_LANES_NVBW_LLAMAVK_LHM.md`, section 8 reads these files. Revisions under test: `ddbf01939` for `first_run_ddbf01939/` (two defects found, fixed since) and
`00ffd4635` for `final_run_00ffd4635/`; the only change of `scripts/apps` after `00ffd4635` is that `nvbandwidth_row.py` scrubs the PCI address and the host name from the logs it keeps
(the files here were scrubbed with the same two rules when they were copied, plus `census.scrub`). The LibreHardwareMonitor row did not run: no Windows machine was used.

Machine: Vast KVM VM (image `vastai/kvm:ubuntu_cli_22.04-2025-05-16`), one RTX 4090 passed through (`environment.txt`: Ubuntu 22.04.5, kernel 6.8.0-59, 15 vCPUs, 48 GB RAM,
driver 575.51.03, 24564 MiB). Instance rented, used and destroyed by its own id (the first offer returned a stopped instance and was destroyed at once); about 0.7 box-hours, about 0.35 USD
(`python3 /root/night/vast_guard.py` before and after). The box was untrusted; nothing but text came back. No GPU UUID, serial, board id, PCI address or IP is in these files
(`python3 scripts/ci/no_gpu_uuids.py` is clean).

| directory | what ran |
|---|---|
| `first_run_ddbf01939/` | `provision_lanes.sh` (nvbandwidth built, the Vulkan step failed: `E: Unable to locate package glslc`), the nvbandwidth row with the GPU visible and with `CUDA_VISIBLE_DEVICES` empty |
| `final_run_00ffd4635/` | `provision_lanes.sh llama_vulkan` with the LunarG SDK step (193 s including the SDK download and the build), then both rows directly, both negative controls, `-ngl 0` alone, and both rows again through the real `run_apps.sh` with and without `KF_NEGCTL=1`, judged by `gpu_use_rows.py annotate` |

## What the files say

- `first_run_ddbf01939/nvbandwidth/summary.txt` and `.../logs/host_to_device_memcpy_ce.log`: **defect 1 (the row's, now fixed).** The JSON of nvbandwidth v0.9 prints `"GPU Device list" : null`
  (the tool records the list while its device count is still 0), so the row saw 0 GPUs and booked every passing case `MISMATCH passed_without_1_gpus` (`mismatch=18`). The row now reads the device
  names from one text-mode case (`final_run_00ffd4635/nvbandwidth/logs/devices.log`).
- `first_run_ddbf01939/provision_lanes.txt`: **defect 2 (the provisioning's, now fixed).** Ubuntu 22.04 has no `glslc` package; `OPENROWS_llama_vulkan=FAIL reason=E:_Unable_to_locate_package_glslc`.
  The step now uses the LunarG Vulkan SDK 1.4.341.1 checked against LunarG's own sha256 when `apt-cache show glslc` finds nothing (`final_run_00ffd4635/llama_vulkan_cmake_coopmat_lines.txt`:
  `GL_KHR_cooperative_matrix supported by glslc`, `GL_NV_cooperative_matrix2 supported by glslc`).
- `final_run_00ffd4635/nvbandwidth/results.tsv`, `summary.txt` (run 2026-11-23, revision `00ffd4635`): `status=PASS pass=18 waived=17 fail=0 mismatch=0 cases=35 gpus=1 gpu_use=PROVEN`; host-to-device 26.4 to 26.9 GB/s by the copy engine and by the SM copy kernel,
  both directions at once 22.6 to 23.0 GB/s, device-local copy 459.7 GB/s, host-to-device latency 568.1 ns; `gpu_use_why=memory.used rose 1975 MiB`; the 45 s of the whole row is in `summary.txt`.
- `final_run_00ffd4635/nvbandwidth_negctl/`: with `CUDA_VISIBLE_DEVICES` empty every case is `FAIL binary_error` (`logs/host_to_device_memcpy_ce.log`: `CUDA_ERROR_NO_DEVICE ... cuInit(0)`), `status=FAIL`, exit 1.
- `final_run_00ffd4635/llama_vulkan/`: `status=PASS device=NVIDIA_GeForce_RTX_4090 coopmat=NV_coopmat2 pp512_tps=6091.44 tg64_tps=533.01 secs=11 gpu_use=PROVEN` (`memory.used rose 1337 MiB`); the same row run again
  through `run_apps.sh` (`run_apps_llama_vulkan_negctl0.log`) took 1 s and printed pp512 28191.95 and tg64 546.32: the first run on the machine creates the Vulkan pipelines, the second finds them cached
  (the cause is [inferred], not tested with the driver's cache disabled).
- `final_run_00ffd4635/llama_vulkan_negctl/` and `run_apps_llama_vulkan_negctl1.log`: with no Vulkan driver for the loader and `-ngl 0`, ggml finds no device, llama-bench falls back to its CPU backend
  (`CPU | 15 threads | pp512 173.23 t/s`), `status=FAIL reason=no_vulkan_device`. `llama_vulkan_ngl0/` (`-ngl 0` alone, GPU visible): `reason=ngl_0`.
- `final_run_00ffd4635/run_apps_negctl0.res`, `gpu_use_default.txt`: through `run_apps.sh` both rows PASS and the judge annotates `gpu_use=PASS`; `run_apps_negctl1.res`, `gpu_use_negctl.txt`: both rows FAIL and the judge
  prints `NEGCTL ... control=CONTROL_OK` for both.

## Findings (none is a kayfabe bug: nothing here ran in a kayfabe guest)

1. nvbandwidth v0.9 `-j` cannot list devices (upstream behaviour; the row works around it).
2. Ubuntu 22.04, the Vast KVM image, has no `glslc`: the Vulkan lane needs the LunarG SDK there, and the bundle host of a gate box is such an image.
3. Under the Vast payload the apt shim holds neither `libboost-program-options-dev` nor `glslc`; the two steps would end `FAIL` there (design document, section 4). Not run under the payload.

Not run: the LibreHardwareMonitor row, any kayfabe guest, the host census (kf3 submissions) of these rows, a second die.
