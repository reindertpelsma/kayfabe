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
   before the code is written. `[measured]` by running
   `cargo run -p kayfabe-crec --example cap1b_report` over
   `traces/cap1b_coldboot_hermetic_d6.rec` at `df9dd36`, for the six `t134a` named:

   | control | in `cap1b`'s prefix? |
   |---|---|
   | `0x20800a87` NVLINK | yes — seq 7 |
   | `0x20800a40` DEVICE_INFO_TABLE | yes — seq 8, 7 queue elements |
   | `0x20800a1c` MEMSYS_STATIC_CONFIG | yes — seq 11 |
   | `0x20800af3` CONF_COMPUTE | yes — seq 13 |
   | `0x20800aac` BIF_STATIC_INFO | yes — seq 14 |
   | `0x20800a4b` DISPLAY_IP_VERSION | ⊘ **no** — the oracle's board never asked |

### 4.2 Triage every refusal by what the sweep does with it

This three-way choice did not exist in the ladder, where a refusal was simply a stop:

- **Amputation is CORRECT.** The chip genuinely lacks the engine, and refusing is RM's own
  vocabulary for saying so. Check that the engine is either on the sweep's sanctioned list
  (`ENG_KERNEL_DISPLAY`, `ENG_INFOROM`, `ENG_HDACODEC` — `gpu.c:2178-2198`) or that the
  caller handles the refusal itself. **Refuse, and write down the argument.**
- **Amputation is WRONG.** We need the engine. **Serve.**
- ★★ **Amputation is UNSURVIVABLE.** Something downstream dereferences the engine pointer
  with no NULL check. **Serve, and serve this first** — it is the class that turns a refusal
  into a guest-kernel Oops attributed to the wrong subsystem.

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

## 5. What this rung did, under the new loop

Pre-flight named six controls and triaged them without a boot:

| control | engine | disposition | argument |
|---|---|---|---|
| `0x20800a1c` MEMSYS_STATIC_CONFIG | `KernelMemorySystem` | ★ **served** | amputation is **unsurvivable** — the measured `t134a` Oops. `crates/kayfabe-abi/src/memsysconfig.rs` |
| `0x20800a4b` DISPLAY_IP_VERSION | `KernelDisplay` | **named refusal** | amputation is **correct**: `ENG_KERNEL_DISPLAY` is the sweep's own whitelisted removal (`gpu.c:2178-2182`), and `kdispStatePreInitLocked_IMPL` returns this very status itself when the display fuse is clear (`ogkm-580: src/nvidia/src/kernel/gpu/disp/kern_disp.c:329-330`). This device has no display plane. ★★ It also **overrules the oracle**, which answered `NV_OK` with `ipVersion = 0` from a board that had one |
| `0x20800a87` NVLINK_DEVICE_INFO | `KernelNvlink` | **named refusal** | amputation is **correct**: a GeForce GA106 has no NVLink, the caller handles the status itself with `NV_PRINTF(LEVEL_INFO, "NVLink is unavailable")` (`ogkm-580: src/nvidia/src/kernel/gpu/nvlink/kernel_nvlink.c:1826-1830`), and ★ **the real GA106's own GSP returns `0x56` for it too** (`C: mode2_initctrl_ga106.h:6251`, `{0x20800a87u, 0x56u, …}`). Refusing *is* the oracle here |

⊘ `0x20800af3` (ConfidentialCompute) and `0x20800aac` (KernelBif) are both **StateInit**-phase
and are left for the next batch; `0x20800a40` (KernelFifo) likewise, and it is the largest
reply this port would encode — 24 580 bytes over seven queue elements.

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
