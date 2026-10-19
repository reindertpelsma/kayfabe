# GB202 Linux 595 concurrency

STATUS: LIVE, 2026-10-19. MEASURED on own instance56788212, product24906bf33,
stock open host595.91.07, exact fast guest595.91.07. Native gates9/9 plus gate10,
raw30/30 and CUDA4/4 passed first. No performance ratio claim (n=1, spread N/A).

Concurrency PASS:41s total,19350ms client, frozen180s bound, normal loglevel6
and default hvc console. The target did not reproduce the earlier timeout.
Heartbeat acceptance remains PARTIAL:31warnings, all raw heartbeat0, from guest
20.127931s to20.345237s (217.306ms). Zero nonzero-heartbeat warnings were observed
in the complete serial capture. Offset is initially ahead of currentTimeMs,
yielding unsigned near2^32 differences until time catches up. This is a startup
gap, not a warning-free result; no causal speedup or complete B3fix is claimed.
The same-binary quiet arm was not needed because normal-console concurrency
finished below180s. No timer, console, grader or heartbeat period was changed.

Additional full raw595 run22:33:40..22:44:34UTC:30/30PASS at frozen180s,
0fail/crash/notrun. All30complete serial logs have0heartbeat warning,0Xid
and0flip timeout. This later observation does not erase the31startup warnings
in the first concurrency run above. Fullsuite concurrency33s/client19030ms;
observed concurrency n=2 total[41,33]s, client[19350,19030]ms, no ratio claim.
Guest CUDA595 now active; graphics/broker595 remain UNRUN. Evidence and export hashes:
b3_concurrency595*, b3_heartbeat_summary.json, b3_export_receipt.json.
