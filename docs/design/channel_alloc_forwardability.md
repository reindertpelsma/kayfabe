# Channel alloc forwardability — what RM honours, what it derives, and what a verbatim forward may not touch

**STATUS: LIVE (2026-08-13).** §0–§5 are the read-only source study. **§7 (2026-08-13, w286) is
the CENSUS that closes §6 items 1–3 against committed captures** — read §7's lead before acting on
§3.5 or §6, because it retires the "no handle to translate" case as unobserved and **inverts §6's
item-2 premise**. Read-only source study, no boot, no build. Ground truth is
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

> ### ★★★ MEASURED 2026-08-13 (w286) — items 1, 2 and 3 are ANSWERED. Read §7 first.
> **1 → ANSWERED, and the answer is "universal":** `hUserdMemory[0] != 0` on **68 of 68** channel
> allocs across all five committed C captures, and `userd=h0x0/off…` appears **0** times in **144**
> USERD projections in our own boot logs. §3.5's no-handle case **has never been observed**.
> **2 → ANSWERED, and it is a KNOWN-POSITIVE, not a plausible zero:** `internalFlags[1:0]` is
> populated and *discriminating* — KERNEL **36**, USER **32**, and the whole dword takes **four**
> distinct values whose every sub-field varies independently and legally. Ruling 1 is **buildable
> as stated**.
> **3 → ANSWERED, negative:** `flags[5:5] == 1` occurs on **exactly** the 36 KERNEL channels and
> **zero** USER channels. No forged bit in this corpus — which bounds libcuda's behaviour and
> **does not** weaken §0.2, because §0.2 is about what a *hostile* guest userspace *can* do.
> ⊘ **Item 2's own premise ("some earlier RM alloc we served") is REFUTED** — see §7.3. Items 4, 5
> and 6 stand unmeasured.

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

## §7 THE CENSUS — 68 channel allocs off our own wire (w286, 2026-08-13)

**No boot, no build.** Everything below was decoded out of captures already committed to
`nvidia-gpu-passthrough/traces/`. Decoder + raw rows:
`scripts/mode2_diag/rpc_channel_census.py` (this commit), run against the five
`traces/mode2_c_reference/*.rec.zst` captures.

### ★★★ 7.0 LEAD — four things that contradict the brief that commissioned this rung

**⊘ 7.0.1 — Q1's gap does not exist in any committed capture. `hUserdMemory[0] == 0` occurs 0
times in 68.** It is not rare, it is **unobserved**. Every channel — RM's own CeUtils scrubbers,
UVM's copy channels, and all 32 of libcuda's — names its own USERD object. §3.5's
"nothing to translate" case is a **source-reachable path with no measured instance**.

**⊘⊘ 7.0.2 — Q2's premise is REFUTED. "If the guest kernel allocated that USERD, it allocated it
through us" is FALSE on the GSP wire.** Memory objects **never reach the GSP as an alloc**. The
handle is unresolvable *for every channel, including the 68 that do supply one* — see §7.3 for the
control that makes this a measurement rather than a failed lookup. ⇒ Q1 is neither "a blocker" nor
"a lookup"; **the handle is not a recoverable key at all, and it does not need to be**, because the
descriptor beside it is complete.

**⊘ 7.0.3 — the nvdiff corpora carry NOTHING for this question, and they fail in the shape the
brief warned about.** `1317` `NV_ESC_RM_ALLOC` ioctls across `traces/host_reference_ga106/`,
`traces/guest_mode2_vh2/`, `traces/nvdiff_w274/`, `traces/nvdiff_w275/` — **0 of them carry a
single parameter byte** (`psize == 0`, `pgot == 0`, `ppre == ""` on every one). Cause, opened:
libcuda passes a valid `pAllocParms` but leaves `NVOS64_PARAMETERS.paramsSize == 0`, because RM
derives the size from `hClass`; the shim's rule *"read `paramsSize` bytes at `pAllocParms`"*
(`tests/mode2/nvdiff/nvdiff_shim.c:168-178`) therefore reads zero.
⚠ **This is the brief's own trap, live:** an empty `ppre` decodes to `hUserdMemory[0] = 0`, i.e. it
would have reported Q1's gap as **universal** — the exact inverse of the truth. Verbatim row,
`traces/host_reference_ga106/ctx_r1.jsonl.zst` record `i=161`, `hClass=0xc56f`:
`pAllocParms@16 = 0x7ffcad257e80`, `paramsSize@32 = 0`, `pptr=0x7ffcad257e80`, `psize=0`.
⊘ **And the same emptiness, from a second and better-shaped instrument:** the three
**real-hardware** GSP RPC ring dumps carry 48 channel-alloc records that each declare
`paramsSize = 368` and each stop at `cap_len = 112` — **zero param bytes, on real silicon, on
three chips** (§7.1.1). Two independent oracles, both silent, both silent in a way that decodes to
"the field is zero".

**⊘ 7.0.4 — half the channel-alloc records in the C traces are OUR OWN REPLY, and they are
all-zero.** Every `GSP_RM_ALLOC` appears twice: once as a `GuestRead` (kind 3 — the guest's request,
which we read) and once as a `GuestWrite` (kind 4 — our scrubbed reply, which we write). The 68
replies carry `internalFlags == 0` and `hUserdMemory[0] == 0` **in all 68 cases**. Censusing the
raw stream without splitting on direction yields exactly **50 % `PRIVILEGE=USER`** and **50 %
`hUserdMemory[0]==0`** — two plausible, wrong, mutually reinforcing numbers, produced entirely by
counting our own silence. ⚠ *A count cannot see a substitution*; here the substitution is
**direction**.

### 7.1 Corpora — what was opened, and what carried nothing

| corpus | what it is | verdict |
|---|---|---|
| `traces/mode2_c_reference/cap1_coldboot_hermetic.rec.zst` | hermetic cold boot, no CUDA | **4** channel allocs — ★ the Q3 known-positive |
| `…/cap1b_coldboot_hermetic_d6.rec.zst` | same, GSP-D6 continuation | **4** |
| `…/cap2_stalequeue_negative.rec.zst` | stale-queue chain | **28** |
| `…/cap2b_stalequeue_nofn47.rec.zst` | the guest-reachable defect fixture | **4** |
| `…/cap3_matmul_forwarding.rec.zst` | `cuCtxCreate` → `cup8` matmul, `bad=0` | **28** |
| **total** | | **68 requests + 68 replies** |
| `traces/host_reference_ga106/`, `guest_mode2_vh2/`, `nvdiff_w274/`, `nvdiff_w275/` | nvdiff ioctl differential | ⊘ **NOTHING** — 1317 `RM_ALLOC`, 0 params (§7.0.3) |
| `nvkvm-rs traces/boots/**` + `traces/guest_boots/**` (443 logs) | our own qemu/serial/dmesg logs | **partial** — 144 `userd=h…/off…` projections, **0** with a zero handle; ⊘ carry **no** `internalFlags` (`grep -c 'internal[_ ]flags' → 0` over all 443) |
| `traces/{rpctrace_ga106,ga102,ad102}_boot1.bin` | ★ **real-hardware GSP RPC ring dumps**, 580.159.04, three chips | ⊘ **NOTHING, and it misses by one byte** — see §7.1.1 |
| `nvidia-gpu-passthrough traces/real_ga106/` | — | empty directory |

#### ⊘⊘ 7.1.1 — the corpus that SHOULD have answered this, and stops exactly at the payload

`traces/rpctrace_ga106_boot1.bin`, `ga102_boot1.bin`, `ad102_boot1.bin` are the best-shaped oracle
we own for this question: **real, unvirtualised hardware**, driver `580.159.04`, the guest kernel
module recording its own GSP msgq elements — no emulator between the driver and the tape, and on
**three different chips**. Each carries **16 channel-alloc records** (8 request/reply pairs;
`0xC56F` ×12, `0xC36F` ×4 per trace).

**Every one of them declares `paramsSize = 368` and carries `cap_len = 112`.**
`112 = 48` (`GSP_MSG_QUEUE_ELEMENT` header) `+ 32` (`rpc_message_header_v`) `+ 32`
(`rpc_gsp_rm_alloc_v03_00`). ⇒ **param bytes available: 0, on all 48 records across all three
traces.** The recorder's cap lands on the exact byte where `NV_CHANNEL_ALLOC_PARAMS` begins.

⚠ **This is the `dlen=0` lesson in a new instrument.** Nothing about these files looks short: they
are ~1.2 MB each, `n_dropped=0`, `n_rx_failed=0`, `wrapped=false`, and `decode_rpctrace.py` — which
is explicitly *"a refuser first"* and has **no `--force`** — accepts all three without complaint,
because a capped payload is not a *hole* in its sense and every declared invariant holds. The
truncation is visible only by comparing `cap_len` against the alloc's own `paramsSize`, which
nothing does.
⇒ ★ **Cheap, high-value follow-up:** raise the recorder's payload cap to ≥ `112 + 368 = 480` and
re-capture one boot. That would make the *real-hardware* answer to Q1/Q3/Q4 directly measurable on
three chips, instead of inferring it from an emulated wire. ⊘ And `decode_rpctrace.py` should
**refuse, or at minimum flag, `cap_len < rpc_len`-implied-payload** — an instrument whose whole
stated purpose is making `dlen=0` impossible to read past currently reproduces it.

★ The `.rec` captures are the **only** corpus that carries `NV_CHANNEL_ALLOC_PARAMS`. That is not
incidental: they record the GSP RPC, and the GSP RPC is the *only* transport on which the guest
kernel's post-adjudication view of a channel exists at all.

**Decode provenance.** RPC framing from `ogkm-580: g_rpc-message-header.h:41-52` (32-byte header:
`header_version, signature, length, function, rpc_result, rpc_result_private, sequence, u`),
signature `0x43505256` (`inc/kernel/vgpu/rpc_headers.h:61`), `GSP_RM_ALLOC = 103`
(`inc/kernel/vgpu/rpc_global_enums.h:113`), body `rpc_gsp_rm_alloc_v03_00`
(`g_rpc-structures.h:1491-1502`). Field offsets are kayfabe's own
(`crates/kayfabe-abi/src/submit.rs:259-270`, `notifier.rs:182-184`), i.e. **the census reads the
wire with the same map production does**.
⚠ **Instrument known-positive, run first:** the scan finds **356 / 336 / 1122** RPC signatures in
cap1 / cap2b / cap3 with a sane function histogram (`GSP_RM_CONTROL 76`, `FREE 10`,
`GSP_RM_ALLOC 103`, `DUP_OBJECT 21`, `UNLOADING_GUEST_DRIVER 47`, …) and **441** distinct
`(hClient, hObject)` allocations. A zero here would have been the *"decisive grep over zero files"*
failure; it is not zero.

### 7.2 Q1 + Q3 + Q4 — the table

All 68 rows, `class ∈ {AMPERE_CHANNEL_GPFIFO_A 0xC56F ×64, VOLTA_CHANNEL_GPFIFO_A 0xC36F ×4}`.

| measure | result |
|---|---|
| **`hUserdMemory[0] == 0`** | **0 / 68 (0.0 %)** — and 0/32 among the USER rows, 0/36 among KERNEL |
| **`internalFlags[1:0]`** | **KERNEL 36 · USER 32 · ADMIN 0** |
| `internalFlags` distinct values | `0x1a` ×15, `0x16` ×5, `0x9a` ×16, `0x1c` ×32 |
| `flags[5:5]` (`PRIVILEGED_CHANNEL`) | `1` ×36, `0` ×32 |
| `flags[5:5]==1 && PRIVILEGE==USER` (forged bit) | **0** |
| `flags[5:5]==0 && PRIVILEGE==KERNEL` | **0** |
| `internalFlags[7]` `UVM_OWNED` | `1` ×16 — **all 16 are `PRIVILEGE==KERNEL`**; violations of the `UVM_OWNED ⇒ KERNEL` invariant (`kernel_channel.c:298-303`): **0** |
| `internalFlags[6]` `GSP_OWNED` | `0` ×68 |
| `flags[21:8]` (the guest's ChID demand, §0.4) | **non-zero on 68 / 68** — the strip is always live, never a corner case |
| `userdMem.addressSpace` | `NV_ADDR_FBMEM = 2` ×68 (`ogkm-580: inc/kernel/vgpu/rm_plugin_shared_code.h:67`) |
| `userdMem.size` | `512` ×68 — matches `NV_RAMUSERD_CHAN_SIZE` (§3.3) on every row |
| `userdOffset[0] % 512` | `0` ×68 — §3.3's unchecked-alignment hazard is not being exercised today |
| distinct USERD backing objects (`userdMem.base − userdOffset[0]`) | **13** across the five captures |

**⇒ Q3 is answered by a known-positive, not by an absence.** `cap1` is a **hermetic cold boot with
no CUDA process at all** — its only four channels are RM's own, and all four read
`PRIVILEGE = KERNEL (2)`. `USER == 0` is therefore *not* what an unpopulated field would look like
here: an unpopulated field would have made cap1 read USER. And the field is not merely non-zero, it
is **discriminating**: within one capture (cap3) it separates 12 kernel channels from 16 libcuda
ones, and the neighbouring bits `[3:2]`/`[7]` vary independently and legally across the same rows.
⇒ **Ruling 1 is buildable exactly as stated, at zero new wire cost** — `notifier.rs:248` already
reads this dword and discards `[1:0]`.

**⇒ Q4 is answered: NO. `ADMIN` appears 0 times in 68.** The owner's cut never routes an emulated
case to passthrough *in this corpus*. Cross-tabulated against what we independently believe:

| `internalFlags[1:0]` | membership rule (`NV0000.processID == 0xFFFF_FFFF`, §5) | `NV2080_ENGINE_TYPE` | `UVM_OWNED` | n |
|---|---|---|---|---|
| KERNEL | KERNEL-CLIENT | `1` = `_GRAPHICS` | 0 | 10 |
| KERNEL | KERNEL-CLIENT | `11` = `_COPY2` | 0 | 10 |
| KERNEL | KERNEL-CLIENT | `9` = `_COPY0` | **1** | 4 |
| KERNEL | KERNEL-CLIENT | `10` = `_COPY1` | **1** | 4 |
| KERNEL | KERNEL-CLIENT | `11` = `_COPY2` | **1** | 4 |
| KERNEL | KERNEL-CLIENT | `12` = `_COPY3` | **1** | 4 |
| USER | **user pid** (1225 / 1302 / 1342 / 1358 …) | `0` = `_NULL`, inherited from the TSG | 0 | 32 |

Mnemonics from `ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:282,291`
(`_GRAPHICS = 1`, `_COPY0 = 9`); `subDeviceId == 0` and `hUserdMemory[1..7]`/`userdOffset[1..7]`
are zero on **68/68** (single-subdevice bench).

★★★ **The two discriminators agree on 68 / 68 rows.** Every `KERNEL` channel belongs to a client
whose `NV01_ROOT` declared `processID == 0xFFFF_FFFF`; every `USER` channel belongs to a client
that declared a real pid. §5's *"a disagreement is a real signal worth a named refusal"* is
therefore a refusal with a **measured zero rate** — which is exactly the condition under which it
is cheap to add and worth adding.
⊘ **But the agreement is not redundancy.** The membership rule is a property of the *namespace*,
decided once at client-root time; `internalFlags[1:0]` is a property of the *channel* and arrives
on the channel's own RPC. The corpus contains no kernel client that allocates on a user proc's
behalf (§16.44's case), so it cannot distinguish the two rules — it can only show they have not yet
diverged.

### ★★★ 7.3 Q2 — the handle is unrecoverable, and that is the GOOD news

The brief's hypothesis: *"if the guest kernel allocated that USERD, it allocated it through us."*
**Measured false.** Built the object table from every `GSP_RM_ALLOC` **request** in all five
captures — **441 `(hClient, hObject)` pairs** — and resolved each channel's handle fields against
it, in the alloc's own client namespace:

| field | zero | **resolvable** | **unresolvable** |
|---|---|---|---|
| `hParent` | 0 | **68** | 0 |
| `hVASpace` | 58 | **10** | 0 |
| `hContextShare` | 52 | **16** | 0 |
| `hObjectError` | 5 | 0 | **63** |
| `hUserdMemory[0]` | 0 | 0 | **68** |

★ **That is the control the answer needs.** The table resolves `hParent` 68/68, `hVASpace` 10/10
and `hContextShare` 16/16 — so it is not broken, and "unresolvable" is a fact about the wire, not
about the lookup. The two fields that miss are exactly the two §0.1/§1 already identified as
**dead on the receive side**, and they miss for one reason: **memory objects are allocated by the
guest's CPU-RM and never sent to the GSP.** `NV01_MEMORY_*` classes appear in the
fn-103 stream only as `0x70` ×1 / `0x79` ×7 / `0x7E` ×3 (11 allocs in cap3); the USERD-bearing objects
(`0x9`, `0xbaba0049`, `0x31415910`, `0xcaf000xx`, `0x5c000014`) appear **never**, under any client,
including via `DUP_OBJECT` (fn 21, 25 requests in cap3).

⇒ **This is not a hole; it is why the header says what it says.** `alloc_channel.h:307-310`:
> *"handle to UserD memory object for channel, **ignored if `hUserdMemory[0]=0`**"*
…and `kernel_channel.c:2294-2298` explains the asymmetry: the GSP *cannot* resolve a client handle,
which is precisely why RM translates for it and sends the **descriptor**. A GSP that could look the
handle up would make §0.1's `if` pointless.

⇒ **What we CAN recover is better than a handle, and it is already on the wire.** All 68
`userdMem` descriptors are complete, and all 68 name **`NV_ADDR_FBMEM`** — the guest's framebuffer,
which *we* back. The address is directly usable: for the 16 libcuda channels sharing object
`0x5c000014`, `userdMem.base` is exactly `object_base + userdOffset[0]` at a 12 KiB stride
(`0x4202000, 0x4205000, … 0x422f000` against `0x2000, 0x5000, … 0x2f000`), and our own boot logs
already read those pages (`fbRING[p0]@… resY byBAR1#167`, `traces/boots/w260/run_w260_off_qemu.log`).

⇒ **Verdict for the design: Q1 is neither a blocker nor a lookup — it is a case with no measured
instance, and its fallback is already built** (`rm.rs:4890-4900`, the `GuestRing { userd: None }`
arm at `b0d6de7`). The forward still needs a **host** USERD object regardless, because a guest GPGA
is not a host RM handle; the census removes the worry that we might have *nothing to work from*, not
the work of minting the host side.

### 7.4 What this rung could NOT determine

1. **Whether a *hostile* guest sets `flags[5:5]`.** §7.2 measures **stock** libcuda/UVM/RM on one
   workload family (`cup2`/`cup8`). A forged bit is a thing a modified guest does on purpose; a
   corpus of well-behaved guests cannot bound it. ⇒ §0.2's ruling stands on the *source*, and the
   zero here must never be cited as "the bit is safe".
2. **Whether `ADMIN` is reachable at all on Linux.** 0/68 is consistent both with "RM never emits
   it here" and with "our workloads never provoke it". `kernel_channel.c:284-287` grants it on
   `rmclientIsAdmin() || hypervisorCheckForObjectAccess()` — a **root-owned** CUDA process is the
   obvious untested arm. ⇒ **Determined by:** re-running `cup2` as root under an existing capture
   and re-running this census; ~one boot, no code change. Until then the owner's `ADMIN → passthrough`
   arm is **unexercised**, not wrong.
3. **Whether a kernel client ever allocates a channel on a user proc's behalf** — the case that
   would separate `internalFlags[1:0]` from §5's membership rule. Not in this corpus (68/68 agree).
4. **Anything about multi-subdevice.** `hUserdMemory[1..7]`/`userdOffset[1..7]` were decoded but
   are uniformly zero here (single-subdevice bench). A multi-GPU guest is unmeasured.
5. **The 610 offsets.** Every row above is 580; `notifier.rs:190-192` shifts `internalFlags` to
   +252 for 610 and nothing in this corpus exercises it.
6. ★ **Everything above is measured on an EMULATED wire.** The 68 rows come from the C artifact's
   fake GSP; the *guest driver* is stock and the *values* are the guest kernel's own, so the census
   is sound — but it has never been confirmed against a real GSP's msgq. ⇒ **Determined by:**
   §7.1.1's one-line recorder change (payload cap 112 → ≥480) plus one boot on `vh`. That is the
   single highest-value follow-up this rung found and it needs no design decision.

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
