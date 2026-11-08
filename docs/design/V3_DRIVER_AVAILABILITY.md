STATUS: DESIGN-ONLY, 2026-11-08. GPU-free publication research and offline planning; no release-driver sweep has run.

# NVIDIA release driver availability and selection

**MEASURED, public HTTP on 2026-11-08:**
[`../driver-availability.json`](../../ci/vast-broker/driver-availability.json) contains 297 exact
OS/version records across 14 branches. Its 625 metadata request receipts include
378 HEAD requests, checked between 01:08:15Z and 01:21:32Z. A publication probe
does not qualify a driver, die, kernel, guest ABI or workload.

| OS | SHA256 + URL + size pinned | Available, no active SHA256 pin | HEAD 404 at every recorded candidate URL |
|---|---:|---:|---:|
| Linux x86_64 full `.run` | 95 | 17 | 63 |
| Windows x64 desktop WHQL | 3 | 119 | 0 |

**MEASURED public HTTP on 2026-11-08:** Linux pins use NVIDIA's exact-filename
`.run.sha256sum` sidecars; the requests and responses are recorded in
[`observations.json`](../../ci/vast-broker/driver-availability/observations.json).
For each available Linux candidate the generator also tries `sha256sum.txt` and
`SHA256SUMS` when necessary. A hash for aarch64 or `-no-compat32` cannot pin the
full x86_64 installer; MD5 cannot substitute for SHA256. The 17 available Linux
records without a retrieved SHA256 remain `UNPINNED:no-sha256`.

**MEASURED:** Windows 595.97 was downloaded once from its official desktop URL,
hashed as 957,358,592 bytes, and deleted before the next operation. Its SHA256 is
`979ed00fea181c786f608967377d6d83ac82e6368275994a4182ec79d97b3122`.
**CODE READ:** Windows 580.88 and 580.97 reuse J2's streamed hash receipts at
`b9d57438d6788c8c6de8c60c087194b269f8d6c6`,
`ci/vast-broker/release-matrix.json`; fresh HEAD sizes matched those receipts.
The receipts retain their 2026-11-07 date with `time_precision: day`; their exact
measurement time is unknown. No installer bytes are in this change.

## Discovery and schema v1

**CODE READ:** [`generate.py`](../../ci/vast-broker/driver-availability/generate.py) uses Python's standard library.
Its normalized HTTP observations are [`observations.json`](../../ci/vast-broker/driver-availability/observations.json);
curated citations, historical references, source fingerprints and the imported
hypothesis are [`sources.json`](../../ci/vast-broker/driver-availability/sources.json). The observations allow exact,
offline regeneration of the catalog rather than requiring another network run.

**CODE READ — NVIDIA source publications:**

- [Linux x86_64 directory](https://download.nvidia.com/XFree86/Linux-x86_64/),
  [Linux display archive](https://www.nvidia.com/en-us/drivers/unix/linux-display-archive/)
  and [data center releases.json](https://docs.nvidia.com/datacenter/tesla/drivers/releases.json).
  The generator retains the exact published Tesla CDN URL, including its
  hostname, instead of assuming another CDN hostname has the same contents.
- [Windows branch history](https://www.nvidia.com/en-us/drivers/rtx-enterprise-and-quadro-driver-branch-history/),
  its linked `GetDownloadDetails` API results, and the public
  [driver search service](https://gfwsl.geforce.com/services_toolkit/services/com/nvidia/services/AjaxDriverService.php?func=DriverManualLookup&psid=107&pfid=877&osID=57&languageCode=1033&beta=0&isWHQL=1&dltype=-1&dch=1&sort1=1&numberOfResults=20&release=570&upCRD=0).
  NVIDIA's driver-search JavaScript supplies the Game Ready/Studio query flags.
  Search is performed per branch, for both types. No response reached the
  20-result cap. Unsuccessful or saturated queries become `discovery_gaps`.
- **CODE READ — archived API responses in `observations.json`:** 41 Studio
  result occurrences did not assert `IsWHQL=1` and are
  recorded in `discovery_exclusions`; all their exact versions also occur in
  admitted WHQL results. A `whql.exe` filename alone does not override the API
  flag. GeForce and RTX Enterprise package lines remain distinct.

**CODE READ:** 168 historical Linux tag candidates, plus their previously
published installer URLs, are imported from the nvkvm-pv availability study and
kayfabe's `tools/drivermatrix/tags.txt`. The historical study reports 87 of 216
tags without public installers. Its old verdicts are not copied: every candidate
in this catalog has a new HEAD observation. A tag remains a candidate until an
installer URL answers. A 403, timeout or missing Content-Length is UNKNOWN;
only a 404 is absence at the recorded URL. All 63 currently unavailable records
have only 404s across their recorded candidate URLs.

**DESIGN:** Schema v1 records `os`, exact `version`, explicit `branch`, official
`url`, `sha256`, byte `size`, release date when published, HEAD result/date,
checksum provenance, candidate/source URLs and scoped repository test records.
`PINNED` requires all of URL/SHA256/size and HEAD 200. Available Windows records
without a computed SHA256 are `UNPINNED:needs-download`. `known_bad` and
`core_reference_versions` start empty. Version bounds are the numeric first/last
**available discovered** version per OS, not a claim about the branch's original
GA date, end of support, or completeness of NVIDIA's entire publication history.

## Branches and family floors

**CODE READ — catalog snapshot dated 2026-11-08 and repository status:**

| Branch | NVIDIA designation | Linux pins | Available Linux without SHA256 | Windows pins | Repository status |
|---|---|---:|---:|---:|---|
| R535 | LTS | 20 | 5 | 0 | referenced-only |
| R545 | new-feature | 3 | 0 | 0 | referenced-only |
| R550 | production | 14 | 3 | 0 | referenced-only |
| R555 | new-feature | 4 | 0 | 0 | referenced-only |
| R560 | new-feature | 3 | 0 | 0 | referenced-only |
| R565 | new-feature | 2 | 0 | 0 | referenced-only |
| R570 | production | 12 | 5 | 0 | known-good |
| R575 | new-feature | 5 | 0 | 0 | known-good |
| R580 | LTS | 15 | 3 | 2 | known-good |
| R590 | new-feature | 2 | 0 | 0 | known-good |
| R595 | production | 9 | 0 | 1 | known-good |
| R610 | new-feature | 3 | 0 | 0 | known-good |
| R615 | new-feature | 2 | 1 | 0 | referenced-only |
| R620 | LTS | 1 | 0 | 0 | never-tested |

**CODE READ:** The data center designations above come from NVIDIA's dated
`releases.json` response; R545 and R555 use NVIDIA's
[R545](https://www.nvidia.com/en-us/drivers/details/212978/) and
[R555](https://www.nvidia.com/en-us/drivers/details/228179/) RTX Enterprise
new-feature descriptions. The source domain is recorded per branch: a data
center LTS designation is not silently attributed to every package line.

**CODE READ:** `known-good` means a named historical revision and trace passed
the **scoped test in that record**. Examples are R570 host 570.148.08 at
`1f3e4fc4` (GA102), R595 host/guest 595.91.07 at `714ec29d3` (GA104), R610 guest
610.57.04 at `1837166d`, and Windows 580.88 at `a5071c24` (AD102, non-strict
LiveKernelEvent 193 handling). R615 has recorded L4 host-control probes and
remains `referenced-only`, with no qualifying full driver pass imported.
It does not certify a whole branch, another die,
Windows or Linux interchangeability, or the current tree. `last_tested` is the
latest **imported dated passing record** for that OS/version; null means no such
date was imported, not proof that nobody ever tested it. References to Windows
595.97 and unsuccessful older bring-up remain references, not passing evidence.

| Family | Minimum stated by cited NVIDIA source | Scoped sweep floor | Evidence checked 2026-11-08 |
|---|---|---:|---|
| Turing | UNKNOWN | 535 | [535.54.03 support list](https://download.nvidia.com/XFree86/Linux-x86_64/535.54.03/README/supportedchips.html) includes RTX 2080 |
| Ampere | UNKNOWN | 535 | Same NVIDIA page includes RTX 3090 |
| Ada | UNKNOWN | 535 | Same NVIDIA page includes RTX 4090 |
| Blackwell | R570 or higher for CUDA | 570 | [NVIDIA-authored migration guide, 2025-01-23](https://forums.developer.nvidia.com/t/software-migration-guide-for-nvidia-blackwell-rtx-gpus-a-guide-to-cuda-12-8-pytorch-tensorrt-and-llama-cpp/321330); [570.86.16 product list](https://www.nvidia.com/en-us/drivers/details/240524/) includes RTX 5090/5080 |

**DESIGN:** The first three scoped floors are the lower edge of this requested
sweep. Their representative support-list entries do not establish family-wide
global minima. Blackwell's R570 floor is necessary, not sufficient for every
later Blackwell die or feature. These family-only inputs cannot replace exact
product support, open-module requirements, kernel build checks or kayfabe ABI
admission before a real installation.

## Imported hypothesis, with falsifier

**HYPOTHESIS imported from nvkvm-pv, not applied to kayfabe selection:** its
selected host ioctl layouts divide into ABI_525/535/545/550/570/580/610 groups.
The catalog preserves the source revision/fingerprint, the non-monotonic
535.43.x split and the 550.40.53 split. Versions after 610 are extrapolation in
that source, not measured equality.

**Falsifier for interchangeability:** any kayfabe control ID, field layout, RPC
or behavior differs inside a proposed group. **CODE READ counterevidence:**
[`V3_CONSTANT_STRATEGY.md` section 1a](V3_CONSTANT_STRATEGY.md)
records renumbering at 555.42.02 and an in-branch 24/28/32-byte control layout;
[`V3_DRIVER_MATRIX.md`](V3_DRIVER_MATRIX.md) records
in-branch R580 changes. Consequently `used_for_selection` is false, and the
validator rejects changing it to true. No ABI group can remove a branch from
the requested branch-by-family coverage.

**Falsifier for numeric Windows branch intervals:** an official Release field
assigns a version outside the interval. **CODE READ — official Release fields
in the archived API responses in `observations.json`:** NVIDIA assigns Windows
576.02 to R570 while 576.28 belongs to R575. Both are regression fixtures.
Windows branch assignments therefore carry explicit official evidence instead
of being computed from the version's leading digits.

## Offline selection contract

**DESIGN; implemented and fixture-tested:**
`selectDrivers(seed, boxes, table)` in [`validate.js`](../../ci/vast-broker/driver-availability/validate.js) accepts a string
or safe-integer seed and boxes `{id, family, role}`. An optional `product_line`
is `geforce` by default or explicitly `rtx-enterprise` for a corresponding box.
It returns one host/Linux-guest/Windows-guest triple **for every supported branch
on every box**. Below-floor branches appear in `excluded_branches`, with a reason.
The output is a proposed availability plan, not an instruction to install it.

- Every selected driver is pinned, HEAD 200, in its requested OS/package line,
  and outside `known_bad`. No unpinned record becomes a fallback.
- Exact-version ownership spans **all boxes, OS axes and branch runs**. The
  omitted Windows patch is normalized to zero; padding does not create another
  version. Host, Linux guest and Windows guest also use distinct versions within
  a box.
- The only configurable sharing exception is an explicit
  `{family, os, version}` core reference. Both boxes must have role
  `core-reference`, the same family and OS, and different IDs. An ordinary box,
  another family or two axes of the same box cannot borrow this exception.
  No exception is currently designated.
- Candidates prefer null `last_tested`, then the oldest imported passing-test
  date. A specified FNV-1a rank of the seed, box, axis and exact version breaks
  ties. Boxes and branches are canonically ordered. There is no clock, I/O,
  random source or input mutation in the function.
- Augmenting assignments can move an earlier choice when a later slot is scarce.
  Exhaustion throws `INSUFFICIENT_PINNED_VERSIONS`, never a partial passing plan.
  The function does not remove a branch to reduce the required version count.

**CODE READ — catalog snapshot dated 2026-11-08; INFERRED consequence:** R620
currently exposes one
Linux version and two Windows versions. A four-family release cannot use
distinct exact R620 versions on four boxes under the requested rule. R565,
R590 and R615 also have very small Linux pools. Most Windows versions need
hashing before selection. These shortages remain blockers; no release profile,
Worker handler or pass rule is changed by this PR.

## Regeneration and checks

**CODE READ:** from the repository root:

```sh
# No network; output must be byte-identical to the committed catalog.
nice -n 19 python3 ci/vast-broker/driver-availability/generate.py --offline

# Fresh public discovery, HEADs and small Linux checksum files only.
nice -n 19 python3 ci/vast-broker/driver-availability/generate.py

# Explicit missing Windows hashes: at most three versions per invocation.
# A nonblocking lock serializes hashers; each temporary file is deleted.
# Refuse above 15 GiB or if the download would leave <6 GiB free on /.
nice -n 19 python3 ci/vast-broker/driver-availability/generate.py --hash-windows VERSION

nice -n 19 node ci/vast-broker/driver-availability/check.mjs
nice -n 19 node --test ci/vast-broker/driver-availability/selector.test.js
nice -n 19 python3 ci/vast-broker/driver-availability/test_generate.py
nice -n 19 python3 ci/vast-broker/driver-availability/mutants.py
```

**CODE READ:** `--cache /tmp/DRV1-http` is an optional public-text/HEAD cache for
research, not a freshness assertion. Omit it to make new requests. No default
path downloads an installer. `check.mjs TABLE.json BOXES.json SEED` also prints
a proposed selection or exits nonzero with the blocking reason.

**MEASURED GPU-free:** 87 selector tests include an exhaustive independent
assignment oracle over 108 small pools; 55 Python tests cover parsing, exact
checksum binding, provenance, HEAD classification, bounded hashing/deletion,
and byte-identical offline regeneration. The mutation receipt is
[`validation.json`](../../ci/vast-broker/driver-availability/validation.json). Every listed mutant must have at least two
distinct named killers; a source/test edit during the mutation run invalidates
its result. The runner explicitly requests TAP instead of relying on Node's
default reporter. MEASURED, first GitHub run at `6076492b7`: all ordinary tests
passed, but the default spec reporter yielded zero parsed JavaScript killers,
so the mutation gate failed. Two runner contract tests cover explicit TAP on
the baseline and mutant paths. Tests and mutants in the dedicated GitHub
workflow run offline. The workflow also installs the same pinned Prettier3.6.2
and Black25.1.0 used by the shared broker CI step and checks this tooling's
formatting before the tests. MEASURED at `f66507d28`: the shared step stopped at
Prettier on the three DRV1 JavaScript files before any broker mocks ran; local
Black also rejected the three Python files. Formatting and the corresponding
exact mutation anchors were corrected without changing generator/test Python
ASTs or the two-killer rule. No AppArmor setting or cleanup was changed.

**Unverified:** actual installation/build on a target kernel; exact per-die
driver support; any new kayfabe host/guest ABI pair; the complete Cartesian
host-branch by guest-branch product; runtime integration with J2; a hardware
release sweep; universal historical completeness outside the cited, dated
public discovery responses. No compatibility claim follows from HEAD 200 or
from a historical branch status.
