# Release resilience: a failed rental is replaced, it never means the test failed

**STATUS: DESIGN-ONLY for hardware, 2026-11-13.** The Worker change and the mock tests are written and GPU-free tested on branch
`claude/release-resilience-20261113` (draft PR against `next`); it is not deployed, not merged, and no box has run it. Parent documents:
[V3_RELEASE_MATRIX.md](V3_RELEASE_MATRIX.md) (the matrix stays `proposed`), [V3_RELEASE_REHEARSAL.md](V3_RELEASE_REHEARSAL.md),
[V3_CI_TIERS_AND_IGNORE_LADDER.md](V3_CI_TIERS_AND_IGNORE_LADDER.md).

OWNER REQUIREMENT (2026-11-13): "rentals can fail without meaning the test failed". A rental that fails, or a box that becomes
unavailable or dies mid-run, is recomputed and replaced on the fly while the other boxes keep running.

Markers: `[CODE READ]` is read from the source and not run; `[MEASURED]` is a result of the mock tests of this branch (a fake Vast, no box,
no GPU, no network); `[INFERRED]` is neither.

## 1. Falsifier (written before the tests)

The design is wrong if any of these can happen:

1. a failed or vanished rental yields FAIL;
2. a replacement ignores the die, the vendor requirement, the "recognised AMD/Intel `cpu_name`" rule, the ARM exclusion, or the price,
   slot and cost caps;
3. a replacement of lane A leaves lane B waiting forever or running on the same machine;
4. other healthy legs are cancelled, restarted or rented twice;
5. the re-run allowance is exceeded;
6. the release ends PASS while a required vendor, SPEC lane or die is uncovered;
7. the cost cap is bypassed by re-runs.

[MEASURED 2026-11-13] None of the seven happened in `test/release-resilience.test.js`; three further defects (section 3, rows a2, b2, d2) did
happen before the fix of this branch and are fixed.

## 2. The code path (what the source does)

[CODE READ: `src/index.js` `adminStart`, `tickBatch`, `admit`, `update`, `close`; `src/release.js` `classifyLeg`, `wantsRerun`,
`releaseVerdict`]

- Leg creation: `adminStart` re-plans (`planRelease`), refuses a plan that is not admitted, and writes one batch record with one PENDING
  leg per planned leg (12 with SPEC: 4 large dies with lane A and lane B, 4 middle dies). The plan fixes the die and the vendor pin
  (`cpuVendor`, set on at most one leg per required vendor); it does not fix an offer. The offer is chosen again at rent time.
- Rent: `tickBatch` (called by the cron sweep) walks the PENDING legs in `(phase, seq)` order and calls `admit` for each, with the leg's
  `avoidMachines` and `avoidHosts`, the vendor pin, the batch cost guard (`spent + cost > cap` is refused) and the slot caps (8 release
  slots, account cap 9). `admit` re-evaluates the whole market, so an offer that vanished is simply not chosen; a die with no admissible
  offer left gives `NO_ALLOWED_OFFERS`.
- A create that Vast answers with `success: false` is `CREATE_REFUSED`; `admit` closes the record as `PROVIDER_FAULT` with no instance and
  returns it. Any other create failure (HTTP 500, unparsable body, transport error) throws after the rental was recorded; the record then
  has no instance id and the cron reconciles it by label, or closes it as GONE/UNAVAILABLE once the boot limit has passed.
- Provider status and box report: `update` closes a box that never reported by the boot limit (`PROVIDER_FAULT`), a box whose provider
  state is not running for three readings with no box post (`ENDED`; a verdict of FAIL only because a first report exists), a vanished
  instance at once (`GONE`), a silent box at the heartbeat timeout and any box at the hard limit. None of these carries a result.
- Outcome: `classifyLeg` reads the Worker's own record. A record without a result is RETRY (reason: the record verdict). A result with only
  harness rows failing (`infrastructure`, `preflight`, `*_retry_*`, `provider_*`) is RETRY. A real failing row is FAIL on a GREEN host
  reference and UNATTRIBUTED otherwise. An owner stop is UNATTRIBUTED with reason `STOPPED`. A PASS needs the Worker's own PASS.
- Re-run: for a newly ended leg `wantsRerun` is true for RETRY and UNATTRIBUTED, never FAIL and never `STOPPED`, while the leg's depth is
  below `RELEASE_RERUNS_PER_LEG` (default 1). `tickBatch` then appends a PENDING copy: same die, family, tier, spec, phase, matrix row,
  `avoidMachines` = every machine any leg of that die used, `avoidHosts` = the failed host, depth + 1.
- Verdict: `releaseVerdict` is FAIL on any FAIL cell, RUNNING while a leg is PENDING or STARTED, PASS only when every cell passes, both
  vendors are counted from passing large legs (offered vendor equals the box's report) and the die set is complete; otherwise INCOMPLETE.
  A leg that ended RETRY next to its PASSING replacement does not block the cell.

## 3. What happens for each failure: before and after

"Before" is the code at the head of PR 361 (`389938621`) running the tests of this branch with the source change removed; "after" is this
branch. [MEASURED 2026-11-13] unless marked.

| # | Failure | Before | After |
|---|---|---|---|
| a1 | `create` refused (`success: false`) for one leg's offer | leg ends RETRY (PROVIDER_FAULT, no instance, cost 0), one replacement on another machine and host of the same die, vendor pin kept; the other 7 legs untouched; release RUNNING then PASS. Passed before the fix. | same |
| a2 | `create` answers HTTP 500 (unproven box) | [2026-11-13] the failing leg's record exists without an instance, but `tickBatch` stopped the whole rent loop (`break`): on a 12-leg release the 3 legs after it were not rented in that tick (4 of 7 rentals made). It is then replaced after the boot limit (75 minutes) because absence cannot be proven earlier. | the legs after it are rented in the same tick (7 of 7); the replacement still waits for the boot limit [CODE READ for the reason] |
| b1 | offers vanish between plan and rent | nothing to replace: the offer is chosen at rent time; no FAIL, no re-run consumed; lane B takes a surviving offer on another machine. Passed before the fix. | same |
| b2 | every offer of a pinned vendor vanishes from the pinned leg's die | the pinned leg waits for `NO_ALLOWED_OFFERS` until its start window ends (360 minutes), the release stays RUNNING and ends INCOMPLETE although other dies could cover the vendor | the pin moves to the next waiting large leg, or is dropped when running or passed legs already cover the vendor; the leg rents unpinned so its die is still covered; the release PASSes with both vendors confirmed |
| b3 | a die has no offer left at all | leg stays PENDING until the start window, then MISSING; release INCOMPLETE, never FAIL; other legs finish. Passed before the fix. | same |
| c | box never boots / never reports | closed at the boot limit as `PROVIDER_FAULT`, RETRY, one replacement; others untouched; one destroy. Passed before the fix. | same |
| d1 | box dies mid-run (provider `exited`, or the instance vanishes) | closed by the three-reading rule or at once; no result, so RETRY; one replacement; lane B waits for a free slot. Passed before the fix. | same |
| d2 | the die has one machine and lane A died on it | lane B (`reuseMachine`) rented the machine that had just failed lane A | lane B avoids the machine and host a non-passing lane A ended on: it is not rented again; with no other machine, lane A's replacement and lane B end MISSING after the window and the release is INCOMPLETE |
| e1 | harness failure row (`infrastructure`) | RETRY, one replacement. Passed before the fix. | same |
| e2 | real failing row on a host reference that is not green | UNATTRIBUTED, one replacement. Passed before the fix. | same |
| e3 | real failing row on a GREEN host reference (a guest FAIL) | FAIL, no replacement; the release is FAIL and stays FAIL after every other leg passes. Passed before the fix. | same |
| f | owner stop | open boxes closed, waiting legs MISSING, ended legs UNATTRIBUTED with reason `STOPPED`, no re-run, no rental, verdict INCOMPLETE. Passed before the fix. | same |
| g | the replacement fails again | one more replacement per allowance step, then the lineage ends RETRY: with `RELEASE_RERUNS_PER_LEG=1` a lane has 2 attempts, with 2 it has 3; the release is INCOMPLETE, never FAIL. Passed before the fix. | same |
| h | a replacement costs more than the plan | refused by the batch cost guard while it does not fit (the leg waits, `release-cost-cap`), rented when budget frees; the batch liability stayed at or below the cap at every observation. Passed before the fix. | same |

## 4. What changed (small, no redesign)

All in `src/index.js` `tickBatch` and `src/release.js` (Worker bundle files):

1. A create that failed after the rental was recorded no longer stops the rent loop (a2).
2. `movePin`: a pinned leg that finds `NO_ALLOWED_OFFERS` gives its pin to another waiting large leg that has not already failed to find
   that vendor, or drops it when `vendorCover` shows the other legs already cover the vendor (b2).
3. `pinForRerun`: a replacement keeps the failed leg's pin, or pins the vendor the failed leg rented when no other leg still covers that
   required vendor (the rule is re-applied to the replacement).
4. Lane B adds the machine and host of a lane A that ended RETRY or UNATTRIBUTED to its own avoid lists (d2).

`started.cpuVendorOffered` is also kept on the batch leg at rent time so `vendorCover` can count running legs.

## 5. Test map

`test/release-resilience.test.js` (28 tests, one 12-leg release each unless stated), helpers in `test/release-resilience-fixture.js`
(a market of four offers per die on four machines and hosts) and two switches in the mock Vast (`test/helpers.js`: `refuseAsks`,
`errorAsks`, `hiddenOffers`). Each failure test asserts the verdict (never FAIL for a rental or harness fault), that exactly the failed leg
has one replacement of the same die, spec and tier on a fresh machine and host with a recognised vendor and the pin's vendor, that the other
legs keep their open record, instance and machine and are never destroyed or re-run, that at most 8 records are open at every observation,
and, where the release completes, `assertMatrixCovered`: one large die per family with both SPEC lanes passed on passing legs, one middle
die per family with a passing leg, every matrix receipt passed, both vendors counted on passing large legs. The checker is itself checked on 2026-11-13
(damaged copies must fail it and the judge). `release-resilience-mutants.mjs` has 17 mutants, each killed by at least two tests.

## 6. Cost, and the value of `RELEASE_RERUNS_PER_LEG`

[CODE READ: `defaults.json`] `releaseRerunsPerLeg` is 1 and `releaseCostCap` is 40; DEPLOY.md suggests a cap of 30.

[MEASURED 2026-11-13 on the synthetic fixture market, `adminPlan`] the 12-leg plan is 25.6071 USD at worst case (every large leg priced at the
300-minute limit, middle legs at 240). Largest single leg 4.4243 USD (AD102 lane B), smallest 0.8665 USD.

- If every leg were replaced once at its own price, the worst case would be 51.2142 USD (2 x 25.6071). [INFERRED from the sum: no test
  replaces all 12 legs]
- The cost guard refuses a rental when settled spend plus open liabilities plus the new rental's worst case exceed the cap. [CODE READ;
  MEASURED 2026-11-13 for the refusal and for liability at or below the cap in `re-runs never pass the release cost cap`] The unrealised exposure
  is therefore bounded by the cap, not by the number of re-runs: 30 USD leaves 4.39 USD above the plan at plan time (one replacement of
  an average leg), 40 USD leaves 14.39 USD (about six). A failed leg frees its liability and keeps only its actual cost, so the real room
  is larger when boxes die early. [INFERRED]
- The ledger's daily, rolling and account caps apply to every rental on the same admission path. [CODE READ]

Recommendation (the owner decides): keep `RELEASE_RERUNS_PER_LEG=1` and let the cost cap bound the extra rentals. Two points the owner may
want to weigh: (1) a refused create costs nothing but consumes the leg's single re-run, so two consecutive refused creates on one lane
end it INCOMPLETE although no box ever ran (measured 2026-11-13 with every offer of a die refused); exempting `noInstance` faults from the depth
count would need its own bound to avoid an unbounded loop and is not done here. (2) A lane that hits the hard limit is also a RETRY and
is re-rented; the cap bounds it.

## 7. Not tested

- Real Vast behaviour: the mock answers `success: false` with HTTP 400 for a refused create, and models an unavailable box only as
  provider status `exited`, an absent instance, a missing report or a stale heartbeat.
- All 12 legs failing at once, more than 4 simultaneous failures, and failures spread over more than one tick of the cron with a restart of
  the Worker between them (`h.restart()` is not used in these tests).
- A vendor pin that moves repeatedly across several dies in one run (only one move is exercised end to end; the rest is unit-tested).
- The daily rental count and the rolling and account USD caps as limits on re-runs (only the release cost cap is exercised).
- Any run on hardware, the payload, the reporter and the deployed bundle.
