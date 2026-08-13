# The error-delivery rung — the emitter is BUILT AND ORPHANED, and the attribution probe carries no identity

**STATUS: LIVE — 2026-08-13 (w288 gate).** Read-only. No code, no boot. Answers the two gates the
rung brief put ahead of construction, and reaches the brief's own **stop condition** on the
second.

---

## 0. ⊘⊘⊘ THE RESEARCH BRANCH I WAS TOLD TO PULL DOES NOT EXIST

> *"Research branch `w288-ogkm-authored-user-visible-structures` has the full write-up; pull it."*

```
$ git fetch origin w288-ogkm-authored-user-visible-structures
fatal: couldn't find remote ref w288-ogkm-authored-user-visible-structures
$ git branch -a | grep -i w288      → (nothing)
```
Newest refs on `origin` are `w287-*`, `w286-*`, `w285-*`. ⇒ **Nothing below is corroborated by
that write-up**; it is derived from this tree and from `ogkm-580.159.04` directly. Where my
conclusion differs from the brief's, that is why.

---

## 1. ★★★ HALF THE RUNG IS ALREADY BUILT — AND IT IS AN ORPHAN

The brief's step 2 is *"write slot 0 of the guest's channel notifier, at the guest-physical
address the guest already gave us, in the format ogkm's own writer uses."* **That is written.**
`crates/kayfabe-rmrpc/src/fault.rs`:

- `pub struct FaultEmission { … pub notifier_gpa: u64, … }`
- `pub fn deliver(…)` — *"Write the notifier, then post the event"*, with the ordering sourced
  to RM's own reason (*"GSP writes to notifiers to …"*).
- It **writes guest RAM at `notifier_gpa`**, in two parts (`ram.write(self.notifier_gpa, …)`
  then `+ split`), so a straddling notifier is handled.
- It encodes `ErrorNotification` from `kayfabe_abi::notifier`, carrying *"the SAME two numbers
  the event routes on — `exceptType` …"*, and it already reasons about the `NvU32`-vs-`NvV16`
  engine-width hazard between the RC event and the notifier.
- Its refusal vocabulary already distinguishes *"guest RAM refused the notifier write, nothing
  posted"* from *"notifier written, transport refused the event"*, and names the second as the
  safe half.

**And nothing calls it.**

| symbol | references outside its own file |
|---|---|
| `FaultEmission` | **none** — and it is **not** in `lib.rs`'s `pub use`, so it is unreachable from any other crate |
| `rc_triggered_for` | re-exported (`lib.rs:197`) — **zero call sites** |
| `ErrorNotification` | used only inside `fault.rs` / `notifier.rs` |

⇒ **The rung is WIRING, not building** — and this is the orphan gate's known shape: *the gate
asks visibility, not reachability*. `FaultEmission` is `pub` within its crate, so it does not
read as dead, while being callable by nobody.

⚠ ⇒ **Step 1 (consume the host RC event) is the only genuinely unbuilt half.** `rmladder.rs`
knows the ioctl *names* (`ALLOC_OS_EVENT` = 206, `RM_GET_EVENT_DATA` = 0x52) as census labels
only; there is no consumer.

---

## 2. ⊘⊘⊘ THE ATTRIBUTION GATE — THE PROBE CARRIES NO CHANNEL IDENTITY. **STOP CONDITION REACHED.**

The brief: *"One probe is justified and is unprivileged AND per-channel:
`NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` (`0x906f0106`) — exactly the identity the RC event lacks.
… If it does not carry the identity, stop and report."*

The parameter struct, `ogkm-580.159.04`,
`src/common/sdk/nvidia/inc/ctrl/ctrl906f.h:213-219`:

```c
typedef struct NV906F_CTRL_GET_MMU_FAULT_INFO_PARAMS {
    NvU32 addrHi;
    NvU32 addrLo;
    NvU32 faultType;
    char  faultString[32];
    NV_DECLARE_ALIGNED(NvU64 shaderProgramVA[7], 8);
} NV906F_CTRL_GET_MMU_FAULT_INFO_PARAMS;
```

⇒ **No `chid`. No handle. No runlist. No engine. Not one identity field.** The reply is a pure
fault *description*: where, what type, a string, and shader VAs.

### ★ The identity is the SUBJECT of the call, not a field in the reply — and that is a different mechanism with a different cost

`kchannelCtrlCmdGetMmuFaultInfo(pKernelChannel, pFaultInfoParams)` is dispatched **on a channel
object**, so "whose fault" is answered by *which channel you ask*, not by what comes back. That
is attribution — but only by **polling candidate channels**, and the header states the cost in
its own words:

> *"This command returns MMU fault information for a given channel. **The MMU fault information
> will be cleared once this command is executed.**"*

⇒ **The read is DESTRUCTIVE.** Polling channel A to ask *"was it you?"* **consumes** A's record.
If we guess wrong about which channels to poll, or poll in the wrong order, we destroy the
evidence for the channel that actually faulted — and the brief's own constraint (*"a wrong
channel's notifier is worse than none"*) then applies to a record we can no longer re-read.

⇒ **This is a design decision for the owner, per the stop condition.** The options are visible
but each has a cost worth ruling on: poll every live channel on every RC event (destructive,
O(channels), racy); poll only channels with work in flight (needs a liveness notion we would
have to *add*, which is the forbidden direction); or find identity somewhere other than this
control.

### ⚠ AND THE HAL RULE APPLIES — the fill code is CLOSED, verified with a known-positive

Applying the rung's own new instrument rule (*"the `.c` you read is often NOT the code that
runs"*): **`kchannelCtrlCmdGetMmuFaultInfo_IMPL` has no body anywhere in the open tree.** It
appears only in generated nvoc dispatch (`g_kernel_channel_nvoc.c:299`, `.h:711/1391`).

⊘ **Known-positive for the grep**, because an absent artefact reads as favourable: the identical
search shape *does* find a sibling's body — `kchannelCtrlCmdGetClassEngineid_IMPL` at
`src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2923`. So the tool works and the absence is
real.

⇒ Whatever fills this struct is **closed firmware**, exactly as the brief anticipated. ⚠ Which
means the **contents** must still be measured even though the **shape** is now known — but the
shape is already enough to answer the gate, because a field that does not exist in the header
cannot be returned by the firmware.

★ One incidental fact, recorded because it is easy to over-read: `0x906f0106` is listed in
`rmapiRmControlCanBeBypassLock` (`rmapi_specific.c:189`), so it bypasses the RM lock. That is
consistent with a cheap fault-info read; it says **nothing** about GSP routing or liveness.

---

## 3. WHAT I DID NOT DO, AND WHY

- **Did not build the probe arm.** The gate it exists to answer is answered by the struct
  definition, and the brief's instruction on that answer is *stop and report*.
- **Did not wire `FaultEmission`.** Wiring it needs an attribution rule (§2) — writing slot 0 on
  a channel we cannot show faulted is precisely *"a wrong channel's notifier is worse than
  none"*.
- **Did not run `--ce-client-fault` or `cup2`.** Both pass criteria are downstream of the
  attribution ruling; running them now would grade a mechanism that is not yet decidable.

## 4. THE SMALL ONE, unchanged and still ready

The kind gate: `internalFlags` is decoded at `kayfabe-abi/src/notifier.rs:171/184` (offset 244
on `580.159.04`, read at `:249`) but **only bits `[3:2]`**, for `ErrorNotifierType`. Bits
`[1:0]` never reach `ChannelFacts`, so `ProcBoundary::channel_kind`
(`kayfabe-core/src/project.rs:311`) falls back to `anchor == SYSTEM_ANCHOR` — a proxy for the
owner's ruled discriminator. Carrying two already-decoded bits into the projection is the whole
job; it is not new tracking, and it is the likely cause of w287's `SEMA-SOURCE-CE = 1` where the
passthrough cut should have given `0`.
