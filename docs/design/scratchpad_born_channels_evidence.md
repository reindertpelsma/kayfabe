# If the SCRATCHPAD births a guest channel instead of the per-proc isolate

**STATUS: LIVE — research deliverable, w748, 2026-09-16.**
Evidence-only. **No decision is recommended here**; the owner rules on posture.

Scope: what changes, in RM and in our own tree, if the long-lived VM-wide **scratchpad isolate
(S)** issues the `NV_CHANNEL_ALLOC_PARAMS` that births a guest GPFIFO channel, instead of the
**per-guest-process isolate (I)** that subsequently drives it.

Driver source read: `research_clones/ogkm-580.159.04/` (open NVIDIA kernel modules, 580.159.04),
paths below are relative to that root unless prefixed. Our tree: `/workspace/kf-master`
(`single-store`, `a4d4ebcd`). Mode-1 sibling: `/workspace/nvkvm-pv`.

## Conventions

- Every claim carries `file:line`. Claims that are reasoning rather than reading are marked
  **INFERRED** and state what would confirm them.
- Greps that came back **empty** are recorded as empty. An empty grep is evidence of *nothing*,
  not evidence of absence of the mechanism — it only bounds what was searched.
- ★ marks a load-bearing finding. ⊘ marks a correction or a limit on a finding.

---

## The premise, restated from the brief (given, not re-derived)

- `NV01_MEMORY_LIST_OBJECT` carries `RS_FLAGS_ALLOC_PRIVILEGED` —
  `src/nvidia/src/kernel/rmapi/resource_list.h:630-640`, flag on `:637`. Its two siblings
  `NV01_MEMORY_LIST_SYSTEM` (`:611-619`, flag `:617`) and `NV01_MEMORY_LIST_FBMEM`
  (`:620-629`, flag `:627`) carry it too, so the privilege is on the whole `MemoryList` family,
  not on one class.
  ⊘ **Pointer corrected.** The brief cites `alloc_free.c:599-606` for the check. That range is
  `_serverAlloc_ValidateVgpu` (`src/nvidia/src/kernel/rmapi/alloc_free.c:590-610`), which is the
  **vGPU-host branch** and is only reached when `hypervisorIsVgxHyper()` is true
  (`:631-644`). The check on the **default path we actually take** is
  `_serverAllocValidatePrivilege`'s `else` arm,
  `src/nvidia/src/kernel/rmapi/alloc_free.c:650-660`:
  `if (pResDesc->flags & RS_FLAGS_ALLOC_PRIVILEGED) { if (privLevel < RS_PRIV_LEVEL_USER_ROOT)
  return NV_ERR_INSUFFICIENT_PERMISSIONS; }`.
  **The conclusion is unchanged and the reading is right — only the pointer was off by one
  function.** Same class this campaign already names: citing the source is not the source saying
  what the claim says.
- ⇒ an **unprivileged** per-proc isolate can never mint a page-slice of the single store.
- Channel birth stamps privilege **and** identity from the creator:
  `src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:277-295`. A `DupObject` does not re-run it.
- ★ An unprivileged scratchpad gives `rmclientIsAdmin == false` ⇒ `_PRIVILEGE_USER`
  (`kernel_channel.c:288-291`), so the *privilege* stamp is free. **The `ProcessID` stamp is the
  open question.** That is what §1 answers.

---

## §1 — What consumes `pKernelChannel->ProcessID` / `SubProcessID`?

### 1.0 Where the stamp is written

`src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:276-295`:

```c
        RS_PRIV_LEVEL privLevel = pCallContext->secInfo.privLevel;
        if (privLevel >= RS_PRIV_LEVEL_KERNEL)            { ..._PRIVILEGE_KERNEL; }
        else if (rmclientIsAdmin(pRmClient, privLevel) ||
                 hypervisorCheckForObjectAccess(hClient)) { ..._PRIVILEGE_ADMIN;  }
        else                                              { ..._PRIVILEGE_USER;   }

        pKernelChannel->ProcessID    = pRmClient->ProcID;
        pKernelChannel->SubProcessID = pRmClient->SubProcessID;
```

The `else` branch above (`:264-274`) is the **GSP/vGPU-plugin-offload** path, where `ProcessID`
comes from the *caller-supplied* `pChannelGpfifoParams->ProcessID` instead — see §6.1, this is the
one place RM accepts a PID it did not derive itself.

`pRmClient->ProcID` is set once per client at client construction; `SubProcessID` is settable
after the fact by `NV0000_CTRL_CMD_SET_SUB_PROCESS_ID` —
`src/nvidia/src/kernel/rmapi/client_resource.c:4754`, `:4766` (`pClient->SubProcessID = pParams->subProcessID;`).

### 1.1 Complete census

`grep -rn "ProcessID\|SubProcessID" --include=*.c --include=*.h src/ kernel-open/` → **72 hits in
26 files** (`src/nvidia/generated/` excluded from the analysis below; those are NVOC-generated
mirrors of the same fields).

Classified by what the read *does*:

| consumer | `file:line` | reads | effect |
|---|---|---|---|
| ★ **HWPM / profiler permission gate** | `src/nvidia/src/kernel/gpu/hwpm/profiler_v2/kern_profiler_v2.c:656` | `pChannel->ProcessID` vs `pClient->ProcID` | **refuses** `NV_ERR_INSUFFICIENT_PERMISSIONS` |
| ★ **FIFO ChID / USERD-page isolation** | `src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c:732`, `:768`, `:509` | `pClient->ProcID/SubProcessID` → `FIFO_ISOLATIONID` | **decides which channels may share a USERD page** |
| FECS context-switch events | `src/nvidia/src/kernel/gpu/gr/fecs_event_list.c:311-312`, `:342-343`, `:362-363` | `pKernelChannel->ProcessID/SubProcessID` | labels the notification record |
| Video (NVDEC/NVENC/NVJPG) event log | `src/nvidia/src/kernel/gpu/gpuvideo/videoeventlist.c:81` | `pKernelChannel->ProcessID` | labels the log entry |
| RC (robust channels) error report | `src/nvidia/src/kernel/gpu/rc/kernel_rc.c:341` | `pKernelChannel->ProcessID` | string in the `Xid` / RC breadcrumb |
| **GSP RPC forwarding of channel alloc** | `src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2814-2815` | `pKernelChannel->ProcessID/SubProcessID` → `pRpcParams` | carries the CPU-RM stamp to GSP verbatim |
| GPU accounting start/stop | `src/nvidia/src/kernel/gpu/device.c:431`, `:437`, `:493` | `pClient->ProcID/SubProcessID` | per-PID accounting bucket |
| GPU accounting sample attribution | `src/nvidia/src/kernel/diagnostics/gpu_acct.c:576-613` | sample `subProcessID` | per-PID utilization |
| FB usage per client | `src/nvidia/src/kernel/mem_mgr/video_mem.c:1032`, `:1044` | `pClient->ProcID/SubProcessID` | per-PID FB accounting |
| `GET_CLIENT_INFO` (mem_mgr ctrl) | `src/nvidia/src/kernel/gpu/mem_mgr/mem_mgr_ctrl.c:557` | `pClient->SubProcessID` | reports it to the caller |
| NVENC session registry | `src/nvidia/src/kernel/gpu/nvenc/nvencsession.c:147` | `pClient->SubProcessID` | labels the session |
| `serverutilGetClientHandlesFromPid` | `src/nvidia/src/kernel/rmapi/rs_utils.c:286`, `:310`; decl `src/nvidia/inc/kernel/rmapi/rs_utils.h:158-170` | `pClient->SubProcessID` | enumerate all `hClient` for a (pid, subpid) |
| `NV0000_CTRL_CMD_GPU_GET_PIDS/PID_INFO` filter | `src/nvidia/src/kernel/gpu/gpu_rmapi.c:1006` | `pClient->SubProcessID` | which clients a PID query returns |
| ★ **`RS_SHARE_TYPE_PID` default dup policy** | `src/nvidia/src/kernel/rmapi/client_resource.c:217-231`; installed at `src/nvidia/src/kernel/rmapi/sharing.c:338-352` | `pSrcClient->ProcID == pDstClient->ProcID` | **grants `RS_ACCESS_DUP_OBJECT`** |
| GSP utilization sample copy-back | `src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:852-864` | sample `subProcessID` | pass-through |

⊘ Note the split: **almost every consumer reads `pClient->ProcID`, not
`pKernelChannel->ProcessID`.** Only five sites read the channel's own stamp — the profiler gate,
FECS events, the video event log, the RC report, and the GSP RPC. Everything else keys off the
*client* that is making the current call. This matters: under scratchpad birth the channel's stamp
is S's, but a control call issued later by I still presents I's client.

### 1.2 ★ The two consumers that are not cosmetic

**(a) HWPM / profiler context permission** — `kern_profiler_v2.c:640-658`:

```c
    if (pParentRef->internalClassId == classId(KernelChannelGroupApi) && pClient)
    {
        if (!_isProfilingPermitted(pGpu, pProfBase, pSecInfo))
        {
            for (pChanNode = ...; pChanNode; pChanNode = pChanNode->pNext)
            {
                KernelChannel *pChannel = pChanNode->pKernelChannel;
                NV_CHECK_OR_RETURN(LEVEL_NOTICE, (pClient->ProcID == pChannel->ProcessID),
                                   NV_ERR_INSUFFICIENT_PERMISSIONS);
            }
        }
    }
```

This is a real refusal, guarding "Bug: 5225901 — restrict B1CC HES the profiler session to user
with profiling permission when TSG is shared between multiple processes (MPS mode)"
(`:637-640`). Under scratchpad birth **every channel's `ProcessID` is S's**, so this check
becomes: *does the profiling client's PID equal S's PID?*

- For I (a different process): the check now **fails where it used to pass** — I profiling its own
  channel is refused. Cost, not a hole.
- For S itself: the check **passes for every channel in the VM**, including channels backing other
  guest processes. That is the hole — but it is only reachable by S, which is ours.
- ⊘ Both arms are gated behind `!_isProfilingPermitted(...)`; if the host is configured to permit
  profiling the loop never runs. **INFERRED** that our path never allocates a `ProfilerCtx` at all
  (guest-side CUPTI would have to be forwarded); confirmable by grepping our tree for the profiler
  classes — see §4.

**(b) ★★★ FIFO channel-ID / USERD-page isolation** — this is the load-bearing one, and it does
**not** read the channel stamp; it reads `pClient->ProcID` at ChID-allocation time, i.e. *the
creator's*.

`src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c:729-777`:

```c
        processID             = pClient->ProcID;
        subProcessID          = pClient->SubProcessID;
        bIsSubProcessDisabled = pClient->bIsSubProcessDisabled;
        ...
        pIsolationID->processID    = processID;
        pIsolationID->subProcessID = subProcessID;
```

The `FIFO_ISOLATIONID` is then handed to `eheapAlloc` as the isolation key together with the
comparator `_kfifoUserdOwnerComparator` (`:817-827`), which is:

`src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c:504-511`:

```c
    if ((pAllocID->domain       != pBlockID->domain)    ||
        (pAllocID->processID    != pBlockID->processID) ||
        (pAllocID->subProcessID != pBlockID->subProcessID))
        return NV_FALSE;
```

and the heap it guards is set up at `kernel_fifo.c:334-342`, with the intent stated in the comment:

```c
    //
    // Enable USERD allocation isolation. USERD allocated by different clients
    // should not be in the same page
    //
    kfifoGetUserdSizeAlign_HAL(pKernelFifo, &userdBar1Size, NULL);
    NvBool bIsolationEnabled = kfifoIsPreAllocatedUserDEnabled(pKernelFifo);
    pChidMgr->pGlobalChIDHeap->eheapSetOwnerIsolation(pChidMgr->pGlobalChIDHeap,
                                                      bIsolationEnabled,
                                                      RM_PAGE_SIZE / userdBar1Size);
```

⇒ **the granularity is `RM_PAGE_SIZE / userdBar1Size` channel IDs — one 4 KiB USERD page's worth.**
The enforcement is `_eheapCheckOwnership` in
`src/nvidia/src/libraries/containers/eheap/eheap_old.c:530-545` and `:1330-1340`; the granularity
field is set by `eheapSetOwnerIsolation` (`eheap_old.c:1284`, `:1307`).

⇒ ★ **If every channel in the VM is minted by S, every channel in the VM carries one
`FIFO_ISOLATIONID`, and RM will freely pack channels belonging to *different guest processes* into
the same USERD page.** RM's own comment says that is exactly what the mechanism exists to prevent.

⊘ **Two limits on that finding, both measured from source:**

1. The isolation is only *armed* when `kfifoIsPreAllocatedUserDEnabled()` is true —
   `src/nvidia/generated/g_kernel_fifo_nvoc.h:2411`: `return !pKernelFifo->bDisablePreAllocatedUserD;`.
   When a client supplies its own USERD (`hUserdMemory[]`, which is our case), whether RM still
   pre-allocates is **not established from source here**. The other reader of the same predicate,
   `src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_channel_gm107.c:624`, is an
   `NV_ASSERT_OR_RETURN`, so on our path the property is at least *expected* true. **Unmeasured:**
   what `bDisablePreAllocatedUserD` actually is on a GA106 with GSP. See the probe in §7.
2. `subProcessIsolation` can switch it off wholesale for the guest case —
   `kernel_fifo.c:344-372`; and note `:355-359`: **"In this case subProcessIsolation is always 0"
   … `if (IS_GSP_CLIENT(pGpu)) subProcessIsolation = 0;`** followed by `:361-366` disabling owner
   isolation entirely. ⇒ **INFERRED, and important: on a GSP-client host (ours), the global ChID
   heap's owner isolation may already be OFF for everyone, in which case scratchpad birth costs
   nothing here because the protection was never armed.** Confirming this needs a read of
   `subProcessIsolation`'s initialisation on the live host — see the probe in §7.

### 1.3 What concretely stops distinguishing guest processes

Taking the census at face value, under scratchpad birth the following **can no longer tell two
guest processes apart**, because all channels carry S's PID:

1. **USERD-page co-tenancy** (`kernel_fifo.c:504-511` + `:729-777`) — subject to §1.2's two limits.
2. **FECS context-switch event records** (`fecs_event_list.c:311-363`) — every switch is reported
   as S. Consumers of that stream are profiling/telemetry tools on the *host*; a guest cannot read
   it. Cost: host-side observability, not guest isolation.
3. **The video event log** (`videoeventlist.c:81`) — same shape.
4. **RC / Xid breadcrumbs** (`kernel_rc.c:341`) — a channel fault is attributed to S, so a host
   operator cannot tell which guest process faulted from `dmesg` alone. ★ This is a real
   operational regression for debugging, and it is exactly the kind of thing that reads as benign
   until an incident.
5. **GSP's copy of the stamp** (`kernel_channel.c:2814-2815`) — GSP receives S's PID and whatever
   GSP does with it downstream is **not visible in this source tree** (GSP firmware is a blob).
   ⊘ Unmeasurable from source. Probe shape in §7.

And the following **still distinguish them**, because they key off `pClient->ProcID` of the
*calling* client, which under our design is still I:

6. GPU accounting (`device.c:431-493`, `gpu_acct.c:576-613`) — but note it buckets by the client
   that opened the *device*, so what it distinguishes is "who called", not "who owns the channel".
7. FB accounting (`video_mem.c:1032-1044`), `GET_CLIENT_INFO` (`mem_mgr_ctrl.c:557`), NVENC session
   registry (`nvencsession.c:147`), `GET_PIDS` (`gpu_rmapi.c:1006`),
   `serverutilGetClientHandlesFromPid` (`rs_utils.c:286-310`).
8. ★ `RS_SHARE_TYPE_PID` (`client_resource.c:217-231`) — see §2, where it is the *central*
   mechanism rather than an isolation consumer.

⇒ **The honest summary of §1:** the `ProcessID` stamp is, in 580.159.04, **one enforcement gate
(the HWPM profiler check), one isolation key that is probably not armed on our configuration (the
USERD ChID heap), and otherwise telemetry and attribution.** It is not a capability. Nothing in
`fifo`, `mem_mgr`, `UVM`, or the OS layer was found that uses `pKernelChannel->ProcessID` to grant
or deny access to memory, to a VAS, or to submission.
⊘ Greps that came back **empty**, recorded so the bound is visible:
`grep -rn "ProcessID" src/nvidia/src/kernel/gpu/mmu/ src/nvidia/src/kernel/mem_mgr/ --include=*.c`
finds only `video_mem.c`; the string does not appear anywhere under `kernel-open/nvidia-uvm/`.

---

## §3 — ★★★ CPU mappings: is this a blocker?

**Short answer from evidence: no, and for two independent reasons — one in RM, one already
measured in our own tree. But the cited line is not what it looks like, so start there.**

### 3.1 ⊘ The cited refusal is in the UNMAP path, and on Linux it does not refuse

`src/nvidia/src/kernel/rmapi/mapping_cpu.c:1058` sits inside **`serverUnmap_Prologue`**, not a map
path. `grep -n "processId\|ProcessId\|osGetCurrentProcess" src/nvidia/src/kernel/rmapi/mapping_cpu.c`
returns hits only at `:991`, `:1058`, `:1060` and the API-entry plumbing `:1331`, `:1363`, `:1396`,
`:1427`, `:1437` — ⇒ **the MAP path in `mapping_cpu.c` contains no PID check at all.**

And the line itself is not a refusal:

```c
    if (!bKernel && (ProcessId != osGetCurrentProcess()))
    {
        rmStatus = osAttachToProcess(&pProcessHandle, ProcessId);
```

`osAttachToProcess` on Linux is a **stub that always succeeds** —
`src/nvidia/arch/nvalloc/unix/src/os.c:677-692`:

```c
NV_STATUS osAttachToProcess(void** ppProcessInfo, NvU32 ProcessId)
{
    // ... On Linux/UNIX platforms, we can't "attach" to a random process, but
    // since we don't create/destroy user mappings in the RM, we don't need to, either.
    *ppProcessInfo = NULL;
    return NV_OK;
}
```

★ The real PID gate is downstream, and it is a **lookup filter, not a permission check**:
`mapping_cpu.c:1080-1082` selects `serverutilMappingFilterCurrentUserProc`, which is
`src/nvidia/src/kernel/rmapi/rs_utils.c:325-332`:

```c
    return (!pMapping->pPrivate->bKernel &&
            (pMapping->processId == osGetCurrentProcess()));
```

The same predicate appears in `src/nvidia/src/kernel/rmapi/mapping_list.c:84`
(`CliFindMappingInClient`, `:44`), used by `src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:4277` and
`src/nvidia/src/kernel/gpu/mem_mgr/mem_mgr_ctrl.c:323`.

⇒ **The consequence of a cross-process mapping is not "refused" — it is "not found".** A process
that did not create a mapping cannot *unmap it by CPU address* or look it up by CPU address.
`processId` is stamped at map time by each resource's own map handler, always as
`osGetCurrentProcess()`: `src/nvidia/src/kernel/gpu/gpu_resource.c:144`,
`src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:4471`,
`src/nvidia/src/kernel/gpu/subdevice/generic_engine.c:157`,
`src/nvidia/src/kernel/gpu/mmu/mmu_fault_buffer.c:116`,
`src/nvidia/src/kernel/gpu/uvm/access_cntr_buffer.c:119`,
`src/nvidia/src/kernel/gpu/ccu/kernel_ccu_api.c:136`, `src/nvidia/src/kernel/gpu/dbgbuffer.c:94`.
Struct field: `src/nvidia/inc/libraries/resserv/rs_resource.h:470`.

### 3.2 ★★★ The channel object cannot be CPU-mapped at all on our shape

`kchannelMap_IMPL`, `src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:1277-1291`, opens with:

```c
    NV_ASSERT_OR_RETURN(!pKernelChannel->bClientAllocatedUserD, NV_ERR_INVALID_REQUEST);
```

⇒ **when the client supplies its own USERD — which is exactly what we do, via
`hUserdMemory[0]`/`userdOffset[0]` (`crates/kayfabe-abi/src/submit.rs:418`, `:430`; encode site
`crates/kayfabe-isolate-host/src/rm.rs:8106-8152`) — `NV_ESC_RM_MAP_MEMORY` on the *channel*
object is refused outright, for everyone, creator included.** What a driving process maps is the
**USERD memory object**, never the channel. The channel is otherwise a legal map target
(`src/nvidia/arch/nvalloc/unix/src/osapi.c:1006-1008` lists `classId(KernelChannel)` alongside
`Memory` and `KernelCcuApi`), but that door is shut on the client-allocated-USERD path.

⇒ **The `ProcessId != osGetCurrentProcess()` question never arises for the channel object.** It
arises, if at all, for the memory objects.

### 3.3 Which resources the *driving* process actually needs mapped — from our own code

`HostRmBackend::alloc_channel_in` (`crates/kayfabe-isolate-host/src/rm.rs:7765`) makes its CPU
mappings at `:8212-8290`, and **both are conditional on provenance**:

```rust
        let ring_view = match owner {
            RingOwner::Ours | RingOwner::OursUnmapped => Some(self.conn.map_cpu(
                ring_obj, RING_OBJECT_BYTES, CachePolicy::WriteCombining)),
            RingOwner::HandedIn => None,
        };
        let userd_view = match userd_owner {
            UserdOwner::Ours     => Some(self.conn.map_cpu(userd, RING_OBJECT_BYTES, ...)),
            UserdOwner::HandedIn => None,
            UserdOwner::InRing   => None,
        };
```

with the reason stated in the comment at `crates/kayfabe-isolate-host/src/rm.rs:8216-8231`:

> ★★★★★ **G4 — THE CPU MAP OF THE RING IS CONDITIONAL, AND THE CONDITION IS PROVENANCE.**
> ⊘ On a guest-backed ring this is not an omission we can get away with; it is a call that
> **cannot succeed**. `map_cpu` issues `NV_ESC_RM_MAP_MEMORY` against the memory object, and the
> object here is an `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over pages RM pinned out of another
> process's address space. **R31 arm B attempts it deliberately and prints what the driver
> answered**, so the claim in this comment is a measurement rather than a plausible sentence.

and the USERD twin at `:8248-8253`:

> ★★★★★ **LEG B's G4 … `[measured, R31 arm B]` an `OS_DESCRIPTOR` over another process's pages
> cannot be CPU-mapped at all**, so on the `HandedIn` arm this is a call that *would fail*, not
> one we are choosing to skip. Every access it would have served is refused by name instead
> (`USERD_NOT_OURS`).

⇒ ★★★★★ **For the guest passthrough channel — the only shape this design question is about — the
birthing process makes NO CPU mapping of the ring and NO CPU mapping of USERD.** There is nothing
for a scratchpad-created mapping to be unusable *as*, because no mapping is created. The
`ProcessId` filter of §3.1 has no object to filter.

⊘ The `Ours` arms still map, and those are **our own** channels (the CE-copy channel and the CUDA
walk kernel's channel, `crates/kayfabe-isolate-host/src/rm.rs:1337-1341`,
`crates/kayfabe-isolate-host/src/cudawalk.rs`). Those already live in the scratchpad under
Constraint 26 (`docs/design/THE_CONSTRAINTS.md:255-264`), so creator and driver are the same
process there by construction, today and under the proposal.

### 3.4 The doorbell needs no channel handle and no channel mapping

The ring is a store into a **per-subdevice `*_USERMODE_*` window**, not into anything owned by the
channel — `HostRmBackend::open_usermode`, `crates/kayfabe-isolate-host/src/rm.rs:1992-2001`:

```rust
    fn open_usermode(&self, class: UsermodeClass) -> Result<UsermodeWindow, RmError> {
        let want = self.mint();
        let object = self.raw_alloc(self.subdevice, want, class.usermode_id().0, &mut [])?;
        self.remember(object, self.subdevice);
        let (node, region) = self.map_cpu(object, USERMODE_WINDOW_SIZE, CachePolicy::WriteBack)?;
```

and the store itself, `crates/kayfabe-isolate-host/src/rm.rs:2041-2075`: `release_fence()` then one
32-bit store of the token at `USERMODE_NOTIFY_CHANNEL_PENDING` inside that window.

⇒ ★★ **Ringing a channel requires (a) the driving process's own usermode window — allocated under
its own subdevice, in its own client — and (b) the 32-bit work-submit token, which is a *value*,
not a handle.** The token is obtained once at birth by the birthing client
(`NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN`, `crates/kayfabe-isolate-host/src/rm.rs:8203-8210`)
and returned as the second half of `ChannelHandles` (`crates/kayfabe-isolate/src/lib.rs:2356`).
**Under scratchpad birth, S obtains it and hands over a `u32`. The isolate never needs to name the
channel to ring it.**
⊘ Not established from source: whether RM or hardware validates *who* stores to the usermode
window. The window is a per-client object and the store is a raw MMIO write with no RM
involvement; **INFERRED** that there is no such check, because there is no software in the path.
Confirming it is the probe in §7.

### 3.5 ★★★★★ And the crossing itself is already MEASURED in this tree, unprivileged

Even where a CPU view *is* wanted on both sides, an armed RM device node crosses a process
boundary as an fd. The RM/kernel reason: `nvidia_mmap_helper`,
`kernel-open/nvidia/nv-mmap.c:506-531`, validates **only that this file descriptor carries a valid
mmap context** —

```c
    /*
     * If mmap context is not valid on this file descriptor, this mapping wasn't
     * previously validated with the RM so it must be rejected.
     */
    if (!smp_load_acquire(&mmap_context->valid))
    { nv_printf(NV_DBG_ERRORS, "NVRM: VM: invalid mmap\n"); return -EINVAL; }
```

— and the context is armed **per-fd** by `NV_ESC_RM_MAP_MEMORY`
(`kernel-open/nvidia/nv-usermap.c`, `nvamc->valid`). **There is no caller identity anywhere in the
path.**

Our own measurement of exactly that, recorded at
`docs/design/the_counter_page_and_the_device_view.md:68-84`, `rmladder --bar1-crossing` on GA106
**at euid 1002, non-root**:

    W393 LEG A = ONE MEMORY: all 64 words the child stored through node A read back through
                 node B in THIS process.  => an armed device node CROSSES a process boundary.
    W393 device slot = KVM_SET_USER_MEMORY_REGION accepted a VM_PFNMAP device view as memslot
    W393 guest exit  = the ONLY exit is the signal store; the data store and the data load
                       took NO exit
    W393 LEG B = a guest CPU store took no VM exit and landed in card memory another CPU
                 view reads.

Probe source: `crates/kayfabe-isolate-host/src/bin/rmladder.rs:1376-1700` — note `:1655-1660`,
*"Our copy of A goes NOW. From here the child's descriptor is the only one, so a mapping the child
makes is a mapping this process could not have made for it."*

⊘ **One limit, measured and recorded in the same doc** (`:88-98`, w596): `NVOS33_FLAGS_ACCESS_READ_ONLY`
does **not** make the resulting `mmap` read-only — `PROT_WRITE` was accepted on a node armed
read-only. So fd-passing hands over a **read-write** capability, and the read-only primitive that
§3 of that doc assumed is absent. That bounds what fd-passing can be used to *contain*; it does not
affect whether the crossing works.

### 3.6 §3 verdict

| question | answer | evidence |
|---|---|---|
| Does RM refuse a cross-process CPU mapping? | **No.** It *fails to find* it on unmap/lookup. `osAttachToProcess` is a Linux no-op. | `os.c:677-692`; `rs_utils.c:325-332`; `mapping_list.c:84` |
| Could the isolate use a mapping S created via RM? | **Not by RM address lookup** — the filter would miss it. **Yes by fd**, which is a different mechanism and is measured. | `rs_utils.c:330-331`; `nv-mmap.c:506-531`; `the_counter_page_and_the_device_view.md:68-84` |
| Does the driving process need a CPU map of the channel? | **Impossible on our shape, for anyone.** | `kernel_channel.c:1291` |
| …of the ring or USERD? | **Not for a guest passthrough channel** — the birth path maps neither, and the calls would fail. | `rm.rs:8212-8253` |
| …of the doorbell page? | **Yes — but it is the isolate's OWN per-subdevice usermode object**, unrelated to who birthed the channel. | `rm.rs:1992-2001`, `:2041-2075` |

★ **§3 is not a blocker.** It was the most plausible blocker on paper, and the evidence in both
trees removes it.

---

## §4 — What our own tree assumes

⊘ **Start here: the question is already asked and answered in this tree, and the answer is a live
refusal.** `docs/design/THE_CONSTRAINTS.md:415-417`:

> ⇒ **"The scratchpad births the channel and dups it to the isolate" is WITHDRAWN as a default.**
> It is admissible only if measured: **scratchpad-born channels come out `_PRIVILEGE_USER`**,
> asserted at birth and refusing otherwise.

enforced by `SCRATCHPAD_BIRTH_IN_A_HANDED_SPACE` (constant `crates/kayfabe-isolate-host/src/rm.rs:1345`,
fired `:7794-7806`, placed *above the first allocation* so the refusal has nothing to unwind).
⇒ **This research question is a request to cross a named, deliberate gate.** §4 enumerates what
else is standing behind it.

### 4.1 The load-bearing mechanisms, each independently cited

| # | mechanism | `file:line` | what it encodes |
|---|---|---|---|
| 1 | `Worker::execute` foreign-handle gate | `crates/kayfabe-isolate/src/lib.rs:3862-3868` | every handle in a plan must belong to the executing worker's `(ProcId, GpuId)` |
| 2 | `HostHandle::belongs_to` | `crates/kayfabe-isolate/src/lib.rs:184-193` | compares **both** halves of `IsolateId` |
| 3 | `VerbPlan::handles()`, `ChannelBirth` arm | `crates/kayfabe-isolate/src/lib.rs:3095-3103` | ring + USERD + host-VAS handles are checked by #1 |
| 4 | `RING_NOT_A_JOINED_WINDOW` | `crates/kayfabe-isolate-host/src/rm.rs:1520-1539`, fired `:6997-7003` | *"this isolate did not mint that object by joining a framebuffer leaf"* |
| 5 | `USERD_NOT_A_JOINED_WINDOW` | `crates/kayfabe-isolate-host/src/rm.rs:1517` | same, for USERD |
| 6 | `RING_HANDLE_REACHED_RM` | `crates/kayfabe-isolate-host/src/rm.rs:1315`, fired `:8094-8105` | the w745 successor assert: a store-slice birth must name handle 0 |
| 7 | `OwnClient` / **F11** | `crates/kayfabe-isolate-host/src/rm.rs:273`, `:292-320`; doc `:231-240` | *"an `OwnClient` value exists"* and *"this process allocated that client"* are **one statement** |
| 8 | `ScratchpadRole` + `HandedVaSpace` | `crates/kayfabe-isolate-host/src/rm.rs:337-453` | the **one** typed exception to #7, and only for a VA space handed UP |
| 9 | `ADOPT_NOT_THE_SCRATCHPAD` | `crates/kayfabe-isolate-host/src/rm.rs:1281`, fired `:5709-5732` | a per-proc isolate may not adopt |
| 10 | `SCRATCHPAD_BIRTH_IN_A_HANDED_SPACE` | `crates/kayfabe-isolate-host/src/rm.rs:1345`, fired `:7794-7806` | **the exact refusal this question proposes to cross** |
| 11 | per-`(proc,gpu)` worker checkout | `crates/kayfabe-fwd/src/lib.rs:1588-1600`, `:1650` | the driving verbs run on the proc's own isolate by construction |
| 12 | birth requires the *routed proc's* isolate | `crates/kayfabe-fwd/src/lib.rs:5485-5493` | `IsolatePending` / `NoTarget` otherwise |
| 13 | commit writes the proc's own channel | `crates/kayfabe-fwd/src/lib.rs:5612-5614` | host channel + token stored per-proc |

Isolate identity is **`(ProcId, GpuId)`** (`crates/kayfabe-isolate/src/lib.rs:1963-1970`), and
`ProcId` is *not* a PID, CR3 or token — it is a derived label for one dup-connected component of
declared user clients (`crates/kayfabe-core/src/lib.rs:205-212`,
`crates/kayfabe-core/src/project.rs:188-205`). ⊘ That matters for §1: **our notion of "guest
process" has no relationship to RM's `ProcessID` today**, so nothing in our tree currently *reads*
the RM stamp. What RM's stamp buys us is whatever RM itself does with it — §1's census.

### 4.2 The ownership split, as the constraints state it

`docs/design/THE_CONSTRAINTS.md:255-264` (Constraint 26):

| | **owns** | **sees** |
|---|---|---|
| per-proc **isolate** | channel, VA space, compute, control | **only its own VA space** |
| **scratchpad** | the vidmem GPGA object, the tables, bounded kernel channels | all vidmem, all VA spaces |

> ⇒ **A per-proc isolate never holds an `hMemory` for vidmem and never receives the guest-RAM
> memfd.** Isolates remain keyed on VA spaces.

★ Note the **direction** the current design is careful about — `tests/tests/vaspace_handover_asserts.rs:138-158`:

> ★★★★★ **CONSTRAINT 30.** … the direction of this hand-over is the whole of its safety: the
> space is created by the per-proc isolate and lent **UP** to the scratchpad. A space the
> scratchpad created and lent **DOWN** would stamp every channel born in it with the scratchpad's
> privilege.

⇒ Scratchpad birth is the *inverse* of the direction the current design chose, and the gate at
`rm.rs:7794-7806` is precisely the "no lending down" enforcement.

### 4.3 ⊘⊘⊘ THE PREMISE IN THE BRIEF IS NOT ESTABLISHED — and this tree already says so

The brief states: *"★ An **unprivileged** scratchpad yields `rmclientIsAdmin == false` ⇒
`_PRIVILEGE_USER`, so the *privilege* stamp is not a cost."*

`docs/design/THE_CONSTRAINTS.md:414-415` contradicts the antecedent:

> ⚠ And F11 records that our isolates' kernel-visible euid is **0** on a root VMM, so
> `rmclientIsAdmin(...)` plausibly holds for the scratchpad — which would make every channel it
> births `_PRIVILEGED_CHANNEL_TRUE`.

The reason, from `crates/kayfabe-isolate-host/tests/own_client_invariant.rs:1-14`:
`surrender_privilege` drops **capabilities, not uid**, so on a root VMM the isolate's euid as the
host kernel sees it is 0.

And the driver-side predicate, `src/nvidia/src/kernel/rmapi/client.c:384-394`:

```c
    return (privLevel >= RS_PRIV_LEVEL_USER_ROOT) && !pClient->bIsRootNonPriv;
```

with `privLevel` set in `src/nvidia/arch/nvalloc/unix/src/escape.c:304`:

```c
    secInfo.privLevel = osIsAdministrator() ? RS_PRIV_LEVEL_USER_ROOT : RS_PRIV_LEVEL_USER;
```

resolving through `src/nvidia/arch/nvalloc/unix/src/os.c:614-617` →
`kernel-open/nvidia/os-interface.c:377-381` → `kernel-open/common/inc/nv-linux.h:537`:
`#define NV_IS_SUSER() capable(CAP_SYS_ADMIN)`.

⇒ ★ **`privLevel` is a property of the CALLING TASK'S CAPABILITY AT EACH IOCTL, not of the
client** (though `client.c:95` also caches it as `cachedPrivilege`). A scratchpad that must be
privileged to mint `NV01_MEMORY_LIST_OBJECT` is, by that same fact, `rmclientIsAdmin == true` —
**unless** `bIsRootNonPriv` is set. See §6.1: there is a mechanism for exactly that, and we are
not using it.

⊘ The other disjunct at `kernel_channel.c:285`, `hypervisorCheckForObjectAccess(hClient)`, is
**dead in the open driver**: `src/nvidia/src/kernel/virtualization/hypervisor/hypervisor_access.c:32-40`
returns `NV_FALSE` unconditionally. **Unmeasurable from source whether the closed driver differs.**

### 4.4 ★ Constraint 29 — what "restating" costs here

`docs/design/THE_CONSTRAINTS.md:358-391` obliges: *"**Restate, do not remove**… **THE REPLACEMENT
MUST TEST THE ARGUMENT THAT RETIRED THE OLD ONE**… ⊘ Never go green by shrinking the universe the
gate quantifies over… ⚠ A retired assert is a commit-message obligation."*

So, for each mechanism in §4.1, the classification:

| # | mechanism | verdict | why |
|---|---|---|---|
| 1, 2 | `Worker::execute` / `belongs_to` | **RESTATE** | the failure (a raw handle value is live-and-different in every other client, `lib.rs:113-127`) is untouched. The gate must become "the handle belongs to the isolate this *verb* is for", with birth verbs routed to S. ⚠ That is a widening, and the successor must go red if a *driving* verb ever carries an S-minted handle. |
| 3 | `ChannelBirth::handles()` | **RESTATE** | the arm must classify S's handles as S's; Constraint 26's existing `StoreSlice` case already shows the shape (`lib.rs:3087-3094`) |
| 4, 5 | `RING_/USERD_NOT_A_JOINED_WINDOW` | **UNTOUCHED** by S-birth *per se* — already superseded by Constraint 26's `StoreMapPort::is_slice_of_the_store` (`crates/kayfabe-qemu-raw/src/storemap.rs:309-331`, oracle `crates/kayfabe-fwd/src/lib.rs:6863-6886`). ⚠ But under S-birth the ring/USERD would be **S's own** store slices, so the oracle's question ("did *this* port place this slice?") becomes trivially yes and stops discriminating. **That is a restate obligation, and it is the one most likely to be missed.** |
| 6 | `RING_HANDLE_REACHED_RM` | **UNTOUCHED** — still the right assert, still fail-closed |
| 7 | `OwnClient` / F11 | **BREAKS, then RESTATES** — the isolate would have to name a channel it did not mint. Today that is structurally unspellable. The existing precedent for widening it *by a type* is `HandedVaSpace` (§4.1 #8), and Constraint 29 clause 3 names that as the approved shape. ⚠ ⊘ **But see §2/§6: it may not need widening at all** — if the isolate never names the channel, F11 stands unmodified. |
| 8, 9 | `ScratchpadRole` / `HandedVaSpace` / `ADOPT_NOT_THE_SCRATCHPAD` | **UNTOUCHED** |
| 10 | `SCRATCHPAD_BIRTH_IN_A_HANDED_SPACE` | **THE GATE ITSELF.** Constraint 29 clause 2 applies in its strongest form: it was installed because *"a channel born here would carry the scratchpad's privilege into a guest proc's space for its whole life."* Retiring it requires the successor to go **red if a scratchpad-born channel ever comes out `_PRIVILEGED_CHANNEL_TRUE`** — i.e. a live read-back of the stamp, not an argument. ⊘ **See §7: that read-back may not be available from userspace.** |
| 11, 12, 13 | routing / checkout / commit | **RESTATE** — mechanical; birth routes to S, driving stays per-proc. No failure is guarded here, only plumbing. |

### 4.5 The tests that would go red

`tests/tests/cross_proc_lifetime.rs:35-37` states the invariant verbatim:

> **No host object is ever released, unmapped, or operated on through an isolate other than the
> one that minted it — on any teardown ordering, however adversarial.**

Exact-variant assertions on `RmError::ForeignHandle` at `:357-400`, `:389`, `:451`, `:492`, `:522`,
`:556`, `:616`, `:641`, `:744`, `:821`, `:1081`, `:1130`.
Further: `tests/tests/concurrency_stress.rs:204-270` (`assert_verb_in_namespace`, `AllocChannel`
arm destructured with **no `..`**, `:239-253`);
`tests/tests/vaspace_handover_asserts.rs:138-158`;
`crates/kayfabe-isolate-host/tests/ownership_split_gates.rs:109-159`
(`the_scratchpad_cannot_birth_a_channel_in_a_space_it_adopted` — asserts the gate's *exact
predicate*, that `adopted_spaces` is written, and the ordering dup < remember < range);
`tests/tests/the_birth_names_the_guests_ring.rs:775-786` (asserts the birth plan offers the
per-proc worker **zero** foreign handles);
`crates/kayfabe-isolate-host/tests/export_backing.rs:417-440`;
`crates/kayfabe-isolate-host/tests/guest_ring_census.rs:221-240`, `:408`, `:530`, `:605`, `:629`;
`crates/kayfabe-isolate-host/tests/own_client_invariant.rs` (F11, `:451`, `:485-520`, `:522`,
`:548-563`, `:581-593`);
`crates/kayfabe-isolate-host/tests/real_isolate.rs:295`.

⊘ **Reported empty:** `unranked_locks` (`tests/tests/unranked_locks.rs`) has **no relation** to
process/resource ownership — it is `l1_concurrency.md` §3.3.1's INLINE-SAFE clause (c) for
unranked locks (`:1-30`). It appears in this search space only as a cautionary instrument. It is
**untouched** by this question.

---

## §5 — `nvkvm-pv`, the shipped Mode-1 sibling

Repo `/workspace/nvkvm-pv`, HEAD `368d2db`.

### 5.1 Who creates channels, and where

**The isolate stub — one freestanding, unprivileged host process per *guest process*.** Not QEMU,
not a daemon.

- Cardinality stated three times: `src/common/nvkvm_proto.h:24-30`
  (*"Each guest userspace process (identified by `mm_struct`) has exactly one 'isolate' host
  process"*), `docs/internal/isolate-model.md:3-5`, `ARCHITECTURE.md:819-823`.
- The `ioctl(2)` that creates a channel executes at exactly one line:
  `src/stub/nvkvm_stub.c:1824` (`stub_ioctl(fd, job.cmd, job.param_buf)`), wrapper at `:437-440`.
- `ARCHITECTURE.md:160-162`: *"**Step 12 runs in a process, not in QEMU.** The isolate has the
  guest process's address-space layout mirrored and its own RM client."*

### 5.2 ★ Why it must be the stub — and it is **not** a security reason

`docs/internal/isolate-model.md:8-21` and `ARCHITECTURE.md:824-831`:

- `rmclientValidate` compares the RM client's `pOSInfo` against the calling task's `nvfp`;
- UVM binds its file's `nvfp` to the calling task's `mm` during `UVM_INITIALIZE`;
- `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` pins the **calling task's** pages.

> *"The isolation property is a consequence of satisfying the driver, not the other way round."*
> — `docs/internal/isolate-model.md:20-21`

★★★ **This is the single most transferable finding for the decision.** The Mode-1 sibling did not
choose per-process host processes for isolation; it chose them because **RM refuses to work
otherwise**, and isolation fell out. ⇒ The question *"may S birth the channel?"* is, on that
precedent, a question about **which RM state is task-bound**, not about which is
privilege-bound. ⊘ Note our shape differs: we pass guest pages as `OS_DESCRIPTOR`s built from a
memfd, and our per-proc isolate is already the one that pins them.

### 5.3 ★ There IS a precedent for one host process serving many guest processes — twice

Neither is for channels, and both are annotated as deliberate exceptions.

**(a) QEMU's own "admin" RM client, one per VM.** `src/qemu/nvkvm_isolate_handlers.c:1664-1671`,
implementation `nvkvm_admin_ensure()` `:1696-1728` (allocates `NV01_ROOT` `hClient 0xad000001`,
`NV01_DEVICE_0`, `NV20_SUBDEVICE_0` via `admin_rm_alloc()` `:1678-1692`, then
`NV_ESC_RM_CONTROL` at `:1788`). It exists because the stub lives inside `CLONE_NEWPID`/`NEWUSER`
where `GET_PID_INFO` attributes 0 bytes. **Query-only; allocates no channel; never accepts a
guest-named pid** (`:1734-1737`).

**(b) All UVM ioctls execute in QEMU's process, for every guest process.**
`src/qemu/nvkvm_isolate_handlers.c:1229-1231` (*"UVM ioctls execute in QEMU's (privileged)
process"*), call at `:3050`, `UVM_REGISTER_CHANNEL` (27) among them at `:1269-1271`.
★ **And how guest processes are kept apart inside that one process is directly on point** —
`src/qemu/nvkvm_isolate_handlers.c:1279-1284`:

> *"Per-HANDLE, deliberately: each guest UVM fd gets its own QEMU-side `/dev/nvidia-uvm` fd and
> therefore its own `uvm_va_space`, and two guest processes legitimately pick the SAME base
> (measured: isolates 8 and 10 both create `0x200000000`). A per-VM or process-wide VA
> reservation would break that; range ownership is only meaningful inside one `va_space`."*

⇒ **One host process, many guest processes, separated by holding a separate device *fd* per guest
process plus a handle-keyed ownership table — not by PID.** That is the closest existing precedent
for what scratchpad birth would require.

### 5.4 ★★★ The `RS_SHARE_TYPE_PID` precedent — a shipped workaround for exactly this problem

`ARCHITECTURE.md:848-854`:

> *"The kernel's default share policy is `RS_SHARE_TYPE_PID`, granting `DUP_OBJECT` only when the
> caller's PID matches the resource owner's. In a split-process model those PIDs differ, so UVM's
> kernel-internal client cannot dup `libcuda`'s VA space and `cuCtxCreate` fails with
> `NV_ERR_INSUFFICIENT_PERMISSIONS`."*

The shipped fix — `src/qemu/nvkvm_isolate_handlers.c:4089-4108` (reasoning), `:4150-4219`
(implementation): after every successful alloc, issue `NV_ESC_RM_SHARE` (`0xc0184635`) with
`.accessMask = 0x1 /* RS_ACCESS_DUP_OBJECT */` and `.type = 1 /* RS_SHARE_TYPE_ALL */`, **executed
on the stub fd** (`:4210`) because *"the share runs on the stub fd so the owner check inside
`_serverShareResource` matches (caller process == resource owner)"* (`:4103-4105`).

⇒ ★★★★★ **`NV_ESC_RM_SHARE` with `RS_SHARE_TYPE_ALL` is a measured, shipping escape from the
PID-matched dup policy, and it must be issued BY THE OWNER.** Under scratchpad birth, S is the
owner, so S can issue it. That is the concrete answer to "how would the isolate get dup rights"
— see §2.

⊘ Two caveats, both recorded by the sibling itself:
- ⚠ **A live comment/code contradiction**: the block comment at
  `src/qemu/nvkvm_isolate_handlers.c:4099-4102` says the share is scoped `RS_SHARE_TYPE_PID`; the
  initializer 100 lines below at `:4204` is `RS_SHARE_TYPE_ALL`, and `ARCHITECTURE.md:856` agrees.
  **The comment is stale; the shipped type is host-wide.** Do not cite the comment.
- The argument that `TYPE_ALL` is not a cross-tenant hole (reach-gating happens at
  `clientGetResourceRef`, not at the share policy) is at `:4177-4200`, demonstrated by
  `tests/security/poc_cross_proc_dup.c`. ⊘ That argument is the sibling's; it is **not
  independently verified here**.

### 5.5 Recorded reasoning about PID-keyed isolation

★ The principal-selection note, `src/guest/nvkvm_session.c:36-46`:

> *"The security PRINCIPAL is the address space (`mm`), not the `tgid`. This is deliberate and
> matches the only sane boundary: nvidia keys access on the tgid (`RS_SHARE_TYPE_PID =
> current->tgid`) and a thread group always has exactly one mm… (mm is also the robust key: tgids
> get recycled — audit H2.)"*

Host-side keys are `session_id` (guest `mm_struct`) / `isolate_id` / `handle_id`
(`src/common/nvkvm_proto.h:31-37`, `:326`, `:333`) — **never a host PID**.
⚠ And the `hClient` allowlist is **per-VM, not per-process**
(`src/qemu/nvkvm_isolate_handlers.c:1600-1603`), with the gap stated outright in
`SECURITY.md:41-51`: *"**This boundary is not currently closed.**… Closing it properly needs a
caller session id in the protocol, not a patch."*

⊘ **Reported empty — and this is a real bound on §5's usefulness.** The following return **zero
hits repo-wide** in `/workspace/nvkvm-pv` (`.git/` excluded): `ProcessID`, `SubProcessID`,
`osGetCurrentProcess`, `RmClient`, `hUserdMemory`, `userdMemory`, `NvRmMapMemory`, `usermodeArea`,
`nv_mmap`, `client-per-guest`. ⇒ **There is no recorded consideration of RM's `ProcessID` /
`SubProcessID` fields anywhere in the sibling.** If the owner wants to know whether a
shared-client-with-`SubProcessID` design was evaluated and rejected, the answer from that repo is:
**it was never written down.**

### 5.6 Privilege posture

- **The stub never runs privileged**, deliberately, with a five-rung degradation ladder and
  measured probes: `docs/internal/isolate-model.md:176-296`; drop/harden at
  `src/qemu/nvkvm_isolate.c:1713-1720`; 20-syscall seccomp allowlist at
  `src/stub/nvkvm_stub.c:3360-3410`.
- **QEMU is the privileged party and is declared trusted**: `SECURITY.md:64-66` (*"The VMM and the
  host are trusted; the guest and the isolates are not"*); the one guest-named ioctl it executes is
  UVM (`ARCHITECTURE.md:843-846`), guarded by a 31-row default-deny schema allowlist.
- ⊘ **That posture differs from kayfabe's Constraint 17** (*"Host userspace stays UNPRIVILEGED.
  Standing, absolute"* — `docs/design/THE_CONSTRAINTS.md:105`). The sibling's split is
  privileged-VMM + unprivileged-stub; ours is meant to be unprivileged throughout. **The sibling is
  therefore a precedent for the mechanism and NOT for the posture.**

### 5.7 And the submission plane needs no host process at all

`ARCHITECTURE.md:1024-1041`:

> *"Once a CUDA channel exists, launching work is a **store to a mapped page**… It touches the
> guest kernel module: no. The virtqueue: no. QEMU: no. The stub: no. There is no VM exit… **There
> is no doorbell interception anywhere in the tree. That absence is the design.**"*

The stub's own mirror mapping exists only for ioctl-time pointer dereference
(`src/qemu/nvkvm_isolate_handlers.c:4760-4767`), not for submission. ⇒ **corroborates §3.4 from an
independent codebase: the process that drives a channel is not, in general, the process that must
hold CPU mappings of it.**

---

## §2 — RM rights and access: if S creates the channel, can I use it at all?

### 2.0 ★★★★★ The headline: **a `KernelChannel` cannot be duped. At all. By anyone.**

This is the single most consequential finding in this document, and it is unconditional — it does
not depend on privilege, on PID, or on any share policy.

`serverCopyResource` refuses before rights are ever consulted —
`src/nvidia/src/libraries/resserv/src/rs_server.c:1719-1723`:

```c
    if (!resCanCopy(pResourceRefSrc->pResource))
    {
        status = NV_ERR_INVALID_ARGUMENT;
        goto done;
    }
```

`KernelChannel` has **no `CanCopy` override**. `grep -rn "kchannelCanCopy_IMPL\|kchannelCopyConstruct" src/`
returns **empty**; the only hits for `kchannelCanCopy` are NVOC glue, and the vtable slot is
`src/nvidia/generated/g_kernel_channel_nvoc.c:708`:

```c
    .vtable.__kchannelCanCopy__ = &__nvoc_up_thunk_RsResource_kchannelCanCopy,
```

which thunks straight to the base (`g_kernel_channel_nvoc.c:862-865`), and the base is
`src/nvidia/src/libraries/resserv/src/rs_resource.c:333-340`:

```c
NvBool resCanCopy_IMPL(RsResource *pResource) { return NV_FALSE; }
```

⊘ Contrast, from the full `grep -rn "CanCopy" src/nvidia/` census: the **TSG**
(`kchangrpapiCanCopy`, `src/nvidia/src/kernel/gpu/fifo/kernel_channel_group_api.c:798-800`), the
**context share** (`kctxshareapiCanCopy`, `src/nvidia/src/kernel/gpu/fifo/kernel_ctxshare.c:327-333`),
the **usermode object** (`usrmodeCanCopy`, `src/nvidia/src/kernel/gpu/usermode_api.c:108-110`), every
`Memory` subclass, and `VaSpaceApi` all return `NV_TRUE`. **The channel is the exception.**

### 2.1 And I cannot allocate engine objects on S's channel either

The alloc parent handle is looked up **only in the calling client's own handle map** —
`src/nvidia/src/kernel/rmapi/alloc_free.c:739-743`:

```c
        status = clientGetResourceRef(pClient, hParent, &pParentRef);
        if (status != NV_OK) goto done;
```

and `clientGetResourceRef_IMPL`, `src/nvidia/src/libraries/resserv/src/rs_client.c:381-392`:

```c
    pResourceRef = mapFind(&pClient->resourceMap, hResource);
    if (pResourceRef == NULL) return NV_ERR_OBJECT_NOT_FOUND;
```

(the same lookup again at `rs_client.c:659-661`). `AMPERE_COMPUTE_B` and `AMPERE_DMA_COPY_B`
require a `KernelChannel` parent (`src/nvidia/src/kernel/rmapi/resource_list.h:1605`, `:2016`).

⊘ **There is no cross-client parent mechanism.** `hParentClient` appears only in GSP RPC
marshalling (`src/nvidia/inc/kernel/vgpu/rpc.h:349`) and NV0005 event internals
(`src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:1353`, `:1586`;
`src/nvidia/arch/nvalloc/unix/src/rmapi_specific.c:74` actually **rejects**
`hParentClient != hClient`). `hClientSrc`/`hSrcClient` appear only on the dup path. `pRightsRequested`
(`alloc_free.c:153-180`) lets the allocator request rights on the object it is *creating*; it names
no second client.

### 2.2 Controls and free are likewise scoped to the handle-holding client

- Controls dispatch on `pRmCtrlParams->pResourceRef`, resolved in the invoking client. No PID check
  exists inside `kchannelCtrlCmdGpFifoSchedule_IMPL`
  (`src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:3086`, checks only
  `kchannelIsSchedulable_HAL` at `:3105`) or `kchannelCtrlCmdBind_IMPL` (`:3172`). Both carry
  `accessRight = 0x0u` (`src/nvidia/generated/g_kernel_channel_nvoc.c:317-318`, `:332-333`).
  ⊘ Two channel controls *do* require a right: `RESTART_RUNLIST` (`0xa06f0111`) and
  `SET_INTERLEAVE_LEVEL` (`0xa06f0109`) carry `accessRight = 0x2u` = `RS_ACCESS_NICE`
  (`src/nvidia/generated/g_kernel_channel_nvoc.c:377-378`, `:362`;
  `src/common/sdk/nvidia/inc/rs_access.h:60`), enforced at
  `src/nvidia/src/kernel/rmapi/control.c:751`.
- Free is looked up in `pParams->hClient`'s own map —
  `src/nvidia/src/libraries/resserv/src/rs_server.c:1125-1173`. **A client cannot free another
  client's channel.**

### 2.3 ⇒ The answer to §2's question

| can I (the isolate)… | answer | evidence |
|---|---|---|
| name S's channel at all? | **No.** Dup is refused unconditionally. | `rs_server.c:1719-1723` + `resCanCopy_IMPL` `rs_resource.c:333-340` |
| allocate a compute/CE object on it? | **No.** Parent must be in my own map. | `alloc_free.c:739-743`, `rs_client.c:381-392` |
| schedule / bind it? | **No** — the control needs a ref in my client. | `control.c:751-752` |
| free it? | **No.** | `rs_server.c:1125-1173` |
| ★ **ring it?** | **YES** — and this is the one that matters. | §3.4: my own usermode window + a `u32` token |

⇒ ★★★★★ **Under scratchpad birth, S does not "birth and hand over". S owns the channel for its
entire life** — birth, every engine object, `BIND`, `GPFIFO_SCHEDULE`, and free — **and the isolate's
role collapses to ringing a doorbell with a token.** That is a far larger change than the brief's
framing ("can I use it at all?") implies, and it is not a rights problem that a grant can fix: the
channel is simply not a shareable object in this driver.

⊘ **The dup would also fail for a second, independent reason even if `CanCopy` were true**, and it
is worth recording because it is the reason `nvkvm-pv` hit this: the default inherited share policy
is `RS_ACCESS_DUP_OBJECT` granted by `RS_SHARE_TYPE_PID`
(`src/nvidia/src/kernel/rmapi/sharing.c:338-352`), and `RS_SHARE_TYPE_PID` is
`pSrcClient->ProcID == pDstClient->ProcID` (`src/nvidia/src/kernel/rmapi/client_resource.c:217-231`).
S and I are different OS processes, so the default grants nothing. The rights check itself is
`clientCopyResource_IMPL`, `src/nvidia/src/libraries/resserv/src/rs_client.c:543-551`.

⊘ **And there are only FOUR access rights in the whole driver** — `RS_ACCESS_DUP_OBJECT`,
`RS_ACCESS_NICE`, `RS_ACCESS_DEBUG`, `RS_ACCESS_PERFMON`
(`src/common/sdk/nvidia/inc/rs_access.h:59-63`). Independently confirmed: **all 197 `RS_ENTRY`
blocks in `resource_list.h` require `RS_ACCESS_NONE`** —
`grep "Required Access Rights" src/nvidia/src/kernel/rmapi/resource_list.h | sort | uniq -c` →
one line, `197 /* Required Access Rights */ RS_ACCESS_NONE`; the `grep -v RS_ACCESS_NONE`
complement is **EMPTY**. Rights are checked only at dup (`rs_client.c:551`), share/grant
(`rs_client.c:176`), control dispatch (`control.c:751`), the SM debugger
(`src/nvidia/src/kernel/gpu/gr/kernel_sm_debugger_session.c:295-301`), the SMC-partition dup
exception (`src/nvidia/src/kernel/gpu/gpu_resource.c:204`) and UVM's internal dup
(`src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:735`). ⇒ **"What `RS_ACCESS_*` rights would the isolate
need?" has no answer, because there is no right that would help.** Access in RM is
*handle reachability within a client*, and the channel cannot be made reachable.

### 2.4 The share verb, for completeness

`NV0000_CTRL_CMD_CLIENT_SHARE_OBJECT` (`0xd06`) is
`cliresCtrlCmdClientShareObject_IMPL`, `src/nvidia/src/kernel/rmapi/client_resource.c:5109`. It
looks the object up **in the caller's own client** (`:5128`), so only an owner may publish policy on
it, and `clientCanShareResource_IMPL` (`src/nvidia/src/libraries/resserv/src/rs_client.c:157`,
`:176`) requires the sharer to already hold the rights it grants. `RS_SHARE_TYPE_CLIENT` additionally
resolves the target and registers a back-ref (`client_resource.c:5161-5165`).
`GET_ACCESS_RIGHTS` is `:5069`; `SET_INHERITED_SHARE_POLICY` is `:5093` and delegates to
`ShareObject` at `:5106`.
⇒ **All of this works, and none of it helps for a channel**, because `CanCopy` is checked *before*
rights (`rs_server.c:1719` precedes `clientCopyResource` at `:1741`).

---

## §6 — The counter-case: can the ISOLATE birth the channel while naming memory it does not hold?

The brief asks for this and warns *"⊘ `NV_CHANNEL_ALLOC_PARAMS` has **no** client field beside
`hUserdMemory[]` (checked). Look wider before concluding there is none."*

**Confirmed and widened.** The complete handle inventory of `NV_CHANNEL_ALLOC_PARAMS`
(`src/common/sdk/nvidia/inc/alloc/alloc_channel.h:298-333`) is: `hObjectError` (`:298`),
`hObjectBuffer` (`:299`, *"no longer used"*), `hContextShare` (`:306`), `hVASpace` (`:307`),
`hUserdMemory[NV_MAX_SUBDEVICES]` (`:310`), `hObjectEccError` (`:321`), `hPhysChannelGroup`
(`:328`, *"reserved"*). **None has a companion client field.** The GPFIFO ring is named by
`gpFifoOffset` (`:300`) — a **VA in the channel's own VAS, not a handle** — so the ring never needs
one.

⊘ **And the `ProcessID`/`SubProcessID` fields in the params (`:332-333`, marked "reserved") are a
dead end for a userspace caller** — CPU-RM zeroes them on entry,
`src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:217-226`:

```c
    // Internal fields must be cleared when RMAPI call is from client
    ...
    pChannelGpfifoParams->ProcessID = 0;
    pChannelGpfifoParams->SubProcessID = 0;
```

and the branch that *honours* them is gated `if (RMCFG_FEATURE_PLATFORM_GSP)` (`:246`) — it runs
only inside **GSP firmware**, which receives already-stamped values over RPC (`:2814-2815`).
⇒ **There is no way for a userspace caller to supply a channel's `ProcessID`.**

### 6.1 ★★★★★ COUNTER-CASE A — `MemoryList` **is** dupable, and the channel resolves USERD in the *allocating* client

`memlistCanCopy_IMPL` — `src/nvidia/src/kernel/mem_mgr/mem_list.c:787-794`:

```c
NvBool memlistCanCopy_IMPL(MemoryList *pMemoryList) { return NV_TRUE; }
```

⇒ **`NV01_MEMORY_LIST_OBJECT` — the very class whose `RS_FLAGS_ALLOC_PRIVILEGED` started this whole
question — can be duped into another client.**

And the dup path **never re-checks the allocation privilege**: `RS_FLAGS_ALLOC_PRIVILEGED` is
consumed only by `_serverAllocValidatePrivilege`
(`src/nvidia/src/kernel/rmapi/alloc_free.c:611-675`), which is on the **alloc** path.
`grep -n "PRIVILEGED\|ValidatePrivilege" src/nvidia/src/libraries/resserv/src/rs_server.c` returns
**one hit, `:1073`**, an unrelated internal `secInfo.privLevel = RS_PRIV_LEVEL_KERNEL`. ⇒ **the copy
path performs no privilege validation at all.**

And the channel's USERD resolution takes the **allocating** client's handle —
`kchannelCreateUserdMemDesc_GV100`, `src/nvidia/src/kernel/gpu/fifo/arch/volta/kernel_channel_gv100.c:183-190`:

```c
    if (serverutilGetResourceRefWithType(hClient, hUserdMemory, classId(Memory),
                                         &pUserdMemoryRef) != NV_OK)
        return NV_ERR_OBJECT_NOT_FOUND;
```

`classId(Memory)` matches derived classes, and `MemoryList` is one
(`resource_list.h:630-640`, internal class `MemoryList`). Reached from
`kernel_channel.c:2299-2308` via `kchannelCreateUserdMemDescBc_GV100`
(`kernel_channel_gv100.c:69-131`).

⇒ ★★★★★ **THE COUNTER-CASE EXISTS, and it inverts the design question:**

> **S (privileged) mints the `NV01_MEMORY_LIST_OBJECT` page-slice of the store → S shares it for
> `RS_ACCESS_DUP_OBJECT` → I dups it into its own client → I births the channel naming its *own*
> duped handle.**
>
> Creator == driver is **preserved**. The privileged mint stays in S. Nothing carries S's
> `ProcessID`. Every gate in §4.1 stands unmodified.

**The one gap this has to close** is the share, and there is a **shipped, measured precedent for
exactly it** in the Mode-1 sibling (§5.4): `NV_ESC_RM_SHARE` (`0xc0184635`) with
`.accessMask = RS_ACCESS_DUP_OBJECT`, `.type = RS_SHARE_TYPE_ALL`, **issued by the owner on the
owner's own fd** — `/workspace/nvkvm-pv/src/qemu/nvkvm_isolate_handlers.c:4150-4219`, reasoning at
`:4089-4108`, rationale for why `TYPE_ALL` is not a cross-tenant hole at `:4177-4200`.

⚠ **What is NOT established, and must be measured before anyone relies on this:**
1. Whether a duped `MemoryList` satisfies `kchannelCreateUserdMemDesc`'s *later* checks — the VPR
   flag test (`kernel_channel_gv100.c:199-203`), `kchannelIsUserdAddrSizeValid_HAL` (`:211`), and
   the page-size override (`:226-229`). **Source says nothing against it; nothing says it works.**
2. Whether the same trick covers everything a per-proc isolate needs from the store, or only
   USERD. ⊘ Under Constraint 26 the isolate is *supposed* to hold no vidmem `hMemory`
   (`docs/design/THE_CONSTRAINTS.md:264`), so **this counter-case is itself a constraint change**
   — a smaller and differently-shaped one than scratchpad birth, but not free.
3. Whether `RS_SHARE_TYPE_ALL` is acceptable posture here. It is host-wide, not scoped to I.
   `RS_SHARE_TYPE_CLIENT` (`client_resource.c:5161-5165`) is the narrower verb and is the obvious
   thing to try first; **no evidence was found either way about whether it works for this case.**

### 6.2 ★★ COUNTER-CASE B — `SubProcessID` is settable by any unprivileged client

If the owner prefers scratchpad birth anyway, §1's lost discrimination is **partly recoverable
without extra processes**.

`cliresCtrlCmdSetSubProcessID_IMPL` — `src/nvidia/src/kernel/rmapi/client_resource.c:4754-4770`:

```c
    pClient->SubProcessID = pParams->subProcessID;
    portStringCopy(pClient->SubProcessName, ..., pParams->subProcessName, ...);
    return NV_OK;
```

**No privilege check, no rights check, no validation of the value.** The export entry
(`src/nvidia/generated/g_client_resource_nvoc.c:1316-1330`) is `methodId = 0x901`,
`accessRight = 0x0u`, `flags = 0x10109u` — and `0x10109 & RMCTRL_FLAGS_PRIVILEGED (0x4) == 0` while
`0x10109 & RMCTRL_FLAGS_NON_PRIVILEGED (0x8) == 0x8`
(`src/nvidia/inc/kernel/rmapi/control.h:202`, `:208`). The command id is
`NV0000_CTRL_CMD_SET_SUB_PROCESS_ID = 0x901`
(`src/common/sdk/nvidia/inc/ctrl/ctrl0000/ctrl0000proc.h:93`).

⇒ **An unprivileged scratchpad holding N root clients — one per guest process, each with a distinct
`SubProcessID` — would birth channels that carry distinct `SubProcessID`s** (`kernel_channel.c:294`),
restoring the `FIFO_ISOLATIONID` discrimination of §1.2(b) (`kernel_fifo.c:504-511`, `:732`, `:768`)
and the `serverutilGetClientHandlesFromPid` / `GET_PIDS` discrimination (`rs_utils.c:310`,
`gpu_rmapi.c:1006`) — **without N processes and without N privilege grants.**

⚠ ⊘ **Three warnings, all from source:**
1. It does **not** restore `ProcessID`, so the HWPM profiler gate (`kern_profiler_v2.c:656`) and
   FECS/video/RC attribution (§1.3 items 2–5) stay collapsed onto S.
2. Setting a non-zero `SubProcessID` **changes the isolation domain**:
   `kernel_fifo.c:745-763` takes `if (0x0 != subProcessID)` → `GUEST_KERNEL` or `GUEST_USER` instead
   of `HOST_USER`. **Unmeasured what else keys on `domain`.**
3. `cliresCtrlCmdDisableSubProcessUserdIsolation_IMPL` (`client_resource.c:4777-4790`) sets
   `pClient->bIsSubProcessDisabled`, which `kernel_fifo.c:771-777` turns into
   `domain = GUEST_INSECURE`, `subProcessID = KERNEL_PID`. **An unprivileged client can therefore
   switch its own USERD isolation off.** That is worth knowing regardless of this decision.

### 6.3 ⊘⊘⊘ COUNTER-CASE C — **REFUTED, an hour after writing it. The escape layer overwrites
the class.**

> ⊘⊘⊘ **CORRECTION, folded in above the text it corrects (doc-hygiene rule).** Everything below
> about `bIsRootNonPriv` is **true of RM's core and UNREACHABLE from Linux userspace.**
> `src/nvidia/arch/nvalloc/unix/src/escape.c:394-403`, in the `NV_ESC_RM_ALLOC` handler:
>
> ```c
>             switch (pApi->hClass)
>             {
>                 case NV01_ROOT:
>                 case NV01_ROOT_CLIENT:
>                 case NV01_ROOT_NON_PRIV:
>                 {
>                     NV_CTL_DEVICE_ONLY(nv);
>                     // Force userspace client allocations to be the _CLIENT class.
>                     pApi->hClass = NV01_ROOT_CLIENT;
>                     break;
>                 }
> ```
>
> ⇒ **`NV01_ROOT_NON_PRIV` is rewritten to `NV01_ROOT_CLIENT` before RM ever sees it, so
> `bIsRootNonPriv` can never be true for a userspace client on Linux** (`client.c:88` compares
> the already-rewritten class). Counter-case C **does not exist**. ⚠ I found the mechanism, wrote
> it up, and only then read the escape layer — the failure mode this campaign names as *"a
> mechanism that is real in the core and unreachable at the boundary."*
>
> ### ★★★★★ AND THE REFUTATION SHARPENS THE REAL CONSTRAINT — this is the load-bearing half
>
> With `bIsRootNonPriv` permanently false, `rmclientIsAdmin` reduces to
> **`privLevel >= RS_PRIV_LEVEL_USER_ROOT`** (`client.c:393`), and `privLevel` is computed
> **per ioctl** from `capable(CAP_SYS_ADMIN)` (`escape.c:304` → `os.c:614-617` →
> `os-interface.c:377-381` → `nv-linux.h:537`). `kchannelConstruct` reads the **call's**
> privLevel, not the client's cached one — `kernel_channel.c:277`:
> `RS_PRIV_LEVEL privLevel = pCallContext->secInfo.privLevel;`
>
> ⇒ ★★★★★ **A scratchpad holding `CAP_SYS_ADMIN` at the moment it births a channel produces
> `_PRIVILEGE_ADMIN`, unavoidably, with no knob to prevent it.** And it must hold `CAP_SYS_ADMIN`
> to mint `NV01_MEMORY_LIST_OBJECT` (`alloc_free.c:650-660`). The two requirements are the same
> capability, evaluated at two different ioctls.
>
> ⇒ **Constraint 30's demand — *"scratchpad-born channels come out `_PRIVILEGE_USER`"* — is
> satisfiable only by SEQUENCING:** mint every slice while privileged, drop `CAP_SYS_ADMIN`, then
> birth. ⊘ **And that collides with the design:** guest channels are born lazily, throughout the
> VM's life, as guest processes appear (`crates/kayfabe-core/src/gpu.rs:3363-3369`,
> `:5479`), each wanting a fresh USERD slice. A VM-lifetime scratchpad cannot both mint on demand
> and be unprivileged at birth **in one process**.
>
> ⊘ **And this corrects Constraint 30's own reasoning.** `docs/design/THE_CONSTRAINTS.md:414-415`
> attributes the risk to euid: *"F11 records that our isolates' kernel-visible euid is **0** on a
> root VMM, so `rmclientIsAdmin(...)` plausibly holds."* **The euid is the wrong quantity.**
> `rmclientIsAdmin` keys on `capable(CAP_SYS_ADMIN)`, which `surrender_privilege` *does* drop; the
> euid governs a **different** check (the UID/security token,
> `client.c:172-190`, `osValidateClientTokens`), which is what F11 is actually about
> (`crates/kayfabe-isolate-host/tests/own_client_invariant.rs:1-14`). **The conclusion stands and
> the mechanism named for it does not** — and the corrected mechanism is *stronger*, because it
> binds to the very capability the scratchpad exists to hold.
>
> ★★★ **Which is the strongest argument in this document for COUNTER-CASE A (§6.1): in A the
> ISOLATE births, and the isolate is never privileged, so the whole problem is absent by
> construction rather than managed by sequencing.**

---

*(The refuted analysis is retained below, because the reading of RM's core is correct and the
reader should be able to check the refutation against it.)*

### 6.3-orig ⊘ REFUTED — `NV01_ROOT_NON_PRIV` in RM's core

This does not let I birth while naming S's memory, but it **repairs the brief's premise** (§4.3) and
is the cheapest thing on this list.

`rmclientIsAdmin_IMPL` — `src/nvidia/src/kernel/rmapi/client.c:384-394`:

```c
    return (privLevel >= RS_PRIV_LEVEL_USER_ROOT) && !pClient->bIsRootNonPriv;
```

and `bIsRootNonPriv` is set purely by the **root class chosen at client allocation** —
`src/nvidia/src/kernel/rmapi/client.c:88`:

```c
    pClient->bIsRootNonPriv  = (pParams->externalClassId == NV01_ROOT_NON_PRIV);
```

`NV01_ROOT_NON_PRIV` is `0x1` (`src/common/sdk/nvidia/inc/class/cl0001.h:31`), a real allocatable
root class (`src/nvidia/src/kernel/rmapi/resource_list.h:74-83`, same flags as `NV01_ROOT` at
`:62-71` minus `RS_FLAGS_ALLOC_GSP_PLUGIN_FOR_VGPU_GSP`, `RS_ACCESS_NONE`).

⇒ ★ **`privLevel` (what `_serverAllocValidatePrivilege` checks for `RS_FLAGS_ALLOC_PRIVILEGED`,
`alloc_free.c:650-660`) and `rmclientIsAdmin` (what `kernel_channel.c:285` checks) are
independently controllable.** A process with `CAP_SYS_ADMIN` that allocates its root as
`NV01_ROOT_NON_PRIV`:
- still has `privLevel == RS_PRIV_LEVEL_USER_ROOT` (`escape.c:304`, per-ioctl from
  `capable(CAP_SYS_ADMIN)`), so it **can still mint `NV01_MEMORY_LIST_OBJECT`**;
- but has `rmclientIsAdmin == false`, so channels it births come out **`_PRIVILEGE_USER`**
  (`kernel_channel.c:288-291`).

⇒ **This is a structural answer to Constraint 30's demand** (*"scratchpad-born channels come out
`_PRIVILEGE_USER`, asserted at birth"*) and to F11's euid-0-on-a-root-VMM problem
(`docs/design/THE_CONSTRAINTS.md:414-415`) — it removes the dependency on the VMM not being root.

⊘ **We are not using it.** `OwnClient::allocate_root`
(`crates/kayfabe-isolate-host/src/rm.rs:297-320`) allocates `NV01_ROOT_CLIENT` (`0x41`), for which
`bIsRootNonPriv == false`. The constant exists in our ABI only as a capability-table row
(`crates/kayfabe-abi/src/capability.rs:893-896`) — never allocated.

⚠ **Not free, and unmeasured:** the complete `bIsRootNonPriv` census is four sites —
`client.c:88`, `:179` (security/UID-token caching is *enabled* for non-priv roots),
`:393` (`rmclientIsAdmin`), `:477` (a client-handle validation path that **rejects** kernel-privilege
callers unless the client is non-priv). ⊘ **Nothing was measured about what a non-priv root client
loses.** Some admin-gated control or alloc we depend on may break. Probe shape in §7.

### 6.4 ⊘ What was looked for and NOT found

Recorded so the search's bound is visible. All in `research_clones/ogkm-580.159.04/`:

- A non-privileged sub-object class of `MemoryList` that slices a parent object: **not found.**
  `NV01_MEMORY_LIST_SYSTEM`, `_FBMEM` and `_OBJECT` are all `RS_FLAGS_ALLOC_PRIVILEGED`
  (`resource_list.h:617`, `:627`, `:637`).
- A companion-client field on any channel-alloc handle: **not found** (§6 opening).
- A cross-client alloc parent: **not found** (`grep -rn "hParentClient"` → GSP RPC + NV0005 only;
  `rmapi_specific.c:74` explicitly rejects it).
- A way for userspace to supply `ProcessID`: **not found**; it is zeroed at
  `kernel_channel.c:225`.
- `RS_ACCESS_PERFMON` enforcement: **not found** — the right is defined
  (`rs_access.h:62`) but no `rsAccessCheckRights` / `RS_ACCESS_MASK_TEST` site gates on it.
- A `CanCopy` for `KernelChannel`: **not found** (§2.0).

---

## §7 — What is unmeasurable from source, and the probe for each

★ Per the brief: each item states the probe's **shape**. ⊘ **None of these has been run.** No probe
code was written for this deliverable.

### 7.1 ★★★★★ The one that costs nothing and Constraint 30 already demands

**Question:** does a scratchpad-born channel come out `_PRIVILEGE_USER` or `_PRIVILEGE_ADMIN`?

**★ It is directly readable from userspace, and we are not reading it.** RM writes its verdict back
into the caller's own alloc-params buffer — `src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:281-287`:

```c
            pKernelChannel->privilegeLevel = ..._PRIVILEGE_ADMIN;
            pChannelGpfifoParams->flags =
                FLD_SET_DRF(OS04, _FLAGS, _PRIVILEGED_CHANNEL, _TRUE, pChannelGpfifoParams->flags);
```

and the alloc params are copied out **on success** — `serverAllocApiCopyOut`,
`src/nvidia/src/kernel/rmapi/alloc_free.c:195-218` (`SKIP_COPYOUT` is set *only* when
`status != NV_OK`), via `rmapiParamsRelease`'s `portMemExCopyToUser`
(`src/nvidia/src/kernel/rmapi/param_copy.c:168-179`).

The bit is `NVOS04_FLAGS_PRIVILEGED_CHANNEL` = **`5:5`**, `_TRUE = 1`
(`src/common/sdk/nvidia/inc/alloc/alloc_channel.h:141-143`).

⇒ **Probe shape: after `alloc_channel_in`'s channel alloc returns `NV_OK`, decode bit 5 of the
returned `flags` field and refuse if set.** No new ioctl, no hardware, no bench — it is a read of
a buffer we already own. ★ This is precisely the *"asserted at birth and refusing otherwise"* that
`docs/design/THE_CONSTRAINTS.md:416-417` demands, and it is available today for the channels we
already birth. ⊘ Our encoder (`crates/kayfabe-abi/src/submit.rs:345`) writes `flags`; **nothing in
our tree reads it back** (no reader found).

⚠ **One caveat that must be checked in the same probe:** `kernel_channel.c:217-226` rewrites
several params fields on entry, and `:2814-2815` re-reads them for the GSP RPC. The copy-out
happens after all of that, so the flag should be RM's final verdict — **but that is a reading of
call order, not a measurement.** The probe settles it by construction: run it once as an
unprivileged process (expect bit 5 clear) and once with `CAP_SYS_ADMIN` (expect bit 5 set). ★ A
probe with only the first arm cannot distinguish *"we are unprivileged"* from *"the readback does
not work"* — that is this campaign's `a_census_zero_needs_a_known_positive` class.

### 7.2 Is the USERD ChID isolation actually armed on our host?

§1.2(b) found the mechanism and §1.2's two limits found that it may not be armed:
`kfifoIsPreAllocatedUserDEnabled()` (`src/nvidia/generated/g_kernel_fifo_nvoc.h:2411`) and the
`IS_GSP_CLIENT ⇒ subProcessIsolation = 0` path that then calls
`eheapSetOwnerIsolation(..., NV_FALSE, ...)` (`src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c:355-366`).

**Probe shape, and it needs no new instrument:** the work-submit token we already read back is
`runlist << 16 | chid` (`crates/kayfabe-chips/src/ga10x.rs:266`,
`crates/kayfabe-chips/tests/gb20x_doorbell_and_regs.rs:96`). The USERD isolation granularity is
`RM_PAGE_SIZE / userdBar1Size` channel IDs (`kernel_fifo.c:338-342`). ⇒ **Birth two channels from
two different RM clients in two different host processes and compare
`chid / granularity`.** Same group ⇒ isolation is **off**; different groups ⇒ **on**.
Then repeat with both channels from one client: if they land in the same group under arm 1's
*different*-group result, the discrimination is confirmed to be the client identity.
⚠ `userdBar1Size` comes from `kfifoGetUserdSizeAlign_HAL`; **derive it, do not assume 4 KiB / 512 B**.

### 7.3 Does a duped `MemoryList` work as `hUserdMemory[0]`? (counter-case A)

**The decisive experiment for §6.1, and it is a userspace-only, two-process test on one GPU.**

Shape:
1. Process S, `CAP_SYS_ADMIN`: allocate the store, mint an `NV01_MEMORY_LIST_OBJECT` page-slice.
2. S: `NV0000_CTRL_CMD_CLIENT_SHARE_OBJECT` (`0xd06`) — try `RS_SHARE_TYPE_CLIENT` **first**
   (narrower; `src/nvidia/src/kernel/rmapi/client_resource.c:5161-5165`), fall back to the
   sibling's measured `NV_ESC_RM_SHARE` / `RS_SHARE_TYPE_ALL`
   (`/workspace/nvkvm-pv/src/qemu/nvkvm_isolate_handlers.c:4150-4219`) only if the narrow one fails.
3. Process I, **unprivileged**: `NV_ESC_RM_DUP_OBJECT` the slice into its own client.
4. I: birth an `AMPERE_CHANNEL_GPFIFO_A` naming its own duped handle in `hUserdMemory[0]`.
5. Assert: alloc returns `NV_OK`, **and** bit 5 of the returned `flags` is **clear** (§7.1).

Three ways it can fail, each informative and each already located in source:
- the share is refused ⇒ §6.1's gap is real and `TYPE_ALL` is the only route;
- the dup is refused ⇒ `memlistCanCopy` is true but something else gates it
  (`clientCopyResource_IMPL`, `src/nvidia/src/libraries/resserv/src/rs_client.c:543-551`);
- the birth is refused inside `kchannelCreateUserdMemDesc_GV100` ⇒ the VPR flag test
  (`src/nvidia/src/kernel/gpu/fifo/arch/volta/kernel_channel_gv100.c:199-203`),
  `kchannelIsUserdAddrSizeValid_HAL` (`:211`), or the page-size override (`:226-229`).

### 7.4 What does GSP do with the `ProcessID` it is handed?

`kernel_channel.c:2814-2815` forwards `pKernelChannel->ProcessID`/`SubProcessID` to GSP verbatim.
**GSP firmware is a signed blob; this is unmeasurable from any source we have.**
⊘ Probe shape: none that is sound. The closest is a **differential** — birth two channels
identically except for the creating client's `SubProcessID` and diff every observable
(`GET_PIDS`, accounting, FECS event stream, RC text, `nvidia-smi`). ⚠ A null result proves
nothing; it would only bound what GSP's use of the field is *visible as*.

### 7.5 Is there any check on *who* rings a doorbell?

§3.4 argues there is none because there is no software in the path.
⊘ **This is INFERRED**, and the grep that would have refuted it came back near-empty:
`grep -n "NV_VIRTUAL_FUNCTION_DOORBELL\|WORK_SUBMIT\|ringDoorbell"
src/nvidia/src/kernel/gpu/usermode_api.c` finds only the comment at `:85`. The usermode classes
are `RS_FLAGS_ALLOC_NON_PRIVILEGED`, parented to `Subdevice`, `RS_ACCESS_NONE`
(`src/nvidia/src/kernel/rmapi/resource_list.h:853`, `:864`, `:875`, `:886`, `:896`).
**Probe shape:** process A births a channel and reports its token; process B, which never touched
that channel, opens its own `AMPERE_USERMODE_A` under its own subdevice and stores A's token.
Assert A's semaphore releases. ⚠ A **positive** result is a cross-tenant submission primitive and
should be recorded as a finding in its own right, independent of this decision.

### 7.6 What does a non-privileged root client lose? — **⊘ MOOT.** Refuted by §6.3's correction:
`NV01_ROOT_NON_PRIV` is unreachable from Linux userspace (`escape.c:394-403`), so there is nothing
to probe.

---

## Summary — what breaks, what restates, what is untouched

★ Scope: this table is about **S births the guest channel and the per-proc isolate drives it**.

| | item | verdict | evidence |
|---|---|---|---|
| **BREAKS** | The isolate can never **name** the channel | dup is refused unconditionally — no rights, no policy, no privilege can fix it | `rs_server.c:1719-1723`; `resCanCopy_IMPL` `rs_resource.c:333-340`; no `kchannelCanCopy` (empty grep) |
| **BREAKS** | The isolate can never allocate compute/CE objects on it | alloc parent must be in the caller's own map; no cross-client parent exists | `alloc_free.c:739-743`; `rs_client.c:381-392`; `rmapi_specific.c:74` |
| **BREAKS** | The isolate can never `BIND`, `SCHEDULE` or free it | controls and free dispatch on a ref in the invoking client | `control.c:751-752`; `rs_server.c:1125-1173` |
| **BREAKS** | ⇒ **S owns the channel for its whole life**; the isolate's role collapses to a doorbell store | structural consequence of the three rows above | — |
| **BREAKS** | `_PRIVILEGE_USER` is **not** achievable while S holds `CAP_SYS_ADMIN` | `rmclientIsAdmin` reduces to `privLevel >= USER_ROOT`; `bIsRootNonPriv` unreachable; privLevel is per-ioctl | `client.c:393`; `escape.c:394-403`, `:304`; `kernel_channel.c:277` |
| **BREAKS** | HWPM profiler context permission | `pClient->ProcID == pChannel->ProcessID` now fails for the isolate and passes for S on **every** channel in the VM | `kern_profiler_v2.c:656` |
| **BREAKS** | Per-guest-process attribution in FECS / video-log / RC breadcrumbs | all report S | `fecs_event_list.c:311-363`; `videoeventlist.c:81`; `kernel_rc.c:341` |
| **BREAKS (conditionally)** | USERD-page co-tenancy across guest processes | one `FIFO_ISOLATIONID` for the whole VM — **but the protection may never be armed on a GSP host** | `kernel_fifo.c:504-511`, `:729-777`; limits at `:338-342`, `:355-366` |
| **RESTATES (C29)** | `Worker::execute` / `belongs_to` / `ChannelBirth::handles()` | the failure (raw handles alias across clients) is untouched; the gate must key on the verb's isolate, and must go red if a *driving* verb carries an S handle | `crates/kayfabe-isolate/src/lib.rs:3862-3868`, `:184-193`, `:3095-3103`, `:113-127` |
| **RESTATES (C29)** | ★ `StoreMapPort::is_slice_of_the_store` | under S-birth the ring **is** S's own slice, so the oracle's question becomes trivially yes and **stops discriminating**. Most likely restate to be missed. | `crates/kayfabe-qemu-raw/src/storemap.rs:309-331`; `crates/kayfabe-fwd/src/lib.rs:6863-6886` |
| **RESTATES (C29)** | `SCRATCHPAD_BIRTH_IN_A_HANDED_SPACE` | the gate being crossed. Successor must read the privilege stamp back live (§7.1), not argue it. | `crates/kayfabe-isolate-host/src/rm.rs:1345`, `:7794-7806`; `THE_CONSTRAINTS.md:415-417` |
| **RESTATES** | routing / worker checkout / commit | mechanical: birth routes to S, driving stays per-proc; no failure guarded | `crates/kayfabe-fwd/src/lib.rs:1588-1600`, `:5485-5493`, `:5612-5614` |
| **RESTATES (C26 change)** | Constraint 26's "an isolate never holds a vidmem `hMemory`" | only under counter-case A; S-birth itself does not touch it | `THE_CONSTRAINTS.md:264` |
| **UNTOUCHED** | ★★★ **CPU mappings** | the channel object is unmappable for everyone on our shape; the guest-backed birth path maps nothing; the doorbell uses the isolate's own usermode object; and an armed node crosses processes anyway (measured) | `kernel_channel.c:1291`; `crates/kayfabe-isolate-host/src/rm.rs:8212-8253`, `:1992-2001`; `nv-mmap.c:506-531`; `the_counter_page_and_the_device_view.md:68-84` |
| **UNTOUCHED** | Ringing the doorbell | needs the isolate's own usermode window + a `u32`; no channel handle | `crates/kayfabe-isolate-host/src/rm.rs:1992-2001`, `:2041-2075` |
| **UNTOUCHED** | `OwnClient` / F11 | the isolate still names only clients it minted — **because it never names the channel at all** | `crates/kayfabe-isolate-host/src/rm.rs:273`, `:292-320` |
| **UNTOUCHED** | `ScratchpadRole` / `HandedVaSpace` / `ADOPT_NOT_THE_SCRATCHPAD` | unaffected | `crates/kayfabe-isolate-host/src/rm.rs:337-453`, `:1281` |
| **UNTOUCHED** | `RING_HANDLE_REACHED_RM` | still the right assert, still fail-closed | `crates/kayfabe-isolate-host/src/rm.rs:1315`, `:8094-8105` |
| **UNTOUCHED** | `unranked_locks` | ⊘ unrelated to ownership; reported empty | `tests/tests/unranked_locks.rs:1-30` |
| **UNTOUCHED** | Per-guest-process discrimination that keys on the **calling** client | GPU/FB accounting, `GET_PIDS`, `GET_CLIENT_INFO`, NVENC sessions, `RS_SHARE_TYPE_PID` — all read `pClient->ProcID` of the current call, still the isolate | `device.c:431-493`; `video_mem.c:1032-1044`; `gpu_rmapi.c:1006`; `mem_mgr_ctrl.c:557`; `nvencsession.c:147`; `client_resource.c:217-231` |

---

## Open questions for the owner

1. ★★★★★ **§6.1 — counter-case A changes the shape of the decision.** `MemoryList` is dupable
   (`mem_list.c:787-794`), the copy path does no privilege check, and the channel resolves
   `hUserdMemory` in the **allocating** client (`kernel_channel_gv100.c:183-190`). ⇒ S can mint and
   share the slice while **I births the channel**, preserving creator == driver and every gate in
   §4.1. **Is that in scope, or has it already been ruled out for a reason not recorded in the
   tree?** The falsifier is §7.3 and it is a userspace-only two-process test.
2. ★★★★★ **Counter-case A costs a Constraint 26 amendment** — the isolate would hold a vidmem
   `hMemory` (`THE_CONSTRAINTS.md:264` says it never does), **and** it puts a `LIST_OBJECT` dup
   inside Constraint 31's teardown barrier (`THE_CONSTRAINTS.md:446-458` — *"Constraint 27 extends
   from MAPPINGS to HANDLES"*; a dup is a second handle with its own lifetime). Which amendment is
   preferable: 26, or 30?
3. ★★★★ **If S births anyway: is "S owns the channel for its whole life" acceptable?** §2 shows
   this is not a rights problem — the channel is simply not shareable. Every engine-object alloc,
   `BIND`, `GPFIFO_SCHEDULE` and free moves into S. ⚠ That makes S a **serialization point for
   every guest process's channel control plane**, which interacts with the three blocking
   invariants (`memory/the_three_blocking_invariants.md`).
4. ★★★ **The privilege stamp is a SEQUENCING problem, not a knob** (§6.3 correction). S must hold
   `CAP_SYS_ADMIN` to mint and must not hold it to birth `_PRIVILEGE_USER`, and both happen
   throughout the VM's life. Options visible from here: (a) a privileged minter + an unprivileged
   birther as two processes — which is counter-case A with extra steps; (b) pre-mint a pool of
   slices, drop, then birth from the pool; (c) accept `_PRIVILEGE_ADMIN` and bound its
   consequences. ⚠ For (c), the concrete consequence found is
   `kgrctxGetRegisterAccessMapId_IMPL` (`kernel_graphics_context.c:3285-3299`): an admin channel is
   given `GR_GLOBALCTX_BUFFER_UNRESTRICTED_PRIV_ACCESS_MAP` instead of the restricted one — **and
   the pushbuffer on a guest channel is the guest's.** ⊘ Not measured; it is a reading.
5. ★★★ **Constraint 30's assert is available today and free** (§7.1): bit 5 of the alloc params'
   `flags`, copied back on every successful channel alloc. **Should it be added to the channels we
   already birth, regardless of this decision?** It would also be the known-positive the eventual
   S-birth gate needs.
6. ★★ **§1's answer may be "this costs almost nothing"** — the `ProcessID` stamp is one enforcement
   gate (HWPM, which we likely never reach), one isolation key that may not be armed on a GSP host
   (§7.2), and otherwise telemetry. ⊘ **But "may not be armed" is unmeasured**, and if it *is*
   armed, S-birth puts every guest process's USERD in one page group. Is §7.2 worth running before
   the ruling?
7. ★★ **Is the RC/Xid attribution loss acceptable?** Under S-birth, a host operator cannot tell
   which guest process faulted from `dmesg` (`kernel_rc.c:341`). ⊘ Recoverable via §6.2's
   `SubProcessID` trick only if the tooling reads subpid, which is **not established**.
8. ★ **§7.5 is a finding waiting to happen, independent of this decision.** If any process can ring
   any channel's doorbell given only a `u32` token, that is a cross-tenant submission primitive on
   the host and should be known either way.
