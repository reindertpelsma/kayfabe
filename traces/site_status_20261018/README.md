# Always-reported Site status

STATUS: PROPOSED, 2026-10-27. Base master commit e54bab5550f09d740d222d667ad497dc8c640ebe; independent PR against next. No merge, Pages/settings change, deployment or rental is performed by WK1.

Hypothesis and falsifier before tests: Site can report on every PR without building unrelated files or exposing a write token to PR code; falsifier is the current filtered workflow already doing so. EXPECTED REDd7ea9b768:52 failures; a6115621b:68 failures including reuse; ef542cc72:13 workflow/boundary failures and2 controls; bc39d25fa:2 tag/release source-pin failures and2 controls. Final expected-red2d9042221 adds master-checkout binding, whose old expression returned mutable master for both pushed SHA controls in pure V8 evaluation.

MEASURED 2026-10-27 (`validation.json`):2447 local Node tests pass without skips before the final push-checkout fix; an additional101-case focused suite passes. Existing Site unit tests run76 with2 external-tool skips and no failures. All27 isolated mutants have>=2 named killing tests in `mutations.json`. Pure V8 evaluation verifies the final checkout expression for PR head, two distinct master pushes and tags. The final full tree is validated by hosted CI because shared disk fell below the lane floor after completed local checks. No further local build was started under that floor.

CODE READ 2026-10-27 (`site_policy.py`, `site.yml`, `site-status.yml`): complete PR file counts/pages and both names of a rename decide the exact three requested path classes. Unknown metadata builds rather than skips. Master publication additionally requires a confirmed path change; unknown push metadata fails and cannot silently publish. Existing tag/release/scheduled/manual rendering/deployment paths keep master tooling. Exact PR-head and pushed-master checkout bindings prevent a status from describing a different tree.

CODE READ 2026-10-27 (`site-status.yml`, `workflow-guard.js`): an independent API-only reporter handles protected PR-target and Site-workflow notifications. No checkout, untrusted artifact, source code, Pages credential or OIDC token enters its status-writing job. The Site build retains pull_request and a read-only token. This split is necessary for fork commit statuses: GitHub documents [read-only fork PR tokens](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#workflows-in-forked-repositories). The guard pins the exact reporter step and minimal permissions, plus the Pages event/dependency/path authorization boundary. Actual fork status writes remain unverified until owner merge; unit callbacks use simulated metadata.

CODE READ 2026-10-27 (`site_policy.py`): parent Site reuse checks exact tree, successful bot status, CI app/conclusion, original workflow/run/repository/SHA, and forbids tag/PR reuse. A necessary deployment still builds its own Pages artifact. Completion independently checks the build result; a skipped master build must have a typed, GitHub-bound policy annotation preserving the actual push range, including multi-commit pushes. No arbitrary artifact supplies that range. Source/callback mismatches cannot post success; delayed start events do not clear final same-SHA results.

Risk: GitHub metadata and normal CI tests remain the authority, not an attestation of HTML or release binaries. Missing API/annotation evidence blocks success. Fork builds run arbitrary proposed source with only a read-only token; the separate status reporter executes only protected default-branch tooling. Tags retain master tooling. No Worker configuration, secrets, protection rules or Pages settings change. The owner adds required context site only after merge. PRs target next and cannot trigger the existing master-only GPU workflow.

Reproducer: `nice -n 19 python3 traces/site_status_20261018/mutants.py`. Mutants use private source copies without dependencies or network calls. API comparison truncation and pagination limits follow GitHub's [comparison API](https://docs.github.com/en/rest/commits/commits#compare-two-commits); reaching the300-file cap is treated as uncertain, not an unchanged site.

| Mutant | Named killing tests |
| --- | ---: |
| ignore-site | 6 |
| ignore-docs | 8 |
| ignore-workflow | 2 |
| ignore-renamed-source | 2 |
| uncertain-paths-skip | 14 |
| uncertain-push-deploys | 14 |
| pr-deploys | 6 |
| publication-skips | 8 |
| pr-count-unbounded | 2 |
| push-files-truncated | 2 |
| site-tree-mismatch | 2 |
| site-foreign-status | 2 |
| site-pending-status | 2 |
| site-failed-ci | 2 |
| site-pending-ci | 2 |
| site-forged-ci | 2 |
| site-wrong-run-sha | 2 |
| site-failed-run | 2 |
| site-tag-reuse | 2 |
| site-pr-reuse | 2 |
| site-foreign-source | 2 |
| site-other-workflow | 2 |
| late-start-clears-final | 2 |
| late-running-clears-final | 2 |
| site-skipped-needed-build | 2 |
| tag-tooling-executes | 2 |
| status-guard-accepts-source | 2 |
