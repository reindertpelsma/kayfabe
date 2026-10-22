# Worker code checkpoint, 2026-10-22

[MEASURED: 2026-10-22 checkpoint run, checkpoint_mutants.json] The baseline passes
966 tests with no failures or skips. The 73 V5 mutations each have at least two
named killers, including four new repo-defined-suite/exact-fetch mutations. The
new regression run had eight deliberate failures before the fix. The earlier
28 V4 and 38 runner campaigns are recorded alongside this file; the 94 legacy
anchors were validated, but their entire historical campaign has not been rerun.

[MEASURED: 2026-10-22 checkpoint bundle run] dist/worker.js is 83808 bytes;
SHA256 c139d8ee64d88de9032b602bff0a5e776c223aa23bb06a0d3c1748491a15e69f.
Protocol 4, Worker v5. deployment.json and wrangler.toml assert that same hash.
The bundle is generated locally; the repository records its source and manifest.

[CODE READ: 2026-10-22 src/onstart.js, src/results.js, runner.py] Onstart fetches
only the exact approved SHA and invokes scripts/ci/vast_payload.sh with SHA and
nonce. Repo-side code defines suite requirements. Transport independently checks
a nonempty successful bounded table, exact bindings and hardware schema. No
provisioning/merge-bar commands or suite counts are embedded in the Worker.

[CODE READ: 2026-10-22 owner V5_DUMMY instruction] The next live acceptance is
short non-VM dummy FAIL/PASS with private multi-blob downloads, capped at $1.
The blob route is the next code addition. KVM/full payload and hold-hook polish
are deferred. Hosted OIDC and Cloudflare billed CPU remain unverified; GitHub
public read throttling can refuse runs. No deployment or renting enablement was
performed. Price $1/hour, daily $6, rolling $30 and reserve $8 are owner-approved.
