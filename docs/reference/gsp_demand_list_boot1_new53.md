# The 53 controls the SUCCESSFUL boot demands and `cap1` never asks for

**Task #180, the `replay-conformance` line.** `successful_boot_demand_ga106.md` (#178/#179's
sequel) measured the shape of the remaining ladder: `rpctrace_ga106_boot1` demands **104**
distinct controls, **53 of them absent from `cap1`'s list**. This document classifies those 53
to the standard #179 met — **DATA / ACT / MIXED / UNKNOWN**, each with a citation to *both* the
params struct and the code that decides direction — and reports, by name, the residue it could
not close.

| artefact | what it is |
|---|---|
| `docs/reference/gsp_control_classification.tsv` | now **106 rows**: #179's 53 (`cap1`) + this task's 53, same columns |
| `scripts/rm_ctrl_index.py` | #179's static evidence tool (id → `#define`, struct, NVOC export, `RMCTRL_FLAGS_*`) — **reused, not rebuilt** |
| `scripts/rpctrace/ctrl_payload_pairs.py` | ★ new: pairs every request with its reply and reports `out_vs_in` and `replayable` |

Regenerate the measured half. **Every measurement in this document comes from one committed
capture** — `traces/rpctrace_ga106_boot1.bin`, recorded 2026-08-03 (task #178) on a real GA106
RTX 3060 running open `580.159.04`, `n_dropped=0`, decoded by `decode_rpctrace.py`:

```
scripts/rpctrace/ctrl_payload_pairs.py traces/rpctrace_ga106_boot1.bin
scripts/rpctrace/ctrl_payload_pairs.py traces/rpctrace_ga106_boot1.bin --dump 2080014b
```

---

## 1. The split, and it is not #179's split

| bucket | controls | share | #179's share, for contrast |
|---|---:|---:|---:|
| **DATA** | 26 | 49.1 % | 71.7 % |
| **ACT** | 4 | 7.5 % | 20.8 % |
| **MIXED** | 1 | 1.9 % | 1.9 % |
| **UNKNOWN** | **22** | **41.5 %** | 5.7 % |

**Bucketed with a citation to the params struct *and* to the direction-deciding code: 31 / 53
= 58.5 %.** #179 reached 94.3 % on its 53.

⚠ **That drop is the headline, not a shortfall in effort.** #179's half of the ladder is
bring-up: `NV2080_CTRL_INTERNAL_*`, which the open driver both defines and consumes in-tree.
This half is `nvidia-smi`-era interrogation, and **41.5 % of it lives in the GSS-legacy command
space that the open source does not model at all** (§4). The readable fraction of the ladder
falls off a cliff exactly where the ladder leaves bring-up.

### 1.1 The four ACTs

| id | control | what must actually happen |
|---|---|---|
| `0x00730151` | `SYSTEM_MAP_SHARED_DATA` | GSP maps a **guest-physical** memdesc — "Maps the memory allocated in Kernel RM into Physical RM using the memory descriptor information provided" (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl0073/ctrl0073system.h:1811-1813`) |
| `0x20800a49` | `INTERNAL_DISPLAY_WRITE_INST_MEM` | GSP writes the display instance-memory register from a **physical address** (`ogkm-580: src/nvidia/src/kernel/gpu/disp/inst_mem/disp_inst_mem.c:386-395`) |
| `0x20800ac6` | `INTERNAL_INIT_BRIGHTC_STATE_LOAD` | GSP takes the host's ACPI `_DSM` backlight blob (`ogkm-580: .../kern_disp.c:383-395`) |
| `0x20800adf` | `INTERNAL_SET_STATIC_EDID_DATA` | GSP takes the host's ACPI `_DDC` EDID table (`ogkm-580: .../kern_disp.c:420-436`) |

★ Note what they have in common, and how it differs from #179's eleven. #179's ACTs were the
memory barrier, the cache invalidate, the page-table publication, the channel bind and the
runlist write — **every one on the path to first compute**. These four are all **display**,
and none is on that path. Two of them hand GSP a physical address, so they are not
table-servable in any form; the other two hand it host ACPI data a replay would have to
manufacture. The ladder to `nvidia-smi` adds *no new compute-path acts at all*.

### 1.2 The one MIXED — and it is a name-trap in the opposite direction

`0x00730108` `SYSTEM_GET_CONNECT_STATE` reads like pure data and carries real `[out]`:
`displayMask` comes back as "the subset of displays in the mask that are connected"
(`ogkm-580: ctrl0073system.h:373-374`) and `retryTimeMs` "is an output to this command"
(`:382-385`). But `flags` selects the **detection method**: `_METHOD_DEFAULT` is documented as
"The system decides what method to use", `_ECONODDC` as "Ping the DDC address of the given
display mask", and `_LOAD` enables load detection (`:336-366`) — i.e. at the default the call
drives a **physical probe of the connectors**. The in-tree caller passes `_METHOD_CACHED`
*explicitly* (`ogkm-580: .../kern_disp.c:1931`), which is itself evidence that the un-flagged
form is not a pure read; and the trace's one call passes `flags = 0`. ⇒ MIXED at MEDIUM.

---

## 2. ★★ A real GSP refuses eleven of these 53 — and RM's own source expects seven of them

`successful_boot_demand_ga106.md` §2 recorded 13 refusals across all 104 controls. Eleven of
those fall in this 53 (the other two, `0x20800a87` and `0x20800b05`, are `cap1` rows #179
already bucketed DATA — worth knowing: **a control can be DATA and still be refused by
hardware**).

**Unconditional `0x56` `NV_ERR_NOT_SUPPORTED` — nine:**

`0x2080012f`, `0x20800157`, `0x20801322`, `0x20801344`, `0x20801357`, `0x20809038`,
`0x2080a0f2`, `0x2080a63c`, `0x90e70113`.

**Conditional — two:** `0x2080014b` (`NV_OK` and `0x57` over 10 calls) and `0x20808546`
(`NV_OK` and `0x56` over 18). §3.

★ **The cross-check that closed:** `ogkm-580` carries a list of commands whose unsupported
status RM declines even to log —
`rmapiutilSkipErrorMessageForUnsupportedVgpuGuestControl` (`ogkm-580:
src/nvidia/src/kernel/rmapi/rmapi_utils.c:198-247`). **Six** of the refused ids are on it
(`0x20800157` `:211`, `0x20801322` `:208`, `0x20801344` `:209`, `0x20801357` `:237`,
`0x90e70113` `:240`, `0x2080014b` `:204`), and the seventh, `0x2080012f`, has an explicit
tolerance arm at its call site:

```c
    if (status == NV_ERR_NOT_SUPPORTED)
    {
        // Nothing to do if ECC not supported
        rmSubDevice->bEccEnabled = NV_FALSE;
        status = NV_OK;
```
`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:1255-1261`

⇒ **Every refused control that CPU-RM has a name for, CPU-RM anticipates being refused — 7 of
7.** The four it does not anticipate (`0x20809038`, `0x2080a0f2`, `0x2080a63c`, `0x20808546`)
are exactly the four GSS-legacy ones, which the source *cannot* name. The negative measured on
2026-08-03 on a real GA106 (`traces/rpctrace_ga106_boot1.bin`, task #178) and the reading of
`ogkm-580` agree with zero exceptions, and the exceptions there are, are structural.

⊘ Scope, honestly: `rmapiutil…VgpuGuest…` is written about a **vGPU guest**. It is evidence
that RM has a named class of legitimately-unsupported controls containing these ids; it is not
by itself a statement about the bare-metal path. What makes the pairing strong is that the
bare-metal refusal was *measured* — `traces/rpctrace_ga106_boot1.bin`, 2026-08-03, a real GA106
replying `0x56` — while the anticipation is *read* out of `ogkm-580`. Two different instruments,
one answer; neither is doing the other's job.

---

## 3. ★★★ Both conditionals are keyed on the ARGUMENTS, and one is measurably deterministic

`successful_boot_demand_ga106.md` §2.2 said "the answer depends on arguments or on state, and a
static policy cannot express it." Having the request payloads, this pass can be more precise —
and the correction matters, because it changes what a replay must build.

### `0x2080014b` `GPU_GET_INFOROM_OBJECT_VERSION` — a pure function of the `[in]` string

A **reading** first: `objectType` is a 3-char `[in]` and `version`/`subversion` are described
as returned (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h:1806-1817`).

A separate and different kind of evidence, decoded from the committed capture
`traces/rpctrace_ga106_boot1.bin` and identical in both of its bring-ups:

| `objectType` | status | reply |
|---|---|---|
| `OBD` | `NV_OK` | `version = 2` |
| `PWR`, `CFG`, `EEN`, `PBL` | `0x57` `NV_ERR_OBJECT_NOT_FOUND` (`ogkm-580: nvstatuscodes.h:116`) | untouched |

The same five requests are re-issued in the second, independent GSP bring-up and get the same
five answers. ⇒ **The status is a deterministic function of the argument**, not of hidden
state: `0x57` means "this board's InfoROM has no such object". A table **keyed on the params**
expresses this exactly; only a table keyed on the cmd id cannot.

### `0x20808546` — keyed on an index, but its determinism has not been measured

Measured in `traces/rpctrace_ga106_boot1.bin` (`--dump 20808546`): 24-byte params whose first
`NvU32` is an index the driver walks: `08 07 06 05 04`, then
`0a 0b … 15`, then `16`. GSP answers `NV_OK` for `0x0a, 0x0c, 0x0d, 0x12, 0x13, 0x16` and
`0x56` for the rest. The status tracks the `[in]` index, and the driver visibly reuses one
struct — the request at `seq878` is byte-identical to the *reply* at `seq876`. ⊘ But **each
index occurs exactly once in `traces/rpctrace_ga106_boot1.bin`**, so unlike `0x2080014b` this is
a correlation over single observations, not a demonstrated function — **unmeasured** as a
function. Recorded as conditional-on-argument at LOW confidence.

⇒ Correction to carry forward: **"a static policy cannot express it" is too strong.** A policy
keyed on the *command id* cannot. A policy keyed on the *params* expresses `0x2080014b`
exactly, and is the only shape that could ever express `0x20808546`. That is the same
`CACHEABLE_BY_INPUT` shape #179 already had to build for four `cap1` rows.

---

## 4. ⊘ The residue, named: 22 GSS-legacy controls, and why they are one refusal

```
0x2080852a 0x2080852c 0x2080852e 0x2080852f 0x20808536 0x2080853a 0x20808542 0x20808546
0x20809009 0x20809019 0x20809038 0x20809064 0x2080a060 0x2080a080 0x2080a084 0x2080a0a4
0x2080a0a7 0x2080a0a8 0x2080a0f2 0x2080a612 0x2080a618 0x2080a63c
```

Every one has **bit 15 of the command set** — `RM_GSS_LEGACY_MASK 0x00008000` (`ogkm-580:
src/nvidia/interface/deprecated/rmapi_deprecated.h:41`) — and for every one, CPU-RM has no
`#define`, no params struct, no NVOC export, no `RMCTRL_FLAGS_*` and no `paramsSize`.
`RmGssLegacyRpcCmd` copies the caller's `paramsSize` bytes in (`rmapi_gss_legacy_control.c:66-80`),
forwards the buffer **verbatim** to physical RM / GSP (`:110-127`), and — only on `NV_OK` —
copies the same buffer straight back out (`:145-151`). Its own comment states the position:

> Some clients are still making these legacy GSS controls. We no longer support these in RM,
> but until all the numerous tools are updated to use alternative APIs, just forward all of
> them to GSP and let it deal with what is or isn't valid.
> — `ogkm-580: src/nvidia/interface/deprecated/rmapi_gss_legacy_control.c:33-37`

⚠ **Checked in both trees**: none of the 22 ids appears anywhere in `ogkm-580.159.04` *or*
`ogkm-610.43.02`. The versions do not disagree here; they are both silent.

⊘ These are **not "probably DATA"**. The kernel treats the buffer as bidirectional *by
construction* — it neither knows nor cares what is in it — so the direction is undecidable
from source in exactly the way #179's three `NV2081_BINAPI` ids were. The difference is scale:
BINAPI was 3 of 53, this is 22 of 53.

### 4.1 What the trace *can* narrow, and what it cannot

This pass has evidence #179 did not: the recorder captures the **request and the reply body**,
so a payload byte-diff is available — the exact settling instrument #179 named as missing.

- **17 of the 22 measured `DIFF`** — the reply carries bytes the request did not, so they are
  **not pure ACTs**. They have `[out]` fields. What stays unknown is whether they *also* act.
- `0x2080a084` succeeded once with `out == in`; per §5 that is evidence of nothing.
- `0x20809038`, `0x2080a0f2`, `0x2080a63c` were refused, so nothing was measured.
- `0x20808546` is the conditional of §3.

That is a genuine narrowing — `UNKNOWN` with "carries `[out]`, may or may not also act" is
strictly more than `UNKNOWN` — but it is **not** a bucket, and it is not recorded as one.

---

## 5. ⊘ The trap in the new instrument, and the two measurements that caught it

`ctrl_payload_pairs.py` reports `out_vs_in`. The sound direction is one-way:

> `DIFF` ⇒ the control has `[out]` fields ⇒ **not a pure ACT**.
> `SAME` ⇒ *this call* added nothing. It does **not** mean the control returns nothing.

Two rows in this 53 are the counterexamples, and both are documented `[out]`:

- `0x0073010c` `SYSTEM_GET_ACTIVE` — 12 calls, all `SAME`. Its `displayId` is explicitly
  "returns the displayId of the active display. A value of zero indicates no display is
  active" (`ogkm-580: ctrl0073system.h:637-640`). The board has nothing attached, so the
  `[out]` equals the zero the caller passed in.
- `0x20801813` `BUS_GET_PEX_COUNTERS` — `SAME` because every PCIe error counter reads 0 on a
  healthy link (`ogkm-580: ctrl2080bus.h:826-849`).

Had `SAME ⇒ ACT` been applied, both would have been bucketed ACT, and a replay would then have
*performed* nothing where the driver wanted a number. This is the same shape as the FIFTH
LIMIT: **an empty answer is not an answer of emptiness.**

---

## 6. Self-contained vs references-state, for the DATA rows

The capture contains **two complete, independent GSP bring-ups** (`decode_rpctrace.split_sessions`
— with persistence off, each `nvidia-smi` is a full bring-up and teardown), so "same request,
same reply, twice" is a real test rather than a repeat.

Of the **26 DATA** rows:

| finding | rows | meaning for a replay |
|---|---:|---|
| reply **stable** for a given request across both bring-ups (`STABLE` or `STABLE/KEYED`) | **11** | self-contained as far as this capture can show — servable from a table, four of them only if the table is keyed on the params |
| reply **VARIES** for a byte-identical request | **1** | `0x20801819` — cannot be served from a table at all |
| issued **once** in the whole capture (`ONCE`) | **14** | settles nothing; 4 of these were refused, so there was no reply to compare |

Across all 53 (not just the DATA rows): 16 `STABLE`, 9 `STABLE/KEYED`, 1 `VARIES`, 27 `ONCE`.

★ `0x20801819` `BUS_GET_PEX_UTIL_COUNTERS`: two calls with the identical request `mask = 1`
answer `0x1994e5f0` and `0x199510e8` — **live PCIe TX/RX byte counters**. It is the one row in
the 53 a table cannot serve *even keyed on the params*.

⊘ Three honest limits on that column:

1. `ONCE` is not `STABLE`. **27 of the 53** were issued exactly once in the whole capture; for
   those the column reports `ONCE` and settles nothing.
2. Stability across two bring-ups **on the same board, minutes apart** is not stability across
   boards or over time. `0x20803400`/`0x20803401` (ECC counters), `0x90e70113` (a flush
   *timestamp*), `0x20801813` (error counters) are all self-contained here **only because the
   board is clean and freshly booted**. They are counters; treat "stable" as "stable on a
   pristine GA106".
3. A stable reply says nothing about whether the control also *acts* — §1.2 and §4.1.

---

## 7. Three things that refuted an expectation

1. **The unreadable fraction is not spread out — it is one wall.** Going in, the expectation
   was a long tail of individually-awkward controls. It is not: **22 of the 22 unknowns are the
   same structural refusal**, the GSS-legacy space, and the other 31 read cleanly. The ladder
   is not 53 hard problems; it is 31 easy ones and one closed-firmware wall.
2. **"A static policy cannot express the conditionals" was too strong** (§3). One of the two is
   a *measured deterministic function of its `[in]` argument*, and the shape that expresses it
   — a table keyed on the params — already exists for four `cap1` rows. The distinction that
   survives is **cmd-keyed vs params-keyed**, not static vs dynamic.
3. **A control can be DATA and still be refused by real hardware.** `0x20800a87` and
   `0x20800b05` were bucketed DATA by #179 off a clean reading of the driver, and a real GA106
   answers `NV_ERR_NOT_SUPPORTED` to both on a boot that then works. The bucket says what the
   reply *means*; it never said the reply exists. A replay needs both facts and they are
   independent.

---

## 8. What this does not settle

- ⊘ **22 of 53 are UNKNOWN** and are not decidable from open source at 580 or 610 (§4). Only a
  payload semantics recovery — or NVIDIA — can close them.
- ⊘ `0x00730108` is MIXED at MEDIUM and `0x208f1105` is DATA at MEDIUM; both reasons are in the
  TSV `note` column.
- ⊘ `0x20808546`'s argument-keying is a correlation over single observations (§3).
- ⊘ Everything static here is read out of `ogkm-580.159.04`; **no claim was checked against
  610.43.02** except the negative in §4, and none should be assumed to carry over
  (`ogkm_is_versioned`).
- ⊘ One board, one driver, two `nvidia-smi` bring-ups, no CUDA, no second process, no reorder —
  the same scope caveat `successful_boot_demand_ga106.md` §3 carries.
- ⊘ Nothing here was run against hardware by this task. The measured half is a decode of a
  committed capture; the static half is a reading. They are different kinds of evidence
  throughout and the TSV keeps them in different sentences.
