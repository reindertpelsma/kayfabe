# Linux compositor and KMS color audit

**STATUS: RESEARCH, 2026-10-06 — experiment in progress.**

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

A separate guest-only fault control now tests compositor behavior when KMS
actually refuses a nonzero gamma update; that run remains in progress.
