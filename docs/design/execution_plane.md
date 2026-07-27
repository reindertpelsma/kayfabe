# The Mode-2 execution / forwarding plane — designed into the core

**Status:** design, 2026-07-24. Repo `kayfabe`, branch of record for the rewrite.
Governs how the core runs *actual GPU work* — GR (compute + graphics), CE (copy),
NVENC/NVDEC video — as an **orchestration** layer that forwards real pushbuffers to a
real host GPU, never as an emulator of any engine.

**Companion docs (this synthesizes, it does not re-derive):**
`../../ARCHITECTURE.md`; the settled architecture in
`../../../nvidia-gpu-passthrough/docs/design/mode2_rust_rewrite_architecture.md`
(esp. §4.2, §4.3.1/§4.3.1a, §4.3.2, §4.5), `mode2_abi_agnostic_layer.md`,
`mode2_gr_forwarding.md`, `mode2_compute_forwarding.md`, `mode2_forwarding_model.md`,
`mode2_dataplane_architecture.md`, `mode2_address_table.md`; the decisions memo
`mode2_rewrite_design_decisions` (the 15 settled decisions).

All bare `crates/…` cites are this repo at the current head. C-repo cites are prefixed
`C:`. Claims I could not verify from source are marked **ASSUMPTION — verify**.

---

## 0. The one invariant that bounds the whole plane

**We do NOT emulate GR/compute.** `C: mode2_gr_forwarding.md:26` — *"FORBIDDEN — a
GR/compute METHOD emulator (execute GR pushbuffers in QEMU). THIS is the throwaway +
non-performant thing. Never build it."* GR's golden context is produced by FECS/GPCCS
microcode on real silicon; it is a hard boundary. The entire approach is to **forward**
real GR/compute/CE pushbuffers to a real host GPU while **faking everything the guest
kernel expects to see**.

Therefore "the execution plane in the core" is an **orchestration layer**, not an engine.
Its four jobs, and nothing more:

1. **Context lifecycle** — recognize the guest allocating a GR/CE/NVENC object on a
   channel; forward the *Case-1* allocs so the host kernel-RM builds and promotes its
   OWN context (golden ctx included); ack the *Case-2* GSP-internal controls the guest
   still issues (`PROMOTE_CTX`, `GET_CTX_BUFFER_INFO`) — they are re-derived host-side.
2. **Pushbuffer / method model** — decode *just enough* of the guest's GPFIFO/pushbuffer
   to (a) forward the real work and (b) capture the address / semaphore / invalidate
   facts the address and completion planes need. Never full method emulation; the
   userspace ring stays opaque and passes through.
3. **Engine abstraction** — GR (compute + graphics), CE, NVENC/NVDEC are *instances* of
   one `Engine` seam bound to a `Channel`/TSG in the RmGraph, so a new engine is a new
   `impl`, not a new subsystem.
4. **Semaphore / completion** — recognize how GR/CE work signals done (real host
   semaphore advance on a shared page; the interrupt/os-event path), and feed the
   existing per-`Proc` `CompletionQueue` correctly, keyed per-`Vas` / per-`Channel`.

This bound is *why the plane is small enough to design into the core now*: it is a
translator and a router, not a compute engine.

---

## 1. Completeness audit — is the core complete for the ~20 apps?

### 1.1 Verdict

> **★★ SUPERSEDED (flagged 2026-07-27, doc audit) — this "No" predates the M3 batches that
> then landed, and its companion gate has said so since 2026-07-24 while this file was never
> amended.** `core_completeness_gate.md`'s rubric note reads: *"`execution_plane.md` §1 (whose
> §1.1 'No' verdict predates the M3 batches that then landed — commits `6f425d2`..`c5489a1`
> **built most of what §1.1 listed as missing**)"*. All four `NEXT` items at the end of this
> section have since landed.
>
> **Two of §1.1's specific "missing" claims are verifiably false now:** the `EngineClass{Gr,Ce,
> Other}` coarse tag it names as the limit no longer exists (replaced by `EngineKind`; the only
> surviving mention is in the rustdoc of `crates/kayfabe-arch/src/ids.rs::EngineKind`, a line
> describing the replacement), and the engine-object forward is idempotent
> (`tests/tests/engine_context.rs::replayed_engine_object_alloc_forwards_exactly_one_host_object`).
>
> **The verdict is left standing rather than flipped**, because *"is the core complete?"* is a
> judgement this audit is not entitled to re-take — and because the reasoning below is the
> record of what the gate was measuring against. But **do not read it as current**: consult
> `core_completeness_gate.md` (itself pin-expired — see its own banner) and re-resolve any
> capability claim against the tree.
>
> ★ The instructive part is that the correction *was* written down, promptly, in the right
> place — and in the **other** document. A note that says "the doc I am citing is out of date"
> repairs the reader's path only if the reader arrives via that doc.

**No — and it is not supposed to be yet.** The current Rust core is the **control /
object / address / isolation / completion-*delivery* spine**, and it is genuinely
complete for that spine (RmGraph source-of-truth + order-independent projections
`crates/kayfabe-core/src/rmgraph.rs`, `project.rs`; per-`Vas` address table
`crates/kayfabe-mmu`; per-`Proc` GPA arenas + isolates `crates/kayfabe-core/src/gpu.rs`;
per-`Proc` `CompletionQueue` + `DeliveryPlane` `crates/kayfabe-completion`; the doorbell
demux + first-touch host-channel materialization `crates/kayfabe-fwd/src/lib.rs`). What it
has of the *execution* plane is a **thin routing skeleton**: `EngineClass{Gr,Ce,Other}`
as a leaf enum (`crates/kayfabe-arch/src/ids.rs` — [symbol gone: `EngineClass` — was cited
here as the leaf routing enum; the surviving successor is
`crates/kayfabe-arch/src/ids.rs::EngineKind`, see the banner above]), `RmBackend` with raw
`alloc_channel`/`schedule`/`ring_doorbell` verbs (`crates/kayfabe-isolate/src/lib.rs`), and
a `Channel.host_channel` first-touch materialization in `handle_doorbell`. There is:

- **no `Engine` abstraction** (GR/CE/NVENC as engine instances under a channel);
- **no GR context lifecycle** (compute/graphics object alloc, PROMOTE_CTX / golden-ctx
  handling, Case-1 forward vs Case-2 ack);
- **no pushbuffer / method parser** — the "ONE parser" (SEM_EXECUTE / MEM_OP /
  LAUNCH_DMA) is a *documented skeleton* only (`crates/kayfabe-fwd/src/lib.rs` "Ports here
  later"; `crates/kayfabe-gsp` is a 31-line `BootPhase` enum);
- **no CE-PT-write capture feed** (the `Vas.pt_pages` field exists,
  `crates/kayfabe-core/src/gpu.rs`, but nothing populates it — #13's populate source);
- **no semaphore/completion *observation*** wiring (the `CompletionQueue.observe` sink
  exists but nothing on the exec path calls it from a real host sema advance);
- **no engine-object / Case-1 shadow-forward table**, no NVENC session, no
  display/present adapter seam.

So the core can *route* a doorbell to a host channel and back a VA range, but it cannot
yet *run a kernel*: the object that would make the channel a compute channel, the
promote/golden handshake that lets the host build its context, the pushbuffer decode that
captures the launch's semaphore, and the completion observation that wakes the guest are
all absent. That is exactly the surface this document designs.

### 1.2 The app-requirement union (what the plane must eventually cover)

The 22-app Mode-1 matrix (`C: tests/perf/run_matrix.sh`, `run_graphics.sh`) and the
Mode-2 bring-up (#12/#13/#14) exercised the following. **Honesty note:** the 22-app
matrix ran on **Mode-1** (host driver owned all scheduling/VAS/sema), so it names engines
at the *workload* level; the concrete channel/pushbuffer/semaphore mechanics come from the
**Mode-2** debugging, which reverse-engineered exactly what these same workloads demand of
an execution plane. Marked `[M1 API-level]` vs `[M2 proven on bench]` below.

| App class (named apps) | Engines | Context / channel types | Semaphore / completion pattern | Display | Core HAS | Core MISSING |
|---|---|---|---|---|---|---|
| **CUDA compute** (stream, reduce, nbody, blackscholes, mandelbrot, conv2d, sgemm-cublas, fft-cufft, sha256, gpu-burn) | GR-compute `[M1]` + CE `[M1]` | compute channel + GR ctx (obj `AMPERE_COMPUTE_B`), CE copy channel | shared-page sema pool poll `[M2 #12]`; CE method `SEM_RELEASE` `[M2 #13]` | none | RmGraph, per-`Vas` table, doorbell demux, per-`Proc` arena/isolate/completion-queue | `Engine`, GR-ctx lifecycle, pushbuffer parser, CE-PT-capture, sema *observe* wiring |
| **LLM** (llama.cpp Qwen2.5-7B, `-ngl 99`) | GR-compute + CE (weights/KV upload) `[M1]` | same as compute; multi-iter reuse | shared-page sema; multi-iter remap `[M2 #13]` | none | (as above) | (as above) + #13 512M-leaf walker + xfer_none guard |
| **PyTorch / AI** (matmul fp32/fp16-TC, ResNet-50 infer/AMP/train, ViT, BERT) | GR-compute incl. **Tensor Cores** (a path *within* GR, not a new engine) + CE `[M1]` | compute channel + GR ctx; UVM VAS for managed mem | shared-page sema | none | (as above) | (as above) + UVM residency pass-through |
| **HPC** (fft-cufft, sgemm-cublas, nbody) | GR-compute + CE `[M1]` | compute + copy | shared-page sema | none | (as above) | (as above) |
| **Multi-process** (2×/3×/4× `cup8`) `[M2 #14]` | GR-compute + CE per proc | per-proc GR ctx + TSG; **identical guest VAs + identical handles** across procs | per-proc completion; **the loser's GR channel FAULT_PDE on host** = the #14 root cause | none | per-`Proc` planes, per-`Vas` host separation (the #14 fix's *seat*) | the exec-plane wiring that *publishes* each proc's VAs into its OWN host GR VAS (§4) |
| **Vulkan / GL graphics** (Vulkan enumerate, vkpeak compute, headless GL render `egl_offscreen.c`) | GR-**graphics** (raster) + GR-compute + CE `[M1]` | **graphics** channel + GR-graphics ctx; extra RM classes (`NV01_EVENT`, `NV_SEMAPHORE_SURFACE`, `NV_CONTEXT_DMA`) | sema; present-complete → synthetic vblank `[M2]` | **present** (only this class) | `EngineClass::Gr` already spans graphics | graphics-ctx object recognition; **present/display adapter** |
| **NVENC video** (H.264/HEVC) `[M2 #99/#101]` | **NVENC = separate engine + session**; CE for input-surface copies | `NVENC_SW_SESSION` + encoder class `NVC7B7` + `NV01_CONTEXT_DMA` | **mapped coherent fence** the GPU writes, read worker-side with NO syscall `[M2 #101]` | none | nothing engine-specific | NVENC as an `Engine` instance + session object |
| **DMA / UVM memory** | CE (the copy IS the workload) `[M1]`; UVM residency | copy channel; UVM VAS + `SET_PAGE_DIRECTORY` | CE method sema; #13 CE-PT-write | none | per-`Vas` table + `SetPageDir` fact in RmGraph | CE-PT-write capture; UVM managed-mem pass-through |
| **NVDEC video** (cuvid) | NVDEC `[UNPROVEN]` | decode session | (unproven) | none | nothing | **gap — never proven** (excluded from matrix, broken on host too) |

**The union = the execution-plane surface to design:**

- **Engines:** GR-compute (incl. Tensor-Core path), GR-graphics, CE, NVENC. *(NVDEC and
  AV1 are honest gaps, deferred.)*
- **Context types:** compute GR ctx, graphics GR ctx, CE copy context, NVENC session —
  all bound to a `Channel`/TSG that names its `hVASpace`/`hContextShare` (RmGraph facts).
- **Pushbuffer methods that matter:** `SET_OBJECT` (engine bind), CE `LAUNCH_DMA` /
  `MEMSET` (dsts + PT-write capture), `SEM_RELEASE` / `SET_SEMAPHORE_A/B` / finishPayload
  (completion), `MEM_OP` + `MMU_TLB_INVALIDATE` (invalidate transport). Everything else
  in a userspace ring is opaque and forwarded verbatim.
- **Completion patterns (all five must be expressible):** (a) shared-page semaphore-pool
  poll, (b) GSP `finishPayload` (SYSMEM *and* BAR1/VIDMEM apertures), (c) CE-method sema
  release, (d) interrupt / os-event re-post via the single SWGEN0 edge, (e) mapped
  coherent fence read (NVENC).
- **Scheduling:** per-`(client,TSG)` `GPFIFO_SCHEDULE` (the #12 lesson — generalized here
  to per-`Proc` `ExecPlane`).
- **Display:** present via `PRIME_HANDLE_TO_FD` only, **never** NVKMS — a *later adapter*,
  but the seam must exist.

---

## 2. The execution-plane model — GR as the driving case

Design stance is unchanged from the settled architecture (decisions #1/#2): the **core
owns algorithms** (context-lifecycle state machine, pushbuffer-decode loop, completion
policy, forward orchestration); the **`Arch` owns encodings** (engine class IDs, method
encodings, USERD/RAMFC offsets, context-buffer formats); the **`RmBackend` owns host
forwarding** (the unprivileged RM verbs). Per the address-table directive: forward-populate
only, MISS = FAULT, no reverse-resolve.

The plane adds **four new seams** and **one new core module**. All four seams are trait
methods an `Arch`/`RmBackend` fills — §3 proves adding an engine-for-an-arch is `impl`s
with zero core edits.

### 2.1 The `Engine` abstraction

An **engine instance** is what makes a `Channel` do work: a GR-compute object, a
GR-graphics object, a CE copy object, or an NVENC session, allocated *on* a channel that
names its VAS via the RmGraph. In the RmGraph this already appears as
`ObjectKind::EngineObject { engine }` (`crates/kayfabe-arch/src/lib.rs::ObjectKind::EngineObject`) and
`Channel { engine }` — the graph *shape* already carries it. What is missing is the
*behavioral* seam: given an engine object on a channel, how the core (a) forwards its
alloc (Case-1) so the host builds the engine's context, (b) recognizes its pushbuffer
methods, (c) attributes its completions.

**Core-invariant vs per-arch split (the anti-dup line):**

| Concern | Core-invariant (owns the algorithm) | Per-arch (`Arch`/ABI owns the encoding) |
|---|---|---|
| Which engine a channel targets | `EngineClass` routing; one exec loop | `Arch::classify(class) -> EngineObject{engine}` (class IDs) |
| "This object makes a compute/graphics/CE/enc context" | the lifecycle state machine (§2.2) | `Arch::engine_of_object(class) -> Option<EngineKind>` |
| How its work is described | the decode loop (§2.3) | `Arch::pushbuffer()` — the `PushbufferAbi` (method encodings) |
| How its completion signals | the completion tie-in (§2.4) | which method is a sema-release; USERD/sema field offsets |
| How its context is promoted | Case-1 forward, Case-2 ack (§2.5) | which controls are Case-2 (`PROMOTE_CTX`, `GET_CTX_BUFFER_INFO`) |

The core NEVER names GR/CE/NVENC-Ampere-vs-Hopper differences; it programs against
`EngineKind` (a small core enum: `GrCompute | GrGraphics | Ce | NvEnc | NvDec`) and the
`Arch` seams. A new engine for an existing arch = a new `EngineKind` arm + the arch's
class-ID row + its method rows; a new *arch* for an existing engine = new class-ID/method
rows behind the same trait. Zero core edits either way.

> **Why an enum, not a `trait Engine` object per engine.** The engines do not have
> divergent *core behavior* — they all: get a Case-1 alloc forwarded, get their pushbuffer
> decoded by the same loop, and signal via a sema. Their differences are entirely
> *encodings* (class IDs, method IDs, sema offsets), which live in `Arch`. Putting engines
> behind `Box<dyn Engine>` in the core would smuggle per-engine *logic* into the core and
> re-create the C's weave. So `EngineKind` is a routing tag; the variation is in the ABI.
> (If a future engine needs genuinely different core orchestration — e.g. a video engine
> whose completion is a mapped fence not a sema — that becomes one new arm in the
> completion tie-in, §2.4, still core-owned, still driven by an `Arch` seam.)

### 2.2 GR context lifecycle

The proven insight the whole lifecycle rests on (`C: mode2_compute_forwarding.md:397-410`,
`mode2_dataplane_architecture.md:416-421`): **when we forward the GR object alloc, the host
kernel-RM allocates its OWN GR context buffers and issues its OWN PROMOTE_CTX entirely
in-kernel** — including the FECS golden-context capture, on real silicon. The guest never
runs its own GR engine, so the guest's golden buffer content can be garbage; the guest only
needs the *completion* the driver's 4-second poll waits on. And because the host self-maps
its GR ctx buffers at the SAME deterministic GR VAs the guest uses (`0x120020000…`), the
guest never needs to read GR-ctx-buffer *content*.

So the lifecycle the core models is deliberately shallow:

```text
guest RM_ALLOC(compute/graphics object on channel C)          [RmGraph: EngineObject]
   └─ core: this channel is now a GR context of EngineKind::GrCompute|GrGraphics
   └─ fwd (Case-1): forward the alloc verbatim through C's Proc isolate
                    → host kernel-RM builds host GR ctx buffers, self-promotes,
                      FECS captures the golden image on host silicon.
guest GSP_RM_CONTROL(PROMOTE_CTX 0x2080012b) / GET_CTX_BUFFER_INFO 0x20801219
   └─ core (Case-2): ACK ONLY — the host already did it (§2.5). Never replay
                     (unprivileged → 0x1b = "wrong layer", not "gain privilege").
guest polls the golden-capture completion (a GSP-event / notifier)
   └─ completion plane: signal it (system-Proc forge, kernel-internal, content-irrelevant).
```

**What the core tracks vs what the host owns:**

- **Core tracks:** which `Channel` carries which `EngineKind` (a field on `Channel`); the
  binding `Channel → Vas (PDB)` (already resolved in `project.rs`); that the object's
  Case-1 alloc has been forwarded (so re-sends are idempotent); the golden-capture
  completion the guest is waiting on (routed to the `system` `Proc`).
- **Host owns:** the GR context buffers, their physical backing, the golden image, and the
  real PROMOTE_CTX. The core neither stores nor forwards any host-physical address — this
  is what keeps every host op unprivileged (decision #9 boundary-2).

The lifecycle binds to the `RmGraph`/`Vas`/`Channel` spine with **zero new identity**: the
GR context IS the `(Channel, Vas)` pair the graph already derives. There is no "GR context
object" in the core beyond a tag on the channel and the fact that its object-alloc was
forwarded. (Two proven arch details the *ABI adapter* carries so the host build succeeds,
NOT the core: strip `IS_EXTERNALLY_OWNED` from the forwarded VASpace alloc params, and
never set an explicit `hVASpace` on a TSG channel — `C: mode2_compute_forwarding.md:630-639`.
These are Case-1 param *lowering* details, Axis-A, quarantined to `kayfabe-abi`.)

### 2.3 Pushbuffer / method model — the parser IS the address-table populator

This is the heart of the plane and the sharpest anti-emulation boundary. The GPFIFO
pushbuffer parser decodes **just enough** to forward the work and to extract three fact
kinds — it is emphatically **not** a method interpreter (`C: mode2_address_table.md`;
`mode2_dataplane_architecture.md:516-529` documents the rejected QEMU CE-emulation path).

**What it decodes (and nothing else):**

| Method (per-arch encoding) | Fact extracted | Consumer |
|---|---|---|
| `SET_OBJECT` | which engine object the subsequent methods target | routing (confirm `EngineKind`) |
| CE `LAUNCH_DMA` / `MEMSET` / `COPY` | destination address(es); whether dst is a PT page | `kayfabe-mmu` CE-PT-write capture (#13) |
| `SEM_RELEASE` / `SET_SEMAPHORE_A/B` + payload / finishPayload | the completion (sema address in a VAS + target payload) | `kayfabe-completion` observe (§2.4) |
| `MEM_OP_A/C/D` with `OPERATION = MMU_TLB_INVALIDATE (0x9)` | the invalidated PDB + membar/ack | `kayfabe-mmu` invalidate (address-table §5) |

**Everything else is opaque and passes through.** The two invalidate transports
(`INVALIDATE_TLB` RPC and the `MEM_OP` pushbuffer method) both carry the PDB and a membar;
a membar is a hard barrier the interpreter honors before advancing
(`C: mode2_address_table.md:132-162`).

**★ The two populate sources are co-equal (grounded, load-bearing).** On the
GSP-emulated compute path, the classic invalidate transports measured **0 occurrences**
and `DMA_FILL_PTE_MEM` = 0 (`C: mode2_address_table.md:114-130`; arch lesson L3). Compute
leaf PTEs are published **exclusively through the CE page-table-write data plane** — the
kernel-RM CeUtils writes COMPUTE-VAS page tables via physical CE copies into PD pages, and
the address table is forward-populated from the **observed CE PT-write, attributed by
destination-FB-address → owning PDB, latched at the CE release semaphore** (#13,
`C: b83d0b4`). So the parser feeds `kayfabe-mmu` from **two** equal sources: bind-time RPC
bindings *and* observed CE PT-writes. The `Vas.pt_pages` set already exists for exactly
this (`crates/kayfabe-core/src/gpu.rs`).

**The opaque-user-ring fast path (ties to trap-min, decision #6).** Userspace channels are
non-privileged: they cannot issue `MMU_TLB_INVALIDATE` (privileged) and their memory maps
are kernel-mediated (observed at bind time, not in the ring). So a **userspace ring never
carries a fact the core must extract** (`C: mode2_address_table.md:229-242`, verified
safe) — it is passed through as shared physical pages, the host GPU fetches it directly, no
per-submit parse. **The parser runs only where the core is already the mediator:** on the
kernel/CeUtils/scrubber channels (the `system` `Proc`) and at the CE-PT-write capture
point. This is what keeps the hot path at parity (`C: mode2_baremetal_32` — zero Mode-2
overhead bare-metal).

**Where the decode is Arch vs core:** the loop (walk GPFIFO entries → for each pushbuffer,
walk methods → dispatch on the decoded method kind) is **core** (`kayfabe-fwd`, one parser).
The method *encodings* (how a raw method word decodes into `{kind, args}`, which method ID
is `SEM_RELEASE`, the sema-field offsets) are **`Arch::pushbuffer() -> &dyn PushbufferAbi`**.
The core sees only a `PushMethod` enum (`SetObject | CeLaunchDma{dst,..} | SemRelease{addr,
payload} | TlbInvalidate{pdb,membar} | Opaque`).

### 2.4 Semaphore / completion — tying real host completion to the `CompletionQueue`

The completion plane already exists and is per-`Proc` (`crates/kayfabe-completion`,
`CompletionQueue::observe → DeliveryPlane::try_post/on_poll`). The execution plane's job is
to **call `observe` from a real completion**, keyed correctly. There are two shapes,
reconciled with decision #7's 2026-07-24 resolution:

1. **Guest userspace busy-polls a shared semaphore page** (the dominant compute path). The
   completion is a **real host-GPU write** into a shared physical page mapped at the right
   GPA (data-plane passthrough, L4/L5) — the guest polls the value directly, **no core
   mediation, no `observe` call needed** for the value itself. The exec plane's only job is
   to have published that sema page into the channel's VAS correctly (per-`Vas`, §4) so the
   host write lands where the guest reads. This is why decision #7's passthrough-sema idea
   *helps* the busy-poll path.

2. **Guest kernel waits on an interrupt / os-event** (`MC_SERVICE_INTERRUPTS`, the
   golden-capture wait, blocking sync). Here the core must `observe` the completion and let
   the per-`Proc` `CompletionQueue` + poll-driven re-delivery raise the single SWGEN0 edge
   (`crates/kayfabe-fwd::deliver_completions`/`poll_completions`, §4.3.2). This is the path
   the per-`Proc` queue fixes structurally.

**★ Reconciliation with the #14 disambiguation (decision #7, RESOLVED 2026-07-24,
`C: 6de85e7`).** The #14 loser-hang was proven by a **real host Xid** to be an **EXECUTION**
fault, not a delivery fault: the loser's GR channel took a `FAULT_PDE VIRT_WRITE` on the
host GPU because its (identical) guest VAs were **never published into its OWN host GR
VAS** — the emulator's FB-shadow had them, but the host page tables did not, so the host
GPU faulted past the shared VA prefix. Its completion legitimately never existed because
the work faulted. **Consequences for this plane, explicit:**

- The **load-bearing exec-plane invariant is per-`Vas` host publication**: every VA a
  channel's work touches must be forward-populated into *that channel's Vas's own host
  VAS* before the doorbell rings. The core already keys the address table per-`Vas` by PDB
  (`crates/kayfabe-mmu`, `crates/kayfabe-fwd::publish_backing` uses the Vas's own `host_vas`) —
  the exec plane must ensure the channel's **working set** is published there, not merely
  in a shadow. §4 makes this the ring-gate.
- The `CompletionQueue` is therefore **adequate and can stay simple** — it was never the
  #14 root cause. Do not over-build it.
- MISS = FAULT is the right posture here too: if a channel's working-set VA is unpublished
  at ring time, that is a loud fault (`FwdFault`), never a content-pick that guesses the
  wrong host VAS (the exact confused-deputy the C's `bar1_wpg` MRU caused — designed out,
  `crates/kayfabe-fwd::FwdFault::UnknownVchid` comment).

**One completion detail per engine (the `EngineKind`-arm in the tie-in):** CE/GR signal via
a sema-release the parser extracts; NVENC signals via a **mapped coherent fence** read
worker-side (`C: nvenc_101`) — expressed as a distinct arm that `observe`s when the mapped
value advances. This is the one place engine variety touches the completion tie-in, and it
is a small core enum arm driven by an `Arch`/`RmBackend` fact, not a new subsystem.

### 2.5 Forward-to-host orchestration — the Case-1 / Case-2 split

The forwarding model (`C: mode2_forwarding_model.md`, lesson L2) partitions everything the
guest issues:

- **Case 1 — the RPC *is* the userspace op.** `GSP_RM_ALLOC` (channel, engine object,
  VASpace, TSG, ctxshare) and forwardable `GSP_RM_CONTROL`s re-issue ~1:1 on the host
  through the owning `Proc`'s isolate. This is what makes the host build the real channel +
  compute object and self-promote its GR context. Handled by a **Case-1 shadow-forward**
  path in `kayfabe-fwd` (documented skeleton today).
- **Case 2 — GSP-internal / ROUTE_TO_PHYSICAL controls with no userspace equivalent**
  (`PROMOTE_CTX 0x2080012b`, `GET_CTX_BUFFER_INFO 0x20801219`, `GET_SURFACE_PHYS_ATTR`,
  profiler class `0xc076`). Their effect is *already achieved* by Case-1 forwarding.
  Correct handling: **ack the guest, do nothing on the host.** Replaying one on an
  unprivileged isolate returns `0x1b` = **wrong layer** (`RmError::InsufficientPermissions`,
  `crates/kayfabe-isolate/src/lib.rs` — already typed with exactly this meaning). The core
  carries a **Case-2 ack-only table** (which controls are ack-only), an Axis-A codegen'd
  set consumed by a core routing decision.

The concrete `RmBackend` verbs the orchestration needs to reproduce GR/CE intent — most
already exist as intent verbs in `crates/kayfabe-isolate/src/lib.rs`
(`alloc_vaspace`/`alloc_channel`/`schedule`/`map_gpu_va`/`ring_doorbell`) plus the generic
`alloc`/`control`/`free` for Case-1 passthrough. §3.2 lists the small additions.

### 2.6 Graphics + NVENC — instances, not bolt-ons

- **GR-graphics** is the **same `Engine`** as GR-compute with `EngineKind::GrGraphics`: same
  Case-1 alloc-forward, same pushbuffer decode loop, same sema completion. The *only* added
  surface is a **display/present adapter** — guest render-target → `PRIME_HANDLE_TO_FD`
  dma-buf → VMM present → host present-complete fed back as a synthetic vblank
  (`C: modeset_strategy.md`, `present_path_b_done.md`). This is a **later adapter behind a
  new `Vmm`/`RmBackend` seam** (`present(dmabuf) -> vblank`), NEVER NVKMS forwarding. The
  seam must *exist* in the design so graphics is not a re-architecture; the adapter is
  deferred (§4).
- **NVENC** is another `Engine` (`EngineKind::NvEnc`): a session object
  (`NVENC_SW_SESSION` + encoder class) allocated on a channel, forwarded Case-1, its
  input-surface copies riding CE, its completion the mapped-fence arm (§2.4). No new plane —
  a new `EngineKind` arm + its ABI rows.

Both prove the anti-bolt-on property: they are `impl`s and enum arms over the *same* four
seams, not new subsystems.

---

## 3. The seams — what an Arch / RmBackend implements (the anti-bolt-on proof)

Adding "GR (or CE, or NVENC) for a new arch" must be new `impl`s with **zero edits to any
logic crate** (repo rule 2). Here is the exact added surface.

### 3.1 On `Arch` (in `crates/kayfabe-arch`) — encodings only

```rust
/// A pushbuffer method, decoded into core terms (no raw bits). The core dispatches on
/// this; the Arch produces it. Mirrors PteDecode's "no raw bits in the core" discipline.
pub enum PushMethod {
    SetObject { class: ClassId },
    CeLaunchDma { dst: GpuVa, len: u64, dst_is_virtual: bool }, // #13 capture input
    SemRelease { addr: GpuVa, payload: u64 },                   // completion extract
    TlbInvalidate { pdb: Pdb, membar: bool },                   // invalidate transport
    Opaque,                                                     // pass through, do not act
}

/// Axis-B seam: the pushbuffer/method + engine encodings for one generation.
pub trait PushbufferAbi {
    /// Decode one method word (at `subchannel`/`method` offsets this arch defines) into
    /// core terms. Anything this arch does not model → `PushMethod::Opaque` (never guessed).
    fn decode_method(&self, header: u32, args: &[u32]) -> PushMethod;
    /// Iterate the GPFIFO entries of a pushbuffer region (entry stride/format per arch).
    fn gpfifo_entries<'a>(&self, ring: &'a [u8]) -> Box<dyn Iterator<Item = PushRange> + 'a>;
}

pub trait Arch {                 // additions to the existing trait
    /// Which engine (if any) an *object* class denotes — the §2.1 EngineKind mapping.
    fn engine_of_object(&self, class: ClassId) -> Option<EngineKind>;
    /// Is this control a Case-2 GSP-internal ack-only? (PROMOTE_CTX, GET_CTX_BUFFER_INFO…)
    fn is_case2_control(&self, cmd: ControlCmd) -> bool;
    /// The pushbuffer/method ABI for this generation.
    fn pushbuffer(&self) -> &dyn PushbufferAbi;
    /// Byte offset of a channel's completion-sema in USERD/the release surface, if the
    /// completion tie-in needs it (else the parser's SemRelease.addr suffices).
    // (folds into the existing UserdModel where natural)
}
```

`EngineKind` is a new core enum in `kayfabe-arch::ids` alongside `EngineClass` (or replaces
it — `EngineClass{Gr,Ce,Other}` is too coarse for NVENC/graphics; `EngineKind{GrCompute,
GrGraphics,Ce,NvEnc,NvDec,Other}` is the refinement). The `MockArch` "Mockingbird"
generation implements all of the above with deliberately-fake encodings — the standing
proof that no core code assumes a real NVIDIA layout.

### 3.2 On `RmBackend` (in `crates/kayfabe-isolate`) — host forwarding verbs

Most verbs exist. The execution plane adds, at most:

```rust
pub trait RmBackend {            // additions
    /// Allocate an engine object (compute/graphics/CE/NVENC) on a host channel — the
    /// Case-1 forward that makes the host build + self-promote its context. `params` is
    /// the ABI-lowered alloc blob (EXTERNALLY_OWNED already stripped, etc. — Axis A).
    fn alloc_engine_object(&mut self, chan: HostHandle, class: ClassId, params: &[u8])
        -> Result<HostHandle, RmError>;
    // alloc_channel / schedule / map_gpu_va / ring_doorbell / control / free: ALREADY PRESENT.
}
```

Note `alloc_engine_object` is *almost* the existing generic `alloc(parent, class, params)`
with `parent = chan`; it is called out only to name the intent. Case-1 shadow-forwarding of
allocs/controls uses the existing generic `alloc`/`control` verbs — the plane adds routing
logic (Case-1 vs Case-2), not new host reach. **This is the anti-bolt-on payoff:** the
*host boundary* (the unprivileged verb surface) does not grow to add an engine; only the
core's routing table and the arch's encoding rows do.

### 3.3 The display/present seam (deferred adapter, seam present now)

```rust
// On Vmm (or a small dedicated Present trait the graphics adapter implements):
fn present(&mut self, dmabuf: RamHandle, meta: FbMeta) -> Result<(), VmmError>;
// completion fed back as a synthetic vblank via the existing CoreEvent/defer path.
```

Named here so graphics is an adapter fill, not a re-architecture; unbuilt until the
graphics milestone.

### 3.4 Single-arch is N = 1

There is exactly one real arch on the bench (GA10x). The seams above are still worth their
weight at N = 1 for two reasons the project already paid to learn: (1) the `MockArch` is a
*second* implementation (decision #1's rule-of-three-in-spirit — the mock forces the seam
honest even before a second silicon arch), and (2) the seam is what keeps engine *logic*
out of the core, which is the specific thing that let the C's execution plane rot into a
god-object. The design targets the seams against the studied Turing/Ampere/Hopper deltas
(decision #1) but *claims* only GA10x + the tested driver version.

---

## 4. Now vs later — the minimal execution-plane CORE surface to build next

The milestone that makes "build a real arch" possible is the **compute-GR + CE path**:
enough execution plane to run the CUDA/LLM/PyTorch compute apps single-process, then
2×/3×/4× concurrent (the #14 raison d'être). Sequence:

**NEXT (the minimal core surface — GPU-free, mock-driven, then bench):**

1. **`EngineKind` + the `Arch` engine/method seams** (§3.1) with `MockArch` filling them.
   Pure core + mock; no GPU. *Unblocks everything below.*
2. **GR/CE context lifecycle** (§2.2/§2.5): Case-1 forward of channel + engine-object
   alloc; Case-2 ack-only table; golden-capture completion routed to `system`. Mock
   `RmBackend` records the forwarded allocs; a scripted golden-capture completion asserts
   the guest's wait is satisfied.
3. **The ONE pushbuffer parser** (§2.3) over `PushbufferAbi`, feeding: `kayfabe-mmu`
   CE-PT-write capture (into the existing `Vas.pt_pages`) and `kayfabe-completion.observe`
   (from `SemRelease`), and honoring `TlbInvalidate` membars. Opaque user ring passes
   through. Mock-driven: a scripted pushbuffer → assert the table populated + completion
   observed (§5).
4. **Per-`Proc` `ExecPlane` working-set publication + ring-gate** (§2.4's #14 fix): before
   `handle_doorbell` rings, the channel's working set is forward-populated into its Vas's
   OWN host VAS; an unpublished VA at ring time is a loud `FwdFault`. This is the proven
   #14 load-bearing fix (decision #7, `C: 6de85e7`), and the structure
   (`crates/kayfabe-core/src/gpu.rs::ExecPlane`, per-`Vas` `host_vas`) is already in place.

That set runs the compute/LLM/PyTorch apps and is the first thing a real `impl Arch for
<GA10x>` fills.

**LATER (adapters behind seams that exist now):**

- **Graphics display/present** (§2.6/§3.3) — GR-graphics already routes; only the present
  adapter is new.
- **NVENC** (§2.6) — a new `EngineKind` arm + session object + mapped-fence completion arm.
- **UVM managed-memory pass-through** — `cudaMallocManaged` → host `cudaMallocManaged`,
  host UVM owns residency (`C: mode2_uvm_residency.md`). Orthogonal to the compute path
  (explicit device mem forwards without UVM); sequence after the compute milestone.
- **NVDEC / AV1** — honest gaps; unproven even in Mode-1. Do not claim.

**Biggest risk + first bench validation.** The single biggest risk is that the
pushbuffer parser + CE-PT-write capture do not, together, publish the compute working set
into the host VAS *exactly* at the commit points the guest relies on (the #13/#14 seam —
"the leaf is filled *then* linked a push later," so capture decodes dirtied pages directly,
not by a root walk). **First bench validation once an arch exists:** reproduce
`cup3`/`cup8` single-process byte-exact (proves forward-and-execute end to end — the host
runs the real shader and writes the sema the guest polls, un-forgeable because QEMU never
parses the compute ring), then 2× concurrent `cup8` both rc=0 (proves the #14 per-`Vas`
publication fix). Differential vs the C oracle for single-process; vs host-native goldens
for output bytes (`C: mode2_rust_rewrite_architecture.md` §4.5 acceptance ladder).

---

## 5. Testability — deterministic, GPU-free (decision #15: over-test invariants freely)

The whole plane is a pure state machine over guest-supplied bytes, so it is
**deterministically testable without a GPU** — the same property the core is judged on.
The test shape (matching the existing suite in `tests/tests/` and the mocks in
`crates/kayfabe-mocks`):

- **`MockArch` scripted pushbuffer → assert facts.** Feed a byte-exact fake pushbuffer
  (Mockingbird encodings) through the parser; assert: (a) the address table is
  forward-populated for the CE `LAUNCH_DMA` dsts (into the right `Vas` by PDB), (b) the
  `SemRelease` was `observe`d on the owning `Proc`'s `CompletionQueue`, (c) a
  `TlbInvalidate` membar was honored (no advance before refresh), (d) an `Opaque` method
  changed no core state. These are **invariant/contract** tests, not internal-state pins
  (decision #15).
- **`MockRmBackend` records the Case-1 forward, refuses the Case-2 replay.** Assert the
  engine-object alloc was forwarded through the *owning* isolate; assert a Case-2 control
  routed to ack-only never reached the backend (and if wrongly replayed, the mock returns
  `InsufficientPermissions` — "wrong layer" — which the test treats as a design error).
- **The #14 exec-plane regression (`t14_*`), meanly.** Two `Proc`s with **identical guest
  VAs and identical handles** (the mock MUST reproduce this, per repo rule) each publish
  their working set; assert each lands in its OWN host VAS (distinct `HostHandle`s from
  distinct isolates) and that an unpublished VA at ring time is a loud `FwdFault`, never a
  cross-proc content-pick. This encodes the proven #14 root cause as a permanent guard.
- **The #13 regression (`t13_*`).** A CE PT-write to a 512M-leaf PT page, captured at the
  release sema, populates the compute VAS; a multi-iter remap re-resolves. Guards the
  512M-leaf/xfer-none lessons at the plane level.
- **The #12 regression (`t12_*`).** A second context's fresh GR TSG is scheduled on its own
  per-`Proc` `ExecPlane` (nothing one-shot); its channel rings on-runlist. Guards the
  CTX2-off-runlist class.
- **Fuzz + soak** (existing `fuzz_rmgraph_invariants`, `soak_llm_like` shape): fuzz the
  pushbuffer byte stream (hostile methods, truncated rings, bogus sema addrs) — every path
  is either a decoded fact or a loud fault, never a panic, never a silent guess. Soak a
  sustained LLM-like submit/complete loop and assert no completion is lost and the address
  table does not grow unbounded (reclaim deferred).

All of it runs in-process, in milliseconds, no GPU/OS/socket — the safety net the rewrite
is built on.

---

## 6. Summary

- **Completeness verdict:** the core has the object/address/isolation/completion-*delivery*
  spine (complete for that spine); it has **none of the engine execution/forwarding**. The
  app-requirement union to design = engines {GR-compute incl. Tensor-Core path, GR-graphics,
  CE, NVENC}; the compute/graphics/CE/enc context lifecycle; the four pushbuffer method
  kinds (`SET_OBJECT`, CE `LAUNCH_DMA`, `SEM_RELEASE`, `MMU_TLB_INVALIDATE`) with the
  opaque user ring passed through; five completion patterns; per-`(client,TSG)` scheduling;
  and present-via-PRIME (later). NVDEC/AV1 are honest gaps.
- **The abstractions designed:** an `Engine` = `EngineKind` routing tag + `Arch` encoding
  seams (not `dyn Engine` — engines differ only in encodings); a shallow GR **context
  lifecycle** (Case-1 forward → host self-promotes its own golden ctx; Case-2 ack-only)
  bound to the existing `(Channel, Vas)` graph identity with zero new identity; the **ONE
  pushbuffer parser** as the address-table populator (co-equal with RPC bindings; CE-PT-write
  capture; opaque user ring fast path); the **completion tie-in** feeding the existing
  per-`Proc` `CompletionQueue`, reconciled with the #14 EXECUTION-plane root cause
  (per-`Vas` host publication is load-bearing, the queue stays simple).
- **The seams an arch fills:** `Arch::{engine_of_object, is_case2_control, pushbuffer}` +
  `PushbufferAbi::{decode_method, gpfifo_entries}` (encodings), and at most
  `RmBackend::alloc_engine_object` (the host verb surface does NOT grow to add an engine).
  Adding GR/CE/NVENC-for-a-new-arch = new `impl`s + enum arms, zero core edits; `MockArch`
  is the standing N=1 proof.
- **Next milestone:** the compute-GR + CE core surface — `EngineKind` + engine/method
  seams, GR/CE context lifecycle, the ONE pushbuffer parser, per-`Proc` working-set
  publication + ring-gate. Mock-driven first, then `cup3`/`cup8` single-process byte-exact
  and 2× concurrent both rc=0 on the bench once a real `impl Arch` exists. Biggest risk =
  the CE-PT-write capture commit-point timing (#13/#14 seam); first bench validation
  disambiguates it.
