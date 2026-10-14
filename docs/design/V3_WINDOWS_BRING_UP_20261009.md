# V3 WINDOWS BRING-UP — what hung it, what fixed it, what is left (2026-10-09)

**STATUS: LIVE, 2026-10-09 (rev `a2966111` = invalidate-completes-over-absence + straddle split + T-mode
piece cap + refusal samples).** Kayfabe-side evidence only: counters and log lines from our own runs,
the guest's own OS-level symptoms, and ogkm. No closed-driver material belongs here
(`OWNER_RULINGS.md`: diagnosis only, nothing derived).

## Where it stands

Windows 11 boots under kayfabe on a real RTX 4070 (Ada, host driver 595.91.07, guest driver 580.x),
signs in, runs Edge and plays several YouTube Shorts (run 237). Not yet reliable: the guest driver
still resets (TDR) about every 40-45 s in some runs.

## Four defects found and fixed (each measured, each a kayfabe defect)

| # | Symptom | Cause (measured) | Fix | Evidence |
|---|---|---|---|---|
| 1 | Guest hung polling `MMU_INVALIDATE` (BAR0 `0xB830B0`, trigger bit 31 set) holding its locks; TDR 0x117 | An invalidate whose walk refused leaves was never cleared (2026-09-25 ruling) | `OWNER_RULINGS.md` §AA: complete over absence (refused map / walk-refused leaf); a refused unmap or invalidate still holds the clear | run 223: `inval=3138 cleared=3137 unreconciled=1`; runs 221/190/181/162 same shape; after the fix `unreconciled=0` |
| 2 | Host GPU faults (Xid 31) at VA `0xb0000`/`0xff000` when apps start | Rows crossing the 1 MiB edge of the twin space's low reservation refused as `VA_STRADDLES_RESERVATION` (0x4B71) | `VaSpace::dma_pieces`: split at reservation edges, all-or-nothing with rollback | run 225 Xids; run 226+ no refused maps |
| 3 | Guest paging completions never arrive until the 30 s TDR | The guest kernel's Translated copy ring died silently: `tspace bind: too_many_pieces` (up to 0x765 = 1893 pieces) vs `MAX_PIECES = 64` | `MAX_PIECES = 4096` (still a fixed bound) | runs 229-232 `DEAD:` lines; run 237: 5496 paging packets, 0 lost, max 0.11 s |
| 4 | `walk REFUSED leaves (refuse_mask=0x400)` (MISALIGNED_LEAF) | **Not** big pages with misaligned bases: the 4 printed samples are ordinary PTEs (valid, vidmem, kind 9, targets stepping by 64 KiB) read one level too high: a table read as a directory (torn or reused page). Accepting them would map nonsense | Keep refusing; §AA means the refusal no longer hangs the guest | run 236 samples (`kf3: walk refusal sample ...`) |

## What is left (measured, not yet explained)

Run 237 shows a reset every ~40-45 s. Each starts when the guest's display service polls for monitor
changes, which suspends the guest GPU scheduler. The suspend finishes, but **the guest's 60 Hz VSync
interrupts never come back afterwards** (60/s until t=134.07 s, then 0 until the reset at t=151 s;
nothing else outstanding: 0 packets in flight, scheduler silent), so the next operation blocks until the
watchdog. Kayfabe's own display head keeps ticking at 60 Hz (`kf3: display fps ... armed=60 tick=60`),
so the lost piece is the delivery or re-enable of the vblank/head-timing interrupt after the resume.
Next experiment (run 238): display register write trace + guest graphics-scheduler trace on the same
timeline. Audit (`V3_COMPLETION_AUDIT.md`, branch `claude/completion-audit-20261009`) lists the stale-state
candidates (no device reset; display state survives guest driver unload; shadow publish race).

## Verification state

- `c7d83a5f` (completion over absence): v3_gates 9/9, fast suite 30/30, Linux broker lane PASS,
  `absent_cleared=0` on every Linux arm.
- `a2966111` (adds the straddle split, piece cap, ABI-6 walk kernel): unit tests green
  (`kf-cuda`, `kf-chan`, `kf-mem`, `kf-host`: 350 passed); **the Linux suite, v3_gates and `kf-gate9` have
  NOT been run on it.** Do not merge before they are.
- Windows evidence runs: 221-237 under `boundary-kayfabe-*` on the bench host (not in git).

## Tooling notes (host-side, outside git)

Scripted keystroke sign-in and Edge launch, QMP memory dumps, guest watchdog dumps read offline, and
the guest graphics-scheduler trace (decoded in the guest). Never edit a script a run is executing; never
`pkill -f` a pattern that appears in your own ssh command line.
