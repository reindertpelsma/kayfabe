# GB206 follow-up provisioning rejected: no GPU exposed

**STATUS: ANSWERED, 2026-10-16 19:08 UTC. Hardware NOTRUN; rental retired.**

MEASURED: advertised verified RTX5060 KVM offer56180125/machine46454 created
own instance56284522. Ubuntu22.04.5, KVM,146GiB disk,5CPUs and50379MiB RAM
worked. Linux PCI enumeration and sysfs both contain ZERO NVIDIA devices.
The installer therefore warns that no supported GPU was detected, even though
it installs the595.91.07 module file. Loading it reports NVRM: No NVIDIA GPU
found; provision_host_driver.sh correctly refuses OPEN_MODULE=no and exits4.
No unprivileged die identification, native gate, guest boot or GPU test ran.
This is an unusable advertised rental, not a GB206/kayfabe regression. Exact
physical die is unmeasured. Exclude this machine from subsequent rentals.

H-rental (INFERRED): the advertised VM exposes its GPU. Falsifier: no NVIDIA
PCI function. MEASURED falsifier fired; do not work around it with driver flags
or infer a die from the offer. The generic reused bootstrap checked KVM/disk
but omitted the earlier Blackwell bootstrap's PCI check; that omission delayed
rejection until driver installation. Future bootstrap checks PCI topology
before package installation or bulk image transfer. No product workaround.

All B1/B2/B3 hardware work in PLAN.md remainsUNRUN. CPU-only PR29 still passes
192display tests/2ignored. Integration36d134bc5 adds compiler-derived590/595
fields and passes its display CPU suite; no physical result. B1 on/off recipes
use fresh sibling overlays of one stopped driver-bound parent, actual process
flags/SHA/ABI receipts, exactly-once QGA execution and public-only body export.
Only after a usable Blackwell box and native gates may those runs begin.

Windows image transfer was explicitly cancelled by the owned controller after
six completed chunks. Its EXIT trap misleadingly prints rc0 after SIGTERM;
there is NO full-image integrity PASS. The partial image was erased with the
rental, never used or copied back. This is not a successful image transfer.
Only text returned. Provider logs and PCI census in provider_no_gpu/.

Owned56284522 destroyed/API absence verified19:07:54UTC, then exact timer
2463545 cancelled after command verification.18m21.7s at quoted0.1680556USD/h,
estimate0.051431USD, rounded-up0.06USD. Campaign compute ceiling7.06USD plus
unbilled transfers; no own rental remains. Earlier GB206 WindowsCode43 and
Linux580desktopNoValidModes remainRED. Ada's completed lanes remain unchanged.
