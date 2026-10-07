# ogkm-authored structures in guest userspace VAs — and who else writes them

```
STATUS: LIVE — 2026-08-13 (w288)
SCOPE:  research_clones/ogkm-580.159.04 ONLY. ogkm is VERSIONED, not the spec;
        every HAL-dispatch claim below is re-derivable from src/nvidia/generated/*.
TARGET: GA106, bare-metal GSP client (IS_GSP_CLIENT true, IS_VIRTUAL false,
        no Confidential Compute, no MIG, no SR-IOV VF).
METHOD: read-only source analysis. NO boot, NO bench. Nothing was measured today;
        the only hardware datum quoted is w287's error-notifier observation.
KNOWN-POSITIVE: the error notifier. Every sweep below reproduced it (see §7).
```

---

## 0. THE DIRECT ANSWER

**YES — ogkm CPU-writes structures into memory that guest userspace maps and reads.**

**⊘ But NO structure carries a guest-side ADDRESS that the GPU subsequently reads.**
The stop-the-line condition named in the brief — *"an ogkm-authored structure carrying a
guest-side address that the GPU reads"* — **is NOT met** in ogkm 580.159.04. Every
ogkm-authored user-visible structure found carries **status codes, exception codes, engine
IDs, timestamps, counters, and two hardware IDs** — never a VA or a physical address that
an engine dereferences.

★★★ **The finding under the owner's re-aim is different and sharper:**

> **ONE SURFACE, TWO AUTHORS, BY DESIGN.** The channel notifier surface
> (`pKernelChannel->pErrContextMemDesc`, the object behind `hObjectError`) is an array of
> `NvNotification` (slot indices `nvos.h:2858-2861`: `_ERROR`=0, `_WORK_SUBMIT_TOKEN`=1,
> `_KEY_ROTATION_STATUS`=2, `__SIZE_1`=3). **Slot 0 (`_TYPE_ERROR`) is authored by the GSP.
> Slot 1 (`_TYPE_WORK_SUBMIT_TOKEN`) is authored by the guest's ogkm** — because, in NVIDIA's own
> words, *"GSP FW is not able to perform the notification … so it still needs to be handled
> by the client/guest outside the RPC"* (`kernel_channel.c:3320-3322`). The two authors are
> already split **inside one page**, and NVIDIA shipped it that way.

⇒ **For us that is good news, not bad.** We are the GSP. Slot 0 is *ours by architecture*,
not a second authority intruding on the guest's structure. See §3.

★ **And the one genuinely scope-divergent value is in slot 1, which is NOT ours:** the
**work-submit token** = `{runlistId, chId}` (`kernel_fifo_ga100.c:226-227`), computed by the
**guest's** ogkm from the **guest's** ChID, written into guest userspace, and then written by
guest userspace to the doorbell register that the **host** GPU decodes. VA-identity mapping
does not save this: the location is right, the *value* names a host channel that is not the
guest's channel. See §2.1.

---

## 1. THE RANKED LIST

Ranked by (contains an address/ID the hardware acts on) ∧ (a second authority can write it).

| # | Structure | ogkm CPU-writes | Address / ID? | GPU or host RM reads it? | 2nd author possible? |
|---|---|---|---|---|---|
| **1** | **`NvNotification[1]` — work-submit token** | **yes** | **ID: `{runlistId, chId}`** | **yes — indirectly, via the doorbell register** | no (ogkm is sole author) — but the **value is guest-scoped** |
| **2** | **`NvGpuSemaphore` — SW-method semaphore release** | **yes** | no (lookup key is a guest GPU VA) | **YES — the GPU acquires on these exact bytes** | **YES — CPU (ogkm) *and* the GPU write the same dwords** |
| **3** | **UVM semaphore pool payload** | **yes** | no (payload is a u32) | **YES — CE `semaphore_release` targets it** | **YES — CPU (UVM) *and* CE** |
| **4** | **`NvNotification[0]` — channel error notifier** | **NO on this target** (GSP writes it) | no | no (`method_notification.c:176-179`) | **YES — GSP *or* ogkm depending on path; §3** |
| **5** | **UVM tools event records** | yes | **`instancePtr` = raw HOST physical addr; `channelId`** | no | no |
| 6 | `NOTIFICATION.OtherInfo32` — ChID at channel birth | yes (legacy ctxdma only) | **ID: guest ChID** | no | no |
| 7 | Subdevice event notifier array | yes | no | no | no |
| 8 | UVM tools control page / counters | yes | no | no | userspace also writes it (untrusted by design) |
| 9 | UVM processor UUID table | yes (`copy_to_user`) | **host GPU UUIDs** | no | no |
| 10 | Key-rotation notifier (`NvNotification[2]`) | yes | no | no | CC-only, not on this target |

Rows 5, 9 and the test-gated PA disclosures are **information-flow** findings, not
correctness findings — they cannot break the engine, but they put host-scoped identifiers
into guest userspace.

---

## 2. THE FINDINGS, IN FULL

### 2.1 ★★★ Work-submit token in `NvNotification[1].info32` — the scope-divergent ID

**1 — Who writes it.**
`kchannelCtrlCmdGpfifoGetWorkSubmitToken_IMPL`,
`src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:3282-3349`. On a GSP client
`bIsVgpuRpcNeeded` is **false** (it requires `IS_VIRTUAL(pGpu)`), so ogkm takes both halves
itself:

- generates the token: `:3343` `kfifoGenerateWorkSubmitToken(...)` →
  `kfifoGenerateWorkSubmitTokenHal_GA100`, `kernel_fifo_ga100.c:226-227`:
  ```c
  val = FLD_SET_DRF_NUM(_CTRL, _VF_DOORBELL, _RUNLIST_ID, runlistId, val);
  val = FLD_SET_DRF_NUM(_CTRL, _VF_DOORBELL, _VECTOR,     chId,      val);
  ```
  with `chId = pKernelChannel->ChID` (`:199`) and `runlistId = kchannelGetRunlistId(...)` (`:223`).
- publishes it: `:3348` `kchannelNotifyWorkSubmitToken(...)` →
  `kchannelNotifyWorkSubmitToken_IMPL` `kernel_channel.c:4076-4093` →
  `kchannelUpdateNotifierMem` `kernel_channel.c:1852-1938`.
- **the store:** `kernel_channel.c:1930` calls `notifyFillNvNotification`, whose CPU stores are
  `src/nvidia/src/kernel/gpu/mem_mgr/method_notification.c:181-185` —
  `MEM_WR32(&pNotification->info32, Info32)` at `:182` carries the token.

★ The comment at `kernel_channel.c:3319-3323` is the vendor stating the authorship rule:
*"GSP FW is not able to perform the notification … so it still needs to be handled by the
client/guest outside the RPC."*

**2 — How userspace sees it.** The notifier surface is a client `Memory` (or legacy
`ContextDma`) named by `NV_CHANNEL_ALLOC_PARAMS.hObjectError`. Two mapping shapes, both real:
- RM-allocated: `NV_ESC_RM_ALLOC_MEMORY` (`arch/nvalloc/unix/src/escape.c:314`, mmap context
  auto-created at `:347`) or `NV_ESC_RM_MAP_MEMORY` (`escape.c:507`, `:529`) → `nvidia_mmap`
  CTL branch → `nvidia_mmap_sysmem` → `vm_insert_page`, `kernel-open/nvidia/nv-mmap.c:458`.
- Client-allocated: `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` → `RmAllocOsDescriptor`
  (`escape.c:205-286`) → `nv_register_user_pages` (`kernel-open/nvidia/nv.c:3357`). RM gets its
  own alias by `vmap()` in `nv_alloc_kernel_mapping` (`nv.c:3696-3760`) — the branch whose
  comment at `nv.c:3711-3717` names this exact case: *"For User allocated memory (like
  ErrorNotifier's)…"*.

**3 — Address / handle / ID?** **An ID.** `{runlistId, chId}` — a *hardware* channel
identifier, allocated by the guest's own `CHID_MGR`. It is **guest-scoped and must be
host-scoped for the doorbell to name the right channel.**

**4 — Does hardware read it?** Not out of this memory. Userspace reads it here and writes it
to `NV_VIRTUAL_FUNCTION_DOORBELL` (`usermode_base + 0x90`, cf.
`docs/design/mode2_doorbell_chid.md` §1) — and the GPU host **does** decode `{runlist, chid}`
there and schedules that channel. ⇒ the value reaches hardware; only the transport is a
register rather than a DMA read.

⊘ **What is new here.** `mode2_doorbell_chid.md` already documents the token's *structure* and
the chid-collision problem. It does **not** record that the token is also **deposited by ogkm
into a guest-userspace page** — i.e. the token is not only an ioctl return value. Anything that
rewrites the token on the ioctl reply path and stops there will leave a **stale guest-scoped
copy in the notifier page**, which some clients read in preference to re-issuing the control.

### 2.2 ★★★ `NvGpuSemaphore` — the one structure ogkm and the GPU BOTH write

**1 — Who writes it.** `semaphoreFillGPUVATimestamp`, `method_notification.c:549-643`; the
CPU stores are `:632-634`:
```c
MEM_WR32(&(pSemaphore->timeStamp.nanoseconds[0]), timeLo);
MEM_WR32(&(pSemaphore->timeStamp.nanoseconds[1]), timeHi);
MEM_WR32(&(pSemaphore->data[0]), ReleaseValue);
```
through `pDmaMappingInfo->KernelVAddr[subdeviceInstance]` (`:630`), bracketed by
`osFlushCpuWriteCombineBuffer()` at `:609` and `:640`.

Callers, both **SW-method** objects whose methods trap out of the guest's pushbuffer:
- `src/nvidia/src/kernel/gpu/timed_semaphore.c:653` — `tsemaRelease_KERNEL`
  (`GF100_TIMED_SEMAPHORE_SW`, class `0x9074`); it also fills a notifier at `:669`.
- `src/nvidia/src/kernel/disp/disp_sw.c:157` — `dispswReleaseSemaphoreAndNotifierFill`
  (`NV9072` display SW class, VBlank-triggered).

**2 — How userspace sees it.** A client surface mapped **both** ways: CPU via
`NV_ESC_RM_MAP_MEMORY`, GPU via `NV_ESC_RM_MAP_MEMORY_DMA` (`escape.c:624`). RM locates the CPU
alias by looking the **guest GPU VA** up in the client's DMA-mapping list —
`CliGetDmaMappingInfo(...)` at `method_notification.c:581-586`.

**3 — Address / handle / ID?** The *contents* are a release value + a GPU timestamp — no
address. ★ But the **lookup key is a guest GPU VA** supplied by the client. That matters for
the two-authority question: if the same object were handed to the host's RM, the host would
resolve the identical VA against **its own** client's mapping list.

**4 — Does hardware read it?** **YES.** It is a semaphore: a host/engine `SEMAPHORE_ACQUIRE`
polls these exact dwords. The write-combine flushes at `:609`/`:640` exist precisely because
the other reader is the GPU. ⇒ **This is a genuine two-authority structure — CPU-RM and the
GPU engine write the same bytes**, and it is the only such case in the RM half of ogkm.

⚠ **Reachability caveat, stated plainly:** `NV9074` and `NV9072` are timed-semaphore and
display SW classes. I found **no evidence they are on the CUDA/`cup2` path**, and I did **not**
establish that they are unreachable either. Treat as *present in the driver, unproven on our
workload.*

### 2.3 ★★★ UVM semaphore pool — the second dual-authority structure

**1 — Who writes it.** `kernel-open/nvidia-uvm/uvm_migrate.c:804`, in
`semaphore_release_from_cpu()`:
```c
WRITE_ONCE(*(NvU32 *)semaphore_cpu_va, semaphore_payload);
```
**2 — How userspace sees it.** `UVM_ALLOC_SEMAPHORE_POOL` (dispatch `uvm.c:1042`); the
`mmap()` of that range reaches `uvm_mem_map_cpu_user()` → **`uvm_mem.c:806`
`vm_insert_page(...)`**. This is the *only* caller of `uvm_mem_map_cpu_user` in the module.
**3 — Address / handle / ID?** The written value is a plain u32. ★ But note
**`uvm_mem.c:1177` `gpu_va = (NvU64)user_addr;`** — UVM installs the **guest user VA as the
GPU VA** for this allocation. Identity by construction; guest-scoped.
**4 — Does hardware read it?** **YES** — the same bytes are released by the CE via
`gpu->parent->ce_hal->semaphore_release(&push, semaphore_gpu_va, semaphore_payload)`
(`uvm_migrate.c:773`). CPU and CE are co-authors; userspace polls.

### 2.4 The error notifier (`NvNotification[0]`) — the known positive, and it INVERTS

**1 — Who writes it.** The stores are the same five at `method_notification.c:181-185`. The
values w287 observed trace to `kernel_rc_notification.c:335` (`0xffff` passed as
`notifierStatus`) and `src/common/sdk/nvidia/inc/nverror.h:49`
(`ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT = 31 = 0x1f`, landing in `info32`).

★★★ **But on this target ogkm does NOT write it — the GSP does.** Compile-time, not inferred:

```
src/nvidia/generated/g_kernel_rc_nvoc.h:280
  #define krcErrorSendEventNotificationsCtxDma(...) \
          krcErrorSendEventNotificationsCtxDma_FWCLIENT(...)
```
The HAL resolves **unconditionally** to `_FWCLIENT` in the open-kernel-module build. That
function is `kernel_rc_notification.c:364-435`; it asserts `IS_GSP_CLIENT(pGpu)` at `:382` and
its body writes **no notifier** — only `notifyEvents(...)` at `:422`. Its header comment
(`:353-361`) states the split: *"GSP writes to notifiers … This function actually sends those
notifications to the client."* Corroborated at `kernel_rc_notification.c:85-89`: the CPU
writer `krcErrorWriteNotifier_CPU` is *"called in GSP_CLIENT, vGPU and MONOLITHIC cases,
**except in the GSP_CLIENT path where GSP has already written to the notifiers**."*

**Residual CPU-RM writers of slot 0 — all real, none on the RC path of this target:**

| call site | trigger | live on GA106 bare-metal GSP? |
|---|---|---|
| `kernel_gsp.c:662` | `RC_TRIGGERED` RPC, **`bIsCcEnabled` only** | **no** (no Confidential Compute) |
| `kernel_gsp.c:745` | `kgspRcAndNotifyAllChannels` — GSP dead / off the bus | yes, but only when GSP is gone |
| `kernel_channel.c:3163` | `kchannelCtrlCmdSetErrorNotifier` — **client asks for it** (`NVA06F_CTRL_CMD_SET_ERROR_NOTIFIER`) | **yes** |
| `kernel_channel.c:1779` | channel stop → `ROBUST_CHANNEL_PREEMPTIVE_REMOVAL` | yes |
| `kern_gmmu_gv100.c:2127` | **non-replayable MMU fault** | **NO — see below** |
| `vgpu_events.c:185` | vGPU guest | no |

⊘ **`kern_gmmu_gv100.c:2127` looks like the fault path and is DEAD here.** It sits in
`kgmmuServiceMmuFault_GV100`; on GA106 the selected HAL is `kgmmuServiceMmuFault_GA100`
(`g_kern_gmmu_nvoc.c:1727`) which does delegate to `_GV100` for non-FLA faults
(`kern_gmmu_ga100.c:398`) — so far so live. It is reached only by draining
`pRmShadowFaultBuffer` in `kgmmuServiceNonReplayableFault_GV100` (`kern_gmmu_gv100.c:1997-2008`),
and **that buffer is never filled on a GSP client**:
`arch/nvalloc/unix/src/unix_intr.c:933-938` —
```c
if (IS_GSP_CLIENT(pGpu)) {
    // Non-replayable faults are copied to the client shadow buffer by GSP-RM.
    status = NV_OK;
    goto done;
}
```
and `kgmmuCopyMmuFaults_HAL` is the stub `_92bfc3` (`NV_ASSERT_PRECOMP(0); return
NV_ERR_NOT_SUPPORTED`, `g_kern_gmmu_nvoc.h:2561`) for everything except the VF variant
(`g_kern_gmmu_nvoc.c:1113-1127`).
⚠ This is exactly the *"a path that reads as live because it compiles"* trap; it was caught
only by resolving the nvoc HAL dispatch, not by reading the C.

**2 — How userspace sees it.** Identical to §2.1 — same surface, slot 0.
**3 — Address / handle / ID?** **No.** `info32` = Xid/exception code, `info16` =
`NV2080_ENGINE_TYPE`, `status` = completion code, plus a GPU timestamp. Consistent with w287's
measurement.
⚠ **Disambiguation — `info16` means two different things on two different surfaces, and the
names do not warn you.** On **this** surface (the `NvNotification`) it is the engine type:
`krcErrorWriteNotifier_CPU` passes `(NvU16)gpuGetNv2080EngineType(localRmEngineType)`
(`kernel_rc_notification.c:172`). On the **`NvUnixEvent`** delivered by
`NV_ESC_RM_GET_EVENT_DATA` (§10) it is `partitionAttributionId` — `rmEngineType` is explicitly
`// unused` at `kernel_rc_notification.c:443`. Same field name, same RC event, different meaning.
**4 — Does hardware read it?** No. `method_notification.c:176-179` states notifiers are not
read by the GPU; nothing in the tree contradicts it. RM reads back only its **own** watchdog
notifier, which is RM-internal and not user-mapped.

### 2.5 ★★ UVM tools event records — host-scoped identifiers into guest userspace

**1 — Who writes it.** `kernel-open/nvidia-uvm/uvm_tools.c:477`, in `enqueue_event()`:
`memcpy((char *)queue->queue_buffer + sn.put_behind * entry_size, entry, entry_size)`, where
`queue_buffer` is a `vmap()` (`uvm_tools.c:308`) of **pinned guest-user pages**
(`NV_PIN_USER_PAGES`, `uvm_tools.c:295`).
**2 — How userspace sees it.** `UVM_TOOLS_INIT_EVENT_TRACKER{,_V2}` — the caller supplies its
own buffer VAs, pinned at `uvm_tools.c:2083-2091`.
**3 — Address / handle / ID?** **Yes, and two are host-scoped:**
- `UvmEventTestAccessCounterInfo{,_V2}.instancePtr` — `uvm_tools.c:1326` / `:1345` — the **raw
  physical address of a channel instance block**, plus `.instancePtrAperture` at `:1327`/`:1346`.
  Copied verbatim, no translation.
- `UvmEventGpuFaultInfo{,_V2}.channelId` — `uvm_tools.c:792` / `:814` — a hardware channel ID.
  Also `.gpcId` (`:790`/`:812`) and `.clientId` (`:793`/`:815`).
- Guest-scoped by contrast: `.address` fields throughout (fault VA, migration VA), `.pc`,
  `.pid`/`.threadId` (`uvm_tools.c:833-838`).
**4 — Does hardware read it?** No — write-only toward userspace. (UVM *reads back*
`ctrl->get_behind`/`put_behind`/`get_ahead` from the user-writable control page at
`uvm_tools.c:467-468`, `:487`, and explicitly distrusts them — `uvm_tools.c:461-462`.)

Related: `uvm_tools.c:2700` `copy_to_user(... uuids ...)` publishes **host GPU UUIDs**
(`UVM_TOOLS_GET_PROCESSOR_UUID_TABLE`), which is the key that makes every `gpuIndex` in the
records resolvable.

### 2.6 Background rows

- **ChID at channel birth.** `_kchannelNotifyOfChid`, `kernel_channel.c:4095-4118`;
  `notifyFillNotifier(pGpu, pContextDma, pKernelChannel->ChID, 0, NV_OK)` at `:4114`; store is
  `notifyFillNOTIFICATION` `method_notification.c:138` (`MEM_WR32(&pNotifyBuffer->OtherInfo32,
  Info32)`). Called from channel construct, `kernel_channel.c:1032`, when `hObjectError != 0`.
  ⊘ **Legacy-only in practice:** it takes the `ctxdmaGetByHandle` branch, so it fires only if
  `hObjectError` names a **ContextDma**. Modern clients pass a `Memory` handle and this write
  never happens. Contains the **guest ChID**; nothing reads it back.
- **Subdevice event notifier array.** `gpu_rmapi.c:580` `notifyFillNotifierMemory(pGpu,
  pSubdevice->pNotifierMemory, ...)`; registered by `NV2080_CTRL_CMD_EVENT_SET_MEMORY_NOTIFIES`
  (size check `subdevice_ctrl_event_kernel.c:184`). No address; not read back. **This is the
  slot `NV2080_NOTIFIERS_RC_ERROR` (= 37, `cl2080_notification.h:74`) lands in.**
- **Key-rotation notifier** (`NvNotification[2]`): `conf_compute_key_rotation.c:95/172/175`,
  `conf_compute_key_rotation_gh100.c:334`. Confidential Compute only — not on this target.
- **`NvUnixEvent`** (`NV_ESC_RM_GET_EVENT_DATA`): `osapi.c:440` `os_memcpy_to_user`. A **copy,
  not a mapping**; carries an RM `hObject` (guest handle). No hardware reader.
- **USERD is allocated-and-zeroed only, never authored.** `kfifoSetupUserD_GM107`
  (`kernel_fifo_gm107.c:797-808`) is a single `memmgrMemSet(..., 0, ...)`. Its real contents
  come from GSP/HW ⇒ excluded by the brief's rule (b)/(d).
- **Test-gated physical-address disclosure.** `uvm_va_block.c:13924/13936/13989/13994`
  (`resident_physical_address`, `mapping_physical_address`) and `uvm_pmm_gpu.c:3905` copy **host
  physical addresses** to userspace, gated on `uvm_enable_builtin_tests=1` (`uvm_test.c:245`,
  default 0, `uvm_common.c:46-47`).

---

## 3. THE OWNER'S RE-AIM: two authorities, and which values diverge by scope

**The rule asked for: would our design make a SECOND authority write a structure the guest's
ogkm owns?**

**3.1 The channel notifier surface — split authorship is ALREADY the shipping design.**
Slot 0 = GSP's. Slot 1 = guest ogkm's. Slot 2 = CPU-RM's, CC-only. They are distinct
`NvNotification` entries in one memdesc, and `kchannelUpdateNotifierMem` indexes into it
(`kernel_channel.c:1905`, `:1924`). ⇒ **Us writing slot 0 is not a second authority. It is the
first one.**

★★★ **And we are handed the address to do it with, by the guest, at channel creation.**
`kernel_channel.c:549-568`, gated on `IS_GSP_CLIENT(pGpu)`:
```c
pChannelGpfifoParams->errorNotifierMem.base =
    memdescGetPhysAddr(pKernelChannel->pErrContextMemDesc, AT_GPU, 0)
    + pKernelChannel->errorContextOffset;
pChannelGpfifoParams->errorNotifierMem.size        = ...;
pChannelGpfifoParams->errorNotifierMem.addressSpace = ...;   /* SYSMEM on 63/63, w287 */
```
plus `_ERROR_NOTIFIER_TYPE` in `internalFlags` (`:589-592`). ⇒ the guest's ogkm ships us the
**guest-physical address and aperture of its own error notifier in the channel-alloc RPC**.
Nothing needs to be reverse-resolved and nothing host-scoped needs to cross.

★ The wire field is `NV_CHANNEL_ALLOC_PARAMS.errorNotifierMem`
(`src/common/sdk/nvidia/inc/alloc/alloc_channel.h:330`), of type `NV_MEMORY_DESC_PARAMS`
= `{ NvU64 base; NvU64 size; NvU32 addressSpace; NvU32 cacheAttrib; }` (`alloc_channel.h:37-42`).
It is marked `// reserved` — i.e. it is a **kernel→GSP internal field, not client-facing**, which
is exactly why the guest fills it only under `IS_GSP_CLIENT` and why it arrives at us for free.

⊘ **The prior reading — "forwarding `hObjectError` makes the host RM a writer" — is the thing
to avoid, and it is avoidable without inventing anything:** do not hand `hObjectError` to the
host RM at all. Author slot 0 ourselves from `errorNotifierMem.base`, exactly as the GSP does.

**3.2 The scope-divergent value is slot 1, and it is NOT ours.** The token is minted by the
guest from the guest's ChID and never passes through us (no RPC on the GSP-client path,
`kernel_channel.c:3297-3302`). Under passthrough the guest will write `{guest_runlist,
guest_chid}` to a real doorbell the host GPU decodes. **This is a value-scope defect with a
single author** — the opposite shape from a two-writer race, and not fixable by anything at the
memory layer. It is the same chid-collision already named in `mode2_doorbell_chid.md` §2;
what is new is that a **second, stale copy of the token lives in a guest-userspace page** and
must be kept consistent with whatever the ioctl reply says.

**3.3 The genuinely dual-authored structures are the two semaphores** (§2.2, §2.3) — CPU and
GPU writing the same dwords. Both are scope-safe by construction under VA-identity (the
payload is a plain integer), and both are already the pattern the campaign's passthrough
model assumes.

---

## 4. "WE ARE THE GSP — report the fault and let the guest's ogkm write its own notifier"

**Verdict: the reporting channel is REAL and it does drive ogkm's own error machinery. ⊘ But
it does NOT make ogkm write the `NvNotification` — on this target that is the GSP's job, and
NVIDIA's own comments say so. The hypothesis is half right, and the half that is wrong is the
half it was named after.**

**4.1 What ogkm expects from the GSP.** Four event RPCs, dispatched in
`_kgspRpcGspEventHandler` (`kernel_gsp.c:1484-1515`):

| RPC | handler | what ogkm does with it | identifiers it carries |
|---|---|---|---|
| `NV_VGPU_MSG_EVENT_RC_TRIGGERED` (`0x1004`, `rpc_global_enums.h:256`) | `_kgspRpcRCTriggered` `kernel_gsp.c:548-670` | `krcCheckBusError`, adds RcDiag records to the system journal (`:616-642`), then `krcErrorSendEventNotifications_HAL` (`:669`) | `nv2080EngineType`, `exceptType`, **`chid`**, `gfid`, `scope`, `rcJournalBuffer` |
| `NV_VGPU_MSG_EVENT_POST_EVENT` | `_kgspRpcPostEvent` `kernel_gsp.c:485-539` | resolves `hClient`/`hEvent` via `CliGetEventInfo` (`:498`), then `gpuNotifySubDeviceEvent` (`:518`) or `osNotifyEvent` (`:530`) | **`hClient`, `hEvent`** (RM handles), `notifyIndex`, `data`, `info16` |
| `NV_VGPU_MSG_EVENT_OS_ERROR_LOG` | `_kgspRpcOsErrorLog` `kernel_gsp.c:769-806` | `nvErrorLog2_va` → the `NVRM: Xid` dmesg line + OS notification | **`chid`, `runlistId`**, `exceptType`, `errString` |
| `NV_VGPU_MSG_EVENT_MMU_FAULT_QUEUED` | `_kgspRpcMMUFaultQueued` `kernel_gsp.c:1047-1056` | `osQueueMMUFaultHandler(pGpu)` — schedules the **UVM** ISR | none |

★★★ **Every identifier in that table is looked up in the GUEST's own tables.**
`_kgspRpcRCTriggered` resolves the channel with `kfifoChidMgrGetKernelChannel(..., rpc_params->chid)`
(`kernel_gsp.c:588-590`) against the guest's `CHID_MGR`; `_kgspRpcPostEvent` resolves
`hClient`/`hEvent` against the guest's client database. ⇒ **we must emit guest ChIDs and guest
RM handles, never the host's.** That is a hard requirement on the delivery, and it is the same
scope rule as §3.2 pointing the other way.

**4.2 Does it drive ogkm's own error machinery?** Point by point, as asked:

- **Write the notifier?** ⊘ **No** — `krcErrorSendEventNotificationsCtxDma_HAL` is
  `_FWCLIENT` unconditionally in this build (`g_kernel_rc_nvoc.h:280`), and `_FWCLIENT` writes
  nothing (§2.4). **The GSP must have already written slot 0.** With CC it would be different
  (`kernel_gsp.c:660-667`), but CC is off here.
- **Post the RC event (`NV2080_NOTIFIERS_RC_ERROR` = 37)?** ✅ **Yes** —
  `krcErrorSendEventNotifications_KERNEL` `kernel_rc_notification.c:439-469` calls
  `gpuNotifySubDeviceEvent(pGpu, NV2080_NOTIFIERS_RC_ERROR, ...)` at `:462-467`, which both
  CPU-writes the subdevice notifier array (`gpu_rmapi.c:580`) and pings registered OS events
  (`gpu_rmapi.c:588`). It also wakes the ContextDma's event list via `notifyEvents(...)`
  (`kernel_rc_notification.c:422`, `NV_OS_WRITE_THEN_AWAKEN`).
- **Set `bIsRcPending`?** ⊘ **No — and there is no way to.** `bIsRcPending` has **no `NV_TRUE`
  setter anywhere in ogkm 580.159.04**. Whole-tree grep returns only the declaration
  (`g_kernel_channel_nvoc.h:303`), one clear to `NV_FALSE` (`kernel_channel.c:3028`), and one
  read (`:3056-3057`). The comment at `:3009-3010` claims *"Kernel-RM is the source of truth on
  this"* — ★ **a comment describing behaviour the open source does not implement.** Any design
  that plans to observe `bIsRcPending` flipping is planning around a field that never flips.
- **Serve `0x83de030c` from guest-side state?** ⊘ **No.**
  > ### ⊘ TRANSPORT CORRECTED 2026-08-14 (w289) — the conclusion holds, the route named below does NOT.
  > The bullet's verdict (*"we must answer it as the GSP; no guest-side cache"*) is **right**.
  > But `rpcCtrlDbgReadAllSmErrorStates_HAL` / `:7338` / the 80-SM chunking is the **vGPU-guest**
  > transport, reached only from `rpcDmaControl_wrapper` (`vgpu/rpc.c:4513`), and **GA106 binds
  > that interface to `rpcCtrlDbgReadAllSmErrorStates_STUB`** — `g_rpc_private.h:414` names GA106
  > in the STUB's own chip list. ⚠ **Reading `:7338` as "what GA106 does" is reading a stub's body.**
  > The real route on a bare-metal GSP client is the **generic** one: `ROUTE_TO_PHYSICAL` in the
  > exported flags (`0x50048`) makes `rmresControl_Prologue_IMPL` (`rmapi/resource.c:266-297`)
  > `NV_RM_RPC_CONTROL` the **whole 4824-byte struct** to GSP and return `NV_WARN_NOTHING_TO_DO`,
  > so the `_IMPL` at `:731` is **skipped entirely** (`rs_resource.c:191-201`).
  > ⇒ We must serve it as **one opaque control reply**, not as an 80-SM chunk stream — and every
  > one of the 4800 output bytes must be initialised.
  > ★ Full analysis, plus the scope/privilege answer:
  > `docs/reference/sm_debugger_scope_and_sm_error_registers.md`.

  `0x83de030c` = `NV83DE_CTRL_CMD_DEBUG_READ_ALL_SM_ERROR_STATES`
  (`src/common/sdk/nvidia/inc/ctrl/ctrl83de/ctrl83dedebug.h:371`). Its kernel implementation is
  `ksmdbgssnCtrlCmdDebugReadAllSmErrorStates_IMPL`
  (`src/nvidia/src/kernel/gpu/gr/kernel_sm_debugger_session_ctrl.c:731`), and on a GSP client it
  is **RPC'd out** — `rpcCtrlDbgReadAllSmErrorStates_HAL` (`vgpu/rpc.c:4763-4764`, impl `:7338`),
  chunked at `VGPU_RPC_CTRL_DEBUG_READ_ALL_SM_ERROR_STATES_PER_RPC` (`:7369`). **We would have
  to answer it as the GSP** with real SM error state; there is no guest-side cache to serve
  from.

**4.3 ⇒ The honest shape of the design.** *"One delivery instead of three fixes"* is **not**
what the source supports. What it supports is:

1. **We author slot 0** of the guest's notifier, at the guest-physical address the guest gave
   us in `errorNotifierMem.base` (§3.1). No forgery — that is the GSP's documented role.
2. **Then** we send `RC_TRIGGERED` with the **guest's** ChID / engine type / scope, and ogkm
   does the journal + `NV2080_NOTIFIERS_RC_ERROR` + wakeups itself.
3. **And separately** `OS_ERROR_LOG` if we want the `NVRM: Xid` line in the guest's dmesg.
4. `0x83de030c` remains a fourth thing we must answer ourselves.

That is **two deliveries plus a control**, not one — but steps 1–3 are a matched pair the
driver already expects in that order (`kernel_rc_notification.c:353-361` exists precisely to
close the race between them), so it is still one coherent mechanism rather than three
unrelated patches.

---

## 5. ⊘ WHAT I COULD NOT DETERMINE

1. **Whether `NV9074`/`NV9072` SW-method semaphore release (§2.2) is reachable on the CUDA
   path.** *Would determine it:* grep a `cup2`/`cup8` guest ioctl trace for allocations of class
   `0x9074`/`0x9072`, or decode the guest pushbuffer for SW-object subchannel binds. The
   `nvdiff` host reference (`traces/host_reference_ga106/`) can answer the alloc half today
   without a boot.
2. ✅ **ANSWERED by §9.5 — struck.** libcuda takes the work-submit token from the **ioctl reply**
   (`0xc36f0108`, `ppost 08000000`, ×16), not from `NvNotification[1]`. Still open only for raw
   clients and for `nvidia-push` (`nvidia-push-utils.h:108` reads slot 1 directly).
3. **Whether `instancePtr` (§2.5) is hypervisor-translated before UVM sees it.** UVM performs
   no translation; the producer is the access-counter buffer parser in `src/nvidia`, which was
   not audited. *Would determine it:* trace `kaccessCntrBufferService`/`uvm_gv100.c` producers
   of `buffer_entry->instance_ptr`.
4. **Which of `pMemory->KernelVAddr` (`method_notification.c:453`) vs the BAR2 transfer window
   (`:460`) is taken for a given notifier write.** Both branches exist and reach the same
   stores; the choice depends on whether the client requested a kernel mapping. Immaterial to
   the answer, but it changes *where* the write is observable.
5. **`uvm_ats_sva.c` / `uvm_ats_faults.c`** were covered only by the global `copy_to_user` /
   `vm_insert_page` sweeps (both silent for those files); not audited line by line.
6. **Everything here is 580.159.04.** The `_FWCLIENT` unconditional dispatch (§2.4) and the
   `bIsRcPending` no-setter (§4.2) are exactly the kind of fact a driver revision can move.
   ★ Any claim built on them must carry the driver version, the way a bench claim carries a
   source revision.
7. ★ **`NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` (`0x906f0106`)** — unprivileged *and* per-channel,
   which is exactly the identity the RC event lacks (§10.2). ⊘ `ROUTE_TO_PHYSICAL`, no
   open-source body: the fill code is closed GSP firmware. **Must be measured, not read** — the
   single empirical probe this question justifies.
8. **Whether libcuda's own `cls=0xc56f` allocs set `hObjectError`.** Structurally unmeasurable in
   `traces/host_reference_ga106/` (§9.4). ⊘ Does *not* block anything: §9.1 already establishes
   UVM's channels always do, which is sufficient for the design. *Cheapest closure, no bench:*
   decompress `traces/mode2_c_reference/cap3_matmul_forwarding.rec.zst` to scratch and run
   `scripts/mode2_diag/rec_dump.py --payload`, filtering the `GSP_RM_ALLOC` RPC for
   `hClass=0xc56f` — `kernel_channel.c:2654` puts `hObjectError` verbatim on the wire, and that
   capture is a real-GA106 `cup8` run at `bad=0`.
   ★ *Structural fix worth doing anyway:* `nvdiff_shim.c:178`, when `nr == 0x2B` and the declared
   `paramsSize == 0`, fall back to a per-`hClass` size table — that un-blinds **every** alloc
   class in the reference oracle, not just this field.

---

## 6. THE ONE-LINE ANSWER FOR THE OWNER

★★★★★ **THE LOOP CLOSES, AND IT NEEDS NO NEW MECHANISM AND NO ROOT** (§10.6). Host side: the
unprivileged isolate learns the Xid by **OS event** — no shared page, no shadow page, no
privilege (§10). Guest side: **we author slot 0** at the guest-physical address the guest already
handed us (§3.1), then send `RC_TRIGGERED` with the *guest's* ChID (§4.1). ⚠ **Both halves are
mandatory** — per §9.2, UVM's only error exit is a non-zero `errorNotifier->status`, so without
the guest-side write a faulted CUDA channel simply waits.
⊘ **And the hoped-for simplification is dead:** the error notifier is **on the CUDA path** via
nvidia-uvm (§9), so it cannot be split off as a raw-client-only job.


> **Yes, ogkm writes structures into guest userspace VAs — but none of them contains an
> address the GPU follows, so the passthrough design is NOT broken by this.** The thing to
> change is narrower: **do not hand `hObjectError` to the host RM.** The guest already ships
> us its notifier's guest-physical address in the channel-alloc RPC
> (`kernel_channel.c:557-560`); we write slot 0 as the GSP, ogkm writes slot 1 (the work-submit
> token) itself, and no host-scoped value ever enters a page the guest reads as its own.
> ★ The unrelated live risk is that slot 1's token is `{guest_runlist, guest_chid}` and the
> doorbell it feeds is real hardware.

---

## 7. ★★★ Q1 (owner, 2026-08-13) — WHO WRITES THE ERROR NOTIFIER: CPU-RM, OR THE GPU?

### 7.1 THE ANSWER: **the CPU. Every writer in ogkm is a CPU store. No engine is ever handed the notifier's address.** "Context DMA" is an ADDRESSING descriptor, not a statement about who performs the write.

The `ADDR_VIRTUAL` branch that prompted the question is the proof, not the counter-evidence.
`kchannelGetNotifierInfo`, `src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2019-2076`: it takes
the notifier's **GPU VA** (`memdescGetPhysAddr(..., AT_GPU_VA, 0)`, `:2028`), looks it up with
`CliGetDmaMappingInfo` (`:2033-2039`) — **and then demands a CPU pointer:**

```c
if (!pDmaMappingInfo->KernelVAddr[subdeviceInstance])   /* :2066 */
{
    NV_PRINTF(LEVEL_ERROR, "Kernel VA addr mapping not present for notifier\n");
    return NV_ERR_INVALID_STATE;                        /* :2070 */
}
```

⇒ **The GPU VA is a LOOKUP KEY used to find a kernel CPU mapping.** If no CPU mapping exists
the channel alloc **fails**. An engine-written notifier would need no kernel VA at all, and
certainly would not fail without one.

★ Corroborated by every other writer:
- `notifyWriteNotifier` obtains its pointer via `ctxdmaGetKernelVA` (`method_notification.c:75`)
  — a **CPU** pointer out of a ContextDma.
- `notifyFillNotifierMemory` writes through `pMemory->KernelVAddr` (`method_notification.c:453`)
  or a BAR2 CPU window (`memmgrMemBeginTransfer`, `:460`).
- `semaphoreFillGPUVATimestamp` writes through `pDmaMappingInfo->KernelVAddr[...]` (`:630`).
- All five field stores are `MEM_WR16`/`MEM_WR32` (`method_notification.c:181-185`) — CPU stores.

**Is the author different per error class?** ⊘ **No — the author varies by which RM HALF runs,
never by error class.** Same function (`notifyFillNvNotification`), same five stores, for RC,
MMU-fault and engine-exception alike. What differs is CPU-RM vs GSP-RM, per §2.4's table.

**And GSP is a CPU too.** `kchannelUpdateNotifierMem` is compiled **into GSP-RM** — see its
`RMCFG_FEATURE_PLATFORM_GSP` assert at `kernel_channel.c:1876-1877` — and performs the same
`MEM_WR` stores from the falcon. ⇒ *"GSP writes the notifier"* means **GSP's processor stores
to it**, not that an engine DMAs into it. The comment at `kernel_channel.c:2021-2024` says so in
the vendor's own terms: *"In GSP client mode + SLI, **GSP won't be able to write to notifiers**
on other GPUs"* — a software reachability constraint, meaningless for a hardware writer.

⇒ ★★ **The hoped-for dissolution does not happen.** *"If hardware is an author, the whole
'which RM owns it' question dissolves"* — hardware is **not** an author, so the ownership
question stands exactly as §3 states it. **Slot 0 is ours (as the GSP); slot 1 is the guest's.**

### 7.2 ⊘ THE TIMESTAMP CROSS-CHECK IS STRUCK (owner, 2026-08-13) — the source is the answer

*"Or read ogkm source and you know the answer."* ⇒ **The `NvNotification` timestamp / GPU-clock
-vs-CPU-wall-time test is withdrawn: not run, not reported.** §7.1 is complete on its own —
there was no outcome where reading the source came up silent (a CPU store ⇒ CPU-RM authors it;
a descriptor handed to hardware ⇒ it does not). ★ An ambiguous empirical result would have
had to be *reconciled* with the authoritative one — pure cost, and the same class as reaching
for a probe when the source is readable.

★ One source fact from that line of enquiry is kept, because it is **evidence for §7.1 rather
than a test of it**: the timestamp `notifyFillNvNotification` stores
(`method_notification.c:169-171`, `:183-184`) comes from `tmrGetCurrentTime` → on GA106
`tmrGetTimeEx_GM107` (dispatch names GA106 explicitly, `g_objtmr_nvoc.c:455-458`), which
**MMIO-reads `NV_PTIMER_TIME_1`/`TIME_0`** (`timer_gm107.c:243-249`). ⇒ **the CPU stores a
GPU-clock value.** That is one more CPU store in the notifier, and it also means a GPU-looking
timestamp in this struct is *expected* under CPU authorship — worth knowing so nobody reads one
later as evidence of a hardware writer.

★★ **The discriminator that actually settled §7.1 was HAL dispatch** — `g_kernel_rc_nvoc.h:280`,
`g_kern_gmmu_nvoc.c:1727`, `g_objtmr_nvoc.c:455`. Compile-time, unambiguous, no boot. It is also
what caught the dead MMU-fault path in §2.4, which reading the `.c` files alone would have
reported as live.

### 7.3 ⊘ THE BOUND ON THIS ANSWER
ogkm **allocates** the RAMFC sub-memdesc (`kernel_channel_gm107.c:288-294`) but never populates
it; instance-block field programming lives in physical/GSP RM, which is **not in this tree**.
⚠ So ogkm cannot prove a negative about what GSP-RM might hand to an engine. What it *does*
prove is that **on every path ogkm can see — including the one that resolves a GPU VA — the
notifier is written by a processor through a CPU mapping**, and that RM's own design comment
(`method_notification.c:176-179`) states notifiers are not read by the GPU.
*What would close it:* the GSP-RM ucode, or a fault taken with the notifier page **unmapped
from the CPU but mapped in the GPU VAS** — if the notifier still updates, an engine wrote it.
⊘ **Do not use a watchpoint** — a DMA write is invisible to x86 debug registers; that
instrument is a negative control only.

---

## 8. THE SANCTIONED FALLBACK (shadow page) — assessed against §7.1

The owner's fallback: *"we present fake fb, but on the real gpu this isn't mapped. there the
host ogkm notifier page sits. real gpu → thinks va belongs to host …; guest → thinks va belongs
to himself …; vmm → sees both at the same va."* One authority per address space, no shared
bytes, nothing host-scoped ever guest-visible. **The idea is sound; for THIS structure it is
unnecessary — and §7.1 is why.**

**Why it is not needed for the notifier.** The design is a **GPU-VAS aliasing** trick, and it
presupposes that the writer reaches the page **through a GPU VA**. §7.1 establishes it does not:
the notifier is written by a **processor through a CPU kernel mapping**, and the GPU VA (when
there is one) is used *only as a lookup key to find that CPU mapping*
(`kernel_channel.c:2033-2071`, which **fails** if no kernel VA exists). ⇒ **the GPU VAS is not
on the notifier's write path at all**, so aliasing it changes nothing about who writes what.

**What replaces it, at zero mechanism cost.** Our host-side isolate is an ordinary RM client. It
can allocate **its own** notifier page on the host, pass **its own** handle as `hObjectError`
when it creates the host channel, and read it with a plain CPU load. Two pages, two address
spaces, one author each — **exactly the property the shadow page was designed to buy** — with no
fake FB, no aliasing, and no VMM-side "read the host variant" step.
★★ And it dissolves §3.1's concern by construction: **the guest's `hObjectError` never needs to
reach the host RM**, so the host RM is never a second author of a guest-owned structure.

✅ **RESOLVED by §10 — the fallback is not needed at all.** The isolate can learn of the fault by
**unprivileged OS event** (`NV_ESC_ALLOC_OS_EVENT` → `NV2080_NOTIFIERS_RC_ERROR` → `poll()` →
`NV_ESC_RM_GET_EVENT_DATA`), so it need not poll *any* notifier page on the host side — its own
or a shadow. ⇒ **the shadow page is retired as unnecessary, not merely as unneeded-for-this-
structure.**

**Where the shadow-page idea DOES have a natural target.** The two structures that are genuinely
GPU-written — the SW-method `NvGpuSemaphore` (§2.2) and the UVM semaphore pool (§2.3) — are
reached through a GPU VA and are co-authored by CPU and engine. If a two-authority problem ever
has to be solved by aliasing, it is those, not the notifier. ⊘ I did **not** assess whether our
mapping model can present two different physical pages at one VA across two VASes; that is a
question about our own code, not about ogkm.

---

## 9. ★★★★★ Q3 — DOES CUDA USE THE ERROR NOTIFIER? **YES. THE HOPED-FOR SPLIT DOES NOT EXIST.**

**The question was:** if libcuda never allocates or polls an error notifier, the notifier serves
only the raw-client requirement and is **irrelevant to `cup2`** ⇒ two separate jobs.

⊘⊘ **It is not two jobs. The notifier is on the CUDA path, and the route is nvidia-uvm.**

### 9.1 ALLOCATE — yes, unconditionally, in-kernel

`src/nvidia/src/kernel/rmapi/nv_gpu_ops.c`, `nvGpuOpsChannelAllocate` (reached from
`nvUvmInterfaceChannelAllocate`):
```
:5887  errorNotifierSize = sizeof(NvNotification) * NV_CHANNELGPFIFO_NOTIFICATION_TYPE__SIZE_1;
:5891  nvGpuOpsGpuMalloc(..., &channel->errorNotifierOffset, /* bGetKernelVA */ ..., ...);
:5933  channel->errorNotifier = (NvNotification*)pDmaMappingInfo->KernelVAddr[subdeviceInstance];
:5941  pAllocInfo->gpFifoAllocParams.hObjectError = hErrorNotifier;
```
⇒ **every UVM channel carries `hObjectError != 0`**, and UVM channels are built inside `cuInit`
(`UVM_REGISTER_GPU` builds a channel manager). ★ This never crosses the ioctl boundary, which is
exactly why an ioctl-level search reads as *"libcuda doesn't do it"*.

★ Corroborated on real GA106 by our own commit **`299b5d7`**: dropping the forwarded channel's
`hObjectError` ref *"fixes `kchannelGetNotifierInfo` OBJECT_NOT_FOUND"*. That function is called
**only when `hObjectError != 0`** — a zero field produces no such error to fix. ⇒ the forwarded
CUDA-path channel alloc carried a non-zero `hObjectError`, measured.

### 9.2 POLL — yes, on **every** channel progress update, and it is UVM's ONLY error exit

`kernel-open/nvidia-uvm/uvm_channel.c:2058-2081`, `uvm_channel_get_status`:
```c
error_notifier = channel->channel_info.errorNotifier;   /* :2066 */
if (error_notifier->status == 0)                        /* :2068 */
    return NV_OK;
...
return NV_ERR_RC_ERROR;
```
called from `uvm_channel_update_progress_with_max` (`:2086`, `:2094`). Consistent with
`docs/design/mode2_channel_ownership_split.md:150` — *"every UVM wait exits **solely** on a
non-zero error notifier … UVM cannot originate a timeout."*

⇒ ★★★ **If we never write slot 0, a faulted UVM channel never reports an error — it waits.**
That makes §3.1 (author slot 0 as the GSP, from `errorNotifierMem.base`) **load-bearing for the
CUDA path**, not a raw-client nicety.

### 9.3 libcuda's own fault-time read is `0x83de030c`, not a notifier — w277 reproduced with the record

`traces/fault_known_positive_ga106/arm1_native/faultgr.jsonl.zst`: `[367]` allocs `cls=0x83de`
(GT200_DEBUGGER, `hObjectNew=0x5c000072`); `[707]` `RM_CONTROL 0x83de030c`, `paramsSize=4824`,
**`pgot=4824` — fully captured**, `rc=0`. Decoding against `ctrl83dedebug.h:382-389`:
`hTargetChannel=0x5c000019`, `numSMsToRead=28`, **`smErrorStateArray` all 4800 bytes zero on both
sides**, and RM wrote **exactly 5 bytes** — `mmuFaultInfo=0x81010000`, `mmuFault.valid=1`,
`mmuFault.faultInfo=0x81010000`. ⇒ the fault reached libcuda through the **MMU-fault tail** of
that control, not the SM array and not a notifier.

⚠ **Do not use `faultce` as the contrast.** `arm1_native/RESULT` records it as
`VERDICT NOFAULT … delivered_on=NOTHING`. Its lack of `0x83de030c` is a **no-fault run**, not
evidence that CE faults skip the debugger. `faultgr` is the only genuine CUDA fault capture we own.

### 9.4 ⊘⊘⊘ A NEW, LOAD-BEARING LIMIT ON THE nvdiff ORACLE — it CANNOT see `hObjectError`

**Every `NV_ESC_RM_ALLOC` in all 12 `traces/host_reference_ga106/` captures has an UNMEASURED
parameter body** — `launch_r1` `nr=43`: **100/100** records at `psize=0, pgot=0, len(ppre)=0`.
Cause: `tests/mode2/nvdiff/nvdiff_shim.c:178` sizes the out-of-line body from
`NVOS64_PARAMETERS.paramsSize`, and **libcuda passes `paramsSize = 0`** (RM derives the size from
`hClass`). Verified genuine, not a decode slip: record 161's raw `hpre` bytes 32-47 are zero.

⇒ **"`hObjectError` is 0 in libcuda's channel allocs" is NOT readable from this oracle** — the one
field the question turned on is the one field it structurally cannot capture. ★ Decoding that
absent body to zeros would have produced exactly the false discovery the brief warns about
(*an absent artefact reads as favourable*), and it would have "confirmed" the hoped-for split.
⚠ This blinds **every** alloc class (`0xc56f`, `0xa06c`, `0xc7c0`, `0x003e`), and belongs beside
the `dlen=0` lesson in `CLAUDE.md`. By contrast `RM_CONTROL` bodies **are** captured (197 records,
`psize` 1…13344), so §9.3's control-command absences **are** measurements.

★ Measured absences (real, from captured control headers) across `launch_r1/r2/faultce/faultgr`:
`0xa06f0108` `NVA06F_CTRL_CMD_SET_ERROR_NOTIFIER` (`ctrla06fgpfifo.h:115`), `0xc36f010a`
`SET_WORK_SUBMIT_TOKEN_NOTIF_INDEX` (`ctrlc36f.h:132`), `0x20800303`
`EVENT_SET_MEMORY_NOTIFIES`, and `RM_ALLOC cls=0x0005` (`NV01_EVENT`) — **all zero**.
⇒ libcuda never *retro-fits* a notifier; if it has one it is named at channel-alloc time.

### 9.5 ★ A correction to my own §2.1 caveat

I flagged a risk that a stale guest-scoped token could persist in `NvNotification[1]`. Measured:
`0xc36f0108` `GET_WORK_SUBMIT_TOKEN` appears ×16 with `ppre 00000000 → ppost 08000000` ⇒
**libcuda takes the token from the ioctl reply**, not from the notifier page. The stale-copy risk
is therefore **not** live for libcuda. ⊘ It is not retired for raw clients or for any consumer
that reads slot 1 directly — `nvidia-push-utils.h:108` reads exactly that slot.

---

## 10. ★★★★★ Q2 — CAN THE UNPRIVILEGED HOST-SIDE ISOLATE LEARN OF A FAULT BY ioctl/event? **YES.**

**No root, no `CAP_SYS_ADMIN`, no shared notifier page.** Every gate on the path is
`RS_FLAGS_ALLOC_NON_PRIVILEGED` / `RS_ACCESS_NONE`. ⇒ **The sanctioned shadow-page fallback is
not needed, and neither is polling a notifier on the host side.**

### 10.1 The mechanism, end to end
`NV_ESC_ALLOC_OS_EVENT` → `NV01_EVENT_OS_EVENT` (0x79) on a `NV20_SUBDEVICE_0` →
`NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION(event=37, action=REPEAT)` → `poll()` →
`NV_ESC_RM_GET_EVENT_DATA` (0x52).

- **Payload store:** `kernel-open/nvidia/nv.c:4013-4017` — `nvet->event.info32 = info32;` etc.
- **Copy-out:** `arch/nvalloc/unix/src/osapi.c:435-438` then `:440` `os_memcpy_to_user(...)`.
- **Production:** `kernel_rc_notification.c:462-467` `gpuNotifySubDeviceEvent(pGpu,
  NV2080_NOTIFIERS_RC_ERROR, NULL, 0, exceptType, partitionAttributionId)`, reached from
  `_kgspRpcRCTriggered` (`kernel_gsp.c:669-676`).
- **Reaches every client:** `gpu_rmapi.c:324-326` — `if (!IS_MIG_IN_USE(pGpu)) return NV_OK;`
  short-circuits the filter. (MIG adds a `NV_RM_CAP_SYS_SMC_MONITOR` gate at `:337-345` — the
  only privilege-ish check anywhere on this path, and MIG-only.)

**What arrives:** `NotifyIndex = 37`; **`info32 = exceptType`** — the Xid-equivalent
`ROBUST_CHANNEL_*` (`nverror.h:49` = 31 for `..._MMU_ERR_FLT`); `info16 =
partitionAttributionId`; `hObject` = which of *our* registrations fired.

**Privilege, registration vs delivery, answered separately as asked:**
- *Registration:* `NV01_EVENT_OS_EVENT` `resource_list.h:2188-2199` and `NV20_SUBDEVICE_0`
  `:429-438` are both `NON_PRIVILEGED`/`RS_ACCESS_NONE`; `alloc_free.c:649-671` rejects only
  `ALLOC_PRIVILEGED`/`ALLOC_KERNEL_PRIVILEGED`. `EVENT_SET_NOTIFICATION` is `flags=0x10118u,
  accessRight=0x0u` (`g_subdevice_nvoc.c:1600-1614`) and its `_IMPL` has no admin check.
- *Delivery:* **no check at all.** `NV_ESC_ALLOC_OS_EVENT` / `NV_ESC_RM_GET_EVENT_DATA` are
  handled in `rm_ioctl`'s **pre-RM switch** (`osapi.c:2779-2825`) — they never build a `secInfo`.

### 10.2 ⊘ The real gap is INFORMATION, not privilege
The RC event is **GPU-scoped**: `gpuNotifySubDeviceEvent_IMPL` (`gpu_rmapi.c:509-596`) iterates
*all* subdevice back-references and never consults `pKernelChannel` or `RC_NOTIFIER_SCOPE`.
⇒ we get the Xid but **not which channel**, and we are woken by other tenants' RC errors too.

The channel-scoped event exists in the same callback —
`krcErrorSendEventNotificationsCtxDma_FWCLIENT` `:409-426` `notifyEvents(..., 0, 0,
RES_GET_HANDLE(pContextDma), ...)` — but carries `info32=0, info16=0`, i.e. identity and no
detail. ⇒ **two unprivileged registrations, correlated by arrival.**
⚠ That per-channel event fires **only if `hObjectError` resolves as a `NV01_CONTEXT_DMA`**
(`ctxdmaGetByHandle`); a `Memory` handle silently never fires it. **Our isolate controls that
choice** — this is the one place where the legacy ctxdma form is the useful one.

### 10.3 ★★ Two traps this turned up
- ⊘ **The NVOC flag table is not the authority on privilege.**
  `NV2080_CTRL_CMD_RC_GET_ERROR_COUNT` (`0x20802205`) and `..._GET_ERROR_V2` (`0x20802213`) are
  flagged `NON_PRIVILEGED` (`g_subdevice_nvoc.c:7146`, `:7311`) but are **admin-gated in the
  function body** — `kernel_rc_ctrl.c:113` → `_KERNEL` HAL → `:88` `rmclientIsAdmin(...)` else
  `:98` `NV_ERR_INSUFFICIENT_PERMISSIONS`. **Root only.** Same class as *a signature bounds what
  can be returned; the call site says what is read.*
- ⊘ **`NV01_EVENT_WITHOUT_EVENT_DATA` silently discards the Xid.** `nv.c:3997` guards the store
  on `data_valid`; `event_notification.c:809-816` clears it for that flag. You get a wakeup with
  **no payload**. Do not set it.

### 10.4 Ruled out, with citations
- `NV906F_CTRL_CMD_RESET_CHANNEL`'s `bIsRcPending` — unprivileged, and **always FALSE**
  (independently re-derived; confirms §4.2).
- `MMU_FAULT_BUFFER` (0xc369) — `RS_FLAGS_ALLOC_KERNEL_PRIVILEGED` (`resource_list.h:975-984`);
  since `escape.c:304` caps any ioctl caller at `RS_PRIV_LEVEL_USER_ROOT`, it is **unreachable
  from userspace even as root**. UVM-in-kernel only.
- `NV2080_CTRL_CMD_GPU_GET_ENGINE_FAULT_INFO` — unprivileged but useless: a **static
  engine→fault-ID table lookup** (`subdevice_ctrl_gpu_kernel.c:1749-1751`), no runtime state.
- `NV0000_CTRL_CMD_NVD_GET_RCERR_RPT` — kernel-only for the useful owner (`kernel_rc_ctrl.c:312-316`).
- ★ **`NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` (`0x906f0106`) is the best untested lead:**
  unprivileged **and per-channel** (`g_kernel_channel_nvoc.c:292-308`, `flags=0x10048u`), params
  `addrHi/addrLo/faultType/...` (`ctrl906f.h:213-220`). ⊘ But it is `ROUTE_TO_PHYSICAL` with no
  open-source body — the fill code is in **closed GSP firmware**. ⇒ **must be measured, not
  read.** That is the one empirical probe this whole question justifies.

### 10.5 gVisor is asymmetric — and it is NOT evidence against us
nvproxy forwards `NV_ESC_ALLOC_OS_EVENT` (`version.go:180`), `NV01_EVENT_OS_EVENT`
(`version.go:412`) and `EVENT_SET_NOTIFICATION` (`version.go:272`), but
**`NV_ESC_RM_GET_EVENT_DATA` (0x52) is not defined anywhere in `pkg/abi/nvgpu/frontend.go`** —
sandboxed clients get the **wakeup but not the payload**. That is a gVisor limitation, not an RM
one; our isolate talks to the host driver directly and can issue `0x52` itself.

### 10.6 ⇒ THE LOOP CLOSES, WITH NO NEW MECHANISM
- **Host side:** isolate registers an OS event on the host subdevice → learns the Xid,
  unprivileged, no shared page (§10.1). Optionally a second, per-channel ContextDma event for
  identity (§10.2).
- **Guest side:** we author slot 0 at the guest-physical address the guest handed us in
  `errorNotifierMem.base` (§3.1), then send `RC_TRIGGERED` with the **guest's** ChID (§4.1).
- **And per §9.2 the guest side is mandatory:** UVM's only error exit is a non-zero
  `errorNotifier->status`, so without step 2 a faulted CUDA channel just waits.

---

## 11. SWEEP VALIDITY — the known-positive fired on every pass

Per the brief's ★★★ trap, no "empty" claim below was reported without a control.

- **Error notifier, RM half.** `grep -rn 'MEM_WR16|MEM_WR32' + NvNotification` over
  `src/nvidia/src/kernel/` **fired** on `notifyFillNvNotification`
  (`method_notification.c:181-185`), `notifyFillNOTIFICATION` (`:138-150`) and
  `semaphoreFillGPUVATimestamp` (`:632-634`). Both observed constants are in-tree:
  `0xffff` at `kernel_rc_notification.c:335`; `0x1f` = `ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT`
  at `nverror.h:49`.
- **Error notifier, mapping half.** The `vm_insert_page|vmf_insert|remap_pfn_range` sweep
  reached `nv-mmap.c:458` (`nvidia_mmap_sysmem`) and `nv.c:3696` (`nv_alloc_kernel_mapping`,
  whose comment names ErrorNotifier) — i.e. the sweep found the surface the known positive
  lives on.
- **UVM negatives are controlled.** `grep -n user uvm_rm_mem.h uvm_rm_mem.c` → **0 lines**,
  while the identical pattern on `uvm_mem.h` fires at `:284-286` (`uvm_mem_map_cpu_user`).
  ⇒ "UVM pushbuffers/GPFIFO/page tables are not user-visible" is a **measured** negative, not
  an unrun grep. The whole-module `vm_insert_page`/`remap_pfn_range` set is exactly four sites
  (`uvm_mem.c:806`, `uvm_test_file.c:106`, `uvm_va_range_device_p2p.c:613`,
  `uvm_va_block.c:8160`); none is channel or pushbuffer memory.
- **OS-layer negative is controlled.** The same store-pattern family run over
  `src/nvidia/src/kernel/gpu/` fired (above), while over `kernel-open/nvidia/` +
  `arch/nvalloc/unix/src/` it found only zeroing (`nv-vm.c:415`, `:540`), decryption
  (`nv-vm.c:395`, `:527`) and PAT changes (`nv-vm.c:214/333/437/470`).
- **HAL-dispatch claims** (`_FWCLIENT`, `kgmmuCopyMmuFaults_92bfc3`,
  `kgmmuServiceMmuFault_GA100`) were each read out of `src/nvidia/generated/*`, not inferred
  from the `.c` files — which is what caught the dead MMU-fault path in §2.4.
