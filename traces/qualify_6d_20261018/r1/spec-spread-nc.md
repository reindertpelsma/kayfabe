## Repeat spread (nc)

> Release: SPECviewperf 15.x (patch level not recorded)
> Configuration: RTX 4070; KVM/kayfabe 01a171e63 versus KVM/VFIO; Windows 11 NVIDIA 580.88; 1920x1080; 12 GiB guest RAM, 8 vCPU. Production store 8192 MiB with Wayland; K2 store 9216 MiB with GDM stopped and synthetic 60 Hz test backend; VFIO guest reports 12282 MiB. Capacities and carriers are not matched.
> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

| arm | workload | n | minimum composite | maximum composite | mean composite |
| --- | --- | --- | --- | --- | --- |
| 6d-production | catia-07 | 2 | 49.39 (nc) | 49.46 (nc) | 49.425 (nc) |
| 6d-production | creo-04 | 2 | 69.31 (nc) | 69.99 (nc) | 69.65 (nc) |
| 6d-production | energy-04 | 4 | 41.16 (nc) | 44.6 (nc) | 42.1675 (nc) |
| VFIO-reference | blender-01 | 2 | 60.61 (nc) | 61.61 (nc) | 61.11 (nc) |
| VFIO-reference | catia-07 | 2 | 63.1 (nc) | 63.13 (nc) | 63.115 (nc) |
| VFIO-reference | creo-04 | 2 | 103.97 (nc) | 104.94 (nc) | 104.455 (nc) |
| VFIO-reference | energy-04 | 2 | 54.57 (nc) | 54.67 (nc) | 54.62 (nc) |
| 6d-K2-9216MiB | blender-01 | 2 | 36.08 (nc) | 41.42 (nc) | 38.75 (nc) |
| 6d-K2-9216MiB | catia-07 | 2 | 50.91 (nc) | 51.47 (nc) | 51.19 (nc) |
| 6d-K2-9216MiB | creo-04 | 2 | 71.44 (nc) | 72.5 (nc) | 71.97 (nc) |
| 6d-K2-9216MiB | energy-04 | 2 | 44.43 (nc) | 44.59 (nc) | 44.51 (nc) |
