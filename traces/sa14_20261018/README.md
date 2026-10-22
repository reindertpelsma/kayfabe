# SA-14: DEFERRED, tests only

MEASURED: e7462af82 imports the audit test unchanged in an EXPECTED RED commit;
it fails by assertion on master. Two more tests fail by assertion when a later
latch-only write changes aperture or an untriggered write changes the raw scope.
Whole kf-trap release result111 passed /3 expected assertion failures /0 ignored.
There is NO product fix, green acceptance or behavior-change mutant claim.
Clippy112 ->112 source fingerprints, added0 vs unchanged-master product baseline.
No assertions, lint baselines or deny sets were relaxed.

INFERRED hypothesis registered before runs: armed_request must bind the complete
arm-time request to its sequence. FALSIFIERS: later latch writes change any field,
a mixed publication is accepted, reset exposes old work, or a stale completion
clears a later arm. The three red tests establish the sequential defect only.

CODE READ: armed_request reconstructs PDB, aperture and raw scope from live cells.
Trigger::arm_next explicitly permits multiple vCPUs and uses fetch_max to retain
the later sequence. Guest root is untrusted; guest-driver serialization is not
a host guarantee. Copying snapshot fields into separate atomics is insufficient:
A begins publishing; B publishes all its fields and later sequence; A resumes
and overwrites one field. B's sequence can be stable before/after a mixed read.
A seq-last store or compare/exchange does not undo the earlier data overwrite.
A reader-only double-check therefore does not prove coherent publication.

CODE READ: a writer mutex/spinlock violates the no-blocking vCPU rule. The complete
request (PDB/aperture, raw32-bit word, full sequence) does not fit one64-bit atomic.
Discarding unknown raw bits changes the documented diagnostic contract; reducing
the sequence to a small wrapping tag creates a stale-completion/ABA obligation.
No new unsafe code or dependency/perimeter relaxation is proposed here.

INFERRED options for owner/reviewer:
1. Recommend an immutable request publication protocol, with a bounded queue or
   per-vCPU ownership and an explicit nonblocking overflow/refusal/fail-closed
   policy. Prove arm-before-publication, multiwriter order, reset and stale clear.
2. A wider-atomic implementation belongs in an approved existing platform
   perimeter and needs architecture support/fallback proof. Do not place it here
   by relaxing the unsafe-file rule.

CODE READ decision: the lane forbids changing production VA behavior without
clear proof. Neither a multiwriter-safe snapshot protocol nor a new overflow
policy is proved/authorized by the existing port contract; do not ship a
sequential-test-only copy fix. The finding remains DEFERRED, not REFUTED.
Hardware validation later cannot substitute for the missing concurrency proof.
All test/lint runs used the shared owner lock, release, two jobs, nice19, own target.
