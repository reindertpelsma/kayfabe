# External downloads: sources, terms status and caching rules

**STATUS: RESEARCH, 2026-10-31.** Draft summary; the terms status is not final. Since 2026-10-31 the
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
- Export: SPEC's licence refers to the US export regulations. Fetch only to boxes in an
  allow-list of host countries (US and other countries the owner approves for the licence).
  The list lives in the Worker var `BENCH_DATA_ALLOWED_COUNTRIES` (default `US,CA`; the 2026-10-31
  code replaces "the box-rental script"), and a box outside it never receives the copy.
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

The private bucket is `kayfabe-bench-enam` (Cloudflare R2, location hint ENAM, private). Its
objects for SPECviewperf 15.0.0 are:

| R2 key | Name the box asks the Worker for | Role |
|---|---|---|
| `specviewperf-15.0.0/specviewperf-15.0.0.tar.zst` | `specviewperf-15.0.0.tar.zst` | the benchmark tree, about 25 GB |
| `specviewperf-15.0.0/CHECKSUMS.txt` | `specviewperf-15.0.0.CHECKSUMS.txt` | checksums of the copy |
| `specviewperf-15.0.0/SPEC_General_License.pdf` | `specviewperf-15.0.0.SPEC_General_License.pdf` | the licence text kept beside the copy |
| `specviewperf-15.0.0/original/SPECviewperf-Setup-15.0.0.exe` | `specviewperf-15.0.0.SPECviewperf-Setup.exe` | the original installer, unmodified |

How a rented box gets them: it calls the Vast CI Worker with its run token and the Worker answers
with a redirect to a presigned URL that lives at most 20 minutes, only while the run is live, only
for a box in the countries of the export allow-list (Worker var `BENCH_DATA_ALLOWED_COUNTRIES`,
default `US,CA`), at most 4 URLs per object per run. The box never holds a Cloudflare credential.
`scripts/bench/box/fetch_bench_data.sh <name> --sha256 <pin> --out <file>` downloads with resume
into a `.part` file and renames only after the sha256 pin matches; with `--extract <dir>` it
streams `zstd -d | tar -x` and never stores the 25 GB archive. The pins (the `.tar.zst` and
`CHECKSUMS.txt` hashes) are not in the repo yet: the owner pins them at the first upload. The
2 GiB per-run ceiling named under "Download etiquette" stays an example for third-party hosts; the
owner-hosted cache is excluded from it.

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
