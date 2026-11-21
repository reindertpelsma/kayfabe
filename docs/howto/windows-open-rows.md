# Run the open Windows rows (VK-GL-CTS and DirectX samples)

Status: experimental. The rows are written and tested without a GPU; nothing has run on a Windows guest or on native Windows yet. Treat every step as unproven until a run is recorded.

Two sets of checks build open-source test programs from their upstream sources inside a Windows machine and run them: the Khronos conformance suite (OpenGL 4.6 and Vulkan cases, plus a two-context OpenGL soak) and Microsoft's DirectX 12 samples (async compute, ExecuteIndirect, reserved resources, work graphs, HDR). You need nothing but this repository: no account, no licence, no private download. Every download is checked against an official upstream source, and the program sources are fetched at a pinned upstream commit.

## What you need

- Windows 11 with an internet connection, an administrator account and a desktop session you are signed in to (the windowed samples and the OpenGL rows open windows).
- Python 3.9 or newer and the `scripts/bench/windows/open_rows` directory of this repository.
- About 30 GB of free disk space and a few hours for the first build (the Visual Studio Build Tools alone are several GB).

A kayfabe gate guest has no internet, so these rows are reported as not run there unless you prepared the build first or pass your own tree with `--prebuilt`.

## Run

```
python scripts\bench\windows\open_rows\kfopen.py list
python scripts\bench\windows\open_rows\kfopen.py plan vkglcts_gl46
python scripts\bench\windows\open_rows\kfopen.py prepare dxsamples_nbody_async
python scripts\bench\windows\open_rows\kfopen.py run dxsamples_nbody_async
```

`plan` prints what a row downloads and what each download is checked with. `prepare` installs the toolchain, fetches the sources and builds; `run` does that first when needed. Everything lives under `C:\kfopen` (`--root` changes it); the build receipt is `C:\kfopen\out\build-receipt.json`.

A row prints `KFOPEN VERDICT PASS ...`, or a `CHECK <name> FAIL <why>` line, or `KFNOTRUN <row> reason=...` (exit 3: it could not run and says why). The exit code is 0, 1 or 3.

| Row | What it checks |
|---|---|
| `vkglcts_gl46`, `vkglcts_vk` | a few hundred conformance cases each; they PASS only when the per-case results equal those of the same program on native Windows on the same GPU |
| `vkglcts_gl_dual_soak` | two OpenGL 4.6 contexts in one process for ten minutes (`--smoke` for one minute), then a search of the event log for a driver crash |
| `dxsamples_nbody_async`, `dxsamples_execute_indirect`, `dxsamples_reserved_resources`, `dxsamples_hdr` | the sample runs for 40 seconds, is driven with keys, shows non-blank window contents, closes cleanly with exit code 0 |
| `dxsamples_work_graphs` | the console sample runs a work graph and prints the expected shape of output; not run if the adapter has no work graph support |
| `dxsamples_multiadapter` | always not run: it needs two GPUs |

## The native control for the conformance rows

On native Windows with the same GPU model, run the same row with `--role native`, then give the result file to the guest run:

```
python scripts\bench\windows\open_rows\kfopen.py run vkglcts_gl46 --role native
python scripts\bench\windows\open_rows\kfopen.py run vkglcts_gl46 --control C:\kfopen\control\vkglcts_gl46.tsv
python scripts\bench\windows\open_rows\kfopen.py diff --native native.tsv --guest guest.tsv
```

Without a control the row is not run and keeps its per-case results. A case that passes natively and fails in the guest is a candidate bug; the diff lists them. A control from another CTS commit, another case list or another GPU model is refused.

## Trust

By default the run trusts git only. Sources are checked out at a pinned upstream commit and refused if the commit differs. Downloads are accepted only with an official checksum from the upstream origin or the vendor's code signature. You can lift a check, but you have to say so and it is recorded:

| Option | Effect |
|---|---|
| `--accept-unanchored-downloads REASON` | skip the download checks (the reason goes into the receipt) |
| `--source-commit-override cts=<40 hex>` | build another commit of `cts` or `dxs` |
| `--tool git=C:\path\git.exe` | use your own tool (also `python`, `nuget`, `msvc=installed`); trusted, its hash is recorded |
| `--prebuilt C:\my\build` | use a tree you built yourself instead of building; trusted, its hash is recorded |

The design, the list of downloads with their checks, and what is still unverified are in the project's internal design notes, which are not part of this manual. <!-- src: docs/design/V3_WINDOWS_OPEN_ROWS.md -->
