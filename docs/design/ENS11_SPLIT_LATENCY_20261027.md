# ENS11: split latency, coalescing and optional sparse work

GPU-free integration of ENS10 with the IRQ-storm diagnostic at `2813c7529`. The owner refresh contract remains: unchanged VAs stay accessible; a changed VA resolves only to old backing, new backing or fault; no GPU pause. M's event-driven relay handoff remains in place. No 64-attempt budget is introduced.

## Evidence and the full-walk question

Supplied evidence, not a new host experiment: `/root/night/opus_relay/MEMO.md`, reviewing boots 7321 and 7391, attributes 297,495/318,747 and 461,389/490,686 walks respectively to the privileged Translated copy-channel token `0x80c`. The memo reports same-target-space coupling and completions near walk ends. Those observations motivate instrumentation; they do not prove the timing of a GPU marker write relative to host observation.

CODE READ: `VaSplit::invalidated` in `crates/kf-qemu/src/chan.rs` requests a split and polls its result. The channel has already suspended later work behind that split (`crates/kf-chan/src/host.rs`). A fresh post-request walk is intentional: reusing an already submitted snapshot could release new work against stale translations.

CODE READ: `crates/kf-cuda/src/diffmodel.rs` describes the existing diff against committed host placements and per-run acknowledgements. `VaManager::on_walk_ready` passes `EntryDiff.runs` to `apply_entry`; `crates/kf-mem/src/apply.rs` applies those changed runs. Whole-space GPU page-table scanning is not whole-space host remapping. The unchanged-diff model test already passed before this work and still requires zero mapping operations on repeated unchanged splits. This change does not replace the GPU page-table scanner with a range-certified incremental scanner.

CODE READ: `GpuMirror::begin_refresh` and `end_refresh` previously retried optional sparse geometry for the space even when the diff was empty. Those retries can reserve/free RM objects without remapping unchanged backed rows. The new per-split RM-verb deltas and diff/map counters distinguish that cost from changed-run application on the next boot. No new hardware attribution or wall-time improvement is claimed here.

## Coalescing and freshness

The manager already deduplicated pending target keys. The device previously started a walk while iterating the first request from a drained inbox snapshot. It now queues the entire drained snapshot before any armed invalidate or split kick can submit that batch. Same-space and same-PDB tickets admitted together share a walk and its verdict. Requests arriving during an active walk remain pending for a later submission. Refusal and root-change paths retain their existing rewalk/failed-result rules.

The admission cutoff is the inbox snapshot, conservatively before physical submission. A request arriving after that drain belongs to the next batch even if the GPU submission has not yet happened. No inbox lock is held across GPU submission to collect such arrivals. Every successful ticket still requires a submission after its admission; an extra conservative walk is preferable to an early result. A single channel issuing each MEM_OP only after its preceding result has no concurrent same-channel requests to coalesce; batching alone cannot remove that serialized dependency.

The seeded test drives the real manager and diff/shadow walker over 256 schedules of 200 actions, alternating named-PDB/exact-space requests, submission and polling. Its oracle records the submission count at each request and requires every completed ticket to belong to the completed batch and a later submission. Backlog is bounded by the modeled 32 live obligations; the native protocol retains one outstanding split per requesting live channel and the existing USER admission cap. The test does not claim liveness if the GPU or VA worker stops making progress.

## Optional layout after split release

A thread-local RAII scope covers every apply in a batch containing a split, including other targets in that batch. Refresh entry/exit skip whole-tree retries there; refresh exit still uses the existing sparse poison/lock refusal guard without retrying geometry; reservation-start checks defer optional sparse admission/refinement/cleanup while preserving accepted sparse coverage and intent. Backed mapping, required invalidation and mandatory replacement repair continue. Empty namespace retirement bypasses the optional-start budget only after its occupancy check; its RM verbs remain charged. It is not a new wall-time bound.

The manager retains at most one continuation per target identity. Its idle retry rotates among identities; one target that always needs another slice cannot monopolize all idle slices. Removing a target and abandoning a guest life prune this metadata. RM pressure/refusal awaits a later guest event rather than spinning; slice-limited or request-interrupted work re-arms an existing VA event. Refusal logging is bounded.

`Inbox` binds the RM slice's atomic interrupt. A split sets it under the existing queue guard. Idle admission clears it while holding queue-to-running guards in the existing order; a request after admission sets it again. Optional work stops between RM verbs; an already running replacement repairs its owner before stopping. The normal VA loop attempts one optional continuation only after split results, scoped wakes and statement settlement have been released, with no active/pending walk, armed invalidate or outstanding split. More continuation work re-arms the existing eventfd. No new timer, vCPU lock or GPU pause is added.

Optional progress assumes idle opportunities. Under continuous mandatory work, sparse intent can remain unhonoured and is reported through the existing bounded warning. Mandatory split requests for other spaces still share subsequent batches. An uninterruptible RM call and mandatory repair can still exceed a time slice; no hard total-apply deadline or Windows stall cure is established.

## Lock and mutation-domain handoff

Every `GpuMirror` row writer uses `RowsWriteGuard`. Wrapper drop clears its registered owner, then Rust field-drop order releases the real row lock before closing activity and calling the existing handoff hub. Error returns use the same sequence. There is no panic fallback or unsafe guard manipulation. PT and RAM holders retain their existing per-scope handoffs; end-loop handoff remains a fallback. Model M, migration and CreditWait semantics are preserved.

A newly queued or unresolved named-PDB split stays global. After the manager publishes its complete active/pending mutation union, admitted named-PDB tickets become `ResolvedPdb`. Queries retain the scope guard through split inspection, so resolution cannot be observed with an older union. Own-space targets and aliases remain held. `PDB_ALL`, unknown/global scope, CPU windows, overflow and a newer armed generation remain conservative. This helps unrelated spaces; it cannot release a ring whose own target is being walked.

## Native latency observations

`KF3_DIAG_LATENCY=1` enables native USER publications and Translated/private completion tails. It records worker demand/PUT observation, gate intervals and cause, successful publication, covering marker's first host observation, and sampled logical-vector notification opportunities on actual physical line-0 writes. Normal versus storm source, failed/masked signals, early opportunities and incomplete coverage remain explicit. A native publication is not a Windows DMA-packet identity, and a GPU-wide vector does not establish packet causality.

Current references and sequence fences carry no GPU timestamp. `marker_gpu_ns=None` means actual GPU-write-to-host-observation lag is unavailable. The real heartbeat loop gap, named RM control/verb scopes, row/PT ownership and worker/VA/act activity provide bounded context instead. Known-GPU-time simulation tests put a post-completion RM block in marker-to-raise; native records without that clock retain the ambiguity in publication-to-observed-marker.

The independent existing probe drains diagnostic metadata at 100 ms when latency alone is enabled, so a blocked worker can produce a first-slow dump. It does not poll GPU completion registers unless the separate completion probe is enabled. Limits: 512 pending/64 slow publications, separate 512 pending/64 slow splits, eight waits, four context spans, 64 threads/vectors and 256 events per atomic journal. Journal writers never wait or acquire a mutex. Metadata contention and overflow are counted. Later milestones refresh incomplete records and context; reset cancels old split bookkeeping without manufacturing completion.

Split records retain request, queue, successful walk start, apply end, readiness, result/gate release and worker consumption, plus token, requested scope, resolved last target, shared walk ID/count, wall time, RM deltas and diff/map counts. Shared-walk cost must not be summed per ticket. The [ten-line host runbook](../howto/ens11-host-runbook.md) specifies the A/B intervention and clock alignment. Host validation remains pending.
