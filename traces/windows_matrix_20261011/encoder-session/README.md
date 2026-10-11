# Authored encoder-session contract

STATUS: LIVE, 2026-10-11. Hardware Windows retry pending.

Candidate0f9c01f9 refused the authored acquire0x20808163 before the host ioctl:
HOST_ABI_REFUSED19314. Windows AV1/H264/HEVC encoding then failed to open a session.
The stock580.88 Windows reference passes all three encoding formats.

Hypothesis before measurement: the existing authored four-zero-byte request is
accepted by host595.91.07. Falsifier: nonzero acquire/release status or a changed
four-byte reply. The probe ran as nobody under the exclusive GPU bench lock,
with no guest bytes, two balanced acquire/release cycles. Every status was0;
each reply remained four zeros. `probe.log` records sourcecdad9267 and verified
binary SHA25626d727fa00e76b91c1c2fbea0cc2b9341ecc27a1c90a0a5c6f6c6ccc4fc37347.

This is measured on AD104/RTX4070,595.91.07 only. It establishes the authored
host ABI contract, not other-die coverage or the cause of separate video recovery.
No reply table ships. Product still asks the real host and propagates its status.
Only these two controls gain exact595.91.07 support, with four-byte size validation;
other tags/opaque controls keep their existing refusal. The published inventory
rows carry the source tag and measurement limits.

`failing-first.log` reproduces the prior ABI refusal; `regression.log` records
its corrected result, including bounded-size and unknown-version negative cases.
`tests.log` summarizes646 GPU-free ABI/host unit and integration test passes.
Linux hardware gates/fast and exact-revision Windows encoding remain required.
