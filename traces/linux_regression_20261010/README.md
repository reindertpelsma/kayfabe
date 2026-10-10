# Linux regression of the integration head and the playback head, 2026-10-10

**STATUS: LIVE, 2026-10-10 (evidence).** Host `root@172.22.1.20` (RTX 4070, driver 595.91.07, kernel 7.0.0-34, not nested), shared with a
Windows-run agent through `flock -o /tmp/kayfabe-fastguest.lock`. Branch `claude/linux-regression-20261010`.
Measured and inferred are kept apart: "measured" = a line in the files here; "inferred" = my reading.

Revisions (each lane names its own; every kf3 binary is `kf3-bins/<rev>/` built by `scripts/bench/build_kf3.sh` from a clean checkout, dirty=0):

| rev | what |
|---|---|
| `c66d1b96` | `origin/integration/windows-20261010` head at the start (carries `9d4803d1`, batched-map D1-D3, combined Windows line, held-hole fix) |
| `89721ca4` | `origin/claude/playback-tdr-20261010` head (display engine join fix `17eb8b2f`, STALL report, console-only refusal, YUV console kernel `kf_yuv`, z_order) = `c66d1b96` + 17 commits, 19 files in `crates qemu cuda`, none in `kayfabe-*` |
| `d3f6c47a` | DIAGNOSTIC ONLY: `c66d1b96` + `1b921070` (INVAL-LAT/SLOW) ported by hand (`diag.patch`; the cherry-pick conflicted in the probe loop). Used for lane 7 only |

## Verdict table (both heads side by side)

| lane | c66d1b96 | 89721ca4 (playback) |
|---|---|---|
| gates `v3_gates.sh` | **9/9, `gate10=PASS`**, births 12/12 USER, refused 0 (`logs/v3_gates.log`) | **9/9, `gate10=PASS`**, births 12/12 USER, refused 0 (`logs/v3_gates_pb.log`) |
| fast suite, ONE revision, one uninterrupted lock hold | **30/30 PASS**, 0 FAIL, 0 CRASH (`logs/lrc66d1b96_suite.out`), sum `client_ms` 65728 | **30/30 PASS** (`logs/lrpb89721ca4_suite.out`), sum `client_ms` 66666 (+1.4%, noise: same arm set, no arm moved by more than its resolution) |
| CUDA ladder host | 12/12 PASS (cup2,cup3,cup8 x3; cup8bench x3) | not run |
| CUDA ladder guest (fat guest, 595.84) | 12/12 PASS, ledger forwarded, `emulated=0` | not run |
| `validate.sh` in the fat guest | **22 PASS / 2 FAIL / 12 SKIP** = the baseline numbers; the 2 FAILs are `dev_nodes` (nvidia-modeset absent) and `cuda_managed_coherence` (sync rc=719), the same two as the baseline | not run |
| broker + desktop (`input_proof.sh`, test backend) | PASS (below) | PASS, identical lines |
| HELD BY HOST / HELD-BY-OURSELVES / batch fallback / micro-reservation refusal / channel-birth refusal / DEAD or POISONED | 0 / 0 / 0 / 0 / 0 / 0 in every QEMU log of every lane (30 fast + 12 ladder + desktop + 4 diag) | same, 0 in the 30 fast logs and the desktop log |
| new host Xid | fast: 5 (negative controls, below); validate: 1 (baseline-identical, below); every other lane 0 | fast: 5 (the same five); desktop 0; gates 0 |

Xid before/after per lane: `logs/xid_before_<lane>.txt` / `xid_after_<lane>.txt` (full `dmesg | grep Xid` snapshots; the kernel ring rotates,
so counts of lines can drop between lanes; the lane's `new_xid=` in `orch3.out`/`orch4.out`/`orch5.out` is a `diff`).

## Every Xid, attributed (coordinator question)

All attributions are by timestamp: the QEMU log's last-write time of each arm against `dmesg -T`.
- **fast suite, both revisions, identical set of 5 Xid 31**: 4 during `--defer-liveness` (CE0 HUBCLIENT_CE1 at `0x60_00010000`, `0x62_00010000` x2,
  `0x64_00010000`, 3 s apart, last one 2 s before the arm ended) and 1 during `--missing-page-fault` (`0xa_11000000`, FAULT_PDE).
  Both arms are **intentional negative controls**: the source says so (`crates/kayfabe-rm-ladder/src/main.rs:7033`
  "This rung deliberately provokes a real Xid 31 and kills its victim channel"; `:11958` `FAULT_MARK=defer_liveness_prime ... THE HOST dmesg WILL CARRY AN Xid`).
  The baseline `a6587d6a` fast suite (host journal, 2026-10-10 01:57) shows the same 4 VAs in the same arm and the same fifth Xid 33 s later; so does every
  earlier suite on this host (heldhole3 at 20:32 etc.).
- **The Xid the coordinator saw at uptime 83739-89631 s** (pids 2015429, 2017502, 2164061, 2165689, 20:30-20:32 CEST) predate my first lane (21:21): they are the
  same two arms of the earlier `heldhole`/`heldhole3` fast suites (consecutive pids = consecutive arms). Not caused by my lanes; and not a new class.
- **validate (c66d1b96)**: 1 Xid 31, GRAPHICS GPC1 GPCCLIENT_T1_0 FAULT_PDE at `0x76b3_a6400000`, at the second of the lane, from the check that
  FAILs in the baseline too (`cuda_managed_coherence`, sync rc 719). Baseline `a6587d6a` (01:57-02:09 journal): one GRAPHICS GPC1 T1 Xid per validate boot at
  `a6587d6a`, `6692e621` and `c6fff2e3` alike (`0x7d83_42400000`, `0x767d_92400000`, `0x7f7b_d2411000`). Same class, not new.
- gates (`kf-gate1..9`, `kf-micro-reserve-probe reserve`), CUDA ladder host and guest, desktop proof (both revisions), the diagnostic arms: **0 new Xid**.
  (Gate 10's `nv01-control` arm that raises one Xid by design was not run: `v3_gates.sh` runs only `reserve`.)

## Desktop smoke (lane 4) and the "broker lane"

`scripts/bench/display/broker_lane.sh` exits 20 (`no desktop session (run prep)`, measured by the earlier agent at `a6587d6a` and by its source):
it looks for `plasmashell`, this host's session is GNOME (gdm, wayland). The equivalent run here is `scripts/bench/display/input_proof.sh <tag>`
(the real nvkvm-pv broker `badf2d707da4` on its `test` backend, as the session user, through the guest's Cinnamon/Xorg desktop on the stock NVIDIA driver;
QEMU with `display=on,gop=on,display-broker=...`). Both revisions, every graded line the same:
GRUB via broker key PASS (editor on serial), `PROOF_DESKTOP cinnamon_up`, ABS pointer PASS x8 (within 1 px of the target; the check allows 2), REL under grab
`(70,30)` PASS, ABS dropped while grabbed PASS, GRAB on/off messages, KEYS PASS (shift+a, ctrl+l, a), broker restart re-attach PASS (QEMU re-sent geometry and
last frame), evdev sums match (rel 70/30, tablet 18 abs events, wheel). The display ran 60.00 Hz armed, `late_max_us` 1034/1244, `held=0`, `over=0`.
Screendumps: `desktop_c66d1b96.png`, `desktop_pb89721ca4.png` (1920x1080 Cinnamon desktop, panel visible; the 25 boot shots and the grub shots are on
the host in `broker-interactive/proof-{lr1,pb1}/`). `PROOF_STATUS`: `broker[up=1 sent=258/266 ... dropped=0 blocked=0 reconnects=1]`.
Human interactive checks (a real window, real mouse, fullscreen, an actual video overlay) remain for the owner. Not run: `PROOF_REBOOTS` (display across reboots is a
named open item, not a regression check), `PROOF_UNLOAD`.

## CUDA ladder, versus `6692e621` (`traces/perf_baseline_6692e621_20261010/`; same host, same fat guest 595.84, 3 reps)

No perf verdict beyond "within noise or not":
- guest cup8bench N=16 `batch_per_launch` 10 us (baseline 10.21 us); N=2048 8.79/8.88/8.87 ms (baseline 8.96 ms): within noise. init 433-451 ms
  (baseline 476), ctx 227-234 ms (baseline 244): within noise or faster.
- host arm N=2048 8.8 ms and ctx 65-73 ms: same as the baseline host arm (8.98 ms / 71.5 ms).
- **trapped BAR0 writes per boot are up about 4% and exactly stable**: cup2 12600/12613/12603 (baseline 12119), cup3 12620-12634 (12117), cup8 12718-12768
  (12249), cup8bench 13291-13312 (12959). The earlier `a6587d6a` ladder run (`val-a6587d6a/logs/step4_guest.out`) has the same counts (12572-12640), so the
  rise is already in the integration line, not in the Windows commits since. Measured; cause not investigated (inferred: extra init-time BAR0 writes from the integration line).
- fast suite sum `client_ms` 65728 vs baseline 65820 (min..max 65641..66057): identical. ce-client-guest-ram 20650 vs 19959 (19880..20080): +3.5%, the only arm outside
  its baseline range by more than 1%; defer-liveness 12510 = 12510; gpga-reserve-probe 9310 = 9300; missing-page-fault 3420 vs 3450.

## 12 ms invalidate bound (OWNER_RULINGS section AD), lane 7

`INVAL-LAT` is not on the integration line (it lives on the diverged `tdr-opus-base-20261010`, commit `1b921070`). I ported it by hand onto `c66d1b96`
(`d3f6c47a`, `diag.patch`, probe flag `KF3_COMPLETION_PROBE=1000` only; the production binary `c66d1b96` has none of it) and ran ONE diagnostic set, separate from the lanes above
(`logs/inval_*.txt`, full summary lines). Guest-visible latency = vCPU arm to VA-thread idle publish:

| workload | invalidates | p50 | p90 | p99 | max | over 12 ms |
|---|---|---|---|---|---|---|
| `--ce-client-guest-ram` (fast guest, 24 s) | 49083 (last summary) | 262 us | 328 us | 393 us | 90.6 ms | 1 at the last summary, 2 INVAL-SLOW lines in total (90.6 ms, 29.5 ms) |
| fat guest cup8, one boot (CUDA init) | 423 | 197 us | 262 us | 33.5 ms | 94.6 ms | **6**, in three (about 91 ms, about 31 ms) pairs |
| `--uvm-invalidate`, `--map-stress` | arms end before the first 2 s summary: n=0 shown; no data | | | | | |

Every slow one reads `VA thread: walk in flight=false pending wants=0 all_settled=true away since its previous publish <same ms>`: the VA thread was busy
with something other than a walk (inferred: a refresh/map outside the walk loop, around CUDA context creation) while the guest was armed. So on Linux the
bound is violated during CUDA init (measured: 6 of 423 in cup8), though the steady state is about 0.26 ms. Not a regression claim (no baseline for INVAL-LAT).

## Lane log and harness traps met (so the next run does not repeat them)

- Build: the host has cargo in `~/.cargo/bin` (not on the non-interactive PATH); clone `$W/kayfabe-linuxreg`, `CARGO_TARGET_DIR=$W/target-linuxreg`; the
  whole build (kf-qemu + QEMU relink + rm-ladder client + kf-harness bins) took about 2 minutes on 24 threads. `W=/var/lib/kf-windows-20261005`.
- **`cuda_ladder.sh host` and `run_fast_guest.sh` take `/tmp/kayfabe-fastguest.lock` themselves**: wrapping them in `flock -o` on the same file self-deadlocks.
  I did that once for the ladder host: it sat 40 minutes (22:20-23:00) holding the lock while blocked on itself, which also held back the Windows agent's queue. Sorry.
  Fix used afterwards: the host ladder runs bare (it self-locks); the fast suite and `input_proof.sh` run under ONE outer hold with a private `KF_LOCK=<file>` so the
  arms do not wait for the Windows queue between arms (the first, interleaved attempt, `logs/aborted_contended_suite.out`, did 10 arms in 20 minutes and was aborted
  and restarted from arm 1 so that the 30/30 is one uninterrupted single revision).
- The first `validate` lane failed in 15 s: the hook copy lacked `+x` (`hook finished: rc!=0`, "workload added ZERO guest kernel lines"); rerun after `chmod +x` gave the numbers above.
  `validate_hook.sh` here is the earlier agent's hook with the checkout path changed.
- A Windows run (`kf3-bins/89721ca4`, pid 2596134) was running when I finished; the host root disk had 227 GB free; IOMMU group 11 was `identity` because of that run
  (not mine). All my QEMUs had exited (`qemu_left=0`).
- `/tmp/kf-stop-winprod` absent; GPU on nvidia 595.91.07.

## Safe to merge?

- **`c66d1b96` (integration)**: no Linux regression found. Gates 9/9 + gate 10 PASS, fast 30/30 on one revision, ladder host+guest 24/24, validate = baseline, desktop and input
  PASS, 0 HELD, 0 refused births, every Xid attributed to an intentional negative control or the baseline-identical validate fault.
- **`89721ca4` (playback head)**: **no Linux regression found; safe to merge on the Linux evidence**: gates 9/9 + gate 10, fast 30/30 (one revision), desktop/input proof
  with the same graded lines and the same display counters as `c66d1b96`, 0 HELD, 0 refused births, the same 5 negative-control Xid. Not covered here (not run on this head): CUDA ladder,
  `validate.sh`, any real video overlay / the YUV console path (no Linux guest exercises FORMAT 0x38), and the Windows playback itself. The display commits are small and separable
  (`17eb8b2f`, `f34d8937`, `f72b0d34`, `0f1ef8c4`) should a later Linux run find a problem.
