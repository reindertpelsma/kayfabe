# GB202 verified Windows preparation (2026-10-19)

STATUS: LIVE,2026-10-19. Preparation PASS; GPU suite still UNRUN.
Generic Windows11EnterpriseLTSC2024Eval installed from pinned public media,
stdVGA only for installation/check. OriginalTPM/SecureBoot state retained.
Actual kayfabe preparation QEMU123704 uses24906bf33 SHA
b33394dd23e515994a4478747a52311d05f1a1bffbb300eb4415f2a0e22220ef,
VGA none, display/GOP on, exact19flags asserted from /proc.

At22:21UTC --no-enable installation verified NVIDIA RTX5090/32.0.15.8088,
oem7.inf, Problem22/disabled. Both actual class key and nvlddmkm service
EnableGpuFirmware read back1. No installer return-code substitution for binding.
Cold-ready verification passed; VM stopped22:21:23, controller rc0.
Stopped driver-bound disk is frozen0400; ON/OFF create separate child overlays.
First coldON boot22:22:07. No NVIDIA initialization verdict yet.
