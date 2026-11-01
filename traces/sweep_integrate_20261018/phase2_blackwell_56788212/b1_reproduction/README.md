# GB202 Windows B1 reproduction

STATUS: LIVE,2026-10-19. Preparation active; hardware verdict pending.
These are lease-only controllers, not product changes. Baseline24906bf33
contains candidate5f8f9d1a5f. Generic LTSC installed with standardVGA; actual
kayfabe boots use VGA none, originalTPM/SecureBoot state and exactP19flags.
Only own56788212; no other instance/host or CE branch used.

bind.sh stages580.88 with --no-enable and checks actualbinding/GSPflags.
arm.sh requires successful binding and stopped QEMU, creates freshON/OFF
children of that stopped parent, verifies disabled actual8088 before one enable.
If any guard refuses, preserve the failure and stop that dependent arm.
No timeout change or repeated driver activation. OFF requests display=off,gop=off
with no substituteVGA; this differs from older stdVGA diagnostics and is explicit.
A Code0 ON result requires separate fullsuite and>=420s zeroTDR hold; controller
completion alone is not a PASS. No opaque Windows payload is exported;
metadata and public scalar-field projections only.
