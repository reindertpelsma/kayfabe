# Read the Site commit status

STATUS: PROPOSED, 2026-10-27. CODE READ: `.github/workflows/site.yml`, `site-status.yml` and `ci/vast-broker/site_policy.py`.

Every PR starts the Site workflow, without a path filter. Its first job reads the changed-file list through GitHub's API. Changes under `site/`, `docs/` or `.github/workflows/site.yml`, including removals and renames out of those paths, require the existing build, render and link checks. A complete unrelated-file decision reports `site` success with `not needed: site and docs untouched`. Uncertain metadata runs the build instead of declaring it unnecessary.

The PR build uses its exact head SHA with a read-only token and no deployment permission. Master pushes check out their exact pushed SHA. Tag, release, schedule and dispatch publication keeps reviewed master tooling. On master pushes, Pages deployment needs positively identified Site/docs/workflow changes; uncertain push metadata blocks deployment and makes the workflow fail. Existing tag/manual/scheduled publication retains its guarded master/version-tag boundary.

Fork PR tokens cannot write commit statuses. A separate trusted `Site status` workflow handles API-only start/completion notifications; it never checks out PR code, consumes artifacts or receives Pages/OIDC permissions. Its status-writing token stays separate from the read-only build. It verifies the source Site workflow, repository, SHA, actual build result and, for skipped master builds, the original typed push-range annotation. Delayed callbacks preserve a final same-SHA result rather than resetting it to pending.

On a master push, the status reporter can immediately copy a successful GitHub Actions `site` status from an identical parent/associated-PR tree with intact successful CI/run evidence. The copied status names the source SHA and links to the original run. Building a Pages artifact still runs when publication is needed; no PR artifact is reused for deployment. A later build/deployment failure replaces the copied status with a non-passing result. Missing proof takes the ordinary path.

The owner adds `site` as a required master check after reviewing and merging the workflows. This change does not modify repository protection or Pages settings. The status is a test result, not release provenance. API outages or incomplete evidence may require rerunning the Site/status workflow; no success is inferred from missing evidence.

CODE READ 2026-10-28 (`ci/vast-broker/site_policy.py`): PR file pagination rechecks the head, base SHA and changed-file count after the last page. A concurrent synchronize or base update leaves a conservative build decision and cannot report the older head as not needed.

CODE READ 2026-10-28 (`ci/vast-broker/site_policy.py`): Site reuse binds every CI record through its job and check suite to the intended ci.yml workflow on the same SHA. All legitimate push/PR duplicates must pass; ambiguity takes the ordinary build path.

## Optional publication instrumentation

CODE READ 2026-10-28 (`.github/workflows/site.yml`, `site/build.py`, `site/deployment_slots.py`): ordinary local, PR and fork builds contain no publication slot or beacon. Only an upstream publication candidate gets one neutral head comment. The build checks its position with a real DOM parser. The protected deploy job validates its runtime input and replaces only that comment's bytes; it never parses or reserializes pages. Empty, absent or invalid input removes the comment and publishes without instrumentation. Valid input with no matching slots fails. Conditional footer text is supplied by the deployment workflow, so it is absent from ordinary built files.
