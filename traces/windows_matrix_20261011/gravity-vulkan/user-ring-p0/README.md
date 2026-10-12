# Windows user-ring P0

**STATUS: RESEARCH, 2026-10-12.** Native permission experiment prepared; results
pending. This is not a Windows migration implementation or GravityMark PASS.

See docs/design/V3_WINDOWS_USER_RING_RELAY_REVIEW_20261012.md for the source
review, owner-approved restrictions and hypotheses/falsifiers written before
hardware. Owner condition: Linux Passthrough forbids host-owned mappings.

Probe: kf-harness/bin/kf-user-ring-probe, read-write then read-only then
write-attempt, each isolated uid65534 process under shared GPU lock. The two
positive cases differ only in GPU mapping permission; the negative case attempts
a GPU store into the read-only command mapping. No VM or personal disk is used.
No product behavior changes. Exact source/binary and journal evidence to follow.
