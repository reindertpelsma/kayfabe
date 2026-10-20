# C6 integration conflict ledger

STATUS: LIVE, updated 2026-10-20.

MEASURED: C6 created the beta branch from candidate-5 `f8f9d1a5f` and has
published the ledger before any topic integration. There were no conflicts
at Stage 1; the subsequent resolved scope/context differences are recorded below.
No unresolved protected semantic conflict remains.

CODE READ: the topic queue and protected boundaries are in [TOPICS.md](TOPICS.md).
Potential overlaps are F-2/CE2 in `kf-chan/src/tmode.rs`, L5/current reset
interaction fixtures, and heartbeat/current GSP/drainer metadata. These are
review requirements, not a claim that a conflict was observed or resolved.

CODE READ: keep current behaviour unless the imported topic is explicitly its
replacement. Stop on any unresolved protected semantic conflict. Record the
source revision, affected hunk, competing contracts, concrete options and
written coordinator decision here before proceeding.


CODE READ CE2 integration, 2026-10-20: the source birth hunk's context named
`deferred_api_trigger()`, now replaced by this device's immutable
`me.settings.deferred_api`. The current profile selection is retained. Only
`ring.set_ce_object(implicit_ce_class(kernel_work, is_copy_engine(engine),
host.ce_object_class()))` is added at the same owned-ring construction site.
No unresolved semantic conflict: F-2 touches bound-class GR address widths;
CE2 adds an independently filtered default CE tier. Guest SET_OBJECT wins.
No USER-ring relay, scheduling, TSG, lazy-observe or pacing change is imported.


CODE READ L5 integration, 2026-10-20: the source tests' insertion context used
an older formatter; the two complete test blocks were inserted at the same
semantic anchors without changing assertions. No production conflict exists.
The typed ScrubError change retains C5's existing bootclean outcome/generation
checks; it changes only whether a stale-life refusal permits reseeding. No scrub
fixture/cache-key None change or surface-address import is present.


CODE READ heartbeat integration, 2026-10-20: source32eebb3ab's exhaustive FSM
fixture mentioned later reply-trace diagnostics absent from C5. Those diagnostic
fields are not imported. The new readiness field is added to C5's actual
exhaustive fixture and its equality assertion; all existing fields/assertions
stay. The production changes match the approved narrow series: constructor/fn72
readiness, fresh-queue clear, try-lock cell read, mapped timer and current-life
Device wiring. No unresolved reset/scrub/drainer semantic conflict remains.
