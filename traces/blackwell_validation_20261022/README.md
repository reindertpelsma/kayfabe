**STATUS: LIVE, 2026-10-22.** One owner-authorized GB202/RTX5090 lease57132969.
N0 PASS, Windows arms UNRUN at a mandatory post-bind safety stop. Lease destroyed and API-absent
at01:30:18UTC after no required decision arrived before the stated idle cutoff. No guard relaxation,
rebind, first enable or validation arm performed while pending. The old resume request can no longer
apply to this destroyed lease; a later rental would require new explicit authority.

| Cell | Frozen source | Measured outcome |
|---|---|---|
| N0 native open595.91.07 | `8f3a9bb3a5d652b3f536ca015bf95735ec588924` | PASS n=1: gates9/9+10, raw30/30, CUDA4/4, corrected GR/CE PASS; only5 source/PID/device-matched intentional31, no native32 |
| LTSC install +580.88 disabled bind | same | Install/base seal PASS; CIM32.0.15.8088, Code22, cold-ready/GSP readback1; post-shutdown QEMU census exited1 before bound-parent seal |
| CAPC ON/display+GOP | same | UNRUN; no Code0/43/TDR result |
| OFF/TDR, display=off,gop=off,gop-efi= | same | UNRUN |
| Optional merged B1 ON | `2add71e8ca8ebb80c419ed2fdb0427dfcdc0ccd4` | Separate binary prepared; hardware UNRUN |

The B1 merge's parents are primary8f3a9bb3a and tested B1
`1c6452a276d7014bece3adf42d68cbd4edffbc18` (runtime source d8c1e65ef).
It is pushed only on `codex/bw-validation-b1-arm-20261022`; both source branches remain untouched.
The CI-repair parent broadens the comparison, so a future difference is not RC-only causality proof.
Exact source/binary hashes are in `lease_metadata/*prepared.json`; no binaries were copied back.
The prepared standalone d8 binary is superseded for the optional arm by this actual merge build.
CAPC remains isolated in draft PR69; no captured PRECOMP words or diagnostic caps-page injection.

The failed check followed `win_vm.sh stop` logging `stopped` at01:08:38UTC. Later read-only audits
found zero QEMU processes and the same5 native controls, no XID_STOP. An exit-window race is an
INFERENCE: the process census at the instant of failure was not retained. The required zero-QEMU
check was not rerun to bypass the stop, and the disabled bound parent was not sealed afterward.
COMMON.md requires stopping/reporting after a safety refusal. `bind_stop.json` records the hold.

Provisioning markers, all UTC: lease00:26:37, SSH00:28:16, media00:31:06-00:34:20,
product build ready00:39:24, N0 complete~00:40:30, install00:40:45-01:01:31,
bind01:01:31-01:08:38. Media fetch overlapped CPU preparation. Install/check/seal20m46s;
bind7m07s; creation to stopped bind~42m01s. This is not a sealed-parent-ready measurement,
because the final census failed. It improves attribution over the old65min allowance; installation
is the largest measured stage. No persistent cache across destroyed leases is assumed.

A future authorized lease must begin with clean N0. Preserve all approved guards and the frozen
19flags. The merged B1 binary is the first queued optional arm after the required CAPC ON/OFF pair
unless the owner changes that order. Its question is whether the exact native32 produces
`RC Translated ... Xid32` and guest RC_TRIGGERED, and how the guest handles it; RESET_CHANNEL
is unchanged. Record private host ring identity and guest channel separately; do not infer them
from debug words. Code0 alone never means PASS; the hold/apps/desktop acceptance gate remains.

Teardown API absence is recorded in `owned_instances.json` and the shared COST.md.
New runtime estimate0.824240USD, lane total3.464692USD;
shared credit42.956391USD, reserve5 intact. Timer cancelled only after absence.
All remote QEMU/CPU/guard processes ended with the destroyed lease. Local merged release tests
are still running at this documentation checkpoint; their final counts/cleanup will be appended.
The trusted host, GPU, VMs, locks and /data were untouched.
