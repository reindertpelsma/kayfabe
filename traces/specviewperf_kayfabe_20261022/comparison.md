> Release: SPECviewperf 15.x (patch level not recorded)
> Configuration: Windows guest (owner image) in a QEMU/KVM VM; arms: kayfabe kf3 virtual GPU (baseline binary 89fc8a222, Phase 1 binary 468a35c9d) and a VFIO physical-GPU reference (GeForce RTX 4070, see the vfio-ref branch); host GPU of the kayfabe arms is not restated here
> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

MEASURED: vendor workload composites. VFIO n=2; baseline/fix n=1 when Completed.
CODE READ: blank scores/ratios mean unavailable; failed/incomplete output is not a composite.

| Workload | VFIO 1 / 2 | Baseline | Phase 1 | Phase 1 / VFIO mean -- (nc) research comparison, not a SPEC-ranked result | Status baseline / fix |
|---|---:|---:|---:|---:|---|
| blender-01 | 61.61 (nc) / 60.61 (nc) | 35.85 (nc) | 35.95 (nc) | 0.588283 (nc) | Completed / Completed |
| catia-07 | 63.1 (nc) / 63.13 (nc) | — | 50.83 (nc) | 0.805355 (nc) | Failed / Completed |
| creo-04 | 104.94 (nc) / 103.97 (nc) | — | 70.42 (nc) | 0.674166 (nc) | Incomplete / Completed |
| energy-04 | 54.57 (nc) / 54.67 (nc) | — | 46.46 (nc) | 0.850604 (nc) | NotRun / Completed |
| enscape-01 | 37.26 (nc) / 37.39 (nc) | — | — | — | NotRun / Incomplete |
| maya-07 | 162.14 (nc) / 160.37 (nc) | — | — | — | NotRun / NotRun |
| medical-04 | 50.54 (nc) / 50.58 (nc) | — | — | — | NotRun / NotRun |
| solidworks-08 | 29.51 (nc) / 29.19 (nc) | — | — | — | NotRun / NotRun |
| unreal_engine-01 | 63.58 (nc) / 63.7 (nc) | — | — | — | NotRun / NotRun |
