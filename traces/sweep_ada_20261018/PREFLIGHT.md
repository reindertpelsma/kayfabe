STATUS: RESEARCH, 2026-10-22. Lane S2; GPU-free prerequisite check.

CODE READ: the lane requests the approved native five-fault baseline plus
source/PID/address/channel/recovery-matched guest negative controls before Windows.
The required starting sweep branch is 6728c49b91ba1f5012cc25826873492650b81eab;
master is e64b8e5f6d0f64c41c9c4d40775e74be08de7164. Their clean, non-rebased
merge is 79925a345fddb7b1ca70239e8a29efd32f3f426a, pushed and origin-verified.

INFERRED hypothesis, stated before this check: the merged native guard cannot
provide the lane's approved guest exception. FALSIFIER: its current supported
CLI accepts the retained native baseline followed by the four fully correlated
guest defer-liveness records, using this run's source, QEMU PID, physical/guest
channel mapping and recovery evidence, while retaining unrelated-Xid rejection.
No missing-page guest acceptance is inferred from the historical incomplete proof.

CODE READ: native_xid_guard.py exposes only native/check/watch. Native proves
five kayfabe-rm-ladd faults; check/watch require an unchanged sequence afterward.
The retained controller starts that watcher before fast_suite.sh. The historical
owner approval explicitly says no guest exception implementation was started.
This check replays retained text through the supported native/check CLI; it does
not run the controller, start a watcher, access hardware, or change acceptance.

CODE READ stop rule: the lane's common instructions say, "If a step is blocked
or refused by your own safety checks, STOP and report; do not reword or work
around it." The S2 instructions also say to collect evidence and not loop after
a failed step. On refusal, no lease or alternative guard will be deployed.
