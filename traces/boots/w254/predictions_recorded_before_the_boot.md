# w254 — PREDICTIONS, recorded BEFORE the boot

**Committed before the boot. Scored unedited below.**

Change under test: the refusal now carries **the host channel it was attempted on**
(`kayfabe_isolate::VerbFailure::on` → `kayfabe_fwd::FwdFault::Rm { err, on }` → the census's
`host_chan=`), and the `ENGINE-OBJECT` report budget is now **per outcome class** instead of
shared.

## ⊘⊘ TWO THINGS IN THE BRIEF ARE REFUTED BEFORE THE BOOT, FROM SOURCE

### R1 — the join key the brief names DOES NOT EXIST

The brief asks to join our refusals and the host's `chandesConstruct_IMPL` lines **on the
channel handle**. The driver does not print a handle there:

```c
NV_PRINTF(LEVEL_ERROR, "Invalid object allocation request on " FMT_CHANNEL_DEBUG_TAG "\n",
          kchannelGetDebugTag(pKernelChannel));
// ogkm-580.159.04: src/nvidia/src/kernel/gpu/fifo/channel_descendant.c:246-250

#define FMT_CHANNEL_DEBUG_TAG "channel 0x%08x"
static inline NvU32 kchannelGetDebugTag(const struct KernelChannel *pKernelChannel) {
    if (pKernelChannel == NULL) return 4294967295U;
    return (pKernelChannel->runlistId << 24) | pKernelChannel->ChID;
}
// ogkm-580.159.04: src/nvidia/generated/g_kernel_channel_nvoc.h:206-207, 1493-1497
```

⇒ `channel 0x00000004` / `0x0000000c` are **`runlistId = 0`, `ChID = 4` and `ChID = 12`** — a
*hardware channel id*, which is allocated from a per-runlist pool and **recycled on free**. Our
side's channel identity is an RM handle from `HostRmBackend`: `FIRST_HANDLE = 0xCAFE_0001`
(`rm.rs:166`) minted by `Conn::mint` (`rm.rs:1286`), which is `next.wrapping_add(1)` —
**monotone, never recycled**.

⇒ ★★ **The two sides can never be equal.** The join is by **grouping and cardinality**, not by
equality — which also means *"the host's 14 land on exactly two channels"* is not a statement
about two channels. Two identical `ChID`s are equally consistent with **one long-lived channel**
and with **N channels that were each created, refused, freed, and had their chid handed straight
back**. Distinguishing those is exactly what `host_chan=` now does.

⚠ The bench runs the **open** driver **580.159.04** (`/proc/driver/nvidia/version`, checked
2026-08-11), i.e. the very tree cited above. The formula is identical in `research_clones/ogkm`
(610.43.02).

### R2 — "ours 12" IS A READING OFF A SATURATED COUNTER, and the numbers fit truncation EXACTLY

`ENGINE_FWD_REPORT_MAX = 32` was a **shared** budget over forwards *and* refusals. In both
`w250` and `w251` the last printed line is `[seen=32 forwarded=18]` and it carries
`⊘ REPORT BOUND REACHED`. Decompose it:

| | w250 | w251 |
|---|---|---|
| `FORWARDED` lines | 18 | 18 |
| `REFUSED NoVas(..)` lines (no host verb issued) | 2 | 2 |
| `REFUSED Rm(Other(64))` lines | 12 | 12 |
| **sum** | **32** | **32** |

⇒ the instrument stopped **exactly at its own limit**, on the last refusal, in both boots.
§16.101.3's row *"our census total (`seen=32 forwarded=18` ⇒ 14 refused)"* reads `seen` off the
**last line the bound allowed**, so it is the last *observable* value, not a total. An outcome
33 and 34 would be invisible — and `18 + 2 + 12 + 2 = 34` is precisely what closes the gap.

⇒ ⊘ **Truncation was never excluded**, and it is the cheapest remaining explanation of 14-vs-12:
no second allocator, no retry, no missing caller — two refusals that happened after the log
stopped. The per-class budget under test decides it.

## Predictions

1. ★★★ **`REFUSED … Rm(Other(64))` = 14**, not 12 — the two missing refusals appear now that
   refusals have their own budget. ⊘ If it is still 12, truncation is **excluded** and the
   14-vs-12 gap is real and unexplained — a fourth refuted hypothesis, and that is a result.
2. Total `ENGINE-OBJECT` lines **> 32** (predicted 34 = 18 forwarded + 2 `NoVas` + 14 `Rm`), and
   the last line's counters read `forwarded=18 refused=16`.
3. ★★ **Every `Rm(Other(64))` refusal prints `host_chan=0xcafe….`, never `host_chan=NONE`.**
   `NONE` would mean the chain never reached `alloc_engine_object`, i.e. the plumbing is on a
   path this rung mis-identified.
4. ★★★ **The number of DISTINCT `host_chan` values among those refusals is NOT 2.** `mint()` is
   monotone, so a channel materialized inside each failing chain gets a fresh handle every time.
   Predicted: **≥ 6 distinct**, and plausibly one per refusal (14). ⊘ If it *is* exactly 2, then
   the failing allocs really are landing on two long-lived host channels and the recycling
   reading is wrong — also a result, and a sharper one.
5. The host side is **unchanged**: 14 `chandesConstruct_IMPL`, 14 `kfifoRunlistSetId_GM107`,
   split `6 × channel 0x00000004` / `8 × channel 0x0000000c`, engines `8 × CE2` / `6 × CE3`.
   This change adds no host verb, so a difference here indicts the change, not the guest.
6. Bootability unchanged: the guest boots, `RmInitAdapter` behaves as in `w251`, and the doorbell
   census stays at `191 arrived, 183 served, 8 REFUSED by name`.
7. ⊘ `CE-SUBMIT` **0**. Nothing here executes guest work, and no rung is claimed on the
   execution plane.

## Configuration (stated, per the standing rule)

`KAYFABE_PT_WITNESS_EXEC=on`, `KAYFABE_CE_EXECUTOR=local` (matching `w251`, the control this is
compared against), `KAYFABE_GUEST_RAM=memfd`,
`POST_CAPTURE_HOOK=scripts/bench/cup2_hook_w232.sh`.

---

# SCORING (added after the run — the predictions above are unedited)

**Boot `w254_e2b6c86_cel_hostdmesg`, rev `e2b6c86` (stamped in the binary), real GA106, host
driver open 580.159.04. `CAPTURE_RC=0`.** Evidence:
`traces/guest_boots/run_w254_e2b6c86_cel_hostdmesg_{qemu,dmesg,probe,hostdmesg}.log`.

| # | prediction | outcome |
|---|---|---|
| 1 | ★★★ `Rm(Other(64))` = **14**, not 12 | ✅ **14** |
| 2 | total lines **> 32** (predicted 34), last line `forwarded=18 refused=16` | ✅ **34**, `[seen=34 forwarded=18 refused=16]` |
| 3 | ★★ every `Rm` refusal prints `host_chan=0xcafe…`, never `NONE` | ✅ 14/14; and both `NoVas` refusals print `NONE`, correctly — they issue no verb |
| 4 | ★★★ distinct `host_chan` values **≠ 2**, predicted ≥ 6 | ✅ **14 distinct**, one per refusal |
| 5 | host side unchanged: 14/14, `6 × 0x04` + `8 × 0x0c`, `8 × CE2` / `6 × CE3` | ✅ byte-identical to `w250`/`w251` |
| 6 | bootability unchanged, doorbells `191/183/8` | ✅ `191 arrived, 183 served, 8 REFUSED by name`; guest up in 37 s (w251: 36 s) |
| 7 | `CE-SUBMIT` 0 | ✅ 0 (`RING-PROJ` 0, `GR-FB-BACKING` 32, `GR-ADDRESS-CENSUS` 16 — all identical to `w251`) |

**7 of 7.** ⊘ Nothing in this rung executes guest work.

## ★★★★★ THE JOIN CLOSES — 14 : 14, POSITIONALLY, IN ORDER

| # | ours (`ENGINE-OBJECT … REFUSED`) | host (`chandesConstruct_IMPL`) |
|---|---|---|
| 1 | `seen=2` c=`0xc1e00006` `host_chan=0xcafe0009` **iso0** | `CE2` `channel 0x00000004` |
| 2 | `seen=5` c=`0xc1e00011` `0xcafe0017` **iso0** | `CE2` `0x00000004` |
| 3 | `seen=7` c=`0xc1d0000a` `0xcafe0025` **iso0** | `CE2` `0x00000004` |
| 4 | `seen=8` `0xcafe002c` **iso0** | `CE2` `0x00000004` |
| 5 | `seen=9` `0xcafe0033` **iso0** | `CE3` `0x00000004` |
| 6 | `seen=10` `0xcafe003a` **iso0** | `CE3` `0x00000004` |
| 7 | `seen=27` c=`0xc1d0000c` `0xcafe0039` **iso2** | `CE2` `0x0000000c` |
| 8 | `seen=28` `0xcafe003e` **iso2** | `CE2` `0x0000000c` |
| 9 | `seen=29` `0xcafe0043` **iso2** | `CE2` `0x0000000c` |
| 10 | `seen=30` `0xcafe0048` **iso2** | `CE2` `0x0000000c` |
| 11 | `seen=31` `0xcafe004d` **iso2** | `CE3` `0x0000000c` |
| 12 | `seen=32` `0xcafe0052` **iso2** | `CE3` `0x0000000c` |
| 13 | `seen=33` `0xcafe0057` **iso2** | `CE3` `0x0000000c` |
| 14 | `seen=34` `0xcafe005c` **iso2** | `CE3` `0x0000000c` |

★★★ **`6 then 8` on BOTH sides, in the same order, 1:1.** And the host's *"two channels"* are
**our two ISOLATES**: every refusal from `iso0` carries chid `0x04`, every refusal from `iso2`
carries chid `0x0c`. Fourteen distinct host channel objects — `mint()` is monotone and no handle
repeats — each created inside its own failing chain, refused, and freed, so the chid returns to
the pool and the *next* attempt in the same isolate is handed the same one back.

⇒ ⊘⊘ **There is no second allocator, no retry, and no missing caller.** §16.102's *"the gap is 2
attempts, not 2 objects"* is superseded: the gap was **2 log lines**, and the *"two channels"*
that framed three rungs of search were two recycled chids that were never two channels.

⇒ ★★ **The unit was never wrong. The INSTRUMENT was truncated and the KEY was misread** — and
both were legible from the artifacts already committed (`18 + 2 + 12 = 32` exactly, and
`kchannelGetDebugTag`) without a single boot.
