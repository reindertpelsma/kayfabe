# Heartbeat nonblocking clock topic

STATUS: LIVE, 2026-10-19. Coordinator approved this isolated drainer topic and
only None in the scrub fixture. Candidate5 f8f9d1a5f refreshed before changes.

MEASURED (2026-10-19) pre-topic kf-qemu full release: 389 passed, zero failed/ignored,
including every candidate4 and candidate5 test; prior scrub fixture E0308 fixed
by 94518f95b, with every assertion retained. No production scrub edit.

Test-first sequence: a behavior-preserving extraction of candidate5 arithmetic
and initially unused API seams, then a tests-only EXPECTED RED commit, then fix.
The temporary serialized seam is never used by hardware; it exists to make the
lock-contention falsifier execute rather than treating a missing API as a kill.

FALSIFIERS (before runs): wrong guest offset epoch or separate-division result;
publication without clock/fn72; an old fn72 reused after reset; failure to return
while another thread holds the GSP or fn72 lock; source call path using an RM
ioctl or a blocking lock. The timer remains 500ms. Only bounded mapped-register
reads, try-lock metadata access and shadow writes enter the new heartbeat path.
Tests use a 250ms watchdog with explicit lock release and thread join, so a
blocking mutant fails without leaving a hung test process.

Mutation acceptance: each changed clock/refusal/locking/lifetime behavior must
have >=2 distinct runtime failing tests per mutant; compilation errors do not
count. Full affected release suites, all touched crate acceptance, clippy zero
new diagnostics on changed lines, rustfmt; preserve all existing tests. Host
remains off-limits. All hardware remains UNRUN at this revision.
