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

## 38.1 The measurement — 2026-08-01, on this box, against every rung of that day

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

---

# 41. Boot `irq1` — `[measured]` at `bb4f48d`, `#151`

| | |
|---|---|
| Rust archive | `cargo build --release -p kayfabe-qemu-raw` in `/workspace/kf-irq`, rc 0 |
| revision | `strings /workspace/bench/qemu-build/qemu-system-x86_64 \| grep kayfabe-rev` → **`kayfabe-rev:bb4f48d4de429f370cc341af523b81dbaa4aba97`**, clean (no `-dirty`) |
| link | `scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build`, rc 0 |
| guest | Ubuntu 24.04, **stock unpatched** NVIDIA 580.159.04 open kernel module |
| evidence | `/workspace/bench/run_irq1_dmesg.log` (31 lines, 28 `NVRM`, 3 adapter), `_qemu.log`, `_probe.log` |
| verdict | `RmInitAdapter failed! (0x25:0x1f:1249)` — was `(0x11:0x45:2134)` at `stateload2` |

## 41.1 ★★★ The verdict moved BACKWARDS, and that is the finding

`0x11:0x45` was `RM_INIT_SYS_ENVIRONMENT_FAILED` + `NV_ERR_IRQ_NOT_FIRING`, from
`osVerifySystemEnvironment` — which runs **after** `gpuStateLoad` returns. `0x25:0x1f` is
`RM_INIT_GPU_LOAD_FAILED` + `NV_ERR_INVALID_ARGUMENT`, from `RmInitNvDevice: *** Cannot
load state into the device`. ⇒ **the boot now stops EARLIER in the sequence than it did
before this rung**, and it is not a regression.

⚠ **Serving a control converted a survivable amputation into a fatal error**, and this is
the sweep's own rule running in the direction nobody plans for. `gpu.c:3438` maps
`NV_ERR_NOT_SUPPORTED` → `NV_OK` past `gpuPreInit`; it maps nothing else. At `stateload2`
the scrubber's post-scheduling callback failed with `0x56` (a refused control) and was
**swallowed**; state load "completed" with the scrubber, the CE utility channel and the
device VA space destroyed. Now the same callback gets further and fails with `0x1f`, which
is **not** in the swallowed set — so `kernel_fifo.c:3129` aborts state load for real.

⇒ ★ *"the verdict advanced"* is not a measure of progress on this driver, and *"the verdict
retreated"* is not a measure of regression. Only the cascade in the dmesg is.

## 41.2 What Candidate A established — CONFIRMED CAUSAL by run `irq1` at `bb4f48d`

`0x90f10106` was causal for the VA-space chain, and run `irq1` at `bb4f48d` says so by
**subtraction** against run `stateload2` at `7819839`.
Every one of these lines was in `run_stateload2_dmesg.log:12-22` and is **absent** from
`run_irq1_dmesg.log`:

```
gpu_vaspace.c:5187 / :4129 / :611     ← the RPC, and its two callers
device_share.c:260                    ← vmmCreateVaspace failed
virtual_mem.c:133                     ← vaspaceGetByHandleOrDeviceDefault → 0x56
mem_utils_gm107.c:322                 ← NV50_MEMORY_VIRTUAL alloc failed
kernel_channel_group_api.c:224        ← the TSG's vaspace lookup
```

⇒ **the device default VA space now constructs**, the TSG allocation now gets a VAS, and the
CE utility channel now gets as far as its *method buffer*. `roots published 2` in the
device's own report, up from a boot that published none through this path.

## 41.3 ⊘ What Candidate B established — NOTHING. The path was NEVER REACHED.

```
nvkvm: interrupts: 0 vectors delivered, 0 undeliverable (guest had not enabled the table),
                   86 status-queue requests dropped
```

**Zero.** `CPU_INTR_LEAF_TRIGGER` was never written, because `osVerifySystemEnvironment`
runs after a `gpuStateLoad` that now fails. ⊘ So this boot says **nothing whatsoever** about
whether interrupt delivery works:

- ⊘ not that `msix_notify` reaches the guest;
- ⊘ not that the guest's MSI-X table was enabled when we would have needed it (the
  `undeliverable` counter is 0 because nothing was attempted, **not** because everything
  succeeded — two very different facts behind one zero);
- ⊘ not that the leaf reads back pending in the ISR's own context.

★ What exists is `crates/kayfabe-device/tests/cpu_interrupt_tree.rs`, which replays
`_osVerifyInterrupts` write-for-write against the real `RegPlane` — and that is a test, not
a boot. Per `only_live_boots_are_proof`, the interrupt tree is **INFERRED** (an `ogkm`
reading, tested against itself) and the class is stated rather than borrowed.

★ One thing IS measurable from this boot: `nv.c:1405-1412` prints *"No interrupts of any
type are available. Cannot use this GPU."* when the device offers neither a message-signalled
capability nor a legacy line, and that line is **absent**. This device advertises MSI-X and
nothing else — no `msi_init`, no `PCI_INTERRUPT_PIN` — so `nv_init_msix` succeeded and
`NV_FLAG_USES_MSIX` is set. `[inferred]` from `ogkm-580: kernel-open/nvidia/nv.c:1385-1412`
plus an absence measured in run `irq1` at `bb4f48d`
(`/workspace/bench/run_irq1_dmesg.log` contains no such line) — the strongest form available
without reaching the test, and stated as INFERRED rather than MEASURED because an absent log
line is evidence about the branch, not about the vector.

## 41.4 ★★★ The new wall, and the value this rung REFUSED to invent

```
kernel_ce.c:843                       NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE → 0x56
kernel_channel_group_gv100.c:78       NV_ASSERT((bufSizeInBytes > 0))
kchangrpInit_IMPL                     "Fault method buffer allocation failed … status 0x1f"
kernel_channel_group_api.c:273 → kernel_channel.c:381 → mem_utils_gm107.c:1301 → :857
ce_utils.c:286 → mem_scrub.c:181 → mem_mgr_scrub_gp100.c:63 → mem_mgr.c:487
kernel_fifo.c:3129                    NV_ASSERT(0) — and 0x1f is NOT swallowed
```

`NV2080_CTRL_CE_GET_FAULT_METHOD_BUFFER_SIZE_PARAMS` is **one `NvU32`**
(`ogkm-580: ctrl/ctrl2080/ctrl2080ce.h:283-285`). It is the cheapest struct on the board and
this rung did **not** serve it.

⊘ **Because the oracle does not know the answer either.** The C artifact's captured GA106
init-control table has the row and an EMPTY body:

```c
{0x20802a08u, 0x0u, 4u, 0u, ctl_20802a08},   /* C: src/qemu/mode2_initctrl_ga106.h:6233 */
static const unsigned char ctl_20802a08[] = { };          /*                       :3270 */
```

out-size **0**. There is no captured value from real silicon to copy. And this number is not
inert: RM allocates a buffer of exactly that many bytes, maps it, and programs it into the
channel group as the destination the copy engine's fault records are **DMA'd into**. A
plausible-looking wrong size is not a wrong reply — it is a buffer overrun with a hardware
writer. ⇒ named, left unserved, and the next rung's first question is where a real value
comes from.

## 41.5 ⊘ A second early-out found and REFUSED

`kernel_channel_group_gv100.c:60-72` returns `NV_OK` and skips the whole method-buffer
allocation when any of `IS_GFID_VF`, `RMCFG_FEATURE_PLATFORM_GSP`,
`IS_VIRTUAL_WITHOUT_SRIOV` or `IS_SRIOV_HEAVY_GUEST` holds. `IS_VIRTUAL_WITHOUT_SRIOV` is
reachable from *our* side: it is decided by `NV_PMC_BOOT_1`'s `VGPU` field, which this device
answers. Claiming to be a legacy vGPU would make this wall vanish for free.

⊘ **Rejected, and this one is worse than "a lie about the die" — it is self-defeating.**
`IS_VIRTUAL_WITHOUT_SRIOV` also makes `intrSetStall_TU102` return immediately
(`intr_tu102.c:390-393`) and `intrGetStallInterruptMode_TU102` report `pending = NV_FALSE`
unconditionally (`intr_swintr_tu102.c:124-129`). ⇒ taking this shortcut would make
`_osVerifyInterrupts` **impossible to pass**, permanently, in exchange for skipping one
allocation. The same posture is already recorded in the C tree's
`docs/design/mode2_vgpu_posture_decision.md`: default bare-metal.

## 41.6 The unserviced ledger and the device's own numbers

```
commands: 87 decoded, 22 UNSERVICED, 18 distinct
registers: 3641 reads / 55060 writes (UNCLAIMED 750r/750w), faults 0, guest-RAM refusals 0
BAR2 (translated): 101 reads / 19896 writes resolved, 0 REFUSED; roots published 2
framebuffer: 112 reads / 53874 writes through the BAR0 window, 0 refusals, resident 147456 B
interrupts: 0 delivered, 0 undeliverable, 86 status-queue requests dropped
```

⚠ `18 distinct` unserviced, down from `stateload2`'s 20 — ⊘ and **cardinality is not the
finding**, per §40.4. `0x90f10106` left the list; the boot stops earlier, so the tail of
state load that produced the other new entries is no longer reached.

## 41.7 Triage, in `SWEEP_TRIAGE`'s vocabulary

⚠ No rows were added to `kayfabe_device::sweep::SWEEP_TRIAGE`. Its universe is the **measured
prefix of `cap1b` up to `rpc.sequence` 51**, which sits inside `gpuStateInit`; every control
below is issued during state **load**, hundreds of sequences later. Widening that table with
rows no trace covers would make its own non-vacuity gate a smaller true statement. The
classifications are recorded here, where the evidence is.

| control | disposition | why, and the line |
|---|---|---|
| `0x90f10106` `VASPACE_COPY_SERVER_RESERVED_PDES` | **`RefusalFailsOpen`** | the refusal is invisible — `kernel_fifo.c:3129`'s `0x56` is swallowed at `gpu.c:3438` — **and** the state left behind is wrong: `pGpuGrp->pGlobalVASpace` is assigned before `vaspaceConstruct_` runs (`virt_mem_mgr.c:126` vs `:134`), so a failed construct leaves the group with no device VA space for every later `vaspaceGetByHandleOrDeviceDefault`. `[measured]` by subtraction, §41.2. |
| `0x20802a08` `CE_GET_FAULT_METHOD_BUFFER_SIZE` | **`AmputationUnsurvivable`** | `[measured]` this boot: refusing it does *not* amputate a CE — the zero size becomes `NV_ERR_INVALID_ARGUMENT`, which `gpu.c:3438` does **not** swallow, so state load aborts at `kernel_fifo.c:3129`. ⚠ It was `RefusalIsInvisible`-shaped for four rungs *only because nothing reached it*. ★ See §41.9 — the mechanism stated here was subsequently traced exactly, and one clause of it is wrong. |

⚠ **This is the fourth-and-fifth instance of the lesson that "halts" is a claim about where a
status ENDS UP.** `0x20802a08`'s refusal returns `0x56` at `kernel_ce.c:843` and would read
as survivable from that line alone. ~~It is fatal because a caller **eleven frames up**
converts it into a different status that the swallow does not cover.~~

★★★ **REFUTED 2026-08-01 by the `fmb` rung — see §41.9.** Nothing converts it. The `0x56` is
**discarded** one frame *down*, and an entirely independent `NV_ERR_INVALID_ARGUMENT` is
manufactured eleven frames later out of the zero it left behind. The correction matters
operationally: a search for where `0x56` *propagates* would never have found this, because it
does not propagate anywhere.

## 41.9 ★★★ The chain, traced exactly — and asked of real silicon

`[measured]` boot `irq1` (`/workspace/bench/run_irq1_dmesg.log`) supplies every line below;
`ogkm-580.159.04` supplies the citations. Two independent readings — mine and a second agent
sent at the tree cold — agree on all of it.

| # | site | what happens |
|---|---|---|
| 0 | our emulated GSP | refuses `0x20802a08` → `NV_ERR_NOT_SUPPORTED` |
| 1 | `kernel_ce.c:843-844` | `NV_ASSERT_OK_OR_RETURN` returns `0x56`; `*size` never assigned |
| 2 | `gpu.c:6031-6043` | ★ `gpuGetCeFaultMethodBufferSize_KERNEL` assigns `*size` **only** `if (status == NV_OK)` and then **`return NV_OK;` unconditionally** — the error is *swallowed*, not converted |
| 3 | `kernel_channel_group_gv100.c:77` | the `NV_ASSERT_OK_OR_RETURN` therefore **passes**; `bufSizeInBytes` is still its initialiser `0` (`:44`) |
| 4 | `kernel_channel_group_gv100.c:78` | `NV_ASSERT((bufSizeInBytes > 0))` is a **bare, non-returning** assert — it logs and execution continues |
| 5 | `kernel_channel_group_gv100.c:109` | `memdescCreate(…, Size = 0, …)` |
| 6 | `mem_desc.c:239-241` | **`if (allocSize == 0) return NV_ERR_INVALID_ARGUMENT;`** ← **the `0x1f`, manufactured here** |
| 7 | `kernel_channel_group.c:246-254` → `kernel_channel_group_api.c:273` → `kernel_channel.c:381` → `mem_utils_gm107.c:1301` → `:857` → `ce_utils.c:286` → `mem_scrub.c:181` → `mem_mgr_scrub_gp100.c:63` → `mem_mgr.c:487` → `kernel_fifo.c:3129` | verbatim propagation |
| 8 | `osinit.c:1249` | `RM_INIT_GPU_LOAD_FAILED` = `0x20 + 5` = **`0x25`**, with `0x1f` and line `1249` |

**So the rejected argument is named: `Size`, the length operand of `memdescCreate` — and it
is rejected because it is zero.** Not an engine id, not a runlist, not a handle. The whole
fix is to make `params.size` non-zero and correct.

### The number, and why it had to come from hardware

⊘ There is **no fallback population path**: `pGpu->ceFaultMethodBufferSize` (the cache
`gpu.c:6033` reads) is *never written anywhere in the open tree*, so the control is always
asked. And the answer is in none of our sources —
`subdeviceCtrlCmdCeGetFaultMethodBufferSize_IMPL` is declared and never defined (GSP
firmware), the control is `KERNEL_PRIVILEGED` so no usermode probe may ask it
(`g_subdevice_nvoc.c:7666` `flags=0x1c040`; `control.c:702-709`), and the C oracle's captured
row is **empty**.

★★★ So a real GA106 was asked, and it answered **20480 (0x5000)**. Provenance, both
independent readings, and the six oracle rows this falsified: `traces/real_ga106/README.md`
and `kayfabe_abi::fmbsize`.

⊘ **What §41.9 does NOT establish:** that serving it clears the boot. That is the next boot's
job, and this section is written before it.

## 41.8 ⊘ What boot `irq1` does NOT establish

- ⊘ **Interrupt delivery is UNTESTED.** §41.3. The register model, the shim wiring and the
  `msix_notify` call are built, gated and unit-tested; not one of them ran in a guest.
- ⊘ **`nvidia-smi` still fails.** `SMI_RC` non-zero, no devices.
- ⊘ **The VA space CONSTRUCTS; nothing says it TRANSLATES.** `roots published 2` is a count
  of publications accepted, not of addresses resolved. The CE channel that would have used
  it never allocated.
- ⊘ **The scrubber is still not working** — it now fails *later*, on its channel's method
  buffer instead of on its VA space.
- ⊘ **One boot, one 4-core box.** `#98` records a Mode-2 symptom that was 1/3 one day and
  9/9 the next on a bit-identical binary.

---

# 42. Boot `fmb1` at rev `b965d46` — MEASURED: the fault-method-buffer wall is **GONE**

`[measured]` boot `fmb1`, 2026-08-01, QEMU stamped `kayfabe-rev:b965d46`; evidence on disk at
`/workspace/bench/run_fmb1_dmesg.log`.

## 42.1 Provenance

| | |
|---|---|
| on-disk evidence | `/workspace/bench/run_fmb1_dmesg.log` (29 lines, 26 `NVRM`, 3 adapter), `run_fmb1_probe.log`, `run_fmb1_serial.log` |
| harness | `scripts/bench/boot_capture.sh fmb1` — the device is opened before `dmesg` is read |
| QEMU binary | stamped `kayfabe-rev:b965d46` (`strings … | grep kayfabe-rev`) |
| this branch's HEAD | `12f3a09`. ⚠ **Not the same commit** — the boot happened first, and the branch was amended afterwards (differential pins, a bite harness, doc and claim-attribution edits). The check that keeps the claim honest is mechanical and was re-run on 2026-08-01 against this branch's final content: `git diff -U0 b965d46 HEAD -- 'crates/*/src/*'` changes **no non-comment line** — the three `src` files it touches differ only in doc comments. So the binary that booted is HEAD's behaviour. Stated rather than glossed, per the rule that a bench claim carries the revision it was measured at. |
| box | local dev box, 4 cores. One boot. |

## 42.2 The result

```
[   21.821681] NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0x40:1249)
```

`0x1f` → **`0x40`** (`NV_ERR_INVALID_STATE`). The verdict changed, and so did the reason.

★★★ **The whole `0x20802a08` chain is absent from this log.** Three lines that were in
`irq1` and are gone:

- `… NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE … @ kernel_ce.c:843` — **gone, both
  occurrences** (state init and state load);
- `Assertion failed: (bufSizeInBytes > 0) @ kernel_channel_group_gv100.c:78` — **gone**;
- `kchangrpInit_IMPL: Fault method buffer allocation failed … status 0x1f` — **gone**.

So the channel group now allocates its method buffers and proceeds. That is the rung.

## 42.3 The new wall, decoded before building anything

```
:11  Assertion failed: kgrmgrGetLegacyKGraphicsStaticInfo(…)->pGrInfo != NULL @ kernel_fifo.c:2789
:13  Assertion failed: numMax == numFree && numMax != 0 @ kernel_channel_group_api.c:913
:14  NV_ERR_INVALID_STATE (0x40) returned from kchangrpapiSetLegacyMode(…) @ kernel_channel.c:660
```

The chain, `[inferred]` from `ogkm-580.159.04` and consistent with every line above:

1. `kfifoGetMaxSubcontextFromGr_KERNEL` (`kernel_fifo.c:2778-2792`) returns **0** on
   `NV_ASSERT_OR_RETURN(… ->pGrInfo != NULL, 0)`;
2. so the subcontext ID heap is constructed with size 0, and
   `kchangrpapiSetLegacyMode`'s `NV_ASSERT_OR_RETURN(numMax == numFree && numMax != 0,
   NV_ERR_INVALID_STATE)` (`kernel_channel_group_api.c:913`) fires;
3. `0x40` propagates out of the channel alloc, through `mem_utils_gm107.c:1301` → `:857` →
   `ce_utils.c:286` → `mem_scrub.c:181` → `mem_mgr_scrub_gp100.c:63` → `mem_mgr.c:487` →
   `kernel_fifo.c:3129`. **The same propagation path as before** — only the status and its
   origin changed.

⚠ `pGrInfo` is NULL because `kgrmgrSetLegacyKgraphicsStaticInfo` only copies it `if
(pKernelGraphicsStaticInfo->pGrInfo != NULL)` (`kernel_graphics_manager.c:390-397`), and the
per-`KernelGraphics` copy is filled from
**`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO` (`0x20800a2a`)** at
`kernel_graphics.c:1231`. That control is currently triaged `AmputationIntended`.

★ **The next rung already has hardware for it, and also already knows why that is not
enough:** `traces/real_ga106/rpc_transcript_real_ga106.txt` carries
`cmd=0x20800a2a psize=3712 gspst=0x0 head=00 00 00 00 01 00 00 00` — a real GA106 answers it
`NV_OK` with 3712 bytes, of which the transcript captured **8**. Serving it needs a re-measure
with a full body dump; the recipe is in `traces/real_ga106/README.md`.

★★ **Done, in §43.** The re-measure was taken (`traces/real_ga106/rpc_bodies_real_ga106.txt`,
all 3712 bytes, twice in one run), the control is served, and this wall is gone.

## 42.4 ⊘ What boot `fmb1` does NOT establish

- ⊘ **It does not establish that 20480 is right** — only that it is non-zero and that RM
  accepted the allocation. A wrong-but-plausible size would look identical here; the reason
  to believe the number is that a real GA106 was asked, not that this boot got further.
- ⊘ **Nothing was written into the fault method buffer by anything.** No engine faulted.
- ⊘ **The scrubber still does not work.** It fails one step later — on its channel's
  subcontext heap instead of on its method buffer.
- ⊘ **Interrupt delivery is still untested** (§41.3 stands unchanged).
- ⊘ **`nvidia-smi` still fails**, no devices.
- ⊘ **One boot, one 4-core box**, and `#98` records a Mode-2 symptom that was 1/3 one day and
  9/9 the next on a bit-identical binary.

---

# §43 — boot `grinfo1`, rev `6b27c1f`: GR's info list is accepted, and the channel now ALLOCATES

**Provenance.** `[measured]` 2026-08-01 16:16 CEST, `scripts/bench/boot_capture.sh grinfo1`,
this 4-core box. Guest: stock, unpatched NVIDIA open 580.159.04. QEMU: the QOM shim built
from `libkayfabe_qemu_raw.a` at **`6b27c1f`** — the revision is stamped in
`/workspace/bench/run_grinfo1_probe.log` line 2 and was read back from it, not assumed.
Evidence on disk: `/workspace/bench/run_grinfo1_dmesg.log` (25 lines, 22 `NVRM`, 3
`RmInitAdapter`).

## 43.1 The rung cleared

```
[   24.599532] NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0xffff:1249)
```

`0x40` → **`0xffff`** (`NV_ERR_GENERIC`). ★★★ **The whole `pGrInfo` chain is absent.** Four
lines that were in `fmb1` and are gone:

- `Assertion failed: … ->pGrInfo != NULL @ kernel_fifo.c:2789` — **gone, all three
  occurrences**;
- `Assertion failed: numMax == numFree && numMax != 0 @ kernel_channel_group_api.c:913` —
  **gone**;
- `NV_ERR_INVALID_STATE (0x40) returned from kchangrpapiSetLegacyMode(…) @
  kernel_channel.c:660` — **gone**;
- `… returned from pRmApi->AllocWithHandle(… hChannelId, hClass,
  &channelGPFIFOAllocParams …) @ mem_utils_gm107.c:1301` — **gone**.

⇒ the scrubber's **channel is now allocated**. `mem_utils_gm107.c:1301` is the allocation
call itself, and it no longer fails. That is the rung, and it is a bigger step than the
status change suggests: `kfifoGetMaxSubcontextFromGr_KERNEL` now returns 64 instead of 0, the
subcontext ID heap is constructed at that size, and a `KEPLER_CHANNEL_GPFIFO` object exists.

## 43.2 The new wall, decoded before building anything

```
[   24.318677] NVRM: _memmgrMemUtilsScrubInitScheduleChannel: Unable to schedule channel, status: 56
[   24.318844] NVRM: … NV_ERR_GENERIC (0xFFFF) returned from
               _memmgrMemUtilsScrubInitScheduleChannel(pGpu, pChannel) @ mem_utils.c:2006
```

`[inferred]` from `ogkm-580.159.04`, and the identification is unambiguous:
`_memmgrMemUtilsScrubInitScheduleChannel` issues exactly one control —
**`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`0xa06f0103`)**, `mem_utils.c:1976-1981` — and turns any
non-`NV_OK` into a bare `NV_ERR_GENERIC` (`:1985-1988`). The `0x56` in the message is this
port's own refusal, and the device's unserviced list for this boot carries `fn 76 cmd
0xa06f0103` to confirm it. The propagation path beyond that point is **unchanged** from
`fmb1`: `mem_utils_gm107.c:1027` → `ce_utils.c:304` → `mem_scrub.c:181` →
`mem_mgr_scrub_gp100.c:63` → `mem_mgr.c:487` → `kernel_fifo.c:3129`.

★★★ **The new wall is a member of the class this rung measured.** `0xa06f0103` is one of the
eleven `dlen = 0` rows of `mode2_initctrl_ga106.h`, and it is one of the nine hardware
contradicted: a real GA106 answers `psize 3` with `01 00 00`
(`traces/real_ga106/rpc_bodies_real_ga106.txt`). A port that had read the empty row as
"three zero bytes" would have served `bEnable = FALSE`.

⚠ **And that is still not a reason to serve it, which is the finding.**
`NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS` is `{ bEnable, bSkipSubmit, bSkipEnable }` — all `[IN]`
(`ogkm-580: ctrla06fgpfifo.h:30-70`) — so `01 00 00` is the *guest's own request echoed
back*, and the load-bearing half of the reply is the **status**, not the body. This control
is an **action on the FIFO**, not a description of silicon: an `NV_OK` would tell the guest a
channel is running on a host that has scheduled nothing. It is the `0x20800a6c` question
again with a much harder answer, and it belongs to the execution-plane rung rather than to
this one.

## 43.3 ⊘ What boot `grinfo1` does NOT establish

- ⊘ **It does not establish that the 58 GR info values are RIGHT** — only that RM accepted
  them and that `infoList[0x2c]` was non-zero. The reason to believe the table is that a real
  GA106 was asked and agreed on all 3712 bytes, not that this boot got further.
- ⊘ **It does not establish anything about the other ten empty rows.** Nine of them are still
  refused by this port; this boot exercised none of them.
- ⊘ **The scrubber still does not work.** It now fails one step later — on *scheduling* its
  channel rather than on *allocating* it.
- ⊘ **Nothing has executed.** A channel object exists; no pushbuffer has been fetched, no
  semaphore released.
- ⊘ **Interrupt delivery is still untested** (§41.3 stands unchanged).
- ⊘ **`nvidia-smi` still fails**, no devices.
- ⊘ **One boot, one 4-core box**, and `#98` records a Mode-2 symptom that was 1/3 one day and
  9/9 the next on a bit-identical binary.

# §44 — boot `schedprobe1`: the schedule wall CONFIRMED by removing it, and a hang prediction REFUTED

**Provenance.** `[measured]` 2026-08-01, this 4-core box. Source revision **`0bf7eb7`
(`origin/master`) plus a throwaway serve arm for `0xa06f0103` that was NEVER LANDED** — it
lives only in a stash on branch `w154-gpfifo-sched` and in
`scripts/`-adjacent scratch, and the bench worktree was restored to `6b27c1f` afterwards.
On-disk evidence: `/workspace/bench/run_schedprobe1_dmesg.log` (25 lines, 22 `NVRM`, 3
adapter lines), alongside the baseline `/workspace/bench/run_grinfo1_dmesg.log`.

⊘ **This boot is a PROBE, not a rung.** Its purpose was to measure the consequence of a
fabricated completion, so that the decision to refuse `0xa06f0103` rests on an observation
rather than on a prediction. Nothing from it is shipped.

## 44.1 The result — the wall moves exactly one step

The two dmesg logs differ in **two lines and nothing else** (timestamps stripped):

```text
- NVRM: _memmgrMemUtilsScrubInitScheduleChannel: Unable to schedule channel, status: 56
- NVRM: ... NV_ERR_GENERIC ... from _memmgrMemUtilsScrubInitScheduleChannel @ mem_utils.c:2006
+ NVRM: _memmgrMemUtilsScrubInitRegisterCallback: event notification control failed
+ NVRM: ... NV_ERR_GENERIC ... from _memmgrMemUtilsScrubInitRegisterCallback @ mem_utils.c:2022
```

The verdict line is **unchanged**: `RmInitAdapter failed! (0x25:0xffff:1249)`.

⇒ Two things are established at once. **`0xa06f0103` really was the wall** — the step it
gates is `mem_utils.c:2006`, and removing the refusal advances the boot past it to the next
statement in `memmgrMemUtilsChannelSchedulingSetup_IMPL`. And **the verdict line is useless
for locating a wall**: it was identical before and after a real change of position, which is
the third time tonight a printed message has pointed somewhere other than the cause.

## 44.2 ★★★ The prediction this refutes — mine

The triage row for `0xa06f0103` was first drafted arguing that serving `NV_OK` would
**hang**: the scrubber would submit CE work and wait on a semaphore no engine would ever
release. **That is false, and the boot says so.** It does not hang. At least two further
setup steps stand between the schedule and any submission —
`_memmgrMemUtilsScrubInitRegisterCallback` (`mem_utils.c:2022`, the one now failing) and
`kfifoRmctrlGetWorkSubmitToken_HAL` (`:2024`) — and each is its own unbuilt control.

⊘ **This makes the case for refusing stronger, not weaker.** A lie that fails loudly on the
next line is cheap; a lie whose cost is *deferred* past two more rungs is the shape that
produces a hang attributed to the wrong subsystem. The first consumer that genuinely waits
is `memmgrTestCeUtils`, which memsets and copies through the CE and then compares the
read-back (`ogkm-580: mem_mgr.c:407-470`, called at `:4158`) — several rungs downstream of
where the fabricated `NV_OK` would have been introduced.

## 44.3 Why this control cannot be answered at all

`kchannelCtrlCmdGpFifoSchedule_IMPL` does two pieces of bookkeeping on the CPU-RM side —
`kchannelIsSchedulable_HAL` (which passes here; it fails with `NV_ERR_INVALID_STATE`, not
`NOT_SUPPORTED`) and `kchannelSetRunlistSet` — and then RPCs to GSP under the comment
**"All real hardware management is done in the host"** (`ogkm-580:
kernel_channel.c:3105-3130`). Export flags are `0x10008` = `NON_PRIVILEGED |
GSP_PLUGIN_FOR_VGPU_GSP`; there is no `ROUTE_TO_PHYSICAL` and no `INTERNAL`, so the control
is dispatched kernel-side and hand-rolls its own RPC.

**We are the GSP.** The runlist write, the RAMFC update and the runlist submit are all on
our side of that line, and none of them exists. So the postcondition — *work submitted to
this channel executes* — is plainly false here, and unlike `0x20800a6c` there is no
structural argument that makes it vacuously true. `0x20800a6c` is served because its
postcondition holds by construction **and** a caller checks it; this control has the checking
caller and not the structural argument, which is the quadrant where the only honest moves are
to perform the action or to refuse.

⇒ **`0xa06f0103` is an execution-plane rung and cannot be closed by a reply.** It is triaged
`RefusalHalts` in `kayfabe_device::sweep`, with the argument and both boots recorded there.

## 44.4 ⊘ What boot `schedprobe1` does NOT establish

- ⊘ **It does not establish that serving `0xa06f0103` is safe.** It establishes that the
  damage is not immediate. Those are different claims and the second is the weaker one.
- ⊘ **It says nothing about `mem_utils.c:2022`'s own control.** The next wall was observed,
  not diagnosed; which control "event notification control failed" refers to, and whether it
  is `0x20800301` or a channel-scoped event, is unanalysed here.
- ⊘ **Nothing executed.** No pushbuffer was fetched, no semaphore released, no CE ran. The
  probe moved a failure; it did not make anything work.
- ⊘ **It does not establish the absence of a hang further down** — only that the hang is not
  where this rung is.
- ⊘ **One boot, one 4-core box**, and `#98` records a Mode-2 symptom that was 1/3 one day and
  9/9 the next on a bit-identical binary.

---

# §44 — boot `evtprobe1` (a PROBE, never landed): `mem_utils.c:2022` diagnosed, and the four controls behind it shown to be ONE wall

**Provenance.** `[measured]` 2026-08-01, `scripts/bench/boot_capture.sh evtprobe1`, this
4-core box. Guest: stock, unpatched NVIDIA open 580.159.04. QEMU: the QOM shim built from
`libkayfabe_qemu_raw.a` at **`4e93f17` + a throwaway probe** — the binary stamped
`kayfabe-rev:4e93f178…-dirty` and `nm` found the probe's two symbols in the exact archive
the shim links, both read back rather than assumed. Evidence on disk:
`/workspace/bench/run_evtprobe1_dmesg.log` (46 lines, 43 `NVRM`),
`/workspace/bench/run_evtprobe1_qemu.log` (the device's own ledger),
`/workspace/bench/build155probe.log`.

⊘ **The probe is NOT in the tree and must never be.** It fabricated three completions:
`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`0xa06f0103`) → `NV_OK`;
`NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` (`0xc36f0108`) → an **invented** token;
and an extra `SILENT_NOTIFIERS` row for index 35. The bench was restored to clean
`4e93f17` afterwards and **verified** `[measured]` 2026-08-01, by reading the artifact back:
the rebuilt binary stamps `kayfabe-rev:4e93f178…` with no `-dirty` suffix, and
`nm -C /workspace/bench/kayfabe-wt/target/release/libkayfabe_qemu_raw.a | grep -ci probe155`
returns **0** where it returned **2** for the probe build
(`/workspace/bench/build155restore.log`).

## 44.1 What `mem_utils.c:2022` requires, and how the statement was located

`mem_utils.c:2022` is `NV_ASSERT_OK_OR_RETURN(_memmgrMemUtilsScrubInitRegisterCallback(...))`.
That function has **three** failure returns and **all three substitute `NV_ERR_GENERIC`**
(`:1884`, `:1915`, `:1933`), so the status cannot distinguish them. The **printed message**
can: the log says `"event notification control failed"`, which is `:1932` alone. ⇒ the NV0005
event alloc at `:1904` **succeeded**, and the failing statement is the
`NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` at `:1923`.

It requires this port to answer that control, and it reaches us: despite carrying **no**
`ROUTE_TO_PHYSICAL` flag (`flags = 0x10118` = `NON_PRIVILEGED | GPU_LOCK_DEVICE_ONLY |
API_LOCK_READONLY | GSP_PLUGIN_FOR_VGPU_GSP`, `g_subdevice_nvoc.c:1601-1614`),
`subdeviceCtrlCmdEventSetNotification_IMPL` **hand-rolls** an unconditional forward under
`if (IS_FW_CLIENT(pGpu))` and `NV_CHECK_OK_OR_RETURN`
(`subdevice_ctrl_event_kernel.c:108-118`) — which returns our status **before** any local
state is touched. ⚠ A flag-only reading of this control would have concluded it never
reaches the GSP.

★★★ **This port serves `0x20800301`, and refused it anyway** — the arming is index **35**,
`NV2080_NOTIFIERS_FIFO_EVENT_MTHD` (`cl2080_notification.h:72`), and `SILENT_NOTIFIERS`
holds only index 194. That is the design working as specified: §12 wrote the promise as a
*list* precisely so "completion notifiers whose silence is a hang nobody could attribute"
would produce a loud boot.

⚠ **And the refusal is INVISIBLE in the unserviced ledger.** `InitTablePolicy::refuse()`
returns `Some(Reply)`, so a control that reaches a `WantedTable` arm and fails a gate is
*answered* and never recorded. `schedprobe1`'s ledger is `grinfo1`'s minus one id; the wall
that boot actually hit appears nowhere in it. **Diffing ledgers alone cannot find this class
of wall.**

## 44.2 Index 35 IS defensible — on a different argument, and it is recorded rather than taken

`NV2080_NOTIFIERS_FIFO_EVENT_MTHD` occurs in **exactly three places in the whole driver**:
`event_notification.c:482` (the switch mapping it to `RM_ENGINE_TYPE_HOST` on the **nonstall**
path) and `mem_utils.c:1901` / `:1920`. It is therefore only ever registered *with*
`NV01_EVENT_NONSTALL_INTR`, by one caller. And the nonstall delivery path
(`_gpuEngineEventNotificationListNotify`) keys on `engineNonstallIntrEventNotifications[]`,
built by the **NV0005 alloc**, never on `pSubdevice->notifyActions[]` — whose only readers are
`gpu_rmapi.c:572` and `:593`, and **no caller anywhere passes 35 to
`gpuNotifySubDeviceEvent`** — `[inferred]` `ogkm-580.159.04`, from an exhaustive grep of
`NV2080_NOTIFIERS_FIFO_EVENT_MTHD` (3 hits, all named above) and of `gpuNotifySubDeviceEvent`
(20 call sites, none passing 35).

⇒ index 194's row says *the event cannot occur*; index 35's would say *the arming is never
read*. Both are honest, and 35's does **not** expire when the execution plane lands.

⊘ **It is not taken.** `0xa06f0103` refuses **fourteen lines earlier in the same function**
(`:2006` < `:2022`), so landing the row alone moves nothing. It belongs to the set below.

## 44.3 Three lies bought one step, into the same wall — `[measured]` boot `evtprobe1`, rev `4e93f17` + probe

`[measured]` 2026-08-01, boot `evtprobe1` at rev `4e93f17` + the throwaway probe;
`/workspace/bench/run_evtprobe1_dmesg.log` and `run_evtprobe1_qemu.log`, diffed against
`run_schedprobe1_*` and `run_grinfo1_*` on the same disk.

With all three faked, `mem_utils.c:2022` **clears**. Absent from the new log, present in
`schedprobe1`'s: `_memmgrMemUtilsScrubInitRegisterCallback`, `mem_utils.c:2022`,
`mem_scrub.c:181`, `mem_mgr_scrub_gp100.c:63`, `mem_mgr.c:487` — **the scrubber's entire
chain**. Present and new: `mem_mgr.c:4155` and `mem_mgr.c:526`, which is the **global**
`CeUtils`, strictly later in `memmgrInitInternalChannels_IMPL` (`:526` > `:487`).

Its channel takes the `bUseVasForCeCopy` arm the scrubber's did not
(`mem_utils.c:1953-1971`), whose control is dispatched CPU-side into `kchannelBindToRunlist`
and **RPC'd straight back to us** under `NV_ASSERT_OK_OR_RETURN`
(`kernel_channel.c:2878-2886`, from `:3230`):

```
NVRM: … NV_ERR_NOT_SUPPORTED (0x56) … @ kernel_channel.c:2886
NVRM: … kchannelBindToRunlist(…) @ kernel_channel.c:3230
NVRM: _memmgrMemUtilsScrubInitScheduleChannel: Unable to bind Channel, status: 56
NVRM: … NV_ERR_NOT_SUPPORTED (0x56) … @ mem_utils.c:2006
```

★★ **The substitution trick identifies the site again, in the other direction.** The status
is `0x56` **verbatim** — `mem_utils.c:1969` returns `rmStatus` — where the schedule arm
(`:1986`) and the callback arm (`:1933`) both substitute `0xFFFF`. The new wall is
**`NVA06F_CTRL_CMD_BIND` (`0xa06f0104`)**, and the device's own ledger carries
`unserviced fn 76 cmd 0xa06f0104` to confirm it.

### How movement was determined WITHOUT the verdict line

The verdict *did* change (`0x25:0xffff:1249` → `0x43:0x59:2239`), but §43's lesson is that it
cannot locate a wall — two different failure points produced the identical line. The
independent evidence, from the device's own ledger:

| | `grinfo1` | `schedprobe1` | `evtprobe1` |
|---|---|---|---|
| commands decoded | 92 | 95 | **137** |
| distinct unserviced | 17 | 16 | **20** |
| `0xa06f0103` refused | yes | no (probe) | no (probe) |
| `0xa06f0104` refused | no | no | **yes** |

★ The guest asked **42 more questions than it ever had**, and **four control ids appear that
no previous boot ever reached** (`0xa06f0104`, `0x2080013f`, `0x2080012b`, `0x402c0101`).
A driver that asks new questions has run new code. That is a positional argument in the
driver's own source order, and it uses the verdict code nowhere.

## 44.4 ★★★ The finding: four controls, one wall — and a fifth thing that is not a control

`0xa06f0103` (schedule), `0xa06f0104` (bind), `0xc36f0108` (work-submit token) and the
index-35 arming at `mem_utils.c:1920` are **not four rungs of a ladder**. They are one
requirement — *put a channel on a runlist, arm its completion, hand back its doorbell* —
asked four times, by two different channels. The probe answered three and did not reach a new
**kind** of wall; it reached the fourth. Serving them individually fabricates completions one
at a time and the boot walks from each to the next.

And `[inferred]` from source, immediately past all four: `memmgrInitCeUtils_IMPL:4158` calls
**`memmgrTestCeUtils`** unconditionally, which writes `0xAABBCCDD` to framebuffer, issues
`memmgrMemCopy(… TRANSFER_FLAGS_PREFER_CE)`, reads it back, and asserts
`sysmemData == vidmemData` (`mem_mgr.c:407-478`). **A functional end-to-end test of the Copy
Engine with a read-back comparison.** There is no control-shaped answer to it.

Its status returns through `:4165` → `:526` `NV_ASSERT_OK_OR_RETURN` →
`memmgrPostSchedulingEnableHandler` → `kernel_fifo.c:3111`, whose `else` arm is
`NV_ASSERT(0); break;` (`:3126-3131`) — **fatal for any non-`NV_OK` status**. ⇒ ★★ the
sweep's *"`NV_ERR_NOT_SUPPORTED` = amputate and carry on"* rule **does not apply in this
phase**.

### Every escape hatch, closed by citation

- `PDB_PROP_GPU_REUSE_INIT_CONTING_MEM` (skips the test, `:421-427`) — set `NV_TRUE` for
  **Blackwell chips only**; GA106 takes the `else` (`g_gpu_nvoc.c:497-506`).
- `!IS_SILICON(pGpu)` (`:503`, skips the global `CeUtils` *and* scrub-on-free) — ★★★ **the
  lever does not exist.** `IS_SILICON` = `!(IS_EMULATION || IS_SIMULATION)`; `bIsSimulation`,
  `bIsFmodel` and `bIsRtlsim` are **declared in `g_gpu_nvoc.h` and assigned nowhere in the
  tree**, and `PDB_PROP_GPU_EMULATION` is never `setProperty`'d either (its only occurrence is
  one read, `common_nvlinkapi.c:618`). `IS_SILICON` is structurally constant `TRUE`. This is
  not a lie we decline to tell; it is a lie we *cannot* tell.
- `bDisableGlobalCeUtils` (`:500`) — set only from the guest regkey
  `NV_REG_STR_DISABLE_GLOBAL_CE_UTILS` (`mem_mgr.c:341-345`). Requires modifying the guest ⇒
  outside "stock, unpatched".
- `IS_VIRTUAL(pGpu) && !IS_VIRTUAL_WITH_FULL_SRIOV` (`:502`) — the standing
  `IS_VIRTUAL_WITHOUT_SRIOV` refusal: it makes `intrGetStallInterruptMode` report
  `pending = FALSE` unconditionally, i.e. the interrupt self-test permanently unpassable.
- The scrubber's own gate `memmgrIsScrubOnFreeEnabled` (`mem_mgr_scrub_gp100.c:52`) — cleared
  only by Windows / MODS / `IS_RTLSIM` / `IS_FMODEL` / `IsDFPGA` / vGPU-host /
  `IS_VIRTUAL_WITHOUT_SRIOV` / GSP-platform build / SLI (`mem_mgr_gm107.c:1473-1482`). None
  available, and the three simulation ones are the unreachable lever above.

⇒ **This needs the execution plane, and it cannot be faked.** That is the result.

## 44.5 ⊘ What boot `evtprobe1` does NOT establish

- ⊘ **It does not license serving any of the four controls.** It was produced *by* lying
  about three of them. Nothing here is a reason to land those lies.
- ⊘ **It does not establish that `memmgrTestCeUtils` is the next wall.** The boot died at
  `objCreate(CeUtils)` (`mem_mgr.c:4155`), **before** `:4158`. The CE read-back test is
  `[inferred]` from source and has **never been reached by any boot**.
- ⊘ **It does not establish that index 35 is safe to serve.** The argument in §44.2 is a
  source reading; the boot used it but did not test it — with `0xa06f0103` faked, a wrong
  answer about the arming would not have shown.
- ⊘ **A SECOND refusal in the same frame is observed and NOT attributed**:
  `GspRmAlloc … hClass=0x0000c56f … status=0x56` (`AMPERE_CHANNEL_GPFIFO_A`), with the
  device reporting one `GpuError::Projection` — so the class is permitted and mapped and the
  failure is in our object model. It is **not** claimed as a cause: the propagating chain
  reaches `ce_utils.c:304` (`memmgrMemUtilsCopyEngineInitialize`), not `:286`
  (`memmgrMemUtilsChannelInitialize`), so channel init returned `NV_OK` despite it.
  ⚠ Adjacency in a log is not a mechanism. This is the next rung's question, not this one's.
- ⊘ **Nothing executed.** No pushbuffer fetched, no semaphore released, no byte copied.
- ⊘ **Interrupt delivery is still untested**; the device dropped 136 status-queue requests.
- ⊘ **`nvidia-smi` still fails**, no devices.
- ⊘ **One boot, one 4-core box, one guest**, and `#98` records a Mode-2 symptom that was 1/3
  one day and 9/9 the next on a bit-identical binary.
