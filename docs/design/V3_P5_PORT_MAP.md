# V3 P5 PORT MAP — the guest kernel's channels, born as host channels, run on the host GPU

**STATUS: PROPOSAL, 2026-09-25 (w827), branch `v3-p5` off `v3` `5dd01b48`.** A survey (two
read-only agents + direct reads) of what `RmInitAdapter` drives after P4, the v3 crates at
`5dd01b48`, the old tree, the C reference traces (`nvidia-gpu-passthrough/traces/mode2_c_reference/`,
`cap1b_coldboot_hermetic_d6` decoded with `scripts/mode2_diag/rec_dump.py`) and ogkm
(`research_clones/ogkm-580.159.04`, the target driver). Format of `V3_P2_PORT_MAP.md` /
`V3_P4_PORT_MAP.md`. ⊘ "P5" here is what `THE_V3_PLAN.md` splits into P5 (channels + doorbell) and
P6 (Translated): the boot cannot pass `RmInitAdapter` without both, so they are one step.

**STATUS UPDATE, 2026-09-25 (w827, branch `v3-p5`, rev `1198d66d`): ✔ `RmInitAdapter` COMPLETES
on the correct planes; `/dev/nvidia0` opens and the raw client's first arms run.** Measured on the
GA106 bench (host 580.159.04), fast-guest boots `p5a`…`p5j`; `v3_gates.sh` 8/8 PASS at `1198d66d`.

```
kf3: chan 0xc1e00006:0x2 BORN Translated: token 0x1 -> host 0x10018 …   (PMA scrubber, chid 1)
kf3: chan 0xc1e00007:0x2 BORN Translated: token 0x2 -> host 0x10019 …   (global CeUtils, chid 2)
kf3: chan 0xc1e00007:0x2 BIND engine=0xb · GPFIFO_SCHEDULE enable=true (both channels)
chan[… tokens=[0x1:fwd=2,subs=2,… 0x2:fwd=2,subs=2,gp_get=Some(2)]] irq[… raised=1 …]
kf3: chan token 0x2 (host 0x10019) RETIRED, forwarded=2 serves=5 last_put=Some(2) gp_get=Some(2)
kf3: chan token 0x1 (host 0x10018) RETIRED, forwarded=18 serves=31 last_put=Some(18) gp_get=Some(18)
guest: ok R2 version … ok R9 host GPU VA … ok R10 isolate … ok R11 through-isolate
       FAIL R13.1/R13.2 channel, R16, R17 = Other(86)   ← user channels: §2.8 (Passthrough), not built
```

`memmgrTestCeUtils` passes (its memset + memcopy ran on the host's COPY2 through our ring, the
engine wrote the guest's finishPayload natively), the interrupt loopback passes (one MSI-X message via
the KVM irqfd), the PMA scrubber forwards 18 GP entries for the raw client's frees with no
`scrubberDestruct` timeout; `poisoned=0`, `contended=0`, no Xid. ⊘ No completion was forged and no
byte moved by the CPU (the CPU read method words from guest RAM and two 4-byte USERD cursors).

**STATUS UPDATE, 2026-09-25 (P5b, branch `v3-p5b`, rev `25fbdf1b`): ✔ §2.8 BUILT — guest USER
channels are Passthrough twins, and the raw client's R13–R17 pass in the guest.** Measured on the
GA106 bench, fast-guest boots `p5ba`/`p5bb` (default arms, `--probe-launch-dma`); `v3_gates.sh` 8/8
PASS at `25fbdf1b`:

```
ok R13.1 channel = 0xcafe000e, engine Ce, token 0x3 · ok R13.2 channel = 0xcafe0013, engine GrCompute, token 0x4
★ R15 SEM LANDED  = sem 0xbeef5ea1, GP_GET 1 -> caught GP_PUT 1        (the host engine, the guest's ring)
★ R17 CE COPY     = 4096 bytes: dst[0] 0x3f0011ff -> 0xc0ffee00 — read back through an INDEPENDENT mapping
★ R16 sandboxed doorbell = … rang channel 0xcafe000d token 0x3
kf3: DOORBELL-LEDGER tok=0x00000003 route=passthrough rung=1 emulated=0 forwarded=1 host=0x1a
kf3: act birth passthrough: … BORN Passthrough: token 0x3 -> host 0x1a … engine=0x9 (3076 us, off the GSP lock)
```

Five changes, each measured or cited:
1. **Births/objects/schedules/frees run on the plane's ACT thread** (`kf3-chan-act`); the drainer
   only queues and the FSM HOLDS the reply on a `kf_gsp::Deferred` until the act resolves its status
   (Q1 is now ANSWERED the owner's way: nothing blocks under the GSP lock; worst act 4.6 ms, measured).
2. **A refused `GSP_RM_ALLOC` was a SUCCESS in the guest.** `rpcRmApiAlloc_GSP` replaces the
   transport status with the params `status` (`rpc.c:11236-11241`), which our echoed body carried as
   0 — `[measured kf3m2]` every refused `NV01_MEMORY_VIRTUAL` "succeeded" and was later freed as
   `FreeUnknown`. Refusals now stamp it; `0x70` decodes (NoDeclaredFacts).
3. **User channels:** TSG/ctxshare facts give a member channel its VAS and engine; the twin is born
   over the guest's GPFIFO VA + USERD (store slice) in the mirror, on the guest's own engine (COPY0
   is a GRCE on GA106 — the guest's pushbuffer already routes for it, as on bare metal); engine
   objects follow the guest's allocs (its class, our params), schedule the guest's own
   `GPFIFO_SCHEDULE`; the token is `Route::Passthrough` (`RingHostInline`). Nothing parses it.
4. **§2.7 completion half:** one dataless non-stall host event per host engine (GR0 + every CE) in
   the workers' poller; a wake on an engine with a live guest twin latches the vector the served
   `intr_table` names (`authored::non_stall_vector_for`) and writes the irqfd. ⊘ `[measured p5ba]`
   ungated, CE3 woke 212× (= the walker's 212 walks) and CE2 16× (= our Translated rings): the
   notifier is GPU-wide, so an engine with no guest twin raises nothing.
   ⚠ Not yet exercised by a guest waiter (no default arm blocks on a non-stall event).
5. **Q6(a) done:** `kf_rm::chanlink::alloc_shape` maps every family's channel / CE / compute / 3D
   classes from `kf_chip`'s generated sets. Q6(b) (Hopper+/Blackwell BAR1 doorbell page) is still
   NOT built: `Device::bar0_write` never matches a doorbell for `DoorbellPlacement::Bar1`.
   Q2 (chid-unique token index): ogkm enables per-runlist channel RAM in a GSP-client guest only
   when SR-IOV is enabled or the `RmDebugOverridePerRunlistChannelRam` regkey is set
   (`kernel_fifo_init.c:141-221`); our device presents no SR-IOV, so chids are device-unique on
   every family — the one escape is that guest-root regkey (blast radius: the guest itself).

**P5b, later the same day (rev `afb294e8`, boots `p5bc`–`p5be`, suite `p5bt`):**
6. ⊘⊘ **§2.1's kernel test was keyed on the wrong handle.** A root alloc's wire `hObject` is `0`;
   P5 recorded pid-sentinel clients under it, so no client outside RM's internal range was ever
   kernel (the real cause of finding 2 below). Keyed on `hClient`, the sentinel marked
   nvidia-uvm's client — AND the raw client's sandboxed-isolate client `0xc1d0000c`, an
   unprivileged guest process. ⇒ The ROUTE's kernel test is now the internal-handle range only:
   a user channel on the Translated route would get its physical CE operands rewritten onto the
   store window (guest userspace → guest-kernel memory). UVM therefore stays Passthrough — see Q7.
7. A Translated segment that crosses from one of our 4 KiB rows into the next was refused "not
   placed by us" (`--concurrency`, PMA scrubber dead after 285 entries); the reader now walks rows.
8. **Suite `p5bt` (30 arms, 60 s): 24 PASS**, FAIL `--defer-liveness` / `--missing-page-fault`
   (a guest channel faults on its host twin and nothing reaches the guest's error notifier: the
   fault/RC plane is not built), TIMEOUT `--uvm-invalidate` / `--uvm-mean` (UVM channels, Q7),
   `--ce-client-guest-ram` (13 000 guest-RAM rows declared before any channel — the mapping plane),
   `--concurrency` (all rungs printed, scrubber forwarded 385 entries, no death — the arm's tail
   outlived 60 s).

**STATUS UPDATE, 2026-09-25 (P5c, branch `v3-p5c`).** Three fixes, each behind a measurement on
the GA106 bench (host 580.159.04); `v3_gates.sh` 8/8 PASS at `20cc4888`.
9. ★ **The fault/RC plane is built.** A passthrough twin is born with a host `NV01_CONTEXT_DMA` over
   the GUEST's own notifier record (`errorNotifierMem`, sysmem ⇒ the guest-RAM descriptor at its memfd
   offset, FB ⇒ the store) as its error context, so when the host RCs the twin the host's GSP writes
   the record into guest memory itself (`kernel_gsp.c:541-545`: GSP-RM writes notifiers, CPU-RM only
   sends events; the CPU writes nothing). One dataless `NV01_EVENT_OS_EVENT` per context DMA (notify
   index 0, the list `krcErrorSendEventNotificationsCtxDma_FWCLIENT` walks) on one RC fd a worker
   polls; a wake reads each armed 16-byte record and queues an event per record the HOST changed; the
   drainer posts `RC_TRIGGERED` (the guest's declared engine + chid, the host's `exceptType`, scope
   CHANNEL — our twin is its own host TSG) and raises the GSP stall vector outside the lock.
   `[measured f1/f2]` `--missing-page-fault` PASS (`fired=true status=0xffff except_type=0x1f`,
   bystander contained, `rc[armed=1 seen=1 posted=1]`); `--defer-liveness` PASS, `DEFER_LIVENESS=B`
   (bare metal: B too), `rc[armed=9 seen=4 posted=4]`.
10. **VA spaces are retired and recycled.** No guest VA-space free was ever carried: `--concurrency`
    (2400 alloc/free) built and LEAKED 2400 mirrors at ~54 ms each (space + 8 GiB store window +
    2 GiB RAM window, `[measured c3]`). `PageDirPolicy` now carries `MemStatement::Retire` for every VA
    space a free takes (its own, its device's, its client's; never a `GPU_DEVICE` reference's
    transient handle); the VA thread unmaps our rows and keeps the host space + windows as a spare
    (≤ 32). A space a live channel runs in is never recycled. `[measured d2/e2]` 6 mirrors built.
11. **The reconcile is O(n log n) and maps only gaps.** `plan_reconcile` scanned linearly per row
    (O(n²): `plan_avg_us` 3 108 at ~2 400 rows) and re-mapped a whole run whenever the walk coalesced one
    more page into it (`unmapped=5579` for `mapped=8611` on an add-only workload). It now searches, and
    completes a run by mapping its uncovered gaps only (kept rows already agree with it); the property
    test found and the rewrite closes a hole where a run backed only by a since-dropped kept row was
    left unmapped.
12. ⊘ **Harness: the verbose ioctl trace was running into a 115 200-baud UART** (~11.5 KB/s):
    `--concurrency` printed 422 KB (≈ 37 s of its 60 s), `--concurrent-fuzz` PASSED ring-buffered and
    TIMED OUT printed. The trace stays verbose, on a virtio console (`hvc0`); `KF_CONSOLE=serial`
    restores the old lane; early boot lands in `fast_<tag>_ttyS0.log`.

13. **Suite `p5cs1` at `c59732a7` (30 arms, 60 s): 27 PASS**, FAIL `--uvm-invalidate` /
    `--uvm-mean`, TIMEOUT `--ce-client-guest-ram`. ⊘ The two UVM arms moved TIMEOUT → FAIL *because
    of* the RC plane, and it answers Q7(b)'s unmeasured premise: nvidia-uvm's four CE channels on
    unprivileged twins are RC'd by the host with **Xid 32** (`except_type=0x20`) — the host refuses
    their physical-mode work — and the guest is now TOLD (`RC_TRIGGERED` ×4), so `UVM_REGISTER_GPU`
    fails `0x60` instead of spinning to the budget. Q7 still decides the route.

**Q8 (NEW, needs an owner decision). `--ce-client-guest-ram` cannot fit 60 s under "full walk + full
diff per invalidate".** It declares 13 000 rows in each of two spaces, one invalidate per map
(~26 000 invalidates). Every invalidate re-walks and re-diffs the WHOLE space: `[measured e3]` GPU walk
~0.53 µs/leaf (the report's emission, `kf_diff_kernel`, is one GPU thread) and host-side decode +
plan ~0.56 µs/leaf ⇒ ~14 ms per invalidate at 13 000 rows, O(rows²) over the arm. Separately, ~2 ms
per guest ioctl (alloc ~1.9 ms, map ~3 ms vs our arrive→clear ~1 ms early) is spent outside the VA
thread and is NOT yet attributed (guest-mode CPU samples are few; ~30 trapped BAR0 writes per row at
31-41 µs each explain ~1 ms). Bare metal: 9 s. Options: (a) an incremental reconcile — the walker
diffs against a GPU-resident copy of OUR ledger (our placements, not the guest's tables; is that the
"shadow" v3 §4.2 forbids?), or dirty-tracking of the guest's page-table pages; (b) a parallel
emission kernel (PTX regeneration + the CUDA suite) — halves the constant, does not change the order.

**Q7 (NEW, needs an owner decision). An unforgeable "guest kernel" identity for a channel.**
nvidia-uvm's CE channels are the guest kernel's and produce physical operands (§57), so they
belong on the Translated route — but the only wire fact naming them kernel (the `KERNEL_PID`
sentinel) is also declared by a guest user process (`[measured p5bd]`). Measured today with UVM
on Translated: its GPFIFO/pushbuffer are in VIDMEM (Q3, refused by name) and it would then need
the `MEM_OP` split (P6). Options: (a) key on RM's internal range + a UVM-specific statement that
userspace cannot make (e.g. the channel's VAS being the one `UVM_REGISTER_GPU`'s
`SET_PAGE_DIRECTORY` named); (b) keep UVM Passthrough and accept that its physical work faults
on the unprivileged host twin (⚠ that the host refuses PHYS-mode CE on an unprivileged channel is
standard RM behaviour but was NOT measured in this work).

**Five findings the boots made, each folded into the section it corrects:**
1. `hVASpace = 0` (the PMA scrubber) names the device-default VAS through a TRANSIENT handle: RM
   allocs `FERMI_VASPACE_A` (index `GPU_DEVICE`), publishes its PDEs, and FREES it before the channel
   alloc (`p5c`). The link keeps it until the DEVICE is freed (§2.1).
2. RM's internal clients are NOT marked kernel by the root alloc's pid sentinel (`p5a`); the guest
   RM's own `serverIsClientInternal` (handle base `0xC1E00000`) is (§2.1).
3. ⊘ §1.1's *"no guest interrupt is on `RmInitAdapter`'s path"* was **wrong**: `osVerifySystemEnvironment`
   runs an interrupt LOOPBACK (`os_sanity.c:119-291`) and fails `0x11:0x45` without it (`p5d`). §2.7's
   tree + irqfd were built in P5 (§1.1 row 15).
4. Host COPY0 on GA106 is a **GRCE** (on the GR runlist): RM's CeUtils pushes on subchannel 0, which
   there routes to GR ⇒ Xid 32 `CTXNOTVALID` (`p5f`). Rings now run on the first host copy engine the
   host's `CE_GET_CAPS_V2` says is not a GRCE (Q4 answered).
5. The PMA scrubber's USERD is mapped through **BAR1** (`bUseBar1`), which P4 left on scratch: 19
   doorbells read `GP_PUT = 0` (`p5h`). BAR1 is now walked from our BAR1 root like BAR2 (P4 row 6).

**The wall this closes.** `[measured p4b4-p4b6, kf3s2 at 5dd01b48]` the thin guest stops at
`_memmgrMemUtilsScrubInitScheduleChannel: Unable to schedule channel, status: 56` — `0xa06f0103`
(`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`) unserviced — then `RmInitNvDevice: *** Cannot load state into
the device`. ⊘ Serving that control `NV_OK` with no host act behind it is the fabricated completion
`kf-rm/src/sweep.rs` row `0xa06f_0103` refuses (measured in the old tree: the lie moves the wall one
statement and defers its cost to `memmgrTestCeUtils`'s CE wait). ⇒ The answer must BE the act.

**Summary.** v3 already has every engine-facing piece, proven on hardware by gates 3-6: the
Translated rewriter + runner (`kf-chan`), the session completion fd, the one worker loop, the token
plane (`kf-core`/`kf-trap`), the host channel verbs (`kf-host`). What is missing is the SEAT: the
served chain never tells anything that a channel exists, `kf-qemu` does not depend on `kf-chan`,
no worker thread runs, and a mirrored host VA space has no windows. P5 is **~0.9k lines of
product code**, almost all composition; **nothing is copied from the old tree but two pure
decoders** (the old tree ran kernel CE on the CPU — `kayfabe-rt/src/ceutils.rs`, `cpu_ce.rs` — the
executor v3 forbids).

---

## 1. Scope — what the guest drives

### 1.1 The two kernel CE channels of `RmInitAdapter` (ogkm-580, GSP-client guest)

`memmgrPostSchedulingEnableHandler` → `memmgrInitInternalChannels` (`mem_mgr.c:554`, `:480`) builds
**two** CeUtils channels, both through `ceutilsConstruct_IMPL` (`ce_utils.c:164-306`) →
`memmgrMemUtilsChannelInitialize_GM107` (`arch/maxwell/mem_utils_gm107.c:521`) →
`memmgrMemUtilsCopyEngineInitialize_GM107` (`:1085`):

| | channel 1 — PMA scrubber | channel 2 — global CeUtils |
|---|---|---|
| built by | `scrubberConstruct` (`mem_scrub.c:181`, `mem_mgr_scrub_gp100.c:63`) | `memmgrInitCeUtils(bVirtualMode=TRUE)` (`mem_mgr.c:526`, `:4155`) |
| client (cap1b) | `0xc1e00005` | `0xc1e00006` |
| VA space | `hVASpace = 0` ⇒ the **device-default** `FERMI_VASPACE_A` (hObject `0xc`, created lazily) | its **own** `FERMI_VASPACE_A` (`hVASpace = 0xa`), with an **FB identity alias** (`memmgrMemUtilsCreateMemoryAlias_HAL`, `mem_utils_gm107.c:595-760`) |
| CE operands | PHYSICAL (`bUseVasForCeCopy = false`, `mem_utils.c:1952`) | VIRTUAL through the alias (`channel_utils.c:1053-1056`, `:1084-1087`) |
| `BIND` (`0xa06f0104`) | not sent | sent before the schedule (`mem_utils.c:1952-1970`) |
| doorbells during init | **none** (it submits only when memory is freed) | **two**, token `0x10002` (runlist 1, chid 2), cap1b #288085 / #288132 — `memmgrTestCeUtils`' memset + memcopy, each a blocking wait (`mem_mgr.c:408-470`) |

The RPC sequence GSP sees, per channel (`cap1b`, fn 103 = `GSP_RM_ALLOC`, fn 76 = `GSP_RM_CONTROL`):

| # | RPC | ogkm | what we must do |
|---|---|---|---|
| 1 | fn103 `NV01_ROOT` (kernel pid sentinel), `NV01_DEVICE_0`, `NV20_SUBDEVICE_0` | `channel_utils.c:103-200` | nothing new (object seat). ★ Remember the client is **kernel** (§7's failure policy) |
| 2 | fn103 `FERMI_VASPACE_A`; fn76 `0x90f10106` `COPY_SERVER_RESERVED_PDES` (the PDB, vidmem) | `mem_utils_gm107.c:600-660` | **P4, built**: the page-dir statement makes a mirrored host VA space (`kf-qemu/src/mem.rs` `apply_statement`), reply held until walked. ★ **New:** map the identity window + guest-RAM window into it (§12: every host VAS) |
| 3 | fn76 `0x20802a08` `CE_GET_FAULT_METHOD_BUFFER_SIZE` | TSG construct | served today (value used; real GA106 = 20480) |
| 4 | fn103 **`AMPERE_CHANNEL_GPFIFO_A` (`0xc56f`)**, parent = the DEVICE, 368-byte `NV_CHANNEL_ALLOC_PARAMS`: `gpFifoOffset` = pbGpuVA + `0x64000`, `gpFifoEntries` = 4096, `flags` = `0x00a00120` (`USERD_INDEX` = the guest's chid), `hUserdMemory[0]` = 9, `engineType` = `0xb` (COPY2), `userdMem` = FB `0x2_eb6e0000`+`0x200`, `hPhysChannelGroup` = `0xd` | `_memUtilsAllocateChannel` (`mem_utils_gm107.c:1203-1300`) → `_kchannelSendChannelAllocRpc` (`kernel_channel.c:938`, `:2707-2830`) | ★ **BIRTH the host twin HERE** (§7: at allocation, never lazily) — §2.2. ⊘ **No `0xa06c` RPC**: the TSG is allocated `RMAPI_ALLOC_FLAGS_SKIP_RPC` (`kernel_channel.c:381-393`); the GSP makes its own group, and so do we (`HostRm::birth_channel`) |
| 5 | fn103 `AMPERE_DMA_COPY_B` (`0xc7b5`) on the channel, `{version 1, engineType 0xb}` | `mem_utils_gm107.c:1117-1150` | nothing new: our host ring owns its own CE object; the guest's object is a graph node |
| 6 | (channel 2 only) fn76 `0xa06f0104` `BIND` | `mem_utils.c:1952-1970` | served today (echo; the device advertised the engine) |
| 7 | fn76 **`0xa06f0103` `GPFIFO_SCHEDULE`** `{bEnable=1, bSkipSubmit=0, bSkipEnable=0}` | `mem_utils.c:1973-1987` → `kchannelCtrlCmdGpFifoSchedule_IMPL` (`kernel_channel.c:3086-3125`) | ★ answer `NV_OK` + the `[IN]` echo **iff the plane owns a born, scheduled twin** — §2.2. Any non-OK is fatal |
| 8 | fn103 `NV01_EVENT_KERNEL_CALLBACK_EX` (`0x7e`) on the subdevice, index `FIFO_EVENT_MTHD \| NONSTALL` (`0x23`); fn76 `0x20800301` `EVENT_SET_NOTIFICATION {0x23, REPEAT}` | `mem_utils.c:1895-1930` | served today (per-subdevice arming). ⚠ Used only when completion callbacks are enabled — never on this path |
| 9 | `0xc36f0108` `GET_WORK_SUBMIT_TOKEN` | `kernel_channel.c:3286-3348`, `kernel_fifo_ga100.c:178` | ⊘ **Not an RPC for these channels on a GSP client** (agent decode of cap1b): the guest computes `RUNLIST_ID << 16 \| ChID` itself and stores it in its error notifier. The old tree's docs say the internal variant (flags `0x10244`) *can* route to us; the link answers it with our table index if it ever arrives (§2.1) |

Then, channel 2, `memmgrTestCeUtils` (the first thing that WAITS):

| # | guest action | ogkm | where it lands |
|---|---|---|---|
| 10 | writes a GP entry (`GP_ENTRY0 = VA[31:2]`, `GP_ENTRY1 = GET_HI \| LENGTH \| MAIN`) into its GPFIFO and `GP_PUT` into USERD, flushes | `channelFillGpFifo` (`channel_utils.c:453-580`), `mem_utils_gm107.c:1880-1915` | GPFIFO + pushbuffer: ONE `NV01_MEMORY_SYSTEM` buffer, **sysmem** (`mem_utils_gm107.c:245-252`, `:779-791` default `INST_LOC_4_CHANNEL_PUSHBUFFER`) — guest RAM. USERD: **vidmem**, reserved heap (`:1152-1200`) — the store. `GP_PUT` at USERD+`0x8C`, `GP_GET` at +`0x88` |
| 11 | rings: `GPU_VREG_WR32(NV_VIRTUAL_FUNCTION_DOORBELL, token)` | `kfifoUpdateUsermodeDoorbell_GA100` (`kernel_fifo_ga100.c:153-163`); `NV_VIRTUAL_FUNCTION_DOORBELL = 0x30090` (`ga100/dev_vm.h:131`) | BAR0 `0xBB0090` — `kf-trap` `Class::Doorbell` (`kf-qemu/src/device.rs` `bar0_write`) |
| 12 | the pushbuffer (cap1b #288086): `SET_OBJECT 0xc7b5`, `SET_REMAP_CONST_A/COMPONENTS`, `SET_DST_PHYS_MODE`, `OFFSET_OUT_UPPER/LOWER`, `LINE_LENGTH_IN`, `SET_SEMAPHORE_A/B/PAYLOAD` = (pbGpuVA+`0x6c004`, n), `LAUNCH_DMA 0x0400058e` (non-pipelined, FLUSH, RELEASE_ONE_WORD, REMAP, pitch, DISABLE_PLC), then host `SEMAPHOREA-D` = (pbGpuVA+`0x6c000`, putIndex, RELEASE\|4BYTE) | `channelFillCePb` (`channel_utils.c:781-835`) | ★ the gate-3 shape: rewritten (PHYSICAL ⇒ identity window) or verbatim (VIRTUAL), run on OUR host ring |
| 13 | **polls** the finishPayload word (sysmem, pbGpuVA+`0x6c004`) until it reaches the payload; free slots by the host semaphore (+`0x6c000`), never `GP_GET` | `channelWaitForFinishPayload` (`channel_utils.c:344-383`), `channelWaitForFreeEntry` (`:391-440`) | ★ **written by the ENGINE**, through the mirror's sysmem row — native, never ours (§8) |
| 14 | while polling, if it holds the GPU lock: `channelServiceScrubberInterrupts` → `intrServiceStallList(CE, ESCHED/FIFO)` | `channel_utils.c:355-370` | reads interrupt registers from the BAR0 shadow; nothing pending is a correct answer |

⊘⊘ **CORRECTED `[measured p5d]` — the next paragraph was wrong.** No *completion* interrupt is on
the path (completion callbacks are off; step 13 polls), but **an interrupt is**: after the channels,
`osVerifySystemEnvironment` → `_osVerifyInterrupts` (`os_sanity.c:119-291`, `osinit.c:2127`) clears
and enables vector 129, writes it to `CPU_INTR_LEAF_TRIGGER` and spins ~4 s for its own ISR, which
checks the LEAF pending bit; nothing arriving is `RmInitAdapter failed! (0x11:0x45:2134)`
(`NV_ERR_IRQ_NOT_FIRING`). ⇒ §2.7 is ON the path — row 15:

| # | guest action | ogkm | where it lands |
|---|---|---|---|
| 15 | interrupt loopback: `LEAF(4)` W1C bit 1, `LEAF_EN_SET(4)`, `TOP_EN_SET(0)` bit 2, `LEAF_TRIGGER = 129`; ISR reads `LEAF(4)` | `intr_swintr_tu102.c:40-90`, `intr_tu102.c:648-744` | ★ the CPU interrupt tree, applied on the vCPU (`kf-trap/src/cpuintr.rs`), one MSI-X message via a KVM irqfd (§2.7) — `[measured p5g]` `irq[raised=1]`, loopback passes |

~~⇒ **No guest interrupt is on `RmInitAdapter`'s path** (completion callbacks are off; step 13 polls).
The interrupt plane (§2.7) is still P5's, because the first user channel (`cuCtxCreate`) and
libcuda's blocking sync need it — but it is not the wall.~~

### 1.2 The unserviced controls seen at `5dd01b48`, and which are on the path

(ogkm names and callers from the survey; `sweep.rs` rows where they exist.)

| cmd | ogkm | reply needed? | on the scrub path? | disposition for P5 |
|---|---|---|---|---|
| `0xa06f0103` | `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` | status only; **non-OK is fatal** | ★ **the wall** | served by the channel plane (§2.1-2.2) |
| `0x20802a0f` | `INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE` | values (PCE→LCE map) | asserts at `kernel_ce.c:1020` for `LCE_TYPE_DECOMP`, **boot continues** (measured) | next if a later step depends on it; derive from the host's `CE_GET_CAPS_V2`/PCE masks per family |
| `0x20800afe` / `0x20800aff` | `INTERNAL_INIT_USER_SHARED_DATA` / `…_SET_DATA_POLL` | status only | assert, **boot continues** (measured) | later (user shared data = a GSP-side poller; needs its own design) |
| `0x20800a4b` | `INTERNAL_DISPLAY_GET_IP_VERSION` | **value** selects the display HAL | no | ⊘ `AmputationIntended` (no display plane) |
| `0x2080017e` | `GPU_GET_VMMU_SEGMENT_SIZE` | value; `0x56` = "no VMMU" (documented) | no | ⊘ refusal is the documented answer |
| `0x20800a87` | `INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO` | `0x56` = no NVLink | no | ⊘ a real GA106 answers `0x56` too |
| `0x20800a70` | `INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR` | status | no | later — an `NV_OK` claims a flush; needs the posted-write argument (`V3_P4_PORT_MAP.md` Q7) |
| `0x20800a80` | `INTERNAL_PERF_GPU_BOOST_SYNC_GET_INFO` | zeros harmless | no | later (hardware answers 16 zero bytes) |
| `0x20800a30/2c/2e/34`, `0x20800b03/b05`, `0x20800a3f` | `INTERNAL_STATIC_KGR_*` | stored into GR static info | no (P7: GR) | P7, host-derived per family |
| `0x20800a38` | `INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE` | `bEnable`; asserts | teardown only | P7 |

⇒ P5 serves **one** of the sixteen, `0xa06f0103` — the only one `RmInitAdapter` cannot pass.
The rest are served when a boot shows one of them is the next wall, host-derived or authored per
family with provenance (never a GA106 row); §3 step 6 is where that is checked.

---

## 2. The pieces

Legend for the old-tree column: **COPY** / **ADAPT** / ✘ (does not fit, with the reason).

### 2.1 The channel link in the served chain (`kf-rm/src/chanlink.rs`, new)

- **Design:** `THE_ARCHITECTURE_v3.md` §7 (birth at allocation), `kf-core/src/channel.rs` `Birth::
  AtAllocation`; `sweep.rs` rows `0xa06f_0103`/`0xc36f_0108`/`0xa06f_0104` (*"ONE requirement
  asked four times: put a channel on a runlist, arm its completion, hand back its doorbell"*).
- **v3 today:** the object seat (`rmrpc/policy.rs` `RmObjects::apply`) records the channel in the
  graph and nothing else; `0xa06f0103` falls to `UnservicedLedger` (`0x56`).
- **Old tree:** `kayfabe-rmrpc/src/policy.rs:3026` `respond_gpfifo_schedule` (the 3-byte `[IN]`
  shape, echo) — **ADAPT** the reply shape only; ✘ its `ObjectModel`/`ExecPlane` (declared-vs-
  performed bookkeeping on `Proc`, with the host act deferred to the doorbell: v3 does the act at
  the statement). `kayfabe-chips/src/ga10x.rs:419` `decode_userd_index_chid` — **COPY** (pure, 12
  lines).
- **New:** a `CommandPolicy` seated FIRST (ahead of the object seat, which terminates allocs/frees,
  and the ledger). It carries `ChanStatement::{Alloc, Schedule, Token, Free}` to a `ChanSink`
  (`Arc<dyn Fn(ChanStatement) -> ChanAnswer>`, called on the drainer) and answers with what the
  plane DID: an alloc the plane refused is refused (status + name); `GPFIFO_SCHEDULE` answers
  `NV_OK` + echo only when the plane owns and scheduled the twin; a channel the plane does not own
  declines (the chain answers as before). It resolves `hVASpace = 0` to the first
  `FERMI_VASPACE_A` under the channel's parent device (the device-default VAS, channel 1's), learns
  kernel clients from the root alloc's pid sentinel, and decodes the guest chid off `flags`.
- **≈ 300 lines.**

### 2.2 The channel plane in the device (`kf-qemu/src/chan.rs`, new)

- **Design:** `THE_TRANSLATED_PLANE.md` §24.2 (the Translated channel is NOT a decoder), §25-§26
  (gates 3-4), §7 (completions); `THE_ARCHITECTURE_v3.md` §5 (Translated: nothing is owed).
- **v3 today:** `kf-chan::host::{HostRing, TranslatedChannel}`, `ring::TranslatedRing`,
  `completions::Completions`, `worker::run`; `kf-core::Plane::{allocate_channel, ring_internal,
  worker_pass}`; `HostOps for Device` returns `run_translated = false`, `operands_translatable = No`.
- **Old tree:** `kayfabe-rt/src/device.rs:670-2107` birth latch → ✘ (it births over the store
  through the join and a bounded queue on the isolate); `kayfabe-rt/src/ceutils.rs`, `cpu_ce.rs` →
  ✘ **the CPU executor** (§46). Nothing to copy; gate 4 (`kf-harness/src/bin/kf-gate4.rs`) is the
  template.
- **New:**
  - **Birth (drainer, at the alloc statement):** kernel client + copy-engine `engineType` ⇒
    Translated (anything else is not ours yet — user channels are Passthrough, §2.8). Look up the
    mirror of `(client, vaspace)`; require the guest chid; arm the USERD view (§2.4); build a
    `HostRing` in the mirror's host space (host RM: memory, map, TSG, channel, BIND, token, CE
    object, schedule); `TranslatedChannel::new(TranslatedRing::new(gpFifoOffset, entries, 0), host,
    chid)`; `Plane::allocate_channel(chid, Route::Translated, host_token, Owner::Kernel)`.
    ⊘ Our host ring's CE and channel are OURS: `HostRing` authors every host flag (COPY0 today; the
    guest's COPY2 is a guest-visible fact, not a host instruction).
  - **Schedule:** records `scheduled` on the slot; a pump on an unscheduled channel does nothing
    (hardware does not fetch an unscheduled channel); work already rung is picked up by one
    `ring_internal`.
  - **Serve (worker, `HostOps::run_translated`):** `TranslatedChannel::pump` with the §2.3 reader,
    the §2.4 USERD, the §2.5 windows, and a `Publisher` that refuses a `MEM_OP` split by name (RM's
    CeUtils never invalidates in its pushbuffer; the split through the VA thread is UVM's, P6).
    A pump error kills the channel by name; the plane then refuses-and-poisons (kernel, §7).
  - **Free:** retire the token (`Plane::free_channel` waits out BUSY), free the twin, release the
    USERD view.
- ★ `[measured p5b-p5d]` two corrections to the resolution, now built: the kernel test is the guest
  RM's own `serverIsClientInternal` (handle base `0xC1E00000`, `rs_server.c:2618-2623` — a user
  client's fixed handle is re-encoded onto `0xC1D00000`, `:3267-3271`) OR the pid sentinel; and the
  device-default VAS outlives its transient handle's free.
- ★ `[measured p5f]` the host engine is AUTHORED as the first host copy engine that `CE_GET_CAPS_V2`
  says is not a GRCE — never the guest's `engineType`, and never COPY0 by default (a GRCE routes
  subchannels 0-3 to GR: Xid 32 `CTXNOTVALID`). The session completion fd also listens on that
  engine's non-stall notifier.
- ⊘ **Token index = the doorbell's `VECTOR` (11:0) = the guest's chid.** Every family's generator
  puts the chid there (`kfifoGenerateWorkSubmitTokenHal_{TU102,GA100,GB100,GB202}`); the RUNLIST_ID
  and the Blackwell flag bits (GB202 bit 30, GB100 bits 22/31) are masked by the trap. Sound while
  chids are device-unique (`kf-arch/src/lib.rs:203-212`: one global `CHID_MGR`, measured on GA106).
  See Q2 for per-runlist channel RAM.
- **≈ 450 lines.**

### 2.3 Reading the guest's GPFIFO and pushbuffer — through OUR placements

- **Design:** `THE_TRANSLATED_PLANE.md` §24.2 (*"the segment's VA is translated through our ledger
  (VA → store offset)"*); `THE_CONSTRAINTS.md` §56 rule 2 (no stored guest tables).
- **v3 today:** `kf_mem::ledger::Ledger::resolve` — but the ledger lives inside `VaManager` on the
  VA thread, and workers cannot reach it.
- **New:** the mirror's `MapTarget` (`kf-qemu/src/mem.rs` `GpuMirror`) records each placement
  `va → (len, offset, ram)` **as it makes it** — before the invalidate's `TRIGGER` is cleared — in a
  shared `PlacedRows`; workers resolve through it (a read lock, never held across I/O). ⊘ It is our
  own map calls, not the guest's tables (Q1 of `V3_P4_PORT_MAP.md`, ratified shape). A sysmem row is
  read from guest RAM by memfd offset (`RamMap::at_file_offset`); a **vidmem** row is refused by
  name (Q3).
- ★ **Why inside the apply:** the guest rings only after its invalidate completes; publishing the
  rows after `on_walk_ready` returned would leave a window where the doorbell resolves nothing.
- **≈ 80 lines.**

### 2.4 USERD — two words, through a CPU view we arm

- **Design:** `kf-core/src/channel.rs` `CPU_MOVE_MAX_BYTES = 8` (a register-sized access is ours,
  anything wider is the engine's).
- **New:** at birth, `userdMem` (the guest kernel's RESOLVED descriptor, `kf_arch::UserdMem`):
  `Framebuffer{base}` ⇒ `arm_cpu_view(store, page(base), 4 KiB)` + an uncached `VolatileRegion`;
  `Sysmem{base}` ⇒ the guest-RAM block. `GP_PUT` is read (+`0x8C`), `GP_GET` authored on completion
  (+`0x88`). ⊘ The guest never reads `GP_GET` on this path (it polls semaphores, §1.1 step 13), and
  a Translated channel's `GP_GET` is one word we author by construction (§7).
- **≈ 50 lines.**

### 2.5 The windows in every mirror

- **Design:** `THE_TRANSLATED_PLANE.md` §12 (*"it goes in EVERY host VAS we create, above the
  guest's range, from the first commit"*), §15 (whole-object map, 1 call), §16-§17 (the RAM window).
- **v3 today:** a mirror is `HostVas{space, store, ram_obj}` — FIXED slice maps only; no window, so
  a PHYSICAL operand has nowhere to go.
- **New:** at mirror creation (VA thread), `map_window(space, store, fb_len, GROWS_DOWN)` and
  `map_window(space, guest_ram_object, len, GROWS_DOWN)`; the bases are READ BACK and recorded per
  mirror (§15.4's per-VAS caveat, answered by construction). A sysmem operand is resolved through
  the VMM's own layout (`RamMap::file_range`: guest-physical → memfd offset), never assumed flat.
- **≈ 60 lines.**

### 2.6 Workers and the completion edge

- **Design:** `THE_ARCHITECTURE_v3.md` §1 (N workers, one wake word), `THE_TRANSLATED_PLANE.md`
  §10.1 (a completion is one more epoll entry), `kf-chan/src/completions.rs` (one session fd; ring
  only the in-flight tokens).
- **v3 today:** the device signals `worker_efd` on `Action::WakeWorker` — and **no thread reads it**.
- **New:** `ChanPlane` opens the session `Completions` at realize (`FIFO_EVENT_MTHD`, dataless,
  non-stall, REPEAT); `kf3_realize` spawns 2 `kf3-worker` threads running `kf_chan::worker::run`
  over the device's plane and `HostOps`. ⊘ Workers make no CUDA call and hold no lock across a host
  call except the slot's own (uncontended by construction: BUSY excludes; counted).
- **≈ 40 lines.**

### 2.7 The interrupt plane — host NSI → guest MSI-X

★ **BUILT (`1af20762`, `663e35bc`), because the loopback is on the path (§1.1 row 15):**
`kf-trap/src/cpuintr.rs` — the tree as atomic words (W1C leaves, SET/CLEAR aliases, TOP derived,
per-family leaf count 8/16), applied in the BAR0 write trap and published to the read shadow before
any message; delivery follows the enables. `kf3.c` — Rust owns one eventfd per MSI-X vector
(`kf3_irq_fd`); the C device wraps each in an `EventNotifier` and registers it as a KVM irqfd on the
vector's MSI route in the MSI-X vector-use notifier (virtio-pci's pattern), removing it on release; no
`kvm_msi_via_irqfd_enabled()` refuses realize. `KF3_ABI` 4. ⊘ Still to build: the COMPLETION half
(a worker latching an engine's authored non-stall vector on a session-fd wake — `Device::
latch_and_deliver` exists, nothing calls it yet) and the GSP stall vector `0x9b`.

- **Design:** `THE_ARCHITECTURE_v3.md` §5 (registration is per engine; the waiter owns the race;
  once armed, a later release MUST interrupt), `THE_TRANSLATED_PLANE.md` §7 (fd → KVM irqfd →
  guest MSI, no thread in between), `the_interrupt_arming_model.md` (fire what the guest ARMED).
- **v3 today:** `kf3.c` reserves 32 MSI-X vectors on BAR5 and raises none; `kf-core::EngineIrq`
  (arm/retire/unmask) is tested, unused; the served interrupt table is authored
  (`kf-rm/src/authored.rs`: GSP stall `0x9b`, DISP `0x9a`; engine non-stall rows GR0 → 0, each
  async CE → its runlist number).
- **Old tree:** `kayfabe-device/src/cpuintr.rs:305` `CpuIntrTree` (leaf/top pending + enable at
  `0xB8_0000+`, W1C, `latch(vector)`) — **ADAPT** (~250: the register decode into kf-trap shadow
  cells, write-1-to-clear semantics); `nonstall.rs:116` `non_stall_vector` — ✘ (its join ran
  through the captured GA106 table; v3's vector is the authored row, looked up by the engine's
  runlist); `nvkvm.c:537` `nvkvm_deliver_vector` — ✘ (`msix_notify` under the BQL).
- **New (the shape):** at realize, one eventfd per MSI-X vector the guest can see (vector 0 in
  practice: RM demultiplexes by reading the TOP/LEAF registers) registered as a **KVM irqfd** by the
  C device (`kvm_irqchip_add_irqfd_notifier_gsi` on the vector's route) — so a Rust thread raises
  a guest interrupt with one `write(eventfd)` and no BQL. The completion that triggers it: the
  worker, on a session-fd wake that retired guest work on an ARMED engine, latches that engine's
  authored non-stall vector into the leaf-pending shadow (`EngineIrq::retire` → `Deliver`), then
  writes the irqfd — **after** the engine's semaphore write is visible (§5's ordering: payload
  first). The GSP message-queue interrupt (stall `0x9b`) uses the same path from the drainer.
- **≈ 400 lines** (C ~60, Rust ~340). ⊘ Not on `RmInitAdapter`'s path (§1.1); on `cuCtxCreate`'s.

### 2.8 Passthrough user channels (next, after the scrub)

`kf-chan/src/passthrough.rs::birth` (gates 5/6) at the user channel's alloc (TSG parent: its VAS
from the `0xa06c` facts), USERD adopted AT CREATION (`rm_takes_a_guest_userd_and_zeroes_it`),
`TokenWord::allocate(Route::Passthrough, host_token)` so the trap rings inline. The link already
carries every channel alloc; the plane today declines non-kernel / non-CE channels by name.

### 2.9 Totals

| crate | new | copied/adapted |
|---|---|---|
| kf-rm (chanlink) | 280 | 20 |
| kf-qemu (chan plane, mirrors' rows + windows, workers, heartbeat) | 650 | — |
| kf-core (allocate through the shared plane) | 20 | — |
| **P5 product (scrub)** | **≈ 950** | **≈ 20** |
| interrupt plane (§2.7) | ≈ 150 + C 60 | ≈ 250 |

---

## 3. Build order, with a hardware check per step

Every step runs on the GA106 box and records its revision; **all eight `v3_gates.sh` gates must keep
passing** after every kf-chan / kf-host / kf-core / kf-mem change.

| # | build | hardware check |
|---|---|---|
| **1** | §2.1 link + §2.2 statements, answering `NotOurs` for everything (seat only) | fast-guest boot: **unchanged** wall and unserviced list (the seat adds no answer) — plus the log names every channel alloc the guest makes (`class`, `engine`, `kernel`, `vaspace`, `chid`, `userdMem`). ⊘ Checks the decode against cap1b's numbers (flags `0x00a00120` ⇒ chid 1, `engineType 0xb`, `gpFifoOffset` = pbGpuVA + `0x64000`) |
| **2** | §2.5 windows in every mirror | boot log: `mirror space=… windows fb=…+<fb_len> ram=…+<ram_len>` for each page-dir statement; the P4 counters unchanged (`named_missed`, `unreconciled`) |
| **3** | §2.2 birth + §2.3 rows + §2.4 USERD + §2.6 workers | boot: `BORN Translated` for BOTH kernel channels, `GPFIFO_SCHEDULE enable=true` for both; `0xa06f0103` **leaves the unserviced list**; the NVRM line `Unable to schedule channel` is GONE |
| **4** | (the scrub runs) | ★ the gate: channel 2's token **`forwarded ≥ 2`** in the heartbeat (`chan[… tokens=[0x2:fwd=N,…]]`), `poisoned=0`, `contended=0`, no `DEAD`; `memmgrTestCeUtils` passes (no `NV_ERR_TIMEOUT` at `mem_mgr.c:463`, no `memmgrTestCeUtils @ :4159`) — ⊘ `RmInitAdapter` completing **without** `forwarded>0` is the w797 lie and FAILS this step |
| **5** | `/dev/nvidia0` opens; the raw client's first arms | `KF_ARMS` default set on the fast guest; `nvidia-smi` rows unchanged from P3 |
| **6** | the next wall's controls (§1.2), each host-derived or authored per family with provenance | one boot per control served; each moves the wall or is reverted |
| **7** | §2.7 interrupts; §2.8 passthrough | `cuCtxCreate` in the guest: GR + CE user channels born Passthrough (`no_worker_ever_saw_the_token`), the first non-stall interrupt delivered via irqfd (count at the guest's `MC_SERVICE_INTERRUPTS`: hardware calls it **0** times, `nvdiff`) |

---

## 4. Open questions (each needs a decision)

**Q1. Births on the register drainer, under the GSP lock.** A birth is ~7 host ioctls (~ms) run
while the drainer serves the RPC with the GSP FSM locked. Only the drainer takes that lock (the
heartbeat `try_lock`s), and no vCPU ever does — so no one blocks behind it. **Recommend: accept**
(it is the RPC-map sync point's shape: the reply IS the act); revisit if a second GPU shares the
drainer.

**Q2. Per-runlist channel RAM.** The token index is the chid (`VECTOR 11:0`), sound only while
chids are device-unique. `kfifoIsPerRunlistChramEnabled` families (and SR-IOV) could reuse a chid
across runlists; the doorbell's `RUNLIST_ID 22:16` then disambiguates. **Recommend:** a per-family
`DoorbellIndex` in kf-trap (`index = VECTOR | RUNLIST_ID << 12`, a 19-bit table, which is what
`RungBitmap::TOKEN_BITS = 19` already sizes) the day a family with per-runlist chram is booted;
measure `per_runlist_channel_ram` from the host (`kf_arch` note) at realize and refuse by name
until then.

**Q3. A pushbuffer in vidmem.** The default is sysmem; a `RmInstLoc4` regkey (or a future default)
moves it to vidmem, and then the reader needs the bytes out of the store. The walker's CUDA context
lives on the VA thread; a worker calling `cuMemcpyDtoH` would be a CUDA sync on a worker's stack
(forbidden). **Recommend:** refuse by name (built); if ever needed, a copy on OUR host ring
(CE: store window → a pinned sysmem staging page) completed through the session fd — the same
suspension point as the `MEM_OP` split.

**Q4. ✔ ANSWERED `[measured p5f]`.** ~~The guest binds COPY2; our host ring runs on COPY0.~~ COPY0 is a
GRCE on GA106 and the ring faulted there; the ring now runs on the first host ASYNC copy engine,
authored from the host's caps (§2.2). Mapping the guest's LCE to a distinct host LCE per channel stays
open for a workload that measures per-engine behaviour.

**Q6. Family gaps found on the way (no GA106 constant was added; these are missing rows).**
(a) `kf_abi::versions::alloc_params` maps only `AMPERE_CHANNEL_GPFIFO_A` (`0xc56f`) to
`AllocParams::Channel`, so the link sees no Turing (`0xc46f`)/Hopper/Blackwell channel alloc — the
class rows belong to the generated per-family tables. (b) The Hopper+/Blackwell doorbell is a BAR1
page (`trappolicy::doorbell_for`), and `kf3.c` still only counts BAR1 IO — a Hopper/Blackwell guest's
doorbell reaches no plane. (c) Per-runlist channel RAM (Q2). **Recommend:** (a) and (b) before any
non-Ampere boot.

**Q5. `0x20802a0f` / user shared data.** Both assert and the boot continues today. **Recommend:**
serve each only when a boot names it as the next wall (§3 step 6) — host-derived (`CE_GET_CAPS_V2`,
the PCE masks) per family.
