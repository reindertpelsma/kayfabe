**STATUS: DESIGN-ONLY, 2026-11-07.** Proposed driver choices and a GPU-free planner contract;
no hardware run, rental, Worker contact or deployment. All driver choices need owner review.

# Release matrix: dies, die families and driver families

OWNER DECISION (2026-11-07, J2 lane instructions): eight dies, two per die family,
each with one FULL run, on eight parallel boxes. Turing uses TU106/TU116 (TU102 is
not offered with VM support); Ampere uses GA102 and GA104 or GA106; Ada uses AD102/AD104;
Blackwell uses GB202/GB205 once GB202 is stable. This document proposes GA104.
The exact selection lives in `ci/vast-broker/release-matrix.json`, alongside driver
choices, minima, subruns, cross runs, host capabilities and blockers.

CODE READ: `defaults.json` keeps the wider release admission lists, including GA106
and GB206. The matrix selects exact dies from those lists; availability does not
silently replace a selected die. `planRelease` validates the matrix and emits one
full leg per selected die. Windows runs twice on that box, preserving the existing
two-run verdict requirement. Fewer than eight configured parallel slots refuses
the plan. Existing cost, daily count, credit reserve, rolling spend and kill
switches still apply. CODE READ (RV1 fix, 2026-11-07): a proposed matrix is now
inadmissible in the trusted Worker (`MATRIX_NOT_EXECUTABLE`), before any provider
create. The offline cost/availability estimate remains visible, but `admitted`
and `execution_ready` are false. The payload refusal remains a second boundary.

## Deferred policies (owner decision, 2026-11-07)

**STATUS: DESIGN-ONLY, 2026-11-07.** OWNER DECISION: defer the preferred
performance tier, the download-time price term (including its derate and
`SLOW_NETWORK` feasibility guard), and per-run offer-fact logging for calibration
until after the first real measured run. CODE READ: none of these policies is
implemented in PR #296; its effective price is compute plus storage plus the
profile's expected transfer charges amortised over the lease. The previously
tested implementation is isolated in draft PR #299 and its separate commits;
that PR is deferred, not part of the first release change.

MEASURED (owner-supplied dev-box experiment, 2026-11-07; coordinator addition 4):
R2 presigned range GETs against the private bench object reached **55.6 MB/s**
(about 445 Mbps) with one stream and **223 MB/s** aggregate (about 1.78 Gbps)
with eight streams over 2 GiB. The dev-box link may itself cap the eight-stream
result. MEASURED (2026-11-05,
[`traces/specviewperf_vast_20261105/`](../../traces/specviewperf_vast_20261105/README.md)):
the earlier Vast transfer of 42,002,015,727 bytes in 804 seconds was about
418 Mbps. INFERRED: derate 0.3 was a candidate from that observation versus
advertised 1562 Mbps; it is not an active policy or a calibrated general rule.
DESIGN-ONLY: use the first measured release run to review preferred CPU/disk/link
thresholds, an R2-limited time model and feasibility bounds, and the minimal
calibration facts to retain. No download-time or performance decision is approved
by these throughput observations alone.

## Proposed drivers

PROPOSED (all rows; minimum supported host/Linux/Windows driver for every die is
UNKNOWN). The combination means the ordered host + Linux guest + Windows guest
triple. All eight triples are unique, and both the host/Linux and host/Windows pairs are unique. Each flagship has newer versions than its
second die on all three axes; this is a candidate allocation, not evidence of
common OS adoption or compatibility. Those two properties remain UNKNOWN.

| Die family | Die | Role | Host | Linux guest | Windows guest media | Windows guest ABI |
|---|---|---|---|---|---|---|
| Turing | TU106 | flagship | 580.159.04 | 580.126.09 | 580.97 | UNKNOWN |
| Turing | TU116 | second | 580.65.06 | 580.65.06 | 580.88 | 580.65.06 |
| Ampere | GA102 | flagship | 580.173.02 | 580.159.04 | 580.97 | UNKNOWN |
| Ampere | GA104 | second | 580.82.07 | 580.82.07 | 580.88 | 580.65.06 |
| Ada | AD102 | flagship | 580.178.04 | 580.173.02 | 580.97 | UNKNOWN |
| Ada | AD104 | second | 580.94.02 | 580.94.02 | 580.88 | 580.65.06 |
| Blackwell | GB202 | flagship | 590.48.01 | 590.48.01 | 580.97 | UNKNOWN |
| Blackwell | GB205 | second | 580.105.08 | 580.105.08 | 580.88 | 580.65.06 |

CODE READ: every Linux version above is listed in `tools/drivermatrix/tags.txt`.
That is a source/layout candidate list, not a per-die supported-driver minimum.
The pinned host is 580.159.04 in `scripts/bench/box/provision_full.sh`;
Windows media is 580.88 and its guest ABI is 580.65.06 in
`scripts/bench/box/windows_gate.sh`. The ABI is distinct from both Windows media
and the Linux guest selection. No ABI mapping for Windows 580.97 was verified.

PUBLIC RELEASE NOTES, checked 2026-11-07: NVIDIA documents Windows 580.97 as
Release 580, dated 2025-08-12 ([release notes](https://us.download.nvidia.com/Windows/580.97/580.97-win11-win10-release-notes.pdf)).
This supplies the additional Windows candidate only; it does not qualify kayfabe,
establish a minimum, or establish that the version ships in a recent OS.

CODE READ, historical evidence: [the Blackwell record](../../traces/die_coverage_20261104/blackwell/README.md)
at `0c515b3b` records GB202 Windows r1 FAIL with bugcheck 0x113, then r2/r3 PASS
on the same box; GB206 and GB205 each passed twice. GB202 remains a release blocker
under the owner's decision. Neither those passes nor the older
[die family audit](V3_FAMILY_COVERAGE.md) qualifies these proposed driver triples.
`docs/STATUS_DETAIL.md` is now a historical page; the direct trace is cited here.

## Subruns and host capabilities

PROPOSED: a full run covers its own driver family (NVIDIA major branch). Each row
adds one Linux MIDDLE subrun for the other of driver families 580/590. A Windows
SMALL subrun exercises the other point version within driver family 580. Each
second die also receives its flagship's full driver triple as a cross run:
SMALL when the major branches match, MIDDLE when Linux crosses to driver family
590. Host driver changes belong to that cross run too, not just its guest version.

PROPOSED: no adjacent driver families share a middle subrun yet. A future sharing
proposal must cite a boundary review of GSP firmware, control-id renumbering and
within-branch layouts, using [the constant strategy](V3_CONSTANT_STRATEGY.md).
The data explicitly disables sharing until that evidence and grouping schema exist.
A shared run must never be inferred merely because two major numbers are adjacent.

OWNER DECISION: root hosts with kernel/vfio access can swap host drivers;
containers with KVM/NVIDIA devices and daemon accounts that forbid unloading cannot.
CODE READ: matching reads `can_swap_host_driver`; a fixed host must already provide
the full and cross-run host versions. Missing capabilities do not grant permission.
The Vast KVM adapter's capability is recorded as data. Admission rechecks matching
when offers are selected again; a plan is not an offer reservation.

CODE READ: each planned leg carries `drivers`, `subruns`, `cross_subrun` and the
complete `release_matrix` row. Batch persistence and retries retain the row.
The existing provider environment alphabet is preserved: `KF_RELEASE_SUBRUNS`
uses bounded base64url JSON. Version strings use `KF_HOST_DRIVER`,
`KF_LINUX_GUEST_DRIVER`, `KF_WINDOWS_GUEST_DRIVER`; a known Windows ABI uses
`KF_WINGATE_GUEST_DRIVER`. The payload maps the host choice to the installer's
existing `HOST_DRIVER` as well as the readiness check's `KF_HOST_DRIVER`.
Both bootstrap and payload validate the transport. Proposed execution produces
`release_retry_matrix_proposed` before provisioning; it cannot silently run the
old media or skip subruns and report coverage.

## Owner decisions and first real eight-box run

PROPOSED decisions, with recommendations:

1. Approve the eight-die selection; recommend GA104 as Ampere's second die and keep
   GA106 as an explicit alternative requiring a matrix edit.
2. Review every proposed driver triple and verify recent-OS/common deployment;
   recommend replacing candidates as needed rather than treating these as measured.
3. Verify per-die minima for all three axes and verify the Windows 580.97 ABI;
   recommend keeping UNKNOWN until supported by versioned public sources or tests.
4. Resolve GB202 Windows 0x113 and record fresh-box stability evidence; recommend
   retaining the blocker rather than substituting GB206 under the same release claim.
5. Approve runtime/profile definitions and revised cost/time estimates; recommend
   retaining all existing spend and verdict gates.

DESIGN-ONLY first-run requirements: implement selected Linux fat/thin guest staging
(the existing `KF_GUEST_DRIVER_DIR` selects a staged thin-guest driver); obtain and
pin the selected Windows media and ABI; build the MIDDLE/SMALL dispatcher and cross
host swap/restoration; record observed drivers and a verdict for every planned
subrun. CODE READ: the updated classifier and release reducer now require that
coverage; missing rows are RETRY/INCOMPLETE, mismatched driver facts are
UNATTRIBUTED, and attributed planned failures are sticky FAILs. Recheck `docs/RELEASE_CLAIMS.md` for every claimed stage.
Then review the exact Worker bundle, separately authorize deployment, enable the
release switch, provide eight eligible VM hosts, set the account concurrency to at
least nine (one slot stays reserved), and authorize sufficient cost/daily/rolling
limits. Capture same-box native references, exact revision, OS/media hashes,
GSP versions, per-subrun results and all guest/host fault deltas. No such run was
performed in J2. The payload refusal must remain until these execution requirements
are implemented and tested; this PR supplies the plan and environment contract.

## GPU-free verification

CODE READ: `node ci/vast-broker/validate-release-matrix.mjs [candidate.json]` validates
schema, die coverage, driver uniqueness, minima, subrun boundaries and cross runs.
`node ci/vast-broker/release-plan.mjs --market <offers.json>` prints an offline plan
without renting. `node ci/vast-broker/release-matrix-mutants.mjs` checks isolated
mutants with two named test killers each. Node fixtures exercise real planning and
mocked batch transport; Python tests exercise bootstrap/payload validation and the
proposed-execution refusal. These checks supply no GPU or driver compatibility evidence.
CODE READ: `release-safety-mutants.mjs` additionally checks the RV1 admission,
receipt grading and SPEC scheduling rules, each with at least two named killers.
Tests use an explicitly synthetic executable matrix and future-capable payload;
production remains proposed and the current payload advertises no executor.
No mock success is evidence that the executor exists.

## Profile resources and capped network pricing

OWNER DECISION (2026-11-07 coordinator addition): the large profile caps each
inbound and outbound GB at USD 0.02; small and middle retain USD 0.005/GB.
OWNER UPDATE (2026-11-07): USD 0.02/GB supersedes the earlier USD 0.01 choice.
The unit-price ceiling is a generous sanity guard; expected network is already
included in effective hourly price and the total-run cap. A cheaper compute box
with dearer network can win when its effective price is lower and all caps hold.
OWNER-REPORTED snapshot of 50 VM offers (not independently measured in J2):
60% at or below USD 0.005/GB, 78% at or below 0.01, 92% at or below 0.02;
maximum 0.039. No new provider query was made.
INFERRED: adhoc retains USD 0.005/GB. These four resource profiles are data in
`defaults.json` (`payloadProfiles`), distinct from the existing check/sweep/release
budget classes. The stage table `payloadStageResources` derives each profile's
allowance. Disabled and optional stages remain budgeted, so enabling them does not
silently increase the price assumption. No new stage was run in J2.

INFERRED allowances, in decimal GB; every row has its source in the JSON data:

| Stage or shared allocation | Profiles | Inbound GB | Outbound GB | Disk floor GB | Basis |
|---|---|---:|---:|---:|---|
| setup_cache | all | 1 | 0 | 0 | INFERRED rounding/headroom; setup-cache byte records in `VAST_SETUP_CACHE_20261023.md` |
| provision | all | 20 | 0 | 120 | INFERRED cold OS, compiler, vendor packages and source from `provision_bench_tree.sh` |
| cuda_host, merge_bar, cuda_guest | all | 0 | 0 | 0 | INFERRED reuse; downloads charged to provision |
| apps | middle, large | 14 | 0 | 120 | INFERRED ceiling of MEASURED 13,082,387,956 compressed bytes, `traces/apps_linux_20261107/README.md` |
| windows_gate | small, middle, large | 35 | 0 | 120 | INFERRED upper endpoint of owner's 15–35 GB planning range; compressed image not measured in `V3_WINDOWS_ON_VAST.md` |
| windows_gate_2 | large | 0 | 0 | 0 | INFERRED reuse of the first install and media |
| specviewperf | large | 43 | 0 | 220 | INFERRED ceiling of MEASURED 42,002,015,727 bytes, `traces/specviewperf_vast_20261105/README.md`; 220 GB floor is owner's planning rule |
| windows_apps | all (adhoc reserves headroom only) | 8 | 0 | 0 | INFERRED ceiling above the 6–7 GB bundle estimate in `docs/DOWNLOADS.md`; bundle pin remains empty |
| media_and_margin | all | 9 | 2 | 0 | INFERRED remaining media, retry headroom and output; split from the prior 17 GB shared apps/bundles allocation |

CODE READ: the resulting profile reservations are:

| Resource profile | Disk GB | Inbound GB | Outbound GB | Maximum USD/GB in either direction |
|---|---:|---:|---:|---:|
| small | 120 | 73 | 2 | 0.005 |
| middle | 120 | 87 | 2 | 0.005 |
| large | 300 | 130 | 2 | 0.02 |
| adhoc | 120 | 38 | 2 | 0.005 |

INFERRED: these are conservative planning allowances, not measured total traffic
or proven minimal disk sizes. The SPEC trace records a 300 GB rental and a 220 GB
guest-disk setting; the configured large host disk is 300 GB. A future larger
stage or a repeated full media download needs a new allowance review. Actual
traffic is not byte-metered by this Worker; the per-GB price cap is enforced.

CODE READ: `scripts/ci/profile_resources.py` rejects a payload stage constructor
without a table entry, unresolved dynamic stage names, missing source labels,
profile sums that disagree with the stage rows, disks below the table floor and
invalid per-GB cap values. Node contract tests pin the owner-selected cap values. It expands the existing literal
host-stage loop and includes conditional stages. The CI job and Python tests run
this guard; a future download stage must add and review its table row.

CODE READ: effective hourly price is compute base plus profile-sized storage,
plus `(inbound allowance * inbound unit price + outbound allowance * outbound
unit price) / hard lease hours`. Worst-case reservation includes the compute and
storage cron margin and the network allowance once. Search, disk allocation,
offer rejection, ranking, live-instance rechecks and release estimates all use
the selected resource profile. Cross-profile offer caches refresh when disk or
unit-price bounds change. A cron-started batch retry constructs its provider
client from that batch's resource profile. Legacy volume overrides can increase
allowances; legacy rate overrides can tighten caps. Neither can under-budget the
stage table or relax its per-GB limit.

INFERRED worked example, same country, five-hour lease, no outbound charge and
USD 0.12/GB/month storage: A has compute USD 0.10/h and inbound USD 0.005/GB; B has
compute USD 0.20/h and free inbound. Small's 120 GB disk and 73 GB inbound make
A USD 0.193/h versus B USD 0.220/h, so A wins. Large's 300 GB disk and 130 GB
inbound make A USD 0.280/h versus B USD 0.250/h, so B wins. Tests pin this reversal
in both offer selection and release-leg sorting while preserving the release
planner's country preference. Mutants must be killed by at least two named tests.

## RV1 admission and receipt corrections (2026-11-07)

MEASURED by RV1's GPU-free probes on 2026-11-07 at commit `14d22c916`: a proposed
matrix admitted eight mock rentals, ordinary protocol-7 receipts could certify
PASS, an accepted attributed `windows_subrun_580` FAIL became RETRY and a ninth
clean rental produced PASS, and an invalid runtime matrix did not block that
cron retry. Sources: the coordinator's RV1 review and the two review probes;
J2 reproduced all three before changing these rules. These were mock provider
creates, not paid rentals or hardware measurements.

CODE READ: every release rental now validates the trusted bundled matrix, including
persisted pending legs and retries. Both root and die statuses must be
`executable`, blockers must be empty, minima and Windows ABI must be known, and
the persisted selected die row must still match trusted data. A request, flag or
persisted snapshot cannot approve a proposed or changed matrix. Cleanup still
runs before batch admission, including when the matrix has become invalid.

CODE READ: selected-commit protocol 7 is necessary but insufficient. Immutable
GitHub source at the selected SHA must declare `BOX_RELEASE_MATRIX_RECEIPTS = 1`
in the reporter and `PAYLOAD_RELEASE_MATRIX_EXECUTOR = 1` in the payload. The
current reporter implements bounded receipt transport; the current payload
explicitly declares executor capability **0**. Changing only matrix status or
reusing an old protocol-7 commit cannot start a release. A future executor must
be built and reviewed before declaring capability 1.

CODE READ: `matrix_receipts` is a closed, size-limited result field (schema 1,
selected die, up to 64 uniquely named receipts). Each receipt names the exact
host/Linux/Windows driver triple, Windows guest ABI and verdict. Required ids are
`full`, one `<os>_subrun_<version_with_underscores>` for every planned point
version, and `linux_cross`/`windows_cross` for a cross run. Receipts must match the
selected trusted drivers and ABI, reported full-run host driver, ordinary gate
rows and each planned subrun suite row. Passing summary gates alone cannot
certify coverage. These are bounded box-reported observations, not cryptographic
hardware attestation. The existing hostile-guest and native-reference rules apply.

CODE READ: every planned subrun is graded. Missing receipt or suite rows cannot
PASS; an attributed suite FAIL with a GREEN host reference makes the leg FAIL
and never earns a retry. Existing persisted ENDED outcomes are regraded from
accepted reports on a running batch's next tick, without creating duplicate
retries. A prior FAIL remains a release FAIL even if a later clean leg exists.
Completed-batch summaries recompute the release verdict under current trusted
policy rather than displaying a pre-fix cached PASS. The result-store path also
requires complete matrix coverage for release PASS,
so ordinary passing reports cannot bypass the batch rule through stored rows.

## SPEC lane coverage and separate leases (disabled)

OWNER DECISION / CODE READ: `releaseStages` remains empty. The active
`required_stage_rows_per_die` map is therefore empty. If SPEC is enabled, data in
`stageSlots.specviewperf` requires **both** `specviewperf_lanea` and
`specviewperf_laneb` on every die. The aggregate `specviewperf` row and either
single lane cannot certify both lanes.

CODE READ: enabling that same stage flag plans two FULL rentals per selected die
(16 leases), lane A first and lane B second, with one ordinary Windows gate per
lease. Both lease quotes are included in the plan and the existing total cost,
daily rental-count and spend caps still apply. The first eight lane-A rentals
run in parallel; each lane B waits until its die's lane A is settled, then gets
its own bounded start window. A second offered machine is preferred; with only
one physical machine, lane B is a fresh paid lease of that machine after the
first lease ends. No single `both` run is squeezed into a 300-minute box limit.
The second lease is an additional cost, not an infrastructure retry. Large
resources remain 300 GB disk with a 220 GB floor.

INFERRED: serialization preserves the eight-box release concurrency policy while
allowing a single offered machine to carry two separate leases. The plan does
not reserve availability; a missing second lease leaves coverage incomplete.
Real completion time, including matrix subruns and all downloads, remains
unverified and must be reviewed before enabling the stage or executor.

## Intended resource-price change recorded by RV1

CODE READ: small, middle and adhoc inbound reservations deliberately change from
15 GB to **73, 87 and 38 GB**, respectively. The additional transfer reservation
is `(new_GB - 15) * inbound_USD_per_GB` per lease, amortised over hard lease hours
in effective hourly price. Their outbound reservation remains 2 GB and their
unit-price guard stays USD 0.005/GB. Large uses its own 130 GB allowance.

MEASURED by RV1 on 2026-11-07 at commit `14d22c916`: synthetic fixtures raised quoted
prices by that transfer delta but left the tested offer ranks unchanged. This
is fixture evidence, not a claim that real-market ranks never change. CODE READ:
actual traffic is **not metered**; reservation GB are conservative inferred
budgets. A new download stage must enter the stage table and pass its guard.

CODE READ / INFERRED integration note (2026-11-07): merging Windows apps adds a
literal downloading stage, which the resource guard initially rejected as
unbudgeted. Its explicit 8 GB inferred bundle row is allocated from the existing
17 GB media/bundles reservation, leaving 9 GB for other margin. Totals therefore
remain 73/87/130/38 GB. Adhoc retains the same conservative reserved bundle
headroom while its payload does not enable Windows apps. The 6–7 GB source is a
planning estimate in `docs/DOWNLOADS.md`, not a measured artifact; the empty
`bundle.pin` confirms packaging is still unverified. Re-measure the packaged
bundle and revise this split if necessary before enabling the stage.

## Official driver installer pins (2026-11-07)

OWNER DECISION / CODE READ: `driver_installers` in the matrix is the catalog for
**every** full-run, cross-run and subrun host/Linux/Windows driver selection.
Entries are keyed by operating system and driver version, so version and installer
changes are reviewed data changes. NVIDIA installers are not mirrored in this
repository, R2 or evidence storage. Only disposable-box installation caches may
contain their bytes.

MEASURED metadata audit on 2026-11-07: eight Linux pins below came from NVIDIA's
version-specific `.run.sha256sum` files plus each installer's HTTP Content-Length.
These are official published digests, not an independent local hash of Linux
installer bytes. Both Windows installers were streamed directly from NVIDIA,
hashed and counted, then discarded without a file. Windows 580.88 agrees with
the existing pin in `scripts/bench/windows/win_vm.sh`. Full URLs, digests, byte
counts, check date and provenance are data in `release-matrix.json`.

| OS | Driver | Status | Installer bytes | SHA-256 / official source |
|---|---|---|---:|---|
| linux | 580.105.08 | PINNED | 396635119 | [`d9c6e8188672f3eb74dd04cfa69dd58479fa1d0162c8c28c8d17625763293475`](https://download.nvidia.com/XFree86/Linux-x86_64/580.105.08/NVIDIA-Linux-x86_64-580.105.08.run.sha256sum) |
| linux | 580.126.09 | PINNED | 396807358 | [`4cac53e48f8adff661d47c8788ed24059a248c9fd8098ceafd088a498986ec26`](https://download.nvidia.com/XFree86/Linux-x86_64/580.126.09/NVIDIA-Linux-x86_64-580.126.09.run.sha256sum) |
| linux | 580.159.04 | PINNED | 397162548 | [`c1e66761b088d17b3adf6cb6979de9af38be2684d102b001a176dcfafabdca1e`](https://download.nvidia.com/XFree86/Linux-x86_64/580.159.04/NVIDIA-Linux-x86_64-580.159.04.run.sha256sum) |
| linux | 580.173.02 | PINNED | 398312209 | [`8d8eb9001e05a9a8a663d3d5d304feb64ef2844ee185ccdfd952786820f46e1b`](https://download.nvidia.com/XFree86/Linux-x86_64/580.173.02/NVIDIA-Linux-x86_64-580.173.02.run.sha256sum) |
| linux | 580.178.04 | PINNED | 397214534 | [`5975a86ee45bffcb626f51ae33d1169b108186a2ea47ad651e72f13fa4b6d6f9`](https://download.nvidia.com/XFree86/Linux-x86_64/580.178.04/NVIDIA-Linux-x86_64-580.178.04.run.sha256sum) |
| linux | 580.65.06 | PINNED | 393832804 | [`04b10867af585e765cfbfdcf39ed5f4bd112375bebab0172eaa187c6aa5024ff`](https://download.nvidia.com/XFree86/Linux-x86_64/580.65.06/NVIDIA-Linux-x86_64-580.65.06.run.sha256sum) |
| linux | 580.82.07 | PINNED | 396359634 | [`061e48e11fe552232095811d0b1cea9b718ba2540d605074ff227fce0628798c`](https://download.nvidia.com/XFree86/Linux-x86_64/580.82.07/NVIDIA-Linux-x86_64-580.82.07.run.sha256sum) |
| linux | 580.94.02 | UNPINNED | UNKNOWN | [official candidate returned 404](https://download.nvidia.com/XFree86/Linux-x86_64/580.94.02/NVIDIA-Linux-x86_64-580.94.02.run) |
| linux | 590.48.01 | PINNED | 416273526 | [`b9e2f80693781431cc87f4cd29109e133dcecb50a50d6b68d4b3bf2d696bd689`](https://download.nvidia.com/XFree86/Linux-x86_64/590.48.01/NVIDIA-Linux-x86_64-590.48.01.run.sha256sum) |
| windows | 580.88 | PINNED | 890807536 | [`90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9`](https://us.download.nvidia.com/Windows/580.88/580.88-desktop-win10-win11-64bit-international-dch-whql.exe) |
| windows | 580.97 | PINNED | 890779664 | [`081878a323fc24b8ec4f7c9b0e72e22ad6f9ae73219a6fac324b59cd6258d076`](https://us.download.nvidia.com/Windows/580.97/580.97-desktop-win10-win11-64bit-international-dch-whql.exe) |

MEASURED on 2026-11-07: Linux 580.94.02 returned HTTP 404 from both the XFree86
candidate above and NVIDIA's `us.download.nvidia.com/tesla/580.94.02/` candidate.
It remains **UNPINNED**, with null hash/size and an explicit AD104 blocker. No hash,
size, alternate point version or approval was invented. The owner must verify
its official media or choose another reviewed driver before executable status.

CODE READ: the existing validator requires a bounded, unique catalog and complete
coverage of all selected versions, including subruns and cross runs. Binary URLs
must use HTTPS and exactly `download.nvidia.com`, `us.download.nvidia.com` or
`international.download.nvidia.com`, with no credentials, query, fragment or
non-default port. NVIDIA paths must name the selected version and installer type.
A Windows Catalog binary may instead use `catalog.s.download.windowsupdate.com`
or `download.windowsupdate.com` only with a separate official
`www.catalog.update.microsoft.com/ScopedViewInline.aspx?updateid=...` evidence URL
for that Windows driver. There are no Microsoft-hosted pins in the current data.
An executable matrix containing **any UNPINNED entry** is invalid, even if that
entry is unused. Pinning does not approve driver compatibility or build an executor.

CODE READ: plan legs expose their selected pins and `KF_RELEASE_DRIVER_PINS`
transports that exact subset to the payload. The reporter and payload check its
closed schema, official URLs, hashes, sizes and selected-version coverage. The
matrix host-driver path uses the common `release_driver_media.py` verifier before
hardware changes; it verifies existing cached bytes too. There is no mirror,
version, URL or checksum fallback in this path. A 404, redirect, hash mismatch or
size mismatch emits `DRIVER_MEDIA_DRIFT`, exits unsuccessfully and cannot become
READY through a previously working stock driver. The payload reports a failing
`driver_media_drift` row, and the Worker classifies it as RETRY / MEDIA_DRIFT,
including when ordinary summary rows claim PASS. A simultaneous attributed test or planned subrun failure remains FAIL; media drift cannot downgrade it into a retry.

CODE READ / DESIGN-ONLY: the verifier supports Linux and Windows installer pins,
but the matrix payload executor still declares capability 0 and refuses all
matrix execution. New guest media staging/dispatch still needs the reviewed
executor; this change does not claim to execute Windows or Linux guest sweeps.
No driver was installed and no hardware/VM/rental/Worker request or deploy was made.


## Mocked market and failure simulations (2026-11-07)

CODE READ: `test/simulation-market.test.js` runs fixed Mulberry32 seeds
**1337 through 1536** (200 seeds), each with eight candidates per die across all
four profiles. Markets vary compute prices, both transfer rates and speeds,
storage, disk capacity/throughput, reliability, CPU, hosts, machines, countries,
VM support and malformed fields. Configured and persisted bad host/machine ids
are excluded. The same market with reversed offer and object-field order must
choose the same ids. A data-based price oracle checks storage and expected transfer charges
independently of `offerPrice`. Deferred stacked-policy tests are outside PR #296.

CODE READ: the large loop uses production `OfferBook`, `planRelease`, `Ledger`
and `Vast` with the existing `MockCloud` transport; at most one selected rental per seed
keeps CI fast. It checks every admitted plan, every eligible quote and every
mock create, reservation, exact-id destroy and settlement. Refused plans may
still display an over-cap quote for operator review; they create nothing.
Only admitted plans/reservations must fit the cap. The full Worker orchestrator
is exercised separately for named failure cases, including eight-die partial
fleets, credit/daily exhaustion, restart and duplicate-machine offers. Lifecycle
cases use one-box admin batches through the same admission and teardown paths.
Foreign fixture instances survive; every owned rental and ledger liability must
be gone at completion. No failed/absent instance is promoted to PASS.

CODE READ: named cases cover one offline status flap (#278), twelve minutes in
`created`, vanished offers, missing VM support, absent dies (`NO_OFFER`), no SSH
callback, host reboot, third-party destruction, 5xx, synthetic timeouts, 429 with
Retry-After, partial account limits, credit/daily caps, persisted restart,
duplicate/late callbacks and backward/forward clock changes. The 429 case checks
safe retention and recovery with that header present; it does not establish an
adaptive Retry-After scheduler. SSH failure is modeled by absent box callbacks,
not a real SSH connection. No clock sleeps or external network are used.

CODE READ: the proposal, unknown compatibility and executor-capability-zero
fences remain. This branch has no download-time feasibility limit or R2 reference
in its offer admission policy; the corresponding tested policy is deferred with
PR #299. The dated owner decision above governs when they may be reconsidered.

CODE READ: run `nice -n 19 node --test ci/vast-broker/test/simulation-*.test.js`.
For a larger reproducible sweep, set `KF_SIM_SEEDS=1000 KF_SIM_START=1337` before
that command; counts are bounded at 10000 and seed starts at 1000000. CI uses the
fixed defaults. The simulation mutation runner uses named paired invariants,
requires two distinct killers and never changes production caps to get green.

UNVERIFIED / CODE READ: seeder selection/upload-pricing code is absent from this
branch. Seeder upload ranking and a seeder dying during consumer downloads need
the owner's implementation branch/PR reference; they are not claimed as covered.

## RV2 review contracts (2026-11-08)

CODE READ (RV2 review of `5e1fb9a3`, regression fixtures in
`ci/vast-broker/test/rv2-regressions.test.js`): owner stops and hardware attribution
precede media-drift classification. A known wrong die or compute capability cannot
fail the selected die merely because its report also names media drift. A matching
box with an attributable test failure keeps that failure when media drift coexists.

CODE READ: enabling SPEC requires exactly lanes `lanea`, `laneb`, required rows
`specviewperf_lanea`, `specviewperf_laneb`, and `requiresSeparateRentals: true`.
Missing, empty or altered contracts refuse planning, admission and verdict reduction;
there is no aggregate-row fallback. The disabled stage adds no required rows.
The redundant persisted copy of the required rows has been removed.

CODE READ: the shared resource accessor accepts only the four declared profile keys,
finite nonnegative inbound/outbound allowances, a positive integral disk allowance,
and a positive finite per-GB cap. Computed quotes must remain finite. The Python
stage-table guard also rejects nonfinite disk allowances before comparing floors.

CODE READ: `scripts/ci/release_driver_pins.py` contains only the pin contract. The
installer helper imports that contract directly; the reporter delegates to it lazily.
The stock-path checker and its inventory remain identical to `origin/next` at
`624bf1dfcf98fac362bc63478e7f9cdb65c4aedf`; no reference-class exemption or baseline
update is needed. Dormant download-time simulation branches were removed; download
time pricing remains DESIGN-ONLY as described above.
