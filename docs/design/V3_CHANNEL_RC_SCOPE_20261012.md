**STATUS: RESEARCH, 2026-10-12.**

# V3 channel RC scope: how far one unprivileged channel's PBDMA error can spread

Source-only analysis of ogkm 595.84 (`/workspace/ogkm-tars/open-gpu-kernel-modules-595.84`). No GPU, QEMU or
host was used. Paths below are relative to `src/nvidia/src/kernel/` unless they start with `src/` or
`kernel-open/`. Labels: **[MEASURED]** = read from the cited source line; **[INFERRED]** = my reading, not
shown by any line; **[EVIDENCE]** = a kayfabe measurement already in git (cited, not redone);
**[EXTERNAL]** = from the public NVIDIA Xid catalog or memory, not from ogkm, unverified here.

## 0. Short answer

**Can one VM's unprivileged channel RC disrupt another VM's work? UNKNOWN from source. The designed
common case is "no" for correctness and "a short preempt gap, size unmeasured" for latency. There are five
named conditions under which it is "yes".**

Why unknown: in a GSP host the PBDMA interrupt is serviced, and the recovery chosen, by GSP-RM firmware.
That code is not in ogkm. The only RC code in the tree is CPU-RM, the receiver of events that firmware
already decided to send [MEASURED: section 1]. ogkm contains no `kfifoServicePbdmaInterrupt`, no engine
reset code and no runlist-preempt-on-RC; `ROBUST_CHANNEL_PBDMA_ERROR` appears once outside `nverror.h`, in a
log-dump list (`gpu/rc/kernel_rc.c:660`). So the question "what is torn down" can only be answered for what
CPU-RM does afterwards, plus what the hardware headers and the CPU-RM crash path show about the primitives.

What the source does support:

- **Per-channel containment is the hardware's design [MEASURED headers, INFERRED semantics].** Channel RAM has
  per-channel `PBDMA_FAULTED` and `ENG_FAULTED` bits with per-channel reset updates
  (`src/common/inc/swref/published/ampere/ga100/dev_runlist.h:40-45, 71-72`), and `RUNLIST_PREEMPT` has a
  `TYPE_TSG` as well as a `TYPE_RUNLIST` (`dev_runlist.h:88-90`).
- **CPU-RM notifies one channel or one TSG.** The RC event carries a `scope` (channel or TSG) and one `chid`
  (`src/nvidia/generated/g_rpc-structures.h:1603-1618`, `gpu/gsp/kernel_gsp.c:597-723`).
- **[EVIDENCE]** Every kayfabe Xid 32 so far (26 and 186 in the display runs, the stray-`0x200` probe) landed
  on the offending client's own channels, and the offender's own FPS collapsed to 1.2-1.3. Whether any other
  client was disturbed was never recorded (`docs/design/V3_DISPLAY.md` lines 417-440; `docs/OWNER_RULINGS.md`
  lines 885-887: "Not measured: cross-object, cross-channel and cross-client isolation, and rate").

Conditions under which one VM's RC reaches another VM:

1. **Same TSG, hence same RM client.** `RC_NOTIFIER_SCOPE_TSG` notifies every channel of the TSG
   (`gpu/rc/kernel_rc_notification.c:270-277, 385-391`). A TSG belongs to one RM client [INFERRED from the
   resource-server parent chain], so this is cross-VM only if kayfabe ever puts two VMs' channels under one
   host client. It must not.
2. **An RC that cannot be done at TSG granularity.** If firmware must reset the engine, or the TSG preempt
   hits the context-switch timeout, every TSG on that engine or runlist is affected. Firmware code, not
   visible. The evidence that these primitives exist: `NV906F_CTRL_CMD_RESET_CHANNEL` is documented as
   "resets the channel ... and also resets the specified engine" (`src/common/sdk/nvidia/inc/ctrl/ctrl906f.h`,
   comment above `NV906F_CTRL_CMD_RESET_CHANNEL`), and notifiers 138/139/140 are TSG-preempt-complete,
   runlist-preempt-complete and CTXSW-timeout (`src/common/sdk/nvidia/inc/class/cl2080_notification.h:177-179`).
3. **The RM GPU-hang watchdog is enabled.** Any non-privileged client can enable it
   (`NV2080_CTRL_CMD_RC_ENABLE_WATCHDOG`, flags `0x18` = `NON_PRIVILEGED|GPU_LOCK_DEVICE_ONLY`,
   `src/nvidia/generated/g_subdevice_nvoc.c:7303-7307`). It is off by default on Linux
   (`src/nvidia/arch/nvalloc/unix/src/osinit.c:2294-2299`). Once on, a GR0 hang of more than 7 s
   (`src/nvidia/interface/nvrm_registry.h:1590`) sends `NV2080_CTRL_CMD_INTERNAL_RC_WATCHDOG_TIMEOUT` to GSP
   (`gpu/rc/kernel_rc_watchdog_callback.c:282-292, 360-376`), whose recovery is firmware. Section 4, W4.
4. **GSP fatal.** A firmware crash or RPC timeout makes CPU-RM RC every user channel of every client and mark
   the device for reset (`gpu/gsp/arch/turing/kernel_gsp_tu102.c:1008-1018`, `gpu/rc/kernel_rc.c:689-761`).
   Reaching it from an unprivileged channel needs a firmware bug; the source cannot say whether one exists.
5. **A GPU-level recovery action raised by firmware** (`GPU_RECOVERY_EVENT_TYPE_GPU_REBOOT`,
   `..._SYS_REBOOT`): sets "GPU reset / node reboot required" (`gpu/gsp/kernel_gsp.c:1695-1720`,
   `gpu/gpu.c:7200-7262`). Whether a PBDMA error can raise it is not visible; contained-ECC errors do.

Separate from disruption: **the RC event is a cross-client information channel.** Every client that
registered `NV2080_NOTIFIERS_RC_ERROR` on its subdevice is woken with `exceptType` for any channel's RC, with no
per-channel filter when MIG is off (`gpu/gpu_rmapi.c:624-728`, filter early-out at `:438-440`). Section 6.

## 1. What is in the tree and what is not

[MEASURED] The open modules require GSP (`README.md:18`). On such a host:

- `kernel_gsp.c` handles firmware events in `_kgspProcessRpcEvent`: `NV_VGPU_MSG_EVENT_RC_TRIGGERED` (dispatch
  at `gpu/gsp/kernel_gsp.c:1459`), `NV_VGPU_MSG_EVENT_OS_ERROR_LOG` (`:1475`), `MMU_FAULT_QUEUED` (`:1463`),
  `RECOVERY_ACTION` (`:1575`), `FECS_ERROR`, `PMU_HALTED` and others.
- Events are drained in the interrupt bottom half under the GPU lock: `kgspRpcRecvEvents_IMPL`
  (`gpu/gsp/kernel_gsp.c:6059-6082`, the comment states the lock exclusion; caller `kernel_gsp_tu102.c:1095`).
- Not in the tree: PBDMA interrupt decode, the software-method dispatcher, RC target selection, engine reset,
  runlist preempt for RC. `grep -rni pbdma` over `intr.c`, `gpu/rc`, `kernel_fifo.c`, `kernel_channel.c` returns
  address-map code only. The PBDMA interrupt bit definitions are not in the published headers either
  (`dev_pbdma.h` is 28 lines, `dev_esched_pbdma.h` 37).
- `chandesGetSwMethods` and `chandesIsSwMethodStalling` are defined (`gpu/fifo/channel_descendant.c:282, 317`)
  but nothing in the tree calls them; the caller is firmware (the same conclusion kayfabe reached for display-SW,
  `docs/design/V3_DISPLAY.md` line 417: "serviced by GSP firmware").

So everything below the line "GSP decides" is [INFERRED] from CPU-RM's receiving code, from the RM control
documentation, and from kayfabe's own measurements.

## 2. The CPU-RM side of one RC (common to all three triggers)

What CPU-RM does when firmware has RC'd one channel. [MEASURED] throughout.

1. **Two events arrive**, typically back to back: `OS_ERROR_LOG` and `RC_TRIGGERED`.
   - `rpc_os_error_log` carries `exceptType`, `preemptiveRemovalPreviousXid`, `runlistId`, `chid` and a 256-byte
     `errString` formatted by firmware (`generated/g_rpc-structures.h:1622-1629`).
     `_kgspRpcOsErrorLog` looks the channel up and calls `nvErrorLog2_va` (`gpu/gsp/kernel_gsp.c:736-771`).
   - `rpc_rc_triggered` carries `nv2080EngineType`, `chid`, `gfid`, `exceptLevel`, `exceptType`, `scope`,
     `partitionAttributionId`, MMU fault address/type, `bCallbackNeeded` and an RC journal
     (`g_rpc-structures.h:1603-1618`).
2. **`_kgspRpcRCTriggered`** (`gpu/gsp/kernel_gsp.c:597-723`):
   - calls `krcCheckBusError_HAL` first (`:614`), which clears the PCIe device status and AER error bits
     (`gpu/rc/kernel_rc.c:477`). Every RC therefore clears host PCIe error state as a side effect.
   - resolves the channel from `(engine type, chid)` and fails with `NV_ERR_INVALID_CHANNEL` if there is none
     (`:628-641`). A stale or foreign `chid` cannot select another client's channel unless firmware says so.
   - stores firmware's RC journal records (`:645-701`).
   - writes the error notifier itself only when confidential computing is on (`:709-716`); otherwise "GSP has
     already written to notifiers" (`gpu/rc/kernel_rc_notification.c:85-89, 379-381`).
   - calls `krcErrorSendEventNotifications_HAL` (`:718`).
3. **`krcErrorSendEventNotifications_KERNEL`** (`kernel_rc_notification.c:439-469`) does two things:
   - wakes the ctxdma event list of the channel, or of every channel in the TSG for TSG scope
     (`krcErrorSendEventNotificationsCtxDma_FWCLIENT`, `:364-421`). Only ctxdma-based notifiers get an event; a
     memory-based `hObjectError` is written by firmware with no CPU-RM wake [INFERRED from `:414`, the only
     lookup is `ctxdmaGetByHandle`].
   - calls `gpuNotifySubDeviceEvent(NV2080_NOTIFIERS_RC_ERROR, info32 = exceptType)` (`:462-468`), the
     broadcast of section 6.
4. **The Xid line**: `krcReportXid` prints
   `NVRM: Xid (PCI:dddd:bb:dd): <exceptType>, pid=<pid>, name=<name>, <errString>` when `bLogEvents` is set
   (`gpu/rc/kernel_rc.c:312, 388-394`); `RmLogonRC` defaults to 1 on Linux
   (`src/nvidia/arch/nvalloc/unix/include/nv-reg.h:1019`). `pid` and `name` are those of the process that
   allocated the channel (`kernel_rc.c:334-348`), so for a kayfabe twin they name the VMM, not the guest
   process. The line also goes to a tracepoint and to a registered error callback
   (`kernel-open/nvidia/nv-report-err.c:60-80`). `osRCCallback` is a stub on Linux (`os/os_stubs.c:142-185`);
   there is no host-side RC policy callback.
5. **Rate limiting** exists only for the optional NvLog dump (1 s, `kernel_rc.c:598-609`). No RC counter,
   back-off or escalation threshold exists in CPU-RM; escalation on repeated RC is firmware's, unknown.

Cost to others [INFERRED]: step 2 to 4 run in the interrupt bottom half with the GPU lock held, so they
serialize with every other RM call on that GPU (alloc, free, control) for the duration. Per RC this is
short; a flood of RCs is a flood of lock holds plus Xid prints.

State left behind [MEASURED/INFERRED]: the channel stays isolated until its owner frees it or calls
`NV506F_CTRL_CMD_RESET_ISOLATED_CHANNEL` (flags `0x10008`, non-privileged; forwarded to physical RM at
`gpu/fifo/kernel_channel.c:2916-2946`, whose comment says "resets a channel which was isolated previously by RC
recovery", `src/common/sdk/nvidia/inc/ctrl/ctrl506f.h:55-75`).

## 3. The three triggers

### 3.1 (i) Invalid or unknown method, malformed push-buffer stream

Where Xid 32 comes from:

- **The code is `ROBUST_CHANNEL_PBDMA_ERROR` = 32** (`src/common/sdk/nvidia/inc/nverror.h:50`). The decision
  that a PBDMA condition is code 32 is taken in GSP firmware and arrives as `exceptType` in the two events of
  section 2. CPU-RM only prints it (`kernel_gsp.c:763-770`) and lists it among the codes that dump NvLog
  (`kernel_rc.c:660`).
- Catalog text for Xid 32 is "Invalid or corrupted push buffer stream" [EXTERNAL]. kayfabe's firmware strings say
  "PBDMA DEVICE" for the display-SW and stray-`0x200` cases (`docs/STATUS_AND_HANDOFF.md` line 98) [EVIDENCE].
- Related codes in the same family: 31 `FIFO_ERROR_MMU_ERR_FLT` (`nverror.h:49`), 80
  `PBDMA_PUSHBUFFER_CRC_MISMATCH` (`:86`), 43 `RESETCHANNEL_VERIF_ERROR` (`:59`), 44 `GR_FAULT_DURING_CTXSW`
  (`:60`), 45 `PREEMPTIVE_REMOVAL` (`:61`), 69 `GR_CLASS_ERROR` (`:75`).

What the stream can contain, from the class headers [MEASURED]: methods below `0x100` are host methods (SET_OBJECT
`0x0`, `SEM_EXECUTE` `0x6c`, `SET_REFERENCE` `0x50`, `MEM_OP_C` `0x30`; `src/common/sdk/nvidia/inc/class/clc36f.h:
130-241`); eight subchannels (`:67`); the header word has method, subchannel, count and an op field including
`GRP0_SET_SUB_DEV_MASK` (`:286-333`). Methods at or above `0x100` go to the object bound on the subchannel. An
undefined host method, a method on an unbound subchannel, a reserved op or a count that runs past the segment are
the malformed cases. Which of them the hardware reports as PBDMA `METHOD`, `DEVICE` or `PBSEG` is in
`dev_pbdma.h`, which is not published [INFERRED]. The oracle measured one engine-side member of the family:
a physical operand on a `USER` CE channel gives Xid 32 and a channel reset (`docs/OWNER_RULINGS.md` lines 913-921)
[EVIDENCE].

Teardown scope:

| Question | Answer |
|---|---|
| Torn down | The offending channel. The `rc_triggered` event names one `chid`; `scope` is CHANNEL or TSG. [MEASURED event shape; INFERRED that firmware picks them] |
| TSG siblings | Get the notifier and event when `scope == TSG` (`kernel_rc_notification.c:270-277`). The hardware halts the TSG's context on a PBDMA fault [INFERRED; channel RAM tracks the fault per channel, `dev_runlist.h:40-45`]. |
| Engine context | Unloaded with the TSG preempt. Whether it is also reset is firmware's; `RESET_CHANNEL` is documented as channel plus engine reset (`ctrl906f.h`). [INFERRED: the engine reset applies only to a channel that was on the engine.] |
| Runlist | A TSG preempt is a runlist-level operation in the hardware (`RUNLIST_PREEMPT` takes a TSG id, `dev_runlist.h:78-90`), so other TSGs on the same runlist are momentarily descheduled. [INFERRED] |
| GPU reset | Not for a channel-scope PBDMA error as far as the tree shows. Reached only through W4/W5 (section 4). |
| Widens when | Section 4. |

### 3.2 (ii) Software method: `NV50_DEFERRED_API` trigger `0x200`

The path in the tree [MEASURED]:

1. Registration. `NV5080_CTRL_CMD_DEFERRED_API_V2` (flags `0x10008`, non-privileged,
   `generated/g_deferred_api_nvoc.c:206-220`) reaches `defapiCtrlCmdDeferredApiV2_IMPL`
   (`gpu/deferred_api.c:316-398`). On a GSP client it does only two things: for `FIFO_UPDATE_CHANNEL_INFO` it
   registers the USERD memory with GSP (`:349-373`), then forwards the whole control as
   `NV5080_CTRL_CMD_DEFERRED_API_INTERNAL` (`:388-395`). CPU-RM never adds the entry to its own list in this path.
   The entry is created in firmware by `defapiCtrlCmdDeferredApiInternal_IMPL` (`:400-410`, flags `0x500c8`,
   internal), which stores `pCallContext->secInfo.privLevel` as the registrant's privilege (`:100`).
2. The inner command is **not** validated at registration (`:316-398` has no check; see also
   `docs/OWNER_RULINGS.md` lines 883-884).
3. Trigger. The PBDMA stalls on the method; the handler table is `{mthdNoOperation 0x100-0x103,
   _class5080DeferredApiV2 0x200-0x203}` (`deferred_api.c:676-679`), the stall is the default
   (`chandesIsSwMethodStalling` returns true, `channel_descendant.c:317-320`), with one exception:
   `FIFO_UPDATE_CHANNEL_INFO` is non-stalling and "clears the PBDMA interrupt before executing" (`deferred_api.c:
   694-715`). So a software method is a PBDMA interrupt serviced by firmware; stalling methods hold the PBDMA
   (and that channel) until the handler returns [INFERRED from the comment at `:710`].
4. Handler `_class5080DeferredApiV2` (`deferred_api.c:444-660`):
   - **handle not registered**: `_Class5080GetDeferredApiInfo` fails with `NV_ERR_INVALID_DATA`; the function
     skips the whole body and returns that status (`:459-461`, `:657-659`).
   - **unknown inner `cmd`** (including `DISABLE_CHANNELS`, which is in the union but not in the switch):
     `NV_ASSERT(0)` and `NV_ERR_INVALID_ARGUMENT` (`:545-551`).
   - **inner control refused for privilege**: the control runs through `serverControl_Prologue` with
     `rmCtrlParams.secInfo.privLevel = pCliDeferredApi->privLevel` (`:575`, `:619-640`). The flag words give:
     `DMA_INVALIDATE_TLB` `0x10008`, `GR_CTXSW_ZCULL_BIND` `0x90348`, `GR_CTXSW_PM_BIND` `0x10048`,
     `GR_CTXSW_PREEMPTION_BIND` `0x10248` are non-privileged; `GPU_INITIALIZE_CTX` `0x14244`, `GPU_PROMOTE_CTX`
     `0x10244`, `FIFO_UPDATE_CHANNEL_INFO` `0x4` are privileged; `GPU_EVICT_CTX` `0x1c240` is kernel-only
     (`generated/g_subdevice_nvoc.c`, lines 315-330, 5085, 5325-5415, 7575; flag meanings
     `inc/kernel/rmapi/control.h:167-205`; check `rmapi/control.c:673-708`). A refused control returns
     `NV_ERR_INSUFFICIENT_PERMISSIONS`.
   - **handler error in general**: the status of the epilogue is returned (`:655-659`); success runs
     `notifyMethodComplete`-style completion only if the object was allocated with `notifyCompletion`
     (`deferred_api.c:244-251`, `gpu/mem_mgr/method_notification.c:41-60`).
5. What the dispatcher does with a non-OK status is firmware. **[EVIDENCE]** one case is measured: an unregistered
   or unusable handle gives host Xid 32 on the firing channel only, the channel is RC'd, no MMU fault reaches
   nvidia-uvm (`docs/OWNER_RULINGS.md` line 885). Since the three failures above return a non-OK status by the
   same route, **[INFERRED]** all of them end in the same Xid 32 RC of the firing channel. Not measured.

Privilege caveat worth a measurement (M5), **[INFERRED]**: the registrant privilege is `secInfo.privLevel` of the
`..._INTERNAL` control as firmware sees it. The CPU-RM-to-GSP control message has no privilege field
(`generated/g_rpc-structures.h:1545-1557`), and CPU-RM forwards `DEFERRED_API_V2` with only the `NON_PRIVILEGED`
check on the outer command. If firmware gives RPC-origin controls kernel privilege, a registered `GPU_PROMOTE_CTX`
or `GPU_EVICT_CTX` would pass the privilege check at trigger and execute. If it gives user privilege, it fails and
is Xid 32. The tree cannot say which; `docs/OWNER_RULINGS.md` lines 883-884 ("the check runs at trigger with the
registrant's privilege") is true of `deferred_api.c` but the registrant's privilege on a GSP host is the open point.
This is a privilege question, not an RC-scope one, but it decides whether trigger (ii) can be more than a
self-RC.

Teardown scope: as 3.1. Additional point: a stalling software method holds a PBDMA while firmware services it.
A burst of triggers from one channel can therefore make firmware service time, not engine time, the shared
resource [INFERRED]. The probe branch noted "a channel appeared to take exactly one `0x200` trigger"
(`docs/STATUS_AND_HANDOFF.md` line 147), cause not established.

### 3.3 (iii) Invalid GP entry

GP entry layout [MEASURED, `class/clc36f.h:262-283`]: 8 bytes; word 0 `FETCH` and `GET` (low address), word 1
`GET_HI` (8 bits, 40-bit address), `PRIV` (user/kernel), `LEVEL`, `LENGTH` (21 bits), `SYNC`, and in the
non-fetch form an `OPCODE` of `NOP`, `ILLEGAL`, `GP_CRC` or `PB_CRC`. Ring setup is in
`gpu/fifo/arch/hopper/kernel_fifo_gh100.c:612-620` and `.../blackwell/kernel_fifo_gb100.c:244-259` (base alignment
and a power-of-two `LIMIT2`, asserted by CPU-RM).

Kinds of invalid entry and the reporting path [INFERRED, except as noted]:

- **Entry encoding is rejected by PBDMA** (opcode `ILLEGAL`, `PRIV_KERNEL` on a user channel, length/alignment
  violations, GP pointer outside the ring): PBDMA interrupt, firmware RC, Xid 32 (`GPENTRY`/`GPPTR` class of
  interrupt, names from memory [EXTERNAL]). Same scope as 3.1.
- **Entry is valid but points at an unmapped or forbidden VA**: the fetch faults in the channel's VA space. That
  is an MMU fault from the PBDMA client, reported as Xid 31 (`ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT`,
  `nverror.h:49`). CPU-RM does not service it on a GSP host; it sees `MMU_FAULT_QUEUED`
  (`kernel_gsp.c:1014-1022`, `osQueueMMUFaultHandler`). The in-tree CPU-RM fault code that RCs with scope TSG
  (`gpu/mmu/arch/volta/kern_gmmu_gv100.c:2038-2145`) is under `IS_VIRTUAL_WITH_SRIOV` (guard at line 2034), i.e. the vGPU guest path; the
  other branch calls `kgmmuServiceChannelMmuFault_HAL` (physical-RM code, not in the tree) [MEASURED guard,
  INFERRED that it does not apply here]. For kayfabe this is the common garbage case, because the
  twin's VA space maps only that VM's memory; a PBDMA fault is confined to that VA space by construction.
- **CRC opcodes** (`GP_CRC`, `PB_CRC`): mismatch is Xid 80 (`nverror.h:86`).

Teardown scope: as 3.1 for the encoding case; for the fault case, one faulted channel (`PBDMA_FAULTED`,
`dev_runlist.h:40-41`) and a TSG-wide error notifier. The MMU fault goes through the fault buffer that GSP owns;
a fault storm has its own buffer-overflow path in firmware [INFERRED, unseen].

## 4. When the scope widens

Each row: what CPU-RM or the headers show, and what remains firmware.

| # | Condition | What is shown | Reaches other VMs? |
|---|---|---|---|
| W1 | Offender shares a TSG | `scope == TSG` notifies all channels of the TSG (`kernel_rc_notification.c:270-277`, `:385-391`). The error notifier falls back to the TSG's own (`kernel_channel.c:503-509`). | Only the same RM client. Not cross-VM unless kayfabe shares a TSG across VMs. |
| W2 | Offender is the only TSG on the engine | Nothing in the tree. A lone TSG's preempt is trivially fast and there is nobody to disturb [INFERRED]. The opposite case (many VMs on one GR runlist) is the shared case. | Delay only. |
| W3 | Engine hang or failed preempt | Hardware has a ctxsw timeout, privileged config `NV2080_CTRL_CMD_FIFO_CONFIG_CTXSW_TIMEOUT` (flags `0x44`, `g_subdevice_nvoc.c:5018-5028`) and the notifier `CTXSW_TIMEOUT` (140). The Xid for it is not in `nverror.h`; it is 109 in the public catalog [EXTERNAL]. Recovery beyond that is firmware. | Yes if the recovery is an engine or runlist reset [INFERRED]. Unknown extent. |
| W4 | RM GPU-hang watchdog | See condition 3 in section 0. The watchdog channel is on `RM_ENGINE_TYPE_GR0` (`gpu/rc/kernel_rc_watchdog.c:1041`). Timeout 7 s and interval 7 s, restart window 4 s (`nvrm_registry.h:1590, 1595`, `kernel_rc_watchdog_callback.c:42`). Enabled by `ENABLE_WATCHDOG` from any non-privileged client (ref-counted; refused with `NV_ERR_STATE_IN_USE` while another client holds a DISABLE, `kernel_rc_watchdog.c:268-277`). Not supported in MIG (`kernel_rc_watchdog_ctrl.c:74-125`, init skip at `kernel_rc_watchdog.c:505`). | Yes: after 7 s of no watchdog notifier, `INTERNAL_RC_WATCHDOG_TIMEOUT` goes to GSP. Effect is firmware's. |
| W5 | Repeated RC from one channel | No counter in CPU-RM (section 2, step 5). Evidence of rate: 26-186 Xid 32 per run with the offender itself at 1.2 FPS [EVIDENCE]. | Unknown. Costs others the GPU lock and the log. |
| W6 | GSP crash or RPC timeout | `kgspCrashCat` handler marks `bFatalError`, then on the first fatal calls `krcRcAndNotifyAllChannels(GSP_ERROR, bSkipKernelChannels = TRUE)` and `gpuMarkDeviceForReset` (`kernel_gsp_tu102.c:1008-1018`). That function disables each user channel and preempts the whole runlist per channel: `kfifoStartChannelHalt_GA100` writes the channel RAM disable and then `RUNLIST_PREEMPT` with `_TYPE_RUNLIST` (`gpu/fifo/arch/ampere/kernel_fifo_ga100.c:877-912`; Blackwell `kernel_fifo_gb202.c:281-317`), then waits one default GPU timeout (`kernel_rc.c:733`). Also Xid 119 `GSP_RPC_TIMEOUT`, 120 `GSP_ERROR` (`nverror.h:114-115`). | Yes, all clients, GPU needs a reset. Trigger from a user channel is only through a firmware bug. |
| W7 | Firmware recovery action | `_kgspRpcGspEventRecoveryAction` can set GPU or node reboot required (`kernel_gsp.c:1695-1720`); raises Xid 154 and `NV2080_NOTIFIERS_GPU_RECOVERY_ACTION` (`gpu.c:7200-7262`). | Yes, if raised. |
| W8 | UVM-owned channel | Marks "reboot required" system-wide (`kernel_rc_notification.c:259-262`). Only a kernel-privileged channel can be UVM-owned (`kernel_channel.c:296-299`). | Not for a user channel. Do not give twins `UVM_OWNED`. |
| W9 | Fatal-level `exceptLevel` | Carried in `rc_triggered` (`g_rpc-structures.h:1608`) with levels INFO/NON_FATAL/FATAL (`nverror.h:297-299`). CPU-RM ignores it on the kernel side ("unused", `kernel_rc_notification.c:444-449`). | No effect in CPU-RM. |

The CPU-RM halt primitive in W6 is the only RM-written description of what "remove a channel now" means in
hardware: disable the channel in channel RAM, then preempt the **whole runlist**, then wait. It is for a dead
GSP; it says nothing about what firmware does for one PBDMA error, but it shows that a runlist-wide preempt is a
normal, shipped RM operation, not an exotic one [INFERRED relevance].

## 5. Other clients and VA spaces on the same engine or runlist

**Common case [INFERRED]:** other clients' correctness is untouched: their VA spaces, channel RAM entries,
USERD and engine contexts are not referenced by the offender's RC. Their latency takes one TSG preempt of the
offender (if it was resident) and one context-restore of whatever runs next; if firmware services the PBDMA
interrupt on the same thread that services everyone's PBDMA interrupts, they also wait for it. The tree gives no
duration. Bound from RM constants: nothing below the default GPU timeout is promised (`kernel_rc.c:733`).

**Worst case, by condition:** W3, W4, W6, W7 of section 4. The two reachable from an unprivileged channel without a
firmware bug are W3 (a hang that defeats preempt) and W4 (if the watchdog was enabled). Both depend on the
offender producing a hang, not merely a PBDMA error: a PBDMA error stops the channel; a software-method stall
(3.2) or an engine-side loop does not.

**Per-trigger summary:**

| Trigger | Expected RC | Others delayed | Others killed |
|---|---|---|---|
| (i) bad method / stream | Xid 32, this channel (TSG notified if scope TSG) | one TSG preempt [INFERRED] | no [INFERRED] |
| (ii) `0x200`, handle missing/inner fail | Xid 32 [EVIDENCE for the missing handle] | one TSG preempt plus the firmware service stall [INFERRED] | no [INFERRED] |
| (iii) bad GP entry, encoding | Xid 32 | as (i) | no [INFERRED] |
| (iii) bad GP entry, unmapped VA | Xid 31, this VA space | as (i) | no [INFERRED] |
| any of the above, repeated | as above, N times | N preempts plus N lock holds | unknown threshold |

## 6. What the host sees afterwards

[MEASURED] unless marked.

- **dmesg**: `NVRM: Xid (PCI:...): 32, pid=<VMM pid>, name=<VMM name>, <errString>` per RC (`kernel_rc.c:388-394`);
  optionally a second line with `caused by previous Xid <n>` for Xid 45 (`kernel_rc.c:370-381`). Xid 45 on a
  channel that did not fault is the marker for spread: it means "removed because of an earlier error"
  (`nverror.h:61`; the function that sets it, `kchannelNotifyRc_IMPL`, `kernel_channel.c:1648-1685`, is not called from anywhere in the tree).
  A flood is not rate-limited here.
- **Owner notifier**: `NvNotification` `info32 = exceptType`, `info16 = engine type`, `status = 0xffff`
  (`kernel_rc_notification.c:329-336` writes `0xffff`; `ctrl506f.h:59-68` states the info32/info16 meaning;
  `method_notification.c:120-155` for the layout), for the channel or the TSG, in the object named by
  `hObjectError` or the TSG's (`kernel_channel.c:503-509`).
- **Events**: ctxdma event wake for that channel or TSG; `RC_ERROR` subdevice event for every subscribed client
  (below).
- **Cross-client leak:** `gpuNotifySubDeviceEvent` walks every subdevice back-reference of the GPU
  (`gpu_rmapi.c:645-728`). Without MIG there is no filtering (`:438-440`), so each client that set an
  `RC_ERROR` event, including a different VM's twin client or a host monitor, receives the exception type of every
  channel's RC. Per-client `notifyActions` gates it (`:703`). kayfabe should subscribe `RC_ERROR` only on a host
  broker client, never on a twin that serves a VM, unless the leak of "some VM's channel hit code N" is accepted.
- **Subsequent submissions** on the RC'd channel are not executed until the channel is freed or reset. kayfabe's
  observed "guest sees device lost" is kayfabe's own emulation reacting, not RM behaviour.
- **Other state**: PCIe AER bits cleared (section 2 step 2); an RC journal record added to the system journal
  (`kernel_gsp.c:645-701`); `NV2080_CTRL_CMD_RC_GET_ERROR_COUNT` can read counts
  (`gpu/rc/kernel_rc_ctrl.c`, `subdeviceCtrlCmdRcGetErrorCount_IMPL`).

## 7. GSP-client versus non-GSP path

ogkm 595.84 ships only the CPU-RM half and requires GSP (`README.md:18`), so there is no non-GSP PBDMA/RC code to
compare against. What the tree does show about the split:

| Aspect | Non-GSP / vGPU / monolithic (code path kept in tree) | GSP client (this host) |
|---|---|---|
| Who services the PBDMA interrupt and picks the RC | the same kernel code as the physical half (not shipped) | GSP firmware (not shipped) |
| Who writes the error notifier | `krcErrorWriteNotifier_CPU` (`kernel_rc_notification.c:85-89, 92-210`) | firmware; CPU-RM writes only with CC on (`kernel_gsp.c:709-716`) |
| Who sends the events | `krcErrorSendEventNotifications_KERNEL` from the interrupt path | the same function, from the `RC_TRIGGERED` handler |
| Who prints the Xid | `nvErrorLog2` in the same process | firmware formats `errString`; CPU-RM prints (`kernel_gsp.c:736-771`, `os/os_init.c:522-570`) |
| RC callback policy | `krcErrorInvokeCallback` (`kernel_rc_callback.c:243-461`), only called from `vgpu_events.c:192` | not used on a GSP client in the tree; `osRCCallback_v2` is a Linux stub (`os_stubs.c:164-185`) |
| Channel reset after RC | `krcResetCallback` calls `RESET_ISOLATED_CHANNEL` (`kernel_rc_callback.c:132-240`) | owner calls it, or frees the channel |
| Critical-error sweep | not applicable | `krcRcAndNotifyAllChannels` (W6) |
| Hang watchdog | RM-owned channel on GR0 | same, recovery by `INTERNAL_RC_WATCHDOG_TIMEOUT` to GSP (`kernel_rc_watchdog_callback.c:360-376`) |

Consequence: on this host the containment guarantee is whatever the firmware build implements. It is a
closed, versioned component (kayfabe records 595.91.07 on the bench host; this analysis is of 595.84 source).
Any claim about spread needs a measurement on the exact firmware, which is why section 8 exists.

## 8. Measurements that would settle it on hardware

All are host-side, bounded, and use unprivileged twins. Each states its falsifier first
(`CLAUDE.md`, hypothesis rule). "B" is the victim VM or client, "A" the offender. Record for every run: kayfabe
revision, host driver and GSP firmware version, GPU, and the Xid lines with timestamps.

| ID | Hypothesis (H) and falsifier (F) | Method | Notes |
|---|---|---|---|
| M1 | H: an Xid-32 RC of A's channel does not change B's results or add an Xid to B. F: any wrong result, any Xid or notifier on B, or B channel gone. | Two VMs (or two host RM clients) with twin channels on the same GR runlist. B runs a continuous known-answer compute loop; A fires (i), (ii), (iii-encoding), (iii-unmapped) one at a time, then 1, 10, 100 per second. Control: A idle. | Primary test. Run the same-runlist case first, then different-runlist (CE vs GR) as the negative control. |
| M2 | H: B's worst-case inter-completion gap during A's RC stays within a stated bound (set from the A-idle p99.9 plus margin). F: gap exceeds the bound in any of N >= 1000 RCs. | Same rig as M1; B timestamps its own completions from the GPU (global timer semaphore release). Report the distribution of gap, not the mean. | Gives the number the source cannot: the preempt gap. Repeat with A resident (running a long kernel) and A idle before the RC. |
| M3 | H: B is unaffected when A's channel is the only one on a different engine. F: B gap increases. | M1 with A on CE0 and B on GR. | Separates runlist sharing from device-wide effects. |
| M4 | H: an RC in A's TSG notifies siblings; a different TSG of the same client is not notified. F: the reverse. | One client, two TSGs, three channels (A1, A2 in TSG1; C in TSG2). Fault A1. Read each error notifier and the Xid 45 lines. | Fixes W1 and the `scope` choice. Tells how many channels per TSG kayfabe may share within a VM. |
| M5 | H: a registered privileged inner control (`GPU_PROMOTE_CTX`, `GPU_INITIALIZE_CTX`, `GPU_EVICT_CTX`) fails at trigger on an unprivileged twin with Xid 32. F: the control executes (any state change, or a success path). | Existing falsify/oracle probe (`claude/deferred-falsify-20261007`, test-only) with each of the three; read back the effect through a harmless observable. | Security-relevant; decides whether (ii) is more than a self-RC. Needs owner approval for the branch (STATUS_AND_HANDOFF line 147). |
| M6 | H: with the RM watchdog enabled, A's GR hang (not a PBDMA error) makes B's channels be RC'd. F: B unaffected after 3 x the 7 s timeout. | Host client enables the watchdog (`NV2080_CTRL_CMD_RC_ENABLE_WATCHDOG`), A runs a never-ending GR kernel. Read `NV2080_CTRL_CMD_RC_GET_WATCHDOG_INFO` first to confirm the default is DISABLED, not RUNNING. | Do not run on the shared bench box without the box rules in `scripts/bench/box/README.md`. If the result is yes, the policy is: no verb that enables the watchdog, and a boot-time check that it is off. |
| M7 | H: the rate of RC does not cause a measurable stall in unrelated RM calls. F: B's `AllocObject`/`Control` latency rises by more than X during A's RC flood. | A fires RC at the maximum rate; B loops a cheap control and times it. | Quantifies the GPU-lock-in-bottom-half cost (section 2). |
| M8 | H: `RC_ERROR` subdevice events leak A's exception type to a subscribed B-side client. F: B's event does not fire. | A client with an `RC_ERROR` event on its subdevice, a different client causes the RC. | Expected to confirm; decides the subscription policy. |
| M9 | H (use only as a deterministic RC source for M1-M4): `NV906F_CTRL_CMD_RESET_CHANNEL` and `NV506F_CTRL_CMD_RESET_ISOLATED_CHANNEL` on one's own channel are non-privileged (flags `0x10008`) and produce a recoverable channel reset without the full PBDMA path. F: they need privilege on this host. | Host test client only, never guest-reachable. | Cheap deterministic RC without relying on garbage. Do not treat its result as the PBDMA path's. |

A two-VM rig exists only as a design here; the existing fast-suite and `scripts/bench/` lanes would be the place to
build it. Order: M9 (rig check), M1/M3, M2, M4, M8, then M7, then M5/M6 under the owner's approval.

## 9. What this means for kayfabe (recommendations, labeled as such)

- Treat the host channel's VA space and the unprivileged privilege as the confidentiality and integrity boundary.
  Source supports that an RC does not touch another client's memory or contexts [INFERRED]. It does not support a
  latency or availability guarantee between VMs until M1-M3 are run.
- One RM client and one TSG per VM at most; never share a TSG across VMs (W1, W8).
- Never forward `NV2080_CTRL_CMD_RC_ENABLE_WATCHDOG`, `..._DISABLE_WATCHDOG`, `FIFO_CONFIG_CTXSW_TIMEOUT` or any
  `NV2080_CTRL_CMD_*_RESET*` from a verb a guest can cause. Read the watchdog state at host start-up (M6 first
  step).
- Do not subscribe `RC_ERROR` or `GPU_RECOVERY_ACTION` on twin clients (section 6).
- Rate-limit at the source: a guest that produces repeated Xid 32 should be stopped by kayfabe (disable the VM's
  submission path) before the host sees N RCs per second. The source has no threshold of its own (W5). Natural
  place: count RC notifiers per twin client and cut the VM's doorbell.
- Treat any Xid 45, 109, 119, 120 or 154 in the host log during a multi-VM run as a stop condition for that run.

## 10. Not determined

- What GSP firmware does on a PBDMA error: which unit is reset, whether a runlist preempt is issued, how long the
  other TSGs on the runlist wait, and whether repeated RCs escalate. No ogkm source; M1-M3 and M7 only.
- Whether the registrant privilege inside firmware for `DEFERRED_API_INTERNAL` is user or kernel (M5).
- Which PBDMA interrupt bit each malformed-stream case sets (published headers carry no interrupt bits).
- The numeric `GPU_TIMEOUT_DEFAULT` on this host (not extracted; `gpu/gpu_timeout.c`).
- Whether the firmware build on the bench host (595.91.07 userspace) differs in RC behaviour from 595.84 source.
- Whether a non-MIG `RC_ERROR` broadcast is filtered anywhere beyond `notifyActions` (read once, `gpu_rmapi.c`).
- Why Windows showed "one GSP cycle" in the guest only: no ogkm mechanism found; the GSP-cycle wording is
  kayfabe's, and RM has no "cycle" for a channel RC.
