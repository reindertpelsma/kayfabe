# Placement: why the Worker prefers US offers (and a US ledger)

STATUS: LIVE for the offer preference (2026-10-24, owner ruling). DESIGN-ONLY for the ledger
location (not implemented; waiting for the owner's nod).

This is a decision record. It exists so that a reader who has not seen the conversation can
argue for the US preference, or argue it away, from the evidence below. Each claim is tagged
`CODE` (read in the repository), `OWNER` (stated by the owner), `INFERRED` (reasoning, not
tested) or a dated run record where one exists.

## Decision

1. Among offers that are already eligible, rank offers located in the United States first and
   Canada second. The preference is a tie-break, never a filter.
2. Do not reject an offer because it is outside the US or Canada. The only location filter is
   the denied-country list (CU, IR, KP, SY, CN, HK, MO), which is an ownership/abuse rule and not
   a performance rule.
3. (Proposed, not built) Create the Durable Object ledger in eastern North America with a
   `locationHint`, for the same reason: the traffic that matters is US-centred.

## What the code does today

- `CODE` `ci/vast-broker/defaults.json`: `preferredCountries: ["US", "CA"]`, overridable with the
  `PREFERRED_COUNTRIES` variable; `deniedCountries` is a separate list.
- `CODE` `ci/vast-broker/src/offers.js`: `countryRank` gives an offer its index in
  `preferredCountries`, or the list length when absent. Eligibility (price, reliability, network
  thresholds, verification, die, denied country) is decided first; the rank only orders what
  survives.
- `CODE` network eligibility is already a speed-threshold gate (download and upload Mbps, bandwidth
  per GB), so a slow US host is rejected and a fast non-US host is accepted.

## Why prefer the US

- `OWNER` The box downloads the SPECviewperf package and other software during setup. The
  owner's reasoning is that these sources, and the supply of suitable offers, are US-centred.
- `OWNER` GitHub Actions runs in the US and its artifacts are US-hosted. The runner, the Worker's
  edge and the box all talk to GitHub, so a US box has the shorter path to all three.
- `OWNER` The location of the author is irrelevant to an automated pipeline. Nobody sits at a
  box; only the path between the box, GitHub, the Worker and the download servers matters.
- `MEASURED` The one real VM-mode payload run recorded before the Worker work (PR #111 report,
  2026-10-23) spent 4690 s provisioning: apt 53 m 52 s, benchmark dependencies 15 m 04 s, host
  driver 3 m 57 s. Setup is download-bound, so anything that shortens the download path pays
  back many times over.
- `INFERRED` Distance, mirror choice and the host's own network all feed that number. The run
  does not separate them, so it supports "setup is download-bound" but not "a US box would have
  been faster".
- `INFERRED` The US has the largest Vast supply, so ranking it first rarely costs an eligible
  offer. This has not been counted; the availability output of a run lists country-rejection
  counts only for denied countries.

## Why a rank and not a filter

A hard US-only filter would throw away eligible, cheaper or faster boxes and would make the
Worker unavailable whenever the US has no qualifying offer. The network thresholds already
reject bad paths; the rank only decides between offers that all passed them.

## What would change the decision

- `FALSIFIER` Record setup time per box with its country over at least ten runs. If US and
  non-US boxes that pass the same thresholds show no difference in setup time, drop the
  preference (set `PREFERRED_COUNTRIES` to `[]`; no code change).
- If the download sources move to a non-US CDN, change the order, not the mechanism.

## Ledger location (proposal)

- `CODE` The ledger object is addressed with `idFromName('owner-shared-vast-account')` and no
  location hint, so it lives wherever Cloudflare first created it, near whoever made the first
  request after deploy. Which location that was is not visible in the logs.
- A `locationHint` only applies when an object is first created, so the existing object cannot
  be moved. The fix is a new object name with `locationHint: 'enam'` (eastern North America,
  closest to GitHub's runners). The old ledger holds no rentals yet, so the cost is one source
  change, a new bundle hash in `deployment.json` and a redeploy. After the first rental the
  ledger holds spend counters and the move would need a migration, so do it before renting.
- `INFERRED` A European object adds roughly 80-100 ms to every authenticated call. Correctness
  does not depend on it; only latency of `/availability`, `/status` polling and box reports.
