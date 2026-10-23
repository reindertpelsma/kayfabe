# Prepaid credit and one live lease — 2026-10-22

STATUS: PARTIAL, run v3-live-credit-20261022. Credit bug fixed; one live callback-liveness failure observed. No production deployment or second rental.

INFERRED before the credit fix: H7 prepaid credit, not legacy balance, must cover reserve plus full TTL cost. FALSIFIER: live API lacks numeric credit, credit=39.5/balance=0 cannot admit, or missing/malformed/low credit admits.

MEASURED: 2026-10-22 run v3-live-credit-20261022 live credit/balance distinction confirmed; all 12 regressions first failed, then passed. No additional current-charge field appeared in the observed response. No undocumented deduction or balance fallback was invented.

CODE READ: 2026-10-22 the [show-user reference](https://docs.vast.ai/api-reference/accounts/show-user) lists the endpoint but its example omits credit; the observed response and owner correction establish that field. [Billing documentation](https://docs.vast.ai/guides/reference/billing) describes prepaid rental credits.

MEASURED: 2026-10-22 run v3-live-credit-20261022 four targeted mutations were applied/restored one at a time before the final full suite. No mutant threshold was reduced.

| Credit mutant | Killing tests |
| --- | ---: |
| legacy-balance | 12 |
| fallback-balance | 2 |
| remove-credit-guard | 10 |
| omit-reserved-cost | 2 |

MEASURED: 2026-10-22 instance 57212118 had an exact script-hash match and generated one authenticated report attempt. The record counted one attempt, read zero body bytes, and closed UNAVAILABLE. The preceding direct provider snapshots said actual_status=created; the exact inspect body at callback time was not retained, so that value cannot be asserted for the rejection instant. The trusted local diagnostic wrapper recorded GET/GET/DELETE and provider delete success, followed by two further callback 401 responses. The onstart sender could reach the Worker through the tunnel, but no payload was accepted. The payload was not read, so no claim is made about its contents. CODE READ: src/onstart.js sends progress first. A malicious host can also use its derived capability: this is not an independent VM boot attestation.

CODE READ: `src/index.js` report-live rejects every non-running actual_status and closes/deletes in its catch; status/update also treats non-running as ended. The [provider show-instance reference](https://docs.vast.ai/api-reference/instances/show-instance) includes both actual_status and cur_state. The installed CLI displays actual_status as instance status, including created/loading/exited. The reference does not establish cur_state as equivalent liveness authority. Replacing actual_status with cur_state merely to pass this run would weaken an unresolved boundary; no such change was made.

INFERRED: 2026-10-22 H8 before any further live investigation: an early callback in a pending state caused premature deletion. FALSIFIER: the exact callback-time inspect result was terminal, absent, ownership-invalid or unavailable instead of pending, or the sender ran outside the intended VM. The existing snapshots and code support this hypothesis but do not prove it. Recommended next step: capture the exact callback-time provider state and establish where onstart executes before changing liveness. Option 1, if pending is confirmed: reject early reports without deleting documented pending states, retain the original TTL, and test pending→running→accepted report plus terminal/unknown refusal before another bounded live run. Option 2: preserve current immediate-delete behavior and use a provider contract that reports running before onstart. Neither option is implemented here; the lane rule requires options rather than an improvised boundary change.

MEASURED: 2026-10-22 run v3-live-credit-20261022 callbacks were exposed only through the loopback ingress proxy and cloudflared. The proxy allowed POST of the exact run's report path only, with CF-Connecting-IP equal to the selected/confirmed Vast instance IP. It preserved the public Host and denied the controller's test request. No public listening Worker port or kernel firewall rule was added; the proxy and tunnel were stopped after provider absence. Origin/path bindings and bearer values are excluded from evidence.

MEASURED: 2026-10-22 run v3-live-credit-20261022 Linux schedstat deltas below sum only this live Worker's own workerd processes, including the native runtime/proxy. They are not Cloudflare billed CPU. n=5 per row after one warmup; rent-cache reused the closed grant and did not create a second instance. The one actual create has only wall time, not a CPU sample.

| Route | HTTP | Response bytes | Local workerd CPU ms min–max |
| --- | ---: | ---: | ---: |
| rent-cache | 200 | 181 | 7.888–10.338 |
| status-terminal | 200 | 181 | 3.887–5.654 |
| stop-idempotent | 200 | 181 | 4.163–5.314 |
| report-revoked | 401 | 90 | 3.59–4.862 |
| stop-all-empty | 200 | 19 | 4.355–7.032 |
| cron-empty | 200 | 2 | 3.378–4.104 |

MEASURED: 2026-10-22 instance 57212118 consumed a 111.425-second ledger interval at USD 0.60/hour, estimated compute/storage USD 0.0185708. This excludes network fees; quoted transfer price was USD 0.00078125/GB in each direction. Full workload completion was not established, and exact invoice consumption was not attributed from shared-account credit changes. The independent timer, Worker-owned destruction and repeated exact-ID/list absence establish that this run left no billed instance behind. TTL cleanup remains covered by the prior synthetic native run, not a second billed rental.
