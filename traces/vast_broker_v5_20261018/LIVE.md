# Non-VM dummy acceptance

[MEASURED: 2026-10-23, run v5-dummy-pr104-20261023; dummy_live.json, dummy_log.txt] All four scenarios completed on the production tiny bootstrap and PR #104-based repo scripts. Six real rentals, including two failed diagnostic attempts, are independently confirmed absent by exact ID and label. The conservative total is $0.268215, including complete transfer allowances, under the shared $1 cap. This is not an invoice. Every rental had its independent reaper armed before creation. Exact-source-IP ingress was removed after teardown; all local servers, tunnels and reapers stopped. No Cloudflare deployment occurred.

| Scenario | Box result / runner decision | Cleanup evidence |
| --- | --- | --- |
| A | FAIL / verified | 60-second local grace, then real cron; delete 43.308 seconds after deadline |
| B | PASS / verified | Scoped kill; Worker confirms absence after 0.688 seconds; independent API absence also verified |
| C | One report, no result / runner UNAVAILABLE, unverified | Heartbeat deadline, real cron delete +11.430 seconds |
| D | No report / runner UNAVAILABLE, unverified | Boot deadline, real cron delete +1.538 seconds |

[MEASURED: 2026-10-23, run v5-dummy-pr104-20261023; dummy_live.json] Scheduled invocations used Wrangler's actual /cdn-cgi/local/scheduled handler at a 60-second cadence with real clock/provider calls. C/D status polling was held until independent API absence, so polling could not perform their cleanup. Temporary local limits are in JSON; committed production limits remain unchanged. C/D never gain a verified result from Worker-authored failure text.

[MEASURED: 2026-10-23, run v5-dummy-pr104-20261023; dummy_log.txt] The box downloaded both private assets (49 and 270177 bytes), checked payload-locked SHA256 values, reused the signed URL after 60 seconds, and reported structured PASS with actual GPU hardware. URL validity is observed for that interval; its full lifetime is not established. The public log contains scrubbed fake bearer text and inert workflow-command text. URLs, capabilities, provider instance IDs and addresses are excluded.

[MEASURED: 2026-10-23, run v5-dummy-pr104-20261023; dummy_live.json] Blob responses were 644/648 bytes of metadata; binary bytes are not carried in the Worker response. Local V8 sampled active deltas were 805.929/8.630 ms, with 310.838/338.692 ms separately unclassified as program time. These wall-delta samples are diagnostic estimates, not a Cloudflare billing measurement or Free-plan guarantee. The first observer lacked its required inspector WebSocket origin; two no-spend native probes pass after that local fix, and profiling runs outside the Worker request path.

[CODE READ: src/blobs.js, country_mutants.json, blob_mutants.json] The Worker reads bounded metadata, cancels the manual redirect body and never relays the asset bytes. Runner URL scrubbing is independent. The initial rental exposed the CLI-vs-REST env defect, fixed test-first in 2588349cb; see REST_ENV.md. The second failed attempt exercised a correct structured FAIL from a diagnostic timeout. Neither is hidden from spend accounting.

[CODE READ: DEPLOY.md, docs/BOX_PROTOCOL.md] This acceptance proves the local Worker/provider lifecycle with local test identity and non-VM containers. Full KVM image/suite payload, hosted OIDC, Cloudflare billed CPU and real SSH login-to-MANUAL remain NOT DONE or unverified. The public repo payload can change by approved SHA without redeploying the generic Worker bootstrap.
