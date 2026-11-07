STATUS: DESIGN-ONLY, 2026-11-07. Proposed download model and feasibility limits;
GPU-free implementation/tests only, with no hardware calibration or deployment.

# Download time in effective price

OWNER IDEA (2026-11-07): count the value of lease time spent downloading beyond a
1 Gbps reference, and reject links whose expected download time is infeasible.
The existing transfer allowances are small overestimates, not measured traffic.

CODE READ: policy choices are data in `ci/vast-broker/defaults.json`:

| Field | Value | Status |
|---|---:|---|
| referenceDownMbps | 1000 | owner reference |
| derate | 0.3 | INFERRED |
| small maxDownloadSeconds | 3600 (60 min) | PROPOSED by J2 |
| middle maxDownloadSeconds | 1200 (20 min) | owner proposal |
| large maxDownloadSeconds | 3600 (60 min) | owner proposal |
| adhoc maxDownloadSeconds | 1800 (30 min) | PROPOSED by J2 |

MEASURED HISTORICAL SOURCE: [the SPEC transfer](../../traces/specviewperf_vast_20261105/README.md)
records 42,002,015,727 bytes in 804 seconds, approximately 418 Mbps. Its matched
offer advertised 613 Mbps, giving an approximately 0.68 observed ratio. The
owner separately reports a current 1562 Mbps advertisement and a native transfer
of about 14 minutes. Those observations are not a matched measurement of the
trace's box. The initial 0.3 derate is INFERRED policy, not a measured ratio from
that trace. No live provider or R2 query was made in J2.

CODE READ, decimal GB and advertised Mbps:

```
reference_seconds = inbound_GB * 8000 / reference_Mbps
expected_seconds  = inbound_GB * 8000 / (derate * advertised_Mbps)
extra_seconds     = max(0, expected_seconds - reference_seconds)
transfer_USD      = inbound_GB * inbound_USD_per_GB + outbound_GB * outbound_USD_per_GB
lost_time_USD     = extra_seconds / 3600 * compute_and_storage_USD_per_hour
network_term_USD  = transfer_USD + lost_time_USD
```

CODE READ: `offerPrice` adds `network_term_USD / lease_hours` to compute/storage
hourly price, and includes that same network term once in the total-run estimate.
The lost-time premium is an economic ranking/reservation penalty, not an additional
Vast invoice charge. Existing ledger spending estimates conservatively include it.
A faster-than-reference link receives no negative charge. Outbound transfer fees
remain included; outbound time is not modeled by this inbound-download estimate.

CODE READ: `SLOW_NETWORK` rejects an otherwise qualifying offer when expected
seconds exceed the smaller of its profile maximum and its lease duration, independently of whether its effective price
fits. The live-instance check uses the same function and original advertised
speed if the live API omits it; a slower live advertisement can close the rental
with `CONTRACT_SLOW_NETWORK`. Missing/invalid speed cannot produce a price.

INFERRED arithmetic from current allowances and 0.3 derate: small needs about
541 advertised Mbps, middle 1933, large 963, and adhoc 563 to fit these limits.
The existing 200 Mbps floor alone is insufficient: 100 GB at 60 effective Mbps
takes 13,333 seconds (3.70 hours). The default preferred inbound value of 300 Mbps
also does not guarantee feasibility; the guard applies first. A download limit
does not prove that downloading plus a 200–305 minute SPEC run fits a 300-minute
box lease. The stage-duration envelope still needs owner review and measurement.

CODE READ: `fetch_bench_data.sh`, the main provisioning downloads and Windows
media fetches log fixed numeric `KF_DOWNLOAD_METRIC` lines using curl's time_total
and size_download. File and streamed benchmark transfers are instrumented.
The Worker retains at most 32 numeric samples, advertised inet_down, summed transfer
seconds/bytes, and a truncation flag. It never retains arbitrary partial log text
in this parser. The settlement queue snapshots these claims into `download_measurements`
in R2/D1 reports alongside `offer_facts.download_estimate`. Unknown guest report
keys cannot replace the Worker's snapshot. Samples are labeled `payload_log_claim`
and `instrumented_curl_transfers_only`; they are not trusted admission evidence. Package-manager traffic,
other media paths, cache hits and uninstrumented traffic are not a measured
full-run total. Summed transfer durations are not wall-clock time when transfers overlap.
Calibration is manual; the samples do not automatically change derate or thresholds.

DESIGN-ONLY, UNTESTED: parallel range downloads, for example eight connections,
might raise usable throughput on shared links. Parallel range downloads were neither implemented nor tested here. Any experiment needs range/content integrity checks, bounded
connections/retries, private URL handling, matched offer/transfer measurements and
an allowance review before changing this model. Parallelism is not a promise of
8x speed and may compete with provisioning or shared storage.
