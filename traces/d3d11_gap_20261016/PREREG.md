# PREREG: where does the remaining Windows Heaven D3D11 gap come from? (2026-10-16)

STATUS: RESEARCH, 2026-10-16. Written and committed BEFORE any run of this campaign (no VM, no GPU job, no new build had
been started when this file was committed). Predictions are not edited afterwards; later learning goes into REPORT.txt.
Scratch branch `claude/d3d11-gap-20261016`, based on `origin/claude/perf-lifecycle-measure-20261016` (tip f8b78fa49). NOTHING in
this branch is mergeable (scratch counters and experiment switches).

Labels: MEASURED = a number from a named run. INFERRED = reasoning, never evidence.

## Starting point (MEASURED by the previous campaign, quoted, re-measured here in G0)
Heaven 4.0 Ultra 1080p windowed, guest driver 580.88, host 595.91.07. Control (scan on) D3D11 ~97 fps, OpenGL ~217 fps.
Scratch build 28a0ee9c with `KF3_SCAN_OFF_AFTER_5080=2` (scan off after 5080, scan while a registration is pending): D3D11 ~189,
OpenGL ~338. Native/VFIO (earlier campaign): D3D11 330, OpenGL 342. So OpenGL reaches native, D3D11 reaches 57%.
With scan off the D3D11 GPU is 93-95% busy at ~122 W (native ~191 W). Mode-2 relay counters (D3D11 session): 305,820 waits for
private-ring GPU credit (control 0), 323,702 SYNC_WAIT entries ungated; ~1,700 GP entries/frame; 0 pushbuffer snapshots.
Because the same GPU, driver, memory path (4 KiB PTEs, compression, memory clock) gives native OpenGL, those are NOT chased here.

## What was read before writing this (code and old logs, no run)
`kf-chan/src/user_storage.rs` (private ring: `ENTRIES = 512`, `BYTES = 1 MiB`, GP array at `0xe0000`, USERD at `0xf0000`),
`user_consumption.rs` (`MAX_ENTRIES = 512`), `user_scan.rs` (`Fetch::new` cap 512), `userstore.rs` (`VIEWS = BYTES/64 KiB`),
`userpump.rs::credit_need` (the credit-wait site), `lifecycle.rs` (existing counters and tick lines). The LIST of counter names that
the existing mode-2 logs carry (`/data/claude-perf2/ev/qemulog/`, e.g. `doorbells_translated`, `relay_stats wakes(...)`,
`display fps[...]`, `display: N updates completed`) was listed to plan G0; no outcome value of D3D11 vs OpenGL was
compared before this file was committed.

## Run protocol (all hypotheses)
Host root@172.22.1.20, startup gate (no foreign QEMU, `flock -n /tmp/kayfabe-fastguest.lock true` twice 10 min apart), then
`flock -o /tmp/kayfabe-fastguest.lock <job>`. Harness: scripts of `origin/claude/perf-win-20261015` (`scripts/bench/windows/perf/`)
plus `scripts/bench/lifecycle/`; per-run asserts (guest driver 580.88, host 595.91.07, kf3 binary, KF3_* from /proc/<qemu pid>/environ).
One fresh boot per arm, D3D11 then OpenGL in the same boot, 260 s Heaven windows, score from Heaven's log, host `nvidia-smi dmon`,
per-thread CPU of the qemu process. Medians of 3 interleaved runs per arm for G1. A difference smaller than the larger arm spread
is "within noise" and counts as below any threshold.
Baseline gate for G0: control D3D11 must land in 90-105 fps and mode-2 D3D11 in 170-205 fps (the old numbers +-10%); if not, STOP
and report before trusting anything new.

## Hypotheses (prediction, FALSIFIER, step)

| id | prediction | FALSIFIER | step |
|---|---|---|---|
| HG1 (private-ring credit waits are on the critical path) | With the private GPFIFO ring enlarged 4x (512 -> 2048 entries; scratch variant of the tip, the size chosen at run time by an env switch, default = today's 512, so the control arm is the SAME binary) Heaven D3D11 in mode 2 rises >= 10% in median fps over the same binary at 512, and `credit_waits` per session drops to ~0 (< 1% of the 512-ring count). | FALSIFIED if the median gain is < 3% (then credit waits are not the limit). 3-10% is "inconclusive/partial" and reported as such. The OpenGL arm is a control: it should be unchanged (< 3%). | G1 |
| HG2 (Translated / CE-relay work) | D3D11 submits >= 3x more Translated-channel (guest kernel channel) work per frame than OpenGL: doorbells to Translated tokens, relays raised, engine wakes, per frame, from per-5 s cumulative counters diffed over each API's benchmark window. Also reported per frame: USER-channel GP entries and turns, Emulated-channel work (display/RC/GSP RPC counts), and relay completion latency where an existing or small new counter gives it. | FALSIFIED if the D3D11/OpenGL ratio of per-frame Translated doorbells and relays is <= 1.5x. | G0 |
| HG3 (present / flip path) | The virtual display's flip handling differs between D3D11 and OpenGL by >= 1 ms per frame (kf-disp counters: updates completed per second, parked/STALL events, vblank tick lateness `late_max_us`, copies/checks per second; plus how Heaven presents in each API: windowed or fullscreen, flip model, DWM composition, read from the guest). | FALSIFIED if the per-frame difference is < 0.2 ms. 0.2-1 ms is reported as "present but small". | G0 |
| HG4 (PCIe / power signature) | `nvidia-smi dmon -s pucvmet -d 1` in mode 2 shows a different PCIe rx/tx rate or a different power per unit utilisation for D3D11 than for OpenGL, and a native VFIO run of Heaven D3D11 (3 min, same dmon columns, 4070 detached and restored in a trap) gives the native power, clocks, PCIe rates. | FALSIFIED if the kayfabe mode-2 D3D11 power, SM/mem clock and PCIe rx/tx signature equals native's within 10% while fps differs. | G2 |

Extra items logged in G0 (not hypotheses; reported as data): backstop wakes and `event_without_progress` per frame
(review finding D3: Pointer-mode waits have no interrupt wake), worker CPU by thread, `contended_turns`, `gates`.

## Steps and time-boxes (not widened)
G0 baseline + attribution counters, D3D11 vs OpenGL, mode 2 (2 h). G1 ring-size experiment, 3 interleaved runs per arm (2 h).
G2 dmon + VFIO reference (1.5 h). G3 one follow-up isolating a hypothesis that held (1 h); if none holds, `perf record -g` on the
QEMU worker and vCPU threads for 30 s of D3D11 mode 2, report top frames. G4 REPORT.txt.
If something fails twice for the same reason: stop and report it.

## Decisions fixed in advance
- Frame counts come from Heaven's own log (frames in the 260 s window), not from guest PresentMon (not used: TDR risk).
- A hypothesis is "held" only if its prediction threshold is met AND the effect is larger than the larger arm spread.
- Scratch counters are cumulative atomics printed on the existing 5 s tick; they never change relay behaviour.
