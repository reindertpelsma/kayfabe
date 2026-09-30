# m2b — the scanout copies are pixel-exact (trace digests); the harness screendumped too late

**STATUS: MEASURED, 2026-09-30 — superseded as the M2 grade by `../m2c_20260930/`; kept as the proof
that the copies carry the guest's pixels.**
Exact binary: kf3 `2444911a` (`run_m2b_rev.txt`), vast 53505783, RTX 3060, 580.159.04,
`bash dlane.sh m2b KF3_DISPLAY_TRACE=1` (tests 658 / 0).

`KF3_DISPLAY_TRACE` prints, for the first copies and every 50th, the copy's source and — once it
completed — the FNV-1a of its R,G,B bytes, the digest `kfdisp_probe` prints for its patterns
(`run_m2b_qemu.log.gz`):
- copies 1–8: window 6, head 3, ISO `0x1008f` (fbcon's framebuffer, base `0x200000`);
- copies 50, 100, 150: ISO `0x10095`, base `0x1200000` → **`fnv=d42aac677e43ad67` =
  `KFDISP_PATTERN_B_FNV`**;
- copy 200: ISO `0x10093`, base `0xa00000` → **`fnv=b46ca47163518677` = `KFDISP_PATTERN_A_FNV`**;
- copies 250, 300: back on `0x1008f` after the probe restored fbcon.

So every traced copy is the guest's surface, byte for byte. `PATTERN_MATCH=no` here (and in m2a)
was the HOOK: its guest-command helper pipes through `tr`, which block-buffers into a file, so the
probe's `KFDISP_SHOWING=A` reached `show.log` only when the probe EXITED — after its restore — and the
screendump caught fbcon. Fixed in `945da292` (line-buffered for the show step; a late screendump is
flagged `SCREENDUMP_LATE`), graded in m2c. Flips ran at 56.73 Hz under the trace (its digest runs
on the worker, before the completions it delays); m2c, untraced, 60.01 Hz.

`SHA256SUMS` = local hashes; all 17 files matched their remote counterparts. No executable was copied back.
