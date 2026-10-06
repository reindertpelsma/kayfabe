# Bounded SDR LUT implementation and comparison

**STATUS: RESEARCH, 2026-10-06.** Branch `codex/sdr-lut-20261006`; opt-in
`KF3_DISPLAY_SDR_COLOR=1`, default off. Product source
`2aa8b92de6c6ab158f9bc8e788be792408a074a3`, immutable QEMU SHA256
`4bf444aeb1aa0b82312c201b7441a89d9c892eb638e255af3eac8a88e0e82c55`.
Borrowed RTX 4070 AD104, open host 595.91.07. No rental or master promotion.

Linux guest open580.159.04 runs Weston13 and Sway1.9. The unprivileged
wlsunset gamma client requests a 2501K output curve through KMS. Final run C
has no injected KMS fault or gamma protocol failure. Its changed console
capture matches the complete actual 1024-entry KMS ramp across **6,220,800
RGB components**, zero mismatches and zero maximum error. Removing the ramp
restores the original bytes. This establishes actual nonidentity output LUT
processing in this cell; it is not inferred from an accepted ioctl.

- [Final Linux run C](linux-c-summary.json), [independent pixel check](linux-c-pixel-oracle.json).
- Earlier source controls: [A](linux-a-summary.json), [B](linux-b-summary.json), [B pixel check](linux-b-pixel-oracle.json).
- [Real GPU fixtures](color-gpu-2aa8.log): nonidentity FP16 input, unsigned output,
  retained snapshot after source mutation, changed snapshot after rearming,
  fractional OLUT interpolation, and invalid FP16 rejection all pass.
- [GPU gates](gpu-gates-2aa8.log): 9/9, 11/11 USER births.

Windows 580.88 runs against the same frozen baseline and UEFI variables, with
constructor probes disabled. [Run 11](windows-11-summary.json) enables the
real kernels before the surface-loading declaration change.
[Run 12](windows-12-summary.json) enables the implemented DIRECT10 capability
sizes and surface loading. Both report Code 43 / smi exit 9, 363 RPC records,
zero display methods and zero scanout refusals. Their clean supervisor exits
and healthy host are recorded. No Windows kernel-execution success is claimed.

The [fresh run-12 dump summary](windows-12-watchdog-summary.json) records a
changed fatal path: helper `0x169d836` before buffer allocation/map operations.
Pinned static driver code strongly identifies a zero-size TMO descriptor when
TMO_PRESENT is absent; the live descriptor size was not recovered. Complete
saved records are readable; the outer NVCD is one byte short and its checksum
cannot be validated. Run 12 at revision `2aa8b92d` used read-only NBD/NTFS
recovery; its receipt records verified cleanup. Raw dumps,
Windows driver bytes and loaded pointers remain private.

Reproduce the bounded static correspondence with the
[pinned inspection script](https://github.com/reindertpelsma/kayfabe/blob/fba14de6/tools/windows-debug-capture/inspect-startdevice-j.py)
and [dataflow](https://github.com/reindertpelsma/kayfabe/blob/fba14de6/tools/windows-debug-capture/evidence/2026-10-05-startdevice-j.md).
The result calls for real TMO behavior or a supported path avoiding it, not a
capability-only workaround. Additional Windows blockers can remain afterwards.

Quality at product source: workspace tests **4,171 passed, 9 ignored** (includes
the frozen tree), format check passed; Clippy existing debt 415, new 0, resolved 7.
Product [CI 37468780443](https://github.com/reindertpelsma/kayfabe/actions/runs/37468780443)
and evidence-head [CI 37469128490](https://github.com/reindertpelsma/kayfabe/actions/runs/37469128490)
passed. The frozen raw client and fast-guest image rebuild successfully.
Its native 30-arm baseline refuses this host driver at R2 because its frozen
encoders support only 580.x: **0/30, all before any arm ran**. This is a
harness/driver-version limitation, not a passing hardware merge bar. No host
module downgrade was attempted. The matched 580.159.04 fast guest passes **30/30**, zero failures/crashes/not-run,
exit zero at the same product/binary. This is a general regression with SDR
opt-in off, alongside the separate opt-in Linux/GPU fixtures above.
[Fast suite](fast-suite-2aa8.log), [native version refusal](native-grader-version-refusal.log),
[quality receipt](quality.json). The first fast attempt omitted the guest-driver
property and therefore mismatched the rebuilt driver; it was stopped after the
R1 refusal was identified, then corrected and rerun. No native parity claim
follows from the matched guest result. All owned VMs exited, NBD is disconnected,
and the host GPU remains healthy.

Scope: one opaque RGB8888 Linux SDR cell. Unsupported input CSC, nonidentity
FMT, OCSC1, active TMO, segmented/mirrored LUTs, nonunity normalization and HDR
input entries are refused. Cursor colour semantics beyond the separate existing
path are not validated. Pure vocabulary/binding tests cover all four families
and both generated class versions; Blackwell surface scanout is not claimed.
Native KMS readback does not capture physical post-LUT pixels, so no native
post-LUT parity claim is made. Completion and lifetime policy, allocation bounds
and failure recovery limits are in [the design](../../docs/design/V3_SDR_COLOR.md).
