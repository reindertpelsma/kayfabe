# Same-tree GPU gate reuse

Addendum2, 2026-10-28: CODE READ (`gpu-vast.yml`), both workflow path lists now include anchored `site/**`. Owner reports PR184's documentation/Site-only change rented unnecessarily; Site content/tooling does not ship GPU-box code and is gated by the Site workflow. The actual trigger/Github source in orchestration V8 produced6 expected Site-only failures/24 controls before the change (EXPECTED RED921cecd4b), then32 passing cases after it, including root README.md/CONTRIBUTING.md, mixed crates/scripts, sitemap/sitegen prefixes, nested non-root site directories, deep Site files and uncertain counts. MEASURED (`site-skip-proof.json`):6 additional mutants have>=4 named killers each, including anchoring, depth, mixed-file every/some and root Markdown behavior. This is a configuration-only matcher extension: trigger-policy.mjs and Worker files are unchanged. Final hosted verification covers the new committed Node cases; no local build was started below the shared disk floor. The original22 private-copy mutation results below remain for the unchanged reuse implementation.

STATUS: PROPOSED, 2026-10-27. Base master commit e54bab5550f09d740d222d667ad497dc8c640ebe; PR target next. This change is never deployed or merged by WK1 and no rental workflow is dispatched for validation.

Hypothesis and falsifier before tests: identical trees with complete trusted GPU/CI provenance can reuse evidence without contacting the Worker; falsifier is current release/workflow policy already accepting that strict proof and replaying complete readable logs. EXPECTED RED64644bcda records52 failing new policy/release cases; existing17 release controls pass. Additional EXPECTED RED82ef087a6 records4 workflow failures;22c36ae5a records2 guard failures/10 controls;8fdb176b9 records4 FF/order failures/104 controls. Tests subsequently pass.

MEASURED 2026-10-27 (`public-parent-proof.json`): a read-only probe finds source858ca1e8ad4a2b264656b8e499d820856bcc7310 for mastere54bab5550f09d740d222d667ad497dc8c640ebe. Both trees are626fb45822b039801d3d254bebc897030581d1bc. Original [GPU run40588779558](https://github.com/reindertpelsma/kayfabe/actions/runs/40588779558) has statusGRACE from GitHub Actions and matching successful CI/run metadata. The probe retrieved its original rental, die, tested revision and final verdict/stage lines. It performed GETs only, no status write, Worker/provider call or paid run. The observed combined-status response omitted creator; the status-list endpoint retains it and is used for proof.

MEASURED 2026-10-27 (`validation.json`): all2454 Node tests pass, without skips, including original runner/status/bootstrap regressions and deterministic Worker bundle verification. New cases cover direct merge parents, associated PR heads and same-SHA fast-forwards; failures of tree/status/creator/CI/run bindings; incomplete/failed/revision-mismatched logs; real Bash final-status decisions; actual Python replay logs/summary without API contact; and independent release proof. The only final formatting adjustment preserves an identical Python AST. Mutation proof and named killers are in `mutations.json`; reproduce with `nice -n 19 python3 traces/gate_reuse_20261018/mutants.py`.

CODE READ 2026-10-27 (`gate_reuse.py`, `gpu-vast.yml`): only push/master probes. Exact-commit HTTPS source downloads retain size/time bounds. Pending/report job names remain; the original GPU job executes a bounded replay step instead of the Worker step. All reused-run jobs have ordinary successful conclusions and readable logs. Missing/truncated/expired evidence falls back to the existing gate. The report requires successful dependency/replay results. Forward links use a separate source-commit status context, preserving the original GPU status. Fast-forwards inspect the existing proof before posting pending and retain bounded same-SHA status provenance. Cross-SHA reuse chains and path-policy NOT_NEEDED are deliberately not promoted to hardware evidence.

CODE READ 2026-10-27 (`release_gate.py`): direct same-master-SHA validation retains existing controls; reused statuses independently recheck candidate membership, full tree equality, original status creator/state, CI creator/state and original workflow/run/SHA. A copied description alone cannot pass. Release builds get only an additional checks/read permission; their build inputs, artifacts and attestation boundary are unchanged. The policy uses GitHub's [check-run API](https://docs.github.com/en/rest/checks/runs#list-check-runs-for-a-git-reference) and [commit statuses](https://docs.github.com/en/rest/commits/statuses).

Risk: reuse trusts GitHub's metadata and an unattested GPU tripwire; it does not attest rented hardware. Original provider lines remain untrusted and only fixed literal summary formats are replayed. API/log uncertainty can cause an extra real gate after deployment of the workflow, as authorized by the owner. No Worker src/defaults/pins/secrets/configuration changes are present. CI itself is unchanged; its existing parent check is evidence for GPU reuse. Hosted master-push execution and live release dispatch are not performed or claimed; both require owner review/merge. PRs against next do not match the current GPU workflow's master-only PR trigger.

| Mutant | Named killing tests |
| --- | ---: |
| tree-mismatch | 6 |
| non-success-status | 9 |
| wrong-creator | 6 |
| pending-parent | 3 |
| pending-ci | 3 |
| tag-push | 3 |
| pr-event | 3 |
| wrong-ci-app | 3 |
| wrong-run-revision | 3 |
| wrong-workflow | 3 |
| log-completeness | 9 |
| log-revision | 3 |
| release-forged-child | 2 |
| release-lost-parent | 6 |
| replay-contacts-api | 3 |
| replay-script-guard | 2 |
| broker-branch-guard | 2 |
| pending-write-guard | 2 |
| report-reuses-failed-pending | 2 |
| report-reuses-failed-policy | 2 |
| report-reuses-failed-policy-check | 2 |
| report-reuses-failed-gpu | 2 |
