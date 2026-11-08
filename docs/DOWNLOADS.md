# External downloads: sources, terms status and caching rules

**STATUS: RESEARCH, 2026-10-31** (the section "SPECviewperf in the Vast gate" below was added 2026-11-07 and is DESIGN-ONLY until a
box has run it). Draft summary; the terms status is not final. Since 2026-10-31 the
code for the private R2 copy exists (Worker route `/bench-data/` and the box helper
`scripts/bench/box/fetch_bench_data.sh`, section "SPECviewperf private copy: the R2 objects");
it is merged only when its PR is, and it is not live until the coordinator deploys the Worker and
sets the R2 secrets (`ci/vast-broker/DEPLOY.md`). The 2026-10-23 text below that says no private
cache exists describes the revision named in it.

Neutral summary of what the test tooling fetches from outside the project, who fetches it, and
what the published terms allow. It is not legal advice. The research with verbatim clauses is in
`spec_terms_20261023.md` (private working note, not in this repo; each statement there is
labelled QUOTED, READING or UNKNOWN). "Unclear" means the terms were read and do not settle the
question, or the text could not be read. No vendor was contacted.

What was checked against the repo (revision `e9c6d39e5`): the sha256 pins, sizes, URLs and
versions in `scripts/bench/windows/win_vm.sh` and `scripts/bench/windows/appmatrix/manifest.json`
match Tables A and B below. The terms, robots.txt and licence statements are NOT checkable from
the repo and rest on the private research note. The SPECviewperf lane exists only on branch
`codex/kayfabe-specviewperf-20261022`, not on master. No private R2 cache exists in the repo
yet: the cache rules below are the intended policy, and the section "What the scripts do today"
lists where the code differs. [SUPERSEDED 2026-10-31 for the R2 cache: the R2 objects, the Worker
route and the box helper are described in "SPECviewperf private copy: the R2 objects" below.]

Terms status values: `OK auto` (OK to download automatically), `OK cache` (OK as private
cache), `unclear`, `not allowed` (for the stated use). Caching rule values: `direct` (direct per
box), `cache` (one cached private copy), `manual` (manual only, nothing automated).

## Table A: what, where, how much, who

| Source | Host / path | Size | Who downloads | How often |
|---|---|---|---|---|
| SPECviewperf 15.0.1 | gwpg.spec.org form | 147 MB + workloads | dev box | once per release |
| SPECviewperf 2020 v3.1 | spec.org (retired) | 141 MB | nobody | not used (retired) |
| Unigine Heaven 4.0 | assets.unigine.com | 260 MB | app-disk builder | once |
| GravityMark 1.89 | tellusim.com/download | 221 MB | app-disk builder | once |
| FurMark 2.10.2 | gpumagick.com (manifest URL) | 35 MB | app-disk builder | once |
| 3DMark / Steel Nomad | benchmarks.ul.com | n/a | nobody | manual only |
| Geekbench 6.4 / 6.5 / 7.0.x | cdn.geekbench.com | 228 MB | Linux lane box | per box |
| CUDA demo suite 12.9.79 | developer.download.nvidia.com | 5 MB | app-disk builder | once |
| CUDA samples v12.5 | github.com/NVIDIA | git | Linux apps bundle | per build (shallow) |
| Win32-OpenSSH 10.0.0.0p2 | github.com releases | 7 MB | `win_vm.sh fetch` | per box |
| MS symbol server | msdl.microsoft.com | per file | debug analysis script | on demand |
| VirtIO-Win 0.1.302 | fedorapeople.org | 877 MB | `win_vm.sh fetch` | per box |
| Windows 11 LTSC 2024 eval | microsoft.com evalcenter | 5.1 GB | `win_vm.sh fetch` | per box |

"App-disk builder" is `scripts/bench/windows/appmatrix/appdisk.py` (run through
`build_appdisk.sh`) on the host that builds the Windows app disk image; the guest never
downloads at run time. Geekbench sizes and the CUDA samples tag come from
`scripts/apps/setup_side.sh`, `scripts/bench/gfxset/provision.sh` and
`scripts/apps/build_bundle.sh`; Geekbench is not in the Windows app matrix (deferred in
`manifest.json`). `verdict.py` cites CUDA samples v12.9 in comments only.

## Table B: terms, caching, pins

Pins are the sha256 already used by `scripts/bench/windows/win_vm.sh` and the appmatrix
manifest (first 12 hex digits shown; the full value stays in those files). "TBD" = to be pinned
at the first fetch by the owner.

| Source | Terms status | Caching rule | sha256 pin |
|---|---|---|---|
| SPECviewperf 15.0.1 | unclear (owner is licensee) | cache, owner only | TBD |
| SPECviewperf 2020 v3.1 | not allowed (retired) | none | md5 7197f85e... |
| Unigine Heaven 4.0 | unclear (Basic: eval only) | manual | 497865a09af2 |
| GravityMark 1.89 | OK cache (non-commercial) | cache | 38e32b475cf9 |
| FurMark 2.10.2 | unclear (EULA unread) | cache | 27ab2e723e2e |
| 3DMark / Steel Nomad | not allowed (automated) | manual | n/a |
| Geekbench 6.x / 7.x | not allowed (scripts need Pro) | manual | none (unpinned today) |
| CUDA demo suite | OK auto | direct | adffd0b0f7cc |
| CUDA samples | OK auto (BSD-3) | direct (pinned tag) | git tag v12.5, no commit |
| Win32-OpenSSH | OK auto | direct | ddec9c538642 |
| MS symbol server | OK auto (debug use) | direct, local store | per file (symchk) |
| VirtIO-Win | OK cache (BSD-3) | cache | 303f7ae40dad |
| Windows 11 LTSC eval | unclear (ISO licence unread) | cache, owner only | 67cec5865eaa |

Notes per row:

- SPECviewperf: SPEC states that retired benchmarks are not available for download, so the
  lane targets 15.0.1. The licence also ends for a release when SPEC issues a subsequent one.
- Heaven: the free edition's licence says evaluation and educational use; command-line
  automation is listed under the paid Advanced edition.
  The manifest's `redistribution` text for Heaven ("non-commercial benchmarking") is looser than
  the EULA wording quoted in the research and should be aligned when the manifest is next edited.
- FurMark, GpuTest: the manifest labels both "Geeks3D freeware licence"; the FurMark EULA file
  itself has not been read, so the status stays `unclear`.
- Geekbench: the licence restricts automated access to the Pro licence. cdn.geekbench.com is
  not blocked, but geekbench.com refuses scripts; nothing here circumvents that.
- 3DMark: the licence is personal and non-commercial, one computer, no distribution.
- Win11 eval: 90-day evaluation intended for IT professionals; rebuild images, do not keep.
- Others not yet reviewed (Blender, PyTorch, llama.cpp, models, Vulkan SDK, ffmpeg, hashcat,
  clpeak, GpuTest, Python, 7-Zip, Q2RTX, Xonotic, Windows SDK debuggers, NVIDIA drivers) stay
  as `unclear` until read. FFmpeg GPL builds are never redistributed from our storage.

## SPECviewperf private copy: handling rules

These follow the SPEC General License Agreement (copies, third parties, distribution, term,
export, trademark clauses). Reading, not advice.

- The owner is the licensee. The owner downloads the installer, accepts the licence and pins
  the sha256 in the repo.
- The private R2 copy holds exact and complete copies only: the original file, unmodified,
  with the licence text and run rules kept beside it.
- The bucket is private. Only the owner's own machines fetch from it (short-lived signed
  URLs). No sharing with contributors, forks, CI of forks, or anyone else. No public mirror,
  release asset, shared Actions cache, container image or VM image that contains the copy.
- A rented box gets the copy only for the run and the copy is deleted at teardown, before the
  box is released. Teardown logs record the deletion.
- Export: SPEC's licence refers to the US export regulations. Owner ruling 2026-11-01: the Worker
  does not gate by country (a valid run token is the gate; the box's country is a rental preference
  only). The copy is not secret (SPEC offers it for download without sign-in); it is private only so
  that we do not distribute it.
- Refresh on every new SPECviewperf release. The licence for a release ends when a later one
  is issued, so re-download, re-accept, re-pin, and remove the old copy.
- Published numbers follow SPEC's Fair Use Rule: benchmark name and version, trademark line,
  date, link to www.spec.org, virtualised-configuration disclosures, and "(nc)" marking for
  non-compliant numbers. Ratios between configurations are derived values. When unsure,
  publish only "completed" or "failed". Keep result files from every run (the licence limits
  use to generating performance data).

## SPECviewperf private copy: the R2 objects

STATUS: DESIGN-ONLY until the Worker change is deployed (code in the PR that added this section; no
deployment, bucket content or Cloudflare setting is checked from the repo).

Size correction, 2026-11-05 (folded above the text it corrects): an earlier version of this section said "about 25 GB" for the
archive. The object is 42,002,015,727 B, and `zstd -d` of it gives a 94,190,929,920 B tar (MEASURED on Vast box 59248850, rev 6a0683af, 2026-11-05,
`traces/specviewperf_vast_20261105/README.md`).

The private bucket is `kayfabe-bench-enam` (Cloudflare R2, location hint ENAM, private). Its
objects for SPECviewperf 15.0.0 are:

| R2 key | Name the box asks the Worker for | Role |
|---|---|---|
| `specviewperf-15.0.0/specviewperf-15.0.0.tar.zst` | `specviewperf-15.0.0.tar.zst` | the benchmark tree, 42,002,015,727 B (the tar inside is 94,190,929,920 B) |
| `specviewperf-15.0.0/CHECKSUMS.txt` | `specviewperf-15.0.0.CHECKSUMS.txt` | checksums of the copy |
| `specviewperf-15.0.0/SPEC_General_License.pdf` | `specviewperf-15.0.0.SPEC_General_License.pdf` | the licence text kept beside the copy |
| `specviewperf-15.0.0/original/SPECviewperf-Setup-15.0.0.exe` | `specviewperf-15.0.0.SPECviewperf-Setup.exe` | the original installer, unmodified |

How a rented box gets them: it calls the Vast CI Worker with its run token and the Worker answers
with a redirect to a presigned URL that lives at most 20 minutes, only while the run is live, only
with a valid run token (no country gate), at most 4 URLs per object per run. The box never holds a Cloudflare credential.
`scripts/bench/box/fetch_bench_data.sh <name> --sha256 <pin> --out <file>` downloads with resume
into a `.part` file and renames only after the sha256 pin matches; with `--extract <dir>` it
streams `zstd -d | tar -x` and never stores the 42 GB archive (about 94 GB unpacked, so a guest disk that will hold the tree needs more than that). The pins (the `.tar.zst` and
`CHECKSUMS.txt` hashes) are not in the repo yet: the owner pins them at the first upload. The
2 GiB per-run ceiling named under "Download etiquette" stays an example for third-party hosts; the
owner-hosted cache is excluded from it.

## Windows app bundle (the Windows apps stage)

STATUS: CODE + DESIGN, 2026-11-08 (latest owner rulings). The first deliverable is build + verification;
coordinator upload is manual and CI publication remains optional/disabled. WB1 has not run a full pinned
build, uploaded media, configured secrets/domains, deployed Worker code or claimed a native/GPU pass.

**OWNER POLICY / CODE READ:** winapps-open.iso (KFAPPS) is the only permanent bundle. It contains only
public-ok packages. The private ISO/R2-secret-set design is superseded: NVIDIA and conditional inputs come
from exact official URLs on the **BOX HOST at run time**, using hash-verified J4 parallel-range fetching.
They are never deliberately stored in a published ISO, R2 or GitHub. A deterministic KFAPPSP disk exists only
for the current run, attaches read-only next to KFAPPS and is discarded after guest teardown. The guest
remains offline. There is no permanent private pin or private publication workflow.

### Shell-free build and the two reproduction levels

**CODE READ:** use Python 3.11+ standard library on Linux, Windows or macOS and the pinned official Zig
release. No bash, curl/wget, pip resolver, distro mingw, host application extractor, ISO utility or host
compression is required. Original application archives/installers remain intact; the compiler distribution
alone is unpacked, and legal texts are read into the licence inventory. The script rejects run tokens and
upload credentials. CI builds require explicit --ci-build; full builds remain prohibited on WB1's dev box.

```text
python3 -I scripts/bench/windows/appmatrix/package_bundle.py check
python3 -I scripts/bench/windows/appmatrix/package_bundle.py fetch --dl downloads
python3 -I scripts/bench/windows/appmatrix/package_bundle.py verify --dl downloads
python3 -I scripts/bench/windows/appmatrix/package_bundle.py build --dl downloads --out out-a
python3 -I scripts/bench/windows/appmatrix/package_bundle.py build --dl downloads --out out-b
python3 -I scripts/bench/windows/appmatrix/package_bundle.py compare-image --iso out-b/winapps-open.iso --receipt out-a/winapps-open.manifest.json --out out-b
```

**OWNER DECISION / CODE READ:** Level 1 is the reproducibility claim and gate: the same complete file list
and the same SHA-256/size per file, including pinned originals and repository-built programs. Level 2 is
best effort: identical ISO container bytes too. If Level 1 matches but Level 2 differs, comparison succeeds
with a WARNING and a container-metadata diff. Dates/IDs/publisher fields, extent layout and non-payload
region hashes are recorded for diagnosis; the difference is a builder bug, not failed reproduction.
The report is also written as reproduction.diff.json.

**CODE READ:** each image has its **own** out/winapps-open.manifest.json with every file/hash/size, licences,
comparison policy, toolchain/source pins, Level 1 inventory digest, ISO hash/size and container snapshot.
Packaging date and Python version are external audit fields. `verify-image --out out-a` checks that stored
image against its own digest/receipt; it does not compare two builds. The harness keeps this object-integrity
check. Corrupt stored bytes fail integrity even if a proposed reproduction's file inventory was unchanged.
Use compare-image for reproduction and verify-image for one object's integrity.

**CODE READ:** all repository programs are byte-checked by default. A reviewed bundle_toolchain.programs
entry may mark one known program inventory-only, with a reason and named functional checks. Both builds
must carry the same policy; the actual program hash remains recorded, and native functional validation is
still required. Vendor inputs/other files cannot opt out. No real program currently uses this exception.
All URLs, sizes and SHA-256 values are pinned; wheels use direct hash-locked URLs with no pip resolution.
Sorted entries, fixed timestamps/IDs, normalized source paths/text, pinned compiler/flags and zero PE
creation timestamps remove host date/user/locale/umask differences by construction. Vendor replacement or
404/410 of pinned inputs is INPUT_DRIFT naming the package, never a Level 2 warning or silent repin.

**Manual GUI path (CODE READ; native GUI use is unmeasured):** install official CPython with Tcl/Tk and .pyw
associations. Use a file manager/Git GUI to obtain a fixed reviewed checkout, then double-click
scripts/bench/windows/appmatrix/package_bundle_gui.pyw. No shell commands are required:

1. Select manifest.json/profiles, an external Downloads directory and an empty Output directory; choose
   Check then Layout. Only the public class is offered.
2. In a browser save every open_packages files[].url using its exact files[].name under
   Downloads/<package id>/. Keep extensions visible and originals unchanged; do not run application
   installers on the packaging machine. Similar versions do not reproduce the pinned bytes.
3. Save the exact bundle_toolchain.hosts archive for your CPU/OS under Downloads/_toolchain/<name>.
   Also save every selected compliance.license_inputs and compliance.sources original under
   Downloads/_compliance/<package id>/<name>. Each legal/source input has its own URL/hash/size pin.
4. Choose Verify (or Fetch to retrieve the same originals using urllib). A mismatch names the package;
   stop and investigate. Do not replace a pin with a newer vendor hash.
5. Choose Build then Check image integrity. Repeat into a second empty Output directory.
6. Choose Compare reproduction, select the first receipt and the second ISO. Its own sidecar is read
   automatically. Level 1 differences fail with paths; Level 2 metadata differences warn. Inspect the
   inventories and reproduction.diff.json, keeping receipts as operator evidence.
7. Inspect LICENSES/index.json, THIRD_PARTY_NOTICES.txt and SOURCE/ before coordinator upload.

**CODE READ / limitation:** manually installing the same official installers can reproduce an installed
application environment, but cannot itself reproduce the ISO filesystem/probe compilation. Use the Python
writer (CLI or GUI) for the file-inventory claim. Fixture tests are not proof of full cross-host builds or
native Zig/mingw functional parity. Keep every native app-matrix verdict and required functional check.

### Official run-time downloads and transient media

**CODE READ:** manifest.json retains package class: public-ok, official-fetch or manual-only.
gen_official.py derives official_fetch.json and official_requirements.txt. The catalogue lists each
artifact's ID, official URL, SHA-256, size, licence ID, guest destination, guest-only installer argv and
staging recipe. apps.json requires.official_fetch links catalogue groups; dependencies and restricted HF
extras are explicit. No extra unlisted package is fetched. Public media contains python_embed,
llama_vulkan, qwen_gguf, hf_model, vkpeak, Blender, clpeak_ocl, hashcat, sevenzip and audited permissive HF
wheels. CUDA/demo/NVRTC/CCCL, torch-related wheels, llama CUDA/cudart, clpeak CUDA, Microsoft/LunarG runtime
items and restricted HF extras use official fetching. Other licensed/manual apps and standalone ffmpeg
remain outside the canonical automatic selection.

**CODE READ / J2 interface:** select --official-driver VERSION or KF_WINAPPS_OFFICIAL_DRIVER per Windows
subrun. Only a pinned version is accepted; the current known Windows pin is 580.88. Additional matrix
versions require exact official records; no version/URL is guessed from the guest. --guest-driver remains
the distinct kf3 ABI setting. official_runtime.driver_plan supplies guest-stage/install arguments for J2
before the parent Windows gate. The app harness checks the installed version; it never switches that
parent driver after the gate. A mismatch is NOTRUN / MEDIA_DRIFT, not a pass.

**CODE READ:** the Linux box host validates the J4 source hash before executing it, with eight ranges and
pinned size/hash, no Worker auth flag and a minimal environment. It captures downloader output, reports
only package IDs/sizes/digests, rechecks files and checks bytes actually copied into KFAPPSP. Temp media
and its local manifest live outside evidence directories, span all guests/teardown and are removed on
return/errors; abrupt box/runner destruction discards run storage. KFAPPS K: and KFAPPSP P: are read-only.
Missing official media gives affected apps NOTRUN / RETRY MEDIA; official URL/hash/size failures give loud
MEDIA_DRIFT. --no-official-fetch cannot make required apps pass; independent public-only apps need no
second disk. The offline guest uses bundled 7-Zip or Windows tar/msiexec at run time.

**OWNER POLICY / CUDA EULA:** the downloader is the person or CI operator running official fetch/verify
for private use; that operator must accept the pinned CUDA EULA. Downloading in CI does not permit public
redistribution. This is engineering support, not legal advice.

**CODE READ / URL verification:** dispatch mode verify-official hashes every official input into temporary
storage and removes it; verify also selects this independent job. It produces no ISO, artifact/cache or
upload. A monthly cron remains disabled pending owner review; its intended command is:

```text
python3 -I scripts/bench/windows/appmatrix/official_runtime.py verify
```

### Licence/source retention, CI verification and coordinator upload

**CODE READ:** every public build includes LICENSES/, provenance, generated third-party notices, exact
pinned copyleft SOURCE/ archives and a written source offer. GPL binaries stay unmodified. **It is OUR
obligation to retain mirrored source while distributing object code and honor the full offer term**, at
least three years after last distribution and while covered support/spare parts remain offered. Source
is offered next to the binary with equivalent access; upstream links alone do not replace our obligation.
Complete transitive source/licence coverage still needs inspection with the full real artifacts.

**CODE READ / CI:** bundle-publish.yml is dispatch-only, minimal-permission, SHA-pinned and disabled for
publication by default. It plans cheaply, builds/reads back twice without secrets and gates on Level 1.
Verify compares the published manifest or committed # inventory digest, never the ISO hash as the
reproduction gate. Container differences produce metadata warnings. An ISO hash alone is insufficient
for Level 1 verification. The computed two-build disk budget is about 10.9 GiB; CI frees unused toolchains,
measures space and fails before downloading if insufficient, requiring a larger runner. Actual runner
capacity/full build duration remain unmeasured. No Actions media artifact/cache is created.

**CODE READ / first upload:** the coordinator reproduces/inspects the public outputs, then uploads with
an authorized manual transport. publish_bundle.py --manual-presigned --out out-a supports caller-supplied
BUNDLE_UPLOAD_URLS and BUNDLE_PUBLIC_BASE_URL in a local environment; it refuses CI/Actions, static
credentials/run tokens and all transient/official media. It never prints capabilities. Neither this lane
nor CI invokes manual upload. The optional default CI uploader requires enabled canonical master context
and a protected master-only environment; only its final step references public-bucket secrets. Nothing
has been enabled/provisioned by WB1.

**CODE READ:** immutable public keys are winapps-open/kfapps-<ISO sha256>.iso,
winapps-open/manifest-<ISO sha256>.json and winapps-open/source-<source sha256>.archive. Upload exact sources
first, then ISO/receipt. Stdlib SigV4 or caller-presigned resumable multipart supports objects above 5 GiB,
with create-only conditions and matching-object reuse; never overwrite a conflicting existing key. Live
R2 conditional completion/source access still need coordinator validation. After successful upload,
review/update bundle.pin with the generated ISO digest, inputs, Level 1 inventory digest and keys. A future
trusted draft pin PR and OIDC broker are design only; no token handling or Worker change is implemented.
This replaces the bundle's old GitHub-Releases private store. Setup cache and WinPE migration are separate;
WB1 does not migrate NVIDIA files or move/delete old-store contents.

Full recipe, qualification rules and rollout details: [Windows apps gate, section 3](design/V3_WINDOWS_APPS_GATE.md#3-inputs-public-bundle-and-official-run-time-media).

## SPECviewperf in the Vast gate (the `specviewperf` stage)

STATUS: DESIGN + CODE, 2026-11-07 (docs/design/V3_SPEC_GATE.md). Draft PR against `next`, disabled by default; no box has run it.
This section is how the stage handles the private copy; the rules above ("handling rules") are unchanged and bind the stage.

- Inputs come only through the Worker's `/bench-data` route (`scripts/bench/box/fetch_bench_data.sh`, this run's token): the
  object `specviewperf-15.0.0.tar.zst`. The stage never reaches spec.org or a vendor, never fetches the installer
  (`specviewperf-15.0.0.SPECviewperf-Setup.exe` stays an allow-listed Worker name for the owner's own use), and the licence
  PDF and `CHECKSUMS.txt` are not fetched per run.
- The sha256 pin is in the repo: `scripts/bench/box/specviewperf.pin`. PROPOSED 2026-11-07, OWNER TO CONFIRM: the values are the
  ones of the object's own `CHECKSUMS.txt` as recorded in `traces/specviewperf_vast_20261105/spec/CHECKSUMS_pins.txt` (the
  archive `ffe0e505...d5a`, the decompressed tar `cf37055c...bdb8`). A missing pin line, a missing object (Worker 404) or a
  mismatch is RETRY class MEDIA; `fetch_bench_data.sh` deletes bytes that do not match. The Worker allows 4 presigned URLs of at
  most 20 minutes per object per run; the stage uses at most 3 (the helper resumes with `-C -` on a fresh URL).
- The copy lives for the run only. The 42 GB archive is deleted right after the lane's viewsets are in the guest; the child
  overlay that holds the tree is deleted at the end of the stage (and by the stage's TERM/EXIT handlers); both deletions print
  a `SPEC_COPY_DELETED` line, which is the teardown record this section asks for. The box is destroyed after the run anyway.
- Only exact content is copied: the tree is streamed from the verified archive (`zstd -d | tarfilter.py | tar -x` in the guest)
  with whole viewset directories left out, never altered; `tarfilter.py` refuses links, devices, absolute paths and `..`.
  The licence text is not part of the guest copy; the owner's R2 copy keeps it beside the archive.
- Nothing is shared: the guest is offline apart from the harness's ssh forward, the archive and tree are not collected into the
  evidence bundle (`disk/`, `*.qcow2` and `*.iso` are never collected), and no score is read: the stage parses the CLI's status
  lines only and never opens the vendor result directory, which dies with the guest. Fair Use: the public log and the run report
  carry viewset names and PASS/FAIL/NOT_RUN with subtest counts and the names of failed subtests, no score and no timing
  (docs/SPEC_RESULTS.md says scores may be published only with `(nc)` and the disclaimer; this stage publishes none).
- Disk: the Worker rents 120 GB today (`diskGb` in `ci/vast-broker/defaults.json`, `disk: 120` in `src/vast.js`); a lane needs
  about 130 GB free after the Windows gate (archive 42 GB plus 44 to 50 GB of tree plus Windows), the whole tree about 190 GB.
  Requesting 300 GB (220 GB floor, MEASURED by the native run) for the release tier is an owner decision and a Worker change,
  not part of this PR. Without the space the stage is RETRY class PROVIDER.

## Download etiquette

- Pin hashes. Every fetched file is checked against a pinned sha256 (and byte length). A
  mismatch stops the run and never retries against another mirror.
- Resume. Use `curl -C -` (range requests) into a `.part` file and rename after the hash passes.
  SPEC's server is a single site in the US with no CDN and is slow from a residential line,
  so resume is mandatory and no parallel connections to it.
- Backoff. On 429, 5xx or timeouts wait with exponential backoff and jitter (30 s, 60 s, 120 s,
  up to a handful of tries), and honour `Retry-After`. SPEC's robots.txt asks for a 10-second
  crawl delay; keep at least that between requests to spec.org hosts.
- Stagger starts. Boxes that need the same file do not start together; offset by minutes, or
  one box fetches and the others copy from the private cache.
- User-Agent. Send a descriptive one, for example `kayfabe-bench/1.0 (+repo URL; contact)`.
- Per-run byte ceiling. Each run has a budget (for example 2 GiB from third-party hosts,
  excluding the owner-hosted cache). Exceeding it aborts the run with a clear message.
- No heavy downloads on PR checks. Pull-request CI downloads nothing larger than a few MB
  and nothing from SPEC, Unigine, Tellusim, Geeks3D, UL, Primate Labs or Microsoft media
  servers. Heavy lanes run on owner-triggered hardware jobs from the private cache.
- Local first. Check the cache and the pinned hash before any network access.
- Record. Log URL, date, size, sha256 and the licence version accepted, in the run evidence.

## What the scripts do today (checked at `e9c6d39e5`)

The rules above are the target. Where the code differs:

- sha256 and byte length: `win_vm.sh fetch` checks both and dies on a mismatch; `appdisk.py`
  checks the manifest sha256 and counts a mismatch as a failure. Neither retries another mirror.
- Resume: `appdisk.py` uses `curl -C - --retry 3` into a `.part` file. `win_vm.sh fetch` uses
  `curl -fL --retry 3` without `-C -` and renames the `.part` file before the hash check, so a
  bad file stays in the cache until removed (the next run dies on the same mismatch).
- User-Agent: `appdisk.py` sends `Mozilla/5.0 (kayfabe-appmatrix)`, not a descriptive contact
  string. `win_vm.sh` sends curl's default.
- [SUPERSEDED 2026-10-31 in part: the private R2 route, the country allow-list (a Worker var, not
  the box-rental script) and the box helper exist in code, see "SPECviewperf private copy: the R2
  objects"; nothing is deployed or checked on a box yet, and teardown deletion of the copy is
  still the box destruction, with no separate deletion log.]
- Not implemented yet: exponential backoff with jitter, `Retry-After`, the 10 s spacing for
  spec.org hosts, a per-run byte ceiling, teardown deletion of the SPECviewperf copy, the unpinned Geekbench and CUDA samples fetches,
  and a CI check that PR jobs download nothing heavy.
