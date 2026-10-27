# Vast CI broker threat model

STATUS: LIVE, 2026-10-27. Purpose: so a reviewer (human or AI agent) does not report a design decision
as a vulnerability. Labels: OWNER = stated by the owner on 2026-10-25; CODE READ = checked at
`origin/master` 93091106d (`path:line`); MEASURED = observed live, with its date.

## 1. Who is trusted

OWNER: the security boundary is (a) the Worker pins `ref=master` and the master workflow identity
through GitHub OIDC (immutable subject); (b) the repository owner must trigger or approve the run for
a SHA; (c) a per-run scoped token (HS256, bound to run, nonce and SHA) plus spend limits in the
ledger; (d) the Vast account key never leaves the Worker.

The box and its operator are untrusted. A box holds no durable secret except the static Cloudflare
Access pair (B) and the broker URL, which contains the secret path prefix.

CODE READ 2026-10-27 (`src/github.js`): admission reads `actions/runs/<id>/attempts/<attempt>` using the signed positive numeric identifiers. Every returned id, attempt, event, in-progress status, repository, head repository, head SHA, branch and owner triggering actor is still checked. The latest-run endpoint remains for lifecycle cleanup, not admission. There is no attempt>=3 policy ceiling. Refusal telemetry records only fixed field names, never values. Historical `github:run` logs do not identify the failing field; see [AUTH401 evidence](../../../traces/vast_auth401_20261018/README.md).

CODE READ: OIDC subject, `ref == refs/heads/master`, repository ids and workflow:
`src/index.js:44-56`, `src/policy.js:92-98` (the subject is an `environment:` subject; the
workflow files declare `environment: gpu-vast`). Owner trigger: `triggering_actor.id == owner` and
fork heads refused, `src/github.js:84-100`. Run token claims run, attempt, sha, nonce, `exp`:
`src/tokens.js:38-62`. Budgets: `src/ledger.js:146`. No Vast key in any box variable: DEPLOY.md
and `src/index.js:876-886`. NOT VERIFIABLE FROM CODE: whether the `gpu-vast` environment has a
required reviewer in GitHub settings; the code enforces the actor check only.

## Why a Worker (and when not)

OWNER, 2026-10-25: the Worker is test infrastructure (`docs/OWNER_RULINGS.md` §AN), used because:

- the Vast key lives in the Worker, outside GitHub;
- the caps are stateful (a ledger), which a workflow cannot keep;
- the teardown cron runs independent of Actions;
- GitHub OIDC means no spend-capable token is stored in GitHub;
- waiting minutes on a public repository are free and unlimited;
- it costs nothing and needs no maintenance when dormant, whereas GitHub disables scheduled workflows after 60
  days of inactivity and PATs expire.

The alternative is a private repository. It needs a dispatch token or polling, gives 2000 free private minutes per
month, and still needs a log relay and a teardown cron.

Split: the Worker serves CI and releases; `scripts/ci/vast_simple` (SSH pull, no Worker, draft PR #137) serves
manual private sweeps.

## 2. Cloudflare Access is a noise gate

OWNER: Access (service-token pair A for the runner, B for boxes, hostname-wide) is an anti-bot and
noise gate in front of the Worker, not the security boundary. A leaked B pair alone cannot rent or
read anything. The Access policy for B should list that specific token.

A malicious approved SHA controls the box code, so it can read the pair, the run token and the URL.
Accepted: the owner approves the SHA, the token is run-scoped and expires with the run, and spending
is capped. A verdict from a box is untrusted data and never contaminates the tested code (see the
[trust trace](../../../docs/design/V3_CI_VAST_RUNNER_TRACE_20261022.md)).

CODE READ: the Worker never reads Access headers, it only injects the pair as `KF_ACCESS_ID` /
`KF_ACCESS_SECRET`: `src/index.js:92-93`, `src/index.js:882-883`. The payload loses the pair, but it
runs as root from the same SHA and can read `/run/kf-ci/env` (`scripts/ci/README.md`).

What a malicious box can and cannot do. OWNER: (1) the box runs the code of the owner-approved SHA. For a
PR that code is untrusted until reviewed and merged, and the owner's approval of the SHA is the gate
(the Worker checks `triggering_actor == owner`, `src/github.js:100`; a required reviewer on the
`gpu-vast` environment would be an additional control, not verifiable from code). (2) The box holds
only low-value material: the run-scoped token, the static Access pair and the broker URL, never the
Vast key. Its verdict and logs are untrusted data and cannot change the PR's code, the built binaries
or the repository. (3) The realistic risks are abuse of the rented machine (mining, spend, proxying),
bounded by the spend caps, the hard deadline, the bad-machine controls and the approval gate, not the
integrity of our artifacts. (4) Hostile PR CI on GitHub runners is a separate, GitHub-level risk
covered by GitHub's fork-PR rules (no secrets, approval for first-time contributors) and is out of
scope for the Worker. Passing tests is not a security signal. CODE READ: `gpu-vast.yml` has no `needs:`
on the GPU-free CI (a separate workflow), so "GPU-free tests run first" is a process fact the
workflow does not enforce.

## 3. `bad-machine:` entries never expire

CODE READ (2026-10-27, `src/ledger.js`, `src/offers.js`, `src/provider-fault.js`): explicit provider results and the bounded pre-product failure summary also learn machine entries. Two distinct failed machines with the same known host create a transactional `bad-host:` entry. Both indexes persist and refuse admissions above 256; writes retain at most the 257-entry refusal sentinel. A retry requires another known host and never continues the failed box. The static host seed `224600` cites runs 40425285335 and 40478005010. Historical source references below describe the original machine-only policy.

OWNER: by design, no expiry. CODE READ: the Worker writes `bad-machine:<machine_id>` on the KVM
boot-failure signature (`src/index.js:296-313`). `evaluate()` lists at most 257 and refuses when there are more than 256 (`src/offers.js:132-133`), a bare `UNAVAILABLE` 503 with no reason code
(`src/policy.js:13-15`). Pruning does not touch them (`expire()`, `src/ledger.js:119-143`).

OWNER: reaching the cap is intended to stop everything. It means something is badly wrong, probably
a bug in our scripts, and new runs are rejected until the owner looks.

Recovery: no route, endpoint or script lists or clears them (`DEPLOY.md` says no owner endpoint
exists or is wanted; the owner-edited `BAD_MACHINE_IDS` vars are a separate static list). INFERRED:
removal means deleting the `bad-machine:` keys from the `LEDGER` Durable Object storage out of band.

## 4. The reaper fails closed

OWNER: the reaper is a separate Worker with its own list-and-delete key. It aborts with
`reaper_unavailable` and a static reason when the instance list is incomplete or malformed, or has
100 or more instances (`ci/vast-reaper/worker.js:69-90`, `:128`). Both are unreachable in
normal use, and deleting is impossible without a trustworthy list, so failing closed is intended.
The reason is readable in Workers observability.

MEASURED 2026-10-25, three Workers-runtime findings: `redirect: 'error'` is rejected by the runtime
(use `'manual'` and check `response.redirected`); observability must be declared in `wrangler.toml`
because a deploy replaces dashboard settings; the injected `fetch` must not be called as a method
(`Illegal invocation` in workerd, invisible in Node tests). CODE READ: the third is fixed on master
in `src/vast.js` (PR #133). The first two are NOT yet on master: `ci/vast-reaper/worker.js:28` still
says `redirect: 'error'` and `ci/vast-reaper/wrangler.toml` has no `[observability]`. The fix is on
branch `claude/reaper-redirect-20261025` (unmerged at 93091106d).

## 5. Workflow allowlist

CODE READ: `CHECK_JOB_WORKFLOW_REFS` and `SWEEP_JOB_WORKFLOW_REF` are parsed and validated
(`src/policy.js:66-90`) and set in `wrangler.toml:36-37`, and carried in `cfg` (`src/policy.js:119-120`), but
`allowedWorkflowRef` never reads them: it hardcodes `gpu-vast.yml` and `gpu-vast-sweep.yml` at
`src/workflow-authority.js:11-12`, and returns false for anything else. The code is authoritative,
the vars are decorative. Renaming a workflow therefore needs a code change. It fails closed.

## 6. Country denylist

OWNER: it uses the host-supplied `offer.geolocation`, which a malicious host can spoof
(`src/offers.js:96`). It is a preference and cost control, not a security control.

## 7. Vast behaviour learned live (MEASURED 2026-10-25)

- A `vm: true` instance gets the create `env` in `/etc/environment`, not in the onstart process
  environment. `load_provider_env` reads it (`scripts/ci/vast_report.py:657`).
- A provider can report `running` while the VM never started (a QEMU port-forward error visible only
  in `vastai logs <id>`).
- `vastai show instances` is deprecated; use `instances-v1`. Older bench scripts still call the old
  form (`scripts/bench/autodestroy_box.sh`, `scripts/bench/box/vast_reaper.sh`).
- A repeated SHA is refused for 24 h (`src/index.js:748-753`, `defaults.json:7`), also on an owner
  re-run, so a re-run of the same SHA does not rent.
- Concurrency slots are held by open ledger records until they close, which for a box that never
  reports is its boot deadline (`src/index.js:769-773`, 40 minutes in `defaults.json:8`).

## 8. Pitfalls (live debugging, MEASURED 2026-10-25)

- Workers reject `fetch` with `redirect: 'error'` (observed 2026-10-25).
- An injected `fetch` must not be called as a method of another object (`Illegal invocation`, observed 2026-10-25).
- Cloudflare error 1010 blocks Python urllib's default User-Agent (observed 2026-10-25); the box reporter sends
  `kayfabe-vast-box/7` (PR #135).
- `vm: true` Vast instances receive env in `/etc/environment` (observed 2026-10-25, see section 7).
- A provider can report `running` while the VM never starts: read `vastai logs <id>`, the console alone does not
  show it (observed 2026-10-25).
- `vastai show instances` is deprecated; use `instances-v1` (observed 2026-10-25).
- A repeated SHA is refused for 24 h, and a failed attempt still reserves a `DAILY_RENTALS` slot (observed
  2026-10-25).
- `deploy.mjs` only allows listed public vars; `BOOT_LIMIT_MINUTES` is not one (observed 2026-10-25).
