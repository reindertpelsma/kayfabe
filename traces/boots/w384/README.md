# w384 — the run logs, committed. The rented boxes are not the artefact.

Every number in `docs/design/w384_the_doorbell_latency_rung.md` §4 comes from these files. Both
boxes were rented and `kb2` was destroyed at the end of the lane, so the logs are the only thing
that survives — ★ *"vast is compute, never storage."*

## `w384_kb.tgz` — box `kb`, vast **50013922**, RTX 3060 GA106, 23 cores

| tag | what it is |
|---|---|
| `run_w384_*` | first differential. ⊘ Its guest arm returned early on the failed closing control, so `arm=bare`/`arm=freshmap` never ran and the drain was uninstrumented. **Superseded by `w384b`; kept because §4.2's "how this was nearly missed" is about it.** |
| `run_w384b_*` | the differential with the instrumented drain. `first_stall_at=63`, 3/3. ⚠ Its hook ran **six** device-opening processes and the 5th and 6th printed nothing — the device-open wedge, §4.2. |
| `run_w384c_*` | the `n=63` / `n=64` bracket, and `missing_page` inside the budget (4 opens). |

## `w384_kb2.tgz` — box `kb2`, vast **50080571**, RTX 3060 GA106, 19 cores (DESTROYED)

| tag | what it is |
|---|---|
| `run_w384pre2_*` | shim built from **`30eb4627`** — master **before** `w383-doorbell-async` merged. |
| `run_w384post_*` | shim built from **`758a5752`** — **after**. Same box, twenty minutes later, native re-calibrated. §4.4's before/after is these two. |
| `run_w384e_*` | the separating experiment: `KAYFABE_LADDER_GPFIFO_ENTRIES=32`, `n=24/32/48`. §4.2's answer is this file. |

⊘ `run_w384pre_*` does not exist: that launch printed `W384_OUTCOME=(E) ⊘ UNMEASURED_NO_BINARY`
because the pre-fix target dir had never been built. ★ The pre-registered vocabulary did its job —
"the build never happened" and "the rung failed" did not share a verdict.
