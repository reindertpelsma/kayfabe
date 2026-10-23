## Instrument sanity: (nc) research comparison

> Release: SPECviewperf 15.x (patch level not recorded)
> Configuration: Separate diagnostic arms4620/4622, one boot and one CATIA/Creo iteration each; same RTX4070, Windows11 NVIDIA580.88, FullHD1920x1080,12GiB RAM,8vCPUs,9216MiB store, GDM stopped, synthetic60Hz test backend. GPU200ms, CPU1s, short KVM collectors active. Baseline01a171e63 KF3_PROF off; instrument70f7fb591 KF3_PROF=1 and KF3_PROF_USER_SUBMIT=1. Profiling sanity comparison only; n=1 cannot establish overhead or repeat spread.
> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

| workload | baseline01a-profOFF composite | instrument70f7-profON composite | ratio instrument70f7-profON/baseline01a-profOFF -- (nc) research comparison, not a SPEC-ranked result |
| --- | --- | --- | --- |
| catia-07 | 51.36 (nc) | 51.43 (nc) | 1.0 (nc) |
| creo-04 | 73.94 (nc) | 74.02 (nc) | 1.0 (nc) |
