# Native USERD reference discriminator

**STATUS: RESEARCH, 2026-10-12.** Prepared, not run yet. No VM relay or app claim.

Hypotheses/falsifiers precede hardware in
`docs/design/V3_WINDOWS_USER_RING_RELAY_REVIEW_20261012.md`, USERD reference section.
PublicC56F only: read-only ring, unmapped USERD, USER channel, virtual4096-byte CE
copy then WFI_ALL/SET_REFERENCE. Identical immediate/held commands; only the
acquire fixture's initial value/publication time differs. The50ms hold is an
explicit bench input; no product gate/timeout release. No writable private GPU
completion mapping. Capture source/SHA/PID/time/journal/cleanup with run_reference.py.

GPU-free probe4 tests and full kf-abi suite PASS; logs alongside. These additions
are bench-only plus unused public-header ABI constants. Exact70aa Linux regression
covers the unchanged VM runtime; this revision has no Windows hardware result.
