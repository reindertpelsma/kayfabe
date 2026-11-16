# The Worker's proprietary-image configuration: one instruction for the Worker run and the standalone run

STATUS: DESIGN-ONLY, 2026-11-16 (built and GPU-free tested; nothing deployed, nothing wired into a rental, no box ran it). Labels: MEASURED (a test or tool run in this session), CODE READ (read in the tree), INFERRED, NOT RUN. Owner decisions of 2026-11-16: the Worker knows image A and image B from configuration, with the applications on them, so it can build the correct instruction; the standalone run accepts any number of images on the command line; the Worker and CI run is STRICT (everything in its configuration must run, zero skips); a gate lane and a discovery lane. Runner and instruction schema: `docs/design/V3_PROPIMAGE_RUNNER.md` (#444, stacked on #434). Files: `ci/vast-broker/propimages.json`, `ci/vast-broker/src/propimages.js`, `ci/vast-broker/test/propimages.test.js`, `ci/vast-broker/propimages-mutants.mjs`.

## What exists

- **`propimages.json`** (schema `kayfabe.worker-propimages/1`): `strict: true`; `objects` (id, role `image` or `gop`, the pin ref the box looks up in `scripts/ci/propimage_apps*_pins.json`, the private object name, size, sha256); `images` (label `imga`, `imgb`; the object; the die `families` it has run on and why; `apps`: id, `lane`, and the reason for the lane; `excluded`: applications that are deliberately not listed, with the reason).
- **`src/propimages.js`**, pure:
  - `propimagesConfigProblems(config, {registry})`: every problem of a configuration as a named token (`NOT_STRICT`, `UNPINNED:<id>`, `SIZE:<id>`, `DUPLICATE_LABEL:<l>`, `UNKNOWN_OBJECT:<l>`, `LANE:<l>:<app>`, `UNKNOWN_APP:<l>:<app>` when the runner's registry is given, `EXCLUDED:<l>`, `FAMILIES:<l>`, `GOP`, ...).
  - `propimagesForLeg({tierName, die, images?})`: the `propimages` and `propimages_strict` part for one leg. Image objects are `{label, kind: "prebuilt", disk: {from: "pin", ref, sha256}, apps: {id: {mode: "present", lane}}}`, the object the runner's `propimage_runspec.validate_list` reads. `propimages_strict` is always `true`; there is no option to say otherwise. `images` selects by label; an unknown label, a duplicate, an empty list, a tier other than `large` and an unknown die are each refused by name.
  - `buildInstruction(profile, options, ...)`: the whole instruction. The base fields are derived from `tierEnvironment(profile, options)` (what the Worker sends today), the part is added ONLY for `propimages: true` with the leg's `die` (default off). A driver-matrix leg is refused (`instruction release matrix`): the instruction has no driver fields and the payload refuses such a leg while `PAYLOAD_RELEASE_MATRIX_EXECUTOR` is 0.
  - `propimageLabelsFor({tierName, die})`: which images a leg takes, `[]` for a leg that takes none, so a planner can ask before it builds.
  - `canonicalInstruction(doc)`: the bytes `payload_instruction.py` writes (`json.dumps(indent=2, sort_keys=True)` plus a newline).
- `tierEnvironment` is not changed (a test pins today's rehearsal variables), `src/index.js` does not import the module (a test pins that), and no environment variable carries the part: the part has no `KF_*` spelling (the box's environment adapter cannot express it), so the Worker sends it as the instruction file (`--instruction`). The route that hands the file to a box is not built (below).

## What the parity test covers (run 2026-11-16 in this tree: `node --test test/propimages.test.js`, 64 tests pass)

For the rehearsal leg (the runnable release scope) on an Ada die (images A and B) and on an Ampere die (image A only):

1. The instruction the Worker builds equals a committed golden file byte for byte (`test/fixtures/propimages/`).
2. The box's own parser (`payload_instruction.parse`) reads the golden and writes the same bytes back; `vast_payload.py --instruction G --emit-instruction H` writes `H` equal to `G` (Ampere golden); `--dry-run` prints `KF_PAYLOAD_PROPIMAGES strict=1 images=imga` and plans `propimage_apps`.
3. The standalone command line built from the same images (`--image prebuilt:pin:REF#sha256=SHA --label L --app ID=present,lane=LANE ... --strict`) is parsed by `propimage_runspec.split_flags` into the same normalized spec as `validate_list` of the instruction's images; `propimage_apps.py plan` prints the same plan for the flags and for `--images-file`. Without `--strict` the same flags are the permissive run of the same images.
4. For eight base legs (rehearsal with and without a SPEC lane, large, release middle, check middle and small, sweep) the instruction `vast_payload.py --emit-instruction` builds from the environment `tierEnvironment` sends equals the Worker's instruction byte for byte.
5. The Ada golden (images A and B) is refused today by the box's dry run with `STRICT_UNKNOWN_APP:imgb:<first app>` because image B's application definitions are another change (#446); the test asserts exactly that refusal, and asserts the accepting form once every image B application has a definition.

Not covered by it: that a box can fetch the objects (below), and that any listed application passes.

## Lanes (`gate` must pass, `discovery` may fail and is a finding)

A row is in the gate lane when it passes today, otherwise in the discovery lane; where I was unsure the row is in discovery. The table is data: moving a row is a one-line change to `propimages.json` and the test that pins the table.

- Image A, gate: `solidworks`, `edrawings`, `edge` (PASS on the pc and on Vast boxes). Discovery: `3dmark` (failed twice on a Vast box), `superposition` (no Vast-box result), `resolve` (crashed 3 of 3 times on an RTX 4090 box; its definition is being replaced by #445). Excluded: `wukong` (no verdict driver; a strict list that names it fails the leg).
- Image B, gate: `gpudiag`, `illustrator`, `acrobat` (PASS on one Vast RTX 4090 box) and `premiere`, `lightroomlaunch`, `dimension`, `uxp` (PASS_LIMITED: launch-only rows whose best verdict is that by design; the runner's treatment of PASS_LIMITED in the gate lane is CODE-UNREAD here and must be checked on #446). Discovery: `photoshop`, `photoshopsample`, `photoshopprojects`, `aftereffects`, `aeparallel`, `aeprojects`, `aetoggle`, `mediaencoder`, plus `indesign`, `indesignsample`, `illustratorsample` (tagged gate by the harness but failing today). Excluded: `animate`, `animatesample`, `animateprojects`, `lightroom` (disabled in the harness), `indesignprojects` (no final driver result), `pugetbench` (not pinned, not redistributable).
- The row sources are the app definitions on #434, #444, #445 and #446 and the run reports of 2026-11-15 and 2026-11-16.

## The die constraint

Both images are listed for the families they have been measured on and nothing else: image A for Ampere and Ada, image B for Ada (one RTX 4090 box). A leg of another family takes no image from this file until a measured pass is recorded; a leg that asks for one on such a family is refused by name (`propimages family:<f>`), never silently emptied. Only large legs can carry images (the instruction schema refuses `propimages` on any other tier), so the middle leg of a family (its second die) takes none; splitting images across the two dies of a family is not expressible without a schema change.

## Delivery and the `/bench-data` allow-list (decision: left unchanged)

The box learns the images as pin refs (`propimage-...zip`) and looks the object up in the pins files, which hold the size and sha256; the instruction repeats the sha256 so the object is checked twice (a mismatch is `PIN_SHA256_MISMATCH`). The instruction carries no object name, size or reason text (a test pins that). How the bytes reach the box is not decided by this change: the standalone run uses a presigned URL file the operator mints (`kayfabe.download-urls/1`) or pushed files.

The Worker route `GET /bench-data/<run>/<name>` answers with a presigned URL for an allow-listed name. It is gated by a live run token and the run's deadlines, but NOT by profile or stage (`DEPLOY.md`, `/bench-data` scope note: any live box can ask for the SPECviewperf copy). A check-profile box runs the tree of a pull request. Adding the images to the allow-list would therefore let that tree request them, which is a larger exposure than the one the owner approved for the benchmark copy. The allow-list in `src/benchdata.js` is NOT changed, and two tests fail if an image name or object key is added to it. The condition to add them is a route that also requires the large release profile and a leg whose instruction lists the image (one run record field); that is the real `/admin/resolve` work below.

## Not done

- No deployment, no Cloudflare change, no secret, no live Worker contact.
- The route that resolves a requirement into this instruction and hands the file to a box (an `/admin/resolve`-style route; it is named in the owner's brief of 2026-11-16 and is not designed or built anywhere in the tree) does not exist; `src/index.js` still sends only `KF_*` variables, so a Worker-rented leg cannot ask for images today. The module is called by nothing but its tests.
- Image B's pin file and application definitions are #446, image A's resolve row is #445; this branch stacks on #444 and takes neither. Until #446 lands the Ada golden is refused by the box's strict check (above).
- No planner decides which large legs ask (`propimages: true`); no result row for these apps reaches the release verdict.
- Presigned-URL minting for a leg and the allow-list decision above.
