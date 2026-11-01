**STATUS: LIVE,2026-10-22.** Provisioning replacement for the failed-start box57142142,
which was destroyed/API-absent02:07:19UTC before this creation. The documented box procedure
explicitly directs destroy/re-rent on GPU-error/unable-to-start. One active box, same7USD lane
hard stop/reserve5 and fixed N0->disabled580bind->CAPCON->OFF/TDR->mergedB1ON sequence.

Ownid57143789, machine152912, providerunverified, RTX5090. Created02:10:10UTC, actualrate
0.716667USD/h, automatic deadline04:50UTC (the shared helper clamps the requested170minutes
at06:50CEST). Independent budget watcher ends at the same deadline. Before every arm leave25min
for observation/export/teardown. Both timer PIDs/starttimes retained; cancel only afterAPIabsence.

The local receipt-parent directory was missing after successful creation. This interrupted
bookkeeping before SSH/GPU work; recovery identified the unique task-label instance against
precreate empty-account proof, armed its exact-id timer and retained ownership metadata.
The first group-identity check raced setsid startup; later bounded revalidation (2026-10-22) proved the same
owned timer live. No timer check was relaxed, and no remote workload ran before verification.
The helper now creates the receipt directory before creation. The original search quote/before-
create credit value were not retained after the exception: the receipt marks them unknown,
uses actual charged rate conservatively, and records a fresh recovery credit42.739799USD.

Product8f3a9bb3a; guard sourcee16402b7c (97tests/22mutants>=2killers); optional mergedB1
2add71e8c (parents8f+1c6452a27, release1127/0/4). Kit reused without product changes from
../blackwell_validation2_20261022/run_kit/. Current preflight/N0/Windows results remain UNRUN.
