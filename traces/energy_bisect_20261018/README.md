# Q2 Energy bisect — resumed

MEASURED: at base rev c80522d70, the required claims gate returned existing-debt=3756 new=74 resolved=2 (claims-gate.json). All flagged paths are inherited; the Q2 evidence directory is not flagged. Q2 initially stopped. The owner subsequently clarified that this evidence-only branch uses a claims check scoped to Q2-added/changed files and pushes with [skip ci]; inherited violations do not block this lane. No debt ledger changed.

MEASURED at rev afe00b3c / A rev 40583bc2: Q1 candidate run 4262 and A run 4263 both have one NoCeObject refusal, three guest event-153 records, zero new host Xids, and successful restoration (4262/result.json; 4263/result.json). Candidate run 4260 is the prior mixed-workload sample retained in ../qualify_cand6_20261022/. Thus candidate already has two failure samples; only 4262 ran Energy alone.

MEASURED at rev 40583bc2: A repeat run 4265 used a fresh CoW, the same FullHD/one-iteration/energy-04 task, and matching flags (4265/config.json). It was explicitly interrupted at the claims stop, before any observed event153; it is INCONCLUSIVE, not a second Energy failure or pass. Raw supervisor status FAIL/HOST_CONTROLLER_STOP is preserved with this interpretation in 4265/result.json. Owner backing size/mtime stayed unchanged.

MEASURED: tests-B.json records no executed tests and exit -15: Q2 canceled its own still-waiting cargo-lock process after the claims refusal. B hardware was not launched. No product behavior changed and no new mutation campaign ran.

CODE READ: inherited ../energy_bisect_20261022/source-analysis.md describes the refusing path; Q2 inspected kernel_channel, ChannelPolicy allocation facts, TranslatedRing::new_tmode and tmode::one/host. The failing KERNEL stamp remains kernel with both predicates. An unseeded decoder CE tier reaches NoCeObject at subchannel4/method0x240; this does not establish absence of a physical host CE object.

INFERRED: no responsible-topic verdict or product fix is justified without B and a completed A repeat. An exact full-RPC replay is unavailable. The owner authorized resumption; B, a completed A repeat and a second matched Energy-only candidate sample are now planned.

MEASURED at rev 40583bc2: cleanup-final.json for run 4265 records cuInit0, GDM active, NVIDIA bound, no own active host process, and successful GPU-lock reacquisition. The prior restoration needed no module reload.
