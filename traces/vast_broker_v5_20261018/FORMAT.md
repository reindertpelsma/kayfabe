# Worker formatting checkpoint, 2026-10-22

[MEASURED: 2026-10-22 format baseline run] Prettier 3.6.2, Black 25.1.0;
width 100, two-space JavaScript, semicolons, single quotes and trailing commas.
The portable suite passes 972/972 before and after, zero failures/skips. Six
checkpoint tests were registered before this comparison; pending blob tests are
parked separately and are excluded from both sides of this formatting comparison.

[CODE READ: 2026-10-22 mutation-anchors.json] All 233 original mutations are
preserved as explicit before/after formatted replacements: 94 legacy, 28 V4,
38 runner, 73 V5. Each replacement was obtained by formatting the original
baseline and original mutant separately, then diffing them. No mutant or killer
threshold was removed. Exact unique-anchor checks remain mandatory. Full
campaign execution is in progress in a separate frozen source copy.

[CODE READ: 2026-10-22 src imports, bundle.js, .github/workflows/ci.yml]
Prettier-ignore applies only to JavaScript module imports, preserving the
linker's single-line import contract. Existing bundle/exports/syntax tests pass.
The bundle embeds readable multiline defaults; source modules are formatted.
CI checks both pinned formatters without adding a repository dependency.

[MEASURED: 2026-10-22 format bundle run] Bundle 103026 bytes. SHA256 changed
from c139d8ee64d88de9032b602bff0a5e776c223aa23bb06a0d3c1748491a15e69f
to 1a06b7c87dd069240c4404503353b458d32e4d3ae64a680f35a5ac8f33d83eb8.
Manifest and wrangler binding match. No deployment or renting enablement occurred.
