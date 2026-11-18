**STATUS: LIVE as GPU-free code, DESIGN-ONLY for the Worker, 2026-11-18.** The planner functions, the data file and the offline
command are implemented and tested (`ci/vast-broker/test/app-lane.test.js`, synthetic offers only). The Worker admin Start kind
`app` is **NOT built**: no route, no rental and no deployed code reads any of this, and nothing in `ci/vast-broker/src/` imports
the planner. No box has run an app-lane plan; no cost below is a measurement.

# The app lane planner

Ruling: `docs/OWNER_RULINGS.md` section AZ (images and SPEC leave the required release gate and run as an invoked app lane;
the lane is required only when a release touches a core path; the first release runs every image on every family; rotation
afterwards). This document says what the planner computes. It does not change the ruling.

## What is implemented (code read, GPU-free)

- **Data, one place:** `ci/vast-broker/app-lane.json` (not read by the deployed bundle): `core_paths`, the rotation window,
  the image role's hard limit and bandwidth price cap, and the list of images with their needs (`disk_gb`, `vram_gb`, `stages`
  as names of `bandwidth.json`, `manual_only`). The planner never names an image; an image is added by editing this file and
  `propimages.json` (the pinned size that `bandwidth.json` reads). Images C and D are not configured yet because they are not
  in `propimages.json`.
- **`appLaneRequired(changedFiles, corePaths)`** (`src/app-lane.js`): pure. A file is core when a pattern without `!` matches it
  and no pattern with `!` does. `*` stays inside one path segment, `**` crosses segments, there is no other glob syntax.
  Returns `required`, the `matches` and the `excluded` (files that matched a core pattern but are excepted). `core_paths` is
  exactly the section AZ item 5 list: the eight core patterns, then the exceptions `crates/kf-harness/**`,
  `crates/kf-rawclient/**`, `crates/kf-trace/**` and the test-only `crates/*/tests/**` (applied everywhere, so also
  `crates/kf-chip/tests/**`). Feed it `git diff --name-only --no-renames <master>...<head>` so that a file moved out of a core
  path still shows under its old name.
- **The app leg** (`planAppLane`): one leg is one image on one family on one box. Offers are evaluated by the Worker's own
  `OfferBook` under the release profile (quality, country, bad machine, price cap), then filtered by the image's VRAM, disk and
  bandwidth price cap, and the cheapest by final total is taken: any die of the family, **not** pinned to a matrix die. The
  price is `appLegTotal`: compute and storage (the image's disk) over the role's hard limit plus the image's bandwidth at the
  offer's per-GB prices. The bandwidth is `bandwidth.json`'s own estimate of the image's stages with its margin, not the release
  profile's flat 130 GB (`policy.js` `offerPrice` is the same formula; a test pins them equal for the release numbers). Two
  legs never share a machine; the scarcest leg chooses first. A manual-only image's leg carries `only` (the stages to pass to the
  payload's `--only`).
- **First release** (`firstRelease: true`): every configured image on every family of the release matrix; the rotation table is
  ignored.
- **Rotation** (`selectRotation`, `recordRotation`): input is a table `{release_seq, last: {image: {family: seq}}}`. A pair
  whose last run is four or more releases old is forced in; every image then runs its oldest pairs until it has
  `ceil(families / 4)` in this release, never-run pairs oldest. A test simulates several shapes over 40 releases: every image
  runs in every release and no pair waits more than four releases. This is a property of the selection when every release is
  recorded; a release that runs less than planned (an unplaced leg) must not be recorded as run (`rotation_after` records only
  placed legs). The escalation of item 6 (an architecture-specific path runs every image on the affected family) is NOT
  implemented.
- **Estimate and daily cap:** the plan sums the legs' worst-case totals in USD. `dailyCapUsd` is a required input; an estimate
  plus `spentTodayUsd` over it refuses the plan (`DAILY_CAP`, `admitted: false`; the command exits 2). A leg with no qualifying
  offer is `INCOMPLETE`. The cap is the caller's number; the planner does not read the ledger.

## Commands (offline, rents nothing)

    node ci/vast-broker/release-plan.mjs --changed-files list.txt
    node ci/vast-broker/release-plan.mjs --app-lane --market offers.json --daily-cap 40 --first-release
    node ci/vast-broker/release-plan.mjs --app-lane --market offers.json --daily-cap 40 --rotation table.json

## Not implemented, not measured

- The Worker start kind `app`, a payload that runs one image stage alone, the schedule between releases, and the record of
  the last app-lane run per image in the release record (section AZ item 4).
- `vram_gb` of every image is an UNMEASURED placeholder; `disk_gb` and `hard_minutes` are ESTIMATES (the large profile's
  numbers). Until an app-lane run measures them, the estimate is a planning figure, not a bill.
- The greedy leg assignment (scarcest first) is not proven cost-optimal across images of one family.
