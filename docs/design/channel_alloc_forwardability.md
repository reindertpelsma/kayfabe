# Channel alloc forwardability — what RM honours, what it derives, and what a verbatim forward may not touch

**STATUS: LIVE (2026-08-13).** Read-only source study, no boot, no build. Ground truth is
`research_clones/ogkm-580.159.04/` (the bench driver version) in the `nvidia-gpu-passthrough`
tree; kayfabe citations are marked with the revision they were read at, because
`origin/master` (`e758778`) is materially behind the in-flight lane branch
`w284-ce-passthrough-for-real` (`b0d6de7`) and the two disagree on this exact code.

Scope: `NV_CHANNEL_ALLOC_PARAMS`, class `0x906f`/`NVA06F`-family, on **GA10x / 580.159.04**.
Every claim below cites a file and line that was opened. Where a claim is a *negative*
("nothing reads this"), the grep that produced it is stated together with a **known-positive
control** run through the same pattern.

---

## §0 LEAD — five things that contradict the brief

The brief's design ("forward the alloc verbatim, keep only a doorbell-token translation")
**survives**, but four of the five reasons given for it are wrong, and one of the wrong ones
is a security-relevant field.

### ⊘ 0.1 — The per-field split is right for the FORWARD and wrong for the RECEIVE. They are different code paths.

The brief treats `NV_CHANNEL_ALLOC_PARAMS` as one struct with one honoured
representation, and asks which fields to translate. There is no such single answer.
**RM honours the handle+offset pair on the CPU-RM path and the `NV_MEMORY_DESC_PARAMS`
descriptor on the GSP path, and the two are mutually exclusive by an explicit `if`.**

- `kernel_channel.c:2299-2312` — if **not** GSP and **not** an SRIOV VF, RM resolves
  `hUserdMemory[]` + `userdOffset[]` and sets `bClientAllocatedUserD = NV_FALSE`; otherwise
  it sets it `NV_TRUE` and never looks at the handles.
- `kernel_channel.c:2315-2331` — the descriptor consumer, `_kchannelDescribeMemDescsFromParams`,
  is reached **only** on the GSP/VF arm.
- `kernel_channel.c:2388-2390` — that function opens by *asserting* it is on the GSP or VF arm.
- The comment that says it outright, `kernel_channel.c:2294-2298`:
  > *"GSP RM and host RM on full SRIOV setup **will not be aware of the client allocated userd
  > handles**, translate the handle on client GSP. GSP RM or host RM on full SRIOV setup will get
  > the **translated addresses** which it will later memdescribe."*

⇒ **We are the GSP.** The representation a real GSP would honour in the message we receive is
the **descriptor** — the very thing the brief says is never forwardable. `hUserdMemory[]` and
`userdOffset[]` arrive on our wire (`kernel_channel.c:2675-2683` copies them into the RPC
unconditionally) but a real GSP-RM **ignores them**.

⇒ And the forward is not an RPC to a GSP; it is an `NV_ESC_RM_ALLOC` to the **host CPU-RM**,
which takes the *other* arm — so on the forward the handles are honoured and the descriptors
are dead. The design is therefore **not "same struct, translate the handles"**. It is a
**re-derivation across an asymmetry**: we consume one representation and must produce the other.
Good news: that is exactly what the isolate already does. Bad news: §2's row for
`hUserdMemory[0] == 0` has no handle to translate at all.

### ⊘⊘ 0.2 — `NVOS04_FLAGS_PRIVILEGED_CHANNEL` is NOT trusted-by-construction. It is guest-userspace-settable and ogkm never clears it.

This is the sharpest correction here. The brief's position — *"a guest userspace process that
asked for privileged and wasn't entitled has been dropped by ogkm before any RPC"* — is **false
as stated about bit 5:5**.

Exhaustive grep, `PRIVILEGED_CHANNEL` over `src/nvidia/` + `src/common/`, excluding
`generated/` — **five hits, three of them code**:

| site | what it does |
|---|---|
| `kernel_channel.c:252-256` | **reads** it — but only under `gpuIsSriovEnabled(pGpu) && IS_GFID_VF(callingContextGfid)`, i.e. the GSP-side SRIOV-VF case |
| `kernel_channel.c:281` | **SETS it TRUE** when the caller is `RS_PRIV_LEVEL_KERNEL` |
| `kernel_channel.c:286` | **SETS it TRUE** when `rmclientIsAdmin()` or `hypervisorCheckForObjectAccess()` |
| `alloc_channel.h:141-143` | the definition |

There is **no** `FLD_SET_DRF(..., _PRIVILEGED_CHANNEL, _FALSE, ...)` anywhere in the tree. The
unprivileged arm, `kernel_channel.c:288-292`, sets `privilegeLevel = ..._PRIVILEGE_USER` and
**leaves `flags` untouched**. `flags` is then copied straight into the RPC at
`kernel_channel.c:2668`.

⇒ **A guest userspace process can set bit 5 of `flags` and we will receive it TRUE while the
guest kernel's own verdict is `USER`.** The header even says so, `alloc_channel.h:135-140`:
> *"This flag tells RM whether to give the channel admin privilege. **This flag will only take
> effect if the client is GSP-vGPU plugin.**"*
— i.e. upstream treats it as a no-op input on every path except vGPU, so nobody sanitizes it.

**The field that IS adjudicated is `internalFlags[1:0]`,** and the reason it is trustworthy is
structural rather than incidental:
- `kernel_channel.c:220` — `pChannelGpfifoParams->internalFlags = 0;` **unconditionally**, at the
  top of construct, under the comment *"Internal fields must be cleared when RMAPI call is from
  client"* (`:218`). Anything guest userspace put there is destroyed before it is read.
- `kernel_channel.c:2804-2807` — refilled from `pKernelChannel->privilegeLevel`, the kernel's own
  verdict, under the comment *"GSP client needs to pass in privilege level as an alloc param since
  GSP-RM cannot check this"*.
- Bit layout, `generated/g_kernel_channel_nvoc.h:181-184`: `PRIVILEGE` is `1:0`,
  `USER=0 / ADMIN=1 / KERNEL=2`. `UVM_OWNED` is `7:7` (`:202-204`), `GSP_OWNED` is `6:6` (`:197-199`).

⇒ **The owner's conclusion is right and the owner's field is wrong.** The alloc *has* been
adjudicated by the guest kernel before it reaches us — in `internalFlags`, not in `flags`.
Discriminating on `flags[5:5]` hands guest userspace the steering wheel for the
passthrough-vs-emulate fork.

★ And we are **already decoding the dword that carries it and throwing the bits away**:
`crates/kayfabe-abi/src/notifier.rs:182-184` pins `internal_flags: 244` for 580 (`:190-192`,
`252` for 610), and `:249-250` reads that `u32` and keeps only bits `[3:2]`
(`NOTIFIER_TYPE_SHIFT`/`_MASK`). Bits `[1:0]` and `[7:7]` are read off the wire and discarded.
Cost of using them: zero new wire archaeology. *(read at `origin/master` `e758778`.)*

### ⊘ 0.3 — Client identity DOES reach us. The owner's doubt is refuted at the wire level.

> *"I think we do not have access to 'which client' since client is a bookkeeping in the kernel
> module, that's not forwarded to us."*

It is forwarded, at **byte 0 of the RPC body**.

- Envelope, `generated/g_rpc-structures.h:1491-1502`:
  `rpc_gsp_rm_alloc_v03_00 { NvHandle hClient; NvHandle hParent; NvHandle hObject; NvU32 hClass;
  NvU32 status; NvU32 paramsSize; NvU32 flags; NvU8 reserved[4]; NvU8 params[]; }`
- Filled at `vgpu/rpc.c:11201` — `rpc_params->hClient = hClient;` inside `rpcRmApiAlloc_GSP`,
  which writes `NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC` (`:11196-11198`).
- The channel takes that path: `NV_RM_RPC_ALLOC_CHANNEL` (`inc/kernel/vgpu/rpc.h:245-268`)
  branches on `IS_FW_CLIENT(pGpu)` → `pRmApi->AllocWithHandle(GPU_GET_PHYSICAL_RMAPI(pGpu), ...)`.
  `IS_FW_CLIENT(pGpu) == IS_GSP_CLIENT(pGpu) || IS_DCE_CLIENT(pGpu)`
  (`generated/g_gpu_nvoc.h:5686`), and for a GSP client the physical RMAPI's `AllocWithHandle`
  **is** `rpcRmApiAlloc_GSP` (`rmapi/rpc_common.c:74-80`).
- The argument is the channel's own owning client:
  `kernel_channel.c:2819-2826`, `NV_RM_RPC_ALLOC_CHANNEL(pGpu, RES_GET_CLIENT_HANDLE(pKernelChannel), ...)`.

And we already capture and key on it: `crates/kayfabe-abi/src/versions.rs:1543-1562`
(`decode_rpc_alloc`, `client: u32_at(payload, 0)`), `crates/kayfabe-abi/src/view.rs:101-129`
(`RpcAllocReq`), `crates/kayfabe-rmrpc/src/lib.rs:1237` (`let client = HClient(h.client);`),
`crates/kayfabe-core/src/rmgraph.rs:1073` (`client_roots: BTreeMap<HClient, ResId>`).
*(all at `origin/master` `e758778`.)*

⇒ **"Adjudicate against which client" is buildable today.** ⚠ Note what is *not* in the envelope:
there is no `privLevel`/`secInfo` field — which is precisely why NVIDIA had to smuggle privilege
into the *params* (§0.2). So client identity and channel privilege are two separate facts and
both are available.

### ⊘ 0.4 — `flags` cannot be forwarded verbatim to a host ioctl. The guest has encoded its own ChID in it.

`kernel_channel.c:2786-2802`, on the `IS_GSP_CLIENT` arm, under the comment
*"Setting these param flags will make the Physical RMAPI use our ChID (which is already decided)"*:
```
CHANNEL_USERD_INDEX_FIXED      := FALSE
CHANNEL_USERD_INDEX_PAGE_FIXED := TRUE
CHANNEL_USERD_INDEX_VALUE      := ChID % 8      (bits 10:8)
CHANNEL_USERD_INDEX_PAGE_VALUE := ChID / 8      (bits 20:12)
```
The consumer is `arch/maxwell/kernel_channel_gm107.c:456-476`: `PAGE_FIXED_TRUE` sets
`bForceUserdPage`, and `userdPageIdx`/`internalIdx` are handed to `kfifoChidMgrAllocChid`
(`:478-490`) as a **demand for a specific hardware channel ID**.

⇒ Forwarding those bits verbatim asks the *host* RM to place our host channel at the *guest's*
ChID. That is a namespace we do not own and cannot honour; it will collide or be refused.
**Bits 21:8 of `flags` must be stripped on the forward.** (The isolate today sets `flags: 0` —
`crates/kayfabe-isolate-host/src/rm.rs:5030` at `b0d6de7` — which is correct for this reason and
not, as far as the code says, for a reason anyone wrote down.)

### ⊘ 0.5 — the `rm.rs:4152` premise and the "collapses to one case" premise are both stale.

The brief describes `RingOwner::{Ours,HandedIn}` as collapsing to one case, and names a latent
double-free at `rm.rs:4152`. At `origin/master` `e758778`, `rm.rs:4152` is
`RingOwner::Ours,` inside the `RingSource::Ours` arm of `alloc_channel_in`; the doc comment
above the enum (`rm.rs:556-559`) claims *"`RmBackend::alloc_channel` lowers unconditionally to
`RingSource::Ours(None)`"*.

At the lane HEAD `b0d6de7` that is **no longer true**, and there is now a **second** ownership
axis the brief does not mention: `UserdOwner::{Ours,HandedIn}`, with the guest's USERD handed in
together with a non-zero `userd_offset` (`rm.rs:4890-4910`, `:5054`). The `RingSource::Guest`
arm is live. ⇒ Any plan written against "one case" is planning against a tree that no longer
exists — and this is the same class as the trap the brief itself flags: **a doc comment is not
the code.** The doc comment at `:556-559` is now stale *at HEAD* and still says otherwise.

---

## §1 The mechanism: one struct, two honoured representations

```
GUEST USERSPACE  --ioctl-->  GUEST CPU-RM  --GSP RPC-->  US (fake GSP)
                                  |
                                  |  honours: hUserdMemory[] + userdOffset[]   (handle+offset)
                                  |           hObjectError                     (handle)
                                  |  derives: instanceMem, ramfcMem, userdMem,
                                  |           mthdbufMem, errorNotifierMem,
                                  |           eccErrorNotifierMem, ChID, cid,
                                  |           internalFlags, ProcessID
                                  v
                          the message we receive carries BOTH representations,
                          but a real GSP honours ONLY the descriptors

US  --NV_ESC_RM_ALLOC-->  HOST CPU-RM  (IS_GSP_CLIENT, PF, not VF)
                                  |
                                  |  honours: hUserdMemory[] + userdOffset[], hObjectError
                                  |  IGNORES: every NV_MEMORY_DESC_PARAMS  (no reader on this arm)
```

The `if` that splits them, verbatim from `kernel_channel.c:2299-2312`:
```c
if (!(RMCFG_FEATURE_PLATFORM_GSP && !pKernelChannel->bGspOwned) &&
    !(IS_GFID_VF(gfid) && !gpuIsWarBug200577889SriovHeavyEnabled(pGpu)))
{
    pKernelChannel->bClientAllocatedUserD = NV_FALSE;
    NV_ASSERT_OK_OR_GOTO(status,
            kchannelCreateUserdMemDescBc_HAL(pGpu, pKernelChannel, hClient,
                pChannelGpfifoParams->hUserdMemory,
                pChannelGpfifoParams->userdOffset),
            failed);
}
else
{
    pKernelChannel->bClientAllocatedUserD = NV_TRUE;
}
```

★ The same asymmetry holds for the **error notifier**, and the brief does not mention it:
`kernel_channel.c:522-533` calls `kchannelGetNotifierInfo(... hErrorContext ...)` only when
`!(RMCFG_FEATURE_PLATFORM_GSP && !bGspOwned)`; on the GSP arm RM instead uses the
`errorNotifierMem` descriptor plus `internalFlags[3:2]` (`ERROR_NOTIFIER_TYPE`, filled at
`:589-596`). ⇒ `hObjectError` is a **dead field on our receive side** in exactly the way
`hUserdMemory[]` is.

**⇒ The good news, and it is the load-bearing conclusion for the design:** on the *forward*,
every `NV_MEMORY_DESC_PARAMS` in the struct has **zero readers**. Grep for
`->instanceMem`, `->ramfcMem`, `->userdMem`, `->mthdbufMem` over `src/nvidia/` excluding
`generated/` yields readers **only** inside `_kchannelDescribeMemDescsFromParams`
(`kernel_channel.c:2406-2500`) and writers in the RPC senders (`kernel_channel.c:2730-2782`,
`vgpu/rpc.c:3538-3553`). Nothing on the CPU-RM path reads them.
⇒ **Forwarding a guest GPGA descriptor verbatim is inert, not dangerous** — the host RM will
neither read it nor be misled by it. The owner's rule (*never forward them*) is correct as
hygiene; it is not load-bearing for correctness on this path. What *is* load-bearing is that we
must **supply the other representation**, because the host RM will otherwise allocate its own.

---

## §2 The per-field table

`ogkm-580` line numbers unless noted. **V** = forward verbatim, **T** = translate,
**⊘** = never forward / strip, **D** = RM derives it, do not fight.

| field | offset (580) | on our RECEIVE | on our FORWARD | why |
|---|---|---|---|---|
| `hObjectError` | +0 | **dead** — GSP uses `errorNotifierMem` + `internalFlags[3:2]` (`:522-533`, `:589-596`) | **T** | host CPU-RM resolves it in the caller namespace (`:522-527`, `kchannelGetNotifierInfo` `:1988+`) |
| `hObjectBuffer` | +4 | always `0` — forced (`:2663`) | ⊘ | sender zeroes it |
| `gpFifoOffset` | +8 | guest VA, **untouched** | **V** | §3.1 — zero readers in ogkm |
| `gpFifoEntries` | +16 | guest count, **untouched** | **V** | §3.1 — zero readers in ogkm |
| `flags` | +20 | bits 21:8 rewritten by guest to encode **its** ChID (`:2786-2802`); bit 5:5 **guest-userspace-forgeable** (§0.2) | **partial** — strip 21:8 | `kernel_channel_gm107.c:456-490` treats 21:8 as a ChID demand |
| `hContextShare` | +24 | guest handle | **T** or 0 | `:190`, `:307-313`; mutually exclusive with `hVASpace` |
| `hVASpace` | +28 | guest handle | **T** | resolved in the caller's namespace, `:988-1002` |
| `hUserdMemory[8]` | +32 | present but **a real GSP ignores it** (`:2294-2298`) | **T** | host CPU-RM resolves it, `kernel_channel_gv100.c:184-192` |
| `userdOffset[8]` | +64 | offset **within** the named object — namespace-free | **V** | `kernel_channel_gv100.c:204-206`, `:228-237` |
| `engineType` | +128 | guest value | **V** | inherited from the group in practice |
| `cid` | +132 | **[OUT]** — RM writes its own (`:208`) | ⊘ | |
| `subDeviceId` | +136 | guest value | **V**/0 | single-subdevice |
| `hObjectEccError` | +140 | dead, same as `hObjectError` | **T** or 0 | |
| `instanceMem` | +144 | **guest GPGA — authoritative for a real GSP** | ⊘ | no CPU-RM reader; host derives its own (`:2730-2736`) |
| `userdMem` | +168 | guest GPGA | ⊘ | ditto (`:2749-2756`) |
| `ramfcMem` | +192 | guest GPGA | ⊘ | ditto (`:2738-2744`) |
| `mthdbufMem` | +216 | guest GPGA | ⊘ | ditto (`:2762-2782`) |
| `hPhysChannelGroup` | +240 | `NV01_NULL_OBJECT` (`:218-219`) | ⊘ | scrubbed in and out (`:1057`) |
| `internalFlags` | +244 | ★ **the adjudicated privilege**, `[1:0]`; also `UVM_OWNED[7]`, `ERROR_NOTIFIER_TYPE[3:2]` | ⊘ | host RM zeroes it on entry (`:220`) — forwarding is pointless, **reading it is the point** |
| `errorNotifierMem` | +248 | guest GPGA | ⊘ | host RM memsets it on entry (`:221-222`) |
| `eccErrorNotifierMem` | +272 | guest GPGA | ⊘ | ditto (`:223-224`) |
| `ProcessID` / `SubProcessID` | +296/+300 | ★ guest's PID — real, from `pRmClient->ProcID` (`:293-294`, `:2814-2815`) | ⊘ | host RM zeroes them (`:225-226`) |
| `encryptIv`/`decryptIv`/`hmacNonce` | +304/+316/+328 | only if `bCCSecureChannel` (`:2685-2701`) | ⊘ | not our config |
| `tpcConfigID` | +360 | guest value | ⊘ | DTD-PG, not our config |

Offsets are as pinned in `crates/kayfabe-abi/src/submit.rs:259-270` (`sizeof = 368`) and
`crates/kayfabe-abi/src/notifier.rs:182-192`; they are 580-specific (610 shifts by 8 from
`hVASpace` onward, `notifier.rs:188-196`). Struct order verified against
`alloc_channel.h:296-342`.

⊘ **One nit in our own map, worth fixing while someone is in there:**
`submit.rs:271` says *"Everything from +144 on is `// reserved` in the header"*. In
`alloc_channel.h` the `// reserved` comments start at **`hPhysChannelGroup` (+240)**;
`instanceMem`…`mthdbufMem` (+144…+239) carry no such marker (`alloc_channel.h:323-329`). The
sentence's *conclusion* — leave them zero on the forward — is right (§1), but its stated reason
is not what the header says.

**⇒ The clean answer the brief asked for:**
> Verbatim works for **`gpFifoOffset`, `gpFifoEntries`, `userdOffset[]`, `engineType`,
> `subDeviceId`**. Translation is needed and sufficient for **`hVASpace`, `hUserdMemory[]`,
> `hContextShare`, `hObjectError`**. It **breaks on `flags`**, because bits 21:8 are the guest
> kernel's demand for a specific hardware ChID (`kernel_channel_gm107.c:456-490`) and bit 5:5 is
> forgeable by guest userspace. Every `NV_MEMORY_DESC_PARAMS` is inert on the forward and must
> instead be **re-derived** — which the host RM does for us, for free, provided we hand it
> handle+offset.

---

## §3 Q2, answered field by field

### 3.1 `gpFifoOffset` — [IN] only, and **nothing in ogkm reads it**

`grep -rn gpFifoOffset` over the whole clone: 16 hits. Every one is either a **writer**
(`nv_gpu_ops.c:5949`, `kernel_graphics.c:2424`, `kgraphics_tu102.c:489`,
`mem_utils_gm107.c:1232`, `kernel_rc_watchdog.c:994`, `nvidia-push-init.c:399`), a **struct/doc
definition** (`alloc_channel.h:300`, `ctrl2080fifo.h:809,827`, `g_sdk-structures.h:225`), or an
**RPC copy** (`kernel_channel.c:2664`, `vgpu/rpc.c:3526`). **There is no reader.** The same holds
for `gpFifoEntries` (`kernel_channel.c:2665`, `vgpu/rpc.c:3527`).

Known-positive control for that grep shape: the identical pattern over
`GROUP_CHANNEL_RUNQUEUE` / `_FLAGS, _CHANNEL_TYPE` returns six real readers
(`kernel_channel.c:193,206,2469,2711`, `kernel_channel_gm107.c:191`) — so the pattern does find
readers when they exist.

Corroborating: **`grep -rn "NV_RAMFC" src/` returns 0 hits** across the whole clone, while
`NV_RAMUSERD_BASE_SHIFT` in the same header tree is found
(`src/common/inc/swref/published/maxwell/gm107/dev_ram.h:49`). ⇒ RAMFC — where `GP_BASE` is
actually programmed from `gpFifoOffset` — is **not in the open source at all**; it lives in GSP
firmware. `gpFifoOffset` is opaque pass-through from ogkm's point of view.

**Is it in/out?** At the transport level, yes: `rpcRmApiAlloc_GSP` copies the whole reply params
block back over the caller's buffer on success (`vgpu/rpc.c:11234-11242`,
`portMemCopy(pAllocParams, paramsSize, rpc_params->params, paramsSize)`).
**But on the channel path the destination is a scratch buffer.** `_kchannelSendChannelAllocRpc`
allocates `pRpcParams` at `:2656-2658` and `portMemFree`s it at `:2843`; nothing between reads it
back. The macro's `pchid` out-parameter is used **only** on the non-`IS_FW_CLIENT` branch
(`rpc.h:255-266`).
⇒ **`gpFifoOffset` is effectively [IN] on our path. Relocating it in a reply is not merely
unsupported — the guest would not read the relocation.** Nothing rounds it, re-maps it, or
re-derives it from a descriptor.

### 3.2 `userdOffset[]` and `hUserdMemory[]` — what RM does exactly

On the CPU-RM arm (`kernel_channel_gv100.c:69-133` → `:152-277`):

1. **`if (phUserdMemory[0] != 0)`** (`:70`). ⇒ **If the guest passed 0, RM chooses where USERD
   lives itself** — `bClientAllocatedUserD` stays `NV_FALSE` and `kchannelAllocMem_HAL`
   (`kernel_channel.c:2333-2338`) allocates it. **This is the answer to "does it also *choose*":
   yes, but only when the caller declined to name one.**
2. Handle resolution in the **caller's** namespace:
   `serverutilGetResourceRefWithType(hClient, hUserdMemory, classId(Memory), &ref)` (`:184-192`).
   ⇒ handle translation is necessary **and sufficient**; RM asks for nothing else.
3. **VPR refused**: `memdescGetFlag(..., MEMDESC_ALLOC_FLAGS_PROTECTED)` → `NV_ERR_INVALID_FLAGS`
   (`:196-201`).
4. **The offset is an offset into the object**, not a VA and not a physical address:
   `userdAddr = memdescGetPhysAddr(pUserdMemDescForSubDev, AT_GPU, userdOffset)` (`:204-206`).
   ⇒ namespace-free ⇒ **verbatim-safe**.
5. A **sub-memdesc** of exactly `userdSize` at `userdOffset` (`:228-237`), with the parent's page
   size temporarily forced to `RM_PAGE_SIZE` so BAR2 does not over-map (`:222-231`).
6. Lifetime: `refAddDependant` if the object is an `OsDescMemory` (`:243-253`).
7. **The zeroing** the brief asks about: it is *not* in this function. It is
   `kernel_channel.c:2340-2356` → `kfifoSetupUserD_HAL` →
   `kernel_fifo_gm107.c:796-808`, `memmgrMemSet(..., 0, NV_RAMUSERD_CHAN_SIZE, ...)` — **512 bytes
   at offset 0 of the sub-memdesc**. Gated on `IS_VIRTUAL(pGpu) || IS_GSP_CLIENT(pGpu)` **and** the
   USERD being in SYSMEM (or FBMEM under full SRIOV). ⇒ **On our forward the host RM is
   `IS_GSP_CLIENT`, so it will zero a handed-in USERD.** That is the already-banked
   `rm_takes_a_guest_userd_and_zeroes_it` fact, now with its line: adopt at creation, because the
   wipe happens *inside* the alloc.

So: **RM does not choose where USERD lives when the caller names it. It resolves, sub-descs,
validates, and zeroes 512 bytes of it.**

### 3.3 Alignment / size / aperture constraints a verbatim forward could violate

| constraint | where | what it means for us |
|---|---|---|
| USERD size = **512 B**, address shift = **9** | `kernel_fifo_gm107.c:1545-1556`; `dev_ram.h:49-50` (`NV_RAMUSERD_BASE_SHIFT 9`, `NV_RAMUSERD_CHAN_SIZE 512`) | the sub-memdesc is 512 B; `userdOffset + 512` must fit the object |
| **object** alignment ≥ 512 | `kernel_channel_gv100.c:255-261` — `pMemDesc->Alignment < userdAlignment && != 0` → `NV_ERR_INVALID_ADDRESS` | a property of the **host** object we allocate, not of the guest's number |
| USERD physical address must fit the runlist entry fields | `kernel_channel_gv100.c:208-220` → `kchannelIsUserdAddrSizeValid_GA100`, `kernel_channel_ga100.c:38-47` (`userdAddrLo` vs `SF_MASK(NV_RAMRL_ENTRY_CHAN_USERD_PTR_LO)`, `userdAddrHi` vs `..._HI_HW`) | host-side; a host allocation that passes today will keep passing |
| USERD not VPR | `kernel_channel_gv100.c:196-201` | host-side |
| `CHANNEL_TYPE` must be `_PHYSICAL` | `kernel_channel.c:192-194` | the guest already satisfies it |
| `hContextShare` and `hVASpace` mutually exclusive | `kernel_channel.c:305-313` → `NV_ERR_INVALID_ARGUMENT` | ⚠ if we translate both, we break a channel that names both |
| `mthdbufMem.size > 0 && base != 0` | `kernel_channel.c:2477-2479` — **GSP arm only** | affects *us as the receiver*, not the forward |

⊘ **What RM does NOT check:** `userdOffset` alignment. `memdescGetPhysAddr(..., userdOffset)`
takes it raw (`kernel_channel_gv100.c:204-206`) and `userdAddrLo = NvU64_LO32(userdAddr) >> 9`
(`:207`) silently truncates a misaligned address rather than rejecting it. ⇒ a misaligned
`userdOffset` forwarded verbatim produces **no error and a wrong hardware address** — the exact
silent-stall shape our own code already warns about at
`crates/kayfabe-isolate-host/src/rm.rs:5037-5054` (`b0d6de7`). **Assert 512-alignment ourselves.**

### 3.4 What RM derives itself — the do-not-fight list

Written by the **guest** kernel before it reaches us (so: already decided, ours to read):
`cid` (`:208`), `ChID` (encoded into `flags[21:8]`, `:2786-2802`), `internalFlags` — zeroed then
refilled with privilege + notifier types (`:220`, `:589-596`, `:2804-2812`), `ProcessID`/
`SubProcessID` (`:293-294`, `:2814-2815`), `hObjectBuffer := 0` (`:2663`),
`hPhysChannelGroup := NULL` (`:218-219`), `errorNotifierMem` + `eccErrorNotifierMem`
(`:551-587`), `instanceMem`/`ramfcMem`/`userdMem`/`mthdbufMem` (`:2730-2782`),
`flags[5:5]` when the guest client really was privileged (`:281`, `:286`).

Re-derived by the **host** RM when we forward (so: do not supply, do not fight):
the same list. Note `:1055-1067` — on the way *out* RM re-scrubs `hPhysChannelGroup`,
`internalFlags`, `errorNotifierMem`, `eccErrorNotifierMem`, `ProcessID`, `SubProcessID`, under
the comment *"These fields are only needed internally; clear them here"*. ⇒ a verbatim forward
of those six is **harmless** — the host RM destroys them before use and again after.

### 3.5 ★ Which representation does RM honour — the sharpest sub-question

**It depends on the path, and the dependence is an explicit `if`, not a preference.**

- **Receive side (we are the GSP): the DESCRIPTOR.** `_kchannelDescribeMemDescsFromParams`
  (`kernel_channel.c:2373-2520`) `memdescDescribe`s `instanceMem`, `ramfcMem`, `userdMem` and
  `mthdbufMem` and never touches `hUserdMemory`/`userdOffset`. It asserts it is on that arm at
  `:2388-2390`.
- **Forward side (host CPU-RM): the HANDLE + OFFSET.** `kchannelCreateUserdMemDescBc_HAL`
  (`kernel_channel.c:2304-2309`), and **zero readers** of any `NV_MEMORY_DESC_PARAMS` on that arm.

⇒ **Handle translation alone IS sufficient on the forward** — the brief's worry does not
materialise there. ⚠ **But it is *insufficient on the receive* in one case that has no handle at
all**: when guest userspace passed `hUserdMemory[0] == 0`, the guest kernel allocated USERD
itself, `hUserdMemory[]` arrives all-zero, and the only description of that memory on our wire is
`userdMem` — **a guest GPGA with no object handle behind it.** For such a channel there is
nothing to translate and the isolate must fall back to allocating its own USERD (which HEAD
already does: `rm.rs:4890-4900`, the `GuestRing { userd: None }` arm at `b0d6de7`).

⚠ **This is the one place the design could still need a different shape**, and it turns on a
number nobody has measured: *what fraction of guest userspace channels name their own USERD?*
See §5.

---

## §4 Q1 — the privileged discriminator

**Verdict: the owner's conclusion holds, the owner's field does not.**

- ✅ *"the guest kernel has already adjudicated it"* — true. `kernel_channel.c:276-296` is the
  adjudication, it runs in the guest's CPU-RM (`RMCFG_FEATURE_PLATFORM_GSP == 0` there), and it
  runs **before** `_kchannelSendChannelAllocRpc` (called at `:936-946`, well after `:296`).
- ✅ *"a guest userspace process that asked for privileged and wasn't entitled has been dropped"* —
  true **of the effect**: `privilegeLevel` is set to `_PRIVILEGE_USER` (`:290`) regardless of what
  the caller asked for, and `internalFlags` was already zeroed at `:220`.
- ⊘ *"⇒ the flag is trusted-by-construction at our boundary"* — **false of `flags[5:5]`.** RM never
  clears the bit; an unentitled request is not *dropped*, it is *not granted*, and the ungranted
  bit rides along into the RPC at `:2668`. See §0.2 for the exhaustive grep.

**Is there any path where a userspace-originated alloc carries `PRIVILEGED_CHANNEL_TRUE` to the
RPC? Yes — the trivial one: userspace sets the bit and RM leaves it.** No path grants
*privilege* that way (`privilegeLevel` is independent), but the *bit* arrives set.

⇒ **Use `internalFlags[1:0]`.** `USER=0 / ADMIN=1 / KERNEL=2`
(`generated/g_kernel_channel_nvoc.h:181-184`). Optionally cross-check `UVM_OWNED[7:7]`
(`:202-204`), which the guest sets only when `privilegeLevel == KERNEL` (`kernel_channel.c:298-303`)
— so `UVM_OWNED=1 && PRIVILEGE!=KERNEL` is an impossible combination and a free integrity check
on the pair.

⊘ **And "privileged" here is VM-scoped, exactly as the owner says.** `internalFlags[1:0] == KERNEL`
means *the guest's own kernel vouched for this channel*, nothing more. It is still untrusted to
us; it is the case we emulate with a scratchpad ring. The fork is *passthrough vs
allocate-and-emulate*, not a trust elevation. ⚠ It defends against guest **userspace**, never
against a compromised guest **kernel** — the same boundary
`docs/design/execution_plane_increments.md:9598-9603` already names for the membership rule.

⚠ Two neighbouring flags with the same shape, worth knowing before anyone reads them:
`NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE` (7:7) and `_DENY_AUTH_LEVEL_PRIV` (22:22) have
**zero readers** in the open tree (`grep -rn 'DENY_AUTH_LEVEL_PRIV\|DENY_PHYSICAL_MODE_CE'` over
`src/`, excluding `alloc_channel.h` and `generated/`, → empty; same pattern finds
`GROUP_CHANNEL_RUNQUEUE` readers, so the grep works). They are GSP-firmware-side. **We are the
GSP** — if we ever want to enforce them, we are the only party that can.

---

## §5 Q1b — client identity, and what task #207 actually keys on

**Settled: client identity reaches us in full.** See §0.3 for the wire and the decode.
⇒ **The "adjudicate against which client" suggestion is buildable**, and the flag is *not* the
only discriminator.

★ **And the tree already has a client-keyed rule** — but it is not #207. I could not find a task
#207 anywhere in either repo (no task/backlog file; `git log --all --grep=207` yields only lane
tags `w207`, an unrelated address-resolution rung; enumerating `task #N` across the tree gives
#2…#243 with **no #207** — the same enumeration does find `task #238`, so the pattern works).

The rule matching that description is **§16.44 / §12.27, "THE MEMBERSHIP RULE"**,
`docs/design/execution_plane_increments.md:9528`, stated at `:9578-9581`. It keys on
**`NV0000_ALLOC_PARAMETERS.processID == 0xFFFF_FFFF` on the client's own `NV01_ROOT` alloc** →
`ClientKind::Kernel` (`crates/kayfabe-abi/src/guest_os.rs:280-295`, call site
`crates/kayfabe-rmrpc/src/lib.rs:1310`), folded to `SYSTEM_ANCHOR` in
`crates/kayfabe-core/src/project.rs:1026`, `:1160-1167`. The channel's kind then falls out of
the anchor, not out of the channel's own alloc — `project.rs:311-317`:

```rust
pub fn channel_kind(&self) -> GuestChannelKind {
    if self.anchor == SYSTEM_ANCHOR { GuestChannelKind::Emulated }
    else { GuestChannelKind::Passthrough }
}
```

⇒ **It is weaker than what §0.2 offers, in one specific way:** it is a property of the
*namespace* (decided once, at client-root time, from a different RPC), not of the *channel*.
A kernel client that allocates a channel on behalf of userspace, or a userspace client whose
root we missed, is misclassified. `internalFlags[1:0]` is per-channel and arrives on the channel's
own RPC. **The two are independent and should agree; a disagreement is a real signal and is worth
a named refusal rather than a silent precedence rule.**

⚠ The sentinel is also OS-specific: `0xFFFF_FFFF` is written only under
`RMCFG_FEATURE_PLATFORM_UNIX` (`guest_os.rs:21-34`, quoting `ogkm-580: inc/kernel/vgpu/rpc.h:67-77`);
on Windows a kernel client declares a real pid. `internalFlags[1:0]` has no such dependence —
`kernel_channel.c:276-296` is platform-independent. ⇒ **`internalFlags` is the more portable
discriminator as well as the more precise one.**

---

## §6 What I could not determine, and what would determine it

1. **What fraction of guest userspace channels name their own USERD** (`hUserdMemory[0] != 0`).
   This decides whether §3.5's no-handle case is an edge or the norm, and therefore whether
   "translate the handle" is a complete story. ⇒ **Determined by:** a census over an existing
   `GSP_RM_ALLOC` capture — decode `hUserdMemory[0]` at +32 and `userdOffset[0]` at +64
   (`crates/kayfabe-abi/src/submit.rs:262-268`) for every `hClass == 0x906f`-family alloc, split
   by `internalFlags[1:0]`. No boot needed; the traces exist.
2. **Whether `internalFlags[1:0]` is actually populated on this bench's wire.** The code path says
   it must be (`kernel_channel.c:2786-2815`, the `IS_GSP_CLIENT` arm), but that is a reading, not a
   measurement — and this repo's own `dlen=0` lesson is that a plausible zero and an unmeasured
   zero look identical. ⇒ **Determined by:** the same census, one extra column. ⚠ A census that
   returns all-zero `PRIVILEGE` is *ambiguous* — `USER == 0` — so it needs a **known-positive**:
   the guest's own UVM/kernel channels must show `PRIVILEGE == 2`. If they do not, the field is
   not reaching us and §0.2's recommendation collapses back to `flags[5:5]`.
3. **Whether `flags[5:5]` is in practice ever set by guest userspace.** §0.2 proves it *can* be;
   it does not prove libcuda does. This matters only for how loudly to refuse, not for the design.
   ⇒ **Determined by:** the same census, comparing `flags[5]` against `internalFlags[1:0]` — any
   row with `flags[5]==1 && internalFlags[1:0]==0` is a forged bit in the wild.
4. **What the GSP firmware does with `gpFifoOffset`.** §3.1 proves ogkm never reads it and that
   `NV_RAMFC` is absent from the open source. The actual `GP_BASE` programming, and therefore any
   alignment constraint on it, is in the blob. ⇒ **Determined by:** the native dataplane trace
   (`nvidia-gpu-passthrough/traces/native_dataplane_ga106/`), or by a deliberate misalignment
   probe on hardware. Neither was run here.
5. **Whether the host CPU-RM's `hContextShare`/`hVASpace` exclusivity (`:305-313`) is reachable
   for a forwarded guest channel.** The guest satisfies it by construction, but our forward
   re-encodes both fields; today the isolate sets both to `0` (`rm.rs:5031-5033`, `b0d6de7`) and
   inherits from the group, which sidesteps it. If the design changes to forward them, this
   becomes live.
6. **The isolate's own re-keying on the guest's `hClient`.** Not audited — out of scope of Q1/Q2
   and it needs its own pass over `kayfabe-isolate` / `kayfabe-isolate-host`.

---

## Provenance

- ogkm: `/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04/`, **580.159.04**.
  Version-specific claims: the `internalFlags` **offset** (+244 on 580, +252 on 610) and the
  `hVASpace`-onward field offsets. The *semantics* — the CPU-RM/GSP `if`, the privilege
  adjudication, the ChID-in-flags encoding — are structural and were not observed to differ.
- kayfabe: `origin/master` = `e758778`; lane HEAD = `b0d6de7`
  (`w284-ce-passthrough-for-real`, **unpushed** at the time of writing). Every `rm.rs` citation
  states which. ⚠ `rm.rs` line numbers differ by ~700 between them.
- Read-only throughout: no build, no boot, no working tree touched (this branch was authored in
  a detached `git worktree`).
