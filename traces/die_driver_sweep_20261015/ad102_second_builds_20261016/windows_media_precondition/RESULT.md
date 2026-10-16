# Windows payload-media precondition failure and recovery

**STATUS: LIVE, 2026-10-16 22:43UTC. Harness failure measured; second-driver GPU validation UNRUN.**


Correction 2026-10-16 22:46UTC: the first manual four-probe command acknowledged bytes but Bash escaped
its filename variable, creating a literal filename instead of cup2/3/8/DXprobe. The readonly
inventory identified this, but a negated grep in the first continuation was exempt from Bash
errexit and failed to stop it. Fresh reference acceptance then failed on a missing cup3.exe
BEFORE any GPU probe ran. The controller was interrupted and its owned VM stopped without
SIGKILL; this attempt is harness failure and is excluded from row-ablation conclusions.
The NVIDIA218chunk stage independently completed successfully in94.709s, using Python argv.
Corrected probe staging now uses Python PureWindowsPath+subprocess argv and the verifier throws
on any missing file. V2controller queues after verified old-controller/VM exit, stages probes
into a separate repair sibling, then freezes a new verifiedpayload parent for all fourtests.
No installer has yet started. Earlier continuation description is superseded by this correction.

Own56302930, actualAD102/PCI2684, native595.91.07. Source9d6f210ba/privatebinary
cc2a86a6dec3ec6bed91d9cd6e1963a1deadf2d75a768e9e3dada91559e34668.
Original fresh baseline guest actual580.88/32.0.15.8088/GSP1/Code0/DWM1/Explorer1,
WindowsEnterpriseSEval build26100/UBR1742; P19 flags in savedflags/processreceipt.
This wave did not run the application suite: its staging precondition failed.
The earlier separate Ada A2/A5 passing cells remain valid.

MEASURED: QEMU attached read-only IDE payload ISO, but Windows enumerated only C:;
CD devices were phantom/problem45. Probe staging exited before any probe executed.
The 581.51 setup script exited at its CD-count check BEFORE hash/signature checks,
BEFORE writing driver58151.started and BEFORE Start-Process. Installer never started;
this is not an NVIDIA initialization failure or an ambiguous installer retry.
Actual8088/GSP1/Code0 remained present in a separately restarted owned VM.
USB-bot/SCSI-CD hotplug reported attached; pnputil /scan-devices still exposed no CD.
Stopped investigating media after these two bounded attempts.

Recovery uses the existing bounded QGA file-put helper (PR28), with no mutating RPC
replay. Four probes transferred with pinned hashes, and the public NVIDIA581.51
910810544-byte package is being sent in218chunks of at most4MiB. Every chunk has
an acknowledged byte count/SHA; Windows will join and verify the full pinnedSHA
cb04aa1e2fbb977ee3911ab2cf51a42351f8126ba991bc9308ce6bf0ba96e83a.
Public 7zr.exe from https://www.7-zip.org/a/7zr.exe is pinned to
256feca8e274e5da655e2a284fabafd9f554365eb164862089dacd4e8276d282.
Archive extraction is packaging only. Both NVIDIA package and setup signatures
must be Valid/NVIDIA before the first installer invocation.

Continuation controller101374 first waits for staging, safely stops the oldVM,
freezes bound580-payload, and runs a fresh reference and the three one-row ablations
from identical parent/VARS/P19/noCD profiles. Then the first real581.51 installation
gets its own sibling, actual8151/GSP1 verification and a cold580.94.02/P19 acceptance
boot with CUDA3/DX11/12/nonblank desktop/at least420s/zero readable-System-log TDRs.
All continuation results remain UNRUN until their exit records and receipts exist.
Only text logs/receipts exported; guest images, executables and raw frames stay on rental.

Export normalizes CRLF/trailing whitespace and redacts identifiers; manifest hashes/sizes refer to raw rental files.
