# Two exact Phase 1 dependencies

STATUS: LIVE, 2026-10-19. Both decisions APPROVED by coordinator/owner.

2026-10-19 supersession: isolated heartbeat series approved with fresh EXPECTED
RED, >=2 runtime killers per mutant, full release counts, clippy/fmt. Hard rule:
no waiting on vCPU locks, no blocking heartbeat clock action, period stays 500ms.
The old proposal's blocking GSP lock and cloning/locking latest() read are rejected
inside the approved implementation; bounded borrowed try-lock reads replace them.
The additional current-life fn72 bit is clock authorization metadata only: the
existing reset constructor clears it without changing console cell retention,
production scrub, channel birth, relay, pacing or lifecycle counters.

Fixture-only None edit APPROVED and committed as 94518f95b. The complete QEMU
release suite now compiles and passes 389 tests. Decisions3-7 accepted: exact
die+branch caps scope; frozen180/no R34 lease; public derived page, no captured
page shipping; proposed new PR stack based on candidate5, no retargeting old PRs;
windows-on-vast unmodified. The recommendations below are historical, resolved.


Recommendation1: approve the isolated heartbeat clock topic with fresh failing-first
coverage, >=2 killing tests per mutant, release counts/clippy/fmt, as for the family
helper. Exact patch: heartbeat_drainer_PROPOSAL.patch (unapplied after corrective
removal). Existing 500ms period stays unchanged. Alternatively explicitly defer
this topic and retain candidate-5 clocks for hardware; do not claim B3 fixed.

Recommendation2: approve only adding None to the cache-key tuple in the existing
life-reset/scrub test, as shown in scrub_test_address_key_PROPOSAL.patch. This
repairs its type against Blackwell address-aware caching while retaining every
assertion. No production scrub or pacing edit. Current failing compiler output
is the falsifier, at display.rs line6420 in checkpoint30a43bc9d.

An earlier audit classified Device by pathname and missed the heartbeat's actual
drainer hunk. That mistake is corrected in code and the ledger. Neither proposal
uses the host; no hardware has been rented.
