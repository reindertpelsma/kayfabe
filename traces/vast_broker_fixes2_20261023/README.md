# Vast Worker fixes2 verification - 2026-10-23

[CODE READ: /root/vast_worker_review_v1_20261023.md; owner rulings 19:10 and 19:25] This round
starts at ad7ca535a784ece692a759b7f66598f6ce07a227 on branch claude/vast-broker-fixes2-20261023.
It made no Vast, Cloudflare or GitHub-write call and deployed nothing. Two read-only `gh api`
reads were made: the `actions/setup-node` v4 tag object and the shape of public
`pull_request_target` runs (github/docs).

| Finding / ruling | EXPECTED RED | Fix |
| --- | --- | --- |
| V11 cwd-relative test paths | de4f58804 | d3a865b31 |
| 19:10/19:25 simplification and removals | 67febe6c4 | 73c566972 |
| V1 owner id / named config error | 6bf6486f2 | 1bfd711d2 |
| V6 box protocol marker before renting | a373b2f0d | d1542197b |
| V9 reaper continues past a failed delete | 99fd98342 | 9dc85e820 |
| V7 pin, second policy decision, no SHA inputs (workflow) | 1ae568164 | cb2359c8e |
| V4 deploy helper without Access vars | 333e90a68 | 94334daf2 |
| V12, V2, docs | 0c97f9aae | 5d87a3bdc |
| Bundle hash | - | af635530b |
| Mutation tables realigned | - | 7d9f91827 |

[MEASURED: 2026-10-23 offline, Node v22.22.1, Python 3.14.4] Suite: 1369/1369 pass, 0 fail, 0
skipped, both from the repository root (`node --test ci/vast-broker/test`) and from ci/vast-broker
(`npm test`). Box unittest suite (scripts/ci): `python3 -m unittest discover -s scripts/ci -p
'test_*.py'` 104/104 OK (it was 44 failures and 6 errors at the base commit, because its fixture
token still used the v1 format). Prettier 3.6.2 and Black 25.1.0 checks pass. Claims gate: new debt
0.

[MEASURED: 2026-10-23 offline bundle build] `node bundle.js`: dist/worker.js 110401 bytes,
SHA256 `5ae53f6172822629e2fff70c44ac4d0accec4fb5c93c2d11349d025cafc67962`; deployment.json,
dist/deployment.json and wrangler.toml agree. Not a deployment claim.

[MEASURED: 2026-10-23 offline mutation campaigns, run with PYTHONDONTWRITEBYTECODE=1 and
caches cleared before each campaign] Every case has at least two named failing tests; sources
are restored in finally.

| Campaign | Mutants | Minimum named killers |
| --- | ---: | ---: |
| [authorization](authorization_mutants.json) | 12 | 2 |
| [blob_mutants.json](blob_mutants.json) | 9 | 2 |
| [bootstrap_mutants.json](bootstrap_mutants.json) | 3 | 2 |
| [country_mutants.json](country_mutants.json) | 9 | 2 |
| [identity_mutants.json](identity_mutants.json) | 6 | 2 |
| [legacy_mutants.json](legacy_mutants.json) | 91 | 2 |
| [rest_env_mutants.json](rest_env_mutants.json) | 3 | 2 |
| [round_mutants.json](round_mutants.json) | 88 | 2 |
| [runner_mutants.json](runner_mutants.json) | 36 | 2 |
| [security_mutants.json](security_mutants.json) | 11 | 2 |
| [v4_mutants.json](v4_mutants.json) | 28 | 2 |
| [v5_mutants.json](v5_mutants.json) | 62 | 2 |
| Worker/runner total | 358 | 2 |
| [box, scripts/ci](box_mutants.log) | 52 | every named killer fails |

[MEASURED: 2026-10-23 offline] A stale `__pycache__/smoke.cpython-314.pyc` written while a
legacy mutant had modified `smoke.py` survived the restore (same size, same-second mtime) and
failed two tests afterwards. The final run therefore disables bytecode caches; the
mutation scripts themselves do not.
