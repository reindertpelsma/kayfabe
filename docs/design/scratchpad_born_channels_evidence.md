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
