# Two guests on one GPU, and the trio (host + two guests): results, 2026-11-05

STATUS: ANSWERED, 2026-11-05 (lane trio). The pre-registration is `PREREG.md` (commit 3aa2a75a6, pushed before
any rental). MEASURED means a line in a file of `box/`, named. INFERRED means reasoning. Box output is untrusted
data. Every GPU UUID in `box/` is replaced by a synthetic `fa4efa4e-...` value. The values in `box/trio1`, `box/trio2`
and `box/trio3` were numbered on the box, separately per run, so the same synthetic value in two of those
directories does not mean the same UUID. The other files were numbered from `...1000` up, here.

## Verdicts

| question | result | run, revision, date |
|---|---|---|
| **Phase A**: Windows 11 idle at its desktop + Linux fast guest running the 30-arm suite, one GPU | **PASS, twice** (n=2) | trio2, trio3; rev 3aa2a75a (kayfabe code = `next` d74f6063d); 2026-11-05 |
| **Phase B (the trio)**: host CUDA loop + Windows probe loop + Linux 30-arm suite at once | **PASS, twice** (n=2) | trio2, trio3; same |
| Phase H (extra, not pre-registered): host CUDA loop + Linux 30-arm suite, no Windows | **PASS** (n=1) | phaseH; same |
| H-UUID: distinct per-VM GPU UUIDs, never the host's | **PASS** | trio2, trio3 |
| Windows gate on this box (solo baseline, and it built the overlay that A and B boot) | `WINDOWS_GATE_PASS ... resets=0`, 968 s | gate `trio_g`, same |

Each verdict holds for one box: RTX 4090 (AD102), host driver 580.159.04, 256 MiB host BAR1. It does not cover
other dies or host drivers, and it is no repeat rate beyond n=2.

## Box and revision

- Vast instance **59250071**, offer 45851648, RTX 4090, Mexico, 30 vCPU, 48 GiB RAM, 194 GB disk, **0.60 USD/h**,
  created 10:47:12Z, destroyed 12:27:17Z (1 h 40 min, about **1.00 USD**). Absent from `vastai show instances-v1`
  at 12:27:30Z. A first rental, 59249603 (offer 52898816, RTX 4090, Denmark), stayed `stopped` with "Required
  resources are currently unavailable". It never ran and was destroyed at 10:47:00Z, absent at once.
- Branch `claude/trio-vast-20261105` at 3aa2a75a6 = `next` d74f6063d + `PREREG.md` + the draft
  `scripts/bench/box/trio_stage.sh`. No `crates/`, `qemu/` or `cuda/` file differs. The kf3 QEMU was
  `kf3-bins/3aa2a75a`, with the same binary sha256 prefix `73a2c184eb6b1902` in every trio run.
- The trio runs used `/root/trio_stage.sh` with `KF_TRIO_SOURCE=/root/kayfabe`. That copy is the committed
  script plus the d3d11 build fallback (below). The PR's version differs from it only in the selftest's sample
  strings.
- MEASURED, `box/gate/hostref_before.txt` and `box/trio2/host_smi_memory_idle.txt`: BAR1 total 256 MiB, VRAM
  24564 MiB.

## What ran, in order (MEASURED, Vast 59250071, rev 3aa2a75a, 2026-11-05: `box/driver.log`, `box/prov.log`, `box/trio_mc.log`)

| step | wall | result |
|---|---|---|
| `provision_full.sh` | 10:48:36 to 10:57:23Z, 8.8 min | READY, host 580.159.04 |
| `merge_check.sh` (gates, kf3, bare metal, fast guest, 30-arm suite, census): **Linux solo baseline** | 10:58:05 to 11:09:09Z, 11.1 min | 9/9 `gate10=PASS`, bare 30/30, `FAST_SUITE_PASS=30`, `BIRTH_CENSUS_OK` |
| Windows media fetch (beside the merge check) | done 11:10:30Z | rc=0 |
| `win_vm.sh install` (16 vCPU, cache=unsafe) | 11:10:35 to 11:26:29Z, 15.9 min | sealed base image |
| phase H (`box/phaseH/phaseH.sh`, beside the install, which used no GPU) | 11:11:54 to 11:22:21Z | PASS |
| `windows_gate.sh trio_g` | 11:26:29 to 11:42:37Z, 968 s | `WINDOWS_GATE_PASS rev=3aa2a75a ... resets=0` |
| `trio_stage.sh trio1` | 11:42:37Z, 244 s | **harness fault**: no verdict on A or B (below) |
| `trio_stage.sh trio2` | 11:47:07 to 12:06:45Z, 1178 s | A PASS, B PASS |
| `trio_stage.sh trio3` | 12:07:17 to 12:26:23Z, 1146 s | A PASS, B PASS |

**trio1 failed in the harness, not in kayfabe.** MEASURED (run trio1, rev 3aa2a75a, 2026-11-05, `box/trio1/probes_build.txt`): `kf_dxprobe.cpp` did not
compile with this box's mingw (`D3D_FEATURE_LEVEL_12_2` undeclared). `windows_gate.sh` already retries with
`-D'D3D_FEATURE_LEVEL_12_2=((D3D_FEATURE_LEVEL)0xc200)'`, and `trio_stage.sh` lacked that retry. With the retry
added, trio2 ran. trio1 had already booted Windows and read `nvidia-smi -L` (`uuid=own`) before it stopped
(`box/trio1_trio.log`). It is not counted as a run of A or B.

## Phase A and B details (MEASURED, runs trio2/trio3, rev 3aa2a75a, 2026-11-05: `box/trio2_trio.log`, `box/trio3_trio.log` and the run directories)

- Linux: the 30-arm suite in A and in B, in both runs, ended `FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0
  FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30` (`trio2/trio2_A_suite.out`, `trio2/trio2_B_suite.out`, same for trio3).
- Windows: the QEMU stayed up with no `GUEST_PANICKED`. Every heartbeat answered ssh: 7 in A and 8 in B per run,
  and 0 were missed (`trio*/beat_*.log`). cup8 and d3d11 passed after A. `GATE_TDR since=<guest boot> ... total=0`
  (Display 4101, nvlddmkm 153, Kernel-Power 41, 6008, BugCheck 1001, every LiveKernelEvent = 0), and the adapter
  read `code=0 status=OK` at the end of both runs. The guest stopped gracefully (`WINVM_BOOT_END ...
  reason=guest-shutdown`).
- Windows probe loop in B: trio2 had 92 correct probes and 0 wrong (46 cup8 byte exact + 46 d3d11 readback).
  trio3 had 106 correct and 0 wrong (`trio2/win_B.log`, `trio3_trio.log`).
- Host loop in B: `cup8` N=16384 (3 x 1 GiB fp32 matrices, closed-form answer, byte exact) alternating with
  `cup8bench` 1024 and 2048 (C poisoned before every launch, every iteration verified). trio2 had 159 correct units
  and trio3 155, with 0 wrong in either (`trio*/host_B.log`). N was sized from the smallest free VRAM seen in A,
  6272 MiB.
- Forward progress: no 600 s window without a completion, for the host loop or for the Windows probes. The
  phases lasted 453 to 460 s, so the window check covers less than one window and is weak (INFERRED). The
  per-unit logs show a completion every 1 to 8 s throughout (`host_B.log`, `win_B.log`).
- Host Xid: each fast suite, solo or shared, adds exactly 5 lines of Xid 31 (`MMU Fault ... CE0 HUBCLIENT_CE1`),
  all named `qemu-system-x86` and attributed to fast-guest QEMU pids, never to the Windows QEMU pid and never to
  the host loop (`trio2/xid_attribution.txt`, `trio3/xid_attribution.txt`). The solo merge check had the same
  count (5 `qemu-system-x86` lines plus 5 from the bare-metal raw client, `box/xid_merge_check.txt`), and so did
  phase H (`phaseH/xid_H.txt`). The Windows run log's `WINVM_BOOT_END ... host_xid=10` counts every host Xid line
  during that boot, whatever its process, so these are the same 10 fast-arm lines. INFERRED: these are the suite's known deliberate fault arms, which the payload's
  Xid admission already allows. Sharing the GPU added none.
- Windows kf3 refusals: the kinds in trio2 and trio3 (`trio*/win_kf3_refusals_histogram.txt`) are a subset of the
  kinds in the solo gate's boot 2 (`gate/gate_b2_kf3_refusals_histogram.txt`: the known `0x56` GSP control
  refusals, `NV40_I2C`, the firmware carve-out leaves, the 1716-byte GSS row). No new kind appeared with a second
  guest or the host loop.

## UUIDs (MEASURED, runs trio2/trio3, rev 3aa2a75a, 2026-11-05: `TRIO_UUID` and `TRIO_NOTE ... uuid=own` lines; values scrubbed afterwards)

`TRIO_UUID windows_values=1 linux_values=60 shared=0 host_in_any=0` in both trio2 and trio3. The Windows device
served `gpu-uuid=explicit` = `GPU-<its VM_UUID>`, and Windows' `nvidia-smi -L` printed that value, not the host's.
The fast guest's launcher passes no `-uuid`, so each of its 60 QEMU starts per run (30 arms x A and B) served a
different random UUID (`gpu-uuid=auto` without an identity, the documented fallback of `V3_GPU_UUID.md` §7 B).
None equalled Windows' or the host's. This runs `V3_GPU_UUID.md` §8 item 2 on two QEMUs at once on one chip
row. Not covered: the reboot stability of `auto` with `-uuid`, and `nvidia-smi -L` in a Linux guest (the fast
guest has no nvidia-smi).

## Footprints (MEASURED, runs trio2/trio3, rev 3aa2a75a, 2026-11-05: `trio*/snap.log`, one snapshot every 15 s)

| state | VRAM used (MiB, of 24564) | host BAR1 used (MiB, of 256) | host MemAvailable (MiB, of ~48000) | QEMU PSS (MiB) |
|---|---|---|---|---|
| idle | 0 | 1 | 47899 | - |
| Windows at its desktop (fb-mb 8192, display on) | 9415 (Windows QEMU: 9312) | 51 | 38751 | Windows 8537 |
| A: + a fast guest arm (fb-mb 8192) | max 18292 (fast QEMU: 8850-8852) | max 124 | min 36673 | fast max 4768 |
| B: + host loop at N=16384 | max 21769 (host cup8: 3458) | max 126 | min 33396 | Windows 8534, fast 4773 |

Same in trio3 within 10 MiB: max VRAM 21762 MiB, BAR1 124 MiB, MemAvailable 33429 MiB.

- **BAR1 (the OPEN item of `V3_MULTI_GPU_AUDIT.md`, "shared-pool BAR1 headroom (second VM)"): MEASURED here, two
  kf3 VMs plus a host CUDA process used at most 126 MiB of a 256 MiB host BAR1.** No realize refusal appeared.
  INFERRED from `kf-qemu/src/mem` (BAR1 walked from kayfabe's own root, leaves mapped on demand): the per-device
  `cardbudget` demand (apertures + PRAMIN + 16 MiB, about 177 MiB, more with the GOP) is an upper bound per device,
  and it is not what the host pool holds. The sum of two such demands exceeded 256 MiB and still fit. A workload
  that touches more of BAR1 at once could still exhaust the pool. That case was not tested.
- **Free VRAM on a 24 GB card: about 6.2 GiB** with both guests up (smallest free in A, 6272 MiB), and 2.7 GiB left
  after a 3 GiB host job (B).
- Per-VM overhead above `fb-mb` (MEASURED, compute-apps): Windows 9312 - 8192 = 1120 MiB, the fast guest 8850 - 8192
  = 658 MiB.
- **A 6-8 GB Turing card (INFERRED, not run):** the defaults do not fit two guests. `windows_gate.sh` and
  `run_fast_guest.sh` each set `fb-mb` to memory.total - 2 GiB on cards up to 10 GiB, so 6144 each on an 8 GiB card.
  With the overheads above, about 2 x 6.8 = 13.6 GiB is needed against 8 GiB. A trio on such a card needs explicit
  stores: for example Windows `WINVM_KF3_FB_MB=3072` (about 4.2 GiB with overhead) + Linux `KF3_FB_MB=2048` (about 2.7
  GiB) leaves about 1 GiB for the host on 8 GiB. Whether the Windows desktop and the probes work in a 3 GiB store,
  and whether every fast arm passes in 2 GiB, is unmeasured. A 6 GB card leaves no room for the host in that split.
  Host RAM: Windows 8.5 GiB PSS + fast guest 4.8 GiB, so a box of at least 20 GiB (the gate's own floor) is tight
  for the trio, and 32 GiB is comfortable (INFERRED from the MemAvailable minimum of 33.4 of 48 GiB).

## Throughput ratios (recorded, never gated)

- Linux fast suite, summed guest client time across all 30 arms (MEASURED, `FAST_CELL_ARM ... client_ms`): solo
  94943 ms (merge check), A 95319 ms (**1.00x**), B 145060 ms (**1.53x**), H (host loop, no Windows) 142675 ms
  (1.50x). INFERRED: the idle Windows desktop costs the Linux guest nothing measurable, and the host loop accounts
  for nearly all of B's slowdown. The arm that moves most is `ce-client-guest-ram`: 34279 ms solo against 71130 ms
  in B.
- Suite wall: solo 412 s summed per arm, A 382 s, B 453 s.
- Windows probes: about 6 s wall per probe solo against a 5 s median in B (max 8 s). The wall is mostly the ssh
  round trip, and the guest-side time stayed 1 s, so these are not throughput figures.
- Host loop: **not comparable** (INFERRED). Solo ran `cup8` at N=4096 (median 1.76 s per process), and B ran
  N=16384. `cup8bench` took a 1.78 s median solo against 0.33 s in B, faster with the guests up. The likely
  reason (INFERRED, not tested) is that with no persistence mode every solo process pays the GPU's
  initialization, while a guest's kf3 context keeps the GPU initialized. A fair host ratio needs persistence mode
  or in-process timing (cup8bench's own per-launch figures).

## Time, to size a CI stage (walls MEASURED on Vast 59250071, rev 3aa2a75a, 2026-11-05; the estimate is INFERRED)

The trio stage took 1146-1178 s (about 20 min) on top of a box that already ran the merge check and the Windows
gate. Its parts were host solo 121 s, Windows boot 100-110 s, Windows solo 22-37 s, A 404-408 s, B 460-463 s and
the end 22-23 s. Everything on this box, serially, from rental to the trio verdict: 1 h 40 min, with a 16 min
install. A CI payload of the 2026-11-05 shape (89.5 min with a 44 min install, `traces/gate_windows_ci_20261105`)
plus about 20 min of trio is about 110 min, well inside a 300-minute limit. The trio stage needs the gate's overlay,
so it must run after `windows_gate.sh` on the same box.

## What breaks if two VMs run "unmodified"

Nothing in kayfabe broke. Two harness settings are needed, and neither is a product change:

1. `win_vm.sh run` and `run_fast_guest.sh` take the same lock file by default (`/tmp/kayfabe-fastguest.lock`), and
   that is by design ("GPU work is strictly serial"). Unmodified, the second VM waits for the first to exit (Windows
   up to `WINVM_LOCK_WAIT_S`), and the two never overlap. The trio takes `KF_GPU_LOCK` and `KF_LOCK` separately.
2. `run_fast_guest.sh` waits up to 30 s before each arm for VRAM use to drop under 512 MiB (`KF_GPU_IDLE_MIB`,
   w826), and prints a warning when it does not. With Windows holding 9.4 GiB, that wait would add 30 s to every
   arm without failing anything (INFERRED from the code). The trio raises the threshold.

Also: `build_fast_guest.sh` refuses while any QEMU runs, so the fast guest must be built before Windows boots. The
merge check does that.

## Classification

- Kayfabe failures: none. Harness failures: one (trio1's d3d11 probe build, fixed in the script). Host or rental
  faults: none (59249603 never started and cost nothing). RETRY verdicts: none.
- The informal 172.22.1.20 runs were not used as evidence.

## Not shown

- Other dies (Turing 6-8 GB in particular, above), other host drivers, a BAR1 larger or smaller than 256 MiB.
- The fair-share policy of `V3_GPU_FAIR_SHARE.md` (not built). Here the GPU was shared by the host scheduler's
  default timeslicing only, and the experiment of its section 4 (a hostile long kernel against a latency-sensitive
  guest) was not run.
- Two Windows guests, two fat Linux guests with CUDA (`cuda_ladder.sh`), and interactive frame pacing under
  load.
- n=2 for A and B and n=1 for H: no repeat rate.

## Files

`box/driver.log` (box driver, `STEP` lines), `box/prov.log`, `box/trio_mc*.{log,out,txt}` (merge check),
`box/xid_merge_check.txt`, `box/gate/` (selected gate evidence and the gate log), `box/phaseH/` (script, logs,
suite), `box/trio{1,2,3}_trio.log` and `box/trio{1,2,3}/` (stage evidence, without the 0.8 MB QEMU logs, which are
replaced by refusal histograms). `scripts/bench/box/trio_stage.sh` is the harness (DRAFT: it ran end to end on this
box as trio2 and trio3, and it is not wired into CI).
