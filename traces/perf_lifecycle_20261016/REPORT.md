# Windows USER-ring lifecycle measurements: REPORT (2026-10-16)

**STATUS: RESEARCH, 2026-10-16.** Text produced by the measurement agent (Sonnet 5.5) and committed by the coordinator because the agent's
Write tool refused to create the file. Pre-registration: `PREREG.md` (commit 2b679fb48, before any run, predictions not edited afterwards).
M = measured, I = inferred. Base binary: 0a746117 (Codex-verified gates 9/9, fast 30/30). Native numbers (D3D11 330, OpenGL 342 fps) are quoted from the
earlier campaign, not re-measured. All scratch binaries are NEVER mergeable.

## Revisions run
0a746117 base (baseline reproduction); 8c6ab6d1 counters (S1); faf7b94b first scan-off build (S3 mode 1, killed the guest); 28a0ee9c main S3 build
(modes 1 and 2 with counters); db8817ac adds mode 3 (reads the pushbuffer, then drops it unscanned); 1c0bb713 adds the MOVNTDQA experiment (scan stays on).
Scan-off modes: mode 1 = off after 5080, no registration window; mode 2 = off after 5080 but SCAN WHILE a FIFO_UPDATE_CHANNEL_INFO registration is pending
(the HB numbers); mode 3 = read then drop.

## Method
Baseline reproduced: D3D11 101.67 / 101.47, OpenGL 231.82; same binary with the new env unset: 99.78 / 233.19. S3: 3 interleaved runs per arm, one fresh boot per
arm, 260 s Heaven each, score from Heaven's log. Counters cost ~4-6% fps and slightly favour scan-off (the control carries more of it); a no-counter pair removes that bias.
Per-run asserts (guest driver 580.88, host 595.91.07, kf3 binary, KF3_* from /proc/<qemu pid>/environ) are in each run's assert.txt.

## Verdicts
| id | verdict | evidence |
|---|---|---|
| HA | held (M) | 100.0% of BAR1 pushbuffer copies come from channels past 5080 with no registration pending: 22,733,982 copies (946/frame) in S1, 23,450,428 (934/frame) on 28a0ee9c; pre-5080 and registration-pending: 0 |
| HB | held for both APIs WITH the registration window scanned; not viable without it (M) | D3D11 +97.3% (median 95.68 -> 188.75; spreads 9.9%, 1.4%); OpenGL +56.1% (216.72 -> 338.22; spreads 6.2%, 10.3%). No-counter pair (n=1): D3D11 99.78 -> 193.81 (+94.2%), OpenGL 233.19 -> 341.75 (+46.5%). Mode 1 killed the guest at the first ring growth: host Xid 32 (PBDMA INTR_0 0x00800000), 3 guest nvlddmkm 153 |
| HC | held by its falsifier; sub-ranges missed (M) | idle GPU (P8, PCIe gen1): 256 B p50 8.72 us, 30 MB/s, 2 threads x1.77. Loaded GPU (P0, gen4): 256 B p50 2.54 us, 93 MB/s, 2 threads x1.99. The BAR1 view is WRITE-COMBINING in the PAT list although kayfabe requests CachePolicy::Uncached (all 376 BAR1 ranges WC; only the 6 BAR0 ranges uncached-minus) |
| HD | held for copy-engine copies; falsified for a kernel gather (M) | idle p50 4.8 us; with Heaven loading the GPU, 256 B copies at 1,000/5,000/20,000 per s: p50 6.4/5.5/5.2 us, fps loss 0.2/0.4/0.7%. Kernel gather at 5,000/s: p50 612 us, -10.5% fps. cuMemcpyBatchAsync_v2 of 128 x 256 B: 36 us |
| HE | held (M) | SYNC_WAIT = 0.63% of GP entries in D3D11 (all length 0), 1.28% in OpenGL (106,759 length 0, 547,123 length > 0) |

## D3D11 frame, scan on vs scan off (mode 2, counters on)
| | scan on (28a0ee9c ctl 1, 100.4 fps) | scan off (188.8 fps median) |
|---|---|---|
| frame time | 9.96 ms | 5.29 ms |
| GP-entry snapshots / frame | 1,722 | 1,700 |
| pushbuffer snapshots / frame | 1,651 | 0 (1,638 skipped) |
| BAR1 copies / frame | 934 (247 KB, 96-416 B) | 0 |
| BAR1 copy time / frame | 4.00 ms | 0 |
| total snapshot time / frame | ~4.7 ms | ~0.35 ms |
| GPU utilisation | 74-78% | 93-95% |
The frame got 4.67 ms shorter and the reads cost ~4.7 ms: the reads sit serially on the critical path (I).
Heaven produces exactly one registration trigger per session (a ring growth to 65,536 entries); mode 2 scans 7,372 pushbuffer reads during it. In the 260 s
windows: 0 triggers of any kind, 0 rare segments. In mode 2 the relay waits for GPU credit 305,820 times per session (control: 0) and sends 323,702 SYNC_WAIT
entries ungated because its parser state is stale (control: 0).

## Read cost vs scan cost (mode 3: read, then drop unscanned)
D3D11 stays at 103.25 fps (n=2) vs controls 95.7-100.0: D3D11's cost is the BAR1 READS. OpenGL reaches 329.0 fps (n=2) vs controls 216.7-228.1: OpenGL's cost is the
SCAN (CPU parse and allocation). OpenGL pushbuffers are in guest RAM: 3.3 BAR1 copies/frame vs 934 for D3D11.

## Faster BAR1 reads with the scan still on (MOVNTDQA)
Standalone probe, loaded GPU (P0), p50: 256 B 0.66 us MOVNTDQA vs 2.54 us memcpy; 416 B 0.74 vs 6.89; 4 KB 3.85 vs 41.6. Wired into the real BAR1 copy
(KF3_BAR1_STREAM_LOAD=1): BAR1 copy time/frame 4.0 -> 0.82 ms; D3D11 97.6 (same-binary control) -> 133.3 / 138.6 fps (+39%); OpenGL unchanged (212.0, 222.3);
0 scan refusals. Not checked: MOVNTDQA can serve a stale cached line; not tested against concurrent GPU writes. Does not satisfy the owner rule (no bulk MMIO reads).

## Guest sanity under mode 2
Heaven D3D11 completed at 193.0 fps; mid-run screenshot mean luma 153, std 85.7, 231 colours; no Xid, no nvlddmkm/Display events; the only 153 events are the benign
Kernel-Boot ones at boot. Edge launched 60 s: process-alive check inconclusive (the launcher exits on hand-off).

## Where the remaining gap is
OpenGL: nothing left (338 fps, 361.8 best, 341.7 without counters vs native 342). D3D11 after scan-off: 189 vs 330 fps with the GPU 93-95% busy, so not relay
latency; energy per frame similar to native (0.46 J vs 0.55 J; native and idle watts quoted from the earlier campaign). Open list is GPU-side: 4 KiB PTEs, no compression,
the P2 memory clock (I).

## Ranked next steps
1. Implement the lifecycle (scan off after 5080, scan while a registration is pending, forward GP entries as pointers). Measured upper bound +97% D3D11, +56% OpenGL.
   Open risk: a software-subchannel method outside the registration window reaches the hardware and resets the guest's channel (Xid 32 plus guest TDR); Heaven only
   shows the ring-growth trigger, other apps' TLB-invalidate and context triggers were not measured.
2. Short-entry scan as the safety net (the captured trigger is a separate 16-byte segment; reads <= 32 B were only 231 per 250 s D3D11 window; not run).
3. Faster BAR1 reads wherever scanning remains (MOVNTDQA, +39% D3D11 measured, coherence unchecked, WC mappings only).
4. If scanned bytes must come from VRAM, cuMemcpyBatchAsync (0.28 us per copy at batch 128). The store is already imported into kayfabe's CUDA context at every
   boot ("store 8192 MiB (imported into the walker)"), so the CUDA-reaches-the-store question is answered. A kernel gather is excluded.
5. OpenGL scan cost: reduce per-frame word and patched-vector allocation and the scan itself if scanning stays for OpenGL.

## Not done
No native VFIO re-measurement, no PresentMon (as instructed). S2's "native graphics load" was Unigine Heaven OpenGL via PRIME offload on the host (321.7 fps alone), not D3D11.
S2 idle numbers were taken at P8/gen1; kayfabe runs at P0/gen4 so the P8 numbers are context only. Mode-1 INTR bit 0x00800000 = DEVICE interrupt is from memory and a chan.rs
comment, not verified in headers (I). No corpus run for other trigger kinds; frame correctness checked only by screenshot statistics and event logs; no 60-minute soak.
`Device::stop` is not reached on guest power-off, so the final counter dump was added to the last-relay-gone path. The agent stopped three of its own display-broker
processes that stopk.sh had left on the owner's desktop. Host restored: no QEMU of the agent's running, GPU on nvidia, lock free, NVRM Xid count 81 (one Xid 32 from mode 1);
overlays, targets, downloads and the 2993 run directory deleted (about 12 MB of scripts and logs remain under /data/claude-perf2).
