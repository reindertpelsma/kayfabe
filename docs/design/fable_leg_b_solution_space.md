# Leg B — the USERD naming problem: an independent survey of the solution space (w749)

**STATUS: LIVE AS A REFERENCE, 2026-09-21 (w822).** Cited by `THE_DESIGN.md`, which is the
current architecture. ⊘ Where this file's *architecture* disagrees with that one, **that one
wins** — this predates it. ★ What stands here regardless: its **measurements**, its **ogkm
findings**, and the **reasoning** behind a constraint.


> ### STATUS — 2026-09-16 / **RESEARCH-ONLY, SOURCE-MEASURED. No production code touched. No posture recommended.**
> Written against `ogkm-580.159.04` (`/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04`),
> the host reference trace `traces/host_reference_ga106/ce_r1.jsonl.zst` (real GA106, libcuda
> 580.173.02), and this tree's own docs. Every claim carries a `file:line`; every inference is
> labelled **INFERRED** with the probe that would settle it. ⊘ This document **adds to**
> `scratchpad_born_channels_evidence.md` (w748) and does not restate it; where the two touch, w748
> is cited rather than repeated. ⊘ **No constraint is proposed relaxed.** Where a candidate costs a
> constraint, the cost is named and the candidate is ranked by it.

Brief: `THE_CONSTRAINTS.md` §26 (isolate borrows, scratchpad holds), §30 (stamps), §31 (USERD
pinning); `SINGLE_STORE_PLAN.md` w745 §2 (leg B open). Terms: **S** = scratchpad isolate, **I** =
per-proc isolate, **A** = I's working client, **the store** = the one reserved vidmem object held by S.

---

## 0. Headlines — what this survey found that the table in the brief does not have

1. ★★★★★ **The premise "there is no SET for USERD" is false.** `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO`
   (`0x20801116`) **re-points a live channel's USERD and GPFIFO** to `{hUserdMemory, userdOffset,
   gpFifoOffset, gpFifoEntries}`, and — by the vendor's own design — resolves the **channel** in
   `params.hClient` and the **memory** in the **caller's** client: two different clients. It is the
   exact verb leg B wants. It is gated `RMCTRL_FLAGS_PRIVILEGED` = `CAP_SYS_ADMIN`. §2.
2. ★★★★★ **There is a route that keeps every stamp and every constraint, and it costs a seam, not a
   property: the isolate *mints* the client the channel is born in, and *hands the fd over*.** The
   `ProcessID` stamp is fixed when the **client** is created (`client.c:112`, `osGetCurrentProcess()`
   of the *creating* task); the privilege stamp is fixed by the **calling task's capability at each
   ioctl** (`escape.c:304`); RM binds a client to a `struct file`, not to a pid (`client.c`
   `rmclientValidate_IMPL`, STRICT by default `g_system_nvoc.c:104`). ⇒ a client I created and S
   drives through a passed fd births a channel stamped `ProcessID = I`, `_PRIVILEGE_USER`, with the
   store named from a dup that **only S ever sees and that S frees before anyone else can act**.
   The cost is that the channel's *handle* lives in a client only S can reach — because a
   `KernelChannel` cannot be duped at all (w748 §2.0). §3.1.
3. ★★★ **Option A is dead, and the discriminator was a grep, not a boot.** The guest UMD issues
   **zero** `NV2080` FIFO controls on real hardware (census of all 197 RM controls in the reference
   `ce` stage, §1.1) and names its own USERD in vidmem on 68/68 channels. There is nothing for the
   kernel's answer to steer. §1.1.
4. ★★★ **Option B is dead at the branch, not "overwritten".** The client-supplied `userdMem` is read
   only inside GSP-firmware builds or on an SR-IOV VF (`kernel_channel.c:2299-2325`,
   `rmconfig.h:260`). On a PF host it is never read; the RPC field is filled from the handle-derived
   memdesc (`:2747-2757`). §1.2.
5. ★★ **A free, fail-closed assert for constraint 30 exists in the driver's own writeback**:
   `kernel_channel.c:281,286` set `NVOS04_FLAGS_PRIVILEGED_CHANNEL` (bit 5, `alloc_channel.h:141`)
   **in the alloc params**, and RM copies the params back on success (`alloc_free.c:207-211`).
   Pass bit 5 clear; if it comes back set, the channel was stamped KERNEL/ADMIN. §6.
6. ⊘ **The whole "USERD in sysmem" family** (A, udmabuf, dma-buf import) has a real, unprivileged
   host-side import primitive (`NVOS32_DESCRIPTOR_TYPE_OS_FILE_HANDLE`, §3.5) — and **no guest-side
   lever**, so it is recorded here to stop it being re-derived, not because it is live.

---

## 1. Refutations of A and B — save the probes

### 1.1 A — "USERD in sysmem, if the guest's init reads something we supply" — DEAD

**The kernel side.** The USERD aperture is a hardcoded constant, not a GSP-sourced value:
`src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:82-87` —
`pUserdInfo->userdAperture = ADDR_FBMEM; … memdescOverrideInstLoc(NV_REG_STR_RM_INST_LOC _USERD …)`.
The only override is the **guest's own registry key** (already noted at
`channel_alloc_forwardability.md:1157-1163`). `kfifoGetUserdLocation_GM107` (`:1519-1533`) just
returns that field; `subdeviceCtrlCmdFifoGetUserdLocation_IMPL` (`kernel_fifo_ctrl.c:367-400`)
just translates it. `bUserdInSystemMemory` (`:1186-1188`) is *derived from* the same field. Nothing
we emulate is consulted.

**The userspace side — and this is the part that closes it.** Census of every `NV_ESC_RM_CONTROL`
cmd in the real-host reference capture (`traces/host_reference_ga106/ce_r1.jsonl.zst`, 197 control
records, decoded from the NVOS54 header at bytes 8–11):

    00000101 00000136 0000013a 000001f0 00000201 00000202 00000205 00000214 00000215 00000216
    0000027b 00000288 00000a04 00000d01 00000d04 00800280 00800289 00800292 00801307 00801402
    0080170d 00801909 20800102 20800110 20800111 20800119 2080012f 20800131 20800145 20800146
    2080014a 20800170 20801201 20801210 20801218 2080121b 20801227 2080122a 2080122b 20801303
    20801701 20801801 20801803 20801823 2080182a 2080182b 2080200a 2080220c 20802210 20802a0a
    20803002 20803601 20803801 20808159 20808162 20810108 83de0309 906f0101 a06c0101 a06c0103
    a06c0105 c36f0108 cb330101

**No `0x208011xx` at all.** `NV2080_CTRL_CMD_FIFO_GET_USERD_LOCATION = 0x2080110d`
(`ctrl2080fifo.h:456`) is never asked; neither is `NV0080_CTRL_CMD_FIFO_GET_CAPS`. And the guest
UMD names its own USERD: `hUserdMemory[0] != 0` on **68 of 68** channel allocs, `userdMem.addressSpace
= NV_ADDR_FBMEM` ×68 (`channel_alloc_forwardability.md:524, :693`).

⇒ **libcuda decides USERD placement without asking the kernel, and puts it in vidmem.** There is
no value we could supply that it reads. ⚠ The same libcuda runs in the stock guest, so the
real-host census transfers. **INFERRED** only that no *other* control it does issue carries a
USERD-placement hint (none of the listed ids is FIFO-scoped; a UMD disassembly would make this
measured).

⊘ Even in a world where the UMD did honour it, the resulting *guest-sysmem* USERD would land in
guest RAM, and §26 forbids I from holding the guest-RAM memfd; §3.5 records the one primitive that
would have made a 4 KiB range of it nameable anyway.

### 1.2 B — "supply the `userdMem` descriptor instead of a handle" — DEAD ON ENTRY, not overwritten

`src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2299-2325`:

- `:2299-2308` — `if (!(RMCFG_FEATURE_PLATFORM_GSP && !pKernelChannel->bGspOwned) && !(IS_GFID_VF(gfid) && …))`
  → `kchannelCreateUserdMemDescBc_HAL(pGpu, pKernelChannel, hClient, hUserdMemory, userdOffset)` —
  **the handle path**. This is the branch a PF host CPU-RM takes.
- `:2319-2325` — the `else if ((RMCFG_FEATURE_PLATFORM_GSP && !bGspOwned) || IS_GFID_VF(gfid) …)`
  → `_kchannelDescribeMemDescsFromParams` — the **only** reader of `pChannelGpfifoParams->userdMem`
  (`:2438-2455`). `RMCFG_FEATURE_PLATFORM_GSP` is **`0`** in the kernel build
  (`src/nvidia/generated/rmconfig.h:260`); a GA106 PF has `gfid == GPU_GFID_PF`.

⇒ On our host the client's `userdMem` is **never read**. `:2747-2757` then fills the RPC's
`userdMem` from `pKernelChannel->pUserdSubDeviceMemDesc`, which came from the handle. So the "sharpest
open question" — *is it overwritten?* — has a sharper answer: **it is unread**. No probe needed.

⊘ For completeness: the only non-firmware consumer of client-described USERD is the **SR-IOV VF**
guest RM path (`:2457-2466`, `MEMDESC_FLAGS_GUEST_ALLOCATED`). We are not a VF.

### 1.3 Two smaller dead ends, so nobody spends a probe on them

- **A `NV50_MEMORY_VIRTUAL`/`NV01_MEMORY_VIRTUAL` handle as USERD.** `kchannelCreateUserdMemDesc_GV100`
  takes `memdescGetPhysAddr(…, AT_GPU, userdOffset)` (`kernel_channel_gv100.c:204`) and programs the
  instance block with it (`kernel_channel_gm107.c:328`). A VA in the hardware's physical field is
  garbage. Dead by shape.
- **I allocates its own `NV01_MEMORY_LOCAL_USER` at the store's physical offset**
  (`NVOS32_ALLOC_FLAGS_FIXED_ADDRESS_ALLOCATE`, `heap.c:1491-1505`): the heap walks its free-block
  list for the desired offset; pages owned by the store are not free. **INFERRED** `NV_ERR_NO_MEMORY`
  (one alloc ioctl would measure it). Dead by construction: two RM owners of one page is the thing
  the heap exists to prevent.

---

## 2. The SET that exists: `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO` — and its gate

`src/nvidia/src/kernel/gpu/fifo/kernel_fifo_ctrl.c:523-597`, `subdeviceCtrlCmdFifoUpdateChannelInfo_IMPL`:

```c
    NvHandle hClient = RES_GET_CLIENT_HANDLE(pSubdevice);        // :532  the CALLER's client
    serverGetClientUnderLock(&g_resServ, pChannelInfo->hClient, &pChannelClient);   // :543-545
    CliGetKernelChannel(pChannelClient, pChannelInfo->hChannel, &pKernelChannel);    // :548-550
    if (!pChannelInfo->hUserdMemory)          return NV_ERR_INVALID_ARGUMENT;         // :553-556
    if (!pKernelChannel->bClientAllocatedUserD) return NV_ERR_NOT_SUPPORTED;          // :558-561
    if (IS_GSP_CLIENT(pGpu)) {
        pRmApi->Control(… physical …);                                                // :565-572 → GSP
        kchannelDestroyUserdMemDesc_HAL(pGpu, pKernelChannel);                        // :576
        kchannelCreateUserdMemDesc_HAL(pGpu, pKernelChannel, hClient,                 // :580-582
                                       pChannelInfo->hUserdMemory, pChannelInfo->userdOffset, …);
```

- **Cross-client by design**: channel from `params.hClient`, USERD memory from the *caller's*
  `hClient`. The SDK comment: *"can be used for migrating a channel to a new userd and gpfifo"*
  (`ctrl2080fifo.h:796-817`). This is a vGPU-plugin migration verb — S's shape exactly.
- Requires the channel to have been born with **some** client USERD (`bClientAllocatedUserD`,
  `:558`), i.e. I births with a 4 KiB object of its own (D-shaped) and S re-points it to the store.
- Host must be `IS_GSP_CLIENT` (`:563`) — it is. The VF HAL variant is a stub (`g_subdevice_nvoc.c:10582-10585`).
- The new USERD is a **sub-memdesc of the store's memdesc**, owned by the channel and destroyed with
  it (`kchannelDestroyUserdMemDesc_GV100`, `kernel_channel_gv100.c:285-300`) — no `LIST_OBJECT`
  snapshot, no second RM object with its own lifetime. Constraint 31's "handles outlive the page"
  hazard does not arise: the store never moves.

★ **The gate.** `g_subdevice_nvoc.c:5041` — `/*flags=*/ 0x4u` = `RMCTRL_FLAGS_PRIVILEGED`
(`control.h:202`). `control.c:686-698`: `if (pSecInfo->privLevel < RS_PRIV_LEVEL_USER_ROOT) return
NV_ERR_INSUFFICIENT_PERMISSIONS`. `privLevel` = `osIsAdministrator()` (`escape.c:304`) = `NV_IS_SUSER()`
= `capable(CAP_SYS_ADMIN)` (`nv-linux.h:537`) — in the **initial** user namespace.

⇒ **U costs exactly what F costs: `CAP_SYS_ADMIN` on S — constraint 17.** But under the *same*
premise F needs (a privileged S), U is strictly cheaper than F: no `MemoryList` object is ever
minted, nothing is shared or duped into I, I's namespace holds nothing of the store, and the
teardown barrier is the channel's own. If F is ever ruled admissible, U should be measured first.
Recorded, not recommended.

### 2.1 U′ — the deferred-API variant, unprivileged registration, physical side unknown

`NV50_DEFERRED_API_CLASS` is `RS_FLAGS_ALLOC_NON_PRIVILEGED`, a channel descendant
(`resource_list.h:1522-1534`); its ctrl `NV5080_CTRL_CMD_DEFERRED_API_V2` (`0x50800103`) carries
flags `0x10008` = `NON_PRIVILEGED` (`g_deferred_api_nvoc.c:213-215`). `deferred_api.c:345-375`
special-cases `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO` inside the bundle, registering
`hUserdMemory` with GSP **from `pParams->hClient`** — the *same* client as the channel on the CPU
side — then forwards to physical RM; execution happens later on a software method the channel
itself pushes (`:711-714`). ⇒ the privileged gate is bypassed (execution is RM-internal), **but**
the CPU side resolves the memory in the channel's client, so as read here I would need the store
handle. Whether the **closed GSP side** honours `hClient ≠ the deferred object's client` for the
memory is unknowable from source.
**Probe** (one boot, no production code): I registers a deferred `UPDATE_CHANNEL_INFO` with
`hClient = S's client, hUserdMemory = the store`, pushes the software method; **discriminator**:
`GP_GET` advancing in the store page *and* `nvidia-smi -q` XID 0 ⇒ honoured; `NV_ERR_OBJECT_NOT_FOUND`
at registration ⇒ CPU side refuses; `NV_OK` at registration but no cursor movement ⇒ GSP dropped it
silently — record which. Ranked low: two closed-source unknowns and a pushbuffer dependency.

---

## 3. New candidates

### 3.1 ★★★★★ K — "birth client on a passed fd": every stamp is I's, and I never holds the store

**The stamps, from source.**
- `ProcessID` is stamped from **the client**, at **client creation**, from the **creating task**:
  `client.c:112` `pClient->ProcID = osGetCurrentProcess();` and copied into the channel at
  `kernel_channel.c:293`. (`SubProcessID` likewise, `:294`.)
- Privilege is stamped from **the call**: `kernel_channel.c:277-292`, `privLevel =
  pCallContext->secInfo.privLevel`, and `rmclientIsAdmin(pRmClient, privLevel)` = `privLevel >=
  USER_ROOT && !bIsRootNonPriv` (`client.c:384-393`). `hypervisorCheckForObjectAccess` is
  unconditionally `NV_FALSE` (`hypervisor_access.c:32-40`).
- A client is bound to a **`struct file`**, not a pid: STRICT validation is the default
  (`g_system_nvoc.c:104` → `PDB_PROP_SYS_VALIDATE_CLIENT_HANDLE_STRICT = 1`) and compares
  `pClient->pOSInfo == pSecInfo->clientOSInfo` (`client.c` `rmclientValidate_IMPL`, the
  `pOSInfo` set at `:91`). The non-strict fallback compares **euid or pid** tokens
  (`os.c:3856-3865`: refused only if *both* differ). ⇒ **an fd passed by `SCM_RIGHTS` carries the
  client with it**, whichever mode.
- The dup destination is validated against the **caller's** fd: `rs_server.c` `serverCopyResource`
  → `clientValidate(pClientDst, pSecInfo)` (`:1695`).
- Same-`ProcID` clients may dup each other's objects **by default**: `sharing.c:341-352` installs
  `RS_SHARE_TYPE_PID` + `RS_ACCESS_DUP_OBJECT` as the default inherited policy; the callback is
  `client_resource.c:217-228` (`pSrcClient->ProcID == pDstClient->ProcID`).
- A dup of a `Memory` refcounts the memdesc (`mem.c:1100` `memdescAddRef`); a channel's USERD
  sub-memdesc holds its **own** parent ref (`mem_desc.c:2676-2684`, `_pParentDescriptor` +
  `memdescAddRef`). ⇒ freeing the dup leaves the channel's USERD intact.
- `CLIENT_SHARE_OBJECT` (`0xd06`) is `NON_PRIVILEGED` (`g_client_resource_nvoc.c:1578-1580`,
  flags `0x9`); `RS_SHARE_TYPE_CLIENT` targets one client handle (`rs_resource.c:378-380`).

**The mechanism.**
1. I opens a second `/dev/nvidiactl`, **fd2**, and allocates client **B** on it (`NV01_ROOT`).
   `B.ProcID = I's tgid` (`client.c:112`); `B.cachedPrivilege = USER` (`:95`).
2. I sends fd2 to S over the existing isolate socket (`SCM_RIGHTS` — the Mode-1 C's shipped
   crossing, `the_passthrough_verb_is_built_and_orphaned.md`) and **closes its own copy**. From here
   on **only S holds fd2**.
3. S: `CLIENT_SHARE_OBJECT(store, {type: CLIENT, target: B, mask: DUP})` on its own client; then
   `NV_ESC_RM_DUP_OBJECT` store → B on fd2 (`clientCopyResource`, `rs_client.c:537-551` — B has
   DUP right; dest validated against fd2 ✓). *Alternative without any share:* export the store to an
   fd (`0x3d05`, `NON_PRIVILEGED`, `g_client_resource_nvoc.c:1638-1640`) and import it into B
   (`0x3d06`) — the import is a **kernel-internal** dup (`rmobjexportimport.c:580,637`,
   `RMAPI_API_LOCK_INTERNAL`), so no `RS_ACCESS_DUP_OBJECT` is consulted; the fd is the capability.
4. S dups I's bare `FERMI_VASPACE_A` from A into B — **no grant needed**, same `ProcID`
   (`sharing.c:341-352`). (This is also why w746's `InsufficientPermissions` disappears for any
   dup whose destination has I's `ProcID`.)
5. S allocates the channel **in B** on fd2: `hVASpace = the VAS dup`, `hUserdMemory[0] = the store
   dup`, `userdOffset[0] = the guest's USERD offset` (known from the guest's alloc RPC,
   `userd_mem_is_on_the_wire.md`). Stamps: `ProcessID = I` (`:293`), `_PRIVILEGE_USER` (`:290`,
   S's call is unprivileged — measured w746: `rmclientIsAdmin == false` for S).
6. S **frees the store dup in B** (`NV_ESC_RM_FREE` on fd2). The channel keeps its sub-memdesc.
7. S drives the channel through fd2: engine objects, work-submit token
   (`NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN`), schedule, and the doorbell (S's own usermode
   mapping; the token is what hardware checks).

**What it keeps.** §30's two stamps, by construction and readable (§6). §26's "I never holds an
`hMemory` for vidmem" — I never sees the dup: it exists only in B, which I cannot reach after step
2, and it is gone after step 6. §26's "never receives the guest-RAM memfd" — untouched. §17 —
nobody is privileged. §31 — the USERD is the channel's own sub-memdesc; teardown of the channel
destroys it; the store never moves, so there is no snapshot hazard. §18/§22 — the USERD is the
guest's own vidmem page, in the store, no substitution. §28 — unchanged. One-store — unchanged.

**What it costs — named so the owner can rule.**
- ★ **The channel's handle lives in B, and only S can reach B.** A `KernelChannel` **cannot be
  duped** (`rs_server.c:1719` + `resCanCopy_IMPL` `NV_FALSE`, `rs_resource.c:334-340`; w748 §2.0),
  so I can never hold the channel in A. §26's table row *"per-proc isolate owns: channel"* becomes
  *"S administers a channel that carries I's identity"*. Whether that satisfies the owner's intent
  behind §30 (*"the channel is created in an unprivileged process … cross guest process isolation"*)
  is a **ruling**: the creating **call** is unprivileged and the creating **client** is I's; the
  administering process is S. This document does not rule.
- **A new seam**: fd passing I→S, with S holding one extra fd per guest process. The `HandedVaSpace`
  newtype discipline (§29.3) extends by one type — `HandedClient` — and the F11 rule becomes *"I may
  never name a foreign client; S may name a VA space the VMM handed it **and a client I minted and
  surrendered**."* Mutations that must go red: I keeping a copy of fd2; S receiving a client it did
  not get from I; the dup outliving step 6.
- **A transient window inside S** between steps 3 and 6 in which B holds a full-use store handle.
  Only S can act in that window (fd2 is S's). S is already trusted with everything (§26's trust
  statement). ⚠ Still, the free in step 6 must be a barrier, not a courtesy, and the census must
  count `dup_outstanding` — a zero here needs a known-positive (skip step 6 in a test and watch the
  gate fire).
- **UVM.** libcuda's forwarded `UVM_REGISTER_CHANNEL {rmCtrlFd, hClient, hChannel}` would name
  `(B, channel)` from I's UVM fd. UVM does **not** validate `rmCtrlFd`
  (`uvm_user_channel.c:945-948`: *"TODO: Bug 1624521: This interface needs to use rmCtrlFd to do
  validation"*) and `nvGpuOpsRetainChannel` performs no pid check (grep of its body for
  `pid|ProcID|Validate` is empty) — **INFERRED** viable; the probe is the forwarded
  `UVM_REGISTER_CHANNEL` itself returning `NV_OK`, with `UVM_REGISTER_GPU_VASPACE` on `(A, VAS)`
  unchanged from today.
- **Zeroing.** CPU-RM zeroes a client USERD at birth **only if it is `ADDR_SYSMEM`** (or FBMEM under
  full SR-IOV): `kernel_channel.c:2342-2356`. For a vidmem slice the CPU side writes nothing; the
  GSP side is closed. See §7 for the discriminator; K births at the guest's alloc RPC, before the
  guest can write `GP_PUT`, so the ordering is safe either way.

**Probe shape (no production code; one boot).** Two ctl fds in a test binary that forks: child
(role I) allocs client B on fd2, sends fd2, closes; parent (role S, unprivileged, separate tgid)
performs steps 3–6 and then a `NV_ESC_RM_FREE` of the dup, then births a second channel to prove B
still works. **Discriminators:** (i) `NVOS04_FLAGS_PRIVILEGED_CHANNEL` bit 5 **clear** on the
copied-out params (§6) — the privilege stamp; (ii) `NV2080_CTRL_CMD_GPU_GET_PID_INFO` /
`nvidia-smi` listing the channel under **I's pid**, not S's — the identity stamp
(`gpu_rmapi.c:1006`, `rs_utils.c:310` key on the client's `ProcID`); (iii) after step 6,
`NV_ESC_RM_MAP_MEMORY` of the freed dup handle on fd2 returns `NV_ERR_OBJECT_NOT_FOUND` while the
channel's `GP_GET` still advances on a real submission — the dup is gone and the USERD is not.
A green (i) with a pid in (ii) equal to S's would mean the client binding is not what
`client.c:112` says; record it, do not explain it away.

### 3.2 U — `UPDATE_CHANNEL_INFO` issued by S (§2)

Privileged; costs §17. If F is ever admissible, U dominates it (§2). Not ranked above any
unprivileged route.

### 3.3 X — fd export/import as the share mechanism (a mechanism, not a solution)

`NV0000_CTRL_CMD_OS_UNIX_EXPORT_OBJECT_TO_FD` (`0x3d05`) / `IMPORT_OBJECT_FROM_FD` (`0x3d06`), both
`NON_PRIVILEGED` (`g_client_resource_nvoc.c:1638-1655`). Export dups into an RM-internal export
client with `RMAPI_API_LOCK_INTERNAL` (`rmobjexportimport.c:331`, `NV01_ROOT` at `:261`); import
dups from it into the destination the same way (`:580,637`). `clientCopyResource` skips the
rights check for `privLevel >= KERNEL` (`rs_client.c:537-541`) ⇒ **no `RS_ACCESS_DUP_OBJECT`
grant is ever consulted; possession of the fd is the capability.** This is CUDA IPC's own route.
It changes nothing about *what* is shared (a whole object), so on its own it is G; it matters as
the second door for K's step 3 and as the explanation of why w746's refusal is a policy fact, not a
hardware one.

### 3.4 C — refined cost, beyond w748

w748 §1 has the `ProcessID` consumer census and §2.0 the "channel cannot be duped" fact. Two
additions:
- **`SubProcessID` is the vendor's own per-guest-process tag for exactly C's shape.**
  `NV0000_CTRL_CMD_SET_SUB_PROCESS_ID` (`0x901`, `NON_PRIVILEGED`, `client_resource.c:4754-4766`)
  sets a per-**client** sub-process id consumed by GPU accounting (`device.c:431-437, :493`), vidmem
  usage reporting (`video_mem.c:1032-1044`), and pid-info lookup (`gpu_rmapi.c:1006`,
  `rs_utils.c:310`). It is what the vGPU plugin uses to say *"this plugin-owned client is guest
  process N"*. ⇒ under C, one client per guest process in S, each tagged with the guest pid, restores
  the *reporting* identity. It does **not** touch the profiler gate (`kern_profiler_v2.c:656` keys
  on `ProcID`) and it is *set by S*, so it is bookkeeping, not a security fact.
- **C therefore has two sub-shapes with different costs:** C1, one S client for all guest channels
  (every stamp is S's, indistinguishable); C2, one S-created client per guest process with
  `SubProcessID` set — reporting distinguishes, kernel gates do not. K is what C2 becomes when the
  *client* is minted by I instead of S: identical administration, real stamps.

### 3.5 The sysmem-USERD family — the host half exists, the guest half does not

Recorded so it is not re-derived. Host side: an **unprivileged** client may create
`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` from a **dma-buf fd** — `NVOS32_DESCRIPTOR_TYPE_OS_FILE_HANDLE`
(`nvos.h:629`) is permitted for user clients (`osmemdesc.c:113-130` restricts only `DMA_BUF_PTR`,
`SGT_PTR`, `PHYS_ADDR` to kernel), imported via `nv_dma_import_from_fd` (`osmemdesc.c:989`,
`nv-dmabuf.c:1867-1890`), and the result is `ADDR_SYSMEM` (`osmemdesc.c:798-800`). A `udmabuf`
over a **4 KiB range** of the guest-RAM memfd would be a range-restricted fd S could hand I without
handing it the memfd, and `kchannelCreateUserdMemDesc_GV100` explicitly expects `OsDescMemory`
USERDs (`kernel_channel_gv100.c:250-254`, `refAddDependant`). UVM itself uses a sysmem client
USERD when `uvm_channel_gpput_loc=sys` (`nv_gpu_ops.c:5976-5989`), and w233 measured host RM
accepting one. ⇒ the primitive is real, unprivileged, and vendor-shaped.
**Dead because the guest never puts USERD in sysmem** (§1.1), and putting it there for the guest
would be §18's substitution. ⊘ The vidmem mirror of the idea — `NV_ESC_RM_EXPORT_OBJECT_TO_DMA_BUF`
with `(handles[], offsets[], sizes[])` ranges (`nv-ioctl.h:134-148`) re-imported on the same GPU —
is dead by shape: the import is always `ADDR_SYSMEM`, the exporter's attach gates on
`nv_grdma_pci_topology_supported(self)` (`nv-dmabuf.c:1036`), and the best case is the GPU DMAing
to its own BAR1 through the root complex. RM has **no** importer that yields a vidmem memdesc.

### 3.6 G — the rights vocabulary is four bits, none of them "map"

`rs_access.h:59-63`: `DUP_OBJECT, NICE, DEBUG, PERFMON`. There is no right that gates
`NV_ESC_RM_MAP_MEMORY` or `NVOS46` on a memory object. A "restricted-rights dup" of the store is a
full-use store handle in I. G costs exactly what the brief says; the share/dup machinery cannot
narrow it. (What *can* narrow it is the object's own flags at alloc — `NVOS32_ALLOC_FLAGS_USER_READ_ONLY`
/ `_DEVICE_READ_ONLY`, `nvos.h` roster — which would cripple the store for everyone.)

---

## 4. The privilege half of C, re-read: `NV01_ROOT_NON_PRIV`

`client.c:88` `bIsRootNonPriv = (externalClassId == NV01_ROOT_NON_PRIV)`; `rmclientIsAdmin` is
`privLevel >= USER_ROOT && !bIsRootNonPriv` (`:393`). ⇒ a `CAP_SYS_ADMIN` process can hold a client
whose channels are stamped `_PRIVILEGE_USER` — the escape w748 §6.1 names. ⚠ It does **not** help
the *control* gates: `RMCTRL_FLAGS_PRIVILEGED` (§2) and `RS_FLAGS_ALLOC_PRIVILEGED`
(`alloc_free.c:599-605`) check the **call's** `privLevel`, which stays `USER_ROOT`. So a privileged
S with a NON_PRIV client could mint `LIST_OBJECT` and issue `UPDATE_CHANNEL_INFO` while birthing
USER channels — everything §17 forbids, with the §30 stamp fixed. Recorded because it is the
precise shape of the "if 17 were ever relaxed" world; it is not a route under the constraints.

---

## 5. Where the stamps come from — one table, so the ranking can be checked

| stamp | source | line | fixed by |
|---|---|---|---|
| `KernelChannel::privilegeLevel` | `pCallContext->secInfo.privLevel`, `rmclientIsAdmin(client)` | `kernel_channel.c:277-292` | the **calling task's** `capable(CAP_SYS_ADMIN)` at the alloc ioctl (`escape.c:304`, `nv-linux.h:537`), and the client's class (`NON_PRIV`) |
| `KernelChannel::ProcessID` | `pRmClient->ProcID` | `:293` ← `client.c:112` | the **task that allocated the client** |
| `KernelChannel::SubProcessID` | `pRmClient->SubProcessID` | `:294` ← `client_resource.c:4766` | whoever holds the client's fd |
| USERD physical address | `memdescGetPhysAddr(sub-memdesc)` | `kernel_channel_gv100.c:204`, `_gm107.c:328` | the handle resolved **in the allocating client** (`_gv100.c:183-190`) |
| client ↔ caller binding | `pOSInfo == clientOSInfo` (STRICT default) | `client.c` `rmclientValidate_IMPL`, `g_system_nvoc.c:104` | the **`struct file`**, which `SCM_RIGHTS` carries |

Every route in this document is a choice of which task performs which of the three actions
(create the client · issue the alloc · hold the memory handle); the table says which stamp each
action fixes. K is the assignment `{I, S, B-transient}`; C is `{S, S, S}`; the brief's I-only world
is `{I, I, I}` and fails on the third.

---

## 6. ★ A free, fail-closed assert on the privilege stamp — for every route

`kernel_channel.c:281` and `:286` do `pChannelGpfifoParams->flags = FLD_SET_DRF(OS04, _FLAGS,
_PRIVILEGED_CHANNEL, _TRUE, …)` **in the alloc params** when the channel is stamped KERNEL or
ADMIN; the USER arm (`:290`) leaves them alone. `NVOS04_FLAGS_PRIVILEGED_CHANNEL` is bit `5:5`
(`alloc_channel.h:141`). RM copies alloc params back to the caller on success
(`alloc_free.c:207-211`: copy-out is skipped only `if (status != NV_OK)`).
⇒ **Submit with bit 5 clear; on `NV_OK` read bit 5 back. Set ⇒ the driver itself says the channel
is privileged; refuse and tear down.** This is §30's *"asserted at birth and refusing otherwise"*,
sourced from the driver's writeback rather than from our belief about euids. ⚠ It must be paired
with a known-positive: birth one channel from a `CAP_SYS_ADMIN` task in a test and watch the bit
come back set. **INFERRED** only that no later code clears the bit before copy-out (the
`pChannelGpfifoParams` object is the one copied out; nothing between `:294` and return writes
`flags` in the USER arm as read here).

---

## 7. The zeroing question is a probe, and it orders every route

> ### ⊘⊘⊘ MEASURED 2026-09-16 (w750) — **THE ANSWER IS "ZEROED", i.e. THE OPPOSITE OF WHAT
> ### THE SOURCE READING BELOW PREDICTS, AND THE BRANCH IT ORDERS IS THE OTHER ONE.**
> The text below says the CPU side writes nothing for a **vidmem** slice, and this document's
> reader (and `w750_route_k_prereg.md` row 5, at ★★★ 0.8) took that to mean the guest's poison
> survives a birth. **On a PF host, open kernel module 580.159.04, GA106, it does not:**
>
> ```
> K_USERD_BEFORE_BIRTH = a5a50000 a5a50001 a5a50002 a5a50003 a5a50004 a5a50005 a5a50006 a5a50007
> K_USERD_AFTER_BIRTH  = 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
> K_USERD_POISON_INTACT_WORDS = 0 of 128
> ```
> (`traces/w750_route_k/route_k_run3.log`; the probe is `kayfabe-rm-ladder --route-k`.)
>
> ★ Both of this section's own falsifiers are closed. The poison was **read back before the
> birth** — so *"the write-combining store never landed"*, which produces an identical
> `0 of 128`, is ruled out. And `K_CHANNEL_LIVE=1` with the engine's own release semaphore
> landing — so *"a wrong `userdOffset` poisoned a different page"* is ruled out: the channel
> ran, over the page that was poisoned.
>
> ⇒ **Take this section's FIRST branch, not its second:** adoption must precede the guest's
> first `GP_PUT` **for every route**. K does by construction. Any `UPDATE_CHANNEL_INFO`-shaped
> route must re-point **before the first doorbell** or it wipes the guest's cursor mid-flight.
>
> ⚠ **And what is NOT settled: WHICH code performs the scrub.** The reading below of
> `kernel_channel.c:2342-2356` is not wrong — that arm really is gated on `ADDR_SYSMEM` (or
> `ADDR_FBMEM` under full SR-IOV), and a PF host is neither. So **something else zeroes it**,
> and this probe does not say what. Recorded as open rather than explained away.
> ★★ **One thing the same run already narrows, for free:** the scrub is **targeted at the USERD
> extent, not at the object**. The same store object carries the GPFIFO entry at offset `0` and
> the pushbuffer at `0x2000`, both written before the birth, and the channel then **fetched that
> entry and executed that pushbuffer** (`K_CHANNEL_LIVE=1`, the release semaphore landed). Those
> bytes survived; only the 512 B at `userdOffset` did not — which is exactly
> `kfifoSetupUserD_HAL`'s shape (`kernel_fifo_gm107.c:797-808`, 512 B).
> ⇒ the open question is **not** *"what wrote zeros"* but *"which arm reaches
> `kfifoSetupUserD_HAL` on a PF host with the open module"*, given that this section's guard
> reads `ADDR_SYSMEM || (ADDR_FBMEM && bFullSriov)` and a PF host is neither.
>
> ⊘ Same class this campaign keeps paying for: *a correct reading of one code path is not a
> statement about the observable end state.*


w233 measured: host RM zeroes a **sysmem** client USERD at birth. Source: CPU-RM does so only for
`ADDR_SYSMEM` (`kernel_channel.c:2342-2356`, `kfifoSetupUserD_GM107` `kernel_fifo_gm107.c:797-808`,
512 B). For a **vidmem** slice the CPU side writes nothing; the GSP side is closed.
**Probe**: pre-poison the store at the USERD offset through S's view, birth a channel over it,
read back. **Discriminator**: zeroed ⇒ GSP zeroes vidmem client USERDs too, so adoption must
precede the guest's first `GP_PUT` for every route (K does by construction; U/U′ must re-point
before the first doorbell, and `bClientAllocatedUserD` guarantees the earlier USERD was I's own, so
the guest's page is never wiped mid-flight); unchanged ⇒ vidmem USERDs may be adopted late, which
would make U/U′ orderings free and would *also* mean the guest's poison survives birth — a second
fact worth having.

---

## 8. Ranking — mine, with the reasoning, for comparison rather than merging

Ranked by **which property each costs**, then by how much of it is measured vs inferred.

| rank | route | property cost | measured / inferred | why here |
|---|---|---|---|---|
| 1 | **K** — I mints the birth client, S drives it via a passed fd, transient store dup freed at birth (§3.1) | **none of 17/18/22/26-memory/28/30/31**; costs the *location of the channel handle* (S, not I — a §26 table row, ruled by the owner) and one new typed seam | stamps, binding, default PID share, refcounts, share/dup gates: **source-measured**; UVM registration from I of a B-owned channel: **INFERRED**; vidmem zeroing: **probe** (§7) | Every stamp is I's by construction and readable (§6). Nothing privileged. I never sees vidmem or the memfd. It is C2 with the identity made real, and it is the only unprivileged route in which `hUserdMemory` is resolved in a client whose `ProcID` is I's. |
| 2 | **C2** — S births in one S-owned client per guest process, `SubProcessID` = guest pid (§3.4) | `ProcessID` identity (reporting restored, gates not); nothing else | all measured | Same administration as K without the fd seam; loses only the kernel-visible identity, which K shows is recoverable for the price of that seam. |
| 3 | **E′/E** — RM-pool USERD, guest's BAR1 view re-pointed | the one-store invariant (a control page whose store slice goes dark) | the C did this; measured as a mechanism | Cheap and known; costs a named invariant, so below anything that costs none. |
| 4 | **D** | one-store invariant + three followers | — | strictly worse than E. |
| 5 | **U** — `UPDATE_CHANNEL_INFO` by S (§2) | **17** (`CAP_SYS_ADMIN` on S) | gate: source-measured | The verb that exactly fits; the gate is the one absolute constraint. Dominates F under F's own premise. |
| 6 | **F** — `LIST_OBJECT` slice, privileged S (w748 §6.1) | **17**, plus a second object with its own lifetime (§31) | measured 0x1b/NV_OK | Below U because U needs no shared object at all. |
| 7 | **U′** — deferred-API `UPDATE_CHANNEL_INFO` (§2.1) | unknown; two closed-source unknowns | **INFERRED** throughout | Cheap to probe; low prior. |
| 8 | **G** — whole-store dup into I (§3.6, §3.3) | **26** by its own words; no rights narrowing exists | measured | Out by constraint. |
| — | **A**, **B** | — | **dead** (§1) | Do not probe. |
| — | sysmem-USERD family (§3.5) | — | **dead on the guest side** | Do not probe; the host primitive is recorded for when a *guest* actually places a USERD in sysmem, which a guest registry key can make happen and we would only learn from `userdMem.addressSpace`. |

⚠ What would move K down: (a) the owner reading §30's *"created in an unprivileged process"* as
*"held by the per-proc process"* — then K and C are the same posture and the ranking collapses to
K ≡ C2 with an extra seam; (b) `UVM_REGISTER_CHANNEL` refusing a channel whose client fd the
caller cannot present (§3.1, INFERRED viable); (c) a future driver adding the pid check
`uvm_user_channel.c:948` promises — a capture-derived dependency of the class
`a_capture_derived_table_expires_as_a_vendor_regression` names, and it should be written down as
an expiry condition on the K design, not discovered.

---

## 9. Probes, each with the step that tells the right answer from a plausible wrong one

1. **K birth-client probe** (§3.1) — discriminators (i) bit-5 readback clear, (ii) pid-info shows
   I's pid, (iii) freed dup unmappable while `GP_GET` still advances. Known-positive for (i): a
   `CAP_SYS_ADMIN` birth returns bit 5 set.
2. **Vidmem zeroing** (§7) — poison / birth / read; a zeroed page is the loud answer, an unchanged
   page must be re-read after the first submission to confirm the birth actually programmed that
   page (a wrong `userdOffset` also leaves the poison intact — pair it with `GP_GET` movement).
3. **UVM registration of a B-owned channel from I's UVM fd** — `NV_OK` vs
   `NV_ERR_INSUFFICIENT_PERMISSIONS`/`INVALID_CLIENT`; and the negative control: register with a
   deliberately wrong `hClient` and confirm UVM does refuse *something*, so a green is not vacuous.
4. **U′ deferred path** (§2.1) — three-way discriminator recorded there.
5. **§1.3 fixed-address refusal** — one alloc; only worth running if someone proposes it again.

⊘ Not proposed: anything on A or B; any probe that "measures" G.
