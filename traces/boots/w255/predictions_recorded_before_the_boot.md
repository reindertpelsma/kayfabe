# w255 — PREDICTIONS, recorded BEFORE the boot

**Committed before the boot. Scored unedited below.**

Change under test: the host channel is allocated on **the copy engine the guest's own object
declares**, instead of on the hardcoded `COPY0`.

## What decides the runlist — read from the driver, not from our code

1. A channel's runlist comes from its **channel group**. `kchangrpapiConstruct_IMPL` takes
   `NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS.engineType` and stores it
   (`ogkm-580.159.04: src/nvidia/src/kernel/gpu/fifo/kernel_channel_group_api.c:161-179`);
   `engineDesc → runlistId` is translated at `:1251-1257`, and `kchannelConstruct_IMPL` stamps it
   onto each channel that joins (`kernel_channel.c:885-897`).
2. An **engine object's** engine comes from its own alloc params.
   `chandesConstruct_IMPL` calls `pParamToEngDescFn` (`channel_descendant.c:159-166`), which for
   every `*_DMA_COPY_*` class is `kceGetEngineDescFromAllocParams`
   (`src/nvidia/src/kernel/gpu/ce/kernel_ce_context.c:99-165`): `VERSION_1` treats
   `NVB0B5_ALLOCATION_PARAMETERS.engineType` as an `NV2080_ENGINE_TYPE_COPY(i)` **ordinal**,
   `VERSION_0` as a bare **instance index**, anything else → `ENG_INVALID`.
3. The two are then required to agree. `chandesConstruct_IMPL:243` →
   `kfifoRunlistSetIdByEngine_GM107` → `kfifoRunlistSetId_GM107`, whose **first branch** refuses
   `NV_ERR_INVALID_STATE` (`0x40` = 64) when the channel already carries a different runlist —
   printing both numbers.

⇒ Checked against ours: `HostRmBackend::alloc_channel` lowered `EngineKind::Ce` through
`engine_type_for`, which hardcodes `engine_type_copy(0)` = `ENGINE_TYPE_COPY0`. Its own measured
sweep (RTX 3060 / 580.159.04, `--engines`) records `COPY(0)`/`COPY(1)` → **runlist 0**,
`COPY(2)` → **1**, `COPY(3)` → **2**. The host log's `current: 0x0` is our `COPY0`; its
`requested: 0x1` for `CE2` and `0x2` for `CE3` are the guest's. **Both ends match the table.**

★ And the core cannot express the instance at all: `EngineKind::Ce` has no index, and
`engine_type_for`'s own closing paragraph says choosing CE2+ *"is a scheduling decision with a
cost … which nothing at this rung is in a position to make."* ⊘ That is still true — and we do
not have to make it. **The guest already did.**

## Which repair, and why not the other one

**Chosen: move the CHANNEL to the engine the object declares.** Rejected: rewrite the object's
`engineType` to `COPY0`.

1. **The declaration is not ours to edit.** The same ordinal leaves the guest again in
   `NVA06F_CTRL_CMD_BIND` — measured `engineType = 11` = `COPY2` on real hardware
   (`traces/real_ga106/rpc_transcript_real_ga106.txt:63`). Rewriting would leave the guest
   believing `COPY2` while the host ran `COPY0`, a disagreement with no error at any layer.
2. **It would be a wrong ANSWER, not a missing one.** `COPY0`/`COPY1` are the GRCE pair on the
   graphics runlist, so forcing them serialises copies against GR work the guest expects to
   overlap.
3. **It re-creates the C's `dma_copy_class_alloc_params` defect on purpose**, immediately after
   measuring it.

⊘ Scope, deliberately narrow: only `EngineKind::Ce`, and only when the guest's blob decodes to a
declaration RM itself would accept. Everything else falls through to `engine_type_for` **byte
for byte as before** — in particular the 8 forwards that already succeed are GR channels taking a
CE object as GRCE, and moving those would break them.

## Predictions

1. ★★★ **`REFUSED … Rm(Other(64))` = 0** (was 14).
2. ★★★ **host `chandesConstruct_IMPL` = 0** and `kfifoRunlistSetId_GM107` = 0 (both were 14), on
   the same join `w254` built.
3. ★★ **The failure does not MOVE.** Total `REFUSED … Rm(..)` of *any* status = **0** — not "a
   different code now". `COPY2`→runlist 1 and `COPY3`→runlist 2 are both in the measured sweep's
   accepted range (`COPY5`+ is where RM answers `0x57`), so the channel alloc itself must succeed.
   ⊘ If a new status appears, the diagnosis was right and the repair was wrong, and that is the
   finding.
4. **`FORWARDED` rises 18 → 32** and the two `REFUSED … NoVas(..)` remain (they issue no host verb
   and this change cannot touch them) ⇒ last line reads `forwarded=32 refused=2`, ≥ 34 lines.
5. **Every refusal that remains prints `host_chan=NONE`** — `NoVas` is refused in the plan phase,
   before a channel exists. (`w254` measured exactly this for both.)
6. ⊘ **`CE-SUBMIT` stays 0.** Nothing in this change submits work, and `KAYFABE_CE_EXECUTOR=local`
   keeps the shell's CPU executor on every copy. **No execution-plane rung is claimed.**
7. Bootability unchanged: `CAPTURE_RC=0`, guest up in ~35-40 s, `RmInitAdapter` as in `w254`.
8. ⚠ **Least confident:** doorbells stay `191 arrived, 183 served, 8 REFUSED by name`. Fourteen
   channels that previously held no engine object now hold one, so a changed count here is a
   *consequence to explain*, not necessarily a regression.

## Configuration (stated)

`KAYFABE_ISOLATES=real`, `KAYFABE_CE_EXECUTOR=local`, `KAYFABE_PT_WITNESS_EXEC=on`,
`KAYFABE_RING_VIDMEM=on`, `KAYFABE_FB_BACKING=on`, `NVKVM_RAM_BACKEND=memfd`,
`POST_CAPTURE_HOOK=scripts/bench/cup2_hook_w232.sh` — identical to `w254` and `w251`.

⊘ **`isolate_plane=real` means the real adapter runs in the CHILD process**, so the declaration
has to cross the isolate wire. It does (`Request::AllocChannel::hosting`); a version of this fix
that widened only the trait would have compiled, passed every in-process test, and been dead on
the one path this boot exercises.

---

# SCORING (added after the run — the predictions above are unedited)

**Boot `w255_76477ab_cel_runlist`, rev `76477ab` (stamped in the binary), real GA106, host driver
open 580.159.04, `CAPTURE_RC=0`.** Control: `w254` at `e2b6c86`, identical configuration.

| # | prediction | outcome |
|---|---|---|
| 1 | ★★★ `Rm(Other(64))` **14 → 0** | ✅ **0** |
| 2 | ★★★ host `chandesConstruct_IMPL` **14 → 0**, `kfifoRunlistSetId_GM107` **14 → 0** | ✅ **0 / 0** — the host dmesg delta is **0 lines** (watermark 921, capture ran and reported zero) |
| 3 | ★★ the failure does not MOVE: total `Rm(..)` of any status = 0 | ✅ **0** — no new status appeared |
| 4 | `FORWARDED` 18 → 32, last line `forwarded=32 refused=2` | ✅ `[seen=34 forwarded=32 refused=2]` |
| 5 | every remaining refusal prints `host_chan=NONE` | ✅ both, and both are `NoVas` — refused in the plan phase, before a channel exists |
| 6 | ⊘ `CE-SUBMIT` stays 0 | ✅ **0** |
| 7 | bootability unchanged | ✅ `CAPTURE_RC=0`; the guest's own `dmesg` is **byte-identical to `w254`** |
| 8 | ⚠ doorbells stay `191 / 183 / 8` | ✅ `191 arrived, 183 served, 8 REFUSED by name` |

**8 of 8.**

## The decomposition, and it is exact

`engine=Ce` forwards: **8 → 22**, of which **14 carry `materialized_channel=true`**.

⇒ the 8 that already worked are unchanged (GRCE objects on GR channels that already existed,
`materialized_channel=false`), and the **14 that used to be refused now materialize their own
channel on the engine the guest declared and succeed**. 8 + 14 = 22, and 14 is the same 14.

## ⚠ ONE HONEST QUALIFICATION, and it is §16.105's own lesson one rung later

`forwarded=32` **is exactly the per-class print bound**, and the last line carries
`⊘ REPORT BOUND REACHED for this outcome class`. ⇒ **32 is a lower bound on the forwards, not a
total.** The refusal class (2) is far below its own bound and *is* exact, which is what
predictions 1-3 rest on — but a future reading that treats "32 forwarded" as a count will be
making the mistake this branch just spent three rungs paying for. ⊘ Recorded here rather than
fixed, because raising the bound changes the instrument and this boot is the fix's measurement.

## ⊘ What did NOT change, stated plainly

The guest's own `dmesg` is byte-identical to `w254`. **The guest is not one step further along.**
This rung removed a real, vendor-named defect and made 14 host allocations succeed that used to
fail; it did not move the wall, and no execution-plane progress is claimed.
