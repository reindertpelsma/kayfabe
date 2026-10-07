# w254 — THE JOIN CLOSES, 14 : 14. And it closed by refuting its own brief, twice.

Revision `e2b6c86` (stamped in the QEMU binary). Real GA106, host driver **open 580.159.04**.
Control: `w251` (`acbb9a3`) — and `git diff acbb9a3 e2b6c86 -- crates/` is **empty apart from
this rung's own change**, so the two boots differ by exactly the instrument.
`CAPTURE_RC=0`, `KAYFABE_CE_EXECUTOR=local`, `KAYFABE_PT_WITNESS_EXEC=on`,
`KAYFABE_RING_VIDMEM=on`, `KAYFABE_FB_BACKING=on`, `KAYFABE_ISOLATES=real`, memfd guest RAM,
`POST_CAPTURE_HOOK=scripts/bench/cup2_hook_w232.sh`.

| | `w251` | **`w254`** |
|---|---|---|
| `ENGINE-OBJECT` lines printed | 32 (**bound**) | **34** |
| `REFUSED Rm(Other(64))` | 12 | **14** |
| `REFUSED NoVas(..)` | 2 | 2 |
| `FORWARDED` | 18 | 18 |
| distinct host channels named on the refusal path | *unprintable* | **14** |
| host `chandesConstruct_IMPL` / `kfifoRunlistSetId_GM107` | 14 / 14 | 14 / 14 |
| host channels named | `6 × 0x04`, `8 × 0x0c` | `6 × 0x04`, `8 × 0x0c` |
| doorbells | 191 / 183 / 8 | 191 / 183 / 8 |
| `CE-SUBMIT` | 0 | **0** |

## ⊘⊘ Two refutations, both from source, both before the boot

1. **The join key the brief named does not exist.** `chandesConstruct_IMPL` prints
   `kchannelGetDebugTag = (runlistId << 24) | ChID` — a **recycled hardware chid**, not an RM
   handle (`ogkm-580.159.04: channel_descendant.c:246-250`,
   `g_kernel_channel_nvoc.h:206-207,1493-1497`). Ours are `0xCAFE_0001`+, minted monotonically.
2. **"ours 12" was read off a saturated counter.** `ENGINE_FWD_REPORT_MAX = 32` was **shared**;
   `18 + 2 + 12 = 32` **exactly**, in both prior boots, with the bound marker on the last line.

## ★★★★★ The result

**Fourteen refusals, fourteen host failures, in the same order.** Ours split `6` on isolate 0
then `8` on isolate 2; the host's split `6 × chid 0x04` then `8 × chid 0x0c`. The *"two channels"*
that framed three rungs of search are **our two isolates** — fourteen distinct host channel
objects, each built inside its own failing chain, refused, and freed, so the chid goes back to the
pool and the next attempt in that isolate is handed the same one.

⇒ ⊘⊘ **No second allocator, no retry, no missing caller.** §16.102's *"the gap is 2 attempts, not
2 objects"* is superseded: the gap was **2 log lines**.

★★ Both facts were legible from artifacts already committed. The unit was never wrong; the
**instrument was truncated** and the **key was misread**.

⊘ Nothing here executes guest work, and the underlying defect is untouched: we still alloc an
async-CE object (`CE2`/`CE3`, runlists 1 and 2) on a host channel bound to runlist 0.

Full record: `docs/design/execution_plane_increments.md` §16.105. Predictions and their unedited
scoring: `predictions_recorded_before_the_boot.md` (7 of 7).
