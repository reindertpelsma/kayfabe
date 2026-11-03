# Sweep integration plan

STATUS: LIVE, 2026-10-18.

Lane: sweep-20261018. GPU-free Phase 1 starts from candidate 4
0452613b5c71cb175271230b3e14c8e3c3eef2db in the isolated sweep worktree.
No historical evidence trees will be integrated. Historical hardware results are
STALE for the current integration line, including every historical PASS.

## H-I1 — stated before the integration preflight

INFERRED: gpu-types CODE can be brought onto candidate 4 without a conflict in
an owner-protected subsystem or a required change to channel birth/groups.
FALSIFIER: the merge preflight reports any conflict in pacing/tick source,
broker/cursor, USER relay/lazyobserve/scanoff, channel birth/TSG/latejoin,
doorbell/drainer, boot-layer scrub, lifecycle counters, or interrupt relay;
or code integration requires editing chan.rs groups, latejoin.rs, or
HostRm::birth_group/birth_member. Such a result triggers STOP AND ASK under
/root/codex_prompt_dies_20261018.md Phase 1.3; no automatic winner is selected.

Method: inspect Git's three-way merge result without applying it to the worktree.
This preflight may inspect historical evidence paths as merge inputs, but imports
none of those trees into this branch. Any conflict is retained as bounded text
with base/current/incoming object IDs and a file/hunk ledger. If clear, integrate
code/tests topic by topic, preserving current product behavior, then run the
required release tests, AD104 oracles, formatting/clippy and mutation checks.
Candidate 5 was observed on origin during discovery; refresh it before hardware.

All Phase 2 hardware cells remain UNRUN. No rental or owner-host access is
permitted during this Phase 1 preflight.
