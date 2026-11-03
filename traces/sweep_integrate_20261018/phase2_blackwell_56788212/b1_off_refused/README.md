# GB202 B1 display-off launch refusal

STATUS: LIVE,2026-10-19. Initial NOTRUN/guard STOP; no guest initialization.

2026-10-19 correction: section2 already authorizes displayOFF. Correcting
contradictoryhelperarguments to the explicitlysupported emptyROMconfiguration
satisfies the existingguard; no protectedcode/guard/TPM/SBchange. Extraapproval
question was unnecessary; no reply or elapsedtime is being treated asapproval.
Proceed under the prior B1 authorization after the current serialLinuxsuite.

The stockWindowshelper always adds gop-efi=<signed image>. Asking its extra
properties for display=off,gop=off leaves that nonempty ROM attached. QEMU
correctly refuses before anything is reserved: gop-efi needs gop=on.
QEMU129170 exited1/noevents at22:32:06. No QGA/NVIDIAinit/Code43/TDR verdict.
The waitingcontroller was stopped (ownPID129049/descendants129797 only); its
EXITtrap misleadingly printsrc0 afterTERM, which is NOT a PASS. Preserve both.
COMMON.md explicitguardSTOP reported and coordinator asked whether to omit
the signedROM for OFF. The configuration below uses the prior section2 authorization.
IndependentLinuxraw595 began22:33:40 after globalnoQEMU guard.

CODE READ: SignedGop::for_config(false,Some("")) returnsNone; existing
a_signed_copy_is_served_and_gop_efi_without_gop_is_refused test pins that and
the nonempty-ROM refusal. This was included in phase1full407QEMUtests. The
proposed extra properties are display=off,gop=off,gop-efi=; no product or guard
change, no TPM/SBremanu, samefrozenparent/acceptedbinary/P19. Use a fresh child/marker, preserving the refused attempt.
