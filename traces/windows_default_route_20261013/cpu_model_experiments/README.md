# CPU-model experiments and host scripts, 2026-10-13

**STATUS: EVIDENCE, 2026-10-13.** Revision under test: `ec45aad9` (production flags, no tracing, `KF3_TIMER_MAP` dropped). Host: RTX 4070, driver 595.91.07.
Scripts are the unattended host scripts of this session (trusted host only; they read secrets from host files and contain none).

| run | CPU model | app | result |
|---|---|---|---|
| E1 | host,-vmx + Hyper-V enlightenments (hv_relaxed, vapic, spinlocks, vpindex, runtime, time, synic, stimer, tlbflush, ipi, frequencies) | GravityMark Vulkan | 107.4 FPS avg, median 9.01 ms (measured from the guest disk) |
| E2 | same | GravityMark D3D12 | 89.9 FPS avg (harness proof gap: copy counter 0.42 < 0.5, not a product failure); cupy_asynccopy PASS in 93 s |
| E3 | host,-vmx (baseline) | cupy_asynccopy | PASS in 145 s |
| E4 | enlightenments + migratable=off (invariant TSC) | GravityMark Vulkan | 101.4 FPS avg, median 9.72 ms |

Measured: the enlightenments remove the IOAPIC/HPET timer exit storm and cut the CPU-bound run from 145 s to 93 s; invariant TSC adds nothing for FPS. The ~7x Vulkan gap against VFIO (about 711 FPS) is NOT explained by the CPU model. Inferred, not measured: remaining exits (EXIT_NPF, EXIT_MSR, EXIT_HLT) are not the cause.
The first E3/E4 attempt failed (pre-guest hook below the 100 GiB disk floor, then my own overlay-cleanup glob deleted the E4 overlay before the frame statistics were read); the rerun directories are `claude-cpuexp2-*` and `claude-cpuexp3-*`.

Owner-instance launchers (`claude-manual-owner*.sh`): the baseline-based instance (run 2920), the full Windows 11 install (runs 2930, 2940, 2950) and `_inst`/`_inst2`/`_net` broker copies (extra QEMU args, network restriction, MAC, GPU UUID). Findings recorded in the session: an image snapshotted with a staged Windows update fails on first boot (dwm crash loop); a default-identity VM got an Autopilot profile for an unrelated tenant during OOBE (use a unique UUID/serial/MAC for any fresh install and stay offline through OOBE); `gpu-uuid=auto` with an explicit VM UUID needs `GPU_GET_GID_INFO` (0x2080014a), which the host-ABI gate refuses at 595.91.07.
