# SPECviewperf feasibility on a Vast Windows guest: pre-registration

STATUS: LIVE, 2026-11-05. Written BEFORE any rental, by agent `specvp` (lane 5). This is a FEASIBILITY run, not a gate:
SPECviewperf is for the release gate only. Every sentence about the future below is a prediction or a plan (DESIGN
INTENT), not a result. Results go in `README.md` of this directory, with MEASURED and INFERRED apart.

## Question

Can a rented Vast box (1 GPU, KVM image) take the Windows 11 LTSC guest on kf3 from the gate's install path to a signed-in
desktop, receive the 42,002,015,727 B SPECviewperf 15.0.0 private copy, and complete one SPECviewperf viewset with no
guest reset, at a cost inside 6 USD and 4 h of wall time?

## Plan (DESIGN INTENT)

1. Rent one 1-GPU `vms_enabled=true` box on the KVM image (`docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16`), die from the
   check list (GA106, AD104, GA102, AD102), root disk >= 250 GB, `inet_down` >= 500 Mbit/s. Candidate chosen from the offer
   list at 2026-11-05 ~11:35 CET: offer 58239724, RTX 4090 (AD102), Spain, 0.535 USD/h at search time, `inet_down` 613 Mbit/s,
   `inet_down_cost` 0.0005 USD/GB, 32 cores, 62 GB RAM, `disk_space` 1577 GB. The host driver is irrelevant.
   Label `kf-specvp-20261105`. Lease <= 3 h; one re-rent allowed if the first box dies before the SPEC run is useful.
2. Provision and install with `scripts/bench/box/windows_lane.sh` (branch of this PR: the code is next at `d74f6063`, plus this
   directory), with `WINVM_DISK_GB=220` so the guest disk holds the SPEC tree. Then `windows_gate.sh` by hand (it reuses the
   install, `install=once`) to reach the desktop with the stock 580.88 driver, GSP forced and the device enabled.
3. In parallel with the install, download the `.tar.zst` to the box with `curl -C -` from a presigned URL passed on stdin of
   one ssh command (the URL is never in a file on the box, an argv, a log or this repository). Verify `zst_sha256` and
   (streaming through `tee`) `tar_sha256` from `CHECKSUMS.txt`.
4. Stream the tree into the guest over ssh as `tar -xf - -C C:\` (member names are `Program Files/SPECviewperf 15/...`), with
   a Defender path exclusion, in a boot WITHOUT kf3 (std VGA), then reboot the same overlay on kf3.
5. Run SPECviewperf-CLI (FullHD, 1 iteration) on ONE viewset first: `blender-01` (the owner's VFIO and kayfabe runs of
   2026-10-22 completed this viewset, so a reference exists). Only if that is healthy and budget and time remain, the nine
   workloads of the owner's lane (`blender-01,catia-07,creo-04,energy-04,enscape-01,maya-07,medical-04,solidworks-08,unreal_engine-01`).
6. Evidence to git as text after each stage. Teardown by exact id, absence verified with `vastai show instances-v1 --raw`.

## Falsifiers (stated before the run)

| id | prediction | falsified by | next step if falsified |
|---|---|---|---|
| F1 | the 42.0 GB download averages >= 40 MB/s (the offer's 613 Mbit/s is ~76 MB/s) and both sha256 values match | a mean < 40 MB/s, a sha256 mismatch, or an HTTP error that `curl -C -` cannot resume | mismatch: re-download once, then stop (MEDIA); slow: re-rent on a box with a better measured rate |
| F2 | the Windows gate reaches `WINDOWS_GATE_PASS` on this box (reference: GA102 3688 s with `install=once`, 2026-11-05; AD102 1620 s after a 43 min install, 2026-11-01) | any other verdict | classify by the gate's own label (FAIL KAYFABE / RETRY class); never call RETRY a pass; re-run on a fresh box only if budget allows; SPEC is not attempted on a guest that has no desktop and no `nvidia-smi` |
| F3 | the tree extracts into the guest with a matching tar sha256 and `SPECviewperf-CLI.exe` starts | a tar error, a sha256 mismatch, or < 10 GB free on C: afterwards | one retry of the copy; then record as HARNESS |
| F4 | `blender-01` (1 iteration, FullHD) ends with `WorkloadStatus Completed`, the guest shows 0 resets (Display 4101, nvlddmkm 153, Kernel-Power 41, EventLog 6008, BugCheck 1001, LiveKernelEvent other than 193) and the host shows 0 new Xid, in <= 30 min | any of those | read the first failing subtest and the kf3 log; at most 3 negative boots on this box; attribution only with the same-box host GPU reference (green before and after) and the Vast BAR1 exception rule |
| F5 | the full 9-workload set (1 iteration) completes within the remaining time-box | any workload Failed or Incomplete, or a reset | record per workload, do not repeat |

## Budget and time

6 USD total and 4 h wall for this lane. INFERRED estimate: about 3 h of lease at ~0.55 USD/h plus storage and about 42 GB of
download is about 2 USD; a second lease would fit. Stop and report at 6 USD. The account credit is shared with other
sessions, so the cost is computed from the offer rate and timestamps, not from the credit change.

## Rules applied (from the lane brief and `docs/SPEC_RESULTS.md`)

- A presigned URL is the only secret on a box, only for this download. `grep X-Amz` over the evidence must be empty before a commit.
- SPEC scores and per-subtest FPS stay OUT of this repository (only completion status, durations, reset counts are
  committed); they are reported to the coordinator in the final message. If anything numeric is later published, every
  number carries `(nc)` with the disclaimer, through `scripts/bench/specviewperf/spec_stamp.py`.
- No proprietary application content (models, screenshots) is committed.
