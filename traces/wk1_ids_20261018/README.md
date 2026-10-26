# Public rental attribution and same-box host reference

STATUS: LIVE, 2026-10-26. CODE READ at base b6e308e3e: admission discarded offer geolocation, provider hourly price and host driver label; the literal renderer hid non-Ada models; the active BASE runner omitted the instance id and only retained summary metadata after a final result.

Hypothesis: the requested identity fields are missing from status/rendering, and the existing pre-guest native stage can supply host_reference without adding a stage. Falsifier, stated before tests: baseline status/rendering already supplies those fields and the payload already emits that enum.

MEASURED 2026-10-26, EXPECTED RED commit 7f625a736: 10 new Node cases fail, and all five initial host-reference Python tests fail (six subcase errors). The prior focused suite passed 65/65.

CODE READ: status retains id and adds instance_id, geolocation, dph_total, host_driver_version and host_reference. The existing machine_id, host_id, die and gpu_model remain public. Offer metadata is persisted with the reservation; host and reported driver fields remain separate and do not affect offer selection. Known old records return explicit unknown/null/MISSING defaults.

CODE READ: the native cuda_host stage already runs before the guest stages and is graded with the existing host-Xid checks. host_reference starts MISSING, is NOT_GREEN while that native stage is attempted, and becomes GREEN only after its grade succeeds. A later guest failure retains the native result. The local result and reporter carry the optional enum; old result inputs remain valid. The coordinator must deploy the updated Worker to accept the new optional report field in live callbacks.

CODE READ: both the literal renderer and the active stdlib-only BASE runner display the public fields. The live RENTAL line precedes box log pages and is printed once after instance assignment. Non-Ada RTX/GTX/Quadro model strings and die labels use a bounded ASCII pattern. Summary cells are HTML-escaped with pipe/backtick neutralization. No environment, token or key fields are copied. The workflow guard continues to require the exact trusted BASE bootstrap; no boundary check was loosened.

MEASURED 2026-10-26: 2084/2084 Node tests and 37/37 workerd TAP tests pass, including real workerd result callbacks for GREEN and NOT_GREEN. The final host-reference suite passes seven Python tests. All 18 source mutations below are caught by at least two cases. Python bytecode is isolated so same-size edits cannot reuse stale cache.

| Mutant | Killing cases |
| --- | --- |
| instance alias | 2 |
| offer geolocation | 4 |
| offer hourly price | 2 |
| host driver label | 2 |
| host reference status | 4 |
| geolocation bound | 2 |
| geolocation controls | 2 |
| result extension | 4 |
| result enum | 2 |
| literal non-Ada models | 2 |
| literal rental line | 2 |
| BASE non-Ada models | 2 |
| BASE rental line | 2 |
| native green | 2 |
| native not green | 2 |
| native missing | 2 |
| reporter forwarding | 2 |
| reporter local schema | 2 |

[Raw mutation results](mutations.jsonl), reproduced from the repository root:

```sh
python3 traces/wk1_ids_20261018/mutate.py
node --test ci/vast-broker/test/v4-public-log.test.js ci/vast-broker/test/logs.test.js
MINIFLARE_ASSERT_BODIES_CONSUMED=true node --test ci/vast-broker/workerd/admission.test.js
```

MEASURED 2026-10-26: the first complete box-script run passed 674/675 tests. The unchanged test_outer_deadline_kills_a_stuck_transport failed at 2.011 seconds against its 1.5-second assertion and also failed in an isolated rerun at 2.010 seconds. Its test and kf-box-blob-url helper are unchanged from b6e308e3e. This local baseline failure is separate from attribution; no timer or cap was changed. Hosted CI is the required independent check.

Local dry-run bundle change (no deployment):

- Base: `9ac12a86ed2dbc4bc7a63c9a35d739c9dcc9acf7e18eb117187f6e92082ecd09`.
- Updated: `7c098c7c787fac5936f65b76644a842fcd270cdd5ec7c1ec37c5a9b63f21eeeb`.
