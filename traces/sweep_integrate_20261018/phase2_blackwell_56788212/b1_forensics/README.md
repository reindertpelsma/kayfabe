# Rental-only read-only Windows evidence helpers

STATUS: LIVE,2026-10-19. CPUchecked; fixedhardwareextractor notyet rerun.
The first extractor correctly stopped/disconnected after a falseparserrefusal:
lsblk PATH/SIZE/FSTYPE returned a flatlist, but code assumed firstroot.children.
The saved receipt actually contains two NTFS partitions. Failing-first67092b587
reproduces two flatlist errors plus an ownership-guard gap. The fix accepts
flat/nested layout, bounds traversal128nodes, admits only actualNTFS partitions
of our exactNBDnode, and keeps>1GiB/max-volume guard. No filesystem guess.
Nine testsPASS;5mutants have2runtimekillers each (bytecodecache removed).
10syntheticEVTXrecords verifypositive/negative/time/PCI/redaction filters.

No product/QEMU/Rust behavior changes;3338cratecountsremainunchanged.
read_only.sh runs only after raw595PASS and globalnoQEMU, attaches stoppedON
image RO, assertskernelRO, never mounts-write/repairs/recovers the volume,
then disconnects only its verifiedread-onlyNBDPID. RawEVTX/XML remainrental.
No NVIDIAactivation or Windowsboot and no ROM/TPM/SBchange. Initialfailure
marker is retained; fixedextractor has its own explicitattemptrecord.
The separate GOPguardSTOP/question remainspending; no OFFhardwarearm allowed.
