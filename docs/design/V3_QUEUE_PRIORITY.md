# V3 queue priority: `VK_KHR_global_priority` in the guest

**STATUS: LIVE, 2026-11-18 (implemented and GPU-free tested; hardware confirmation NOT yet run).** Owner ruling
2026-11-18: a known kayfabe bug is fixed, and every kayfabe-core failure gets a GPU-free Rust reproduction first
(`docs/design/V3_REPLAY_TESTS.md`). Code: `crates/kf-rm/src/queuepriority.rs`. Replay:
`crates/kf-rm/tests/queue_priority.rs` (fixture `tests/fixtures/queue_priority_refusal.txt`). Oracle:
`crates/kf-rm/tests/queue_priority_oracle.rs`. Mutants: `scripts/ci/queue_priority_mutants.py`.

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

`NVA06C_CTRL_CMD_MAKE_REALTIME` (`0xa06c0110`) and `NVA06F_CTRL_CMD_RESTART_RUNLIST` (`0xa06f0111`) change GPU
preemption modes and restart a runlist: GPU work. `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO` (`0x20801116`) is another
control. the replay test `the_neighbours_stay_refused` pins that they still reach the ledger.

## What the replay shows, and what it cannot

Shows (**[measured] by `cargo test -p kf-rm`**): through the whole served chain, with the channel and memory planes
seated and recording, the logged control (and each params shape the closed-source driver may send) is answered `NV_OK`
in the envelope and in the control header, nothing reaches the ledger, and no host statement is recorded; hostile
sizes, values and envelopes are refused by name; repeats do not grow the state. 18 mutants, each killed by at least two
named tests.

Cannot show: that the Vulkan driver then succeeds; which params it sends (all shapes are covered); that interleave
controls are what it calls next; timing.

## Hardware confirmation (not run; no hardware in this change)

Boot a guest with this build and run, host as the control: the four CTS cases above and the `gap_vkprio` probe
(`scripts/apps/gap_probes`, PR #488). **Expectation:** all pass, `vkCreateDevice` 0 for LOW/MEDIUM/HIGH/REALTIME on
every family. **Falsifier:** still -3. Then capture the kf3 refused set again (`GSP REFUSED` lines) and look first at
`0xa06c0110` and `0xa06f0111`, then at any `0xa06c01xx`/`0xa06f01xx` or `0x2080111x` id that is new. Also check the
guest's `dmesg` for Xid lines: none are expected, because no host call is made.

## Gap-matrix row (for `docs/design/V3_GPUPV_GAP_MATRIX.md` on PR #488, which this base does not have)

| id | gap | status |
|---|---|---|
| vk-global-priority | guest `vkCreateDevice` -3 for LOW/HIGH/REALTIME queue priority; kf3 refused `fn76/0x20801115` (`FIFO_RUNLIST_SET_SCHED_POLICY`) | FIXED in code 2026-11-18 (`queuepriority.rs`, GPU-free replay + 18 mutants); hardware confirmation pending; priorities accepted, NOT honoured on the host |
