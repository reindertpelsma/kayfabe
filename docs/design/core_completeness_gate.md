# Core-completeness gate — is the pure logic core done enough to descend?

**Status:** audit, 2026-07-24, at head `1c7ae84` (post-M3 batches 1–4 + M4 concurrency +
#18A security pass + #18B regression matrix). READ-ONLY audit; no code changed.

> **⚠️ PIN EXPIRED — read before trusting any `file:line` or count in this document
> (flagged 2026-07-27, doc audit).** Every "WHERE" citation and every test count below is
> against `1c7ae84`, which is now **79 commits** back. Spot-checks found **three** of them
> already wrong, in increasing order of severity: §1.3's `kayfabe-arch/src/ids.rs` pin for
> `EngineKind` (the old `:100` is now `ControlCmd(u32)`; re-pinned by symbol 2026-07-27);
> §1.3's GR/CE-lifecycle row, whose two
> named gaps have **both since been closed in code** (see the note on that row); and §1.5's UVM
> row, which cited **two tests that had since been deleted** (see the withdrawal note under §1.5).
>
> **The verdict is not withdrawn — the citations are.** Treat the capability ratings as a
> dated judgement and re-resolve any name before relying on it. Counts here (`86/86`, `96/96`)
> are likewise a 2026-07-24 snapshot: the workspace is at **≥531 `#[test]`** as of 2026-07-27.
> They are left unedited on purpose, per this repo's rule that a dated number is more useful
> than a number that will rot again next week.

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
| RM object model (client/device/subdevice/VASpace/TSG/ctxshare/channel/engine-obj/**memory**/**event**) | **yes** | `kayfabe-core/src/rmgraph.rs` (`ObjectKind` via `Arch::classify`); `object_model.rs` (`map_populates_the_address_table`, `event_objects_are_graph_derived`) | — |
| **DUP refcount** (dup survives src free; resource/handle split; alias chains; order tolerance) | **yes** | `rmgraph.rs` `Resource{refs, map_refs}`, `pending_dups`; `wo_dup_then_free_src_keeps_dst_alias_alive`, fuzz `a4_dup_object_is_reference_counted` | — |
| Order-independent projections (`by_pdb`, `by_vchid`, Proc grouping = dup-connected components **of declared user clients**, §12.27) | **yes** | `project.rs`; `rmgraph_order_independence.rs` (incl. `one_kernel_client_two_processes_stay_two_procs`, `the_kernel_declaration_may_arrive_before_between_or_after`), fuzz `a2_valid_streams_project_order_independently` + INV6 | — |
| **Client privilege typing** (User vs Kernel from the declared `processID`; kernel dup = reference; one reserved system component) | **yes** | `kayfabe-arch::ClientKind`, `kayfabe-abi::client_kind_from_process_id`, `project.rs` `SYSTEM_ANCHOR`; `rmgraph_order_independence.rs` (15 tests) | — |
| Hostile-input containment (atomic apply rollback, capacity caps, collision refusal) | **yes** | `gpu.rs::apply` snapshot/rollback; `security_boundary.rs` `b1_*`/`b2_*`/`b6_*` | — |
| Case-1 forward / Case-2 ack-only control routing | **yes** | `kayfabe-fwd::route_control` + `Arch::is_case2_control`; `engine_context.rs::case2_controls_are_ack_only_never_forwarded` | Case-2 *set values* (PROMOTE_CTX etc.) are Axis-A rows — L3 codegen, by design |
| Alloc-param-size class (the L11 bug family: cuCtxCreate-401, 3× Vulkan-enum, NVENC ctx-DMA) | **deferred by design** | `kayfabe-abi::DriverAbi::alloc_param_size` (trait shape only) | Table *content* = L3 codegen + diff-vs-C tests (matrix row 25). Not core logic |

### 1.2 Address plane

| Core capability | Modeled? | Where | Gap |
|---|---|---|---|
| ONE forward-populated per-`Vas` VA→phys table, PDB-keyed, MISS=FAULT, overlap loud, unmap eager | **yes** | `kayfabe-mmu::AddressTable`; `taddr_*`, `b4_miss_is_fault_never_silent_wrong_resolve`, `b4_identical_va_distinct_pdb_never_cross_leaks` | — |
| RPC populate source (`MapMemoryDma` → `memory→phys` resolution, idempotent sync, unbind-on-unmap) | **yes** | `gpu.rs::sync_rpc_mappings` + `rmgraph::backing_of`; `object_model.rs` (incl. `unbacked_mapping_is_a_loud_fault`, `map_before_backing_and_pdb_resolves`) | — |
| CE-PT-write capture source, commit-point plumbing (#13) | **partial** | `parse_pushbuffer` CE arm → `Vas.pt_pages` + co-populate; `cb13_pt_write_capture_is_direct_no_root_reachability_needed` | The capture binds `dst→phys=dst` as a stand-in; it does **not decode the written PTE bytes** to recover the published leaf `VA→phys`, and the latch-dirty→decode-at-release-sema ordering (#13 v6, the named "biggest risk") is not modeled. Needs the walker (below) |
| GMMU walk **algorithm** (decode-dirtied-PT-pages loop over `GmmuFmt`/`FbRead`) | **no** | `kayfabe-mmu/src/walker.rs` — 41-line placeholder | ★ This loop is declared *core* ("regime-independent core logic"). Matrix row 9 = GAP-MILESTONE (arch §4.5 step 3). See verdict §3 |
| 512M-leaf / per-gen leaf-size *formats* (incl. loud-fault on un-enumerated size) | **deferred by design** | `GmmuFmt::page_sizes` contract + `PageSize` doc | Format rows = the GA10x `impl GmmuFmt` (arch port). The *contract* (never silent-drop) is stated but only testable once the loop exists |
| Per-proc GPA arenas, disjoint by construction, release/recycle | **yes** | `kayfabe-core/src/gpa.rs` (`carve`/`release` by-value); `t14_arena_disjoint_by_construction`, `cb_lifecycle_process_churn_never_exhausts_the_window` (which found + fixed the #80 re-leak) | — |

### 1.3 Execution plane

| Core capability | Modeled? | Where | Gap |
|---|---|---|---|
| Doorbell demux (token → vChid → own proc/channel/isolate; malformed/unknown = loud) | **yes** | `kayfabe-fwd::handle_doorbell`; `t14_doorbell_demux_routes_to_own_isolate`, `t14_malformed_and_unknown_tokens_fault_loudly` | — |
| Per-proc scheduling, nothing one-shot (#12 CTX2 class) | **yes** | `gpu.rs::ExecPlane` per `ChanId`; `wo_12_second_context_recreate…`, matrix rows 2/16 | — |
| `EngineKind` routing tag + `Arch::engine_of_object` (compute/graphics/CE/NVENC/NVDEC) | **yes** | `crates/kayfabe-arch/src/ids.rs::EngineKind` *(re-pinned by symbol 2026-07-27; the old `:100` line pin had drifted onto `ControlCmd(u32)`)*; `tests/tests/engine_context.rs::engine_of_object_classifies_all_kinds` | — |
| GR/CE context lifecycle: Case-1 engine-object forward → host self-promotes own ctx; golden-capture completion typed to system proc | ~~**partial**~~ **★ both named gaps CLOSED — see note** | `forward_engine_object`, `signal_golden_capture` (`Traffic::System`-typed); `engine_context.rs`, `cb12_system_forge_never_reaches_a_user_proc_queue`; matrix row 24 | ~~Two §2.2 items absent: **(a)** `Channel` carries only coarse `EngineClass{Gr,Ce,Other}` — the `EngineKind` the design says the core tracks per channel is never recorded (`gpu.rs::Channel` has no field); **(b)** the engine-object forward is **not idempotent** — a re-sent Case-1 alloc re-allocs a *second* host object (`case1_second_forward_reuses_channel` pins channel reuse only). §2.2: "the object's Case-1 alloc has been forwarded (so re-sends are idempotent)"~~ **★ BOTH CLOSED — verified 2026-07-27, see below** |

> **★ (2026-07-27, doc audit) — the GR/CE row's two gaps are both fixed in code; the "partial" was stale.** Verified directly, not inferred:
>
> - **(a) is closed.** `crates/kayfabe-core/src/gpu.rs::Channel::engine` — `pub engine: EngineKind` is a field on `Channel`, with rustdoc citing `execution_plane.md` §2.2 by name (*"NVENC vs GR-compute is distinguishable HERE"*). **And the type the gap named no longer exists at all:** `EngineClass{Gr,Ce,Other}` was removed; the only surviving mention in the tree is in the rustdoc of `crates/kayfabe-arch/src/ids.rs::EngineKind`, a line reading *"(The coarse `EngineClass{Gr,Ce,Other}` this replaced …)"*.
> - **(b) is closed, by a test that names the exact §2.2 sentence this gap quoted.** `tests/tests/engine_context.rs` — `replayed_engine_object_alloc_forwards_exactly_one_host_object`, whose rustdoc reads *"a REPLAYED Case-1 engine-object alloc … yields exactly ONE host engine object."* The gap's parenthetical — *"`case1_second_forward_reuses_channel` pins channel reuse only"* — was true when written and is now merely incomplete: that test still exists and still pins only channel reuse, but it is no longer the only one.
>
> ★ **Worth noting how this one decayed**, because it is the opposite of the §1.5 failure: nothing here was ever *wrong*, and no citation broke. The row simply kept describing a gap after the gap was filled — which is the cheaper failure to make and the harder one to notice, since every name in it still resolves.
| Anti-bolt-on: host verb surface does not grow per engine | **yes** | `engine_context.rs::host_verb_surface_does_not_grow_per_engine` | — |
| The ONE pushbuffer parser (4 fact kinds + opaque passthrough; bounded, fuzzed) | **yes** | `parse_pushbuffer` + `PushbufferAbi`; `pushbuffer_parser.rs` (scripted + hostile + proptest), `b2_pushbuffer_length_flood_is_bounded` | Method-encoding *semantics* (xfer_none/remap bits) = real-arch adapter, matrix row 12, by design |
| Per-`Vas` working-set publication + **ring-gate** (#14's load-bearing fix) | ~~**partial**~~ **★ CLOSED — see note** | `crates/kayfabe-fwd/src/lib.rs::publish_backing`, `::gate_working_set`, `::handle_doorbell`; `t14_per_vas_publication_gates_the_ring`, `t14_unpublished_va_is_a_loud_fault` | ~~**Two ring paths exist**: `handle_doorbell` rings *ungated* while `ring_gated` gates. The #14 invariant "unpublished at ring time = loud fault" holds only if the caller picks the right entry point — it is not structural. The C's "one exec path" refactor-debt lesson (`mode2_gpu_emul_refactor_debt`) applies verbatim~~ **★ NO LONGER TRUE — verified 2026-07-27, see note** |

> **★★ (2026-07-27, doc audit) — the "two ring paths" gap is fixed in code, and this row was
> the one the pin-expiry banner most needed re-checked: it described the C's "one exec path"
> debt as *reproduced*.** It is not.
>
> - **`ring_gated` no longer exists.** The ungated sibling was removed; the only surviving
>   mentions of the name in the workspace are two rustdoc lines saying it *stays* removed
>   (`crates/kayfabe-fwd/src/lib.rs::handle_doorbell` and `tests/tests/pushbuffer_parser.rs`).
> - **The gate is structural, not caller discipline.** `crates/kayfabe-fwd/src/lib.rs::plan_doorbell`
>   is the sole constructor of `VerbPlan::Doorbell` within the production crates and runs the
>   #14 ring-gate before any host op exists.
> - ⚠→★★ **That honest limit is now CLOSED (2026-07-27), and closed where it said to close
>   it: on the constructor.** It read: *"structural" describes the call graph, not the type
>   system — `VerbPlan` is a public enum with public fields and `Worker::execute` is public,
>   so a `VerbPlan::Doorbell` can be built outside `kayfabe-fwd`
>   (`tests/tests/cross_proc_lifetime.rs` does exactly that)*. `VerbPlan::Doorbell` is now
>   `#[non_exhaustive]` — hand-building it outside `kayfabe-isolate` is a compile error
>   (E0639), pinned by that crate's `tests/ui/ungated_doorbell.rs` — and its only
>   constructor, `VerbPlan::gated_doorbell`, RUNS the #14 gate over an abstract
>   `RingWorkingSet` view of the ringing channel's own `Vas`. `cross_proc_lifetime.rs` was
>   rewritten to go through it, which is the seam's own usability check.
>   The residual that replaces it is smaller and is stated at the constructor: Rust's
>   privacy unit is the crate, so *"only `kayfabe-fwd` may call this"* is inexpressible and
>   the address plane is caller-supplied. Bypassing the gate went from **omission** (build
>   the struct, forget the check) to **commission** (write a lying address plane).
> - ★ Also corrected in that rustdoc on the same day: `handle_doorbell` **is not on the L1
>   path at all** — a real guest MMIO write goes through `kayfabe_rt::SharedDevice::doorbell`,
>   which drives plan/execute/commit itself.
| Multi-process: identical VAs + identical handles, disjoint everything | **yes** | `sim_14_two_process.rs`, `identical_handles_across_procs_do_not_collide`, `cb14_*` (no arming window, atomic LateMerge) | — |

### 1.4 Completion plane (the five patterns of `execution_plane.md` §1.2)

| Pattern | Modeled? | Where | Gap |
|---|---|---|---|
| (a) shared-page sema busy-poll (dominant compute path) | **yes** (by passthrough design) | core's whole job = correct per-`Vas` publication so the host write lands where the guest polls: `publish_backing` + ring-gate; the poll itself is deliberately un-mediated (decision #7) | — |
| (b) GSP finishPayload (system-scoped forge; aperture-carried) | **partial** | `Binding.aperture` (matrix row 4: no second resolver exists to disagree); forge typed to system (`signal_golden_capture`, row 7) | Queue *encoding* (seqNum ring) = `kayfabe-gsp` — see §3 |
| (c) CE-method `SEM_RELEASE` → per-proc observe | **yes** | parser `SemRelease` arm → owning proc's `CompletionQueue`; `cb12_sema_release_routes_to_owner_never_a_foreign_proc`, soak loop | — |
| (d) interrupt / os-event re-post off the poller's OWN poll (starvation-proof) | **yes** | `kayfabe-completion::DeliveryPlane::on_poll`, `poll_completions`; `t14_per_proc_completion_no_starve`, `t14_polling_proc_is_not_starved` | — |
| (e) **mapped coherent fence (NVENC)** | **no** | nothing — only a doc comment in `kayfabe-fwd` | The §2.4 "distinct arm that `observe`s when the mapped value advances" does not exist in code. See verdict §2 |

### 1.5 Per-app behavior union (the 20-app surface)

| App behavior | Modeled at core level? | Where / gap |
|---|---|---|
| CUDA compute (9 benches + gpu-burn): compute chan + GR ctx + CE, sema completion | **yes** (modulo §1.3 partials) | full chain graph→table→doorbell→Case-1 fwd→parser→observe→gate, all mock-tested |
| LLM (llama.cpp 7B) / PyTorch alloc churn, multi-iter reuse | **yes** | `soak_llm_like.rs` (1000-token × 3 concurrent procs; 20k CI variant), `wo_13_multiiter_realloc_same_va_new_backing_each_iter`, lifecycle churn. Tensor-Core = path *within* GrCompute, no core surface |
| Multi-process (2×–4× concurrent) | **yes** | the #14 suite; the core's strongest area |
| DMA HtoD/DtoH (copy IS the workload) | **yes** | `EngineKind::Ce` + `CeLaunchDma` + `publish_backing`; data movement itself is passthrough by design |
| UVM: dup-grouping, second VAS per proc | ~~**yes**~~ **★★ WITHDRAWN 2026-07-27 — see note below** | ~~`dup_edge_groups_uvm_and_compute_into_one_proc`, `multi_vaspace_per_process_keys_address_ops_on_vas_not_proc`~~ **both tests no longer exist** |
| UVM **managed memory** (`cudaMallocManaged`) passthrough | **no (named)** | design = pass through to host managed alloc, host owns residency (`mode2_uvm_residency`). No verb/routing row yet. NOT a bar-blocker: the 20-app matrix exercises explicit device mem, not managed (see §3) |
| Vulkan / GL: GR-graphics ctx, enum, present | **yes** for the core half | `EngineKind::GrGraphics` same lifecycle; `present_scanout` → `Present` seam → vblank → completion queue (`present_seam.rs`). Vulkan *enumeration* was an Axis-A param-size bug class → `DriverAbi` row (L3) |
| NVENC H.264/HEVC: session + engine routing + fence completion | **partial** | routing + Case-1 forward: yes (`EngineKind::NvEnc`, tested). Session object = a graph node (fine). **Completion arm (e): missing** |
| NVDEC / AV1 | **no — honest gap by declaration** | `EngineKind::NvDec` arm named only. Excluded from the 20-app bar (broken on the Mode-1 host too, `realapp_matrix_done`) |
| DUP refcount across teardown | **yes** | §1.1 rows |
| Teardown / device restart | **yes** for the state half | retire/reap split + arena recycle: `cb_lifecycle_full_teardown_reap_rebuild_identical`, `wo_teardown_during_active…`. GSP-reboot FSM half: **not modeled** (matrix row 23) — see §3 |
| Concurrency under multi-vCPU | **yes** | `concurrency_stress.rs` (4 tests incl. per-proc lock-free parallelism), compile-time `Send+Sync` asserts |

> ### ★★★ WITHDRAWN (2026-07-27, doc audit) — the UVM dup-grouping row was a **yes** resting on two tests that had been DELETED, one of them *for asserting a shape hardware proved impossible*
>
> **[verified] Neither `fn` exists anywhere in the tree.** Both were removed in
> `062ea67 mode2-rs: ★★ THE PROC GROUPING RULE — measured on hardware, and it was wrong`.
>
> **★ And the deletion was not a rename or a refactor — it was a retraction.** The surviving
> successor file says so in its own header (`tests/tests/rmgraph_order_independence.rs`, the
> `//!` module doc, ~:11-15):
>
> > *"## ★ What this file used to assert, and why it was fiction … the scenario gave process A
> > and process B **one UVM client each** … `dup_edge_groups_uvm_and_compute_into_one_proc`
> > asserted "A + its UVM, B + its UVM". **That shape cannot occur.**"*
>
> The measurement that killed it is `../reference/rm_semantics_measured.md` §3:
> `nvUvmInterfaceSessionCreate` fires **exactly once per `nvidia_uvm` module load**, so there is
> no per-process UVM client to group. See also §3's *"the C's stale comment seeded a wrong model
> that was then encoded in **both** the Rust core and its test suite — so no test could have
> caught it; the tests asserted the same wrong model as the code."*
>
> **★★ This is the worst failure shape this gate can have, and it is worth naming precisely.**
> Not "a citation drifted" — the row read **yes**, with evidence, and the evidence had been
> deleted *because it was false*. A reader auditing the gate would have found a confident row,
> followed the names, found nothing, and had no way to tell whether the tests were renamed or
> repudiated. `testing_doctrine.md` §1's *"a green instrument on an unexercised path is worse
> than none"* has an exact analogue here: **a cited test that no longer exists is worse than an
> uncited claim, because it borrows the authority of a check nobody ran.**
>
> **What is actually true now is not recorded anywhere, and this audit did not establish it.**
> `[unverified]` — the real UVM grouping rule (one session client per module load, `processID
> == KERNEL_PID` as the discriminator) is implemented and tested somewhere in
> `rmgraph_order_independence.rs`, but **whether that covers this row's two claims — dup-grouping
> *and* second-VAS-per-proc — was not checked.** **What would settle it:** name the current test
> functions covering each half and re-rate the row; until then it is neither **yes** nor a
> declared gap, which is the one state this gate is designed to forbid.

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

### 2.1 Must-close before descending — ✅ ALL THREE CLOSED (see below)

**Status update (post-audit):** the three must-close items are now landed and tested
(96/96 workspace tests). Details:

1. **Engine-object forward idempotency + fine `EngineKind` on `Channel`** — ✅ DONE.
   The coarse `EngineClass{Gr,Ce,Other}` was **removed** and collapsed into the fine
   `EngineKind{GrCompute,GrGraphics,Ce,NvEnc,NvDec,Other}` everywhere (`ObjectKind`,
   `ChannelFacts`, `Channel`). `EngineKind` is now graph-derived onto the `Channel`
   (channel-class default, refined by the engine object allocated on it — order- and
   replay-independent), so NVENC vs GR-compute is distinguishable AT the channel.
   `forward_engine_object` is now idempotent: a per-channel `host_engine_objects`
   table (keyed by declared class) makes a replayed Case-1 alloc resolve to the
   ORIGINAL host object — exactly one host object per `(channel, class)`. Tests:
   `replayed_engine_object_alloc_forwards_exactly_one_host_object`,
   `engine_kind_lands_on_the_channel_via_the_graph`.
2. **One gated ring path** — ✅ DONE. The ungated `ring_gated` sibling was **removed**;
   `handle_doorbell` is now the ONLY function that reaches `RmBackend::ring_doorbell`,
   and it ALWAYS runs the #14 ring-gate before any host op (bound-but-unpublished =
   loud fault, empty declaration = trivially gated). The gate is structural, not
   caller discipline. `gate_working_set` remains as the read-only QUERY form (cannot
   ring). Test: `t14_ring_gate_is_structural_no_ungated_door` (+ the existing
   `t14_per_vas_publication_gates_the_ring` updated to the single path).
3. **The mapped-fence completion arm (pattern e)** — ✅ DONE. `kayfabe-completion`
   gained `FenceArms` — per-`Proc` mapped-fence arms (`arm`/`observe`) that fire when
   the observed value reaches/passes target under 32-bit wrap, with the #12 backwards-
   jump guard (`MAX_FENCE_JUMP`, mirroring UVM's `2 × GPFIFO`). `kayfabe-fwd` exposes
   `completion_arm` (engine→arm selection, NVENC=fence, everything else=shared-sema),
   `arm_fence`, `fence_observed`. Distinct from event delivery by construction (never
   enters `CompletionQueue`, never posts, never raises SWGEN0). Tests: four unit tests
   in `kayfabe-completion` + `nvenc_mapped_fence_arms_and_fires_distinct_from_event_delivery`,
   `nvenc_fence_wrap_guard_fires_across_wrap_and_refuses_backwards_jumps`,
   `fence_arm_selection_is_exact_at_the_channel`.

None was a redesign; all landed as pure core against existing seams.

### 2.2 Deferred-as-milestone but honestly PURE LOGIC (descent will write more core)

Strictness demands naming these: they are **not adapter work**, they are scheduled
pure-logic buildout the project has already accepted as migration steps — so
"descending is only wiring adapters" is not literally true, it is "wiring adapters
plus two named pure-logic milestones whose scope is bounded and oracled":

- **The GMMU walker loop** (`kayfabe-mmu::walker`, arch §4.5 step 3; matrix rows 9/10).
  The walk algorithm is core by the repo's own definition; the #13 CE-PT-write
  capture currently stands in for it (binds the PT page itself, no PTE decode, no
  latch-until-release commit ordering). This is also the design's self-declared
  **biggest risk** (`execution_plane.md` §4). Oracle: ogkm formats + #13's banked
  traces.
- **The GSP boot FSM + seqNum transport** (`kayfabe-gsp`, ~~31~~**34**-line skeleton *(counted 2026-07-27)*; matrix
  row 23's honestly-open half). Pure state machine, resettable-in-process by design;
  oracle: trace replay of the C emulator's recorded boots (replays the cont.32
  `gsp_reloaded` misfire directly).

### 2.3 Legitimately deferred to layers (NOT core gaps)

- **Axis-A codegen** (real class IDs, control-cmd values, alloc-param sizes,
  NVOS/RPC layouts, Case-2 set contents) → `kayfabe-abi` L3 codegen, diffed against
  the C's hand tables (matrix row 25). The core consumes these only through seams.
- **The real GA10x `Arch` impl** (token/USERD/method encodings, `GmmuFmt` format
  rows incl. the 512M `PageSize` entry, xfer_none/remap method bits — matrix row 12).
- **Adapters:** Linux isolate (spawn/sandbox/wire), QEMU `Vmm` shell, PRIME/QEMU
  `Present` impl, the register/BAR model (`kayfabe-regs`-equivalent) and with it the
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
