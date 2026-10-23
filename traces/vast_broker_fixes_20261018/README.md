# V1 security-fixes verification — 2026-10-23

[SUPERSEDED IN PART 2026-10-23 by ../vast_broker_fixes2_20261023/README.md] The candidate bundle
below (da460023...) is replaced by 5ae53f61...; the Access, MANUAL/hold/stop-timer, review-mode,
dispatch-SHA and branch-copy items verified here were removed by the owner rulings of 19:10/19:25.

[CODE READ: ../../docs/design/V3_CI_VAST_FIXES_20261023.md] This round starts at
ffd1d1d518e8550e9b44e070f070ca78e5d37fad on the new branch
codex/vast-broker-fixes-20261023. The controller owns deployment and hosted acceptance.
No Vast or Cloudflare request, deployment, rental or host operation was performed.

[MEASURED: 2026-10-23 offline run v1-fixes] The pre-change suite passes 1113/1113.
The separate EXPECTED RED commits below precede implementation. Their counts describe
the focused run at each checkpoint, rather than mutually exclusive test sets.

| Finding / owner ruling | Tests | EXPECTED RED commit | Before: pass/fail |
| --- | --- | --- | ---: |
| N1/N2/F12 ingress, nonce and stale capabilities; N3/N4/N6/N8; F7 | round-fixes | 2b59db169 | 7/28 |
| F11 runner retry/cache; N5 quotas; N9 full-log controls | round-lifecycle | afee19f2e | 0/10 |
| Independent reaper and optional Access transport | round-reaper, round-access | 3ffbdb867 | 0/23 |
| Access route matrix; Worker GitHub token and formats | round-access-policy, round-owner-token | d2952890c | 11/79 |
| No credential-bearing asset redirects | round-owner-token | 75023263c | 11/2 |
| Fixed label-mismatch log and create-derived ownership | round-owner-token | 7d06ae676 | 13/2 |
| Access A secret placement | round-access | 5d79725a3 | 9/2 |
| Non-JSON HTTP errors preserve paid runs | round-lifecycle | 23decf8f3 | 11/2 |
| Pure owner branch workflow authority | round-authorization | 342011e98 | 0/32 |
| Owner push and selected dispatch | round-push | 881dd9b3f | 14/6 |
| Ancestor comparison path and lifecycle controls | round-push | 4815db508 | 21/1 |
| Manual selection associated with a PR | round-push | 0c1325fef | 22/2 |
| Reviewed YAML trigger/input/concurrency | round-workflow-policy | 2c7bdccee | 0/7 |
| Access owner diagnostics and dispatch input types | round-access-policy, round-workflow-policy | 507347e63 | 125/4 |
| Preserve liability when contract prices decrease | round-lifecycle | d693eb0fe | 13/2 |
| Access audience-array compatibility | round-access-audience | 6ca6d9e25 | 14/4 |
| Trigger policy belongs to trusted workflow code | round-trigger-policy | 8b1e13383 | 0/16 |

[MEASURED: 2026-10-23 selected-dispatch controls] Both lifecycle controls already
passed without adding a workflow-SHA field. The actual failing ancestor probe exposed
the generic path guard rejecting GitHub's comparison delimiter. The fix permits only
an exact hex-SHA comparison with fixed pagination; it retains traversal refusal.

[CODE READ: ../../docs/design/V3_CI_VAST_FIXES_20261023.md] The latest 18:20 ruling
supersedes prior Worker branch/path/draft trigger filters. Authorization still checks
signed identity, API actor and triggering_actor, repository, run, head and approval.
The workflow owns trigger choices. Its trusted BASE file-policy job fails safe to run
on uncertain metadata, explicitly succeeds on complete excluded changes, and leaves
unexplained skipped/failed jobs failing. Manual selection never changes controller BASE.
The YAML/controller changes are a separate commit for owner review.

[CODE READ: ../../ci/vast-broker/DEPLOY.md] A v2 rollout requires disabling admission,
draining existing boxes and using a repository SHA containing the updated reporter.
Do not discard the ledger. Configure the independent reaper in a separate quota domain
for request-quota independence. Its default threshold ends boxes older than 195 minutes,
including owner-extended runs. Per-GB price caps and reservations do not establish a hard
provider bill ceiling. Keep the prepaid account balance small.

[CODE READ: ../../ci/vast-broker/DEPLOY.md] Hosted OIDC/Access assertions, live contract
fields/scopes/billing, scheduler CPU/quota usage and the real KVM/GPU payload remain
unverified. The review's CLI transitive-integrity, display over-redaction, sweep
reachability, payload availability and deterministic-selection findings remain outside
this round. No hardware performance claim is made; hardware n=0.

[MEASURED: 2026-10-23 offline mutation campaign] Every case below has at least two named failing tests. Files are restored in finally; no source mutation is left active.

| Campaign | Mutants | Minimum named killers |
| --- | ---: | ---: |
| [authorization_mutants.json](authorization_mutants.json) | 26 | 2 |
| [blob_mutants.json](blob_mutants.json) | 9 | 2 |
| [bootstrap_mutants.json](bootstrap_mutants.json) | 3 | 2 |
| [country_mutants.json](country_mutants.json) | 9 | 2 |
| [identity_mutants.json](identity_mutants.json) | 8 | 2 |
| [legacy_mutants.json](legacy_mutants.json) | 94 | 2 |
| [rest_env_mutants.json](rest_env_mutants.json) | 3 | 2 |
| [round_mutants.json](round_mutants.json) | 61 | 2 |
| [runner_mutants.json](runner_mutants.json) | 38 | 2 |
| [security_mutants.json](security_mutants.json) | 14 | 2 |
| [v4_mutants.json](v4_mutants.json) | 28 | 2 |
| [v5_mutants.json](v5_mutants.json) | 73 | 2 |
| Total | 366 | 2 |

[CODE READ: ../../ci/vast-broker/DEPLOY.md; mutation-anchors.json] Owner amendments
moved the draft/base/path trigger decisions into YAML and trusted BASE policy code.
Historical probes now target those boundaries. Equivalent source-format anchors were
shortened where the removed trigger block had surrounded an unchanged binding guard.
The sweep probe breaks both its entry gate and the propagated authorization profile,
because the new independent check rejects an entry-gate-only mutation. Its threshold
remains two named failures. The round scripts now set their child test working directory
explicitly, so running them from the broker directory cannot misattribute fixture path
errors to mutations. All baselines must pass before any mutation attribution.

[MEASURED: 2026-10-23 final offline run v1-fixes] The final suite passes **1465/1465**,
with zero failures, cancellations or skips. Source format (Prettier), Black 25.1.0,
Python/TOML/workflow/helper syntax and diff hygiene checks pass. The six-file
YAML/controller change is commit **4dd877bed** for separate owner review. Worker,
reaper, box helpers and regression/mutation implementation are commit **76b5be2b2**.
Those implementation commits follow every EXPECTED RED row above.

[MEASURED: 2026-10-23 offline bundle build] Candidate single-file module: 128712 bytes, SHA256 `da460023ef114344e1172cc82d1641ed84b0fd8efd782acb35f8be973e5fa233`. The generated manifest, tracked deployment.json and wrangler.toml agree. This is not a deployment claim.

[CODE READ: ../../ci/vast-broker/DEPLOY.md] Owner follow-up: review the YAML/controller
commit separately, choose an independent reaper quota domain and the intended manual
horizon, then perform hosted/provider acceptance with admission disabled first. The
committed configuration keeps renting OFF. No branch-protection, secret, deployment or
account setting was changed by this round. The original root README and ci.yml have
zero changed lines against the round base. No merge-queue enablement is included.
