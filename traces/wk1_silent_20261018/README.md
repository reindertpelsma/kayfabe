# Silent controller exits and legacy no-instance receipts

STATUS: LIVE, 2026-10-26. Base master ba74e2929. No deployment, Cloudflare setting/secret change or direct Vast call.

INFERRED from source, hypothesis before tests: exhausted valid JSON 503 availability replies return through a path that never assigns a diagnostic; legacy no-instance proof is missed when a receipt retains a price or its record omits id. Falsifiers: those paths already print one trusted line or release only the proven failed rental's count. EXPECTED RED 70b23d5e0 precedes the fix.

## Gate failure

MEASURED at rev ba74e2929, GitHub run 40365979581 attempt 4 shows the trusted BASE header at 17:26:06 UTC and an empty final bounded-log group at 17:26:39, then exit 1/state UNAVAILABLE. Stored Cloudflare events in observability.json show six GET /availability calls returning 503 from 17:26:06.470 through 17:26:38.986 UTC; no /rent appears in that attempt. Earlier rows show availability 200/rent 409 before the configuration change. Only fixed route categories, timestamps, status and method are retained: no URL/prefix, token, account id or raw payload.

MEASURED in the read-only investigation of rev ba74e2929: a read-only Worker-settings request returned the public var DAILY_RENTALS="40". CODE READ: policy.js:148 uses number('DAILY_RENTALS', DEFAULTS.dailyCount, 20); the helper at policy.js:32-37 refuses above 20. Configuration runs before availability/provider access. The local 21/40 probes reproduce HTTP 503 with no outbound calls. Coordinator action: restore DAILY_RENTALS to an allowed value <=20. No cap is raised by this PR.

CODE READ: retry_read accepts a protocol-shaped JSON 503, retries six times using the existing delays, then returns its last error object. execute validates that object and raises Invalid because availability was not 200; the old catch sets UNAVAILABLE but no diagnostic. Infrastructure failure prevents /stop; no /rent was reached. The six stored 503s and the local reproduction establish this failure path. Observability stores status, not a captured response-body explanation; the configuration cause is derived from the read-only setting and matching source/runtime reproduction.

Read-only observation used an ad-hoc dry telemetry query over IPv4, as described by the [Cloudflare query API](https://developers.cloudflare.com/api/resources/workers/subresources/observability/subresources/telemetry/methods/query/). No query was saved and no settings were changed.

## Changes

CODE READ from source runner.py: Runner now records a fixed phase, bounded HTTP class/status and validated state. Every non-PASS return emits one trusted runner line before cleanup; main covers failures before Runner creation and output/summary errors, suppressing duplicates. A verified FAIL also reports its result phase. Existing budget reasons, disabled-renting wording and rejected-check line numbers remain; reasoned offer/preference/time refusals name their closed reason vocabulary. Successful/recovered runs remain silent. Raw exception text, response bodies, tokens and endpoints never supply a diagnostic. The final line is capped below 1024 characters and cannot contain embedded terminal/newline controls. Retry timing, cleanup decisions, eligibility and cap values remain unchanged.

CODE READ: a legacy receipt with no created marker is now compared to its retained durable record. Only matching start time, settled GONE and null/absent id establish no-instance proof. A stale priced receipt is then treated as zero spend and zero rentals for calculations, even if already ended. The receipt remains in storage. Admin calculations remain read-only; admission and expiry persist the derived created=false marker before the old record is discarded, so the correction survives retention. Real ids, unended/uncertain records and mismatched starts keep counting. The existing conservative fallback for old zero-price records remains. This cannot reconstruct proof after a record has expired; no live count reduction is claimed before coordinator deployment/inspection.

MEASURED at rev 70b23d5e0, EXPECTED RED: 18/25 initial Node cases fail and both real workerd cases fail. The tests exercise actual main(), including availability 5xx, malformed JSON/shapes, OIDC, setup/preferences, refusals, bundle checks, verified FAIL and Actions output failures. Additional cases cover phase context, bounded single-line output and negative legacy controls. A second EXPECTED RED commit 012fcc02f adds four failing retention cases before persisting that derived marker. Full final counts and named mutation kills are recorded below after validation.

Replay mutations from ci/vast-broker with `python3 ../../traces/wk1_silent_20261018/mutate.py`. Each source file is restored in finally and Python bytecode is isolated. All workerd outbound calls are disabled.

MEASURED 2026-10-26, mutations.jsonl: all 16 mutants have at least two named killing cases. The replay first requires the unmutated Node/workerd baseline to pass.

| Mutant | Killing cases |
| --- | --- |
| runner-failure-line-omitted | 16 |
| setup-failure-line-omitted | 5 |
| main-duplicates-runner-line | 16 |
| phase-lost | 22 |
| http-class-lost | 14 |
| diagnostic-unbounded | 2 |
| diagnostic-newline-injection | 2 |
| oidc-phase-lost | 3 |
| legacy-receipt-marker-required | 6 |
| legacy-absent-id-not-recognized | 5 |
| legacy-gone-proof-skipped | 2 |
| legacy-time-binding-skipped | 2 |
| legacy-real-instance-proof-skipped | 2 |
| legacy-admin-writes-storage | 6 |
| legacy-admission-proof-not-persisted | 2 |
| legacy-expiry-proof-not-persisted | 3 |

MEASURED 2026-10-26, /tmp/wk1-silent-node-release.log: all 2194 Node tests pass. /tmp/wk1-silent-workerd-release.log: all 49 real workerd TAP tests pass. Formatting, changed-code line cap and staged secret scanning pass. No timer, retry interval, eligibility decision or cap value changed.

MEASURED 2026-10-26, /tmp/wk1-silent-bundle-final/index.js: emitted module SHA-256 is 64cd16cadf0d06a49c3738171daef41be26d8db8023d3cc1aad77348112b0e01. deployment.json and toml carry that same hash. Regenerate after combining with the separate IDS/host-reference PR.
