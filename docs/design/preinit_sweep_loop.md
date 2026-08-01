# The engine sweep — what replaces one-boot-one-rung

**Status:** current as of the `t134a` boot (`1c79474`) and the rung that followed it.
**Audience:** whoever picks up the bring-up ladder next and is about to budget one boot per
control.

⊘ **Do not budget one boot per control. That loop is over, and this document exists because
its replacement is not obvious from the ledger alone.**

---

## 1. What the old loop was, and why it worked

Six boots, six rungs, one new control each:

| run | revision | `commands` | distinct unserviced | the guest's own `LEVEL_ERROR` |
|---|---|---|---|---|
| `t127a` | `f870288` | 3 | **1** — fn 1 | `kgspInitRm_IMPL: SET_GUEST_SYSTEM_INFO failed: 0x56` |
| `t127b` | `0db7c61` | 5 | **1** — fn 228 | `kgspInitGspTraceCrashBuffer … kernel_gsp.c:4239` |
| `t127c` | `110c857` | 6 | **1** — `0x20800a36` | `_gpuInitChipInfo … gpu.c:886, 2124` |
| `t132a` | `f83ce31` | 7 | **1** — `0x20800a41` | `gpuConstructUserRegisterAccessMap … gpu.c:2125` |
| `t133a` | `c88f803` | 8 | **1** — `0x208001b0` | `gpuBuildGenericKernelFalconList … gpu.c:5368, 2126` |
| `t134a` | `1c79474` | **27** | **6** | `gpuConstructDeviceInfoTable_HAL … kernel_fifo.c:2208`, then a kernel `Oops` |

The loop was: *serve the one control the ledger names → boot → read the one it names next.*
It worked because the first five were all reached from **`gpuPreInit`**, which is a chain of
`NV_ASSERT_OK_OR_RETURN`s (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:2121-2126`). The first
refusal ends the function. Exactly one control can be *reached* per boot, so the ledger
could not have listed two.

★ Note the third column of rows 3-5: `:2124`, `:2125`, `:2126`. The ledger and the guest
agreed not merely on which control but on the **adjacent source statement** that consumed
it. Three consecutive boots (`t127c` at `110c857`, `t132a` at `f83ce31`, `t133a` at
`c88f803`, each a stock 580.159.04 guest) agreed that way. ⊘ That agreement is what made
"one boot, one rung" look like a property of *refusing by name*. It was a property of
`gpuPreInit`'s shape, and nothing measured it as anything else — the reading that would
have said so is `ogkm-580: gpu.c:2121-2126`, and it was not made until `t134a` forced it.

---

## 2. What changed at `t134a`

`gpuBuildGenericKernelFalconList` was the **last** statement in that chain. Past it the
guest enters `gpuStatePreInit_IMPL`'s engine sweep
(`ogkm-580: gpu.c:2152-2219`), and the sweep has a different disposition table:

| an engine's `StatePreInit` returns | what the sweep does |
|---|---|
| `NV_OK` | next engine |
| **`NV_ERR_NOT_SUPPORTED`** | **destroys the engine object, NULLs `pGpu`'s pointer to it, resets `rmStatus` to `NV_OK`, and continues** |
| anything else | `break` — the boot aborts and propagates |

★★★ **`NV_ERR_NOT_SUPPORTED` is not an error to this loop. It is an engine-removal
request.** And it is this port's default answer to every control it does not serve
(`kayfabe_gsp::GspFsm::answer`). So the moment the guest left `gpuPreInit`, every unserved
control stopped being a boot-stopper and became a **silent amputation of whichever engine
asked for it**.

That is the whole of the change. `commands` went 8 → 27 and distinct unserviced went 1 → 6
because the sweep kept walking, not because the port got worse.

### Three details that each cost time to establish

- ⚠ **The message lies.** The `default:` arm prints
  `"disallowing NV_ERR_NOT_SUPPORTED PreInit removal of untracked engine (%s)"` at
  `LEVEL_ERROR` (`gpu.c:2199-2205`) and then `break`s out of the *switch* and falls into
  `gpuDestroyMissingEngine` at `:2208` **anyway**. Nothing is disallowed. On a release
  module neither guard on that arm has any effect: `DBG_BREAKPOINT()` is defined empty
  (`ogkm-580: src/nvidia/inc/kernel/core/printf.h:153`) and `NV_ASSERT(0)` expands to a
  logging macro documented as having *"no other action"*
  (`ogkm-580: src/nvidia/inc/libraries/utils/nvassert.h:336`).
- ⚠ **The sweep reports success afterwards.** `:2211` overwrites `rmStatus` with
  `gpuDeleteEngineOnPreInit`'s `NV_OK`, so `gpuStatePreInit_IMPL` returns `NV_OK` (`:2229`)
  having amputated engines, and `gpumgrStatePreInitGpu` records success
  (`ogkm-580: src/nvidia/src/kernel/gpu_mgr/gpu_mgr.c:2024`).
- ⚠ **`gpuStateInit` is looser still.** Its per-engine loop maps `NV_ERR_NOT_SUPPORTED` to
  `NV_OK` and does **not** remove the engine (`gpu.c:2286-2287`), so the object survives
  *unconstructed*. A NULL check passes and the garbage is used. PreInit at least NULLs the
  pointer.

### The damage is displaced, and that is the real cost

`[measured]` run `t134a`, a stock 580.159.04 guest at `1c79474`: `KernelMemorySystem` was
amputated at `ogkm-580: kern_mem_sys.c:122`, the boot continued, and the guest died several thousand
lines later in a different subsystem:

```
BUG: kernel NULL pointer dereference, address: 0000000000000268
RIP: memmgrGetBlackListPagesForHeap_GM107+0x23/0x140 [nvidia]
  heapInit_IMPL ← memmgrCreateHeap_IMPL ← memmgrStateInitLocked_IMPL ← gpuStateInit_IMPL
```

`nvidia-smi` **hangs** rather than failing, because the Oops kills the `nv_open_q` kthread
mid-`RmInitAdapter`. ⊘ That is a *different observable* from every earlier rung and must not
be read as "progress stalled" — it is the guest kernel damaged, not the guest kernel
refusing.

---

## 3. ⊘ The escape hatch that is not one

The obvious reaction is: *refuse with a status other than `NV_ERR_NOT_SUPPORTED`, so the
sweep `break`s instead of amputating*. It was considered and rejected, and the reasoning is
recorded so nobody re-derives it:

1. It does not boot. `gpu.c:2215-2218` propagates the error, `gpuStatePreInit` returns it,
   `RmInitAdapter` fails. It trades a crash for a clean early abort — the right *diagnostic*
   posture, and no closer to a running GPU.
2. ★★ **It would destroy the one thing the sweep gave us.** A break-on-first-refusal default
   restores one-rung-per-boot. The sweep's compensation for losing attribution is that a
   single boot now enumerates *every* control the whole engine list wants. That is worth
   more than the clean abort.
3. `NV_ERR_NOT_SUPPORTED` is also what the driver's *own* unimplemented-control stub returns
   (`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.h:7376-7379`). Answering differently
   would make this port's refusals distinguishable from a real GSP's, in a direction nobody
   asked for.

⊘ So the answer to an amputation is to **serve the control**, not to refuse it differently.

---

## 4. ★★★ The new loop

The unit of progress is no longer *a control*. It is **an engine surviving its
`StatePreInit`** — because that is the unit the sweep acts on. Serving two of an engine's
three controls buys nothing: the engine is still deleted.

### 4.1 Pre-flight the sweep offline, before booting

Three sources answer "what will be asked" with no bench at all:

1. **The engine order is a static list.** `gpuStatePreInit` walks
   `gpuGetInitEngineDescriptors`, ordered by `gpuChildOrderList_GM200`
   (`ogkm-580: src/nvidia/src/kernel/gpu/arch/maxwell/kern_gpu_gm107.c:605`, accessor at
   `:706`) — the array every discrete Turing-through-Blackwell part uses, with its own
   comment *"DO NOT FORK THIS LIST"*. The INIT-phase order around where we are now:

   | idx | engine | its control |
   |---|---|---|
   | 1 | `ConfidentialCompute` | `0x20800af3` (StateInit) |
   | 5 | `KernelBif` | `0x20800aac` (StateInit) |
   | 15 | `Intr` | `0x20800a5c` — served |
   | 21 | **`KernelMemorySystem`** | **`0x20800a1c`** |
   | 22 | `MemoryManager` | — |
   | 24 | `KernelNvlink` | `0x20800a87` |
   | 48 | `KernelHFRP` | `GPU_GET_HFRP_INFO` |
   | 50 | `KernelDisplay` | `0x20800a4b` |
   | 79 | `KernelFifo` | `0x20800a40` (StateInit) |

   Each engine's `StatePreInit`/`StateInitLocked` is greppable, and its RM controls with it.
   ⇒ **The next several rungs can be enumerated without spending a boot on discovery.**

2. **The `unserviced` ledger** (`crates/kayfabe-device/src/unserviced.rs`) names every
   refusal in one run. What it lost is *attribution* — which caller wanted which control —
   and source 1 supplies exactly that. Ledger + engine list together are complete; neither
   is alone.

3. **The `cap1b` differential** (`crates/kayfabe-crec/tests/cap1b_differential.rs`) replays
   the C oracle's own boot and closes at txn 1028 / `rpc.sequence` 51. Anything inside that
   prefix gets **reply-plane coverage with no bench at all** — the only pre-boot check that
   a served reply is byte-right rather than merely present.

   ⚠ And it is a *gate*, not just a convenience:
   `every_control_this_port_serves_is_exercised_by_the_replay` asserts
   `reached == WantedTable::ALL`. **A served control the capture never reaches makes it go
   red and cannot be made green.** So membership in the prefix is a design input, decided
   before the code is written.

   ★★★ **CORRECTED, and this is the important change to this section.** The version of this
   document that named six controls here was reading source 1 and checking six of its
   answers against the capture. That is backwards. **Source 3 is not a check on the
   pre-flight — it IS the pre-flight**, because it is the only one of the three that is a
   *observation* rather than a reading: it lists exactly what the guest asked, in order,
   with nothing inferred about which engine wanted it.

   `[measured]` `cargo run -p kayfabe-crec --example cap1b_report` over
   `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6.rec` at `a96f867`. **28 distinct
   `fn 76` controls** inside the closure limit (txn 1028 / `rpc.sequence` 51):

   | seq | control | engine | served at `a96f867`? |
   |---|---|---|---|
   | 3 | `0x20800a36` GPU_GET_CHIP_INFO | `gpuPreInit` | ✅ |
   | 4 | `0x20800a41` USER_REGISTER_ACCESS_MAP | `gpuPreInit` | ✅ |
   | 5, 6 | `0x208001b0` CONSTRUCTED_FALCON_INFO | `gpuPreInit` ×2 | ✅ |
   | 7 | `0x20800a87` NVLINK_DEVICE_INFO | `KernelNvlink` PreInit | ⊘ refused |
   | 8 | `0x20800a40` INTERNAL_GET_DEVICE_INFO_TABLE | `KernelFifo` | ✅ |
   | 9 | `0x20801112` FIFO_GET_DEVICE_INFO_TABLE | `KernelFifo` | ✅ |
   | 10 | `0x20800a5c` INTR_GET_KERNEL_TABLE | `Intr` | ✅ |
   | 11 | `0x20800a1c` MEMSYS_GET_STATIC_CONFIG | `KernelMemorySystem` PreInit | ✅ |
   | 12 | `0x20801803` BUS_GET_PCI_BAR_INFO | `KernelBus` | ✅ |
   | 13, 44 | `0x20800af3` CONF_COMPUTE_GET_STATIC_INFO | `ConfidentialCompute` | ★ **now served** |
   | 14 | `0x20800aac` BIF_GET_STATIC_INFO | `KernelBif` | ★ **now served** |
   | 15, 16, 17, 34 | `0x20800a61` FIFO_GET_NUM_CHANNELS | `KernelFifo` | ★ **now served** |
   | 18 | `0x20802a08` CE_GET_FAULT_METHOD_BUFFER_SIZE | `KernelCE` | ⊘ refused |
   | 19 | `0x20800afe` INIT_USER_SHARED_DATA | RUSD | ⊘ refused |
   | 20 | `0x20800aff` USER_SHARED_DATA_SET_DATA_POLL | RUSD | ⊘ refused |
   | 25 | `0x20800301` EVENT_SET_NOTIFICATION | subdevice | ⊘ refused |
   | 26 | `0x20800a59` GMMU_GET_STATIC_INFO | `KernelGmmu` | ★ **now served** |
   | 28, 29 | `0x20800a70` BUS_FLUSH_WITH_SYSMEMBAR | `KernelBus` | ⊘ refused |
   | 30, 31, 32 | `0x20800a6c` MEMSYS_L2_INVALIDATE_EVICT | `KernelMemorySystem` | ⊘ refused |
   | 33, 38 | `0x20800a80` PERF_GPU_BOOST_SYNC_GET_INFO | `KernelPerf` | ⊘ refused |
   | 39 | `0x20802a0f` CE_GET_PCE_CONFIG_FOR_LCE_TYPE | `KernelCE` | ⊘ refused |
   | 40, 42 | `0x20802a06` CE_UPDATE_CLASS_DB | `KernelCE` | ⊘ refused |
   | 41 | `0x20802a0d` CE_UPDATE_PCE_LCE_MAPPINGS_V2 | `KernelCE` | ⊘ refused |
   | 43 | `0x2080017e` GPU_GET_VMMU_SEGMENT_SIZE | `gpuInitVmmuInfo` | ⊘ refused |
   | 45 | `0x20800a9f` GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES | `OBJGVASPACE` | ⊘ refused |
   | 49 | `0x20800a1f` STATIC_KGR_GET_CAPS | `KernelGraphics` | ⊘ refused |
   | 50 | `0x20800a2a` STATIC_KGR_GET_INFO | `KernelGraphics` | ⊘ refused |
   | 51 | `0x20800a26` STATIC_KGR_GET_FLOORSWEEPING_MASKS | `KernelGraphics` | ⊘ refused |

   ⊘ `0x20800a4b` DISPLAY_IP_VERSION is **not in this list** — the oracle's board never
   asked, so it must stay refused; serving it would make
   `every_control_this_port_serves_is_exercised_by_the_replay` red with no way back.

   ★★ **And the table is now a gate rather than a document.**
   `every_control_the_oracle_asks_is_either_served_or_triaged` in the same file derives this
   universe from the capture and demands every entry be in `WantedTable::ALL` or in
   `kayfabe_device::sweep::SWEEP_TRIAGE`. ⇒ a control nobody has written anything about is a
   **red test**, which is exactly what `t134a` did not have.

### 4.2 Triage every refusal by what the sweep does with it

This choice did not exist in the ladder, where a refusal was simply a stop.

★★★ **CORRECTED: it is FIVE outcomes, not three.** This section named three — correct /
wrong / unsurvivable. Pre-flighting the *whole* observed prefix rather than six controls
produced two the three could not express, and collapsing them would have meant writing down
a consequence that is not the one the source says. The five live in
`kayfabe_device::sweep::SweepDisposition`, and the classes are distinguished by **what the
guest ends up in**, not by how bad they sound:

- **`AmputationIntended` — refuse.** The chip genuinely lacks the engine, and refusing is
  RM's own vocabulary for saying so. Check that the engine is either on the sweep's
  sanctioned list (`ENG_KERNEL_DISPLAY`, `ENG_INFOROM`, `ENG_HDACODEC` — `gpu.c:2178-2198`)
  or that the caller handles the refusal itself. **Write down the argument.**
- ★★ **`AmputationUnsurvivable` — serve, and serve this FIRST.** Something downstream
  dereferences the engine pointer, or a pointer the failed path *freed*, with no NULL check.
  It is the class that turns a refusal into a guest-kernel Oops attributed to the wrong
  subsystem.
- ★★ **`RefusalFailsOpen` — serve.** RM pre-zeroes or ignores the destination, so nothing
  distinguishes a refusal from an answer, **and** the zeros are not what a real GSP would
  have said. Not a crash; a port defaulting where it could be stating, with nothing able to
  tell.
- ★ **`RefusalIsInvisible` — refuse.** The same invisibility, but the state a refusal leaves
  is **byte-identical to what the oracle's own GSP answered**. ⊘ The class that is easiest
  to get wrong in the flattering direction, so an entry must cite the C artifact's captured
  reply and not merely `ogkm-580`. Refusing is still distinguishable at the *envelope*
  (`rpc_result`), which is a diagnostic cost and can justify serving it anyway.
- ★★ **`RefusalHalts` — refuse, for now.** The caller turns the failure into a status
  `gpuStateInit_IMPL` does **not** map to `NV_OK`, so the boot aborts at a named statement
  rather than continuing damaged. Refusing is *safe*; it is simply the end of the road.
  **This is the class the next batch is drawn from**, and 13 of the 23 triaged controls are
  in it.

⚠ Note what the correction did to the two controls this document told the next agent to
serve. `0x20800af3` and `0x20800aac` are **not** amputations at all: both are asked from
`gpuStateInit`/`gpuStatePostLoad`, whose loops map `NV_ERR_NOT_SUPPORTED` to `NV_OK` without
removing the engine, and both destinations are pre-zeroed — so serving them changes **no
guest state whatsoever**. They were served for the envelope and the `dmesg`, and the
`RefusalFailsOpen` row says so instead of implying a crash that is not there.

### 4.3 Batch, then spend one boot confirming the batch

Serve everything the pre-flight says the sweep will reach, then boot **once** and check the
batch. A boot is now a confirmation instrument, not a discovery instrument.

### 4.4 What to read off that boot

★ The two counters move in **opposite** directions, and neither alone is progress:

- `commands` rising means **more engines were reached**.
- distinct-unserviced falling means **more engines survived**.

★★ And `t134a` is the standing warning that both can look fine while the guest gets worse:
27 commands, six distinct — and a kernel Oops. **The observable that matters is how far
`RmInitAdapter` got**, and guest health is not in the counters at all. Always read the guest
`dmesg`, not only the host's ledger line.

---

## 5. What the rungs did, under the new loop

### 5.1 The first rung (`0x20800a1c`), and the two named refusals

| control | engine | disposition | argument |
|---|---|---|---|
| `0x20800a1c` MEMSYS_STATIC_CONFIG | `KernelMemorySystem` | ★ **served** | amputation is **unsurvivable** — the measured `t134a` Oops. `crates/kayfabe-abi/src/memsysconfig.rs` |
| `0x20800a4b` DISPLAY_IP_VERSION | `KernelDisplay` | **named refusal** | amputation is **correct**: `ENG_KERNEL_DISPLAY` is the sweep's own whitelisted removal (`gpu.c:2178-2182`), and `kdispStatePreInitLocked_IMPL` returns this very status itself when the display fuse is clear (`ogkm-580: src/nvidia/src/kernel/gpu/disp/kern_disp.c:329-330`). This device has no display plane. ★★ It also **overrules the oracle**, which answered `NV_OK` with `ipVersion = 0` from a board that had one |
| `0x20800a87` NVLINK_DEVICE_INFO | `KernelNvlink` | **named refusal** | amputation is **correct**: a GeForce GA106 has no NVLink, the caller handles the status itself with `NV_PRINTF(LEVEL_INFO, "NVLink is unavailable")` (`ogkm-580: src/nvidia/src/kernel/gpu/nvlink/kernel_nvlink.c:1826-1830`), and ★ **the real GA106's own GSP returns `0x56` for it too** (`C: mode2_initctrl_ga106.h:6251`, `{0x20800a87u, 0x56u, …}`). Refusing *is* the oracle here |

### 5.2 ★★★ The first BATCHED rung — four controls, one change

Pre-flighted from the measured §4.1 table and served together, because the sweep reaches all
four in one boot and there is nothing to learn from serving them one at a time:

| control | engine | disposition | why it is in the batch |
|---|---|---|---|
| `0x20800a59` GMMU_GET_STATIC_INFO | `KernelGmmu` | ★★★ `AmputationUnsurvivable` | `_kgmmuInitStaticInfo`'s `fail:` label `portMemFree`s `pKernelGmmu->pStaticInfo` and does **not** NULL the field (`ogkm-580: kern_gmmu.c:139-166`), while `gpuStateInit_IMPL` maps the refusal to `NV_OK` and carries on. A **dangling** pointer is worse than `0x20800a1c`'s NULLed one: every NULL check passes it, and guest-reachable control handlers read through it (`mmu_fault_buffer_ctrl.c:84, 176`) |
| `0x20800a61` FIFO_GET_NUM_CHANNELS | `KernelFifo` | ★★ `RefusalHalts` | the wall. `kfifoRunlistQueryNumChannels_KERNEL` returns 0 on failure (`kernel_fifo.c:1330-1336`) and `kfifoChidMgrConstruct` turns that into `NV_ERR_INVALID_STATE` (`:300-308`), which `gpuStateInit_IMPL` does **not** map to `NV_OK`. Every engine after `KernelFifo` in `gpuChildOrderList_GM200` is unreachable behind it |
| `0x20800af3` CONF_COMPUTE_GET_STATIC_INFO | `ConfidentialCompute` | `RefusalFailsOpen` | ⊘ **changes no guest state** — see §4.2's ⚠. Served for the envelope, and because the encoder can then forbid the *widening*: either trust bit deletes RM's own refusal to map CPR vidmem through BAR1 (`mapping_cpu.c:227-235`) |
| `0x20800aac` BIF_GET_STATIC_INFO | `KernelBif` | `RefusalFailsOpen` | same, and quieter still: `kbifStateInitLocked` calls `kbifStaticInfoInit` as a **bare statement** and discards its status (`kernel_bif.c:132`) |

⊘ **What was deliberately NOT served, and it is most of the list.** Sixteen controls in the
observed prefix are refused with a written argument. Twelve of them are `RefusalHalts` — the
roadmap — and the two largest clusters are worth naming because they must be decided
*together* rather than one at a time:

- **The copy-engine topology triple** — `0x20802a0f` (PCE config), `0x20802a06` (class DB),
  `0x20802a0d` (156-byte PCE→LCE mapping). A topology served in pieces is worse than one
  refused whole: a wrong PCE→LCE map surfaces as a copy that lands nowhere, which is the one
  wrongness that is not diagnosable from the reply.
- **The GR static-info triple** — `0x20800a1f` (caps, 184 B), `0x20800a2a` (info, 3 712 B),
  `0x20800a26` (floorsweeping masks, 3 008 B). Floorsweeping masks state which TPCs and GPCs
  the die has; a capability bitmap without them is a partial description of the one engine
  the north star runs on.

---

## 6. The fail-open encoding this rung found

Every rung is expected to find the field combination that fails **open** and make it
unencodable. This one's is sharper than its predecessors' because it is not a bounds
question:

★★★ **`kmemsysStatePreInitLocked_IMPL` `portMemSet`s the params to zero before the call**
(`ogkm-580: kern_mem_sys.c:114`). So an [`inert`]-style empty reply and a served all-zero
reply reach RM as **the same forty bytes** — the trap named in `inert.rs`'s eligibility rule,
now instantiated. And those forty bytes are not a dull answer; they are three distinct
guest-kernel faults:

| zero field | consequence |
|---|---|
| both comptag policy bits clear | violates a disjunction **RM asserts on itself**: `NV_ASSERT_OR_RETURN(bOneToOne \|\| bUseRawMode, NV_ERR_INVALID_STATE)` (`kern_mem_sys.c:422`, again at `kern_gmmu.c:951`). Every compressed allocation then fails |
| `comprPageSize == 0` | **integer divide-by-zero in the guest kernel**: `*pMemSize = ((*pMemSize + alignPad + comprPageSize - 1) / comprPageSize) * comprPageSize` (`mem_mgr_gm107.c:210-211`), unguarded |
| `ltcCount` or `ltsPerLtcCount == 0` | the Ampere memory-boundary math multiplies and branches on the product (`kern_mem_sys_ga100.c:332-345`) |

Made unencodable in `crates/kayfabe-abi/src/memsysconfig.rs`:

- `ComptagAllocationPolicy` is an **enum with no *neither* variant**, so the first row can
  never be written. ⊘ It also has no `OneToFour` variant: that bit exists in the ABI, but
  RM's disjunction above does not accept it *alone*, so offering it would offer a policy
  that is expressible on the wire and rejected by the guest.
- `ComprPageSizeZero` and `ComprPageSizeNotPowerOfTwo` (the same field is used as
  `comprPageSize - 1`, an alignment **mask**, at `mem_mgr_gm107.c:216`).
- `NoLtcSlices`.
- `comprPageShift` **is not a row field at all** — it is derived as
  `compr_page_size.trailing_zeros()`, so the pair cannot drift. Same discipline as
  `FalconInventoryRow` having no count.
- `FbpaAbsent`: `bFbpaPresent = false` sends RM down an `l2CacheSize`-derived BAR0 window
  placement (`kern_bus_gm107.c:230-247`) instead of the fixed `PRAMIN` span this device's
  register plane decodes. One placement served ⇒ one value advertisable.

⊘ **A claim I looked for and did not find, recorded so nobody re-derives it.**
`offsetBar0 = l2CacheSize - DRF_SIZE(NV_PRAMIN)` at `kern_bus_gm107.c:244` looks like an
unsigned underflow on a zero L2. It is **guarded** by `if (l2CacheSize < DRF_SIZE(NV_PRAMIN))
offsetBar0 = 0;` on the line above. There is no underflow and nothing here claims one.

---

## 7. What remains inference

- The engine-order table in §4.1 is read from `gpuChildOrderList_GM200` and the per-chip
  `gpuChildrenPresent_*` filters. `[inferred]` — it has **not** been checked against an
  observed sweep order on the bench, and the `unserviced` ledger records first-seen order,
  not engine index.
- That refusing `0x20800a4b` produces the *sanctioned* `ENG_KERNEL_DISPLAY` removal rather
  than the `default:` arm's `LEVEL_ERROR` is `[inferred]` from `gpu.c:2178-2182`. The next
  boot's `dmesg` settles it: a sanctioned removal prints nothing at `LEVEL_ERROR`.
- Whether anything downstream dereferences `KernelNvlink` or `KernelDisplay` after
  amputation is `[inferred]` from the absence of an unchecked `GPU_GET_*` on those paths,
  not from a boot that survived one.
- ★★★ **The whole of §5.2 is `[inferred]`. NO BOOT has been spent at a revision that serves
  those four controls.** The measurement behind them is the *capture* — which controls the
  oracle asks, and what its GSP answered — plus `ogkm-580` source readings of what refusing
  does. That is strictly weaker than a boot and strictly stronger than a guess, and it is
  the whole point of §4.3: the batch is built offline and **one** boot confirms it.
- ★★ The single most consequential unverified thing: serving `0x20800a59` lets
  `kgmmuStateInitLocked_IMPL` reach `kgmmuFaultBufferInit_HAL` for the first time, and this
  port answers that path's `REGISTER_FAULT_BUFFER` with a deliberate refusal
  (`kayfabe_device::faultbuffer`, gated by `resume_from_fault.md` §7 step 0). The reading is
  that `gpuStateInit_IMPL` maps that `NV_ERR_NOT_SUPPORTED` to `NV_OK` and the boot survives.
  `[inferred]`. A boot settles it in one line of `dmesg`.
- ⊘ That refusing `0x20800a61` is where the boot currently *stops* is also `[inferred]`,
  from `kernel_fifo.c:300-308`. The `t135a` `dmesg` named `KernelFifo` and `KernelCE` as the
  starved suppliers but did not reach a `pChidMgr->numChannels is 0` line.
