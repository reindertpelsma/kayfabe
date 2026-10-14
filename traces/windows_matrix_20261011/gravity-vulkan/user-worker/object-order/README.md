# Recorded 5080 object order on Windows USER channels

STATUS: RESEARCH, 2026-10-12. Offline measurements; no new hardware run.

Five retained campaigns (1360 nbody,1361 GravityVk,1495 control,1510 probe,
and registration-refusal) contain301 WINDOWS-USER-WORK Passthrough births.
All301 have a logged successful native5080 child allocation. Of213 with a logged
relay forward, all213 allocate5080 before their first recorded forward; zero
late/missing-object forwards. Other88 have no logged forward: UNMEASURED, not
proof of idle. No unmatched events. Reproduce with audit.py; audit.json retains
input hashes and per-lifetime line references. This is not a full-matrix census
or a proof of universal guest order/native execution.

Only3 of301 have a logged successful API registration (the GravityVk channel in
1361/1495/1510); all3 register FIFO_UPDATE_CHANNEL_INFO. The refused1540 case
has zero successful registrations. Allocation of5080 and installation of an API
bundle are different events: this distinction matters to the owner's fast-path
request. Absence here means no successful registration in these logs only.

INFERRED: choosing inspection at5080 allocation admits most/all active channels
in these campaigns, so an object-absent CopyRing fast path alone may save little.
Delaying until API registration could avoid scanning many ordinary channels,
provided initial packet/binding state is established correctly. CPU exclusion
alone is not a parser reset. A complete trigger can follow registration while
using a binding made before it; no early/guessed handle is needed. Existing
user_scan tests cover binding persistence and hardware packet continuation.
No observed Windows registration-boundary violation is claimed.

The current worker diagnostic scans from birth; no inline/vCPU inspection.
The no-API fast-path optimization and reverse switch are not implemented.
