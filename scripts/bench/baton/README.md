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

---

# Baton debug, 2026-10-20 (branch claude/baton-debug-20261020)

STATUS: RESEARCH, 2026-10-20 16:05. Labels: MEASURED (a run), CODE READ (source), INFERRED (reasoning).
Host root@172.22.1.20, RTX 4070, 595.91.07; fast guest 6 GiB, 4 vCPUs, `KF3_DEBUG=1 KF3_DEBUG_FORCE_MIGRATABLE=1`,
`KF3_TSG_MIRROR=0` unless stated. Evidence on the host: `/data/claude-baton-debug-20261020/` (runs/, rf/, q/); small evidence
here in `results-20261020/` (`ab_table.txt`, `events.tsv`, per session `report.tsv`, `loop_summaries.txt`, `watchdog.txt`).

## New tools (all GPU-free parts tested: `python3 -I test_*.py`)

- `baton.c`: `--burst N`, `--sleep-ms`, `--wd-ms/--wd-out/--hb-dir` = the SELF-WATCHDOG: a second thread; if no iteration completes for
  2 s it dumps, per episode, `/proc/self/task/*/{comm,wchan,syscall,stat,stack,status}`, `/proc/self/fd`, `/proc/driver/nvidia*`
  files, nvidia lines of `/proc/interrupts`, the peers' last completion age (mmap'd heartbeat files) and `cuStreamQuery` from the
  watchdog thread; then `WD_RECOVER` with the gap. `wd_parse.py` parses/classifies it (11 tests). No strace anywhere.
- `guest_driver.sh` (in-guest, replaces `/bin/kftool`; `MODE:DUR[:k=..:burst=..:spin=1:wd=..]`), `session3.sh` (one boot, env, vCPUs,
  binary, xid abort), `holder.sh` (queue under the lock), `campaign_report.py`, `ab_table.py`, `events_table.py` (tests),
  `stallprobe.c` (stop-the-others controller), `qsampler.py` (host thread sampler, little value: workers are ~15 % busy during a stall).

## 1. Reproducer: honest result

No harness parameter gives >= 60 % of runs >= 4 s within 120 s. Best known: plain `pair`, K = 5 ms, blocking sync, 2 clients, 4 vCPUs,
USER route forced. MEASURED hit rates per 120 s pair run (control-equivalent builds, n = 16 runs: 0d7ba743 base+a+b1+b2, and 0d325edab):
**>= 1 s: 7/16 (44 %), >= 2 s: 5/16, >= 4 s: 2/16 (13 %)**. Variation tried (x1, 80 s runs, 1 each): K=1, burst 8, 3 and 4 clients, spin, heavylight,
one client bursting, K=20, grid 16: all 0 stalls (n = 1 each, uninformative). Three clients + a keep-alive client: 2/4 stalls but only 2.1-2.3 s; a dense
keep-alive (500 Hz): 0/4. Stalls cluster in the second run of a boot and in the first 70 s; no config reached 40 % at >= 4 s.
The default path (no force) is clean: 0 stalls in 2 pairk runs (n small) and in the previous agent's ~16 runs.

## 2. Guest-side picture of a starved client (MEASURED, 11 watchdog episodes incl. 3 from the fix arm)

Identical every time (`events.tsv`): the main thread is in `cuStreamSynchronize` = `futex_wait` (`wchan=futex_do_wait`, syscall 202);
`cuda00001400006` is in `poll` with an INFINITE timeout, `cuda-EvtHandlr` polls with 61-100 ms timeouts, no thread is in an nvidia
ioctl, no D state, fds normal; the other client's last completion is 0-6 ms old; **`cuStreamQuery` = CUDA_ERROR_NOT_READY** in all 11 (a
lost interrupt alone would show SUCCESS). So the stream's completion is not visible to the guest, not merely unannounced. Drivers
stats (`/proc/driver/nvidia*`) show nothing; the guest `dmesg` shows no Xid.

## 3. Host counter that marks every stall (MEASURED, 14 events)

`USER_RING_LOOP SUMMARY ... backstop_found(contended=N)` rises by >= 1 in exactly the sessions/runs with a stall (all 14 events incl.
sub-second ones and both arms; 0 in clean runs, e.g. rf1_C, rf4_F pair1). `contended` = the safety net re-rang a relay that was waiting
as `Wait::Contended` or had `retry` demand (`userpump.rs user_ring_peek_one`).

## 4. Knob A/B (MEASURED, n = 2 per arm per round; second rounds dropped for the coordinator's relay-fairness test)

| arm | n | >=1 s | >=2 s | >=4 s | max s |
|---|---|---|---|---|---|
| base (0d7ba743) | 2 | 1 | 1 | 1 | 9.1 |
| a PUT_CACHE=0 LAZY_OBSERVE=0 | 2 | 2 | 1 | 0 | 3.1 |
| b1 LIFECYCLE_MODE=always_scan | 2 | 0 | 0 | 0 | 0 |
| b2 LIFECYCLE_MODE=no_pointer_markers | 2 | 1 | 1 | 0 | 2.8 |
| c read-back 03e6dfe2 | 2 | 1 | 1 | 1 | 8.5 |
| d VA-refresh 5c1a864d | 2 | 0 | 0 | 0 | 0 |
| e mirror ON (default) | 2 | 0 | 0 | 0 | 1.0 |
| f 1 vCPU, g control (no force) | not run (time) | | | | |

Reading: a, b1, b2 are NO-OPS for this workload by construction: all baton channels stay in Scan mode (`entries_pointer=0` in every
USER_RING_LIFECYCLE line), PUT cache / lazy observe / markers-in-Pointer never run (CODE READ + MEASURED). So base+a+b1+b2 = 8 runs of
the same config (4/8 >= 1 s). c and d binaries all contain `KF3_DEBUG_FORCE_MIGRATABLE` (strings); they are different builds, not a one-flag
change. No arm separates from base at n = 2.

## 5. Relay-fairness fix (coordinator's mechanism; binaries built here from the archive trees, `kf3-bins/{0d325edab,cfdcaf73b}`, differ only by the fix)

Pair, 120 s x 2 per boot, interleaved C F F C C F F C, n = 8 runs per arm: **C (0d325edab): 3/8 >= 1 s, 2 >= 2 s, 1 >= 4 s (4.2 s);
F (cfdcaf73b): 3/8 >= 1 s, 2 >= 2 s, 1 >= 4 s (6.2 s)**, with `busy_looks` 38-89 k per session in F. Same guest picture in F (futex wait,
NOT_READY). The fix does not remove the starvation. CODE READ why it is incomplete: the busy `Look` only calls `relay.peek(Event/Spin)`, and
`user_loop::may_retry` refuses those two looks once a relay has had `CONTENTION_RETRIES` (64) consecutive contended turns; only
`Peek::Backstop` (an idle 50 ms epoll timeout) may ring it. MEASURED consistent: in F every stall still ends with `backstop_found(contended)`.

## 6. Sync-mode discriminator (control binary 0d325edab)

Spin sync (`spin=1`, the client polls the stream): **0/6 runs >= 1 s**; blocking sync (same binary, n = 8): 3/8 >= 1 s. One-sided Fisher
p = 0.15 (3/8 vs 0/6), 0.07 pooled with the 0d7ba743 base (7/16 vs 0/6). Suggestive only (n). Reading if it holds: the guest's
wake-up path (completion interrupt / event) is involved, but cuStreamQuery = NOT_READY says the completion state itself is late too.

## 7. Barrier / marker analysis (CODE READ; H-WFI / H-BARRIER)

Per published batch in Scan mode (`userpump.rs publish_gate`, staged-copy path ~2285): `[staged copy GP entry of the guest segment]
[barrier][witness/marker]`. Barrier = `Storage::write_host_barrier`: one 64-bit CONTROL GP entry, `1<<63` = NOP, LENGTH 0, SYNC_WAIT
(`user_storage.rs:551`): fetches no byte, no method. Marker = `write_host_marker`: a 16-byte LEVEL_SUBROUTINE segment
`[SET_REFERENCE hdr, value, NON_STALL_INTERRUPT hdr, 0]` (`user_storage.rs:~580`). No `SEM_EXECUTE` acquire/release, no `WFI` method anywhere in
relay-authored entries. SYNC_WAIT semantics (ogkm `kernel-open/nvidia-uvm/uvm_hal_types.h:378-387`): "fetch of this entry waits until the
previous entry has finished executing by ESCHED; a complete engine sync needs WAIT_FOR_IDLE in the previous entry", i.e. a HOST-side wait, not an
engine wait. SET_REFERENCE is described in the repo review (`V3_WINDOWS_USER_RING_RELAY_REVIEW_20261012.md:66-72`) as waiting the targeted engine
(not verified in public headers: `clc56f.h` only defines the method). So the barrier cannot by itself park a PBDMA behind the other TSG's engine
time; the marker might (INFERRED). Existing flag: `KF3_LIFECYCLE_MODE=no_pointer_markers` removes markers only in Pointer mode, which baton never
reaches, so no flag exists to publish without barrier/marker in Scan mode. Minimal experiment (NOT made): in `publish_gate`/staged path skip
`write_host_marker` (use the control witness, `write_host_control_witness`) for compute-only channels, and observe whether the stalls vanish. Against
H-BARRIER: the stalled client's work is not on the engine at all (the peer runs at the solo rate, GPU 94-97 %), and the stall ends without
anything on the GPU changing.

## 8. Which RM/hardware rule could starve one TSG for tens of seconds (CODE READ ogkm 595.84; GSP scheduler source is NOT in the tree)

Default TSG timeslice 1024 us (`kernel_fifo_gv100.c:46-52`), interleave MEDIUM, ctxsw timeout 4.19 s then Xid 109 (`RmEngineContextSwitchTimeoutUs`):
a TSG that is runnable and not preempted for 6-59 s with no Xid is NOT what a time-slice/preempt rule produces. What does match: the TSG is not
pending (work not published, or a doorbell missed: `NVC361_NOTIFY_CHANNEL_PENDING`, `uvm_turing_host.c:225-236` orders GPPUT, wmb, doorbell). Observable
that would prove a hardware-side cause: out-of-band CHRAM read (`NV_CHRAM_CHANNEL`: PENDING, ENABLE, ACQUIRE_FAIL, ON_PBDMA bits,
`dev_runlist.h:26-73`) of the starved channel during a stall, or FECS context-switch events (`fecs_event_list.c`, `cl90cdfecs.h`) showing no
CONTEXT_START for it. NOT FOUND: any scheduler counters in `/proc/driver/nvidia`, any debugfs, any nvidia tracepoint except `nvidia_dev_xid`
(`nv-tracepoint.h:34`); nvidia-smi/NVML source is not in the tree, I did not run nvidia-smi probes. UVM tracker waits CPU-spin on a tracking
semaphore (`uvm_tracker.c:331-358`); only libcuda's blocking sync uses RM non-stall events.

## 9. Ranked hypotheses, falsifiers, next experiment

1. H-RELAY-ABANDON (relay-side, CODE READ + MEASURED correlation 14/14): a relay whose turns end `Need::Contended` is re-rung only a bounded
   number of times by spin/event/busy looks (64), then only by an idle-timeout backstop; with a busy neighbour that timeout is rare, so the
   relay's next publication/completion service waits seconds. Falsifier: a build that rings an exhausted contended relay from the busy look (or a
   time-based backstop independent of epoll idleness) still shows stalls with `backstop_found(contended)` = 0, or no `contended` ever > 0 in a stall.
   NEXT EXPERIMENT: that one change on top of cfdcaf73b (keep `busy_looks`), pair 120 s, n >= 12 per arm, ABBA; plus a host log line when a relay
   crosses 64 contended turns and when it is next served (time between = the stall).
2. H-WAKE (guest wake-up/completion service): spin sync 0/6 vs blocking 3/8 (p 0.15). Falsifier: spin shows >= 1 s stalls at a blocking-like rate with n >= 12.
3. H-MARKER-WAIT (section 7, SET_REFERENCE waits the engine): weak (peer runs at solo rate). Falsifier: marker-less publication still stalls.
4. H-GPU-SCHED: unsupported (no Xid, 4.19 s ctxsw timeout, peer solo-rate); proof would need CHRAM/FECS reads.

## 10. Caveats

- n is small everywhere; ">= 40 % control hit rate at n >= 10" was NOT reached (best 44 % at >= 1 s, 13 % at >= 4 s), so F vs C is underpowered (3/8 vs 3/8).
- Harness faults: the first `session3.sh` was edited while a session ran (e2_pair_probe post-processing redone by hand, its tag is complete);
  the host sampler matched the `timeout` wrapper in the first sessions (b1 and earlier have no samples); a first copy of the fix binary was
  identical to the control (cargo mtimes) and was deleted and rebuilt (md5 differs, `busy_looks` string present).
- The stall probe (`stallprobe`) never fired (needs >= 1.5 s stall in a pair with detection); the stop-the-others experiment is untested.
- Owner VM: stopped cleanly 13:32:16 (exit 0, 5 min after READY), restored 15:52 as run 3432 (READY 15:52:03, `hostname` answers), host Xid count 25 throughout.

---

# Baton Pointer-mode and worker-spin runs, 2026-10-20 (branch claude/baton-pointer-20261020)

STATUS: RESEARCH, 2026-10-20 18:40. MEASURED / CODE READ / INFERRED kept apart. Host root@172.22.1.20 (RTX 4070, 595.91.07), host Xid count 25 before,
during and after. Fast guest 6 GiB, 4 vCPUs, `KF3_DEBUG=1 KF3_DEBUG_FORCE_MIGRATABLE=1 KF3_TSG_MIRROR=0`, plain `pair`, K = 5 ms, blocking sync,
2 x 120 s pairs per boot (session3.sh). Data: `results-pointer-20261020/` (`results_fp.tsv` = every pair, `events_and_counters.txt`,
`wd_*.txt` = six guest watchdog dumps); `fp_table.py` makes the tables. Evidence on the host: `/data/claude-baton-debug-20261020/{runs,qp,results_fp.tsv}`.
The owner's VM was restored as run 3433 (READY 18:31:25, `hostname` = kayfabe-windows).

## T1. Why the Linux baton channels never reach Pointer mode

CODE READ: Scan -> Pointer is exactly one transition, `Lifecycle::entry` `Mode::Scan`, `o.sw_mask != 0` with no live registration
(`user_lifecycle.rs`; `Cause::SoftwareBound`). `sw_mask` is the set of subchannels 5-7 on which a SCANNED `SET_OBJECT` bound the channel's SOFTWARE object
(`user_scan.rs software_mask`), and that object exists only if the guest allocated a class 0x5080 (`NV50_DEFERRED_API_CLASS`) child on the channel before
its first publication (`userpump.rs user_ring_software_object`, sets `t.user_defapi`). Windows dxgkrnl does that on every channel; libcuda/UVM on Linux never does.
Not conditions of the switch: ring size, SYNC_WAIT use, a window trigger, `KF3_DEBUG`. `KF3_LIFECYCLE_COUNT` and `KF3_LIFECYCLE_PERIOD_S` (the owner launcher's
WIN_FLAGS) appear in NO binary (`grep -c` = 0 in 0d7ba743 and 0d325edab) and in no source: they do nothing, and no PERIOD line exists in any log.
MEASURED: all 80 channels of an old Linux session (rf4_C): `scan_no_software: 1`, `set_object_5080_total: 0`, `mode=Scan`. Windows run 3432 qemu.log: 96 lifecycle lines,
89 `mode=Pointer` with `set_object_5080_total: 1`, 7 Scan with 0. (The hypotheses "ring size / SYNC_WAIT / window / KF3_DEBUG" are dead by code, so no n = 3 runs
were spent on them; falsifier that would have revived them: a Linux channel with `set_object_5080_total > 0` still in Scan: none in the 28 Linux sessions' lifecycle lines.)

How to get Pointer on Linux: no guest-only way (the guest would have to allocate the 5080 object AND emit `SET_OBJECT` into libcuda's pushbuffer). Added a
MEASUREMENT-ONLY mode `KF3_LIFECYCLE_MODE=force_pointer` (`user_lifecycle.rs`, `FORCE_POINTER_AFTER` = 16 scanned entries, then the same
transition as `SoftwareBound` with no registration; not in `LifecycleMode::ALL`, unit test added). MEASURED: 15 of 80 channels (all the CUDA compute ones)
`mode=Pointer`, `entries_pointer` 20k-26k, `pointer_markers` 13k-17k, `pointer_pb_read_violation` 0, `pointer_backstop` 0-1, in every force_pointer session (13 sessions). `USER_RING_LOOP SUMMARY`
IS printed with `markers` > 0 in Pointer mode, so the missing summary in Windows run 3432 is not "Pointer has no markers" (INFERRED: the VM was not torn down
cleanly enough to print "all relays gone"; not checked).

## T2. Hit rates (pairs; >= 1 s / >= 2 s / >= 4 s starved, max s). Binaries: C = 0d325edab, F = cfdcaf73b (busy-look fix), both + force_pointer patch (default mode unchanged)

    arm                                n    >=1s  >=2s  >=4s  max
    Pointer, C   (PC, 5 boots)        10     2     1     1   15.4
    Pointer, F   (PF, 4 boots)         8     1     1     1   18.9
    Scan,    C   (SC, 4 boots)         8     2     1     1    5.8
    Scan,    F   (SF, 4 boots)         8     3     2     1    4.3
    Pointer, pw fix  (WF, KF3_DIAG_PROGRESS=1000)            4     2     1     1    5.4   (2 boots only, see below)
    Pointer, pw ctl  (WC, + KF3_NEGCTL_NO_BUSY_LOOK=1)       4     1     1     1    6.5

MEASURED, interleaved ABBA blocks (C F ... / F C ... per boot order in `qp/`). Pointer mode does not remove the stall and does not visibly worsen it (C: Pointer 2/10 vs Scan 2/8; F: 1/8 vs 3/8;
n far too small to rank). The fix does not separate from the control in either mode (Scan C 2/8 vs F 3/8; Pointer C 2/10 vs F 1/8). The reproducer rate in this campaign (2-3 of 8 at >= 1 s, 1 of 8 at >= 4 s) is
lower than the earlier 44 % (the SCb arm below, same binary and config, gave 6/8): the rate varies by boot batch. pw arms: only 2 boots per arm run (WF1 WC2 WC3 WF4; the second ABBA half
was parked for time): n = 4 pairs each, NOT an A/B result. Same host state, same guest, same pair script.

## T2b. H-WAKE falsifier: the worker never blocks (separate commit, `KF3_DEBUG_WORKER_SPIN=1`)

Stated before the run: H-WAKE = the baton is a lost/late-wake class; with a worker whose park wait is `epoll_wait(0)` (every empty pass ends in the relays' backstop peek) the >= 1 s rate falls
to ~0; falsifier: rate stays >= 20 % at n >= 8 pairs. Binary 0d325edab-fp-spin (C + force_pointer patch + the spin diagnostic; Scan mode, env only on the spin arm), interleaved SP SC SC SP SP SC SC SP (4 boots per arm).

    arm                      pairs  >=1s  >=2s  >=4s  max gap
    SP  worker never blocks    8      0     0     0    0.01 s (every pair, both clients)
    SC  control (same binary)  8      6     5     2    17.6 s       (one-sided Fisher 0/8 vs 6/8: p = 0.0035; pairs of one boot are not independent)

H-WAKE NOT falsified: SP stayed at 0 of 8 against 6 of 8 in the interleaved control. Confounds, MEASURED: the sampler shows both `kf3-worker` threads in state R in ~88 % of 0.25 s samples with SP
(1887/2134, 1903/2146, 1871/2144, 1896/2146) versus 6-12 % with SC (127/1924, 240/1931, 165/1928, 208/1932), i.e. two cores at ~100 %; host has 24 cores, guest 4 vCPUs in both arms. SP also changes more than the wake:
it bypasses `user_loop::may_retry` (the 64-contended-turn limit) because the backstop peek always rings, and `backstop_found(work=...)` is 390-733 per SP session versus 0 in all SC sessions (INFERRED, ambiguous: work found
by a constant poll can be a few microseconds ahead of the normal wake, or work the normal wake would only find at the 50 ms backstop; the counters cannot tell). So the result says "a worker that polls everything continuously
removes the stalls" (wake/look class, or the contended-retry limit), it does not by itself separate a lost wake from the retry limit. Next discriminator: spin only the doorbell/event wake but keep `may_retry` (not built).

## T3. Evidence of the starvations (six guest watchdog dumps in `results-pointer-20261020/wd_*`; events in `events_and_counters.txt`)

MEASURED, identical in every watchdog episode of this campaign (14 episodes >= 2 s, in Pointer and Scan, C, F and pw arms; e.g. PF15 18.9 s, SCb3 17.6 s, SCb7 10.2 s, SC11 5.8 s, WF1 5.4 s, WC2 6.5 s): the main thread in `cuStreamSynchronize` = `futex_do_wait`
(syscall 202), `cuda...06` in `poll` with infinite timeout, no thread in an nvidia ioctl, `cuStreamQuery` = NOT_READY, the peer's last completion 0-6 ms old. Nothing in guest dmesg.
Host side, pw build (`PROGRESS_*` blocks): in WF1 (fix arm, 5.4 s stall at 1.4-6.9 s) and WF4 (1.3 s stall) the host printed ONLY `PROGRESS_ARMED`: no required visit point starved and no relay was
"satisfied but unpeeked" for the 1 s threshold during the stall. In the negative-control arm WC2/WC3 the blocks appear (`PROGRESS_STARVED point=worker_poll passes_since_poll=0x16..0xa2 bound=0x10`, age 3-6 ms: the count
rule, expected without busy looks) with `contended_run=0x0`, `busy_looks=0x0`, `backstop_contended=0x0` and relay states `wait=marker waiting_flag=yes`; they also fire in WC3, which had NO stall (6 STARVED lines), so they
mark the missing busy look, not the starvation. `backstop_found(contended)` (last summary of the session): 1 in stalled sessions SCb3, SC11, PF15, WF1, SF5, SF12; 2-4 in SCb2/SCb6/SCb7/fp1_C; 0 in stalled-free PC1, PC16, PF10, WC3, SF4 but also 1 in stall-free SC6, SC14, PF7 and 11 in PF2, 17 in SC3 (sub-2 s or no stall). A weaker correlation than the "14/14" of the earlier note: >= 1 is necessary in this data (every stalled session has it) but not sufficient.
INFERRED (from the fix-arm silence): the stale object during a starvation is NOT a worker that failed to visit a poll point and NOT a satisfied relay wait left unlooked; the stalled client's relay was waiting on a wait that
was not satisfied (private ring progress / marker / completion not produced), i.e. downstream of the relay's looks, or the worker was not the thing that had to move. That fits SP (which also looks at unsatisfied waits much more often)
only if the wake of the guest-visible completion, not the relay's wait, is what lags. What is missing: the relay state AT the stall (the diagnostic dumps only on its rules). Next: an `unsatisfied for > N s while the peer completes` block in progress.rs.

## Caveats

n is small and boots are the unit of correlation; the SP vs SC difference is large, the Pointer vs Scan and F vs C differences are not resolved. `force_pointer` forwards without ever scanning (a bench switch; not a product mode).
The pw binary is the progress-watchdog branch (299fd0ed5) + the force_pointer patch, built here. The second ABBA half of the pw arms was parked for time.

---

# S2: budget vs latency (KF3_DEBUG_WORKER_SPIN=2), 2026-10-20 19:40

STATUS: RESEARCH. Binary `pw3f4b9e82-s2` = progress-watchdog branch 3f4b9e82d + `KF3_DEBUG_WORKER_SPIN` (1|2, commit 527fb1ed7 on this branch, same hunks) + `PROGRESS_STRAND` diagnostic
(`pw-strand-diag.patch`). One binary, env-only arms, Scan mode, plain pair 2 x 120 s per boot, 4 vCPUs, `KF3_DIAG_PROGRESS=1000` in both arms:
C = `KF3_NEGCTL_NO_BUSY_LOOK=1` (the pre-fix loop = control), S2 = C + `KF3_DEBUG_WORKER_SPIN=2` (epoll timeout 0 always; the look after the "park" is `Peek::Spin`, so the retry budget
`CONTENTION_RETRIES` = 64 IS respected; `Peek::Backstop` is never issued, not even the 50 ms net). Boots interleaved C S S C C S S C C S S C. Owner VM (run 3433) stopped per the flag procedure at 18:39
(no `/tmp/kf-owner-playing`), restored as run 3434 (READY 19:35, `hostname` = kayfabe-windows); host Xid 25 throughout.

Stated before the run: H-BUDGET = the baton is the exhausted-budget strand; S2 returns to the control level (>= 4/8 pairs >= 1 s) and `backstop_found(contended)` stays ~0.
H-LATENCY = S2 stays <= 1/8, the cause is worker wake latency.

MEASURED (`results-pointer-20261020/results_s2.tsv`, `strand_lines.txt`):

    arm                        boots  pairs  >=1s  >=2s  >=4s  max gap
    C   control (no busy look)   6     12     2     1     1    14.74 s  (+ two sub-second: 0.59 s, 0.16 s)
    S2  never blocks, budget     6     12     0     0     0     0.01 s  (every pair, both clients)

CPU states (0.25 s sampler): `kf3-worker` threads R in 1863-1909 of 2144-2150 samples with S2 (about 88 %, two cores), 163-203 of 1925-1930 with C (about 9 %). Guest 4 vCPUs, host 24 cores in both arms.
Neither hypothesis is falsified by the RATE: the control level in this batch was only 2/12 (earlier batches: 2/8, 6/8), far below the >= 4/8 H-BUDGET needs, so S2 0/12 vs C 2/12 is one-sided Fisher p = 0.24.
The precondition of the test failed; do not read the rate as a result.

The new datum is the strand line (`PROGRESS_STRAND`, my addition: a relay that is `pending||waiting` with `contended_run >= 64`, announced after 1 s, end line with the total):

    KC6: guest stall A 14.74 s (80.4-95.1 s)  <->  PROGRESS_STRAND relay=0xc1d00010:0x5c00001f contended_run=0x40 h64_backstop_only=yes retry_flag=yes pending=yes, STRAND_END total_ms=0x39a0 (14752 ms)
    KC9: guest stall A 1.77 s                 <->  same relay, contended_run=0x40, STRAND_END total_ms=0x7d0 (2000 ms; tick 250 ms)

Both >= 1 s control stalls coincide with an exhausted-budget strand whose length equals the stall (within one tick); the 10 other C/S2 boots (and all 6 S2 boots) printed no strand. The 0.59 s and 0.16 s stalls are below the 1 s threshold, not checked.
So the stall IS the strand in 2 of 2 cases (MEASURED correlation, n = 2, strand ends when something finally re-rings the relay). In S2 no strand formed even though the budget rule was active and no Backstop fired:
INFERRED: with a worker that looks continuously the 64-consecutive-contended-turn count does not accumulate (the contention clears within a few prompt retries), i.e. budget exhaustion is a consequence of late/rare looks, so
"budget" and "latency" are not separable by this knob; S2 does not distinguish them. The minimal-fix binary (relay-fairness-2) did not exist under /data/claude-*/kf3-bins/ when I looked; the F arms earlier (busy look, no budget change) did not remove the stalls.
