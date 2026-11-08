# Rebuild the Linux apps image

STATUS: DESIGN-ONLY, 2026-11-07. MEASURED: only GPU-free fixtures have run. A maintainer must build twice, cross-verify and run the apps gate before adopting the image.

**CODE READ:** this path produces an open Ubuntu image and an audited app bundle. NVIDIA proprietary files are excluded. NVIDIA installers and wheels are fetched from pinned official sources on the box host for each provisioning run, supplied on a transient read-only data disk to the offline guest and deleted afterwards. They must never be uploaded to R2 or GitHub, or distributed in an image or ISO. The owner decides whether to host the qualified open layer publicly.

**DESIGN:** use Linux with KVM access, Python 3.12+, QEMU 9.2+ (`qemu-system-x86_64`, `qemu-img`), about 60 GiB free and RAM for a 12 GiB guest. Host root is unnecessary if your account can open `/dev/kvm`. A GPU is unnecessary for building. The repository includes J4's `scripts/ci/kf_fetch.py` downloader. Keep it with the recipe; official provisioning has no alternate downloader.

**CODE READ:** from the repository root, run these individual programs:

```text
python3 scripts/apps/image/build_image.py plan
python3 scripts/apps/image/build_image.py verify-official
python3 scripts/apps/image/build_image.py build --out out --cache apps-image-cache --ssh-authorized-key /path/to/bench-key.pub
python3 scripts/apps/image/build_image.py verify --image out/apps-image.qcow2 --manifest out/apps-image.manifest.json --out out/verification
```

**CODE READ:** supply only a public SSH key for the bench harness, or omit the option for serial-only work. The builder verifies every fetched input's hash/size, builds in a disposable scratch guest, exports only audited open app files to a fresh guest, and scans that guest before publishing `out/apps-image.qcow2`, `out/apps-image.manifest.json` and `out/apps-open-bundle.tar.gz`. Existing outputs are refused. Open direct downloads total about 3.54 GB, plus apt packages and temporary official compiler inputs.

**DESIGN:** build again into a fresh directory, then verify the second image against the **first** manifest. The comparison covers the manifest’s PINNED part: package names/versions, pip freeze with wheel hashes, input URLs and SHA256 pins, source commits/tags and recipe/tooling hashes. BUILT binary, installed-file and image hashes are printed for information and may differ. Missing required apps and PINNED differences fail; each image must independently pass its clean NVIDIA scan and existing transport/integrity checks. Manifest schema 3 keeps both parts separate; schema-2 manifests require rebuilding with the new checkout. Disk timestamps, paths, machine IDs and compiler output bytes need not match.

**CODE READ, 2026-11-08:** recipe `2026-11-08.2` also checks actual bytes of the five copied Python scripts and retained compatibility runner against their local-source pins. Recipe-written files, including the HF model's `refs/main` selector, are collected and checked against their declared content. Changed or missing copies/configuration fail; compiled binaries remain informational. Use a fresh build and manifest from this checkout so both sides include these observed input bytes.

**CODE READ:** test per-run offline provisioning with:

```text
python3 scripts/apps/image/build_image.py provision-check --image out/apps-image.qcow2 --manifest out/apps-image.manifest.json --out out/provision-check --cache apps-image-cache --driver-version 580.159.04
```

**CODE READ:** it installs official NVIDIA inputs in a disposable overlay, verifies its separate runtime inventory and deletes the overlay/downloads. Only metadata remains. An actual gate coordinator uses the Python `provisioned` context while its transient disk exists. A 404 or hash mismatch is a failing `MEDIA_DRIFT`, never a pass. The monthly URL check tests availability; every actual fetch still verifies SHA256.

**CODE READ:** all inputs are public or officially downloadable without sign-in; no private image or credentials are needed. Apt is main/universe only. The two models are Qwen2-0.5B-Instruct and Qwen2.5-1.5B-Instruct Q4_K_M. CPU torch is in the open image; CUDA torch, NVIDIA wheels, Triton and CuPy are runtime-only. CuPy bundles proprietary CUDA headers. The CDP quicksort sample requires static CUDA device runtime and is built only at provisioning. Other CUDA builds request shared linkage and are audited before export.

**DESIGN:** inspect the complete image scan and licences before hosting. Scan results are evidence rather than legal certification. The legacy bash path stays unchanged pending coordinator qualification; successful fixture tests do not qualify a new image for the GPU gate.
