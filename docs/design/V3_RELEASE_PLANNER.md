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
cannot be necessary for correctness. Version 1 uses one level of optional source-box serving:
seed sources build locally; they are not themselves seed consumers. Metadata lives in the release session; bytes do not survive the source box. Data controls which
resource tiers may seed and which artifacts may be optional seed inputs.

CODE READ: assignments retain the complete cold ordered run list. A hint may skip only an
unrequested, single-output setup producer; it cannot satisfy an explicit required build.
It names the source, artifact scope, expected transfer, absolute plan-relative deadline and
fallback step. A source has `seed_exports` with the producing step, consumer, GB, estimated
upload, expected availability and transfer deadline. Exports may begin after that step
while later primary work continues. They are future executor instructions, not implemented
HTTP/registry operations in this PR. Source loss or a missing/late publication means local fallback.

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
HTTP publication and executor behavior still require their later integration.

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

## Optional direct-serving protocol (owner update, 2026-11-08)

OWNER DECISION: a seed is an option, never a requirement. This replaces the earlier
assumption that an artifact survives the source box in a durable cache. A release-session
registry in D1 stores metadata, with state `ready` or `gone`; bytes stay on the source box.
Ending a source assignment makes every artifact from it gone. A source with
`can_expose_port: false` (or an omitted capability) cannot seed; it may still consume a
public seed. Missing mappings at runtime also disable publication. Removing every seed
edge still leaves complete local-build coverage.

CODE READ: `artifacts.js` is a pure protocol validator/reducer. It does not implement an
HTTP route, verify a token signature, contact Vast, write D1 or create a box. Its trusted
context must come from the existing Worker run-token verification and ledger lookup;
INFERRED adapter contract: the authentication-success flag comes from that adapter, never a request field. The
context binds session, assignment and current instance, token expiry, current admission
permission and fresh provider facts. Calls reconstruct/validate the executable parent plan.
The route adapter must preserve the existing caps, sticky FAIL/receipts and exact-ID teardown.

PROPOSED endpoints are data in `protocol-endpoints.json`. Every request uses the existing
`KF_BROKER_URL` and `KF_RUN_TOKEN` Bearer credential over the Worker's HTTPS origin, without
redirects. The run token is never sent to the artifact server. GET responses use `Cache-Control: no-store`; seed transfers refuse redirects and verify the complete byte count/hash before reuse. Request JSON is closed and
versioned; GET normalizers attach contract version 1 and reject extra query fields.

| Method and path | JSON contract | Behaviour |
|---|---|---|
| `GET /assignment` | response `assignment_response` | Return only this token's immutable assignment, launch mode, actual start and bounded artifact resolutions. |
| `POST /artifact/ready` | `artifact_ready` | Accept `{version, artifact, url, sha256, size}`; 204 after durable state/claims and permitted immediate starts. Identical repeats are idempotent. |
| `POST /artifact/gone` | `artifact_gone` | Producer reports ended/unavailable, or a planned consumer reports hash/size mismatch; 204. A tombstone cannot be republished into ready. |
| `GET /artifact?name=X` | normalized `artifact_query`; response `artifact_response` | 200 ready, 202 waiting, 410 gone/expired, always with `fallback: build_local`. Unknown/unplanned names are 404. |

CODE READ: auth failure/expiry/mismatched live identity maps to a generic 401; malformed
wire data is 400; conflicting metadata or a source past its serving window is 409.
POST responses have no body; reducer state and `start_requests` are internal to the Worker.
The Worker must never return another assignment or expose its registry as a listing.
If multiple driver contexts share a logical artifact name, X is the URL-encoded `scope_key`;
an ambiguous short name is refused. A consumer can access only its planned producer edge.

CODE READ: a ready POST requires a planned seedable artifact, canonical HTTP(S) URL with
an IP literal, no credentials/query/fragment/encoded path, a file under a source-generated
8–128-character capability path, a SHA256 and a sane positive byte size. The host must equal
the instance public IP from fresh provider facts; the public TCP port must map to the
assignment's requested container port, which must be recorded among ports requested at create.
Vast facts must come from the authenticated Vast API and the trusted create record, never
from the POST. Non-Vast daemon facts require the corresponding trusted provider/registration
adapter; a NAT box with no exposed mapping sets the capability false. The Worker validates
metadata and never fetches the submitted URL.

PROPOSED data: source container port 8080 and provider-fact maximum age one minute.
These are initial policy values, not measurements. CODE READ: a ready file must fit
the artifact GB allowance already included in seeder/consumer pricing; a larger file
is declined and the consumer builds locally. Executors compute SHA256 and byte size over the completed transfer
before reusing it. A planned consumer's mismatch report marks that artifact gone. It cannot
certify those bytes; it may build locally and submit ordinary independently earned receipts.
Pinned driver media still use the official-source/media-drift rules and cannot PASS a 404
or hash mismatch.

CODE READ: every source assignment has `seed_server.container_port` and `serve_until_minute`.
Every consumer hint has an explicit build-local fallback, deadline and final fetch deadline.
The planner keeps the fetch window inside the source's assignment lifetime, charges serving
minutes through the final planned fetch, and reserves the cold fallback plus bounded wait.
If no such overlap fits, the optimization is dropped. One concurrency slot cannot seed a
later box from a source that has ended. A source executor must remain available until its
serve-until even when its primary work finishes early, and must stop within its hard lease.
Scheduling still reserves conservative worst-case intervals.

CODE READ: `execution_steps` is the executor order: optional `wait_for_artifact` instructions
followed by references to the complete cold `runs`. Wait instructions name the fallback setup
steps. After a validated fetch, the executor skips only those optional setup steps; on timeout,
source loss or verification failure it executes them locally. A box already running polls
GET; the Worker does not push. The assignment response clamps the wait to the actual start,
token expiry and source lifetime, so an early event-triggered start cannot gain an unbounded
wait. The immutable plan deadline is an upper bound; the response may shorten it. All minute values
share the release-session origin. Responses include `server_minute`, so the executor can
convert remaining wait/fetch/serve time to its monotonic clock, subtracting request latency
and respecting its own hard lease. It must cancel a transfer at `fetch_until_minute` and
build locally after a failed or expired transfer. No synchronized box wall clock is assumed.

DESIGN-ONLY integration: in the artifact-ready handler, atomically compare/update the D1
session revision, write ready metadata and claim eligible pending consumers as `starting`.
Perform their rental/daemon dispatch from that handler, without waiting for cron. Before each
create, use the existing live credit/daily/price/reliability/performance/machine and concurrency
guards. Persist the normal rental intent before calling Vast; ambiguous creates use exact
reconciliation, not a second create. Duplicate/late ready posts cannot start an assignment
already starting/running/ended. If admission or concurrency is closed, keep the ready metadata
and start no consumer; later normal admission can use the cached metadata while it is live.
D1 compare-and-swap/transaction semantics and the HTTP adapter remain unimplemented here.

OWNER DECISION: one executor core has three launch modes. `worker_vast` uses onstart and pulls
its assignment; `worker_daemon` runs on a registered non-Vast box and pulls/receives work from
the Worker; `direct` reads a local assignment and writes `report.json`. All use the same cold
run/parameter/driver-pin shape. Direct mode needs no Worker credential, never publishes or
waits for Worker artifacts, and builds locally. `directAssignment` validates and wraps a
local assignment without a token. It does not produce a release verdict for a partial fleet.

CODE READ: `artifact-simulation.test.js` uses the existing mocked Vast transport and Ledger
with an explicitly test-only future-handler adapter. It covers immediate ready-triggered
consumer creation, duplicate ready, source death during download, expired consumer deadline,
URL host/port mismatch and actual byte/hash mismatch from synthetic buffers. It checks exact-ID
teardown, settled ledger rows, surviving foreign fixtures and no synthetic PASS. Port mappings
are normalized fixtures; this is not a claim about real Vast create-port syntax or live D1/HTTP
routes. The Python executor, port exposure on Vast VMs and non-Vast daemons remain UNVERIFIED.
