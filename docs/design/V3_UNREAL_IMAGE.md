STATUS: DESIGN-ONLY, 2026-11-17. The tooling in `scripts/bench/unreal/` is GPU-free and unit-tested with a fake runner (136 tests, 55 mutants, RUN on 2026-11-17); no source tree was built, nothing was downloaded, no guest was booted, no box was rented and nothing was uploaded. Every number is an ESTIMATE unless it is labelled otherwise.

# Unreal Engine as a cached installed build, run in the open Linux guest

Labels: **CODE READ** (read in the Unreal source tree the project owner supplied, through the code host's API, not run), **TESTED** (a GPU-free unit test or mutant ran), **ESTIMATE** (a number nobody measured), **UNVERIFIED** (an inference with its falsifier), **DESIGN** (a decision or proposal, not built).

Wording note. The Unreal Engine source is licensed to the user who holds it. This repository neither contains nor redistributes it, nor any Epic content, and it describes the tree only as "an Unreal Engine source tree supplied by the user". Built artifacts are kept in a private object store because the licence forbids redistribution (they are not secret). Anyone can run the same pipeline with their own licence and their own store (section 7).

## 1. What changed from a sealed image to a cache, and why

There is **no sealed Unreal image**. No vendor sign-in, entitlement or licence state is involved: Unreal is a source build. So the Linux proprietary-image tooling (`docs/design/V3_PROPIMAGE_LINUX.md`: init, setup, seal, harden-offline) is not used. Unreal runs in the **open Linux guest** that the payload already builds and boots for `cuda_guest`, `apps` and `kfml` (`scripts/fastguest/`, `scripts/apps/`, offline), and the installed build is **loaded into it at run time**.

The real input is the **source tree**. The private object is only a **cache** that spares a rented box the build:

| Who | Gets the installed build by | Notes |
|---|---|---|
| A standalone user with a source tree | building it on their own machine with `scripts/bench/unreal/build_installed.py`, or by naming a prebuilt artifact they already have | Hours of build, 200+ GB of disk; nothing is uploaded unless they choose |
| The deployment's Worker and its rented boxes (untrusted) | fetching a finished artifact by key from the deployment's private store | A box never builds; a cache miss is a named failure |
| The maintainer's trusted build machine | the same script, then `pin` and `upload` | Produces the cache for the deployment |

## 2. The artifact

### 2.1 Installed build, not the full build tree

An "installed build" is what Epic's own BuildGraph script produces for the launcher-style binary distribution: editor, tools, templates, content, the derived-data pack and the bundled toolchain, without sources or intermediates. CODE READ: the script is `Engine/Build/InstalledEngineBuild.xml`; the node for a Linux host is `Make Installed Build Linux`; it is run as

```
Engine/Build/BatchFiles/RunUAT.sh BuildGraph -target="Make Installed Build Linux" -script=Engine/Build/InstalledEngineBuild.xml \
    -set:HostPlatformOnly=true -set:WithLinuxArm64=false -set:WithDDC=true -set:WithClient=false -set:WithServer=false \
    -set:CompileDatasmithPlugins=false -set:GameConfigurations=Development -set:BuiltDirectory=<dir> ...
```

CODE READ details that shaped the recipe: (a) `HostPlatformOnly` switches off every platform except the host's, but on a Linux host it leaves the Linux AArch64 target **on**, so the recipe switches it off explicitly (a cross-build of a second architecture would add hours and gigabytes); (b) `WithDDC` makes the script build a derived-data pack for the engine content and the templates and include it as `Engine/DerivedDataCache/Compressed.ddp`; (c) `GameConfigurations` defaults to three configurations, the recipe keeps Development only (the benchmark and stat switches need a non-shipping build; saves time and size); (d) `BuiltDirectory` defaults to a folder inside the source tree, the recipe moves it so the output can sit on another filesystem; (e) the Linux installed tree is `<BuiltDirectory>/Linux`.

| Artifact | What it holds | Size (ESTIMATE) |
|---|---|---|
| **Installed build (the artifact)** | Linux editor and tools, templates and the engine's starter content, `Compressed.ddp`, bundled toolchain, third-party libraries | 25 to 40 GB uncompressed; 14 to 24 GB as `tar` + zstd level 3 |
| Full build tree (never stored) | source, dependencies, intermediates | 150 to 250 GB |
| Source tree (never stored) | working tree and dependency set | 45 to 65 GB |

Compression: zstd level 3, multi-threaded, streamed from `tar` (no second uncompressed copy). Much of the tree is already compressed (content packages, the derived-data pack), so the ratio is modest (ESTIMATE 0.55 to 0.7). Level 3 is chosen over a higher level because the box pays CPU for decompression and the saving is small.

Transfer to a box at the Vast bandwidth price of at most 0.01 USD per GB (the project's budget figure): about 20 GB is about 0.20 USD per box (ESTIMATE), and 4 to 11 minutes at 30 to 80 MB/s. The artifact is unpacked on the box host (ESTIMATE 2 to 6 minutes, bounded by disk writes), so a box needs about 20 GB for the archive plus 30 GB for the tree.

### 2.2 The derived-data cache: part of the artifact, plus an optional second artifact

The pack inside the artifact (`Compressed.ddp`) is the engine's own, built for the editor target platform with whatever shader formats the build's default settings select. DESIGN, UNVERIFIED which formats those are: if the Vulkan SM6 shader format that Nanite and Lumen need on Linux is not among them, the first scene run still pays a shader compile.

A **second artifact**, `ddc-warm`, is evaluated, not decided:

| | Without | With `ddc-warm` |
|---|---|---|
| First scene row on a clean box | compiles the scene's shaders: ESTIMATE 20 to 90 minutes on the guest's vCPUs | reads them: ESTIMATE 2 to 10 minutes |
| Transfer and disk | none | ESTIMATE 3 to 12 GB more per box |
| What it is | nothing to build | the local derived-data folder of one run of the scene rows on the trusted machine, packed with the same manifest and key rules (its key adds the artifact key and the project list) |
| Portable across dies? | | Shader bytecode in the derived-data cache is keyed by shader and format, not by GPU (INFERRED; driver pipeline caches are separate and are not in it) |

Recommendation: do **not** build `ddc-warm` first. Run `ue_shader_storm` and the first scene row once on a box, record the compile time, and build the second artifact only if the first-run compile of a scene is longer than about 20 minutes. The storm row itself wants an **empty** cache, so rows choose their cache folder: empty for the storm row, warm for the scene rows.

## 3. The cache key and the manifest

`build_installed.py key --source DIR` prints the key and its inputs. The key is the sha256 of the canonical JSON of:

| Input | Where it comes from |
|---|---|
| Source identity | the git commit of the tree; or, for a tree without history, a user-declared id (`--source-id`). A tree whose tracked files differ from the commit is refused unless `--allow-dirty`, and then a digest of the difference is part of the key |
| Dependency manifest | the sha256 of the tree's dependency list file, when present |
| Engine version | the version numbers, branch and compatible changelist from the tree's version file |
| Build-recipe revision | `r<version>-<12 hex>` of the recipe dict in `ue_lib.py`: every flag, target and archive setting. Editing the dict changes the key; editing tests or docs does not |
| Toolchain | the SDK version string the tree asks `Setup.sh` to fetch (the clang version is part of it). Readable before any build, so a key can be computed from source alone. The compiler the build actually ran is recorded in the manifest as an observation, never in the key |
| Target platform | Linux, x86_64 |
| Build flags | the BuildGraph target and every `-set:` option |

The manifest (`kayfabe.unreal-artifact/1`, `manifest.json`, next to the archive) records the key **and its inputs**, the member name, size, sha256, compression, uncompressed size and file count, engine version, source id, recipe revision and the kayfabe revision, the observed compiler, the sonames the installed binaries need from the system (an `ldd` audit on the build machine), the builder's CPU count and RAM, `built_utc` and `created_utc`, and `loads_software: true`. Validation is strict and **recomputes the key from the inputs**: a manifest whose key is not the hash of its inputs is refused (TESTED). The manifest holds no source content.

**Reproducibility.** Anyone with the same source and the same recipe revision can regenerate the artifact: the archive is sorted, with a fixed owner, and the build id is derived from the key. This is **reproducible modulo timestamps**. Bit-identical output of two builds is **UNVERIFIED** (compiler timestamps, the build's own random identifiers and zstd framing are the suspects); nothing here depends on it, because identity is the key, not a hash.

## 4. Build-or-fetch: where a run gets the installed build

`resolve_unreal.py` implements the order; `build_installed.py resolve` prints the decision. First match wins (TESTED):

1. **`--unreal-artifact PATH|URL`** (with `--unreal-manifest`, or a `manifest.json` beside it): use it. Its members are checked against **its own manifest** in every mode. If `--unreal-source` is also given, the key that source and recipe produce must equal the manifest's key, else `UNREAL_CACHE_KEY_MISMATCH`. A plain installed-build **directory** (for example one the user already has) is accepted in standalone mode only, with no key and a note that provenance is theirs.
2. **A cache entry named by the deployment** (the instruction or a URL file): fetch it. With a source also given and a different key: standalone rebuilds (and prints `UNREAL_CACHE_KEY_MISMATCH action=rebuild`), strict fails.
3. **`--unreal-source PATH`** or **`--unreal-source-clone [REPO]`** (default repository the vendor's own; the user's `gh` login is used, never prompted for): build it, and say so: hours, the disk estimate, and a refusal below the disk budget. Optionally write the artifact and manifest to `--unreal-cache-out DIR`. If neither a login nor repository access exists, standalone **skips**.
4. **Nothing**: standalone **skips** with `UNREAL_NO_SOURCE_OR_CACHE` (not a failure, exit 0); strict fails with the same class; worker fails with **`UNREAL_CACHE_MISS`**, which tells the owner to rebuild on the trusted machine, upload and pin.

| Mode | Pins enforced | Builds | Nothing available |
|---|---|---|---|
| `standalone` (default on a user's machine) | **no** | yes, from a source | skip, exit 0 |
| `strict` (a user who wants a missing input to fail) | yes | yes | fail |
| `worker` (a rented, untrusted box) | yes | **never** (`UNREAL_WORKER_BUILD_REFUSED`) | `UNREAL_CACHE_MISS` |

**Hashes in standalone mode.** A user who compiled the engine has different bytes, so a standalone run does **not** require a sha256 pin for anything Unreal. Integrity checks stay: the artifact's members against the artifact's own manifest, and the key consistency check when both a source and an artifact are given. A standalone run only **records** the sha256 it computed in its evidence; `--expect-sha256` exists for a user who wants to demand one (`UNREAL_EXPECT_SHA256_MISMATCH`). Pins (name, size, sha256) are the deployment's data and are enforced only in strict and worker modes (`UNREAL_PIN_MISMATCH`).

**Why a Worker box never builds.** Boxes are untrusted and carry no secrets. A build needs a credential to the source, 200+ GB of disk beyond what the box request guarantees, 3 to 7 hours of box time per leg (ESTIMATE) against about 4 to 11 minutes of download, and tens of GB from the vendor's delivery network; and a compiled artifact made on an untrusted box could not be trusted by anything that loads it. A runtime build is acceptable only on a from-scratch standalone user's **own** machine.

The standalone flags (`--unreal-artifact`, `--unreal-manifest`, `--unreal-source`, `--unreal-source-clone`, `--unreal-ref`, `--unreal-cache-out`, `--expect-sha256`) are meant for the existing runner's flag parser; `resolve_unreal.decide()` is the importable function it calls, and `build_installed.py` is the thin wrapper a maintainer uses today. Wiring them is not done.

## 5. Delivery into the open Linux guest

The open guest is the one the `apps` stage boots (`scripts/apps/image/recipe.json`: Ubuntu 24.04, 74 GiB disk, 12 GiB RAM, 2 CPUs, NVIDIA driver supplied at run time, offline).

**Primary: a read-only virtio-fs share**, the pattern of the `llm_parity` stage (`docs/design/V3_LLM_PARITY_STAGE.md`, draft PR #428): the box unpacks the artifact into a directory, `virtiofsd --readonly` (rootless, uid 65534, `--sandbox none`) serves it, the guest mounts it `-t virtiofs -o ro` at a fixed path and proves the mount is read-only and a write fails. The guest never has write access, and 25 to 40 GB is not copied into the guest disk. Prerequisites from that design, **not met today**: the kf3 QEMU build is configured without `vhost-user` (so no `vhost-user-fs-pci`), the box needs a static `virtiofsd` for glibc 2.35, and the guest kernel needs `virtiofs`. The 9p share of the same design is the documented fallback.

**Alternative worth deciding** if virtio-fs stays blocked: the same tree as a read-only **block device** (squashfs or ext4 image, `readonly=on` on a virtio-blk drive): no daemon and no QEMU rebuild, one file, compressed on the box's disk. Costs: an image build step on the trusted machine (a second artifact format, same key) and guest CPU for decompression on random reads. DESIGN, not built.

**`inject`** (`scripts/bench/linux/propimage_linux.py`, PR #457: tar over ssh, destinations under `/opt/kf` or the guest home, a size cap defaulting to 20 GB) is the fallback for **small** trees only (a project, a packaged sample); it is not meant for the installed build.

**Writable scratch.** The editor needs a writable place for the project copy, `Intermediate`, `Saved` and the local derived-data folder (the local derived-data path is an overridable setting, CODE READ). The base image is not touched: the stage attaches a **second sparse virtual disk**, ESTIMATE 60 GiB (project copy and intermediates 8 to 20 GB, shader cache 5 to 15 GB without `ddc-warm`, logs and screenshots), formatted at boot. UNVERIFIED which paths an installed build writes under the engine directory; the read-only share turns any such write into a visible failure on the first run, which is the falsifier.

**Guest size.** The default open guest is too small. The stage boots it with at least 24 GiB RAM and 8 vCPUs (the boot variables `NVKVM_RAM_MB` and `KF_SMP` exist for this; ESTIMATE, the editor's own stated minimum is 16 GB), so the rented box needs the RAM for it.

**Packages the editor needs.** They go through the open guest's own provisioning (`apt.packages` in `scripts/apps/image/recipe.json`, snapshot-pinned, documented in that recipe's inventory), **not** a sealed image. The Vulkan loader and tools (`libvulkan1`, `vulkan-tools`) and several X libraries are already in the list. The delta is **found by an audit, not guessed**: `build_installed.py build --cache-out` runs `ldd` over the installed binaries and records the sonames the build machine lacks in the manifest (`host_libs`); mapping them to Ubuntu 24.04 packages is a one-time step. Candidates UNVERIFIED until the audit: `libx11-6`, `libxcb1`, `libxext6`, `libxrandr2`, `libxcursor1`, `libxinerama1`, `libxkbcommon-x11-0`, `libasound2t64`, `libpulse0`, `libdbus-1-3`, `libgbm1`, `libdrm2`, `libfontconfig1`, `libfreetype6`, `libglib2.0-0t64`, `libatomic1`, and `xvfb` as a fallback display. `zstd` is needed on the **box host** (to unpack), not in the guest. The recipe is not edited by this change.

**Vulkan only.** The editor on Linux renders through Vulkan; DXR is Windows. A software Vulkan driver must not be able to pass a row: the guest has no `mesa-vulkan-drivers`, the ICD is the one the NVIDIA driver installs, and each row asserts GPU use from the host (below).

**Offline.** No derived-data cloud, no vendor services: the guest is offline (`restrict=on`). UNVERIFIED: how long the editor takes to start with no network (analytics, account and update checks may wait for a timeout); falsifier: time the same start online and offline on the first run, and add the switches that remove the wait.

**Clock.** The open guest's rule applies (`docs/design/V3_CLOCK_RULE.md`): offline means the recorded clock. The artifact is **loaded software**: its pin carries `loads_software: true` and `created_utc`, so the recorded clock is raised to at least `created_utc` plus the 1 h margin and never refused for being older (TESTED: `clockrule.pin_created` reads the entry the tool emits). No per-application clock branch.

## 6. The rows

Defined in `scripts/bench/unreal/rows.json` (validated by `ue_lib.validate_rows`, TESTED). Every row starts in the **discovery lane**; promotion to the gate lane after three consecutive clean runs on at least two dies (proposal). Exit status zero is never a verdict: each row names its host-side verification and its GPU-use assertion. GPU use is asserted from the **host** (`nvidia-smi` utilisation and memory rise sampled outside the guest; the guest's own counters read `[N/A]` under kf3, `V3_PROPIMAGE_LINUX.md` section 7). "Verified in source" means the name was found in the source tree; nothing was run.

| Row | Driven by | Host verification | GPU assertion | Names |
|---|---|---|---|---|
| `ue_vk_caps` | `vulkaninfo --json` in the guest | the ray-tracing, mesh-shader and shading-rate extension list equals the host's; device name equals the host's (a software ICD fails) | none (a query; says so) | tool |
| `ue_rhi_automation` | `UnrealEditor-Cmd ... -ExecCmds="Automation RunTests Rendering.RHI; Quit"` | exported report: tests run at least the bare-metal count, none failed | memory | `Rendering.RHI` verified in source; the plugin is off by default and its presence in an installed build is UNVERIFIED |
| `ue_rt_automation` | `Automation RunTests System.Renderer.RayTracing.BasicRayTracing` | report; the host decides skip: host supports Vulkan ray tracing and the guest skipped is a FAIL (parity) | memory | verified in source |
| `ue_vt_automation` | `Automation RunTests System.Renderer.VirtualTexture` | report | memory | verified in source |
| `ue_shader_storm` | empty local derived-data folder, `RecompileShaders all` | no hang, no worker crash, shader count equals the host's | none (CPU and scheduler row; says so) | command verified in source; its argument and the log lines UNVERIFIED |
| `ue_scene_nanite_vsm` | a template map, `r.Nanite`, `r.Shadow.Virtual.Enable`, `HighResShot` | image not flat; mean absolute difference to the bare-metal image under a threshold fixed from two host runs; sha256 recorded, not required | utilisation | cvar names UNVERIFIED |
| `ue_scene_lumen_sw` | same scene, software Lumen | same | utilisation | UNVERIFIED |
| `ue_scene_lumen_hwrt` | same scene, `r.RayTracing 1`, hardware Lumen on Vulkan ray tracing | same; skipped (`CONSTRAINT`) when `ue_vk_caps` shows no host support | utilisation | Vulkan ray-tracing cvars exist (verified); `r.RayTracing` UNVERIFIED |
| `ue_packaged_benchmark` (phase 2) | a packaged template run with `-benchmark -benchmarkseconds= -fps=` | fixed time step, so the frame count for the stated seconds is checkable; image not flat | utilisation | `-benchmark`, `-benchmarkseconds=`, `-fps=` verified in source; `-dumpstats` was **not found** in the sources read, so stat output comes from the log |

Shared UNVERIFIED: whether Nanite, Lumen and virtual shadow maps run on Linux Vulkan only with the SM6 shader format enabled (a project setting), and whether headless rendering needs a virtual display (`-RenderOffScreen` is verified to exist; whether the windowing library starts without one is not). The first run answers both; the fallback is `xvfb`. A bit-identical image is not expected (temporal passes). Templates and the engine's starter content come from the engine's own tree; no Marketplace or launcher content is used or available.

`ue_packaged_benchmark` needs a **packaged sample**, a second artifact built (cook and package of an engine template) on the trusted machine with the same key rules: phase 2.

## 7. Build, upload, pin: your own deployment

### 7.1 The build (`scripts/bench/unreal/`)

Stdlib Python. `build_installed.py build --source DIR --work DIR --allow-modify-source [--package] [--cache-out DIR] [--reclaim] [--clone REPO --ref REF]`.

- Phases: `clone` (optional: `gh repo clone ... --depth 1`), `setup` (`./Setup.sh --force`), `genproj` (`./GenerateProjectFiles.sh`), `buildgraph`, `package` (`tar | zstd`, then the manifest). A log per phase under `WORK/logs/`, each with a start marker and an exit line; a manifest of the source commit; `WORK/state.json` marks finished phases **for one key**, so a second run resumes at the failed phase, a state of another key refuses (`UNREAL_STATE_KEY_MISMATCH`), and `--restart` clears it. Inside the BuildGraph phase the compiler's own incremental build applies; whether an interrupted run resumes cheaply is UNVERIFIED.
- **Disk checks refuse to start** below the phase budget plus a 10 GB reserve, per filesystem (locations on one filesystem add up), and again before every phase; there is no override. **RAM**: refuse below 24 GB (32 recommended); parallel compile actions are bounded by CPUs and by memory (about 1.5 GB per action: 24 CPUs and 29 GB gives 16).
- CODE READ: `Setup.sh` (a) adds `--prompt` unless `--prompt` or `--force` is given, so the tool passes `--force` and never waits on a prompt; (b) installs git hooks into the source's `.git/hooks`; (c) on Debian-family systems runs `sudo apt-get install build-essential` itself if it is missing, so the tool refuses up front (`UNREAL_PREREQ_MISSING`) instead of letting a password prompt appear; (d) downloads the bundled Linux toolchain. Because the build writes into the source tree (hooks, dependencies, intermediates), `--allow-modify-source` is required.
- **Network.** `Setup.sh` downloads the tree's dependency set and the bundled toolchain from **the vendor's content delivery network**; `--clone` uses the user's own `gh` login. Nothing else in the tool opens a socket. The environment of every build step is an allow-list (no credential; only the clone step may carry a token the user exported); git and `gh` never prompt.
- `--reclaim` deletes only `Engine/Intermediate/Build` and `LocalBuilds/InstalledDDC` inside the source after a verified build, refusing a symlinked or escaping path (TESTED).
- `plan` prints the phases, the disk and time estimates and this machine's job count, and runs nothing.

### 7.2 Upload and pin, with any S3-compatible store

Plain S3 semantics only: `PutObject` through the `aws` CLI (or any S3 client), and a **presigned GET** through any S3-compatible API. Nothing in the scripts is specific to one provider; the maintainer's store happens to be R2, which is S3-compatible, as are MinIO, Ceph and AWS.

```
build_installed.py upload --manifest M --artifact A --endpoint https://s3.example.org --bucket my-bucket --prefix my/prefix [--execute]
build_installed.py pin    --manifest M --prefix my/prefix
```

`upload` refuses an artifact that does not match its own manifest, prints the `aws s3 cp --endpoint-url ...` lines, and runs them only with `--execute`; credentials come from the standard `AWS_*` environment variables, never from an argument. The object name is **derived from the key**: `<prefix>/unreal-<version>-<12 hex of the key>/artifact.tar.zst` and `.../manifest.json`, so another source or recipe can never overwrite it.

`pin` prints (1) the deployment's pin entry in the shape of the `objects` list of `ci/vast-broker/propimages.json` (`id`, `role: artifact`, `ref`, `object`, `size`, `sha256`) plus `key`, `loads_software` and `created_utc`; (2) the same for `manifest.json`; (3) a `kayfabe.download-manifest/1` document the box downloader accepts (`sources: [{kind: signed, ref}]`, TESTED against its parser). This is **your** deployment's data: put your key and pin in your configuration; strict and worker runs enforce its size and sha256, standalone runs only record them.

### 7.3 Run your own deployment

1. Obtain a source tree under your own licence (a folder, or a clone through your own login).
2. `build_installed.py build ... --cache-out DIR` on a machine with the disk and RAM of section 8; keep `DIR`.
3. `upload` to **your** bucket and prefix; `pin` and put the entries in **your** deployment's configuration.
4. Presigned GET URLs for the objects reach the boxes through your own broker. In the maintainers' deployment that is the Worker's private route (`GET /bench-data/<run>/private/<prefix>/<object>`, a run token with the private scope, prefixes listed in `private-objects.json`; `docs/design/V3_WORKER_PRIVATE_PRESIGN.md`).

Nothing in the tooling depends on the maintainers' store, bucket, prefix or hashes. The maintainers' own object names and pins live only in their deployment configuration.

### 7.4 What the Worker and payload need (DESIGN, not built)

An optional instruction field `unreal` (`key`, `object`, `size`, `sha256`); an optional payload stage `unreal` (large tier, manual-only, a flag that is off by default, the pattern of `llm_parity` and `install_flow`) that runs the `unreal` row set in the open Linux guest after `apps`; a bandwidth entry for the artifact (class: private pinned object); a `private-objects.json` prefix (the existing `linux-proprietary` prefix is already reserved for the owner's Linux proprietary objects; a deployment may choose another); and the instruction's cache entry feeding `resolve_unreal` in worker mode.

## 8. Phases, disk, RAM, time

ESTIMATES for a Linux x86_64 build of a recent 5.x tree. "Disk" is the **additional** space the phase needs, and the build machine must also keep the 10 GB reserve.

| # | Phase | Where, who | Disk (additional) | RAM | Wall time | Network |
|---|---|---|---|---|---|---|
| 0 | Free disk, install prerequisites | build machine, its owner | frees the amount below | | minutes | |
| 1 | `clone` (shallow) or supply a folder | build machine | 35 to 40 GB (a full-history clone of a large fork: more) | small | 0.2 to 1 h | the code host |
| 2 | `setup` | build machine | 25 GB (dependencies, toolchain archive and unpack) | small | 0.2 to 1.5 h | the vendor's delivery network, about 15 to 20 GB |
| 3 | `genproj` | build machine | 2 GB | 4 GB | minutes | none |
| 4 | `buildgraph` | build machine | 110 GB (intermediates, derived-data staging) plus 45 GB (installed output, can be another filesystem) | 24 GB minimum, 32 recommended; link steps are the peak | 3 to 7 h on 24 CPUs | none |
| 5 | verify and `--reclaim` | build machine | frees about 110 GB | | minutes | none |
| 6 | `package` | build machine | 22 GB (output filesystem) | small | 0.3 to 0.8 h | none |
| 7 | `upload` | build machine | none | small | 0.1 to 0.6 h at 10 to 100 MB/s | your store, about 20 GB up |
| 8 | verify the upload | any machine with 25 GB free | 20 GB transient | small | minutes | about 20 GB down |
| 9 | per release run | box host and open guest | box: 20 GB archive + 30 GB tree; guest scratch 60 GiB sparse | guest 24 GiB or more | download 4 to 11 min, unpack 2 to 6 min | about 20 GB (0.20 USD at 0.01 USD per GB) |

Peak on one filesystem before any reclaim: about 35 + 25 + 2 + 110 + 45 = 217 GB, so a machine needs about 230 GB free, or the installed output and the packaged artifact on a second filesystem (then the source filesystem peaks at about 172 GB plus reserve). A machine with less must free space or add a disk first; the tool refuses to start otherwise.

## 9. Windows, later

The same artifact **kind**, built in a Windows guest (or machine) with the Visual Studio Build Tools, never on a rented box:

- The BuildGraph target is `Make Installed Build Win64` in the same script (CODE READ); the script has an option for the newer Visual Studio toolchain; host platform Win64; the entry points are the `.bat` equivalents (`Setup.bat`, `GenerateProjectFiles.bat`, `RunUAT.bat`).
- The recipe, the key's target (`Win64`) and the toolchain input change; `recipe_rev` therefore differs and the two kinds never collide. `build_installed.py` is Linux-only today (it reads `/proc/meminfo`, runs `.sh` files); a `--platform win64` follow-up supplies the batch-file argument vectors. The state, budget, manifest and key code are shared.
- Disk and time are larger (ESTIMATE 250 to 300 GB of build tree, an installed build of 40 to 60 GB); packaging as zip or `tar.zst` is a choice for the Windows image's own unpacker.
- Delivery into a Windows guest has no virtio-fs by default; the existing app-disk mechanism (a read-only disk or ISO, `scripts/bench/windows/appmatrix/`) is the candidate. The clock rule is the same: the artifact is loaded software.
- New coverage there: ray tracing through DirectX 12 (`ue_rt_automation` and the Lumen row run unchanged in name), mesh shaders for Nanite on D3D12, and whatever the D3D12 RHI does for sampler feedback and async compute (UNVERIFIED which engine paths use them). The Linux rows cover Vulkan only.

## 10. Risks and falsifiers (all UNVERIFIED until a box runs them)

1. **kf3 under an engine that stresses everything.** The first rows may hang the guest or raise host Xids; each row has its own boot and time box, and the discovery lane exists for this.
2. **virtio-fs is not available in the kf3 build** (section 5): falsifier `qemu-system-x86_64 -device help | grep vhost-user-fs-pci`; fallback 9p or the block-device alternative.
3. **Installed build writes into its own folder:** falsifier, a write failure on the read-only share in the first run.
4. **Offline start-up waits** for network time-outs; falsifier, compare online and offline start-up time.
5. **Nanite, Lumen and virtual shadow maps need SM6 on Linux** and **headless rendering needs a display**: the first scene run answers.
6. **The RHI test plugin is not in the installed build:** falsifier, its binaries missing from the unpacked tree; fallback, enable and build it in a tiny project inside the installed build (needs the toolchain it ships).
7. **The key does not capture everything** that changes the output (an untracked file, an environment difference): the dirty check covers tracked files only; the manifest records the observed compiler so a difference is visible.
8. **Edits to the recipe silently change what a deployment fetches:** by design the key changes with the recipe, so an old pin keeps pointing at its old object; the `UNREAL_CACHE_MISS` text says to rebuild and re-pin.

## 11. Decisions the maintainer must take

1. Which paths of the build machine's filesystem may be freed for the 230 GB (or a second disk for the installed output and the artifact).
2. Whether to build `ddc-warm` (default: no, decide after the first scene run has been timed), and whether the packaged sample (phase 2) is wanted.
3. virtio-fs (needs the kf3 QEMU build change and a static `virtiofsd`) or the read-only block device as the delivery path.
4. The guest's RAM and vCPUs for the stage (24 GiB and 8 proposed) and whether the stage may go in the large tier's budget.
5. The object prefix for the cache in the deployment (the reserved `linux-proprietary` prefix, or another).
6. Whether `--unreal-*` flags go into the existing runner's parser (proposed) rather than a new command.

## 12. Tested, and not done

TESTED (RUN 2026-11-17, GPU-free): `scripts/ci/test_unreal_build.py` (136 tests: the key for every input, the manifest, disk groups and boundaries, RAM, the build steps and their order with a fake runner, resume, refusal cases, credentials never reaching a step, the build-or-fetch order and modes, pins enforced only in strict and worker, the plain-S3 upload plan, the clock rule reading the pin, the box downloader accepting the manifest, the rows) and `scripts/ci/unreal_build_mutants.py` (55 mutants, each killed by at least two named tests). Both run in CI.

NOT done: any run of `Setup.sh`, a download, a build, a package, an upload; any guest boot; the recipe and Worker changes of 7.4; the Windows variant; the packaged sample; the audit of required libraries (needs a built tree); every UNVERIFIED item above.
