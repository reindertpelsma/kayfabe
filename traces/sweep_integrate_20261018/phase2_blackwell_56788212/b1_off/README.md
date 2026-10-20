# GB202 Windows580.88 display-off arm

STATUS: LIVE,2026-10-19. INCOMPLETE/initunresponsive;Code43/TDRUNVERIFIED.
Owner follow-up, 2026-10-20: explicitly approved omitting the GOP ROM for the
display-off arm. The reply arrived after the valid arm and teardown described
below. That earlier run used the existing section2 authorization; no additional
initialization was performed in response to this later approval.
Second/finalmeasuredWindowsinitializationthislease. Samefrozenstoppedparent
hash49b96c...5414ca recheckedOK; freshchild, originalTPM/SBidentity, accepted
QEMU24906bf33/P19allasserted. Initialcontradictory-ROMrefusalwasNOTRUN.
Priorsection2authorizationcoversvalidOFFconfiguration;extraapprovalquestion
wasunnecessary, no reply/elapsedtimeusedasapproval. ExistingemptyROMpositive
testandguardretained;no product/protectedcodechange.

Coldboot23:16:54, actual8088/oem7/disabled/GSP1verified, oneenable23:21:28.
Pnputilrc0thenSSH255at23:23:11 beforestatus;QGAping1/QMPstillrunning.
At23:23..23:24boundedtrace0GSPRPCs/0CA70/0CA7A/0fn47, phaseCold,
0trappedwrites/readexits. ThisdoesnotreproduceONdisplayteardownpath.
ActualQOMdisplayfalse/gopfalse/ROMempty/ABI580.65.06querieddirectly.
NoCode43/TDRorfullsuiteclaim;absenceofdisplayallocationsfromanuninitialized
GSPcannotsettlethedisplay-causalityfalsifier. NoCEarm/no3rdinitthislease.
Powerdown23:25:24; firstquitcheckat117srefusedandwasobeyed. After138sgrace
QMPquit23:27:42, cleanprocessexit23:27:44/host-qmp-quit/0newhostXid.
Finaltracealso0RPCs/0allocations/0unload; finalrawWindowspayloadlog687459bytes
stayedrental. Thusnonstrictsame-abort/display-causefalsifierNOTSETTLED.
No moreWindows/B2initthislease. Headlessgfx595preparationbeganafterall
WindowsQEMUprocessesexited.
