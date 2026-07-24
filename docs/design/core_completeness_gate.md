# Core-completeness gate — is the pure logic core done enough to descend?

**Status:** audit, 2026-07-24, at head `1c7ae84` (post-M3 batches 1–4 + M4 concurrency +
#18A security pass + #18B regression matrix). READ-ONLY audit; no code changed.

**The bar** (from the task, consistent with repo rule 1): the core is "complete" when
every object / address / completion / engine / lifecycle item the ~20 Mode-1-passing
apps exercise is **modeled in pure logic and mock-tested**, so descending to
L1/L2/L3 is *wiring real adapters to a finished core*, not discovering new core logic.

**Rubric reconciled:** `execution_plane.md` §1 (whose §1.1 "No" verdict predates the
M3 batches that then landed — commits `6f425d2`..`c5489a1` built most of what §1.1
listed as missing), the 20-app surface (`realapp_matrix_done`, `llm_7b_inference_done`,
`nvenc_encode_working`, `vulkan_device_enumerates`, the #12/#13/#14 ledgers), the
crates as they stand, and `c_bug_regression_matrix.md` (whose 5 GAP-MILESTONE rows are
folded in below). Test evidence: **86/86 workspace tests green** at audit time.

---

## 1. The capability table

Legend: **yes** = modeled + mock-tested; **partial** = the seam/state exists but a
named piece of the §1/§2 design is absent; **no** = named only. "WHERE" cites are
this repo at head.

### 1.1 Object / control plane

| Core capability | Modeled? | Where | Gap |
|---|---|---|---|
| RM object model (client/device/subdevice/VASpace/TSG/ctxshare/channel/engine-obj/**memory**/**event**) | **yes** | `nvkvm-core/src/rmgraph.rs` (`ObjectKind` via `Arch::classify`); `object_model.rs` (`map_populates_the_address_table`, `event_objects_are_graph_derived`) | — |
| **DUP refcount** (dup survives src free; resource/handle split; alias chains; order tolerance) | **yes** | `rmgraph.rs` `Resource{refs, map_refs}`, `pending_dups`; `wo_dup_then_free_src_keeps_dst_alias_alive`, fuzz `a4_dup_object_is_reference_counted` | — |
| Order-independent projections (`by_pdb`, `by_vchid`, Proc grouping = dup-connected components) | **yes** | `project.rs`; `rmgraph_order_independence.rs`, fuzz `a2_valid_streams_project_order_independently` | — |
| Hostile-input containment (atomic apply rollback, capacity caps, collision refusal) | **yes** | `gpu.rs::apply` snapshot/rollback; `security_boundary.rs` `b1_*`/`b2_*`/`b6_*` | — |
| Case-1 forward / Case-2 ack-only control routing | **yes** | `nvkvm-fwd::route_control` + `Arch::is_case2_control`; `engine_context.rs::case2_controls_are_ack_only_never_forwarded` | Case-2 *set values* (PROMOTE_CTX etc.) are Axis-A rows — L3 codegen, by design |
| Alloc-param-size class (the L11 bug family: cuCtxCreate-401, 3× Vulkan-enum, NVENC ctx-DMA) | **deferred by design** | `nvkvm-abi::DriverAbi::alloc_param_size` (trait shape only) | Table *content* = L3 codegen + diff-vs-C tests (matrix row 25). Not core logic |

### 1.2 Address plane

| Core capability | Modeled? | Where | Gap |
|---|---|---|---|
| ONE forward-populated per-`Vas` VA→phys table, PDB-keyed, MISS=FAULT, overlap loud, unmap eager | **yes** | `nvkvm-mmu::AddressTable`; `taddr_*`, `b4_miss_is_fault_never_silent_wrong_resolve`, `b4_identical_va_distinct_pdb_never_cross_leaks` | — |
| RPC populate source (`MapMemoryDma` → `memory→phys` resolution, idempotent sync, unbind-on-unmap) | **yes** | `gpu.rs::sync_rpc_mappings` + `rmgraph::backing_of`; `object_model.rs` (incl. `unbacked_mapping_is_a_loud_fault`, `map_before_backing_and_pdb_resolves`) | — |
| CE-PT-write capture source, commit-point plumbing (#13) | **partial** | `parse_pushbuffer` CE arm → `Vas.pt_pages` + co-populate; `cb13_pt_write_capture_is_direct_no_root_reachability_needed` | The capture binds `dst→phys=dst` as a stand-in; it does **not decode the written PTE bytes** to recover the published leaf `VA→phys`, and the latch-dirty→decode-at-release-sema ordering (#13 v6, the named "biggest risk") is not modeled. Needs the walker (below) |
| GMMU walk **algorithm** (decode-dirtied-PT-pages loop over `GmmuFmt`/`FbRead`) | **no** | `nvkvm-mmu/src/walker.rs` — 41-line placeholder | ★ This loop is declared *core* ("regime-independent core logic"). Matrix row 9 = GAP-MILESTONE (arch §4.5 step 3). See verdict §3 |
| 512M-leaf / per-gen leaf-size *formats* (incl. loud-fault on un-enumerated size) | **deferred by design** | `GmmuFmt::page_sizes` contract + `PageSize` doc | Format rows = the GA10x `impl GmmuFmt` (arch port). The *contract* (never silent-drop) is stated but only testable once the loop exists |
| Per-proc GPA arenas, disjoint by construction, release/recycle | **yes** | `nvkvm-core/src/gpa.rs` (`carve`/`release` by-value); `t14_arena_disjoint_by_construction`, `cb_lifecycle_process_churn_never_exhausts_the_window` (which found + fixed the #80 re-leak) | — |

### 1.3 Execution plane

| Core capability | Modeled? | Where | Gap |
|---|---|---|---|
| Doorbell demux (token → vChid → own proc/channel/isolate; malformed/unknown = loud) | **yes** | `nvkvm-fwd::handle_doorbell`; `t14_doorbell_demux_routes_to_own_isolate`, `t14_malformed_and_unknown_tokens_fault_loudly` | — |
| Per-proc scheduling, nothing one-shot (#12 CTX2 class) | **yes** | `gpu.rs::ExecPlane` per `ChanId`; `wo_12_second_context_recreate…`, matrix rows 2/16 | — |
| `EngineKind` routing tag + `Arch::engine_of_object` (compute/graphics/CE/NVENC/NVDEC) | **yes** | `nvkvm-arch/src/ids.rs:100`; `engine_context.rs::engine_of_object_classifies_all_kinds` | — |
| GR/CE context lifecycle: Case-1 engine-object forward → host self-promotes own ctx; golden-capture completion typed to system proc | **partial** | `forward_engine_object`, `signal_golden_capture` (`Traffic::System`-typed); `engine_context.rs`, `cb12_system_forge_never_reaches_a_user_proc_queue`; matrix row 24 | Two §2.2 items absent: **(a)** `Channel` carries only coarse `EngineClass{Gr,Ce,Other}` — the `EngineKind` the design says the core tracks per channel is never recorded (`gpu.rs::Channel` has no field); **(b)** the engine-object forward is **not idempotent** — a re-sent Case-1 alloc re-allocs a *second* host object (`case1_second_forward_reuses_channel` pins channel reuse only). §2.2: "the object's Case-1 alloc has been forwarded (so re-sends are idempotent)" |
| Anti-bolt-on: host verb surface does not grow per engine | **yes** | `engine_context.rs::host_verb_surface_does_not_grow_per_engine` | — |
| The ONE pushbuffer parser (4 fact kinds + opaque passthrough; bounded, fuzzed) | **yes** | `parse_pushbuffer` + `PushbufferAbi`; `pushbuffer_parser.rs` (scripted + hostile + proptest), `b2_pushbuffer_length_flood_is_bounded` | Method-encoding *semantics* (xfer_none/remap bits) = real-arch adapter, matrix row 12, by design |
| Per-`Vas` working-set publication + **ring-gate** (#14's load-bearing fix) | **partial** | `publish_backing`, `gate_working_set`, `ring_gated`; `t14_per_vas_publication_gates_the_ring`, `t14_unpublished_va_is_a_loud_fault` | **Two ring paths exist**: `handle_doorbell` rings *ungated* (line ~234) while `ring_gated` gates. The #14 invariant "unpublished at ring time = loud fault" holds only if the caller picks the right entry point — it is not structural. The C's "one exec path" refactor-debt lesson (`mode2_gpu_emul_refactor_debt`) applies verbatim |
| Multi-process: identical VAs + identical handles, disjoint everything | **yes** | `sim_14_two_process.rs`, `identical_handles_across_procs_do_not_collide`, `cb14_*` (no arming window, atomic LateMerge) | — |

### 1.4 Completion plane (the five patterns of `execution_plane.md` §1.2)

| Pattern | Modeled? | Where | Gap |
|---|---|---|---|
| (a) shared-page sema busy-poll (dominant compute path) | **yes** (by passthrough design) | core's whole job = correct per-`Vas` publication so the host write lands where the guest polls: `publish_backing` + ring-gate; the poll itself is deliberately un-mediated (decision #7) | — |
| (b) GSP finishPayload (system-scoped forge; aperture-carried) | **partial** | `Binding.aperture` (matrix row 4: no second resolver exists to disagree); forge typed to system (`signal_golden_capture`, row 7) | Queue *encoding* (seqNum ring) = `nvkvm-gsp` — see §3 |
| (c) CE-method `SEM_RELEASE` → per-proc observe | **yes** | parser `SemRelease` arm → owning proc's `CompletionQueue`; `cb12_sema_release_routes_to_owner_never_a_foreign_proc`, soak loop | — |
| (d) interrupt / os-event re-post off the poller's OWN poll (starvation-proof) | **yes** | `nvkvm-completion::DeliveryPlane::on_poll`, `poll_completions`; `t14_per_proc_completion_no_starve`, `t14_polling_proc_is_not_starved` | — |
| (e) **mapped coherent fence (NVENC)** | **no** | nothing — only a doc comment in `nvkvm-fwd` | The §2.4 "distinct arm that `observe`s when the mapped value advances" does not exist in code. See verdict §2 |

### 1.5 Per-app behavior union (the 20-app surface)

| App behavior | Modeled at core level? | Where / gap |
|---|---|---|
| CUDA compute (9 benches + gpu-burn): compute chan + GR ctx + CE, sema completion | **yes** (modulo §1.3 partials) | full chain graph→table→doorbell→Case-1 fwd→parser→observe→gate, all mock-tested |
| LLM (llama.cpp 7B) / PyTorch alloc churn, multi-iter reuse | **yes** | `soak_llm_like.rs` (1000-token × 3 concurrent procs; 20k CI variant), `wo_13_multiiter_realloc_same_va_new_backing_each_iter`, lifecycle churn. Tensor-Core = path *within* GrCompute, no core surface |
| Multi-process (2×–4× concurrent) | **yes** | the #14 suite; the core's strongest area |
| DMA HtoD/DtoH (copy IS the workload) | **yes** | `EngineKind::Ce` + `CeLaunchDma` + `publish_backing`; data movement itself is passthrough by design |
| UVM: dup-grouping, second VAS per proc | **yes** | `dup_edge_groups_uvm_and_compute_into_one_proc`, `multi_vaspace_per_process_keys_address_ops_on_vas_not_proc` |
| UVM **managed memory** (`cudaMallocManaged`) passthrough | **no (named)** | design = pass through to host managed alloc, host owns residency (`mode2_uvm_residency`). No verb/routing row yet. NOT a bar-blocker: the 20-app matrix exercises explicit device mem, not managed (see §3) |
| Vulkan / GL: GR-graphics ctx, enum, present | **yes** for the core half | `EngineKind::GrGraphics` same lifecycle; `present_scanout` → `Present` seam → vblank → completion queue (`present_seam.rs`). Vulkan *enumeration* was an Axis-A param-size bug class → `DriverAbi` row (L3) |
| NVENC H.264/HEVC: session + engine routing + fence completion | **partial** | routing + Case-1 forward: yes (`EngineKind::NvEnc`, tested). Session object = a graph node (fine). **Completion arm (e): missing** |
| NVDEC / AV1 | **no — honest gap by declaration** | `EngineKind::NvDec` arm named only. Excluded from the 20-app bar (broken on the Mode-1 host too, `realapp_matrix_done`) |
| DUP refcount across teardown | **yes** | §1.1 rows |
| Teardown / device restart | **yes** for the state half | retire/reap split + arena recycle: `cb_lifecycle_full_teardown_reap_rebuild_identical`, `wo_teardown_during_active…`. GSP-reboot FSM half: **not modeled** (matrix row 23) — see §3 |
| Concurrency under multi-vCPU | **yes** | `concurrency_stress.rs` (4 tests incl. per-proc lock-free parallelism), compile-time `Send+Sync` asserts |

---

## 2. VERDICT

**CONDITIONAL GREEN — descend after closing three small, genuinely-core items.**
The spine §1.1 called complete is complete, and the execution-plane surface §1.1
called absent has since been **built and tested** (M3 batches 1–4): engine seams,
Case-1/Case-2 lifecycle, the ONE parser, working-set publication + ring-gate, the
present seam, per-proc completion, teardown/reap, 86/86 green, and the C-bug matrix
pins 20 of 25 incident classes as impossible-or-tested. Two procs with identical VAs
and identical handles is the single best-covered scenario in the codebase — the #14
class cannot silently recur.

### 2.1 Must-close before descending (core logic genuinely missing — all small)

1. **Engine-object forward idempotency + `EngineKind` on `Channel`** (§1.3 lifecycle
   row). `execution_plane.md` §2.2 names both as core-tracked state; neither exists.
   Today a re-sent Case-1 alloc allocates a duplicate host engine object (a real
   guest-retry hazard — retried-RPC tolerance is a stated invariant everywhere else,
   `wo_retried_duplicate_events_are_idempotent`), and the channel never learns which
   `EngineKind` its context is (needed by the §2.4 completion tie-in's per-engine
   arm). Fix ≈ one `Option<(EngineKind, HostHandle)>` field + a test.
2. **One gated ring path.** `handle_doorbell` must not be able to ring an
   ungated channel while `ring_gated` exists as the safe sibling — the #14
   ring-gate must be structural, not caller-discipline (the exact "one exec path"
   debt the C accumulated). Fold the gate into the single doorbell path (empty
   working-set = trivially gated for pre-publication channels, loud otherwise).
3. **The mapped-fence completion arm (pattern e).** NVENC is in the 20-app surface
   and its completion shape is *proven different* (`nvenc_101`: the worker reads a
   GPU-written mapped fence with NO syscall; the C initially mis-diagnosed this as
   an event gap and paid for it). The core needs the small arm §2.4 specifies —
   "observe when the mapped value advances" — driven by an `Arch`/`RmBackend` fact,
   plus one mock test. Without it, NVENC's completion semantics would be
   *discovered* at the adapter layer, which is precisely what the bar forbids.

None of these is a redesign; all are hours-scale against existing seams.

### 2.2 Deferred-as-milestone but honestly PURE LOGIC (descent will write more core)

Strictness demands naming these: they are **not adapter work**, they are scheduled
pure-logic buildout the project has already accepted as migration steps — so
"descending is only wiring adapters" is not literally true, it is "wiring adapters
plus two named pure-logic milestones whose scope is bounded and oracled":

- **The GMMU walker loop** (`nvkvm-mmu::walker`, arch §4.5 step 3; matrix rows 9/10).
  The walk algorithm is core by the repo's own definition; the #13 CE-PT-write
  capture currently stands in for it (binds the PT page itself, no PTE decode, no
  latch-until-release commit ordering). This is also the design's self-declared
  **biggest risk** (`execution_plane.md` §4). Oracle: ogkm formats + #13's banked
  traces.
- **The GSP boot FSM + seqNum transport** (`nvkvm-gsp`, 31-line skeleton; matrix
  row 23's honestly-open half). Pure state machine, resettable-in-process by design;
  oracle: trace replay of the C emulator's recorded boots (replays the cont.32
  `gsp_reloaded` misfire directly).

### 2.3 Legitimately deferred to layers (NOT core gaps)

- **Axis-A codegen** (real class IDs, control-cmd values, alloc-param sizes,
  NVOS/RPC layouts, Case-2 set contents) → `nvkvm-abi` L3 codegen, diffed against
  the C's hand tables (matrix row 25). The core consumes these only through seams.
- **The real GA10x `Arch` impl** (token/USERD/method encodings, `GmmuFmt` format
  rows incl. the 512M `PageSize` entry, xfer_none/remap method bits — matrix row 12).
- **Adapters:** Linux isolate (spawn/sandbox/wire), QEMU `Vmm` shell, PRIME/QEMU
  `Present` impl, the register/BAR model (`nvkvm-regs`-equivalent) and with it the
  #11 *content* half (live-USERD byte guard, matrix row 1).
- **UVM managed-mem passthrough** — the design is "don't model residency; forward
  to host managed alloc" (`mode2_uvm_residency`), i.e. an adapter verb + one routing
  row; the 20-app matrix does not exercise it. Sequence after the compute milestone
  as planned.
- **NVDEC / AV1** — excluded from the bar; unproven even in Mode-1. Do not claim.

### 2.4 What is proven (the GREEN half, for the record)

Object model incl. DUP refcount and memory/event nodes; order-independent
projections; hostile-input atomicity + capacity caps; the per-`Vas` MISS=FAULT
address table with both populate sources plumbed; per-proc arenas with recycle
(#80 re-found and fixed by its own regression); doorbell demux + per-proc
scheduling (#12); Case-1/Case-2 routing with the anti-bolt-on verb-set proof;
the ONE bounded fuzzed parser; per-`Vas` publication + ring-gate (#14) under
identical-handle adversarial conditions; completion patterns (a)/(c)/(d) with the
starvation fix; the present/vblank seam; teardown-retire/reap-rebuild; multi-vCPU
safety. 86/86 tests, mock-driven, GPU-free, milliseconds.
