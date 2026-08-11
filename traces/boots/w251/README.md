# w251 — the second path is NOT the CE executor. Hypothesis refuted by its own discriminator.

⊘ `CE-SUBMIT` **0**; nothing executed. Revision `acbb9a3`. Control: `w250` (identical but
`KAYFABE_CE_EXECUTOR=host`).

| | `w250` (`ce=host`) | `w251` (`ce=local`) |
|---|---|---|
| ours `REFUSED Rm(Other(64))` | 12 | **12** |
| host `chandesConstruct_IMPL` | 14 | **14** |
| host `kfifoRunlistSetId_GM107` | 14 | **14** |
| engines named | 8×`CE2`, 6×`CE3` | **8×`CE2`, 6×`CE3`** |
| channels named | 6×`0x04`, 8×`0x0c` | **6×`0x04`, 8×`0x0c`** |
| `RING-PROJ` | 8 | **0** |

**Host-side failures are byte-identical across both executors**, while `RING-PROJ` 8→0 proves the
arms genuinely differed. ⇒ the second path is **invariant to the CE executor**, so it is not
`HostRmBackend::ce_channel` reached through `forward_ce`.

## ⊘ The brief's item 2: neither branch

`ce_channel` **is** under `Worker::execute` — `forward_ce → verb_op → VerbPlan::CeSplit → ce_copy
→ ce_copy_outcome → ce_channel` — and `forward_ce` was already one of §16.92's eight. **The funnel
census is not stale.** What nobody had noticed is that a *different verb*'s **side effect two
layers down, inside the isolate, is an engine-object allocation the core never asked for**.
★★ `ENGINE-OBJECT` is a census of the **core's intent**, not of engine objects allocated.

## ★★★ What the numbers actually say

The host's 14 failures land on **two** host channels; our 12 refusals name **twelve** guest
parents. ⇒ **a 1:1 refusal↔object model was never right.** The gap is **2 attempts, not 2
objects**.

★ Leading hypothesis, labelled: `verb_op` **retries** (converging staleness, `IsolatePending`,
bounded by `MAX_COMMIT_RETRIES`) and each retry **re-issues the host alloc** while the core reports
one outcome — 14 attempts, 12 outcomes ⇒ 2 retries. ⊘ `[HYPOTHESIS — NOT MEASURED]`; the next
discriminator counts retries directly instead of inferring them from a difference.
⇒ ★★ If it holds: **our census counts OUTCOMES, the driver counts ATTEMPTS** — and a per-refusal
join can never close across a retry loop.

⊘ `w251` ran `ce_executor=local`, where `RING-PROJ` is 0 by construction: **it says nothing about
route B** and is not offered as if it did.

Full record: `docs/design/execution_plane_increments.md` §16.102.
