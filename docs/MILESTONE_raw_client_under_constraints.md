# ★★★★★ MILESTONE — the raw client passes UNDER THE CONSTRAINTS

**STATUS: ACHIEVED, 2026-09-11, n=2.** Tag: `w447-client-under-constraints` (`98bc7eca`).

> **Owner, setting the bar:** *"getting llm to pass isn't impressive anymore, we already have a
> dozen times incl mode 2 c. getting it to pass under these constraints is what I finally call
> as progress."*

The pass itself is not the milestone. **Passing while the three constraints hold is.**

## The measurement

Two boots, same binary, box `50585481` (RTX 3060, GA106, host driver 580.159.04 open both
sides), guest client `kayfabe-rm-ladder` static-musl, `MEAN_THREADS=8 MEAN_ROUNDS=8`:

| run | rev | P1 `rm-invalidate` | P2 `uvm-memop` | P3 `rpc-bind` | THREADS |
|---|---|---|---|---|---|
| `w443cli` | `a6aa3619` | ✔ VERIFIED ×8 | ✔ VERIFIED ×8 | ✔ VERIFIED ×8 | 8 of 8 ✔ |
| `w447cli` | `98bc7eca` | ✔ VERIFIED ×8 | ✔ VERIFIED ×8 | ✔ VERIFIED ×8 | 8 of 8 ✔ |

`W392D_OUTCOME=(P)` and `W392D_GUEST_OUTCOME=(P)` on both — content-verified, in the guest,
with the falsifier surviving. 28 overlapping pairs of graded intervals.

## The constraints, on the same boots

| constraint | before | w443cli | w447cli |
|---|---|---|---|
| BAR1/BAR2 trapped | 88 070 BAR1 traps | `arm=on` | `arm=on` |
| declared inline vCPU blocking | 166 | **0** | **0** |
| GSP RPCs serviced off the vCPU | 0 | 561 | 561 |
| publication lanes ever full | — | 0/0/0 | 0/0/0 |

## What got it there

- **`w431`** — both BAR passthrough defaults were `false` since the branch merged. The code was
  in master and **nothing armed it**; every ordinary boot trapped both BARs.
- **`w432`** — `NV_PGSP_QUEUE_HEAD` writes record and return; a worker services the RPCs and
  raises the interrupt. No inline-on-a-vCPU fallback, deliberately: the count carries the work
  and the job is only a wake.
- **`w441`** — `GPU_PROMOTE_CTX` BACKS the rows it binds. Synchronization point (2), the one
  with no barrier coming, because on a GSP part **we are the GSP**. Its own comment had said
  *"latch, do not publish … the drain's job"*, and that drain is the sweep that loses the race.

## ⊘⊘ WHAT THIS MILESTONE DOES **NOT** CLAIM

1. **Synchronization point (3) never fired.** `MEMOP-CENSUS seen=0`, `refresh passes: 0` on
   BOTH runs, while P2 `uvm-memop` verified. The consumption added in `w437` **never executed**;
   a green P2 is not evidence for it. Whether those channels emit anything we can observe is a
   SOURCE question, being audited — *instrumentation confirms, it does not reason*.
2. **The worst trap did not move.** `inline_exceptions` 166 → 0 while
   `worst_trap=1 779 094us at=bar0+0x110c00` stayed put, with `slow_traps(>1000us)=194`. `w448`
   attributes that to the drain holding the plane lock the vCPU also takes — *"no blocking calls
   in lock, especially the one shared with vcpus"* — and is **not yet measured**.
3. **The LLM.** Untouched since this landed.
4. **One machine, one build.** n=2 rules out a per-boot lottery; nothing more.
