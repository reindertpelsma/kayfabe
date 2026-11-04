# Walk wake: the pending-walk wait on the walk's time scale (Ada PC, RTX 4070)

STATUS: ANSWERED, 2026-11-03. H1 (root cause) confirmed, H2 (no regression) holds, on one die (AD104) and one
host. The hypotheses and falsifiers were committed before any run (`PREREG.md`, commit `557448dc9`). Every
number below is MEASURED unless marked INFERRED.

**Host:** the owner's trusted PC: RTX 4070 (AD104), host driver 595.91.07, kernel 7.0.0-34. No other QEMU was
running; the owner's VM was off. **Under test:** `4f420b373` (`claude/walk-wake-20261103` = `next` 70e3fabd8 +
this fix). **References:** the Ada lane's runs of 70e3fabd8 (`next`, regressed) and 9297ffdd9 (`next` before
#239) on the same host, harness, fast guest and Windows image (`origin/claude/ada-pc-regression-20261103`,
`traces/ada_pc_20261103/`). The references were not re-run today; the host rebooted in between.

## What changed and what was kept

Why the callback went (history): `fc5c83379` (R34, TU106) removed the per-walk `cuLaunchHostFunc` eventfd signal
because it raised one GPU nonstall interrupt per walk (45,190 vs 3), and made the current walk's `cuEventQuery`
the only completion proof. Both properties are kept: `completion::ready` is unchanged and no host function runs
per walk. What was missing was a timely look: the VA worker slept a fixed 1 ms while a walk was pending.

Now the VA worker sleeps `clamp(age / 2, 50 us, 1 ms)` (`completion::poll_us`) in `epoll_pwait2` with a
microsecond timeout, and its thread's timer slack is 5 us instead of 50 us. Idle stays at 50 ms. Every QEMU
logged `kf3: VA manager: pending-walk wait 50us..1ms (epoll_pwait2) (timer slack 5us)`.

## Linux: fast suite 30 arms, 180 s, host-mode fast guest (guest driver 595.91.07)

| | fix 4f420b37 | next 70e3fabd | pre 9297ffdd |
|---|---|---|---|
| verdict | **30/30** (`FAST_SUITE_RC=0`) | 30/30 | 30/30 |
| `v3_gates.sh` | **9/9 + gate10** | 9/9 + gate10 | - |
| per-walk latency `vat[walk_avg_us]`, arms with walks | **121-234 us** | 1008-1011 us | 106-226 us |
| GPU time `gpu_avg_us` | 81-183 us | 81-199 us | 82-199 us |
| `concurrency` invalidate arrival-to-clear | **141 us** | 1029 us | 152 us |
| suite client time, sum of 30 arms | **70,186 ms** | 128,352 ms | 67,176 ms |
| ratio to pre | **1.04** | 1.91 | 1.00 |
| `concurrency` / `ce-client-guest-ram` / `concurrent-fuzz` | 3,600 / 21,270 / 1,339 ms | 14,560 / 62,890 / 3,350 ms | 3,370 / 20,940 / 1,290 ms |
| host Xids per suite | 5, the admitted shapes | the same 5 | the same 5 |

The 5 Xids are the predeclared shapes: `defer-liveness` 4x Xid 31 CE `FAULT_PTE` at `0x60_/0x62_/0x62_/0x64_00010000`,
`missing-page-fault` 1x `FAULT_PDE VIRT_WRITE` at `0xa_11000000` (`host_xid_lines.txt`, the last 5 lines).

Gate 8 (bare metal, one walk each, n=1): `walk_ns` 111,789 vs 1,058,601 on next; the vCPU's spin until the
invalidate cleared 219/179/231 us vs 1171/1142/2140 us (`linux/gates.log`).

Per-arm tables: `linux/fix_vs_pre.txt`, `linux/fix_vs_next.txt` (`ab_compare.py` of the Ada lane).

Not attributed to this fix: the arms with no walks still cost about 60-100 ms more each than pre (e.g. `timer`
480 vs 419 ms), and `uvm-invalidate`/`uvm-mean` cost 819/1,190 ms vs 490/619 ms pre (next: 799/1,270 ms).
INFERRED: another change in #239; this run cannot tell which.

## Windows 11 (the Ada lane's recipe: owner image chain, run-4501 flags, display on), boot `w1fix`

Guest passes every H3 criterion of the Ada lane (`windows/w1fix/guest_summary.log`):
- desktop signed in (`dwm=1 explorer=1`), adapter **Code 0** at every one of the ten idle polls;
- `nvidia-smi -L`/`-q` work before and after the idle;
- `cup8` PASS `bad=0 maxerr=0`, d3d11 OK (`ms=109`, then `ms=16`), d3d12 OK, cup3 rc=0, before and after a
  600 s idle;
- gears 90 s windowed: 3470 fps before, 2659 fps after; 0 STOPPED, 0 STALL, 0 flip timeouts;
- reset counters since boot: `display4101=0 nvlddmkm153=0 kernelpower41=0 eventlog6008=0 bugcheck1001=0
  livekernelevent193=0`. `livekernelevent_other=78` is the same stale queue as in both Ada-lane boots: 27 queued
  report lines, all with event times from 2026-10-21 to 2026-10-22, none from this boot (`steps/28_wer.txt`).

Stop was by agent shutdown (`rc=0`). The backing image was unchanged. There was no new host Xid (15 -> 15).

| | fix w1fix | next w1 | pre w2pre |
|---|---|---|---|
| walk latency, weighted | **178 us** (23,954 walks) | 885 us (25,442) | 169 us (23,954) |
| GPU time | 117 us | 115 us | 115 us |
| invalidate arrival-to-clear | **396 us** | 1199 us | 390 us |
| `kf3-vamgr` CPU, whole run | **1.86 %** of one core | 1.76 % | 2.04 % |
| `kf3-vamgr` CPU, idle (604 s) | **0.50 %** | 0.45 % | 0.46 % |
| `kf3-vamgr` CPU, while gears ran | **1.07 %** (pre-idle), 0.14 % (post) | 1.09 % | 1.46 % |

## Verdicts against `PREREG.md`

- **H1 confirmed.** The falsifier (walk latency above 500 us, or client total above 100 s, with the microsecond
  wait in effect) is not met: walks take 121-234 us, the client total is 70.2 s (within 4.5 % of pre). The
  1 ms pending wait was the added latency.
- **H2 holds.** Fast suite 30/30, the same 5 admitted Xids, Windows Code 0 with 0 resets. `kf3-vamgr` CPU is
  within +0.1 percentage points of next and below pre.
- GPU-free: `cargo test -p kf-linux-raw -p kf-cuda -p kf-qemu -p kf-harness` 786 passed, 0 failed (dev box).

Negative runs: 0. Runs: 1 Linux validate (build, gates, fast suite), 1 Windows boot.

## Files

- `linux/`: validate summary, gates log, `build_kf3` tail, suite table, per-arm logs (tgz), the two A/B tables.
- `windows/w1fix/`: guest summary, raw guest step outputs, host summary, CPU phases, `qemu.log.gz`, `cpu.jsonl.gz`,
  the scrubbed command, screenshots. `windows/probes.sha256`: the probes, cross-built from `4f420b373`.
- GPU UUIDs and VM identity are replaced with `fa4efa4e-...` placeholders. Host-local scripts: the Ada lane's
  `tools/` with only the run directory changed, plus `win_run.sh` and `collect_ww.sh` (not product).
