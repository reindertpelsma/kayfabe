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
