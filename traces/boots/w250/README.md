# w250 — the HOST's dmesg is captured per boot, and the join does NOT close

⊘ `CE-SUBMIT` **0**; nothing executed. Revision `acbb9a3`, all three flags armed,
`ce_executor=host`. `CAPTURE_RC=0`.

## ★★★ The join, both sides in the tree, same boot

| | count |
|---|---|
| ours: `ENGINE-OBJECT … REFUSED Rm(Other(64))` (issued a host verb) | **12** |
| ours: census `seen=32 forwarded=18` ⇒ refused | 14 (12 + 2 `NoVas`, which issue no verb) |
| **host: `chandesConstruct_IMPL` failures** | **14** |
| **host: `kfifoRunlistSetId_GM107` failures** | **14** |
| host: Xid | 0 |

⇒ **12 ours, 14 the host's.** Reproduced identically on two independent boots (`w249`, `w250`).
**Two host-side engine-object failures come from a path that is not `forward_engine_object`** —
named, unexplained, and the thread to pull.

The host names what we never did:

```text
kfifoRunlistSetIdByEngine_GM107: Unable to program runlist for CE2  ×8  → channel 0x0000000c
kfifoRunlistSetIdByEngine_GM107: Unable to program runlist for CE3  ×6  → channel 0x00000004
kfifoRunlistSetId_GM107: … (requested: 0x1 current: 0x0) ×8 / (requested: 0x2 current: 0x0) ×6
```

## ⊘⊘ It is a SOURCE IDENTITY, not a correlation — and it is NOT the GR wall

`kfifoRunlistSetId_GM107` **returns `NV_ERR_INVALID_STATE`** (`0x40` = **64**) at the same
statement that prints the runlist line; `chandesConstruct_IMPL` prints the second line and
returns that status to our alloc. ⇒ the two log lines and our `64` are **one failure at three
levels of one call** (`kernel_fifo_gm107.c:407-418`, `channel_descendant.c:243-252`,
`nvstatuscodes.h:93`).

⊘ **And the two refusal populations are disjoint**: all **12** `Rm(Other(64))` are class
`0xc7b5` (**copy-engine** objects, zero graphics); the **8** `Route::NotACopyEngineChannel`
doorbell refusals are **our own router** on `GrCompute` channels and produce **no host line at
all**. **The driver has never said anything about the GR wall.**

## The harness

`boot_capture.sh` now watermarks the host ring buffer before the boot and persists the delta as
`run_<tag>_hostdmesg.log`. ⊘ A watermark, not a snapshot — `dmesg` holds every boot the host ever
served, which is why *241 lines* was an unattributable campaign total.
⊘⊘ **Deliberately not asserted non-empty**: zero host lines is a legitimate result, so the count
is **stated in the probe log** instead of demanded to be non-zero.

★★★ **Its first placement was wrong and the validating boot caught it**: before the workload hook
it captured **3 of 53** lines. Moved to phase 3c, after the hook. **The instrument was placed
where it could not see the event.**

Full record: `docs/design/execution_plane_increments.md` §16.101.
