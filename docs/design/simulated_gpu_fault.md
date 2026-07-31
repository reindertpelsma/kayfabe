# A bad application pointer, as a simulated GPU fault (task #111)

**Status:** designed and implemented at the *decision + encoding + transport* layers;
**not exercised on a live guest** — see §9, which says exactly what that leaves open.
**Epistemic class:** everything below is either a **reading** of the vendored driver
(`ogkm-580:` / `ogkm-610:` citations) or **[inferred]** from one. Nothing here was
measured on hardware; no bench run exists for it and none is available (task #123).

---

## 1. The problem, and why Mode 2 changes it

In Mode 1 the guest forwards its own ioctls. A bad userspace pointer is caught by the
host kernel and comes back as `EFAULT`, or the application takes `SIGSEGV`. That is
correct, and it is unavailable here.

In Mode 2 the guest runs the **stock NVIDIA driver against an emulated GPU**. When a
guest application hands the GPU an address it never mapped, there is no host ioctl to
return `EFAULT` from — the guest believes it is talking to hardware. So the failure has
to present as the thing real hardware produces: **a GPU fault the guest's own driver
already knows how to handle.**

The alternatives are both worse, and both are attested:

- **A hang.** Today, an unmapped VA in a submission's working set is refused by the #14
  ring-gate (`kayfabe_fwd::plan_doorbell` → `kayfabe_isolate::VerbPlan::gated_doorbell`
  → `FwdFault::Address(AddressFault::Miss)`), the channel is never rung, and *nothing
  tells the guest*. The application waits on a semaphore that will never be released.
- **A silent wrong answer.** The C artifact's actual behaviour, at seven sites.

## 2. What the C did — the negative result this design starts from

The C research artifact **never fabricated a fault for the guest, ever**. It has zero
occurrences of `NVC369`, `NV_PFAULT`, `fault_buffer`, `replayable`, `ROBUST_CHANNEL` or
`error_notifier` in `src/`. Its only in-code "FAULT" is a sentinel whose documented
contract is to do nothing:

```c
#define NVKVM_GMMU_FAULT (~0ull)   /* C: src/qemu/nvkvm_gpu_emul.c:116 */
/* "NVKVM_GMMU_FAULT on any miss (caller then does nothing — safe)."  C: :4915 */
```

Its two interrupt raise sites are the GSP status-queue vector and an echo of the guest's
own self-test write (`C: nvkvm_gpu_emul.c:1830-1843`, `:4375-4396`). There is no error
vector, no RC path, no fault vector.

The consequences, as the C's own comments record them:

| situation | detected? | guest told? | guest-visible result |
|---|---|---|---|
| GPFIFO entry VA unresolvable | logged (`C:6088`) | **no** — cursor advanced to `gp_put` anyway (`C:6579`) | work vanishes; poll hangs |
| CE copy faults mid-stream | logged, **capped at 256** (`C:6386`) | **no** — and the completion semaphore is released regardless (`C:6503`) | **wrong data + "success"** |
| BAR2 VA unmapped | not logged | **no** — read returns 0 (`C:6604`), write dropped (`C:6615`) | zeros / lost write |
| channel GPFIFO unresolvable | not logged | **no** — `continue` (`C:9059`) | channel never rings |

The design directive existed and was never implemented: *"miss = a real GPU page fault,
surfaced loud and forwarded to the guest as a fault"*
(`C: docs/design/mode2_address_table.md:188-189`) — the only place in that repository
where guest fault delivery is proposed. It names no mechanism.

⚠ **Naming collision, stated so nobody chases it:** the C repository also has a task
`#111`, and it is unrelated (async event delivery / WB-coherency on shared pages /
NVENC — `C: docs/design/async_event_delivery.md:121`).

## 3. What the hardware actually does — the readings

### 3.1 The fault-buffer entry

An MMU fault on this generation is a **32-byte packet**
(`NVC369_BUF_SIZE`, `ogkm-580: src/common/sdk/nvidia/inc/class/clc369.h:31-71`), written
into a ring in memory. Its fields, by dword:

| dword | bits | field |
|---|---|---|
| 0 | 9:8 / 31:12 | `INST_APERTURE` / `INST_LO` |
| 1 | 31:0 | `INST_HI` — with `INST_LO`, the faulting channel's **instance block** physical address |
| 2–3 | 31:12 / 31:0 | `ADDR_LO` / `ADDR_HI` — the faulting VA, as a 4 KiB page number |
| 4–5 | 31:0 | `TIMESTAMP_LO` / `_HI` |
| 6 | 8:0 | `ENGINE_ID` |
| 7 | 4:0 / 7:7 / 14:8 / 19:16 / 20:20 / 28:24 / 29:29 / 30:30 / **31:31** | `FAULT_TYPE` / `REPLAYABLE_FAULT` / `CLIENT` / `ACCESS_TYPE` / `MMU_CLIENT_TYPE` / `GPC_ID` / `PROTECTED_MODE` / `REPLAYABLE_FAULT_EN` / **`VALID`** |

`FAULT_TYPE` is a closed 16-value enum
(`ogkm-580: kernel-open/nvidia-uvm/hwref/ampere/ga100/dev_fault.h:224-239`): `PDE = 0x0`,
`PDE_SIZE = 0x1`, `PTE = 0x2`, `VA_LIMIT_VIOLATION = 0x3`, `UNBOUND_INST_BLOCK = 0x4`,
`PRIV_VIOLATION = 0x5`, `RO_VIOLATION = 0x6`, `WO_VIOLATION = 0x7`, …
`ATOMIC_VIOLATION = 0xf`. `ACCESS_TYPE` likewise (`:459-472`): `VIRT_READ = 0x0`,
`VIRT_WRITE = 0x1`, `VIRT_ATOMIC = 0x2`, `VIRT_PREFETCH = 0x3`, `VIRT_ATOMIC_WEAK = 0x4`,
and a physical set at `0x8`–`0xb`.

★ Out-of-range is **not** a lint. `kgmmuGetFaultType_GV100` returns
`NV_ERR_INVALID_ARGUMENT` from its `default:` arm
(`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:1699-1780`), which
fails `kgmmuParseFaultPacket_GV100`'s `NV_ASSERT_OR_RETURN` (`:1851-1852`) and **aborts the
guest's whole drain loop** (`:2004-2005`) — so one bad code also loses the faults queued
behind it.

### 3.2 The registers and the interrupt

The buffer's control registers live in the virtual-function PRIV aperture, reached by the
physical function at BAR0 + `0x00B80000`
(`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_vm.h:26-28`;
`gpuGetVirtRegPhysOffset_TU102`,
`ogkm-580: src/nvidia/src/kernel/gpu/arch/turing/kern_gpu_tu102.c:93-100`). Per buffer
(index 0 = non-replayable, 1 = replayable — `dev_vm.h:72-73`), 32-byte stride from
`0x3000` (`dev_vm.h:74-112`):

| register | PF BAR0, non-repl / repl |
|---|---|
| `MMU_FAULT_BUFFER_LO/HI` | `0xB83000/04` / `0xB83020/24` |
| `MMU_FAULT_BUFFER_GET` (RW; `PTR 19:0`, `GETPTR_CORRUPTED 30`, `OVERFLOW 31`) | `0xB83008` / `0xB83028` |
| `MMU_FAULT_BUFFER_PUT` (**read-only**) | `0xB8300C` / `0xB8302C` |
| `MMU_FAULT_BUFFER_SIZE` (`VAL 19:0` in **entries**; `ENABLE 31`) | `0xB83010` / `0xB83030` |
| `MMU_FAULT_STATUS` | `0xB83094` (`dev_vm.h:113-119`; field layout `ogkm-580: volta/gv100/dev_fb.h:138-200`) |

The interrupt vectors are fixed constants
(`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_fb.h:31-32`): replayable
**64**, non-replayable **132**. With `LEAF_REG(v)=v/32`, `LEAF_BIT(v)=v%32`
(`ogkm-580: src/nvidia/arch/nvalloc/common/inc/dev_ctrl_defines.h:70-78`) that is
`CPU_INTR_LEAF(2)` bit 0 at `0xB81008` and `CPU_INTR_LEAF(4)` bit 4 at `0xB81010`.
Acknowledge is write-1-to-clear on the leaf
(`intrClearLeafVector_TU102`, `ogkm-580: src/nvidia/src/kernel/gpu/intr/arch/turing/intr_tu102.c:647-660`).

★ **The interrupt is a level re-derived from `GET != PUT`, not an edge.** RM writes GET
back even when it copied nothing, precisely so the condition is re-evaluated
(`ogkm-580: kern_gmmu_gv100.c:1085-1102`, and the same contract at
`src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:2344-2357`). An emulation that treated it as an
edge would drop faults the guest had not finished draining.

### 3.3 ★★★ …and on a GSP driver the CPU never reads any of it

This is the finding the whole design turns on:

```c
/* ogkm-580: src/nvidia/arch/nvalloc/unix/src/unix_intr.c:933-938 */
if (IS_GSP_CLIENT(pGpu))
{
    // Non-replayable faults are copied to the client shadow buffer by GSP-RM.
    status = NV_OK;
    goto done;
}
```

Corroborated three more ways at the same tag:

- the `NON_REPLAYABLE_FAULT`, `NON_REPLAYABLE_FAULT_ERROR` and `GMMU` interrupt services
  are registered **only when `!IS_GSP_CLIENT`** (`ogkm-580: kern_gmmu.c:2267-2288`);
- `kgmmuEnableMmuFaultInterrupts` is a kernel-side "not supported" stub
  (`ogkm-580: src/nvidia/generated/g_kern_gmmu_nvoc.c:1507-1515`) — arming the leaf enables
  is the GSP's job;
- `kgmmuServiceChannelMmuFault` — the function that would RC the channel — **has no open
  kernel implementation at all** (`ogkm-580: g_kern_gmmu_nvoc.h:1157, 2110-2115`).

And the vendor states the split in prose, on the receiver we are going to use:

> *"RC error handling ("Channel Teardown sequence") is executed in GSP-RM. Client
> notifications, OS interaction etc happen in CPU-RM (Kernel RM)."*
> — `ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:541-545`

**In Mode 2 we are the GSP.** So the fault shape is not something we approximate on the
guest's behalf; it is a message the component we are impersonating is the one to send.

## 4. Which failure mode this handles, and what happens to the other

★★★ Two different things produce an untranslatable address and **only one of them may
become a simulated fault.**

| | who chose the address | meaning | this design |
|---|---|---|---|
| **A** | a guest **application** | the app named memory it never mapped — its bug; hardware answers with an MMU fault | **emit** |
| **B** | the guest **kernel** (its RM/UVM clients), or our own derivation | a driver we are impersonating hardware for cannot reach an address it set up: either it is broken or **we** are | **escalate, emit nothing** |

Emitting for B would be worse than the hang it replaces. It converts *our* defect into a
well-formed message blaming the guest; the guest kills a channel, reports a clean error,
and the run looks like a correctly-diagnosed application bug. Every measurement taken
afterwards is measuring a lie. This project already has that failure shape written down
twice — the C's capped-log-then-release-the-semaphore path (`C: nvkvm_gpu_emul.c:6376-6395`
with `:6503`) and the GA10x 512M-leaf gap that silently dropped page-table writes for
weeks. A fault emitter with no attribution rule would be the third.

**The discriminator is a fact the guest declared, not a heuristic.** Every client that
declared itself `ClientKind::Kernel` on its own `NV01_ROOT` belongs to the one reserved
system component (`SYSTEM_ANCHOR`, `kayfabe_core::project`), by rule and never by
inference. A channel owned by `Gpu::SYSTEM_PROC` is B; any other proc's is A. This is the
same key the whole `Proc` grouping already turns on, so it cannot drift away from the
classification the data plane uses — there is one, not two.

A third case is separated rather than folded in: a channel with **no declared VAS**
(`Channel::vas_pdb = None`, a GSP-managed system-routed channel) escalates with its own
reason, because a caller told only "not attributable" cannot tell an ownership question
from a modelling gap.

⚠ **What the rule does not claim.** It says the address was chosen *inside an
application's context*; it does not say the application caused it. A kayfabe bug that
loses an application's binding produces an A-verdict and blames the app. That residual is
real and unmitigated — see §7.

⊘ **Out of scope, deliberately:** a guest-userspace pointer that is bad in *host* terms
(a `void*` handed to an ioctl). That is Mode 1's `EFAULT` case and the ioctl path still
handles it; nothing here changes it.

## 5. The fault shape we emit, and the one we do not

### 5.1 Chosen: `NV_VGPU_MSG_EVENT_RC_TRIGGERED` (`0x1004`)

`rpc_rc_triggered_v17_02` — **byte-identical at both vendored tags**
(`ogkm-580: src/nvidia/generated/g_rpc-structures.h:1560-1577`,
`ogkm-610: :1481-1498`), 48 bytes with an empty journal:

```
nv2080EngineType @0   chid @4   gfid @8   exceptLevel @12   exceptType @16
scope @20   partitionAttributionId @24 (u16)   mmuFaultAddrLo @28
mmuFaultAddrHi @32   mmuFaultType @36   bCallbackNeeded @40 (u8)
rcJournalBufferSize @44   rcJournalBuffer[] @48
```

We set `exceptType = ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT`
(`ogkm-580: src/common/sdk/nvidia/inc/nverror.h:49`) — **31**, the number a host kernel
log prints as `Xid 31`, and the same code the driver's own fault handler sets
(`ogkm-580: kern_gmmu_gv100.c:2127-2131`). `scope = RC_NOTIFIER_SCOPE_TSG`, matching the
same site. `gfid = 0`, because `IS_GFID_PF(0)` is what makes the receiver look the channel
up at all (`ogkm-580: kernel_gsp.c:585-593`).

What the receiver does with it (`_kgspRpcRCTriggered`,
`ogkm-580: kernel_gsp.c:548-678`): resolves the engine to a channel-id manager, finds the
`KernelChannel` by `chid`, folds the journal records into the system journal, and calls
`krcErrorSendEventNotifications_HAL` — which signals the channel's error-notifier ctxdma
and posts `NV2080_NOTIFIERS_RC_ERROR` to every registered client
(`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:439-467`).

★ The **journal is empty** (`rcJournalBufferSize = 0`). The receiver's loop is
`for (i = 0; i < rcJournalBufferSize / recordSize; i++)`
(`ogkm-580: kernel_gsp.c:596-641`), so zero runs it zero times — an honest "no diagnostic
records". We ran no shader and read no MMU; a fabricated register dump would put invented
numbers into the guest's OCA journal.

### 5.2 Rejected (for now): writing the 32-byte fault buffer

It is the more "hardware-real" answer and it is the wrong first move, for four reasons,
each a reading:

1. **The CPU driver would not read it.** §3.3 — on a GSP client the HW non-replayable
   buffer is not touched by the CPU at all. The bytes would go nowhere.
2. **It would still not RC the channel.** `kgmmuServiceChannelMmuFault` has no CPU-side
   implementation, so even a perfectly-formed packet consumed by the shadow path ends in
   the guest handing it *back* to us
   (`nvGpuOpsReportNonReplayableFault`, `ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:11154-11185`).
   `RC_TRIGGERED` is still required.
3. **We cannot fill it honestly.** The entry's attribution key is the **instance block
   physical address**, matched by a linear scan over live channels
   (`kfifoConvertInstToKernelChannel`,
   `ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:572-656`);
   a mismatch produces `"Could not get chid from inst addr"` and silence. It also wants a
   `GPC_ID`, a uTLB `CLIENT` and an MMU timestamp — none of which we observed, because no
   shader ran.
4. **The delivery contract is subtle.** All eight dwords before `VALID`, a store fence,
   PUT advanced, and the leaf pulsed with `GET != PUT` re-derivation (§3.2). Getting it
   wrong loses faults invisibly.

⚠ **This is a deferral, not a refutation.** There is one path it is *required* for:
**replayable** faults, which UVM services by mapping the real `GET(1)`/`PUT(1)` registers
and polling the `VALID` bits over BAR0 directly
(`nvGpuOpsInitFaultInfo`, `ogkm-580: nv_gpu_ops.c:9170-9345`;
`kgmmuGetFaultRegisterMappings_TU102`,
`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:187-232`). The
shadow-buffer indirection for replayable faults exists only under Confidential Compute
(`ogkm-580: kern_gmmu_gv100.c:933-937`), which is off in our target
(`mode2_rewrite_design_decisions`). So **[inferred]**: managed/UVM demand paging will need
the real buffer emulated, and this design does not cover it. It is not needed for the
bad-pointer case, which is fatal by definition and never replayed.

## 6. Which layer owns emitting it

Five layers, one job each. The split is the deliverable as much as the code is.

| layer | crate | what it owns | new? |
|---|---|---|---|
| **detect** | `kayfabe-mmu` | `AddressFault::Miss` — MISS = FAULT, forward-populated, never reverse-resolved | **already existed** |
| **derive** | `kayfabe-fwd` | `fault_facts` — collect the channel, engine, VAS and address *beside the refusal*, in the same locked snapshot | new |
| **decide** | `kayfabe-core::fault` | `verdict` — the A/B rule; `FaultReport` / `NotAttributable` / `FaultVerdict` | new |
| **encode (Axis A)** | `kayfabe-abi::rc` | the `rpc_rc_triggered_v17_02` bytes, `ROBUST_CHANNEL_*`, `NV2080_ENGINE_TYPE_*`, the scopes | new (struct generated) |
| **encode (Axis B)** | `kayfabe-arch::fault` + `kayfabe-device::ga10x` | `MmuFaultCodes`: `NV_PFAULT_FAULT_TYPE_*` / `ACCESS_TYPE_*` for one chip | new |
| **compose** | `kayfabe-rmrpc::fault` | `rc_triggered_for` — the only crate permitted to name both `kayfabe_core` and `kayfabe_gsp` vocabularies | new |
| **transport** | `kayfabe-gsp` | `GspFsm::post_event` + flow control + `RpcFunction::RcTriggered` | **already existed** (bar the function code) |

Two boundary decisions worth stating:

- **No chip number and no NVIDIA layout entered a logic crate.** The fault-type codes are
  `kayfabe-device`'s, behind a `kayfabe-arch` trait; the wire struct is `kayfabe-abi`'s,
  generated by the offline generator from the vendored tree, not transcribed.
- **The composer is in the bridge crate** because it is the seam between core vocabulary
  and GSP wire, and `gsp_core_bridge.md` §1.2 already reserves that crate for exactly
  that. Putting it in the core would give the core an RPC encoder; putting it in
  `kayfabe-gsp` would give the faked GSP a view of the RM graph, which that document
  forbids on purpose.

★ **The emitter declines rather than guesses.** `EngineRoute::for_engine` answers only for
GR. A copy engine has no answer, because `NV2080_ENGINE_TYPE_COPY0` is the *base of a
range* and `EngineKind::Ce` carries no instance. The reason this matters is the receiver's
own shape: it resolves `nv2080EngineType` to a channel-id manager and **returns early on
failure, silently** (`ogkm-580: kernel_gsp.c:578-583`). A guessed engine therefore produces
a message that is parsed, dropped, and counted *by us* as delivered — a false green on the
emitter's own side, which is this project's most-repeated failure class. So a CE fault
gets `FaultEmitRefusal::NoEngineRoute` and the caller must escalate it.

## 7. Attributability — what the fault honestly carries

The task's bar: *if the guest cannot tell which access faulted, the fault is nearly
useless to it.*

**Carried, and each is a fact we held when we refused:**

- `chid` — the runlist index of the channel whose doorbell we declined. This is the
  receiver's own lookup key, and the `#14` test in `tests/tests/simulated_fault.rs` drives
  two processes faulting on the **identical** VA and asserts the two events carry
  different `chid`s.
- `mmuFaultAddrLo/Hi` — the exact VA the submission named.
- `exceptType` — MMU fault (`Xid 31`), not a generic error.
- `mmuFaultType` — the chip's code for the abstract cause.

**Not carried, and not invented:** `GPC_ID`, the uTLB `CLIENT`, the MMU `TIMESTAMP`, and
the access direction (which `rpc_rc_triggered_v17_02` has **no field for** — it lives in
the 32-byte entry we do not write). `FaultReport::attribution_note` returns that limit as
data, so a consumer that logs a fault logs its own limits beside it.

★ The cause is reported as `NV_PFAULT_FAULT_TYPE_PDE` and not `_PTE`, and that is a
deliberate under-claim. The hardware distinction is "the walk ran out at a directory" vs
"it reached a leaf and the leaf was invalid"; the address table records bindings, not the
level a hypothetical walk would have died at. Claiming the leaf variant would be inventing
a walk that never happened.

**Unmitigated residual (§4):** a kayfabe bug that loses an application's binding is
indistinguishable, at this layer, from the application never having mapped it. The
mitigation would be a **census** — a per-proc count of emitted faults, surfaced beside the
existing `RefusalCensus`, so that "this workload took 4 000 simulated faults" is a number
an operator reads rather than a suspicion. It is **not built**.

## 8. What is implemented

- `crates/kayfabe-abi/gen/src/main.rs` — `rpc_rc_triggered_v17_02` and three constants
  added to the generator's slice; regenerated `src/generated/rpc.rs`.
- `crates/kayfabe-abi/src/rc.rs` — `RcTriggered`, `EngineRoute`, the scopes.
- `crates/kayfabe-arch/src/fault.rs` — `MmuFaultCause`, `MmuFaultAccess`, `MmuFaultCodes`.
- `crates/kayfabe-device/src/ga10x.rs` — `Ga10xFaultCodes`.
- `crates/kayfabe-core/src/fault.rs` — `FaultFacts`, `verdict`, `FaultVerdict`,
  `NotAttributable`, `FaultReport`.
- `crates/kayfabe-fwd/src/lib.rs` — `fault_facts`.
- `crates/kayfabe-rmrpc/src/fault.rs` — `rc_triggered_for`, `FaultEmitRefusal`.
- `crates/kayfabe-gsp/src/rpc.rs` — `RpcFunction::RcTriggered`, `FunctionCodes::rc_triggered`.
- `tests/tests/simulated_fault.rs` — 8 tests; `crates/kayfabe-abi/src/rc.rs` unit tests.

**Not implemented, named rather than implied:**

- **Nothing calls it in production.** The shell has no doorbell wiring yet
  (`kayfabe-shell` is the reactor; the register plane is stage Q4 and the memory plane is
  Q5), so there is no site that takes a `FwdFault::Address(Miss)` off a real guest
  doorbell and posts the event. The path is built and unwired.
- **No error-notifier write.** With Confidential Compute off, CPU-RM skips
  `krcErrorSetNotifier` (`ogkm-580: kernel_gsp.c:660-667`), which **[inferred]** means the
  GSP is expected to have written the channel's error-notifier memory itself. We do not.
  What that costs is unmeasured: the RM event still fires, but whether libcuda reads the
  notifier's contents to distinguish *which* error is not answerable from this tree —
  `ILLEGAL_ADDRESS` appears nowhere in it, that mapping being closed-source.
- **No fault census** (§7).
- **No replayable-fault buffer** (§5.2).

## 9. ⊘ What the evidence does NOT cover, and the live test that would close it

**The evidence here is mock-level.** No guest ran. What the tests establish is:

1. the bytes land at the offsets the generator lifted out of the vendored driver;
2. the values are the driver's own constants, not transcriptions;
3. a real `GspFsm` accepts the event under its real flow control and only once `Running`;
4. an **independent re-implementation of the guest's own msgq/RPC receive path**
   (`tests/src/gspworld.rs`) reads it back with the function id and `chid` intact;
5. the A/B rule refuses everything it should, by exact variant.

**What none of that establishes:** that a stock NVIDIA driver, having received these
bytes, tears the channel down and that the application sees an error. Four specific
things could still be wrong and would not show up here — the engine-type→chid-manager
resolution could fail on a real runlist; `chid` may need to be the *hardware* channel id
rather than our vChid; the missing error-notifier write may leave CUDA reporting an
unhelpful generic failure; and the event could arrive at a moment the guest's RPC state
machine does not tolerate.

**The live test that would close it**, when a guest reaches compute (task #123):

1. Boot a Mode-2 guest to a working `cuCtxCreate` → matmul (the `cup8` ladder).
2. Run a CUDA program that dereferences a device pointer it never allocated — the
   canonical `cudaMemcpy` to a garbage `void*`, or a kernel indexing past its allocation.
3. **Expect, in order:** our host log records `AddressFault::Miss` with that VA and an
   `Emit` verdict naming the channel; the guest's `dmesg` prints
   `Xid ... 31, ... MMU Fault`; the program terminates with a CUDA error rather than
   hanging; `rc=124` from `timeout(1)` is a **failure** of this test.
4. **The negative control, and it is the important one:** the same program with a
   *correct* pointer must complete at `bad=0 maxerr=0` and emit **zero** faults. A fault
   emitter that fires on legitimate traffic converts a working forwarder into a broken
   one, and that is the regression this feature can cause.
5. **The B-mode control:** unload and reload the guest driver, and confirm that no
   simulated fault is emitted for any system-component channel during bring-up or
   teardown.

Until step 3 has been run, the correct description of this feature is: *the decision, the
encoding and the transport are built and unit-proven; the guest's acceptance of them is
inferred from its source and has not been observed.*
