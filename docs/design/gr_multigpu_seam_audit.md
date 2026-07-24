# GR + multi-GPU seam audit — what must change NOW vs what is already a plug-in

**Status:** READ-ONLY architectural audit, 2026-07-25, at head `0ba300b` (+ the mutation-gate
commit). No code changed by this audit. Companion docs: `execution_plane.md` (§2.2/§2.6
graphics, §3 seams), `multi_gpu_and_mig.md` (the GpuId target-axis design),
`core_completeness_gate.md`.

**The bar applied to every row** (the owner's directive): *would adding the capability later
force editing an EXISTING core type, enum, or function signature — or is it just NEW code
plugging into a seam that already exists?* Forces-an-edit → bolt-on risk → the minimal seam
change is specified NOW plus the test that pins it. Plugs-into-existing → seam-correct →
explicitly **no change** (an unused speculative abstraction is itself a bolt-on of
complexity — flagged in §3).

Two failure modes this audit refuses: (a) letting GR/multi-GPU become surgery later, and
(b) manufacturing abstractions the enum/trait seams were explicitly designed to make
unnecessary.

---

## 1. PART 1 — Graphics-GR seam audit

Ground truth for what graphics-GR actually needs (C-repo learnings): the GR golden context
is produced by FECS on real silicon — a hard boundary — so graphics context creation
**forwards** to the host exactly like compute (`mode2_fakeboot_complete`,
`execution_plane.md` §0/§2.2). The guest's extra graphics RM classes (`NV01_EVENT`,
`GF100_DISP_SW`, `NV_SEMAPHORE_SURFACE`, `NV_CONTEXT_DMA` — `vulkan_device_enumerates`)
are Axis-A alloc-param rows plus ordinary graph nodes. Display is the present path ONLY:
guest render target → **isolate-side `PRIME_HANDLE_TO_FD` dma-buf export** → VMM scanout →
host present-complete fed back as a synthetic vblank; NEVER NVKMS (`modeset_strategy`,
`present_path_b_done`, `present_window_dualpath_design`). Render-target *bindings* live in
the opaque userspace ring the host GPU consumes directly (`graphics_buffer_parity_plan`:
most graphics buffers are GPU-VA-only, never CPU-mapped) — the core never models them.

### 1.1 The GR table

| Capability | Current state | Verdict + minimal change now (if any) |
|---|---|---|
| **Graphics context representation** (`Channel`/context model) | `Channel` carries `engine: EngineKind` (graph-derived, refined by the engine object — `project.rs` pre-pass), `host_engine_objects: BTreeMap<ClassId, HostHandle>` (idempotent Case-1 table), `vas: Option<Pdb>`; the GR ctx is the `(Channel, Vas)` pair with zero new identity (§2.2). Golden-ctx image, render-target bindings, ctx buffers are **host-owned**; guest-side graphics RM objects (event/sema-surface/ctx-dma) are ordinary `ObjectKind::{Event, Memory, Other}` graph nodes. | **Seam-correct — NO change.** A graphics context is `EngineKind::GrGraphics` on the existing fields; nothing needs to be added to `Channel`/`Vas`/`Proc` later. Adding a "graphics context object" to the core would contradict the settled zero-new-identity design. |
| **Engine routing** | No routing arm falls through to a compute-hardcoded path: `handle_doorbell`, `forward_engine_object`, `route_control` are engine-agnostic by design (engines differ only in *encodings*, behind `Arch`); `GrGraphics` is a first-class `EngineKind` variant, produced by `Arch::classify`/`engine_of_object` and landed on the channel by the projection's engine-object refinement (tested: `engine_of_object_classifies_all_kinds`, `engine_kind_lands_on_the_channel_via_the_graph`). `completion_arm`'s `_ => SharedSema` wildcard is deliberate and documented (NVDEC stays on the default until bench-proven). | **Seam-correct — NO change.** Graphics-GR routes today; the only graphics-specific dispatch (`present_scanout`) already exists as a separate entry point. Do NOT introduce a `dyn Engine` object — the enum-not-trait decision (§2.1) is load-bearing. |
| **Present/display seam — consumer half** (`Present` port) | `nvkvm-vmm::Present { present(buffer, meta) -> Vblank }` exists, `nvkvm-fwd::present_scanout` wires it to the owning proc's `CompletionQueue` as a synthetic vblank, `MockPresent` exercises it (`present_seam.rs`). Host-agnostic (QEMU/PRIME later), NVKMS unrepresentable. | **Seam-correct in shape** — but see the buffer-type row below. The trait, the vblank feedback, and the per-proc keying need no change. |
| **Present buffer type** | `Present::present` takes `RamHandle` — a type *documented and produced as a shareable handle over guest RAM* (`Vmm::export_ram`). The proven present path (`present_path_b_done`) exports **host VRAM** via the owning **isolate's** PRIME export; the scanout source is a host-GPU surface, not a slice of guest RAM. When the real adapter lands, the parameter type must change → editing an existing port trait + `present_scanout` + every impl. | **BOLT-ON RISK → change now (GR-2a):** introduce `SurfaceHandle(pub u64)` (opaque host-surface token, `nvkvm-vmm`) and retype `Present::present(buffer: SurfaceHandle, meta: FbMeta)` + `present_scanout` + `MockPresent`. Rename-level edit today (2 consumers); a signature break later. **Test:** `present_seam.rs` updated — a `SurfaceHandle` minted by the isolate (below) presents and feeds the vblank; `RamHandle` no longer typechecks into `present`. |
| **Present seam — producer half** (who mints the surface) | Nothing. `RmBackend` has no verb that turns a host memory object into a presentable surface. The C proved the export runs **in the isolate** (stub `PRIME_HANDLE_TO_FD`, session-owned, graphics-gated — `present_path_b_done`); the security rule is one-way guest→host (`present_window_dualpath_design`). A seam with only a consumer is not a seam: adding the producer later means adding a required method to the existing `RmBackend` trait (every impl edits). | **BOLT-ON RISK → change now (GR-2b):** add ONE intent verb `RmBackend::export_surface(memory: HostHandle) -> Result<SurfaceHandle, RmError>` + `MockRmBackend` impl (records the export; unknown handle → `BadHandle`, loud). This is the display seam `execution_plane.md` §3.3 already committed to naming — it is not engine growth (the anti-bolt-on rule "the verb surface does not grow per engine" is about engines; display was always one named verb). **Test:** mock chain `export_surface → present_scanout → vblank on the OWNER's queue`; export through a *foreign* proc's isolate is unrepresentable (per-proc `rm()` reachability), pinned by the existing blast-radius pattern. |
| **Completion for graphics** (Vulkan/GL fences) | Vulkan/GL completion shapes map onto the existing patterns with no new type: shared sema-surface poll = (a) passthrough; CE/GR `SEM_RELEASE` = (c) parser-observed; `NV01_EVENT` os-event waits = (d) per-proc queue + poll re-post; the synthetic vblank already rides the owner's `CompletionQueue`; and if a mapped-fence shape ever appears, pattern (e) `FenceArms` exists with the #12 jump guard. `completion_arm(GrGraphics) = SharedSema` is the documented correct arm. | **Seam-correct — NO change.** No new completion pattern, no edit to `CompletionQueue`/`DeliveryPlane`/`FenceArms`. |
| **Standalone GR / forward-to-host generality** | `forward_engine_object` is class-agnostic (a graphics class is just another `ClassId` → `EngineKind` row on the arch); Case-2 ack-only covers `PROMOTE_CTX`/`GET_CTX_BUFFER_INFO` for graphics identically; `signal_golden_capture` is engine-agnostic and system-typed. `RmBackend::alloc(parent, class, params)` + `alloc_engine_object` carry any graphics alloc blob (Axis-A lowers params). | **Seam-correct in verbs** — except the one signature below. |
| **Host channel is engine-blind** | `RmBackend::alloc_channel(vas: HostHandle) -> (HostHandle, u64)` conveys **no engine**, so an adapter cannot choose the runlist/engine type for the host `NV_CHANNEL_ALLOC_PARAMS`. This is the C's proven wrong-runlist bug class verbatim (`dma_copy_class_alloc_params`: missing engine typing → `engineType=0` → wrong runlist → cuCtxCreate 401). The channel's `EngineKind` is *known* at both materialization sites (`handle_doorbell`, `forward_engine_object`) and simply not passed. When GR-graphics (or even the real compute/CE adapter) lands, this signature MUST change. | **BOLT-ON RISK → change now (GR-1):** `alloc_channel(vas: HostHandle, engine: EngineKind)`. One trait method, two call sites, one mock. **Test:** the mock records the engine per channel alloc; materializing a `Ce`, `GrCompute`, and `GrGraphics` channel records three distinct engines (pins the wrong-runlist class as a core-level regression, not an adapter hope). |
| **Pushbuffer model for graphics** | Graphics methods (render-target setup, draw calls) are userspace-ring content → `PushMethod::Opaque` passthrough by explicit design (§2.3: the parser runs only where the core is mediator). No new fact kind is needed for graphics. | **Seam-correct — NO change.** Decoding graphics methods would be the forbidden method emulator. |

### 1.2 GR verdict in one line

The core was designed for exactly this and it shows: **six of eight rows are already clean
plug-ins.** Two genuine seam changes are needed now, both small and both C-lesson-backed:

- **GR-1** — `alloc_channel` gains the channel's `EngineKind` (the wrong-runlist class).
- **GR-2** — the present seam's missing half: `SurfaceHandle` newtype replacing `RamHandle`
  in `Present::present` (2a) + the `RmBackend::export_surface` producer verb (2b).

Everything else about graphics-GR later is **new code**: an `Arch` class-ID row for the
graphics object class, Axis-A alloc-param rows (`NV01_EVENT` 0x0005, `GF100_DISP_SW`
0x9072, `NV_MEMORY_MAPPER` 0x00fe — the L11 family), and the QEMU/PRIME `Present` adapter.
Zero further core edits.

---

## 2. PART 2 — Multi-GPU seam audit (pushed: near-term build)

### 2.1 Current state, honestly

There is **no global/singleton state**: every field lives on the `Gpu` struct (grep-clean of
`static`/`lazy`/interior mutability; the concurrency contract enforces it). N independent
*devices* already instantiate fine. What is single-GPU is the **axis inside one device**:
one `arch`, one `GpaSpace`, one `DeliveryPlane`, one isolate per `Proc`, and — critically —
device-global `by_pdb: BTreeMap<Pdb, _>` / `by_vchid: BTreeMap<VChid, _>` with
`PdbCollision`/`VchidCollision` refusals.

★ **The single most important finding of this audit:** `Pdb` and `VChid` are **per-GPU
namespaces**, not global identities. A PDB is a page-directory FB address in *that GPU's*
framebuffer; a vChid is an index in *that GPU's* CHRAM/runlist. Two physical GPUs can — and
with a deterministic guest driver *will* — present **identical PDB values and identical
vChids**. Today's core would refuse that legal state as a hostile
`PdbCollision`/`VchidCollision`: the collision guards, which are a security feature at N=1,
become a **false-positive denial of legal multi-GPU traffic** at N=2. Every routing map,
fault variant, and fwd entry point keyed bare-`Pdb`/bare-`VChid` hard-codes N=1, and every
new caller written against those signatures (the upcoming L1 adapter above all) deepens the
assumption. This is exactly the #14 lesson lifted one axis: identical identities across
boundaries must be disjoint **by key construction**, not by hoping values differ.

### 2.2 The multi-GPU table

| Capability | Current state | Verdict + minimal change |
|---|---|---|
| `Gpu` instantiable N times (N devices / N VMs) | All state instance-owned; `Send + Sync`; no statics. | **Seam-correct — NO change.** |
| **`GpuId` target identity** | Does not exist (no hit in the tree). | **Pure ADD (MG-1):** `GpuId(u32)` newtype in `nvkvm-arch::ids`, MIG-accommodating per `multi_gpu_and_mig.md` (a *routable target*, not "physical device"). |
| **Device → GpuId derivation** | `ObjectKind::Device` exists but `AllocFacts` records no `deviceInstance`; no projection resolves an object's `Device` ancestor. | **Touches an existing type (MG-2):** `AllocFacts.device_instance: Option<u32>` (struct has `Default`, blast radius small) + a pure projection `gpu_of(node) → Option<GpuId>` via the typed `origin_of_kind` parent walk (MISS = `None`, never a guess). |
| **Per-GPU routing keys** | `Boundaries`/`Gpu::{by_pdb, by_vchid}` keyed bare `Pdb`/`VChid`; `ProjectionError::{PdbCollision, VchidCollision}` compare globally; `FwdFault::{UnknownPdb, UnknownVchid}` name no target; `fwd::{resolve, handle_doorbell, publish_backing, fence_observed}` take bare identities. | **Touches existing types/signatures (MG-3) — the finding above.** Keys become `(GpuId, Pdb)` / `(GpuId, VChid)`; doorbells carry the GpuId of the BAR they arrived on (the adapter knows which emulated device trapped). Collision refusal stays — scoped per target. |
| **Per-(Proc, GPU) isolation** | `Proc.isolate: Box<dyn Isolate>` (one), `Proc.arena: GpaArena` (one), `IsolateFactory::spawn(id)` (no target). A proc spanning two GPUs must not let a bug on GPU0 reach GPU1's handles — the #14 blast-radius boundary lifted onto the GPU axis (`multi_gpu_and_mig.md` item 3). | **Touches existing types/signatures (MG-5):** `Proc.isolates: BTreeMap<GpuId, Box<dyn Isolate>>`, `Proc.arenas: BTreeMap<GpuId, GpaArena>`, `IsolateFactory::spawn(id, gpu)`. Every `proc.isolate.rm()` site re-routes by the op's target. |
| **Per-GPU device state** | One `arch`, one `gpa: GpaSpace`, one `delivery: DeliveryPlane` on `Gpu`. Each emulated GPU has its own GSP queue (the drain gate is per queue) and its own guest-physical window. | **Touches existing types (MG-6):** a `GpuTarget { gpa: GpaSpace, delivery: DeliveryPlane }` per `GpuId` on `Gpu`. Keep ONE `arch` for V1 (homogeneous multi-GPU, loudly refused otherwise at realize) — heterogeneous archs later fold into the same `GpuTarget` without re-surgery. |
| **`Vas`/`Channel` target tag** | Neither names a GPU; a VASpace/channel hangs off exactly one `Device`, so the tag is graph-derived. | **Touches existing types (MG-4):** `Vas.gpu: GpuId`, `ChannelFacts`/`Channel` gain the derived target (or key `Proc.vases` by `(GpuId, Pdb)` — same thing; pick one, the projection re-derives it either way). |
| **Cross-GPU data (peer copies)** | `Aperture::Peer` already exists in the PTE vocabulary. | **Seam-correct — NO change now.** P2P forwarding is a later capability behind the existing aperture arm; do not build it speculatively. |
| **MIG** | Nothing built; `GpuId` designed as a target, not a device node. | **NO change and NO tests now** (absent hardware — `multi_gpu_and_mig.md`'s own rule). The GpuId-as-target shape is the whole accommodation. |

### 2.3 The near-term build plan (ordered, small, each step tested)

Steps marked **[SIG]** touch existing types/signatures; **[ADD]** is pure new code.
**Timing recommendation: do MG-1..MG-3 immediately** — they edit the most-central types
(`AllocFacts`, `Boundaries`, the fwd entry points) whose caller count only grows; doing them
before the L1 adapter exists means the adapter is *born* multi-GPU-shaped and no rewrite
ever happens. MG-4..MG-7 follow as the build proper.

1. **MG-1 [ADD]** — `GpuId` newtype (+ `assert_send_sync`, Display). Test: compile + id
   discipline (a `BTreeMap<GpuId, _>` cannot be indexed by `Pdb` — free from the newtype).
2. **MG-2 [SIG-lite]** — `AllocFacts.device_instance` + `gpu_of` projection (typed
   `Device`-ancestor walk). Tests: two `Device`s under one client; channels/vases under
   each resolve to the right `GpuId`; an object with no resolvable `Device` ancestor →
   `None` (MISS, never default-GPU0-guess); the shuffle/order-independence property holds
   with the new fact present.
3. **MG-3 [SIG]** — routing keys become `(GpuId, Pdb)` / `(GpuId, VChid)` in
   `Boundaries`/`Gpu`; `FwdFault`/`ProjectionError` variants carry the target;
   `handle_doorbell(gpu, target: GpuId, token, ws)` (the BAR that trapped names the
   target). **The pinning test (the #14-across-GPU bar):** two GPUs presenting *identical*
   PDB values, identical vChids, identical guest VAs, identical RM handles — both route,
   neither collides, and the old global-collision refusal is asserted GONE for the
   cross-GPU case while still firing within one GPU.
4. **MG-4 [SIG]** — `Vas`/`Channel` carry their derived `GpuId`; `Gpu::refresh` syncs it
   from the projection (never accreted). Test: re-derivation stability across events.
5. **MG-5 [SIG]** — per-(Proc, GPU) isolates + arenas; `IsolateFactory::spawn(id, gpu)`.
   **Cross-GPU isolation test:** a hostile proc bound to GPU0 cannot reach GPU1's PDBs,
   arenas, completions, or backing — mock backends per target record verbs; an op for
   GPU0 appearing on GPU1's backend fails the test (op-lands-on-correct-GPU).
6. **MG-6 [SIG]** — per-target `GpuTarget { GpaSpace, DeliveryPlane }`; completion
   pump/poll/drain per target (each GSP queue has its own drain gate). Test: a batch
   outstanding on GPU0's queue does not gate GPU1's post (no cross-GPU serialization);
   starvation fix holds per target.
7. **MG-7 [ADD]** — the acceptance suite of `multi_gpu_and_mig.md`: correct-GPU routing,
   cross-GPU isolation, #14-across-GPU, determinism with the GPU axis; plus a lifecycle
   churn test (per-GPU arena recycle — the #80 class per target).

---

## 3. Speculative over-abstractions to AVOID (named, so they don't creep in)

- **No `dyn Engine` trait objects** — the enum-not-trait decision is settled and correct;
  graphics proved it (zero new subsystem).
- **No core surface registry / render-target tracking** — the RmGraph `Memory` node + the
  opaque ring already carry everything; `graphics_buffer_parity_plan` proved buffers are
  mostly GPU-VA-only. The core's present surface is ONE opaque token.
- **No present format/modifier negotiation, multi-head model, or vsync pacing policy in the
  core** — adapter concerns (`present_window_dualpath_design`'s fast/fallback autodetect is
  entirely QEMU-side).
- **No NVKMS anything** — already unrepresentable; keep it so.
- **No MIG code or tests** — the GpuId-as-target shape is the entire accommodation until
  datacenter hardware exists.
- **No per-engine completion re-plumbing** — patterns (a)/(c)/(d)/(e) cover the union;
  `completion_arm`'s conservative default for unproven engines is a feature.

---

## 4. VERDICT — the ordered list of core changes to make NOW

**Graphics-GR: 2 changes (the core is otherwise already seam-correct — say it plainly:
`EngineKind` + the Case-1/Case-2 lifecycle + the present consumer seam + the completion
patterns were designed for exactly this, and they hold).**

1. **GR-1:** `RmBackend::alloc_channel(vas, engine: EngineKind)` — the engine/runlist
   declaration the host adapter cannot invent (pins the C's `dma_copy_class_alloc_params`
   wrong-runlist class). One method, two call sites, one mock. Test: per-engine channel
   materialization recorded at the backend.
2. **GR-2:** the present seam's missing/mistyped half — `SurfaceHandle` newtype;
   `Present::present(SurfaceHandle, FbMeta)`; `RmBackend::export_surface(HostHandle) ->
   SurfaceHandle` (the isolate-owned PRIME export, C-proven). Test: mock
   export→present→vblank chain on the owner's queue; guest-RAM handles no longer typecheck
   into present.

**Multi-GPU (near-term build): seat the axis now, in order MG-1 → MG-7 (§2.3).** MG-1/MG-2
are cheap adds; **MG-3 is the load-bearing one** — `(GpuId, Pdb)`/`(GpuId, VChid)` routing
keys, because Pdb/vChid are per-GPU namespaces and the current global maps would refuse
legal multi-GPU traffic as collisions. MG-3..MG-6 all touch existing signatures, which is
precisely why they are cheaper NOW than after the L1 adapter calcifies the single-GPU
shapes. MIG stays a named, unbuilt target kind.

**Safely deferred (new code at existing seams, no core edits):** the graphics `Arch`
class-ID + Axis-A param rows; the QEMU/PRIME `Present` adapter; P2P peer-copy forwarding;
NVDEC; MIG; heterogeneous-arch multi-GPU (contained in `GpuTarget` when it comes).
