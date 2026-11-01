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
Guest CUDA595completed4/4PASS22:46:24..22:55:13UTC, allboot_rc0/valuechecks;
graphics/broker595 remainUNRUN. Fatguestheartbeatdiagnostics are reported
separately;0warningsabove applies only to the30rawseriallogs. Evidence and export hashes:
b3_concurrency595*, b3_heartbeat_summary.json, b3_export_receipt.json.

Additionalimportantfatguestresult at24906bf33: successful595CUP3boot has
144warnings inpreworkcapture and304inpostworkcapture (overlappingviews;
do notadd). Allare nonzero rawheartbeat, fromguest100.921698..106.674757s
(5.753059s span). First currentTimeMs1448864316/heartbeat5472/offsetadjusted
1448869753; lastcurrent1448870068/heartbeat5345/adjusted1448875322.
Unsigneddiff isnear2^32 becauseadjustedheartbeatisahead. ThisisNOTmerely
thefirstconcurrencyraw0startupissue. CUDAkernel43passed/0hostXid.
Completepre/postlogs inlinux595_cuda/rungs/cup3; no warningfreeB3claim.

H_CLOCK_VIEWS (INFERRED, beforeprobe): mappedPTIMERandVF_TIMEreadbytheguest
maydiffer. Existingkf-timer-probecomparesthem3timeswithoutGPUwork, UID65534
andnocaps. Falsifier: viewsagreewithinexisting10msboundineveryround.
ProbeplannedafterCUDA580ends; no heartbeat500ms/clock/timeoutchange.

H_CLOCK_VIEWSresult (2026-10-19): FALSIFIED for the native sampledviews. Existingprobe
ranUID65534/allcaps0/NoNewPrivs1, source24906bf33,3open/read/drop rounds.
PTIMERvsVFdelta5696/5696/5728ns, wellbelowexisting10msbound. NoGPUwork/VM.
Thisdoesnotproveguestepoch/lifecyclebehaviorcorrect; no clocksourcefixmade.
CUP3warningsare two shortbursts:144records/81.426ms at100.921698..101.003124
and160records/61.867ms at106.612890..106.674757, gap5.609766s. ItisNOTone
continuous5.75s storm. RawHB5472then5345; epochtransition/stalepublishedvalue
is an INFERRED nextlead, not proved or patched.500ms period unchanged.

Headless595gfx5/5+10hashPASSbutbeforecapture104nonzeroHBwarnings,after214
(overlapping).0capturedXid/flip. ThusnonzeroHBburstsalsorecurinasecondfat
workload; no clockfixclaimed. Linux580CUDA/gfxcaptureshave0HBwarnings.
