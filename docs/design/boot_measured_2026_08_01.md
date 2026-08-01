# The boot, MEASURED — 2026-08-01, a stock 580.159.04 driver against master

> **Everything in this file is `[measured]`**: a live boot of the guest against the QOM shim,
> not a reading of `ogkm` and not a replay of `cap1b`. Per `only_live_boots_are_proof`: reading
> the open kernel modules says what the driver *does*; only a boot says what *happens*.

## 0. Provenance — stated first, because this bench has lied about it before

| | |
|---|---|
| Rust archive | built from `/root/kfshim` at **`55a106f`** on the 38-core box, `cargo build --release -p kayfabe-qemu-raw`, rc 0 |
| why `55a106f` and not master `049ecae` | `git diff --name-only 55a106f..049ecae` touches **zero** `crates/` files — only `docs/design/gpu_promote_ctx.md`, `scripts/bite_promote_ctx.py`, `tests/tests/promote_ctx.rs`. The archive is behaviourally master's. |
| C overlay | `qemu/hw/misc/nvkvm/{nvkvm.c,kayfabe_shim.h,nvkvm_compat.h}` — **byte-identical** between master and the installed tree (`cmp` clean), so the shim ABI did not move |
| link | `ninja -C /workspace/bench/qemu-build`, rc 0, binary 05:38 |
| guest | Ubuntu 24.04.4, kernel 6.8.0-136-generic, **stock unpatched** NVIDIA 580.159.04 open kernel module |

⚠ **The previous bench binary could not name its own revision.** It contained exactly one 40-hex
string, `8bab26f4f68e0e26f0bb7960be334d5b520ea452`, which is **not a commit in either repo** — a
build-id. Its only provenance was the worktree it came from, `/workspace/bench/kayfabe-wt`, which
was **19 commits behind master**. This is the trap `CLAUDE.md` already records ("the bench silently
served a binary built from `862c7c2` for weeks"), still live. ⇒ **`strings` cannot answer "what
revision is this"; embed the SHA.**

★ What *did* discriminate: `nm -C … | grep -c kayfabe` went **1558 → 1746**. A release build strips
enum-variant names and inlines hex constants, so `strings` scored **0 for every marker in both
binaries** — my first instrument was useless and said so only because I checked it against a
known-good symbol (`kayfabe_shim_realize`).

## 1. What happened

The device presents, the driver binds, and the adapter init runs a long way:

```
nvkvm: presenting 10de:2504 class 030000 rev a1 subsys 1462:397d (8 interrupt vectors)
nvkvm: memory plane realized (bar0=0xfd000000 bar1=0xe0000000 bar2=0xf0000000,
                              register plane has guest memory=yes)
```
```
00:03.0 VGA compatible controller: NVIDIA Corporation GA106 [GeForce RTX 3060 Lite Hash Rate]
NVRM: loading NVIDIA UNIX Open Kernel Module for x86_64  580.159.04
[drm] [nvidia-drm] [GPU ID 0x00000003] Loading driver
[drm] Initialized nvidia-drm 0.0.0 for 0000:00:03.0 on minor 0
```

Five modules load (`nvidia`, `nvidia_modeset`, `nvidia_drm`, `video`, `ecc`). ⊘ **No `/dev/nvidia*`
node is created**, because `RmInitAdapter` does not complete — see §3.

## 2. ★★★ Three predictions settled by the 2026-08-01 boot at rev `55a106f` — two confirmed, one REFUTED

★ All three verdicts below are read off **the boot of 2026-08-01 at rev `55a106f`** whose full
dmesg is quoted in §1 and §3; nothing in this section is a source reading.

The batched rung (`55a106f`) triaged 28 controls offline against `cap1b` and made three claims that
only a boot could test.

| prediction | verdict |
|---|---|
| (`[measured]` 2026-08-01, rev `55a106f`) `0x20800a59` GMMU_GET_STATIC_INFO lets `kgmmuFaultBufferInit_HAL` reach the `REGISTER_FAULT_BUFFER` this port refuses, and `gpuStateInit` maps that to `NV_OK` so **the boot survives** — `[inferred]` | ★ **CONFIRMED.** Zero occurrences of `REGISTER_FAULT_BUFFER`, `kgmmuFaultBufferInit` or any fault-buffer error in the whole log. The boot goes straight past it. |
| `0x20802a08` CE fault-method-buffer size is `RefusalIsInvisible` — refused, and the sweep carries on | ★ **CONFIRMED**, and it is the **first** assertion in the log: `NV_ERR_NOT_SUPPORTED … NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE @ kernel_ce.c:843`. Non-fatal; the boot continues for another ~8 ms and dozens of operations. |
| `0x20800a61` FIFO_GET_NUM_CHANNELS is **where the boot now stops** — `[inferred]` from `kernel_fifo.c:300-308` | ⊘ **REFUTED.** Zero occurrences of `numChannels`, `kfifoChidMgr` or `ChidMgrConstruct`. Serving it worked and the wall is elsewhere entirely. |

★★ Two `RefusalHalts` rows are also confirmed as genuinely halting — **observed in the same
2026-08-01 boot at rev `55a106f`**, not inferred:
`NV2080_CTRL_CMD_INTERNAL_INIT_USER_SHARED_DATA` (`gpu_user_shared_data.c:233`, and again at
`:213`) and `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` (`mem_mgr.c:625`, then
`memmgrRegisterSuspendCallbacks` at `:777`).

## 3. ⊘ REFUTED — kept as the record of a wrong inference. See §7 and §10.

> ⊘ **The causal claim in this section is FALSE and was refuted by measurement.** It reads
> *"refusing object allocation starves the heap"*; that was inferred from **adjacency in a
> single log**. `GspRmAlloc` was then fully served (`7b5d8c3`) and boot `alloc2` showed **zero**
> `rpcRmApiAlloc_GSP` and **zero** `rpcRmApiFree_GSP` lines — **and the heap was still null**
> (§7). The heap is not starved at all: it is **created** at `mem_mgr.c:684` and then **torn
> down** by the `failed:` label's `memmgrStateDestroy` (`:963-975`) (§10).
>
> ★ It is kept, not deleted, because the *observation* below is accurate and only the
> *inference* was wrong — and because this is the clearest example in the file of the failure
> mode the whole document exists to resist: **a chain of assertions read as a chain of
> causes.** ⇒ Do not cite this section for causation; cite §7 and §10.

Not a missing **control** — a missing **verb**. `GspRmAlloc` is refused for every class:

```
rpcRmApiAlloc_GSP: GspRmAlloc failed: hClient=0xc1e00004 hParent=0x00000000 hObject=0x00000000
                   hClass=0x00000000 paramsSize=0x78 status=0x56
rpcRmApiAlloc_GSP: … hClass=0x00000080 (NV01_DEVICE_0)     status=0x56
rpcRmApiAlloc_GSP: … hClass=0x00002080 (NV20_SUBDEVICE_0)  status=0x56
rpcRmApiAlloc_GSP: … hClass=0x0000007e                     status=0x56
```

`0x56` is `NV_ERR_NOT_SUPPORTED` — our named refusal. The consequence chain is exact:

```
pHeap != NULL                                     @ mem_desc.c:152
  → _memdescSetSubAllocatorFlag → NV_ERR_INVALID_STATE @ mem_desc.c:404
    → kern_bus_gm107.c:1798 → :1413
      → kbusInitBar2_HAL          → NV_ERR_INVALID_STATE @ kern_bus_gm107.c:332
        → kbusStateInitLockedKernel_HAL                  @ kern_bus_gm107.c:465
          → RmInitNvDevice: *** Cannot initialize the device
            → RmInitAdapter failed! (0x24:0x40:1220)
```

⇒ **Refusing object allocation starves the heap, and a null heap fails BAR2 init.** The next rung
is therefore *not* another init-table control: it is **serving `GspRmAlloc` (RPC object allocation)**
for at least `NV01_DEVICE_0` (`0x80`), `NV20_SUBDEVICE_0` (`0x2080`) and `0x7e`.

★ This is a good failure. Every refusal is **named**, every consequence is **logged**, and the
driver bails out cleanly rather than wedging — which is what `#111`'s "emit fewer faults, but say
which" and `#127`'s "a named refusal, never a silent `NV_OK`" were for. ⊘ Compare the C, whose
generic `NV_OK` fall-through would have let this proceed on a lie.

## 4. What this does NOT establish

- ⊘ **No compute.** Nothing here reaches `cuInit`, let alone `cuCtxCreate`. `nvidia-smi` prints
  *"No devices were found"*.
- ⊘ **No host GPU is involved.** This box has none; forwarding is off. The boot exercises the
  emulated GPU and the fake GSP only.
- ⊘ **One boot, one guest, one driver version.** No claim about 610, about a second arch, or about
  reproducibility across boots — `#98` records that a Mode-2 symptom was 1/3 one day and 9/9 the
  next on a bit-identical binary.
- ⊘ The reachability shadow (`4a93d54`) and promote-ctx (`0644241`) are **not exercised** by this
  boot: it never gets far enough to publish a page table or promote a context.

---

# The SECOND and THIRD boots of 2026-08-01 — the `GspRmAlloc` rung, measured

> Same discipline as everything above: these are two live boots of the same bench, not a
> replay and not a reading. Both are reported, including the one that failed, because the
> failure is where the finding is.

## 5. Provenance

| | |
|---|---|
| Rust archive | built from `/root/kfalloc` on the 38-core box, `cargo build --release -p kayfabe-qemu-raw`, rc 0 |
| boot `alloc1` | rev **`2ced035`** — the archive says so itself (see §8) |
| boot `alloc2` | rev **`a6412c0`** |
| C overlay | `qemu/hw/misc/nvkvm/{nvkvm.c,kayfabe_shim.h,nvkvm_compat.h,meson.build}` — **byte-identical** to the installed tree (`cmp` clean, all four), so only the archive moved |
| link | `ninja -C /workspace/bench/qemu-build`, rc 0 |
| guest | Ubuntu 24.04, kernel 6.8.0-136-generic, **stock unpatched** NVIDIA 580.159.04 open kernel module |

## 6. ★★★ Boot `alloc1` (`2ced035`) — the rung was built and the wall did not move

Every class was still refused `0x56`, byte for byte the §3 log. What made it diagnosable
in one boot instead of five was the device's own audit, printed at teardown:

```
nvkvm: commands: 34 decoded, 7 UNSERVICED, 6 distinct
nvkvm:   unserviced fn 76 cmd 0x20800a87   (INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO)
nvkvm:   unserviced fn 76 cmd 0x20800a4b   (INTERNAL_DISPLAY_GET_IP_VERSION)
nvkvm:   unserviced fn 76 cmd 0x20802a08   (CE_GET_FAULT_METHOD_BUFFER_SIZE)
nvkvm:   unserviced fn 76 cmd 0x20800afe   (INTERNAL_INIT_USER_SHARED_DATA)
nvkvm:   unserviced fn 76 cmd 0x20800aff   (INTERNAL_USER_SHARED_DATA_SET_DATA_POLL)
nvkvm:   unserviced fn 76 cmd 0x20800301   (EVENT_SET_NOTIFICATION)
```

⊘ **All seven are `fn 76`. Not one `fn 103` reached the ledger** — so the alloc *was*
claimed and answered, and the answer was a refusal from inside the bridge. That single
negative is what turned "it still fails" into "it fails inside `translate_alloc`", and it
is the payoff of the unserviced ledger being a **host-side list** rather than something
read one boot at a time.

★★ The refusal is `ParamsSizeExceedsPayload`, and it is a **protocol fact**:
`rpcRmApiAlloc_GSP` declares `length = sizeof(rpc_message_header_v) +
sizeof(rpc_gsp_rm_alloc_v03_00)` (`ogkm-580: rpc.c:11196-11199`, `rpc_common.c:183`) and
that struct's last member is a **flexible** `NvU8 params[]`
(`ogkm-580: g_rpc-structures.h:1491-1502`) — so the declared length stops exactly where
the params begin, and the params are copied in afterwards without updating it. They still
arrive, because the queue transfers whole `RM_PAGE_SIZE` elements
(`message_queue_cpu.c:563-565`). ⊘ The C artifact reads them exactly that way, at a fixed
element offset with no reference to `length` (`C: nvkvm_gpu_emul.c:6775-6781`).

## 7. ★★★ Boot `alloc2` (`a6412c0`) — the wall is GONE

```
NVRM: loading NVIDIA UNIX Open Kernel Module for x86_64  580.159.04
[drm] Initialized nvidia-drm 0.0.0 20160202 for 0000:00:03.0 on minor 0
```
**Zero `rpcRmApiAlloc_GSP` lines. Zero `rpcRmApiFree_GSP` lines.** All four classes
(`0x0`, `0x80`, `0x2080`, `0x7e`) and all three frees are served. The device audit for this
boot lists the same six controls and, again, no `fn 103`.

### ⊘ And §3's causal claim is REFUTED by it

§3 said, in bold: *"Refusing object allocation starves the heap, and a null heap fails
BAR2 init."* That was inferred from adjacency in one log, and it is wrong. Serving every
allocation changed **nothing** downstream — the identical chain still runs:

```
pHeap != NULL @ mem_desc.c:152 → _memdescSetSubAllocatorFlag → NV_ERR_INVALID_STATE
  → kern_bus_gm107.c:1798 → :1413 → kbusInitBar2_HAL → kbusStateInitLockedKernel_HAL
    → RmInitNvDevice: *** Cannot initialize the device → RmInitAdapter failed! (0x24:0x40:1220)
```

★ What the two boots together establish is the *real* dependency, and it is one rung
further back: `pHeap` is `memmgrGetDeviceSuballocator`'s answer
(`ogkm-580: mem_desc.c:150-152`), and the heap is created inside `memmgr`'s own state init
— which does not complete, because **`NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` (`0x20800301`)
is refused at `mem_mgr.c:625`**, followed by `memmgrRegisterSuspendCallbacks` at `:777`.

⇒ **The next rung is `0x20800301`**, not another alloc class. `[measured]` 2026-08-01 —
this is a differential between the `alloc1` boot at rev `2ced035` and the `alloc2` boot at
rev `a6412c0`, two boots one commit apart, which is the only reason it can be stated as
cause rather than as correlation.

## 8. ★ The archive can now name its own revision

§0 recorded that the previous bench binary could not: its only 40-hex string was a
build-id. `build.rs` now stamps `kayfabe-rev:<sha>` into `.rodata` behind a `#[used]`
anchor, and it survives a release build:

```
$ strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -o 'kayfabe-rev:[a-f0-9]\{40\}'
kayfabe-rev:a6412c0c4ec506301f1df295a7eaa6196cec2974
$ nm -C /workspace/bench/qemu-build/qemu-system-x86_64 | grep -c kayfabe
4148          # was 1746 at 55a106f
```

★ The first thing it caught was the author. The `2ced035` archive stamped itself
`…-dirty` from a tree `git status` called clean at the repo root, because `cargo build`
had rewritten `Cargo.lock` and nothing had committed it.

## 9. What these two boots do NOT establish

- ⊘ **No `/dev/nvidia*` node, no compute.** `nvidia-smi` still prints *"No devices were
  found"*. The adapter does not initialise.
- ⊘ **No host GPU.** This box has none; forwarding is off, and the shipped isolate factory
  is `StillbornIsolates` — every isolate is retired at birth and can issue no verb. Nothing
  here says anything about a forwarded operation.
- ⊘ **The data plane was never exercised.** `Ga10xArch` refuses every MMU, USERD and
  pushbuffer seam, and the boot never reaches one. That the refusals are *correct* is a
  unit-test claim, not a boot claim.
- ⊘ **One boot each, one guest, one driver version.** `#98` records a Mode-2 symptom that
  was 1/3 one day and 9/9 the next on a bit-identical binary.

---

# The `EVENT_SET_NOTIFICATION` rung — what was built, and the two things §7 got slightly wrong

> Written at the rung, **before** the boot that tests it — that boot is §14-§19, `evt1` at
> rev `0d82456`. Everything in §10-§13 that does not name a run is a source reading of
> `ogkm-580`, said as one.

## 10. ★★★ §7 named the right control for a slightly wrong reason

§7 concluded *"memmgr state init does not complete, because `0x20800301` is refused at
`mem_mgr.c:625`, followed by `memmgrRegisterSuspendCallbacks` at `:777`"*. The **control is
right** and the differential that identified it stands. Two details of the mechanism are
not:

1. **`:625` and `:777` are not two steps — `:625` is *inside* `:777`.**
   `memmgrRegisterSuspendCallbacks` is `mem_mgr.c:601-634`; line 625 is the
   `NV_ASSERT_OK_OR_RETURN` around the control, and line 777 is its **only** call site,
   inside `memmgrStateInitLocked_IMPL`. There is exactly one `0x20800301` in the file.

2. **The heap is created and then TORN DOWN — it is never "starved".**
   `memmgrCreateHeap` runs at `mem_mgr.c:684`, *ninety lines before* the control, and sets
   `pMemoryManager->pHeap` (`:1113`). The refusal takes `NV_ASSERT_OK_OR_GOTO(…, failed)`
   at `:775-778`, and the `failed:` label calls `memmgrStateDestroy`, which does
   `objDelete(pHeap); pMemoryManager->pHeap = NULL;` (`:963-975`) under a comment that calls
   itself a WAR for `Bug 3482892: Need a way to roll back StateInit steps`.

⇒ The end state is indistinguishable from "no heap", which is why §3's inference was
plausible. But a port that read the null heap as *"the guest needs memory"* would be
chasing a symptom ninety lines downstream of the statement that actually failed. ⊘ The
lesson is §3's lesson again, one level finer: adjacency in a log is not a mechanism.

## 11. ★★★ The reply may not be empty — a transport fact, not a control fact

`NV2080_CTRL_EVENT_SET_NOTIFICATION_PARAMS` has **zero `[OUT]` fields**
(`ogkm-580: ctrl2080event.h:83-94`), and `subdeviceCtrlCmdEventSetNotification_IMPL`
consumes the RPC only through `NV_CHECK_OK_OR_RETURN(LEVEL_WARNING, …)`
(`subdevice_ctrl_event_kernel.c:110-117`). By `kayfabe_device::inert`'s own rule — *"the
guest reads nothing but the status"* — this is an inert command, and the obvious
implementation is an empty body.

⊘ **That implementation is silently wrong**, and the reason belongs to the transport:

```c
if (paramsSize != 0)
{
    portMemCopy(pParamStructPtr, paramsSize, rpc_params->params, paramsSize);
}
```

(`ogkm-580: rpc.c:11085-11090`) — `pParamStructPtr` **is** the caller's own
`eventNotificationParams`, and the caller reads `->event` and `->action` out of it *after*
the RPC returns, to drive its `notifyActions[]` switch
(`subdevice_ctrl_event_kernel.c:119-146`). So an all-zero reply rewrites
`event = 194, action = REPEAT` into `event = 0, action = ACTION_DISABLE`, arms the wrong
notifier, and returns `NV_OK`.

★ This port therefore **re-encodes the decoded request** rather than echoing its bytes: the
guest's field values come back because they must, and both pad runs come back zero because
nothing unmodelled may be reflected. On the *failure* path the copyout is skipped entirely
(`rpc.c:11066-11070`; this control carries no `RMCTRL_FLAGS_COPYOUT_ON_ERROR` — flags
`0x10118`, `g_subdevice_nvoc.c:1606`), so a refusal leaves the guest's struct untouched.

## 12. ★★ Why accepting the registration is honest, and the scope that makes it so

The triage row for `0x20800301` argued against serving it: *"this port gates event delivery
off after `GSP_INIT_DONE` … so accepting a notification registration would promise an
interrupt nothing raises."* The observation (`IrqRaise == 1` across `cap1`, zero `IRQSCLR`)
is correct; the inference is not. It conflates **registering** an arming with **delivering**
an event, and an undelivered notification costs something only for an event that can occur.

The registration `memmgrRegisterSuspendCallbacks` sends is
`NV2080_NOTIFIERS_POWER_RESUME` = 194, action `REPEAT`, callback
`memmgrSuspendResumeCallback` (`mem_mgr.c:567-599`) — which requeues `MemoryMapper` work on
resume from a power-state transition. **This device performs none.** So the number of
`POWER_RESUME` events it will fail to deliver is zero.

⊘ Which is why the promise is scoped to a **list**
(`kayfabe_abi::eventnotify::SILENT_NOTIFIERS`) and not to the control. A rule shaped
*"anything below `NV2080_NOTIFIERS_MAXCOUNT`"* would quietly cover fault and completion
notifiers whose silence is a hang nobody could attribute. Forgetting a row costs one loud
boot; widening the rule costs an unexplained one.

## 13. ★★ The instrument `alloc1` did without

§6's diagnosis turned on `fn 103` being **absent** from the unserviced ledger's six lines.
That works, and it is diagnosis-by-absence — the reasoning `UnservicedLedger` exists to
abolish for the other half of the chain. The gap was **ownership**, not instrumentation:
`ObjectPolicy` owns its `Gpu` and is installed as `Box<dyn CommandPolicy>`, so its census
was unreachable the moment the composition root boxed it.

`kayfabe_rmrpc::SharedRefusalCensus` is a clonable handle taken *before* boxing and kept by
`kayfabe_qemu_raw::shim::Regs`, and `KayfabeRegAudit` now carries the census across the seam
— one row per `FaultTag`, **name by value** (the host-pointer gate forbids an address
outside `*_unsafe.rs`, and smuggling one as a `u64` would defeat the gate rather than
satisfy it). The C shell prints it unconditionally, including when it is zero:

```text
nvkvm: bridge refusals: N total, M distinct (these ANSWER the command and so never reach
                                             the unserviced list)
nvkvm:   bridge refusal BridgeRefusal::<Tag> xK
```

★ `bridge refusals: 0` is a **positive** statement that the bridge refused nothing. The
absence of a line was never one.

---

# The FOURTH boot of 2026-08-01 — `evt1`, and the wall is GONE

> A live boot of the bench, reported in full including what it does not establish.

## 14. Provenance

| | |
|---|---|
| Rust archive | built from `/root/kfevt` on the 38-core box, `cargo build --release -p kayfabe-qemu-raw`, rc 0, from a tree `git status --porcelain` reported as **0 files dirty** |
| boot `evt1` | rev **`0d82456`** — the archive says so itself: `strings … \| grep kayfabe-rev` → `kayfabe-rev:0d824561134a68bf0be5f5dcf0717871ad0aa473`, with **no `-dirty`** suffix |
| C overlay | `nvkvm.c` and `kayfabe_shim.h` **changed** (ABI 6 → 7, and the bridge-refusal print) and were copied; `nvkvm_compat.h` and `meson.build` `cmp`-clean |
| link | `ninja -C /workspace/bench/qemu-build qemu-system-x86_64`, rc 0 |
| discriminator | `nm -C … \| grep -c kayfabe` went **4148 → 4361** |
| guest | Ubuntu, kernel 6.8.0-136-generic, **stock unpatched** NVIDIA 580.159.04 open kernel module, driven with `nvidia-smi` |

## 15. ★★★ The rung is CLEARED — four independent signs, not one

```
NVRM: kbusVerifyBar2_GM107: Pre-L2 invalidate evict: Address 0x2efbae000 programmed
      through the bar0 window with value 0xabcdabcd did not read back the last write.
NVRM: nvAssertOkFailedNoLog: … [NV_ERR_MEMORY_ERROR] (0x00000072) returned from
      kbusVerifyBar2_HAL(pGpu, pKernelBus, NULL, NULL, 0, 0) @ kern_bus_gm107.c:360
NVRM: nvAssertOkFailedNoLog: … returned from kbusStateInitLockedKernel_HAL @ kern_bus_gm107.c:465
NVRM: RmInitNvDevice: *** Cannot initialize the device
NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x24:0x72:1220)
```

Against §7's wall, which was byte-identical across two boots:

| | `alloc1`/`alloc2` | `evt1` |
|---|---|---|
| `pHeap != NULL @ mem_desc.c:152` | present | ★ **absent** |
| `_memdescSetSubAllocatorFlag` → `NV_ERR_INVALID_STATE` | present | ★ **absent** |
| which BAR2 call fails | `kbusInitBar2_HAL` (`kern_bus_gm107.c:332`) | ★ `kbusVerifyBar2_HAL` (`:360`) |
| `RmInitAdapter failed!` | `(0x24:**0x40**:1220)` — `NV_ERR_INVALID_STATE` | ★ `(0x24:**0x72**:1220)` — `NV_ERR_MEMORY_ERROR` |
| `0x20800301` in the unserviced ledger | **yes** | ★ **no** |

⇒ `memmgrStateInitLocked_IMPL` **completes**. The heap it creates at `mem_mgr.c:684` is no
longer rolled back by the `failed:` label, `memmgrGetDeviceSuballocator` returns it, and the
whole `kbusInitBar2_HAL` chain that consumed the null heap now succeeds.

★★ And the call order makes "further" a **structural** claim rather than an impression:
`kbusStateInitLockedKernel_GM107` is a linear chain of `NV_ASSERT_OK_OR_RETURN`s —
`:332 kbusInitBar2_HAL` then `:360 kbusVerifyBar2_HAL`. Reaching the second means the first
returned `NV_OK`. It cannot be reached any other way.

## 16. ★★ The unserviced ledger's membership churned exactly as its own docs predict

```text
nvkvm: commands: 37 decoded, 8 UNSERVICED, 7 distinct
nvkvm:   unserviced fn 76 cmd 0x20800a87   nvkvm:   unserviced fn 76 cmd 0x20800a4b
nvkvm:   unserviced fn 76 cmd 0x20802a08   nvkvm:   unserviced fn 76 cmd 0x20800afe
nvkvm:   unserviced fn 76 cmd 0x20800aff   nvkvm:   unserviced fn 70
nvkvm:   unserviced fn 76 cmd 0x20800a70
```

`commands` **28 → 37**. Distinct **6 → 7**, and the *set* moved: `0x20800301` left it
(served), and **`fn 70`** and **`0x20800a70`** entered — two the boot had never got far
enough to ask. In `cap1b` those sit at `rpc.sequence` **27, 28 and 29**, immediately after
`0x20800301` (25) and `0x20800a59` (26), so the guest is walking the oracle's own order.
⊘ `kayfabe_device::unserviced`'s rule holds again: watch the membership, never the
cardinality.

## 17. ★★★ The new wall is a DATA-PLANE wall, and it is not BAR2

⚠ The message names BAR2 and the test is not one. `kbusVerifyBar2_GM107`
(`ogkm-580: kern_bus_gm107.c:3970`) allocates a 16-byte FB buffer and, at `:4084-4090`,
runs a check that touches **no BAR2 and no MMU**:

```c
GPU_FLD_WR_DRF_NUM(pGpu, _PBUS, _BAR0_WINDOW, _BASE, NvU64_LO32(bar0TestAddr >> 16));
GPU_FLD_WR_DRF_NUM(pGpu, _PBUS, _BAR0_WINDOW, _TARGET, testAddrSpace);
testData = GPU_REG_RD32(pGpu, DRF_BASE(NV_PRAMIN) + NvU64_LO32(bar0TestAddr & 0xffff));
GPU_REG_WR32(pGpu, DRF_BASE(NV_PRAMIN) + NvU64_LO32(bar0TestAddr & 0xffff), SAMPLEDATA);
if (GPU_REG_RD32(pGpu, DRF_BASE(NV_PRAMIN) + NvU64_LO32(bar0TestAddr & 0xffff)) != SAMPLEDATA)
```

It programs the **BAR0 moving window** (`NV_PBUS_BAR0_WINDOW` at BAR0 `0x1700`, base =
`phys >> 16` in bits 23:0, target aperture in 25:24 —
`ogkm-580: dev_bus.h:43-50`), then does a plain **read-after-write of one dword through
PRAMIN** (BAR0 `0x700000..0x7FFFFF`, `ogkm-580: dev_ram.h:26-33`). For
`bar0TestAddr = 0x2efbae000` that is window base `0x2efba` and BAR0 offset `0x70e000`.

⇒ what this port must provide is a BAR0 window that **aliases real framebuffer storage** at
`(BASE << 16) + (off - 0x700000)` with dword write-then-read coherency, honouring the full
24-bit base. Not page tables, not GMMU translation — those come in the *later* sub-tests at
`:4155-4200` that this boot never reaches.

## 18. ⊘ What this boot does NOT establish

- ⊘ **No compute, no `/dev/nvidia*`.** `nvidia-smi` still prints *"No devices were found"*.
- ⊘ **BAR2 bring-up is not shown to be CORRECT — only to have reported success.**
  `kbusInitBar2`'s own BAR0-window writes are never read back, so a window that silently
  drops writes lets every step of it return `NV_OK` and is caught only here. "Further along"
  is proven; "right" is not.
- ⊘ **No host GPU.** This box has none, forwarding is off, and the isolate factory is
  `StillbornIsolates`.
- ⊘ **That serving `0x20800301` is what moved the wall is `[inferred]` from the four signs
  in §15, not isolated.** This boot changed one rung *and* added the refusal instrument; the
  instrument answers nothing and cannot move a wall, but only a boot at a revision with the
  instrument and without the rung would isolate it, and none was spent.
- ⊘ **One boot.** `#98` records a Mode-2 symptom that was 1/3 one day and 9/9 the next on a
  bit-identical binary.

## 19. ★ The refusal instrument, first light

```text
nvkvm: bridge refusals: 0 total, 0 distinct (these ANSWER the command and so never reach
                                             the unserviced list)
```

⊘ **Zero is the finding, and it is a finding rather than a null result.** §6 could only say
*"`fn 103` is absent from six lines"*; this boot says *"the bridge refused nothing"* as a
positive statement, in one line, without a reader having to know which function numbers
should have been present. The object model accepted every allocation and free this boot
issued.

---

# The FIFTH boot of 2026-08-01 — `bar0win`, and the window READS BACK

> A live boot of the bench, reported in full including what it does not establish.

## 20. Provenance

| | |
|---|---|
| Rust archive | built from `/root/kfbar0` on the 38-core box, `cargo build --release -p kayfabe-qemu-raw`, rc 0, from a tree `git status --porcelain` reported as **0 files dirty** |
| boot `bar0win` | rev **`f43668b`** — the archive says so itself: `strings … \| grep kayfabe-rev` → `kayfabe-rev:f43668be6d5a295c4777514e419b9d825b8da1d1`, with **no `-dirty`** suffix, and it is the binary's **only** 40-hex string |
| C overlay | `nvkvm.c` and `kayfabe_shim.h` **changed** (ABI 7 → 8, and the framebuffer report) and were copied; `nvkvm_compat.h` and `meson.build` `cmp`-clean, and all four `cmp`-clean after the copy |
| link | `ninja -C /workspace/bench/qemu-build qemu-system-x86_64`, rc 0 |
| discriminator | `nm -C … \| grep -c kayfabe` went **4361 → 4385** |
| guest | Ubuntu, kernel 6.8.0-136-generic, **stock unpatched** NVIDIA 580.159.04 open kernel module, driven with `nvidia-smi` |

## 21. ★★★ The rung is CLEARED, and "cleared" is a STRUCTURAL claim

```
NVRM: kbusVerifyBar2_GM107: L2 evict failed
NVRM: nvAssertOkFailedNoLog: … [NV_ERR_NOT_SUPPORTED] (0x00000056) returned from
      kbusVerifyBar2_HAL(pGpu, pKernelBus, NULL, NULL, 0, 0) @ kern_bus_gm107.c:360
```

Against §15's wall:

| | `evt1` (`0d82456`) | `bar0win` (`f43668b`) |
|---|---|---|
| `Pre-L2 invalidate evict: Address 0x2efbae000 … did not read back the last write.` | present | ★★★ **absent** |
| where `kbusVerifyBar2_GM107` fails | the BAR0-window read-back, `:4084-4090` | ★ `kmemsysSendL2InvalidateEvict`, `:4110` |
| `kbusVerifyBar2_HAL` status | `NV_ERR_MEMORY_ERROR` (`0x72`) | ★ `NV_ERR_NOT_SUPPORTED` (`0x56`) |
| `RmInitNvDevice` | *"Cannot **initialize** the device"* | ★★ *"Cannot **load state** into the device"* |
| `RmInitAdapter failed!` | `(0x24:0x72:1220)` | ★ `(0x25:0x40:1249)` |

★★ **Why "the window works" is a structure and not an impression.**
`kbusVerifyBar2_GM107:4084-4114` is straight-line code: the read-back check at `:4090` is
`if (… != SAMPLEDATA) { … goto kbusVerifyBar2_failed; }`, and
`kmemsysSendL2InvalidateEvict` is the **next statement**. The string *"L2 evict failed"*
exists only past that branch. ⇒ printing it **proves** that
`GPU_REG_RD32(DRF_BASE(NV_PRAMIN) + 0xe000)` returned `0xabcdabcd` after
`GPU_REG_WR32` put it there, with the window programmed to base `0x2efba`. It cannot be
reached any other way.

## 22. ★★ The device's own framebuffer report, first light

```text
nvkvm: framebuffer: 6 reads / 33973 writes served through the BAR0 moving window
       (18 window register reads / 16 writes), fb refusals 0,
       translated-window drops 0r/0w, resident 86016 bytes
nvkvm: registers: 3464 reads / 35089 writes (chip-constant 32, rom 2316,
       gsp 347r/367w, UNCLAIMED 700r/733w), faults 0, guest-RAM refusals 0
nvkvm: bridge refusals: 0 total, 0 distinct
```

★★★ **33 973** `PRAMIN` writes. The C oracle's cold-boot census — decoded 2026-07-31 from
`traces/mode2_c_reference/cap1_coldboot_hermetic` through this port's own window classifier —
is **33 978**. Two independent implementations, five years of driver apart from each other's
code, within **five writes** of the same number. ⊘ It is a corroboration and not a
verification: the C's capture is a different boot of a different emulator and nothing forced
the two to agree, so the right reading is *"the guest is doing the `PRAMIN` work the oracle
saw"*, not *"the count is correct"*.

★ `fb refusals 0` is the positive statement the rung was built for: **not one framebuffer
access was dropped**, said as a number rather than inferred from the absence of a later
failure. `resident 86016 bytes` = 21 pages, i.e. the whole boot's framebuffer footprint fits
in 84 KiB — the 1 GiB residency ceiling is four orders of magnitude away from being reached.

⊘ `translated-window drops 0r/0w`: the framebuffer aperture and the instance window were
**never touched** this boot, so this boot says nothing about them. The BAR2 sub-test at
`:4155-4200` is past the L2 evict and was not reached.

## 23. ★★★ The boot advanced a whole PHASE — and part of that is an artefact of the ERROR CODE

`gpuStateInit_IMPL` maps `NV_ERR_NOT_SUPPORTED` to `NV_OK` and carries on
(`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c`, the engine sweep's
`if (rmStatus == NV_ERR_NOT_SUPPORTED) rmStatus = NV_OK;`). `NV_ERR_MEMORY_ERROR` is **not**
in that map. So `evt1`'s `0x72` aborted `gpuStateInit` outright, and `bar0win`'s `0x56` is
absorbed — `KernelBus` is amputated and the boot runs on into `gpuStateLoad`.

⇒ **Both halves are true and neither should be reported alone.** The window really does read
back (§21's structural argument does not depend on the error code at all). *And* the reason
the boot now reaches `kgmmuStatePostLoad`, `kceGetPceConfigForLceType` and
`kgraphicsLoadStaticInfo` is that the failure it still hits changed class, not that the
failure went away. Everything downstream of `kbusStateInitLockedKernel` this boot ran
**without a `KernelBus`**.

The new tail, in order:

```
NVRM: kperfGpuBoostSyncStateInit_IMPL: Failed to read Sync Gpu Boost init state, status=0x56
NVRM: … NV2080_CTRL_CMD_INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE @ kernel_ce.c:1020
NVRM: … NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER @ gpu_vaspace.c:4148
NVRM: kgmmuStatePostLoad_IMPL: Failed to create GVASpace, status:56
NVRM: … NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CAPS @ kernel_graphics.c:1212
NVRM: nvAssertFailedNoLog: Assertion failed: pKernelGraphicsStaticInfo != NULL @ kernel_graphics.c:485
NVRM: nvAssertFailedNoLog: Assertion failed: 0 @ kernel_fifo.c:3129
NVRM: RmInitNvDevice: *** Cannot load state into the device
NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0x40:1249)
```

## 24. The unserviced ledger — membership again, not cardinality

```text
nvkvm: commands: 62 decoded, 18 UNSERVICED, 14 distinct
  0x20800a87  0x20800a4b  0x20802a08  0x20800afe  0x20800aff  fn 70  0x20800a70
  0x20800a6c  0x20800a80  0x20802a0f  0x2080017e  0x20800a9f  0x20800a1f  0x20800a38
```

`commands` **37 → 62**, distinct **7 → 14**. Nothing **left** the set and **seven entered**,
every one of them a control the boot had never got far enough to ask:

| new | what asked for it |
|---|---|
| `0x20800a6c` `INTERNAL_MEMSYS_L2_INVALIDATE_EVICT` | ★ **the immediate wall** — `kbusVerifyBar2_GM107:4110`, the statement after the read-back that now passes |
| `0x20800a80` | `kperfGpuBoostSyncStateInit_IMPL` |
| `0x20802a0f` `INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE` | `kernel_ce.c:1020` |
| `0x2080017e` | `gpu_vaspace.c:4148`, `GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER` |
| `0x20800a9f`, `0x20800a1f`, `0x20800a38` | the `StateLoad` sweep, incl. `STATIC_KGR_GET_CAPS` |

⇒ **The next rung is `0x20800a6c`.** It is the one that stands between this boot and the
*rest* of `kbusVerifyBar2` — the BAR2 sub-test at `:4155-4200`, which is where an actual MMU
translation is exercised for the first time.

## 25. ⊘ What this boot does NOT establish

- ⊘ **No compute, no `/dev/nvidia*`.** `nvidia-smi` still prints *"No devices were found"*.
- ⊘ **BAR2 is still not exercised.** The window this rung built is `PRAMIN`, which is
  **untranslated**. Nothing here says anything about a GMMU walk, and
  `translated-window drops 0r/0w` says the two translated apertures were not even touched.
- ⊘ **The `KernelBus` is amputated.** §23: everything after `kbusStateInitLockedKernel` ran
  without one, so no `StateLoad` result in this log may be read as *"that subsystem works"*.
- ⊘ **No host GPU.** This box has none, forwarding is off, and the isolate factory is
  `StillbornIsolates`. Not one byte of the 33 973 framebuffer writes went near real hardware.
- ⊘ **That serving the window is what moved the wall is `[inferred]` from §21's five signs**,
  not isolated — this boot changed one rung *and* added the framebuffer report. The report
  answers nothing and cannot move a wall, but only a boot at a revision with the report and
  without the rung would isolate it, and none was spent.
- ⊘ **One boot.** `#98` records a Mode-2 symptom that was 1/3 one day and 9/9 the next on a
  bit-identical binary.

---

# The SIXTH boot of 2026-08-01 — `l2evict1`, and the wall moved INTO the MMU test

> A live boot of the bench, reported in full including what it does not establish.

## 26. Provenance

| | |
|---|---|
| Rust archive | built from `/root/kf-l2evict` on the 38-core box, `cargo build --release -p kayfabe-qemu-raw`, rc 0, from a tree `git status --porcelain` reported as **0 files dirty** |
| boot `l2evict1` | rev **`9551dd1`** — the archive says so itself: `strings … \| grep kayfabe-rev` → `kayfabe-rev:9551dd18158c03c9f2033c7324e9660536b03116`, with **no `-dirty`** suffix and exactly one occurrence. ⊘ Not "the only 40-hex string in the binary": the literal is concatenated with the neighbouring `KvmVmfd` in rodata, so a `\b`-anchored search misses it and finds only QEMU's own hash. The `kayfabe-rev:` prefix is the discriminator, not the hex shape |
| C overlay | **unchanged and not copied.** All four of `nvkvm.c`, `kayfabe_shim.h`, `nvkvm_compat.h`, `meson.build` were `cmp`-clean against the deployed copies *before* the link, so this boot changes the Rust side and nothing else |
| link | `ninja -C /workspace/bench/qemu-build qemu-system-x86_64`, rc 0 |
| discriminator | `nm -C … \| grep -c kayfabe` went **4385 → 4393** |
| guest | Ubuntu, kernel 6.8.0-136-generic, **stock unpatched** NVIDIA 580.159.04 open kernel module, driven with `nvidia-smi` |
| rollback | `/workspace/bench/libkayfabe_qemu_raw.a.PREV-f43668b`, `/workspace/bench/qemu-system-x86_64.PREV-f43668b` |

## 27. ★★★ The rung is CLEARED — and the two halves of that must be reported together

```
NVRM: kbusVerifyBar2_GM107: MMUTest BAR0 window offset 0x70e000 returned garbage 0x0
NVRM: nvAssertOkFailedNoLog: … [NV_ERR_MEMORY_ERROR] (0x00000072) returned from
      kbusVerifyBar2_HAL(pGpu, pKernelBus, NULL, NULL, 0, 0) @ kern_bus_gm107.c:360
NVRM: nvAssertOkFailedNoLog: … returned from kbusStateInitLockedKernel_HAL @ kern_bus_gm107.c:465
NVRM: RmInitNvDevice: *** Cannot initialize the device
NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x24:0x72:1220)
```

Against §21's wall:

| | `bar0win` (`f43668b`) | `l2evict1` (`9551dd1`) |
|---|---|---|
| `kbusVerifyBar2_GM107: L2 evict failed` | present | ★★★ **absent** |
| where `kbusVerifyBar2_GM107` fails | `kmemsysSendL2InvalidateEvict`, `:4110` | ★★★ the **MMU test's** read-back, `:4200` |
| `kbusVerifyBar2_HAL` status | `NV_ERR_NOT_SUPPORTED` (`0x56`) | `NV_ERR_MEMORY_ERROR` (`0x72`) |
| `RmInitNvDevice` | *"Cannot **load state** into the device"* | *"Cannot **initialize** the device"* |
| `RmInitAdapter failed!` | `(0x25:0x40:1249)` | `(0x24:0x72:1220)` |
| `0x20800a6c` in the unserviced list | yes | ★★★ **no** |

★★ **Why "cleared" is a structure and not an impression.** `kbusVerifyBar2_GM107:4110-4200`
is straight-line code. The string *"L2 evict failed"* is printed by the `if (NV_OK != status)`
arm at `:4113` and there is no other producer of it; the string *"MMUTest BAR0 window offset
… returned garbage"* exists only at `:4200`. Between them lie the **first** L2 evict
(`:4110`), the BAR0-window restore (`:4138-4141`), the BAR2 write loop
(`MEM_WR32(pOffset + index, SAMPLEDATA)`, `:4163-4166`), and the **second** L2 evict
(`:4175`). ⇒ printing the second message proves both L2 evicts were answered `NV_OK` and the
guest ran past them. It cannot be reached any other way.

## 28. ⊘ AND THE BOOT WENT BACKWARDS IN PHASE — §23 in reverse, and both halves are true

⚠ This is the exact trap §23 named, running the other direction, and reporting either half
alone would be wrong.

`gpuStateInit_IMPL` maps `NV_ERR_NOT_SUPPORTED` to `NV_OK` and does **not** map
`NV_ERR_MEMORY_ERROR`. `bar0win`'s `0x56` was therefore *absorbed* — `KernelBus` was
amputated and the boot ran on into `gpuStateLoad`, `kgmmuStatePostLoad`,
`kceGetPceConfigForLceType` and `kgraphicsLoadStaticInfo`. `l2evict1`'s `0x72` is **not**
absorbed, so `kbusStateInitLockedKernel` aborts `gpuStateInit` outright and none of those
later phases run at all.

⇒ **The wall moved FORWARD ninety lines inside `kbusVerifyBar2` and BACKWARD one phase in the
boot.** Both are consequences of the same change and neither is the headline on its own:

- ⊘ It is **not** a regression. `bar0win` reached `gpuStateLoad` *without a `KernelBus`*, and
  §23 already said no `StateLoad` result in that log could be read as *"that subsystem
  works"*. Trading a deeper log full of results from an amputated bus for a shallower log in
  which the bus is real and fails honestly is the trade this rung was for.
- ⊘ It is **not** unqualified advancement either. The boot ends earlier in wall-clock phase
  terms, and every count below is smaller because of it.

## 29. ★★★ The new wall is the FIRST GMMU TRANSLATION this port has ever been asked for

`:4155-4200` is the sub-test §25 recorded as *"past the L2 evict and not reached"*. It is
reached now, and what it does is the whole point:

1. `MEM_WR32(pOffset + index, SAMPLEDATA)` — sixteen bytes written through the **BAR2 CPU
   mapping**, which is a *translated* aperture: the address goes through the GMMU page
   tables RM published.
2. `GPU_REG_RD32(pGpu, DRF_BASE(NV_PRAMIN) + …)` — the same sixteen bytes read back through
   the **untranslated** BAR0 moving window at the physical framebuffer address.

The guest read `0x0` where it had written `0xabcdabcd`. ⇒ the BAR2 write **did not land in
the framebuffer**. That is the data-plane wall §17 predicted, and it is now the boot's own
statement rather than a forecast.

⊘ `translated-window drops 0r/0w` again, which says the two translated apertures were still
never *reached through the window classifier* — so this log does not yet say **where** the
BAR2 write went, only that it did not arrive.

## 30. ★★ The unserviced ledger — one control left because it is SERVED, seven because their phase is gone

```text
nvkvm: commands: 39 decoded, 8 UNSERVICED, 7 distinct
  0x20800a87  0x20800a4b  0x20802a08  0x20800afe  0x20800aff  fn 70  0x20800a70
```

| | `bar0win` | `l2evict1` |
|---|---|---|
| commands decoded | 62 | 39 |
| UNSERVICED | 18 | 8 |
| distinct | 14 | **7** |
| framebuffer writes through the window | 33 973 | 17 520 |
| `fb refusals` | 0 | 0 |
| registers | 3464r / 35089w | 2844r / 18434w |

★★★ **The two reasons a control left this set are different and must not be conflated.**

- `0x20800a6c` left because it is **served**. It is still asked — §27's structural argument
  requires both of `kbusVerifyBar2`'s first two calls to have been answered — and it is
  simply no longer refused.
- `0x20800a80`, `0x20802a0f`, `0x2080017e`, `0x20800a9f`, `0x20800a1f`, `0x20800a38` left
  because **nothing asks them any more**: every one of them is issued from `gpuStateLoad` or
  later, which §28 says this boot does not reach. Their absence is not progress and must not
  be read as any.

★ `0x20800a70` is still there, deliberately. Its triage row was corrected in the same commit
— `RefusalHalts` → `RefusalIsInvisible`, because `kbusFlush_GM107` overwrites its status only
for `NV_ERR_TIMEOUT` (`ogkm-580: kern_bus_gm107.c:3384-3405`) and GA106 dispatches `kbusFlush`
there (`g_kern_bus_nvoc.c:1871-1881`) — and it is still refused, because vacuity makes an
`NV_OK` *permissible* while a caller that checks is what makes one *necessary*. ⚠ This boot
does **not** test that correction: `kbusVerifyBar2_GM107:4218`, the one site that checks a
flush, is past `:4200` and was not reached.

## 31. ⊘ What this boot does NOT establish

- ⊘ **No compute, no working device.** `nvidia-smi` prints *"No devices were found"*.
  ⚠ `/dev/nvidia0`, `/dev/nvidiactl`, `/dev/nvidia-uvm` and `/dev/nvidia-uvm-tools` all
  **exist** — they are created by the module load, before `RmInitAdapter` — so their presence
  says nothing about the adapter and must not be quoted as if it did.
- ⊘ **Nothing about a real L2.** This device has none. §27 shows the guest accepted the
  answer; it does not show the answer was *right* in any world but this one. The three
  futures that falsify it are in `kayfabe_abi::l2evict`, and the first of them — real
  host-GPU forwarding — requires the row to be **re-decided**, not inherited.
- ⊘ **The third L2 evict is unexercised.** `kbusVerifyBar2_GM107:4224` is past `:4200`.
  Two of the three calls were answered; the third was never made.
- ⊘ **The `WAIT_FB_PULL` prediction is unconfirmed.** `kayfabe_abi::l2evict` predicts the
  wire value is `0x31` (`ALL | CLEAN | WAIT_FB_PULL`, because `bL2CleanFbPull` is `NV_TRUE`
  for GA106 in `ogkm-580: g_kern_mem_sys_nvoc.c:256-262`). This boot proves only that the
  value the guest sent **decoded** — a `0x11` would have decoded too. Distinguishing them
  needs a recorder this boot did not run.
- ⊘ **`interrupt requests dropped 38`** (was 61). This port still delivers no vectors. The
  number moved only because the boot is shorter; it is not an improvement in anything.
- ⊘ **One boot.** `#98` records a Mode-2 symptom that was 1/3 one day and 9/9 the next on a
  bit-identical binary.

---

# The SEVENTH boot of 2026-08-01 — `gmmu1`, and the MMU TEST PASSES

> A live boot of the bench, reported in full including what it does not establish.

## 32. Provenance

| | |
|---|---|
| Rust archive | built on the 38-core build box, `cargo build --release -p kayfabe-qemu-raw`, rc 0, from a tree `git status --porcelain` reported as **0 files dirty** |
| boot `gmmu1` | rev **`12b001f`** — the archive says so itself: `strings … \| grep kayfabe-rev` → `kayfabe-rev:12b001f145c5a641c20a4675ded02556b5494318`, no `-dirty`, exactly one occurrence, and the same string is present exactly once in the linked `qemu-system-x86_64` |
| C overlay | **changed and copied.** `nvkvm.c` and `kayfabe_shim.h` differed and were copied; `nvkvm_compat.h` and `meson.build` were `cmp`-clean and were not. ⚠ So this boot changes the Rust side **and** the shell, and §35 says what that costs |
| link | `ninja -C /workspace/bench/qemu-build qemu-system-x86_64`, rc 0 |
| discriminator | `nm -C … \| grep -c kayfabe` went **4393 → 4388** |
| guest | Ubuntu 24.04.4, kernel 6.8.0-136-generic, **stock unpatched** NVIDIA 580.159.04 open kernel module, driven with `nvidia-smi` |
| rollback | `/workspace/bench/libkayfabe_qemu_raw.a.PREV-9551dd1`, `/workspace/bench/qemu-system-x86_64.PREV-9551dd1` |

## 33. ★★★ The rung is CLEARED — and this time the phase moved forward on a REAL `KernelBus`

```
NVRM: nvAssertOkFailedNoLog: … NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE @ kernel_ce.c:843
NVRM: kperfGpuBoostSyncStateInit_IMPL: Failed to read Sync Gpu Boost init state, status=0x56
NVRM: … NV2080_CTRL_CMD_INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE @ kernel_ce.c:1020
NVRM: … NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER @ gpu_vaspace.c:4148
NVRM: kgmmuStatePostLoad_IMPL: Failed to create GVASpace, status:56
NVRM: … NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CAPS @ kernel_graphics.c:1212
NVRM: nvAssertFailedNoLog: Assertion failed: pKernelGraphicsStaticInfo != NULL @ kernel_graphics.c:485
NVRM: nvAssertFailedNoLog: Assertion failed: 0 @ kernel_fifo.c:3129
NVRM: RmInitNvDevice: *** Cannot load state into the device
NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0x40:1249)
```

Against §27's wall:

| | `l2evict1` (`9551dd1`) | `gmmu1` (`12b001f`) |
|---|---|---|
| `kbusVerifyBar2_GM107: MMUTest BAR0 window offset … returned garbage` | present | ★★★ **absent** |
| `kbusVerifyBar2_HAL` status | `NV_ERR_MEMORY_ERROR` (`0x72`) | ★★★ **no failure at all** |
| `RmInitNvDevice` | *"Cannot **initialize** the device"* | *"Cannot **load state** into the device"* |
| `RmInitAdapter failed!` | `(0x24:0x72:1220)` | `(0x25:0x40:1249)` |
| `fn 70` in the unserviced list | yes | ★★★ **no** |
| `translated-window drops` | `0r/0w` (never reached) | `0r/0w` (reached and **served**) |

★★ **Why "cleared" is a structure and not an impression, and it is stronger than §27's.**
`kbusVerifyBar2_GM107:4155-4230` is straight-line code with **four** failure prints, each with
exactly one producer: *"MMUTest BAR0 window offset … returned garbage"* at `:4200`,
*"MMUTest BAR2 Read of virtual addr … returned garbage"* at `:4240`, and two *"L2 evict
failed"* at `:4175`/`:4224`. **None of the four is in this log**, and the function's caller
`kbusStateInitLockedKernel` is under `NV_ASSERT_OK_OR_RETURN` (`:360`) whose failure prints
`kern_bus_gm107.c:360 → :465`, also absent. ⇒ every statement of the MMU sub-test ran and
passed: the BAR2 write, the BAR0 read-back, the reverse BAR0 write and the BAR2 read-back.

## 34. ★★★ AND IT IS NOT §23's TRADE — the bus is real this time

⚠ §23 and §28 both warn that a deeper log can be bought by an **amputated `KernelBus`**:
`gpuStateInit_IMPL` absorbs `NV_ERR_NOT_SUPPORTED` and not `NV_ERR_MEMORY_ERROR`, so a `0x56`
lets the boot run on without a bus. This boot ends in `gpuStateLoad` and its statuses are
`0x56`, which is the *shape* of that trade — so the question has to be asked directly, and
the answer is that it is not that trade:

- ⊘ `bar0win`'s depth came from `kbusVerifyBar2` **failing** `NV_ERR_NOT_SUPPORTED` at the L2
  evict, i.e. from the bus's own init being absorbed. Here `kbusVerifyBar2` does not fail at
  all — §33's four-producer argument — so `kbusStateInitLockedKernel` returned `NV_OK` and
  `KernelBus` is constructed.
- ★ The `0x56`s in this log come from **different controls, at different call sites**:
  `CE_GET_FAULT_METHOD_BUFFER_SIZE`, `INIT_USER_SHARED_DATA`,
  `CE_GET_PCE_CONFIG_FOR_LCE_TYPE`, `GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`,
  `STATIC_KGR_GET_CAPS` — every one a control this port has not built, none of them
  `kbusVerifyBar2`'s.

⇒ **forward in phase AND with a real bus**, which is the first time both have been true
together.

## 35. ★★ The device's own translated-window report, first light

```text
nvkvm: framebuffer: 67 reads / 46380 writes served through the BAR0 moving window
       (26 window register reads / 22 writes), fb refusals 0,
       translated-window drops 0r/0w, resident 102400 bytes
nvkvm: BAR2 (translated): 56 reads / 12402 writes resolved through the GMMU,
       0 REFUSED by name; roots published 2 (0 bodies refused), BAR2 root entry 0x0
```

- ★★★ **12 402 writes and 56 reads resolved through a page walk**, and **zero** refused. The
  guest's BAR2 traffic is not a handful of dwords: `kbusUpdateRmAperture_GM107` writes page
  tables *through BAR2 itself* after bootstrap, so most of that number is the guest editing
  its own page tables through the aperture those tables describe.
- ★★ `fb refusals 0` and `translated-window drops 0r/0w` **together**. Before this rung the
  second number was also `0` and meant *"never reached"*; it now means *"reached, and served
  by the other counter"*. The pair is only readable because they are separate counters.
- ⚠ **`BAR2 root entry 0x0` with `roots published 2`, and that is the guest's own teardown.**
  `kbusDestroyBar2GpuVaSpace` publishes `entryValue = 0` to unroot the aperture
  (`ogkm-580: kern_bus_gm107.c:2137`), which is what the *second* publication is — after
  `RmInitAdapter` failed. The 12 402 writes resolved against the **first**. ⊘ This is exactly
  why `PublishedPde::entry` is `u64` and not `Option<u64>`, and why the **count** rather than
  the value is what says whether a root ever arrived: the value alone would read as "none".

## 36. The unserviced ledger — seven → twelve, and every new one is a `gpuStateLoad` control

```text
nvkvm: commands: 64 decoded, 15 UNSERVICED, 12 distinct
  0x20800a87  0x20800a4b  0x20802a08  0x20800afe  0x20800aff  0x20800a70
  0x20800a80  0x20802a0f  0x2080017e  0x20800a9f  0x20800a1f  0x20800a38
```

| | `l2evict1` | `gmmu1` |
|---|---|---|
| commands decoded | 39 | 64 |
| UNSERVICED | 8 | 15 |
| distinct | 7 | **12** |
| framebuffer writes through the window | 17 520 | 46 380 |
| BAR2 writes through the GMMU | — (unbuilt) | **12 402** |
| registers | 2844r / 18434w | 3517r / 47504w |

★ `fn 70` **left the set because it is served**, and the five that entered
(`0x20800a80`, `0x20802a0f`, `0x2080017e`, `0x20800a9f`, `0x20800a1f`, `0x20800a38`) are the
`gpuStateLoad` sweep's — the same five that entered at `bar0win` and left again at
`l2evict1` when the phase went away. They are back because the phase is back, and this time
it is back with a bus. ⊘ Membership, never cardinality (§24, §30).

★ `0x20800a70` is still refused, deliberately, and this boot **does** now reach a site that
checks a flush — `kbusVerifyBar2_GM107:4218` is inside the region §33 shows ran. Its triage
row (`RefusalIsInvisible`) therefore survived its first real exercise.

## 37. ⊘ What this boot does NOT establish

- ⊘ **No compute, no working device.** `nvidia-smi` prints *"No devices were found"*.
  ⚠ `/dev/nvidia0`, `/dev/nvidiactl`, `/dev/nvidia-uvm` and `/dev/nvidia-uvm-tools` all
  **exist** — created by the module load, before `RmInitAdapter` — so their presence says
  nothing about the adapter.
- ⊘ **Nothing about the CORRECTNESS of 12 402 translations beyond the one the guest checks.**
  `kbusVerifyBar2` verifies sixteen bytes. The other translations are unaudited by anything
  in this log; a walk that resolved a *different* wrong page for some later mapping would
  look identical here.
- ⊘ **BAR1 is untranslated and unexercised.** Its root is recorded and nothing resolves
  through it; `translated-window drops 0r/0w` says the framebuffer aperture was never touched
  either, so that zero is *"not reached"*, not *"served"*.
- ⊘ **Two things changed at once.** This boot carries the Rust rung **and** a C-overlay change
  (the BAR2 region became a trap region — it could not be otherwise, since a reservation
  region never reaches the archive at all). The two are not separable by this boot, and no
  boot was spent separating them.
- ⊘ **No host GPU.** This box has none, forwarding is off, and the isolate factory is
  `StillbornIsolates`. Not one of the 12 402 translated writes went near real hardware.
- ⊘ **No sysmem leaf, no big-page leaf, no 512 MiB leaf was exercised by this guest** — or
  rather, this log cannot say whether one was: `bar2_faults` is 0, which proves no access was
  *refused*, and there is no counter that says which leaf size resolved.
- ⊘ **One boot.** `#98` records a Mode-2 symptom that was 1/3 one day and 9/9 the next on a
  bit-identical binary.

---

# ★★★ 38. THE INSTRUMENT WAS BROKEN, AND EVERY SECTION ABOVE IS AFFECTED

Read this before citing anything above it.

## 38.1 The measurement

```
$ grep -ci nvrm /workspace/bench/run_*_serial.log
run_bar0win_serial.log:0    run_evt1_serial.log:0     run_l2evict1_serial.log:0
run_gmmu1_serial.log:0      run_alloc1_serial.log:0   run_alloc2_serial.log:0
```

**Zero, for every boot rung of 2026-08-01.** `[measured]` on this box, 2026-08-01.

## 38.2 Why, and why it looks fine

`-serial file:` captures the **guest console**. The NVIDIA driver is `modprobe`d **over ssh,
after boot**, so every `NVRM:` line it prints goes to the kernel ring buffer and is read by
`dmesg` in *the invoking process's* terminal. For six consecutive rungs that invoking process
was an agent, and an agent's transcript is not a file anybody can re-read.

⇒ every `RmInitAdapter failed! (0x…)` quoted in §1-§37 is **true and unsourced**. The rungs
were real; the evidence for them was ephemeral. ⊘ The one exception is
`/workspace/bench/dmesg_master_55a106f.txt`, saved by hand at one rung and not at the others —
which is the giveaway: a convention that depends on somebody remembering is not an instrument.

## 38.3 The fix, and the fix's own first failure

`scripts/bench/boot_capture.sh` — boots, waits for ssh, loads the driver **cold**, opens the
device, reads `dmesg` into `/workspace/bench/run_<tag>_dmesg.log`, **verifies the capture**, and
powers down so the next run gets a fresh WPR2.

⊘ **Its first version reproduced the exact defect it was written to prevent.** Run
`master7d16c37`: it read the ring buffer straight after `modprobe` and captured **four lines** —
an nvlink banner, a vgaarb line, and `loading NVIDIA UNIX Open Kernel Module`. Its content check
was *"does this file contain `NVRM`"*; the banner **is** an `NVRM:` line; the check passed and
the script exited 0 on a capture with no adapter output in it whatsoever.

Two things were wrong, and both are now encoded in the script rather than in this document:

1. ★★★ **`modprobe` does not run `RmInitAdapter`.** It registers the PCI driver. The adapter is
   initialised on the first `open()` of `/dev/nvidia0` — and that node **does not exist** until
   something creates it (`ls /dev/nvidia*` → *"No such file or directory"*, in that same probe
   log). `nvidia-smi` does both. The device is now opened **before** the buffer is read.
2. ★★★ **The check is on `RmInitAdapter`, not on `NVRM`.** A predicate satisfied by a banner is
   a predicate that cannot fail for the reason you care about. ⊘ This is `suspect_the_instrument
   _first` at one remove: the instrument I *built to fix a flattering instrument* was itself
   flattering, and only a by-hand read of the four-line file caught it.

## 38.4 What a green exit from `boot_capture.sh` means

**Only that an observation was made and stored.** Not that the boot went well, and not that the
capture is complete. `run_<tag>_probe.log` carries `MODPROBE_RC`, `SMI_RC`, the `/dev` listing
and `lsmod` beside it, because each of those distinguishes a different way the file below can be
short.

---

# 39. Boot `stateload1` — rev `041b4f1`, and the row the boot wrote for me

| | |
|---|---|
| provenance | `/workspace/kf-stateload` at **`041b4f1`**, `scripts/build_qom_shim.sh` → `ninja`, rc 0. ★ The archive is built from the same tree the SHA names — no `kayfabe-wt` in the loop. |
| harness | `scripts/bench/boot_capture.sh stateload1` |
| **evidence** | **`/workspace/bench/run_stateload1_dmesg.log`** — 22 lines, 19 `NVRM`, 3 adapter. ★ The first boot rung of this project whose driver output is on disk. |
| result | `RmInitAdapter failed! (0x25:0x40:1249)` — **the same triple as `gmmu1`** |

## 39.1 The same code, a different boot

⊘ **The triple is not the instrument.** These five lines are GONE:

```
kgmmuStatePostLoad_IMPL: Failed to create GVASpace, status:56
nvAssertFailedNoLog: NV_OK == status @ gpu_vaspace.c:611
nvAssertFailedNoLog: (NV_OK == rmStatus) @ kern_gmmu.c:245
... returned from kgraphicsLoadStaticInfo(...) @ kernel_graphics.c:444   [the CAPS cause]
... and every one of the five GR static-info refusals
```

and one line is new:

```
... NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO @ kernel_graphics.c:743
... returned from kgraphicsInitializeDeferredStaticData(...)     @ kernel_graphics.c:1527
```

## 39.2 ★★★ It answered a question the rung REFUSED to answer

`#150` left `0x20800a32` unserved **on purpose**, and wrote the reason into
`kayfabe_abi::grstatic`'s header before the boot: `kgraphicsShouldDeferContextInit` was
`[assumed]`, and `bInitialized = NV_TRUE` is set at `:1521` *before* the branch that depends
on it, so the two outcomes are distinguishable in one log.

**It does not defer.** ⇒ `0x20800a32` is the sixth mandatory GR control, and refusing it
reaches the same `cleanup:` label as refusing the first.

⊘ The general point, because it generalises: **recording the absence of a measurement beat
borrowing one.** The guess had even odds. Naming it `[assumed]` cost one paragraph; the boot
that settled it was already being spent on something else.

# ★★★ 40. Boot `stateload2` — rev `7819839` — `gpuStateLoad` COMPLETES

| | |
|---|---|
| provenance | `/workspace/kf-stateload` at **`7819839`**, rebuilt, rc 0 |
| **evidence** | **`/workspace/bench/run_stateload2_dmesg.log`** + `_probe.log` + `_qemu.log` |
| result | **`RmInitAdapter failed! (0x11:0x45:2134)`** |

## 40.1 The wall left state-load

`0x11` is **`RM_INIT_SYS_ENVIRONMENT_FAILED`** — *not* in the `0x20` GPU block at all
(`ogkm-580: osinit.c:95-102`). `0x45` is **`NV_ERR_IRQ_NOT_FIRING`**
(`nvstatuscodes.h:98`), and `2134` is the `osVerifySystemEnvironment(pGpu)` call
(`osinit.c:2127`). The log says it in words:

```
NVRM: RmInitAdapter: osVerifySystemEnvironment failed, bailing!
```

⇒ **`RmInitNvDevice` returned `NV_OK`.** *"Cannot load state into the device"* is gone. The
engine walk — `gpuStateInit`, `gpuStateLoad`, `gpuStatePostLoad`, all ~60 engines — is
**through**, and the driver has moved on to a phase that is not about controls at all.

## 40.2 ★★ Why the scrubber's identical-looking failure is survivable and GR's was not

`kernel_fifo.c:3129` still fires. The callback that failed is now the **memory scrubber's**,
not GR's:

```
memmgrScrubHandlePostSchedulingEnable_HAL   @ mem_mgr.c:487
 └ scrubberConstruct                        @ mem_mgr_scrub_gp100.c:63
    └ objCreate(CeUtils)                    @ mem_scrub.c:181
       └ _memUtilsAllocateChannel           @ mem_utils_gm107.c:857
          └ vaspaceGetByHandleOrDeviceDefault @ kernel_channel_group_api.c:224
             └ gvaspaceConstruct_           @ gpu_vaspace.c:611
```

★★★ **The difference is one status code.** GR's handler returned `NV_ERR_INVALID_STATE`
(`0x40`), which `gpu.c:3440` does not swallow. The scrubber's returns
`NV_ERR_NOT_SUPPORTED` (`0x56`), which `gpu.c:3438` **does**. Same assert, same line, same
`NV_ASSERT(0)` — opposite consequence. ⇒ *"`kernel_fifo.c:3129` fired"* is not a finding;
**which status reached it** is the finding, and only the dmesg above distinguishes them.

## 40.3 ★★★ The next wall, named, and it was PREDICTED

`gpu_vaspace.c:611` is back — but at `:5187` and `:4129`, which is the **other branch** of
`gvaspaceCopyServerRmReservedPdesToServerRm`. `#150` served the branch taken when
`resservGetTlsCallContext()` is `NULL` (state load, internal client, `0x20800a9f`). A real
client allocating a channel takes the **first** branch, which issues:

1. `NV_RM_RPC_ALLOC_OBJECT(… FERMI_VASPACE_A = 0x000090f1 …)` — `gpu_vaspace.c:4106-4113`
2. **`NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES = 0x90f10106`** — `:5160-5190`,
   asserted at `:5187`

★ **`0x90f10106` is now in the device's own unserviced list**, and it takes the *identical*
184-byte `NV90F1_CTRL_VASPACE_COPY_SERVER_RESERVED_PDES_PARAMS` that
`kayfabe_abi::gvaspacepdes` already decodes and validates. ⇒ the cheapest rung on the board.

## 40.4 The unserviced ledger — 19 distinct → 20, and five are new

```
kept:  0x20800a87 0x20800a4b 0x20802a08 0x20800afe 0x20800aff 0x20800a70 0x20800a80
       0x20802a0f 0x2080017e 0x20800a2a 0x20800a30 0x20800a2c 0x20800a2e 0x20800a3f
       0x20800a38
new:   0x20800a34  0x20800b03  0x20800b05  0x90f10106  0x2080013f
gone:  0x20800a1f 0x20800a26 0x20800a22 0x20800a3d 0x20800a48 0x20800a32 0x20800a9f
```

⊘ Membership, never cardinality. The seven that left are the seven this rung serves; the five
that entered are reached only *because* state-load completed, so their appearance is the
rung's own evidence.

| | `gmmu1` | `stateload1` | `stateload2` |
|---|---|---|---|
| commands decoded | 64 | 77 | **90** |
| distinct unserviced | 12 | 19 | 20 |
| BAR2 writes through the GMMU | 12 402 | 12 396 | **23 244** |
| framebuffer writes | 46 380 | — | **57 222** |
| interrupt requests **dropped** | 63 | 76 | **89** |

## 40.5 ⊘ What these two boots do NOT establish

- ⊘ **Nothing works.** `nvidia-smi` still prints *"No devices were found"*, `SMI_RC=6`.
- ⊘ **The GR geometry is ACCEPTED, not VALIDATED.** RM read 34 592 bytes of SM order and did
  not complain. Nothing here says it agrees with them — a wrong-but-well-formed geometry
  produces an identical log. The only check on the numbers is
  `kayfabe-abi/tests/gr_static_info.rs`, against the C oracle, which is a different artifact
  and not this boot.
- ⊘ **The golden-image channel was never reached**, either time.
  `_kgraphicsPostSchedulingEnableHandler` got past its `NULL` check at `stateload2` — and
  then the *scrubber's* callback failed first. `kgraphicsCreateGoldenImageChannel` is still
  untested machinery this port has not built.
- ⊘ **The scrubber is AMPUTATED, not working.** State load completed *with* it destroyed. A
  guest that later needs scrubbed memory has not been shown to get it.
- ⊘ **`IRQ_NOT_FIRING` is a product gap, not a discovery.** `interrupt requests dropped 89`
  and the device's own realize-time warning have said so since the first rung. This boot
  proves the driver now gets far enough to *care*, and nothing more.
- ⊘ **One boot each**, on one 4-core box. `#98` records a Mode-2 symptom that was 1/3 one day
  and 9/9 the next on a bit-identical binary.
