## Separate profiled-run composites: (nc) research comparison

> Release: SPECviewperf 15.x (patch level not recorded)
> Configuration: Separate diagnostic profiling arms, not headline score timing. RTX4070; KVM/QEMU, Windows11 NVIDIA580.88; FullHD1920x1080,12GiB guestRAM,8vCPUs; 6d01a171e63 with9216MiB store and synthetic60Hz test backend versus VFIO12282MiB with physical display. Host NVML595.91.07 versus guest physical NVML580.88; capacities, carriers and driver telemetry windows differ. GPU/CPU/KVM collectors active.
> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

| workload | 6d-profile composite | VFIO-profile composite | ratio 6d-profile/VFIO-profile -- (nc) research comparison, not a SPEC-ranked result |
| --- | --- | --- | --- |
| catia-07 | 51.36 (nc) | 63.67 (nc) | 0.81 (nc) |
| creo-04 | 73.94 (nc) | 107.36 (nc) | 0.69 (nc) |
