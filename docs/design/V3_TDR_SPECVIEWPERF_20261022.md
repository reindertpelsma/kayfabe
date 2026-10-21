# Windows SPECviewperf TDR loop, run 3440

**STATUS: LIVE-RESEARCH, 2026-10-22. Lane T1.**

## Scope and provenance

CODE READ: research branch starts at `origin/master` `e64b8e5f6`.
The lane identifies the measured VM binary as `89fc8a222`; conclusions about
that binary must be checked against that revision. Host access is evidence-only:
no guest-agent calls, VM/GPU/broker actions, or lock operations. No closed-binary
reverse engineering. Public evidence contains only bounded, sanitized text.

MEASURED: lane start 2026-10-21 23:19 UTC; deadline 2026-10-22 02:19 UTC.

## Preregistered hypotheses and falsifiers (before evidence inspection)

The initial ordering is provisional; no log-based conclusion has been drawn.

1. **H1 — memory reconciliation/invalidate progress blocks kernel channels.**
   FALSIFIER: complete per-trigger and per-channel observations show all relevant
   invalidates cleared, walk busy released, and kernel work completed before each
   application failure. Missing logs cannot falsify this hypothesis.
2. **H2 — an RM-life transition retains unusable channel/VA state, amplifying
   recovery failures.** FALSIFIER: an identity-complete census proves all old-life
   nonpersistent channels/bindings/leases retired and new-life operations succeed
   with distinct valid identities through repeated failures. Persistent T-space
   infrastructure alone is not evidence of a defect.
3. **H3 — a notification/relay publication is lost, including a Translated
   channel outside the USER-ring progress watcher.** FALSIFIER: every failing
   channel's recorded doorbell reaches a GP_PUT read and publication, with a real
   execution witness and required guest interrupt delivered; GP_GET alone is
   insufficient proof of execution. Do not add GP_PUT polling as a cure.
4. **H4 — workload/driver failure also occurs without kayfabe.** FALSIFIER:
   matched repeated fresh-VM catia-07-only failures under kayfabe and successful
   VFIO runs with the same guest image, driver, viewset and settings, plus an
   identified kayfabe progress defect. A single successful control is suggestive,
   not a general exclusion of intermittent driver failures.

## Investigation and diagnostic plan

INFERRED: the cheapest next evidence is the existing three-minute pre-failure
window, followed by life 3 versus life 9. Align host local time, guest time
(lane says host minus two hours), and monotonic memory timestamps explicitly.
Record counters as deltas with their reset scopes, distinguish log throttling
from absence, and preserve source line numbers.

INFERRED: minimal diagnostics should be default-off, bounded, observational,
and worker/VA-thread owned: Translated-channel last doorbell/GP_PUT/GP_GET and
execution-witness state, invalidate-pending age, and walk-busy age. No blocking
on a vCPU or lock it takes, no changed waits or completion policy. Detailed
TEST-FIRST acceptance cases will follow code inspection. Any implementation
must follow this pushed design, a separate EXPECTED RED test-only commit,
green fix commit, at least two killing tests per mutant, and zero added clippy
diagnostics. Hardware testing is outside T1 authorization.

## Findings

Pending evidence inspection. No causal verdict or runtime change yet.
