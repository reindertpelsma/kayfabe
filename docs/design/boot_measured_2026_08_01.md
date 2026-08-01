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

## 3. ★★★ The new wall, and it is a different KIND of thing

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
