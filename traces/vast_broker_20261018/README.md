# V1 Vast broker evidence — 2026-10-22

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

## Endpoint secret and prefix proof (owner request, 2026-10-22)

[CODE READ] VAST_BROKER_URL is an Actions secret; optional VAST_BROKER_PATH matches
Worker secret BROKER_PATH_PREFIX. Only the broker execution step receives them.
The origin remains the OIDC audience; the API/callback URL adds the optional path.
A generic 404 gate with native constant-time verification precedes ledger/auth.
Known endpoint parts are redacted from box text; shell tracing and raw transport
errors are suppressed and derived values masked before requests. URL secrecy is
scanning/DoS-noise defence, never authentication; the box/onstart reader knows it.

| Proof | EXPECTED RED commit | Before |
| --- | --- | --- |
| Secret scope/guard and prefix routing | a64b34ce7 | 83 total, 5 pass, 78 fail |
| Actual shell request/mask/error/trace behaviour | 1eb9df642 | 10 total, 0 pass, 10 fail |
| Scalar/folded run guard bypass | 0de729c3f | 39 total, 35 pass, 4 fail |
| Reusable inheritance and step-name aliases | a8042039a | 5 total, 1 pass, 4 fail |

[MEASURED: implementation checkpoint 833a1a865] The complete Node suite passes
473 tests with zero failures/skips. Actual generated Python sender tests now use
a prefixed callback for both success and failure exit statuses. Root README and
the existing CI two-line insertion are unchanged. The first 94-case campaign passed after correcting broker sink fixtures; the
unchanged logging mutation kills two tests (4d50d443a). Final inherited-secret and
metadata probes add two guard checks. The expanded 96-case campaign retains the
75 original policy cases; final counts
and restored-source results are recorded in the mutation table.

## Amendment 3/3b callback proof

[CODE READ: 2026-10-22] Amendment 3 replaces log retrieval/marker parsing with a
per-run HMAC callback. Amendment 3b treats that bearer as hostile input authority
only: strict stream/schema/depth/sequence/count/byte/rate gates, live ledger plus
current Vast status, immediate revocation on result/stop/TTL and fail-closed
provider errors. Status returns a sanitised tail. Owner-only stop-all and same-run
OIDC stop delete only recorded owned labels. No Vast log API request is needed.

| Proof | EXPECTED RED commit | Before |
| --- | --- | --- |
| Reports, token binding, liveness, caps, stop | cf31b9925 | 64 tests: 3 pass, 61 fail |
| Fixed callback sender, step token binding, outbound model names | ed5c287b3 | 14 tests: 0 pass, 14 fail |
| Partial ledger cleanup and embedded command delimiters | 95c148fbe | Reporting: 76 pass, 3 fail; sanitizer: 28 pass, 3 fail |

[MEASURED: 2026-10-22 checkpoint] Implementation 48e988538 passes 367 tests.
Cleanup/delimiter fix 7f1305566 passes 376 tests, zero failed/skipped. Tests run
the actual generated Python sender with mocked workload/curl and execute the
actual workflow shell with fake curl/sleep. Full raw output stays only in the
mocked local backup; structured verdict follows exit status. Tokens are not
returned to the runner or retained in ledger scripts. Every rendered box line
has a visible prefix; neither command delimiter survives anywhere, including
legacy markers embedded inside a line. The final mutation results follow below. The initially surviving free-text-verdict mutation is killed by three terminal-state assertions after strengthening those same negative tests; the mutation was not weakened.

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

## Final Amendment 3/3b verification

[MEASURED: 2026-10-22] The amended campaign kills all 75 hand-authored mutants,
with at least two independently named test failures each. The current
[complete table](../../ci/vast-broker/test/mutants.md) records counts and example
killers; mutants.js contains every exact patch. Source restoration was checked
against the implementation commit before the final full run. Worker mutations
run all eight source-facing test files in parallel; workflow mutations also run
the shell-client fixtures, which read only YAML and never import Worker source.
The final full command remains node --test ci/vast-broker/test (376 tests).

[MEASURED: 2026-10-22] Python CI tests: 39 passed; dependency isolation: 22 crates,
0 violations; frozen prototype: 37 files, 0 changed; claims ratchet: new=0.
Secret/hygiene scans cover each staged diff plus the complete branch diff and
commit history. Root README is unchanged; existing ci.yml adds only lines 51–52.
No live API, provider, deployment, rental, GPU or host validation is claimed.

## Historical pre-callback commands and mutation table

[MEASURED: 2026-10-22 fresh backup run] node --test ci/vast-broker/test: 291 passed, 0 failed,
0 skipped after the final mutation campaign and source restoration. The historical pre-callback campaign killed all 51 mutants, each by at least two named tests; the new table supersedes that source baseline. node --test uses Node built-ins,
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
metadata, callback delivery/onstart, actual Cloudflare CPU/limits and
provider billing/debt/key restrictions need an owner-authorized smoke run.
Nominal compute allowance is 1.25 USD per 150-minute rental, with 3 USD daily and
30 USD rolling admission estimates; failed cleanup can continue billing beyond
these estimates. At the 0.50 USD/hour compute cap an undeleted box adds 12 USD/day
or 360 USD/30 days plus fees. No finite hard maximum is proven with auto-funding.
Only a future explicitly authorized deployment can test those contracts.
