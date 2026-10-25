# Vast CI broker threat model

STATUS: LIVE, 2026-10-25. Purpose: so a reviewer (human or AI agent) does not report a design decision
as a vulnerability. Labels: OWNER = stated by the owner on 2026-10-25; CODE READ = checked at
`origin/master` 93091106d (`path:line`); MEASURED = observed live, with its date.

## 1. Who is trusted

OWNER: the security boundary is (a) the Worker pins `ref=master` and the master workflow identity
through GitHub OIDC (immutable subject); (b) the repository owner must trigger or approve the run for
a SHA; (c) a per-run scoped token (HS256, bound to run, nonce and SHA) plus spend limits in the
ledger; (d) the Vast account key never leaves the Worker.

The box and its operator are untrusted. A box holds no durable secret except the static Cloudflare
Access pair (B) and the broker URL, which contains the secret path prefix.

CODE READ: OIDC subject, `ref == refs/heads/master`, repository ids and workflow:
`src/index.js:44-56`, `src/policy.js:92-98` (the subject is an `environment:` subject; the
workflow files declare `environment: gpu-vast`). Owner trigger: `triggering_actor.id == owner` and
fork heads refused, `src/github.js:84-100`. Run token claims run, attempt, sha, nonce, `exp`:
`src/tokens.js:38-62`. Budgets: `src/ledger.js:146`. No Vast key in any box variable: DEPLOY.md
and `src/index.js:876-886`. NOT VERIFIABLE FROM CODE: whether the `gpu-vast` environment has a
required reviewer in GitHub settings; the code enforces the actor check only.

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
