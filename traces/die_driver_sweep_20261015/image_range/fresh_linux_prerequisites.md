# Fresh restored-Linux prerequisites

**STATUS: ANSWERED, 2026-10-15.** Rental56096336, Ubuntu22.04.5 minimal,
running kernel6.8.0-138-generic. Provisioning failures, not hardware limits.

The NVIDIA595.91.07 installer first refuses because the running kernel's source
headers are absent. Installing linux-headers-6.8.0-138-generic succeeds, then
595.91.07 open loads and nvidia-smi reports the RTX4090. Teach the provisioner
to install missing matching headers before stopping/removing the current driver.

The next bench precondition refuses because the fresh host installed directly
at595 has no cached580.159.04 package. That package belongs to the independently
pinned fat guest. Download it atomically when absent instead of assuming the
host's earlier install cached it. The actual image's receipt still selects its ABI.
Shell syntax passes. The live recovery uses these same commands; full bench
provisioning and all guest hardware verdicts remain pending at this checkpoint.
