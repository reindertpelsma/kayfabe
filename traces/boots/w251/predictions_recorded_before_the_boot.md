# w251 — PREDICTIONS, recorded BEFORE the boot

## The hypothesis, from source

The two unaccounted host-side engine-object failures come from **`HostRmBackend::ce_channel`**
(`kayfabe-isolate-host/src/rm.rs:4298`), which allocates the **isolate's own CE engine object**
via `alloc_ce_engine_object` — *"the one call site in the tree that allocates an engine object
from a `HostClasses` rather than from guest intent"* (its own doc, `:3472-3482`).

Chain: `SharedDevice::forward_ce` → `verb_op` → `Worker::execute` → `VerbPlan::CeSplit` →
`RmBackend::ce_copy` → `ce_copy_outcome` (`:4218`) → `ce_channel` (`:4298`) →
`alloc_ce_engine_object`.

⇒ ⊘ **The funnel census (§16.92) is NOT stale, and the path IS under `Worker::execute`.** It is a
*different verb* — `forward_ce`, one of the eight the census already enumerated — whose **side
effect two layers down, inside the isolate, is an engine-object allocation the core never asked
for.** Our `ENGINE-OBJECT` census counts what the **core** requested; it cannot see one the
**isolate** makes on its own behalf while serving a copy.

## The discriminator

`forward_ce` runs only when `KAYFABE_CE_EXECUTOR=host`. With `local`, the shell's CPU executor
serves every copy and `forward_ce` is never called ⇒ `ce_channel` is never reached.

**Boot `w251_acbb9a3_cel_hostdmesg`: identical to `w250` except `KAYFABE_CE_EXECUTOR=local`.**

## Predictions

1. Our `ENGINE-OBJECT … REFUSED Rm(Other(64))` count is **unchanged at 12** — `forward_engine_object`
   does not depend on the CE executor (w244a/w244b measured `seen=32 forwarded=18` on both arms).
2. ★★★ **The host's `chandesConstruct_IMPL` failure count DROPS from 14 to 12**, and
   `kfifoRunlistSetId_GM107` with it. ⇒ the 2 extras are `ce_channel`'s.
3. The engine split changes: the **2 fewer** come out of one of the `CE2`(×8)/`CE3`(×6) groups.
4. `RING-PROJ` is **0** (the `local` fall-through is dead code — measured at `acbb9a3`), so this
   boot says nothing about route B and is not offered as if it did.

⇒ If (2) fails — still 14 — the hypothesis is wrong, the second path is not CE-executor-gated,
and that is the finding. ⊘ `CE-SUBMIT` will be 0 either way; nothing here executes guest work.

---

# SCORING (added after the run — the predictions above are unedited)

| # | prediction | outcome |
|---|---|---|
| 1 | ours unchanged at 12 | ✅ 12 |
| 2 | ★★★ **host drops 14 → 12** | ⊘⊘ **FALSIFIED — still 14** |
| 3 | the engine split loses 2 | ⊘ N/A — split byte-identical (8×CE2, 6×CE3) |
| 4 | `RING-PROJ` is 0 | ✅ 0 (and 8 on the `host` arm — the arms genuinely differed) |

⇒ **The second path is NOT the CE executor.** Host-side failures are byte-identical across both
executor arms. The hypothesis is refuted by its own discriminator, and the search narrows: the
host's 14 land on **two** host channels while our 12 name **twelve** guest parents, so the gap is
**2 attempts, not 2 objects**. Leading (unmeasured) hypothesis: `verb_op`'s retry loop re-issues
the host alloc while the core reports one outcome.
