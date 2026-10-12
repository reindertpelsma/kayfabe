# Windows1580: source-buffer policy refusal before benchmark

STATUS: MEASURED, 2026-10-12. Exact source4ec9f9a2415f565a50fd57faa82b071afd799258.
RTX4070/Windows580.88; production setup plus default-OFF USER relay experiment.
Hypothesis/falsifier were recorded by runner before launch (campaign.log).

Guest agent answered33s, sign-in verified, NVIDIA ready and GravityMark installer
staged. No benchmark launch/result: several desktop channels clean-refused
USER_RING_GP_FORMAT Budget on~100KiB source entries (including
0x018b2a0014d0b058). This falsifies sufficiency of the64KiB prototype policy,
not migration semantics. ABORTED_PRE_BENCHMARK; no appPASS/FAIL claim.

Controller stopped exact matrix process at18:57:30UTC; harness143. ACPI stop
18:57:49UTC, QEMU gone18:57:51UTC, DMA-FQ18:57:53UTC/no other QEMU. Bounded
18:54–18:58UTC host journal has noXid. Dmesg counts fell with ring rotation,
so numeric deltas are not used. Owner personal image preserved. Only this own
completed disposable child was later removed under GPUlock after all-qcow header
and protected-image audit (overlay-cleanup.json).

Next hypothesis: public ordinary source entries through existing896KiB staging
budget preserve desktop/GravityVk progress. Falsifier: native padded-barrier test
fails, buffer/cache refusal persists, or Windows loses progress/device/output.
One behavior variable: trusted source/cache budget, no cursor/migration semantics
change. Private staging moves before shared publication guards; admission remains
exact before privatePUT. Benchmark oracle/media/production flags unchanged.
