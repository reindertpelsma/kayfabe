# V1 Vast broker evidence — 2026-10-21

[CODE READ: 2026-10-21] STOP: the provider VM/onstart launch contract conflicts
with strict no SSH. This is local prototype evidence, not a completed/deployable
deliverable. See the design STOP section.
[CODE READ: 2026-10-22 owner NIGHT RUN] The owner authorized committing and pushing
this blocked prototype for GitHub backup and opening a draft PR. That publication
authorization does not resolve the launch decision or authorize deployment/rental.

[MEASURED: 2026-10-21] Worktree based on origin/master e64b8e5f6 (PR #64), lane
codex/vast-broker-20261021. All API/account/billing tests use mocks and generated
local RSA keys. Hardware n=0; no rentals, deployment, SSH or GPU work.

[CODE READ] The implemented policy follows owner Amendments 1 A–E and 2 through
2e. The [design](../../docs/design/V3_CI_VAST_GPU_20261021.md) records assumptions,
primary API documentation, falsifiers, deployment and residual billing risk.
The scoped [README](../../ci/vast-broker/README.md) has nine owner setup steps.
Root README.md is unchanged. Merge-group support is deferred by owner ruling.

## Tests first

[MEASURED: 2026-10-21] Each listed focused run preceded its corresponding fix;
test changes were committed alone with EXPECTED RED. The first implementation
is a new feature, so its initial red run has missing-module failures. Later
focused runs used the implementation under development, with assertion failures
for the named cases. Original superseded TypeScript/Vitest tests are historical,
not counted as validation of this JavaScript deliverable.

| Proof | EXPECTED RED commit | Before |
| --- | --- | --- |
| Amended JS admission/account/log/workflow tests | 7ff50febd | 175 failing cases before implementation |
| Owner push/drafts/supersession amendment | bc9c8d909 | 189 failing cases before implementation |
| Scheduled handler wiring | 041cea9e9 | 2 focused failures |
| Documented offer representation | d7c04b94b / 272e2a871 | 3 / 1 focused failures |
| Deliberate control harness | 1eb3c3622 | 1 focused failure |
| Workflow client, corrected mock serialization | fe57dbffe | 2 pass, 4 fail |
| Sanitizer command-prefix edge cases | 5919fca1a | 4 focused failures |
| Quoted-job/terminal-client edges | 8817fe843 | 2 focused failures |
| Cold-box disk preflight | bd77804f4 | 4 focused failures |
| Base, exclusions, head race, branch dispatch, request token | 73fd4529c | 37 pass, 17 fail |
| Read-only broker and explicit final status | 2ac4b753a | 26 pass, 11 fail |
| Public GitHub fallback and mandatory status writers | ec6c8c815 | 83 pass, 4 fail |
| Token-shaped log redaction | cfbfbb876 | 42 pass, 2 fail |
| Cache metadata for a second PR sharing the SHA | cced69f56 | 81 pass, 2 fail |

[MEASURED: 2026-10-21] The earlier client fake-curl fixture split status JSON
incorrectly; fe57dbffe fixes that fixture and supplies the meaningful assertion
red proof. Its corrected result supersedes the overstated earlier commit-message
count. No fixture error is counted as a sanitizer/client defect proof here.

## Final commands and mutation table

[MEASURED: 2026-10-22 fresh backup run] node --test ci/vast-broker/test: 291 passed, 0 failed,
0 skipped after the final mutation campaign and source restoration. All 51 mutants were killed, each by at least two named tests. node --test uses Node built-ins,
generated JWKS, mocked storage/GitHub/Vast and a local HTTP integration server.
Actual workflow bash runs against fake curl/sleep: PASS/FAIL/refusal/pending,
NOT_NEEDED, cancelled/skipped dependencies and hostile output are exercised.

[CODE READ] The [mutation table](../../ci/vast-broker/test/mutants.md) records every
hand-authored patch, named killers and count. The runner checks syntax first,
requires at least two independent named failures, restores source in finally
and rebuilds the dashboard bundle. Ordinary tests must pass again afterward.

[MEASURED: 2026-10-21] python3 -m unittest discover -s scripts/ci -p 'test_*.py':
39 passed; dependencies.py: 22 crates, 0 violations; debt.py frozen: 37 files,
0 changed; debt.py claims: no new unsupported prose. No Rust sources changed;
Cargo builds and full hosted Rust CI were not run in this lane.

## Exact existing CI change

[MEASURED: 2026-10-21 diff] Only these two lines are added to ci.yml after the
stable job's checkout (new lines 51–52):

```yaml
      - name: Vast broker mocks and workflow boundary
        run: node --test ci/vast-broker/test
```

[CODE READ] gpu-vast.yml is a new workflow: status-only pending job, read-only
GitHub plus OIDC broker/path-filter job, and always-running status-only final
job. No box log is written to a workflow output; only fixed allowlisted Worker
state/verdict metadata is passed to the status writer. No artifact channel or
attestation/release dependency exists. Workflow changes remain review-sensitive.

## Limits

[INFERRED from source: untested live contracts] A lying box can fabricate PASS.
Cold-build duration/download volume, KVM keyless onstart, live OIDC/run/head
metadata, asynchronous log URL/completeness, actual Cloudflare CPU/limits and
provider billing/debt/key restrictions need an owner-authorized smoke run.
Nominal compute allowance is 1.25 USD per 150-minute rental, with 3 USD daily and
30 USD rolling admission estimates; failed cleanup can continue billing beyond
these estimates. At the 0.50 USD/hour compute cap an undeleted box adds 12 USD/day
or 360 USD/30 days plus fees. No finite hard maximum is proven with auto-funding.
Only a future explicitly authorized deployment can test those contracts.
