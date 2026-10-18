# Sweep integration conflict ledger

STATUS: LIVE, 2026-10-18. STOP: protected conflicts require the owner's decision.

MEASURED: the GPU-free, unapplied `git merge-tree --write-tree --messages -z`
preflight of candidate 4 `0452613b5` and gpu-types `1cb048610` reports content
conflicts in three protected code/test files, comprising twelve hunks. It also
reports a documentation conflict in `docs/STATUS_AND_HANDOFF.md`; that document
is outside the intended code-only import. No merge result was checked out, no
conflict was resolved, and no historical evidence tree was imported.

The preflight result tree is `921827adbd65d90061eb1a6236e327363592e2dd`.
Ranges below identify that tree's conflict-marker lines, **not** candidate 4's
source line numbers. `preflight_conflicts.json` records stage 1/2/3 blob IDs,
per-side hunk hashes, sizes, and short source excerpts for reproduction.

| File | Hunk / result lines | Current side | Incoming side | Winner and reason |
|---|---|---|---|---|
| `crates/kf-chan/src/user_loop.rs` | H1, 254–331 | Lifecycle and lazy-observe statistics | Older flow statistics and stage timers | UNRESOLVED; protected lifecycle/USER relay |
| same | H2, 397–401 | Current summary format | Batching/flow summary including PUT-ahead field | UNRESOLVED; protected relay diagnostics |
| `crates/kf-chan/tests/user_loop_model.rs` | H1, 153–159 | Retired-PUT-poll mutant | Missing-final-doorbell mutant | UNRESOLVED; protected USER/doorbell model; no test may be silently dropped |
| `crates/kf-qemu/src/userpump.rs` | H1, 481–523 | `core` relay with `Aux` and software state | Inline marker/gate/batching fields | UNRESOLVED; protected relay state architecture |
| same | H2, 1039–1059 | Initialize `Aux` and software state | Initialize inline relay/batch state | UNRESOLVED; coupled to H1 |
| same | H3, 1881–1947 | Current relay turn, lifecycle deltas and lazy flush | Chained turns, deferred bell and timing counters | UNRESOLVED; protected lifecycle/lazyobserve/doorbell |
| same | H4, 2014–2242 | `kf_chan::user_turn::Relay::step` | Inline observation/read/gate/scan | UNRESOLVED; protected production relay execution |
| same | H5, 2262–2383 | No old duplicate inline body | Inline no-op/segment handling | UNRESOLVED; coupled to H4, not an independent addition |
| same | H6, 2405–2504 | Current scan-failure diagnostic | Inline marker/end-batch/credit methods | UNRESOLVED; protected completion/relay behavior |
| same | H7, 2666–2759 | Shared `publish_guarded` admission | Inline admission plus deferred `ring_owed` | UNRESOLVED; protected doorbell/publication ordering |
| same | H8, 2844–2855 | `core` scan/cursor and lifecycle accounting | Old scan/cursor and segment-turn count | UNRESOLVED; protected lifecycle/relay |
| same | H9, 2939–2948 | `core` scan plus lifecycle accounting | Old scan plus rare-entry turn counter | UNRESOLVED; protected lifecycle/relay |

CODE READ: incoming history affecting these files includes `24e7a297a`
(flow/stage measurement), `639acf57d` (histograms), `7476ef69f` (EXPECTED RED
batching tests), and `edcaaca67` (claim batching), merged through `29eae05a4`.
The conflict is not merely additive die data: it overlaps the later production
relay implementation. A blanket incoming-side choice would discard current
relay/lifecycle code; a blanket current-side choice alone would leave incoming
nonconflicting dependent edits unreviewed. Neither choice was applied.

## Owner decision requested

INFERRED recommendation: authorize excluding this obsolete USER relay batching
and measurement topic, including its dependent nonconflicting edits, from the
gpu-types import. Preserve current integration relay/lifecycle/doorbell behavior
and its tests; integrate the independent die/ABI/constants/display topics only.
If those topics themselves require changes in a protected subsystem or channel
birth/groups, stop again with the exact dependency. Any desired measurement
port would be a separate topic against the current relay, with failing-first
tests and mutation checks, after explicit direction.

Alternative: request a dedicated port of the old batching/measurement behavior
onto the current relay before continuing the sweep. This is not covered by an
automatic conflict resolution and would require its own tests and review.

H-I1 FALSIFIED: protected conflicts were measured. The mandatory STOP rule in
the owner's 2026-10-18 steering, Phase 1.3, fired. Phase 1 integration and all
Phase 2 hardware work remain pending that decision.
