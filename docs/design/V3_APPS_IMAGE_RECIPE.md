STATUS: DESIGN-ONLY, 2026-11-07. CODE READ: implementation and GPU-free fixtures exist. MEASURED: LB1 built or booted no real image. Coordinator builds, cross-verification and the GPU apps gate remain required before adoption or hosting.

# Linux apps image recipe

**DESIGN — owner decision, 2026-11-07:** distribute an **open image only**. NVIDIA proprietary files belong in neither images nor ISOs, R2 nor GitHub. For each provisioning run, the box host downloads pinned official installers and wheels using J4's fetcher, supplies them on a transient read-only data disk to the offline guest, and deletes the runtime overlay and downloads afterwards. The open image could be public in principle; the owner decides hosting after checking the complete result and applicable licences. This recipe grants no redistribution rights.

**CODE READ:** the reproduction claim is **input-pinned + inventory-identical**, not byte-identical qcow2. ext4 timestamps, machine IDs and generated keys can differ. The recorded image SHA256 identifies one output; `verify` accepts another image with the same app inventory. This does not establish reproducibility of the historical shell-built 20.2 GiB image, whose complete transitive inputs were not recorded. Its measured app results remain in [R5](../../traces/apps_linux_20261107/README.md), at that record's revision.

**Notation:** MEASURED means an LB1 test or public-input observation dated 2026-11-07; CODE READ describes implementation; DESIGN means required coordinator work; INFERRED identifies an estimate. No hardware result is inferred from a mock transcript.

## Two layers, three phases

**CODE READ:** [`recipe.json`](../../scripts/apps/image/recipe.json), schema 2 / recipe `2026-11-07.2`, separates `artifacts` from `official_fetch` and `inventory` from `runtime_inventory`:

| Phase | Inputs and output | Retention |
|---|---|---|
| Scratch builder | Official Ubuntu base, main/universe apt packages, pinned sources, transient official driver userspace and CUDA compiler supplied by a read-only data disk; builds the open apps with shared CUDA runtime linkage | Entire scratch guest disk, official data disk and official download directory deleted; only an audited open app bundle crosses to assembly |
| Open assembly | A **fresh** base overlay, open packages/wheels/models, Blender and the audited bundle; no NVIDIA installers or libraries are supplied | After full file/package scan, compressed `apps-image.qcow2`, `apps-image.manifest.json`, `apps-open-bundle.tar.gz` |
| Per-run provisioning | Verify open image first; box host fetches official NVIDIA inputs; offline guest installs them and builds the static-runtime exception | Runtime inventory and verification metadata may remain; NVIDIA downloads, input data disk, seed, receipt and runtime overlay are transient and never uploaded |

**CODE READ:** using a fresh assembly disk avoids promoting deleted NVIDIA files from a builder disk's free blocks. Export checks independently match every regular file against the builder's hashes, audit declared ELF shared dependencies, reject special files, escaping links and CUDA library/header directories, and exclude static-runtime outputs. Assembly must retain every audited export file with the same hash, including upstream licence notices.

**CODE READ:** the open image contains CPU builds of torch/torchvision, transformers and their locked dependencies, llama.cpp, CUDA samples and gpu-burn built outputs, clpeak, vkpeak, hashcat, Blender, and exactly the two small gate models. No restricted or multiverse apt component is allowed. The recipe has 50 direct open inputs (3,541,368,563 bytes), nine frozen apt metadata inputs, 31 open wheels and 52 declared builds, of which 51 belong to the open layer. Apt dependencies are additional downloads, with a 12 GiB unique-download budget and their resolved pins recorded in the manifest.

**MEASURED / CODE READ:** CuPy 13.3.0 is an explicit exception: its unmodified wheel contains proprietary/confidential CUDA headers under `cupy/_core/include/cupy/_cuda/`. It is therefore runtime-only (`requires: nvidia-layer`) in `official_fetch`, **not** an open-image wheel. Triton's wheel also belongs to runtime provisioning because it bundles NVIDIA tooling. These decisions preserve the original payloads and their notices rather than stripping files to pass a scan. The bounded CPU torch and Blender archive audits found no flagged proprietary headers, but are not a complete image scan or legal certification; see [evidence](../../traces/apps_image_recipe_20261107/README.md).

**CODE READ / source inspection:** `cdpSimpleQuicksort` links `cudadevrt`; it is tagged `requires: nvidia-layer-static`, omitted from the open export/inventory, and built only during provisioning. Other nvcc builds specify `--cudart shared` and `--cudadevrt none` where applicable; CUDA-sample make invocations request `cudart=shared`, and llama.cpp sets `GGML_STATIC=OFF` / `CMAKE_CUDA_RUNTIME_LIBRARY=Shared`. Declared ELF dependencies are checked without `readelf`. A real build may reveal additional exceptions or proprietary embedded material: fail and investigate rather than weakening the checks.

## Pins and public sources

**CODE READ:** all direct inputs carry URL, SHA256 and size; official inputs also carry licence and install argv. The complete requested apt list, files to write, guest user, key placeholders, build argv lists and output declarations are data in the recipe.

| Input | Pin in recipe (CODE READ) |
|---|---|
| Ubuntu 24.04 amd64 cloud image | Official `release-20261017`; SHA256 `04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94`; 621,284,352 bytes |
| Apt | Snapshot `20261101T000000Z`; noble, noble-updates, noble-security; **main and universe only**; SHA-pinned InRelease and Packages.xz; signed metadata remains mandatory |
| Kernel | `6.8.0-147-generic`, matching headers `6.8.0-147.147`; periodic upgrades disabled and kernel metapackages held |
| Driver (runtime only) | Official NVIDIA Linux `.run` **580.159.04**, 397,162,548 bytes; SHA256 `c1e66761b088d17b3adf6cb6979de9af38be2684d102b001a176dcfafabdca1e` |
| CUDA (transient builder/runtime only) | Official CUDA 12.6.3 component archives, including nvcc 12.6.85 and cudart 12.6.77; each independently pinned |
| cuDNN / CUDA Python (runtime only) | Official-index NVIDIA wheels plus CUDA torch/torchvision, Triton and CuPy; complete 46-wheel [`requirements-runtime.lock`](../../scripts/apps/image/requirements-runtime.lock) |
| Open Python | CPU torch 2.6.0+cpu / torchvision 0.21.0+cpu, transformers 4.46.3, accelerate 1.1.1, numpy 1.26.4; complete 31-wheel [`requirements.lock`](../../scripts/apps/image/requirements.lock), including pip/build tooling and transitive dependencies |
| CUDA samples | v12.5, commit `9c688d7ff78455ed42e345124d1495aad6bf66de`; pinned source archive |
| llama.cpp | Commit `4b1a27fa0eb875bbca4f6cfe936e3d65adc685c0`; pinned source archive; native CPU tuning disabled |
| gpu-burn / clpeak / vkpeak | gpu-burn commit `0d19c94dcb2e9b6858b7586a4c327674cd67113b`; clpeak 1.1.2 source archive; vkpeak 20250531 official release archive; hashes and sizes in recipe |
| hashcat / Blender | Ubuntu hashcat `6.2.6+ds1-1build2` and data deb; official Blender 4.5.0 archive; each independently hashed |
| Project sources | Every authored C/CUDA/Python input is content-pinned; local drift fails before downloading |

**CODE READ:** exactly these two model payloads are included:

| Model | Exact files |
|---|---|
| Qwen/Qwen2-0.5B-Instruct | Revision `c540970f9e29518b1d8f06ab8b24cba66ad77b6d`: model.safetensors, config.json, generation_config.json, tokenizer.json, tokenizer_config.json, merges.txt, vocab.json and LICENSE; each file pinned |
| Qwen/Qwen2.5-1.5B-Instruct-GGUF | Revision `91cad51170dc346986eccefdc2dd33a9da36ead9`: **only** qwen2.5-1.5b-instruct-q4_k_m.gguf (1,117,320,736 bytes; SHA256 `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e`) and LICENSE |

**CODE READ:** `refs/main` is written as data pointing at the fixed HF revision; no moving `resolve/main` download occurs. Larger models and Geekbench are absent. All builds currently declare output hashes as `inventory-only`: the first output is measured and later compared, not assigned an invented pre-build hash.

**MEASURED:** the official driver installer was streamed and hashed against NVIDIA's published checksum. Source archives, small model metadata files, the cudart archive, CPU torch, Blender and CuPy were also downloaded and hashed. Other large pins derive from upstream checksum/index metadata; LB1 did not hash every large payload. Availability and audit records distinguish HEAD observations from full digest verification. A URL answering HEAD does not prove its bytes still match.

## Transport, installation and scan boundary

**CODE READ:** host code uses Python 3 standard-library networking, hashing, HTTP serving and file operations. Its **only external host programs are `qemu-system-x86_64` and `qemu-img`**. No host ssh/scp, curl, shell, mount, mkisofs or compiler is invoked. The deterministic ISO9660/Joliet writer generates a NoCloud `cidata` seed directly; it contains configuration/code only, never NVIDIA payloads. A separate temporary raw data disk carries unchanged official installer bytes behind a bounded, sector-aligned index. The guest reads it directly with Python; no filesystem, host mount or extra binary is needed. Both sides verify payload hashes/sizes, and QEMU attaches it read-only under a fixed virtio serial identity. Cloud-init receives JSON `packages`, `apt`, `users` and `write_files`, plus a Python executable in its per-instance directory; no `runcmd`/`bootcmd` shell renderer is used. Cloud-init 23.4+ supports the supplied deb822 sources format ([apt configuration](https://docs.cloud-init.io/en/latest/reference/yaml_examples/apt.html)); the pinned base manifest reports cloud-init 26.1. Upstream mechanisms: [NoCloud](https://docs.cloud-init.io/en/latest/reference/datasources/nocloud.html), [Ubuntu snapshots](https://snapshot.ubuntu.com/), [NVIDIA redistribution manifests](https://docs.nvidia.com/cuda/cuda-installation-guide-linux/index.html#parsing-redistrib-json).

**CODE READ:** guest Python invokes apt/dpkg, pip, nvcc, gcc, make, CMake/Ninja and system utilities with argv lists. Official installers, distro maintainer scripts and upstream makefiles may themselves use a shell. The promise removes reliance on project-authored shell provisioning; it does not replace vendor installers or Ubuntu internals. The unchanged legacy app runner is copied as inert compatibility data, not executed by this builder.

**CODE READ:** open direct inputs use verified urllib downloads. Official inputs and apt package downloads use the J4 `scripts/ci/kf_fetch.py` API on the host, followed by independent size/SHA256 verification. There is **no fallback downloader** if J4 is missing. J4 is integrated by merging `next` after PR #298. **MEASURED:** the actual adapter fetched and verified the 1,126,072-byte official cudart archive with this API; its temporary payload directory was deleted. [Adapter observation](../../traces/apps_image_recipe_20261107/j4_adapter_fetch.json). This checks acquisition, not guest installation.

**CODE READ:** every guest mode uses `restrict=on`; install modes add one explicit `guestfwd` from `10.0.2.100` to the loopback **open-input/apt** HTTP server. Official NVIDIA hashes are excluded from that server; those bytes can only arrive from the read-only per-run data disk. Verification has no forward. HTTP responses have explicit lengths and preserve the forwarding chardev stream across sequential requests. This follows [QEMU's explicit-forward exception to restricted networking](https://www.qemu.org/docs/master/system/qemu-manpage.html). Real transport qualification still requires a coordinator boot. The host HTTP server serves only exact open-input hashes and apt paths admitted by the pinned main/universe indexes. Arbitrary URLs, restricted/multiverse pool paths and unbounded package downloads are refused. Apt retains signature verification against the Ubuntu keyring; only frozen metadata expiry is disabled. The guest never contacts vendor sites. A 404, non-success response, unexpected length or SHA256 mismatch is a loud `MEDIA_DRIFT` failure; no automatic mirror, pin update or passing receipt is substituted.

**CODE READ:** open assembly and open verification scan regular-file contents in full, package names/origins, vendor payload paths and symlinks. Only pseudo filesystems `/proc`, `/sys`, `/dev`, `/run` are skipped. Scan errors or byte/file budget exhaustion fail. Proprietary/EULA NVIDIA headers and unknown NVIDIA licence notices fail; documented OSS copyright notices are retained and listed with licence evidence. This is a deliberately conservative evidence check, not proof that arbitrary binary code has no proprietary origin or permission to host it. The manifest declares `no_nvidia_proprietary_files: true` only after a clean scan; there is no blanket NVIDIA-copyright waiver.

**MEASURED / CODE READ:** a full audit of the pinned Ubuntu `linux-headers-6.8.0-147` main package initially identified 14 classifier findings: GPL NVIDIA platform-source names and the standard Linux SPDX expressions `GPL-2.0 WITH Linux-syscall-note` / `GPL-2.0 OR Linux-OpenIB`. The corrected scan has zero findings and retains 63 copyright notices ([audit](../../traces/apps_image_recipe_20261107/kernel_headers_audit.json)). Source namespaces are derived from installed header package names; there is no directory exclusion from content scanning. Proprietary headers, vendor libraries and undocumented NVIDIA notices still fail inside those source paths. Upstream definitions: [kernel licensing rules](https://docs.kernel.org/process/license-rules.html), [Linux syscall note](https://spdx.org/licenses/Linux-syscall-note.html), [Linux-OpenIB](https://spdx.org/licenses/Linux-OpenIB.html).

**CODE READ:** each layer's inventory includes dpkg installed versions/architectures; `pip freeze --all`, wheel hashes and independently computed installed-distribution digests; app/model/library hashes and symlink targets; and guest kernel/module versions. Open inventory includes clean scan findings/notices; runtime inventory includes actual official install receipts and the measured NVIDIA module version. pip-generated bytecode and RECORD, machine identities, DKMS output bytes and unrelated filesystem/log content are outside app identity. Full scans still run each open boot; scan file/byte counters describe work, not reproducibility of growing cloud-init logs.

**CODE READ:** completion requires a fresh nonce, mode, recipe digest, complete inventory, matching serial digest marker and zero QEMU exit after shutdown. Serial/inventory/export/stdout channels have byte caps. Timeouts and interrupts reap the owned QEMU process and remove transient directories; bounded failure diagnostics remain. Compression and `qemu-img check` must succeed before publishing a manifest. A partial run cannot produce a success manifest.

## Reproduction and verification

**DESIGN:** use a Linux coordinator with Python 3.12+, QEMU 9.2+, KVM access, about **60 GiB free** and memory for a 12 GiB guest. INFERRED: 16 GiB host RAM is a practical minimum. Host root is unnecessary if the account can open `/dev/kvm`; cloud-init runs the guest installers as root. A GPU is unnecessary for building, but is needed later for the apps gate. Provisioning additionally checks room for both official download bytes and their transient data-disk copy, plus 12 GiB for scratch work. QEMU 9.2+ is required for the forwarding socket's `reconnect-ms` option ([QAPI version annotation](https://github.com/qemu/qemu/blob/v9.2.0/qapi/char.json)); LB1 queried QEMU 10.2.1's version only. The 60 GiB build requirement remains an estimate pending a real build. Do not build on the shared development box.

**CODE READ:** the guest is `ubuntu`, uid 1000, with caller-supplied public SSH keys; serial building/verification needs no key. DHCP supports QEMU user networking, and the declared secondary `192.168.77.2/24` address preserves the legacy tap layout. Run these individual programs from the repository root (no shell wrapper or shell syntax required):

```text
python3 scripts/apps/image/build_image.py plan
python3 scripts/apps/image/build_image.py verify-official
python3 scripts/apps/image/build_image.py build --out out --cache apps-image-cache --ssh-authorized-key /path/to/bench-key.pub
python3 scripts/apps/image/build_image.py verify --image out/apps-image.qcow2 --manifest out/apps-image.manifest.json --out out/verification
```

**CODE READ:** `inputs` optionally downloads **open inputs only**. `build` also needs transient official compiler inputs on that coordinator; it never persists them in the open cache/output. Existing image, manifest or bundle outputs are refused. `verify` boots a child overlay, compares the correct layer's inventory and checks the supplied input image was unchanged. Output: `apps-image.verify.json` only on success.

**DESIGN:** build a second image in a fresh output directory, then verify it **against the first build's manifest**. Verifying each image against its own manifest does not establish reproducibility. Different qcow2 hashes are permitted and reported. Inventory drift must be investigated, not fixed by recording a new passing baseline.

**CODE READ:** exercise offline per-run provisioning and a second runtime-inventory boot with:

```text
python3 scripts/apps/image/build_image.py provision-check --image out/apps-image.qcow2 --manifest out/apps-image.manifest.json --out out/provision-check --cache apps-image-cache --driver-version 580.159.04
```

**CODE READ:** this command deletes its NVIDIA overlay/installers/input data disk even on failure; it retains only verification/inventory metadata. For an actual coordinated app run, import `build_image.provisioned(...)` as a Python context manager and run the gate while its yielded disk/manifest exist. Its transient `.STAGED` receipt uses the measured module version. Only driver versions with official pins in `driver_versions` are accepted; the release matrix chooses the version. Additional sweep versions require reviewed official pins, not a moving download.

**CODE READ:** a person without R2 can use these steps: every input is public or officially downloadable without sign-in. No private prebuilt bundle, Worker, Cloudflare or R2 credential is required. No NVIDIA payload is fetched from or registered in R2/GitHub. The monthly `apps-official-inputs.yml` workflow HEAD-checks **every** official URL and fails on drift; HEAD does not rehash all payloads. DESIGN: its schedule becomes operational only after reaching the repository's default branch.

## Coordinator handoff

**DESIGN:** build twice and cross-verify, inspect the complete scan and inventory, verify normal subsequent boot and runtime driver installation, then run the existing apps gate and same-binary host reference. The open bundle gives the coordinator the guest-built binaries; host runtime libraries still need official provisioning. `apps_hook.sh` overwrites guest bin files from that host bundle, so the coordinator must ensure it uses this audited bundle and supply the runtime-only CDP output consistently. Gate adapter integration is not exercised here.

**CODE READ:** all old bash paths are **untouched**, following the owner's superseding instruction. No upload, rental, VM, GPU run or deployment occurred in LB1. This implementation does not approve the historical image for upload. The open candidate and manifest need measured coordinator qualification; NVIDIA runtime overlays never become distribution artifacts. GPU-free tests and two-killer mutation results are recorded in [LB1 evidence](../../traces/apps_image_recipe_20261107/README.md).
