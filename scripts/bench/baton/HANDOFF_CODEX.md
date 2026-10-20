# HANDOFF to Codex: baton hardware campaign (written 2026-10-20 23:30, Claude Sonnet 5.5)

STATUS: LIVE. MEASURED unless marked. Host root@172.22.1.20 (RTX 4070, 595.91.07), Xid count 25 throughout. Owner VM is NOT running
(stopped 21:01:19 by `touch /tmp/kf-stop-claude-manual`; last run 3434). Restore: `cd /data && rm -f /tmp/kf-stop-claude-manual && N=3435 REV=0d7ba743
WIN_FLAGS='KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5 KF3_TSG_MIRROR=0' setsid nohup flock -o /tmp/kayfabe-fastguest.lock bash /data/claude-manual-owner-perf6.sh
> /data/claude-perf6-3435-launch.log 2>&1 < /dev/null &`, wait for OWNER_SESSION_READY, `python3 /data/kfagent_server.py run hostname 150` = kayfabe-windows.
Check `/tmp/kf-owner-playing` first; never break /tmp/kayfabe-fastguest.lock (wait with flock -w). The queue holder (qp3) holds the lock until its STOP file is seen
(STOP exists; YC52 was the last session running at 23:25 and finishes by itself; nothing chains after it; unstarted 53-55 are in `qp3/parked/`). If `fuser -v /tmp/kayfabe-fastguest.lock`
shows a holder still, look at `qp3/holder.log`; do not kill a running boot.

## Binaries (all under /data/claude-baton-debug-20261020/kf3-bins/<rev>/, with pc-bios/qemu-bundle symlinks)
    pw3f4b9e82-s2  C control   sha256 bc4cf654...b1269   (pw branch 3f4b9e82 + KF3_DEBUG_WORKER_SPIN + PROGRESS_STRAND; arm C is env KF3_NEGCTL_NO_BUSY_LOOK=1)
    aaceb73cb      B (OLD)     6b33f297...cd089   relay-simple head; its Watched wrapper did not forward unsourced(): runs WITH KF3_DIAG_PROGRESS are INVALID for B
    fc8e97315      B2          922047b5...31127   Watched unsourced fixed
    c452ca7cb      B' (use)    5087dd61644eee83496d78f0006a80f722a12fa71e4f69b669042fab415c46bb   relay-simple head (Watched + act-hold + phantom demand fixes)
    89fc8a222      M  (use)    d9dbb6553f256e6f27af65ec8e13999e1a72bc605fa9a107b1c5c5794133ced9   relay-fairness-2 (handoff + PtLock); KF3_WORKER_PARK_INFINITE=1 = net off
Windows launcher copies (same files, symlinks resolve, `test -x` ok): /var/lib/kf-windows-20261005/kf3-bins/{89fc8a222,c452ca7cb}/ -> use REV=89fc8a222 or REV=c452ca7cb with
/data/claude-manual-owner-perf6.sh. No Windows run was done by me.

## How a session runs (one boot = 2 pairs x 120 s, 4 vCPUs, K=5 ms, blocking sync, KF3_DEBUG=1 KF3_DEBUG_FORCE_MIGRATABLE=1 KF3_TSG_MIRROR=0, set by session3.sh)
Queue: `/data/claude-baton-debug-20261020/qp3/NN_TAG.sh` scripts (`. qp3/lib.sh; sess TAG pair:120,pair:120 QEMU_BIN=$B/<rev>/qemu-system-x86_64 FGD=<dir> ENV...`), run by
`flock -o /tmp/kayfabe-fastguest.lock bash $D/holder.sh $D/qp3` (runs scripts in name order, waits for new ones, exits when `qp3/STOP` exists; remove STOP first). Results: `$D/results_z.tsv`,
`$D/runs/<TAG>/` (fast_<TAG>_qemu.log, guest_baton.txt, files/*.wd = guest watchdog dumps, qsample.tsv = host thread sampler, report.tsv). Initrd FGD: `fg4` (idle mode) or `fg5` (idlectx mode, newer baton).
Arm env: C = `KF3_NEGCTL_NO_BUSY_LOOK=1 KF3_DIAG_PROGRESS=1000`; B', M = `KF3_DIAG_PROGRESS=1000`; M-infinite = + `KF3_WORKER_PARK_INFINITE=1`. Report: `python3 -I qp/tools/z_report.py runs results_z.tsv`
(copy in this directory: z_report.py, qs_cpu.py; CPU = sum of d_cpu_ticks / wall, NOT the share of sampler rows). Host build: `buildB3.sh`-style (rf/wt*, CARGO_TARGET_DIR copy, `find crates -name '*.rs' -exec touch {} +`
first, otherwise cargo reuses a stale artifact: mtimes survive rsync -a).

## Results so far (pairs; a boot = 2 pairs). `results-pointer-20261020/{results_z.tsv,z_report.txt}`
    arm (flag on)                 boots pairs  >=1s >=2s >=4s  max     PROGRESS_STRAND  backstop_found_work  worker cores (load / idle ctx)
    C  pw3f4b9e82-s2 (YC*)           7    14    5    5    4   12.59 s       8                0              0.09 / 0.001
    M  89fc8a222 (YM*)               6    12    0    0    0    0.01 s       0                0              0.016 / 0.001
    M  + PARK_INFINITE (YMI40)       1     2    0    0    0    0.01 s       0                0              (backstop=0 in the summary: no hang)
    B' c452ca7cb (YT37, YT50)        2     4    0    0    0    0.01 s       0              1028 (366+662)   0.011 / 0.000
    B2 fc8e97315 (YB*)               5    10    0    0    0    0.02 s       0              2239             0.011-0.015
    B  aaceb73cb WITH flag (ZB*)     4     8    0    0    0    0.01 s       0              2542             INVALID for B (diag wrapper bug); no valid unflagged B run exists
    C earlier batch (ZC*)            4     8    1    1    1    8.89 s       1                0
    C earlier batch (KC*, other binary) 6   12    2    1    1   14.74 s       2                0
Idle (guest idle, 60 s): kf3-worker CPU 0.000-0.001 cores in every arm (IDB/IDC no CUDA context, IXB/IXC/IXM/IXT idle CUDA context; `IXx` ">=1 s" rows are the 1 s sleep of the idle client, not stalls).
M cost: `unannounced_spins` 0.88-1.02 M per session (the spin obligation), `handoff(registered=756 rung=756 retried=645)` in YM36: registered <= rung+retried (falsifier not hit). `timeouts_with_work` is not printed (WorkerStats prof off): NOT measured.
Control hit rate by batch: ZC 1/8 pairs, YC 5/14 (>=4 stalled pairs reached), KC 2/12. Every control stall ended with a PROGRESS_STRAND (contended_run=0x40) of the same length (KC6 14.75 s, KC9 2.0 s).

## Verdict per arm against the stated falsifiers (MEASURED counts above; small n)
- M: no stall, no strand, backstop_found_* = 0, no hang with the infinite park, handoff accounting ok in the one summary read. Not falsified so far (6 boots + 1 infinite boot; the plan was 12).
- B' / B2: no stall and no strand, BUT `backstop_found_work` is 300-700 per boot: the literal falsifier "backstop_found_work > 0" IS hit (the 50 ms net finds satisfied waits that nothing announced; no
  guest-visible stall at the 10 ms gap resolution). Decide whether that is a bug counter hit or accepted cost; I did not run it with a net-off knob (B has none as an env; only a test knob).
- C: reproduces (5/14 pairs >= 1 s), as required for power.

## Remaining
1. More interleaved ABBA boots, 3 arms C / M / B' (each >= 12 boots, now C 7+4, M 6, B' 2): script pattern `qp3/{50..55}` (parked: `53_YT53 54_YT54 55_YC55`).
2. On the winner: `scripts/bench/v3_gates.sh` (9/9) then `KF3_DEVICE=kf3 scripts/fastguest/fast_suite.sh <tag> 180` (30/30), strictly serial, same lock.
3. Windows: owner starts REV=<winner> only after Linux passes.

## Traps
Builds during a boot perturb it (nice 19 helps; some of my boots overlapped builds). cargo mtime trap (above). `fg` initrd driver has no `--sleep-ms` baton; `fg5` has. The "R-state fraction" in my earlier README
sections is rows in state R over sampler ROWS (the sampler omits quiet threads), so for the control it overstates; use qs_cpu.py. Pairs of one boot are correlated; rates vary by batch (2/12 to 6/8).
Evidence of B's invalid arms and S2/force_pointer work: README.md sections of this directory.
