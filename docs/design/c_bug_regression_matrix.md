# The C-bug regression matrix (decision #18B)

Every bug the C research artifact (`../nvidia-gpu-passthrough`, branch `consolidation`)
paid bench-days to find, classified against this core: is it **IMPOSSIBLE** by
construction, **TESTED** by a named regression, or a **GAP**? Gap rows either got a
mock-driven regression in `tests/tests/c_bug_regressions.rs` this pass
(**GAP→TESTED**) or are honestly deferred to a named milestone (**GAP-MILESTONE** —
the subsystem is not modeled yet; a test now would be theater).

Sources: the memory ledgers `mode2_12_layered_status`, `mode2_13_multiiter_idle_hang`,
`mode2_14_concurrent_apps`, plus the flagged singles in `MEMORY.md` (#11 USERD-wipe,
scrubber-timeout, golden-ctx, sema-wrap, teardown hardening #80, the ABI-layer bug
family). Classification rule (decision #15): cite the *structural property* or the
*named test*, never vibes. "IMPOSSIBLE" means the buggy code shape is
unrepresentable — the fallback/scalar/gate it lived in does not exist here — and is
still usually backed by a test of the observable contract.

## The matrix

| # | C incident (memory ref) | What it was | Class | Why / where |
|---|---|---|---|---|
| 1 | **#11 USERD-wipe** (`mode2_baremetal_32`) | Emulator CE zero-fill wiped a live USERD page (PyTorch's ring state); C fix `nvkvm_fb_is_live_userd` | **GAP→TESTED** (state half) + **GAP-MILESTONE** (content half) | `cb11_ce_write_never_clobbers_live_binding`: an observed CE write never silently replaces live core state (parser is resolve-first). The *byte-content* guard over a live USERD needs the FB-shadow/regs model — not yet built; lands with that port. |
| 2 | **#12 sticky one-shot doorbell** (`mode2_12` cont.34 fix B) | `doorbell_setup` early-returned on a sticky global; CTX2's GR TSG rang off-runlist | **IMPOSSIBLE** + TESTED | No scalar/one-shot exec state exists: scheduling lives in per-proc `ExecPlane::scheduled` per `ChanId` (`gpu.rs`). Tested: `wo_12_second_context_recreate…` (`scheduled_now` for CTX2), `sim_14`. |
| 3 | **#12 stale sysmem pins / `va_seen` never cleared** (cont.29/31, fix A) | `{client,VA}`-keyed dedup + FB backings survived teardown; CTX2 re-swept `backed=0`, re-backed onto stale pages | **IMPOSSIBLE** + TESTED | There is no global dedup table: address state lives in per-`Vas` tables that die with the `Proc` at teardown; rebind-over-live is a loud `Overlap`. Tested: `wo_12` (identical-VA re-back clean), `wo_13`, `taddr_unmap_eager_and_overlap_loud`. |
| 4 | **#12 finishPayload forge aperture mismatch** (cont.23–26) | Forge wrote FB via a BAR1 shortcut; guest read the channel-buffer aperture — same VA, three different wrong phys | **IMPOSSIBLE** | There is exactly ONE resolution path (`AddressTable::resolve`, per-PDB, MISS=FAULT); no forge-side resolver, no content-scan, no BAR1 shortcut exists to disagree with it. `Binding` carries its aperture. |
| 5 | **#12 global `chan_gpfifo_phys` stomp** (cont.9 L1) + stale `m2_fbback` USERD overlay (cont.31 fix 2) | MRU content-scan globals stomped per doorbell; stale overlay shadowed CTX2's fresh USERD in an in-order scan | **IMPOSSIBLE** | No MRU, no content-scan, no overlay list exists (`FwdFault` docs: "no content-pick, no MRU scan — those do not exist"). Ring/token identity is per-`Channel` (`host_channel`/`host_token`), dropped with the channel. |
| 6 | **#12 sema-write VAS collapse → UVM MAX_JUMP** (cont.32/33) | Blind foreign-PDB fallback resolved a scrubber's sema release onto UVM's persistent tracking-sema page → backwards jump → UVM fatal | **IMPOSSIBLE** + **GAP→TESTED** | No fallback resolve exists to reach a foreign VAS. `cb12_sema_release_routes_to_owner_never_a_foreign_proc`: a release lands ONLY on the owning proc's queue; after teardown it is a loud `RetiredProc` refusal, never consumed against a survivor. |
| 7 | **finishPayload-forge scoping / scrubber-timeout** (cont.33 fix 2, `ce_utils.c:349` class) | Forge had to cover kernel CeUtils but NEVER user-CE/GR | **IMPOSSIBLE** (typed) + **GAP→TESTED** | Forging for a user proc is unrepresentable: the forge entry (`signal_golden_capture`) is typed to the system proc (`Traffic::System`, lesson L5). `cb12_system_forge_never_reaches_a_user_proc_queue` pins the runtime half. |
| 8 | **#13 multi-iter realloc churn** (`mode2_13`, cup8_iter) | Iter-2's remap of the same VA landed unbacked → host Xid 31 FAULT_PDE | **TESTED** | `wo_13_multiiter_realloc_same_va_new_backing_each_iter` (eager-unbind discipline, fresh backing per iter, loud overlap without unbind). |
| 9 | **#13 512M PD1-leaf walker gap** (round 4) | Walker didn't know GA10x PD1 512M leaves → CE PT-writes **silently dropped** | **GAP-MILESTONE** (root) / TESTED (consequence) | The GMMU walker is a skeleton (`nvkvm-mmu::walker`); its port (arch §4.5 step 3) carries the codified requirement: *an un-enumerated leaf size is a loud fault, never a silent drop*, property-tested against ogkm formats + #13's banked traces. The consequence (stale mapping at realloc) is already `wo_13`. |
| 10 | **#13 CE-PT-write publication / leaf-filled-then-linked** (rounds 1–3 mystery, v6 fix) | Root walks read `runs=0` while the leaf page already held committed PTEs; v6 decodes dirtied pages DIRECTLY | **IMPOSSIBLE** + **GAP→TESTED** | There is no root-walk resolve path; capture binds directly from the observed write (`parse_pushbuffer` CE arm). `cb13_pt_write_capture_is_direct_no_root_reachability_needed`: an orphan PT-write resolves immediately, before any linking write. |
| 11 | **#13 part-2 over-broad trigger** (Opus cont.2) | Pre-sema sweep fired on every kernel-CE release → broke cuCtxCreate | **IMPOSSIBLE** | No global sweep exists to over-fire: capture is per-observed-write, per-`Vas`, on the parsed channel only; kernel/scrubber traffic routes to the system proc (`Traffic`), never arming user-proc state. |
| 12 | **#13 `xfer_none` zeroed CE MEMSET** (round 4 part-4 bug) | "No data transfer" guard also zeroed REMAP/SCRUB memsets → killed the scrubber's zero-fill | **GAP-MILESTONE** | Method-encoding semantics (transfer-type vs remap/scrub bits) are Axis-B decode — the real arch adapter's job. The mock's `PushbufferAbi` doesn't model remap bits; faking it would test the mock. Lands with the real-arch adapter + its differential decode tests vs the C parser. |
| 13 | **#14 shared-GPA `ALREADY-MAPPED` collision** (`mode2_14`, `multiproc_collision_blocker`) | Two procs' identical guest VAs collided in one GPA/backing space | **IMPOSSIBLE** + TESTED | Per-proc `GpaArena`s are disjoint sub-ranges by construction (`gpa.rs`). Tested: `t14_arena_disjoint_by_construction`, `wo_14`, `t14_identical_va_disjoint_backing`, `b4_identical_va_distinct_pdb_never_cross_leaks`. |
| 14 | **#14 process-blind VAS content-pick** (round 1) | `chan_execute` scanned `chan_vas[]` in order, first non-zero walk won → B's pushbuffer walked under A's PDB | **IMPOSSIBLE** + TESTED | No content-pick exists: routing is `by_pdb`/`by_vchid` pure projections, MISS=FAULT (`UnknownVchid`/`UnknownPdb`). Tested: `t14_malformed_and_unknown_tokens_fault_loudly`, `t14_doorbell_demux_routes_to_own_isolate`. |
| 15 | **#14 VA-only channel dedup + `chans[]` capacity overflow** (round 3) | B's channel registrations overwrote A's in place; fixed arrays (32) silently overflowed | **IMPOSSIBLE** + TESTED | Nodes key on `(client, handle)`; channels live in per-proc `BTreeMap`s — no fixed capacity, no keyed overwrite. Hostile growth is loud-capped instead (`b2_*` flood tests). Tested: `identical_handles_across_procs_do_not_collide`. |
| 16 | **#14 scalar `(client,tsg)` sched aliasing** (round 3) | Value-only TSG-sched scalar aliased two procs' identical TSG handles → B off-runlist | **IMPOSSIBLE** + TESTED | Same property as row 2: per-proc `ExecPlane`, keyed by per-proc `ChanId`. Tested: `wo_12`, `t14_per_vas_publication_gates_the_ring` (distinct host tokens rung). |
| 17 | **#14 multiproc arming window** (round 3 wall) | Divergences gated on `n>1`, known only after proc-2's alloc — corruption landed before arming | **IMPOSSIBLE** + **GAP→TESTED** | There is no multiproc gate: N=1 is the only code path (lesson L9). `cb14_second_proc_arrives_after_first_is_active_no_arming_window` (the serialized order wo_14 didn't cover) + `cb14_late_merge_after_touch_is_loud_and_atomic` (`LateMerge` refusal is atomic — no silent state fold). |
| 18 | **#14 per-proc host-VAS publication / the EXECUTION fork** (disambig 2026-07-24: host Xid 31 FAULT_PDE) | Loser's identical guest VAs never published into its OWN host GR VAS → host GPU faulted past the shared prefix | **TESTED** | The load-bearing fix in code: per-`Vas` host VAS + the ring-gate. `t14_per_vas_publication_gates_the_ring`, `t14_unpublished_va_is_a_loud_fault`, `gate_working_set` + the structural gate inside `handle_doorbell` (`nvkvm-fwd`; the ungated `ring_gated` sibling was removed in `484eb86` — one ring path). |
| 19 | **#14 round-8 completion starvation** | Delivery ran only off doorbells + `any_completed` + one SWGEN0 batch → a polling proc starved when the other went quiet | **TESTED** | `t14_per_proc_completion_no_starve` (unit), `t14_polling_proc_is_not_starved` (sim): re-post driven off the poller's OWN poll (`DeliveryPlane::on_poll`). |
| 20 | **#14 MRU ring eviction / RING-DARK** (rounds 5/7, `bar1_wpg`) | Global 64-entry MRU lost the PT-writer's ring page under 2-proc BAR1 traffic → PT-write pushes never executed | **IMPOSSIBLE** | Same property as row 5: no MRU/heuristic ring resolution exists; ring identity is forward-populated from channel-alloc facts, unknown = loud fault. |
| 21 | **Teardown reap-at-root-free hang** (P0, `mode2_14` refactor: eager reap hung the dying ctx's residual polls) | Heavy-table reap at the root free broke CTX2-destroy; C deferred it to the GSP re-handshake | **TESTED** (+ core API added) | Retire (eager, refuses new ops) vs reap (deferred, adapter-declared quiesce) split: `Proc::retire` + **new** `Gpu::reap_retired`. Tested: `wo_teardown_during_active…` (staging), `cb_lifecycle_*` (reap + recycle). |
| 22 | **#80 GPA free-list / window exhaustion under churn** (`teardown_hardening_done`) | Sequential process create/destroy exhausted the shared GPA window | **was a LIVE CORE GAP → FIXED + TESTED** | ★ Writing `cb_lifecycle_process_churn_never_exhausts_the_window` **exposed the same leak in this core**: `GpaSpace::carve` never recycled and `Gpu::retired` was never reaped → generation ~4 died `WindowExhausted` (A/B-verified). Fixed: `GpaSpace::release` (by-value: releasing a live arena is unrepresentable) + `Gpu::reap_retired`. |
| 23 | **★ Device teardown→restart lifecycle** (fn-47 idle-release → GSP reboot; #12 cont.12/16/30-32; #13 signature 2; "fresh boot per GPU run", L12) | The whole down/up cycle: context teardown, WPR2 down/up, seqNum-preserving `GSP_INIT_DONE` re-post, `gsp_reloaded` misfire | **split — see below** | Teardown→recreate half: **GAP→TESTED** (`cb_lifecycle_full_teardown_reap_rebuild_identical` + churn). GSP-reboot FSM half: **GAP-MILESTONE — NOT MODELED** (`nvkvm-gsp` is a ~31-line skeleton). |
| 24 | **Golden-ctx silicon boundary** (`mode2_fakeboot_complete`, `mode2_grctx_privilege_wall`) | GR golden context cannot be fabricated; PROMOTE_CTX/GET_CTX_BUFFER_INFO privileged | **TESTED** | Case-1 forward (host kernel-RM builds + self-promotes its OWN ctx) / Case-2 ack-only (never replayed on an unprivileged isolate): `engine_context.rs` suite + `signal_golden_capture` for the capture-completion wait. |
| 25 | **ABI/OS-layer bug family** (`nvos64_abi_fix`, `abi_struct_truncation`, `writeback_bug_pattern`, `stub_status_offset_bug`, `vmap_stack_dma_bug`, `ioctl_nr_collision_bug`) | Hand-maintained `#[repr(C)]`/ioctl/DMA mistakes in the C's Mode-1 stack | **out-of-core by construction; GAP-MILESTONE for the adapters** | The pure core holds no `#[repr(C)]`, no ioctls, no DMA (grep-gated). The classes re-arise only in `nvkvm-abi` (Axis-A codegen, diffed against the C's hand tables — kills the field-order/truncation class) and the L1 OS layer (decision #16's bounded-memory + trybuild discipline). Tests land with those ports. |

**Tally: 25 rows.** By primary class: **14 impossible-by-construction** (rows
2–7, 10, 11, 13–17, 20 — four of which, rows 6/7/10/17, additionally gained a new
contract test this pass), **5 already-tested** (rows 8, 18, 19, 21, 24), **3
gap→tested now** (rows 1-state, 22, 23-teardown), **5 gap-deferred-as-milestone**
(rows 9, 12, 25, plus the content half of 1 and the GSP half of 23 — each named to
the milestone that models the missing subsystem). Net new: **8 regression tests**
in `c_bug_regressions.rs`, one of which exposed + fixed a live core gap (row 22).
Rows carry two classes where the C bug had a structural half and an
observable-contract half.

## The core fix this pass surfaced (row 22)

The churn regression did exactly what decision #18B hoped: it exposed that the
rewrite had **silently reintroduced the C's #80 leak** — `GpaSpace::carve` was
documented "recycling is a later concern" and `Gpu::retired` had no reap point, so
the device teardown→restart lifecycle exhausted the GPA window after a handful of
generations (A/B-verified: with recycling neutered, both lifecycle tests fail with
`WindowExhausted`). Fix, kept in the core's idiom:

- `GpaSpace::release(arena: GpaArena)` — takes the arena **by value**, so releasing
  an arena a live `Proc` still owns is unrepresentable; recycled GPA reuse is clean
  by construction (host mappings died with the isolate session, tables with the
  `Vas`es — nothing exists to be stale, the #12-cont.29 class).
- `Gpu::reap_retired()` — the explicit adapter-declared quiesce point (lesson L10:
  the C proved reaping at the root free hangs the dying context's residual polls;
  the split is retire-eager / reap-deferred).

## ★ The honest status of the device-restart / GSP-reboot lifecycle (row 23)

What the C actually fought there was **two separable things**:

1. **Teardown→recreate of device state** (contexts, procs, arenas, channels,
   isolates). This IS modeled, and is now pinned by `cb_lifecycle_full_teardown_
   reap_rebuild_identical` (full teardown → reap → rebuild with identical
   handles/PDBs/VAs on fresh isolate sessions, arenas provably recycled) and
   `cb_lifecycle_process_churn_never_exhausts_the_window` (24 generations through a
   4-arena window).
2. **The GSP-reboot FSM** — fn-47 UNLOADING, WPR2 down/up, FWSEC/SEC2-booter
   mailbox latches (`mbox0==0xff` unload detection), the seqNum-preserving
   `GSP_INIT_DONE` re-post, and the C's `gsp_reloaded`-misfire bug class. **This is
   NOT MODELED**: `nvkvm-gsp` is a placeholder enum. No test here can honestly
   guard it — a mock of an unwritten FSM tests the mock. It is the named scope of
   migration step 2 (arch §4.5), whose design already mandates the two properties
   the C's bugs demand: the FSM is **resettable in-process** (kills the
   fresh-boot-per-run tax, lesson L12) and its oracle is **trace replay against the
   C emulator's recorded boots** (which will replay the cont.32 `gsp_reloaded`
   misfire and the #13-round-4 reboot signature directly). Until that port, this
   row is the matrix's one honestly-open lifecycle exposure.
