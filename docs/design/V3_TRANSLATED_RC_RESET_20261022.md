**STATUS: RESEARCH, 2026-10-22 — reset refusal accepted; Linux negative control in progress.**

CODE READ owner decision, 2026-10-22: preserve RESET_CHANNEL refusal until recovery
proof exists. Option 1 below is accepted; option 2 is not selected. The reset
question is parked and is not a prerequisite for implementing the error-delivery
negative control. The control must check error delivery without claiming recovery.
The historical blocked runbook below is superseded by
`V3_TRANSLATED_RC_CONTROL_20261022.md` as that implementation progresses.

CODE READ: follow-up to PR #71, based on commit
`1c6452a276d7014bece3adf42d68cbd4edffbc18`; branch
`codex/translated-rc-reset-20261022`. The error-observation implementation in
`V3_TRANSLATED_RC_20261022.md` remains the first half. This follow-up changes no
runtime behavior. Hardware runs in this follow-up: n=0.

INFERRED pre-inspection hypothesis: RESET_CHANNEL's permission flag may permit
recovery of an owned host channel. Falsifier stated before inspection: the
implementation requires privileged handling or cannot confine recovery to that
channel. CODE READ: the export is unprivileged; confinement is unresolved below.
The additional post-reset FIFO/pending-work questions also remain open.

CODE READ source: NVIDIA open-gpu-kernel-modules 580.159.04, commit
`b81d58ee0224d1d290bef1c080592b619e184042`, MIT; paths below are relative to that
public repository, not firmware or reverse-engineered material.

- CODE READ: [ctrl906f.h](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/580.159.04/src/common/sdk/nvidia/inc/ctrl/ctrl906f.h#L94-L148)
  specifies `NV906F_CTRL_CMD_RESET_CHANNEL` (`0x906f0102`), including engine reset.
  Parameters are engineID, subdeviceInstance, resetReason and one-byte
  bIsRcPending. External reasons are 0..2. The listed success status is NV_OK.
- CODE READ: [g_kernel_channel_nvoc.c](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/580.159.04/src/nvidia/generated/g_kernel_channel_nvoc.c#L265-L280)
  exports this control with flags `0x10008` (NON_PRIVILEGED), accessRight zero.
  **There is an unprivileged verb. No privileged-only requirement was found.**
- CODE READ: [kernel_channel.c](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/580.159.04/src/nvidia/src/kernel/gpu/fifo/kernel_channel.c#L3036-L3083)
  validates external resetReason, replaces bIsRcPending with the CPU-RM channel's
  shadow fact, then invokes physical RM through RPC/internal control. That body
  neither defines FIFO cursor restoration nor clears this pending flag on success.
  The physical reset implementation is not present in this public source tree.
- CODE READ: [ctrl506f.h](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/580.159.04/src/common/sdk/nvidia/inc/ctrl/ctrl506f.h#L60-L117)
  separately defines RESET_ISOLATED_CHANNEL (`0x506f0105`) and its internal
  physical-RM form (`0x506f0106`). The public form is also exported unprivileged.
  `kernel_channel.c:2997-3030` clears the pending shadow after its success. It is
  not evidence that substituting it fulfills RESET_CHANNEL's engine-reset promise.

CODE READ blocker: the requested affected-channel-only recovery cannot be derived
from an interface that also promises engine reset. Public CPU-RM code does not
describe whether this can affect other channels, which private FIFO entries remain
executable, how outstanding failed work is disposed of, or when a new error record
may be armed. B1 marks the errored slot dead and retains outstanding work; merely
returning NV_OK or clearing that bit would invent the missing recovery contract.
No host reset was issued and no success reply, completion, or notifier clear was
added. This is a scope/lifecycle question, **not a missing unprivileged verb**.

CODE READ guest expectations: `kernel_rc_callback.c:398-421` distinguishes ignore,
isolate-without-reset and isolate actions; its GSP isolate path calls the separate
RESET_ISOLATED_CHANNEL control. `kernel-open/nvidia-uvm/uvm_channel.c:2058-2082`
returns NV_ERR_RC_ERROR for a nonzero error notifier (with an ECC override).
INFERRED: these facts do not establish that refusing RESET_CHANNEL makes every
Linux client tear down, or that Windows necessarily takes TDR rather than another
path. Windows' relevant driver implementation is not public. The historical
Blackwell timeout/0x116 evidence in `V3_BLACKWELL_CONTINUATION_20261022.md` is not
a recovery result for this branch. Teardown versus TDR remains unverified.

CODE READ stop/options under the lane AGENTS.md instruction to stop on a conflict:
1. Preserve refusal and prove isolation, cursor disposition, pending-work handling
   and notifier rearming before adding a success path (recommendation).
2. Clarify that “affected channel only” permits the documented engine reset via
   the owned twin; then design the outstanding-work lifecycle explicitly and test
   it before enabling that control. An owned handle alone does not prove recovery.

CODE READ hardware-arm placement: new support belongs in `kf-harness` and
`scripts/fastguest`, not frozen `kayfabe-rm-ladder`. `kf-rm/src/chanlink.rs`
`kernel_channel()` requires the CPU-RM kernel stamp or an internal RM client.
A normal Linux userspace raw client gets a Passthrough route. Its invalid stream
alone would not exercise a Translated slot. `kf-chan/src/translated.rs`
`rewrite_counted()` refuses an undecodable header before host submission; do not
weaken it just to manufacture a hardware error. Public `clc56f.h:300-308` names
SEC_OP_RESERVED6, allowing a candidate host-authored invalid word `0xc0000000` in
a private **bench** ring. An Xid32 outcome for that word is a hypothesis, not a
hardware result or an established interrupt signature.

INFERRED proposed host runbook, **blocked before fault injection; no executable
new arm has been implemented**:
1. On the trusted Linux host, use only the lane directory and the documented
   revision-specific binaries. Acquire `flock -w 600 -o
   /tmp/kayfabe-fastguest.lock` for the complete native/guest run; a timeout is a
   failed reservation, never permission to break the lock. Record source revision,
   host/guest driver versions, GPU PCI identity and a kernel-journal cursor.
2. Implement/test the native `kf-harness` private-HostRing rig first: positive
   fence, armed private RC context/event, a bystander fence, then exactly one
   reserved-word submission. Require a real private record and OS event. Never
   report a modeled RcDelivery callback as a guest IRQ. Establish the actual Xid
   signature before approving any intentional-fault exception.
3. Before that submission print `FAULT_MARK=translated_rc_invalid_pb` with run id,
   process identity/start time, PCI identity, host token/chid, stream word and
   source revision. Freeze the exact Xid code/interrupt signature from the native
   rig. Accept only the marker-matched process/channel and exact record count
   inside the bounded journal interval. Any additional Xid fails the arm.
4. Implement a guest-kernel-origin client or a separate host-authored bench hook
   for one selected Translated slot. First prove its route and host-twin/guest-chid
   mapping. Require the guest's actual 16-byte notifier, actual RC event, no
   success completion, unrelated-channel progress before/after, and the real
   RESET_CHANNEL reply. The lifecycle decision above is a prerequisite to defining
   the expected reset verdict; do not make either success or refusal pass by default.
5. Stop cleanly, capture bounded text evidence under
   `traces/translated_rc_reset_20261018/`, verify owned processes exited and release
   the shared resource. Do not start another arm after an unmatched host fault.

INFERRED Blackwell plan: a merged-B1 ON arm remains optional on the next already
authorized lease. This memo authorizes no rental and asserts no hardware acceptance.
