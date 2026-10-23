# V4 complete mutation campaign

MEASURED: 2026-10-22 run v4-final-20261022; 96 legacy and 32 V4 cases, each with at least two named killers in legacy-mutants.json and mutants.json.

| Mutation | Killers |
| --- | ---: |
| inherited endpoint secrets | 7 |
| endpoint aliases in step metadata | 2 |
| optional prefix admission | 4 |
| native constant-time prefix comparison | 2 |
| public prefix gate | 3 |
| ledger service prefix gate | 29 |
| prefix charset and bounds | 6 |
| fixed callback prefix | 3 |
| endpoint text redaction | 5 |
| Actions secret instead of variable | 7 |
| workflow API prefix | 3 |
| derived hostname masks | 2 |
| encoded audience mask | 2 |
| shell tracing disabled | 2 |
| transport diagnostic suppression | 2 |
| secret context placement guard | 12 |
| secret object access guard | 2 |
| secret alias logging guard | 2 |
| secret job output guard | 2 |
| secret alias job isolation | 2 |
| literal run and renderer enclosure | 4 |
| HMAC tag and commit binding | 2 |
| closed ledger rejects reports | 2 |
| actual Vast liveness | 4 |
| liveness cache expiration | 2 |
| report count cap | 2 |
| cumulative report byte cap | 2 |
| streamed request byte cap | 2 |
| report rate cap | 3 |
| preparse JSON depth gate | 2 |
| strict report schema | 6 |
| strict result schema | 2 |
| report sequence immutability | 3 |
| structured result nonce | 2 |
| free text cannot decide verdict | 3 |
| structured result owned cleanup | 16 |
| native owner secret comparison | 13 |
| owner-only stop-all | 3 |
| OIDC run-scoped status and stop | 4 |
| ledger-only owner stop | 4 |
| stop-all cannot adopt account entries | 4 |
| revocation precedes failed delete | 15 |
| all log lines prefixed | 36 |
| embedded command delimiters | 13 |
| per-line UTF8 byte cap | 3 |
| log line count cap | 3 |
| log tail byte cap | 3 |
| orphan index cannot block cleanup | 2 |
| kill switch | 6 |
| owner numeric identity | 7 |
| review head SHA | 2 |
| draft refusal | 2 |
| OIDC claims | 11 |
| credit reserve | 18 |
| daily rental count | 2 |
| daily spend | 2 |
| rolling 30-day spend | 3 |
| one live instance | 6 |
| superseding push | 2 |
| run idempotence | 6 |
| SHA verdict cache | 5 |
| same SHA minimum gap | 2 |
| live label ownership | 4 |
| ambiguous-create binding | 2 |
| TTL destruction | 2 |
| SHA/nonce shell validation | 11 |
| callback shell validation | 2 |
| fixed offer admission | 10 |
| documented verification representation | 2 |
| documented model representation | 2 |
| outbound model names | 2 |
| documented offer pricing type | 260 |
| poll permissions guard | 3 |
| attestation dependency guard | 2 |
| cold-box disk guard | 2 |
| must-fail control | 2 |
| scheduled cleanup wiring | 3 |
| Unicode format sanitizer | 3 |
| unsupported job syntax | 2 |
| distinct client refusal | 3 |
| terminal client completion | 2 |
| public cron isolation | 6 |
| automatic base allowlist | 3 |
| docs-only exclusion result | 5 |
| unknown file-list skip | 5 |
| canonical path validation | 12 |
| renamed previous path | 2 |
| cached PR display grant | 2 |
| request bearer transport | 3 |
| request bearer retention | 3 |
| public GitHub fallback | 4 |
| owner branch without PR | 2 |
| immutable API cache | 2 |
| mandatory status writer jobs | 2 |
| token-shaped log redaction | 2 |
| isolated writer permissions | 2 |
| no-result-grace | 9 |
| kill-no-destroy | 5 |
| kill-no-capability | 4 |
| kill-wrong-route | 3 |
| no-absence-verification | 4 |
| no-provision-deadline | 2 |
| no-cron-reserve | 2 |
| no-hard-ttl | 2 |
| no-rate-limit | 3 |
| address-scrub | 4 |
| opaque-token-scrub | 2 |
| instance-scrub | 2 |
| full-log-retention | 3 |
| rendered-cap | 2 |
| summary-verdict | 4 |
| summary-fence | 4 |
| renderer-commands | 4 |
| public-summary-guard | 2 |
| workflow-fail-grace | 4 |
| workflow-cursor | 4 |
| sender-discards-prefix | 2 |
| sender-repeats-tail | 2 |
| unreleased-upload | 2 |
| direct-network-body | 4 |
| unbounded-forward-body | 2 |
| remaining-grace-ttl-cap | 2 |
| remaining-grace-ack | 3 |
| sender-ignores-grace-ack | 2 |
| lose-result-after-box-kill | 2 |
| stale-status-cursor | 2 |
| overlapping-worker-delimiters | 2 |
| overlapping-renderer-delimiters | 2 |
