# Read-only persisted event evidence

STATUS: LIVE,2026-10-19. Prepared/fixturechecked; actualimage UNRUN.
After Linux595 raw completesPASS, and only with no QEMU running, attach the
stoppedON child using read-onlyNBD, assertkernelRO=1, readNTFSfiles without
mounting/repairing/recovering the volume, then disconnect only our verifiedPID.
No new Windows boot/driveractivation and no ROM/TPM/registry change.
RawEVTX/XML stayrental; export only selectedpublic event identifiers/numeric
problem fields, timestamps and hashes. Absence cannot prove noTDR/Code43 after
forcedQMPshutdown because buffers may be unflushed. Positiveevents can settle
what could not be queried interactively. Entire operation bounded.

10syntheticrecords exercised knownpositive4101/116 andCode43, oldtimestamps,
wrongprovider/PCI, nonTDR119, ordinarynvlddmkm153, Problem0 andprivatefields.
Only4101fromDisplay or verifiedTDR116/117WERcodes countaspositiveTDR evidence.
119is a schedulerfatalviolation, notautomaticallyTDR. Code43requiresactual
NVIDIAPCIinstance andProblem0x2b withKernel-PnP411 insideONtimewindow.
Microsoftsources: [Code43](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/cm-prob-failed-post-start),
[TDR117](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/bug-check-0x117---video-tdr-timeout-detected),
[119](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/bug-check-0x119---video-scheduler-internal-error).
This is a rental-onlyevidence helper; no productbehavior or cratetestcountschange.
