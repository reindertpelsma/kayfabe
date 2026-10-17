STATUS: ANSWERED, 2026-10-17. Exact-revision raw regression PASS; app acceptance reported separately.

| Die | OS/build | Actual profile | Result |
|---|---|---|---|
| GA104 arch0x170/impl4 | NativeLinux595.91.07 | Stock open driver | Gates9/9+gate10, CUDA4 and raw30/30 PASS |
| GA104 | GuestLinux595.91.07/kernel6.8.0-59-generic | KF3={} normalconsole6/frozen180 | Raw30/30 PASS;161USERbirths/90CUDApostures/30actualprocessreceipts |
| GA104 | GuestLinux595 full CUDA/graphics/broker | Production Linux profile | Reported separately after app validation |
| GA104 | Windows580.88 | OwnerP19 required when run | Still UNVERIFIED/UNRUN at this checkpoint |

MEASURED source9d6f210ba1dc6ab0edbc49ff265101091e8b08f1; privateQEMU SHA
cc2a86a6dec3ec6bed91d9cd6e1963a1deadf2d75a768e9e3dada91559e34668;
rawclient0db4104321bc9f38d73d7e54e80902646a93547702f3218deb14295bebd4c0bf;
595thininitrdbd19b0237b49c20f20a352aacdd1d0a8b4d8456967d2d95b6caf0215e22c3c8a.
Raw start01:47:52/end02:01:34UTC. All30receipts assert exact executable/source,
host+guest595, actual flags and console6. No grader/budget/console change.
161/161suite births and12/12gate births USER; all30arms have CUDAposture proof.

Raw-component falsifier stated before rental: a native-pass arm fails/times out/notrun
under this exact product at180seconds. It did not fire:30PASS/0FAIL/0CRASH/0NOTRUN.
R34guestRAM wall143s/client124850ms; other arm metrics are in arm_metrics.json.
Do not infer a die/driver/box cause from comparisons with runs varying multiple axes.

Warnings:246 heartbeat report lines, all heartbeat0/heartbeatWithOffsetMs0; zero
nonzero-heartbeat formula warnings in this suite. Startup readiness remains a named
gap; this is not a warning-free or drainer-readiness fix claim.

Ten hostXid31s match the deliberate raw negative cases: native and guest each have
four CE/PTE/read faults in defer-liveness and one PDE/write in missing-page-fault (native GR/PBDMA, guest CE).
Guest PIDs90894(4)/91720(1) map directly to actual QEMU receipts and arms. Native
PIDs89289(4)/89329(1) correlate by serial-arm log creation/mtime, kernel timestamps,
explicit fault kinds and addresses; native child-PID receipts were not generated.
Adjacent rounded time windows overlap, so time alone is insufficient; fault kinds
are also checked. Keep this attribution limit visible. No application Xid waiver:
apps must independently show zero guestXid/flip and unchanged host journal.

The first fat-image recipe failed before any guest boot because its driver-cache
parent directory was absent. App continuationV3 reuses the verified host595 package
and does not replay the passed raw/native runs. Original precondition failure retained;
this is a harness failure, not a guest-driver result.

166screened text/process artifacts exported; one large log excerpt-only with full
rawSHA/size/counts in manifest.json. Only own56326220 active; timer2714225/04:25UTC,
absolute05:00UTC APIabsence deadline. No shared-host use, master merge or family completion.
