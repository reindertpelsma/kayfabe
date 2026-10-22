# V5 working evidence — acceptance pending

STATUS: LIVE, 2026-10-22. No V5 rental or deployment has occurred.

[MEASURED: 2026-10-22, run v5-readonly-20261022-availability-1, read_only.json] Native local
Wrangler 4.152.0 executed the current Worker policy against real Vast offers,
prepaid-credit and instance-list APIs, with local fake GitHub/JWKS identity.
A private adapter blocked all upstream mutations; none was attempted. Both
profiles returned only allowed classes. Check returned AD102 at $0.666536/hour
including 120 GB storage and its transfer allowance. A deliberately tiny daily
budget let preference admission be checked without creation: disallowed classes
refused; mixed lists chose the allowed class and reported one ignored entry.

[MEASURED: 2026-10-22, run v5-readonly-20261022-availability-1, read_only.json] Twelve concurrent
clients plus a sequential rate-limit tail sent 95 requests: 25 HTTP 200 cache
responses, 58 HTTP 409 busy/refusals, and 12 HTTP 429. Every successful availability
response was a cache hit; one offer search served the whole phase. Response size
was at most 1072 bytes; wall latency median 40.15 ms, maximum 323.03 ms. Instance
reads before/after the phase were separate read-only diagnostics. Provider IDs,
addresses, account secrets and callback capabilities were not returned publicly.

[MEASURED: 2026-10-22, run v5-readonly-20261022-availability-1-cpu, local_cpu.json] Local V8
sampling requested 100 us intervals and recorded 38 requests. Idle/root/program
samples are excluded from the JS estimate; program time is reported separately as
unclassified. These are local profiling estimates, not Cloudflare billed CPU or
proof of its Free-plan CPU contract. Full per-request observations are in JSON.

[MEASURED: 2026-10-22, run v5-readonly-20261022-price-compare, price_compare.json] In a later
43-offer snapshot, neither a $1.00 nor $1.05 effective cap admitted a check offer
after all quality/driver/die filters. Raising the price alone did not help that
snapshot; market availability changed from the earlier AD102 result.

[CODE READ: test/fixtures/v5-offers.json, src/policy.js; official
[offer API](https://docs.vast.ai/api-reference/search/search-offers)] Recorded
fields use compute_cap = CUDA capability x100, storage_cost in USD/GB/month,
inet_down_cost/inet_up_cost in USD/GB, RAM in MB, network in Mb/s, CPU in GHz,
and disk bandwidth in MB/s. The live allocated_storage=120 responses satisfy
`dph_total = dph_base + storage_cost * 120 / 720`; the Worker checks consistency
and never adds storage twice. Documentation example rows are illustrative;
recorded live fixtures establish the actual response shapes used here.

[MEASURED: 2026-10-22, policy_tests.txt at the V5 policy checkpoint] Local baseline:
901 tests, zero failures. [MEASURED: 2026-10-22, policy_mutants.json] All 56 focused V5
mutation cases have at least two named killers. Standalone EXPECTED RED commits
precede lifecycle, price, offers, transfer liability, frozen boot learning and
selection-on-refusal fixes; the expired-soft-deadline probes were green controls.

[CODE READ: src/index.js] Endpoint inventory: POST /rent, GET /availability,
GET /status/<run>, GET /logs/<run>, POST /stop/<run>, POST /stop-all,
POST /report/<run>, POST /kill/<run>, POST /hold/<run> (MANUAL), and owner-only
POST /stop-timer/<run>. GET /blob-url remains planned and outside this lane.

[CODE READ: 2026-10-22, src/policy.js, src/offers.js] Verified counts and rejection totals in
availability describe the bounded current query sample, not the entire market;
a single offer can have multiple rejection reasons. Numeric quality thresholds
are infrastructure admission filters, never tested-code verdict thresholds.

[MEASURED: 2026-10-22 protocol_tests.txt, v5_mutants.json, v4_migrated_mutants.json,
runner_migrated_mutants.json] The later protocol checkpoint passes 964/964 Node
tests, 69 focused V5 mutations, 28 migrated V4 cases and 38 migrated runner cases;
every case has at least two named killers. The 94 legacy anchors pass preflight;
full legacy execution remains pending. No V5 rental/deployment has occurred.
