# V3: propimages are configuration (bundles, the resolver, the generic stage)

**STATUS: LIVE for step 1 (schema, validator, resolver, the project's own entries as data) and for the planning half of step 2 (`propimage_stage.py plan`, with the `arm` parameter), 2026-11-19. DESIGN-ONLY for the running half of step 2 (the generic `run` replacing the three per-image stages) and for steps 3 and 4 (the payload, the Worker and the planner reading the entries).** Rulings: `docs/OWNER_RULINGS.md` §BA item 1 (propimages are configuration, no image names in code), the owner's 2026-11-19 refinement (the division of applications into images is arbitrary, only Windows versus Linux matters, scripts may drive VFIO as the reference arm). Labels: MEASURED = a command run on 2026-11-19 (named); CODE READ = read in a named file, not run.

## 1. What a bundle is

A **bundle** is one stored image (one object, one Windows or Linux disk) and the applications installed on it. The repository has no concept of "image A": the code reads a bundle table, and a request for applications is turned into the bundles to fetch by a resolver.

```
{"schema": "kayfabe.propimage-bundles/1",
 "bundles": {ID: {"os": "windows|linux", "r2": "object/key.zip", "size_bytes": N, "apps": [...],
                  "sha256"?, "ref"?, "licence_note"?, "basis"?, "needs_gpu_families"?, "run"?}}}
```

The full field list, with what each field is for, is the module docstring of `scripts/ci/propimage_config.py`. Three groups:

- **Identity and cost** (`os`, `r2`, `size_bytes`, `apps`, optional `sha256`, `ref`): what the planner needs to compute which bundles an application list needs and what a box downloads. `sha256` is an identity pin, enforced only by a strict run (§AW); a bundle without one is allowed (the project's two newest bundles have none recorded).
- **Notes** (`licence_note`, `basis`): free text that the validator checks for the shape of a secret (an `@`, a token of 24 or more letters and digits). They state what is unverified. They never hold a key, an account, an e-mail address, an institution or a credential.
- **`run`** (optional; absent means a fetch-only bundle): the per-image facts a stage reads today. Derived from the three stages (CODE READ, `scripts/ci/propimage_apps.py`, `propimage_apps_b_stage.py`, `matlab_stage.py`): the module whose `CHECKS` table holds the rows (`rows_file`), the pins file with the boot GOP, the swtpm sources and the data volume (`pins_file`), the rows in order (`rows`), the rows that always boot alone (`solo_rows`), whether the open-asset data volume is passed (`data_iso`), the per-boot RAM and sound-card overrides (`guest_ram_mb`, `audio`), the harness argv words (`harness_args`: before the apps, after the apps, per row), and for Linux the row module, the pins file and the isolation (`row` or `shared`).

What the stages do **not** read from configuration and which therefore has no field: the clock and network (they come from the image's sealed `state.json` and `docs/design/V3_CLOCK_RULE.md`), the lane of each application (`ci/vast-broker/propimages.json` per family, step 4 moves its reading behind the bundle table), and the measured families (the same file).

## 2. The resolver

`bundles_for(config, apps, os=None)` returns the bundle ids to fetch so that every requested application is on one of them.

- An application on no bundle is `UNKNOWN_APP:<name>`: nothing is guessed, skipped or counted as a pass (§BA item 2: "not in this config" is stated, never silent).
- Applications may overlap (an application on two bundles, one huge bundle holding everything, a subset spread over several). The answer is the cover with the **fewest bytes**, then the **fewest bundles**, then the **smallest ids**; the list is sorted by id. It depends only on the configuration and the set of applications, never on request order or on the order of the entries. Bytes come first because they are what a rented box is billed for.
- An application on both a Windows and a Linux bundle is `AMBIGUOUS_APP:<name>` unless the caller passes `os`.
- The search is exhaustive over the bundles that hold a requested application (at most 16 bundles, `MAX_BUNDLES`), so "minimal" is exact, not greedy.

`scripts/ci/propimage_config.py validate|resolve` is the command-line form (`KF_PROPIMAGE_CONFIG_REFUSED <reason>` and exit 2 on a refusal).

## 3. The project's own entries

`ci/vast-broker/propimage-bundles.json` holds the project's five bundles as **operator deployment data** (the same place as `propimages.json`, `bandwidth.json` and `private-objects.json`), not code: object keys depend on the project's store layout, and a third party passes its own file of the same schema. Sizes are read-backs of the store listing of 2026-11-19 (MEASURED, `rclone lsl` on the read-only remote).

| id | os | object | what is stated |
|---|---|---|---|
| `imga` | windows | `windows-proprietary/kfimg-20261112/image-v2.zip` | same size, sha256 and ref as `propimages.json` |
| `imgb` | windows | `windows-proprietary/kfimg-20261116/image.zip` | version 1; a rebuilt candidate exists in the store and is **not** entered (not verified) |
| `imgc` | linux | `linux-proprietary/kfimg-matlab-20261117/image.zip` | identity is the full read-back (8128391922 bytes); `propimages.json` and `matlab_pins.json` still carry an earlier build until PR 467 merges |
| `imgd` | windows | `windows-proprietary/kfimg-20261118/image.zip` | **UNVERIFIED**: eleven applications listed (Ansys Student, VRED, Maya as `maya_win`, 3ds Max, AutoCAD, ArcGIS Pro, Solid Edge, KeyShot, Steam, Quake II RTX, Bright Memory), each listed and not yet row-tested; no sha256, no row table; a vendor token is not yet revoked |
| `imge` | linux | `linux-proprietary/kfimg-maya-20261119/image.zip` | **UNVERIFIED**: no sha256, no token check recorded, no row table |

The tests compare `imga` and `imgb` field by field with `propimages.json`, the rows and harness words of `imga` and `imgb` with the stage constants (`APPS`, `WUKONG_FB_MB`, `FB_MB`, `GUEST_RAM_MB`, `AUDIO`), and every `pins_file` with the object that the bundle names. If a stage constant changes, the entry must change with it, in the same change.

*Tension recorded, not decided here.* §BA's 2026-11-18 refinement says the concrete entries and their object keys belong to the Worker's deployment configuration and not to the repository. The repository already carries them (`propimages.json`), and the coordinator's brief for this work asked for the real paths and sizes, so they are in `ci/vast-broker/`, which the Worker deploys from. If the owner wants them out of git, the file moves to the operator's side and the repository keeps only a fixture with synthetic ids; nothing in the validator or resolver changes.

## 4. Next steps (DESIGN-ONLY)

1. **One generic stage.** *Planning half, LIVE:* `scripts/ci/propimage_stage.py plan --bundle ID` computes, from a bundle's `run` entry and its rows table, the rows in order, the lanes, the timeouts, the boots (a discovery-lane row or a `solo_rows` row boots alone; the others fill a boot up to the 2700 s evaluation budget) and the exact `propimage_check.py` argv of every boot (order: fixed words, `before_apps`, `--data-iso`, `--apps`, `after_apps`, `--ram-mb`, `--audio`, `per_row` words, then the arm). A Linux bundle's plan is its rows module's own boot plan for the entry's isolation. **Equivalence (GPU-free test, `test_propimage_stage.py`, run 2026-11-19; no hardware involved):** for `imga`, `imgb` (all rows and a row subset) and `imgc` (row, shared and default isolation) the plan is byte-identical to the committed golden (`scripts/ci/fixtures/propimage_stage_golden.json`, captured from the hardcoded stages of `origin/next`), and the golden is checked on every run against what `propimage_apps.py`, `propimage_apps_b_stage.py` and `matlab_stage.py` compute from their own functions, so the two cannot drift apart. *Running half, DESIGN-ONLY:* the three stages still do the booting, grading and records; replacing their `run` functions (and the three payload stage names and environment names, `vast_report` validators and their mutants) with one stage that takes `--bundle` is a larger change that touches the payload and is not part of this step; the old stage names stay until it lands. **The `arm` lane parameter** is `kf3` (default) or `vfio`: the harness already accepts `--platform kf3|vfio` and `--vfio-gpu ADDRESS`, and the plan adds exactly those words for `vfio` (a PCI address of function 0 is required) and nothing for `kf3`, so the default plan is unchanged; only Windows bundles have a VFIO arm today. Planning never binds a GPU: a real VFIO bind cycle on the maintainers' PC needs the owner's explicit go each time.
2. **The Worker and the planner** (`ci/vast-broker/app-lane.json`, `src/app-lane.js`, `release-plan.mjs`, `bandwidth`) read `bundles_for` and the bundle sizes where they name images today, again behind an equivalence test of the generated plans.
3. **DESIGN INTENT: the lane of an application and the families it is shown to work on stay in `propimages.json`, not in the bundle entries**, until a recorded run moves them; the bundle table says what exists, not how well it has been shown to work.

## 5. What this does not do

It does not change a stage, the payload, the Worker or any generated plan: the generic planner is new code that nothing calls yet. It does not verify the unverified entries. It does not switch image B to its rebuilt candidate, and it does not make any image redistributable.
