# Linux parity probes

STATUS: RESEARCH, 2026-10-25/26, LX2. Hardware results remain unmeasured until the private host campaign completes. The preregistration is [PREREG.md](../../../traces/linux_parity_20261025/PREREG.md).

`surface.c` is an offscreen NVIDIA EGL/OpenGL workload: RGBA8 + depth, 64 ordered layers, then a sampled render-target pass. It records CPU completion and per-pass GPU timestamps, checks final color/depth, and emits 1000 frame records after timing. Build once on the test host with `gcc -O2 surface.c -lEGL -lOpenGL -lm -o surface`. This probe does not claim to reproduce an application's full rendering workload or guarantee compression use.

`suite.sh` takes an executable directory and a serial-clock-handshake toggle. Its CUDA probe is the existing `scripts/bench/cup8bench.c`, compiled once on the host. The separate private launcher must own the shared GPU lifetime lease before any workload or sampler runs. Stop on every new Xid/OOM/instability and restore captured desktop state after owned QEMU exit. These standalone files do not acquire the lease or authorize stopping another session's processes.

`sampler.py` writes query brackets plus GPU, pressure and identified thread telemetry; actual internal NVML averaging is not changed by the polling interval. `clock_probe.c` emits realtime/monotonic pairs for a bounded serial handshake. `analyze.py` reads private `runs/*/{result.json,stdout.log}` files and computes local distributions and paired run statistics; n<5 has no parity comparison. It deliberately rejects invalid timing values, content failures and incomplete record counts. The analysis tests use synthetic inputs only:

```sh
cd scripts/bench/linux_parity
python3 -m unittest -v test_analyze
```

The missing analyzer import in the tests-only EXPECTED RED commit establishes the new harness contract; it is not a claim about an existing product defect. Production source and defaults are unchanged.

`power_button.c` is a diskless-guest shutdown helper: it checks for QEMU DMI, waits for the guest ACPI power-button event, and invokes guest poweroff. It is never run by the native benchmark suite. This supports QMP `system_powerdown`; failure retains the lease and is reported. Kernel and driver source are unchanged.

`cupti_trace.c` is optional CUDA activity collection for separate diagnostic passes. Build against matching CUPTI/runtime/compiler SDK headers and libcupti, then use the documented `CUDA_INJECTION64_PATH` mechanism with a new `LX2_CUPTI_OUT` file. It records driver and concurrent-kernel activity, callback drop counts and collector failures. This collector is compile-checked only: hardware operation remains unverified. CUPTI tracing can perturb short kernels and does not belong in primary parity data; see [NVIDIA's CUPTI usage documentation](https://docs.nvidia.com/cupti/main/main.html).
