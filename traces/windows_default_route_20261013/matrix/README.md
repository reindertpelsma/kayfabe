# Regression matrix on the flag-free Windows default route (source ec45aad99), 2026-10-13

**STATUS: RESEARCH (measured), 2026-10-13.** Exact source `ec45aad99a81cb16eb0554b77f0ff331c47cf6d3`, RTX 4070, host 595.91.07, Windows 580.88, no `KF3_WIN_USER_RING_RELAY` flag (the default route), production profile otherwise unchanged. Script: `claude-default-matrix-ec45aad99.sh`
(85 apps that passed at earlier revisions in Codex's combined checkpoint, batches of 8, one fresh disposable guest per batch, `--max-tdr 99`); log: `claude-default-matrix-ec45aad99.log`; per-batch evidence: `b0` to `b10` (`summary.md`, `session.log`, per-app verdicts).

## Result (84 rows run)
**80 PASS, 1 TIMEOUT, 3 BOOT_FAIL.**
- **TIMEOUT: `cupy_asynccopy`** (batch 1): 182 s with no output after the device line (it passed in 3 to 6 s before). Open regression candidate; no relay refusal, dead channel or host Xid in that guest's log. Rerun alone in `claude-campaign-ec45aad99` stage A.
- **BOOT_FAIL: `cupy_cg`, `cupy_check`, `cupy_sha256`** (batch 1): not product failures. The harness restart path failed the next three guest boots because the post-guest hook returned 1: its last step checks `/var/lib` free space above 100 GiB, and the finished guest's overlay left the disk just below it (the same cause broke Codex's run 1650). The hook was patched (finished disposable overlays of the run-number range are deleted before the check; original at `/data/kf-post-hook-original-b081f705.sh` on the host); batches 2 to 10 ran with the patched hook. Rerun in stage A.
- No previously passing app that ran shows a guest TDR, WER, host Xid or wrong output in these summaries. `gravitymark_d3d12` is in the 1660 evidence directory (harness proof verdict only).

## Not claimed
Not a zero-flag run (19 production behaviour flags still on); not a performance result (see `../PERF_GRAVITYMARK_VULKAN.md`: 103 vs 711 FPS on Vulkan); not a cross-VM, soak or other-driver result.
