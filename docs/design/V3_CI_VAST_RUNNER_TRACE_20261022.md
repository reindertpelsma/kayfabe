STATUS: LIVE, 2026-10-22 — owner trust-model correction; documentation only. Hosted identity, deployment and release execution remain unverified.

# Approved code, untrusted GPU tripwire, GitHub build provenance

[CODE READ: owner correction 2026-10-22; runner.py, gpu-vast.yml, src/index.js and src/onstart.js] Approval precedes paid execution. GPU test honesty, safe handling of hostile replies and final build provenance are separate trust boundaries:

| Who distrusts whom | Why and enforced boundary |
| --- | --- |
| Approved-code/release policy → Worker **and** Vast GPU verdicts | The exact code is approved **before Vast starts**. Either a malicious Worker or a malicious box can forge a GPU verdict, but cannot change that approved code or supply final build inputs: **no contamination**. `gpu-vast` is an unattested tripwire. |
| CI → Worker | Build provenance must not trust the developer's assertions or machine; putting a developer-controlled Cloudflare Worker in an attested result's path would bypass that independence. Worker/GPU results are never attested. The protected BASE runner validates structure, bindings and every displayed string itself, and independently scrubs/caps output. |
| Worker → Vast box/operator | These are community/consumer hosts with observable guest root. The box never receives the Vast account key, App key, owner stop token or Worker signing secret; it gets only its limited run capability. |
| Worker → CI content/secrets | The billed Vast key must **never be a GitHub secret**: LLM agents and future contributors may reach repository secrets. CI presents only OIDC identity; the Worker independently checks identity, approved content and spending policy. |
| CI → Vast | Box output can inject logs/commands; a community box cannot supply an attested result. CI sees Vast only through the Worker, with no direct box connection and no executable/artifact input from Vast. |
| Vast box → Worker, but not its network | The box trusts the Worker's fixed task/callback, uses **HTTPS only, no redirects**, and has a per-run token with no authority beyond report/kill/hold/blob-URL access for that run. Live authority dies with the box/run. The host operator can already see the box and its token; network interception must not add authority. |

[CODE READ: src/index.js, src/tokens.js] Current run authority is report/kill/hold; no blob-URL route exists. Blob-URL scope above is a policy ceiling. Completed kill receipts have no live authority.

[CODE READ: gpu-vast.yml, release_gate.py, release-build.yml] Trace: **approved commit → GitHub run/OIDC → Worker version and `kf-ci-<run_id>-<attempt>` label → untrusted result → runner verdict/`gpu-vast` status → separate GitHub build/provenance**. The same-SHA status is an operational gate, never an attested GPU result or provenance material. GitHub checks out the approved master SHA, builds final binaries there and attests their digests; no developer-machine or Vast binary enters that build.

[CODE READ: runner.py, ledger.js] A CSPRNG nonce, exact SHA and rolling hash bind received UTF-8 log chunks before local scrubbing. Invalid schemas/types/ranges, duplicate keys, bindings, version/hash, redirects, size or deadlines fail closed. Consistency cannot establish honest execution; prior approval prevents contamination even with a forged PASS.

[CODE READ: runner.py, gpu-vast.yml] Protected BASE code uses pull_request_target, Python -I, env -i and OIDC-only permission. Forks refuse; status writers are separate; no PR checkout/cache/artifact channel exists. The runner independently strips terminal/control/bidi/zero-width content, redacts tokens, neutralizes commands, caps bytes/lines and frames displays with fresh stop-commands tokens. Free text stays display-only.

[CODE READ: deployment.json, runner.py, release-build.yml] Public verification follows the approved commit, Actions log, expected Worker bundle hash, structured summary/status and GitHub provenance digest. Compare that digest with the obtained binary. The deployed Worker hash is an owner assertion, not remote attestation; excerpt redaction prevents reconstructing the full log chain publicly. Existing KVM, hosted OIDC, billed CPU, login-to-MANUAL and cap-confirmation gaps remain open; renting stays disabled. See [operating details](../../ci/vast-broker/README.md).
