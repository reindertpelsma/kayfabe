# §6 step 1 — the LIVE walk shadow

*(evidence pending — this file is completed from the boot, not before it)*

## What was built

`SINGLE_STORE_PLAN.md` §6 step 1's live half: the walk kernel runs **alongside** the host walk
at every off-vCPU page-table sweep, and the two answers are compared by kind. Nothing is
published from the kernel.

- `GmmuFmt::relocate_entry` (`kayfabe-arch`, implemented for VER2 in `kayfabe-chips::ga10x`)
- `kayfabe_mmu::walkshadow::build_image` — the compact, relocated image
- `Request::WalkShadowStage` (32) / `WalkShadowRun` (33), `cudawalk::{stage,run}`
- `SharedDevice::sweep_pt_tables_observing` — the hook, at EXECUTE, no lock held
- `crate::walkshadow::WalkShadowPort` / `WalkShadowObserver` — the shell half and the census

## How to reproduce

```
KAYFABE_SHIM_FEATURES="host-isolates cuda-scratchpad" bash scripts/build_qom_shim.sh
PREFIX=w731off SHADOW=off bash scripts/bench/single_store_e6_boot.sh   # the control
PREFIX=w731on  SHADOW=on  bash scripts/bench/single_store_e6_boot.sh   # the armed arm
```
or `bash scripts/bench/w731_live_shadow_run.sh` on the bench box, which does both.
