# Shadow diagnostic source review

STATUS: REVIEWED, 2026-10-12 20:28 UTC. Independent existing reviewer
review_late_fifo_probe, second pass after actual failing-first regressions.

Two original blockers: strict GP writer could still reject a descriptor the
observer was meant to log; failed publication retained advanced parser history
for a retry of the same GP. Both corrected. Separate write_shadow_gp checks the
private slot/live storage and preserves the u64; strict writer remains unchanged.
finish_publication(false) discards all semantic/address history, keeping quota
and attempt counts. Unknown history precedes and prevents all subsequent CPU PB
reads. No CPU API action, staging or witness route is reachable from shadow_entry.
Private fetch/guest GET receipts remain actual native descriptor consumption.
Logs include guest key, native channel/token, source epoch/generation and issued
sequence; mutable observations/attempts are not hardware execution evidence.

No remaining source blocker for the default-OFF exclusive single-VM diagnostic
after tests and exact Linux gates. Cross-VM reset availability is not established.
Compile-only log field corrected from handle to chan before final checks.
