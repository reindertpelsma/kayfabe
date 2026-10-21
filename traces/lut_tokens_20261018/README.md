# LUT snapshot identity across guest lives, 2026-10-18

CODE READ: the display worker retains its DisplayGpu across guest life resets.
`DisplayGpu::color_snapshot` caches the tuple `(src, interpolate, token, entries)`.
Resetting ColorDmas to its default cleared bindings but also restarted tokens at
zero. Identical LUT location and shape in the next life could therefore reuse the
old GPU snapshot without issuing a fresh device-to-device copy.

Falsifier stated before testing: if identical location/shape after a reset gets a
fresh snapshot key and resolves its binding again, the suspected reuse is absent.
MEASURED: two tests in `a2164f5d7` fail on gated hitch (both reused token 1).
The GPU-free key model is pinned to the actual GPU cache tuple by a source assertion.
CODE READ: the fix clears all per-life slots and retains the checked token counter;
exhaustion retains the existing named refusal, without wrapping or token reuse.
MEASURED: the fixed tree passes all 327 QEMU release tests.

MEASURED: restarting the counter kills both new tests. Retaining per-life slots
also kills both. No GPU run is claimed; actual snapshot submission needs hardware.
