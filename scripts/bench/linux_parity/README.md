# Linux parity probes

STATUS: RESEARCH, 2026-10-25/26, LX2. Surface attribution was explicitly resumed in [phase 3](../../../traces/linux_parity_20261025/PHASE3_REPORT.md); CUPTI remained disabled. Primary results and the native diagnostic safety stop are recorded in [REPORT.md](../../../traces/linux_parity_20261025/REPORT.md). The preregistration is [PREREG.md](../../../traces/linux_parity_20261025/PREREG.md).

`surface.c` is an offscreen NVIDIA EGL/OpenGL workload: RGBA8 + depth, 64 ordered layers, then a sampled render-target pass. It records CPU completion and per-pass GPU timestamps, checks final color/depth, and emits 1000 frame records after timing. Build once on the test host with `gcc -O2 surface.c -lEGL -lOpenGL -lm -o surface`. This probe does not claim to reproduce an application's full rendering workload or guarantee compression use.

`suite.sh` takes an executable directory and a serial-clock-handshake toggle. Its CUDA probe is the existing `scripts/bench/cup8bench.c`, compiled once on the host. The separate private launcher must own the shared GPU lifetime lease before any workload or sampler runs. Stop on every new Xid/OOM/instability and restore captured desktop state after owned QEMU exit. These standalone files do not acquire the lease or authorize stopping another session's processes.

`sampler.py` writes query brackets plus GPU, pressure and identified thread telemetry; actual internal NVML averaging is not changed by the polling interval. `clock_probe.c` emits realtime/monotonic pairs for a bounded serial handshake. `analyze.py` reads private `runs/*/{result.json,stdout.log}` files and computes local distributions and paired run statistics; n<5 has no parity comparison. It deliberately rejects invalid timing values, content failures and incomplete record counts. The analysis tests use synthetic inputs only:

```sh
cd scripts/bench/linux_parity
python3 -m unittest -v test_analyze
```

The missing analyzer import in the tests-only EXPECTED RED commit establishes the new harness contract; it is not a claim about an existing product defect. Production source and defaults are unchanged.

`power_button.c` is a diskless-guest shutdown helper: it checks for QEMU DMI, waits for the guest ACPI power-button event, and invokes guest poweroff. It is never run by the native benchmark suite. This supports QMP `system_powerdown`; failure retains the lease and is reported. Kernel and driver source are unchanged.

`cupti_trace.c` is optional CUDA activity collection for separate diagnostic passes. Build against matching CUPTI/runtime/compiler SDK headers and libcupti, then use the documented `CUDA_INJECTION64_PATH` mechanism with a new `LX2_CUPTI_OUT` file. It records driver and concurrent-kernel activity, callback drop counts and collector failures. The combined native RM/strace/CUPTI run ended with SIGSEGV and no kernel records; the failing component is unisolated and this collector is not hardware-qualified. CUPTI tracing can perturb short kernels and does not belong in primary parity data; see [NVIDIA's CUPTI usage documentation](https://docs.nvidia.com/cupti/main/main.html).

`rm_trace.c` observes documented 595.91.07 RM ioctl fields using the matching open headers, then forwards the actual ioctl. `rm_fixture.c` is a CPU-only `/dev/null` fixture. One fixture preserved the underlying ENOTTY result; this is not complete transparency or GPU-tracer qualification. Serialized and unsupported parameter layouts remain undecoded. Phase-2 native traces were preserved after the combined tracing crash; the guest comparison had not run at that stop. The separate phase-3 surface pair is linked above; no CUPTI workaround was attempted.


`surface_variants.c` is a separate attachment/size probe; it does not replace the
original executable. It takes `colour-depth|colour|depth width height`, checks a
single-sample render FBO and verifies the applicable final attachments. Depth-only
omits the sampled pass. Build once on the host and run only under the GPU lease:

```sh
gcc -O2 -Wall -Wextra -Werror surface_variants.c -lEGL -lOpenGL -lm -o surface_variants
./surface_variants colour-depth 1920 1080
```

`rm_trace.c` also marks public GL storage entry/exit without inspecting driver code.
The phase-3 storage calls deferred their allocations, so these markers did not
establish an exact GL-object/leaf-VA join. The matched RM captures and separate
host-store capture are qualified for this surface experiment; general observer
transparency and the earlier CUDA crash remain unresolved. Explicit-size known
layouts are decoded; implicit-size/serialized payloads remain undecoded.

CODE READ, `target_preload.py:3`: environment assignments are placed at the target
after an affinity wrapper. MEASURED 2026-10-26, run `lx2-phase3-20261026`, CPU fixtures: Applying LD_PRELOAD to taskset itself consumes the observer's
exclusive output file before target exec. Use the helper or strace's `-E` target
assignment, and always use a new log path. This does not change product behavior.

GPU-free observer fixtures need matching open NVIDIA headers and host-built binaries:

```sh
gcc -shared -fPIC -O2 -I"$OPEN_NVIDIA_INC" rm_trace.c -ldl -o rm_trace.so
gcc -shared -fPIC -O2 mock_gl.c -o libmock_gl.so
gcc -O2 rm_gl_fixture.c -L. -lmock_gl '-Wl,-rpath,$ORIGIN' -o rm_gl_fixture
gcc -O2 -I"$OPEN_NVIDIA_INC" rm_fixture.c -o rm_fixture
LX2_FIXTURE_TMP="$PWD" LX2_GL_FIXTURE="$PWD/rm_gl_fixture" \
LX2_RM_FIXTURE="$PWD/rm_fixture" LX2_RM_OBSERVER="$PWD/rm_trace.so" \
python3 -m unittest test_rm_gl test_target_preload
```

MEASURED 2026-10-26, run `lx2-phase3-20261026`: all six public fixture checks pass; mutation
receipts and scope are in
[phase3-validation.json](../../../traces/linux_parity_20261025/phase3-validation.json).
These fixtures open `/dev/null` or fake GL entry points; they perform no GPU work.
