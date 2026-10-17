STATUS: ANSWERED, 2026-10-17. GA104 Windows580.88 binding UNVERIFIED after one incomplete setup attempt and one saved-state recovery.

Correction: the before-launch marker proves the script reached the setup launch boundary.
It does NOT prove child process creation/success. Actual NVIDIA child PID/exit were lost
when the host-side QGA program deadline expired; do not call this a measured child timeout,
Code43, unsupported GPU or a successful NVIDIA installation. The guard correctly refused
any installer replay. The earlier recovery hypothesis/falsifier overstated this marker's
power to prove child execution; marker presence makes the launch outcome uncertain.

| Phase | Measured result |
|---|---|
| Fresh stock LTSC26100 baseline | PASS allplatformchecks, separateverificationboot, SSH/QGA |
| GPU staging QEMU106866 | QGAready03:17:13; source9d/privatecc2a/58065.06/actualP19 |
| Guest program4196 | Host600s deadline03:27:38; original VMguestshutdown03:29:05 |
| Saved-state stock recovery | QGAready03:37:49; before-launch and exit markers exist |
| Stored NVIDIA device | VEN10DE/DEV2484; actualDriverVersion10.0.26100.1 (BasicDisplay), not8088 |
| GSP configuration | Stored class null→1/service1 configured; actualNVIDIA GSP operation UNRUN |
| Recovery exit | Guardrefusal03:38:13; ownstop/QMPquit03:40:14; no furtherbootloop |
| CUDA/D3D/desktop/420hold | UNRUN: actual8088binding never verified |

MEASURED source9d6f210ba1dc6ab0edbc49ff265101091e8b08f1; privateQEMU
cc2a86a6dec3ec6bed91d9cd6e1963a1deadf2d75a768e9e3dada91559e34668;
host595.91.07; officialNVIDIA580.88 packageSHA90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9.
All19 owner flags printed in flags-measured.txt and actualprocessreceipt; VGA=none/GOPon.
This is not a flagless result. Source's packagehash/EXE+setup signature guards precede
the before-launch marker; their stdout was not recovered, so this is control-flow
corroboration rather than a retained signature-report transcript.

Saved marker03:22:52.395945Z; saved SCRIPT exit rc1 at03:28:54.159449Z. Initial preparation
interval fromhostPS_START03:17:37.571046 was314.825s. Script advertises childWaitForExit360s;
parent600s does not cover that measured preparation plus childwait/capturemargin. The
outer deadline fired before the advertised childwait could finish. The marker-to-exit
interval~361.76s is consistent with innerWaitTimeout, but actualchildexitcode is UNKNOWN.
Budget correction prepared separately, CPU failing-first PASS; hardware rerun UNRUN.

410 periodic GPU samples allphaseCold, maxima trapped/applied/refused/serviced all0.
This is not a full RPC trace; no first unload/refusal ordinal can be assigned. HostXid
journal10→10 exactunchanged. Old GA104 QGA timeouts remainhistorical; this freshstaging
boot reachedQGA, with no isolatedoldfailurecause claimed. No595caps scalar enters580.

Read-only offline extraction found full30653Bsetupapi.dev.log; retained printerINF
prnms009 errors are unrelated to NVIDIA. No NVIDIA/10DE records in this retained file;
do not infer globalabsenceofdriverwork from one log. Full sourceSHA/bytes in offline
manifest; no image/executable/privatekey/password returned. First ntfscat invocation
used an unsupported length option and returned no files; corrected bounded reader
succeeded, both ownNBD8 attachments detached. Initial-query PIDstatus was attempted
AFTER original shutdown and returnedQgaError; that has no installer diagnostic weight.

NEXT: same-die native/VFIO Windows580 reference or verified NVIDIA image; use the repo's
INF/GSP-before-first-start order (scripts/bench/windows/gsp_on.ps1), verify actual8088 and
class+serviceGSP1, then one coldP19 run. NativeGA104Windows580 remainsUNRUN. No more
wallclearing until comparison; no guessedreply/capturedpage or familycompletion claim.
Onlyown56326220active/quiescent at export; manualretire04:15UTC, originaltimer04:25:28,
absoluteAPIabsence05:00UTC/07:00CEST. Linux595functionalraw30/CUDA4/gfx10/brokerPASS separately.
