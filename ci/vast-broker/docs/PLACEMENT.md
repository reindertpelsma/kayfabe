# Placement: why the Worker prefers US offers (and a US ledger)

STATUS: LIVE for the offer preference (2026-10-24, owner ruling). Ledger source implementation
tested locally on 2026-10-25; NOT DEPLOYED, owner acceptance pending. Live ledger placement
remains unmeasured. Local evidence: [WD2](../../../traces/worker_placement_20261018/WD2.md).

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
3. Request initial placement of a new Durable Object ledger in eastern North America with
   `locationHint: 'enam'`, for the same reason: the traffic that matters is US-centred.
   This branch implements that request in source; it has not changed the live Worker.

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

## Ledger location (source implemented; deployment pending)

- `CODE READ` Before this branch, both HTTP forwarding and cron addressed
  `idFromName('owner-shared-vast-account')` without a location hint. Its actual placement
  remains unmeasured; this task did not contact Cloudflare or query either live ledger.
- `CODE READ` The shared `placedLedger` accessor in `src/index.js` now selects
  `owner-shared-vast-account-enam-v1` with `locationHint: 'enam'` for both HTTP and scheduled
  cleanup. The binding, class and storage schema are unchanged. There is no fallback to the
  old identity, deletion of its contents or migration in this change.
- A `locationHint` applies to initial creation; it cannot move the existing ledger's contents.
  It requests eastern North America, rather than proving an actual runtime location or latency.
- `OWNER REVIEW REQUIRED` Before deployment, the owner/controller must review the old ledger
  for active state and spend and decide how to handle any remaining liabilities. The earlier
  assertion that it held no rentals is withdrawn: emptiness is not established by this task.
  Do not deploy the new identity on an assumption that the old ledger is empty. No live review,
  reset or migration was performed here. Renting remains disabled in the prepared configuration.
- `MEASURED` Local gateway/cron tests and mutation checks are recorded in
  [WD2 evidence](../../../traces/worker_placement_20261018/WD2.md). These establish source behavior
  against fake namespace bindings; deployment, owner acceptance and live placement remain pending.
- `INFERRED` A European object adds roughly 80-100 ms to every authenticated call. Correctness
  does not depend on it; only latency of `/availability`, `/status` polling and box reports.
