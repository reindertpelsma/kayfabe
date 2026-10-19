# GB202 B1 display-off launch refusal

STATUS: LIVE,2026-10-19. NOTRUN/guard STOP; no guest initialization.
The stockWindowshelper always adds gop-efi=<signed image>. Asking its extra
properties for display=off,gop=off leaves that nonempty ROM attached. QEMU
correctly refuses before anything is reserved: gop-efi needs gop=on.
QEMU129170 exited1/noevents at22:32:06. No QGA/NVIDIAinit/Code43/TDR verdict.
The waitingcontroller was stopped (ownPID129049/descendants129797 only); its
EXITtrap misleadingly printsrc0 afterTERM, which is NOT a PASS. Preserve both.
COMMON.md explicitguardSTOP reported and coordinator asked whether to omit
the signedROM for OFF. Proposed configuration below is not yet authorized.
IndependentLinuxraw595 began22:33:40 after globalnoQEMU guard.

CODE READ: SignedGop::for_config(false,Some("")) returnsNone; existing
a_signed_copy_is_served_and_gop_efi_without_gop_is_refused test pins that and
the nonempty-ROM refusal. This was included in phase1full407QEMUtests. The
proposed extra properties are display=off,gop=off,gop-efi=; no product or guard
change, no TPM/SBremanu, samefrozenparent/acceptedbinary/P19. Pending explicit
answer, do not run this dependent arm.
