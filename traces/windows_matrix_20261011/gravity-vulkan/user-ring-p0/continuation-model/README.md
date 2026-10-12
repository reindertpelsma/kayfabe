# Bounded user-stream packet continuation

**STATUS: RESEARCH, 2026-10-12.** Unconnected parser prerequisite; no GPU mapping,
submission, source retirement or migration implementation.

Failing-first tests require metadata across GP snapshots before the implementation
exists. Full kf-chan suite passes afterwards, including14 stream cases. Every split
and one-word fragments produce the same IR/final bindings as a full snapshot.
Pending zero/header-shaped data remain data, all guest references stay bookkeeping,
malformed suffixes restore consumed pending metadata and bindings transactionally,
and method overflow is rejected at header admission. No payload buffering grows
across fragments. Public C56F counts bound metadata; trusted Commands budget bounds
allocation/output per feed. Explicit finish refuses a dangling packet; idle is not
end-of-stream and no timer completes/releases it.

Independent review_late_fifo_probe found no parser blocker and requested the added
rollback/reference-data/final-state assertions. Native MAIN-GP-entry continuation
semantics, source pins, software-action ordering and SYNC_WAIT remain adapter gates.
Previously accepted fragments cannot be rolled back after a later refusal: stop
and quiesce a refused live stream; never raw-forward or fake a GPU completion.
