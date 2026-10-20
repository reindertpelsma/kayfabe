# C6 integration conflict ledger

STATUS: LIVE, 2026-10-19.

MEASURED: C6 created the beta branch from candidate-5 `f8f9d1a5f` and has
performed no topic merge or cherry-pick. There are no resolved or unresolved
merge conflicts at the Stage 1 ledger checkpoint.

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
