# ENS12 CPU checkpoint

STATUS: PARKED, 2026-10-28. No ENS12 hardware arm started; HA, HB and H1 remain untested.

CODE READ at source `5c98bbb197c9e0f6d1a17aeaa40c322831d7cd9d`: latency histories previously had only a first-slow dump and a VA-loop shutdown hook. The first dump could precede any slow split, while normal QEMU exit bypasses device teardown. Existing campaign markers for 7400/7401 say ACPI shutdown; there is no evidence of SIGKILL in those arms.

The candidate adds bounded live snapshots from the existing non-vCPU diagnostic poll and an independent, panic-contained normal-exit callback. Live output has one dump per second and 16,384 dumps per process; first-slow and shutdown each retain one separate allowance. Histories and their existing per-record bounds are unchanged. Snapshots can repeat tickets, can add later milestones, and can omit overwritten or contended metadata. These hooks do not establish a hardware stall cure.

MEASURED in the ENS12 CPU run 2026-10-28 at commit `5c98bbb197c9e0f6d1a17aeaa40c322831d7cd9d`: 2,627 tests passed, zero failed, 14 existing ignores across 82 targets; Clippy new=0 and claims new=0. `validation.json` records seals, expected-red commits and log digests. `mutations.json` records five mutants with at least two failing tests each. An initial run omitted test serialization and failed the existing process-wide RSS assertion; that log is retained separately, and the complete serial rerun passed without changing the test.

The owner checkpoint stopped work after CPU tests and the QEMU build. Raw evidence, the sealed binary, source checkout and preregistration are retained in the lane's private evidence locations, listed in its report. The adapted host harness is unvalidated and has never launched an ENS12 guest. Final-success ETL export was identified as remaining harness work; no readiness or host-pass claim is made. No PR or experiment merge was made.
