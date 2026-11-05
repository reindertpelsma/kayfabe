# Vast GPU CI broker

STATUS: LIVE, 2026-11-05 (tiers, admin start and the release run added by branch
`claude/worker-tier-20261105`, not deployed; everything below that section is unchanged).

The broker connects GitHub Actions to rented NVIDIA GPU machines on Vast.ai so kayfabe can run GPU checks beyond the hardware available to hosted runners. It is a Cloudflare Worker backed by a Durable Object ledger that tracks rentals, deadlines and spending. GitHub asks for a check; the Worker handles the provider account and relays the box's result and bounded logs back to the runner.

## How a GPU check uses it

1. The [GPU workflow](../../.github/workflows/gpu-vast.yml) decides whether a master push or PR needs a check. It fetches the controller from the trusted master base, without checking out proposed code on the GitHub runner. Two independent path decisions must agree before it reports a check as not needed.
2. The controller presents GitHub OIDC identity. The Worker checks the pinned workflow, the exact run attempt's owner triggering actor and the exact tested commit, then applies offer eligibility, concurrency and spending policy before creating a rental.
3. The box bootstrap checks out that exact commit and runs the [repository-side payload and reporter](../../scripts/ci/README.md). Its scoped run token lets it report progress, logs and a structured result, request its own destruction, and obtain permitted private asset download URLs.
4. The GitHub controller independently validates the result bindings and log hashes, scrubs displayed output and supplies the workflow's `gpu-vast` status. After fetching the final log it requests immediate destruction; result grace and Worker deadlines provide fallback cleanup.

## Trust and spending boundaries

The Vast box and its operator are untrusted. The Vast account key stays in the Worker, outside GitHub secrets and rented machines. A box receives only its run capability and, when configured, a Cloudflare Access service-token pair. GitHub communicates through the broker and accepts no executable or build artifact from the box. A GPU PASS is an unattested tripwire: a dishonest box or Worker can forge a verdict. It supplies no release provenance. See the [trust and release trace](../../docs/design/V3_CI_VAST_RUNNER_TRACE_20261022.md).

The ledger reserves liability before rental and applies daily and rolling budgets, a prepaid credit reserve and limits on concurrent work. The default check policy is $1 effective/hour, $6/day, $30 per rolling 30 days and an $8 credit reserve, with one active rental and a 300-minute hard deadline for the check profile, which runs the Windows stage of the master gate ([howto](../../docs/howto/gpu-vast-windows.md)); the sweep keeps three hours. The separate owner-dispatched sweep profile has its own hourly and daily caps. Transfer allowances are reservations, not provider-enforced usage limits. Missed cron runs or provider failures can leave a box billing beyond its deadline; these controls do not guarantee a hard external bill ceiling.

## Current setup and further reading

[CODE READ: wrangler.toml, gpu-vast.yml, gpu-vast-sweep.yml] The public broker origin is
`https://ci.kayfabe.dev`; Worker var `WORKER_URL` and GitHub environment `gpu-vast` variable
`VAST_BROKER_URL` must match because the origin is the OIDC audience. The path and both Access
credentials stay secrets. Protect the whole `ci.kayfabe.dev` hostname with Access and the same
runner/box service tokens; keep workers.dev as a fallback only if it also stays behind Access.
The admin UI uses `https://admin.kayfabe.dev` and its separate owner-identity Access app. Before
the first var deploy the coordinator deletes the old `WORKER_URL` secret, following [DEPLOY](DEPLOY.md).

Renting is enabled by the deployed vars file, which is kept outside the repository ([deployment guide](DEPLOY.md)); `RENTING_ENABLED=false` in the committed [Worker configuration](wrangler.toml) is only the safe default. `RUN_TOKEN_SECRET` signs run capabilities and `OWNER_STOP_TOKEN` controls emergency cleanup. As of 2026-10-25 acceptance of the current hosted workflow with the protocol-7 payload is still open; historical local smoke results do not establish it.

- [Threat model](docs/THREAT_MODEL.md): who is trusted, and which behaviours are intended design rather than vulnerabilities.
- [Box protocol](docs/BOX_PROTOCOL.md): bootstrap environment, callbacks and result contract.
- [Placement decision](docs/PLACEMENT.md): US/Canada offer preference and the proposed ledger location.
- [Evidence summary](../../traces/vast_broker_summary/README.md): recorded runs, retained evidence and its limitations.
- [Internal implementation and evidence notes](docs/IMPLEMENTATION_NOTES.md): the former README's dated rounds, corrections and superseded behavior.

After deploying the matching Worker bundle, use a fresh GPU-relevant commit to start validation; reruns retain the original trusted BASE pin.

CODE READ (`src/provider-fault.js`, `src/ledger.js`, `runner.py`): provider failures stop and destroy the box before one replacement on a different known host. Two failed machines condemn their shared host; static seed `224600` comes from runs 40425285335 and 40478005010. An exhausted replacement stays a non-passing infrastructure error, with no kayfabe verdict. See [provider-fault handling](../../docs/howto/vast-provider-faults.md).

CODE READ 2026-10-27 (`src/github.js`): attempt3 and later have no special authorization ceiling. Admission reads the signed attempt's REST resource and retains strict actor, repository, status and SHA bindings. For a 401, use the fixed field-only observability code; see [rerun authorization](../../docs/howto/vast-rerun-auth.md).

CODE READ 2026-10-27 (`gate_reuse.py`): proposed master-push reuse preserves the existing GPU job with readable original result logs and summary. It requires identical trees and complete GitHub status/CI/run evidence; uncertainty uses the existing gate. See [reuse behavior](../../docs/howto/gpu-gate-reuse.md).

## Tiers, the admin start and the release run (how to)

CODE READ (`src/tier.js`, `src/release.js`, `src/index.js`, branch `claude/worker-tier-20261105`,
2026-11-05; not deployed, no rental made). Design: `docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md`
(PR #256, DESIGN-ONLY). Every number here is a configuration default, not a measurement.

**Which tier a run is.** The event decides; no request field can. A PR to master is **middle**: one
box, Windows required, the check profile's caps and its 300-minute limit, and the die comes from a
**family rotation** (Turing, Ampere, Ada, Blackwell; the next family after the last one that ran,
falling through to the next family that has an eligible offer; the run records its family). A master
push is **small**: today's check run (the cheapest of `CHECK_DIES`), until the small tier's own
driver-patch matrix exists. The die sweep stays outside the tiers (`adhoc`). **large** is the
release; a release tag cannot reach the Worker by itself (GitHub admission accepts master workflows
only), so the owner starts it from the admin page. The middle tier's VRAM floor
(`MIDDLE_VRAM_FLOOR_MB`) is off (0) until the footprints of the first trio run exist (TBD).

**What a release rents.** Two measured dies per family from `releaseDies` in `defaults.json`
(TU106, TU116; GA106, GA104, then GA102; AD104, AD102, then AD103, AD106; GB205, GB206, then GB202,
GB203), falling to the next listed die when one has no eligible offer. Every box runs Linux and
Windows. Each die gets two Windows runs: two boxes on different machines (different hosts when
offered), or one box that runs the Windows gate twice when only one machine offers the die. A
PASS needs every family, two dies each, a Linux pass and two Windows passes per die and no FAIL
anywhere. RETRY and UNATTRIBUTED (a failure without a green same-box host reference) are never a
pass: each schedules one fresh-box re-run on another machine (`RELEASE_RERUNS_PER_LEG`, default 1)
inside the release cost cap. A FAIL is never re-rented. An owner stop is not a verdict and is not
re-run. Release cells count only this release's own boxes of the same commit: no result is carried
over from another commit or tree, and one FAIL in a cell fails it whatever passed before or after.

**Limits of the release profile (explicit, per tier).** A release runs only with the kill switch
`RELEASE_ENABLED=true` (default false). Price at most 2 USD/hour per box, hard limit 300 minutes per
box (the check rule: one box runs the Linux bar, the Windows install and up to two Windows gate runs;
installs took 1436 s and 1825 s and gates 906 s to 3454 s on 2026-11-04 at rev `0c515b3b`,
`traces/die_coverage_20261104/`; `RELEASE_HARD_RUNTIME_MINUTES` may lower it but not raise it,
because the reaper deletes `kf-ci-*` boxes after 300 + 15 minutes), at most the account cap minus 1
boxes at once (`MAX_CONCURRENT_RELEASE`, at most 8; one slot always stays for the master gate, so 8
release boxes need `MAX_CONCURRENT_RENTALS=9`; the committed account cap of 1 gives a release no
slot), 40 USD and 24 rentals per day for the release profile, a **release cost cap of 40 USD**
(`RELEASE_COST_CAP_USD`) and an account-wide cap of 50 USD per UTC day over all profiles
(`ACCOUNT_DAILY_USD`). The shared 30-day rolling cap and the credit reserve still apply to every
box. A release whose plan exceeds any of these is refused before the first rental; every later box
(pending legs, re-runs) is admitted only while spent plus reserved plus its own worst case stays
under the cap, and a box whose contract price (as the provider reports it) breaks the cap is closed. Turning
`RENTING_ENABLED` or `RELEASE_ENABLED` off stops a running release at the next cron tick. A release
writes no commit-gap marker, so the master gate of the same commit still rents. A PASS counts only
when the box itself reports the planned die (its GPU name and compute capability) and a known
driver; anything else is a RETRY.

**Plan first (rents nothing).** On the admin page (`kayfabe-vast-admin`, owner Access identity),
enter the commit, choose the tier and press **Plan**: the Worker evaluates the live market with the
release rules and prints the dies, machines, legs, the estimate per family and any refusal
(`INCOMPLETE`, `COST_CAP`, `NO_SLOTS`, `RELEASE_DISABLED`, `DAILY_USD`, `ACCOUNT_DAILY_USD`,
`DAILY_COUNT`, `ROLLING_USD`, `CREDIT_RESERVE`). Offline,
the same planner runs on an offer list:

```sh
node ci/vast-broker/release-plan.mjs --market ci/vast-broker/test/fixtures/release-market-synthetic.json \
  --var MAX_CONCURRENT_RENTALS=9 --var RELEASE_ENABLED=true
```

**Start.** Press **Start** (it asks for confirmation). The commit must be on this repository's
`master` or `next` history (a fork commit is refused) and must declare the Worker's box protocol.
Each box goes through the same admission as a PR-triggered rental: offer rules, caps, concurrency,
own-instance labels (`kf-ci-<batch>-<leg>`), teardown by exact id, the cron's deadlines. The page
lists the batch with its state, verdict, estimate and spend; **Stop** on a row stops one box, the
batch stop (`{"batch": "<id>"}` to `/api/stop`) or **Stop all** ends the release without
re-runs. A small or middle start is one box and, like a PR rental, is not rented for a commit that
already rented within `SHA_GAP_HOURS` (the leg stays pending and shows `BUDGET:sha-gap`).

**Die lists, caveat (review F10).** The fallbacks GB202 (Windows gate 1 FAIL in 3 runs at
`0c515b3b`, `traces/die_coverage_20261104/blackwell/`) and AD103/AD106 (Windows not run there) are in
`releaseDies`, and GA102 and GB203 in `middleRotation` have no Windows PASS at that revision (GA102
passed once through the Worker path: gpu-vast run 41936606326, rev `d74f6063`, 2026-11-05); a
plan that falls back to them can fail on the die, not on the change.

**Stages.** `specviewperf` (release only) is plumbing: enabled by `RELEASE_STAGES=["specviewperf"]`,
off by default, and until the SPECviewperf lane exists its row is `specviewperf_retry_not_built`,
never a pass; scores stay on the box (`scripts/ci/vast_stage_specviewperf.sh`). `two_guests` and
`trio` are reserved names, DESIGN-ONLY, refused if requested, and part of no required check.

**Results.** Every box of a batch is an ordinary ledger rental (`kf-ci-<batch>-<leg>`), so the
results store (PR #258, `src/ingest.js`, not merged here) records it like any other rental; each
record carries `tier`, `family`, `source` (`github` or `admin`) and `batch`. The release verdict
itself lives in the batch (`batch:<id>` in the ledger, shown on the admin page): per (family, die,
platform) cells with their runs, passes and machines.
