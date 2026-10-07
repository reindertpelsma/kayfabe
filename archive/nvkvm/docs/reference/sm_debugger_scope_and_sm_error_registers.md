# `GT200_DEBUGGER` — is `READ_ALL_SM_ERROR_STATES` scoped, and are the registers real?

```
STATUS: LIVE — 2026-08-14 (w289)
SCOPE:  research_clones/ogkm-580.159.04 ONLY (ogkm is VERSIONED, not the spec) +
        the vendored gvisor/ nvproxy. Read-only source analysis: NO boot, NO bench,
        NO build. The ONE hardware datum quoted is w277's committed GA106 capture,
        and it is labelled as such everywhere it appears.
TARGET: GA106, bare-metal GSP client (IS_GSP_CLIENT true, IS_FW_CLIENT true,
        IS_VIRTUAL false; no MIG, no SR-IOV, no Confidential Compute).
ANSWERS: the owner's two open questions on 0x83de030c (Q1 scope, Q2 real registers).
KNOWN-POSITIVE for "the _IMPL exists": the same grep shape finds
        kchannelCtrlCmdGetClassEngineid_IMPL at kernel_channel.c:2923, and it finds
        ksmdbgssnCtrlCmdDebugReadAllSmErrorStates_IMPL at
        kernel_sm_debugger_session_ctrl.c:731. The _IMPL is PRESENT. It is BYPASSED.
        That distinction is the whole of §1.
```

---

## 0. THE DIRECT ANSWERS

### Q1 — scoped to the caller's own context, or genuinely global?

**BOTH, at two different times, and the split is the answer.**

- ★ **CREATION is scoped, and RM does validate ownership.** `ksmdbgssnConstruct_IMPL`
  (`src/nvidia/src/kernel/gpu/gr/kernel_sm_debugger_session.c:213`) resolves
  `allocParams.hClass3dObject` **inside `hAppClient`'s handle space**
  (`clientGetResourceRef(pAppClient, hClass3dObject, ...)`, `:255`) and then, on CPU-RM,
  demands `RS_ACCESS_DEBUG` on **that** resource ref for **the calling client**
  (`:291-302`). You cannot attach a debugger session to a compute object you neither own
  nor have been granted DEBUG on. ⇒ **The owner's "cross-context" worry is answered NO at
  the object level.**
- ⊘ **THE READ ITSELF IS NOT SCOPED — not by the session, not by `hTargetChannel`, not by
  anything.** NVIDIA's own SDK header says so in the command's doc block:
  *"Note that this acts upon the **currently resident GR (graphics) context**. It is up to
  the RM client to ensure that the desired GR context is resident, before making this API
  call."* (`src/common/sdk/nvidia/inc/ctrl/ctrl83de/ctrl83dedebug.h:325-327`; the identical
  sentence appears at `:250`, `:397`, `:447` for READ_SINGLE / CLEAR_SINGLE / CLEAR_ALL.)
  ⇒ The returned array is **whatever the SMs hold at the instant of the call**, and which
  context that belongs to is decided by **GPU residency**, which the session does not
  constrain.

⇒ **Net: the gate is at `NV_ESC_RM_ALLOC` time, and it is a real gate. There is no second
gate at control time.** A caller who legitimately holds a session on *its own* object can
issue `0x83de030c` at a moment when *another* context is resident and receive that
context's SM error registers. See §3 for what that is worth to an attacker — it is narrower
than it sounds, and it is not nothing.

### ⊘ CORRECTION to a prior lane's note — two facts were being carried as one

A prior lane recorded *"global SM registers for the currently resident GR context;
`hTargetChannel` never validated"* (the sentence lives at
`/workspace/nvkvm-rs/docs/design/road_to_v1_after_cup2.md:60` — **not corrected there by this
lane; that tree was off-limits today**). Measured against ogkm 580.159.04:

- **The residency half is RIGHT** and is quotable straight from NVIDIA's header
  (`ctrl83dedebug.h:325-327`, above).
- **The `hTargetChannel` half is right in letter and misleading in force.**
  `hTargetChannel` (`ctrl83dedebug.h:383`) is **not validated by CPU-RM — because CPU-RM
  never looks at it at all.** Whole-tree grep for `hTargetChannel` over ogkm returns exactly
  two kinds of hit: the SDK header (`ctrl83dedebug.h:315,383,438,482`) and the **vGPU
  serializers that copy it verbatim** (`rm_plugin_shared_code.h:647,676,888,921,4344,4368,
  4580,4613`; `g_sdk-structures.h:1567,1598,2101,2110`). **Zero CPU-RM logic reads it.**
  ⇒ Naming a channel you do not own does not get you that channel: it gets you **nothing**,
  because the field selects nothing on this path. The exposure is created by *residency*,
  not by *forging `hTargetChannel`*.
- ★★ **And `hTargetChannel` is a DIFFERENT field from `HClass3DObject`.** `HClass3DObject`
  is an **alloc** parameter, validated at `:255` + `:291-302`. `hTargetChannel` is a
  **control** parameter, validated nowhere. Treating them as one fact is what made the
  refusal look better-founded than it is (`HClass3DObject` really is checked) *and*
  worse-founded than it is (`hTargetChannel` is not a bypass — it is inert).

### Q2 — real SM hardware registers, or values RM constructed?

**REAL registers, read by GSP-RM firmware. Nothing in the open driver synthesises them, and
nothing in the open driver reads them either.** The HAL binding, resolved:

- The exported entry for `0x83de030c` is a **real function pointer**, not a NULL and not a
  `_fcf1ac` forwarder: `g_kernel_sm_debugger_session_nvoc.c:455-465` →
  `&ksmdbgssnCtrlCmdDebugReadAllSmErrorStates_IMPL`, `flags = 0x50048`, `accessRight = 0x0`.
- **There is no `_TU102` / `_GA100` / `_GA10x` variant.** Grep for
  `ksmdbgssnCtrlCmdDebugReadAllSmErrorStates` outside `generated/` and outside the `_IMPL`
  returns **nothing** — the symbol has no HAL family at all.
- ★ **But the `_IMPL` never runs on GA106.** `flags` carries
  `RMCTRL_FLAGS_ROUTE_TO_PHYSICAL` (`0x40`, `control.h:233`), so on an `IS_FW_CLIENT` GPU
  the resserv prologue `rmresControl_Prologue_IMPL`
  (`src/nvidia/src/kernel/rmapi/resource.c:266-297`) issues `NV_RM_RPC_CONTROL` (`:289`)
  and returns `NV_WARN_NOTHING_TO_DO` (`:297`), which `resControl_IMPL`
  (`rs_resource.c:191-201`) reads as *"Call handled by the prologue"* and **skips the
  method body**. The `_IMPL`'s own non-virtual tail returns `NV_ERR_NOT_SUPPORTED`
  (`kernel_sm_debugger_session_ctrl.c:773`) and is **dead code on this chip**.
- **Where the RPC goes:** `NV_RM_RPC_CONTROL` for `IS_FW_CLIENT` is *not* the vGPU RPC —
  it calls the **physical RMAPI** (`vgpu/rpc.h:230-234`), which `rpc_common.c:74-79` binds
  to `rpcRmApiControl_GSP`. The whole 4824-byte parameter struct is serialised to GSP-RM.
- ⊘ **The chunked per-control RPC is a DIFFERENT, non-applicable path — and this is exactly
  the "the `.c` you read is not the code that runs" trap.**
  `rpcCtrlDbgReadAllSmErrorStates_v21_06` (`vgpu/rpc.c:7338-7380`, 80 SMs per RPC) is
  reached only from `rpcDmaControl_wrapper`'s switch (`vgpu/rpc.c:4513`, case at `:4763`),
  which is the **vGPU-guest** transport. On GA106 the chip HAL binds that interface to
  **`rpcCtrlDbgReadAllSmErrorStates_STUB`** — `g_rpc_private.h:414` lists the STUB's chips
  verbatim, and **GA106 is in the list**. Anyone reading `rpc.c:7338` as "what GA106 does"
  is reading a stub's body.
- ⇒ **The register access is in GSP-RM firmware, which is closed and absent from this
  tree.** What the open tree *can* say: (a) no CPU-RM code fabricates these fields; (b) no
  CPU-RM code reads a PRI for them; (c) the fields are declared as register contents
  (`hwwGlobalEsr`, `hwwWarpEsr`, `hwwWarpEsrPc64`, `hwwEsrAddr`, `hwwCgaEsr` —
  `ctrl83dedebug.h:300-309`); (d) **the one measured GA106 run we own is consistent with
  "real, and empty":** w277's `traces/fault_known_positive_ga106/arm1_native/faultgr.jsonl.zst`
  `[707]` — `0x83de030c`, `paramsSize=4824`, `pgot=4824` (fully captured), `rc=0`,
  **all 4800 bytes of `smErrorStateArray` zero on both sides**, RM writing back exactly
  **5 bytes** in the MMU-fault tail (`docs/reference/ogkm_authored_guest_userspace_structures.md:649-656`).
  ⇒ On an **MMU** fault the SM array carries nothing. That is evidence against fabrication
  (no plausible junk was invented) and is **not** evidence that a real SM trap populates it.
  §5 says what would settle that.

### Privilege — what an unprivileged client gets

**Everything this class offers.** `accessRight = 0x0` on **every one of the 31 exported
`0x83de03xx` methods** (`g_kernel_sm_debugger_session_nvoc.c`, all `/*accessRight=*/0x0u`),
and `rsAccessCheckRights` **returns `NV_OK` immediately when the required mask is empty**
(`rs_access_map.c:181-194`, *"Return if nothing to check"*). And **every one of the 31
carries `RMCTRL_FLAGS_NON_PRIVILEGED` (`0x8`, `control.h:208`)** — `0x8`, `0x48`, `0x248`,
`0x10208`, `0x10248`, `0x50048` all have bit 3 set. `rmControlValidateClientPrivilegeAccess`
(`control.c:676-712`, invoked from `serverControl_ValidateCookie` at `:832`) therefore takes **neither** the `PRIVILEGED` branch (`:686`) **nor**
the kernel-only default branch (`:702`, which fires only when *none* of
NON_PRIVILEGED/PRIVILEGED/INTERNAL is set). **There is no `osIsAdministrator` on this path.**
⇒ This is consistent with the banked fact that an RM fd does not carry the opener's
privilege: the check is re-derived from the caller's `pSecInfo->privLevel`, and here the
flags make the check a no-op.

⇒ **The one and only gate for the whole GT200_DEBUGGER surface is `RS_ACCESS_DEBUG` at
session creation.** Post-creation, per-control authorisation does not exist.

---

## 1. THE DISPATCH, END TO END (GA106 + GSP client)

```
ioctl NV_ESC_RM_CONTROL cmd=0x83de030c hObject=<GT200_DEBUGGER>
  → resControl_IMPL                       rs_resource.c:160
      resControlLookup                    → exported entry, flags=0x50048, accessRight=0
      serverControl_InitCookie            rmapi/resource.c:164-176   (copies flags+rights)
      serverControl_Prologue → serverControl_ValidateCookie
                                          rmapi/control.c:717-…
          rsAccessCheckRights(required = EMPTY)  → NV_OK immediately
                                          rs_access_map.c:181-194
          rmControlValidateClientPrivilegeAccess → NV_OK (NON_PRIVILEGED)
                                          rmapi/control.c:676-712 (called at :832)
      resControlSerialization_Prologue    rmapi/resource.c:207-229  (IS_FW_CLIENT → serialise DOWN)
      resControl_Prologue → rmresControl_Prologue_IMPL
                                          rmapi/resource.c:250-300
          IS_FW_CLIENT && ROUTE_TO_PHYSICAL  → TRUE
          NV_RM_RPC_CONTROL(...)          rmapi/resource.c:289
              → IS_FW_CLIENT branch       vgpu/rpc.h:230-234
              → pRmApi->Control = rpcRmApiControl_GSP   rpc_common.c:76
          return NV_WARN_NOTHING_TO_DO    rmapi/resource.c:297
      "Call handled by the prologue"      rs_resource.c:196-201   ⇒ _IMPL SKIPPED
```

⊘ **Not on this path, despite looking like it is:**
`ksmdbgssnCtrlCmdDebugReadAllSmErrorStates_IMPL` (`kernel_sm_debugger_session_ctrl.c:731`)
— its whole body is an `IS_VIRTUAL(pGpu)` branch, and `IS_VIRTUAL` is false here.
`rpcCtrlDbgReadAllSmErrorStates_v21_06` (`vgpu/rpc.c:7338`) — STUB for GA106
(`g_rpc_private.h:414`).

### The flag words, decoded (`control.h`)

| flags | bits | which `0x83de03xx` |
|---|---|---|
| `0x8` | NON_PRIVILEGED | `0315` READ_MEMORY, `0316` WRITE_MEMORY, `031a` READ_SURFACE, `031b` WRITE_SURFACE, `031c` GET_MAPPINGS, `0326`/`0327` BATCH R/W, `0328` READ_MMU_FAULT_INFO — **kernel-implemented, no routing** |
| `0x48` | + ROUTE_TO_PHYSICAL (`:233`) | `0301`/`0302` SM_DEBUG_MODE EN/DISABLE, `0314`, `0320`, `0322`–`0325` |
| `0x248` | + ROUTE_TO_VGPU_HOST (`:253`) | `032a`/`032b` MMU_GCC_DEBUG |
| `0x10208` | GSP_PLUGIN_FOR_VGPU_GSP (`:290`) + ROUTE_TO_VGPU_HOST + NON_PRIV, **no ROUTE_TO_PHYSICAL** | `031d` **EXEC_REG_OPS** |
| `0x10248` | + ROUTE_TO_PHYSICAL | `0307`/`0308` MMU_DEBUG, `0309` SET_EXCEPTION_MASK, `030b` READ_SINGLE, `030f` CLEAR_SINGLE, `0313`, `0317` SUSPEND_CONTEXT, `0318` RESUME_CONTEXT, `031f`, `0321` |
| `0x50048` | ROUTE_TO_PHYSICAL + GSP_PLUGIN_FOR_VGPU_GSP + **PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST** (`:308`) + NON_PRIV | **`030c` READ_ALL**, **`0310` CLEAR_ALL** — the two, and only two, with a live kernel `_IMPL` |

★ `0x40000` (`PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST`) is precisely why `030c`/`0310` have a
kernel `_IMPL` at all: it declares *"the guest half is implemented in Guest-RM"*, which is
the `IS_VIRTUAL` branch at `:741-772` that also merges the per-context MMU-fault tail. On
bare metal that flag buys nothing.

---

## 2. Q1 IN FULL — what the session is bound to, and what it is not

### 2.1 Creation binds to a `KernelGraphicsObject` the caller must hold DEBUG on

`ksmdbgssnConstruct_IMPL` (`kernel_sm_debugger_session.c:213-350`):

1. `hAppClient = pNv83deAllocParams->hAppClient`; **zero means "myself"** (`:246-249`).
   A non-zero value may name **any client in the system** — `serverGetClientUnderLock`
   (`:252-253`) is a global lookup, not a same-process one.
2. `clientGetResourceRef(pAppClient, hClass3dObject, &pGrResourceRef)` (`:255`) — the 3D
   object is resolved **in the named client's handle space**, so the attacker must know a
   valid `(hClient, hObject)` pair. Handles are guessable in principle; this is not the
   gate.
3. **The gate** (`:285-302`), on CPU-RM / non-GSP-firmware builds:
   ```c
   RS_ACCESS_MASK_CLEAR(&debugAccessMask);
   RS_ACCESS_MASK_ADD(&debugAccessMask, RS_ACCESS_DEBUG);
   status = rsAccessCheckRights(pGrResourceRef, pCallContext->pClient, &debugAccessMask);
   ⇒ NV_ERR_INSUFFICIENT_PERMISSIONS
   ```
   `RS_ACCESS_DEBUG` is `2` (`src/common/sdk/nvidia/inc/rs_access.h:61`) and its metadata is
   **`RS_ACCESS_FLAG_ALLOW_OWNER`** (`rs_access_rights.c:49-52`). `rsAccessGetAvailableRights`
   (`rs_access_map.c:139-165`) returns the ref's own mask when the invoking client owns it,
   and otherwise only **explicitly shared** rights. ⇒ **self-debug always passes; debugging
   a stranger requires that stranger to have shared DEBUG** (`RmShare` /
   `NV0000_CTRL_CMD_CLIENT_SHARE_OBJECT`, `rs_client.c:176`, `:274-278`).
4. `pKernelSMDebuggerSession->pObject` is that `KernelGraphicsObject` (`:304`);
   `hChannel` is taken from **the object's parent ref**, not from anything the caller
   supplies (`:310-311`, `:334`).

⚠ **The GSP-side check is WEAKER, and NVIDIA says so in a comment** (`:265-274`): on
`RMCFG_FEATURE_PLATFORM_GSP` builds the rights check is replaced by
`VALIDATE_MATCHING_SEC_TOKENS`, and *"When it is NULL, this allows access to any client in
the system but in order to take advantage of this CPU-RM would already have to have been
compromised anyway."* ⇒ **GSP-RM trusts CPU-RM's rights check and does not repeat it.**
★★ For our architecture that sentence is load-bearing: **in Mode-1 / isolate forwarding, the
host stub IS the CPU-RM client**, so if the stub multiplexes several guests' objects into one
RM client, `rsAccessCheckRights` sees one owner and grants DEBUG across all of them. The
isolation that ogkm provides here is per-**RM client**, and it is the only isolation on this
control. (This is a design consequence, not a defect found; it is why
`isolate_exists_for_VA_IDENTITY_not_security` should be read as scoping, not reassurance.)

### 2.2 Nothing re-checks at control time

- `accessRight = 0x0` ⇒ the resserv rights check at `control.c:751-755` is fed an **empty**
  required mask and short-circuits (`rs_access_map.c:190-192`).
- `hTargetChannel` is **inert on CPU-RM** (§0, the correction). It is forwarded verbatim into
  the GSP RPC as part of the opaque parameter blob. Whether GSP-RM validates it is not
  determinable here — but if it did, the SDK's own residency sentence would be false, so the
  documented contract says it does not.
- **The one thing that IS re-scoped** is the MMU-fault tail, and only in the vGPU-guest
  branch: `kgrctxLookupMmuFault` (`kernel_graphics_context.c:600-618`) reads
  `pKernelGraphicsContextUnicast->mmuFault` off **the session's own object's** context,
  which was populated by `kgrctxRecordMmuFault` attributed via `kgrctxFromKernelChannel`
  from the faulting channel (`arch/volta/kern_gmmu_gv100.c:2170-2185`). ⇒ **The MMU half is
  per-context by construction. The SM half is not.** That asymmetry inside one control is
  the cleanest statement of the finding.

---

## 3. WHAT AN ATTACKER ACTUALLY LEARNS — named precisely, not softened

**Whose state:** any GR context resident on the same GPU (same MIG instance, if MIG) at the
instant of the call — i.e. **another process's, or another VM's guest process's, compute
context**.

**When observable:** only while that context is **resident** *and* has **non-zero SM error
state**, i.e. it is in the middle of taking a warp/global SM exception and has not yet had
that state cleared. RM does not hold the GPU lock across a victim's whole kernel, and the
call itself takes `rmapiLockIsOwner() && rmGpuLockIsOwner()` (asserted at
`kernel_sm_debugger_session_ctrl.c:740`), so the window is a genuinely racy one.

**What is learned** (per `NV83DE_SM_ERROR_STATE_REGISTERS`, `ctrl83dedebug.h:299-309`),
per SM, up to 100 SMs per call:
- `hwwGlobalEsr`, `hwwWarpEsr`, `hwwCgaEsr` + their report masks — **which exception class
  fired**. Low value.
- ★ `hwwWarpEsrPc64` — a **64-bit program counter inside the victim's shader code**.
- ★ `hwwEsrAddr` — the **64-bit faulting address in the victim's GPU VAS**. Under UVM
  unified addressing that VA **is** the victim's process VA (see
  `shape_cannot_discriminate_origin`), so this is an ASLR / heap-layout disclosure about
  another process.

**What is NOT learned:** no memory contents, no register file, no results. This is a
**metadata side channel about a faulting neighbour**, not a read primitive. It requires the
victim to be faulting — a state in which the victim is about to be torn down anyway.

⇒ ★ **And the mirror image is the sharper edge: `0x83de0310` `CLEAR_ALL_SM_ERROR_STATES`
carries the identical `0x50048` flags and the identical (absent) scoping.** An attacker who
can *read* an unscoped resident context can also *clear* it. Clearing a victim's SM error
state before the victim's own `cuCtxSynchronize` reads it is a **fault-suppression /
integrity** attack — the victim's error is silently lost — and it is strictly easier to land
than the read, because it does not require winning a race to *observe*, only to *precede*.
**Neither we nor gVisor treat CLEAR_ALL differently from READ_ALL, and on this evidence it
is the more dangerous of the two.**

### 3.1 gVisor carries this too — said plainly

`gvisor/pkg/sentry/devices/nvproxy/version.go:334-336` permits **exactly three** `0x83de03xx`
controls — `0309` SET_EXCEPTION_MASK, `030c` READ_ALL, `0310` CLEAR_ALL — each as
`ctrlHandler(rmControlSimple, compUtil)`, i.e. **straight pass-through of the parameter blob
under the *compute* capability**, not under `nvconf.CapProfiling`. The `GT200_DEBUGGER` alloc
is likewise `allocHandler(rmAllocSMDebuggerSession, compUtil)` (`:427`), while its neighbours
`GF100_PROFILER` (`:425`) and `MAXWELL_PROFILER_DEVICE` (`:426`) **are** gated on
`nvconf.CapProfiling`. `rmAllocSMDebuggerSession` (`frontend.go:1271-1279`) records a
dependency on `allocParams.HClass3DObject` **for object-lifetime tracking only** — its own
comment says it "elide[s]" the driver's `RmDebuggerSession` indirection — and performs **no
ownership or scope check of its own**; it relies entirely on the driver's `:291-302` gate.

⇒ **A sandbox explicitly built for untrusted workloads exposes an unscoped, resident-context
SM-error read and clear to any container holding the compute capability.** That is a true
statement about gVisor, it follows from their own table, and it should be said as such
rather than used as reassurance. ★ It is also the reason the exposure is **acceptable** for
us: it is the industry's current line, the leak is metadata-about-a-faulting-neighbour, and
the alternative — denying it — costs the CUDA fault-reporting path outright (§4).

---

## 4. RECOMMENDATION ON THE FIVE DENIED `0x83de03xx` CONTROLS

The refusal I previously recommended was justified by a cross-context-visibility worry.
**That worry is real but mis-sized**: it is not a channel-forging bypass (`hTargetChannel` is
inert), it is a residency race yielding a faulting neighbour's PC and fault VA, and it is a
line gVisor already crosses. Against that: `0x83de030c` is **how libcuda surfaces a fault to
the application** on real GA106 (w277, `faultgr.jsonl.zst[707]`, `rc=0`) — denying it does not
harden anything a guest could not get from gVisor, and it breaks fault reporting.

| cmd | name | flags | recommendation |
|---|---|---|---|
| `0x83de030c` | READ_ALL_SM_ERROR_STATES | `0x50048` | ★ **ADMIT.** Required for CUDA fault reporting; exposure = §3, bounded, and matched by nvproxy. |
| `0x83de0310` | CLEAR_ALL_SM_ERROR_STATES | `0x50048` | ★ **ADMIT** — it is the mandatory partner of `030c` (the driver's own MMU-fault tail is armed/disarmed by it, `:776-816`) and nvproxy permits it. ⚠ **But log it**, and record in the capability table that it is the *integrity* half (§3), not a read. |
| `0x83de0309` | SET_EXCEPTION_MASK | `0x10248` | already restored by owner ruling 2026-08-14 — **consistent with this analysis**; it programs a mask, not a read. |
| `0x83de0307` | SET_MODE_MMU_DEBUG | `0x10248` | ⊘ **KEEP DENIED.** nvproxy does **not** permit it. It *changes GPU state* (MMU debug mode is a device-wide mode with a request/release refcount, `ctrl83dedebug.h:121-123`), so it is a write to a shared resource, not an observation. Different class from `030c`. |
| `0x83de0317` / `0x83de0318` | SUSPEND_CONTEXT / RESUME_CONTEXT | `0x10248` | ⊘ **KEEP DENIED.** nvproxy does not permit them. ★ **And note the composition with §3:** SUSPEND/RESUME are precisely the primitives that would turn the §3 *race* into a *deterministic* read — a caller who can stop a context can guarantee which context is resident when it calls `030c`. Denying `0317`/`0318` while admitting `030c` leaves the exposure at "race", which is where nvproxy leaves it too. **This pairing is the reason the recommendation is coherent rather than arbitrary.** |

⇒ **Admit two, keep three denied**, and the boundary lands exactly on nvproxy's own table —
which is the defensible position, since matching a shipped untrusted-workload sandbox is a
justification we can state, and "stricter than gVisor for reasons we cannot articulate" is
not.

⚠ **One thing the admission must carry:** `0x83de030c`'s parameter struct is **4824 bytes
with a 4800-byte output array**. Serving it means **writing 4800 bytes into guest memory from
a reply**. Whatever we return must be **fully initialised** — the `dlen < psize` class from
the C oracle applies directly: a short/partial reply decodes to zeros with no marker, and
here the zeros are indistinguishable from "no SM error", which is a **wrong answer that looks
like a right one** to libcuda's fault path.

---

## 5. ⊘ WHAT I COULD NOT DETERMINE

1. **Whether GSP-RM firmware applies a residency or ownership filter that the SDK header
   says it does not.** The register access is in closed firmware; it is not in this tree in
   any form. Everything in §0/Q2 is "no synthesis and no read exists in the open driver",
   which is a strictly weaker statement than "GSP reads the PRIs".
2. **Whether the SM error registers are context-switched** (saved/restored with the GR
   context image) or are pure live PRIs. If they are context-switched, the leak is exactly
   as described. If they are live-and-cleared-on-ctxsw, the window narrows further. ogkm
   contains **no** `HWW_WARP_ESR` register definition or ctxsw list — grep returns only SDK
   header text and vGPU serializers.
3. **Whether `hTargetChannel` is validated by GSP-RM.** Inert on CPU-RM; opaque beyond.
4. **Whether a real SM trap populates the array at all on GA106+GSP.** The only capture we
   own is an **MMU** fault, where the array was all zeros by design.

### The measurements that would settle these — all on the real GA106 bench

- ★ **M1 (settles 4, and half of Q2).** Run a CUDA kernel that raises a genuine **SM**
  exception rather than an MMU fault — `__trap()`, or an out-of-range `__shfl_sync` /
  misaligned shared-memory access — then read `0x83de030c` from the same process.
  **Pass = any non-zero `hwwWarpEsr` / `hwwWarpEsrPc64`.** All-zero here, given a fault the
  driver otherwise reports, would mean the array is inert on this path and the whole
  exposure question is moot. ⊘ Reuse the w277 `faultgr` harness shape; do **not** reuse
  `faultce` (recorded `VERDICT NOFAULT`).
- ★★ **M2 (settles 1 and 3 together — the decisive one).** Two processes, A and B, each with
  its **own** client, context and `GT200_DEBUGGER` on its **own** 3D object. A runs the M1
  trapping kernel in a loop; B polls `0x83de030c` and checks whether it ever observes
  non-zero SM state it did not produce. **Pass (exposure confirmed) = B sees A's
  `hwwWarpEsrPc64`.** Variant: B passes `hTargetChannel = <A's channel handle value>` vs
  `hTargetChannel = 0` vs its own — **if all three return identical bytes, `hTargetChannel`
  is inert end-to-end, GSP included**, which closes (3) with a positive control.
- **M3 (settles the CLEAR_ALL / integrity half).** A traps in a loop and reads its own error
  state; B issues `0x83de0310` continuously. **Pass (integrity exposure confirmed) = A's
  reads start returning zero** while A is still faulting.
- ⚠ **All three need a KNOWN-POSITIVE first**: before B's zero result is read as "no leak",
  A must be shown reading its **own** non-zero SM state through the same code path
  (`a_census_zero_needs_a_known_positive`). Without that, M2 returning zeros is
  indistinguishable from M1 having failed silently.

---

## 6. CITATION INDEX (every file:line above was opened in this lane)

**ogkm 580.159.04** — `src/common/sdk/nvidia/inc/ctrl/ctrl83de/ctrl83dedebug.h`
:250,:299-309,:315,:325-327,:371,:382-389,:397,:433,:447,:477,:682,:705,:844 ·
`src/common/sdk/nvidia/inc/rs_access.h:61` ·
`src/nvidia/src/kernel/gpu/gr/kernel_sm_debugger_session.c`
:213,:246-255,:265-302,:304-311,:334,:459-468 ·
`src/nvidia/src/kernel/gpu/gr/kernel_sm_debugger_session_ctrl.c`
:603-636,:731-773,:776-816 ·
`src/nvidia/src/kernel/gpu/gr/kernel_graphics_context.c`:567,:600-660 ·
`src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c`:2170-2185 ·
`src/nvidia/src/kernel/rmapi/resource.c`:164-176,:207-229,:250-300 ·
`src/nvidia/src/kernel/rmapi/control.c`:640-712,:751-755,:773-786,:880-950 ·
`src/nvidia/inc/kernel/rmapi/control.h`:208,:233,:253,:290,:308 ·
`src/nvidia/inc/kernel/vgpu/rpc.h`:223-235 ·
`src/nvidia/src/kernel/rmapi/rpc_common.c`:60-102 ·
`src/nvidia/src/kernel/vgpu/rpc.c`:980-994,:4513,:4524,:4763-4764,:7338-7380,:8803-8826 ·
`src/nvidia/src/kernel/gpu/gpu_resource.c`:365-390 ·
`src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_regops.c`:59-101 ·
`src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2923` (known-positive) ·
`src/nvidia/src/libraries/resserv/src/rs_resource.c`:160-205 ·
`src/nvidia/src/libraries/resserv/src/rs_access_map.c`:139-207,:560-612 ·
`src/nvidia/src/libraries/resserv/src/rs_access_rights.c`:36-58 ·
`src/nvidia/src/libraries/resserv/src/rs_client.c`:176,:274-278 ·
`src/nvidia/inc/kernel/vgpu/rm_plugin_shared_code.h`:647,:657-700,:4344,:4354-4381 ·
`src/nvidia/generated/g_kernel_sm_debugger_session_nvoc.c`:365-830 ·
`src/nvidia/generated/g_kernel_sm_debugger_session_nvoc.h`:635-730 ·
`src/nvidia/generated/g_rpc_private.h`:413-414,:3180 ·
`src/nvidia/generated/g_sdk-structures.h`:1567,:1596-1606

**gvisor (vendored)** — `pkg/sentry/devices/nvproxy/version.go`:334-336,:425-427,:645-647,:736 ·
`pkg/sentry/devices/nvproxy/frontend.go`:1271-1279

**This repo** — `src/qemu/nvkvm_ctrl_allowlist.h`:154-156 (the C already allowlists
`0309`/`030c`/`0310`) · `docs/reference/ogkm_authored_guest_userspace_structures.md`:393-411,
:647-658 (w277's GA106 fault capture; §4.2's *"a fourth thing we must answer ourselves"*) ·
`docs/audits/nvproxy_control_allowlist.md`:331-340
