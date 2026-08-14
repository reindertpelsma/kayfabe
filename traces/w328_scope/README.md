# `traces/w328_scope/` — w328's per-boot artefacts

Branch `w328-scope-the-publication`, base master `308799cd`. Bench `vh`, real GA106, host
driver open `580.159.04`. Write-up: `docs/design/the_breadth_is_not_the_cost.md`.

## What is here

| path | what it is |
|---|---|
| `w328_all.log`, `w328_all2.log` | the two launchers, with the **pre-registration in their own headers**, written before any boot ran |
| `w328_arm_<prefix>.log` | one file per arm; every boot's graded lines |
| `census/<tag>_census.txt` | ★ the distilled publication plane per boot — `scripts/bench/w328_census.sh` |
| `boots/run_<tag>_{probe,dmesg,hostdmesg}.log` | the workload's own output, the guest's `dmesg`, the host's `dmesg` |

⊘ **The `run_<tag>_qemu.log` files are NOT here.** They are ~3.6 MB each × 24 boots. The
`census/` files are the distillation and carry every number this rung's claims rest on; each
one names its source log and its byte count so the omission is auditable rather than silent.

## The arms

| arm | `KAYFABE_PUBLISH_SCOPE` | `KAYFABE_DRAIN_BATCH` | `KAYFABE_DIRTY_GATE_PUBLISH` | workload | n |
|---|---|---|---|---|---|
| `w328a` | ⊘ unset (`all`) | ⊘ unset (`off`) | ⊘ unset (`off`) | cup3 | 3 |
| `w328s` | `doorbelled` | ⊘ unset | ⊘ unset | cup3 | 3 |
| `w328c` | `doorbelled` | `coalesce` | ⊘ unset | cup3 | 3 |
| `w328e` | `doorbelled` | `coalesce` | ⊘ unset | cup8 | 3 |
| `w328x` | `doorbelled` | `coalesce` | ⊘ unset | R33 arm 1 | 3 |
| `w328g` | `doorbelled` | `coalesce` | **`on`** | cup3 | 3 |
| `w328ge` | `doorbelled` | `coalesce` | **`on`** | cup8 | 3 |
| `w328gx` | `doorbelled` | `coalesce` | **`on`** | R33 arm 1 | 3 |

★ **One lever per arm**: `a`(none) → `s`(+scope) → `c`(+coalesce) → `g`(+dirty gate).
⊘ Arm `w328a` is master's behaviour **on this binary**, so the control and the evidence differ
in exactly one environment word each step.

## ⚠ How to read a boot — grade on STATE, never on `CUP3_VAL`

`the_drain_budget_truncation.md` §6: the binary outcome is a ~20 %-probability *consequence*;
the drain's completeness is a per-boot **deterministic** observable of the same event.

- `complete=true` **and** `pinned == asked` ⇒ the pre-existing intermittent did not fire.
- `complete=false` ⇒ truncated. Compare `last_pinned_va` against the `Xid`'s VA.
- ⊘ `budget_hit` alone is **not** the discriminator (`w314br4` hit it and was green).
- ★ `scripts/bench/w319_attribute.sh <tag>` prints the verdict; **`--selftest` first**, and it
  is run on every boot here, green ones included.

## ⊘ Two things these logs do NOT contain, said rather than discovered

- `host_rows` prints `⊘UNMEASURED` on every boot: it is emitted from **inside** the PT-sweep
  line and `KAYFABE_PT_SWEEP` is off. **The publication never stopped; its only reporter did**
  — w298's finding, and the reason `regression_check_e.sh` no longer grades it.
- `KFTIME-SEG vas_publish` is absent: the segment census is not enabled on these boots.
  `W328SCOPE`'s `target_us`/`other_us` are the substitute and are bracketed at the source.
