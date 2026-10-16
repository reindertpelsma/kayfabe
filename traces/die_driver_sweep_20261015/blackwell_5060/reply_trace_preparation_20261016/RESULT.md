# Bounded reply-body diagnostics preparation

**STATUS: LIVE, 2026-10-16 12:36UTC. CPU tests green; hardware UNRUN.**

MEASURED: clean draftPR22 source4d83e661f, test-firsta2373bd9c, initialcode
be7fffaaa. Actual GSP queue tests before51pass/3fail, after54pass/0fail.
Initial field/trait integration compilation failure preserved. Childprocess
fixtures exercise actual successful/failed posts, byte/record limits, default
off, excluded static/board and unrelated controls, and guest reset preserving
lifetime budget. The logging point is after queue write-pointer publication,
including held-reply patches; no policy result is mistaken for a sent reply.

Default off, host environmentKF3_RPC_BODY_TRACE=1. Fixed512records/256KiB
payload lifetime/4096bytes per record, explicit truncation. Select NV0073
controls, public580.65.06 a58 display pushbuffer params, allocation replies,
and measuredOther(4) replies without assumed semantics. Static-info/board
replies excluded. No new guest RAM reads, host GPU verbs, completion, reply
mutation or interrupt filtering. Rawbodyhex remains rental-local; screen a
narrow allowed display set before exporting text, no raw static/UUID data.

IntegratedBW sourcebabe7d898 includes clean QGA deadline and safe fastguest
lifecycle fixes. QGA8/8 and lifecycle2/2 CPU tests pass on integration. Full
GPU workload regression and this new diagnostic's hardware run UNRUN.
Production Windows reports use exact P19 plusRPC_TRACE1/BODY_TRACE1 (21actual
variables), never called flagless; actual process env/source/SHA and host/guest
builds must be printed/asserted. Code PR22 remains draft againstclaude base.

MEASURED public595.91.07 tag fetchedcommit9f087a6d4e86d85acc0ce1d354d6276fe5047b29,
sourcekernel_gsp.c:2279-2318 shows PTIMER/offset/system-offset heartbeat formula,
including a conditional FSP workaround. Exact580.65.06 publictag also fetched
commit307159f2623d3bf45feb9177bd2da52ffbc5ddf9. No heartbeat code changed before
same-binary loglevel3 ablation. Headera58 names DISPLAY_CHANNEL_PUSHBUFFER,
not a memory-info query. Current host heartbeat authoring reads native usermode
GPU timer; clock-domain causality remains INFERRED until measured.

MEASURED market12:32UTC: no5060/5060TiKVMoffer, includingno-default/unverified
and multiGPU searches. AvailableRTX5070 offer54561458/machine153389,16CPU/
32109MiBRAM/600.5GBavailabledisk,0.2416666667USD/h including150GBrequested.
Planned nextBlackwell die perB4, exact die PENDING unprivilegedRM verification.
No GB206constant/pass copied. Priority adapts because GB206hardware unavailable;
GB206Code43 remains red. B3first on this freshdie, nativeWindows reference
before blaming kayfabe; do not reinterpret this as a GB206falsifier result.
