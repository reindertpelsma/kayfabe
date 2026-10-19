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
