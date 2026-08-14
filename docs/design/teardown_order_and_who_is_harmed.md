# Teardown order, and who is harmed — is stop-before-unmap guaranteed by a compliant guest?

> **STATUS: LIVE — 2026-08-14 (w307).** Read-only adjudication. ⊘ **Nothing was built, booted or
> measured on hardware for this document.** Every claim is read off `ogkm-580.159.04`, kayfabe
> source at master `74200b2b`, or a committed measurement made by somebody else — and each claim
> says which. Where a claim needs a boot it says so and is **not** asserted.
>
> Supersede in place. Parents:
> - `docs/design/cancellation_plane.md` §4.1 / §6 finding #1 — **this document reclassifies that
>   finding and re-ranks it.** §4.1's mechanism ("the guest's belief is unfounded") stands; its
>   implied *harm* (an MMU fault) does not survive §6 below.
> - `docs/audits/w301_cancellation_error_leaks.md` §3.2, §3.3 — the leak census this ranks against.
> - `docs/design/guest_blast_radius.md` §2, §5.2 — BRICK/WEDGE and the measured fault containment.
> - `docs/design/resume_from_fault.md` §S5 — the escalation this checks the boundary against.

---

## 0. The answer, in six lines

1. **The guest does not guarantee the order — but not for the reason the brief expects.** UVM
   *blocks* on the stop: `pRmApi->Control` → `NV_RM_RPC_CONTROL` → `_issueRpcAndWait` is
   **synchronous**. What it never does is *check the answer*. ⇒ **the window is real and it is
   ours**; what is decorative is the **status**, not the **call**. §1.
2. **RM has no backstop.** Neither channel FREE, nor RC recovery, nor GR-context teardown idles an
   engine or unmaps anything for a UVM-owned VA space. `nvUvmInterfaceStopChannel` is the **only**
   serialization in the entire sequence — and its whole body is one RPC. §2, §3, §4.
3. ⇒ ★★★★★ **The owner's premise fails, and it fails structurally.** *Every* route by which a
   compliant guest could obtain "stop before unmap" — the explicit `STOP_CHANNEL`, and the
   implicit *free-implies-preempt* that RM asserts in its own comment — terminates in a single
   synchronous RPC to the GSP. **We are the GSP.** There is no by-construction path that does not
   run through our answer. §5.
4. ★★★★★ **And yet the harm the brief models cannot occur in this port.** The guest's unmap
   reaches us **only** as page-table writes, and `apply_settlement` **refuses** the unbind of any
   host-published row *by name* (`UnbindsPublished`). ⇒ the translation the engine would have
   faulted on is precisely the one thing we keep. **No spurious MMU fault. Not on the rows that
   matter.** §6.
5. ⇒ **#1 and #2 are not two hazards to rank. They are one hazard with two halves.** The refused
   stop supplies a **writer**; the refused unbind and the never-released pin supply a **target**.
   Neither alone writes anything; **together** they are a silent cross-process write inside the
   guest. §7.
6. ⇒ **The re-ranking in the brief is right in its conclusion and understated in its reason** —
   and §7.4 is a **new** finding of the same class that neither w301 nor w306 has: the one guard
   against *"unbind a row hardware still resolves"* is **blind to the population it matters most
   for**, because that population's host-ness is recorded in a second place.

⊘ **This is not the "great result" the brief hoped for.** The refusal is *not* correct by design,
and this document does not delete a work item. It deletes the wrong *reason* for the work item and
replaces it with a sharper one.

---

## 1. Q1 — does the guest WAIT? **It waits for the CALL and never for the OUTCOME.**

The brief's framing was: *"If the guest never waits, our answer is decorative and the order is
unenforceable from our side."* Both halves need splitting, because the source splits them.

### 1.1 The chain, every hop `void`

| # | site | note |
|---|---|---|
| 1 | `ogkm-580: kernel-open/nvidia-uvm/uvm.c:1029` | `UVM_UNREGISTER_CHANNEL` ioctl route |
| 2 | `kernel-open/nvidia-uvm/uvm_user_channel.c:862` | `uvm_unregister_channel`; **the comment is `:875-876`**, the stop `:880` |
| 3 | `kernel-open/nvidia-uvm/uvm_user_channel.c:765` | `void uvm_user_channel_stop`; RM call `:787-788` |
| 4 | `kernel-open/nvidia/nv_uvm_interface.c:1431` | `void nvUvmInterfaceStopChannel` |
| 5 | `src/nvidia/arch/nvalloc/unix/src/rm-gpu-ops.c:779` | `rm_gpu_ops_stop_channel` |
| 6 | `src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:10916` | `void nvGpuOpsStopChannel` |
| 7 | `src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:1947` | `kchannelCtrlCmdStopChannel_IMPL` |

### 1.2 The status is discarded — verbatim

`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:10956-10965`:

```c
    stopChannelParams.bImmediate = bImmediate;
    NV_ASSERT_OK(
        pRmApi->Control(pRmApi, ..., NVA06F_CTRL_CMD_STOP_CHANNEL, ...));

    pKernelChannel->bIsContextBound = NV_FALSE;
```

`NV_ASSERT_OK` (`src/nvidia/inc/libraries/utils/nvassert.h:467-473`) expands with
`/* no other action */`. The assignment at `:10965` is **not guarded by the status**. UVM's mirror
`atomic_set(&user_channel->is_bound, 0)` (`uvm_user_channel.c:792`) is likewise unconditional, and
`uvm_user_channel_detach` then *asserts* on it — `UVM_ASSERT(!atomic_read(&user_channel->is_bound))`
at `:816`, under the comment *"The caller is required to have already stopped the channel"*
(`:813-814`). ⇒ **`is_bound == 0` is the entire confirmation, it is bookkeeping, and the only thing
checking it is an assertion on the value we just wrote ourselves.**

### 1.3 ★★★ But the guest is BLOCKED while we answer, and that is the load-bearing half

`kchannelCtrlCmdStopChannel_IMPL` on a GSP client is one `NV_RM_RPC_CONTROL`
(`kernel_channel.c:1958-1969`) → `rpcRmApiControl_GSP` (`src/nvidia/src/kernel/vgpu/rpc.c:10855`)
→ `_issueRpcAndWait` (`:11055`) → `rpcRecvPoll` (`:1972-1985`). **Synchronous.** The calling thread
does not proceed until the reply lands, and in our shim the vCPU is halted inside its own MMIO trap
for the whole of it (`crates/kayfabe-qemu-raw/src/shim.rs:12303-12308`, cited in
`cancellation_plane.md` §2.4).

⇒ ⊘ **"Our answer is decorative" is FALSE.** The *status* is decorative. The *window* is not: the
guest hands us a synchronous interval in which it is stopped and in which the stop is expected to
happen, and then it proceeds on the assumption that it did. **The ordering is enforceable from our
side.** What we lack is a host verb to perform in that window and somewhere off-BQL to run it
(`cancellation_plane.md` §2.4) — a build problem, not an unenforceability.

### 1.4 ⊘ And NVIDIA does not think the stop is final either

`ogkm-580: kernel-open/nvidia-uvm/uvm_user_channel.c:784-786`, a live TODO immediately above the
call:

> *"Bug 1737765. This doesn't stop the user from putting the channel back on the runlist, which
> could put stale instance pointers back in the fault buffer."*

and `:819-822`, on removing the channel from the instance-pointer table:

> *"only prevents new faults from being serviced. It doesn't flush out faults currently being
> serviced, nor prior faults still pending."*

⇒ the stop is a **fault-noise mitigation**, phrased as prevention. That matters for §7: the
sequence is not a safety invariant NVIDIA claims to hold absolutely, which bounds how much of the
harm can be attributed to defeating it.

---

## 2. Q2 — ★★★ does RM's own channel-FREE path idle the engine? **CPU-RM: no. GSP: it claims to — and that claim is ours to keep.**

This was named as the question that decides the whole thing. It does, but not in the direction the
brief anticipated.

### 2.1 CPU-RM's free does no hardware teardown at all

`kchannelDestruct_IMPL` (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:1132-1274`),
read in order:

| step | line | what it is |
|---|---|---|
| lock/params, CC bookkeeping (**not taken on GA106**) | `:1146-1190` | SW |
| `RMCFG_FEATURE_PLATFORM_GSP` err-context free | `:1192-1199` | **compiled out** (`generated/rmconfig.h:260` = 0) |
| ★ **`NV_RM_RPC_FREE`** under `IS_GSP_CLIENT \|\| IS_VIRTUAL` | **`:1204-1221`** | **the entire hardware teardown, delegated** |
| `shrkgrctxDetach` → `kgrctxUnmapBuffers_HAL` | `:1223-1232` | VA unmap only — and see §2.3 |
| `_kchannelFreeHalData` → instance block / RAMFC / USERD memdescs | `:1234` | memdesc frees |
| list removal, buf-pool release, TSG free, ChID release | `:1243-1261` | SW |

**Absent from the whole function:** `kfifoStartChannelHalt`, `kfifoCompleteChannelHalt`, any
channel-disable, any RC kill, any poll loop, any runlist or `NV_CHRAM_CHANNEL` write.
★ *Known-positive for those zeros:* each of those names returns live hits elsewhere in the tree
(`kernel_gsp.c:724`, `:742`; `kernel_fifo_ctrl.c`; `kernel_idle_channels.c:44`), so the searches
are live and the function genuinely contains none. This independently re-confirms
`cancellation_plane.md` §5 at a second reading.

**HAL bindings resolved for GA106 + GSP client**, since the brief required it:

| symbol | binds to | manifest |
|---|---|---|
| `kchannelDestroyMem_HAL` | `_GM107` | `generated/g_kernel_channel_nvoc.c:1011-1019` (the `_b3696a` no-op is `T234D\|T264D` only) |
| `kchannelFreeHwID_HAL` | `_GM107` | `g_kernel_channel_nvoc.c:1031-1039` |
| `kchangrpSetRealtime_HAL` | `_56cd7a` (`return NV_OK`) | `g_kernel_channel_group_nvoc.h:382`, `:447` — unconditional |
| `rpcFree_HAL` | `rpcFree_v03_00` | `g_rpc_hal.h:447`, assigned for IP ≥ `0x03000000` at `g_rpc_private.h:3016`. ⚠ The per-chip `rpcFree_STUB` rows are the **static vGPU** tables and are superseded |
| `kfifoStartChannelHalt` / `kfifoCompleteChannelHalt` | **`_GA100`** (the arm explicitly lists **GA106**) | `generated/g_kernel_fifo_nvoc.c:919-933`, `:935-949` |
| `kfifoIdleChannelsPerDevice_HAL` | `_KERNEL` (pure RPC passthrough) | `g_kernel_fifo_nvoc.h:1090` |

`kfifoPreemptChannel`, `kfifoChannelGroupPreempt`, `kfifoRunlistPreempt`, `kfifoDisableChannel`,
`kchannelDisableChannel`, `kchannelUnbindFromEngine` — **do not exist in this tree at all**
(physical-RM entry points; `RMCFG_FEATURE_PHYSICAL_RM` is defined nowhere). TSG level is the same
story: `kchangrpDestruct_IMPL` is literally `{ return; }`
(`src/nvidia/src/kernel/gpu/fifo/kernel_channel_group.c:40-44`), and `kchangrpapiDestruct_IMPL`
(`kernel_channel_group_api.c:592-665`) only recurses into per-channel frees.

### 2.2 ★★★★★ RM asserts free-implies-preempt — in a comment, about firmware we impersonate

The one place in the tree that answers the owner's question directly,
`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:10911-10915`:

> *"It does not prevent the user from freeing the associated `hClient` and `hChannel` handles,
> which means the instance pointer may no longer be associated with a user object at this point.
> **If the instance pointer still has an associated channel, the channel is preempted and
> disabled. Otherwise that must have already happened**, so we just need to drop the ref counts on
> the resources"*

⇒ **RM does believe channel-free implies preempt-and-disable.** So the owner's hypothesis is
*correct about NVIDIA's design* — and it does not help us, because:

- the preempt lives in **GSP firmware**, not in this tree (unauditable — stated as UNRESOLVED, not
  guessed at); and
- on a GSP client the only thing CPU-RM does for free is one synchronous `NV_RM_RPC_FREE`, and the
  only thing it does for `STOP_CHANNEL` is one synchronous `NV_RM_RPC_CONTROL`. **Structurally
  identical.** ⇒ *"rely on free instead of the explicit stop"* is a choice between **two GSP-side
  handlers, both of which are us.**

★ And note *why* UVM issues the explicit stop anyway even though it knows the above: it needs the
engine stopped **while keeping the channel object alive** — a strictly different requirement from
free, and the reason the two verbs both exist.

### 2.3 One real ordering guarantee CPU-RM does buy, and its scope

Within `kchannelDestruct_IMPL` the synchronous RPC at `:1216` precedes GR-ctx VA unmapping
(`:1230`), instance-block teardown (`:1234`) and ChID release (`:1260`); and at client teardown
channels/TSGs are `RS_FREE_PRIORITY_HIGH` (`src/nvidia/src/kernel/rmapi/resource_list.h:354`,
`:469`), hoisted to the front of the pending-free list by
`rmclientPostProcessPendingFreeList_IMPL` (`src/nvidia/src/kernel/rmapi/client.c:520-572`,
esp. `:539-542`). ⇒ **channels die before the client's memory objects and VA spaces.**

⚠ **This is real and it is not the guarantee we need.** It orders *CPU-RM's own* teardown against
the GSP reply. It says nothing about whether the engine stopped, and the UVM unmap that actually
matters (§4) is not in this ordering at all — it happens in `nvidia-uvm`, before the channel is
freed, and touches page tables RM does not own.

⊘ One caveat found while reading, recorded so it is not re-derived: generic resserv removes **CPU
mappings before** the destructor runs (`src/libraries/resserv/src/rs_client.c:829-849`, mappings at
`:832-837`, `objDelete` at `:849`). For a channel that is the USERD BAR1 window, so the guest's
CPU-visible USERD disappears *before* GSP is told to free the channel. Not an engine-state issue;
it is a note for anyone modelling USERD lifetime.

### 2.4 ⊘ The one CPU-side preempt that DOES exist on GA106, and why it is not a route

`kfifoStartChannelHalt_GA100` (`src/nvidia/src/kernel/gpu/fifo/arch/ampere/kernel_fifo_ga100.c:876-912`)
and `kfifoCompleteChannelHalt_GA100` (`:924-955`) are a genuine CPU-side preempt-and-wait —
`NV_CHRAM_CHANNEL` `_ENABLE _IN_USE` clear, `NV_RUNLIST_PREEMPT _TYPE _RUNLIST`, then a poll on
`_RUNLIST_PREEMPT_PENDING` — via bare `GPU_REG_WR32`/`RD32`, **bypassing GSP**, and they **bind for
GA106**. Their only caller in the whole tree is `kgspRcAndNotifyAllChannels_IMPL`
(`src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:724`, `:742`) — the **GSP-has-crashed** path, whose
own comment (`:727-732`) says the preempt is done from the CPU precisely because *"the GSP is no
longer servicing interrupts."*

⇒ ★ There exists a register-level preempt the guest will perform **itself** if it believes the GSP
is dead, and those registers are BAR0 writes we already trap. ⊘ **Not a viable route**: reaching it
requires declaring GSP death, which tears down everything else. Recorded because it is the only
CPU-side enforcement on this chip and someone will find it again.

---

## 3. Q3 — is there a "cleanup on channel death" mechanism? **No.**

The owner asked specifically for a UVM flag, a GSP-side teardown, or an RM invariant that unmaps or
fences when a channel dies. There is none.

- **No such flag exists.** `uvm_user_channel_t` has exactly one field in this space —
  `atomic_t is_bound` (`kernel-open/nvidia-uvm/uvm_user_channel.h:187`, documented `:184-186`) — and
  it gates nothing in hardware. Its RM twin `bIsContextBound`
  (`generated/g_kernel_channel_nvoc.h:297`) has exactly one functional consumer, and it is a
  *schedulability veto*, not a teardown action: `kernel_channel.c:2200` refuses to schedule an
  externally-owned-VAS GR channel whose allocations are unbound.
  ★ *Known-positive for the zero:* a whole-tree case-insensitive grep for
  `closeonstop|close_on_stop|unmapOnStop|teardownOnStop|autoUnmap` returns only
  `serverUpdateLockFlagsForInterAutoUnmap` (an unrelated resserv lock-flag helper, e.g.
  `src/nvidia/src/kernel/rmapi/mapping.c:556`), while the same grep style returns 5 real hits for
  `bIsContextBound`. The search is live; the symbol does not exist.
- **RM's RC path is notify-only.** `krcErrorSetNotifier_IMPL`
  (`src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:234-345`) does three things: the
  reboot-required WAR if `bUvmOwned` (`:258-261`, see §8), the notifier write via
  `krcErrorWriteNotifier_HAL` (`:329-336`), and `kbusFlush_HAL` (`:339-341`) — **which orders the
  notifier write and nothing else.**
  ★ *Decisive negative:* grepping all of `src/nvidia/src/kernel/gpu/rc/` for
  `unmap|invalidate|fence|idle|preempt` yields zero hits outside `kernel_rc_watchdog.c`, and those
  four (`:1374`, `:1388`, `:1445`, `:1467`) are the watchdog's own pushbuffer fences on its own WRC
  channel. **RM's recovery unmaps nothing, fences nothing, invalidates no TLB.**
  ⚠ **HAL bindings, resolved:** `krcErrorSendEventNotifications_HAL` → `_KERNEL`
  (`generated/g_kernel_rc_nvoc.h:271`, `:435`); `…CtxDma_HAL` → `_FWCLIENT` (`:280`, `:436`) — which
  writes **no** notifier, its own header saying GSP already did (`kernel_rc_notification.c:352-358`),
  confirming the prior lane's finding; `krcErrorWriteNotifier_HAL` → `_CPU` (`:213`, `:434`);
  `kchannelNotifyRc_HAL` → `_IMPL` (`g_kernel_channel_nvoc.h:432`, `:942`). None of these is
  chip-dispatched — there are no HAL rows for them in `g_kernel_rc_nvoc.c` at all, so GA106 is not
  a special case.
- **Channel free implies no TLB invalidation of a UVM VA space.** `kchannelDestruct_IMPL` contains
  no invalidate of any kind. Its GR-ctx teardown reaches `kgrctxUnmapBuffers_KERNEL`
  (`src/nvidia/src/kernel/gpu/gr/kernel_graphics_context.c:2631`, invoked `:3699`) — and **two
  gates neuter it for UVM**: `kgrctxShouldCleanup_KERNEL` (`:2489-2496`) returns
  `gpuIsClientRmAllocatedCtxBufferEnabled`, and the PTE write itself is behind
  `if (vaListGetManaged(pVaList))` (`src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:2043`) while
  UVM-promoted context buffers are explicitly marked unmanaged —
  `vaListSetManaged(&pEngCtxDesc->vaList, NV_FALSE)` at `kernel_channel.c:3758`, comment *"Since
  the memdesc is already virtual, we do not manage it"*, and `vaListSetManaged`'s own doc
  (`src/nvidia/src/kernel/mem_mgr/vaddr_list.c:255-257`): *"`NV_FALSE` to indicate addresses not
  mapped by RM."* The GR-context mapping paths bail out wholesale for externally-owned VA spaces
  (`kernel_graphics_context.c:1614`, `:1884`, `:2072`; `kernel_graphics.c:1816`, `:1934`).

⇒ **RM does not own, does not walk, and does not invalidate UVM's page tables.**

---

## 4. Q4 — what the unmap path actually does, and what it fences

`uvm_unregister_channel` → `uvm_user_channel_detach` (`uvm_user_channel.c:795`) → `destroy_va_ranges`
→ `uvm_va_range_destroy_channel` (`uvm_va_range.c:529`, called at `:604`) →
`uvm_page_table_range_vec_clear_ptes` (`:539`, membar chosen `:538`) + `_deinit` (`:540`).

⊘ `nvUvmInterfaceUnmapExternalAllocation` **does not exist in this tree** (whole-tree grep: zero).
**UVM owns the page tables of an externally-owned VA space and writes the PTEs itself; it never
asks RM to unmap.** That is the fact §6 turns on.

The route is not blind — it does invalidate — but only against itself.
`uvm_page_table_range_vec_clear_ptes` dispatches to the `_gpu` variant (`uvm_mmu.c:1980`; the
`_cpu` form at `:1944` needs `uvm_mmu_use_cpu`, false on GA106):

```
1. STOP_CHANNEL pushed (RPC to us; result not awaited)   uvm_user_channel.c:787-788
   ... va_space lock dropped and retaken in write mode   :882-888
2. PTE clear, invalid_pte = 0                            uvm_mmu.c:2017 (invalid_pte :1989)
3. TLB invalidate + membar, SAME push, final iteration   uvm_mmu.c:2028
4. push end                                              uvm_mmu.c:2030
5. WAIT — uvm_tracker_wait_deinit                        uvm_mmu.c:2042
6. put_ptes (free the page-table memory)                 uvm_mmu.c:1362-1365
7. deferred: fault-buffer flush, then ReleaseChannel     uvm_user_channel.c:819-858
```

The membar is chosen at `uvm_va_range.c:538` by `uvm_hal_downgrade_membar_type`
(`uvm_hal.c:964-978`): `UVM_MEMBAR_GPU` for local vidmem on a non-coherent part (GA106 qualifies),
`UVM_MEMBAR_SYS` otherwise. Its own comment (`:965-970`) scopes it to ordering at the L2 level for
the mapped memory.

★★ **Both waits are on UVM's OWN channels.** `page_tree_begin_acquire` (`uvm_mmu.c:55-67`) pushes to
`UVM_CHANNEL_TYPE_GPU_INTERNAL` on `tree->gpu->channel_manager` — UVM's own LCE pool. **Not one
instruction in the unmap path waits on, preempts, idles or fences the user channel.**

⇒ **Clear-then-invalidate-then-wait, and the wait is against the wrong engine.** The only
serialization between *"the user's engine may be executing"* and *"the PTEs are gone"* is
`nvUvmInterfaceStopChannel`.

---

## 5. ★★★★★ The premise, adjudicated: the guarantor is us, and that is what the architecture *means*

> Owner: *"If ogkm already by construction (a compliant guest) tears down the channel and waits for
> us to complete it before killing UVM, then we don't have to assert this order."*

**No — and the "by construction" idea cannot be repaired by picking a different verb.** Assemble
§1–§4:

| candidate guarantor | what it would give | where it actually executes |
|---|---|---|
| UVM waiting on the stop | the ordering | ⊘ UVM performs **no** independent wait (§1.2). It blocks only inside our RPC (§1.3) |
| CPU-RM's `STOP_CHANNEL` handler | the preempt | ⊘ one `NV_RM_RPC_CONTROL` (`kernel_channel.c:1958-1969`) ⇒ **us**. Its non-RPC arm is `kchannelFwdToInternalCtrl_56cd7a`, a literal `return NV_OK` (`g_kernel_channel_nvoc.h:1270-1272`) |
| RM's channel FREE | the preempt, implicitly (§2.2) | ⊘ one `NV_RM_RPC_FREE` (`kernel_channel.c:1204-1221`) ⇒ **us** |
| RC recovery on channel death | an unmap or a fence | ⊘ notify-only (§3) |
| GR-context teardown | an unmap | ⊘ neutered for externally-owned VA spaces (§3) |
| the unmap path itself | a fence against the user channel | ⊘ fences UVM's own channels only (§4) |

⇒ **Every row that could carry the guarantee terminates at the GSP, and we are the GSP.** The
compliant guest is not failing to do its part; it *delegated* this part, which is exactly what the
`ROUTE_TO_PHYSICAL`/GSP split is. ⇒ **"guaranteed by construction" is unavailable as a category
here**, and the correct sentence is: *the order is guaranteed by our answer, and we answer `0x56`.*

### 5.1 ⊘ The brief's threat-model framing does not reach this finding, and that is worth saying

The owner's rule licenses *guest-kernel non-compliance* that harms only the guest. **The guest
kernel is not non-compliant here.** UVM follows the published contract exactly — it calls a control
whose header says *"disabling and unbinding the channel and removing it from runlist"*
(`ogkm-580: ctrla06fgpfifo.h:216-243`) and proceeds on the synchronous return. **We** are the
deviating party, against the spec we ourselves present to the VM.

⇒ The *"guest harms itself ⇒ not our obligation"* allowance is **the wrong instrument for this
finding**, regardless of who is harmed. It is the right instrument for the case the owner actually
named — *"a UVM free that faults before channel close"* — i.e. a guest kernel that chose to unmap
without stopping. **That is not what UVM does**, so the case is hypothetical today and the
allowance is unexercised.

---

## 6. ★★★★★ And in THIS port the modelled harm cannot occur — because we refuse the unmap too

Here is where the brief's harm model breaks, and it breaks in the dangerous direction.

**The guest's unmap never reaches us as a verb.** §4: UVM writes its own PTEs. `nvidia-uvm`'s
`uvm_release` / `uvm_va_space_destroy` / `uvm_va_space_mm_shutdown` are, in this port's own words,
*"**not observable events here at all**"* — they reach us only once the guest driver turns them
into `RpcFunction::Free` (`crates/kayfabe-qemu-raw/src/shim.rs:7545-7551`). So the unmap arrives as
**page-table writes**, and the sweep is the only thing that sees it.

**And the sweep refuses to act on it.** `kayfabe_mmu::reach::apply_settlement`
(`crates/kayfabe-mmu/src/reach.rs:799-819`) — *"the one place a reachability decision becomes a
table mutation"* — takes each proposed unbind and, if the row is host-published, refuses:

```rust
Some((_, _, b)) if b.host.is_some() => {
    out.refusals.push(crate::walker::PopulateRefusal::UnbindsPublished { va });
    // ★ The shadow is NOT told the unbind happened, because it did not.
}
```

The refusal's own doc says exactly why (`crates/kayfabe-mmu/src/walker.rs:956-968`):

> *"dropping the range from the table would leave the host object still allocated and still mapped
> into that address space's host VAS, with no core state naming it. **That is worse than a leak —
> hardware would keep resolving it** — and it is what a teardown would do to a published range if
> the unbind were performed rather than refused. Unpublishing needs a worker and an unmap verb,
> i.e. the forwarding plane. So the refusal is the answer, and the binding stays."*

⇒ ★★★ **The host translation the engine would have faulted on is precisely the one thing we keep.**
The refused `STOP_CHANNEL` and the refused unbind are two halves of one posture, and their
composition is not a fault — it is **silence**.

### 6.1 Neither population produces the fault UVM feared. Stated per population.

| population | `[measured w290]` | host mapping after the guest's unmap | can the engine fault on it? |
|---|---|---|---|
| **host-published rows** (`Binding::host` set) | `host_rows = 4 of 16425`, moved to **34** by w290 leg 8 | **kept** — `UnbindsPublished` refuses | ⊘ **no.** It resolves |
| **unpublished rows** (99.4 % today) | the remainder | there never was one | ⊘ not caused by the unmap — the engine faults `FAULT_PDE` on these *before and after*, which is the wall w290 measured |
| **guest-RAM pins spanning >1 row** | `Vas::guest_ram_pins`, see §7.4 | **kept, and now unnameable** | ⊘ no — and this is the bad one |

⇒ **In no population does the guest's unmap-after-a-refused-stop cause an MMU fault.**

⚠ **And the exposure grows as the campaign succeeds.** The publication work (`host_rows` 4 → 34,
legs 4–7) is *increasing* the first population by design. ⇒ **the silent population is the one we
are building.** That is a ranking fact, not a rhetorical one.

---

## 7. Who is harmed — per failure mode

⚠ Scope note before the table: **the two halves compose.** A stale translation is a *capability*,
not a write; it needs a writer. The refused stop is what leaves a writer alive. Rows are therefore
classified for the composition, and say when they need the other half.

| # | failure mode | harmed | verdict under the owner's rule | evidence |
|---|---|---|---|---|
| **A** | Refused `STOP_CHANNEL`, channel keeps executing, then the process's own pages are unmapped **and the guest re-uses them for another process** while our host mapping is retained | ★★★ **another process inside the guest** — silent write, no fault, no notifier | **OURS.** In the violation list. And *not* excused as self-harm: the guest is compliant, we are the deviating party (§5.1) | §6 + `w301` §3.2/§3.3; `guest_blast_radius.md` §5.2 for host-side containment |
| **B** | Same, but the pages are *not* reused before the process dies | the dying process only | **not our obligation** — self-harm, contained | — |
| **C** | Refused stop + a row we never published ⇒ engine faults `FAULT_PDE` | the faulting channel; ★ but for a **replayable** fault the guest **hangs** rather than erroring, because `DELIVERY_UNBUILT` | self-harm, **but the guest is not told** — a hang, not an error | `crates/kayfabe-abi/src/faultbuffer.rs:126-129` |
| **D** | Leaked `guest_ram_pins` after VAS death (no removal path anywhere) | ★★★ any later process in the **same guest** that gets those pages | **OURS** | `crates/kayfabe-core/src/gpu.rs:233`; §7.4 |
| **E** | We free a **host** channel we never stopped (`stage_dropped_channels`, `dispose_on` after an R5 refusal) | depends on GSP's unaudited free handler (§2.2) | **OURS** — and `cancellation_plane.md` §5.1's invariant (*never free a host channel we have not first stopped*) makes the firmware question moot | `gpu.rs:3336+`; `cancellation_plane.md` §4.3 |
| **F** | Escalation to a GPU-wide / host-wide error | see §8 | **not established; the host-side measurement says contained** | §8 |
| **G** | Cross-**tenant** or host-side reach | **nobody** | blast radius is one sandbox per `(Proc, GpuId)`, guest-RAM memfd granted per-isolate; reaches any page of **its own VM's** RAM and no other VM's | `w301` §3.3; `multi_tenant_isolation_assessment.md` §1 |

★ **G is the good news and it is unchanged**: nothing in this document reaches the host or a
neighbour tenant. The VM↔host boundary is not what is at stake; the **process↔process boundary
inside the guest** is.

### 7.4 ⊘⊘ NEW — the `UnbindsPublished` guard is blind to the population it matters most for

`apply_settlement` tests **`b.host.is_some()`** (`reach.rs:809`) — the `Binding::host` field. But
guest-RAM pins are recorded in `Vas::guest_ram_pins` (`gpu.rs:233`), and w291's merge writes
`Binding::host` **only for an exact-extent row**:

> *"⊘⊘ **BOUNDED TO AN EXACT-EXTENT ROW, AND THAT BOUND IS THE WHOLE DESIGN.** … A pin whose grant
> spans several rows (legs 4-6's run pins) therefore **binds NOTHING here** and behaves exactly as
> before."* — `crates/kayfabe-fwd/src/lib.rs:1916-1921`, guard at `:1930-1932`

and w301 §3.2 records that **the production caller routinely produces multi-row run pins**.

⇒ For a multi-row run pin the row's `host` is `None`, so **the guard does not fire, the unbind is
performed, and the table row is dropped — while the host mapping and the `pin_user_pages` pin
survive.** That is verbatim the state `UnbindsPublished`'s own doc calls *"worse than a leak —
hardware would keep resolving it"*, reached by **ordinary teardown**, not by a race.

★ This is the tree's own named class — `a_second_source_of_truth_beside_a_complete_value` — arriving
as a **safety** defect rather than an instrument one: the kind is recorded beside the value, and the
guard reads only one of the two places.

⚠ **Stated as a source reading, not a measurement.** It needs one boot to confirm the sequence
actually occurs (a multi-row pin whose VA the guest later unbinds); the falsifier is cheap and
three-valued in §10.

---

## 8. ★★ The boundary condition — can it go GPU-wide? Two scopes, and they answer differently

The brief calls this *"the one thing that would flip the verdict."* It does not flip it, but it
splits.

### 8.1 Host hardware scope — **measured contained, on this exact fault class**

`[run: scripts/bench/gpu_fault_containment.sh, 2026-08-01T23:34Z, vast 46529600, RTX 3060 GA106,
host 580.159.04 open, repo eea787f; log docs/reference/bench_evidence/fault-containment-eea787f-ga106.log]`,
recorded at `guest_blast_radius.md` §5.2. The Xid is **exactly the class in question**:

```
NVRM: Xid (PCI:0000:00:08): 31, … MMU Fault: ENGINE GRAPHICS GPC1 GPCCLIENT_T1_0
  faulted @ 0x7000_00000000. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_WRITE
```

| arm | result |
|---|---|
| attacker's own context | `CUDA_ERROR_ILLEGAL_ADDRESS`, sticky |
| **bystander holding a LIVE context across the fault** | **2 682 576 / 2 682 576 correct, 0 errors** |
| the GPU afterwards | util 3 %, **no reset, no "reboot required", no "fell off the bus"** |

⇒ the escalation path was *entered* (Xid fired, RM's recovery ran), and the three named hazards —
whole-runlist preempt, node-level reboot-required latch, GSP-death halting every channel — did not
materialise. ⚠ **Scoping this honestly:** the measured fault was a wild address *never mapped*, not
a mapping *revoked under execution*. Both are MMU faults at translation time, but they are not
proven identical, and `guest_blast_radius.md` §5.1 lists **MMU-fault storms** as explicitly
untested. And per §6, this port cannot produce the revoked-mapping fault on published rows anyway.

⇒ **The verdict is not flipped: no host-wide or cross-tenant escalation is established, and the one
measurement points the other way.** ★ Note also that a wedge — a hung engine needing a GPU reset —
sits **inside** property P by construction (`guest_blast_radius.md` §2: an unprivileged local
process can wedge the GPU too). So even an escalation to engine-reset would be a DoS inside the
accepted model, not a boundary violation.

### 8.2 ★★★ Guest driver scope — an error on a `bUvmOwned` channel IS guest-global

`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:255-262`:

```c
    // WAR bug 4503046: mark reboot required when any UVM channels receive an error.
    if (pKernelChannel->bUvmOwned) { sysSetRecoveryRebootRequired(pSys, NV_TRUE); }
```

with UVM's own fatal error driver-global and never cleared outside test builds
(`atomic_cmpxchg(&g_uvm_global.fatal_error, …)`, `kernel-open/nvidia-uvm/uvm_global.c:420-445`;
contract at `uvm_global.h:262-266`), reported onward by `nvGpuOpsReportFatalError`
(`nv_gpu_ops.c:11538-11549`), and NVIDIA's own note that *"UVM currently attributes all errors as
global and fails operations on all GPUs"* (`kernel_gsp.c:702-707`). Full treatment:
`resume_from_fault.md` §S5(c).

⇒ **inside the guest, a channel-scoped fault can become a guest-wide, reboot-required DoS.**

★ **But `bUvmOwned` does not cover the channel in this sequence.** The flag is set only where UVM
allocates **its own** channels — `nvGpuOpsChannelAllocate` sets
`FLD_SET_DRF(_KERNELCHANNEL, _ALLOC_INTERNALFLAGS, _UVM_OWNED, _YES, …)`
(`nv_gpu_ops.c:6024-6027`), the only setter in the tree (the only other `bUvmOwned` reads are
`kernel_rc_notification.c:259`, `kernel_channel.c:2808`, `kernel_fifo.c:3864`). The channel being
stopped in `uvm_unregister_channel` is a **user** channel registered via `UVM_REGISTER_CHANNEL`,
not a UVM-owned one. ⇒ **the reboot-required WAR is not on this path.**

⇒ **Answer to the boundary question: NO for the host; NO for this path inside the guest; YES for a
different path (a fault landing on a UVM-owned channel), which this sequence does not take.** The
verdict stands.

---

## 9. ★★ The re-ranking — I agree with the conclusion, and the reason is stronger than stated

The brief proposed inverting w306's ranking: leaked pins **above** teardown order, because the
first is a silent cross-process write and the second is "a fault, contained, self-harm."

**Agreed on the conclusion. The reason is wrong, and correcting it strengthens the case.**

- ⊘ *"teardown order → a fault"* is **false in this port** (§6). We refuse the unbind, so there is
  no fault on the rows where a host mapping exists, and on the rows where none exists the unmap is
  causally irrelevant.
- ⇒ **Teardown order has no independent harm at all.** Its harm *is* the pins' harm. The refused
  stop contributes the **writer**; the retained translation contributes the **target**.
- ⇒ The right structure is not `#2 > #1`. It is: **one composite finding, ranked first**, with
  #1 as its enabling half and #2 as its exposure half — and a **third** half, §7.4, which neither
  audit has.

**Revised ranking** (replacing `cancellation_plane.md` §6 rows 1 and 2):

| rank | finding | why here |
|---|---|---|
| **1** | ★★★★★ **The retained-translation composite**: refused stop (writer) + refused unbind / never-released pin (target) ⇒ a **silent** cross-process write inside the guest, on ordinary teardown, no fault and no signal | it is the only row with *no observable*, it is reachable by ordinary use in 115/195 boots, and its population **grows with the publication work** |
| **2** | ⊘ **§7.4 — the guard is blind to multi-row guest-RAM pins**: the unbind is *performed*, leaving a live host mapping with no core state naming it | strictly worse than the refused-unbind case the guard was written for, and produced by the same teardown |
| **3** | **No off-BQL execution site for host verbs** (`cancellation_plane.md` §2.4) | the structural blocker on fixing #1's writer half |
| **4** | Free-after-ring, first-doorbell scoped | needs a race; four of five `Stale` variants non-retryable |
| **5** | `GPU_EVICT_CTX`, `RESET_CHANNEL`, descheduling, `CompletionQueue::pending` | as `cancellation_plane.md` §6 |

⚠ **What would falsify the re-ranking**, pre-registered: if the composite's write is *not*
reachable — i.e. if by the time UVM unmaps, no channel is ever still executing — then #1 collapses
to a pure capability leak and #2 in the old ranking is simply right on its own. That is a boot
question (§11.2), and the honest state today is that UVM's own comment (*"to prevent spurious MMU
faults"*) is evidence that engines **can** still be executing at teardown, which is why the sequence
exists at all — but evidence about NVIDIA's driver is not a measurement of ours.

---

## 10. ⊘ What is wrong in the brief, named

The brief asked for this explicitly, so it is stated plainly.

1. ⊘ **"If the guest never waits, our answer is decorative and the order is unenforceable from our
   side."** The guest *does* block — synchronously, inside our RPC (§1.3). Only the status is
   discarded. **The order is enforceable from our side**; the obstacle is a missing host verb and a
   missing off-BQL site, not unenforceability. Acting on the original reading would have retired a
   fixable item as impossible.
2. ⊘ **"teardown order → a fault, contained, self-harm."** No fault occurs on the rows that have a
   host mapping, because we refuse the unbind (§6). The harm is silent, not loud.
3. ⊘ **The owner's premise as stated is unavailable as a category** (§5): not "the guest does not do
   it" but "the guest **delegated** it, to us." No choice of verb repairs this, because free and
   stop are structurally the same single RPC on a GSP client (§2.2).
4. ⊘ **The threat-model allowance is aimed at the wrong party** (§5.1). It licenses guest-kernel
   non-compliance; here the guest kernel is compliant and we are the deviating party. The allowance
   is unexercised on this path.
5. ⊘ **`research_clones/ogkm/` is 610.43.02, not 580.159.04.** The brief names the former with the
   latter's version. Everything here is read from `research_clones/ogkm-580.159.04/`, the pinned
   host driver. `w301`'s driver-version note records that the load-bearing bodies were diffed
   against 610 and are byte- or semantically identical.
6. ⊘ **"Master is `74200b2b`"** is the **kayfabe** master; `/workspace/nvidia-gpu-passthrough`'s
   master is `2a5df06` and has no object `74200b2b`. This document is on `w307-teardown-order` off
   kayfabe `74200b2b`, in its own worktree; `/workspace/nvkvm-rs`'s working tree was not touched.
7. ★ **What the brief got right and should be kept:** the instruction to check the GPU-wide boundary
   rather than assume it (§8 splits into two scopes that answer differently, and the guest-scope
   one is a real escalation on a *neighbouring* path), and the instruction to re-rank (§9 — the
   conclusion holds).

---

## 11. What this document could not settle

1. **Whether the GSP fences the engine on channel free.** Out of tree; `nv_gpu_ops.c:10911-10915`
   asserts it does, and that assertion is unauditable. ⇒ **it does not matter for the design**, for
   `cancellation_plane.md` §5.1's reason: where we forward, the host GSP's behaviour *is* what a
   native process gets; where we free on our own initiative, the fix is the invariant *never free a
   host channel we have not first stopped through the host RM*, which makes the firmware question
   moot. §5.2 of that document gives the one native experiment if severity is ever wanted.
2. ★ **Whether any channel is still executing when UVM unmaps, in OUR guest.** This is the single
   fact that decides whether §9's rank-1 composite is a live write or a capability leak. It is a
   boot question and it is cheap: on the teardown edge, for each channel being unregistered, record
   whether its host twin's `GP_GET != GP_PUT`. Three-valued, so the uninteresting answer is
   distinguishable: **advancing** ⇒ the composite is live; **equal and stable** ⇒ capability leak
   only; **no host twin** ⇒ the whole row is vacuous for that channel and tier V (`cancellation_plane.md`
   §2.1) is most of the plane.
3. **Whether §7.4's sequence actually occurs.** Needs one boot: a multi-row guest-RAM pin whose VA
   the guest later unbinds. Falsifier: count `UnbindsPublished` refusals **and** performed unbinds
   whose VA is covered by a `guest_ram_pins` entry. Non-zero in the second counter is the finding;
   zero in **both** means the sweep never proposes those unbinds at all, which is a different and
   also useful answer.
4. **Whether a revoked-mapping fault behaves as the wild-address fault of §8.1.** Same Xid class at
   translation time, not proven identical, and MMU-fault *storms* are untested
   (`guest_blast_radius.md` §5.1 item 4).

## See also

- `docs/design/cancellation_plane.md` — the verb table and the tiers; §6 rows 1–2 are re-ranked here.
- `docs/audits/w301_cancellation_error_leaks.md` §3.2, §3.3 — the leak census §7 ranks against.
- `docs/design/guest_blast_radius.md` §2, §5.1, §5.2 — BRICK/WEDGE, and the containment runs.
- `docs/design/resume_from_fault.md` §S5 — the `bUvmOwned` reboot-required escalation §8.2 scopes.
- `docs/design/blocking_and_completion_model.md` — `INLINE-SAFE`, the constraint on fixing §9 #3.
