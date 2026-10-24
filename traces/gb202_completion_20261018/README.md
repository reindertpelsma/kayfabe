# GB202 enable-to-idle regression preparation

STATUS: RESEARCH, 2026-10-25. READY for controller review; production and probe UNRUN.

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

CODE READ: future planning is one lease, stop $2.25/cap $2.75, at most $1/hour and two hours,
with $8 reserve. Cost guards can shorten that duration. This is not rental authorization.
Retain verified-only VM offers, country/blacklist checks, unchanged BW6 identity admission,
precreate watchdog, spend checks, hash-verified capture before destroy, exact-id teardown and
API absence before timer cancellation. BW7 staged admission remains native-template-only.
Prefer machine 152098/host 702330; no retry after real refusal and no SW3 authority reuse.
Historical kit authorities and unverified-offer exceptions confer no BW8 permission.
The private unexecuted kit awaits a separate controller grant after review.

Reproduce the local checks without a GPU:

```sh
python3 -B -m unittest discover -s scripts/bench/bw8 -v
python3 -B scripts/bench/bw8/mutations.py
```
