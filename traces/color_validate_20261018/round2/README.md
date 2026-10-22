# CL1 round 2: corrected BGRA comparison, CUDA checks pass

MEASURED: on 2026-10-22, lease 57221842 at revision `e636236cf34621b0c9e53170638c54a55a22f979` (GTX 1660 SUPER, compute capability 7.5, driver 610.43.02) passed all four `color_mirror` cases. Actual logical RGB was `[0,160,64]`, `[64,160,64]`, `[192,0,0]`, `[192,64,64]`, matching every expectation; each readback contained eight identical opaque pixels. One run per case; no repeated-run spread is claimed. See [gpu-summary.log](gpu-summary.log) and [gpu-validation.log](gpu-validation.log).

MEASURED: on 2026-10-22, lease 57221842 at revision `e636236cf` passed all eight named `color_sdr` checks and all eight named `color_tmo` checks, with all three example exits zero. The exact checks that ran are listed below and in [gpu-summary.log](gpu-summary.log).

CODE READ: the provided display tests that require a GPU are the three examples `color_mirror`, `color_sdr` and `color_tmo`; the crate's unit/integration suite provides GPU-free ABI, packing, bounds and posture checks rather than additional hardware-gated display tests.

| Executed GPU example | Named checks |
|---|---|
| `color_mirror` | `mirror_negative_channel`, `unmirrored_negative_channel`, `mirror_digital_vibrance`, `unmirrored_digital_vibrance` |
| `color_sdr` | `input_fp16_nonidentity`, `direct8_input`, `direct8_output`, `output_unorm_nonidentity`, `immutable_snapshot`, `rearmed_snapshot`, `fractional_olut_interpolation`, `invalid_fp16` |
| `color_tmo` | `intensity_only`, `pq_pipeline_zero`, `immutable_snapshot`, `rearm_snapshot`, `variable_segment_header`, `compact_table`, `compact_bounds`, `invalid_table` |

MEASURED: on 2026-10-22, lease 57221842 at revision `e636236cf` completed `cargo test --locked --release -j2 -p kf-cuda`: 76 passed, zero failed, zero ignored (47 unit + 1 posture + 9 refusal + 1 origin + 7 ABI + 11 synchronization tests; zero doctests). The new `display::tests::the_output_kernel_packs_bgra_bytes` test passed; see [gpu-validation.log](gpu-validation.log). Before/after baseline and mutation runs were not performed by this evidence-only lane.

CODE READ: round 1's assertion failure was the example treating BGRA bytes as logical RGB; the author corrected that at this head. The kernel and committed PTX are byte-for-byte unchanged from revision `81626530e`. These synthetic CUDA tests establish the implementation's odd-extension result, not real display-engine MIRROR semantics or Windows console recovery.

MEASURED: on 2026-10-22, lease 57221842 was destroyed by its own numeric ID and absent from the authenticated inventory at 14:08:35 UTC, after 282.102166 seconds from instance start. The runner removed its target and a separate check found its process gone before teardown; timer and service were stopped after verified absence. Account credit after teardown was 37.70116863646115 USD; see [cleanup.txt](cleanup.txt) and [rental.txt](rental.txt).

MEASURED: on 2026-10-22, the CPU-only trusted-host build at revision `e636236cf34621b0c9e53170638c54a55a22f979` completed all 1682 QEMU build steps and installed `kf3-bins/e636236c/qemu-system-x86_64` in the owner's launcher root. Both build and installed copies have SHA-256 `10437793d52eb39a323159a016ac1971cfb23902b531c37415a66af86741d689`; the installed executable reports QEMU 10.2.4 and its firmware links resolve. The build ran from 14:08:24 to 14:11:16 UTC and exited zero; see [host-build.log](host-build.log).

CODE READ: the CPU build uses the requested `scripts/bench/build_kf3.sh`, a private QEMU source/build directory, an independent Rust target, niceness 19, `CARGO_BUILD_JOBS=2` and a local `ninja -j2` wrapper. It does not modify shared source/build trees or the running binary. The owner can select REV `e636236c` with the existing perf8 launcher; the exact host command and full host log location are in the local CL1 report.

MEASURED: on 2026-10-22, after the build at revision `e636236cf`, the own host Cargo target was absent, the build process was gone, and owner QEMU PID 2123004 retained the same start time as before round 2. The dev-box target was never created and the rental target was removed before teardown; no lane job/timer remains active. See [host-build.log](host-build.log) and [cleanup.txt](cleanup.txt).

CODE READ: no VM/GPU operation or fastguest-lock acquisition on the trusted host, no Windows install, no merge and no source-branch write was performed. No Windows recovery or real display-engine MIRROR result follows from this CPU build and synthetic CUDA validation.
