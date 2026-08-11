# w255 — THE 14 REFUSALS ARE GONE. 14 → 0, on both sides of the join.

Revision `76477ab` (stamped in the QEMU binary). Real GA106, host driver **open 580.159.04**.
Control: `w254` at `e2b6c86`, identical configuration. `CAPTURE_RC=0`.
`KAYFABE_ISOLATES=real`, `KAYFABE_CE_EXECUTOR=local`, `KAYFABE_PT_WITNESS_EXEC=on`,
`KAYFABE_RING_VIDMEM=on`, `KAYFABE_FB_BACKING=on`, `NVKVM_RAM_BACKEND=memfd`,
`POST_CAPTURE_HOOK=scripts/bench/cup2_hook_w232.sh`.

| | `w254` | **`w255`** |
|---|---|---|
| our `REFUSED … Rm(Other(64))` | 14 | **0** |
| our `REFUSED … Rm(..)`, any status | 14 | **0** |
| host `chandesConstruct_IMPL` | 14 | **0** |
| host `kfifoRunlistSetId_GM107` | 14 | **0** |
| host dmesg delta | 42 lines | **0 lines** |
| `FORWARDED` | 18 | **32** (at the per-class print bound — a lower bound) |
| `engine=Ce` forwards | 8 | **22**, of which **14** `materialized_channel=true` |
| remaining refusals | 2 × `NoVas`, `host_chan=NONE` | **2 × `NoVas`, `host_chan=NONE`** |
| doorbells | 191 / 183 / 8 | 191 / 183 / 8 |
| `CE-SUBMIT` | 0 | **0** |

**8 of 8 predictions**, scored against an unedited pre-registration.

## The defect

A channel's runlist comes from its **group's** `engineType`
(`ogkm-580.159.04: kernel_channel_group_api.c:161-179`, `:1251-1257` →
`kernel_channel.c:885-897`). An engine object's engine comes from its **own alloc params** —
`chandesConstruct_IMPL` → `pParamToEngDescFn` (`channel_descendant.c:159-166`) =
`kceGetEngineDescFromAllocParams` (`kernel_ce_context.c:99-165`). `kfifoRunlistSetId_GM107`
then refuses `NV_ERR_INVALID_STATE` (`0x40` = 64) if they disagree, printing both numbers.

⇒ Ours: `engine_type_for` lowered `EngineKind::Ce` to a **hardcoded** `COPY0`. Its own measured
sweep says `COPY0`/`COPY1` → runlist 0, `COPY2` → 1, `COPY3` → 2. The log's `current: 0x0` is our
`COPY0`; `requested: 0x1` for `CE2` / `0x2` for `CE3` are the guest's. Both ends match the table.

★★ The core cannot express the instance — `EngineKind::Ce` carries no index — so the adapter
picked one, and its own doc said choosing CE2+ *"is a scheduling decision … which nothing at this
rung is in a position to make."* That is still true. **We never had to make it: the guest already
did**, in eight bytes it hands us.

## The fix

`RmBackend::alloc_channel` gains `hosting: Option<HostedObject>` — the engine object the channel
is being materialized to host — and the adapter reads the instance out of the guest's own
declaration. ⊘ Rejected: rewriting the guest's `engineType` to `COPY0` (the same ordinal leaves
the guest again in `NVA06F_CTRL_CMD_BIND`; `COPY0`/`COPY1` are the GRCE pair and would serialise
copies against GR; and it re-creates the C's `dma_copy_class_alloc_params` defect on purpose).

⊘⊘ The declaration **crosses the isolate wire** — the real adapter runs in the child process, so
widening only the trait would have compiled, passed every in-process test, and been dead here.

## ⊘ What did NOT change

The guest's own `dmesg` is **byte-identical to `w254`**. The guest is not one step further along,
`CE-SUBMIT` is 0, and **no execution-plane rung is claimed**. This removed a real, vendor-named
defect and made 14 host allocations succeed; it did not move the wall.

⚠ And `forwarded=32` sits **exactly on the per-class print bound**, so it is a lower bound, not a
total — §16.105's own lesson, one rung later, on the other outcome class.

Full record: `docs/design/execution_plane_increments.md` §16.106. Predictions and their unedited
scoring: `predictions_recorded_before_the_boot.md`.
