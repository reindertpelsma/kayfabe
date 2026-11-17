# Run applications on your own Windows images

STATUS: DESIGN-ONLY, 2026-11-16. The runner is built and tested without a GPU; no box has run it yet. Plain mode (no `state.json`) was never booted. Treat every step below as unproven until a run is recorded.

The runner starts any number of Windows disk images as throwaway copies, finds out which applications each one has, runs the applications it has a driver for, and reports one verdict per application. It never writes to the image you give it.

## Two kinds of run

| | the image comes with a `state.json` | the image has no `state.json` |
|---|---|---|
| use it for | repeatable checks (CI) | your own VM disk, a quick look |
| machine identity | fixed by the state | not fixed: a fresh random identity each run |
| guest clock | recorded in the state, only moves forward | the real current time |
| network | off | off; internet only if you ask for it with `--net on` |
| reproducible | yes | no |

Two rules hold for every start of these images and are checked from the QEMU command line itself:

- **A guest with internet must have the real clock.** A guest whose clock is set to a recorded time must have its network off.
- So an image with a `state.json` cannot be given internet (`--net on` is refused by name). To give such an image internet, run it as a plain image with `--state none`: you get the real clock and no fixed identity.

## Run one image

An image is an unpacked image directory, the image zip, or a bare disk file (qcow2 and similar). The runner computes the sha256 and size of what it read and writes both into the result. To make a run refuse any file other than the one you expect, add `--expect-sha256 NAME=<64 hex>` (NAME is the image's label or its file name).

```
python3 -B -I scripts/ci/propimage_apps.py run --sha <40 hex> --out result.json --image prebuilt:/data/image.zip
python3 -B -I scripts/ci/propimage_apps.py run --sha <40 hex> --out result.json --image prebuilt:/data/image.zip --expect-sha256 image.zip=<64 hex>
```

With a `state.json` inside the image, the applications come from its `software` list. Say which ones to run with `--apps`:

```
... --image prebuilt:/data/image.zip --apps resolve,3dmark
```

Several images in one call (options after `--image` belong to that image):

```
... --image prebuilt:/data/first.zip --apps resolve,3dmark \
    --image prebuilt:/data/second.zip --app photoshop=present
```

An image without `state.json` needs a login for the guest (an ssh key with access, the user name and the port the guest listens on). Without `--app` or `--apps` the runner starts the guest and reads the installed programs:

```
... --image prebuilt:/data/mydisk.qcow2 --ssh-user me --ssh-key ~/.ssh/id_ed25519 --ssh-port 22
... --image prebuilt:/data/mydisk.qcow2 --ssh-user me --ssh-key ~/.ssh/id_ed25519 --net on --apps 3dmark
```

## swtpm 0.10 or newer

The images' TPM state needs swtpm 0.10 or newer; Ubuntu 22.04 ships 0.6.3. If the box's `swtpm` is older (or missing), the runner builds swtpm 0.10.1 under `/opt/tpm` before the first image starts, as the legacy stage does, instead of refusing the run (not run on a box yet: the build recipe is unproven). This needs root, `apt-get`, `git` and network access to the upstream source repositories, and takes several minutes. A failed step ends the run as REFUSED with a name (`SWTPM_SOURCE`, `SWTPM_SOURCE_COMMIT`, `SWTPM_BUILD`, `DOWNLOAD`, `PIN_UNSET:<role>`), never as a failed application. A swtpm of 0.10 or newer, including one already built under `/opt/tpm`, is used as it is.

## Which applications run

Each application has a short name (`resolve`, `3dmark`, `superposition`, `solidworks`, `edrawings`, `edge`, `wukong`; more can be added, see `docs/design/V3_PROPIMAGE_RUNNER.md`). For an image the runner uses, in this order: the applications you declare with `--app NAME=present`; else the `software` list of the `state.json`; else the installed programs it reads from the guest (plain images only); else the names in `--apps`. `--app NAME=skip` removes one; `--apps` restricts the set.

Anything installed that has no driver is listed as not run (no driver) and is never an error. An application you name that is not on the image is reported not run (not on image).

`python3 -B -I scripts/ci/propimage_apps.py plan --image ... --apps ...` prints what would run for each image and starts nothing.

## Gate rows and discovery rows

Every application can be marked `gate` (the default) or `discovery`: `--app NAME=present,lane=discovery`. A gate row must pass: if it fails, the run fails. A discovery row is allowed to fail or crash (these are rows that try to break the emulation): its failure does not fail the run, and is listed instead as a known breakage with a class (`CRASH`, `HANG`, `TDR`, `XID`, `ASSERT` or `OUTPUT_MISMATCH`) and the reason, one `KF_PROPIMAGE_RUN_BREAKAGE` line each. The summary line shows both lanes: `[gate: ran 3, passed 3 | discovery: ran 2, passed 1, known breakages 1]`. Each discovery row starts its own throwaway copy of the image with a hard time limit, so one that crashes or hangs cannot spoil the next row.

The result file also has one row per application with fixed keys (die, family, host and guest driver, revision, image, application, lane, result, failure class, reason, duration, start and end time in UTC). A value the runner does not know is null.

## Bring your own images

Your images are yours: you can build a Windows image yourself, install software on it that you compiled or obtained yourself, or spread your applications over several images. A normal run does not need any list of approved images or fixed hashes, and a run never refuses an image because its hash is not one it knows.

- **A normal run records, it does not judge.** For every image the result file has the sha256 and size that were read, so you can tell later exactly which file a result belongs to. A `#sha256=<64 hex>` after a path or URL (or `sha256` in a spec file) is kept as a pin. In a normal run a pin that does not match is recorded (`identity: differs`, and one `KF_PROPIMAGE_RUN_PIN_NOTE` line) and the run goes on.
- **Make a check yourself with `--expect-sha256 NAME=<64 hex>`.** NAME is the image's `--label`, or the file name of its source. A file with another hash is refused before anything starts (`EXPECT_SHA256_MISMATCH`, exit code 2). It also works with `--strict`. A directory has no single hash, so an expectation on a directory is refused (`EXPECT_SHA256_ON_DIRECTORY`). A name that matches no image is an error, so a typo is never silently ignored.
- **`--strict` enforces pins.** A strict run is a deployment's check, for example a CI service that lists exactly the images it means. There, a pin that does not match refuses the image (`SOURCE_SHA256_MISMATCH`). Pins that are not about your own images, such as those of tools the repository downloads itself, always hold.
- The integrity of an image you give is still checked: the three-member image zip must match its own `manifest.json` (hashes of the disk and the state), and its seed must match.

To fetch images from your own object store, run your own deployment as described in `docs/design/V3_PINS_ARE_DEPLOYMENT_DATA.md`: build the images with the scripts in this repository, upload them to your own bucket (any S3-compatible store), write a pins file that names them, and pass it with `--pins FILE`. Then `--image prebuilt:pin:NAME` fetches the object `NAME` through presigned URLs that you supply with `--urls FILE`.

## What happens to things that cannot run

By default the run does as much as it can. Anything that cannot run is skipped with a reason, and a skip is not a failure: the exit code is 0 when everything that ran passed, and the last line says plainly `ran 3, passed 3, skipped 2 (NO_DRIVER 1, NOT_INSTALLED 1)`. The reasons are `MISSING_INPUT` (something you did not give: a file, a login), `NO_DRIVER` (no driver for that application), `NOT_INSTALLED` (not on the image), `NOT_PROVIDED` (you declared nothing and there is nothing to read) and `CONSTRAINT` (you asked to skip it, or this machine cannot run the image).

For checks that must be complete, add `--strict`: then every image and application you list must run, and any skip fails the run (exit code 4, `STRICT_SKIP:<image>:<app>:<reason>`). A strict run needs every image to list its applications with `--app NAME=present` (`--apps` and `--app NAME=skip` are refused) and refuses an application that has no driver before starting anything.

| exit code | meaning |
|---|---|
| 0 | everything that ran passed (skips allowed unless `--strict`); also when nothing could run |
| 1 | an application failed |
| 2 | refused: a bad option, a file that does not have the hash you gave with `--expect-sha256`, an image that cannot be read, the wrong hardware |
| 3 | retry may help (a harness problem, no time left) |
| 4 | `--strict` and something was skipped |

## The result

One line `KF_PROPIMAGE_RUN_RESULT {json}` and the same text in the `--out` file. Per image: the shape (`ci` for a state image, `plain`), the clock and network each boot proved, each application's verdict and evidence, and the applications that were not run with the reason. The exit codes are in the table above.

## Not available yet

- `--image built:...` (install Windows from an ISO and install the applications from files or URLs) is accepted by the option parser and refused by name: `BUILT_NOT_IMPLEMENTED`.
- A plain image that needs the project's Secure Boot key or boot display driver to start.
