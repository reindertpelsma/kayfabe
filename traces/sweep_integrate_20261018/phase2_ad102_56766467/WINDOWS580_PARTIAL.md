# AD102 Windows580 full cell result

STATUS: LIVE, 2026-10-19. MEASURED full requested Windows580cell PASS at source24906bf33.
This supersedes the partial hold checkpoint below.420.9seconds,11hardwareD3D
rounds,zeroSystem4101TDRs; Code0/DWM/Explorer and nonblankscreen retained.
Final WIN_CHECKfail0. windows580_result.json and fulltext/PNGs give proof.
The initial sign-in verification failure is retained honestly; same coldboot
was independently verified and then completed the suite. No full family claim.

SUPERSEDED PARTIAL CHECKPOINT:

# AD102 Windows580 short suite

STATUS: LIVE, 2026-10-19. MEASURED short suite PASS; full cell PARTIAL while420shold runs.
Frozen source24906bf33, own56766467, open host595.91.07, actualguest32.0.15.8088,
wire580.65.06, all19P19flags asserted in actualprocess136024, VGA none/GOP on.

Code0/DWM/Explorer and nonblank1920x1080desktop verified. CUDA cup2 copied
abcd1234; cup3 returned43; cup8 N2048,bad0,maxerr0. Hardware NVIDIA D3D11/12
both RESULT OK; Basic Render/WARP excluded. Since-boot System4101count0.
The420shold is still active; no full cell PASS until its duration/TDRchecks finish.

The original cold validator (2026-10-19) stopped at SIGNIN_FAILED before probes. SSH then
observed real KFWIN\kf, noLogonUI, Explorer1,DWM1 andCode0; screenshot showed
its desktop. Same logical readiness guard was independently satisfied over
SSH and recorded before resuming the still-unrun probes in the SAMEcoldboot.
No password/input retry, new boot, installer replay or timeout change. The exact
reason the agent-verification helper reportedfailure remains UNRESOLVED; do not
report it as a measured GPU failure or silently replace its original failure log.
