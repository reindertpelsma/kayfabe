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
