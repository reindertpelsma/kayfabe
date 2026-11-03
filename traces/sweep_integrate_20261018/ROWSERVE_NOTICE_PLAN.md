# Imported responder notice bound

STATUS: LIVE, 2026-10-19. GPU-free security correction required before acceptance.

CODE READ: notice() keeps a process-wide BTreeSet of text without a limit.
The refusal text includes the monotonic refusal counter, so repeated identical
guest requests insert fresh strings and emit fresh lines forever. This conflicts
with the hostile-guest bounded-state/emit rule; this topic cannot be accepted as-is.

H-N1: repeated same-cell refusals and changing guest branches each defeat the
old deduplication. Falsifier: the unchanged responder emits at most 64 notice
lines plus one limit marker for both 2048-request experiments. Tests exercise
the actual resolver in isolated child processes and require every response to
remain refused. Commit the two failing tests alone before the fix.

Fix: bound the existing notice set at 64 process-lifetime entries, then print
exactly one truncation marker; do not reset on guest policy/life changes. No
control reply, counter, host action, timing or protected relay behavior changes.
Mutations: remove the entry bound and renew the set per request; both tests
must kill each mutant at runtime. Check retained memory directly in a unit test.
