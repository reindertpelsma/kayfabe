**STATUS: DESIGN-ONLY, 2026-11-08.** Pure planner, schemas, CLI and GPU-free fixtures;
no assignment HTTP route, Python executor, artifact registry integration, rental or deploy.
The owner's latest decision replaces both the fixed tier layout and the Python-planner hold.

# Flat release requirements and executor assignments

OWNER DECISION (coordinator, 2026-11-08): the control plane is JavaScript. CI supplies
requirements; the Worker builds the plan. Python executes assignments and may use a tiny
local dependency resolver. Vast is one provider. The planner itself performs no I/O and
imports no Workers or Node APIs. Integration into `src/index.js` waits for its separate
refactor; this change does not modify that file or the existing rental/report routes.

CODE READ: the public module is `ci/vast-broker/planner/index.js`. It exports `plan`,
`validateInput`, `validatePlan`, `validateAssignment`, `withoutSeeds`,
`resolveDependencies`, `checkSchema` and `PlanError`. All functions are synchronous.
The Node boundary is `ci/vast-broker/kfci`; its file/stdin/stdout operations are outside
the pure module. The existing release planner remains the active Worker path.

## Version 1 contracts

CODE READ: `planner/schemas/contract.schema.json` contains closed JSON Schema 2020-12
`$defs`; individual catalog, requirements, limits, box, input, plan and assignment schema
files reference those definitions. Every wire object carries version 1. The module's
validator is a non-generating interpreter of the keywords used by these fixed schemas:
no eval, downloaded schemas or remote references. Semantic checks follow structural checks. Candidate entries in the input envelope are
checked individually against the box schema so malformed offers can be rejected without
discarding a usable market; malformed trusted policy/requirements fail the whole call.

CODE READ: the input contains a trusted catalog, trusted limits, normalized box/provider
facts, a tested `source_revision` and flat requirements. `catalog.revision` separately
identifies the catalog snapshot. Future Worker routes must supply catalog, limits, source
revision and provider facts from their own authorized state; CI cannot replace them by
posting the CLI's whole input envelope. The local CLI lets an operator supply that envelope
for their own boxes. It is not an authentication or approval boundary.

CODE READ: each requirement has exactly one target (`die` or `die_family`), one concrete
pinned driver triple or a branch/patch sweep referencing catalog driver-set IDs, a run-ID
list, a resource tier, evidence class and `must: true`. Scope is explicit on each run.
A die-family target may select any supporting die of that die family. Branch sweeps must
actually select the specified driver family on the axes used by each run; changing a label
cannot claim another branch. Driver compatibility and minima are catalog data, including
the owner's Blackwell floor of driver family 570 in the proposed example.

CODE READ: each requirement/run occurrence/driver-set combination has one coverage cell.
Repeated non-setup runs remain separate executions. A shared setup execution can satisfy
multiple setup requests, with each request named exactly once in its coverage list.
Coverage in a plan means planned work, not a result receipt or a release PASS. Future
integration must retain #296's matrix/capability, sticky FAIL and exact receipt rules.

## Runs, artifacts and ordering

PROPOSED estimates from the owner's catalog message (2026-11-08): provision 10 minutes;
merge bar 11; Linux image build 30; Linux apps full/middle 20/8; nested 10; Windows
install-and-gate 33 and subsequent gate 17; Windows apps small/mid/full 12/25/94;
SPEC lane A/B about 150/120; SPEC smoke 10; tiny Linux guest 5, tiny Windows guest 13,
and tiny host swap 12. Full guest Linux/Windows and host runs use the earlier proposed
15/35/20 minute subrun estimates. None is a new complete-run measurement in this lane.

INFERRED placeholders in the proposed fixture: kayfabe build 20 minutes and SPEC tree
preparation 30. The latter is included in the owner's 150/120 minute SPEC estimates:
`includes_setup` subtracts that producer's body from a lane body, then dependency resolution
adds the tree setup once per box. It never subtracts the same producer twice merely because
it produces multiple needed artifacts. These splits need stage traces before approval.

CODE READ: artifacts name a unique setup producer and either box-wide or driver-triple
reuse. All outputs of one producer have the same reuse scope in version 1. A Windows base
uses the complete driver triple as a conservative cache key; an open image and kayfabe
binaries can use box scope when the catalog explicitly declares compatibility. Requested
runs are ordered deterministically within evidence/driver groups, with dependencies first.
Unknown producers, cycles, mismatched declared outputs and negative setup credits fail.

CODE READ: `fixtures/dependency-vectors.json` is the shared conformance file for JavaScript
and the Python executor's local resolver. It uses plain catalog `needs`/`produces`, requested
IDs, available artifacts and either expected order or a stable error code. Needs are visited
in lexical artifact order; explicit host swaps first, then lexical requested IDs. Setup producers run once;
non-setup occurrences remain distinct. Definition/object order is not a tie breaker.
The JavaScript tests also compare real single-driver assignment order against that resolver.
The Python lane must consume the same JSON vectors, not copy their expected results into code.

## Packing and provider-independent boxes

CODE READ: a box snapshot identifies its provider, die, physical machine and host, current
host-driver pin, capabilities (`can_swap_host_driver`, `kvm`, `vfio`), limits and prices.
Machine/host keys must be globally qualified by the provider adapter; different offer IDs
cannot disguise the same physical machine. A fixed-driver workstation/container is usable
when its current host driver matches the assignment. A swappable box may report a null
current host pin, since provision installs the pinned target; a fixed host may not. Runs declare their capability needs;
providers do not get implied KVM, VFIO or host-driver-swap permission from their names.
Disk bandwidth fields use the explicit `disk_mb_per_second` spelling; network Mbps is bits/s.

CODE READ: the deterministic heuristic orders primary work and potential seed producers,
then uses descending single-run closure time. It reuses compatible existing boxes before
opening another physical machine when that is cheaper, keeps shared setup together and
opens another box when work exceeds hard minutes minus margin. Resource tier follows the
highest ranked requirement on a box. Primary boxes keep one host target. Isolated driver-only boxes may change targets through
an explicit `host_transition_run` selected by the limits data; the action is not cacheable.
Its prerequisites must be host-independent. Within a driver context, an explicit host
swap precedes guest work. Host-swap work is
isolated from primary evidence by
the `separate_from_primary` policy. Price/reliability/performance/resource caps and banned
machine/host keys apply before selection. This is a bounded heuristic, not a global optimum
or a promise to find every feasible packing.

CODE READ: absent dies have `not_offered` coverage, never a FAIL verdict. Ineligible or
unaffordable work has `unsatisfied` coverage. Either makes the plan incomplete and prevents
rental admission, while retaining the offline partial quote. Proposed catalog/requirements/
limits, unknown selected-die support/minima and any UNPINNED catalog entry prevent execution.
Selected driver ABI mappings must also be known; the installer hash alone does not
approve an unknown Windows guest ABI. Windows 580.88 maps to 580.65.06 from the existing
gate configuration; 580.97 remains unknown. Official NVIDIA HTTPS hosts and Windows
Catalog evidence rules match the pin contract;
missing installer versions/URLs are null UNPINNED records, never invented values.

## Optional seeding and bounded fallback

OWNER DECISION: removing every seed edge must leave a complete local-build plan. Consumers
wait only until their deadline, then build locally. Seeding can improve the forecast but
cannot be necessary for correctness. Version 1 uses one level of durable registry publication:
seed sources build locally; they are not themselves seed consumers. Data controls which
resource tiers may seed and which artifacts may be optional seed inputs.

CODE READ: assignments retain the complete cold ordered run list. A hint may skip only an
unrequested, single-output setup producer; it cannot satisfy an explicit required build.
It names the source, artifact scope, expected transfer, absolute plan-relative deadline and
fallback step. A source has `seed_exports` with the producing step, consumer, GB, estimated
upload, expected availability and publication deadline. Exports may begin after that step
while later primary work continues. They are future executor instructions, not implemented
registry operations in this PR. Source loss or a missing/late publication means local fallback.

PROPOSED transfer model: decimal GB at advertised uplink/downlink speeds, with upload and
download counted separately for seed benefit. This is a planning assumption, not a throughput
measurement or an upload feasibility guard. An unhelpful or slow optional transfer removes
the hint and preserves the cold plan. A hint is selected only when local build minus both
transfers and expected wait exceeds the data threshold. Seeders are repriced for their actual
assigned consumers; data weights can prefer faster/cheaper uplinks. General preferred tiers,
derated download-time price premiums and SLOW_NETWORK rejection remain deferred in #299.

CODE READ: outbound allowance is base evidence GB plus all per-consumer artifact GB.
Consumers reserve incoming seed GB in addition to the conservative cold allowance, allowing
for a failed partial transfer followed by a local build. INFERRED fixture allowances are
13 GB for an open-only image and 0.11 GB for the binaries bundle (0.1 GB binaries plus
0.01 GB instruments); these are not measured sizes of
a newly built open image. Actual traffic is not metered. Registry content/source verification,
publishing and executor behavior still require their later integration.

CODE READ: a source reserves cold minutes plus a maximum publication wait per export;
a consumer reserves cold minutes plus its seed wait. Every such worst case must fit hard
minutes minus margin. Scheduling reserves those worst-case intervals against concurrency;
start offsets also respect expected artifact availability. Forecast box minutes include
upload tails and downloads. `withoutSeeds` preserves physical assignments and coverage,
removes hints/exports, and recomputes cold offsets/quotes. Total box minutes cannot decrease
when the optimization is removed, even if source upload overhead disappears.

## Cost and execution validation

CODE READ: each quote includes allocated storage, expected inbound/outbound transfer and
compute. Effective price amortizes expected network over the reserved hard lease. Forecast
cost uses estimated minutes; reservation cost uses the full per-box hard limit. Admission
uses the total reservation and each tier's effective-price cap, including both per-GB
network guards. Dollar fields round conservatively to six decimals; reservation accounting
uses the same rounded hourly/transfer fields that an executor/ledger receives.

PROPOSED: the example cost cap is USD 45, carried from the owner's planning suggestion. The
example deliberately has no market and cannot establish that a real fleet fits USD 45.
The planner reports per-box, per-resource-tier and total forecast/reservation costs and both
forecast and worst-case finish times. Any seed graph that breaks caps is reduced to a capped
cold plan. A full cost-limited plan is not admitted by hiding missing requirements.

CODE READ: `validatePlan` reconstructs assignments, ordering, coverage, pins, deadlines,
prices and totals from trusted input. It also independently walks the cold run graph,
validating setup availability, host transitions and full driver-context keys; supplied estimates or a claimed proof cannot authorize
anything. Execution validation additionally requires executable data and complete admission.
`validateAssignment` needs the parent validated plan for a seeded/multi-box assignment;
a standalone assignment must cover the supplied self-run requirements and have no seed edges.
The assignment contains source/catalog revisions, provider/physical identities, run parameters,
setup reuse, tier-specific artifact sources and all selected installer pins. It contains no
credential. Future routes must bind this immutable assignment to the existing run token and
ledger record, check live provider facts/budgets before each create, and preserve exact-ID teardown.

## Validation and limits of evidence

CODE READ: run `node --test ci/vast-broker/planner/test/*.test.js`. Fixed market seeds default
to 1337 through 1536. Increase locally with `KF_PLANNER_SEEDS=1000 KF_PLANNER_START=1337`;
counts are bounded at 2000. The mocks use the existing MockCloud, Vast client and Ledger with
a simulation-only assignment adapter. They do not prove an assignment route or Python executor.
The original end-to-end Worker failure regressions remain in #296.

CODE READ: named simulations include status flaps, provider reads failing with 500/429/timeout,
ambiguous create reconciliation after restart, vanished offers, five-of-eight provider refusal,
third-party destruction, fixed-host non-Vast planning and seeder loss during a consumer download.
Owned IDs are torn down and ledger liabilities settle; foreign fixture instances survive.
Independent arithmetic and coverage oracles run after each plan and simulated rental boundary.
The mutant runner requires two distinct named test failures per mutant and rejects setup errors.

UNVERIFIED: the proposed example is not the approved release matrix. Production-branch
completeness, per-die driver support/minima, missing official media, run duration splits,
real market cost, artifact publishing/security, Python execution and hardware behavior remain
open. Executors must verify installer SHA256/size; a 404 or byte mismatch is media drift and
cannot PASS. No hardware, VM, Vast rental, deployed Worker/Cloudflare request or deploy
was performed.
