**STATUS: DESIGN-ONLY, 2026-11-08** for the Worker and release-tier rollout (WB1 makes no deployment/object/native claim; no
Worker setting sends the stage). **LIVE for the harness code** (merged with this document, GPU-free tests), and the
**first hardware run** is in section 8 (`traces/winapps_20261107/`). The harness is present in this tree; the owner reviews release enablement. Parent
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

## Owner policy, 2026-11-08 (DX1)

**CODE READ:** `gate_profiles.json` classifies all eight `overlay_yuv_*` entries, including probe and soak, as
`manual: needs real display`. They are outside every Vast profile, including explicit `ids:` lists, and remain in
stage logs (`APP_POLICY`), Markdown summaries and every structured report, even when the stage did not run.
The evidence template is in each manual item: physical monitor/connector, exact revision/die/drivers, same-GPU native
DIRECT-overlay result, command/version/hash, modes/transitions/latency, screenshots, duration and reset/Xid counts.
**CODE READ of the native Vast RTX 4090 record, 2026-10-10:** the recorded probe outcome is SETUP/refused (code 5),
with no DIRECT overlay and no scenario frames on the simulated display
([record](../../traces/windows_overlay_native_20261010/README.md)). No real-monitor native result is committed;
manual real-display qualification remains pending.

**CODE READ:** the public grader also refuses manual or known-unsupported gate expectations: each such id is
reported as a harness fault (NOTRUN), retains batched/isolated observations and gets zero pass credit.
A policy-only expectation is RETRY; a supported guest failure still takes precedence as FAIL.

**Owner policy:** `managed_t` and Linux `UnifiedMemoryStreams`, `UnifiedMemoryPerf`, `conjugateGradientUM`, `attach_verify`
are `known_unsupported` with the UVM reason. Reports say `unsupported (UVM)`; the owner holds UVM unmerged because the
per-fault GR hold remains unmeasured in this app qualification. Hardware timing and an explicit bound are required
before merge (`docs/OWNER_RULINGS.md` §E). `app_policy.py` keeps raw observed verdicts in notes
when reporting historical runs, and matrix tables retain their observed columns. Supported failures such as
`nbody_demo_gl` still gate. This supersedes the earlier "all nine failures remain in full" statement below.

**CODE READ:** policy rows are NOT_RUN classification metadata (SKIP for a historical test row), never passing results;
they record no app execution or timing sample. They are placed first under the 500-app report cap so truncation cannot silently drop them.
The payload's supported-app completeness, failure and host-Xid rules are unchanged. No Worker code changed, nothing
was deployed, and the stage remains disabled pending its first Worker run.

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
`tests/test_gate_profiles.py` keep it consistent with `apps.json` and `manifest.json`. **CODE READ / generated counts, DX1 2026-11-08 at `b370e4f12`:** 120
apps, in four classes.

| Class | Apps | What decides | Goes in an automated profile? |
|---|---:|---|---|
| automatable (legacy `open` app class) | 78 | dependencies are public `open_packages` and/or pinned `official_packages`; manual overlays and known-unsupported UVM are separate, and source/reference assets remain additional prerequisites | yes, with required media and prerequisites |
| owner decision | 29 | needs GravityMark, FurMark, GpuTest, the Vulkan SDK or the ffmpeg GPL build: free to download, storage rights still pending | no, until the owner rules |
| manual | 12 (+6 non-matrix items) | eight overlays need a real display; four Heaven/Valley rows have paid-automation/evaluation EULA limits; other items include 3DMark, Geekbench 6, TensorRT, Superposition, Xonotic, Q2RTX | never |
| known unsupported | 1 | `managed_t`: UVM merge hold pending per-fault GR timing and an explicit bound (unmeasured) | never; explicitly reported |

Open apps by category (**COMPUTED from `gate_profiles.py ids full`, 2026-11-08**): `ai` 1, `browser` 5, `crypto` 1, `cuda-demo` 7, `cuda-equiv` 27, `cuda-ladder` 3, `d3d` 8, `llm` 4, `opencl` 3, `opengl` 1, `probe` 3, `render` 1, `smi` 1, `stream-probe` 6, `stress` 1, `torch` 4, `vulkan` 2.

The guest is **offline** for every app: QEMU user networking with `restrict=on` (`win_vm.sh`), only the harness's ssh forward;
apps and models come from the app disk, Edge opens pages from `C:\kf\web`. **CODE READ.**

**Manual list and evidence template** (`gate_profiles.json` `manual`): a person installs the app, runs it by hand in the
gate guest and commits `traces/winapps_manual_<date>/<app>.md` with the revision, die, driver, the app's own result and
the host reference. The licence or key is never written down.

<a id="3-inputs-one-bundle-from-our-r2-through-the-worker"></a>

## 3. Inputs: public bundle and official run-time media

**STATUS: CODE + DESIGN, 2026-11-08.** This section incorporates the latest owner rulings. The permanent
private bundle, private R2 bucket/secrets and two-class publication workflow are **SUPERSEDED**. NVIDIA
software is never deliberately included in a distributed release ISO or uploaded to R2/GitHub. Public
release media contains only public-ok packages. Official run-time downloads use a transient second disk;
this is local working media, never a published bundle. No full media build, native run, upload, secret
configuration or Worker deployment is claimed by WB1.

**CODE READ:** `package_bundle.py` runs with Python 3.11+ standard library on Linux, Windows and macOS.
Application ZIP/7z/MSI/EXE/wheel originals ride unchanged on the public ISO, label KFAPPS. Only the pinned
official Zig compiler distribution is unpacked on the host; legal-text members are read into the compliance
inventory without extracting application payloads. All compiler invocations use direct argument lists,
never shell commands, a distro mingw package or host extraction/compression utilities.

```text
python3 -I scripts/bench/windows/appmatrix/package_bundle.py check
python3 -I scripts/bench/windows/appmatrix/package_bundle.py fetch --dl downloads
python3 -I scripts/bench/windows/appmatrix/package_bundle.py verify --dl downloads
python3 -I scripts/bench/windows/appmatrix/package_bundle.py build --dl downloads --out out-a
python3 -I scripts/bench/windows/appmatrix/package_bundle.py build --dl downloads --out out-b
python3 -I scripts/bench/windows/appmatrix/package_bundle.py compare-image --iso out-b/winapps-open.iso --receipt out-a/winapps-open.manifest.json --out out-b
```

**CODE READ:** outputs include winapps-open.iso, winapps-open.manifest.json, winapps-open.pin.line and the
compatibility bundle.pin.line. Each receipt lists **every ISO path and its actual SHA-256 and size**, including
original pinned artifacts, repository programs, guest files, legal texts, sources and generated metadata.
It records toolchain/source pins, per-file licence/comparison policy, Level 1 inventory digest, ISO digest,
container metadata and external packaging date/Python version. The date and Python version are not put
inside the ISO. A private release class is refused before download/build; no permanent private pin is emitted.

### 3.1 Reproducibility: file claim first, container equality best effort

**OWNER DECISION / CODE READ, 2026-11-08:** reproduction has exactly these levels:

| Level | Comparison | Reproduction result |
|---|---|---|
| **1: claim and gate** | Same complete file list and same SHA-256/size per file, including repository-built programs | A difference fails reproduction, with added/removed/changed paths. |
| **2: best effort** | Entire ISO container bytes also match | A difference with Level 1 equal is a WARNING, with container metadata differences; it is a builder bug to fix. |

**CODE READ:** `bundle_repro.py` compares inventories independently of image digests. A metadata-only
change in a date, publisher field, extent layout, system area or padding cannot fail Level 1 when the
complete payload remains the same in both namespaces. The independent ISO reader walks both primary
ISO9660 and Joliet directories, derives the permitted sorted short aliases, checks both endian path tables
and directory references, and requires identical logical file lists and payloads. A primary-only file,
redirected payload or swapped payload fails before comparison. Relocation of the same payload is allowed.
The reader snapshots descriptor fields, recording dates, extents and hashes of all non-payload regions.
`compare-image` prints the Level 1 result, any Level 2 metadata warning, and writes reproduction.json.
The second image is read against **its own** receipt before comparison; the first receipt supplies the
reference file inventory. Optional `--candidate-receipt` selects a second sidecar at another location.

**CODE READ / integrity is separate:** `verify-image` checks one stored image against its own image SHA-256,
size and file hashes. The harness's public downloader still checks the repo-selected ISO's own digest and
receipt. A corrupt download is an integrity failure; it is never excused as Level 2 variation. Do not use a
first build's ISO digest as the second build's integrity digest when assessing reproduction.

**CODE READ, 2026-11-08 / recipe binding:** `verify-image` additionally recomputes the embedded recipe
identity from its package/toolchain records, open_packages and checkout source/recipe files. Every intact
package and mirrored source must match its declared SHA-256/size. Bounded seekable ISO views replay the
licence recipe without host extraction; repository guest files, generated staging/notices/index and per-file
licences must match that recipe, and unlisted payloads are refused. Authorized publication also requires
all seven owned probes and the compiler notices, and checks the recorded recipe-source digests. The embedded
record must use the generated schema and exact fields; additional payload fields are refused. A self-consistent
sidecar is insufficient. The verify-image CLI and publisher supply the trusted checkout input digest;
publication checks it **before constructing a store client or making any store request**. An honestly hashed
but unreviewed recipe is also refused. Run the verifier from the matching reviewed checkout. Compiled probes still require independent reproduction and
native functional qualification: this check is not an attestation that a compiler produced their bytes.

**CODE READ:** all seven repository programs currently use `mode: sha256`. No real program has been marked
inventory-only. The pinned `bundle_toolchain.programs` policy supports a deliberately qualified exception:
a known repository program under tools/ may be marked `inventory-only`, with a nonempty reason and named
unchanged native functional checks. Its actual SHA-256/size remain in the receipt. Both inventories must use
the same reviewed policy; a candidate cannot waive a reference's byte check. Vendor inputs, legal texts and
other files cannot use this exception. The payload identity marker normalizes only these expressly waived
program hashes/sizes so it does not accidentally reinstate the waived comparison. The report names each
exception and requires functional validation; CI does not claim that an unrun GPU check passed.

Example policy shape (DESIGN example, not enabled in the committed manifest):

```json
{"programs":{"kf_glgears.exe":{"mode":"inventory-only","reason":"Document the observed cross-host difference and its investigation","functional_checks":["glgears_wgl"]}}}
```

**CODE READ:** flakiness is removed by construction: exact URL/SHA-256/size pins; no latest artifact URL;
hash-locked direct wheel URLs and no pip resolution; intact application archives; sorted entries; fixed
ISO timestamps/identifiers; normalized repository text; no host user/umask/date/locale fields; official pinned
Zig, normalized build paths, fixed SOURCE_DATE_EPOCH, deterministic target flags and zero PE timestamps.
Host/compiler caches are explicit. A vendor replacing pinned bytes or removing a pinned URL is
**INPUT_DRIFT**, naming the package, and fails loudly. No pin is rewritten and no newer version is substituted.
GPU-free tests exercise differing directories, umasks, TZ and locale; a Level 2 difference is diagnostic.
Full cross-host/native parity still requires coordinator evidence.

**CODE READ:** `input-hash` prints inputs=<sha256>, covering selected public pins, open_packages, builder/
writer/compiler/compliance/reproduction recipes, every ISO-carried guest/app/licence file, probe/ladder/
header/import-library sources and all toolchain/program-policy pins. Host paths, dates, Python version,
historical receipts, publisher/CI helper code and research-only metadata are excluded. Official driver pin
changes do not themselves rebuild the public bundle: app requirements reference stable catalogue groups,
and the official catalogue lives on the host.

**CODE READ / CI:** bundle-publish.yml remains **workflow_dispatch only**, with contents:read permission and
SHA-pinned actions. Build/verify is the first deliverable. A cheap plan compares inputs with the repo pin;
verify mode forces a rebuild. The secret-free job builds twice in fresh directories under differing
umask/TZ/locale, verifies each image's own integrity, and gates only on the Level 1 inventory. Verification
against a published release uses its complete manifest or `# inventory=<sha256>` in bundle.pin. An ISO hash
alone cannot establish Level 1; an absent reference fails setup before vendor downloads. When available,
the public manifest supplies a field-level container diff as well as the file inventory. A Level 2 warning
never changes reproduction's exit status. The local GUI's Compare reproduction uses the same comparison.

**COMPUTED, not measured on GitHub:** public two-build disk admission reserves original cache, both images,
legal/source copies, compiler scratch and margin (currently about 10.9 GiB). CI frees unused preinstalled
toolchains, measures free space and stops before download if insufficient; use a larger runner if needed.
The coordinator's approximate 14 GB default-free premise is not a measured runner result. No Actions media
artifact/cache is created. No vendor application code runs during packaging; the only vendor executable
is the hash-verified pinned compiler.

**DESIGN / optional publication:** publication remains disabled, requiring the explicit enable variable,
canonical master dispatch/push context and a protected master-only environment. No environment, secret,
variable, domain, bucket or enable flag was configured by WB1. Only the final optional public upload step
receives the public bucket's secret set; build steps reject credentials/run tokens. Forks and PRs cannot
publish. Future input-filtered PR builds and master pushes are documented intent, not enabled triggers.
Do not add pull_request_target. The determinant paths are root licence/notice files; appmatrix manifest,
profiles, apps, package_bundle.py, bundle_iso.py, bundle_tools.py, bundle_compliance.py, bundle_wheels.py,
bundle_repro.py, guest/**, tools/src/**; the CUDA import definition/minimal header and cup2/3/8 sources.
Pin/output/docs-only changes are not bundle determinants.

**CODE READ / coordinator manual upload:** first reproduce and inspect the files on a machine permitted to
perform the full build. The coordinator can upload the resulting public objects with their own transport,
or invoke publish_bundle.py --manual-presigned --out out-a after a caller supplies BUNDLE_UPLOAD_URLS and
the public custom-domain setting through the environment. This local mode accepts presigned capabilities
only, rejects CI/Actions, static credentials and run tokens, and never configures a token. WB1 does not
invoke it. The default publisher remains protected-CI only. Both modes reject transient/official content.

**CODE READ:** public keys are winapps-open/kfapps-<ISO SHA-256>.iso and
winapps-open/manifest-<ISO SHA-256>.json. Exact copyleft sources use
winapps-open/source-<source SHA-256>.archive in the same bucket/prefix and upload **before** object code.
The stdlib publisher uses bounded/resumable S3 multipart or caller-presigned plans, supports objects above
5 GiB, and requires create-only conditions for single PUT and multipart completion. Existing matching
objects are reused; conflicting keys fail. No existing object is overwritten/deleted. Provider URLs,
credentials and response bodies are not logged. Live R2 conditional-completion semantics/source access
remain unverified; check them before optional publication, and expire incomplete multipart uploads.

**DESIGN:** after a successful coordinator upload, verify the published objects and update bundle.pin with
the generated ISO SHA-256, inputs, Level 1 inventory digest and immutable keys. A later trusted automation
may open a draft PR against next after checking the reviewed tree still has those inputs; no PR-token
handling is implemented. A future OIDC presigner must validate issuer/audience, canonical master workflow,
protected-environment subject/run/expiry and exact public keys/digests/sizes/create-only conditions. No Worker
code or OIDC permission/route call is added. The old GitHub-Releases bundle store
reindertpelsma/win-gpu-image-private-store is superseded for this bundle; setup cache and WinPE are a separate
migration. WB1 does not move/delete those contents or migrate NVIDIA archives to R2.

<a id="31-owner-refinement-build-in-github-publish-immutable-objects"></a>
<a id="32-two-classes-licence-inventory-and-source-availability"></a>

### 3.2 Licence inventory and source availability

**CODE READ:** package class remains explicit in manifest.json. Only public-ok packages enter release media:

| Class | Packages | Handling |
|---|---|---|
| public-ok | python_embed, llama_vulkan, qwen_gguf, hf_model, vkpeak, blender, clpeak_ocl, hashcat, sevenzip, audited permissive hf_wheels | Unmodified pinned originals on KFAPPS; licences/notices and required exact source mirrors. |
| official-fetch | CUDA demo/runtime/NVRTC/CCCL, torch and related wheels, llama CUDA/cudart, clpeak CUDA, vc_redist, vulkan_rt, restricted HF extras, selected NVIDIA driver | Exact official download on the box host; transient KFAPPSP only; no R2/GitHub release object. |
| manual-only | Other licensed/owner-decision software and the separate GPL ffmpeg build | Outside both canonical automatic selections. |

This is engineering support, not legal advice. The older licence-inventory distribution value private-only
records a redistribution restriction; it does **not** authorize any private ISO/bucket publication.

**CODE READ:** each public build carries LICENSES/, a provenance index, THIRD_PARTY_NOTICES.txt and exact pinned
SOURCE/ archives. Each ISO file has machine-readable licenses metadata. Upstream licence texts come from
hash-checked archive members or independently pinned official licence files; no legal text is silently omitted.
Unmodified Blender 4.5.0, 7-Zip 26.04 and clpeak 3.0.1 sources are mirrored alongside their object code;
notices also give the exact upstream source locations. No GPL binary is modified. The written source offer
lasts at least three years after our last distribution and while covered support/spare parts remain offered.
**It is OUR obligation to retain mirrored source while distributing binaries and honor the full offer term.**
Upstream links alone do not discharge that retention obligation. Full transitive source/legal inventory still
needs inspection with the full real artifacts; no qualified licence clearance is implied by this engineering.

**MEASURED, 2026-11-07, commit 4e8aa9a58 (manifest.json public_audit records):** eleven public HF wheels were downloaded from existing pins, hash/size
verified and inspected without execution/extraction, including seven native members. Their exact metadata/
member audits are in manifest.json and are rechecked at build time. Unknown/nonpermissive metadata or
NVIDIA library names/references are refused for public wheels. Certifi/tqdm remain official-fetch extras.

### 3.3 Official downloads, transient media and J2 driver selection

**CODE READ:** gen_official.py generates official_fetch.json and official_requirements.txt from the main
manifest. Each entry has an ID, official URL, SHA-256, size, licence ID, class, guest destination, stage
recipe and guest-only installer argv. Catalogue groups bind apps.json requires.official_fetch links to
exact files; dependency closure and the separated HF extras are explicit. Wheel lock lines use direct
artifact URLs with hashes; neither the host nor the guest resolves dependencies with pip.

**CODE READ:** winapps.py on a real box host chooses the selected apps' groups and an explicit
--official-driver VERSION (or KF_WINAPPS_OFFICIAL_DRIVER supplied by J2). Only catalogue versions are
accepted; currently 580.88 is pinned from the existing win_vm.sh record. Additional release-matrix versions
must have exact official URL/SHA-256/size records; an absent pin is MEDIA_DRIFT, never a fallback to another
version or latest URL. --guest-driver remains the separate kf3 ABI setting and is not the Windows installer
selector. official_runtime.driver_plan gives J2 guest staging/installer argv for its **pre-gate** driver
phase. The app harness never switches a driver after the parent Windows gate; it checks the guest's
installed version against the host-selected version and returns NOTRUN on a mismatch.

**CODE READ:** the host hash-verifies the pinned scripts/ci/kf_fetch.py source before invoking it with an
argument list, eight parallel range streams and exact size/hash pins. No Worker auth flag or credential/run
token is passed; child environment is minimal and downloader output is captured. Each original is checked
again, and hashes of the bytes actually streamed into the transient ISO must match the pins. No application
is executed or extracted on the host. The offline guest stages archives using bundled 7-Zip or built-in
tar/msiexec. J4's run-time client is Linux-box specific; the public builder remains portable.

**CODE READ:** a context-managed directory outside the run's evidence tree holds downloads, the deterministic
KFAPPSP ISO and its local manifest. It spans every guest and their teardown and is removed on normal return
or exceptions; a destroyed box/runner discards the run's storage on abrupt termination. There is no permanent
second ISO/pin or private upload secret set. KFAPPS K: and KFAPPSP P: attach read-only. A guest label alone
is insufficient: the controller must have attached its own generated media. Missing media yields NOTRUN /
RETRY MEDIA for affected apps; official URL/hash/size failures and wrong selected-driver versions yield loud
MEDIA_DRIFT, never PASS. Public-only apps can run without the second disk; --no-official-fetch leaves
required inputs unavailable and cannot turn those apps into passes.

**CODE READ / optional monthly check:** dispatch mode verify-official downloads/hashes every official entry
in temporary storage, removing each after validation; verify mode also selects this job independently of
public verification. No ISO, Actions artifact/cache or upload is created by this job. The monthly cron
remains disabled until owner review; the same command is its intended scheduled verification path:

```text
python3 -I scripts/bench/windows/appmatrix/official_runtime.py check
python3 -I scripts/bench/windows/appmatrix/official_runtime.py verify
```

**OWNER POLICY / CUDA EULA:** the downloader is the person or CI operator that runs official fetch/verify
for private use; that operator must accept the pinned CUDA EULA. A CI download does not authorize public
redistribution. The guest stays offline. Native driver installation/staging, full vendor URL availability,
full cross-host file inventories and live object-service behavior remain coordinator verification work.

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
| `full` | every supported open app (78 after DX1 policy classification) | tier large (release tag) | 300 min **(budget sufficiency is unverified; missing new source/reference inputs cannot pass)** |

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
   on a Vast box: their applicability was an open owner decision, resolved by the DX1 policy above. Historical policy before 2026-11-08: all nine failures remained in `full`. Superseded by the DX1 owner policy above: eight overlay entries are manual, `managed_t` unsupported, and `nbody_demo_gl` stays gating.
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
