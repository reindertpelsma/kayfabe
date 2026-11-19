# Run everything in one go (the standalone full run)

One command runs every check kayfabe has on one rented or owned GPU machine, strictly: a check that cannot run stops the run by name, it is never dropped. This page is the usage; the list of what is covered, what each part needs and what is known not to run is in `docs/design/V3_STANDALONE_FULL_RUN.md` (internal, not part of this manual).

Nothing here has run end to end yet. Treat the steps as unproven until a run is recorded.

## What you need

- A Linux machine with KVM, one NVIDIA GPU and root, 700 GB of disk and about 64 GB of RAM (the big images ask for a 16 GB guest). The run takes about 28 hours (an estimate: `docs/design/V3_STANDALONE_FULL_RUN.md` has the parts) and downloads about 240 GB.
- The repository at the revision you want to test, `python3`, and the network. The run installs what it needs; it trusts the repository and the upstream checksums it lists, and nothing else (`docs/howto/ci-downloads.md`).
- An instruction file (JSON, `kayfabe.payload-instruction/1`) for the large tier: Windows with two gate runs, the Windows apps profile, the Linux apps profile, the SPECviewperf lanes (`"spec": "both"`), and the list of your private images with `"propimages_strict": true`. `python3 -I scripts/ci/vast_payload.py --emit-instruction FILE` writes one from the environment variables the payload has always read.
- The private objects (images, the SPECviewperf bundle) as files or as presigned URLs in one JSON file (`KF_SPEC_URL_FILE`). They are yours: nothing is checked against a hash of ours, the computed sha256 and size are recorded.
- For the Linux image with the MATLAB rows (the stage is refused by name without it, or skip it): `--matlab-image PATH|URL` and `--matlab-driver VERSION|PATH|URL`. Pass a driver of 590.48.01 or newer: that image refused 580.159.04 and ran with 590.48.01 on one RTX 3080 (2026-11-17).

## The command

```
KF_SOFT_DEADLINE_MS=<now + 32 h in ms> KF_SPEC_URL_FILE=/run/kf-ci/download-urls.json \
python3 -I scripts/ci/vast_payload.py --full --instruction /path/instruction.json \
    [--matlab-image PATH|URL --matlab-driver VERSION|PATH|URL] [--skip STAGE]... <sha40> <nonce32>
```

`--dry-run` (the same arguments) prints the plan and refuses what is missing, and starts nothing. Add it first.

## What it plans

Sixteen stages, in this order: `provision`, `cuda_host`, `merge_bar`, `cuda_guest`, `apps`, `apps_discovery`, `kfml`, `install_flow`, `windows_gate`, `windows_apps`, `windows_gate_2`, `windows_open_rows`, `propimage_apps`, `propimage_apps_b`, `matlab`, `specviewperf`.

Before the first stage runs, every stage whose input is missing is refused by name, one line each, and the run exits with `KF_PAYLOAD_FAIL suite=preflight`:

```
KF_PAYLOAD_FULL_REFUSED propimage_apps:propimages     # the instruction lists no private image
KF_PAYLOAD_FULL_REFUSED matlab:matlab_image            # no --matlab-image and no matlab object in the instruction
KF_PAYLOAD_FULL_REFUSED windows_open_rows:online_host  # this machine cannot reach the internet the online Windows guest needs
KF_PAYLOAD_FULL_REFUSED tier:large  /  propimage_apps:strict  /  windows_gate_2:windows_runs_2  /  specviewperf:spec_both ...
```

## Leaving a stage out

`--skip STAGE` (repeatable) is the only way a stage leaves the run. Every skip is printed in the line below and saved in the result file (`full.skipped`), and the run then says `complete=0`:

```
KF_PAYLOAD_FULL manifest=1 stages=14 skipped=windows_open_rows,matlab complete=0 by_design=apps,apps_discovery
```

A run with no skip says `complete=1`. `provision` and `merge_bar` cannot be skipped, and a stage that builds on another (`windows_apps` and `windows_open_rows` on `windows_gate`, `apps_discovery` on `apps`) cannot be kept when the one it needs is skipped. `--full` cannot be combined with `--only`.

## Rows that do not run by design

A few rows cannot run on one machine for a reason that is not a defect; the line above names the stages that have some (`by_design=`), and the result file names the rows:

- `apps`: four rows that need unified memory (unsupported, documented).
- `apps_discovery`: the DCGM half of `nvml_dcgm` (the run's package installs hold nothing from NVIDIA). Run `scripts/apps/open_rows/run_standalone.sh` for it (`docs/howto/open-linux-rows.md`).
- `windows_open_rows`: `dxsamples_multiadapter` (it needs two GPUs) and the two conformance rows `vkglcts_gl46` and `vkglcts_vk` (they pass only against a control run on native Windows with the same GPU; `docs/howto/windows-open-rows.md`).

Everything else that does not run fails the run, with the stage and the reason in its name (for example `apps_discovery_not_run`, `windows_open_rows_not_run`, `STRICT_SKIP:<row>:<reason>`).

## Open rows: report, never gate

The open rows (`apps_discovery`, `windows_open_rows`, and the discovery rows of the images) are in the discovery lane: a row that ran and failed, crashed or hung is listed as a known breakage in the record and does not fail the run. A row that did not run does.

## The result

`/workspace/kf-result.json`, one `suites` row per stage, `selection` (your `--skip` list), `full` (the plan, the skips, `complete`, the by-design stages) and one record per optional stage (`apps_discovery`, `windows_open_rows`, `kfml`, `matlab`, `propimage_apps_b`, ...). The last lines of the log are `KF_FINAL_SUMMARY`. A stage that was planned and not reached says `result=NOT_RUN`.
