**STATUS: DESIGN-ONLY for deployment and combined hardware qualification, 2026-11-09.**
CODE READ: the request policy, payload guards and evidence checks below are implemented.
Only local fake-runner tests and offline Worker bundling are in scope; no box, VM, rental,
Worker/Cloudflare request or deployment was performed for this change. Large workload
execution remains DESIGN-ONLY; the LLM adapter is deliberately refused.

# Middle-tier real applications

## Tier contents (owner decision, 2026-11-09)

| Tier | Contract | Execution scope of this change |
|---|---|---|
| small, push | Today's check: Linux bar and required Windows gate | Unchanged; no Linux/Windows apps request |
| middle, PR to master | Small check plus the complete Linux middle gate list and Windows `mid` probe profile | Implemented, pending matching Worker deployment and first combined run |
| large, release | Everything in middle plus SPECviewperf and a real LLM workload | Intent only; no new large-tier execution wiring |

CODE READ: `gate_list_middle.txt` names **63 Linux apps**. It is unchanged. The full supported
list has 66; `vkpeak`, `clpeak` and `gpu_burn` were already outside the middle list. This change
removes no app. The four known-unsupported UVM entries remain explicitly reported, with no
passing credit. Windows `gate_profiles.json` names **8 small** and **10 mid** apps; every small
id is in mid. Therefore the request is `mid`, with no ad hoc union. Manual and unsupported
Windows ids are outside this profile. The catalogues and both trusted count tables have tests.

CODE READ: `TIER_CONTENTS` in `src/tier.js` records the inheritance. Both
`ci/vast-broker/defaults.json` and the payload's `STAGE_SLOTS` reserve `llm_parity` for large
with state `DESIGN_ONLY`. Owner configuration cannot enable it through `RELEASE_STAGES`;
`KF_STAGES=llm_parity`, alone or with SPEC, is refused before work. The existing fat-guest
`llm_parity.sh` is not a CI stage adapter and is not invoked here. A short llama.cpp probe in
the app list is not qualification of that real workload. SPEC keeps its existing `KF_STAGES`
/ `KF_SPEC` plumbing. No LLM execution is wired and no large-tier qualification is claimed.

## Trusted selection and the two opt-ins

CODE READ at base `3c5c21e49`: Linux selection was a source-only `APPS_STAGE_ENABLED=False`
flag in `vast_payload.py`, enabled for middle/large when changed by the owner. There was no
Linux request field. Windows selection was `KF_WINAPPS=small|mid|full`; `tierEnvironment`
accepted `winApps`, but `/rent` rejected that body key and admission never forwarded it.
The existing trusted runner knew only nonce, SHA, profile and optional die preferences.
Consequently a BASE-runner-only change would not work with that Worker bundle.

CODE READ: BASE `runner.py` now always requests the policy extension of the existing
OIDC-authenticated `GET /availability?profile=check&apps=1` path (sweep uses its own profile).
There was no existing apps-config path. This opt-in extension preserves the old seven-field
availability response for old callers. Its `apps_policy` contains only run id, canonical tier
and an enabled boolean. The runner validates its run binding, tier/profile and exact types;
a missing or malformed policy fails before `/rent`.

CODE READ: the Worker derives the tier from signed GitHub claims via `tierForClaims`, not
from PR inputs, request body tier names or informational box fields. For enabled middle only,
the trusted runner sends `linuxApps: true, winApps: "mid"`. Admission independently requires
exactly that pair. Missing, false or substituted options are refused, and small/push cannot
request either. There is no caller override of tier or switch policy. BASE `main()` does not
read any apps toggle from the event's inputs.

CODE READ: admission sends `KF_TIER=middle`, `KF_LINUX_APPS=1`, `KF_WINAPPS=mid` and the
unchanged `KF_MERGE_WINDOWS=1`. The payload consumes these transport flags before running
children. Explicit Linux selection is valid only for middle. The legacy source flag remains
false for old/manual callers. `KF_STAGES` still cannot select Linux apps. Stage order is the
Linux bar, Linux apps, Windows gate, then Windows apps. Every stage is graded independently.
The Linux and Windows app graders bind their output paths when the stages are created;
otherwise reusing the same local variable would make Linux read the later Windows log.

## Owner kill switch

CODE READ: the public Worker var **`MIDDLE_APPS_ENABLED`** is exactly `true` or `false`;
unset defaults to true under this owner decision. False disables both new requests for new
runs, restoring today's check. It is validated by the Worker and `deploy.mjs`, and exposed
through the authenticated availability extension. It is not read from PR inputs or a rental
request. Change the Worker binding to false to stop asking for apps without a source change.
A change between policy read and admission rejects the stale request before creating a box.
It does not cancel an already admitted run; the existing owner stop control remains separate.

## Budget: estimates are not hardware qualification

| Component | Existing observation | Admission allocation |
|---|---|---:|
| Windows gate | OWNER-REPORTED, 2026-11-09: last passing AD102 gate took 3,847 s | 4,500 s / 75 min |
| Linux middle | MEASURED historical run `g2`, RTX 4090, 2026-11-07: 63/63, 451 s; watched `g3` stage 456 s | 3,000 s matrix + 600 s margin = 60 min |
| Windows apps | MEASURED historical `wa1`, rev `b11655b7`, 2026-11-07: small 8/8, 735 s; mid's 10-app duration has no record found here | 3,600 s profile budget + 600 s margin = 70 min |

Historical sources: [`apps_linux_20261107`](../../traces/apps_linux_20261107/README.md),
[`g2 output`](../../traces/apps_linux_20261107/g2/gate_mid.out),
[`watched g3`](../../traces/apps_linux_20261107/g3/gate_watched.out),
[`Windows small output`](../../traces/winapps_20261107/small/wa1_winapps.log), and
[`winapps_20261107`](../../traces/winapps_20261107/README.md). The Windows full run was a FAIL,
not a successful mid timing: its stage took 5,624 s with 58 pass, 9 fail/timeout and 12 not run.
No timings from these different runs are presented as a combined middle measurement.

INFERRED admission envelope: the suffix is **60 + 75 + 70 = 205 min**. Of the 300-minute
hard limit, the existing 10-minute soft margin leaves 290 minutes; at most **85 minutes**
remain for provisioning, build, merge bar and CUDA before this suffix. Within the 205 minutes,
the gate allocation has 653 seconds above the owner's AD102 sample, and the two apps stages
have ten minutes each beyond their configured matrix budgets. This is a conservative declared
budget, not a proof that every die/media setup will finish in time. The small timing is not
substituted for an unmeasured mid duration. No additional app has to move to large to make this
declared envelope fit. The selected lists are never shortened to manufacture a fit.

CODE READ: the existing `execute_stages` remaining-time guard is reused. Linux apps need the
whole 205-minute suffix before they start; Windows gate needs its 75 minutes plus the requested
apps' 70; Windows apps need 70. A clock/budget shortfall starts no corresponding child and
records `apps_retry_time`, `windows_retry_time` or `windows_apps_retry_time`. The payload check
fails, and the trusted runner exposes RETRY. It is never PASS or credit for an app that did not
run. If earlier work exceeds the available 85 minutes, retry rather than silently dropping apps.
An actual overrun remains bounded by the existing soft deadline and cannot produce PASS.

## Evidence and verdicts

CODE READ: the payload retains its exact per-app/grade completeness checks. A requested stage
with empty, truncated or inconsistent output becomes an OPERATOR retry. Missing media is MEDIA:
new Linux stage executions emit `APPS_STAGE_RETRY class=MEDIA`, including a missing or empty
bundle beside a stale provisioning marker, before launching the matrix; historical `NOT_BUILT` is accepted
and normalized to the same failing `apps_retry_media` row. Windows keeps its MEDIA rule.
The frozen Bash fixture is unchanged; only that documented historical nonpassing spelling is
normalized in migration equivalence tests. Both old and new outcomes remain RETRY / rc 3.

CODE READ: the Worker freezes whether apps were requested on each rental. A claimed PASS
without exactly the 63/10 successful suite counts is narrowed to FAIL with an appropriate
`*_retry_operator` row, bounded to the result row cap with unique names. An attributed FAIL
is never relabelled by that guard. The BASE runner independently checks the same counts before
accepting PASS. Its retry output now includes `apps_retry_*` and `windows_apps_retry_*`, while
actual `apps` or `windows_apps` guest failures remain FAIL. Manual/unsupported entries never
become passing app evidence. No silent successful no-op remains at any of these three layers.

## Bundle and rollout

CODE READ: `tier.js`, `policy.js` and admission changed, so **a Worker redeploy is required**.
`deployment.json` and `wrangler.toml` carry the same regenerated bundle hash. It must remain
paired with the trusted BASE runner. No status/result protocol bump is needed: the availability
extension is explicitly requested and old response shapes remain unchanged. A new runner
against an old Worker fails before renting; an old middle runner against the enabled new Worker
is rejected for omitting the required pair. These are failures, not compatibility passes.

DESIGN rollout recommendation: keep rentals paused during the BASE/Worker cutover, install the
matching pin and bundle, then enable rentals with the intended switch value. No deployment
was performed here: Wrangler was used only with `--dry-run` in an isolated, credential-free
configuration. Linux's 60-minute allocation assumes prepared application media. This request
policy does not enable the separate two-hour `KF_APPS_PROVISION` path; a fresh box without its
Linux bundle returns MEDIA before the matrix. Windows keeps its existing pinned public-media
fetch and MEDIA checks. Confirm media availability during the coordinated rollout; the source
change neither uploads nor packages application bundles. First combined middle hardware
qualification remains the coordinator's work.

## GPU-free validation commands

The focused suite is `python3 -m unittest discover -s scripts/ci -p test_middle_apps.py` and
`node --test ci/vast-broker/test/middle-apps.test.js`. The mutation gate is
`python3 scripts/ci/middle_apps_mutants.py`: each variant must have two distinct named killers,
with an unchanged green baseline. CI runs it with the existing broker/helper checks. Coverage
includes signed-tier and PR-input isolation, owner on/off/invalid settings, push exclusion,
required stage rows/counts, guest FAIL priority, media retries, independent log paths,
downstream time reservations, subset/count policy and refused LLM intent. Final run counts and
pin receipts are recorded with the PR; none qualify new hardware.
