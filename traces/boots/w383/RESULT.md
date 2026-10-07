# w383 — the doorbell lane, and the dirty gate that was switched off

Seven boots, real GA106 (`580.159.04`), 2026-09-06. Full argument:
`docs/design/w383_the_doorbell_is_a_schedule.md`. `w383_logs.tgz` holds every probe log,
host-dmesg delta and this summary.

## ★★★★★ THE MILESTONE — `w383llmgate`

```
LLM_OK=1  LLM_TOKENS=16  LLM_MS=722820.0
LLM_TEXT= ______. A. Paris B. London C. New York D
MINMM_OK=1  MINMM_SUM=64        HOST_DMESG_XID=0   W382_XIDS=9/9/9
```

## Every boot, one line each

| tag | doorbell arm | dirty gate | result | `DIRTY-GATE` | publication wall | host Xid |
|---|---|---|---|---|---|---|
| `w383cup3ctl` | off | off | `^CUP3_VAL=43` | `skipped=0` (0.0 %) | 1 488 ms / 229 | 0 |
| `w383cup3` | **on** (coalescing) | off | ⊘ `CUP3_VAL` **ABSENT**, `cuCtxCreate → 999` | `skipped=0` | 4 230 ms / 53 | **2** |
| `w383cup3nc` | **nocoalesce** | off | `^CUP3_VAL=43` | `skipped=0` | 1 756 ms / 229 | 0 |
| `w383cup3gate` | off | **on** | `^CUP3_VAL=43` | **95.7 % skipped** | **380 ms** / 229 | 0 |
| `w383llmctl` | off | off | `LLM_TOKENS` **ABSENT** @1500 s | `skipped=0` | **508 904 ms** / 16 890 | 0 |
| `w383llmnc` | **nocoalesce** | off | ⊘ minimal matmul faulted | `skipped=0` | 2 910 ms / 358 | **4** |
| `w383llmgate` | off | **on** | ★★★★★ **`LLM_TOKENS=16`** | **99.7 % skipped** | **6 576 ms** / 22 654 | **0** |

`SUPERSEDED=0` and `⊘ SUPERSEDE CAPPED=0` on **all seven**.

## The three things these boots settled

1. ⊘ **Coalescing is refuted.** `cup3` `off`/`on`/`nocoalesce` is a three-boot discriminator
   with one variable: deferring is fine, **folding two doorbells into one act is not**.
   `pubqueue` §2's premise is verified of `ceutils::run_submission` and was only *asserted* of
   the forwarding path.
2. ⊘⊘ **Deferring is not yet safe for the multi-process LLM**, and the control proves it is
   the deferral. The mechanism is that the host channel is born over the **guest's own USERD**
   (`adopt=GUEST-RING` ×17, `userd=GUEST-USERD` ×17 in one `cup3` boot), so from the second
   submission the guest's `GP_PUT` starts the engine and our doorbell is not in the path.
3. ★★★★★ **The latency was the dirty gate, not the thread.** w330's `off → on` default was
   written into `dirty_gate_from`'s `None` arm and **that arm had no caller**; the gate had
   never been consulted, and `skipped=0` reads identically for *"disarmed"* and *"everything
   was dirty"*.
