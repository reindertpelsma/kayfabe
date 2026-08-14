# `traces/w327_cliff/` — the boots behind `docs/design/w327_the_allocation_cliff.md`

**All on `vh2`, real GA106 (RTX 3060), driver `580.159.04`, stock guest driver, 2026-08-14.**
Base pinned at **`df3043be`** (master = merge of w322). The sweep arms up to and including
`w327u*`/`w327r`/`w327s` ran at `869a45f5`; `w327x*` and `w327f*` ran at `6fb7575a`, which adds
the **print-only** `PUBCONFLICT_VAS` diagnostic and changes nothing else.

## What each boot asked

| tag | `KAYFABE_BENCH_BW` | asks |
|---|---|---|
| `w327a` | `16,17,18,20,22,24,28,31,32` | ★ the denser grid — is w322's "32 MiB" a constant or an artefact of a power-of-two grid? |
| `w327b1` | `31` | is 31 MiB fatal **on its own**? |
| `w327b2` | `29,30,31` | where exactly between 28 and 31? |
| `w327b3` | `64` | is 64 MiB fatal on its own? |
| `w327c1` | `16,24,28,29,30,31,32,40,64` + `KAYFABE_DRAIN_BATCH=coalesce` | ★ does w321's coalescing raise it? |
| `w327n` | `4,16` + `BENCH_NOLAUNCH=1` | ★ THE KNOWN-POSITIVE |
| `w327u1` | `31,31` | same size twice — is the axis the cycle? |
| `w327u2` | `28,31` | ★ the two-row minimal repro |
| `w327u3` | `4,31` | small predecessor, VA still moves |
| `w327u4` | `4,64` | small predecessor, VA **must** move |
| `w327r` | `4` ×16 | is the axis allocate/free CYCLES? |
| `w327s` | `16` ×8 | is the axis CUMULATIVE BYTES? |
| `w327x1b` | `28,64` | ★★★★★ the single-variable pair against `w327u4` |
| `w327x2` | `16,31` | w322's own predecessor size, in two rows |
| `w327f1..3` | `28,31` | n=3 on the failure, with `PUBCONFLICT_VAS` armed |
| `w327big` | cup8 `N=3072` | ★ bit-exact **above** w322's claimed 32 MiB ceiling |
| `w327v3*`, `w327v8*`, `w327vr*` | — | the three-workload grading ladder, n=3 |

## ⊘ VOID artefacts, kept named rather than deleted

- `w327b_VOID_dirty_tree.log` — five arms in 43 s, all `rc=91`. The first build of a fresh
  clone rewrote `Cargo.lock`, and `w290p_run.sh:50` refuses a dirty tree. **The batch wrote its
  terminator and exited 0 having measured nothing.**
- `w327f_VOID_concurrent.log` and `/workspace/bench/run_w327x1_*` — **two detached batches ran
  at once for ~90 s and both used the tag `w327x1`**; one's `pkill -x qemu-system-x86` killed
  the other's guest mid-boot. Re-run as `w327x1b`. ⊘ Not salvaged: *"which boot wrote this
  line"* has no answer.

## How to reproduce the failure in two rows

```
KAYFABE_REPO=/workspace/kayfabe_w327 \
  bash scripts/bench/w327_sweep.sh <tag> 28,31
```
Expect `W327_LAST_OK_MIB=28`, `W327_FIRST_FAIL_MIB=31`,
`BW_FILL_FAIL mib=31 at_element=2097152 … rc=0/719`, **zero Xid on both sides**.
