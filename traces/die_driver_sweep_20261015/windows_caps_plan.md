# Native Windows display-capabilities measurement

**UPDATE: MEASURED, 2026-10-15.** Native AD102/RTX4090 Windows580.88 LTSC26100
loads the bounded module after on-target test signing and reads capsword0x00c=0x100,
both before and after enabling GSP580.65.05.99nonzero words; only the decisive
scalar ships through an exact AD102/580 T3 row. Evidence:
[load/read](ad102_4090/native-module-caps-580.txt),
[GSP-enabled read](ad102_4090/native-small-caps-gsp580.txt).
The SHA256 values below identify the trusted unsigned artifacts before signing.
GA104 nativeWindows580 remains unmeasured. The original not-run checkpoint
below is superseded for AD102 only.


**STATUS: RESEARCH, 2026-10-15.** The stock Linux 595.91.07 driver on our
GA104 rental exposes word `0x00c = 0x100` in the physical display-capabilities
page. This is measured text in `ga104_3070/codex_ampere_caps_linux595.txt`.
It is not yet a published Windows 580 row: the task requires a native Windows
reference on each die before using that row for Windows acceptance.

H-WCAP, before implementation/runs: extend the existing privileged, read-only
Windows measurement driver with one bounded snapshot of `NV_PDISP_FE_SW`, and
native Windows on the same die can provide the deciding word and its own driver
version without capturing identifiers or touching GPU state.
Falsifier: the tool reads outside the fixed page / writes device state, Windows
cannot load it, or the stock driver leaves that page unreadable. A differing
native Windows word falsifies reuse of the Linux value for that branch.

Public source for the fixed read range: ogkm display-capabilities classes
`C573/C673/C773/CA73`, `kdispGetDisplayCapsBaseAndSize`, and the pinned source
and field map in `docs/design/V3_DISPLAY_CAPS_DERIVATION_20261014.md` §0/§1.
No guest-selected address or length is accepted by the measurement ioctl.

GPU-free checks: the new caps whitelist test failed before implementation;
C tests and the 1024-word snapshot bounds tests pass with ASan/UBSan. The
read-only mutation guard passes. The current Windows build at `32973658a`
succeeded using pinned official WDK/SDK NuGet packages on the trusted host.
Driver SHA256: `371744be56f471aca3a5e882bf08afc03525cc008525ef7cf44a41d81306e2af`.
Collector SHA256: `766dd94a088decfbd76db18b1ad4c18bd819bfa4461797e9dfbfaae755449afd`.
Original pre-run checkpoint (superseded by the AD102 update above): native load/read had not run. The output is unsigned;
the existing installer test-signs it with a disposable on-target certificate.
Evidence: `windows_caps/{before,after,read_only_guard,build}.txt`.
