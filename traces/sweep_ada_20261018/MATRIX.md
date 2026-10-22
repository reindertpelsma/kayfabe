STATUS: LIVE, 2026-10-22. S2 AD102 substitution explicitly approved by the owner.

MEASURED: own lease57138158; RTX4090 unprivileged probe confirms AD102,
arch0x190/impl0x2, UID65534/CapEff0. Source35ff2820e5573199072ab2d274f5f0b91a1826a8.

| Die / board | Cell | Result | Evidence |
|---|---|---|---|
| AD102 / RTX4090 | Stock Linux595.91.07 open | MEASURED SOUND; gates9/9+gate10PASS;12/12 USER births; native raw/CUDA/GR pending | lease_57138158/native_preflight/ |
| AD102 / RTX4090 | Linux guest595 | MEASURED UNRUN; build in progress | lease_57138158/RUN_PLAN.md |
| AD102 / RTX4090 | WindowsLTSC/actual580.88 ON | MEASURED UNRUN | lease_57138158/RUN_PLAN.md |
| AD102 / RTX4090 | Windows580.88 observer ON | MEASURED UNRUN; public observer source applied before initial build | lease_57138158/native_preflight/observer_source_apply.log |

MEASURED: guard software97/0/0 both dev box and rental Python;22mutants>=2killers
on dev box. Hardware sample n=1 so far; no spread/performance/Windows PASS claimed.
