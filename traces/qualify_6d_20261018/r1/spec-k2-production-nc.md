## K2 versus production: (nc) research comparison

> Release: SPECviewperf 15.x (patch level not recorded)
> Configuration: RTX 4070; KVM/kayfabe 01a171e63 versus KVM/VFIO; Windows 11 NVIDIA 580.88; 1920x1080; 12 GiB guest RAM, 8 vCPU. Production store 8192 MiB with Wayland; K2 store 9216 MiB with GDM stopped and synthetic 60 Hz test backend; VFIO guest reports 12282 MiB. Capacities and carriers are not matched.
> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

| workload | 6d-production composite | 6d-K2-9216MiB composite | ratio 6d-K2-9216MiB/6d-production -- (nc) research comparison, not a SPEC-ranked result |
| --- | --- | --- | --- |
| blender-01 |  | 38.75 (nc) |  |
| catia-07 | 49.425 (nc) | 51.19 (nc) | 1.04 (nc) |
| creo-04 | 69.65 (nc) | 71.97 (nc) | 1.03 (nc) |
| energy-04 | 42.1675 (nc) | 44.51 (nc) | 1.06 (nc) |
