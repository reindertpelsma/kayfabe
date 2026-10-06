# Explicit Linux plane TMO gate

**STATUS: RESEARCH, 2026-10-06.** Implements the owner's correction: ordinary
rendering and output gamma do not establish that a tone-mapping buffer exists.
The new `linux_color_audit.py --sdr-color --require-tmo` experiment runs a
stock Linux580.159.04 guest with Sway, on an immutable product binary.
The hardware result is pending at this checkpoint; no TMO success is claimed.

The guest-only `request_tmo.c` adapter requests a fixed 1024-entry zero-intensity
UNORM16 LUT on the active primary plane in Sway's real atomic submission.
It uses the stock driver's exposed property identifiers and blob API. It does
not change a shader or implement TMO. Missing `TMO_LUT` is an explicit adapter
error before kernel submission; it is never replaced with ordinary rendering.
The diagnostic is not loaded into the host or VMM, and no host output is changed.

Baseline, requested and restored phases use the same grayscale EGL scene and
neutral background. The strict checker requires all of the following:

- A ready NVIDIA-rendered grayscale baseline and a loaded request adapter.
- A successful real atomic request, not just TEST_ONLY.
- A nonzero TMO binding and 1029-entry control armed at the warm capture.
- An unexhausted method trace and no scanout refusal.
- A ready warm scene, changed zero-intensity output and byte-exact restoration.

Missing/skipped stages fail the experiment, even if the compositor stays alive.
The grayscale zero-intensity fixture is a narrow processing witness; it does not
qualify general TMO/HDR or native post-LUT parity. `test_tmo_stage.py` has five
GPU-free tests, including false-positive controls for successful ioctls without
bindings, unarmed bindings, disabled/freed bindings and missing properties.
Its synthetic positive case only tests the checker, not a hardware result.

Primary source path: OGKM580.159.04 `nvidia-drm-crtc.c` exposes `TMO_LUT` in the
ICtCp pipeline and builds its linear VSS table in `create_drm_tmo_surface()`.
`nvkms-kapi.c::AssignLayerLutConfig()` only forwards TMO when the layer supports
it. `nvkms-evo.c::nvNeedsTmoLut()` can skip absent TMO independently of OLUT.
These are source reads, not evidence of active tone mapping in a guest.
