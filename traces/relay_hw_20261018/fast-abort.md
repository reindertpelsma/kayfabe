# M fast-suite abort — 2026-10-21

STATUS: LIVE / BLOCKED ON AN EXPLICIT RULE CONFLICT.

MEASURED: exact M QEMU, rebuilt raw client from verified M archive, native
9/9 plus gate10 PASS. Fast suite passed timer, engines, doorbell-census,
gpu-info-sweep, bus-info-sweep and concurrency. The seventh arm was aborted;
23 remained unrun. This is neither 30/30 nor an observed failure of M's relay.

MEASURED: the seventh guest arm printed:

```
FAULT_MARK=defer_liveness_prime arm=PLAIN(rig-control) va=0x0000006000010000
```

The same line explicitly says that the host dmesg WILL carry Xid 31 for this
channel and that this is the test's point. CODE READ: the raw client's
`defer_liveness_arm` deliberately reads the absent leaf before testing a
subsequent deferred mapping. This is an expected negative-test fault.

MEASURED kernel record:

```
[973057.584604] NVRM: Xid (PCI:0000:01:00): 31, pid=669294, name=qemu-system-x86, channel 0x00000020, intr 00000000. MMU Fault: ENGINE CE0 HUBCLIENT_CE1 faulted @ 0x60_00010000. Fault is of type FAULT_PTE ACCESS_TYPE_VIRT_READ
```

MEASURED QEMU record: host twin 0x20, guest token 0x3, engine 0x9, wrote
except_type=0x1f (Xid 31), and QEMU forwarded RC_TRIGGERED. The address and
channel match the deliberate prime. The guardian saw Xid count 25 -> 26,
terminated only owned descendants while retaining the resource lease, and
recorded reason=new_xid, rc=-15, epoch=1792540570.0197375. The kernel's raw
monotonic timestamp is retained; dmesg -T conversion is not used to establish
wall-clock ordering.

MEASURED: production restoration was queued after cleanup; run 3437,
REV=0d7ba743, OWNER_SESSION_READY 01:56:13 CEST, hostname kayfabe-windows.
The lifetime resource lock belongs to that owner launcher. M Windows runs
and Steel Nomad experiments have not been started.

CODE READ: the owner briefing says abort on ANY new Xid. The lane also requires
all 30 arms. Following both literally cannot pass this intentional-fault arm.
Options sent for clarification before further hardware:

1. Permit only explicitly marked, exactly matching negative-test faults
   (PID/channel/address/type); retain the stop on every other new Xid.
2. Keep the unconditional stop; report hardware qualification incomplete and
   do not promote M to Windows.

Recommendation: option 1 after an explicit clarification. No stop condition,
arm, assertion, timeout, or baseline has been loosened while awaiting it.
Raw logs and the partial scoreboard remain on the trusted host.
