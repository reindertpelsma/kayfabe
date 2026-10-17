# Native H-BC3 replay result

**STATUS: LIVE, 2026-10-16. Complete-capture FAIL; native applications PASS.**

Own56165326, source1d302de82, unsignedmodule62e89c...f530, actualWindows
32.0.15.8088/GSP580.65.05 before/after both native oracles. NativeCUDAcopy/kernel/
matmul andhardwareD3D11/12 PASS,zeroSystem4101TDR in both short app windows.
WindowsUpdate briefly installed610.88 while580setup was in flight; actual580
was restored BEFORE capture and unusedoem13/NVIDIA32.0.16.1088 removed while
active580package was healthy. Qualitydriverupdate policy1; service status is
not claimed disabled across reboot (wuauserv returned toRunning/Manual).

| Capture | Discovery | Firstseq cmd/status | Messages cmd/status | Loss records | Result |
|---|---|---|---|---:|---|
|seed580|method2|3162/3208|1647/1658|2|census1,sequence1,completeFAIL|
|revisit580|method2|3178/3233|1653/1664|2|census1,sequence1,completeFAIL|

Allcaptured frames havezeroexactloss/gaps/torn/invalid/drop/refused counters,
but BOTHdirections haveunseen initialization prefixes. Seedlost6370messages,
replaylost6411. Method3 neverattached; H-BC3falsified. The code's unit/WDK
passes did not prove nativeallocatorreuse. No native-complete reference or
aligned firstreply/cause claim is justified. NativeGPUwork itself passed.

The first scan read3982782464bytes; second4327407616bytes. Difference344625152
bytes is consistent with a moved table [INFERENCE], not proof of its physical
address or the reason the exact-point validator rejected it. No address was
exported. Fullrawfiles remainonlyonrental C:/ProgramData/KayfabeSweep/{seed580,
revisit580}/capture.kfgspcap. Textmetadata andcensuses are saved beside this file.

Next rankedcapture hypothesis H-BC4: sameGPU's validated oldtable can prioritize
a bounded1GiB forwardRAM window for15seconds before the ordinarybulkdelay.
Scan inexisting<=1MiB steps and<=16step bursts, thenyield; requirethe samefull
validator andempty/checksummedseq0bothdirections. Reportmethod4 distinctly.
Thisis a genericordering/budget heuristic, NOTa per-die address/fact or runtime
reply. Falsifier: noearlybind, nonzero firstseq, staleepoch, loss,corruption,
unbalancedcontrols or nativeappfailure. One furtherboundedreplay after CPU/WDK
checks; no guessedCCU/CMC/CBCBreplies. If false, stop this discoverywall and rank
measuring the actualFSPboot chain against captureperformance. Rentalstillactive,
originaltimer2120908 unchanged01:39:19UTC/hardcap01:44:19.
