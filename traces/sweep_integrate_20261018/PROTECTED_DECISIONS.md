# Two exact Phase 1 dependencies

STATUS: LIVE, 2026-10-19. Pending coordinator decision; other work continues.

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
