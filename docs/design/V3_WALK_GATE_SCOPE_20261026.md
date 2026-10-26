**STATUS: DESIGN-ONLY, 2026-10-26.** FX1 analysis on candidate `6564dd597`. The production walk gate remains VM-wide. Four executable scope questions are parked as explicitly ignored tests in `crates/kf-qemu/src/mem/walk_gate_tests.rs`; they are expected to fail when selected with `--ignored`. This note proposes a scope, not a verified implementation.

# Scope of the USER-ring mapping-walk gate

## Current protection and its limits

CODE READ at `6564dd597`: `Relay::put_step` (`crates/kf-chan/src/user_turn.rs:633`) checks `Io::walk_busy` before either the cached or fresh `GP_PUT`, before `observe_put`, and before GP descriptor loads. `PumpIo::walk_busy` (`crates/kf-qemu/src/userpump.rs:710`) calls `Inbox::walk_busy` and registers the ring's token with `wait_walk`. `Inbox::walk_busy` (`mem.rs:2317` at the base) ORs the VA thread's busy bit, all unsettled statements, queued splits, and running split tokens. The VA loop (`device.rs:2396`) includes in-flight walks, pending manager work, unsettled statements and an armed invalidate in its busy publication. Its idle loop takes all waiters and rings them.

CODE READ at `6564dd597`: the protection is broader than the turn-entry check. `publish_guarded` (`userpump.rs:565`) repeats the gate before admitting a descriptor/pushbuffer and writing the private ring. Ring migration repeats it before swapping source/USERD and calling `Relay::switch_source` (`userpump.rs:2596`). The Translated resolver also uses the global predicate through `Rows::walk_pending` (`chan.rs:1489`). Changing only `put_step` or only the fresh `gp_put` accessor would leave these paths inconsistent.

The gate protects the following distinct operations (CODE READ at `6564dd597`; ordering interpretation is INFERRED):

1. **CPU source selection and availability (CODE READ at `6564dd597`).** The current GPFIFO descriptor, whole-ring declaration admission and scanned pushbuffer are resolved through confirmed placement rows, not through the guest's current page tables. `snapshot_with` (`userpump.rs:645`) leads to `userread::snapshot` (`userread.rs:239`); `PumpIo::load_dword` and `read_pb` use it. A pending walk may replace the selected backing or add an absent row. Waiting avoids reading the old pushbuffer ahead of a paging-fence acquire and avoids treating a not-yet-landed mapping as a terminal missing-source refusal.
2. **Pointer mode as well as scanning.** Pointer mode reads no pushbuffer, but still snapshots and validates a GP descriptor, admits its source, creates anchors and publishes an unchanged GP reference. A cached `GP_PUT` must not bypass the gate.
3. **Window anchors and parser boundaries.** A clean observation records a previously read `GP_PUT` as the latest safe anchor before a registry epoch/reply. Gating before `observe_put` prevents a pending-mapping turn from advancing this boundary. The same ring generation, packet-boundary rule, full-ring exclusion, seeded SYNC_WAIT and reset of stale marker proofs must survive narrowing.
4. **Publication after an intervening walk.** The second gate in `publish_guarded` covers a walk becoming pending after a turn began. Exact GP/PB source guards, allocation/software liveness and birth RAM generation remain required. `Snapshot::admit` (`userread.rs:77`) re-resolves exact spans under row/topology guards; its documentation explicitly says that it is not the VA-manager ordering gate. These checks do not prove that the guest's pending page-table change has landed.
5. **Migration and Translated reads.** The new source ring must be admitted only with the appropriate mapping state and generation. Translated CPU GPFIFO/pushbuffer/operand resolution must retain its own pending/epoch checks; narrowing the Windows USER gate does not automatically authorize changing Translated policy.
6. **Event-driven resumption.** A blocked ring registers a waiter and wakes the VA thread. Completion must re-ring the right token after rows are applied and invalidated. No idle `GP_PUT` poll or timer may replace this event.

CODE READ at `6564dd597`: this gate does not stop already submitted GPU work or native Linux Passthrough. It does delay additional publications from every relayed USER ring in the VM. It is a conservative CPU relay ordering barrier; it is not a proof that all device activity is paused or that the guest cannot race its own tables.

## Lifecycle invariants that remain binding

The authoritative definitions are `V3_USER_RING_LIFECYCLE_20261016.md` section 3, with sections 0a–0c's corrections. The gate contributes mapping readiness; it does not establish these invariants by itself (INFERRED from the cited turn and lifecycle code).

| Invariant | Required preservation under a scoped gate |
|---|---|
| I1 | Pointer reads no PB; waiting/resuming must not silently switch it to scanning. |
| I2 | Every doorbell-covered entry appears once, in order, bit-exact; unrelated walks cannot strand a ring or reset its cursor. |
| I3 | SYNC_WAIT protects later reads through the tagged processed-marker witness; a walk-completion event is not that witness. |
| I4 | Post-registration-reply entries are scanned in a Window; retain GP/PUT load before registry epoch, the clean anchor and registration-live check. |
| I5 | Only documented lifecycle events change mode; walk completion is a wake, not a new mode transition. |
| I6 | Progress is event-driven; racing waiter registration/completion cannot lose the trailing wake. |
| I7 | The descriptor validated is the exact snapshot published; a resumed turn cannot fetch a replacement dword for publication. |
| I8 | Nothing at/beyond the GP_PUT in use is read; at most one fresh PUT load per turn, same-generation cache only, PB only for that loaded GP, anchors only from observed PUT values. |

INFERRED: conservative waiting preserves these properties by withholding a turn before source/anchor creation and by retrying publication. A narrower predicate is safe only if it covers every mapping dependency of that ring for the whole pending interval; existing byte/source validation alone cannot substitute for it.

## Proposed per-space protocol

1. Key holds by the trusted mirror's `VasKey` plus RM-life/mirror generation, never by a guest-supplied deferred-API target or a reusable numeric token alone. The ring already retains its mirror. Maintain a separate global/unknown hold count. Do not repurpose global received/settled counters: those still order held RPC replies.
2. Publish the affected key/count before enqueueing or exposing an operation, with Release/Acquire ordering. Preserve scope through queued, walking, applying, target-settlement and host-invalidate stages. Hold counts or request generations are necessary: completing one of two requests must not clear the other. A refused host invalidate/unmap stays pending under the existing rulings; guest-leaf absence follows OWNER_RULINGS section AA.
3. Carry `SplitTarget::Space(key)` through running completion. CODE READ at `6564dd597`: `take_split_requests` currently retains only `ticket -> token` in `split_tokens`; a scoped implementation must retain target identity too. Track PDB-to-all-matching-spaces from the manager's trusted root statements. Unknown PDB scope, all-PDB work, global memory operations, reset and topology uncertainty keep a global hold until their affected set is known and safely published.
4. Account for shared roots/subtrees. CODE READ at `6564dd597`: the memory plane learns a root from `SET_PAGE_DIRECTORY`/`COPY_SERVER_RESERVED_PDES`; `PageDirStatement` retains client, VA-space, root and aperture, not a subtree alias graph (`kf-rm/src/rmrpc/mod.rs:299`). Distinct root addresses therefore do not prove independent pushbuffer mappings. INFERRED: until dependency provenance is available, shared/unknown table relationships need a conservative dependency-group or VM-wide hold. The earlier design note's assumption that user PBs avoid RM-reserved shared ranges has not been proved.
5. Apply the same USER scope at all three sites: turn-entry gate, guarded publication, and source migration. Keep Translated `Rows::walk_pending` policy unchanged until its own dependency proof exists. Narrowing the aggregate inbox bit alone would affect unrelated callers.
6. Keep waiters as `(life, key, token, observed request generation)`. Register then recheck readiness, or have the completion loop take and re-ring waiters after the release publication. A completion between readiness check and insertion must still produce a wake. Clear/reset old-life tokens; source migration invalidates a waiter's old identity. No vCPU lock, RM ioctl, sleep or GPU scheduling pause is added.
7. No timeout, polling interval or behavior flag implements this change. The scoped state must be bounded by admitted spaces/outstanding work and refuse overflow by name. GPU work already submitted continues; only dependent future relay reads/publications are held.

## Tests before any implementation

MEASURED results belong in `traces/mem_retirement_fx1_20261018/README.md`; the following are requirements, not claimed results.

- Keep all default `kf-chan` suites green, especially `tests/user_lifecycle_model.rs`, `user_loop_model.rs`, `user_loop_model_v2.rs`, `user_loop_model_v3_pointer.rs`, `user_lifecycle_props.rs`, `user_lifecycle_stress.rs`, `user_lazy_model.rs` and `user_lazy_turn.rs`. Preserve their existing explicit OPEN exclusions; do not weaken invariants, transitions or expected publication sequences.
- The four ignored FX1 contract tests must first fail on the global predicate, then pass through the real scoped production adapter: unrelated queued split, unrelated running split, completion of A while B remains pending, and a scoped statement not holding B. The current test adapter deliberately calls the global API; these tests document the missing behavior, not proof of an unimplemented scoped API.
- Same-space queued/running work and all-PDB fallback must continue to hold. Add two independent tests per mutant that drops own-space holds, drops global fallback, clears an overlapping request early, discards running target scope, treats aliases as independent, bypasses the cached PUT path, or gates only entry and not publication/migration.
- Extend the real-turn bounded model to two spaces with independent publication streams. Interleave scope publication, entry/GP/PB reads, registry bump/reply, alias discovery, host-invalidate failure/retry, source swap and all three gate checks. Check I1–I8 on every transition and exact output at terminal states.
- Model check-complete-register and register-complete-take races, two pending requests for one key, global-to-resolved-scope transfer, token/key reuse across reset, a foreign-space flood and completion while another space remains blocked. Progress must require no timer step.
- Add source snapshot tests where an own-space mapping changes between entry and publication, and where an unrelated space changes. Preserve exact row, permission, kind, RAM-generation and allocation/software admission; reject unknown/expired epochs conservatively.

## Questions to settle before narrowing

1. May USER relay ordering be narrowed with a conservative shared/unknown/global fallback, while Translated callers retain their current gate? Recommendation: yes, but approve the dependency rule explicitly; the earlier walk-gate design requested a relay-semantics ruling.
2. What trusted evidence establishes that two mirrors' user source mappings cannot share mutable table subtrees? Recommendation: do not infer it from distinct PDBs or OS identity; keep unknown relationships globally held until a bounded dependency mechanism is designed and tested.
3. Which global statements must also hold USER publication, particularly Sysmembar, BAR windows, topology/reset and all-PDB work? Recommendation: retain current global behavior until each can be scoped with its completion/wake contract.
4. Is the initial per-space granularity acceptable despite holding unchanged VAs in the affected space? Recommendation: treat it as an intermediate relay optimization; it does not fully implement the owner's per-changed-VA refresh contract.

A later range-based seqlock would snapshot source bytes with a mapping/request epoch, await the relevant pending completion, and publish only if the entire GP/PB range stayed unchanged; otherwise it rereads. It also needs ring/migration, registration-anchor, alias and bounded-log-unknown rules. It cannot simply ignore a pending request because no change has been logged yet (INFERRED: the changed range is unknown until the walk lands). No such behavior is implemented here.
