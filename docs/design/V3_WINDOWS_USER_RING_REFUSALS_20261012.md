# Windows USER worker refusals and observation plan

**STATUS: RESEARCH, 2026-10-12.**

★ 2026-10-14 (`V3_USER_RING_SCAN_RACE.md`): the `user_scan Header / Legacy / MethodOverflow / SubDeviceMask`
refusals of the Win11 + 580.88 runs were early reads, not framing the scanner lacks — the relay read pushbuffers
behind an unreached `GP_ENTRY1_SYNC_WAIT` (now gated, `kf_chan::user_gate::SyncGate`), and a relay that stopped
reading `GP_PUT` without a doorbell left work unread (now polled, `Poll`). The table below is unchanged.

**STATUS: RESEARCH, 2026-10-12.** Source auditedb50774a7a. DefaultOFF diagnostic;
not a Windows compatibility result. Windows1600 exposes an unsupported zero-ID
admission assumption, not a GPU fault. No more Windows boots until this audit
and the zero-profile native discriminator are recorded.

This folds the coordinator's batching suggestion into the investigation. It
corrects treating every parser refusal as a hardware-containment condition.
The exact runtime label inventory is
[here](../../traces/windows_matrix_20261011/gravity-vulkan/user-worker/refusal-audit/runtime-labels.tsv):
111 occurrences in connected userpump/userrelay before test modules. The inventory
includes log/phase labels and retries as well as refusals; nested typed failures
are separated below. Generated categories are an investigation aid, not a proof
that bypassing a check is safe. Prototype-only parsers/ledgers are distinguished.

| Category | Invariant |
| --- | --- |
| a | Host/guest memory containment and exact allocation authority, including bounded host CPU access. Hardware USER/ownVAS/physical-CE denial are independent of parser success. |
| b | Correct recognition, ownership and ordering of software method200 on a channel-owned5080 object. |
| c | Native descriptor credit, immutable staging lifetime and correct guest-source attribution; no fabricated completion/reuse. |
| d | Trusted CPU/memory/metadata/sequence resource budget. |
| e | No justified invariant for this case yet; do not describe it as a security requirement. |

| Refusal family | Category | Actual reason and relaxation limit |
| --- | --- | --- |
| GP_FORMAT Entry | b,c | Runtime understands public direct entries/NOP and their fetch positions. Unsupported CONTROL/conditional/reserved/extended entries cannot be assumed to name a bounded PB or supply equivalent cursor receipts. This is not evidence hardware USER containment failed. |
| GP_FORMAT Budget / source length / pieces / cache windows | d; a for CPU range arithmetic |896KiB/225pieces/16views are prototype quotas. A complete bounded CPU read is still mandatory. Fragmented legal guest mappings may exceed these limits without violating GPU permissions. |
| LEVEL / SYNC_WAIT / CONTROL NOP | c | Already supported and preserved atb507. Native probes establish ordering; never replace a synchronized NOP with CPU bookkeeping. LEVEL alters top-level reporting, not authority. |
| user_scan Header / Legacy / MethodOverflow | b; a for CPU indexing | Need packet framing to locate a software call. Default legacy decoder lacks increment mode. Invalid framing destroys trustworthy later parser state; there is no general resynchronization promise. Ordinary engine operands remain native. |
| user_scan SubDeviceMask(raw) / identity unknown | b | Hardware ignores inactive methods; scanner must not execute ignored software actions or invent bindings. SET/STORE/USE supported with known matching identity. Unknown stored initial state stays unresolved. |
| USER_RING_SUBDEVICE_ID for declared0 | e | Public ordinary UVM/watchdog callers intentionally leave0; public CPU/GSP forwarding copies it. Rejecting0 as malformed is unsupported. Effective GSP/RAMFC filtering semantics are not defined by the available source. Do not normalize0 tobit1 from GPU ordinal. |
| user_scan SoftwarePacket / trigger partial suffix / first-only5080 / late5080 / notify unsupported | b,c | Current interception represents a whole trigger and staged suffix with one admitted software-object identity. These are implementation subset limits, not general hardware danger. Native notification semantics cannot be invented. |
| Unregistered/foreign API, software identity changed, foreign context target | a,b | CPU host actions require exact own-client/channel/registry authority. Opaque native methods cannot authorize CPU actions. Never bypass ownership to make progress. |
| userread Bounds/Missing/Topology/Store/Poisoned; memory_access Gone/Unsupported | a,c; d for piece/view cap | Complete source spans and USERD writes must resolve through live owned mappings/topology. An unavailable view is not permission to read arbitrary host memory or reuse old backing. |
| userread Busy/Changed; record try-lock busy; VA-manager pending | c | Retry/backpressure, no completion and no timeout release. Changed source is rescanned under current authority. |
| Fetch FULL/GET/UNISSUED_GET/SEQUENCE and consumption Capacity/Lifetime/Full/Fetch/Sequence/Allocation | c,d | Native private ring has bounded credits and host generation. GET must refer to published descriptors in this allocation. Full backpressures; invalid/uncertain progress never retires staging. |
| Native birth/doorbell/write uncertainty, cleanup failure, token/lifetime collision, poisoned authoritative locks | a,c | Keep/quarantine attempted resources until confirmed native free/session close. Failure is not proof that GPU access stopped. |
| Source/migration declarations, USERD aperture, owned-ring admission | a,b,d | Validate own channel/client, source span, power-of-two ring/alignment and supported CPU access. Vidmem USERD remains an implementation gap, not proof that Windows forbids it. |
| Initial cursor0 / migration cursor0 / TOP_LEVEL_GET | c | Diagnostic hypotheses/unfinished parity, not silently established by parser acceptance. CurrentGP_GET propagation exists. Exact transfer semantics remain unproven. |

The disconnected user_stream full interpreter additionally refuses Object/Unbound/
Method/Truncated (b), Budget/Allocation (d), Header/Entry (b with bounded CPU
access). Connected runtime uses only its inspect_entry; it uses user_scan for
methods. user_batch/user_progress/user_ring model failures are plan/order/lifetime/
credit proofs (b,c,d), not a claim that all ordinary methods need interpretation.
Their tests are useful but they do not secretly expand the connected runtime.

## Batched observation — implemented diagnostic, default OFF

The KF3_WIN_USER_RING_SHADOW adapter copies original GP descriptors unchanged into a private
native ring, perform no CPU software action, stage no PB and add no witnesses.
It must still preserve exact allocation/USERD/source authority, private ring
credits and original native SYNC/LEVEL semantics. Staging-retirement checks do not
apply when no private command bytes exist. Native descriptor fetch may retire
only the copied descriptor/source receipt; never engine work or arbitrary mapping
lifetimes. Scanner/read quotas become counted observation limits, not unbounded
CPU reads. A malformed/unreadable observation stops parsing that channel, while
logging an explicit unknown suffix; it cannot truthfully enumerate every later
refusal after losing packet framing.

The observer must retain bounded first samples per named class plus counters,
not dump all workloads/RAM. Source words are snapshots for a GPU-free regression
corpus, never interfaces/constants. Keep real GP, source revision/channel and
sequence attribution; classify incomplete samples explicitly. Existing1510/1540
logs provide partial interface/trigger observations;1570 has rawNOP and1580 a
raw100KiB descriptor.1590 has no raw failingmask/header and1600 no submitted
worker stream because admission refused. A complete historical PB corpus cannot
be reconstructed from those logs. Do not invent words from inferred headers.

An unscanned native FIFO-update trigger is expected to RC its own channel based
on1540. That is not proof a channel RC cannot cause engine/runlist reset or affect
another VM. The proposed coordinator RC-scope branch was not onorigin at20:04UTC,
then became available and was read at20:14UTC (claude/rc-scope-20261012).
It identifies channel/TSG notification scope, but GSP recovery policy is absent
from public source; engine reset, fatalGSP and GPU-level recovery can affect
others. Source reading does not prove a malformed channel always stays local. A two-VM native
containment test remains the hardware discriminator. No shadow hardware run is claimed in this checkpoint. Use an exclusive single-VM
diagnostic after exact Linux regression; do not infer other-VM availability.
The observer has a 4 MiB attempted-read quota per channel, one bounded first
sample per named class (at most 128 words), and saturating attempt counts. Once
framing/history is unknown, it stops all later CPU pushbuffer reads. Any failed
publication also discards parser history; retries retain quota and counters.
Default-zero native birth uses the ordinary host profile. Its observer assumes
effective bit 1 explicitly as a diagnostic hypothesis, never CPU action authority.
Native zero/id1 fixture equivalence is measured only on host595.91.07 USER/C7B5.

Review found two issues, both fixed with failing-first regressions: strict
descriptor validation accidentally remained on the opaque writer, and a failed
publication could leave advanced parser history for a retry. The separate raw
writer retains slot/live-storage bounds without changing the strict writer.
Original owned-source/native-credit publication guards and native GET receipts
remain; there are no CPU API actions, private PB staging or witnesses in shadow.
Independent re-review found no source blocker for this scoped diagnostic.

## Engine evidence and concrete authority

OWNER_RULINGS §V is a DRAFT about Translated unknown-entry forwarding. Its measured
physical-operand oracle covers CEclassC7B5 on RTX4070/host595.91.07: physical source
and destination/localFB/coherentSYS refused, virtual control delivered. Graphics
C997, computeC9C0/C6C0 and video/OFA have public-source no-physical-operand analysis,
not equivalent per-class hardware measurements. Do not broaden that DRAFT's
allowlist using inferred clearances. A shadow USER-channel adapter must preserve
its existing Passthrough eligibility, native USER privilege, own refreshed
restrictedVAS and physical-CE deny; it is not a Translated privileged parser.

With those protections, guest mutation of ordinary native methods grants no CPU
host verb. Only validated registry interpretation can grant that authority.
A concrete remaining concern is shared-engine reset availability after RC, not
an identified arbitrary-memory capability. This is unresolved, not asserted safe.
Full snapshot interception plus consistent prior parser/binding/mask history remain
necessary for a CPU action to correspond to the native prefix. Shadow execution makes no such guarantee
because it executes no CPU actions. Scan-only forwarding is not a migration fix.
