# `traces/w754_gsp_queue_off_vcpu/` — what is in here and what each file can answer

`[vast 51217315, GA106, 580.159.04 OPEN, 2026-09-16]` Two three-arm runs plus one diagnostic
boot. **Every claim in `docs/design/w754_gsp_queue_off_vcpu.md` §MEASURED is cut from these.**

| file | revision | what it is |
|---|---|---|
| `w754_run1.log` + `w754_run1_evidence.tgz` | `8286ee65` | cuts **A+B**. Arms `arena` / `dev` / `inline`. The A/B that attributes `bar0+0x110c00` to the ring-adopt settlement, and the run that found the 46 ms at `bar0+0x110118`. |
| `w754_run2.log` + `w754_run2_evidence.tgz` | `018c41d4` | cuts **A+B+C**. The graded run. Control goes **sub-millisecond** (`920 µs`, `slow_traps=0`); device `9 649 µs`. |
| `w754_stall.log`, `w754_stall_backtrace.txt` | `8286ee65` | the diagnostic boot: `KAYFABE_STALL_ALARM_US=15000 KAYFABE_STALL_ALARM_AT=110118`. **The backtrace is the whole evidence for cut C** and names `RegPlane::gsp_register_offsets::{closure#0}` between `publish_gsp_registers` and `kvm_vcpu_thread_fn`. |

## ⚠ How to read these without repeating this campaign's mistakes

- **`worst_trap … at=<site>` names WHERE THE CLOCK STOPPED, not what spent the time.** For four
  rungs `at=bar0+0x110c00` was read as *"the queue-head write services the GSP queue inline"*.
  It has not done that since w432. Read `cpu_of_that_trap` beside it: 96–100 % CPU and 48 % CPU
  are different defects with opposite fixes.
- **The third arm (`inline`) is not padding.** It is the same binary with
  `KAYFABE_MATERIALIZE_INLINE=1`, which forces the settlement back onto the vCPU. Without it,
  `RING-ADOPT on_vcpu=0` on the device arm is a counter nobody has shown can move.
- **`SLOW-SITES` is the distribution; `worst_trap` is one event.** Both are in every arm's
  report, and the interesting movements this rung are in the distribution: four `0xb81…` sites
  leaving it entirely, and the 10–100 ms decade emptying.
- ⊘ **The raw client is `(R)` on the device arm in both runs, exactly as at HEAD.** Another
  lane owns that. It is `(R)` on the known-positive arm too, so it cannot hide a difference
  between the two arms whose comparison is this rung's claim.
- ⊘ **`run_w754c*` is run 2** (tag `w754c`); `run_w754*` without the `c` is run 1.
