**STATUS: LIVE (PRE-REGISTRATION, written and committed BEFORE any run), 2026-10-16.**
Branch `claude/setobj-20261016`. Question (owner): is a second `SET_OBJECT` on a software subchannel accepted or
rejected by the hardware / host RM? The Windows USER-ring lifecycle (`docs/design/V3_USER_RING_LIFECYCLE_20261016.md`,
`crates/kf-chan/src/user_scan.rs`) assumes the `5080` (`NV50_DEFERRED_API_CLASS`, an `ENG_SW` class) `SET_OBJECT`
binds ONCE per channel. Owner logic: a rejected second `SET_OBJECT` makes a rebind impossible by construction; an
accepted one proves only that the hardware allows it (not that the driver does it), and the app-matrix counters stay
the answer for the driver.

Rules kept: no reverse engineering of any closed binary; only public headers (open-gpu-kernel-modules 595.91.07
headers, `dev_ram.ref` / `dev_pbdma.ref` as cited by the design docs), the open-source RM, and our own raw program
(`scripts/bench/setobj/setobj_probe.c`, a plain C program over `/dev/nvidiactl`, the same style as
`scripts/bench/vasfmt_probe.c`; no kayfabe crate is touched). Labels: **MEASURED** = read from a run log of this
branch; **INFERRED** = reasoning or open-source reading. Nothing below is a result yet.

# 0. What the open source says (INFERRED, before any run)

* `deferred_api.c:676-680` (`Nv50DeferredApi`): the class `0x5080` software method table is `0x100..0x103`
  `mthdNoOperation` and `0x200..0x203` `_class5080DeferredApiV2` (looks `data` up in THAT object's table). Any other
  method on a bound 5080 object is outside its table.
* `kchannelRegisterChild_IMPL` (`kernel_channel.c:3340-3399`): every `ENG_SW` child of a channel gets the channel's
  next 16-bit `classID` (1, 2, ... never rolled back, unique per channel). `SET_OBJECT` for a software object names
  that number (`kchannelGetClassEngineID_GM107`, `kernel_channel_gm107.c:72-82`: `DRF_NUM(906F,_SET_OBJECT,_NVCLASS,
  classID)`), obtainable unprivileged via `NV906F_CTRL_GET_CLASS_ENGINEID` (`0x906f0101`). So a second 5080 object B
  on the same channel is distinguishable from A by its number; nothing in `chandesConstruct_IMPL` /
  `kchannelRegisterChild_IMPL` forbids a second `ENG_SW` child, so the RM alloc is predicted to succeed.
* The software-method service itself (PBDMA SW-method interrupt) runs in GSP firmware on this host (closed). The open
  tree therefore cannot tell what the firmware does with a second bind: that is exactly what is measured here, by
  behaviour only.
* Public mapping (`dev_ram.ref`/`dev_pbdma.ref`, cited in `user_scan.rs`): subchannels 0..4 go to hardware engines,
  5..7 to SW DEVICE dispatch; "SetObject is not required by engines; Host holds no subchannel object map".

# 1. The instrument and how each outcome is DETECTED

A fresh child process per (case, repetition) (new RM client, new TSG, new channel; one process = one fresh channel).
The process is named `setobj_probe` so a host Xid line carries `name=setobj_probe` and its PID. Channel: TSG
(`KEPLER_CHANNEL_GROUP_A`) on the GR engine + `AMPERE_CHANNEL_GPFIFO_A` (`0xc56f`) channel, GPFIFO / USERD /
pushbuffer / semaphore in vidmem mapped for the CPU, a channel error notifier (`hObjectError`), doorbell through the
`AMPERE_USERMODE_A` window. Engine objects: `ADA_COMPUTE_A` (`0xc9c0`), `ADA_A` 3D (`0xc997`). Software objects:
`NV50_DEFERRED_API_CLASS` (`0x5080`).

Detection (each reported per step; every step prints its own RM status; a missing observation is "unobserved", never
"fine"):
* **D1 progress**: after every submitted sequence a host-method semaphore release (SEMAPHOREA/B/C/D, release, 32-bit
  payload, a distinct payload per step) is polled by the CPU (timeout 3 s). Arrival = the channel executed everything
  before it. No arrival = the channel stopped or faulted.
* **D2 channel error notifier**: the `hObjectError` notifier word (`info32` / `status`) read after each step; a
  non-zero status or error number = RM robust-channel error.
* **D3 host journal**: `journalctl -k` (the dmesg ring is full and useless) between a start marker and the end of the
  run, filtered to `NVRM: Xid` lines with `pid=<child pid>` / `name=setobj_probe`. The runner records the pid, the
  journal cursor before and after, and the matching lines.
* **D4 RM status** of every alloc / free / control, and of a post-step `NVA06F`-family probe of the channel (a
  `GET_WORK_SUBMIT_TOKEN` and a doorbell write that must still be accepted).
* **D5 handler reached (the "software method reaches the object" oracle)**: register a deferred API on the object
  (`NV5080_CTRL_CMD_DEFERRED_API`, `0x50800101`, `DMA_INVALIDATE_TLB` of the probe's own VA space, flag
  `DELETE_IMPLICIT`), send method `0x200` with that handle on the subchannel under test, release a semaphore, wait,
  then `NV5080_CTRL_CMD_REMOVE_API` (`0x50800102`) on the SAME object: `deferred_api.c:653-670` deletes an executed
  implicit entry, so a failing REMOVE_API (`NV_ERR_INVALID_DATA`) = "executed by that object's handler" and an OK
  REMOVE_API = "not executed". The registration control is privileged in the open source (the registrant's privilege
  level is stored) and the harness runs as host root; this is used ONLY as the observation oracle. If the host
  refuses the registration the case reports "reach unobserved" (D1-D4 still apply) and nothing is inferred.
  Oracle self-test (case R0): handler reach on the first bind must read "executed" (positive) and a registered
  handle never triggered must read "not executed" (negative) before any reach reading is trusted.

**Positive control (S4, run FIRST, before any "no error" is trusted).** The harness must detect a fault:
* C-INVALID-SW: after a good 5080 bind on subchannel 6, method `0x300` (outside the 5080 table, `deferred_api.c:676-680`)
  on subchannel 6. Prediction: fault (RC / Xid, no semaphore). 
* C-INVALID-CLASS: `SET_OBJECT` with an invalid class number (`0xdead`) on hardware subchannel 1. Prediction: fault.
* C-OK baseline: `SET_OBJECT(compute)` on subchannel 1 then a semaphore. Prediction: no fault, semaphore arrives.
**If neither invalid control produces a detectable fault (D1..D3), no "accepted" below counts, and the report says so.**
Each control must be seen to fault in THIS harness, on a fresh channel, before S1..S3 run.

# 2. Cases, predictions, FALSIFIERS (3 repetitions each, each in a fresh channel)

Notation: `SO(sub, v)` = method `SET_OBJECT` (offset 0) on subchannel `sub`, data `v` (`v` = the object's classID
number for software objects, the class id for engine objects); `SW(sub, m, d)` = software method `m` with data `d`
on `sub`; `SEM(n)` = semaphore release of payload `n` (host methods `0x10..0x1c`).

* **S4 (control)**: see above, plus S4d: `SO(1, compute)`, `SEM`, `SO(1, compute)` again, `SEM`; and S4e:
  `SO(1, compute)`, `SO(2, compute)` (the same engine object on two hardware subchannels), `SEM`.
  Prediction: S4c/S4d/S4e accepted, no fault (hardware subchannels hold no object map, engine class state is shared).
  FALSIFIED if S4d/S4e fault (then even a hardware re-send is rejected and the controls need re-reading).
* **S1 (re-send the same object)**: allocate A (`0x5080`); `SO(6, A)`; `SW(6, 0x100, 0)`; `SEM(1)`; wait;
  `SO(6, A)` again (identical); `SW(6, 0x100, 0)`; `SEM(2)`; wait. Prediction: accepted, no channel error (idempotent).
  **FALSIFIED if the channel faults (Xid / PBDMA interrupt / RC / error notifier / no `SEM(2)`) after the second
  `SO`.** Reach (D5) is read with a registered handle sent with `SW(6, 0x200, H)` after the second bind.
* **S2 (a second 5080 object, after the channel ran work; owner's order)**: (1) allocate A; `SO(6, A)`; short
  sequence + `SEM(1)`; WAIT. (2) THEN allocate B (`0x5080`) on the SAME channel; record the exact RM status of the
  alloc (the kayfabe relay currently assumes the object cannot be allocated after the first publication, so this
  status is a result in itself). (3) if B exists: `GET_CLASS_ENGINEID(B)`; `SO(6, B)`; `SW(6, 0x100, 0)`; `SEM(2)`; wait.
  Prediction: the alloc succeeds (open source forbids nothing; B gets the next classID) and the bind is accepted.
  **FALSIFIED if the alloc is rejected or the channel faults; the report says which and at which step.**
  Variants, each in a fresh channel:
  * **S2b**: after step (1) free A first, then allocate B, then `SO(6, B)`. (classID of B is expected to be 2, not
    reused: the counter is never rolled back.)
  * **S2c**: `SO(7, B)` (or 5) on a DIFFERENT software subchannel while A stays bound on 6.
  * **S2d**: allocate B BEFORE the first submission; `SO(6, A)`; run work; then `SO(6, B)`.
  Per variant: RM status of every alloc / free, whether the channel faulted, and (D5) whether a `0x200` method on the
  subchannel reaches the intended object (A or B) rather than the other.
* **S2e (the same object bound on a second software subchannel; coordinator addition)**: fresh channel: allocate A;
  `SO(6, A)`; work + `SEM`; wait; then `SO(7, A)`; `SW(7, 0x100, 0)`; `SW(7, 0x200, H)`; `SW(6, 0x200, H2)`; `SEM`; wait.
  Repeat in another fresh channel with subchannel 5 as the second one. Also try `SO(k, A)` for a hardware subchannel
  k in 0..4 (record accepted / rejected / ignored). Prediction (coordinator ~85 %, unmeasured): accepted, and the
  method on the second subchannel reaches A's handler. **FALSIFIED if the second bind faults or is rejected.**
  If accepted, the lifecycle's Pointer mode cannot see a second binding of an object it already knows.
* **S3 (an engine class on a software subchannel)**: `SO(6, compute)` (and 5, 7; also the 3D class), then the
  smallest legal compute-class method that is observable, then `SEM`. Prediction (public mapping): rejected or ignored
  by hardware because 5..7 are SW-dispatched. **FALSIFIED if accepted and executes engine work there** (observable
  as: an engine method (compute `NO_OPERATION` / a compute semaphore release by the class) takes effect and the
  channel keeps running without fault). If the result is "ignored" (no fault, no engine effect) the report records it
  as ignored, not as rejected.

# 3. What the verdicts will say

Per case S1 / S2 / S2b / S2c / S2d / S2e / S3, exactly one of:
* "second SET_OBJECT is rejected by hardware/RM (rebind impossible by construction for case X)", with the fault
  evidence (D1-D3) and its positive control; or
* "accepted: the hardware allows a rebind; the question stays with the app-matrix counters", with the semaphore
  evidence and, where D5 worked, which object's handler a method reached; or
* "UNOBSERVED / INCONCLUSIVE" with the missing instrument named (e.g. the positive control did not fault).
Acceptance is MEASURED only as: no D2 error, no D3 Xid for the pid, D1 semaphore arrived, subsequent doorbell accepted.
"The driver does it" is never inferred from these results.

If a step fails twice for the same reason the work stops and the failure is reported. The GPU lock
(`flock -o /tmp/kayfabe-fastguest.lock`) is held per job (< 5 min); no process not started by this work is touched.
