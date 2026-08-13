# The error notifier over the GUEST'S OWN PAGES — all three gates answered, and the route exists

**STATUS: LIVE — 2026-08-13 (w288n).** Source + tree analysis, no boot yet. Answers the owner's
three verify-before-building gates, **all three favourably**, and locates every piece of the
build. Supersedes the recommendation of `w288_error_delivery_gate.md` (a correction block is
folded into that file's head).

---

## 0. THE DESIGN, IN ONE PARAGRAPH

The guest's error notifier is **SYSMEM, in guest RAM**, and its guest-physical address arrives in
the channel-alloc RPC as `errorNotifierMem` (measured complete **63/63**). Build a host RM
`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` **over those very pages** and pass it as `hObjectError` on the
host channel. The host RM/GSP then writes the guest's actual notifier memory when the channel is
robust-channel killed. The guest's `uvm_channel_get_status` poll reads what the host wrote.
**We are not a writer, not a translator and not an intermediary.**

---

## 1. ★★★ GATE 1 — *does the host accept our object as `hObjectError`, and does the WRITE require `NV01_CONTEXT_DMA`?*

### ANSWERED: it accepts, and **NO ctxdma is required.** Two independent sources.

**(a) By measurement, on real GA106.** `2a9d2e1` (`origin/w287-raw-clients`, `580.159.04`,
`traces/real_ga106/w287_raw_clients_real_ga106.txt`) wired `hObjectError` to an
`NV01_MEMORY_LOCAL_USER` — **a `Memory` handle, not a context DMA** — and the notifier **fired**:

```
status 0xffff, info32 0x0000001f, info16 0x0001, timestamp 0x72b7e7a018cb5d5f
```

with the negative control **on the same sixteen bytes in the same run** reading
`status 0x0000 info32 0x00000000` after the positive control retired and before the fault was
issued. ⊘ **That commit was not an ancestor of the w288 branch**, which is why the gate doc
recorded this as unchecked; the merge is this doc's parent commit.

**(b) By source, with the HAL rule applied.** `kchannelGetNotifierInfo`
(`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2016-2088`):

```c
if (memGetByHandle(pRsClient, hErrorContext, &pMemory) == NV_OK) {
    if (memdescGetAddressSpace(pMemory->pMemDesc) == ADDR_VIRTUAL) { ... }   // GPUVA branch
    else {
        *ppMemDesc     = pMemory->pMemDesc;
        *pNotifierType = ERROR_NOTIFIER_TYPE_MEMORY;                        // :2074-2078
    }
    return NV_OK;
}
if (ctxdmaGetByHandle(...) == NV_OK) { ... ERROR_NOTIFIER_TYPE_CTXDMA ... }
```

A SYSMEM `Memory` object takes the `else` and is accepted as `ERROR_NOTIFIER_TYPE_MEMORY`.
**No CPU mapping is required, no `KernelVAddr` is checked, no ctxdma is involved.**

### ⊘⊘ AND THE BRIEF'S "SETTLED" CITATION IS MIS-SCOPED — READ AS STATED IT KILLS THE DESIGN

> *"The CPU writes it, never hardware — `kchannelGetNotifierInfo` resolves the GPU VA only to
> obtain a CPU kernel mapping and **fails without one** (`:2066-2071`)."*

`:2066-2071` is **strictly inside the `ADDR_VIRTUAL` branch**:

```c
if (!pDmaMappingInfo->KernelVAddr[subdeviceInstance]) {
    NV_PRINTF(LEVEL_ERROR, "Kernel VA addr mapping not present for notifier\n");
    return NV_ERR_INVALID_STATE;
}
```

It is a property of the **GPUVA** case, not of notifiers. This matters because
`[measured, R31 arm B]` **a guest-backed `OS_DESCRIPTOR` cannot be CPU-mapped** —
`NV_ERR_NOT_SUPPORTED`, the driver's own *"memMap_IMPL: CPU mapping not supported for
addressSpace: 0x1"* (`crates/kayfabe-isolate/src/lib.rs:715`). ⇒ Taken as a general requirement
the citation would make this design **impossible**; correctly scoped, **we never need a CPU
mapping of the object at all**. ⚠ *A correct citation narrowed by the reading* — running, here,
in the direction that makes a live design look dead.

★ Corroboration that the ADDR_VIRTUAL branch is the real hazard and is **already measured on our
own channels**: `0f62499` — *"the ADDR_VIRTUAL branch the owner suspected is on the ERROR
NOTIFIER, where **31/68** of our own channels take it."*

### ★★ WHAT THE HOST ACTUALLY SENDS — why the guest's page gets written

On a GSP client CPU-RM does **not** write the notifier; it RPCs the physical base to the GSP
(`kernel_channel.c:549-568`):

```c
pChannelGpfifoParams->errorNotifierMem.base =
    memdescGetPhysAddr(pKernelChannel->pErrContextMemDesc, AT_GPU, 0)
    + pKernelChannel->errorContextOffset;
pChannelGpfifoParams->errorNotifierMem.size         = ...;
pChannelGpfifoParams->errorNotifierMem.addressSpace = memdescGetAddressSpace(...);
pChannelGpfifoParams->errorNotifierMem.cacheAttrib  = memdescGetCpuCacheAttrib(...);
```

For an `OS_DESCRIPTOR` over the guest's page, that base **is the host-physical address of the
guest's own notifier page**. The GSP writes there directly.

⚠ **HAL rule, applied and passed.** `krcErrorSendEventNotificationsCtxDma` resolves
unconditionally to `_FWCLIENT` (`g_kernel_rc_nvoc.h:280`), which writes **no** notifier — and
`kern_gmmu_gv100.c:2127` reads as the fault-path writer and is **dead on a GSP client** (its
shadow fault buffer is never filled: `unix_intr.c:933-938`). ⇒ **The GSP is the writer**, which
is exactly why the physical base above is the load-bearing field, and why *we* writing it would
have made us a second author.

---

## 2. GATE 2 — *do the written fields carry anything host-scoped?*

`NvNotification` is 16 bytes: `timeStamp` (8), `info32` (4), `info16` (2), `status` (2).

| field | value | host-scoped? |
|---|---|---|
| `status` | `0xffff` = `NOTIFIER_STATUS_RC`, a literal (`kernel_rc_notification.c:335`) | **no** — a constant |
| `info32` | `exceptType`, a `ROBUST_CHANNEL_*` code (`31` = `..._MMU_ERR_FLT`, `nverror.h:49`) | **no** — hardware-derived, and it is the number a log prints as `Xid` |
| `info16` | `(NvU16)gpuGetNv2080EngineType(localRmEngineType)` (`kernel_rc_notification.c:172`) | ⚠ **engine TYPE, see below** |
| `timeStamp` | host GPU PTIMER | **no** — a timestamp is not an identifier |

⇒ **No structure here carries a guest-side address, handle or ChID.** Nothing that could leak a
host pointer crosses.

⚠ **The one residual risk is `info16` for COPY ENGINES, and it is worth naming.**
`NV2080_ENGINE_TYPE_GRAPHICS` is `1` on both sides, so for the GR fault this rung is aimed at
(`Xid 31 ENGINE GRAPHICS HUBCLIENT_FE FAULT_PDE`) there is nothing to diverge. But copy engines
are `NV2080_ENGINE_TYPE_COPY0..n` — an **instance**, and host CE instance numbering need not
equal the guest's. A CE fault could therefore deliver a correct *"an RC happened"* with an
engine number naming a different CE than the guest's. ⇒ **Not a blocker for the hypothesis under
test, and it must be measured before the CE path is trusted.**

⊘ **Disambiguation, because the names do not warn you:** on the `NvNotification` `info16` is the
engine type; on the `NvUnixEvent` delivered by `NV_ESC_RM_GET_EVENT_DATA` the same-named field is
`partitionAttributionId` and `rmEngineType` is explicitly `// unused`
(`kernel_rc_notification.c:443`). Same RC event, same field name, different meaning.

---

## 3. GATE 3 — *can we `OS_DESCRIPTOR` the guest's notifier pages?*

### ANSWERED: **yes, and the route is production-wired end to end.**

⊘⊘ **This retires the standing note** *"guest RAM has never crossed into the isolate — there is
no route at all"* (`passthrough_scoping_findings`, 2026-08-10). That was true then. It is false
now, and the memory index still carries the stale form.

| piece | where | state |
|---|---|---|
| guest RAM reaches the isolate | fd 6 = a **dup of QEMU's own guest-RAM memfd**, granted at spawn; adopted at `kayfabe-isolate-host/src/child.rs:187-196` into `GuestRamPlane` (`guestram.rs:61`) | **live**, armed on the bench by `KAYFABE_GUEST_RAM=memfd` |
| ask for a slice | `Request::MapGuestRam` (`proto.rs:260`) → `GuestRamPlane::honour` (`guestram.rs:125`) | **live wire verb** |
| build the RM object | `Request::DescribeGuestRam` (`proto.rs:286`) → `HostRmBackend::describe_guest_ram` (`rm.rs:4723`) → `alloc_os_descriptor` | **live wire verb** — *"the one call that makes the host GPU able to reach the guest's own pages"* |
| the guest's notifier GPA | `Channel::error_notifier` = `ErrorNotifier::Sysmem { gpa }` (`kayfabe-core/src/gpu.rs:538`), written at `gpu.rs:3161` / `:3180` | **populated in production**, zero readers |

★ Precedent: the guest's **ring** already gets exactly this treatment — *"a joined leaf is an
`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over a host memfd, mapped FIXED at the guest's own VA ⇒ real
host memory"* (`c78ecaf`), and hardware proved it reachable by fetching a GPFIFO entry from it
and advancing `GP_GET 0→1`.

⚠ **Straddling.** `FaultEmission::deliver` splits its write in two for a reason. The grant here
must cover the notifier's full extent; `errorNotifierMem` carries `base` **and** `size`, and the
`OS_DESCRIPTOR` is built at `HostOffset::ZERO` for `mapped.len`, so the grant must be computed
from both and must not assume the record sits within one page.

⚠ **The pages become RM-pinned and stay pinned until the handle is freed** — `describe_guest_ram`'s
own warning. That is a lifetime coupling to guest channel teardown, not a blocker.

---

## 4. WHAT REMAINS TO BUILD — the whole list, nothing hidden

`h_object_error` is **hard-coded to `0`** on both host objects today:
`NvChannelGroupAllocationParameters` at `rm.rs:5095` (the TSG) and `ChannelAllocParams` at
`rm.rs:5136` (the channel). Its own ABI doc says *"Zero is legal and means 'do not report channel
errors through a notifier'"* (`kayfabe-abi/src/submit.rs:278-281`). **Nothing in the isolate can
give a host channel a notifier today.**

1. **VMM → wire.** Carry the guest channel's `ErrorNotifier::Sysmem { gpa, size }` into
   `Request::AllocChannel` (`proto.rs:81`) as an optional grant. Wire change: `encode`/`decode`
   both sides.
2. **Isolate.** On that field: `map_guest_ram` → `describe_guest_ram` → pass the resulting
   `HostHandle` as `h_object_error` in `ChannelAllocParams` (`rm.rs:5136`). The merged
   `alloc_channel_at_with_error_notifier` (`rm.rs`, from `2a9d2e1`) already threads a notifier
   handle through `alloc_channel_in`'s fourth parameter — **wire it, do not rewrite it.**
3. **★ The TSG gate, by name.** `alloc_channel_in` mints a fresh `CHANNEL_GROUP` per channel
   (`rm.rs:5119`), so every host channel is alone in its TSG and `RC_NOTIFIER_SCOPE_TSG` collapses
   to channel scope. **Assert it** — refused by name — rather than commenting it.
4. **Instrumentation.** A census line stating the object was accepted and typed
   `ERROR_NOTIFIER_TYPE_MEMORY`, and the notifier's own bytes read back. ⊘ `read_error_notifier`
   **cannot** be used on a guest-backed descriptor (R31 arm B: no CPU map) — the readback has to
   come from the **guest side**, which is also where the pass criterion lives.

⚠ **`failed=0` is not "nothing refused"**: RM reports status inside the parameter struct while
`ioctl(2)` returns 0. Any gate here must read the struct.

---

## 5. ⊘ WHAT THIS DESIGN DOES *NOT* NEED — and one thing it quietly retires

- ⊘ No RC-event consumer, no `poll()` waiter, no new isolate lifecycle.
- ⊘ No isolate→VMM push channel. **Which matters**: there is none, and building one is real work
  — the parent has no descriptor it polls for the child, and the worker sockets are strictly
  request/response (`isolate.rs:360-376`). The old design needed exactly that.
- ⊘ No attribution machinery, no `GET_MMU_FAULT_INFO`.
- ⊘ `FaultEmission` stays orphaned.

### ★★★★★ AND IT SIDESTEPS A CONTRADICTION THE OLD DESIGN COULD NOT HAVE SURVIVED

`kayfabe_core::fault::verdict` (`crates/kayfabe-core/src/fault.rs:277`) **escalates rather than
emits** whenever `facts.proc == system_proc` — `NotAttributable::GuestKernelContext` — and its
own documentation says the rule *"must never be relaxed into a heuristic"*, its second reason
being precisely that a `bUvmOwned` channel's RC calls `sysSetRecoveryRebootRequired`.

⊘⊘ **The hypothesis under test is about UVM channels** — `uvm_channel_get_status` is UVM's only
error exit. So the orphaned emitter, wired as designed, would have **refused exactly the
delivery the hypothesis needs**, and the rung would have produced a `CUP2_RC` that could not
distinguish *"the hypothesis is wrong"* from *"our own policy declined"*.

★ The two are not in conflict once the producer is named: `verdict`'s module docs open with
*"A bad guest pointer, made into a fault the guest's own driver handles"* — it adjudicates
**faults WE INVENT** from `kayfabe_mmu::AddressFault::Miss`. This rung's fault is one **real
hardware raised**, on a VA `w277` measured as **BOUND in our own table**
(`TABLE-DESCRIBES`, `contradicting=0`). Different producer, different question — and under the
new design the question does not arise at all, because we never adjudicate: **the host RM does
what it would do natively, into the page the guest itself nominated.**
