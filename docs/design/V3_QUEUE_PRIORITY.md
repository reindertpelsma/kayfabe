# V3 queue priority: `VK_KHR_global_priority` in the guest

**STATUS: LIVE, 2026-11-18 (second cut: the first cut's hardware falsifier hit and is folded in below; the second cut is GPU-free tested, hardware re-test pending).** Owner ruling
2026-11-18: a known kayfabe bug is fixed, and every kayfabe-core failure gets a GPU-free Rust reproduction first
(`docs/design/V3_REPLAY_TESTS.md`). Code: `crates/kf-rm/src/queuepriority.rs`. Replay:
`crates/kf-rm/tests/queue_priority.rs` (fixture `tests/fixtures/queue_priority_refusal.txt`). Oracle:
`crates/kf-rm/tests/queue_priority_oracle.rs`. Mutants: `scripts/ci/queue_priority_mutants.py`.

## ★ Correction (2026-11-18, same day): the policy control alone was not enough

**[measured]** (RTX 4090, kf3 on the tree that includes the first cut, PR #508; scrubbed logs held by the coordinator under
`/workspace/bench/sparse_c2_hw_20261118/priority_evidence/scrubbed/`, not committed): `phigh` Pass; `pcreate` and `pquery`
Fail `VK_ERROR_INITIALIZATION_FAILED` (-3); `prt` NotSupported (the test's `createDevice` -3); host Pass on all four.
The kf3 log, whose `GSP REFUSED` line prints once per control per boot, holds exactly two refusals across all five windows:
`fn76/0xa06f0111=0x56` (seq 1942) and `fn76/0xa06c0110=0x56` (seq 2009). The first cut's falsifier ("still -3 after accepting the
control") had hit, for a reason the doc named as the next look: REALTIME queue creation reaches `NVA06C_CTRL_CMD_MAKE_REALTIME`
and `NVA06F_CTRL_CMD_RESTART_RUNLIST`. `phigh` passing shows the policy and interleave controls were the whole HIGH path.

Both are `ROUTE_TO_PHYSICAL | NON_PRIVILEGED`, access right `NICE` (flags `0x48`), params `{ NvBool bRealtime }` (1 byte) and
`{ NvBool bForceRestart; NvBool bBypassWait }` (2 bytes). Real effect (`ctrla06c.h:367-401`, `ctrla06fgpfifo.h:176-214`):
promote a TSG to realtime (first in its runlist; every other channel gets CTA compute preemption) and restart the runlist,
preempting the running channel. **Decision:** the same as for the policy: accepted as the VM's own virtual state, never
forwarded. `MAKE_REALTIME` records one boolean, `RESTART_RUNLIST` is acknowledged and counted; nothing is promoted, no
preemption mode changes, no runlist restarts, nothing is preempted on the host. The cost is the same cost: a "realtime" guest
queue is not preferred, and the preemption the guest asked for does not happen. They order scheduling, never data, and no work
is completed here (the guest's work still runs on its host twin and completes when the GPU completes it), so accepting them
cannot make a result wrong. `NvBool` is a byte RM tests for non-zero, so any byte is a boolean; only the size is checked.

### What the guest REPORTS: a harness artifact, not a kayfabe answer (inferred)

The 4090 `gap_vkprio` guest run reported `MEDIUM` only on every family and returned `NOT_PERMITTED` (-1000174001) for HIGH and
REALTIME, while the host reported LOW..HIGH (families 0, 2, 5: ..REALTIME). The NVIDIA driver gates HIGH/REALTIME on the calling
process's `CAP_SYS_NICE`; in the guest that is evaluated by the guest's own kernel (`osCheckAccess(RS_ACCESS_NICE)` ->
`capable(CAP_SYS_NICE)`, `ogkm-580: os-interface.c:395-398`); no RPC to kayfabe decides it. The harness (`ogr_hook2.sh`) ran the
CTS cases as `sudo bash run_apps.sh` but `gap_vkprio` as `$G /tmp/gap_vkprio`, i.e. over ssh as the guest's non-root `ubuntu`
user, while the host probe ran as root. **Falsifier:** `sudo gap_vkprio` in the guest still reports MEDIUM only; then look at what
the driver queries at enumeration. The earlier 3090 root probe reported per family exactly what the host reported, so **no
reported-capability difference is built**; whether the REALTIME set {0, 2, 5} on the 4090 matches in the guest as root is
unmeasured. Not served and still refused: the interleave GETs (the guest answers them), `UPDATE_CHANNEL_INFO`, everything else.

## The defect

**[measured]** (gap agent, RTX 3090 box, kf3 on the PR #480 binary; evidence `traces/gap_probes_run_20261118/` on
PR #488): in the guest `vkCreateDevice` with a non-default `VK_KHR_global_priority` queue priority fails with
`VK_ERROR_INITIALIZATION_FAILED` (-3) on every queue family, while the host passes and the guest reports the same
priorities. Probe `gap_vkprio`, family 0: LOW -3, MEDIUM 0, HIGH -3, REALTIME -3 (host: all 0). Four CTS cases
reproduce it alone in fresh boots (`api.device_init.create_device_global_priority_khr.basic`,
`..._query_khr.basic`, `synchronization.global_priority_transition.high...from_graphics_to_compute`,
`...realtime...from_compute_to_transfer`). In all four failing boots, and in none of the neighbour boot that ran only
info cases, the kf3 log has `GSP REFUSED fn76/0x20801115=0x56`.

**[inferred]** The refusal is the cause. The NVIDIA Vulkan driver is closed source, so no trace shows it calling the
control; the evidence is 4 of 4 against 0, and that LOW (below the default) fails the same way as HIGH, which fits a
policy that makes interleave levels matter. **Falsifier:** a guest with this change whose four cases still fail.

## What the control is (read from ogkm)

`NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY` (`0x20801115`), params `{ NvU32 flags; NvU32 schedPolicy }`, policy
values `DEFAULT` 0, `CHANNEL_INTERLEAVED` 1, `CHANNEL_INTERLEAVED_WDDM` 2, `flags` bit 0 `RESTORE`
(`ogkm-580: ctrl2080fifo.h:702-793`; identical in 595.84). The header says it sets the **global** scheduling policy of
all runlists of the GPU, that other clients are then refused a different policy until it is restored, and that only
privileged clients may set it. The export table row is flags `0x68` = `ROUTE_TO_PHYSICAL | NON_PRIVILEGED |
PRIVILEGED_IF_RS_ACCESS_DISABLED`, access right `RS_ACCESS_NICE` (`g_subdevice_nvoc.c:5020-5030`): the guest's own RM
checks `CAP_SYS_NICE` and sends it to GSP, which here is kayfabe. The implementation is not in the open tree (it is
GSP-side). There is **no `GET_SCHED_POLICY`** in the SDK.

Two more controls are unserviced today and **[inferred]** are reached next by the same driver path (the failing boots
stop at the first refusal, so they cannot show it): `NVA06C_CTRL_CMD_SET_INTERLEAVE_LEVEL` (`0xa06c0107`) and
`NVA06F_CTRL_CMD_SET_INTERLEAVE_LEVEL` (`0xa06f0109`), one `NvU32` level, LOW 0, MEDIUM 1, HIGH 2. The guest's RM
sends them to GSP before updating its own TSG state (`kernel_channel_group_api.c:1393-1409`, `kernel_channel.c:3238-3262`)
and answers their GETs itself (`:1431-1451`, `:3265-3275`), so no GET is ever sent to us.

## The decision: accept in the VMM as the VM's own virtual state

SET stores the request in kayfabe's per-VM state and answers `NV_OK`; nothing is forwarded; no host policy changes;
no GPU work is involved. Reasons, each checked against the rules:

- **Hostile guest.** A host-wide scheduling policy is shared by every VM and by the host's own clients; it must not be
  settable from guest bytes. The link holds no sink, handle or host client, so it cannot act on the host.
- **No GPU work, no forged completion.** The controls carry a policy word and a level word, not a channel, a preemption
  mode or a submission. Only controls of that kind are answered in the VMM.
- **Derive, never capture.** The ids, struct sizes and values are constants pinned to the ogkm headers of both oracle
  trees by `queue_priority_oracle.rs` (skips unless a tree is present; `KF_REQUIRE_ORACLES` makes absence a failure). They
  are not in the generated driver matrix, because regenerating it needs the 36-tag sweep and `pyelftools`; moving
  them into `tools/drivermatrix/consumed.txt` is the follow-up (the same shape as `vfguest.rs`'s ids).
- **Bounded.** State: a policy word, the last level, four saturating `u64` counters. Log: `rate_eprintln!`. Size checks
  precede any field read; arithmetic is checked.

### The cost, plainly

**Queue priorities are accepted but NOT honoured on the host.** Every guest queue schedules at the host default, as
under a cloud vGPU. A compositor, VR or low-latency app that asks for HIGH/REALTIME gets a working device and no
boost. Honouring them needs the host's global runlist policy and a per-VM share, which is the owner's 2026-10-08 "later
idea" (`docs/OWNER_RULINGS.md`, software-runlist bullets), not this change.

### Deliberate simplifications

- **Last writer wins.** The real control refcounts clients ("until restored or died") and answers `NV_ERR_INVALID_STATE`
  to a different policy. Here nothing is global, so there is nothing to protect, and a holder count would need the death
  of a guest client, which kayfabe does not observe: a leaked holder would bring `-3` back. RESTORE sets `DEFAULT`.
- Only bit 0 of `flags` is read; `schedPolicy` is ignored under RESTORE and must be 0..=2 otherwise
  (`NV_ERR_INVALID_ARGUMENT`). A wrong or lying size is `NV_ERR_INVALID_PARAM_STRUCT`; a FINN-serialized envelope is
  `NV_ERR_NOT_SUPPORTED`.
- Interleave SETs are not tied to an object (the guest already resolved the handle); only the last level is kept.

### Still refused, unchanged

~~`MAKE_REALTIME` and `RESTART_RUNLIST` stay refused as GPU work~~ *(superseded, see the correction above: they are served
virtually, because served virtually nothing reaches the GPU; the first cut's reasoning "they change preemption" was about the
real RM, and the virtual answer changes none).* Still refused: the interleave GETs, `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO`
(`0x20801116`) and everything else; the replay test `the_neighbours_stay_refused` pins that they reach the ledger.

## What the replay shows, and what it cannot

Shows (**[measured] by `cargo test -p kf-rm`**): through the whole served chain, with the channel and memory planes
seated and recording, the logged control (and each params shape the closed-source driver may send) is answered `NV_OK`
in the envelope and in the control header, nothing reaches the ledger, and no host statement is recorded; hostile
sizes, values and envelopes are refused by name; repeats do not grow the state. 25 mutants, each killed by at least two
named tests. Replay 2 (the REALTIME controls) is built from the logged ids of the 4090 refusals; it was red before the fix.

Cannot show: that the Vulkan driver then succeeds; which params it sends (all shapes are covered); that interleave
controls are what it calls next; timing.

## Hardware confirmation (not run; no hardware in this change)

Boot a guest with this build and run, host as the control: the four CTS cases above and the `gap_vkprio` probe
(`scripts/apps/gap_probes`, PR #488) **as root in the guest (`sudo`)**, like the CTS cases and the host. **Expectation:** all pass, `vkCreateDevice` 0 for LOW/MEDIUM/HIGH/REALTIME on
every family. **Falsifier:** still -3. Then capture the kf3 refused set again (`GSP REFUSED` lines) and look first at
the guest's `dmesg` and the kf3 refused set for any new id, then at any `0xa06c01xx`/`0xa06f01xx` or `0x2080111x` id that is new. Also check the
guest's `dmesg` for Xid lines: none are expected, because no host call is made.

## Gap-matrix row (for `docs/design/V3_GPUPV_GAP_MATRIX.md` on PR #488, which this base does not have)

| id | gap | status |
|---|---|---|
| vk-global-priority | guest `vkCreateDevice` -3 for LOW/HIGH/REALTIME queue priority; kf3 refused `fn76/0x20801115` (`FIFO_RUNLIST_SET_SCHED_POLICY`) | FIXED in code 2026-11-18 in two cuts (policy + interleave, then `MAKE_REALTIME` + `RESTART_RUNLIST` after the 4090 falsifier; `queuepriority.rs`, GPU-free replays + 25 mutants); hardware re-test pending; priorities accepted, NOT honoured on the host; the probe must run as root in the guest |
