# w292 — NAMING THE CONTROL BEHIND `cuCtxCreate → 801`

**STATUS: LIVE, 2026-08-14.** Two boots at `31a6bc9`, real GA106 580.159.04, `vh`, same
instrument (`scripts/bench/nvdiff_hook.sh` → `nvdiff_shim.c` LD_PRELOAD recorder), one
variable: `KAYFABE_VAS_PUBLISH`. Relaxations carried and labelled: `KAYFABE_PT_SWEEP=on`,
`KAYFABE_OPERAND_JOIN=join`, `KAYFABE_FB_JOIN=shared`.

⊘ **cup2 was NOT run on these two boots** — the nvdiff hook runs `nvd_prog`, not `cup2`, so
there is **no `CUP2_RC`** here. The cup2 numbers stay the ones from the `drain` boot at
`bbd6ab6c` (`^CUP2_RC=1`). ★ `nvd_prog` reproduced **the identical `801`**, so the wall is
not a cup2 artefact.

Decode with `scripts/nvdiff_inband.py <capture>`; four states, never a bool
(`Ok / Refused(status) / NoStatusField / Truncated`).

## ★ KNOWN-POSITIVE, RUN FIRST

`scripts/nvdiff_inband.py traces/host_reference_ga106/ctx_r1.jsonl.zst` (the C repo's native
GA106 reference) → **exactly one** `Refused`: seq 86, `0x2080012f`, `0x56`, `rc=0 errno=0`.
That matches that capture's own `MANIFEST.txt`. ⇒ the reader **can** see a refusal that
`errno` cannot. Without it, "no divergence" is what a blind reader prints.

## THE RESULT

| | `both` (Xid=1, **719**) | `drain` (Xid=0, **801**) |
|---|---|---|
| records | 532 | **565** |
| last `RM_ALLOC` | i=367 | **i=389** |
| in-band refusals | 5 | 7 |
| `0x83de0309` | ⊘ **NEVER ISSUED** | **i=390, `0x56`** |
| `0x20801210` | ⊘ **NEVER ISSUED** | **i=391, `NV_OK`** |
| `0xa06c0103` | ⊘ **NEVER ISSUED** | i=407, `0x56` |
| `0xa06c0105` | i=385, `0x56` | i=418, `0x56` |

⇒ ★★★★★ **`719` AND `801` ARE NOT THE SAME CONTROL, AND NOT THE SAME SET.** The `both` arm
faults before it ever issues three of these. The `drain` arm runs **22 allocations further**
and reaches them. In particular `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` (`0x20801210`)
— the §16.59 wall — is **reached and answered `NV_OK` for the first time**.

**The last control that fails before the `RM_FREE` burst (first burst FREE at i=392; the only
earlier FREEs are i=4 and i=21) is `0x83de0309` at i=390.**

## THE FIVE NON-FORGIVEN REFUSALS, BY IDENTITY — and the native host SERVES EVERY ONE

| cmd | name (ogkm-580.159.04) | guest | **native GA106** | `psize` | C `cap3` (the GREEN run) |
|---|---|---|---|---|---|
| `0x20810108` | (0x2081 `NV2081_BINAPI` class) | `0x56` @41 | **`NV_OK`** @77 | 992 = 992 | ★ **SERVED `NV_OK`, `dlen=992` COMPLETE** |
| `0x2080012f` | `NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS` | `0x56` @50 | `0x56` @86 | 1464 | served as `0x56` — ✔ **AGREES**, the forgiven one |
| `0x2080200a` | `NV2080_CTRL_CMD_PERF_BOOST` | `0x56` @95 | **`NV_OK`** @130,478 | 8 = 8 | ⊘ never issued in cap3 |
| `0x83de0309` | `NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK` | `0x56` @390 | **`NV_OK`** @425 | 4 = 4 | ★ **SERVED `NV_OK`, `dlen=4` COMPLETE** |
| `0xa06c0103` | `NVA06C_CTRL_CMD_SET_TIMESLICE` | `0x56` @407 | **`NV_OK`** @427 | 8 = 8 | ★ **SERVED `NV_OK`, `dlen=8` COMPLETE** |
| `0xa06c0105` | `NVA06C_CTRL_CMD_PREEMPT` | `0x56` @418 | **`NV_OK`** @457 | 8 = 8 | ⊘ never issued in cap3 |

## ⊘⊘ HYPOTHESIS (B) — SIZE/PARAM MISMATCH — IS REFUTED FOR ALL SIX

For every row above: the guest's **declared `paramsSize` equals the native host's**, the shim
captured `pgot == psize` on **both** sides, and `trunc=0`. Nothing is short, nothing is
mis-shaped. **They are refusals, not malformed replies.**
⚠ **This does NOT retire (B) in general.** An in-band status reader is *structurally blind*
to a control we answer `NV_OK` with a **short or wrong-shaped body** — the exact defect that
zero-filled `numEntries` in `cuInit` (task #203). Only a **body diff** can see that, and this
capture carries `ppre`/`ppost` for every row, so it is the next instrument, not a new boot.

## ★★★ HYPOTHESIS (A) — A CONTROL THE C FORWARDED AND WE REFUSE — **CONFIRMED, THREE TIMES**

`cap3_matmul_forwarding` (532 824 records, `m2fwd=1 m2exec=1`, `n_errors=0`) serves
`0x20810108`, `0x83de0309` and `0xa06c0103` with `NV_OK` and a **COMPLETE** body
(`dlen >= psize` on every one) — so the oracle's own limit is **satisfied**, not stretched:
these are not `dlen=0` rows and not short rows. ⊘ `0x2080200a` and `0xa06c0105` never appear
in cap3 at all (`cup8`'s path differs from `nvd_prog`'s), so on those two the C is **silent,
not negative**.

## AND OUR OWN REFUSAL NAME, FROM THE BOOT'S OWN LOG

`run_w290pdrain_qemu.log` carries **44 distinct** `unserviced fn 76 cmd 0x…` ids, including
`0x20810108`, `0x83de0309`, `0xa06c0103`, `0xa06c0105`. That is the `UnservicedLedger` at the
**end of the `PolicyChain`** — the id is on the capability allowlist (`capability.rs`) but not
on `ObjectPolicy::OBJECT_CONTROLS`, which is the tree's own named defect class
**"ADMITTED and SERVED are different gates"** (`tests/tests/admitted_is_served.rs`).

⊘ **`0x2080200a` appears ZERO times in our QEMU log.** Our device **never saw it** — its
`0x56` is produced **inside the guest's own `nvidia.ko`**, not by us. It must not be counted
as one of our refusals.

⚠ `0x83de0309` is *also* listed in `capability.rs`'s `DENIED_CONTROLS` as a **deliberate**
denial (`DeniedBecause::SmDebuggerTrapping` — *"this port does not implement SM debugger
trapping at all, so permitting the controls was a promise it could not keep"*). Our log
records it reaching the **unserviced ledger**; which of the two paths emitted the `0x56` is
**not measured here**. Both are ours and both produce `NV_ERR_NOT_SUPPORTED`.

---

## ★★★★★ w292 STEP 2 — THE FOUR ARE SERVED. Boot `w290pdrain` @ `0221095`, real GA106

`serve_r1.jsonl.zst` is the capture. Same instrument, same arm, one variable: the four
controls are now answered by `ObjectPolicy::respond_input_only`.

### Pass criterion 1 — served, with bodies that match the reference

| cmd | before (`bbd6ab6`) | **after (`0221095`)** | native GA106 |
|---|---|---|---|
| `0x20810108` | `0x56` | ★ **`0x00`** | `0x00` |
| `0x83de0309` | `0x56` | ★ **`0x00`** | `0x00` |
| `0xa06c0103` | `0x56` | ★ **`0x00`** | `0x00` |
| `0xa06c0105` | `0x56` | ★ **`0x00`** | `0x00` |

**`unserviced fn 76 cmd 0x…` for all four = 0** in the boot's own QEMU log. The seam is
closed, measured on our side as well as the guest's.

### Pass criterion 2 — `^CUP2_RC=` ANCHORED = **1**

Baseline 1. Loose anchor `1`; unanchored `[CUP2_RC=0 CUP2_RC=1]` (the `0` is `GCC_CUP2_RC=0`).
⊘ **Does not cross — seventh necessary-not-sufficient.** `Xid = 0`, `host_rows = 18 295 of
18 309`, the drain still completes.

### Pass criterion 3 — is `nvd_prog`'s `801` gone? **NO — and it is now a DIFFERENT control**

572 records (was 565). The refusal set changed by identity:

| | before | after |
|---|---|---|
| `0x83de0309` | `0x56` **← the wall** | served |
| `0xa06c0103` / `0xa06c0105` / `0x20810108` | `0x56` | served |
| **`0x20801210`** `GR_SET_CTXSW_PREEMPTION_MODE` | `NV_OK` | ⚠ **`0x56` @391 — THE NEW WALL** |
| **`0x00801909`** `NV0080_CTRL_CMD_PERF_CUDA_LIMIT_SET_CONTROL` | ⊘ never issued | ⚠ **`0x56` @412** (`psize=1`; native serves `NV_OK` twice) |

### ★★★★★ AND THE NEW WALL IS OUR OWN §16.59 CLASSIFIER — WITH ITS PREMISE OVERTURNED

`NV2080_CTRL_GR_SET_CTXSW_PREEMPTION_MODE_PARAMS` is
`flags@0, hChannel@4, gfxpPreemptMode@8, cilpPreemptMode@12`
(`ogkm-580: ctrl2080gr.h:822-828`), and `COMPUTE_CILP = 2` (`:846`).

| | `flags` | `hChannel` | `gfxp` | `cilpPreemptMode` |
|---|---|---|---|---|
| ours, before | `1` | `0x5c000012` | `0` | **`0` WFI** |
| ours, after | `1` | `0x5c000012` | `0` | ★ **`2` CILP** |
| **native GA106** | `1` | `0x5c000016` | `0` | **`2` CILP** |

⇒ **Every word now matches a real GA106 except the channel handle.** §16.59 records
*"`2` (`COMPUTE_CILP`) in the C against `0` (`COMPUTE_WFI`) in ours"* as a fact about our
payload; `[measured]` it was a fact about **our refusals**, one control upstream. The guest
asked for WFI *because we refused the exception mask*, whose `0x3a` includes `_CILP` (`0x10`).
⊘ `0x20801210` returning `0x56` is therefore **not a regression** — the same classifier met a
different request, and it now refuses `PreemptionNotImplemented` honestly. Whether to answer
honestly and block, or promise preemption we do not have, is an **owner** question.

⊘ `0x2080200a` still `0x56` @95 — unchanged and still not ours. `0x2080012f` still `0x56` @50 —
unchanged and still AGREEING with hardware.
