# Native Windows reference and seven-minute hold

**STATUS: LIVE, 2026-10-16. Native580 PASS; new guest tests pending.**

Own56180232, provider13058, RTX2060SUPER. Fresh exact die still pending
unprivileged RM after Linux restoration; PCI1f06 is not the derivation.
WindowsLTSC26100, actual32.0.15.8088, stock GSP firmware reportedN/A.
No Kayfabe binary/KF3 environment exists in this native reference.

| Check | Measured result |
|---|---|
| Actual adapter before/after both oracles |580.88, Code0|
| CUDA copy/kernel/matmul |3/3 before and3/3 after|
| Hardware D3D11/12 |Both before/after; WARP excluded by the established probe|
| Desktop hold |DWM+Explorer,421.03seconds,13 rounds of D3D11+12|
| System4101TDRs during hold |0, readable System log required|
| Whole replay |exit0 at02:15:00UTC|

Hold start02:07:40.5865646UTC, exit02:14:41.6324433UTC. Its minimum was
420seconds; original R34 budget remains180seconds. The native oracles check
all probe hashes, actual580.88 and post-run Code0. Each periodic D3D probe
creates its own device; the desktop state remains live throughout. This is
native acceptance, not a nested Windows or full die-family/multiple-build pass.

First hold attempt failed at DWM/Explorer precondition before timing, despite
passing GPU oracle. It remains saved in native-first/. A genuine console
autologon was configured with a fresh credential generated and retained only
on this rental, never printed/returned. After own native reboot, replay1's
gate requires healthy580 and DWM/Explorer before starting. Criteria unchanged.

WindowsUpdate transiently installed591.86 during setup; only its Code0 driver
query was observed, no591 suite. Pinned580 then installed. Update-service
stopping was not persistent. The full CUDA13 toolkit finished naturally
before the setup-stop check; state.cuda/complete True, completed own setup
task disabled. No claim that toolkit was omitted. Native probes import only
nvcuda.dll and system DLLs; no host GPU, owner VM or external credential used.

All replay text saved in native-replay1/ before queuing Linux-return
preparation. Only text returned, no executable or raw capture. Next verify
CodexLinuxReturn task exit0, own reboot, finalUbuntu,595.91.07stock open
driver and unprivileged RM tuple, then nestedWindows hold/R34/regressions.
