# Two guests on one GPU, and the trio (host + two guests): pre-registration, 2026-11-05

STATUS: DESIGN-ONLY, 2026-11-05 (lane trio). Written and pushed BEFORE any box was rented. Results go to
`README.md` in this directory, never into this file (a correction is added above the text it corrects).

## Why

`docs/STATUS_DETAIL.md` says "Two VMs sharing one GPU: not measured", and `docs/design/V3_MULTI_GPU_AUDIT.md`
lists "shared-pool BAR1 headroom (second VM)" as OPEN. The owner's criteria for Ada and later are "2 guests
sharing 1 gpu" or "1 guest and the host simultaneously using the gpu". Earlier two-VM runs on 172.22.1.20 are
informal and are not cited as measured here. `docs/design/V3_GPU_UUID.md` section 8 item 2 (two guests, two
different UUIDs) has not been run either.

## What runs (one box, product code = `next` at d74f6063d)

The branch `claude/trio-vast-20261105` is `next` at d74f6063d plus this directory and the draft harness
`scripts/bench/box/trio_stage.sh`; no `crates/`, `qemu/` or `cuda/` file differs (`git diff --stat d74f6063d`
is checked before the run and quoted in the README). Box: one Vast KVM box with a 24 GB card, Ada first
(RTX 4090, AD102), RTX 3090 (GA102, the gate PASS of 2026-11-05) as the fallback; host driver as provisioned
(580.159.04).

1. `provision_full.sh`, then `merge_check.sh` (gates 9/9, kf3 build, raw client, bare-metal suite 30/30, fast
   guest, the 30-arm fast suite: the **Linux solo baseline**, birth census). The Windows install
   (`win_vm.sh install`, CPU only) starts after `FG_RC=0` and runs beside the rest of the merge check (stated
   in the README; a solo-baseline arm time that ran beside the install is marked so).
2. `windows_gate.sh <gtag>` (the install is already there): a gate verdict on this box and revision, and
   the overlay with the driver and the desktop. This is the **Windows solo baseline**.
3. `trio_stage.sh <tag> <gtag>`: host solo loop (120 s), Windows boot from the gate overlay with
   `gpu-uuid=GPU-<VM_UUID>` and `--once` (a guest reset is never restarted silently), 3 solo probe rounds
   (cup8, d3d11), **phase A**, **phase B**, Windows reset counters since its boot, graceful stop.

The harness changes against a solo run, and none is a product change: the two VMs take different GPU lock
files (`KF_GPU_LOCK`, `KF_LOCK`; the shared default lock is what keeps GPU work serial on purpose), and
`KF_GPU_IDLE_MIB` is raised so each fast arm does not wait 30 s for a GPU that Windows keeps in use. The fast
guest's launcher is otherwise unmodified: no `-uuid`, so `gpu-uuid=auto` takes its documented fallback (a random
UUID per QEMU start, with a named warning).

Workloads:

- host: `scripts/bench/cup8.c` (one NxN fp32 matmul with a closed-form answer, byte exact) and
  `scripts/bench/cup8bench.c` (C poisoned before every launch, every iteration verified), alternating in a loop.
  N in phase B is sized from the smallest free VRAM seen in phase A: 3·N²·4 bytes ≤ 0.5 of it, N a multiple of
  1024, at most 16384.
- guest 1, Windows 11 LTSC at its signed-in desktop: idle in A; in B a loop of `cup8.exe` (byte exact) and
  `kf_dxprobe.exe d3d11` (compute, draw, readback), the gate's probes through `gate_run.ps1`.
- guest 2, the Linux fast guest: the 30-arm fast suite (`fast_suite.sh`, budget 180 s per arm) in A and in B.

## Hypotheses and falsifiers

**H-A (two guests).** With Windows at its desktop on kf3, a second QEMU (the fast guest, one per arm) realizes
on the same GPU, and the fast suite ends `FAST_SUITE_PASS=30 ... ARMS=30` while Windows stays up: QEMU alive,
no `GUEST_PANICKED`, ssh answers every 60 s heartbeat, cup8 and d3d11 still correct after A, every Windows reset
counter 0 since its boot (`GATE_TDR ... total=0`), and no new host Xid naming the Windows QEMU's pid.
FALSIFIER: a refused realize of either QEMU (named cause from its log, e.g. a host BAR1 or store refusal),
any fast arm FAIL/CRASH/TIMEOUT/NOTRUN, a missed heartbeat, a wrong Windows probe, a reset counter > 0, or a
host Xid on the Windows QEMU. Xid lines on the fast guest's QEMUs are compared with the same arms' solo run
(merge check): more or other kinds than solo count against H-A.

**H-B (trio).** Host loop + Windows probe loop + fast suite at once: every host unit correct, every Windows
probe correct, the fast suite 30/30, each of the three completes at least one unit in every 600 s window while
it runs (forward progress), and the H-A conditions hold. FALSIFIER: any wrong result, an empty 600 s window,
or any H-A falsifier during B.

**H-UUID.** The Windows QEMU's realize line and Windows' `nvidia-smi -L` show `GPU-<its VM_UUID>`; every fast
arm's realize line shows a different value; none equals the host GPU's UUID. FALSIFIER: an equal pair, or the
host's value anywhere. (UUIDs are compared on the box; the committed evidence carries synthetic ones.)

**Footprints (recorded, no prediction gated).** VRAM used (nvidia-smi, MiB) idle, with Windows up, and the
maximum while a fast arm runs; host BAR1 used/total; host MemAvailable and the PSS of each QEMU (MiB). INFERRED
expectation, stated so it can be wrong: Windows `fb-mb=8192` plus overhead, the fast guest `fb-mb=8192` plus
overhead, so about 17-19 GiB used and 5-7 GiB free on a 24 GB card. **BAR1, no prediction:** each device asks
for its apertures + PRAMIN + 16 MiB headroom (about 177 MiB, more with the Windows GOP), so the sum exceeds a
256 MiB host BAR1; whether that refuses depends on whether the host apertures are taken eagerly, which this run
measures.

**Timing (recorded).** Wall time of each stage, to size a CI stage against the 300-minute Worker limit.
Throughput ratios (B or A against solo) are recorded and never gate.

## Classification

Bare metal first: a fast arm that fails in A or B is compared with the same arm in the merge check's solo run
on the same box and revision. A failure with a named cause in kayfabe's code is reported as kayfabe's and is NOT
patched in this lane beyond a separate, minimal PR. A host, rental or harness fault is named as such; RETRY is
not PASS. The informal 172.22.1.20 runs are not evidence for or against any hypothesis here.

## Budget

4 USD, 4 h wall, lease ≤ 3 h per box, one re-rent allowed. Instance ids, rates and times go to the README.
