# INTERIM (2026-10-18, ~20:15 CEST) — where the ~0.84 ms of one tiny GPU packet goes under kayfabe

STATUS: RESEARCH, interim. Branch `claude/submit-latency-20261018` (off `claude/steelnomad-gap-20261018` 4f37bd12). Raw data: host `/data/claude-submitlat-20261018/` (not in git).
Subject: owner VM run 3006, binary kf3-bins/0452613b, GPU RTX 4070, guest D3D12 probe (`scripts/bench/submit_latency/sl_probe.ps1`, DIRECT queue, 64 KiB copy + Signal, polled fence).
Labels: MEASURED = a run named here; CODE READ = source at 0452613b (`git show 0452613b:...`); INFERRED = reasoning, not evidence.
One clock: host CLOCK_MONOTONIC (bpftrace `nsecs`, `fencepoll`, `cuda_lat`). Guest QPC is NOT used for host-side hop values.

## (a) Runs so far (all against the running VM 3006; no flag, no restart, no guest setting changed)
| run | what it probes | n | usable? |
|---|---|---|---|
| mmio1 / up1 / up2 (root of data dir) | first look: kvm_mmio exits; up1 = 41 uprobes (too hot, 3.4 M events lost); up2 = 12 uprobe pairs | 2000/1000/600 submits | mmio1 yes (offset census); up1 NO; up2 structure only (probe cost +20% per submit) |
| r0_fence | no tracing, fencepoll only: guest baseline + fence word scan | 3000 | yes (baseline 805-846 us) |
| r1_tp (first try) | full tp set | 3000 | NO (1.0 M events lost; profile too heavy, rewritten) |
| r1_tp, r1b_tp, r1c_tp | light tracepoints (kvm_mmio, wakeups of kf3 threads, eventfd writes) + fencepoll | 3000/2000/2000 | yes; guest 849/845/847 us (unperturbed within noise) |
| r2_sys | + every syscall of the two workers | 400 | yes (futex / epoll pattern); guest 828 |
| r3_lock | + uprobes on RelayLoop::peek/waiting, user_ring_record, serve, Mutex::lock_contended (+caller) | 600 | yes, but perturbed (guest 971 us, +15%) |
| r4_pub | + entry-only uprobes serve/pub_entry/pub_marker/wr_guest_get | 3000 | partly: counts yes; times perturbed (guest 960) |
| r5_mk | + entry-only uprobes on pub_marker, wr_guest_get (about 10 hits per submit) | 3000 | yes, least-perturbed relay timestamps (guest 877, +3%) |
| r6_irq, r9_dir | + host IRQ 85 (nvidia) entry, wakeups of nvidia threads, + mk probes | 2000/3000 | yes; r9_dir guest 844 (best hop table) |
| r7_copyq, r8_*, r9_copy | other queue types | 2000 each | guest numbers yes; host hop table for copy queue needs a different window filter (hops.py assumed 600-1500 us period) |
| r10_gg, r11_gpuprog | relay's own USERD reads (GP_GET / tagged REFERENCE) per private channel | 500/800 | r11 yes for ORDER of GPU progress (uretprobe per peek read, times +20%) |
| cuda_lat (host-native) | host CUDA GR launch+sync / CE copy+sync latency, beside the idle VM | 2000 | yes |
| r12_cudaduring | host CUDA latency DURING the VM loop | - | NO: cuda_lat did not overlap the VM loop (its samples started 46 s late); to be redone or dropped |

## (b) Per-hop numbers, DIRECT queue, 64 KiB copy + Signal, polled (MEASURED, r9_dir unless noted; us from D0 = first doorbell trap; n = submits)
Guest, no tracing, n=2995: submit-return -> fence visible median 805-846 (p95 965-1009, p99 1080-1215, max 54 ms outlier). ExecuteCommandLists 17 us, Signal 2.5 us.
| hop | median | p90 | max | n |
|---|---|---|---|---|
| H1 D0 -> last of 3 doorbell traps (3 x 0xbb0090, tokens 0x3b/0x3d/0x3c, ~9 us per trap) | 32.4 | 37.5 | 545 | 3181 |
| H2 D0 -> first kf3-worker wake (trap -> eventfd/wake -> IPI -> thread runs) | 9.8 | 11.5 | 90 | 3181 |
| H2b first wake -> first `serve()` entry (r1/up2, probe-perturbed) | ~4 | 6 | 38 | 941 |
| H3 relay: 57 GP entries published per submit (2.3 us each) ; last batch marker published at | 150.4 | 247.6 | 1084 | 3053 (r9) ; 146.6 in r5_mk |
| (guest GP_GET write-backs per submit 6; markers 3; trapped writes per submit 25 = 3 doorbells + 22 interrupt-tree) | | | | |
| HG last marker published -> first GPU interrupt at the HOST (irq 85 `nvidia`) = "GPU leg" | 404.3 | 476.6 | 704 | 3053 |
|    first host GPU interrupt, rel D0 (3 per submit; last at 681) | 551.8 | 556.5 | 914 | 3181 |
| HN host GPU interrupt -> first eventfd write to the guest irqfd (host RM + worker) | 11.8 | 16.4 | 48 | 3181 |
| H5 irqfd write -> first guest ISR trap (injection + vector entry) | 16.9 | 20.9 | 107 | 3181 |
| H6 first -> last interrupt-tree trap (3 ISR/DPC cycles x 6-8 trapped writes at ~9-10 us) | 238.4 | 334 | 1319 | 3181 |
| H7 last interrupt trap -> completion cell written (r1b/r1c: 17 us; r9: 18.3 us) | 18.3 | 20.9 | 59 | 2940 |
| H8 completion cell -> next submit's first doorbell (guest sees fence, Execute, doorbell) | 21.9 | 46.5 | 175 | 2940 |
| period D0 -> next D0 | 868.6 | 955 | 1454 | 3181 |

Which registers (MEASURED, kvm_mmio, mmio1/r9): per submit 25 trapped BAR0 writes = 3 x 0xBB0090 (NV_USERMODE_NOTIFY_CHANNEL_PENDING doorbells) + 22 on the CPU interrupt tree 0xB81xxx:
per ISR/DPC cycle LEAF_EN_CLEAR(2)=0xB81408 (0x101), LEAF_EN_CLEAR(4)=0xB81410 (0xFFFFFFFF), TOP_EN_CLEAR=0xB81610 (8), LEAF_EN_SET(2)=0xB81208, LEAF_EN_SET(4)=0xB81210 (0x0C10007A), TOP_EN_SET=0xB81608 (0xFFFFFFFE), then LEAF(0) W1C 0xB81000 = 1, 2. Names from `kf-trap::cpuintr` offsets. 3 cycles per submit (EN_CLEAR(2) count), ~7.3 writes each.
Per trapped write: kvm_mmio -> Rust `Device::bar0_write` entry about 3.7 us, handler 1.7 us (uprobe-inflated), consecutive guest writes 8.8-10 us apart.

Other MEASURED facts
* Queue type matters (guest, polled, 64 KiB copy + Signal, n=1995 each): DIRECT 846 us, COPY queue 223 us, COMPUTE queue 1495 us; DIRECT with an empty list 1.7 us (no submission at all). First host GPU interrupt after D0: DIRECT 552 us, COPY 50 us.
* Host-native (no VM involvement) CUDA GR empty-kernel launch+sync: median 3.9 us (idle) / 4.8 us (800 us gaps); CE 64 KiB D2D copy+sync 4.6 / 5.3 us. So the idle host GPU wakes in microseconds; 400 us is not a property of the GPU in isolation.
* Worker threads: both spin in epoll_wait(0) and `RelayLoop::peek`; r2_sys shows a futex FUTEX_WAIT/FUTEX_WAKE hand-off on ONE mutex address every ~5.4 us between worker0 and worker1 for hundreds of us after each doorbell.
  r3_lock: of 56 265 `Mutex::lock_contended` calls 42 534 come from `RelayLoop::peek+0x9a` (the `self.user_rings.lock()` at the top of `user_ring_peek`, CODE READ userpump.rs:`user_ring_peek`) and 13 690 from `ChanPlane::user_ring_record+0x15b`; per submit (both workers) peek ran 107 times for 677 us, serve 65 times for 225 us, user_ring_record 195 times, lock_contended 78 times for 152 us. Uprobe cost is included in these durations (about 1 us per probe pair).
* Fence words (fencepoll): two kinds of guest-RAM cell hold the fence value: one written by the guest CPU about 3 us BEFORE the next doorbell (submit-side), one written 17-18 us AFTER the last interrupt-tree trap and ~22-25 us before the next doorbell (INFERRED: the DPC's completion write that the polling thread sees). So the guest sees completion only after the whole ISR/DPC chain; the interrupts are on the critical path of this polled test.
* GPU progress order (r11_gpuprog, relay's own USERD reads): the three private channels reach their batch markers one after another at ~554, ~623, ~678 us after D0, not in parallel.

## (c) Current attribution of the guest-measured ~840 us (medians, host clock, D0 = 0; guest Execute-to-doorbell ~25 us comes before D0 and is in the guest's 17-25 us ExecuteCommandLists)
| span | us | share | what | label |
|---|---|---|---|---|
| D0 -> last doorbell | 32 | 4% | 3 trapped doorbell writes | MEASURED |
| doorbell -> relay done (last marker published) | ~115 beyond H1 (done at ~150) | 14% | wake 10 us + 57 serial turns on two workers with a futex-convoyed mutex; overlaps the GPU | MEASURED (end time), cause CODE READ + MEASURED (r2/r3) |
| last marker -> first GPU interrupt | 404 | 48% | GPU/host-GPU leg: private channels execute the relayed work and reach their markers in series; NOT the relay threads (they are idle/spinning) and NOT an inherent host GPU wake latency (cuda_lat 4 us) | MEASURED duration; cause NOT established |
| GPU interrupt -> guest ISR starts | 29 | 3% | host RM + worker (12) + injection (17) | MEASURED |
| ISR/DPC chain, 3 cycles, 22 trapped writes | 238 | 28% | guest Windows ISR/DPC mask/unmask, every write a ~9-10 us userspace exit | MEASURED |
| last trap -> completion cell -> next doorbell | 18 + 22 | 5% | DPC completion write, poller, next Execute | MEASURED |
Sum of medians = ~870 us vs the guest's 805-846 (+ Execute ~17 us) — closes within the median/p90 spread. Unaccounted: the cause inside the 404 us GPU leg (the largest single item).

Answers so far
* Mutex worker0/worker1: ON the relay's critical path (relay done at ~150 us instead of ~2.3 us x 57 spread over two workers is unproven) but the relay finishes 400 us BEFORE the first GPU interrupt, so it is not the dominant term for this probe. It is a real CPU burn (19.7% each in Steel Nomad) and a candidate for the loaded case. INFERRED/open.
* 25.7 trapped writes: 3 doorbells (needed by design) + 22 interrupt-tree writes of 3 guest ISR/DPC cycles. The doorbells are inherent; the 22 are Windows' own mask/unmask around each host interrupt edge; avoidable only by delivering fewer edges (6 irqfd writes -> 3 cycles per submit) or by not trapping those registers (conflicts with "only BAR0 writes trap" synchronous-apply rule, CODE READ cpuintr.rs).

## (d) Still to run (aim: finish and restore the VM within ~45 min of the coordinator's request)
1. ONE batched guest launch (hidden window) running DIRECT / COPY / COMPUTE and an event-wait variant back to back with host `hostpath` tracing + fencepoll: gives n>=2000 per queue type and a copy-queue hop table. ~6 min incl. setup. KEEP.
2. Probe-cost calibration for the hop-table numbers (guest median with/without each profile): already available from the runs above (849 vs 805-846 unperturbed; +3% mk; +15% pub/lock). DONE, only to be written up.
3. GPU-leg decomposition (why 404 us): candidate causes (a) skew between channel batches makes PBDMA semaphore acquires wait, (b) GR TSG switch between the three twin TSGs, (c) PCIe/IOMMU fetch of pushbuffers. Cheapest discriminator with no rebuild: the batch above plus the existing r11 order data. A rebuild with a diag flag is possible but exceeds the 45 min budget: DROP, list as the first fix experiment.
4. r12 (host CUDA during the VM loop): redo only if time remains (2 min); else drop.
5. Write-up, README, push, delete worktree, restart VM as 3007. ~15 min. KEEP.
