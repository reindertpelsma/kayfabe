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
