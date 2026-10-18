# L4 soak attempt: stopped before host handoff

STATUS: LIVE, 2026-10-18. Evidence-only record; no product or harness changes.

MEASURED: Session start 01:52:38 UTC. Candidate-4 source is
`0452613b5c71cb175271230b3e14c8e3c3eef2db`, verified against origin.
The task was a serial candidate-5/candidate-4 Windows degradation reproducer,
using disposable overlays and capturing evidence at the first failure.

## Outcome

- MEASURED: The required L2 completion marker was absent at the initial check,
  at 01:53:23 and 01:55:23 UTC polls, and at 01:55:49 UTC verification.
- MEASURED: The upstream FAILED marker was read at 01:54:58 UTC; READY was absent.
  Origin advertised candidate-4 and no candidate-5 branch.
- CODE READ: L1's marker and completed report state its reviewed hitch source
  `b233037d95da2ae157cfcf50066fb500991c5b7b` failed the required default-off
  instrumentation gate: display step meters and event-ring recording are always
  active. L1 stopped before merges, candidate publication, binaries, or READY.
  L4 did not independently repeat that source audit.
- CODE READ: The common stop-on-blocked-step rule applies. L4 stopped early for
  the failed prerequisite; neither the 12-hour lane limit nor the marker-wait
  maximum expired. No unapproved build or C4-only substitute was used.
- MEASURED: No VM/GPU operation, guest command, GPU lock, build, or workload was
  started. The original owner QEMU PID 3087958 was live and still identified as
  run 3004 at 01:56:19 UTC. It was neither stopped nor restarted by L4.
- MEASURED: The owned marker poller was terminated with SIGTERM at 01:56:19 UTC;
  its shell completed with status 143. No target directory was created.

## Counts and limits

MEASURED: Before/after tests run by L4: 0/0 (not a passing test result).
Hardware arms/cycles/samples: 0/0/0; spread and time-to-event: unavailable.
Mutation table: not applicable; no behavior change and no mutants run.
No reproduction, comparative performance, reservation leak, recovery, or build
quality conclusion can be drawn. Restore was not performed: L2 never handed
over VM ownership, and the original owner VM remained running.

## Hypotheses registered before new observations

INFERRED H1: Resources accumulate across driver lives. FALSIFIER BEFORE RUN:
the same failure occurs before any reset, or controlled resets release resources
and preserve settled fd/thread/PSS counts without the failure over the observed
cycles. A clean finite run only bounds that exposure. NOT TESTED.

INFERRED H2: Fresh Heaven alone reproduces the slowdown. FALSIFIER BEFORE RUN:
all completed pre-reset Heaven phases retain at least 30 kms flips/s over every
full 60-second running interval, without a new failure event. NOT TESTED.

INFERRED H3: Degradation coincides with saturated System copy utilization and
idle 3D engines. FALSIFIER BEFORE RUN: System copy utilization is low at the first
event, or a different engine/process dominates. NOT TESTED.

INFERRED H4: C5 changes failure incidence/time. FALSIFIER BEFORE RUN: comparable
arms reproduce the same pattern with overlapping time-to-event spread; n=1 or
censored arms permit only descriptive comparison. NOT TESTED.

## Owner decision

INFERRED recommendation: fix and prove the source branches' default-off gates
with EXPECTED RED tests and mutation checks, rerun L1 and L2, then resume L4 on
their exact READY revision and completed ownership handoff. The owner may
instead explicitly amend the instrumentation requirement; L4 did not assume it.
