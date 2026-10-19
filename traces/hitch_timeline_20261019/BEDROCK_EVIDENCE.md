# Bedrock long-stall evidence and act interpretation

STATUS: RESEARCH, 2026-10-19. Part A, read-only. No VM, guest channel, GPU or
GPU-lock interaction. Recorder collection contract remains unresolved (STOP.md).

## Evidence read

MEASURED in the coordinator's saved host note
`/data/claude-stall10s-20261019/README.txt`, dated 15:28–15:35 CEST:

- Ten zero-kms/zero-copy runs in approximately eight minutes while the source
  remained broker and tick/check rates stayed about 59.8/59.4 per second.
- Means per approximately two-second interval: GR0 completions 271 versus 492,
  CE2 92 versus 24, CE3 129 versus 137, invalidates 4.6 versus 0.8; trapped
  writes 5914 versus 9145. These are reported counter-window comparisons,
  not ETW-confirmed stall rates or exactly timed engine throughput.
- allocstall_normal unchanged at 383369 between the two named samples, no Xid,
  no logged act above 100 ms in that window. PSI memory some avg60 was 1.1%.
  INFERRED: unchanged global allocstall does not exclude all compaction,
  task-local scheduling, IO or unobserved memory delay; S7 is not globally removed.
- The GSS burst comparison (6.2 versus 7.0 per interval) and slot-reclamation
  counts did not establish a cause. Follow the coordinator's instruction to
  treat reclaimed-slot messages as a consequence, not a culprit to fix.
- Guest VRAM was 1969/8192 MiB at 15:35 with Minecraft no longer running.
  It is not a gameplay residency sample.

The note contains a duration discrepancy: its headline says 3–31 lines
(6–62 seconds), while its enumerated runs have lengths
5,6,5,5,6,13,3,3,4,5 (3–13 lines, roughly 6–26 seconds). The coordinator's
summary uses 6–26 seconds. Preserve both descriptions; do not turn either into
exact stall durations without timestamped ETW and event windows. Static/idle
desktop windows are not separated in this note.

MEASURED in P1's published RESULTS.md (read from its branch):
`mirror-b5-sn-r1/r2`, source `0d7ba743`, max new-content gaps
1066.717/1083.384 ms, n=2 captures. Earlier *mirror* captures maxed at
116.672–183.342 ms; other diagnostic arms also contain 416.686/433.354-ms
gaps. INFERRED: long gaps exist, but these samples do not establish aging or
tie the mirror-OFF Bedrock observation to a mirror-ON mechanism.

## What the long act actually times

CODE READ, kayfabe candidate-5 and `0d7ba743`:
`ChanPlane::start` measures wall time beginning just before `act(self)` and
ending after delayed-act bookkeeping. It excludes time waiting in ActQueue,
includes CPU descheduling, metadata-lock contention and all calls inside the
closure, and resolves the deferred reply afterward. Executor thread name is
`kf3-chan-act`; existing lines do not identify its numeric TID or the guest
waiting process/thread. Silent successful acts and requeued attempts can omit
per-act log lines. A printed act duration is not a per-ioctl latency.

CODE READ: the timeslice closure resolves owned members and calls
`HostRm::set_timeslice` once per distinct host group. That method authors
control `0xa06c0103` on the TSG with an eight-byte requested timeslice. The
`400 us` value is the desired scheduler quantum, not a bound on control latency.

CODE READ, NVIDIA open-gpu-kernel-modules 595.91.07 commit
`9f087a6d4e86d85acc0ce1d354d6276fe5047b29`:
`src/common/sdk/nvidia/inc/ctrl/ctrla06c.h:129` documents that the change takes
effect immediately and may force a channel off the GPU. The implementation
`kchangrpapiCtrlCmdSetTimeslice_IMPL` in
`src/nvidia/src/kernel/gpu/fifo/kernel_channel_group_api.c:1296` uses
`NV_RM_RPC_CONTROL` for a GSP client and returns its status; the physical path
issues `INTERNAL_SET_TIMESLICE`. This supports a possible scheduling/preemption
or firmware wait, not proof of the cause of a particular 875734-us act. No
closed-firmware behavior is inferred as fact. Host RM lock/RPC waiting,
descheduling and multi-call closure time must be separated by per-call stamps,
scheduler evidence and a live stack capture in an authorized run.

CODE READ: `EngineObj::run/finish` verifies allocation identity, may hand over a
video context mapping, calls `kf_chan::passthrough::engine_object`, then keeps
the host object under metadata locks. DmaCopy/Compute objects use authored
`HostRm::alloc_engine_object` -> synchronous host `RM_ALLOC` -> bookkeeping.
Allocation/context setup and RM/GSP waiting are plausible sources of delay;
the existing closure timer cannot distinguish them from CPU/lock delay.
The video steering path is not evidence that a DmaCopy object used that path.

## Read-only long-act snapshot

MEASURED: current named qemu.log snapshots yielded 12 observed acts >20 ms in
3014 (5085 lines with act durations) and 9 in 3015 (3914). The two 3015
SET_TIMESLICE 400-us acts reproduce at lines 13648/13679:
265635/875734 us. The coordinator dates these to boot/desktop, approximately
45 seconds after start; the unprefixed log lines do not independently provide
VM uptime or guest waiter identity.

The current 3014 log snapshot's engine-object acts are 25962–31290 us (n=9),
not the coordinator-reported six 92–100-ms observations during M2 testing.
The current 3015 snapshot has five engine-object acts at 29504–31317 us.
Do not overwrite the coordinator's measurements: identify the alternate
log/capture for the 92–100-ms series and keep its provenance separate. Existing
files may represent different sessions; the filename alone is not a session ID.

NOT VERIFIED: the root cause of a slow ioctl, actual guest waiter identity,
Bedrock package/version/API/world, active-game VRAM residency, exact ETW stall
durations, or a causal relationship between VM age and stalls. Nothing here
authorizes stopping the owner's VM or collecting live GPU/guest data.

## Addition 4: active-game snapshot and S8 hold semantics

MEASURED by the coordinator at 15:45, Bedrock running: dedicated 2010 MiB of
8192, committed 2396 MiB, shared 268 MiB; host GPU 9093/12282 MiB, swap off.
INFERRED: this is evidence against sustained guest overcommit at that instant,
not a peak-at-stall measurement or grounds to remove VidMm from the experiment.
The one-second guest memory sampler remains a Part B requirement.

CODE READ, candidate-5 `kf-gsp/src/boot.rs:2500`: `held_this_command` is the
OR of `policy.holds_for_refresh(cmd)` and `policy.defers(cmd).is_some()`.
The same "this map reply WAITS for its rows" line is printed for either.
Thus an RmControl HELD line is not proof of a map wait; it can be a deferred
timeslice/control act. The original held structure has the RPC (including its
sequence), optional deferred status, alloc bit and refusal detail, but no
per-reply arrival/hold timestamp or row/page-count record.

CODE READ: `Device::release_settled` (`device.rs:3212`) first requires the memory
inbox to be all-settled; then `GspFsm::release_held` also waits for the head's
deferred act to settle, preserves reply order, and removes the reply only after
`post` succeeds. Instrument global settlement watermarks as well as per-reply
reasons so an unrelated mapping or earlier reply cannot be mistaken for the
held request's own work. Stamp a successful post, not an attempt.

MEASURED by the coordinator: approximately 3800 held/posted pairs, depth/n=1,
without timestamps. INFERRED: pair totals cannot reveal whether any reply
waited ten seconds, nor identify its guest waiter. S8 is now the first priority,
S1 second. Full correlated host reply spans plus feasible guest ETW wait/ready
evidence are needed before classifying a particular stall as S8.
