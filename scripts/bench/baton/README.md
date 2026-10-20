# baton - GPU time-slice starvation probe

STATUS: LIVE, 2026-10-20 (native baseline measured; guest run pending).

Question: when two GPU-heavy processes share one GPU, does one of them make no progress for
seconds while the other runs (a "baton"), and then complete its queued work at once?  This probe
gives the native baseline and the same test for a Linux guest under kayfabe.

Files: `baton.c` (libc + `dlopen("libcuda.so.1")`, CUDA driver API, hand-written PTX, no nvcc),
`run_baton.sh` (solo / pair / heavylight drivers), `baton_parse.py` (log parser, stats, baton
detector, report), `test_baton_parse.py` (GPU-free tests).

## Build and run

    gcc -O2 -Wall -o baton baton.c -ldl          # needs only gcc + libc; libcuda is loaded at run time
    ./run_baton.sh -o out solo 60                # one client: calibration baseline
    ./run_baton.sh -o out pair 180               # two clients, K = 5 ms launches
    ./run_baton.sh -o out heavylight 120         # light K = 5 ms + heavy K = 50 ms (-k / -H change them)

`run_baton.sh` calibrates once solo (`./baton --calibrate-only`), then starts all clients on a
shared `--start-at` epoch so their timed loops begin together, samples
`nvidia-smi --query-gpu=timestamp,utilization.gpu,clocks.sm,temperature.gpu` at 1 Hz, aborts
(SIGTERM to its own children only) if the kernel-log `Xid` count rises, and prints the report.
Outputs per client: `<mode>.<label>.log` (stdout of baton), `.dump.csv` (every iteration), `.err`.

Two processes by hand (e.g. in a guest where `run_baton.sh` is awkward):

    CPM=$(./baton --calibrate-only | sed -n 's/^CYCLES_PER_MS=//p')
    S=$(python3 -c 'import time; print(time.time() + 8)')
    ./baton --label A --duration 180 --cycles-per-ms $CPM --start-at $S --dump-all A.csv > A.log &
    ./baton --label B --duration 180 --cycles-per-ms $CPM --start-at $S --dump-all B.csv > B.log &
    wait; python3 -I baton_parse.py report A.log:A.csv B.log:B.csv

Each client's loop is `launch(kernel) ; cuStreamSynchronize ; record`.  A kernel is 128 blocks x 256
threads (fills every SM) and lasts ~K ms.  `--kernel burn` (default) executes a fixed amount of
dependent integer work, so its run time is GPU *occupancy* time.  `--kernel clock` spins on
`%clock64` until N cycles passed; because the SM clock keeps counting while the kernel is
time-sliced out, its duration is wall time and it overstates work done under sharing (measured:
two clients appeared to do 1.64x of the solo work per second).  Use burn for shares, either for gaps.
Context sync mode: blocking sync by default, `--spin` for spin-wait.

## In a Linux guest (kayfabe)

Build in the guest (`gcc -O2 -o baton baton.c -ldl`) or on a box with an older glibc and copy
`baton run_baton.sh baton_parse.py` over (the binary is dynamically linked against libc/libdl only).
Check `./baton --calibrate-only` prints `CYCLES_PER_MS=...` first, then:

    mkdir -p /tmp/baton && cd /tmp/baton
    ./run_baton.sh -o out solo 60 && ./run_baton.sh -o out pair 180 && ./run_baton.sh -o out heavylight 120

Compare gaps and BATON events with the native numbers below.  Inside the guest `dmesg` may not
show Xid; the script then simply has no abort trigger, so keep the durations.

## Metrics

- iterations / iters_per_s: completed launch+sync pairs; per second bucketed by completion time.
- `longest_gap_ms` (the starvation metric): largest interval between successive iteration
  COMPLETIONS, the first interval being t0 -> first completion.  Equals the longest iteration.
- `zero_sec`: whole seconds in [0, duration) with no completion.
- `share`: fraction of GPU work done = iterations x work-units per iteration, normalised over clients.
- BATON event (`baton_parse.py`): a client has a completion gap >= `--min-gap` (4 s) while another
  client completes >= 3 iterations strictly inside that gap (clients aligned by `t0_epoch`).  If
  every client is silent the whole GPU stalled, which is not a baton.
- Output: `sec iterations max_iter_ms` per second, `SUMMARY k=v ...`, and `SLOW_CSV_BEGIN/END`
  with every iteration > `--slow-ms` (50).  Times are seconds since the client's t0.

## Native baseline (host RTX 4070, driver 595.91.07 open modules, desktop on the same GPU)

MEASURED 2026-10-20, source rev = this branch's baton.c; `Xid` count unchanged (25) throughout.
Burn kernel (shares are valid), K = 5 ms, heavy K = 50 ms, clients on a shared start:

    run (duration)         client  iters/s  share   longest gap  zero-sec  iter ms p50/p99
    solo (20 s)            A        197.1   1.000     6.2 ms        0       5.0 / 5.5
    pair (90 s)            A         96.8   0.500    11.9 ms        0      10.3 / 10.5
    pair                   B         96.8   0.500    11.9 ms        0      10.3 / 10.5
    heavylight (90 s)      light     84.9   0.434    12.8 ms        0      11.9 / 12.1
    heavylight             heavy     11.1   0.566   109.4 ms        0      89.9 / 91.6

Clock kernel, longer runs (gaps valid, shares not): solo 60 s 197.2 it/s, gap 6.0 ms; pair 180 s
109.0 it/s each, gaps 12.2 / 10.4 ms; heavylight 120 s light 135.3 it/s gap 10.5 ms, heavy 19.3 it/s
gap 59.5 ms.  No zero-second anywhere, 0 BATON events in all runs.  GPU util at 1 Hz: 99-100 %.
