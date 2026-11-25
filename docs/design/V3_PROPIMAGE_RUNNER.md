# The generic image runner: any number of Windows images, their applications, and the clock/network rule

STATUS: DESIGN-ONLY, 2026-11-16 (identity pins strict-only: corrected 2026-11-17, see below; phase 1 built and GPU-free tested; run on no box. Phase 2, the `built` kind and the SPEC-lane unification, is NOT built). Owner ruling: `docs/OWNER_RULINGS.md` section AT. Code: `scripts/bench/windows/clocknet_assert.py`, `propimage_registry.py` with `app_defs/`, `propimage_runspec.py`, `propimage_plain.py`; `scripts/ci/propimage_images.py` (entered through `propimage_apps.py run --image ...`). Tests: `scripts/ci/test_clocknet_assert.py`, `test_propimage_runspec.py`, `test_propimage_images.py`; mutants: `clocknet_assert_mutants.py`, `propimage_runner_mutants.py` (two named killers each, in `ci.yml`).

Labels: MEASURED (a record shows it), CODE READ (read in the tree), INFERRED, NOT RUN (nothing here ran on a box).

## Correction, 2026-11-17: a box with swtpm older than 0.10 is not refused, the runner builds swtpm 0.10.1 first

The generic runner's preflight used to refuse a box whose `swtpm` was older than 0.10 or missing (`REFUSED SWTPM_VERSION`), and the lines of this document and of the instruction that say a precondition such as the hardware refuses the run were read that way. MEASURED 2026-11-17, the first hardware run of the strict generic path on a Vast Ubuntu 22.04 box: swtpm 0.6.3, `REFUSED SWTPM_VERSION` after 7 seconds, before any download or boot. Ubuntu 22.04's package is 0.6.3 and the image's TPM state needs 0.10 (`docs/howto/recreate-the-proprietary-image.md`), so a stock box of that image was refused. The legacy stage (`propimage_apps.py run` without image flags) had the build all along; the generic runner now does the same (`scripts/ci/propimage_images.py`, `build_tools`; the recipe is `propimage_apps.build_swtpm`, not a copy):

- When every other preflight row that can refuse (GPU family, the kf3 QEMU of this revision, free disk) is fine and the box's swtpm is older than 0.10 or missing, the `swtpm` row is `WARN` ("built under /opt/tpm before the first boot"), not `MISMATCH`, and swtpm 0.10.1 is built under `/opt/tpm` BEFORE the first image is fetched or booted. After the build the row is `MATCH` ("built 0.10.1 under /opt/tpm (...)") and `SWTPM` and `LD_LIBRARY_PATH` are set in the environment of every later boot, as the legacy stage sets them. A swtpm of 0.10 or newer changes nothing; a prefix already built by an earlier run is used (the probe runs with `/opt/tpm/lib` on the library path, MEASURED 2026-11-16 for the legacy stage).
- Route signed: the URL file (`kayfabe.download-urls/1`) names the private `ref` of both tarballs in the pins file (`propimage_apps_pins.json`, roles `libtpms` and `swtpm`, `--pins FILE` for your own). They are fetched through the download manifest with the signed sources only (the GitHub archive mirror is not tried), checked against the pinned sha256 and size, which hold in every mode (an unset pin refuses `PIN_UNSET:<role>` before anything is fetched), and built.
- Route git (the standalone default: no URL file, or one without both refs): no tarball. libtpms and swtpm are fetched by the commit that upstream's tag `v0.10.1` points at (`SWTPM_GIT` in `propimage_images.py`: shallow fetch of that one commit, then `git rev-parse HEAD` must equal the pin) and built with the same recipe inside those checkouts. There is no sha256 pin of a built binary. The commits were taken from `git ls-remote <repo> 'refs/tags/v0.10.1^{}'` and a shallow fetch of each was checked on 2026-11-17 (MEASURED: the commit is fetchable by id and its `configure.ac` says 0.10.1); the build itself has NOT run on a box.
- Named refusals, never a guest failure and never a boot: `DOWNLOAD` (route signed fetch), `PIN_UNSET:<role>`, `SWTPM_SOURCE` (git fetch failed or `git` is not on the box), `SWTPM_SOURCE_COMMIT` (the checkout is not the pinned commit; nothing is built), `SWTPM_BUILD` (a step of the recipe failed, or the result is still older than 0.10; the row becomes `MISMATCH`). Exit code 2 (REFUSED), as every precondition.
- The record gains `swtpm: {version, built}` (as the legacy record); `vast_report.validate_propimage_run` accepts it and refuses another shape. Old records without it still validate.
- Unchanged and NOT measured: the build recipe (`SWTPM_DEPS`, `./autogen.sh --prefix=/opt/tpm`) has not been shown to complete on a box; a run of a plain image (no TPM state) builds swtpm too, as it was refused for it before. Tests `test_propimage_images.SwtpmTools`; mutants `swtpm-*` and `report-swtpm-*` in `propimage_runner_mutants.py`.

## Correction, 2026-11-17: a `sha256` in a source is an identity pin, enforced only when strict

Owner rule 2026-11-17 (`docs/OWNER_RULINGS.md` §AW, `docs/design/V3_PINS_ARE_DEPLOYMENT_DATA.md`). Where the text below says a hash is "checked" or a run is refused for "a hash" (the `#sha256=` suffix, `--propimage PATH[:sha256]`, `sha256` in a source, the `SOURCE_SHA256_MISMATCH`, `PIN_SHA256_MISMATCH` and `SHA256_ON_DIRECTORY` refusals, exit code 2 "a hash"), read: only a strict run enforces it. A standalone run always records the computed sha256 and size of what it read (`source.sha256`, `source.size`) with the pin it was given (`source.pin_sha256`) and `source.identity` (`none`, `match`, `differs`, `unverifiable`); a pin that differs is a `KF_PROPIMAGE_RUN_PIN_NOTE` line, not a refusal. The user's own opt-in is `--expect-sha256 NAME=HASH` (spec key `expect_sha256`), enforced in every mode. The record has `pins_enforced` (equal to `strict`). `run --pins FILE` names the deployment's own pins file for `pin:` sources and the legacy stage. The `url` source without a sha256 is no longer "fetched unverified": its hash is computed and recorded like every other.

## What changed and why

The first stage (`docs/design/V3_PROPIMAGE_APPS_STAGE.md`) runs ONE pinned image with seven fixed applications. Owner, 2026-11-16: images and applications are not fixed. The runner now takes a list of images; for each image the applications that run are those the registry has a driver for, intersected with what the image declares. Nothing in the runner names a particular image: names come from the arguments or a `label`.

## The two shapes of a run, and what each guarantees

| | with `state.json` (deterministic) | without it (plain) |
|---|---|---|
| harness | `propimage_check.py`, unchanged | `propimage_plain.py` (new, a sibling; the first file is not edited) |
| identity | fixed: SMBIOS, MAC, disk serial, GPU UUID, TPM and OVMF state from the state | NOT pinned: a per-run seed |
| clock | recorded (`last_time_at_stop + gap`, never earlier) | the host's real UTC now |
| network | off (`restrict=on`); `net: on` is refused by name | off by default; `net: on` gives the guest internet |
| CPU | the fixed profile `kayfabe-amd-v1` (AMD hosts only) | the host's own CPU plus the Hyper-V enlightenments |
| login | from the state | `ssh` in the spec (user, key file, port) |
| reproducible | yes (as far as the image's own state allows) | no: nothing proves two runs saw the same machine |

The safer default for a plain run is offline; the flag that gives internet is explicit (`--net on`). Plain mode is for someone's own VM disk; the Worker's CI shape is the left column.

## The clock/network rule (asserted)

Owner: VMs with internet must have a valid clock; the clock can be faked only if the internet is off. `clocknet_assert.py` holds one rule with two checks that carry distinct codes: `FAKE_CLOCK_NEEDS_OFFLINE` (the guest clock is faked, a recorded base or a clock that does not follow the host, and the network is not provably off) and `INTERNET_NEEDS_REAL_CLOCK` (the network is not provably off and the RTC base is not the host's real UTC now). "Provably off" means no NIC, or only user networking with `restrict=on`; a default NIC, `restrict=off`, a missing `restrict`, tap, socket or an unparsable option counts as internet. The assert reads the QEMU argv itself, never a flag a launcher claims.

It runs in every launch path of these images: `propimage.py build_argv` (all modes and the new `plain`), `propimage_plain.py` (through `build_argv`), the generic runner's proof of each boot (`propimage_images.proof` parses the `run/last_cmd.txt` the harness wrote), `propimage_run.sh` (on the line it is about to exec), and `win_vm.sh` (`vm_start`, every QEMU start of the Windows gate). Not covered: the experiment launchers under `scripts/bench/windows/host_tools/` and `glcrash/` (all use `-rtc base=utc`, so they cannot break the rule today: INFERRED from CODE READ, no assert there).

## The run spec (one schema for the Worker, a file and the command line)

```
{"schema": "kayfabe.propimage-images/1", "images": [ IMAGE, ... ]}            at most 8
IMAGE  = {"label": "any-name",                       default image1, image2, ...
          "kind": "prebuilt" | "built",
          "disk": SOURCE,                            prebuilt: an unpacked image directory, the three-member image zip, or a bare disk
          "iso": SOURCE,                             built (phase 2)
          "state": "auto" | "none",                  auto: use the state.json found with the disk
          "state_file": SOURCE,                      a state.json given separately (needs state auto; makes the shape deterministic)
          "net": "off" | "on",
          "only": ["app", ...],                      restrict what runs
          "apps": {"app": {"mode": "present" | "skip" | "install", "source": SOURCE}},
          "ssh": {"user": "...", "key_file": "...", "port": 22}}
SOURCE = {"from": "official" | "file" | "url" | "pin", "ref": "...", "sha256": "64 hex"}
```

Named refusals (`propimage_runspec.SpecError.reason`): `SPEC_NOT_OBJECT`, `SPEC_SCHEMA`, `SPEC_IMAGES`, `IMAGE_KEYS`, `LABEL_BAD`, `LABEL_DUP`, `KIND_BAD`, `DISK_REQUIRED`, `ISO_REQUIRED`, `ISO_ON_PREBUILT`, `BUILT_KEYS`, `STATE_BAD`, `STATE_CONFLICT`, `NET_BAD`, `NET_ON_WITH_STATE`, `ONLY_BAD`, `APPS_BAD`, `APP_ID_BAD`, `APP_ENTRY_BAD`, `APP_MODE_BAD`, `APP_SOURCE_REQUIRED`, `APP_SOURCE_FORBIDDEN`, `INSTALL_ON_PREBUILT`, `BUILT_NEEDS_APPS`, `BUILT_APP_MODE`, `SOURCE_BAD`, `SOURCE_FROM`, `SOURCE_REF`, `SOURCE_URL_NOT_HTTPS`, `SOURCE_SHA256`, `SSH_BAD`, `SPEC_DUPLICATE_KEY`, `SPEC_NOT_JSON`.

Command line (`propimage_apps.py run|plan ...`; per-image flags belong to the nearest `--image` before them):

- `--image prebuilt:SOURCE` or `--propimage PATH[:sha256]` (a hash here is a pin: see the correction above); `--expect-sha256 NAME=HASH` (repeatable, any mode); `--pins FILE`; `--image built:SOURCE` (validated, refused by the runner: phase 2). `SOURCE` is a path, an `https://` URL, `pin:NAME` or `official`, each with an optional `#sha256=HEX`.
- `--label NAME`, `--state auto|none|PATH`, `--net off|on`, `--apps a,b` (restrict), `--app ID=present|skip|install:official|install:file:PATH|install:url:URL` (repeatable), `--ssh-user U --ssh-key FILE --ssh-port N`.
- `--images-file FILE` (the JSON above) instead of image flags; the two together are refused (`IMAGES_FILE_AND_FLAGS`).
- global: `--sha`, `--checkout`, `--prefix`, `--out`, `--urls`, `--bench`, `--deadline-epoch`, `--evidence-dir`, `--qemu`, `--gop-efi`.
- `propimage_apps.py plan --image ...` prints one line per image (the applications that would run and those that would not) and touches nothing. Without any image flag, `run` is the old stage, unchanged (one pinned image, seven applications, schema 1).

CODE READ: the record is one line `KF_PROPIMAGE_RUN_RESULT {json}` (schema `kayfabe.propimage-run/1`), validated in `vast_report.py` (`validate_propimage_run`): per image the mode, the source name and sha256, per boot the clock and network read back from its argv, per app the verdict and evidence, and `not_run` with reasons (`NO_DRIVER`, `NOT_ON_IMAGE`, `SKIPPED`, `NEEDS:<resource>`).

## Two policies: standalone and strict

STANDALONE (the default): run as much as the inputs allow. Whatever cannot run is SKIPPED with one reason class and that is neither a failure nor a defect of kayfabe: `MISSING_INPUT` (a needed file, login or resource was not given), `NO_DRIVER` (no definition for the application), `NOT_INSTALLED` (not on the image), `NOT_PROVIDED` (nothing declared which applications to run), `CONSTRAINT` (the user's `skip`, or the host cannot run the image's CPU profile). Exit 0 when everything that ran passed; the summary line says `ran N, passed N, skipped M (CLASS n, ...)`. When nothing ran the verdict is SKIPPED and the exit code is still 0 (the summary says `ran 0`).

⊘ **Correction 2026-11-24 (owner, `OWNER_RULINGS.md` §BG):** the STRICT paragraph below ("every item must run, and `zero skips` is asserted") is replaced where it conflicts by the skip rule: a row may end NOT_RUN or skipped when the cause is outside kayfabe (host RAM or VRAM too small for the test, a hardware feature absent, a row that cannot surely run on bare metal), with a named reason; every other NOT_RUN or skip fails its stage. NOT YET IN CODE: `--strict`, `STRICT_SKIP:...` and exit 4 still implement zero skips.

STRICT (`"strict": true` in the spec, `--strict`, the instruction's `propimages_strict`): the Worker's configuration says exactly which images and applications must run; every item must run, and `zero skips` is asserted. A strict spec must name what runs (each image lists at least one `present` app; `only` and `skip` are refused: `STRICT_NEEDS_APPS`, `STRICT_ONLY`, `STRICT_SKIP_ENTRY`); an application with no definition fails before any boot (`STRICT_UNKNOWN_APP:<image>:<app>`, also at the payload's dry run); any skip that still happens (a missing resource, a host that cannot run the CPU profile, an image that could not be read) ends the run FAIL `STRICT_SKIP:<image>:<app or ->:<CLASS>` with every skip in `strict_failures`. What could run still runs and is reported.

Result states per application: PASS, FAIL (and INCONCLUSIVE, LAUNCH_ONLY, HARNESS), SKIPPED (class). Run verdicts and exit codes, distinct: PASS 0, SKIPPED 0, FAIL 1 (an application failed), REFUSED 2 (the spec or a precondition: a hash, an unreadable image, the hardware), RETRY 3, strict skip 4. The record carries `strict`, `summary`, `strict_failures`, and per skip `{app, reason, class}`. The emitted and dry-run instruction show the policy (`propimages_strict`, `KF_PAYLOAD_PROPIMAGES strict=N images=...`).

NOT DONE for the Worker's strict lists: image A's Wukong has no verdict driver today (`propimage_check.py` grades it INCONCLUSIVE; the menu driver is research tooling), so a strict instruction that lists it fails the leg with `APP_FAIL`. The Worker's configuration must not list it until a driver exists; that is a separate follow-up.

## Two lanes, and the per-row result records

Each application row carries `lane: gate | discovery` (default `gate`; the spec's `apps.<app>.lane`, the flag `--app NAME=present,lane=discovery`, the instruction). GATE rows must run and pass: a failure is fatal always (`APP_FAIL`), in both policies. DISCOVERY rows are exotic rows that try to break kf3 and may fail or crash: a failure never fails the run by itself; it is listed in the record's `breakages` (`row`, `class`, `reason`, `host_xid`; class one of CRASH, HANG, TDR, XID, ASSERT, OUTPUT_MISMATCH, read from the harness exit, the guest event log and the host kernel log) and printed as one `KF_PROPIMAGE_RUN_BREAKAGE` line each. The verdict line prints both lanes: `[gate: ran N, passed N | discovery: ran N, passed N, known breakages N]`. Isolation: every discovery row boots alone, after the gate rows, in a fresh throwaway clone with its own hard time-box (its timeout plus the boot slack), so a crashed or hung guest fails only its own row and never masks later rows. A boot whose clock/network proof fails is a gate failure whatever the row's lane. Strict policy treats skips the same in both lanes (zero skips).

Every application of every image also gets one result row in `rows` with stable keys, for comparing runs across a matrix: `die` (GPU name), `family`, `host_driver`, `guest_driver` (the state's `qemu.guest_driver`), `kf3_rev` (the git sha the run was given), `image` (label), `app`, `row` (`label:app`), `lane`, `platform` (`kf3`), `result` (PASS, FAIL, SKIPPED), `failure_class` and `reason` (when not PASS; a skip gives its class and reason), `duration_s`, `started_utc`, `ended_utc` (of the boot the row ran in). A value that is not known is null, never invented: the host driver comes from the caller's hardware facts or `nvidia-smi` and must look like a version; a plain image has no guest driver. At most 96 rows are kept. CODE READ: `vast_report.validate_propimage_run` checks the shape.

## Later (not built; one paragraph each)

- Expected-fail list and ratchet: a small versioned JSON in git (row id, reason, since revision, optional backlog link) would turn the breakages table into a ratchet: a listed discovery row that still fails is fine; a listed row that now passes is reported as XPASS (remove it from the list) and fails a strict CI run; an unlisted discovery failure is a NEW BREAKAGE (non-fatal standalone, fatal in strict CI until triaged); a gate failure stays fatal always. Today every discovery failure is only listed.
- Platform comparison: `--platform vfio` (the same rows on a VFIO reference guest) and a script that compares two result files row by row. The result files already carry `platform`.
- Census: the breakages table carries the host Xid count of the row's window; a parsed census of unserviced IDs is not built.
- Matrix scale (many dies, drivers and updates, a large suite): tiers (a smoke subset on every matrix cell, the full suite on a sparse sample of cells), sharding by application across boxes, and the per-image download time (an image is tens of gigabytes and is fetched per box; a shared cache or a box that keeps its image between cells would change the cost). No scheduler is built; the Worker would choose the rows.

## Which applications run

`propimage_registry.py` keys applications by identity (`resolve`, `3dmark`, ...). The set for an image, in order of precedence: the apps the spec declares `present`; else the state.json `software` list (strings, or objects with `id` or `name`); else a probe of the installed programs over ssh (plain only: the Uninstall registry keys, written to `probe.json`); else the `--apps` list taken as the declaration. `skip` always wins and `--apps` always restricts. A state.json image with nothing declared is refused by name (`NO_APP_DECLARATION`): that shape has no probe. An installed program with no definition is listed in `unsupported` and never an error; an id with no definition is NOT RUN `NO_DRIVER`.

PLUG-IN CONTRACT: a definition set is one file, `scripts/bench/windows/app_defs/<name>.py`, with an `APPS` list (fields in the `propimage_registry.py` docstring). Nothing else is edited; a duplicate identity or alias is refused by name at load. The seven first applications are `app_defs/core_apps.py` (a test pins them equal to `propimage_check.CHECKS` and `propimage_apps.EXPECTED_KIND`). A definition with its own driver names `driver: "module:function"` and, if its driver needs its own harness switches, its own `group` (apps of different groups never share a boot) and `harness_args`.

## The Worker side

The instruction (`kayfabe.payload-instruction/1`) gained one optional list, `propimages`, whose items are the IMAGE objects above (schema in both copies, `payload_instruction.py`, `vast_payload.py`). The Worker builds it from its own configuration of images and the applications on them. With the list present the payload plans the `propimage_apps` stage in the large tier, validates the list up front (`KF_PAYLOAD_PROPIMAGES_REFUSED <reason>`), writes it to a file and runs `propimage_apps.py run ... --images-file`. Only a `PASS` record passes the stage. NOT DONE: the Worker's own configuration and the code that builds `propimages` from it (the Worker code is not touched here).

## Phase 1 (this change) and phase 2 (after the release run)

Phase 1: the list of images; `prebuilt` fully (disk path, URL or pin; optional state.json; apps `present`/`skip`, `--apps`); state.json optional with the two shapes; the clock/network assert everywhere; the registry; the instruction field; docs.

Phase 2 (NOT built; the runner refuses a `built` image by name, `BUILT_NOT_IMPLEMENTED`, before anything else): the `built` kind: an install ISO (official URL, file or URL, sha256 pins in the repository), the unattended install as `win_vm.sh install` does it today (online, real clock: the assert allows it), then each `install` app from its source (official, file, URL) through a per-app install recipe in the registry (silent flags, archive extraction), then the same drivers run the apps. It needs: a recipe field and `official` source pins per application in the definition files; a builder that wraps `win_vm.sh install` and the GOP signing without changing its flows; a place for the built disk; and unifying the SPEC lane. The SPEC lane (`scripts/bench/box/specviewperf_stage.sh`, `docs/design/V3_SPEC_GATE.md`) could be one `built` LTSC image with the apps `spec` (a zip) and `superposition`; it is NOT unified here and what blocks it is: it runs on the gate's own overlay (`windows_gate.sh`), streams a filtered 94 GB tree through `tarfilter.py` rather than copying a zip, and grades through `lane.py` with its own status lines and verdict (`SPEC_STAGE_*`), none of which the registry's driver contract (`function(name, out, guest)` returning a verdict row) expresses yet. Phase 2 would touch `win_vm.sh` (a callable install entry), `specviewperf_stage.sh`, `lane.py`, `vast_payload.py` (stage selection) and `vast_report.py`.

## What is NOT done

- Nothing here ran on a box. The first real proof is the release run's `propimage_apps` stage with two images given on the command line (the exact line is in the pull request).
- `propimage_plain.py` was never booted; its swtpm, OVMF-variables and `-cpu host` choices are CODE READ. A plain image whose Secure Boot, TPM or boot GOP needs the kayfabe database (as the state.json images do) is NOT supported: it boots with the distro's OVMF variables and without the boot GOP (`display=off`).
- Plain-mode drivers: only the core table (`propimage_check.run_check`) and `module:function` drivers; a definition needing harness switches other than `--superposition-run` and `--fb-mb` is refused in plain mode (`PLAIN_HARNESS_ARGS`).
- A state.json image with its apps read from a probe: not supported (see above).
- The experiment launchers under `host_tools/` and `glcrash/` have no assert.
- Every `file` and `url` source is hashed and the record carries its sha256 and size (since 2026-11-17); a `pin` carries its pinned sha256 (verified by the downloader). A directory has no single hash: size only.
