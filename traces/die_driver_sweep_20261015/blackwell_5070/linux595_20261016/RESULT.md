**STATUS: LIVE, 2026-10-16 13:44 UTC.** B3 bounded result: normal-logging concurrency and raw30 pass; startup heartbeat warnings remain a named gap.

MEASURED: rental56246613 is GB205 (arch0x1b0/impl5), independently queried as UID65534/CapEff0. Host and thin guest are595.91.07. Native Linux concurrency passes4s. Native gates pass9/9, gate10PASS,12/12 USER births. The privileged read-only caps oracle has127 nonzero words and0x00c=0x102; this is a Linux595 oracle, never a shipped page or a Windows580 fact.

| GB205 Linux595 test | Actual source | KF3 environment | Loglevel | Budget | Measured result |
|---|---|---|---:|---:|---|
| Native concurrency |99ce8207c|none|n/a|180s|PASS4s|
| Before, noisy |99ce8207c|{}|6|180s|TIMEOUT180s|
| Before, quiet |99ce8207c|{}|3|180s|PASS18s; raw client7140ms|
| PTIMER selector candidate |277e5d895|{}|6|180s|TIMEOUT180s; heartbeat still offset twice|
| Fn72 offset candidate, targeted |304ca9adb|{}|6|180s|PASS18s; client7009ms; zero heartbeat warnings|
| Fn72 offset candidate, full suite |304ca9adb|{}|6|180s per arm|PASS30/30;13:33:39–13:41:03UTC|

The same binary and initrd were used for the noisy/quiet ablation. Process receipts assert /proc/exe, argv, actual inherited KF3 environment, host/guest versions and loglevel. The noisy serial file is5,290,164bytes with41,645 heartbeat-related lines and20,823 formula lines; only a screened excerpt, size and SHA return to git. Quiet serial is5128bytes: suppressed messages do not prove a repaired clock. The console-storm falsifier did not fire on GB205. No GB206 result is inferred from this die.

The first candidate was hardware-FALSIFIED: the validated native PTIMER page, like NV_VIRTUAL_FUNCTION_TIME, already contains a system offset. Its CPU tests proved selector behavior but rested on a wrong clock-domain assumption. DraftPR23 remains unvalidated until the second hardware candidate passes.

Public595.91.07 source (ogkm9f087a6d4e86d85acc0ce1d354d6276fe5047b29) provides the missing derivation: kernel_gsp.c _kgspPrepareSystemInfo sends sysTimerOffsetNs in fn72; _kgspIsHeartbeatTimedOut separately divides that value by1e6 and adds it to the heartbeat. Existing compiled GSPSYSTEMINFO layouts already describe the field. Second test-first commitdeff13df8 gives4pass/5fail before; fixe0b4c28e6 gives290 kf-qemu tests pass. Hardware source304ca9adb subtracts the guest offset from the native system-clock milliseconds with wrapping arithmetic. No per-die constant, captured page, changed budget or logging workaround.

Second-candidate falsifier, declared before run: normal loglevel6 still reports heartbeat timeout or concurrency exceeds180s. Full raw30 starts only if the targeted run passes and heartbeat warnings are absent. Native gates are rerun and the per-revision QEMU, raw client and thin595 guest rebuilt first. Full Linux/CUDA/graphics/broker and all Windows acceptance remain UNRUN until their own receipts exist.

Lease: created12:40:48UTC; timer15:35:50UTC, hard cap15:40:48UTC. Rate0.2416666667USD/hour, max0.73USD; campaign conservative ceiling5.09USD plus transfers. Only own56246613 is active.

MEASURED final: gates9/9+gate10PASS; full raw30/30 at180s, all31 actual process receipts have KF3 env{} and loglevel6, source304ca9adb10528ac73d809636174625199f40ba1. The concurrency falsifier did not fire. The final QEMU SHA is2cc299c67b35241d7b8b5692a5b062eef02fbc1e3125975bf64512b988e64903; raw client remains0db4104321bc9f38d73d7e54e80902646a93547702f3218deb14295bebd4c0bf. Thin guest rebuilt before the full run (SHA728792631cbc92e391221035c9bd7f77a954ee4ecb679b781af6b765751c91df). Full suite and birth census text are saved. This is raw-suite acceptance, not full Linux CUDA/graphics/broker acceptance.

MEASURED limitation: the full suite has74 startup heartbeat-formula warnings in six arms, all with heartbeat0 and a current time just before offset time; the targeted concurrency arm has none. Thus the sustained doubled-offset storm is fixed here, but warning-free startup is NOT claimed. INFERRED remaining cause: the drainer advances its500ms heartbeat deadline even before fn72 offset state is available, delaying the first real sample. This is also a drainer-readiness issue, not only clock conversion. Preserve this in the draft; do not walk another wall before the Windows reference. Six generic strings mentioning Xid are raw-client explanations of fault tests, not observed NVRM guest Xids; the warning audit stores actual NVRM matches separately.

One initial attempt of304ca9adb was NOTRUN before QEMU launch: the parent SSH command included the lane-name text, which the harness's broad live-process guard matched. The corrected launch removed that parent text; the NOTRUN output remains archived and is not a hardware failure or pass.

Exact caps equality on native595 is RED:126 differing words;0x00c is explicitly unmeasured for GB205. Checker attempt against the old CPU checkout lacked a595 class table and exited2 (not a comparison). Repeated with own caps-oracle-runtime0b6c501f plus diagnostic-only PR15 differences; class595/capsderive SHA match the hardware source (capsderive70955fcb88cb8eeb2ea46d7f61c7c7c4572ef23b44f5e3a1d2c79d1ea1cd14d7, classes3252dacacb9602f0006d8061c3778ded4b9be3c99b7ee0294a5bd9c5d3ddbda2). No captured page or neighbor value is installed. Next: native Windows580 stock workload and fixed-aperture caps query, then the nested guest. GSP queue replay/body capture remains deferred until the virtual display bodies show a difference worth checking, as section2 requires.

B3 timing12:54:05–13:44UTC is inside its one-hour box. This is GB205 evidence; GB206595 revalidation is UNRUN. PR23 remains draft; no merge or overall family completion.
