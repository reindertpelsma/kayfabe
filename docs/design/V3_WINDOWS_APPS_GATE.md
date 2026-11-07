**STATUS: DESIGN-ONLY, 2026-11-07** for the Worker and release-tier parts (nothing is deployed, no R2 object exists, no
Worker setting sends the stage). **LIVE for the harness code** (merged with this document, GPU-free tests), and the
**first hardware run** is in section 8 (`traces/winapps_20261107/`). Draft PR against `next`; the owner reviews. Parent
documents: [`V3_WINDOWS_APP_MATRIX.md`](V3_WINDOWS_APP_MATRIX.md) (the inventory, the guest protocol and the verdict
vocabulary; unchanged), [`V3_WINDOWS_GATE.md`](V3_WINDOWS_GATE.md) (the gate this stage follows) and
[`../DOWNLOADS.md`](../DOWNLOADS.md) ("Windows app bundle").

# V3 Windows apps gate: the Windows app matrix as a stage of the Vast gate

Marking in this document: **CODE READ** = repository inspection; **MEASURED** = recorded observations such as the
2026-11-07 `wa1`/`wa1f` run in section 8 (with revision, box and die); **INFERRED** = reasoned, not seen;
**DESIGN** = intent.

**CODE READ, J1 integration, 2026-11-07:** the successor to draft PR #286 merges `next` (including Linux apps,
run-report support and evidence relocation). `vast_run_report.py` consumes the stage's `WINAPPS_APP` output as
Windows `apps[]` and `apps_windows` test rows, using the Windows subrun. Its suite rows still come from the
payload result, including `windows_apps_retry_*`; an apps failure cannot be hidden by a preceding gate PASS.
Missing/harness rows map to NOT_RUN (test SKIP), never PASS. `KF_WINAPPS` absent still selects no Windows apps stage.
The successful payload suite counts the checked apps (rather than one stage): it requires a nonzero matching
`WINAPPS_PASS apps=N`, grade summary, and N distinct passing per-app rows. Truncated or inconsistent output
is `windows_apps_retry_operator`.

## 1. The problem

Owner requirement (2026-10-10, restated 2026-11-07): the release gate runs, in the Windows 11 LTSC guest, the Windows
version of the roughly 70-app matrix (the apps that worked before). **CODE READ at `b11655b7`:**

- the Worker payload (`scripts/ci/vast_payload.py`) had no apps stage: after `windows_gate` only `windows_gate_2` and
  `specviewperf` existed;
- the Windows matrix driver (`scripts/bench/windows/appmatrix/winapps.py`) could only launch guests through
  `windows_broker_prod2.sh`, a host script of the trusted host that is no longer reachable (`172.22.1.20` timed out on
  2026-11-07), and talked to the guest through the QEMU guest agent, which the gate's guest image does not give us (see
  the Win11 diagnostics note: the agent MSI rolls back; the gate uses ssh);
- the app disk (`kfapps.iso`, 6.8 GB, 44 sha256-pinned downloads) lived only on that host; a rented box must not fetch
  from a vendor, and the pinned ffmpeg URL answers 404 today (**MEASURED** 2026-11-07: `appdisk.py fetch` from this
  machine, `FETCH_DONE failures=1`, `ffmpeg-n8.1.3-14-g330caae0c1-win64-gpl-8.1.zip: http 404`).

## 2. What can be automated offline, and what cannot

`scripts/bench/windows/appmatrix/gate_profiles.json` is the one source; `gate_profiles.py check` and
`tests/test_gate_profiles.py` keep it consistent with `apps.json` and `manifest.json`. **CODE READ / generated counts, J1 definitions 2026-11-07:** 120
apps, in three classes.

| Class | Apps | What decides | Goes in an automated profile? |
|---|---:|---|---|
| open | 87 | every package the app needs is redistributable and in the bundle (`open_packages`); source/reference assets are additional prerequisites | yes, with prerequisites |
| owner decision | 29 | needs GravityMark, FurMark, GpuTest, the Vulkan SDK or the ffmpeg GPL build: free to download, storage rights still pending | no, until the owner rules |
| manual | 4 (+3 non-matrix items) | Unigine Heaven and Valley (paid automation, evaluation EULA); 3DMark, Geekbench 6, TensorRT, Superposition, Xonotic, Q2RTX | never |

Open apps by category (**generated, J1 definitions**): CUDA equivalents 28, CUDA demo 7, stream probes 6, D3D probes/samples 16, CUDA ladder 3, torch 4,
LLM 4, OpenCL 3, probes 3, browser 5 (Edge WebGL/WebGPU, local pages), Vulkan 2 (vulkaninfo, vkpeak), OpenGL 1 (glgears),
and one each of render (Blender), crypto (hashcat), AI (HF generate), SMI, stress (torch burn).

The guest is **offline** for every app: QEMU user networking with `restrict=on` (`win_vm.sh`), only the harness's ssh forward;
apps and models come from the app disk, Edge opens pages from `C:\kf\web`. **CODE READ.**

**Manual list and evidence template** (`gate_profiles.json` `manual`): a person installs the app, runs it by hand in the
gate guest and commits `traces/winapps_manual_<date>/<app>.md` with the revision, die, driver, the app's own result and
the host reference. The licence or key is never written down.

## 3. Inputs: one bundle, from our R2, through the Worker

**CODE READ, WB1, 2026-11-07:** `scripts/bench/windows/appmatrix/package_bundle.py` replaces the
shell packaging recipe. It uses Python 3.11 or later, standard library only, and the official pinned Zig 0.13.0
release. It runs unchanged on x86_64/aarch64 Linux, Windows and macOS by design; the WB1 report below distinguishes
local measurements from portability intent. Run the following from a checkout of the reviewed revision:

```text
python3 -I scripts/bench/windows/appmatrix/package_bundle.py check
python3 -I scripts/bench/windows/appmatrix/package_bundle.py layout
python3 -I scripts/bench/windows/appmatrix/package_bundle.py fetch --dl downloads
python3 -I scripts/bench/windows/appmatrix/package_bundle.py verify --dl downloads
python3 -I scripts/bench/windows/appmatrix/package_bundle.py build --dl downloads --out out
python3 -I scripts/bench/windows/appmatrix/package_bundle.py verify-image --out out
```

**CODE READ:** `fetch` uses urllib, retries the same URL, resumes a `.part` file with validated HTTP ranges,
and checks the manifest's byte length and SHA-256 before promotion. A mismatching existing cache entry fails;
it is never silently repinned. Every failure names the package and input. The owner refinement below permits explicit credential-free CI with `--ci-build` and permits the cheap
`input-hash` command in CI. Default local commands still refuse `CI`; `KF_RUN_TOKEN` and upload credentials
remain forbidden in every builder invocation. The independent allow-list and dependency closure refuse packages outside
`gate_profiles.json` `open_packages`; no owner-decision or licensed package is selected. `--only` permits a
smaller open subset for local investigation; the coordinator's release bundle uses the full default selection.

**CODE READ:** application ZIPs, wheels, 7z archives, MSIs and installers ride on the ISO byte for byte.
The guest's existing `tar.exe`, `msiexec.exe`, installer modes and bundled `7zr.exe` do the staging. The only
staging change is that `kf_stage.ps1` prefers the ISO's `stage.json` over the controller's legacy table. This
prevents a legacy `image_form=tree` hint from skipping guest extraction. Python's `._pth` adjustment and wheel
extraction already ran inside the guest and remain there. All app definitions and verdict rules are unchanged.

**CODE READ:** the compiler's own release archive is the sole host extraction: Python unpacks a hash-verified
Zig distribution into a fresh directory, rejecting links, special files and escaping paths. It is never put on
the ISO. Zig's bundled headers/runtime replace the distro mingw package; no host compiler, shell, curl, wget,
unzip, 7z, ISO utility or msiexec is invoked. Direct argument lists build `kf_dxprobe`, `kf_overlayprobe`,
`kf_glgears`, `kf_cuda_control` and `cup2/3/8` from the same repository sources as `build_tools.sh`.
Zig 0.13.0 lacks the feature-level 12_2 enum; the build supplies Microsoft's documented `0xc200` value,
also used by `windows_gate.sh`. Stripped baseline x86_64 builds use normalized source paths and a fixed source
epoch; the builder refuses nonzero PE timestamps, build-directory strings and external compiler runtime DLLs.
The source hashes, compiler pins and resulting binary hashes are recorded in the receipt. Repository licence
texts and the Zig-bundled C/C++ runtime notices are retained on the ISO.

**CODE READ:** `bundle_iso.py` writes ISO9660 plus Joliet, label `KFAPPS`, without an ISO utility. Both
namespaces share file extents. Entry order, short aliases, all timestamps (2000-01-01 UTC), padding and volume
fields are fixed; host ownership, mode, mtime, umask, locale and timezone never enter the image. Repository text
is normalized to LF, covering CRLF checkouts. Individual files over 4 GiB minus one byte are refused before
writing; the selected pins fit, including the 2.9 GB torch wheel. Data is streamed, including the readback check.
The image excludes wall-clock/Python-version metadata; the external receipt includes these audit fields.

**CODE READ:** outputs are `out/winapps-open.iso`, `out/winapps-open.manifest.json` (every ISO file's SHA-256,
size and `licenses` records, payload identity, image hash/size, pins, source mirrors, build source hashes, date and Python version) and
`out/bundle.pin.line`. The identity hashes sorted payload records excluding `KFAPPS.ID` to avoid a circular
hash; that file is itself included in the receipt. `verify-image` hashes the entire ISO and independently reads
its Joliet directory records to check exact file membership, bytes and the identity marker. No mount is needed.

**SUPERSEDED for CI by owner refinement below:** the original person-only restriction is lifted for the
credential-free GitHub workflow. A full build still must not run on this lane's dev box. Operator validation
may use a person's machine with sufficient disk. Build into two fresh output directories, then check the second image
against the first receipt to compare the ISO bytes (receipt dates and Python versions may differ):

```text
python3 -I scripts/bench/windows/appmatrix/package_bundle.py build --dl downloads --out out-a
python3 -I scripts/bench/windows/appmatrix/package_bundle.py build --dl downloads --out out-b
python3 -I scripts/bench/windows/appmatrix/package_bundle.py verify-image --iso out-b/winapps-open.iso --receipt out-a/winapps-open.manifest.json
```

**MEASURED status and falsifiers:** WB1's GPU-free test suite builds small fixtures in two empty directories
under different umasks, timezones and locales and compares bytes; optional `isoinfo`/`xorriso` tests read the
same fixtures when available. A divergent image, changed installer byte, extra/missing file, bad identity or
non-reproducible PE is a failure. Full installer downloads, cross-host compiler equality, Windows filesystem
mounting and GPU equivalence to the mingw binaries require operator evidence; no fixture test establishes them.
The functional comparison must run both compiler sets through the existing unchanged app-matrix oracles on the
same native Windows machine/driver, including D3D11/12, WGL, cup2/3/8 and every overlay transition. It cannot be
claimed from PE inspection alone. The shell wrapper is a deprecated pointer; optional source-built release
assets still use the separate legacy `appdisk.py` workflow and are not added to this canonical open bundle.

**MEASURED, WB1 builder v1 at `8ddffcd6d`, local x86_64 Linux, 2026-11-07:** two fresh Zig builds produced byte-identical copies of all
seven probes, with zero PE timestamps and no build-path strings. The second build used CRLF source copies and
a build path containing spaces. Two final fixture ISOs (real probes and notices, one synthetic installer;
**not the full pinned media**) under umasks 022/077, TZ Pacific/Honolulu/Asia/Tokyo and locales C/C.UTF-8 were
byte-identical: 1,122,304 bytes, 47 files, SHA-256
`9a3688db4b0508086646379f960b2c94cf169377ecac6c0b667c5471063d530a`.
This is local reproducibility evidence, not a Windows GPU or cross-host compiler result.

**DESIGN:** a rented box gets one input, `winapps-open.iso`, through the Worker's proposed presigned
`/bench-data` route (`fetch_bench_data.sh`, run token only), pinned by SHA-256 in `appmatrix/bundle.pin`.
The coordinator uploads the ISO and receipt only after review and full reproduction. This builder has no upload
operation. Worker names remain proposed, no deployment or pin change is part of WB1. Without a pin the stage
answers `RETRY class=MEDIA`. `docs/DOWNLOADS.md` gives the manual file-manager recipe and storage exclusions.

### 3.1 Owner refinement: build in GitHub, publish immutable objects

**CODE READ:** the earlier fixture hash above describes builder v1. The embedded `inputs` field and v3
compliance payload change the image identity; that earlier fixture hash is not a new release-media pin.

**CODE READ / DESIGN, 2026-11-07:** the owner changed the packaging location to GitHub CI and permits a final
master-only upload. `.github/workflows/bundle-publish.yml` implements the structure but remains **disabled for
automatic execution**: its only trigger is `workflow_dispatch`; no push, PR or monthly schedule is enabled.
No environment, repository variable, secret, credential or Worker route is configured by this PR. The first
manual `mode=build` run uses the job with no environment or secret references. Publication additionally
requires `vars.BUNDLE_PUBLISH_ENABLED == 'true'`, default absent/false.

**CODE READ:** `package_bundle.py input-hash` prints exactly `inputs=<sha256>`. The hash covers the builder
version and normalized builder/ISO/compiler/compliance recipe source, the selected open-package records and pins, sorted
`open_packages`, every ISO-carried guest/app/licence text, all probe/ladder/header/import-library sources,
and all official toolchain pins. Compiler-provided notices are covered by their archive pins. Host paths,
Python version, packaging dates, historical image receipts, publisher/CI helper code and research/lint-only
metadata are excluded. `inputs` is embedded in the ISO and receipt. Changing a determinant changes the hash;
merely reformatting the source manifest or changing historical image metadata does not. The publisher verifies
the receipt against the current checkout and reads the image back before sending any bytes.

**CODE READ:** `bundle_ci.py plan` compares the input hash with `# inputs=` in the committed publication pin.
An equal hash means `needs_build=false`, and neither large job runs. Missing input metadata is conservatively
a build. This is a credential-free comparison with the repo-selected publication; it never fetches R2.
`mode=verify` deliberately rebuilds even when equal, requires a pin, compares with its ISO hash and cannot
upload. A future optional monthly trigger must use that same mode. No mutable `latest` object exists.

**CODE READ:** the protected combined job builds and publishes on the same runner. It creates no Actions
artifact, Actions cache, release asset or intermediate transfer. Both build paths fetch only `open_packages`,
execute only the pinned/hash-verified Zig distribution as vendor code, build twice in fresh output directories
with different umasks/TZ/locales, and require matching ISO hashes plus cross-receipt readback. PR mode reports a
new hash when it differs from the pin; verify mode fails on a difference. Upload credentials are rejected by
the builder and are referenced only in the final publish step's `env:`. The combined job itself requires the
canonical repository, `refs/heads/master`, push/dispatch, the enable flag and protected environment
`bundle-publish-master`. **Deployment branches must be restricted to master and reviewers configured there
before enabling.** Fork PRs and every PR event are refused again by the publisher's own entry point.

**COMPUTED from committed pins, not measured on GitHub:** originals total 6,743,509,381 bytes and additional
licence/source inputs total 91,613,555 bytes (shared pins are counted conservatively per package). The admission
budget is `3 * originals + 4 * compliance inputs + 2 * package count * 64 MiB + 2 GiB` =
25,294,602,843 bytes (23.56 GiB), including both images, legal-text caps, source upload staging and compiler
scratch. The workflow measures `df` before/after removing unused preinstalled Android, .NET, GHC,
Boost and hosted tool caches, then checks actual free bytes **before any vendor download**. It uses system
`/usr/bin/python3` so deleting the hosted cache cannot remove its interpreter. Default free space around 14 GB
(the coordinator's sizing premise) is insufficient; whether cleanup frees enough is **unverified until a manual
run**. If not, a larger runner is required; neither the disk gate nor the second build is bypassed. Both jobs
remove only their own ephemeral workspace on success/failure. Logs are public; app archives/credentials and
presigned URLs are never printed. Only redistributable packages are eligible for these jobs.

**CODE READ:** `publish_bundle.py` uses standard-library HTTPS, SigV4 (`auto` region), bounded 64 MiB parts,
service ETags, per-part integrity checks, remote ListParts validation and local resume state. It supports files
above 5 GiB without aws/rclone. Resume state contains upload IDs/part digests, never credentials or URLs. The
credentials are environment secrets scoped to **one dedicated bundle bucket**, loaded only by the final
uploader process. A caller-presigned multipart plan is also accepted through `BUNDLE_UPLOAD_URLS`; capabilities
remain in memory and final URLs must bind `If-None-Match` in their signature.

**CODE READ / service contract:** keys are exactly `winapps-open/kfapps-<ISO sha256>.iso` and
`winapps-open/manifest-<ISO sha256>.json`. Existing matching objects are reused; conflicting metadata/receipts
fail. Both single PUT and multipart completion require `If-None-Match: *`; a concurrent winner is checked and
only this run's pending multipart is aborted. HTTP 200 containing an XML Error is a failure. No existing key
is deleted, overwritten or retried without its create-only condition. **R2 conditional multipart-completion
semantics must be verified by the coordinator before enabling; no live service check occurred in WB1.**
Configure an incomplete-multipart expiry lifecycle for interrupted/lost runners. Transport failure preserves
local state for up to three uploader invocations in the same final step; a fresh runner does not receive state or media from Actions artifacts.

**DESIGN: enabling automatic triggers after review.** Add input-filtered `pull_request` for the no-environment
build job, and `push` restricted to `master` for planning, using only these determinant paths: root `LICENSE`,
`LICENSE-APACHE`, `LICENSE-GPL`, `THIRD_PARTY_NOTICES.md`; appmatrix `manifest.json`, `gate_profiles.json`,
`apps.json`, `package_bundle.py`, `bundle_iso.py`, `bundle_tools.py`, `bundle_compliance.py`, `guest/**`, `tools/src/**`;
`scripts/bench/windows/cuda/nvcuda.def`, `scripts/bench/cuda_min/cuda.h`, `scripts/bench/cup3.c`,
`scripts/bench/cup8.c`, `archive/nvkvm/tests/mode2/cup2.c`. Pin/output/docs-only commits do not trigger a build.
The explicit event guards already distinguish the jobs. Do not add `pull_request_target`. This PR does not
turn these triggers on or provision secrets.

**DESIGN: optional OIDC presigner route, no Worker code change.** A future upload broker would validate GitHub's
JWT issuer/audience, canonical repository, master ref, reviewed workflow identity and protected-environment
subject, run identity and expiry. It would authorize only the two ISO-derived keys and source-digest keys
listed in the reviewed receipt, expected input hash,
size and bounded multipart plan; sign the create-only condition; and return short-lived method-specific URLs
for HEAD/GET, parts, completion and abort. Static R2 credentials stay at the broker. It must reject fork/PR/run
credentials and arbitrary keys, expire unused uploads and bind retries to the same run. Current workflow
permissions are only `contents: read`; no OIDC permission or route call is installed. GitHub `id-token: write`
is job-scoped, so adding it to the combined job would grant earlier build steps token-minting capability too;
that isolation tradeoff needs owner review before adopting this alternative.

**DESIGN: successful publication then pin PR.** The uploader emits `published.pin.line` and the workflow puts its
non-secret input hash/digest/keys in the job summary. A separate trusted automation should verify the published
receipt, ensure its input hash still equals the current reviewed master tree, and open a **draft PR against
`next`** updating only `appmatrix/bundle.pin`. Its first line remains the existing consumer-compatible
`winapps-open.iso sha256=...`; comment lines record inputs and both immutable keys. Reviewer approval and merge
select the new object; publication alone never advances the pin. No PR-token handling is implemented here.
The Worker's current fixed-key read mapping does not consume these new keys yet: before enabling consumer
use, a separate reviewed read-route/helper change must map the committed digest to the immutable object.

**DESIGN / migration:** this replaces the old GitHub-Releases bundle store
`reindertpelsma/win-gpu-image-private-store`. That repository's other contents, including the setup cache and
WinPE image, are a separate migration. No contents are moved or deleted by WB1.

### 3.2 Licence inventory, distribution classes and source availability

STATUS: CODE + DESIGN, 2026-11-07 (WB1 owner addition). Engineering checks and small fixtures exist;
the full archive inventory and complete transitive Corresponding Source have not been audited here.
Publication remains disabled. This section does not establish legal clearance for a public release.

**CODE READ:** every build, including private builds, carries `LICENSES/<package>/<text sha256>`,
`LICENSES/index.json` and generated `THIRD_PARTY_NOTICES.txt`. The index preserves original legal-text
paths, upstream URLs, parent archive SHA-256, covered input and each text's SHA-256. Legal members of pinned
ZIPs/wheels are read into memory using `zipfile`; application payloads are never extracted on the host.
7-Zip legal texts are read from its exact pinned source tar using `tarfile`. Neither path executes vendor
code. Whole-archive hashes transitively pin the contained texts; separate upstream licence downloads have
their own URL/hash/size pins. Missing coverage for any binary input, an archive with no legal text and no
pinned fallback, an invalid source pin or a source/binary version mismatch fails the build.

**CODE READ:** notices list package, exact version, licence identifier, upstream project, conditions,
exact wheel versions/identifiers, and source locations. Every ISO file in the external manifest has a
machine-readable `licenses` list with package/component, version, identifier, upstream, `distribution`,
conditions and mirrored source pins. Its metadata participates in the payload identity. Repository licence
texts are in `LICENSES/kayfabe`; exact Zig/runtime notices are in `LICENSES/zig`.

**CODE READ, engineering policy:** `compliance.distribution` in `manifest.json` controls this table.
This is engineering support, not legal advice.

| Packages | Class | Required handling |
|---|---|---|
| python_embed, llama_vulkan, qwen_gguf, vkpeak, hashcat, hf_model | `public-ok` | Retain pinned upstream/as-shipped licences and notices; inspect bundled dependency inventory before release. |
| Blender 4.5.0, 7-Zip 26.04, clpeak_ocl 3.0.1 | `public-ok` | Unmodified upstream object code; exact source archives on ISO and mirrored beside it; retain notices and honor the source offer. Complete transitive source coverage still needs operator review. |
| vulkan_rt | `conditions` | Retain VulkanRT-License.txt and copyrights; comply with the pinned LunarG Runtime redistribution terms. |
| vc_redist | `conditions` | Comply with Microsoft terms for permitted redistribution, unmodified distributable code, notices and end-user restrictions; confirm applicability to the exact installer. |
| hf_wheels | `conditions` | Includes MPL-2.0 certifi; source is not mirrored yet. Review its exact source/retention requirements and bundled notices before public distribution. |
| cuda_demo_suite, cuda_cudart, cuda_nvrtc, pywheels | `private-only` | NVIDIA CUDA EULA components, including CUDA libraries inside torch wheels, require qualified licence review before public distribution. |
| llama_cuda, clpeak_cuda, cuda_cccl | `private-only` | Conservatively apply the same rule to their CUDA runtime/EULA material; an open-source wrapper does not clear it. |

**CODE READ:** `--public-only` refuses the entire requested selection if any selected package or dependency
is outside `public-ok`; it never silently omits one. Consequently the default complete open selection is
refused in this mode. For a limited reproduction, for example:

```text
python3 -I scripts/bench/windows/appmatrix/package_bundle.py check --only blender,sevenzip,clpeak_ocl --public-only
```

Use the same flags for fetch, verify and build. This subset is diagnostic media and cannot pass the full
open-app gate or the canonical full-selection publisher. `verify-image --public-only` also checks receipt
classes. A class records engineering policy, not a conclusion that every embedded component has been reviewed.

**MEASURED, 2026-11-07:** the existing 7zr.exe SHA-256 matches the official 26.04 release byte-for-byte;
its mutable download URL is replaced by that release's immutable URL. The original Visual C++ installer hash
still matches; its PE FileVersion and ProductVersion are `14.44.35211.0` (read without execution). The pinned
clpeak 3.0.1 upstream LICENSE contains GPLv3, contradicting the earlier Apache-2.0 label; both backend records
are corrected and share its pinned source archive. No app definition or verdict rule changes.

**MEASURED source downloads, 2026-11-07:** official Blender 4.5.0 source (85,059,376 bytes), official 7-Zip
26.04 source (1,560,440 bytes), and upstream clpeak 3.0.1 source (2,335,991 bytes) were downloaded and SHA-256
pinned in `compliance.sources`. Their URL, exact version, size, digest and retrieval date are in the manifest.
They are copied unmodified into `SOURCE/<package>/<archive name>` on the ISO. This measures archive identity,
not completeness of third-party dependency source or GPU functionality.

**CODE READ / DESIGN:** the final uploader streams each source archive from the verified ISO and publishes
it **before** the ISO/manifest under `winapps-open/source-<source sha256>.archive`, in the same dedicated
bucket and prefix. Original archive names/format and keys are in the notices and receipt. Shared exact
source is reused by digest across builds; conditional multipart completion never overwrites an existing key.
Source failure stops object-code publication. Caller-presigned plans must include these exact source keys,
not just the ISO and manifest. All multipart plans use the same bounded part size (at most 64 MiB). For source
objects, both `x-amz-meta-image-sha256` and `x-amz-meta-inputs` contain the source digest, so recipes can reuse
the same source object; the caller must set that metadata when pre-initiating a presigned upload.
The future download route must provide equivalent source access at no extra
charge alongside binary access, including for private recipients; a private bucket alone is not that route.

**OWNER OBLIGATION / DESIGN:** the notices contain our written offer of Corresponding Source, valid for at
least three years after our last distribution and as long as covered product support/spare parts are offered.
It offers network access at no charge, or physical media at no more than the reasonable cost of conveying it;
requests go to the distributor through the repository's issues page. It is **OUR obligation to keep the
mirrored source available as long as we distribute the binary**, and to honor the offer for its full term.
Do not expire these objects with binary cleanup, rely only on upstream URLs, or modify GPL object code.
For designated-place downloads the intended mechanism is equivalent source access under
[GPLv3 section 6(d)](https://www.gnu.org/licenses/gpl-3.0.html#section6), with this additional written offer;
the source offer alone is not a substitute for operating the source download route. Read the exact
[7-Zip distribution terms](https://github.com/ip7z/7zip/blob/26.04/DOC/License.txt) and shipped licences.

**UNVERIFIED / release prerequisite:** a coordinator must run the full build, inspect its legal-text inventory
(including native dependencies embedded in wheels and Blender), ensure all required Corresponding Source,
build scripts, exceptions and notices are represented, and complete qualified review of restricted packages
before public distribution. Merely pinning the main project's source does not establish transitive source
completeness. No full bundle or live source/binary upload was performed in this lane.

## 4. Verdict rules (guest passes only)

CODE: `appmatrix/grade.py` (per app) and `scripts/bench/box/windows_apps.sh` (host reference, classes). The rule is the gate's
(owner, 2026-11-01): **only the guest passing is a pass; a host, harness, input or rental fault is RETRY, never a pass and never
an excuse.**

- Per app, the verdict of the re-run **alone in a fresh guest** (phase 2) is final when it exists, else the batched one. PASS =
  rc 0, the app's own success string, no `CHECK ... FAIL`, a GPU-use proof, no activity on a non-NVIDIA adapter
  (`verdict.py`, unchanged).
- `PASS`/`PASS*` pass; `FAIL`/`TIMEOUT` are **guest failures**; `BOOT_FAIL` is a guest that did not come up; `NOTRUN` (a package
  would not stage, a binary is absent, the launch failed) and a **missing row** (the time budget ended, the driver died) are
  **harness faults**.
- The stage passes only if every expected app passed **and** the driver ran to its end **and** the same-box host reference
  (nvidia-smi plus a CUDA matmul on the Linux host) is green before and after **and** the host logged no new Xid.
- A guest failure is `WINAPPS_FAIL diag=KAYFABE` only with the host reference green before and after; otherwise it is
  `RETRY class=HOSTREF`. The documented Vast BAR1 exception is `RETRY class=BAR1_EXCEPTION` (kf3 refused a BAR1 size and no guest
  ever answered). Harness faults are `RETRY class=OPERATOR` (our script) or `MEDIA` (no or wrong app disk). A guest failure
  outranks a harness fault in the same run.
- Fail closed: a log with no `WINAPPS_EXIT`, with two verdicts, with a PASS of another revision or profile, or with a PASS and a
  non-zero exit is not a pass (`vast_payload.py winapps_rows`, `tests`: `test_vast_payload_winapps.py`).
- No exclusion by failure: an app that fails on kayfabe is not removed from `full`. An exclusion needs a native-reference
  failure with evidence (`gate_profiles.json` `excluded`, empty today).

## 5. How it runs (CODE, one design decision each)

- **Same guest as the gate.** `windows_apps.sh` runs after `windows_gate.sh` ended in `WINDOWS_GATE_PASS` for the same tag, on
  the same box. Every apps guest is a new qcow2 overlay **backed by the gate's overlay** (`WINVM_OVERLAY_BACKING`, a new opt-in
  in `win_vm.sh`), so the NVIDIA 580.88 driver, GSP forced on, autologon and the one-time device enable are already there, and a
  fresh guest after a reset is clean. The gate's overlay is sealed read-only first. **INFERRED preregistration for the
  2026-11-07 `wa1`/`wa1f` runs in section 8:**
  that the device comes up at code 0 on a boot of such a child overlay without a second enable.
- **App disk on the machine's own AHCI bus** (`WINVM_APPS_ISO`, `ide.3` of q35), read-only, present from power-on. The 2026-10
  design hot-plugged a USB CD-ROM through QMP on a broker's `xhci.0`; `win_vm.sh` has no xhci, and a boot-time disk needs no
  hot-plug and no guest network.
- **ssh instead of the guest agent** (`appmatrix/sshguest.py`): the same call surface as `qga.Qga` (`ping`, `exec`,
  `powershell`, `file_write`, `file_read`, `file_exists`), so the driver, the guest scripts, the supervisor protocol and
  `verdict.py` are unchanged. Commands travel as `-EncodedCommand` (no quoting layer); files by `scp`. No password goes through the
  harness (`KF_GUEST_PW` is unset; sign-in is the gate's autologon).
- **Budget.** The driver stops starting guests at `KF_WINAPPS_BUDGET_S` (profile default) and the rest is `NOT_RUN`, which is a RETRY.
- **Payload.** `KF_WINAPPS=small|mid|full` (new, optional, validated by the reporter, popped by the payload) adds the stage
  `windows_apps` right after `windows_gate`, with its own row (`windows_apps`, `windows_apps_retry_<class>`,
  `windows_apps_retry_time`). Absent means no stage: today's Worker changes nothing. The Worker side is one optional
  argument of `tierEnvironment` that nothing passes today; **turning it on per tier is the owner's decision and a deploy.**

## 6. Profiles

`gate_profiles.json`. DESIGN, with the evidence of section 8 filled in where a box has run them.

| Profile | Apps | For | Default budget |
|---|---:|---|---:|
| `small` | 8 (one per family: SMI, CUDA demo, CUDA python, driver-API ladder, D3D11, OpenGL, Vulkan, torch+CuPy) | tier small (master push) | 40 min |
| `mid` | 10 (small plus Edge WebGL and D3D12) | tier middle (PR to master) | 60 min |
| `full` | every open app (87 with J1 definitions) | tier large (release tag) | 300 min **(budget sufficiency is unverified; missing new source/reference inputs cannot pass)** |

Proposed mapping (**DESIGN**, not implemented in the Worker): small -> `small`, middle -> `mid`, large -> `full`.

**CODE READ, J1 definitions:** `gate_profiles.py ids release` is a separate list of 17 approved software definitions,
including owner-supplied packages and source/reference blockers. It is not a Worker `KF_WINAPPS` value.
All remain unverified; see [release software definitions](V3_WINDOWS_RELEASE_SOFTWARE_DEFINITIONS.md).

## 7. What this does not do

No change to a pass rule, to `verdict.py`, to the gate, or to the Linux twin. It does not make the owner-decision or manual
apps pass or disappear: they are listed, with a reason, until a person rules. It adds no secret to a box and no vendor
download to a run.

## 8. Measured: 2026-11-07 hardware runs `wa1` and `wa1f`

MEASURED 2026-11-07, Vast box 59505074, RTX 4090 (AD102), kf3 revision `b11655b7` (`next`), Windows 11 LTSC 2024 guest with the stock
580.88 driver; evidence and the per-app table in [`traces/winapps_20261107/`](../../traces/winapps_20261107/README.md).

- The gate passed first: `WINDOWS_GATE_PASS rev=b11655b7 ... hostref=green/green secs=1012` (install 16 min).
- **Small profile: 8/8 PASS** (12 minutes including a guest boot).
- **Full profile (79 open apps): 58 PASS, 9 FAIL/TIMEOUT, 12 NOT RUN.** Verdict `WINAPPS_FAIL diag=KAYFABE`, hostref green/green, no
  host Xid, 0 guest TDR and 0 WER in the 67 apps that ran. The failures: seven `overlay_yuv_*` probes (`CheckOverlaySupport reports no
  DXGI_OVERLAY_SUPPORT_FLAG_DIRECT`), `nbody_demo_gl` (`no-gpu-proof`) and `managed_t` (TIMEOUT, known from 2026-10-11). The batched verdict
  is the only one (phase 2 did not run; the driver was stopped for the lease limit), and no failure is attributed: no same-box native
  reference ran.
- **The first-run falsifiers did not trigger:** a child overlay of the gate's overlay boots with the adapter at code 0 and `nvidia-smi` working
  (6 of 6 guests), and the ssh transport ran the QGA-shaped driver with 0 `NOTRUN` and 0 `BOOT_FAIL`.
- **Cost of the harness, not of the apps:** the apps' own runtimes sum to 900 s, the run took about 94 minutes: at least 47 s per app and
  about 3.5 minutes per guest. 79 apps would need about 2 h before the slowest apps (clpeak 2 x 1500 s, LLM, Blender).

What is NOT measured: a Linux-side reference for the failing apps, the 10 video apps (ffmpeg pin 404), any die other than AD102, the
Worker path (no run token, no R2 bundle), the `mid` profile, and the stage inside the payload.

## 9. Open questions and what blocks the release tier

1. **Bundle upload.** A person must reproduce and verify the full `package_bundle.py` build, upload `winapps-open.iso` to R2 and commit the pin; until then the stage is
   RETRY class MEDIA by design. The Worker names are proposed (not deployed).
2. **Owner decision on the 29 apps** that need GravityMark, FurMark, GpuTest, the Vulkan SDK or ffmpeg GPL: private copy in R2 for our own
   boxes (as for SPECviewperf) or manual. **MEASURED, J1:** the ffmpeg pin is replaced by a downloaded,
   hash/CRC-checked dated artifact; source and hash are recorded in the release software definitions document.
   The earlier run's 404 remains historical evidence, not the status of the new pin.
3. **Failure attribution (CODE READ, 2026-11-07):** `managed_t` has a native PASS in
   `traces/windows_baseline_20261010/apps/managed_t.json` (verdict in `verdict/managed_t.json`), and
   `nbody_demo_gl` has two native PASS records: `traces/windows_baseline_20261010/verdict/nbody_demo_gl.json`
   and `traces/windows_matrix_20261011/reference-unigine-nbody/apps/nbody_demo_gl.verdict.json`.
   These older, different-box references establish that both apps can pass natively; they do not establish
   same-box attribution for the November run. The seven failing `overlay_yuv_*` probes have no native PASS
   on a Vast box: their applicability remains an open owner decision. All nine failures remain in `full`;
   none is excluded or moved to another class.
4. **Harness overhead** (47 s per app, 3.5 min per guest) puts `full` near 2 h plus the slow apps: the release hard limit (300 min after the
   gate's 20-60 min) is tight. Candidates: batch the per-app scp writes into one, poll less, a bigger `--per-guest`, run the slow apps (clpeak,
   LLM, Blender) as their own leg. Not done.
5. **Phase 2 and the budget:** the driver checks the budget only when it starts a guest, so one guest can overrun it (it did, by the stop).
   A per-guest deadline is not implemented.
6. **Mapping tier to profile** (small, middle, large to small, mid, full) and turning on `KF_WINAPPS` in the Worker: owner decision plus a deploy.

## 10. Guest grouping review (CODE READ and INFERRED, 2026-11-07; no hardware run)

Falsifier stated before inspection: grouping requires a boot for every app, changes the verdict predicate,
or removes isolated reruns. **CODE READ:** `winapps.py::run_matrix` already takes `todo[:cfg.per_guest]`,
runs that batch in one `GuestSession`, moves remaining apps to a fresh guest on a health/restart condition,
and reruns each phase-1 failure alone in phase 2. The stage exposes `KF_WINAPPS_PER_GUEST` (default 8);
the recorded full run used 16. The falsifier did not hold for the current grouping mechanism.

**DESIGN proposal:** evaluate `KF_WINAPPS_PER_GUEST=32` for phase 1, keeping restart conditions, per-app
cleanup, health checks and phase-2 isolated verdicts unchanged. **INFERRED arithmetic**, assuming all
79 apps finish phase 1 without a health restart: ceiling(79/8)=10 boots, ceiling(79/16)=5,
ceiling(79/32)=3. Applied to the recorded approximate 210 seconds per boot, 16 to 32 could avoid
about 420 seconds. This saving is **unmeasured**, a cost model only; isolated failures add boots.
It does not address the approximate 47-second per-app transport cost. A later implementation could
combine script writes into one ssh operation, with the existing output and health checks retained.
No grouping default, timeout, budget or verdict rule is changed here.
