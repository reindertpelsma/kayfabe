# w256 — PREDICTIONS, recorded BEFORE the boot

**Committed before the boot. Scored unedited below.**

Change under test: the engine-object census's **row** budget goes `32 → 256` per outcome class,
and past that budget the **three totals** keep being printed on a doubling schedule
(`engine_fwd_report_action`). ⊘ The measuring boot for §16.106 (`w255`) is already committed and
cannot be contaminated by this.

## ★★★ THE ANSWER IS ALREADY BOUNDED BY `w254`, AND THAT IS THE ARGUMENT

`w255` printed `forwarded=32` **exactly on the bound**, so 32 is a lower bound. But its control
is not saturated:

| | `w254` | `w255` |
|---|---|---|
| `FORWARDED` | 18 | 32 ← **at the bound** |
| `REFUSED Rm(Other(64))` | 14 | 0 |
| `REFUSED NoVas(..)` | 2 | 2 |
| last line's `seen` | 34 | 34 |

In **`w254` both classes were under their budgets** (18 < 32 forwards, 16 < 32 refusals), so every
outcome printed and `seen=34` is a **true total**: `18 + 14 + 2 = 34`. And `w255`'s guest `dmesg`
is **byte-identical** to `w254`'s, i.e. the guest ran the same program to the same place.

⇒ if the workload is deterministic, `w255` had the same 34 outcomes with 14 refusals turned into
forwards: **32 forwards + 2 `NoVas` = 34**. The forward class hit its bound by coincidence — it
saturated at exactly the number it would have printed anyway.

## Predictions

1. ★★★ **`FORWARDED` = 32 exactly.** Not 33+, not fewer.
2. ★★★ **No `REPORT BOUND REACHED` marker anywhere in the log**, and **no `ENGINE-OBJECT CENSUS`
   totals line** — both classes now sit far below 256.
3. **Total `ENGINE-OBJECT` lines = 34**, last line `[seen=34 forwarded=32 refused=2]`.
4. `REFUSED` = 2, both `NoVas`, both `host_chan=NONE`. Host `chandesConstruct_IMPL` = **0**.
5. `CE-SUBMIT` = **0**, doorbells `191 / 183 / 8`, guest `dmesg` byte-identical to `w254`/`w255`.
   ⊘ No execution-plane rung is claimed; this rung is an instrument change only.

⊘ **If (1) is wrong — if `forwarded` > 32 — then the instrument was hiding outcomes in `w255`
after all**, the w254↔w255 comparison had a difference nobody saw, and **every number in §16.106.5
that rests on "32" needs re-checking.** That is the whole reason to spend a boot on this.

⊘ If (1) is right, the honest statement is that **`forwarded=32` was correct and unverifiable**,
which is a different thing from being right, and is exactly why the qualification was recorded
rather than assumed away.

## Configuration (stated)

`KAYFABE_ISOLATES=real`, `KAYFABE_CE_EXECUTOR=local`, `KAYFABE_PT_WITNESS_EXEC=on`,
`KAYFABE_RING_VIDMEM=on`, `KAYFABE_FB_BACKING=on`, `NVKVM_RAM_BACKEND=memfd`,
`POST_CAPTURE_HOOK=scripts/bench/cup2_hook_w232.sh` — identical to `w254` and `w255`.

---

# SCORING (added after the run — the predictions above are unedited)

<!-- filled in after the boot -->
