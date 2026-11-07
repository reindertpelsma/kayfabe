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
plus transfer fees and a download-time premium divided by hard lease hours.
The premium is excess download seconds over the per-profile proposed 445 Mbps R2
reference, with expected speed capped by R2 reference times the data streams factor
(currently 1), valued at the
compute/storage hourly rate; see [the download model](V3_DOWNLOAD_PRICE.md). Worst-case reservation includes the compute and
storage cron margin and the network allowance once. Search, disk allocation,
offer rejection, ranking, live-instance rechecks and release estimates all use
the selected resource profile. Cross-profile offer caches refresh when disk or
unit-price bounds change. A cron-started batch retry constructs its provider
client from that batch's resource profile. Legacy volume overrides can increase
allowances; legacy rate overrides can tighten caps. Neither can under-budget the
stage table or relax its per-GB limit.

INFERRED worked example, same country, advertised inbound at least 4000 Mbps
(so no time premium with the current derate), five-hour lease, no outbound charge and
USD 0.12/GB/month storage: A has compute USD 0.10/h and inbound USD 0.005/GB; B has
compute USD 0.20/h and free inbound. Small's 120 GB disk and 73 GB inbound make
A USD 0.193/h versus B USD 0.220/h, so A wins. Large's 300 GB disk and 130 GB
inbound make A USD 0.280/h versus B USD 0.250/h, so B wins. Tests pin this reversal
in both offer selection and release-leg sorting; country preference is a tie-break
within the preferred performance tier in this stacked change. Mutants must be killed by at least two named tests.


## Preferred host performance before price (stacked follow-up)

OWNER DECISION (2026-11-07 coordinator update): apply a preferred performance tier
before price, then fall back to hosts meeting the existing hard floor when no
preferred host fits the caps. All four preferred dimensions must pass together.
The preferred tier supersedes country-first ranking: effective hourly price is
next, with country preference only a price tie-break. Country exclusions remain.
Explicit die preferences, die-family rotation and retry host/machine exclusions
still determine the candidate pool; they do not grant permission to weaken a floor.

PROPOSED data in `defaults.json`, under `preferredHostQuality`, independently
recorded for small, middle, large and adhoc:

| Execution profile | Status | dlperf / die median | Effective CPU cores | Disk bandwidth MB/s | Inbound Mbps |
|---|---|---:|---:|---:|---:|
| small | PROPOSED | 1.0 | 12 | 150 | 300 |
| middle | PROPOSED | 1.0 | 12 | 150 | 300 |
| large | PROPOSED | 1.0 | 12 | 150 | 300 |
| adhoc | PROPOSED | 1.0 | 12 | 150 | 300 |

PROPOSED: CPU, disk and inbound values are 1.5 times the committed default hard
floors (8 cores, 100 MB/s, 200 Mbps). The preferred GPU ratio is 1.0 rather than the
hard floor's 0.5. Reliability 0.98, CPU clock/RAM, outbound throughput and all other
admission requirements remain hard floors. A configured higher floor still applies.
These values are active candidate-ranking policy in this code, labeled PROPOSED
because they have not been calibrated by J2 hardware runs.

OWNER-REPORTED motivation, not independently measured in J2: SPEC took about
200 minutes on a fast host versus 275–305 on slow hosts, whose CPU/disk behavior
was slow despite their GPU score. The earlier [SPEC trace](../../traces/specviewperf_vast_20261105/README.md)
records substantial copying costs and overlay growth. INFERRED: GPU score alone
is insufficient to choose a fast full-run host; these thresholds are candidates,
not a demonstrated predictor or a promise of those durations.

CODE READ: verified positive dlperf samples produce the existing per-die median.
Offers must pass the hard floor, price/unit-price rules and capability checks
before ranking. The preferred test requires GPU ratio, effective cores, disk and
inbound throughput together. Default selection and release planning choose the
cheapest effective price within the best eligible performance tier. Expected
network and profile-sized storage remain in that effective price; a dear-network
host can win when its total effective price is lower within the same tier.

CODE READ: affordability is considered before tier preference. Runtime admission
filters against remaining profile/account daily, rolling and credit-reserve
headroom and the existing release cost guard, then rechecks ledger budgets before
allocation. If no offer fits, the original budget refusal is retained. Release
planning walks the matrix's deterministic die order, reserving the cheapest
possible remaining dies before preferring a host for the current die. It retains
all final plan refusal gates if even the cheapest complete plan cannot fit.
Runtime starts reserve the planned costs of other pending legs before consuming
the batch's remaining cap. This is a conservative deterministic allocation, not
a claim to optimize total benchmark duration or maximize preferred-host count.

CODE READ: each admitted run stores `offerFacts` in its ledger record. The facts
include `dlperf`, `die_median_dlperf`, `median_ratio`, `cpu_cores_effective`,
`disk_bw`, `inet_down`, `performance_tier` (preferred/floor), `resource_profile`
and the proposed threshold snapshot. `source: provider_offer` distinguishes this
metadata from observed benchmark results. The settlement queue snapshots those
facts for each provider leg; the publisher writes `offer_facts` into report.json
in both R2 and D1. Guest report claims cannot set or replace this field. Existing
reports without a snapshot publish null. The public API forwards the stored
report, so calibration can join the facts to its timestamps/stage durations.
Nonfinite or extreme numeric fact values become null rather than unbounded JSON.

PROPOSED owner review: approve or adjust all four per-profile preferred thresholds;
retain the hard floors and spend gates. Compare the recorded offer facts with
measured run/stage durations before promoting a proposed value to an established
performance claim. No hardware, provider query or deployment was performed here.

## RV1 admission and receipt corrections (2026-11-07)

MEASURED by RV1's GPU-free probes against PR #296's previous head: a proposed
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
is `(new_GB - 15) * inbound_USD_per_GB` per lease for the transfer term,
amortised over hard lease hours in effective hourly price. The stacked timing
model separately adds its nonnegative excess-download-time reservation. Their outbound reservation remains 2 GB and their
unit-price guard stays USD 0.005/GB. Large uses its own 130 GB allowance.

MEASURED by RV1 on its synthetic fixtures: these allowance changes raised quoted
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


CODE READ (2026-11-07 stacked integration): preferred selection budgets both SPEC
leases before choosing a die's pair. Remaining die reservations use the cheapest
valid two-machine pair, or two leases of a sole machine. A and B preserve the
preferred-tier/effective-price order within their combined cap; if no preferred
pair fits, an affordable floor pair can be selected. Every persisted leg carries
its own planned quote, including the deferred lane B. The total plan remains
subject to the same run, daily, rolling and credit caps. GPU-free tests cover
both floor fallback and a preferred A with affordable B; no extra stage is enabled.
