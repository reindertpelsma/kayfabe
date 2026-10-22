STATUS: PARTIAL, 2026-10-23. Renting stays disabled; no V5 rental or deployment.

[MEASURED: 2026-10-22/23 security-transition run, security_mutants.json] The
implemented transport/lifecycle suite passed 996/996. Automatic discovery then
included the 11 pending bootstrap/auth contract probes: 1007 total, 1001 passed,
6 failed, zero skipped. The six failures are mandatory-prefix (rent and
availability), two tiny-bootstrap golden templates, and two create-env checks.
These remain deliberately visible; no test is disabled to make the gate green.

[MEASURED: 2026-10-22/23 security-transition run, security_mutants.json] All 14
focused security/recovery mutants have at least two named killers. They cover
pre-queue auth, frontend signature/claims, priority cleanup, authenticated
admission accounting, JWKS cooldown/single-flight, definitive/id-less create
settlement, SHA retry release, result-only bounded hold/cancellation, and hashed
deployment configuration. The mutation scripts now refuse attribution on an
unmodified failing baseline. Pending bootstrap tests prevent a full new-source
campaign; the 233-case formatted-source campaign is recorded separately.

[CODE READ: 2026-10-23 src/index.js, src/oidc.js, src/vast.js] Authentication
precedes the state queue; cron/owner stop/kill have priority. Only concurrent
rents use the busy flag. OIDC fails at the frontend before Durable Object entry;
box record MACs are checked before joining its state queue. Requests revalidate
live authority after queueing. JWKS refresh is single-flight with a 60-second
cooldown and no signature-failure refresh. Provider explicit refusals settle
zero liability and release their SHA cache; ambiguous no-contract records need
spaced observations and boot expiry. Holds require a result, are capped at
30 minutes/hard limit, and retain cancellation checks until owner confirmation.

[CODE READ: 2026-10-23 owner security checklist] Remaining: mandatory path prefix;
cheap per-IP/global status/log/stop controls; iat/age, numeric repository and
subject/workflow claims, dead pull_request removal, jose test oracle; exact-host
redaction, nonce-bound MAC/random comparison key, sweep branch reachability,
GitHub read-rate mitigation/header decision. Non-object JWT shapes are guarded,
but their dedicated cross-check corpus is pending. The job_workflow_ref policy
needs the owner ruling requested asynchronously: GitHub documents that claim
only for reusable workflows, whereas this workflow is a normal one.

[CODE READ: 2026-10-23 owner continuation order] Tiny generic onstart/create env,
repo-side scripts and clear absent-payload failure, blob URL route, format pass 2
and line check, real A/B/C/D non-VM runs, then README/docs restructure remain
unfinished. Production KVM/full merge-bar payload and SSH polish are deferred.
Hosted OIDC and Cloudflare billed CPU remain unverified. No paid resource was
created, no reaper/tunnel was needed, and no rent-enable/deploy action occurred.

[MEASURED: 2026-10-23 security bundle run] dist/worker.js 110672 bytes, SHA256
2ab3856a7430129334fa430070419f83d1e2f920d55a3190aff7718f04aaf897.
Manifest/wrangler match; main points at dist/worker.js with no_bundle=true.
README pins wrangler 4.152.0. This bundle is reviewable work, not renting-ready.

[CODE READ: 2026-10-23 host_ip_scrub.json] Two historical public host-IP fields
were scrubbed from the current tree; the file names and original commits are
recorded in that JSON. No history rewrite occurred.
