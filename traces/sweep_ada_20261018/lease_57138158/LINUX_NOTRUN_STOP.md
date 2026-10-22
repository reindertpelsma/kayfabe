STATUS: LIVE, 2026-10-22. S2 failed harness precondition; no replay or Windows stage.

MEASURED: stock AD102/Linux595.91.07 open SOUND; gates9/9+10 and12USER births;
native raw30/30, CUDA4/4, nativeGR PASS. The exact five source-matched native31s
were approved. No new/unrelated31 or32 is present in the retained kernel record.
MEASURED: the guarded Linux suite stopped on its first --timer arm: underlying
fast_suite rc2/NOTRUN, outer guard rc7, controller EXIT7 at01:37:16UTC.
No QEMU log was created for that arm; no Linux guest workload or Windows stage ran.
CODE READ: I omitted KF_FASTGUEST_DIR when adapting the controller invocation.
The builder wrote /workspace/bench/fastguest595, while the inherited runner uses
/workspace/bench/fastguest by default. This is my harness plumbing error, not
an observed kayfabe guest failure and not a guest-Xid guard rejection.
MEASURED: retained-file inspection shows both default guest files absent, both
fastguest595 files present, wrapper present, no controller FASTGUEST_DIR setting.
The predeclared missing-directory falsifier was not met. No command was rerun.
CODE READ: the common/lane STOP rule requires collection and teardown after a
failed step. No environment workaround, timer change, retry or next GPU arm was
attempted. Future work should first prove configuration propagation with tests.
MEASURED: software guard97green on both dev and rental;22mutants>=2killers remain
valid software evidence. Actual guest4+1 correlation/recovery remains UNVERIFIED.
