# Public broker origin and Worker var

STATUS: LIVE, 2026-10-26. CODE READ at base b6e308e3e: both GPU workflows still used the deleted GitHub URL secret, and deploy.mjs treated WORKER_URL as a check-only secret. The sweep already had environment gpu-vast.

Hypothesis: the public origin can change context without changing secret or trusted-runner boundaries. Falsifier before edits: the old guard already accepts the public URL variable, or the deploy helper already uploads it and rejects a different configured audience.

MEASURED 2026-10-26, EXPECTED RED e5c051dc0: focused tests passed 64/76 and failed 12. After the change they pass 76/76. Full Node suite passes 2075/2075; the real workerd lane passes 43/43, with fake outbound services only. No deployment or provider calls were made.

CODE READ: WORKER_URL is vars.VAST_BROKER_URL in both workflow broker steps. The path and two Access bindings remain secrets. All other guard boundaries and the exact trusted BASE bootstrap remain unchanged. Worker WORKER_URL is a public toml var. An optional vars-file value must equal toml; an invalid toml origin refuses even without an override. The helper prints the coordinator migration prerequisite, `wrangler secret delete WORKER_URL`, but never executes it.

CODE READ: runner.py validates a hostname without a workers.dev suffix restriction; policy.js requires an exact bare HTTPS origin. routes.js builds/redacts the configured origin, onstart.js uses the supplied callback base, and vast_report.py validates a generic HTTPS broker URL. Runner and Worker use that same origin as the OIDC audience. Both must be changed together. Access covers all of ci.kayfabe.dev with the same runner and box service tokens. The fallback workers.dev hostname requires equivalent Access. Admin uses its separate owner-identity Access app at admin.kayfabe.dev.

CODE READ: pinned wrangler 4.152.0, installed wrangler-dist/cli.js: the triggers/deploy.ts implementation calls publishCustomDomains only inside `if (customDomainsOnly.length > 0)` (line 165282). That method replaces custom-domain state only when called (line 164462 onwards). No route/custom-domain config means no such call, so routes remains absent and the dashboard custom domain remains coordinator-managed. [Cloudflare custom-domain configuration](https://developers.cloudflare.com/workers/configuration/routing/custom-domains/) documents the separate dashboard and Wrangler configuration paths.

MEASURED 2026-10-26, /tmp/wk1-domain-node.log bundle tests: the deterministic emitted module still matches 9ac12a86ed2dbc4bc7a63c9a35d739c9dcc9acf7e18eb117187f6e92082ecd09. Public vars are deployment configuration, not JavaScript module contents. deployment.json and DEPLOYED_BUNDLE_SHA256 need no change for this branch. After combining with other source changes, regenerate both from the combined module.

Run the mutation replay from ci/vast-broker: `python3 ../../traces/wk1_domain_20261018/mutate.py`. It restores each changed file in finally. The adjacent mutations.jsonl records at least two named killers per mutation.

MEASURED 2026-10-26, mutations.jsonl: all nine mutations have at least two named killers. The replay includes the existing workflow-boundary tests so reverting only the sweep binding is checked by independent binding and guard assertions.

| Mutant | Killing cases |
| --- | --- |
| guard-old-url-secret | 8 |
| workflow-check-url-secret | 6 |
| workflow-sweep-url-secret | 3 |
| public-url-not-uploaded | 2 |
| url-equality-unchecked | 2 |
| toml-origin-unchecked | 2 |
| migration-prerequisite-omitted | 2 |
| public-var-allowlist-omitted | 4 |
| public-toml-origin-omitted | 4 |
