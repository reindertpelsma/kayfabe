# The GSP → core bridge — `RpcCommand` becomes `RmEvent`

> **Status:** design only. Nothing described here is built. **Nothing in the Rust stack has ever
> run against a real GPU**, and this document does not change that.
>
> **Citations** follow `testing_doctrine.md` §6.1: our own tree by **symbol**, pinned trees
> (`ogkm` = `research_clones/ogkm` @ 610.43.02, `ogkm-580` = `research_clones/ogkm-580.159.04`,
> `C:` = `/workspace/nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c`) by `file:line`.
> Claims are tagged `[src]` / `[measured]` / `[inferred]` / `[unverified]`.

## 0. Why this file exists

`kayfabe-gsp` is built (S0–S5) and `kayfabe-core` is built (`RmGraph`, `Gpu`, the projections).
They do not touch. `kayfabe_gsp::rpc::RpcCommand` has **zero references outside its defining
crate**, and the only manifest naming `kayfabe-gsp` is `tests/Cargo.toml`. So `Gpu::apply`'s
input is synthesised by `tests/src/lib.rs::Scenario` and by nothing else.

This file specifies the piece between them. It does not build it.

`mode2_gsp_port_plan.md` §2 item 3 already claims the seam exists — *"RPC decode/encode →
abstract `RmEvent` … and control intents"* is listed as **owned by `kayfabe-gsp`** — and §5's S4
row repeats it. **That is not what was built**, and it is not what should be built: `kayfabe-gsp`
has no dependency on `kayfabe-core` and must not acquire one (§1.2). The plan's own S4 row is
otherwise accurate about what landed (dispatch, dispositions, replies, the async set), so treat
"→ `RmEvent`" as the one unimplemented clause, now relocated by this file. `kayfabe-gsp`'s
`rpc.rs` module doc already anticipated the relocation — *"the `RmEvent` bridge stays where the
plan puts it — `kayfabe-fwd`"* — and §1.2 argues that placement is also wrong, with reasons.

---

## 1. The seam, stated exactly

### 1.1 What crosses, in which direction, owned by whom

One direction only: **guest → us**. There is no reverse flow of core types into the GSP crate.

| # | value | owner | shape |
|---|---|---|---|
| in | `kayfabe_gsp::rpc::RpcCommand` | `kayfabe-gsp` | `{ function: RpcFunction, code: u32, sequence: u32, payload: Vec<u8>, elements: u32 }` |
| in | `kayfabe_abi::versions::DriverAbiTable` | `kayfabe-abi` | the Axis-A wire table, selected once at realize |
| out | `kayfabe_core::rmgraph::RmEvent` | `kayfabe-core` | six variants: `Alloc`, `Dup`, `SetPageDir`, `MapMemoryDma`, `Unmap`, `Free` |
| out | `kayfabe_gsp::boot::Reply` | `kayfabe-gsp` | `{ rpc_result: u32, body: Vec<u8> }` — clamped by `RpcCommand::reply` |
| out | a refusal (§4) | this crate | typed; becomes a non-zero `rpc_result` **and** a trace event |

`RpcCommand::payload` is **the RPC body after the 32-byte envelope** — `element.rs::decode_message`
slices `body[RpcEnvelope::SIZE..]`. So for a `GSP_RM_ALLOC` the payload's byte 0 is
`rpc_gsp_rm_alloc_v03_00.hClient`. `[src]` `ogkm: src/nvidia/generated/g_rpc-structures.h:1408-1419`.

### 1.2 Which crate the bridge lives in — a **new** crate, `kayfabe-rmrpc`

**Not `kayfabe-core`.** The core is *"a pure state machine over guest-supplied bytes: no QEMU
types, no syscalls, no OS knowledge, no real-time reads, **no NVIDIA struct layouts**"*
(`crates/kayfabe-core/src/lib.rs` crate doc). Depending on `kayfabe-gsp` would put the msgq
transport under the object model. Depending on `kayfabe-abi` would breach decision #2's
quarantine from the wrong side. Both are refused.

**Not `kayfabe-gsp`.** Its `Cargo.toml` has no `kayfabe-core` dependency and must keep none. The
reason is not tidiness: the crate's own governing property is *"every transition fires on what
the guest DID … There is no `if version == …` in this crate"* (`crates/kayfabe-gsp/src/lib.rs`
crate doc, ★ paragraph). A GSP FSM that can see the RM graph can start firing on graph state —
which is protocol-not-trace violated one level down, and it is exactly the shape the C fell into
(`C:2835-2837`'s `m2_poll_kick`, where a control command's *side effect* on the object model
became a transport-level action). `gsp_boot.rs` drives the whole FSM today with
`kayfabe_gsp::boot::EchoOk` and no `Gpu` in the process; that must stay possible.

**Not `kayfabe-abi`.** Stated by that crate: *"**The wire → `RmEvent` mapping.** This crate does
not depend on `kayfabe-core`"* (`crates/kayfabe-abi/src/lib.rs` §4). It supplies decoders and
views; it does not know what they mean.

**Not `kayfabe-fwd`, despite the plan.** `crates/kayfabe-fwd/Cargo.toml` depends on neither
`kayfabe-abi` nor `kayfabe-gsp` today. Adding both would (a) make `fwd` the top of the crate
lattice, (b) mix two unrelated jobs — *"recover intent → **unprivileged host ops**"* (its crate
doc) versus "decode wire bytes → declared protocol facts", and (c) drag `kayfabe-isolate`,
`kayfabe-vmm` and `kayfabe-completion` into the dependency closure of a translation that needs
none of them. Concretely: **stage B1 below has no `Worker`, no `RmBackend`, no `Vmm` and no host
object of any kind.** A bridge that cannot be built without them is the wrong shape.

**Verdict — a new crate.**

```
kayfabe-rmrpc
  ├── kayfabe-util     (assert_send_sync!)
  ├── kayfabe-arch     (ids: HClient/HObject/ClassId/Pdb/GpuVa; ClientKind)
  ├── kayfabe-abi      (DriverAbiTable + the views; decision #2 quarantine)
  ├── kayfabe-gsp      (RpcCommand, RpcFunction, CommandPolicy, Reply)
  ├── kayfabe-core     (RmEvent, Gpu, GpuError)
  └── kayfabe-trace    (the refusal's trace vocabulary)
```

It is the **only** crate in the tree permitted to name both `RpcCommand` and `RmEvent`. That is a
one-line CI grep and should be added as one (the boundary gate already has this shape).

`kayfabe-fwd` remains where the *control-command* long tail lives (Case-1 forward / Case-2
ack-only, `nvidia-gpu-passthrough/docs/design/mode2_forwarding_model.md:37-50`). The split is
sharp and worth stating: **this bridge produces `RmEvent`s only.** A control that must be
*forwarded to the host* is not this crate's business — it returns `Translation::Forward{cmd}` and
the caller hands it to `kayfabe_fwd::classify_control`.

### 1.3 What does **not** cross, and why that is load-bearing

- **Guest RAM.** `boot.rs::CommandPolicy::respond(&mut self, cmd: &RpcCommand) -> Option<Reply>`
  takes **no `GuestRam` parameter**, and `gl11_region_arguments.md` §2.2a makes that the reason
  the GSP command queue is not a `LockPath` region: *"It therefore cannot re-read guest memory
  even if a future command wanted to."* The bridge must be implemented **without widening that
  signature**. Any design that needs to chase a guest pointer out of an RPC payload has broken a
  written safety argument and must say so out loud rather than add the parameter.
  ★ This is genuinely easier here than on the Mode-1 path: `rpc_gsp_rm_alloc_v03_00.params[]`
  and `rpc_gsp_rm_control_v03_00.params[]` are **inline flexible arrays**, not pointers
  (`ogkm: g_rpc-structures.h:1416, 1433`). The guest already copied the params into the queue.
- **Host state.** No `Worker`, no `RmBackend`, no `Isolate`. `Gpu::apply` is core-side only.
  (Caveat, named: `Spine::refresh` mints `Proc`s through `IsolateFactory`, so applying an event
  *can* spawn an isolate. That is pre-existing core behaviour and the bridge inherits it; it is
  why B2 runs `apply` under the device write lock and why no host verb may be issued from inside
  `respond` — R1, no blocking under lock.)
- **Identity.** The bridge mints no `ClientKey`, no `ResourceKey`, no `ProcId`. §3.

### 1.4 What `kayfabe-abi` still owes

The existing `view::AllocReq` / `view::ControlReq` are the **ioctl** shapes: they carry
`params_ptr: u64` — *"a guest pointer. **Never dereferenced here**"* — decoded from
`NVOS21`/`NVOS64`/`NVOS54`. **The GSP RPC shapes are different structs with inline params.**
Reusing `decode_alloc`/`decode_control` on an RPC body would read `hClass` out of `status` and a
pointer out of `paramsSize`. Two new views and two new decoders are owed:

```rust
// kayfabe-abi::view
pub struct RpcAllocReq   { client, parent, handle, class, params_size, flags, params_at: usize }
pub struct RpcControlReq { client, object, cmd, params_size, rmapi_rpc_flags, params_at: usize }
```

`[src]` layouts, both from `ogkm: g_rpc-structures.h`:

| `rpc_gsp_rm_alloc_v03_00` `:1408-1419` | off | `rpc_gsp_rm_control_v03_00` `:1423-1435` | off |
|---|---|---|---|
| `hClient` | 0 | `hClient` | 0 |
| `hParent` | 4 | `hObject` | 4 |
| `hObject` | 8 | `cmd` | 8 |
| `hClass` | 12 | `status` | 12 |
| `status` | 16 | `paramsSize` | 16 |
| `paramsSize` | 20 | `rmapiRpcFlags` | 20 |
| `flags` | 24 | `rmctrlFlags` | 24 |
| `reserved[4]` | 28 | `rmctrlAccessRight` | 28 |
| `params[]` | **32** | `reserved0` (`NV_ALIGN_BYTES(8)`) | 32 |
| | | `params[]` | **40** |

★ An independent second oracle exists and agrees: the C artifact transcribed the same offsets by
hand from a live trace — *"fn=103 (GSP_RM_ALLOC) body: hClient@80, hParent@84, hObject@88,
hClass@92, paramsSize@100, params@112"* and *"hClient@80, hObject@84, cmd@88, status@92,
paramsSize@96, …, params@120"* (`C:2132-2135`, repeated `C:6464-6465`, `C:2732-2733`). Its
offsets are element-relative with a 48-byte element header and a 32-byte envelope, so subtract
80: alloc `0/4/8/12/20/32`, control `0/4/8/12/16/40`. **Both tables agree exactly.** Use that
subtraction as a unit test (§5).

`FreeReq` and `DupReq` need **nothing new**: `rpc_free_v03_00` *is* `NVOS00_PARAMETERS_v03_00`
and `rpc_dup_object_v03_00` *is* `NVOS55_PARAMETERS_v03_00` (`ogkm: g_rpc-structures.h:162-167,
200-205`), whose field order and widths are byte-identical to the ioctl structs the generator
already emits (`ogkm: g_sdk-structures.h:261-267, 368-377` vs
`crates/kayfabe-abi/src/generated/nvos.rs::Nvos00Parameters` (16 B) and `::Nvos55Parameters`
(28 B)). `DriverAbiTable::decode_free` and `::decode_dup` apply **verbatim** to an RPC payload.
`[src]`, and worth a pinning test rather than a comment.

### 1.5 ★ The fourth axis: what a Windows guest changes

The question the brief asks directly. The answer is **not** "the bridge is rewritten", and the
reason is structural, the same one `kayfabe-gsp`'s crate doc gives: every struct above lives in
`ogkm: src/nvidia/`, NVIDIA's **OS-independent RM core**. The per-OS layer (ioctl vs. escape)
sits *above* it and never reaches the GSP wire. `rpcRmApiAlloc_GSP`, `rpcRmApiControl_GSP`,
`rpcRmApiDupObject_GSP`, `rpcRmApiFree_GSP` (`ogkm: src/nvidia/src/kernel/vgpu/rpc.c:10945,
10659, 11067, 11120`) are compiled for every OS.

**Protocol-universal — unchanged for a Windows guest:** every offset in §1.4, the function ids,
the `(function, sequence)` reply match, `NV01_ROOT`'s "the hClient IS its root object's handle"
rule, `DUP_OBJECT` as the only cross-client transfer edge, continuation records.

**★ Exactly ONE thing in this design is genuinely Linux-shaped, and it is the most dangerous
field on the wire.** `[src]` The `processID` a client root declares:

```c
if (RMCFG_FEATURE_PLATFORM_UNIX &&
   (pCallContext->secInfo.privLevel >= RS_PRIV_LEVEL_KERNEL))
{   root_alloc_params.processID = KERNEL_PID;   }
else
{   root_alloc_params.processID = pClient->ProcID;   }
```
`ogkm: src/nvidia/inc/kernel/vgpu/rpc.h:67-77`, inside
`NV_RM_RPC_ALLOC_SHARE_DEVICE_FWCLIENT`, which allocates `NV01_ROOT` through
`GPU_GET_PHYSICAL_RMAPI` (`:83-88`) — i.e. **over the GSP wire, as `GSP_RM_ALLOC`**.

The `KERNEL_PID` sentinel branch is gated on **`RMCFG_FEATURE_PLATFORM_UNIX`**. On a non-UNIX
platform the compiler takes the `else`, and a kernel-privileged client declares a **real pid**.
`kayfabe_abi::client_kind_from_process_id` would then classify every Windows kernel client as
`ClientKind::User` — and `AllocFacts::client_kind` is *"THE decision-#14 grouping
discriminator"*, whose two failure modes are named in `RmGraphError::UndeclaredClientKind`:
*"'user' folds the guest kernel's UVM session into a guest process's blast radius … 'kernel'
folds a guest process into the guest kernel's isolate."*

**Where the Linux-specific part is isolated:** it is already isolated, and this is the payoff of
having done it as a value rather than a branch. `client_kind_from_process_id` is **one total
function of one declared field** in `kayfabe-abi`, called from nowhere else. The fourth-axis seam
is therefore: *that function becomes a method on a **guest-OS profile** selected at realize,
beside `DriverAbiTable`* — a table, not an `if`, per `four_axes_of_variation.md` §5 rule 1. The
bridge above it never changes: it reads `ClientAllocFacts.process_id`, hands it to the profile,
and puts the resulting `ClientKind` in `AllocFacts`. **Nothing else in this document is OS-aware.**

A second, smaller instance in the same macro: `if (!IsT234DorBetter(pGpu))` (`rpc.h:57`) skips
the whole block, leaving `processID == 0`. `[inferred]` on Tegra-class parts a client root
therefore declares pid 0 and would classify `User{pid:0}` — same seam, same fix. Not a target,
recorded so the seam is not designed for exactly one deviation.

**Not a Windows question but recorded here:** `RMCFG_FEATURE_PLATFORM_UNIX` is a *build* flag of
the guest driver, so this is a property of the guest image, invisible on the wire. It cannot be
detected from protocol facts. That means the guest-OS profile is **configuration**, not
inference — and per `four_axes_of_variation.md` §2, an unconfigured/unsupported guest OS is a
**refusal at realize**, never a default-to-Linux guess. `[unverified]` — no Windows guest has
been observed; the code path above is read, not measured.

---

## 2. The mapping, RPC by RPC

Scope: the functions a GSP-client guest actually sends. Everything else is §4's refusal.

### 2.1 The table

| fn | name | becomes | note |
|---|---|---|---|
| 103 | `GSP_RM_ALLOC` | **`RmEvent::Alloc`** | §2.2 |
| 10 | `FREE` | **`RmEvent::Free`** | §2.3 |
| 21 | `DUP_OBJECT` | **`RmEvent::Dup`** | §2.4 |
| 76 | `GSP_RM_CONTROL` | **`RmEvent::SetPageDir`** for `cmd == 0x0080_1813` only; **nothing** otherwise | §2.5 |
| 71 | `CONTINUATION_RECORD` | **nothing** — it is a *transport* fragment; reassemble, then translate the reassembled message | §2.6 |
| 1 | `SET_GUEST_SYSTEM_INFO` | **nothing** | guest→GSP system description; no object-model content |
| 65 | `GET_GSP_STATIC_INFO` | **nothing** | a *query*. Its reply is the device data model's job (`nvidia-gpu-passthrough/docs/design/mode2_device_data_model.md` class C), not the graph's |
| 47 | `UNLOADING_GUEST_DRIVER` | **nothing** | the FSM already owns it: reply first, then `Suspending` (`boot.rs::GspFsm::answer`, transition E9). ★ It is **not** a graph teardown, and it is not even a reliable *unload* signal: `[measured]` `docs/reference/mode2_bench_lifecycle.md` §2 — *"`rmmod` emits NO fn-47"*, the idle release at process exit having already consumed it. RM's object teardown is the fn-10 stream |
| 72, 73 | `GSP_SET_SYSTEM_INFO`, `SET_REGISTRY` | **nothing**, and **no reply** | `Disposition::NoReply`, already enforced in `boot.rs::GspFsm::answer` |
| 14, 15 | `MAP_MEMORY_DMA`, `UNMAP_MEMORY_DMA` | ★ **never arrive** — see §2.7 | |
| any other | — | **refusal** (§4) | |

★ **The "becomes nothing" column is the interesting one, and it is not laziness.** Three
different reasons are collapsed in the table and must not be:

1. **No object-model content at all** (1, 65, 72, 73) — the RPC carries device description or
   registry keys. Nothing in `RmEvent`'s vocabulary could express it.
2. **Owned elsewhere** (47, 71) — the FSM or the transport already handles it, and a second
   handler is a second source of truth.
3. **Forwarded, not modelled** (most of 76) — the fact *matters*, but it is a host op, not a
   graph edge. It leaves this crate as `Translation::Forward`.

Collapsing 3 into 1 is how the C ended up answering every unmodelled control `NV_OK` with the
request echoed back (`C:3214-3226`, `C:2839`). §4 refuses that.

### 2.2 fn 103 — `GSP_RM_ALLOC` → `RmEvent::Alloc`

```
RmEvent::Alloc {
    client: HClient(hdr.hClient),
    parent: HObject(hdr.hParent),
    handle: HObject(hdr.hObject),
    class:  ClassId(hdr.hClass),      // opaque; Arch::classify interprets it
    facts:  AllocFacts { … },         // from params[], per class — §2.2b
}
```

**★ 2.2a — the client-root normalisation, and it is required.**
`[src]` For `hClass == NV01_ROOT (0x0)` / `NV01_ROOT_CLIENT (0x41)`
(`ogkm: src/common/sdk/nvidia/inc/class/cl0000.h:42`,
`ogkm: src/nvidia/generated/g_allclasses.h:289`), the wire carries
`hParent = hObject = NV01_NULL_OBJECT = 0`: the FWCLIENT macro calls
`pRmApi->AllocWithHandle(pRmApi, hclient, NV01_NULL_OBJECT, NV01_NULL_OBJECT, NV01_ROOT, …)`
(`ogkm: rpc.h:85-87`) and `rpcRmApiAlloc_GSP` copies all three through verbatim
(`ogkm: rpc.c:11007-11009`).

`kayfabe-core` requires *"Allocating the client root itself uses `parent == handle`"*
(`crates/kayfabe-core/src/rmgraph.rs::RmEvent::Alloc` doc). Passing `0/0` through would create a
node at `(client, HObject(0))` whose relationship to the namespace is accidental.

The normalisation is not an invention: **in RM the `hClient` IS its root object's handle** —
`serverAllocClient` writes `pParams->hResource = hClient`
(`ogkm: src/nvidia/src/libraries/resserv/src/rs_server.c:625`), and
`rmgraph.rs::ClientId`'s own doc cites exactly this. So:

> **if `class ∈ {NV01_ROOT, NV01_ROOT_CLIENT}` then `parent = handle = HObject(client.0)`.**

★ Cross-check the value rather than assuming it: `NV0000_ALLOC_PARAMETERS.hClient` is at
params+0 (`kayfabe_abi::view::ClientAllocFacts`, decoded from the **8-byte prefix** for the
reason that view documents), and RM stamps it (`ogkm: src/nvidia/src/kernel/rmapi/client.c:226-227`).
**If `params.hClient != hdr.hClient`, refuse** (`BridgeRefusal::ClientHandleDisagrees`) — the
two are the same fact declared twice, and a disagreement means we have mis-decoded, not that the
guest meant something clever.

**2.2b — `AllocFacts` from `params[]`, per class.** This is where the real work is, and where
today's `kayfabe-abi` is thin. `AllocFacts` has six fields; here is the honest status of each:

| field | source class | abi decoder today | status |
|---|---|---|---|
| `client_kind` | `NV01_ROOT` → `NV0000_ALLOC_PARAMETERS.processID` | ✅ `decode_client_alloc_facts` + `client_kind_from_process_id` | **ready** (and see §1.5) |
| `device_instance` | `NV01_DEVICE_0` → `NV0080_ALLOC_PARAMETERS.deviceId` | ✅ `decode_device_alloc_facts` | **ready** |
| `h_vaspace` | TSG / CtxShare / Channel alloc params | ❌ none | **owed** — B3 |
| `h_ctx_share` | Channel alloc params | ❌ none | **owed** — B3 |
| `userd_flags` | Channel alloc params | ❌ none | **owed** — B3 |
| `mem_phys` | `NV01_MEMORY_*` alloc params | ❌ none | **owed** — B3 |

Until a class has a decoder, its `AllocFacts` is `Default` (all `None`). ★ That is **safe but not
free**, and the split is exactly the core's declared miss taxonomy: a channel with no declared
`h_vaspace` *materialises no `Vas`* (deferred — `Gpu::sync_proc_to_boundary`) and then takes
`FwdFault::NoVas` the instant a doorbell rings it (`kayfabe_fwd::gate_working_set_in`). So a
missing decoder is a **hang at first doorbell, not a wrong answer** — which is the right
direction, and is why B3 can be incremental. It is *not* safe for `NV01_ROOT`: an absent
`client_kind` is `RmGraphError::UndeclaredClientKind`, a hard refusal, by design.

★ `abi`'s own rule applies to growing this table and should be quoted at the site: *"Each needs a
consumer first; a broad table with one wrong entry is invisible until a guest trips it"*
(`crates/kayfabe-abi/src/lib.rs` §4). One class per commit, each with its `RUSTC_OFFSETS`
assertion.

**2.2c — the serialization flag is an observed protocol fact and must be checked.**
`[src]` `rpc_params->flags |= RMAPI_RPC_FLAGS_SERIALIZED` when
`serverSerializeAllocDown` reports a FINN-serialized payload (`ogkm: rpc.c:11021`;
`RMAPI_RPC_FLAGS_SERIALIZED = NVBIT(1)`, `ogkm: src/nvidia/inc/kernel/rmapi/rmapi.h:163`). When
that bit is set, `params[]` is **not** the flat `#[repr(C)]` struct and every offset in §2.2b is
wrong.

⇒ **`flags & RMAPI_RPC_FLAGS_SERIALIZED` ⇒ `BridgeRefusal::SerializedParams { class }`.** Loud,
named, and it fires on a *declared* bit rather than on a length heuristic. `[unverified]` which
classes set it — see §7.

### 2.3 fn 10 — `FREE` → `RmEvent::Free`

`rpc_free_v03_00` = `NVOS00_PARAMETERS_v03_00 { hRoot, hObjectParent, hObjectOld, status }`.
`rpcRmApiFree_GSP` fills `hRoot = hClient`, `hObjectParent = NV01_NULL_OBJECT`,
`hObjectOld = hObject` (`ogkm: rpc.c:11147-11149`).

```
RmEvent::Free { client: HClient(hRoot), handle: HObject(hObjectOld) }
```

`hObjectParent` is **discarded** — it is always 0 on this path, and `RmEvent::Free` does not take
one. ★ Do **not** derive "is this a client-root free?" from `hRoot == hObjectOld`. The C does
exactly that (`bool root = (fClient == fObj);`, `C:1796`) and `rmgraph.rs::HandleRef`'s doc is a
written warning against the same equality: *"free the origin handle while a dup keeps the
resource alive, then dup it BACK … and the alias becomes literally indistinguishable from the
origin allocation … for a `Client`-classed resource the mis-fire is catastrophic."* The graph
already records the declaration; the bridge must not re-derive it.

### 2.4 fn 21 — `DUP_OBJECT` → `RmEvent::Dup`

`rpc_dup_object_v03_00` = `NVOS55_PARAMETERS_v03_00`, filled by `rpcRmApiDupObject_GSP`
(`ogkm: rpc.c:11098-11103`).

```
RmEvent::Dup {
    src: NodeKey::new(HClient(hClientSrc), HObject(hObjectSrc)),
    dst: NodeKey::new(HClient(hClient),    HObject(hObject)),
}
```

`hParent` and `flags` are discarded — `RmEvent::Dup` takes neither. ★ Discarding `hParent` is a
real (small) loss: the destination alias's parent is a declared fact we drop. It is not needed
today (`RmGraph` records a dup as a leaf `HandleRef::Alias`), and adding it to `RmEvent` is a
core change, not a bridge change. Recorded, not done.

`[measured]` Only **25 of 82** dups observed reach the GSP wire
(`docs/reference/rm_semantics_measured.md` §3 — *"Only 25 of the 82 dups reach GSP. Any rule keyed
on the GSP wire must therefore be…"*). This is *the* reason a dup's missing **source object** is DEFER
category 2 and not a fault — and it is a fact about the bridge's input, not about the graph. It
belongs in this document because it is the bridge that makes the graph's input incomplete.

### 2.5 fn 76 — `GSP_RM_CONTROL`

Two arms, and a third that is refused.

**(a) `cmd == NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`0x0080_1813`)** →

```
RmEvent::SetPageDir {
    client:  HClient(hdr.hClient),
    vaspace: HObject(params.hVASpace),      // params+16
    pdb:     Pdb(params.physAddress),       // params+0
}
```

`[src]` layout confirmed three ways: `kayfabe_abi::generated::ctrl::Nv0080CtrlDmaSetPageDirectoryParams`
(generator + `RUSTC_OFFSETS`, size 32, `physAddress@0`/`numEntries@8`/`flags@12`/`hVASpace@16`),
`ogkm: src/common/sdk/nvidia/inc/ctrl/ctrl0080/ctrl0080dma.h:802-809`, and the C's independently
transcribed `physAddress@cmd+120, flags@cmd+132, hVASpace@cmd+136` (`C:2532-2535`) which, minus
the 120-byte control-params base, is `0 / 12 / 16`. **Three oracles, one answer.**

★ The client comes from **`hdr.hClient`, the RPC envelope's own field** — never from a params
field. The C got the analogous case wrong on `GPU_PROMOTE_CTX`: it *ignores* the RPC's `hClient`
and uses `params+12` (`hChanClient`) instead (`C:2283`). Sometimes right, and unprincipled: the
namespace a control is *issued in* is the envelope's, full stop. A params field naming a
different client is a **cross-namespace reference**, which is a different fact and, if we ever
need it, needs its own event — not a silent substitution.

`aperture` (`flags[1:0]`) is decoded by `kayfabe_abi::view::PdbAperture` but **`RmEvent::SetPageDir`
has nowhere to put it.** Recorded as a known drop; the core keys the address plane on `Pdb`
alone. `[open]` — if the walker needs to know whether the root is in vidmem or sysmem, the field
has to reach it, and today it cannot.

**(b) any other `cmd`** → **no `RmEvent`**, and `Translation::Forward { client, object, cmd,
params }` for `kayfabe-fwd` to classify. The Case-1/Case-2 split
(`nvidia-gpu-passthrough/docs/design/mode2_forwarding_model.md:37-50`) is *that* crate's table
and is deliberately not duplicated here.

**(c) `rmapiRpcFlags & RMAPI_RPC_FLAGS_SERIALIZED`** (`ogkm: rpc.c:10805-10806`) → refusal, same
as §2.2c.

★ **What is deliberately NOT modelled, with the C's own contrary practice named:** the C treats
`NV2080_CTRL_CMD_GPU_PROMOTE_CTX` (`0x2080012b`, `C:2425`, `C:2280-2311`) and
`NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` (`0x90f10106`, `C:2492-2530`) as *primary*
address/PDB sources, and `0x00801813` only as an **extra candidate** — *"APPEND a candidate root
(do NOT overwrite the RESERVED_PDES root for the same hVASpace — they can differ"* (`C:2538-2545`).
It then resolves the ambiguity at use by **walking every captured root and taking the first that
resolves without faulting** (`C:5070-5133`).

That resolver is precisely what `mode2_address_table_of_truth.md` and
`nvidia-gpu-passthrough/docs/design/mode2_address_table.md:183-196` forbid — *"We explicitly do
**NOT** do an opportunistic 'walk the PDB one last time' on a miss"* — and the C's own comment
records it causing a permanent hang for a second process (`C:5092-5099`). `RmEvent::SetPageDir`
is single-valued by design and a second, different PDB for one VASpace must therefore be a
**refusal, not a second candidate**. ⚠ **The consequence is named in §7:** if the compute VAS's
real PDB arrives on `0x90f10106` and not on `0x00801813`, this design has no PDB at all for that
VAS and every channel in it defers forever. That is a bench question and it is not answered.

### 2.6 fn 71 — `CONTINUATION_RECORD`: reassembly, not translation

`[src]` `_issueRpcLarge` splits a message larger than `pRpc->maxRpcSize`: the **first** element
carries the real function with `length = maxRpcSize`, and each subsequent element carries
`function = NV_VGPU_MSG_FUNCTION_CONTINUATION_RECORD` with `length = entryLength +
sizeof(rpc_message_header_v)` and a raw payload slice (`ogkm: rpc.c:2074-2143`, notably `:2109`
`entryLength = maxRpcSize - sizeof(rpc_message_header_v)` and `:2124`). It is reached from
`rpcRmApiControl_GSP` when `message_buffer_remaining < paramsSize` (`ogkm: rpc.c:10856`) and from
`_issuePteDescRpc` (`:2323`).

So the bridge needs a **reassembly buffer**: hold the head message, append each continuation's
payload, translate when the declared total is complete. Bounds, all mandatory:

- a **maximum reassembled size**, refusing beyond it (`BridgeRefusal::ContinuationOverflow`);
- a **maximum continuation count**;
- a continuation arriving with **no head in flight** is a refusal, not a new head;
- a **new head while one is in flight** is a refusal (`ContinuationInterleaved`) —
  `[inferred]` the driver issues one large RPC at a time under the GPU lock, so interleaving is
  not a legal trace; refusing it is category 3.

#### 2.6a ★★ SETTLED AT B6 — corrections this section needed

**(1) The "declared total" is fn 76's `paramsSize`, and fn 103 has no large path at all.**
There is no total-length field on the wire; the head's `length` is `maxRpcSize` and each
continuation's is its own fragment size. The only total is the head's *own body*:
`total_size = fixed_param_size + paramsSize` (`ogkm: rpc.c:10785`, `ogkm-580: :10981`), i.e.
`params_at + paramsSize` in payload coordinates. `[src]` `rpcRmApiAlloc_GSP` **never** calls
`_issueRpcAndWaitLarge` — it bounds the copy and returns `NV_ERR_BUFFER_TOO_SMALL`
(`ogkm: rpc.c:11024-11029`, `ogkm-580: :11218-11223`) — so a short fn-103 is malformed, not
fragmented, and must keep reaching `ParamsSizeExceedsPayload`. **The complete set of fragmenting
producers is three**, and only one is a function this bridge translates: `rpcRmApiControl_GSP`
(fn 76, `bBidirectional = NV_TRUE`), `rpcSetRegistry` (fn 73, `_issueRpcAsyncLarge`), and
`_issuePteDescRpc` (fn `ALLOC_MEMORY`, unidirectional) — the last of which this port classifies
`Other`.

**(2) A head always carries the control's whole 40-byte fixed header.**
`rpcRmApiControl_GSP` opens with `message_buffer_remaining = pRpc->maxRpcSize - fixed_param_size`,
an *unsigned* subtraction over 72 (`ogkm: rpc.c:10678-10679`), and `maxRpcSize = RM_PAGE_SIZE`
(`:1000`). So the tightest legal split is 72 bytes, a shorter head is category 3, and the
reassembler refuses it immediately (`Abi(Truncated)`) rather than holding it.

**(3) ⇒ Nothing this port models fragments at production `maxRpcSize`.** `SET_PAGE_DIRECTORY`'s
body is 40 + 32 = 72 bytes and fits in one 4096-byte message; even at the tightest legal split it
reaches two records and never more. B6's value today is for the control **long tail** — it is what
lets an unmodelled large control be refused by its real `cmd` instead of by a truncated one.

**(4) A `CONTINUATION_RECORD` *does* earn its own reply — for fn 76, and it is mandatory.**
`_issueRpcLarge` posts every fragment and waits **once** at `(expectedFunc, firstSequence)`
(`ogkm: rpc.c:2156-2158`) — the reading this section guessed. But that is only the *send* half.
When `bBidirectional && recordCount > 0` the receive half then polls
`rpcRecvPoll(…, NV_VGPU_MSG_FUNCTION_CONTINUATION_RECORD, waitSequence)` per record until the
reply bytes fill the request's own `bufSize` (`:2186-2226`) — and `rpcRmApiControl_GSP` is
`_issueRpcAndWaitLarge(…, NV_TRUE)` (`:10856`, `ogkm-580: :11051`). ⇒ **Withholding a per-fragment
reply hangs the guest.** The FSM already posts one, echoing each fragment's own
`(function, sequence)` and length, which is exactly what that loop consumes — so B6 changed no
transport code.
★ And the driver reads `rpc_result` from the **last** record it received (`:2230-2241`), which is
the same fragment on which reassembly completes. The status therefore rides the final reply by
construction; head and intermediates ack `NV_OK`.

⚠ **The one remaining hole, named rather than absorbed.** `SET_REGISTRY` is `bWait = NV_FALSE`
and `RpcFunction::SetRegistry`'s `Disposition` is correctly `NoReply` — but its *continuations*
take `ContinuationRecord`'s disposition, which is `Reply`. A registry table over 4064 bytes would
therefore draw spurious status posts. It is not fixable from `kayfabe-rmrpc`: `Disposition` is
computed in `GspFsm::answer` from the arriving function alone, `CommandPolicy::respond` has no
"post nothing" value, and making a fragment inherit its head's disposition would put a second copy
of the reassembly state inside the FSM. **This is a `kayfabe-gsp` question and it is open.**

### 2.7 ★★ fn 14 / fn 15 do not exist here — `RmEvent::MapMemoryDma` has **no producer**

This is the largest finding in this document and it changes what "the boot path" means.

`[src]` On GA106, `rpcMapMemoryDma` and `rpcUnmapMemoryDma` are **stubs**:
`rpcHalIfacesSetup_GA106` → `_GA102` → `_GA100` (`ogkm: src/nvidia/generated/g_rpc_private.h:4339,
4311, 4142`), whose interface block contains `rpcMapMemoryDma_STUB` (`:4292`) and
`rpcUnmapMemoryDma_STUB` (`:4295`); the stub returns `NV_VGPU_MSG_RESULT_RPC_UNKNOWN_FUNCTION`
(`ogkm: src/nvidia/generated/g_hal_stubs.h:1765-1772`, whose own comment lists **TU10X, GA100,
GA102, GA103, GA104, GA106, GA107, AD10X, GH10X, GB…** — every GSP-client part). The real
implementation `rpcMapMemoryDma_v2C_05` is installed only by the vGPU IP-version table
(`g_rpc_private.h:3224`, inside `rpc_iGrp_ipVersions_Install_*`).

`[measured]` The C artifact — which booted a real GA106 guest to CUDA — has **no fn-14 or fn-15
hook at all**. Its complete snoop set is 47 / 76 / 21 / 103 / 10 / 65 / 70 / 72 / 73
(`C:2415-3291`).

`[src]` And the C-era design docs say the same thing flatly, twice:
*"channels are filled GSP-side — there is NO forwardable MAP_MEMORY_DMA (fn=14)"*
(`nvidia-gpu-passthrough/docs/design/mode2_compute_forwarding.md:47`); *"(… MAP_MEMORY_DMA=14,
DMA_FILL_PTE_MEM=27) **never fire**"* (`…/mode2_address_virtualization.md:134`); and
`…/mode2_address_table.md:89` states the mechanism — *"the common map path is the CPU-side MMU
walker `dmaUpdateVASpace` … **there is no per-map GSP-RPC carrying VA↔phys for it**."*

**Three independent oracles agree.** Consequences, stated plainly:

1. `RmEvent::MapMemoryDma` and `RmEvent::Unmap` are **unreachable from this bridge**. They stay
   in `RmEvent` — they are real facts on the Mode-1 ioctl path and the core is right to model
   them — but the GSP bridge never emits one, and no stage below builds a decoder for `NVOS46`.
2. Everything in `Gpu::sync_rpc_mappings` — the RPC populate source — is therefore driven by
   **nothing** in Mode 2 until the *other* two sources port:
   the `GPU_PROMOTE_CTX` control (`0x2080012b`) and the **CE page-table-write capture**
   (`…/mode2_address_table.md:117-129`, the 2026-07-22 correction: *"the address table has **two
   co-equal populate sources, not 'RPC + read-at-invalidate'**"*).
3. ⇒ **`GPU_PROMOTE_CTX` is not an optional extra — it is the only address-populating RPC there
   is.** It is nevertheless deferred out of this design, because it does **not** map onto
   `RmEvent`: it declares a *set* of `(gpuPhysAddr, gpuVirtAddr, size, bufferId)` promote
   entries (`C:2276-2295`), which is an address-table population, not an object-model edge. It
   belongs with the CE-capture feed in `kayfabe-fwd`, against `kayfabe_mmu::AddressTable`, and
   `kayfabe-fwd`'s own crate doc already lists *"CE PT-write capture feed (#13)"* as a
   documented skeleton. **This bridge's scope ends at the object model.** Said out loud so the
   next reader does not conclude the compute path is one stage away.

---

## 3. ★ The identity story

### 3.1 The bridge mints nothing. This is the whole answer.

`RmEvent`'s payloads are `HClient` / `HObject` / `NodeKey` — **raw guest-declared handle values**.
`ClientKey` and `ResourceKey` appear nowhere in `RmEvent`. They are minted **inside**
`RmGraph::apply`, from the live set, at the moment of declaration:
`rmgraph.rs::RmGraph::next_incarnation` (*"the smallest ordinal not held by a live resource at
this key"*) and `::next_client_incarnation`. The bridge cannot mint one because it cannot see the
live set, and it must never acquire the ability.

So the brief's premise — *"the bridge mints identities from guest-supplied handles, so it is
squarely in that blast radius"* — is **false as stated**, and it is worth being precise about
why, because the true risk is adjacent and sharper.

### 3.2 What the bridge *is* responsible for: **namespace attribution**

The bridge chooses, for each fact, **which `HClient` namespace it belongs to**. That is one
field, from one place, and getting it wrong is exactly the §12.38 shape (a fact landing in the
wrong component). The rule:

> **The namespace is always the RPC body's own `hClient`. Never a params field. Never inferred.**

Enforced structurally: the translate function takes the header's `client` once, at the top, and
the per-class params decoders are **not given it**, so a params-derived client cannot be
substituted without a visible signature change. (The C's `GPU_PROMOTE_CTX` handler is the
counter-example: `C:2283` reads `hChanClient` from `params+12` and never looks at `cmd+80`.)

The one place two sources exist is the client root, where `hdr.hClient` and
`params.hClient` are the same fact twice — and §2.2a **compares them and refuses a
disagreement** rather than picking one.

### 3.3 Recycle: the bridge does nothing, deliberately

`hClient` and `hObject` values are recyclable **by RM's own design** — the citations are in
`rmgraph.rs::ResourceKey` and `::ClientId` (caller-supplied handles honoured verbatim under
`RS_COMPATABILITY_MODE=1`; a generator that wraps with **no free list and no quarantine**; no
epoch anywhere in `RsClient`/`RmClient`/`CLIENT_ENTRY`/`RsResourceRef`). Refusing a recycle
hangs a legal guest.

Therefore the bridge:

- keeps **no handle table**, **no seen-set**, **no dedup cache**. It is a pure function of one
  message. Statelessness is what makes recycle-safety structural rather than tested.
- keeps **no mapping** from guest handle to anything of ours.
- never asks "have I seen this handle before?" — that question has exactly one correct owner,
  and `RmGraph::apply` already answers it with `ConflictingAlloc` / `DuplicateClientRoot` /
  idempotent acceptance of an identical re-send.

The only bridge state that exists at all is the §2.6 continuation-reassembly buffer, which is
keyed by nothing (one in flight, or a refusal) — chosen that way for this reason.

★ The two identity bugs the brief cites are both *downstream* of this seam
(`ResourceKey`/`ClientKey` incarnations, §12.41/§12.42; and `IsolateId` naming half of what it
identified, commit `07da582`). The bridge's protection against joining that list is that it holds
no identity at all. **If a future stage wants to add a per-handle cache here for performance, it
is re-opening that entire bug class** and should be refused unless the cache is keyed by
something the graph itself minted.

### 3.4 Does `MISS = FAULT, never reverse-resolve` apply here?

**No — and saying so precisely matters, because the rule is about a table and the bridge has
none.**

`mode2_address_table_of_truth.md` and `…/mode2_address_table.md:183-196` govern *lookups in the
VA→phys table*: a miss means the guest never committed that VA, so walking for it would read
uncommitted page-table state. The bridge performs **no lookup of any kind**. It resolves no
handle, consults no table, and never asks whether a referenced object exists.

The MISS/DEFER/FAULT question is therefore entirely `RmGraph::apply`'s, and it is already
categorised there in three buckets (`crates/kayfabe-core/src/lib.rs` crate doc):

| the RPC references… | who answers | answer |
|---|---|---|
| an **undeclared client namespace** | `RmGraph::undeclared_namespace` | **FAULT** — category 3, `RmGraphError::UndeclaredClient` |
| a **not-yet-seen parent / VASpace / dup source** | `RmGraph::apply` (parks it) | **DEFER** — category 2, resolved when the fact lands |
| a **VA with no binding** | `kayfabe_mmu::AddressTable` at doorbell time | **FAULT** — the address-table rule proper |

**The bridge's only duty is not to pre-empt any of these.** It must not, for example, "check that
the client exists" before emitting — that would be a second, weaker copy of the category-3 rule
in a crate with no graph to check against. It emits the fact and lets `apply` decide.

What it **must** do is not swallow the answer: `Gpu::apply` returning
`Err(GpuError::Graph(RmGraphError::UndeclaredClient{..}))` becomes a non-zero `rpc_result` on the
reply and a trace event (§4). The C's failure was the opposite — it accepted everything and
answered `NV_OK` (`C:3326`).

---

## 4. The refusal surface

**The governing rule:** *"An unrecognised or malformed RPC is a LOUD REFUSAL, never a best-effort
guess or a silent drop."* This is an **authorised deviation from the C**, and the C's behaviour is
named so the deviation is a decision rather than an omission:

> `C:2737` `memcpy(resp, cmd, 4096)` → `C:3326`
> `nvkvm_m3_post_status(s, resp, fn, 0 /* NV_OK */)`. An unknown RPC function is answered
> **affirmatively, indistinguishably from a real success**, with the request body reflected back
> as the reply body, with no allowlist, no counter and — outside `-trace` — **no log line at
> all**. The unrecognised-*control* path is the same (`C:3214-3226`, `C:2839`: *"else: void/SET
> control — echo with status=NV_OK"*).

★ Per `testing_doctrine.md` §8, the C's omission is evidence: it booted a guest to CUDA this way,
so **most** unknown RPCs genuinely are inert. That is an argument for making the refusal
*observable and cheap to widen*, not for keeping the echo. The two failure directions are not
symmetric: an over-strict refusal is a guest that stops with a named reason at a known RPC; the
echo is a guest that proceeds on a lie and fails somewhere else entirely.

### 4.1 What a refusal *is*

```rust
pub enum BridgeRefusal {
    UnknownFunction { code: u32 },
    UnknownControl { cmd: u32 },                 // only if we choose to refuse rather than Forward
    Truncated { need: usize, have: usize },
    ParamsSizeExceedsPayload { declared: u32, available: usize },
    SerializedParams { class: u32 },             // §2.2c
    ClientHandleDisagrees { header: u32, params: u32 },   // §2.2a
    ReservedClient,                              // hClient == 0, refused before it reaches the graph
    ContinuationWithoutHead,
    ContinuationInterleaved,
    ContinuationOverflow { total: usize, max: usize },
    Abi(kayfabe_abi::wire::AbiError),
    Graph(kayfabe_core::gpu::GpuError),          // the graph refused an otherwise well-formed fact
}
```

### 4.2 Where it surfaces — **three places, and all three are mandatory**

1. **On the wire, as a reply.** ★ A refusal **still posts a reply**, with a non-zero
   `rpc_result`. It is never a drop. `[src]` the guest blocks in `_issueRpcAndWait` polling
   `(function, sequence)` (`ogkm: src/nvidia/src/kernel/vgpu/rpc.c:9146-9170`, cited in
   `kayfabe-gsp`'s `rpc.rs`); an unanswered command hangs it for the full RPC timeout, and for
   fn-47 that hangs `rmmod`. Mechanically: `respond` returns
   `Some(Reply { rpc_result: <NV_ERR_*>, body: <request body> })`, which
   `RpcCommand::reply` clamps to the request's own length — the M9 clamp the C learned the
   expensive way (`C:3237-3252`).
   `[open]` **which** `NV_ERR_*` to send is not settled here. `NV_ERR_NOT_SUPPORTED (0x56)` is
   the value the C uses for the two controls it deliberately fails (`C:2883`, `C:2894`) and its
   note is that RM then sets `SKIP_COPYOUT` — i.e. the status changes the *guest's* copy-out
   behaviour. Picking the wrong one is a real bug and it needs the `NV_STATUS` table in
   `kayfabe-abi`, which does not exist. **B1 sends one value, names it, and B4 revisits it.**
2. **In the trace.** A typed `TraceEvent` per refusal, so refusals are *countable*. Per
   `testing_doctrine.md` §2 rule 4, the invariant is a **bound** ("zero refusals over a clean
   boot script"), never an absence.
3. **In the return value.** `Translation` is a `Result`; the caller cannot ignore it
   (`#[must_use]`).

### 4.3 The four cases the brief names

- **Malformed** — short payload, `paramsSize` beyond the payload, an alloc whose class has a
  known param size that the declared size contradicts. All are `Truncated` /
  `ParamsSizeExceedsPayload` / `Abi(AbiError::Truncated)`. ★ Note `paramsSize` is
  **guest-declared**, so it is *"an assertion by the guest, not a fact"*
  (`kayfabe_abi::view::AllocReq` doc) — validate against the payload length **and** against the
  class's own size where `DriverAbi::alloc_param_size` knows it, and refuse the mismatch rather
  than taking the smaller.
- **Unknown function** — `RpcFunction::Other(code)` → `UnknownFunction`. Every id in
  `FunctionCodes` that this document maps to "nothing" is *known and inert*, which is a
  different state and must be a different arm. There is no third state.
- **Out of order** — **not the bridge's**, twice over. Element-level ordering is already
  `GspFault::SeqNumGap` in `element.rs::decode_message`. Fact-level ordering is
  `RmGraph::apply`'s three-category taxonomy (§3.4). The bridge adds exactly one ordering rule of
  its own, and only because it holds one piece of state: the continuation head (§2.6).
- **Replayed** — also not the bridge's, and this is a genuine strength of the existing design.
  A *retransmitted element* is caught by the per-message `seqNum` discipline. A *legitimately
  re-sent fact* is accepted idempotently by the graph: `RmGraphError::ConflictingAlloc`'s doc —
  *"an identical re-send is accepted idempotently — retried-RPC tolerance"*. The bridge is
  stateless and pure, so it maps a replayed message to the identical event, which is precisely
  what makes that tolerance reachable. **A bridge with a dedup cache would break this.**

---

## 5. ★ Test strategy, per stage

Everything below runs with **no GPU, no guest, no QEMU, no hypervisor and no OS** — the whole
bridge is a pure function from bytes to events, and the one stateful piece (continuation
reassembly) is a value.

### 5.1 The oracle rule this project keeps violating

`tests/src/gspworld.rs::Guest::recv` was written as an *independent* re-implementation of the
driver's receive path, and the crate doc says why that matters. It is independent about the
checksum and the acceptance predicate. It is **not** independent about length:

- it reads `rpc_length` **out of the element under test** (`let rpc_length = get32(&first,
  self.p.hdr + 8)`), derives `msg_len` and `n = msg_len.div_ceil(self.msg_size)` from it, folds
  the checksum over that same derived `msg_len`, and slices the payload by it;
- it **never reads `p.elem_count`** — `Profile::elem_count` is used in `Guest::send` and nowhere
  in `recv`.

`[src]` But the 580 driver **reads `elemCount`@40**, and *that read drives
`msgqRxMarkConsumed`* (`mode2_gsp_port_plan.md` §14.1 item 1, §9 D3). So
`kayfabe_gsp::element::encode_message`'s `elem_count` write — a field whose *absence* on 610
would corrupt `rpc.sequence` if written at the wrong offset — is **unobserved by its own
oracle**. A self-consistent wrong length passes, because both sides compute it from the same
bytes.

**The rule this yields, and the bridge must obey it:**

> **An oracle may not derive the value under test from the artifact under test.** For every
> assertion, name the *independent* source of the expected value.

Concretely for the bridge — the trap is obvious and would be easy to walk into: *build the RPC
bytes with a helper, decode them with the bridge, assert the round trip.* That tests nothing but
the helper's agreement with itself.

**Instead:**

- **Byte fixtures come from a builder written from `ogkm: g_rpc-structures.h:1408-1435`**, in a
  file that does not `use` the decoder, with each field's offset written as a literal beside its
  `ogkm` line. At least one fixture per function is a **hand-written hex array** with an
  offset-annotated comment — unreadable, deliberate, and the thing a decoder bug cannot agree
  with.
- **A differential against the C's independently-transcribed offsets** (§1.4): a table test
  asserting `abi_offset == c_element_offset - 80` for all twelve fields. Two humans read two
  trees; the test is the agreement.
- **`RUSTC_OFFSETS`** already pins `Nv0080CtrlDmaSetPageDirectoryParams` against the compiler.
- **The end-to-end oracle is `Scenario`, which is *not* replaced.** `tests/src/lib.rs::Scenario`
  stays exactly as it is: the spec-level DSL for the facts the design docs name. The bridge adds
  `RpcScript`, which produces **bytes**, and the top-level test is:

  > `Gpu::apply` fed from `RpcScript(X)` produces the **same `project::Boundaries`** as
  > `Gpu::apply` fed from `Scenario(X)`.

  `Boundaries` already has `PartialEq` and is already compared whole across shuffled orders
  (decision #4), so the comparison is exact and cheap. This is the strongest oracle available
  and it retires "the tests synthesise `RmEvent`" **without deleting the synthesiser** — it
  turns it into the reference implementation. ★ Non-vacuity arm, per `testing_doctrine.md` §2
  rule 2: mutate one field in the `RpcScript` bytes and assert the boundaries **differ**.

### 5.2 Per-stage test obligations

| stage | testable with no GPU? | oracle | non-vacuity arm |
|---|---|---|---|
| **B0** abi decoders | yes, entirely | ogkm struct defs + the C-offset differential + `RUSTC_OFFSETS` | a one-byte fixture change moves exactly one decoded field |
| **B1** translate (root/free) | yes | hand-hex fixtures; expected `RmEvent` written by hand | a fixture with `hClient=0` refuses `ReservedClient`; one with class≠ROOT does not take the normalisation branch |
| **B2** `CommandPolicy` adapter | yes — `gspworld` drives the whole FSM in-process | `Boundaries(RpcScript) == Boundaries(Scenario)` | the graph refusal path: a `Dup` into an undeclared namespace must produce a **non-zero `rpc_result`**, asserted by variant |
| **B3** per-class `AllocFacts` | yes | one `ogkm` alloc-params struct per class, `RUSTC_OFFSETS` each | with the decoder removed, the channel gets no `Vas` and the doorbell takes `FwdFault::NoVas` — assert *that*, so the decoder's value is measured, not assumed |
| **B4** SET_PAGE_DIRECTORY | yes | three oracles (§2.5) | a second, **different** PDB for one VASpace refuses (never a second candidate — the C's `C:2538-2545` behaviour is the anti-oracle) |
| **B5** dup + the mean test | yes | `l1_mean.rs` | §5.3 |
| **B6** continuation records | yes | `ogkm: rpc.c:2074-2143` fragment arithmetic | a head with no continuations still translates; a continuation with no head refuses |

### 5.3 The mean test (`testing_doctrine.md` §3.1 — the bar for "done")

Not a happy-path boot. The obligation is a composed run, wired into `tests/tests/l1_mean.rs`:

- **two guest processes' RPC streams interleaved element-by-element** in one command queue, with
  identical `hObject` values across them (the #14 shape `identical_handles` already builds),
  asserting two distinct `Proc`s, two arenas, two host VASes;
- a **handle recycled mid-stream** — free a client root while a dup keeps one of its resources
  alive, then re-declare the same `hClient` — asserting two `ClientKey` incarnations and that
  the older declaration's resources still belong to a boundary (the §12.42 regression, driven
  from bytes for the first time);
- **malformed messages interleaved with valid ones**, asserting the valid stream is unaffected
  and each refusal is counted by variant;
- a **serialized-params alloc** and an **unknown fn** in the middle of a live stream;
- **both lock modes**, and the boot exercised through **both element layouts** — `gsp_boot.rs`
  already does the latter and the bridge inherits it for free.

★ And the rule that outranks all of them: *"NEVER narrow a test to make it pass."* A failing mean
test here is the finding.

---

## 6. Staged build order

Each stage is independently green (`cargo test --workspace` passes at every commit) and names
what it cannot prove.

| stage | builds | first-time capability | cannot prove |
|---|---|---|---|
| **B0** | `kayfabe-abi`: `view::RpcAllocReq`/`RpcControlReq` + `DriverAbiTable::decode_rpc_alloc`/`decode_rpc_control`; a pinning test that `decode_free`/`decode_dup` apply to RPC bodies | the GSP wire shapes are decodable | that any of it is ever called |
| **★ B1** | crate `kayfabe-rmrpc`; `translate(&DriverAbiTable, &RpcCommand) -> Result<Translation, BridgeRefusal>` for **fn 103 with `hClass ∈ {NV01_ROOT, NV01_ROOT_CLIENT}`** and **fn 10**; everything else `UnknownFunction`. Plus a `#[test]` that constructs a real `Gpu` (mock arch/isolate) and drives `Gpu::apply` from a hand-hex fixture | ★ **`RpcCommand` reaches `Gpu::apply` for real** — a client namespace is declared and freed from wire bytes | anything about the FSM; anything about channels |
| **B2** | `GraphPolicy<'a> { gpu: &'a mut Gpu, … }: kayfabe_gsp::CommandPolicy`; `respond` = translate → `Gpu::apply` per event → `Reply`. Wire into `gspworld` so a scripted boot drives the graph | the FSM and the core are connected end to end, in one process, no GPU | that a real guest sends what the script sends |
| **B3** | per-class `AllocFacts` decoders, **one class per commit**: Device (already), Channel (`h_vaspace`, `h_ctx_share`, `userd_flags`), TSG, CtxShare, Memory (`mem_phys`) | a compute-process-shaped subgraph from wire bytes; `Boundaries(RpcScript) == Boundaries(Scenario)` for `compute_process` | that the `userd_flags`→`VChid` recovery matches real silicon (`Arch`'s job, and unmeasured) |
| **B4** | fn 76: `SET_PAGE_DIRECTORY` → `RmEvent::SetPageDir`; `Translation::Forward` for every other cmd; the `NV_STATUS` question from §4.2 revisited | a routable `Vas` with a PDB, from bytes | which control actually carries the compute VAS's PDB (§7) |
| **B5** | fn 21 dup; the §5.3 mean test | two `Proc`s from two interleaved RPC streams | multi-process against a real guest |
| **B6** ✔ | `Reassembler` (in `kayfabe-rmrpc`, held by `GraphPolicy` — `translate` stays a free function); five refusals; the hostile-length matrix | large controls: an unmodelled one is refused by its **real** `cmd` rather than by a truncated one | that any of it is reachable at production `maxRpcSize` — §2.6a(3): nothing this port *models* fragments at 4096 |

**Deliberately out of scope, and each is a separate piece of work:** `GPU_PROMOTE_CTX` and the CE
page-table-write capture (§2.7 — they populate `kayfabe_mmu::AddressTable`, not the graph); the
Case-1/Case-2 control tables (`kayfabe-fwd`); reply *bodies* for `GET_GSP_STATIC_INFO` and the
init-control corpus (the device data model); and everything downstream of §11-O7a.

### 6.1 ⛔ The O7a boundary — where this design stops

`mode2_gsp_port_plan.md` §11-O7a and §14.6 record an unresolved, version-split fact: at 580 the
GSP **resume** handoff lives inside `kgspExecuteSequencerCommand_TU102`'s
`GSP_SEQ_BUF_OPCODE_CORE_RESUME` arm and is reachable **only** from `_kgspRpcRunCpuSequencer`,
i.e. only from a `GSP_RUN_CPU_SEQUENCER` event **we** would have to emit; at 610 the same handoff
is a local HAL call with no RPC involved (`ogkm-580: kernel_gsp_tu102.c:913-960` vs
`ogkm-610: kernel_gsp_falcon_tu102.c:441-471`). The plan's own words: *"nothing in the 580 tree
was traced from a resume entry point down to a `GSP_SEQ_BUF_OPCODE_CORE_RESUME` buffer … That is
a genuine hole and it is named as one."*

**This design touches it at exactly one point and stops there.** `GSP_RUN_CPU_SEQUENCER` (0x1002)
is an *event we would send*, not a command we receive, so it is outside the bridge's direction of
flow entirely. The bridge's contact with the question is one line in §2.1: id 0x1002 is neither
mapped nor refused **by the bridge**, because the bridge never sees it. If a future stage needs
the faked GSP to emit sequencer buffers, that is an emitter in `kayfabe-gsp` (and an owner
decision about which tag governs), and **nothing in this document may be read as having taken a
position on it.**

---

## 7. What could not be determined

Plainly, and each is a real hole rather than a formality.

1. **★ Which control carries the compute VAS's PDB.** §2.5 models only
   `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`0x00801813`). The C treated
   `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` (`0x90f10106`) as a **co-equal, and in
   places primary, root source** and kept both as candidates (`C:2492-2545`). We reject
   two-candidates-and-probe on principle (it is the C's named bug), but that leaves an open
   question of fact: **if the compute VAS's real PDB only ever arrives on `0x90f10106`, this
   design produces no `SetPageDir` for it at all** and every channel in that VAS defers forever.
   `kayfabe-abi` has no decoder for `0x90f10106` and its params (`levels[]`) were not read.
   *Settling it:* read `ogkm`'s `vaspaceCopyServerReservedPdes` callers for a GSP client — **no
   GPU needed**. This should be done **before B4**, not after.
2. **Whether fn-14/fn-15 can reach the wire on any path.** §2.7's `[src]` is the per-chip HAL
   table, and I checked GA106's chain (`_GA106`→`_GA102`→`_GA100`) only. Three oracles agree,
   but I did not enumerate every caller of `NV_RM_RPC_MAP_MEMORY_DMA` for a
   `!IS_VIRTUAL && IS_GSP_CLIENT` path that might bypass the HAL. `[inferred]` no such path
   exists. Note `virtual_mem.c:461` sets `bRpcAlloc` for `IS_GSP_CLIENT` too, so the call *is*
   reached and returns `NV_VGPU_MSG_RESULT_RPC_UNKNOWN_FUNCTION` from the stub — **I did not
   trace what the caller does with that status**, and if it is treated as a hard failure then my
   reading of some adjacent guard must be wrong. Worth ten minutes before B0.
3. **Which alloc classes set `RMAPI_RPC_FLAGS_SERIALIZED`.** §2.2c refuses them by name, which
   is safe, but if a *boot-path* class is serialized then B3 hits a wall rather than a long tail.
   Requires reading the FINN serializer registration (`g_finn_rm_api.h`), not done.
4. ~~**Whether a `CONTINUATION_RECORD` earns its own reply** (§2.6).~~ **CLOSED at B6 — see
   §2.6a(4).** The send-side read was right and incomplete: fragments are sent with one wait at
   the end, *and* the bidirectional receive half then polls one reply per record. For fn 76 —
   the only fragmenting function this bridge translates — a per-fragment reply is **required**,
   and the FSM already posts one on the right `(function, sequence)`. What replaces it as an open
   question is narrower and named there: `SET_REGISTRY`'s continuations inherit the wrong
   `Disposition`, which is `kayfabe-gsp`'s to answer.
5. **Which `NV_STATUS` a refusal should carry** (§4.2). There is no `NV_STATUS` table in
   `kayfabe-abi`. The C's only two deliberate failures both use `0x56` (`NV_ERR_NOT_SUPPORTED`)
   and its comments say the choice changed guest copy-out behaviour — so this is not cosmetic.
6. **Whether refusing an unknown control (vs. forwarding it) is right.** §2.1 routes unknown
   controls to `Translation::Forward`, which defers the decision to `kayfabe-fwd`'s
   classification. That crate's table does not exist yet, so today "forward" means "nothing
   happens", which is the C's echo by another name. **B4 must pick one and say which.**
7. **The `RmEvent::Dup` `hParent` drop and the `SetPageDir` `aperture` drop** (§2.4, §2.5) — two
   declared facts this design discards because `RmEvent` has nowhere to put them. Neither is
   known to be needed; neither is known not to be.
8. **Everything about real hardware.** No guest driver has ever posted a message to
   `kayfabe-gsp`; every byte it has parsed was written by a test into a `BTreeMap`. Nothing here
   is validated against a boot.

---

## 8. Corrections this file makes to its own brief and to the plan

Recorded per §14.5's class fix — a superseded claim should be visibly superseded, not silently
edited around.

1. **`mode2_gsp_port_plan.md` §2 item 3 and §5's S4 row place "RPC decode → `RmEvent`" inside
   `kayfabe-gsp`.** It is not there and must not be (§1.2). `crates/kayfabe-gsp/src/rpc.rs`'s
   module doc places it in `kayfabe-fwd`; that is also wrong (§1.2). It goes in a new crate.
2. **"The bridge mints identities from guest-supplied handles."** It does not, and must not
   (§3.1). `RmEvent` carries raw handles; `RmGraph::apply` mints. The real risk at this seam is
   **namespace attribution** (§3.2), which is a different and narrower thing.
3. **"MISS-is-FAULT applies here."** It does not — the bridge performs no lookup (§3.4). The rule
   is alive and unweakened; it simply has a different owner.
4. **"The mapping, RPC by RPC for the boot path."** The boot path's *object-model* RPCs are five
   (103, 10, 21, 76, 71). The address-populating path is **not** among them and is not one stage
   away: `MAP_MEMORY_DMA` never reaches the wire on a GSP client (§2.7), so `GPU_PROMOTE_CTX` and
   the CE page-table-write capture are the only populate sources, and both are `kayfabe-fwd`
   work against `kayfabe_mmu::AddressTable`.
