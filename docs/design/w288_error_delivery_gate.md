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

## 2a. ⊘ THE RESEARCH DOC EXISTS — IN THE **C** REPO — AND IT CORROBORATES §1–§2

`w288-ogkm-authored-user-visible-structures` @ `c896ed4` is on the origin of
**`/workspace/nvidia-gpu-passthrough`**, not kayfabe:
`docs/reference/ogkm_authored_guest_userspace_structures.md`. Its §10.1 gives the consume
sequence end to end, and its §10.2 states the same gap I reached independently: *"we get the Xid
but **not which channel**, and we are woken by other tenants' RC errors too."*

## 2b. ★★★ STEP 1 IS **100 % UNBUILT AT THE IOCTL LAYER** — and it needs an IPC crossing the brief does not mention

Measured over `kayfabe-linux-raw` and `rm.rs`: **zero** occurrences of `ALLOC_OS_EVENT`,
`GET_EVENT_DATA`, `EVENT_SET_NOTIFICATION`, or `NV01_EVENT_OS_EVENT`. The only hits anywhere are
the two **census label strings** in `rmladder.rs` (`0x52 => "RM_GET_EVENT_DATA"`,
`206 => "ALLOC_OS_EVENT"`), which are a decoder's names for numbers, not a client.

What wiring actually requires, from §10.1:

1. `NV_ESC_ALLOC_OS_EVENT` and `NV_ESC_RM_GET_EVENT_DATA` (`0x52`) bindings + their param
   structs — new to `kayfabe-linux-raw`.
2. `NV01_EVENT_OS_EVENT` (`0x79`) allocated on an `NV20_SUBDEVICE_0`.
3. `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION(event=37, action=REPEAT)`.
4. An event fd and a `poll()` loop — i.e. a **blocking waiter**, which is a lifecycle this
   backend does not currently have.
5. ⚠ **A PROCESS CROSSING.** The RC event arrives in the **isolate host**, which holds the host
   `/dev/nvidia*` fds. `FaultEmission::deliver(ram, fsm)` needs **`GuestRam` and the `GspFsm`**,
   which live in the **VMM/shim**. Those are different processes. ⇒ Wiring is not "call
   `deliver()` from the event handler"; it is a new isolate→VMM message carrying the fault, on
   the IPC that already exists for descriptors.

⇒ The rung is *"wire it, don't rewrite it"* for step 2, and a genuine multi-crate build for
step 1. I am not starting it inside this task's remaining budget, because a **half-wired
delivery path that boots is exactly the artefact this campaign refuses**: it would produce a
`CUP2_RC` that cannot distinguish *"the hypothesis is wrong"* from *"our emitter never fired"*,
and the rung's whole value is that the refutation be clean.

## 2c. ⚠ A CORRECTION TO THE UNBLOCK'S REASONING — "one channel" is true of the CLIENT, not of the GPU

> *"The raw faulting client has ONE channel. Attribution is trivial there — there is no candidate
> set to poll."*

True of the client; **not** of the fault source. The RC event is **GPU-scoped**
(`gpuNotifySubDeviceEvent_IMPL` *"iterates all subdevice back-references and never consults
`pKernelChannel`"*), and `[measured, w287 boot 2]` the same boot carried **six** `GR-BIRTH`
channels — the guest driver's own, plus the client's. ⇒ On an RC event we learn *a* fault
happened **on this GPU**, and the client's channel is one of several that could have produced it.

★ For `--ce-client-fault` this is a **caveat, not a blocker**: the client provokes the fault
deliberately, so it is the overwhelmingly likely source, and the arm is still worth running.
⊘ But it is **not** trivial in the sense of *"nothing else could have faulted"*, and the
falsifier must be stated up front: **if we deliver on any RC event, a fault from the guest
driver's own channel writes the client's notifier — a false positive that looks exactly like a
pass.** Under the standing ruling that is the wrong-channel case, at GPU scope.

⚠ And §10.2 closes the obvious escape: the **channel-scoped** event fires *"only if
`hObjectError` resolves as a `NV01_CONTEXT_DMA`; a `Memory` handle silently never fires it"* —
and w286's census measured **all 32 USER channels as `TYPE_MEMORY`**. ⇒ For a guest *userspace*
client, the per-channel correlation event **cannot fire as things stand**.

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

---

## 5. ★★★ CHECK 4 (the one most likely to kill it) — ANSWERED. **IT SURVIVES, ON AN INVARIANT NOTHING ASSERTS.**

> *"Does a fault on channel A ever write channel B's notifier (e.g. TSG-wide or engine-wide RC)?"*

**YES — for exactly the fault class this rung chases.** The MMU-fault handler hardcodes TSG
scope (`kern_gmmu_gv100.c:2124-2131`):

```c
// Update the per-channel error notifier before performing the RC
rmStatus = krcErrorSetNotifier(pGpu, GPU_GET_KERNEL_RC(pGpu), pKernelChannel,
    ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT, rmEngineType,
    RC_NOTIFIER_SCOPE_TSG);          // ← not RC_NOTIFIER_SCOPE_CHANNEL (= 0), which exists
```

and `krcErrorSetNotifier_IMPL` (`kernel_rc_notification.c:270-289`) widens on that scope:

```c
if (scope == RC_NOTIFIER_SCOPE_TSG && kfifoIsSubcontextSupported(pKernelFifo) &&
    !pGpu->getProperty(pGpu, PDB_PROP_GPU_IS_VIRTUALIZATION_MODE_HOST_VGPU))
    pChanList = pKernelChannel->pKernelChannelGroupApi->pKernelChannelGroup->pChanList;
else  { …single-element list containing only pKernelChannel… }

for (pChanNode = pChanList->pHead; pChanNode; pChanNode = pChanNode->pNext)   // writes EACH
```

⇒ One MMU fault writes the error notifier of **every channel in the faulting channel's TSG**.
*"Whichever of our pages was written names the channel"* is **false in general.**

### ★★ Why it survives anyway — and why that is now load-bearing

`HostRmBackend::alloc_channel_in` allocates a **fresh `CHANNEL_GROUP` per channel**
(`rm.rs:5120-5122`, `raw_alloc(self.conn.device, …, CHANNEL_GROUP, …)`). Every host channel we
create is therefore **alone in its own TSG**, so the TSG channel list has exactly one member and
**TSG scope collapses to channel scope**. Attribution *is* structural — **because of a property
of our allocator, not a property of RM.**

⊘⊘ **That invariant is currently true by construction and is asserted NOWHERE.** It is also
exactly what a real driver stops doing: TSGs exist to group channels, and any future work that
puts two host channels in one group — subchannels, multi-engine contexts, a TSG reused for
cheapness — makes one fault write **both** notifiers. The guest then sees an RC on a channel
that never faulted: **a false positive shaped exactly like a pass**, and the wrong-channel case
the standing ruling forbids.

⇒ **Proceed, with a gate, not a comment:** the per-channel notifier design must be accompanied by
an assertion that the host channel's TSG contains exactly that channel, refused **by name** if
not. That is a check on our own allocation, not new tracking of the guest.

⚠ Two conditions gate the widening and neither rescues us in general: `kfifoIsSubcontextSupported`
(unverified for GA106 — but if it is *false* the `else` branch is taken and scope is per-channel
anyway, so the design is safe in both settings **only while our TSGs are singletons**), and
`PDB_PROP_GPU_IS_VIRTUALIZATION_MODE_HOST_VGPU`, which we are not.

### Status of the other three checks

| check | state |
|---|---|
| 1. does the notifier **write** need `NV01_CONTEXT_DMA`? | ⊘ **NOT CHECKED.** The `CONTEXT_DMA` requirement is established for the correlation **event** only. Must be resolved with the HAL rule before building. |
| 2. host RM writes a per-channel notifier we supply | ★ partly — `krcErrorSetNotifier_IMPL` demonstrably walks a channel list and writes per channel; the exact store and any error-class variation still to be cited. |
| 3. cost | one notifier page per host channel. `[measured, w287 boot 2]` six channels in a `cup2`-class boot ⇒ six pages. **Not prohibitive.** |
| 4. group scope | ★ **ANSWERED ABOVE — survives on the singleton-TSG invariant.** |
