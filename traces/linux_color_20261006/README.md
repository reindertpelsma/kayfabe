# Linux compositor and KMS color audit

**STATUS: LIVE, 2026-10-06 — Linux compositor/color result and limits.**

Product source `8bbcd7f3b50a1cfc1bf0f01f0c580527ce7848c0` is the repaired
Windows branch plus an opt-in, engine-lifetime-bounded DMA method diagnostic.
RTX 4070 / host595.91.07; Linux guest open580.159.04, Weston13.0.0, Sway1.9.
No host display settings are changed.

Native read-only control: `native-color-properties.log`, uid1000 / master0,
NVIDIA KMS has 1024-entry gamma/degamma properties; GNOME currently has an
identity gamma blob installed. This is an API/state control, not native pixel
qualification or a native headless compositor run.

First run A: `first-weston.log` records a live NVIDIA-rendered Weston DRM
compositor and the fixed EGL scene (hash `ad584bcbbd263ea5`, ready callback).
Sway did not start: its seatd socket was absent (`first-sway-invalid.log`).
The three Sway screenshots therefore do not qualify any color behavior.
The harness now requires live scenes and uses the installed seatd interface.
The first command also supplied a mistyped full source revision; the actual
immutable binary was `kf3-bins/8bbcd7f3`, SHA256
`1b94b06e95e357eb506e4f584c8bf9ebcd3c9b0a77dcedf72a888351d172bd62`,
built from the full product source above. Run B supplies that exact revision;
the harness now rejects nonexistent source commits.

Run B completed with supervisor/VM exit0. Weston and Sway render the fixed
scene with NVIDIA; its RGBA hash is `ad584bcbbd263ea5` on the native GNOME
client and both guest clients (`native-scene.log`, `weston-result.log`,
`sway-before.log`). The guest console's centered scene matches that buffer
byte-for-byte, using bottom-up RGBA like glReadPixels (`run-b-summary.json`).

The unprivileged wlsunset client requests 2501K via Sway. KMS installs a
1024-entry gamma blob whose endpoint is `65535,42001,19482` (before/after:
null blob). All three console screenshots have identical SHA256
`856320f2970d3b445b30f948faf5c28cde26c96f369628e68ad746bf953bfc69`.
The request emits no gamma-control failure event. Thus output gamma is accepted
but has no effect in this virtual display (`run-b-summary.json`, `sway-warm.log`).

The method diagnostic also records an active input LUT in ordinary composition:
DIRECT10, interpolation off, size1029 (four header entries plus1025 data/endpoint
entries), context DMA nonzero. The default output LUT is also bound; the warm
request changes its context and enables interpolation. The oracle is OGKM
580.65.06 `nvkms-evo3.c:4539–4581,4672–4723,5315–5398`; these paths are unchanged
in580.159.04. The immutable generated table supplies all decoded offsets/fields.
The trace budget has27914 writes remaining, so it covers the full run. This shows
why a bounded identity subset can preserve ordinary images while ignoring an
arbitrary output curve cannot implement its advertised effect. It does not
establish Windows' minimum requirements or general LUT equivalence.

A separate guest-only fault control tests compositor behavior when KMS
actually refuses a nonzero gamma update; its result follows below.

Fault-control run C was invalid: no rejection marker appeared and the nonidentity
KMS blob was installed (`run-c-inactive-fault.log`). libdrm calls its own
`drmIoctl` directly, so the first interposer did not affect it. The revised
control hooks libc `ioctl` and emits an explicit armed/rejection marker;
the runner requires that rejection marker for a valid failure-control result.

Run D is the valid refusal control (`run-d-summary.json`, `run-d-refusal.log`):
the interposer rejects the nonzero GAMMA_LUT atomic TEST_ONLY request with
EOPNOTSUPP. wlsunset receives `zwlr_gamma_control_v1.failed()`, KMS retains its
null gamma blob, and the fixed client remains displayed pixel-exact through
all three snapshots. Sway continues displaying the image; it does not supply a
shader replacement for the rejected output gamma. The VM exits0 cleanly.
This tests client/compositor degradation for a KMS failure, not a new product
refusal policy, and not Windows' behavior.

Interpretation: ordinary SDR images can work with the current default identity
color states. LUTs are used during image composition, and the tested output
adjustment is not working. Refusing an optional color feature need not break
this Linux desktop. Under OWNER_RULINGS §H, a future production subset must
express absent capabilities or named refusal for unsupported actions; it must
not silently acknowledge arbitrary transforms. Physical host monitor settings
remain the host compositor's responsibility. None of this requires declaring
full general LUT emulation a prerequisite for Windows startup.

Validation of the diagnostic source at `8bbcd7f3`: kf-disp unit/integration
tests108/0, Clippy new0, exact Rust/C rebuild (`logic-tests.log`, `clippy.log`,
`build.log`). GPU gates9/9 with11/11 USER births (`gpu-gates.log`); all14
fast gates clean (`ci-gates.log`). Product source GitHub CI
`37460961228` passes. Method logging is default-off and capped at65536 DMA
writes per engine lifetime; free/reallocate cannot replenish the budget. The
nonzero final budgets in both summaries exclude truncation of these runs.
No merge-bar/application claim; master remains `906a76a4`. Full private
run data/overlays stay under `/var/lib/kf-linux-color-20261006/` on the borrowed
PC; local text/PPM data copies are under `/data/kayfabe-runtime/linux-color-20261006/`.
No rental was created; no guest remains running; host595.91.07 is healthy.
