STATUS: LIVE,2026-10-22. Real GPU-free pthread lifetime reproduction.
FALSIFIER stated beforeprobe: leaderexe remainsreadable or ownedPopen already
exited whileworkerblocks onpipe. MEASURED: leaderZ,PF_EXITINGset,mainexeENOENT,
Popenstilllive,workerexe exactcapturedimage. Two regressionfailures at test-only
EXPECTED RED538b8e613; original23remain green. This explains a broader lifecycle
than the first poll-after-exit repair modeled; actual QEMU shutdown hit ENOENT
again,so completion gate correctly refusedtimerbefore successor,no newXids.
MEASURED afterfix:117guard+5abort=122PASS. Three manualmutants each2distinct
killers/0errors;restoredgreen. Realownedprocess tests also rejectchangedworker
images,starttimes,and missingliveexecutables. No sleeps in the reproduction.
CODE READ: mainexe missing is acceptable only after kernel PF_EXITING; every
retained task image must match,missing task images require exiting/reapedstate,
and mainstarttime is checked again. Process lifetime is still owned by Popen;
final exact proof+successfulwhole-process exit+atomiccompletion remain required.
PF_EXITING0x4 confirmed in Linux public sched.h installedheaders. No timeouts,
observation intervals,guest fault matching or recovery criteria changed.
MEASURED: currentboot baseline remains exactnative5;no guestfault ever occurred
in this attempt. Authorized samelease continuation can therefore retain this
full history without another reboot or a new nativefault sequence. Native31b56
product source and four suite files must be byte-identical to newharnesssource.
