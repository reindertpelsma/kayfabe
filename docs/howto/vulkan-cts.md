# Run the Vulkan conformance subset (deqp-vk) on the host and in a guest

Status: supported. It has run natively once (RTX 3060, driver 580.159.03): the build takes about 10 minutes on 13 cores and the run about 90 seconds. It has run once in a guest (RTX 3090): the guest takes about 15 minutes for the list, and three cases pass on the host and fail in the guest, reproducibly. <!-- src: traces/vkcts_host_20261117/, traces/open_rows_guest_20261117/ -->

This open row runs a fixed list of 291 cases of the Khronos Vulkan conformance suite (ray tracing, acceleration structures, mesh and task shaders, sparse resources, video decode and encode, timeline semaphores, synchronization2, descriptor buffers, device-generated commands, cooperative matrices) with one `deqp-vk` binary, and compares the per-case results of the host with those of the guest on the same GPU. It needs no account, no licence and no private download: the sources come from the pinned upstream commit named in `scripts/apps/vkcts/pins.json`, and the binary is built on your machine.

## What you need

- Linux x86-64, an NVIDIA GPU with the driver installed, `git`, `cmake`, `python3`, `g++`, `make` (`ninja-build` makes the build faster), 25 GB free disk, and the Vulkan loader (`libvulkan1`) with the NVIDIA Vulkan ICD. In a plain CUDA container the ICD also needs `libxext6 libx11-6 libegl1`.
- Check it: `python3 scripts/apps/vkcts/vkcts.py check`.

## Build once, run on the host

```text
python3 scripts/apps/vkcts/vkcts.py build            # fetches the pinned sources, builds deqp-vk only (about 10 minutes on 13 cores)
python3 scripts/apps/vkcts/vkcts.py run --side host --out out/host
```

The build refuses a source that is not the pinned commit. The run prints a `VKCTS_SUMMARY` line (counts of pass, not supported, fail, crash, timeout), the device it ran on, the GPU-use verdict from the host's own `nvidia-smi` counters, and a per-case table between `VKCTS_RESULTS_BEGIN` and `VKCTS_RESULTS_END` (also written to `out/host/results.tsv`). A case whose extension or feature the GPU lacks is `NotSupported`; that is not a failure.

The run exits 3 if the Vulkan device is a software rasteriser, and 4 if the host counters do not show the GPU working.

## Run inside a guest and compare

On a kayfabe benchmark box the matrix scripts do all of it (the row is a discovery row: it reports and never gates):

```text
python3 scripts/apps/vkcts/vkcts.py build --root /workspace/apps/vkcts
bash scripts/apps/apps_matrix.sh host  RUN vkcts
bash scripts/apps/apps_matrix.sh guest RUN vkcts
python3 scripts/apps/vkcts/vkcts.py diff /workspace/apps/results/RUN
```

By hand, copy `scripts/apps/vkcts/` and the built directory (`<root>/current`) into the guest and run `vkcts.py run --side guest --artifact <dir> --out out/guest` there, redirect the printed output of each run to a file (`... > out/guest.log`) and compare with `vkcts.py diff --host out/host.log --guest out/guest.log`.

## Reading the diff

The comparison is the result, not "all pass". A case that passes on the host and fails or is missing in the guest (`GUEST_ONLY_FAIL`, `GUEST_LOSES_SUPPORT`) is a candidate bug of the guest stack; run the host a second time as a control. A case that fails on both sides is not the virtualisation's. `VKCTS_EXT_DIFF` lists the Vulkan extensions only one side exposes.

Use your own case list with `--caselist FILE` (lines `area<TAB>case name`) and your own build with `--artifact DIR`; both are trusted as you give them.
