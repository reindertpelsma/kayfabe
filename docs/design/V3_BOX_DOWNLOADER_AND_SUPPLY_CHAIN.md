# Box downloader and build supply chain

**STATUS: DESIGN-ONLY 2026-11-13.** Records the owner's decisions of 2026-11-13, taken in the coordinator's session. Nothing here is implemented, deployed or run; every "will", "must" and "proposed" below is DESIGN. This document touches no code and no Worker.

Labels used below. **OWNER DECISION (2026-11-13)** is binding and is not re-litigated here. **DESIGN** is a proposal that follows from a decision. **MEASURED** names a file or log line. **INFERRED** is arithmetic or reasoning from measured lines. **UNVERIFIED** is a fact about an outside system that nobody has checked. **CODE READ** means the code was read and not run.

## 0. Inputs from the stage timing analysis

Source: `/root/night/stage_timing_analysis_20261113.md` (an analysis of past `gpu-vast` runs and committed traces; the file is outside the repository, and its numbers are quoted here only as they appear there).

| input | number | label in the source |
|---|---|---|
| Windows install (media fetch, unattended Setup, seal) | median 39.1 min over 9 boxes (min 10.7, max 58.7); the Windows gate median is 58.2 min over 7 boxes | MEASURED (`WINDOWS_GATE_TIMING install=`) |
| Windows install inputs | the same pinned inputs on every box; the install runs before any NVIDIA driver is present | CODE READ (`windows_gate.sh` lines 728 to 733, `nv_install` at line 798) |
| Planner key for `windows_base` | the full driver triple, which is probably too narrow because the install is driver-independent | CODE READ, not tested |
| Linux apps media (build bundle plus guest image) | 25 to 29 min per fresh box | 25 is INFERRED from the GH-5 apps stage minus the matrix; 29 is the 2026-11-07 fresh-box record |
| SPEC download (42.0 GB from private R2, presigned) | 13.4 min at 52.2 MB/s, one box | MEASURED |
| SPEC copy into the Windows guest (94 GB, tar over ssh) | 35 to 40 min at 40 to 45 MB/s guest disk write | rate MEASURED, total INFERRED |
| Download rate per stream on Vast boxes | 4.3 to 85 MB/s, median about 37 MB/s (Windows ISO, 6 leases) | timestamps MEASURED, rates INFERRED by division |
| Upload speed from a box | not measured anywhere | MISSING |
| `provision` stage | median 7.4 min over 7 payload runs (min 4.9, max 10.5) | MEASURED |
| `merge_check.sh` rebuild | appears to rebuild kf3, the ladder client and the fast guest a second time (lines 84 to 91); the cost is not measured | CODE READ, unmeasured |

## 1. Parallel downloader

**OWNER DECISION (2026-11-13).** Each box runs a separate downloader process that fetches all dependencies in the order the stages need them and reports each completed item. The executor starts a stage as soon as that stage's items are ready; it waits only for what its next stage needs. The goal is that the box's bandwidth is used from the start and during execution, in place of the lock-step "run, then download with zero compute".

### 1.1 Sources, in order per item (OWNER DECISION)

a. Private R2, through a signed URL obtained from the Worker API.
b. Official https URLs (checksum from the publisher, or pinned in the manifest).
c. apt packages: download-only into a cache; the install happens in the executor, and never two apt processes at once.
d. Seeders, only as an optimisation (section 4: deferred, so this source is empty in the first version).

### 1.2 Manifest item (OWNER DECISION for the fields; the file format is DESIGN)

`{name, ordered sources, sha256, size, dest, needed_by_stage}`.

- Download to `<dest>.unverified`, check sha256 and size, rename atomically, delete on mismatch. Build on `scripts/ci/kf_fetch.py`, which already does this: `.unverified` staging, a pin that comes from the caller and never from the server, deletion of the file and its resume state on a size or hash violation, resume after a plain interruption, a fallback URL, and no capability header sent to a redirect target (CODE READ of the docstring).
- Ordering is by first use (the `needed_by_stage` of the earliest stage that needs the item), not by size.
- The downloader runs niced and ionice idle, and is bandwidth-capped by default, so Windows boot and network-sensitive tests are not starved. The default cap value is DESIGN-open and unmeasured.
- A failed or mismatched download is a harness failure of the RETRY class, never a guest FAIL.
- Per-item completion and bytes are reported through the existing box report channel.
- Disk is reserved up front (the sum of manifest sizes plus unpack space).
- The zip of a big image is not unpacked while a Windows guest runs, or it is stream-unpacked.

### 1.3 Consequences (DESIGN, INFERRED)

- With one downloader per box the 13.4 min SPEC download (MEASURED, one box) can overlap `provision`; the source analysis says it already did in that one run.
- The downloader does not shorten the 35 to 40 min guest copy (INFERRED); that is a separate item (an attachable disk instead of a tar stream) and is not part of this decision.
- Parallel streams per item may raise throughput; the source reports 223 MB/s over eight streams from the dev box, owner-reported, not verified, and not from a Vast box. UNVERIFIED on a box.

## 2. Supply chain for the kayfabe binary and the Linux apps image

**OWNER DECISION (2026-11-13).**

- The kayfabe binary (`kf3_bins`) and the Linux apps image (`open_image`, about 13 GB) are built by trusted CI runners that use only git at that commit and official sources (crates.io, the pinned QEMU tarball, Ubuntu mirrors with pinned hashes). These build jobs hold no secrets, have no access to our infrastructure, and are isolated from the `gpu-vast` job.
- They produce a GitHub artifact attestation. This gives provenance, not reproducibility.
- A separate PUBLISH job is the only job with an R2 write credential. It never builds. It runs only for `next`, `master` and release tags. It downloads the artifact, verifies the attestation with `gh attestation verify`, uploads to R2, and records the hash for the Worker.
- Boxes download the attested binary or image from R2. They compile only as a fallback when CI built no artifact for that commit (manual runs, forks, runs that start before CI finishes), under a deadline rule. The deadline value is DESIGN-open.
- The binary that passes the gate is the released binary.
- Forks: the build jobs work out of the box. Attestations name the fork repository, so consumers must pin the repository they trust. There is no publish job in forks. Pull requests from forks cannot attest (read-only token), so their boxes compile.

### 2.1 Is the binary per driver context? (what is known, what is not)

- Known (CODE READ): `scripts/bench/build_kf3.sh` takes a QEMU source tree and an optional build dir, names no driver in its arguments or body, and installs the result per source revision under `kf3-bins/<rev>/`. `crates/kf-gop-image/build.rs` is the only `kf-*` build script, and it does not mention ogkm.
- Known (project rule, CLAUDE.md): per-die facts come from the host at run time and register offsets are generated from ogkm, a pinned oracle.
- Unknown (INFERRED, not checked): whether any compile-time constant depends on the host or guest driver version, so the binary is probably one per commit; this was not verified across `crates/`. The planner catalog currently keys `kf3_bins` with scope `box` (CODE READ in the source analysis). Before the first CI-built binary is published, one check is needed: build once and compare against a build with a different pinned oracle, or read every use of the oracle pins in `crates/kf-*/build.rs` and generated sources. If any dependency is found, the artifact key becomes commit plus driver context.
- Unknown: whether the 13 GB image depends on a driver context. The source analysis lists the CUDA toolkit installed from NVIDIA's repo and a conflict with the owner rule that nothing from NVIDIA is stored by us (see section 5.4). The image content split is an open owner item, not decided here.

### 2.2 Unverified (UNVERIFIED, nobody has checked)

- GitHub artifact and release-asset size limits for a 13 GB image. If the limit is lower, the build job cannot hand the image to the publish job through GitHub as a single artifact, and a split or a chunked format is needed.
- Details of offline verification of an attestation: `gh attestation verify` needs network access to GitHub unless the attestation bundle is checked offline; whether the publish job can store the bundle beside the object in R2 so that a box can check it without the Worker is untested.
- Whether a pull request from a fork can be attested at all beyond the read-only-token statement above (that statement is the owner's, not tested here).

## 3. Trust table

**OWNER DECISION (2026-11-13).**

| box kind | git source | official URLs | public R2 objects | private R2 (SPEC stage, proprietary image, licensed content) | seeds from other boxes | counts for the large tier |
|---|---|---|---|---|---|---|
| trusting (owned, container, standard) | yes | yes | yes | yes, through the Worker's signed URL | none in the first version (section 4) | yes |
| `borrowed` (git-only; distrusts the Worker) | yes | yes | only if the box verifies a GitHub attestation itself | never | never | never: community results never count |

Private R2 exists only to comply with vendors' anti-distribution terms, not for secrecy. Nothing in it is sensitive to the owner. Everything in it can be re-obtained by anyone by accepting licences online (free, a form, or a purchase such as 3DMark Advanced). It never ends up in released builds because the build runners cannot access it.

## 4. Seeding (box-to-box transfer): DEFERRED

**OWNER DECISION (2026-11-13): dropped from the first version.** Reasons:

- Nothing was found that every box repeats, takes long, and is shareable. The 8 release dies have 8 unique host plus guest driver combinations (project memory, release driver matrix, 2026-11-07).
- The big items come from R2, where egress is free.
- Seeding costs double network, including Vast `inet_up` billing.
- Seeder upload speeds are unknown (the source analysis found no upload measurement) and are often slow; the Worker's only threshold is `upMbps=50`, which is 6.25 MB/s, and 13 GB at that rate is about 35 min against a 9 to 14 min local image build (INFERRED).

The only value left is the fallback case (no CI binary): one box compiles and hands the small kf3 binary (0.11 GB in the planner fixture, an assumption) to the others, and outputs that embed NVIDIA binaries, which owner rules forbid in R2 and GitHub, built per box. The source analysis bounds the saving per small box at a few minutes (INFERRED).

### 4.1 Design worked out, kept in case it is revived (DESIGN, not built, not measured except where marked)

- Transport: HTTPS with a per-session self-signed certificate whose SAN is the generated server name. The public-key pin is posted to the Worker (`pinned_public_key`).
- nginx: the default server uses `ssl_reject_handshake on` for unknown names, plus `return 444`.
- Server name: generated randomly by the Worker. It is anti-scan only and not a secret (SNI is cleartext).
- Capability: a path of at least 128 bits of entropy. Exact-match nginx locations per file; no autoindex; no symlink directory; the path is not written to logs.
- Registration: the seeder POSTs only `{path, file, sha256, size, pinned_public_key, status_path}`, authenticated with the per-run box token. The Worker rebuilds the URL from the IP and port it already knows (Vast API or admin config). A box never supplies a host. The Worker validates every field with strict patterns.
- Consumer: connects with `curl --resolve name:port:ip` or `--connect-to`; in Python it connects to ip:port with `server_hostname=name` and checks the pin.
- Hash status: the seeder's sha256 is `seeder_reported` (transfer integrity only), because no independent hash exists for run-local builds.
- The Worker never probes seeders: a Worker `fetch()` of a raw IP returns Cloudflare error 1003 (MEASURED 2026-11-12, `docs/design/V3_CUSTOM_HOSTS.md`). The box posts that it is online instead.
- QUIC and HTTP/3 are rejected as overkill.

Status of existing code: the seed code already merged on `next` (#347 hosts DO seed probe, #345 kfseed, the planner's seed exports) stays dormant with `seeder.enabled` off. A later cleanup PR may remove it. This design supersedes the seed-probe parts of `docs/design/V3_CUSTOM_HOSTS.md` for the first version only; that document is not edited here.

## 5. Implementation order

DESIGN, in this order (the order is the owner's; the step text is the proposal).

### 5.1 Step 1: `KF_STEP` instrumentation

Add one helper to the payload scripts that prints a bounded line `KF_STEP name=<x> secs=<n> bytes=<n>` and make it a `[key]` line, so it survives the cut of the runner's public log (the source reports about 1.05 MB dropped from the middle). Call it around:

1. each step of `scripts/bench/box/provision_full.sh` (BOX, DRIVER, TREE, LADDER, FG, KF3);
2. the three rebuilds at `merge_check.sh` lines 84 to 91 (settles whether the second build is cold or incremental);
3. `win_vm.sh fetch` (per file bytes and seconds, which also gives the download rate), a `KF_STEP name=win_setup` between "fetch done" and `SEALED`, and `stat -c %s disk/base.qcow2` after sealing;
4. the apps provisioning steps (`build_bundle`, `provision_guest_apps`), and one timed `curl -w '%{speed_upload} %{time_total}'` of a small PUT to the evidence endpoint for the missing upload figure;
5. once per run, `KF_BOX cores=$(nproc) cpu="$(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2)"` (CPU model is in no public log today).

Data this fills: substep times in `provision`, the cost of the second rebuild, the split of the Windows install, compressed image sizes, upload speed, cores and CPU model.

### 5.2 Step 2: sealed Windows base

DESIGN. A trusted build keeps an installed Windows base in private R2, keyed more broadly than the driver triple. Needs a check first: that the install is driver-independent. What is known: the install runs with no kf3 device (standard VGA) and before the NVIDIA driver (CODE READ, `windows_gate.sh`); what is not tested: that a base installed once boots and passes with every driver context. The source analysis lists items that must change (INFERRED, from reading): per-install secrets (win_password, harness ssh key, swtpm state) re-keyed or generated at first boot on the box; the evaluation image expires after 90 days, so a rebuild schedule is needed; the planner fixture assumes 35 GB for `windows_base`, an assumption that would put a download at about 12 to 15 min at 40 to 50 MB/s against the 39.1 min install median (INFERRED); the compressed size has never been measured (step 1 measures it). Distribution stays private (private R2, section 3 reasoning).

### 5.3 Step 3: the downloader

Section 1, built on `scripts/ci/kf_fetch.py`.

### 5.4 Step 4: CI-built attested binary and image

Section 2. The CUDA toolkit question remains an owner item: a prebuilt image that contains the toolkit conflicts with the rule that nothing from NVIDIA is stored by us; either the toolkit install (about 8 min per `build_bundle.log`, from the source analysis) stays on the box, or the owner relaxes the rule. Not decided here.

## 6. What this document does not decide

- Downloader bandwidth-cap default, the fallback compile deadline, and manifest file format.
- Whether the kf3 binary is per driver context (section 2.1).
- GitHub size limits and offline attestation verification (section 2.2).
- The CUDA toolkit versus the no-NVIDIA-storage rule (section 5.4).
