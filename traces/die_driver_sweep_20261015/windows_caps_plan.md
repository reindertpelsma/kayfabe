# Native Windows display-capabilities measurement

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
