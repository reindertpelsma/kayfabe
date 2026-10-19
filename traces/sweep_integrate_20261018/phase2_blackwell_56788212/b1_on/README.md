# GB202 Windows580.88 display-on arm

STATUS: LIVE,2026-10-19. MEASURED FAIL initialization/connectivity; Code43 and TDR UNVERIFIED.
Freshchild of frozen stopped8088/oem7 parent, originalTPM/SBidentity retained,
QEMU24906bf33/exactP19process asserted. Coldstart22:22:07; actualdisabled
8088/GSP1 verified again. One enable22:26:26; pnputil rc0. SSH then disconnected
(255), before CIM/status returned; no retry/replay. Postdisconnect SSH255 and
QGAping1, QMPquery-status running, guest windowblack, QEMUalive656%CPU.

Correction folded here:99calls was a transient snapshot. Final export after
cleanup has386calls, CA70@233, CA7A@271/273/275/277, CBCB@150/refused,
2801@151/refused,2806@162/refused, explicitfn47unload385. Later display
traffic appeared between the unresponsive snapshot and final shutdown; its
exact time is unknown. Powerdown was requested22:29:51, QMPquit22:31:57
after126s grace; QEMU exited22:31:58 rc0/host-qmp-quit, no new hostXid.
Therefore the final unload is NOT proven to be a StartDevice-abort ordinal: it
may be associated with requested shutdown. Code43/TDR remain UNVERIFIED.
The assertion that final metadata equalled the99call snapshot correctly failed;
no equality claim accepted. Both snapshots/projections are retained.

Early99call Ada alignment (94matched/5GB202-only) remains explicitly an
early snapshot; it cannot locate the final failure. Full final alignment and
public reply comparison are separate. Fullsuite/hold UNRUN. OptionalCE95e23df77
unmerged/UNRUN because no actualTDR was established.
Resolved OFF configuration: the valid empty-ROM OFF arm completed later; see
../b1_off/README.md. Existing section2 authorization covered it, so the extra
approval question was unnecessary. No guard or protected code was changed.
The original OFF sibling refused22:32:06 before VM initialization: helper includes signed
gop-efi but requests gop=off. Exact guard STOP reported; coordinator asked
whether to omit the ROM for authorized OFF profile. No guard bypass.
Only APImetadata/publicscalarprojections exported; opaqueWindowsbodylog staysrental.
