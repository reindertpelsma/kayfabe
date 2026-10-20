# Translated slot contention — 2026-10-21

STATUS: RESEARCH; GPU-free HW3 lane, based on M (`89fc8a222`).

CODE READ: the worker's Translated `serve` returns on a lost slot `try_lock`
without a durable re-ring. The drainer's group selection, token query and
schedule update block on the same slot. Act retirement and worker pumping can
hold that slot across host calls. These observations do not prove the remaining
Windows freezes have this cause.

Hypothesis H1 (before tests): a lost slot attempt can strand queued work when no
new external hint arrives. FALSIFIER: forced contention followed by holder
release always gives a durable ring and bounded worker-pass progress without a
new doorbell, engine edge or timeout.

Hypothesis H2 (before tests): a busy slot blocks drainer metadata and schedule
processing. FALSIFIER: those operations finish or defer while the holder remains
held and cannot delay an unrelated drainer request.

Preparation: extract the existing lock/try-lock and drainer access behavior to a
generic production cell so fake payload tests execute the adapter's real locking
code. This is a behavior-preserving preparation, before EXPECTED RED tests;
no fix is included. Token/group identity is immutable after birth (CODE READ).
