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

CODE READ: all CUDA checks passed, allowing the requested CPU-only trusted-host build of revision `e636236c` to start. Its build/install outcome is pending in this checkpoint; no guest, Windows install or trusted-host GPU operation is part of this lane.
