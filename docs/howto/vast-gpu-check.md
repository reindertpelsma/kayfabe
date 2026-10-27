# Reading the GPU check

CODE READ (2026-10-27, `.github/workflows/gpu-vast.yml`, `trigger-policy.mjs`): one action-free policy job reads trusted repository metadata. Complete changes confined to excluded documentation paths can report `NOT_NEEDED`, without renting a box. Code, broker paths, odd filenames, incomplete file lists and uncertain metadata request the GPU check.

CODE READ (2026-10-27, `.github/workflows/gpu-vast.yml`): `NOT_NEEDED` requires a successful policy job, explicit `run=0`, a successful pending-status job and a skipped GPU job. A failed job or missing output stays non-passing. The Worker cannot supply a skip; a GPU success requires a verified PASS in GRACE/NONE.

CODE READ (2026-10-27, `.github/workflows/gpu-vast.yml`, `workflow-guard.js`): the policy uses the runner's preinstalled Node with JSON import attributes (Node 20.10.0+; 18.20.0 backport). This workflow simplification needs no Worker deployment. For a PR, `pull_request_target` still executes the BASE workflow; the new single-job layout takes effect after the workflow change is merged.
