# hitch-20261017 — Heaven fullscreen on Windows 11 under kayfabe: evidence summary

**STATUS: LIVE (evidence note), 2026-10-18.** Text tables and six PNGs; no secrets. Raw evidence stays on the bench host under
`/data/claude-hitch/ev/` (not in git): per-boot `arm/*.dispts.txt` (epoch-stamped kf3 counter lines), `heaven/*/guest/hv_*.log` (Heaven result lines and
the DXGI Present trace), `arm/*.broker.log` (broker stats lines), `qemulog/*.qemu.log.gz`, `arm/S2.allthreads.txt` (startup stack sample), and
**all 26 plots** under `/data/claude-hitch/ev/plots/` (graph `_A` = Heaven fps / kayfabe counters / broker counters / max gap per second; graph `_B` =
cumulative frames per stage and the gaps between stages). Six of them are copied under `plots/` here.

Labels: **MEASURED** = taken from those files; **CODE READ**; **INFERRED** = never written as evidence.

## What was run (the claim's n)

Windows 11 guest, RTX 4070 host, Unigine Heaven 4.0 D3D11, Ultra, 1920x1080, **fullscreen exclusive, vsync on**, F9 in-app benchmark (260 s), one
fresh boot per row (except H1/H2, two passes per boot), the owner's production flag set, own overlay (run id 3081), DXGI ETW present trace on in every arm.
One further arm (H6, `KF3_DISPLAY_SDR_COLOR` dropped) ran Heaven composed by DWM at 190 fps — a different mode, excluded everywhere.

* **Baseline**: candidate-4 `0452613b` and the meters build `54266072`: **9 boots, 11 passes**, Heaven average **51.14-51.78 fps** (mean 51.46). Rows: H1 p1/p2,
  H2 p1/p2, H3 (display write trace on), H4 (free clock: x11 broker, no tick source), H5, A2, A3, D1, D3.
* **Fix** (the colour path completes flips by the mailbox rule): **5 boots**, average **56.92-57.56 fps** (mean 57.13): B1, B2, B3 on `6c2c7b96`
  (= `0ea257193`'s code before the review round; ref `hitch-ab-6c2c7b96` on the bench host), C1, C3 on `388f3289` (+ ring + broker line). The first fix boot (B1)
  alone was 57.56; it was compared with five baseline boots at 51.3-51.6 at that moment, the table below is the complete set.
* Average 51.5 -> 57.1 fps (+5.7). **Stalls >= 100 ms and the 1% low did NOT improve beyond the spread** (below).

## Per-boot table (Heaven's own result; dip windows = 2 s kayfabe windows with tear < 46)

```
arm                         avg    min    max | dip-windows(tear<46) dipsec | copies>tick+1 | ETW p99 interval ms / max ms
H1 p1 (cold, 0452613b)    51.49  29.52  69.04 |   49    98 s          |   0           |
H1 p2 (warm)              51.33  22.38  68.37 |   50   100 s          |   0           |
H2 p1 (cold)              51.37  28.08  67.28 |   52   104 s          |   0           | 46.9 / 173.0
H2 p2 (warm)              51.40  29.15  68.03 |   51   102 s          |   0           | 46.9 / 165.2
H3 (WTRACE on)            51.39  30.09  67.40 |   47    94 s          |   0           | 46.7 / 182.2
H4 (free clock, x11)      51.33  23.77  69.92 |   50   100 s          |   0           | 47.1 / 227.9
H5 (meters 54266072)      51.58  27.79  70.39 |   49    98 s          |   0           | 47.0 / 166.0
A2 (54266072)             51.14  23.43  68.00 |   52   104 s          |   0           | 47.1 / 229.6
A3 (54266072)             51.44  29.12  64.95 |   47    94 s          |   0           | 47.1 / 160.2
B1 FIX 6c2c7b96           57.56  32.07  70.64 |    0     0 s          |   0           | 45.7 / 147.8
B2 FIX                    56.99  31.23  68.19 |    6    12 s          |   0           | 46.0 / 125.4
B3 FIX                    57.03  28.95  70.68 |    3     6 s          |   0           | 46.0 / 123.3
D1                        51.50  29.72  68.42 |   50   100 s          |   0           | 47.1 / 168.1
D3                        51.78  29.30  67.63 |   47    94 s          |   0           | 47.0 / 164.4
C1                        56.92  27.52  69.82 |    6    12 s          |   0           | 46.3 / 150.5
C3                        57.15  33.80  70.06 |    3     6 s          |   0           | 45.9 / 121.6
```
(columns: Heaven avg / min / max fps | 2 s windows with tear < 46 and their seconds | windows with copies > tick + 1 | ETW p99 per-present gap ms / max gap ms; H1 has no ETW trace.)

## Headline metrics by arm (final_table.txt; mean [min..max])

| metric | baseline (7 passes with an ETW trace) | fix (5 boots) |
|---|---|---|
| (e) Heaven average fps | 51.46 [51.14..51.78] | 57.13 [56.92..57.56] |
| Heaven min fps | 28.1 [23.4..29.7] | 30.7 [27.5..33.8] |
| (a) guest presents/s (ETW) | 51.47 | 57.16 |
| (d) repeated presents/s (ticks - copies, derived) | 8.5 | 2.8 |
| (b) 1% low fps (1000 / p99 of per-present gap, guest ETW) | 21.27 [21.21..21.32] | 21.75 [21.61..21.88] |
| 0.1% low fps | 15.9 [14.4..18.4] | 16.8 [15.0..18.2] |
| (c) gaps > 33 ms per minute (guest) | 513 [496..534] | 177 [152..191] |
| gaps > 50 ms per minute | 6.4 | 6.0 |
| **gaps >= 100 ms per minute** | **0.56 [0.23..0.92]** | **0.51 [0.23..0.69]** |
| max gap (ms) | 175 [160..230] | 134 [122..151] |

Broker view (stats line v2, one boot each; D3 baseline, C3 fix): new-content presents 51.8 -> 57.1 per second, repeats 8.2 -> 2.8, commits merged 0 in both;
new-content gaps > 33 ms 247 -> 87 per minute, > 100 ms 0.23 -> 0 per minute, max new-content gap 116.7 -> 66.7 ms. One boot per arm is not beyond a spread.

Verdict by the owner's criterion (gaps >= 100 ms and the 1% low must improve beyond the spread): **not shown**. What did improve beyond the spread:
the average, the 33 ms-class gaps and the repeated presents. The >= 100 ms events sit at the same benchmark seconds (about 169 s and 180 s, plus a loading
hitch near the start) in every boot of every build, cold or warm; in the traced run (H3) the 169 s gap already exists between the guest's flip
submissions (128 ms) and the kayfabe latches follow it (119 ms): it is born in the guest application. Whether native Heaven has it is **not measured**
(the 4070 also drives the owner's desktop; detaching it for a VFIO run was not possible without ending that session).

Frame accounting (MEASURED, all ten ETW arms checked): guest presents, flips latched, copies and frames sent agree within 1% over the pass. No frame is
lost between stages; `skipped` stayed 0.

## The mechanism (MEASURED / CODE READ / INFERRED)

* MEASURED, baseline: 40-42 fps stretches at the same benchmark seconds in every pass and boot; per-frame work (traps, interrupts, UPDATE methods) identical in
  dip and steady seconds; GPU 22-30% busy, no CPU thread saturated; correlation of fps with mapping activity -0.04..+0.09; the same profile on the free clock (no
  broker tick, no lock step); ETW: Present() blocks about 42 ms in a repeating two-frames-per-three-vblanks cycle; `checks` = ticks without a new flip (r = -1.00 with tear).
* CODE READ: with `KF3_DISPLAY_SDR_COLOR=1` (production flag) the worker set `barrier = started + 1` and `want = true` at every window latch: the flip waited for
  its own copy (launch 1.0-1.5 ms in the display worker, `lat[start]`) before the completion and the vblank edge reached the guest, and the copy was wanted outside the
  mailbox. The `V3_DISPLAY_PACING` section 6 rule never applied in production.
* INFERRED: that extra latency pushes the guest's present loop across a vblank cliff. The A/B supports it; it does not isolate it.
* The fix (`7ab2aeb88`): one rule for every window flip; the colour bindings resolved at the latch.

## Startup request timeouts (symptom A) and tick flapping (symptom C)

* MEASURED: the broker's 40-46 request timeouts per session all happen in the first 5 seconds after attach (broker lines: seconds -138..-134 of the first boot; none in any
  benchmark window). The lock-step event ring (`f89c5a50a`) shows the relay received no ticks for 4.27 s, then a burst of 14 ticks within 30 us and a replaced request.
  A gdb sample at +5 s: the QEMU main thread waits for the BQL in `main_loop_wait`; `kf3-vamgr` is inside the RM `alloc_os_descriptor` ioctl called from
  `tspace::prewarm` (guest RAM); another QEMU thread sits in `mprotect` inside `malloc`.
* INFERRED, not proven: the ioctl holds the process memory-map lock; the malloc waits on it while holding the BQL; the main loop (which owns the broker relay) cannot read
  the socket for ~4 s. Not fixed here (outside display.rs; candidates: prewarm before the broker attaches, or in chunks).
* CODE READ: the broker's tick pump re-arms after a discarded feedback only through the 250 ms watchdog, while kayfabe falls back to the free clock after 50 ms of silence. In
  my runs the fallbacks happened at boot and on an idle desktop, never during the benchmark. Not fixed.

## Not done / limits

No native Heaven baseline; no compositor control client; no kayfabe per-event gap tracker (kayfabe stage = per-window counters and the one traced run); the broker `loop_*` and
latency fields of the first stats binary (D1, C1) come from a possibly sanitizer-built binary; D3's refresh-gap field used the pre-fix measurement (its new-content fields are valid);
the commit-to-present latency is meaningless (the compositor discards every COMMIT's feedback). Carrier (dma-buf / F_SHM) switches are not in the stats line.
