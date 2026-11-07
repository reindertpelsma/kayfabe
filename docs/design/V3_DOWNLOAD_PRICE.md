STATUS: DESIGN-ONLY, 2026-11-07. Proposed download model and feasibility limits;
GPU-free implementation/tests only, with no hardware calibration or deployment.

# Download time in effective price

OWNER DECISION (2026-11-07, coordinator addition 4): use the achievable R2
single-stream speed as the per-profile reference and cap expected speed by both
the derated host advertisement and the modeled R2 stream capacity. This supersedes
the earlier 1 Gbps reference. Reject links whose expected download time is infeasible.
The existing transfer allowances are small overestimates, not measured traffic.

CODE READ: policy choices are data in `ci/vast-broker/defaults.json`:

| Field | Value | Status |
|---|---:|---|
| each profile r2ReferenceMbps | 445 | PROPOSED, owner reference based on the measurement below |
| each profile streamsFactor | 1 | PROPOSED; current single-stream payload |
| derate | 0.3 | INFERRED |
| small maxDownloadSeconds | 3600 (60 min) | PROPOSED by J2 |
| middle maxDownloadSeconds | 1200 (20 min) | owner proposal |
| large maxDownloadSeconds | 3600 (60 min) | owner proposal |
| adhoc maxDownloadSeconds | 1800 (30 min) | PROPOSED by J2 |

MEASURED, OWNER-SUPPLIED SOURCE (coordinator addition 4, 2026-11-07): the dev box
used presigned GET range requests against the private bench object. Single stream:
55.6 MB/s, rounded 445 Mbps. Eight parallel streams over 2 GiB: 223 MB/s aggregate,
rounded 1.78 Gbps (1780 Mbps). These measurements were supplied to J2; no private
URL, credential or new R2 request is part of this lane. INFERRED limitation: the
dev box link may itself cap the eight-stream aggregate. These are achievable
observations from one path, not a universal R2 service maximum or guarantee.

MEASURED HISTORICAL SOURCE: [the SPEC transfer](../../traces/specviewperf_vast_20261105/README.md)
records 42,002,015,727 bytes in 804 seconds, approximately 418 Mbps. Its matched
offer advertised 613 Mbps, giving an approximately 0.68 observed ratio. The
owner separately reports a current 1562 Mbps advertisement and a native transfer
of about 14 minutes. Those observations are not a matched measurement of the
trace's box. The initial 0.3 derate is INFERRED policy, not a measured ratio from
that trace. No live provider or R2 query was made in J2.

CODE READ, decimal GB and advertised Mbps:

```
reference_seconds = inbound_GB * 8000 / profile.r2ReferenceMbps
expected_Mbps     = min(derate * advertised_Mbps,
                        profile.r2ReferenceMbps * profile.streamsFactor)
expected_seconds  = inbound_GB * 8000 / expected_Mbps
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

INFERRED arithmetic: 100 GB at the proposed 445 Mbps reference takes about
1798 seconds (30 minutes), rather than the superseded 800 seconds at 1 Gbps.
INFERRED arithmetic for the current allowances at the 445 Mbps origin ceiling,
with streams factor 1 (these are modeled minima, not measured run durations):

| Profile | Inbound GB | Minimum minutes | Profile maximum minutes |
|---|---:|---:|---:|
| small | 73 | 21.9 | 60 |
| middle | 87 | 26.1 | 20 |
| large | 130 | 39.0 | 60 |
| adhoc | 38 | 11.4 | 30 |

INFERRED arithmetic: with streams factor 1 and derate 0.3, small needs about 541
advertised Mbps, large 963, and adhoc 563 to fit their profile limits. Middle's
87 GB takes at least 1564 seconds (26.1 minutes), exceeding its unchanged proposed
20-minute maximum: every middle offer is rejected even on an arbitrarily fast
host. Owner decision pending: retain this refusal or raise middle's allowance to
at least its origin-limited minimum (30 minutes recommended). A shortened
20-minute small lease also cannot fit small's 73 GB at this reference. These
infeasibilities are tested and never bypassed by a faster advertisement.

CODE READ: unrelated admission/authentication/cleanup tests explicitly simulate a
separately configured 1 Gbps origin. The pricing tests restore the shipped profile
data and prove the 445 Mbps ceiling and default middle refusals. Passing synthetic
protocol tests is not evidence that a default middle rental can be admitted.

INFERRED arithmetic: the existing 200 Mbps floor alone is insufficient: 100 GB
at 60 effective Mbps takes 13,333 seconds (3.70 hours). The default preferred
inbound value of 300 Mbps also does not guarantee feasibility; the guard applies
first. A download limit does not prove that downloading plus a 200–305 minute
SPEC run fits a 300-minute box lease. The stage-duration envelope still needs
owner review and measurement.

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

DESIGN-ONLY, UNTESTED IN J2: J4 is building a parallel range fetch tool. Its
integration may justify raising the per-profile reference toward the owner-measured
1780 Mbps after matched payload measurements; no such change is active here.
`streamsFactor` remains 1. Changing the factor models extra origin capacity without
changing reference seconds; changing `r2ReferenceMbps` changes the reference too, so
any eventual rollout must set both deliberately and avoid counting the same measured
gain twice. Parallel range downloads were neither implemented nor tested in J2. Any experiment needs range/content integrity checks, bounded
connections/retries, private URL handling, matched offer/transfer measurements and
an allowance review before changing this model. Parallelism is not a promise of
8x speed and may compete with provisioning or shared storage.
