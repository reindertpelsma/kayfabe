# VA-4: root-move invalidate regressions; fix deferred

STATUS: RESEARCH, 2026-10-22. Base origin/master e64b8e5f6.

INFERRED hypothesis, registered before inspection: retain the original object
keys when retrying a moved-root invalidate. FALSIFIER: an original request clears
without reconciling its current objects, or a hard unmap/invalidate failure is
accepted. The hypothesis was not established for root withdrawal (below).

MEASURED: the tests-only EXPECTED RED commit 79600196f fails all three new tests
on unchanged master product code: named PDB, ALL_PDB, and two successive root
moves. Each requires the original trigger to remain busy until a current-root
report applies, without an intermediate unreconciled disposition. No passing
existing test was loosened.

CODE READ: on_walk_ready queues a root follow-up after a moved root, but drops
the original invalidate into unreconciled. Retrying only by its old PDB can
instead resolve nothing and clear vacuously. A private experiment retained the
original keys and passed the 189-test kf-mem suite (3 existing ignored replays),
but that result is NOT acceptance: its tests omitted the following real producer.
The experimental product change was rejected and restored out of this branch.

CODE READ counterexample: VasTable::clear_root explicitly keeps host mappings.
The production MemStatement::UnsetPageDir arm in kf-qemu/src/mem.rs calls it and
notes that UVM frees the directory immediately after the reply: it must not be
walked again. The proposed retry filtered keys by root().is_some(); after this
withdrawal, an empty retry scope could clear while the old host mappings remain.
Reading the withdrawn directory again is also forbidden by that producer's
lifetime. This blocks a universal retry-to-clear proof.

CODE READ disposition: DEFERRED under the lane's instruction to stop a VA topic
without a clear production proof. No product change or mutant is published here;
no fixed/green claim applies to this branch. Baseline Clippy completed; a fix
Clippy delta and mutation campaign are inapplicable after rejecting the fix.
The red tests are review inputs, not candidates to merge on their own.

INFERRED options for the owner:
1. Recommended: specify a bounded root-withdrawal operation that retires this
   object's host placements and invalidates before any original request clears,
   with GPU committed-placement state kept consistent. Then prove moved,
   withdrawn, removed and re-created identities together in executable tests.
2. Retain the current named held failure for withdrawn roots while fixing only
   stable rooted moves. This narrows the fix but leaves AD's guest-wide busy
   problem unresolved; it is not a universal retry policy.
3. Do not re-read the freed directory or clear blindly over the old mappings.

INFERRED later hardware obligations: UVM root migration/unset/retirement,
Linux/Windows trigger-to-clear latency and absence of stale translations.
No hardware run occurred (n=0, spread N/A).
