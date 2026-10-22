# AT-1: stale tail can poison a nonfull PrivRing

INFERRED hypothesis, registered before the probe: reading head before tail prevents
an old tail/new head pair from wrapping the occupancy calculation. FALSIFIER:
either scheduled interleaving poisons a nonfull ring, or a genuinely full ring
stops poisoning without claiming a slot.

MEASURED unchanged-master baseline:1 passed /2 assertion failures. The integration
test compiles the actual ring source with only its atomic import substituted by
safe wrappers. A one-shot pause after the tail load allows another producer and
the real consumer to finish1 or17 writes. Master falsely returns Poisoned;
head-before-tail instead retries the existing CAS and queues correctly. There
are no production test hooks, conditional branches or new unsafe code.

MEASURED whole kf-trap release suite:114 passed /0 failed /0 ignored. Restoring
the old load order kills both schedule regressions; restored fix3/3 passed.
Clippy all targets112 ->112 source-bound warning fingerprints, added0.
No tests, deny sets or lint baselines loosened. Source and tests are from public
repository code and this lane's deterministic schedules.

CODE READ: this changes only the two acquire-load positions before reservation;
the existing cursor CAS, slot publication, full-ring poison and drainer commit
remain intact. It removes the stale-tail/new-head underflow window. This does
not claim to eliminate every conservative full observation under preemption.

INFERRED hardware obligation later: SMP privileged-register bursts, boot/reset,
and scheduling under nested virtualization. The source-instrumented schedule is
a falsifier; it is not an exhaustive weak-memory or hardware proof.
