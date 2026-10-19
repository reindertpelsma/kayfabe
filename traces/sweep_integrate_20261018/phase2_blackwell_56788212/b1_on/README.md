# GB202 Windows580.88 display-on arm

STATUS: LIVE,2026-10-19. MEASURED FAIL initialization/connectivity; Code43 and TDR UNVERIFIED.
Freshchild of frozen stopped8088/oem7 parent, originalTPM/SBidentity retained,
QEMU24906bf33/exactP19process asserted. Coldstart22:22:07; actualdisabled
8088/GSP1 verified again. One enable22:26:26; pnputil rc0. SSH then disconnected
(255), before CIM/status returned; no retry/replay. Postdisconnect SSH255 and
QGAping1, QMPquery-status running, guest windowblack, QEMUalive656%CPU.

Bounded trace snapshot99calls, no quota loss. No CA70/CA7A/NV0073allocation,
no explicitfn47unload. LastAPIordinal98=90f10106/NV_OKpolicy; actualposted
effective result remains unknown. This is earlier than historicalGB206Code43;
it does not reproduce that abort point. Do not call pnputil success a PASS,
or infer a TDR/Code43 from the black window. Fullsuite/hold UNRUN.
OptionalCE95e23df77 unmerged/UNRUN because no actualTDR was established.
Powerdown requested22:29:51 with120s grace before documentedQMPquit fallback.
Only own shutdowncontroller descendants stopped; no GPUVM SIGKILL. Final
trace snapshot/cleanup and matchedOFF result will be recorded separately.
Only APImetadata/publicscalarprojections exported; opaqueWindowsbodylog staysrental.
