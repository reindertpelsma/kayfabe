# PREREG: are Windows USER-ring pushbuffer reads the D3D11 gap? (2026-10-16)

STATUS: RESEARCH, 2026-10-16. Written BEFORE any run of this campaign (no Windows or GPU run, no counter build, no probe had
been executed when this file was committed). Predictions below are not edited afterwards; anything learned later goes into
REPORT.md as a dated addendum. Base revision: `0a7461173` (`origin/claude/perf-fix-20261015`). Host: root@172.22.1.20
(RTX 4070 AD104, host driver 595.91.07). Windows guest driver 580.88. Scratch branch `claude/perf-lifecycle-measure-20261016`:
nothing in it is for merge.

Labels used in the report: MEASURED = a number from a run named in the report; INFERRED = reasoning, never evidence.

## What was read before writing this (code, not runs)

- `userpump.rs::turn_body`: for every GP entry the relay reads the 8-byte entry (`snapshot(.., va, 8)`), then reads the whole
  segment pushbuffer (`snapshot(.., d.gpu_va, d.len_bytes)`) and scans it (`stage_plan`). There is NO mode switch after a 5080
  SET_OBJECT in the code: every USER-channel segment is read and scanned for life. A non-trigger ("not rare") segment is then
  forwarded as the guest's own GP entry (pointer, `write_user_gp`); only a trigger segment is staged (copied).
- The scanner (`kf_chan::user_scan::Scan`) keeps `bindings[8]` (last SET_OBJECT data per subchannel). "The channel has seen its
  5080 SET_OBJECT" is defined here as: some subchannel >= 5 binding satisfies `Software::matches` (value == 0x5080 class or the
  host software number), evaluated on the committed scan state (state after the previously published segment).
- "FIFO_UPDATE_CHANNEL_INFO registration pending" is defined here as: the defapi registry holds, for the channel's 5080 object,
  an entry decoded as `Bundle::UpdateChannelInfo` that has not been executed yet.
- GP_ENTRY1 SYNC_WAIT is bit 63 of the 64-bit GP entry in the code (`entry >> 63`).

## Hypotheses (prediction, FALSIFIER, step)

| id | prediction | FALSIFIER | step |
|---|---|---|---|
| HA (attribution) | >= 80% of the per-frame BAR1 (store/vidmem) pushbuffer copies come from channels that have already seen their 5080 SET_OBJECT and have no registration pending. | FALSIFIED if < 50%. (Unit: BAR1 copy count of the pushbuffer site, Heaven D3D11, steady state; the "867 per frame" of the earlier trace is reproduced only as a cross-check, not as a condition.) | S1 |
| HB (upper bound) | Disabling the pushbuffer scan after 5080 (scratch build) raises Heaven D3D11 median fps by >= 25% over the SAME binary with the scan on. Same question for OpenGL: prediction >= 10%. | D3D11: FALSIFIED if the change is < 10% (then the cost is elsewhere: wake/latency/GPU path). OpenGL: FALSIFIED if < 3%. Medians of 3 interleaved runs per arm; a change smaller than the larger arm spread is "within noise" and counts as below the threshold. | S3 |
| HC (MMIO) | A single-thread UC BAR1 read of 256 B costs 4-8 us and the bulk rate is 30-70 MB/s; two threads on separate cores scale >= 1.6x. | FALSIFIED if the single-thread rate is outside 20-120 MB/s, or two threads scale < 1.2x. | S2 |
| HD (copy engine) | An async device-to-host copy of 256 B-4 KB from a VRAM allocation has p50 latency <= 100 us with the GPU idle and <= 300 us with a graphics load in another process; a native graphics load loses < 5% fps while the copies are issued at 5,000/s. | FALSIFIED if the loaded p50 > 1 ms, or the fps loss is >= 10%. | S2 |
| HE (SYNC_WAIT) | The Windows driver sets GP_ENTRY1 bit 31 (SYNC_WAIT) on < 5% of GP entries in Heaven. | FALSIFIED if >= 20% (then inline copying would leave the fast path too often). Reported split by length 0 / > 0. | S1 |

Additional pre-registered expectations (stated now, not conditions of the hypotheses above):
- E1: the first Windows run of the base build reproduces the known baseline within noise: D3D11 98-113 fps, OpenGL 222-228 fps
  (earlier campaign: 97.65 / 222.5 on this revision). If it does not, no new binary is trusted and the campaign stops at S1.
- E2: the counters build with `KF3_LIFECYCLE_COUNT` unset behaves like the base (same fps within noise).
- E3 (S3): with the scan off after 5080, a registration/trigger (TLB invalidate, FIFO_UPDATE_CHANNEL_INFO ring growth) that
  reaches the hardware unscanned may break the guest (TDR / Xid / event 153). That would be a finding, not a failure of the
  experiment: it is recorded with the number of registrations seen.
- E4 (S2): crossover where a device-to-host copy beats an MMIO read is at a few hundred bytes to ~1 KB if copy latency is ~50-100 us
  and the MMIO rate is ~47 MB/s (INFERRED arithmetic: 47 MB/s x 100 us = 4.7 KB); the pre-registered guess is "crossover in 1-8 KB for
  a single copy per read; far lower for a batched gather".

## Method commitments

- Every Windows run prints and asserts: guest driver (580.88), host driver, kf3 revision (binary path under `kf3-bins/<rev>/`),
  and the KF3_* environment actually in the QEMU process (`/proc/<pid>/environ`). PERF_FLAGS empty unless a step says so.
- One GPU/VM job at a time (`flock -o /tmp/kayfabe-fastguest.lock`). No PresentMon in the guest. Footprint < 40 GB under
  `/data/claude-perf2/`; build targets and overlays deleted at the end; host restored (no QEMU of mine, GPU on nvidia, lock free).
- Heaven: 1920x1080 windowed Ultra, no AA, in-app benchmark (F9), fps from the engine's own `log.html`. S3: 3 runs per arm,
  interleaved on/off/on/off, medians and (max-min)/median spreads, 300 s benchmark each.
- Time-boxes: S1 3 h, S2 3 h, S3 4 h, S4 1 h (wall clock). A step that is not finished at its time-box is stopped and reported.
- A failure that repeats for the same reason is reported, not walked around.
