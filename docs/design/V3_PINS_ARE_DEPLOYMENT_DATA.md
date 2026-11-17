# Pins are deployment data: identity pins are strict-only, a standalone run records, and anyone can run their own deployment

STATUS: LIVE, 2026-11-17 (owner rule of 2026-11-17, `docs/OWNER_RULINGS.md` §AV; built and GPU-free tested; no box ran it). Code: `scripts/ci/propimage_images.py` (`settle_identity`, `materialize`, `all_pins`), `scripts/bench/windows/propimage_runspec.py` (`pins_enforced`, `check_expect`, `--expect-sha256`), `scripts/ci/propimage_apps.py` (`--pins`), `scripts/ci/vast_report.py` (`validate_propimage_run`), `scripts/bench/linux/propimage_linux.py` (`install_driver`). Tests: `scripts/ci/test_propimage_pins.py`; mutants: `scripts/ci/propimage_runner_mutants.py` (two named killers each, in `ci.yml`). User-visible behaviour: `docs/howto/run-windows-images.md`, section "Bring your own images".

Labels: CODE READ (read in the tree), DESIGN INTENT (decided, not built). Test results are stated as plain facts of the unit tests. Nothing here ran on a box.

## The rule

A pin says which object a deployment means: a fixed sha256 of "the" image, a fixed object name, a fixed size. That is data about OUR deployment (the Worker and strict CI), not a property of the product. Someone who runs the runner themselves may have built their own Windows images, compiled their own software, or spread their applications over different images. So:

1. **Identity pins on user-supplied objects are strict-only.** The `sha256` of an image or state file given by `--image`, `--propimage PATH|URL`, `#sha256=`, a spec file or a state.json is enforced only when `propimage_runspec.pins_enforced(spec)` is true, and that is true only for a strict spec (`--strict`, the instruction's `propimages_strict`, which is what the Worker always sends). It is ONE switch, read in one place (`propimage_images.settle_identity`).
2. **A standalone run records.** For each user-supplied object the record always carries the computed `sha256` and `size`, the pin that was given (`pin_sha256`) and `identity` (`none`, `match`, `differs`, `unverifiable`). A pin that differs is a note (`KF_PROPIMAGE_RUN_PIN_NOTE`), never a refusal. An image whose pin differs in a strict run is refused by name (`SOURCE_SHA256_MISMATCH`, `PIN_SHA256_MISMATCH`, `SHA256_ON_DIRECTORY`, `STATE_FILE_SHA256_MISMATCH`), after the hash was recorded.
3. **`--expect-sha256 NAME=HASH` is the user's own opt-in** (spec key `expect_sha256`): enforced in every mode (`EXPECT_SHA256_MISMATCH`, `EXPECT_SHA256_ON_DIRECTORY`). NAME is an image label or its source's file name; a name that calls no image is a spec error (`EXPECT_SHA256_UNKNOWN`), duplicates with different hashes are refused (`EXPECT_SHA256_DUP`). It is not a pin, so it is not subject to the switch.
4. **Supply-chain pins stay in every mode.** What the repository itself downloads (the swtpm and libtpms sources, the Ubuntu cloud image, the noVNC release and its wheel, the Windows install media, the OpenSSH MSI and the NVIDIA installer of the image builder, the Resolve open media) is pinned so that a changed upstream or a poisoned mirror is caught. These pins are not about a user's image and nothing a spec says can weaken them.
5. **Integrity checks stay.** A pack's `manifest.json` member hashes, the seed binding and the cache-key checks are about whether the files are consistent, not about which image is meant.

## Audit: where a pin is enforced on a path a standalone run takes (CODE READ, before this change)

| # | Place | What it pins | Standalone before | Now | Class |
|---|---|---|---|---|---|
| 1 | `propimage_images.materialize`, `url` source: `fetch_url(.., sha256)` | the image bytes a user named by URL (`#sha256=`) | yes | strict only (recorded otherwise) | user image identity |
| 2 | `materialize`, `file` source: `SOURCE_SHA256_MISMATCH` (`--propimage PATH:sha256`, `#sha256=`) | the user's image file | yes | strict only | user image identity |
| 3 | `materialize`, directory with a sha256: `SHA256_ON_DIRECTORY` | a directory cannot carry one | yes | strict only (recorded `unverifiable`) | user image identity |
| 4 | `materialize`, `pin:` source: spec sha256 vs the pins file `PIN_SHA256_MISMATCH` | which object a deployment's spec means | yes | strict only | deployment identity |
| 5 | `_synth_dir`, `state_file` sha256 | the user's state.json | no (the field was parsed and ignored) | recorded always, strict enforces | user image identity |
| 6 | `propimage_linux.install_driver`, `--sha256` (was required) | the user's NVIDIA `.run` | yes (required) | optional opt-in; the result carries sha256 and size | user artifact |
| 7 | `vast_report.validate_propimage_run` source block | shape of the record | n/a | records `size`, `pin_sha256`, `identity`; refuses a record that contradicts the rule | integrity of the report |
| 8 | `materialize`, `pin:` source: `PIN_UNKNOWN`, `PIN_UNSET:<role>` | an object must be in a pins file with sha256 and size to be fetched | yes | unchanged (both modes) | deployment object |
| 9 | `box_downloader.py` check/fetch/verify of a `pin:` object | transfer integrity of that object | yes | unchanged (both modes) | deployment object |
| 10 | legacy `propimage_apps.py run` (no image flag): the image and GOP of `propimage_apps_pins.json`, preflight row `pins`, `PIN_UNSET:image` | the deployment's fixed image | yes | unchanged: this IS the deployment stage; your own file with `--pins FILE` | deployment stage |
| 11 | `propimage_apps_pins.json` roles `libtpms`, `swtpm` (`build_manifest`) | swtpm 0.10.1 sources the box builds when its swtpm is old | yes | unchanged | supply chain |
| 12 | `propimage_linux.BASE` (`fetch-base`, `BASE_SHA_MISMATCH`) | the Ubuntu cloud image | yes | unchanged | supply chain |
| 13 | `propimage_linux.NOVNC` (wheel with `--require-hashes`, the noVNC tarball) | noVNC and websockify | yes | unchanged | supply chain |
| 14 | `propimage.py` `PINS` (OpenSSH MSI, NVIDIA 580.88 installer) in `write_tools_dir` | the tools CD of the image builder | yes | unchanged | supply chain |
| 15 | `win_vm.sh` `MEDIA` (Windows media, virtio-win, tools) | install media | yes | unchanged | supply chain |
| 16 | `resolve_media.py` (open sample media, "the pin decides") | the Resolve test media | yes | unchanged | supply chain |
| 17 | `propimage_check.load_image`, `propimage.unpack`: manifest members, state blobs, seed | consistency of a pack | yes | unchanged | integrity |
| 18 | `ci/vast-broker/propimages.json`, `src/propimages.js` (`NOT_STRICT`, `UNPINNED:<id>`, object sizes and sha256) | the Worker's own objects | n/a (Worker) | unchanged: strict always | Worker |

Counts: 6 user-supplied identity pins changed (rows 1 to 6; row 5 was not enforced before), 3 deployment-identity places unchanged but reachable standalone only through `pin:` or the legacy stage (rows 8 to 10), 6 supply-chain pins unchanged (rows 11 to 16), 1 integrity group unchanged (row 17), 1 Worker config unchanged (row 18); row 7 is the report's validator, extended. NOT covered: the experiment launchers under `scripts/bench/windows/host_tools/` read no pin.

Why rows 8 to 10 stay: choosing `pin:NAME` or the legacy stage means "fetch the object my deployment named". The downloader needs the pinned sha256 and size to resume and verify a transfer (its manifest takes them from the pins file, never from a server), so an object with no pin is never fetched through it. A user without a deployment passes the image as a path or URL, where nothing is required.

## Run your own deployment

Everything the Worker does has a standalone shape, using plain S3 semantics. R2 is S3-compatible and nothing here uses an R2-only feature (CODE READ: `box_downloader.py` presigns with AWS SigV4 against any `endpoint` and `region`; its `signed` source reads a presigned-URL file; its `s3` source takes an endpoint, key pair and region from `--s3-config` or the `KF_S3_*` environment).

1. **Build the artifacts with the repository's scripts.** The images with `scripts/bench/windows/propimage.py` and `docs/howto/recreate-the-proprietary-image.md` (or the Linux image with `scripts/bench/linux/propimage_linux.py`); the signed boot GOP with `win_vm.sh`; applications you install yourself. Nothing is redistributed by the repository.
2. **Publish them to YOUR object store.** Any S3-compatible store: upload with any S3 client; keep the bucket private.
3. **Write YOUR pins file** (`kayfabe.propimage-pins/1`, same format as `scripts/ci/propimage_apps_pins.json`): per object a `ref` (the name the runner looks up), `dest`, `sha256`, `size`, `role` (`image` and `gop` are required) and `needed_by`. Your refs add to the repository's and win over one of the same name (`propimage_images.all_pins(extra=...)`).
4. **Presign.** Write the presigned-URL file (`kayfabe.download-urls/1`, mode 0600, `{"urls": {"<ref>": "<https url>"}}`) with any S3-compatible presign (the AWS SDK, `aws s3 presign`, rclone, `box_downloader.py`'s own SigV4 code). Pass it with `--urls FILE`.
5. **Run.** `propimage_apps.py run --pins YOURS.json --urls URLS.json --image prebuilt:pin:<ref> ...` fetches your object through the downloader and verifies your pin. Add `--strict` to make the run enforce spec pins like the Worker does, or leave it off to have the sha256 and size recorded.

DESIGN INTENT, not built: the Worker's own configuration (`ci/vast-broker/propimages.json`) is the same data in the Worker's format (`object` is the key in its private bucket). A second operator would write their own file of that shape and deploy their own Worker; this note does not change the Worker.

## Tests and mutants

`test_propimage_pins.py`: the matrix {standalone, strict} x {pin matches, pin differs, no pin} for a file source (6 tests), the same rule for a URL, a `pin:` object, a directory and a state file, `--expect-sha256` in both modes (match, differ, name by label or file name, directory, unknown name, duplicates, command line), the `--pins` flag and the own-pins-file override, the validator's contradictions, the downloader still verifying `pin:` objects and the supply-chain pins still refusing. The existing strict tests of `test_propimage_images.py` now pass a strict spec and assert the same refusals. 27 mutants in `propimage_runner_mutants.py` (two named killers each).

## What is NOT done

- The `official` source and the app `install` sources belong to the `built` kind (phase 2, refused by name); when that lands, their pins (the install ISO of an `official` source) are supply-chain pins and stay in every mode, while a user's own ISO or app media given as `file` or `url` follow rule 1.
- `ci/vast-broker/propimages.json` and the Worker's `src/propimages.js` are unchanged and stay strict.
- Nothing ran on a box.
