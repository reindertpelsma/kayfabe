# Explicit Linux plane TMO gate

**STATUS: RESEARCH, 2026-10-06.** Implements the owner's correction: ordinary
rendering and output gamma do not establish that a tone-mapping buffer exists.
The new `linux_color_audit.py --sdr-color --require-tmo` experiment runs a
stock Linux580.159.04 guest with Sway, on an immutable product binary.
**Run C: PASS for the direct zero-intensity fixture**, product and harness
`74cd590cec952e1b206d54532345ba0d481d0cd4`, QEMU SHA256
`72f2d50d0b4a93330cd359119cbcf2ffff487bc311fde5439e3ca6baaa185937`.
The [strict verdict](linux-c-tmo-verdict.json) passes every check: property
available on plane49, retained 1024-entry zero curve, three accepted real atomic
commits, three nonzero TMO bindings (`0x10099`/`0x1009c`) and matching control
`0x40509`. Window7 remains armed at capture, with 45,272 trace budget remaining.
The requested output is all black; removing the curve restores the baseline
RGB bytes exactly. No scanout refusal. This establishes the owner's explicit
Linux stage-presence/processing fixture, not general HDR/native pixel parity.

The register pipeline and tone snapshots now execute on the GPU. Six existing
SDR fixtures pass in [SDR GPU log](tmo-74cd-color-sdr.log); five tone-fixture
categories pass in [TMO GPU log](tmo-74cd-color-tmo.log), including intensity-only
behavior, the full source-derived PQ/CSC program, snapshot retention/rearm and
invalid-table rejection. [GPU gates](tmo-74cd-gates.log) pass 9/9, with 11/11 USER
births; [QEMU build](tmo-74cd-build.log) records the immutable binary. The first
synthetic run at `82328c29` found a diagnostic-priority regression: invalid input
set both input-invalid and derived-nonfinite verdict bits. `74cd590c` reports the
primary table failure; the rerun above passes. No success no-op was introduced.

[Manifest](linux-c-manifest.json), [events](linux-c-events.json),
[provenance/hashes](linux-c-provenance.json), [runner](tmo-c-runner.log), complete
compressed Sway phase logs and capture-time/full method traces are retained.
The requested RGB hash is
`1f56bd4f609fab80a2b9cce7487d5c08de2768476849e1353881ca748d8d3b6a`;
before/restored both retain the run B baseline hash below.

**Broader work remains:** the owner explicitly wants table-format and chroma
control refusals replaced with real processing. This current no-chroma-correction
TMO checkpoint is not completion of that scope. Windows behavior remains a
separate test; the absence fallback identified in NVKMS is documented as Unix
code, not established as shared Windows behavior.

The earlier missing-capability result follows for comparison:

**Run B: FAIL**, product `2aa8b92de6c6ab158f9bc8e788be792408a074a3`,
harness `8316cb612040c0123539de512560f4ea8a5eb26d`, immutable QEMU SHA256
`4bf444aeb1aa0b82312c201b7441a89d9c892eb638e255af3eac8a88e0e82c55`.
Borrowed RTX 4070 AD104, host open595.91.07/kernel7.0.0-34, guest
open580.159.04/kernel6.8.0-142. No TMO success or Windows fix is claimed.

[Strict verdict](linux-b-tmo-verdict.json): active primary plane 49 has no
`NV_PLANE_TMO_LUT`. Three requests are rejected by the guest adapter with
EOPNOTSUPP at TEST_ONLY, before a kernel TMO submission. Zero real TMO atomic
commits, zero nonzero TMO bindings and zero TMO control words are recorded.
The capture trace has 19,913 method records and 45,623 remaining budget, so an
exhausted trace does not explain the absence. The scene callback still succeeds;
all three console captures have identical RGB SHA256
`9449c5353875d3b003b607a478df9e6303feb0216c34719564a549f8a597340f`.
This is the skipped-stage case the owner's stricter test was intended to catch.
The runner exits 1; the VM shuts down with exit 0. Restoration is byte-exact,
with no scanout refusal. [Host health](host-health.log) records the connected
DP-1 output, live GNOME, responsive GPU, no VM and no NBD attachment.

The [native read-only control](native-tmo-properties.log) exposes
`NV_PLANE_TMO_LUT` and size 1024 on four host planes, read as uid1000/master0.
No native TMO curve was submitted and no host display setting changed.
The host and guest driver/kernel versions differ: this is an API-availability
control, not matched native TMO processing or pixel parity. OGKM580 attaches the
property conditionally on extended properties and `supportsICtCp`; this run
does not capture the live value of that resource capability.

The first diagnostic run A was not a qualified result: the adapter initially
used an incorrect unprefixed property name, and the checker expected a window
FREE method absent from the class. Both were corrected before fresh run B.
Run B independently retains the complete KMS color-property listing and grades
end to end without that instrumentation error.

[Manifest](linux-b-manifest.json), [events](linux-b-events.json),
[runner](linux-b-runner.log), [provenance/hashes](linux-b-provenance.json),
[KMS baseline](linux-b-kms-before.log), compressed full Sway phase logs and
capture-time/full QEMU traces are committed here. Console PPMs and raw guest
logs are retained privately under `/data/kayfabe-runtime/tmo-stage-20261006/`.
The product binary was unchanged; only the diagnostic harness was rebuilt.
Five GPU-free checker cases, Python compilation and C `-Wall -Wextra -Werror`
compilation pass; these validate the harness, not an implemented TMO stage.

The guest-only `request_tmo.c` adapter requests a fixed 1024-entry zero-intensity
UNORM16 LUT on the active primary plane in Sway's real atomic submission.
It uses the stock driver's exposed property identifiers and blob API. It does
not change a shader or implement TMO. Missing `NV_PLANE_TMO_LUT` is an explicit adapter
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
bindings, unarmed bindings, disabled bindings and missing properties.
Its synthetic positive case only tests the checker, not a hardware result.

Primary source path: OGKM580.159.04 `nvidia-drm-crtc.c` exposes `NV_PLANE_TMO_LUT` in the
ICtCp pipeline and builds its linear VSS table in `create_drm_tmo_surface()`.
`nvkms-kapi.c::AssignLayerLutConfig()` only forwards TMO when the layer supports
it. `nvkms-evo.c::nvNeedsTmoLut()` can skip absent TMO independently of OLUT.
These are source reads, not evidence of active tone mapping in a guest.

## Variable table extension and Windows retest

Product `8dde9b51d` on the same borrowed RTX4070 passes all eight GPU tone
fixture categories ([log](tmo-8dde-color-tmo.log)) and six existing SDR fixtures
([log](tmo-8dde-color-sdr.log)). These add a nonuniform-segment lookup with an
independent expected pixel, a minimum 65-sample/552-byte source table and a
malformed compact header rejection. The snapshot allocation remains fixed;
copy and lookup extents follow the bounded register-authored sample count.
This is a synthetic GPU qualification, not native display-chroma parity.

Windows run13 uses product `74cd590c`, constructor probes off, same pinned
580.88 driver and baseline. [Command](windows-13-command.json),
[status](windows-13-status.json), [full trace](windows-13-qemu.log.gz),
[recovery receipt](windows-13-receipt.json) and
[labelled watchdog comparison](windows-13-watchdog-comparison.json) are retained.
Code43 and nvidia-smi exit9 persist. It reaches 8,330 display methods, zero
UPDATEs/scanouts, and changes after the same first17 assertions compared with
SDR run12: first changed hint `0x169d836` becomes `0x1a103e6`. The fresh dump
mtime is after this run's start; the linked receipt records NBD cleanup. Its NVCD
outer payload is one byte short, so only complete saved records are compared;
no checksum-valid claim or live-local inference. This establishes a changed
initialization boundary, not that Windows now works or that TMO was its only
blocker. Raw dump/driver binaries remain private.
