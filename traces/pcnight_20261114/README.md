# pcnight 2026-11-14: PC night run on kf3 b20fd84b (next head, merge of #410)

**STATUS: LIVE, 2026-11-15.** Everything below is MEASURED on the PC (RTX 4070, host driver 595.91.07, kernel 7.0.0-34,
Windows proprietary image offline, slirp `restrict=on`, faked RTC) unless marked [inferred]. kf3 binary `kf3-bins/b20fd84b`
(source tree rsync-verified by sha256 against `origin/next` b20fd84bf). Raw logs, screenshots and the full evidence tarballs are
on the coordinator in `/workspace/bench/pcnight_20261114/` (not in git: bulk).

## J2 reset lane on bare metal (2 runs, `reset_lane/`)
QMP `system_reset` x5 while the fast guest runs `--defer-liveness --ce-client-guest-ram`. `waits_timed_out=0`, no `NOT complete after 2s`,
5 busy resets waited 48-78 ms (idle-device resets 22-23 ms), `pramin_pool_empty=0`, `pramin_retire_full=0`, no Xid. The Vast 3090 box
(nested KVM) timed out 4 of 5 (`traces/vastgate_20261114`). [inferred] the 2 s overrun there is the nested-KVM cost, not the batch.

## App verdicts (`apps/`, harness `propimage_check.py`, fresh overlay per boot)
3DMark PASS FILE (7297, graphics 7387; earlier runs 7400, 7377), Edge PASS UIA, Superposition PASS FILE (177 s from the log),
eDrawings PASS UIA, SolidWorks PASS FILE (STEP), Resolve PASS FILE.

## Black Myth Wukong benchmark, driven through its own menu (`wukong/`)
Driver: `scripts/bench/windows/wukong_drive/` (OCR of guest screenshots on the host, SendKeys + mouse in the interactive session).
Sequence: language, brightness, data agreement (scroll + E; offline guest), any key, click Benchmark, click Confirm, wait for the
result screen or a GPU watchdog event (guest `LiveKernelEvent` 141/117, `nvlddmkm` 153, game `GPUCrash`).

| Config | starts that reached the benchmark | result screen | GPU watchdog event after Confirm |
|---|---|---|---|
| kf3, guest-visible FB 4096 MiB (image default) | 12 | 0 | 11 (19 s to 323 s after Confirm; one of them hung at loading in a watchdog storm and the guest then exited; one overlapped a harness Edge task); the 12th reached the HUD and was cut by the driver |
| kf3, `--fb-mb 8192` | 5 | 5 | 0 |
| real GPU through VFIO (12 GB), same image/driver script | 3 | 3 | 0 |

- VFIO (12 GB): average 93 / 92 / 93 FPS (max 107-108, min 4-6, low 5th 83-84), 5.3-5.5 GB VRAM used, 1920x1080 Very High, DLSS + frame generation on.
- kf3 8192 MiB: one result screen read: average 36 FPS (max 43, min 4, low 5th 30), 5.6 GB VRAM used. [the other four result screens were not read]
- kf3 4096 MiB: the game's VRAM peak stays at 3.3-3.7 GB (nvidia-smi in the guest); at 8192 and VFIO it is 5.4-5.6 GB.
- Conclusion (correlation from the 2026-11-14/15 Wukong boots w4-w12 on the RTX 4070; causal link [inferred]): the benchmark-phase TDR follows the 4096 MiB guest-visible framebuffer,
  not the image or the guest software. Falsifier tried and not met: kf3 with 8192 MiB TDR'ing in 3 of 3 driven runs.
- Host side, 4096 MiB runs: `mem large apply` took up to 2.0 s (34 applies of at least 1 s in one run) during the watchdog storms, against
  at most 0.35 s in the 8192 MiB benchmark windows (`wuk_w9/slow_summary.txt`). [inferred] slow applies follow VRAM over-commit; in the 19 s
  crash the host log shows no slow apply at all, so they are not the first cause.
- A second failure, independent of the framebuffer size: the game itself TDRs 35-70 s after LAUNCH (before any benchmark) in 3 of 8 valid launches
  at 8192 MiB and twice at 4096 (first launch of a boot, 55-94 s); 0 of 3 under VFIO. At 8192 MiB, two of these were the second launch of a boot right after a completed
  benchmark (`wuk_w9` a2, `wuk_w12` a2, both vram_peak 2064). Cause unknown.
- `tdr_ctx.txt` per run: counters and event counts in the 5 s before the first LiveKernelEvent (guest-to-host clock offset from the polls,
  line numbers from a 0.5 s sampler). No refused BAR0 write is visible (heartbeat `refused` stays 0/1), `walk capacity` re-walks occur (2-14 in the window).
  These windows are about 3000-4000 log lines long and the keyword counts are noisy.
- Driver false positive: a game that has not spawned `b1-Win64-Shipping` after 25 s was scored as exited (threshold now 70 s); such attempts are noted as invalid.

## Not done
J5 (Enscape): not in the image's app list, not run. The Wukong driver is not part of `propimage_check.py`; it needs host-side OCR and is research tooling.
