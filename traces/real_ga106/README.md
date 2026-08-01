# `traces/real_ga106/` — what a **real GA106** answers, asked directly

★★★ These are not captures of this port, and they are not the C oracle. They are the
**physical-control reply plane of a genuine NVIDIA GA106**, read out of the driver that owns
it, on the day it was asked.

## Provenance

| | |
|---|---|
| part | NVIDIA GeForce RTX 3060 (GA106), `GPU-e28d7776-e4f9-704b-d392-d46f187343f8` |
| host | vast.ai instance `46494693`, kernel `6.8.0-59-generic` |
| driver | **NVIDIA open kernel modules 580.159.04** — the same version the Mode-2 guest runs |
| source | `research_clones/ogkm-580.159.04`, git tag `b81d58e` (`version.mk: 580.159.04`) |
| date | 2026-08-01 |
| method | that exact source rebuilt with `NV_PRINTF(LEVEL_ERROR, …)` probes, `insmod`ed in place of the stock DKMS module, and `RmInitAdapter` driven by `nvidia-smi` opening `/dev/nvidia0` |

The box was returned to its stock module afterwards.

## The files

### `fmb_real_ga106.txt`

The targeted measurement (RTX 3060, open 580.159.04, 2026-08-01), from two probes placed
independently:

- `kceGetFaultMethodBufferSize_IMPL` — the deserialised `NvU32` *after* the RPC returns;
- `kchangrpAllocFaultMethodBuffers_GV100` and `kfifoCalcTotalSizeOfFaultMethodBuffers_GV100`
  — the value as each consumer actually uses it.

```
0x20802a08 -> status=0x0 params.size=20480 (0x5000) sizeof=4
kchangrpAllocFaultMethodBuffers bufSizeInBytes=20480 (0x5000) runQueues=2
calcTotal perGroup=20480 maxChannelGroups=4096 runQueues=2
```

### `rpc_transcript_real_ga106.txt`

Every `ROUTE_TO_PHYSICAL` control the CPU-side RM sent to GSP during one cold
`RmInitAdapter`, logged at `rpcRmApiControl_GSP`'s reply (`ogkm-580: rpc.c:11064`) — **88
calls, 55 distinct commands**, each with its `paramsSize`, the GSP's own status, and the
first eight reply bytes.

⚠ **`head=` is the first 8 bytes only.** It is enough to settle a `NvU32` and to falsify an
"empty" row; it is *not* a reply body. A control needing more must be re-measured on an
RTX 3060 (open 580.159.04, as on 2026-08-01), and the recipe above is how — see the next
file, which is that re-measure.

### `rpc_bodies_real_ga106.txt`

★★★ **Whole reply bodies**, taken 2026-08-01 on the same part with the same recipe, with the
probe widened from an 8-byte head to a chunked full-body dump over an allowlist: **the eleven
`dlen = 0` rows of the C oracle's table, plus `0x20800a2a`** (3712 bytes, whose head was all
the transcript above could see). 612 lines, 12 distinct commands.

Every block is bracketed by a `BEGIN` carrying `psize` and an `END` carrying it again, and
every line is `+<offset> <up to 16 bytes>`, so a dropped line is detectable rather than
readable as a short reply — `crates/kayfabe-abi/tests/real_ga106_bodies.rs` asserts exactly
that before it compares anything.

★ Repeated blocks for one command are repeated **calls**, kept verbatim. `0x20800a6c`
answers `0x31` three times during adapter init and `0x11` afterwards in the same run — it
**echoes its `[IN]` `flags`**, and those are the two words `kbusVerifyBar2_GM107`'s call
sites pass. A deduplication would have hidden it, and it is what refuted
`kayfabe_abi::l2evict`'s claim that a real GSP answers this control with four zeros.

⚠ `0x20800a4c` (`INTERNAL_GPU_GET_SMC_MODE`) is **not** an `RmInitAdapter` control. It is
reached only when a client asks `NV2080_CTRL_GPU_INFO_INDEX_GPU_SMC_MODE`
(`ogkm-580: subdevice_ctrl_gpu_kernel.c:232-266`), so the run was widened with
`nvidia-smi -q` to reach it. It is the eleventh row and it was the only one the previous
transcript never saw.

## ★★★ Why this exists: the C oracle is wrong about a whole class of rows

The C research artifact's `src/qemu/mode2_initctrl_ga106.h` is labelled *"real, captured from
host"*, and for rows carrying a body it **is** — `0x20800a61` and `0x20800a80` match this
transcript byte for byte. But every row it recorded with `dlen = 0` is contradicted:

★★★ **All eleven were then re-measured, not a sample** (2026-08-01,
`rpc_bodies_real_ga106.txt`):

| control | C's row | real GA106 | verdict |
|---|---|---|---|
| `0x20802a06` | `psize 4, dlen 0` | `10 00 00 00` | **contradicted** |
| `0x2080017e` | `psize 8, dlen 0` | `00 00 00 02 00 00 00 00` | **contradicted** |
| `0x20800af3` | `psize 2, dlen 0` | `01 01` | **contradicted** |
| `0x20802a08` | `psize 4, dlen 0` | `00 50 00 00` | **contradicted** |
| `0xa06f0103` | `psize 3, dlen 0` | `01 00 00` | **contradicted** |
| `0xa06f0104` | `psize 4, dlen 0` | `0b 00 00 00` | **contradicted** |
| `0x20800a4b` | `psize 4, dlen 0` | `00 00 01 04` | **contradicted** |
| `0x20800aac` | `psize 4, dlen 0` | `00 00 01 00` | **contradicted** |
| `0x20800a6c` | `psize 4, dlen 0` | `31 00 00 00` **and** `11 00 00 00` | **contradicted**, and an `[IN]` **echo** |
| `0x20800a4c` | `psize 4, dlen 0` | `00 00 00 00` | ⚠ coincides |
| `0x20800a70` | `psize 0, dlen 0` | `<empty>` | ⚠ coincides |

**Nine of eleven are wrong**, and the two that are not are the interesting ones.
`0x20800a70` has `psize = 0` — there is no body it could have failed to capture, and that is
checkable *from the row*. `0x20800a4c` genuinely answers zero because SMC is disabled on this
part — and **nothing about its row says so**; in the capture it is byte-identical to the nine
that are wrong.

⇒ *"the oracle answered it with an empty body"* is **not** evidence that a real GSP answers
zero. It is a capture artefact, and a triage row that cites one is citing nothing.
`0x20802a08`'s row did exactly that for four rungs. The rule is now a named refusal in
`kayfabe_abi::oracle`, keyed on `psize > 0 && dlen == 0` rather than on a list of ids, and
`crates/kayfabe-device/tests/sweep_triage.rs` fails any triage row that cites one of these
rows without naming what hardware said.

## ★★ And the other direction: a FULL row was corroborated, all 3712 bytes

`0x20800a2a` (`INTERNAL_STATIC_KGR_GET_INFO`) carries `dlen == psize == 3712` in the C's
table. Asked of the real part it answered **byte-identically**, twice in one run. That is the
class result stated the other way round and it is why this file does not demote the artifact:
rows the C actually captured are right; rows it recorded empty say nothing. See
`kayfabe_abi::grinfo`.

⊘ **Untouched: the TRUNCATED rows** — `0 < dlen < psize`, of which `0x20800a22` (16 376 of
34 592) is the largest. A third class, not measured here. The recipe now works for whole
bodies of any size, so the measurement is available; it has not been taken.

⊘ This does **not** demote the C generally: it remains the only implementation a real driver
has accepted end to end, and its non-empty bodies are corroborated here. It demotes one
specific move — reading an empty `ctl_` array as *"hardware says zero"*.

## ⊘ What these traces do NOT establish

- ⊘ **They are one part, one driver version, one boot.** Nothing here is a claim about AD10x,
  GH100, or a different driver.
- ⊘ **They are not a replay.** There is no ordering guarantee strong enough to diff against —
  they are an *answer sheet*, consulted per control, not a sequence.
- ⊘ **They cover the physical-control plane only.** Register accesses, the message queue, DMA
  and interrupts are all invisible to this probe.
- ⊘ **`gspst=0x56` means this part's own GSP refuses that control** — which is a fact worth
  having (`0x20800a87`, `0x20800b05`), but says nothing about whether *our* refusal is
  survivable in *our* guest.
