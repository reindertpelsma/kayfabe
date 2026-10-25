# GB202 enable-to-idle regression preparation

STATUS: RESEARCH, 2026-10-25. Probe UNRUN. The additional grant5 lease failed in operator
upload before provisioning; its remote capture also failed. H1/H2 remain UNRESOLVED.

MEASURED in [grant5_result.json](grant5_result.json), lease57714244: settled SSH admission
passed, then upload raised StopIteration while reading the historical rehearsal ledger.
No kit upload, native qualification, parent rebuild, product boot, enable or sample occurred.
The absent remote kit also prevented capture. Safety teardown verified exact API absence;
the remaining timer was cancelled afterward with PID identity checks. Local error hashes
are preserved privately; no verified remote capture before destroy is claimed.

MEASURED in [grant5_validation.json](grant5_validation.json), BW8 synthetic checks:236 tests,
73 mutants with at least two killers each, fresh574 inherited tests across27 suites and
both fake timer paths were green before create. The adapter rehearsal stubbed upload/media
subprocess boundaries, so it missed the real upload ledger/kit relocation defect. The
operator needs repair and real upload assembly qualification before another execution.
No retry, product fix or additional lease followed this failure. Prior results below are history.

CODE READ: [BW6, lease 57556167](https://github.com/reindertpelsma/kayfabe/blob/2cf144fa259c5574275e57d16a8401352e5cb5a7/traces/blackwell_ab_20261023/ROUND_B1r_bw6/REPORT.md)
records Code 0 followed by 0x141 and fatal 0x113 before its timed hold began.
[BW7, lease 57565048](https://github.com/reindertpelsma/kayfabe/blob/08242fc08136fde9519cbdc6e50af7da45f73df2/traces/blackwell_native_ref_20261018/BW7/REPORT.md)
records the native 580.88 idle pass on the exact machine. INFERRED under the matching-native
rule: attribution to kayfabe is supported; the faulting operation remains unidentified.

CODE READ: the unchanged combined source is
`007ba1eb03db9e21d351e465e95b46cbc3134a95`, with route parent
`b85d02bae3ed085a10cd1c8376c1a842e86c571b` and second-life parent
`38fd948fd67d89ebf42827f56a3435355df592ae`.
[The existing completion probe](https://github.com/reindertpelsma/kayfabe/blob/007ba1eb03db9e21d351e465e95b46cbc3134a95/crates/kf-qemu/src/chan.rs#L8154)
is optional. No product source, boot requirement, timer or workaround changes here.

CODE READ: the new operator overlay preserves the BW6 production path, default p19 flags,
existing image/580.88, one enable from disabled and no application. It starts collection
before invoking the unchanged single-enable helper. Any new 0x141/0x113, System bugcheck
1001 or reboot fails from enable onward, including before readiness. Application WER1001
is separate; classified 0x193/0x1A8/0x1B8 alone do not fail. Missing or ambiguous metadata
cannot establish a pass. Dump identity includes file generation; basename alone is insufficient.

CODE READ: the oracle requires complete bounded collection, consistent UTC/monotonic clocks,
stable boot/driver identity, the inherited readiness observer's successful exit, and at least
60 seconds after the first observed Code 0. The original 600-second hold remains in the arm.
A regression pass is not full acceptance. Collection failures are inconclusive evidence,
and do not establish a production RED. PowerShell execution and collection overhead are UNRUN.

CODE READ: [inventory.json](inventory.json) pins source, binary, media and baseline receipts.
The sole effective flag delta is `KF3_COMPLETION_PROBE=1000`. H1-T is falsified by the relevant
host fence completing with the expected guest release while that channel still times out.
H1-G is falsified by a complete correlated witness showing that fence never completed before
timeout. Missing records falsify neither. Correlate channel/fence/placement/release/armed event;
the probe does not cover every relayed twin. An idle channel may legitimately be silent.
Its up-to-16-byte completed operand samples are PRIVATE DATA. No samples, command bytes,
dump contents or full private analysis are published. No PB peek or ring trace is scheduled.
Survival or an inconclusive probe leaves the mechanism UNRESOLVED.

MEASURED (2026-10-25, BW8 synthetic run; [validation.json](validation.json)): 99 checks pass,
including four retained-outcome metadata replays with synthetic cadence. Twenty-seven source
mutants each have at least two distinct killing tests; see [mutation_kills.json](mutation_kills.json).
The tests-first commits are synthetic EXPECTED RED checkpoints, never a production RED.
MEASURED (BW8 inherited guard replay; validation.json): 115 unchanged identity/watchdog/budget/native-template
checks pass with fake clocks/providers. These checks create no lease and prove no hardware result.

CODE READ: the original preparation proposed one lease, stop $2.25/cap $2.75, at most $1/hour and two hours,
with $8 reserve. Cost guards can shorten that duration. This is not rental authorization.
Retain verified-only VM offers, country/blacklist checks, unchanged BW6 identity admission,
precreate watchdog, spend checks, hash-verified capture before destroy, exact-id teardown and
API absence before timer cancellation. BW7 staged admission remains native-template-only.
Prefer machine 152098/host 702330; no retry after real refusal and no SW3 authority reuse.
Historical kit authorities and unverified-offer exceptions confer no BW8 permission.
The preparation inventory preserves its historical planning inputs; it is not live authority.


MEASURED (2026-10-25, reconstructed fixture closure;
[fixture_rehearsal.json](fixture_rehearsal.json)): all 27 inherited suites pass, 574 tests,
both fake-provider timer teardown paths pass, and the inherited static check reports zero
problems. No suite is replaced or skipped. Fixture sources are restored from the pinned
BW6 execution history and provider history; admission logic is unchanged. Historical ledgers
remain evidence, with a separate empty ledger for fake-provider rehearsals. Git operations
use a private fixture repository and remove inherited Git environment overrides.
The BW8 suite now has 122 passing tests and 35 source mutants with at least two distinct
killing tests each; see [fixture_mutation_kills.json](fixture_mutation_kills.json).

MEASURED (2026-10-25; [execution_result.json](execution_result.json)): one verified VM-capable
RTX 5090 lease, 57694407, ended with `provider_host_error` after 408 seconds of startup.
No SSH identity was admitted, parent rebuilt, product enabled or probe run. Console capture
was one newline, hash-verified before exact-ID destruction; API absence and both timer exits
were independently rechecked. The provider-status refusal receipt stays private.

CODE READ: a reviewed amendment permitted rebuilding BW6's unavailable sealed parent through
the same pinned provisioning pipeline, recording a new digest and differences. That permission
resolved the artifact blocker. This lease never reached provisioning, so no parent digest or
new hardware result exists. H1-T/H1-G/H2 remain UNRESOLVED. This is an operator/provider
refusal, not a product RED; the single-lease allowance was consumed without retry.

MEASURED (2026-10-25, synthetic/operator checks; execution_result.json): 216 BW8 tests and 11 unchanged
cost/watch checks pass, in addition to all 574 inherited checks. All 68 source mutants have
at least two distinct killing tests; see [execution_mutation_kills.json](execution_mutation_kills.json).
The adapter rehearsal used fake provider/SSH transports and upload/media subprocess fixtures.
It exercised the actual settled admission, capture/hash, owned-ID teardown and timer ordering.
The binary digest gates the product arm; PowerShell parsing must pass before enable. Actual
PowerShell/process/binary/flag admission and the 60s/600s observations remain UNRUN.

Reproduce the local checks without a GPU:

```sh
python3 -B -m unittest discover -s scripts/bench/bw8 -v
python3 -B scripts/bench/bw8/mutations.py
```

2026-10-25 grant3 market check: MEASURED in `grant3_result.json` at 17:48:44Z,
none of six verified VM-capable RTX 5090 offers met the unchanged 1 USD/hour guard,
including requested 180 GB storage (quotes 1.09–1.50 USD/hour). No additional lease,
spend, enable or observation; H1-T/H1-G/H2 remain UNRESOLVED. The earlier provider
refusal and its retained charge are unchanged. The additional grant required a
different machine and host; previous machine56897/host292781 remained excluded.

2026-10-25 grant4 check: MEASURED fresh quotes in `grant4_result.json` at18:15:04Z
include two verified VM offers within the amended1.40 USD/hour ceiling. The cheapest
quote1.295 USD/hour allows3443 seconds after unchanged stop/margins, below the retained
BW6-based4931-second provisioning/observation estimate. Earlier UK offers disappeared.
No lease created; UNRUN because the estimate does not fit, as required by the amendment.
H1-T/H1-G/H2 remain UNRESOLVED; no fresh rehearsal or hardware result is claimed.
