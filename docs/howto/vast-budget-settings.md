# Vast check capacity and spending settings

CODE READ from source at rev 9a995ad97 plus this branch's count-ceiling change: `DAILY_RENTALS`
accepts integer1..40. Its default stays4; deployments choose their own value. Raising the
configuration ceiling changes no deployed setting. Daily rental counts apply per profile and
reset at UTC midnight; real and uncertain attempts count, confirmed no-instance failures do not.
CODE READ from Ledger.reserve and budgetSummary in the combined source: a bounded provider
replacement shares its original attempt/day; both legs retain dollar receipts.

Daily count, daily dollar liability, rolling30-day dollars, remaining prepaid credit reserve
and concurrency are independent checks. An extra count slot never overrides a dollar refusal.
Settled rentals release unused reserved duration. The check and sweep daily dollar limits are
separate; rolling dollars and prepaid credit are shared across the account.

INFERRED from source reservation math: at $1.32 estimated per full check, a $10 daily limit
admits7 full-length runs before prior spend, not40. Twenty PR pushes plus twenty merge runs
need40 attempt slots and about $52.80 if every attempt incurs that full estimate.
A provider replacement shares the original deadline and configured dollar allowance; it can
have a different eligible price, but adds no new daily attempt slot.
Actual short settled runs can cost less. A $30 rolling limit also restricts sustained usage;
raising daily count alone cannot promise every busy-day push gets a box. Keep dollar caps
until the owner chooses an explicit budget; inspect BUDGET reason/numeric diagnostics first.

CODE READ from source at rev 9a995ad97: account concurrency2 permits two simultaneous check
rentals only when MAX_CONCURRENT_CHECK is also2 or unset. A third gets BUSY, and the runner
waits for first admission only while its remaining job budget can hold a complete rental
plus admission margin. A proved replacement uses the original remaining deadline instead.
It can fail visibly before either long rental completes. No durable waiting queue is implied.

CODE READ (2026-11-07): a release plan now selects eight explicit dies on eight
parallel boxes, with two Windows runs on each box. It shows independent host,
Linux guest and Windows guest driver selections and their proposed subruns.
The account needs at least nine concurrent slots, including the reserved slot.
These driver choices are proposed; the current payload refuses them before
provisioning. A budget-admitted plan is not a completed release verification.

CODE READ (2026-11-07): resource reservations now follow the run profile. Large
reserves 300 GB of disk, 130 GB inbound and 2 GB outbound, with a $0.02/GB sanity cap in
each direction. Small reserves 73 GB inbound, middle 87 GB and adhoc 38 GB; each
keeps a 120 GB disk, 2 GB outbound and a $0.005/GB cap. Effective hourly prices
include those transfer allowances amortised over the lease. Cheaper compute can
therefore lose to a host with cheaper inbound. A higher per-GB price is rejected
even when the effective hourly price would otherwise fit.


Host selection first tries offers meeting the preferred GPU ratio, CPU, disk and
inbound throughput values for the run profile. Within that tier it chooses the
lowest effective hourly price. If no preferred offer fits the spending limits,
it falls back to offers meeting the hard floor. Country preference breaks price
ties. Explicit die choices, rotation and retry exclusions still apply.

The proposed defaults are a GPU score at least the die median, 12 effective CPU
cores, 150 MB/s disk bandwidth and 300 Mbps inbound. Each run's published report
includes the provider offer facts, median ratio, selected tier and threshold
snapshot under `offer_facts`, alongside run and stage timings for calibration.
These are provider metadata, not measurements of that run's actual host speed.


Effective price also includes the value of expected download time beyond each
profile's proposed 445 Mbps R2 reference. Expected speed is the smaller of the
derated advertisement and that reference times the profile's streams factor (1). The initial advertised-speed derate is inferred at 0.3. Offers exceeding
the proposed download-time limit are rejected as `SLOW_NETWORK`: middle 20 minutes,
large 60, small 60 and adhoc 30. This guard can reject a link that meets the advertised
200 Mbps hard floor. At 445 Mbps, middle's 87 GB needs at least 26.1 minutes,
so its unchanged 20-minute limit currently rejects every middle offer. A 20-minute
small lease is also infeasible. These limits do not establish that every benchmark fits its lease.

The report's `download_measurements` records bounded curl timing/byte claims from
benchmark, provisioning and Windows media transfers beside advertised inbound speed.
Coverage is partial, labeled `instrumented_curl_transfers_only`; the data supports manual calibration. The time premium
is a modelled opportunity cost, not an extra provider billing rate.

## Release driver media

Update driver selections and their `driver_installers` catalog in
`ci/vast-broker/release-matrix.json` together. Record the official HTTPS installer
URL, exact SHA-256, byte count, check date and source; use NVIDIA's official
checksum file or hash the official byte stream. Do not upload NVIDIA installers
to repository or object storage. If a pin is unavailable, retain `UNPINNED` with
null hash and size; an executable matrix cannot contain it.

Validate with `node ci/vast-broker/validate-release-matrix.mjs`. A runtime 404 or
changed installer is reported as `MEDIA_DRIFT`, never PASS. Refresh the official
pin only after investigating the drift; no automatic alternate URL or version is
selected. The proposed matrix and payload executor remain disabled.


## Reproduce the renting simulations

Run the fixed 200-seed market sweep and named outage cases without provider access:

```sh
nice -n 19 node --test ci/vast-broker/test/simulation-*.test.js
```

To expand the same reproducible generator locally:

```sh
KF_SIM_SEEDS=1000 KF_SIM_START=1337 nice -n 19 node --test ci/vast-broker/test/simulation-*.test.js
```

The large market loop checks all four profile plans and makes at most one mocked rental
per seed; named fleet scenarios exercise the complete Worker orchestration.
