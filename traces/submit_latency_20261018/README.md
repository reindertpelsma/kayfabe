# Where the ~0.84 ms of one tiny GPU packet goes under kayfabe (2026-10-18)

STATUS: RESEARCH, 2026-10-18 (Sonnet 5.5 agent, owner away). Measurement only; no production change, no merge.
Branch `claude/submit-latency-20261018` (off `claude/steelnomad-gap-20261018` 4f37bd12). Scripts: `scripts/bench/submit_latency/`. Raw data (not in git): host `/data/claude-submitlat-20261018/`. Evidence excerpts: `evidence/`. `INTERIM_superseded_by_README.md` is the first-pass note.
Labels: MEASURED = a run named here. CODE READ = source at 0452613b. INFERRED = reasoning, not evidence.

## Setup
Subject: owner VM run 3006 (rev 0452613b, perf profile, no KF3 measurement flag), RTX 4070, driver 595.91.07 host / 580.88 guest. Guest probe `sl_probe.ps1` (DIRECT queue, ExecuteCommandLists(64 KiB copy) + Signal + poll `GetCompletedValue`), run as a hidden one-shot task.
Tools, all against the unmodified binary, one clock (host CLOCK_MONOTONIC): bpftrace kernel tracepoints (`kvm_mmio` = every trapped BAR0 write, `sched_wakeup` of kf3 threads, eventfd `write`s, `irq_handler_entry` of the host `nvidia` IRQ 85), a few uprobes by address on existing symbols (`gen_bt.py` resolves them with `nm`), and `fencepoll` (read-only observer of the guest-RAM memfd that finds and timestamps the guest's fence word). Probe cost is stated per profile: guest median 849 us with tracepoints only (baseline 805-846), 877 with 10 uprobe hits/submit (`mk`), 960-971 with ~130-450 hits (`pub`, `lock`: counts and orderings only).

## Per-hop table (DIRECT queue, polled; MEASURED; us from D0 = first doorbell trap; r22_seq phase 0, n = 2943 submits, light profile `hostpath`; r9_dir n=3181 agrees within 2 us)
| # | hop | median | p90 | max | n |
|---|---|---|---|---|---|
| - | guest-measured submit-return -> fence visible (r22 step 1 / baseline r0) | 834 / 805 | 975 / 965 | 1649 | 2995 |
| 1 | guest doorbell/USERD write -> BAR0 trap: 3 doorbell traps (0xBB0090, tokens 0x3b/0x3d/0x3c), D0 -> last | 29.4 | 31.7 | 193 | 2943 |
| 2 | trap -> worker thread running (D0 -> first kf3-worker wake) | 8.9 | 9.9 | 16.5 | 2943 |
| 3 | worker relay: 57 GP entries, 3 batch markers; D0 -> last marker published (r5_mk, 3% probe cost: 146.6) | 164.6 | 254.5 | 506 | 2943 |
| 4 | last marker published -> first host GPU interrupt (the private channels execute on the real GPU) | 390.1 | 474.3 | 640 | 2943 |
|   | first / last host GPU interrupt after D0 (3 per submit) | 550.6 / 679.1 | 554.1 / 687.3 | 732 / 1153 | 2943 |
| 5 | host GPU interrupt -> eventfd write to the guest irqfd (host RM + worker) | 10.9 | 13.4 | 29 | 2943 |
| 6a | irqfd write -> first guest ISR trap (injection + vector entry) | 15.9 | 18.1 | 129 | 2943 |
| 6b | first -> last interrupt-tree trap (3 ISR/DPC cycles, 22 trapped writes) | 233.7 | 258.8 | 889 | 2943 |
| 6c | last trap -> completion cell written by the guest (DPC) | 18.0 | 19.0 | 35 | 2916 |
| 6d | completion cell -> next submit's first doorbell (guest sees fence, next Execute) | 24.3 | 41.7 | 93 | 2916 |
|   | period D0 -> next D0 | 856.0 | 896.1 | 1361 | 2943 |
Sum of hops 1,4,5,6a,6b,6c,6d from D0 to the next D0: 29 + (551 to first GPU interrupt) + 11 + 16 + 234 + 18 + 24 = 856 us = the measured period, i.e. the hops account for the whole round trip (the relay overlaps the GPU leg and is not additive). The guest's 834 us excludes the ~22 us of ExecuteCommandLists before the doorbell.

Critical-path attribution of the ~840 us (medians): 3.5% doorbells, ~1% worker wake, ~19% relay (on the path until the last marker at ~165 us, but fully overlapped by the GPU leg), **46% GPU leg after the relay is done (390 us), 27% guest ISR/DPC chain (234 us, interrupts ARE on the critical path of the polled test because the guest only sees the completion cell 18 us after the last ISR write)**, 5% host IRQ + injection (27 us), 5% completion + next submit (42 us).

## Other queue types (same method; r22_seq phases 1-3 and the guest lines in `evidence/r22_seq_guest.txt`; polled unless TE)
| queue | guest round trip med (n) | doorbells | markers | last marker | first host GPU irq | GPU leg (hop 4) | ISR cycles | trapped writes |
|---|---|---|---|---|---|---|---|---|
| COPY (type 3) | 231 (2995) | 1 | 1 | 39 | 45 | **5.6** | 2 | 15 |
| DIRECT (0) | 834 (2995) | 3 | 3 | 165 | 551 | **390** | 3 | 25 |
| COMPUTE (2) | 1485 (1995) | 2 | 2 | 64 | 1275 | **1213** | 2 | 16 |
| DIRECT, event wait (TE) | 887 (2995) | 3 | 3 | 165 | 553 | 393 | 3 | 25 |
Event wait costs +50 us over polling because the thread wake follows the same completion. The compute leg is ~3.1 x the direct leg; the copy leg is ~5 us. A DIRECT submit with an empty command list takes 1.7 us (nothing is submitted).
Host-native reference (`cuda_lat`, no VM traffic): GR empty-kernel launch+sync 3.9 us median (800 us idle gaps: 4.8), CE 64 KiB copy+sync 4.6 us. So an idle host GPU does not take 400 us to wake.

## The answer to "which component"
1. **MEASURED: the largest term is not a kayfabe thread.** The relay has published everything by ~165 us; the first host GPU interrupt (the private channels' first marker) comes 390 us later on DIRECT and 1213 us later on COMPUTE, ~5 us on COPY, while the worker threads are idle/spinning and the host RM -> VMM leg is 11 us. The three private channels reach their markers in series (~554, 623, 678 us in `evidence/r11_gpuprog_one_submit_timeline.txt`). The leg scales with the number of GR-engine channels, not with work size (the earlier size sweep is flat 64 KiB..16 MiB) and not with host GPU wake latency. **INFERRED (not established):** host GPU scheduling of the per-queue twin TSGs/channels on the GR engine (a time-slice or switch cost of ~400 us per GR hop; compute = 3 x direct), or PBDMA semaphore-acquire stalls caused by the relay's channel skew. Neither was isolated; that needs a diag build (below).
2. **MEASURED: the guest ISR/DPC chain is the second term (27%).** 3 cycles per submit, each ~7.3 trapped interrupt-tree writes at 9-10 us (a userspace exit each).
3. **MEASURED/CODE READ: the relay mutex convoy exists but is not what keeps this probe at 840 us** (below).

## Is the 25.7 trapped writes per submit avoidable by design?
MEASURED (kvm_mmio, 2000-3000 submits): 25 per submit on DIRECT = 3 doorbells (0xBB0090, NV_USERMODE_NOTIFY_CHANNEL_PENDING, one per hardware queue channel) + 22 interrupt-tree writes = 3 ISR/DPC cycles x {0xB81408 LEAF_EN_CLEAR(2)=0x101, 0xB81410 LEAF_EN_CLEAR(4)=0xFFFFFFFF, 0xB81610 TOP_EN_CLEAR=8, 0xB81208 LEAF_EN_SET(2), 0xB81210 LEAF_EN_SET(4)=0x0C10007A, 0xB81608 TOP_EN_SET=0xFFFFFFFE, then 0xB81000 LEAF W1C =1, 2 (cycles 1-2)}. (The earlier "6.1 interrupt edges" are the 6 irqfd writes = 3 host GPU interrupts, each written twice, producing 3 guest cycles.)
* The 3 doorbells are inherent to the stock driver (one per channel) and cost 29 us.
* The 22 are Windows' own ISR mask / DPC unmask around each edge; on bare metal they are posted PCIe writes (~free), here each is a ~9-10 us exit (CODE READ cpuintr.rs: applied synchronously on the vCPU because the ISR reads pending right after; an ioeventfd-style deferred apply is not allowed by that rule). They are avoidable only by fewer guest interrupt cycles (fewer edges). With ptnsi's rule "a raise merged into a pending one cannot lose a wake" (CODE READ ptnsi.rs), cycles 2-3 (~120-160 us of the 234) are candidates for merging.

## Is the worker0/worker1 mutex on the critical path?
CODE READ + MEASURED. `ChanPlane::user_ring_peek` takes the global `user_rings` Mutex with a blocking `lock()` and then does `try_lock`/USERD reads on every relay while holding it; both workers run it in their spin loop (SPIN=64) and `user_ring_record` takes the same Mutex on every `serve`. MEASURED (r3_lock, probe-inflated): 42 534 of 56 265 contended `lock_contended` calls come from `RelayLoop::peek+0x9a`, 13 690 from `user_ring_record+0x15b`; per submit both workers spend 677 us in `peek` (107 calls) and 225 us in `serve` (65 calls); r2_sys shows a FUTEX_WAIT/WAKE hand-off every ~5.4 us between the two workers for hundreds of us after each doorbell. Held time is short but the two spinning workers keep it hot, so the relay's 57 serial turns run at one hand-off per turn instead of ~2.3 us per entry (estimate from a 2-point probe-count extrapolation, INFERRED).
Verdict for this probe: **on the relay's path (relay done at ~165 us), but NOT on the critical path of the 840 us**, because the GPU leg (390 us) starts after it and dominates. It is a real CPU cost (the Steel Nomad profile: workers 19.7% each, peek 6.2%, contended lock 2.5%) and could matter when the relay is the pacing item (many small dependent submits under load) - not shown here.

## Ranked fixes (none implemented; each with expected gain and falsifier)
1. **Find and remove the GR-engine GPU leg (390 us direct, 1213 us compute).** Candidates, cheapest first: (a) put a guest queue's twin channels into ONE host TSG (as the guest has them) or change the host timeslice for the twin TSGs; (b) ring the host doorbells of all channels of a submit together after the relay finished (no skew between channels). Expected gain: up to ~390 of 834 us (47%) on DIRECT, up to ~1200 of 1485 us on COMPUTE; this is the 88%-of-Steel-Nomad's-gap term if the fixed 1.4 ms per dependent step is this leg (INFERRED). Falsifier: with (a) or (b) in a diag build (default-OFF `KF3_DIAG_*`, never merged) the HG leg stays >300 us. Also record the GPU's own timeline (the relay already reads private GP_GET/REFERENCE; log them with timestamps behind a diag flag) to see which entry stalls.
2. **Coalesce the guest interrupt cycles (3 -> 1-2 per submit).** Expected gain ~120-160 us (14-19%) on DIRECT, ~70 us on COPY. Existing knob to try first without code: `KF3_PT_NSI_MIN_INTERVAL_US` (default 0 = off). Falsifier: guest A2 median unchanged, or a missed wake (hang) in the 2000-submit loop.
3. **Cheaper trapped writes** (9-10 us each; 22 + 3 per submit = ~250 us of vCPU time per submit): not a kayfabe code issue alone (KVM exit + userspace dispatch ~3.7 us to handler entry); any change must keep the synchronous-apply rule. Expected gain: proportional to the exit cost; falsifier: kvm_mmio->next-write spacing unchanged after a change.
4. **Remove the worker spin/peek mutex convoy**: take a snapshot of the waiting relays without the global lock (atomics per record, or only one worker spins), `user_ring_record` lookup without blocking. Expected gain: relay done earlier (~165 -> ~100 us?), no gain on this probe's critical path, saves CPU (up to ~2 x 20% of cores in Steel Nomad). Falsifier: last-marker time (`r5_mk` method) unchanged after the fix.

## Not measured / limits
* The cause inside the 390 us GPU leg (needs a diag build or GPU-side timestamps; the earlier agent's GPU timestamp probe removes the device on kayfabe).
* No native VFIO per-hop trace (not requested): native hop timing is only the guest total (61 us). Whether native also issues 3 doorbells / 57 entries per submit is unknown.
* Who writes the completion cell (guest DPC vs GPU) is inferred from its timing (18 us after the last ISR write), not observed.
* Probe cost: uprobe profiles add 3-15% (`mk` +3%, `pub`/`lock` +15%); hop values from the light profile (`hostpath`, kernel tracepoints + 10 uprobe hits) are within the noise of the unperturbed guest median (834-849 vs 805-846).
* cuda_lat during the VM loop (r12) did not overlap and was dropped.
* Steel Nomad itself was not run; the link from this leg to its 1.4 ms dependent step is INFERRED from the earlier fit (fixed ~1.4 ms; here DIRECT 0.84 and COMPUTE 1.49).
* 25.7 vs 25: the earlier count came from counters; the kvm_mmio census here gives 25 (3 + 22) on DIRECT.

## Files
`scripts/bench/submit_latency/`: `sl_probe.ps1` (guest probe, `-Seq` batches several queue types in one process), `sl_it.ps1` (hidden one-shot launcher), `run_trace.sh` (one traced run: bpftrace + fencepoll + guest), `gen_bt.py` (bpftrace programs: `tp` `mk` `pub` `lock` `sys` `irq` `hostpath` `gpuprog` ...), `hops.py`, `fence_vs.py`, `lock_stats.py`, `fencepoll.c`, `cuda_lat.c`, `iter_stats.py`, `align.py`, `tl.py`, `cuda_during.py`.
