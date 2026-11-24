# Fixtures of the open lanes tests (scripts/ci/test_open_lanes.py)

STATUS: LIVE, 2026-11-23.

- `llama_vulkan_nvidia_windows_real.log`: the output of llama-bench on the Vulkan backend of a native Windows RTX 4060 Ti, copied from
  `traces/windows_baseline_20261010/apps/llama_vulkan_bench.log` (harness header and end lines dropped, the UTF-8 plus-minus sign written as `+-`). It is the only
  committed real Vulkan output of this application; the Linux log of the native box run is in `traces/open_lanes_20261123/`.
- `llama_vulkan_nvidia_no_table.log`: the real log without its table (the device lines only), the case of a benchmark that died after loading.
- `llama_vulkan_llvmpipe.log`, `llama_vulkan_no_device_cpu.log`, `llama_vulkan_nvidia_ngl0.log`: the three ways the row can fall back, written in the same format
  (a CPU Vulkan driver listed first, no Vulkan device and the CPU backend, `-ngl 0` on the real device). Constructed, not captured: the format is the real one above, the
  numbers are made up.
- `nvbandwidth_real_*.json`: the output of nvbandwidth v0.9 (commit 4a49bdae82d69e69330a88746cb12b449796ee8b) captured on a native RTX 4090 Vast KVM box, driver 575.51.03, 2026-11-23, run at
  revision ddbf01939 of the branch (`traces/open_lanes_20261123/`): a host-to-device copy-engine case, the latency case, and the same case with `CUDA_VISIBLE_DEVICES` empty (the `error` document).
  Note `"GPU Device list" : null` in the first two: the JSON of v0.9 does not list devices.
- other nvbandwidth outputs are built by the tests from the JSON writer of the pinned source (`json_output.cpp`).
