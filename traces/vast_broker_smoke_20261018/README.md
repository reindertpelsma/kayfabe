# V3 local Worker smoke — 2026-10-22

STATUS: LIVE SMOKE RESUMED, 2026-10-22. Run `v3-live-credit-20261022`; prepaid-credit fix commit `520c63c5c`; latest V1 `584124edb` merged. No deployment.

MEASURED: 2026-10-22 run v3-live-credit-20261022 corrects the earlier account diagnosis: live credit exceeds USD 8 while balance is zero. The Worker and its mock read the wrong field. No account funding is required for this smoke. EXPECTED RED d51212cd0 failed all 12 new credit tests; fix 520c63c5c reads credit, preserves numeric/finite checks and reserve plus TTL cost. Four mutants have 12/2/10/2 killers. The earlier no-spend evidence remains valid; its funding recommendation is withdrawn.

MEASURED: 2026-10-22 run v3-local-20261022 passed baseline 376/376 and final 514/514 Node tests, zero skips/failures; V1 contributes 97 added cases and this lane 41. Temporary Wrangler 4.151.3, Node v22.22.1, npm 9.2.0; bundle syntax check passed (43110 bytes). No repo dependency added. [Raw route samples](routes-final.json), [initial failures](routes-initial.json), [mutant killers](mutations.json), [native SQLite lifecycle](mock-lifecycle.json).

MEASURED: 2026-10-22 run v3-local-20261022 used the configured real Vast key in ignored mode-0600 `.dev.vars`. Own startup/runtime logs contained none of the account key, run secret or owner token. No credentials, bearer values or unfiltered account responses are evidence. Local RSA private key remained in memory. Cleanup removes local secrets and temporary runtime state.

## Hypotheses, stated before inspection/runs

INFERRED: H1 native workerd/SQLite and real read-only Vast responses satisfy mocked contracts. FALSIFIER: startup/storage failure, schema mismatch, redirect refusal, auth oracle or failed cleanup. Result: falsified by five defects below; corrected no-spend paths pass.

INFERRED: H2 explicit loopback trust enables local identities without changing public authentication. FALSIFIER: missing/false opt-in, public caller or non-loopback issuer/API enables test trust; exact issuer checking breaks; any guard mutant has fewer than two killers. Result: guard tests and native RSA/JWKS/GitHub admission pass.

INFERRED: H3 existing create request starts a VM and fixed callback sender. FALSIFIER: rejection, no running VM/onstart/callback, or unacknowledged destroy. Result of the first run: not tested because the wrong balance field caused a false refusal. The resumed run uses prepaid credit; existing vm/args/onstart launch contract is preserved.

CODE READ: code-level H3/H4/H5/H6 falsifiers precede the corresponding EXPECTED RED tests in `test/native-fetch.test.js`, `provider-absence.test.js` and `native-github.test.js`; native POST and local trust falsifiers precede `local-smoke.test.js`.

## Route results and CPU

MEASURED: 2026-10-22 run v3-local-20261022 table shows warm n=5 per row. CPU is the sum of Linux task schedstat deltas over this session's own workerd processes, including native runtime, local proxy and observability. Wall times are separately retained in routes-final.json. This is NOT Cloudflare billed CPU and does not certify its Free limit. Read-only probe rows invoke the unchanged Vast/OIDC/GitHub classes from a temporary loopback harness, not added public Worker routes.

| Route/case | HTTP | Response bytes | Local workerd CPU ms (min–max) |
| --- | ---: | ---: | ---: |
| unknown | 404 | 9 | 2.517–4.111 |
| rent-disabled | 503 | 94 | 3.656–5.483 |
| status-missing | 401 | 90 | 4.857–5.809 |
| status-bad | 401 | 90 | 5.112–6.887 |
| status-local-jwt | 404 | 87 | 4.244–7.939 |
| stop-bad | 401 | 90 | 4.51–6.525 |
| report-bad | 401 | 90 | 4.876–6.925 |
| stop-all-bad | 401 | 90 | 5.913–7.226 |
| stop-all-owner | 200 | 19 | 4.575–6.798 |
| scheduled | 200 | 2 | 4.709–6.819 |
| local-authorization | 200 | 97 | 9.987–13.798 |
| real-offer | 200 | 2009 | 6.809–9.648 |
| real-balance | 200 | 139 | 6.024–8.206 |
| real-list | 200 | 236 | 5.319–7.561 |
| real-absent | 200 | 186 | 6.553–7.991 |

MEASURED: 2026-10-22 run v3-local-20261022 unknown paths return generic 404/9 bytes; missing/bad status/stop/report/stop-all auth gives identical 401/90-byte JSON. Distinct unknown run IDs have identical unauthorized bodies. Disabled /rent deliberately gives 503/94-byte RENTING_DISABLED for missing/bad auth and GET/POST, matching its existing kill-switch contract. Same-run local OIDC on an absent ledger grant returns 404 only after authentication. Owner stop-all with empty ledger returns 200. Scheduled handler is callable via --test-scheduled (200/2 bytes).

MEASURED: 2026-10-22 run v3-local-20261022 native SQLite synthetic rent returned 200, progress/log reports 200, and status retained sanitized text. After stopping/restarting Wrangler with the same SQLite directory, the record/log survived. An injected test clock expired the synthetic record; scheduled cleanup persisted TTL/FAIL, removed the synthetic provider instance and revoked callbacks (401); owned stop remained idempotent (200). The injected clock also expired the five-minute OIDC fixture, so terminal TTL state was asserted directly in the local persisted ledger. No real provider create/delete occurred in this harness.

CODE READ: 2026-10-22 [Cloudflare limits](https://developers.cloudflare.com/workers/platform/limits/) specify 10 ms Free Worker HTTP CPU, and [local profiling guidance](https://developers.cloudflare.com/workers/observability/dev-tools/cpu-usage/) describes local profiles.

MEASURED: 2026-10-22 run v3-local-20261022 local authorization consumed 9.987–13.798 ms including harness/proxy/native costs. Production billed CPU remains unverified.

## Reality versus mocks; live arm stop

MEASURED: 2026-10-22 run v3-local-20261022 real offers/balance/filtered instance-list/absent-instance reads succeeded after fixes. Vast returned offers/truncated, verification=verified (not a verified boolean), space-separated GPU names and numeric balance; v1 list returned success/total_instances/label_counts/instances_found/instances/next_token. The cheapest policy-valid read initially found RTX 4090 offer 54902716 at USD 0.7222222222/hour including selected disk; <=USD 0.20 and <=USD 0.50 searches were empty. That offer later disappeared. Stock changes are recorded, not treated as code failure. Unknown inspect returned HTTP 200 with instances=null, not HTTP 404. Canonical trailing slashes avoid observed 301 redirects. Provider shape samples omit private account fields.

MEASURED: 2026-10-22 run v3-local-20261022 real balance was below the configured USD 8 reserve, before adding the USD 0.12037 ten-minute allowance for the initial cheapest offer. The first live arm stopped at this incorrect account-field check; reserve was not lowered, and no funding/spend/create was attempted. This was a Worker defect, not insufficient prepaid funds. Real spend USD 0; exact run-label list has zero instances (final-instance-check.json). RENTING_ENABLED stayed false in the real configuration. No independent reaper was armed because no real create was attempted; it remains mandatory before any future create.

CODE READ: cloudflared 2026.3.0 was available without installing an account; no tunnel or firewall rule was created after the credit stop. VM launch, onstart execution, real callbacks and provider log API availability remain unverified. The earlier strict no-SSH VM/args/onstart design question remains open. TTL was exercised against the synthetic native provider and existing Node mocks, not a second billed instance.

## Test-first defects and mutation proof

MEASURED: 2026-10-22 run v3-local-20261022 committed each failing proof alone before its implementation. Final complete suite is 514/514; the exact restored source was bundled and syntax-checked.

| Defect / addition | EXPECTED RED commit | Red failures / cases | Fix commit |
| --- | --- | ---: | --- |
| Native empty POST + opt-in loopback identity | 69e85d3bc | 7 / 18 | 464fbf0d3 |
| workerd rejects redirect:error | af6e0d1ae | 3 / 7 | 5b76155b5 |
| Canonical provider slashes | aa51e055a | 5 / 12 | 7b9c168a8 |
| Provider 200 instances:null absence | ef7d7c311 | 2 / 7 | 284df199e |
| Native GitHub fetch receiver | bcd8e37b5 | 4 / 4 | 68b45f739 |

MEASURED: 2026-10-22 run v3-local-20261022 all twelve targeted mutants have >=2 independent test killers; mutations.json names them. The first accept-redirect experiment survived because an empty response failed JSON parsing even without status enforcement. Commit 442196799 strengthened the same redirect tests with parseable attacker JSON, then the unchanged mutant was killed by all four redirect cases. No threshold was weakened.

| Mutant | Killing tests |
| --- | ---: |
| empty-stream-regression | 4 |
| accept-nonempty | 2 |
| local-off | 7 |
| remove-optin | 2 |
| remove-caller-gate | 2 |
| issuer-not-exact | 6 |
| issuer-origin-guard | 4 |
| unsupported-redirect | 3 |
| accept-redirect | 4 |
| remove-canonical-slash | 5 |
| reject-native-null-absence | 2 |
| restore-fetch-receiver | 4 |

## Verdict and remaining decisions

INFERRED from corrected source commit 68b45f739 and run v3-local-20261022: suitable for a staged Cloudflare deployment with RENTING_ENABLED=false after required configuration/secrets are filled; no disabled-path native blocker remains in this smoke. Hosted execution, plan-specific billed CPU and production GitHub OIDC still need validation. This is not a production deployment or a green hosted-CI claim.

INFERRED: keep RENTING_ENABLED=false. The initial funding recommendation is withdrawn: the existing prepaid credit covers the reserve and smoke allowance; resume the owner-authorized live arm while preserving the reserve. Owner decision 2: resolve the documented VM startup contract and require running/onstart/callback/owned-destroy proof before enabling production rentals. No launch-mode redesign was made here.

MEASURED: 2026-10-22 run v3-local-20261022 stopped all three own Wrangler sessions and the local identity server; zero real instances were created. No GPU job, lock, firewall rule, tunnel or timer was started. No cargo target directory was created. Local secrets, synthetic SQLite state and temporary Wrangler install are deleted before handoff; only sanitized evidence and source changes remain.
