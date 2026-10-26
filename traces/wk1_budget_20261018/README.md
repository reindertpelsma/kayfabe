# Budget refusals and ledger accounting

STATUS: LIVE, 2026-10-26. Base master b6e308e3e. No deployment, Cloudflare changes or direct provider calls. All runtime tests use fake outbound services.

INFERRED hypothesis, stated before tests: confirmed no-instance failures consume daily rental slots, and interrupted settlement can leave open spend after durable end evidence exists. Falsifiers: those rows already release the count, and injected cost/index failures leave settlement atomic. These are separate from the unknown reason for the live refusal in run 40318395156 attempt 3. Its elapsed response time cannot identify which check refused it.

MEASURED 2026-10-26, EXPECTED RED 41260f910: 42/47 new Node cases and both new workerd cases fail. EXPECTED RED c01c31440: all four additional interrupted-settlement cases fail. The original budget response hid the deciding check.

## Accounting audit

All baseline line references below refer to b6e308e3e; updated code remains in src/ledger.js and src/index.js.

- CODE READ: baseline src/ledger.js:208-220 computes ended compute/storage using started..ended, with no remaining TTL on settled rows. The full network allowance remains conservative because actual bytes are unmeasured; see test/v5-network-liability.test.js. There is no full-TTL charge for a normally settled short-lived instance. New tests compare elapsed compute plus the unchanged transfer allowance.
- CODE READ: baseline src/index.js:897-904 sets compute, effective price and transfer to zero on a confirmed CREATE_REFUSED, then settles and retains cost:<key>. Baseline src/ledger.js:221 counts every same-day row regardless of that proof. This is a bug: repeated failures without instances can exhaust DAILY_RENTALS. New settlements persist created=false only for a confirmed no-instance GONE record; these rows release the count but retain the one-attempt receipt. Pending/ambiguous intent, actual rentals including free ones, and unknown legacy rows still count. A legacy zero row is excluded only when its retained durable record proves no instance and GONE.
- CODE READ: baseline src/ledger.js:136-148 saves the ended record, cost row, open index and ended index in separate awaited writes. A failed write can leave a durable end with an open spend row; budgets then uses now indefinitely. MEASURED in c01c31440: injected cost/index failures leave partial state, and both legacy end-evidence cases overstate spend. Settlement now commits those writes in one storage transaction. The read-only summary also derives an old interrupted cost row's end, prices and created status from a matching durable ended record. It never invents a stop from elapsed TTL and never changes storage while serving admin.
- CODE READ: baseline src/index.js:197-216 requires spaced absent-create observations after boot expiry before settling an ambiguous create at zero; src/index.js:335-341 settles a known instance when inspection confirms absence. cleanup at src/index.js:227-232 settles only after Vast.destroy, whose src/vast.js:191-197 path verifies absence after delete acknowledgement. Those paths remain. A provider/network failure or a cost row with no durable end evidence keeps liability: a possibly billing instance cannot be declared free from age alone. A retained cost row whose record has already expired cannot have its missing proof reconstructed by this lane.
- CODE READ: baseline src/ledger.js:212-222 applies daily spend and daily count only to the selected profile, while rolling spend and outstanding liability include both profiles. This is intentional, not a profile-mixing bug. New tests preserve both directions of this rule. Admin shows per-profile values plus account totals; a diagnostic's daily_usd/count_today refer to its selected profile.
- CODE READ: baseline src/ledger.js:210-222 adds elapsed plus remaining liability to daily/rolling estimates but returns only remaining compute plus transfer. The baseline credit test at src/index.js:789-790 adds that returned remainder to reserve and the proposed cost. It does not add the whole daily/rolling total, so no elapsed-spend double count was found. Exact credit-boundary tests admit reserve+new-cost+remaining and refuse one cent less. Contract repricing still excludes its own existing reservation.
- CODE READ: admission refusals before reserve write no cost row; reserve keeps the one-attempt key and all caps inside its storage transaction. The count fix changes which confirmed failures count, not any cap value. No defaults, timeouts, TTLs, spend caps, rental caps or credit reserve were changed.

## Public diagnostics and owner summary

CODE READ: every UNAVAILABLE_BUDGET error now has one of these fixed reasons: BUDGET:cost-key, BUDGET:sha-gap, BUDGET:no-credit, BUDGET:daily-count, BUDGET:daily-usd, BUDGET:rolling-usd, BUDGET:credit-reserve, BUDGET:ledger-full, BUDGET:invalid-cost or BUDGET:credit-unavailable. Its budget object contains only the public numbers available at that check. The late/transactional checks include daily, rolling, outstanding, count, credit, cost and caps; early key/gap checks show the prior timestamps or gap/elapsed time. No unperformed API lookup or invented amount is added to an early refusal. Missing/nonfinite numeric inputs become null.

CODE READ: runner.py accepts that closed reason vocabulary and a fixed numeric-key vocabulary, validates finite bounded numbers, and prints one rejecting line outside the untrusted box-text block. Negative prepaid credit is permitted as a signed public amount; other diagnostic amounts remain nonnegative. Unknown keys, strings, objects and unknown reasons are rejected. Old unreasoned refusals remain parseable during rollout. Contract repricing preserves its budget diagnosis while still closing the instance.

CODE READ: the existing DO-only GET https://internal.invalid/admin/list returns runs plus budget. No public route, new capability or UI feature was added. Budget includes account daily_usd/count_today, profiles.check and profiles.sweep daily/count values, rolling_usd, outstanding_usd, outstanding_rows and the newest 20 spend_rows. Rows contain only key/profile/start/end/hourly price and billed/reserved USD. billed_usd is an accounting estimate: elapsed compute/storage plus the full retained transfer allowance, not a provider invoice. Outstanding rows include overdue open records even when their remaining-TTL estimate is zero. Admin does not prune or repair storage. The existing UI may ignore the additional object.

MEASURED 2026-10-26, /tmp/wk1-budget-node-final.log: final full Node suite passes 2145/2145; /tmp/wk1-budget-focus4.log passes all 77 focused cases. /tmp/wk1-budget-workerd-final.log passes 47/47 real workerd tests, including rollback at both fault sites. /tmp/wk1-budget-admin-ui-final.log passes 77/77 existing admin UI tests after installing its pinned jose dependency locally.

MEASURED 2026-10-26, /tmp/wk1-budget-bundle-final/index.js: emitted module SHA-256 is 0d1b89536c254ba598249109c1a5d5a379fc36c8572ddf550c8778559647d1e5, replacing 9ac12a86ed2dbc4bc7a63c9a35d739c9dcc9acf7e18eb117187f6e92082ecd09 in deployment.json and wrangler.toml. Combining this branch with IDS requires a fresh combined module hash in both files. The domain PR's public var changes config only.

Run the mutation replay from ci/vast-broker: `python3 ../../traces/wk1_budget_20261018/mutate.py`. Each file is restored in finally; Python bytecode is isolated. The adjacent mutation results list named killing cases.

The actual deployed refusal remains unverified until the coordinator merges/deploys and reads its new reason/summary. These two proven defects are not asserted to be the cause of that particular request.

MEASURED 2026-10-26, mutations.jsonl: all 27 mutants have at least two named killing cases. The settlement-transaction mutant is killed at both Node fault sites and both real workerd fault sites.

| Mutant | Killing cases |
| --- | --- |
| diagnostic-cost-key | 6 |
| diagnostic-sha-gap | 4 |
| diagnostic-no-credit | 5 |
| diagnostic-daily-count | 6 |
| diagnostic-daily-usd | 4 |
| diagnostic-rolling-usd | 4 |
| diagnostic-credit-reserve | 7 |
| diagnostic-ledger-full | 2 |
| diagnostic-invalid-cost | 2 |
| diagnostic-credit-unavailable | 2 |
| no-instance-marker-lost | 5 |
| legacy-no-instance-evidence-ignored | 3 |
| legacy-interrupted-end-ignored | 2 |
| settlement-not-transactional | 4 |
| no-instance-still-counted | 9 |
| profiles-mixed | 3 |
| elapsed-spend-counted-again-as-credit-liability | 2 |
| settled-rows-billed-full-ttl | 5 |
| http-reason-omitted | 20 |
| http-public-numbers-omitted | 18 |
| runner-budget-line-omitted | 12 |
| runner-unknown-fields-accepted | 3 |
| admin-prunes-expired-cost-rows | 2 |
| admin-spend-list-unbounded | 2 |
| admin-raw-cost-fields-leaked | 2 |
| admin-outstanding-rows-omitted | 2 |
| repricing-does-not-exclude-own-reservation | 2 |
