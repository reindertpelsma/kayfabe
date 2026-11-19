# SA-03 / VA-2: ABI-bounded VA walk batches

STATUS: RESEARCH, 2026-10-22. Base: origin/master e64b8e5f6.

INFERRED hypothesis (before the runs): successive walks of at most 64 objects
can retain each original completion obligation. FALSIFIER: an omitted object,
an early clear/split completion, a forgotten hard failure, a lost async target,
or unsubmitted old-life work surviving reset. A root move must still prevent
a clear over a report that no longer describes that object.

CODE READ: master resolves ALL_PDB to every rooted object, then refuses the
whole batch above MAX_SPACES_PER_WALK=64. The production walker has 128 slots.
The change retains all original wants, partitions the captured object/slot/root
set, applies and acknowledges each chunk before submitting the next, and only
then evaluates completion across the original objects. Hard failures, partial
verdicts and rewalk obligations survive chunk boundaries. Async settlement is
checked across all original keys. Reset discards unsubmitted chunks and their
original obligations while allowing the submitted GPU report to land. Earlier
root identities are checked again before the final clear. No timer, ABI bound,
unsafe surface, guest page-table CPU walk or production flag changed.

MEASURED: tests-only EXPECTED RED commits 50206fb3f and 64489a633 preceded the
product fix. Final focused baseline: 0 passed / 13 failed / 0 ignored. The audit's
original test is copied unchanged; the added public-API walker/target tests cover
70 and 128 objects, async settlement in the first chunk, a newer invalidate,
hard invalidate refusal in either chunk, reset before/after the first report,
split tickets, and root moves in earlier chunks.

MEASURED: release/locked kf-mem tests after the fix: 199 passed / 0 failed /
3 existing ignored debug-replay tests, across six summaries including doc tests.
After all mutations were restored: 13 focused tests passed / 0 failed / 0 ignored.
All Cargo commands used the owner's shared lock, own target directory, nice 19,
and two jobs; disk guards preceded each command.

| Hand mutation | MEASURED killed tests |
|---|---:|
| Restore master's >64 refusal | 13 |
| Finalize after the first chunk | 11 |
| Retain the old multi-chunk obligation on reset | 2 |
| Omit the final root-identity check | 2 |

MEASURED: Clippy --all-targets had 136 source-bound diagnostic fingerprints before
and after: added 0, removed 0, using scripts/ci/debt.py. The new test files have
no warning fingerprints. No existing passing test was loosened. Before the last
EXPECTED RED commit, an uncommitted assertion requiring the separate VA-4
unreconciled outcome was removed: the root tests require no early clear and a
follow-up walk, without prescribing VA-4's defective final disposition.

CODE READ limitation: VA-1 remains DEFERRED. kf_walk.cu's report assembly sets
run_count=0 on aggregate output overflow. WalkKernel::capacity_retry explicitly
cannot repair report-level truncation; GpuWalker::poll rejects it, without
applying any unmaps. Clearing that report could leave stale host mappings,
contrary to owner AA. Retrying an identical oversized single-space diff cannot
prove progress. Recommendation: GPU-side bounded diff/ack chunks, or an explicit
owner-approved teardown/refusal policy; never a larger timeout or blind clear.
Root-move trigger retry (VA-4), failures of host unmap/invalidate and walker-slot
exhaustion are also separate obligations; this change does not claim to fix them.

INFERRED hardware obligations (not run; n=0, spread N/A): validate real walker
commit-on-ack across chunks, 70+ process contexts including CPU apertures, split
completion, reset, Linux/Windows boot/work, and trigger-to-clear distribution
against owner AD's roughly 12 ms requirement. A bounded object count is not a
measured latency guarantee. No hardware claim is made by the model tests.
