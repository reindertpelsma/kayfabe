# STATUS 2026-10-18 (night): branches in flight, evidence, decisions waiting

STATUS: LIVE, 2026-10-18 04:00 CEST. A hand-off note, not a design doc; the binding decisions stay in `docs/OWNER_RULINGS.md`.

## Integration lineage
- `integration/prod-candidate-4-20261017` (0452613b5): candidate-3 + display pacing phases 1+2 (no parked flips, broker tick source, lock step, 50 ms clock fallback). Includes the broker-side cursor plane (true XOR cursor, formats, hover, import, resize). Running on the owner's Windows VM (run 3004).
- `integration/prod-candidate-5-20261018`: NOT created yet. Plan: candidate-4 + `claude/hitch-20261017` (b233037d9) + `claude/bootlayer-20261018` (0dbcc8a14).
- `claude/hitch-20261017` (b233037d9): colour-path flip fix (a window flip completes behind the copy in flight; the mailbox copies once per tick), the window latch as one `ScanState` method, colour bindings resolved at the latch, step meters, bounded lock-step event ring, broker `--stats-line` (off by default), bench scripts and evidence (`traces/hitch_20261017/`). Heaven D3D11 Ultra 1080p fullscreen vsync on: average 51.5 -> 57.1 fps (9 baseline boots, 5 fix boots, spread about 0.5); gaps >= 100 ms and the 1% low did NOT improve beyond the spread. Reviewed; all review items done.
- `claude/bootlayer-20261018` (0dbcc8a14): boot layer shown only while store [0,G) is clean, [0,G) zeroed on the GPU at driver unload, reboot scrub of the whole store (policy `OWNER_RULINGS.md` section AM: scrub only where the same process reuses the same store for a new OS), realize zeroing awaited. Reviewed twice; blocking defects fixed. NEVER run on hardware.

## Open findings (not fixed)
- Startup stall: all broker request timeouts (40-46 per session) fall in the first ~5 s; gdb shows the QEMU main thread waiting for the BQL while `kf3-vamgr` is inside the RM `alloc_os_descriptor` ioctl from `tspace::prewarm`.
- Windows: the BAR1 physical view is refused at every driver unload (`56 placement(s) ... still overlap [0,G)`), so the boot layer's return on Windows is not exercised.
- Taint at fn 72 freezes the boot spinner for ~12 s; a BAR1-mode PHYSICAL write mid-life could trigger the unload zero under a live RM.
- Long-run degradation: the owner's VM (run 3004) slowed progressively after ~2 h of mixed use (Heaven, Minecraft, Blender) to a 2 fps desktop with app crashes. Evidence on the bench host `/data/claude-hang3004-20261018/` (qemu.log, broker log, kernel journal, `analysis/STATE.md`). MEASURED: guest nvlddmkm 153 x3 at 02:52:52 CEST and, 6 s later, host `NVRM virtmemAllocResources: VA Space alloc failed! Status 0x51` (kayfabe falls back to the 4 KiB path); only guest copy engine 3 (Windows paging queue) busy at ~100%; guest VRAM 2.6-3.4 of 8 GiB (overcommit hypothesis falsified); QEMU's 24 GiB RES is the 12 GiB guest-RAM memfd mapped twice, not a leak. Cause OPEN.

## Decisions waiting on the owner
1. Candidate-5 as the owner's default launcher once the hardware validation is green.
2. Taint only when the console reservation C < G (boot spinner freeze).
3. P1 scan-off safety: raw-client rebind test + tripwire counter (not started, awaiting go).
4. Delete the kayfabe-side cursor composing path (cursor plane is broker-side); ack-detector / dmabuf_pitch fixes.
5. Flap damping keep/remove, vblank cap under flood, strict stall bound (phase 2 deviations).

## Overnight work (2026-10-18): six Codex lanes L1-L6 (prompt files outside the repo), reports under `/root/codex_overnight_20261018/reports/`.
