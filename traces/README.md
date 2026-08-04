# The C reference replay traces — the standing oracle

`docs/design/c_rust_trace_differential.md` §4: **recorded traces are the durable artifact; a
bootable C on a rented box is not.** So the artifact is committed here, uncompressed, and the
harness that reads it (`crates/kayfabe-crec`) needs no decoder, no external binary and no
third-party crate.

| file | records | md5 | properties |
|---|---|---|---|
| `cap1_coldboot_hermetic.rec` | 359 062 | `87c310e1afcf0ee44054d8462b117158` | `m2fwd=off m2exec=off m2romregs=off`, full mask — **HERMETIC** |
| `cap1b_coldboot_hermetic_d6.rec` | 360 725 | `e503b6f797c1109532f9e3aa05ace9d7` | identical vector, plus BAR0 trace — **HERMETIC**, and **GSP-D6 witnessed** |

## ★★★ Why there are two, and why the second one is the one a reply plane is differenced on

`cap1`'s replay closes at **txn 978** and the reason is a hole in the *recorder*, not in the
protocol: the C acts on element 0 of a multi-element command and advances its read pointer
past the continuations **without reading them** (GSP-D6), so the capture holds no observation
of them while they were live and no named assumption reconstructs one.

`cap1b` is the **same experiment**, re-captured at the C's `819282d`, where
`nvkvm_m3_service_cmdq` reads the continuation slots through the recorder chokepoint and throws
the bytes away. The defect is unchanged — the C still acts on element 0 alone, and the reply
stream is proven byte-unchanged against a same-binary control on every channel — it is merely
**witnessed**. Measured, by shape, inside each file
(`crates/kayfabe-crec/tests/cap1b_differential.rs`):

| | `cap1` | `cap1b` |
|---|---|---|
| multi-element commands | 5 | 9 |
| continuation elements owed | 24 | 32 |
| continuation elements **witnessed** | **0** | **32** |
| replay closure limit | txn 978 (GSP-D6, oracle blindness) | txn **1028** (GSP-D2, our own flow control) |
| served controls the replay reaches | 1 of 5 | **5 of 5** |

★★ That last row is why `cap1b` had to be brought in before any protocol fix was landed.
Device-info, the interrupt kernel table, the PCI-BAR table and the user-register access map all
arrive **after** `cap1`'s wall, so four of the five served controls had no reply-plane
differential coverage at all — a defect in any of them could not have turned a test red.

⊘ `cap1b` is **not** a superset of `cap1`. It is driven by a script rather than by hand, so its
`nvidia-smi -q` is not SIGPIPE-truncated and it carries more RPC work after `GSP_INIT_DONE`
(859 `GuestWrite` vs 563). The bring-up prefix is identical, which is what a boot differential
needs — and it is why a raw read-count delta between the two captures means nothing.

★ `cap1` is kept rather than replaced: the two together are the audit trail for GSP-D6, and
`cap1_differential.rs` remains the transport proof against the C's own `EchoOk` baseline.

## Provenance

Recorded 2026-07-29 from the C Mode-2 emulator (`nvkvm-gpu-emul`) on real hardware:
vast.ai box, RTX 3060 = **GA106**, host driver **580.159.04 open**, host kernel 6.8.0-59,
QEMU 9.2.0; guest Ubuntu 24.04, kernel 6.8.0-117-generic, **stock unpatched** open NVIDIA
580.159.04, VBIOS `ga106_vbios.rom` md5 `48df40a04432aca6a35bee2785857eba`. Emulator source
md5 `cced661c16f6856801d16dae151bc2f0`, recorder md5 `d2ab3a95291396c0dce81e422a68e73a`.
`cap1b` was captured at emulator commit **`819282d`**, `nvkvm_gpu_emul.c` md5
`2132bbdbf98ab85449e9513c9c230bbf`, recorder md5 unchanged — and, unlike the first four
captures, its header carries that source revision, because the first four were taken from a
tree where `git rev-parse` silently yielded nothing.
The whole provenance block is *inside the file* (`CHeader::provenance`) — an oracle whose
provenance is not in the artifact stops being an oracle the moment the bench dies.

Format: `nvkvm_m2_rec.h` in the C repo. Reference decoder: `scripts/mode2_diag/rec_dump.py`
there. The Rust decoder is `kayfabe_crec::format`, and it is **cross-validated against that
reference decoder** by `crates/kayfabe-crec/tests/decoder_matches_reference.rs`, which pins
the exact per-kind census `rec_dump.py` prints — the instrument is checked before a single
divergence is believed.

## Why only these two

Three other captures exist in the C repo
(`traces/mode2_c_reference/`: `cap2_stalequeue_negative`, `cap2b_stalequeue_nofn47`,
`cap3_matmul_forwarding`). **They are non-hermetic by construction**: with `m2fwd=on` the stub
`MAP_FIXED`s guest RAM into itself and the host GPU DMAs into it directly, so guest-visible
bytes pass through no recorder at all. A replay cannot be *closed* over them. `cap1`/`cap1b`
are the ones that can, and they are the ones committed here.

## What these captures cannot witness

Stated before any result, so a green diff is never mistaken for coverage — see
`docs/design/c_rust_trace_differential.md` §5a and `crates/kayfabe-crec/src/lib.rs`.

---

# `rpctrace_ga106_boot1.bin` — a GSP-RM RPC trace of a boot that **SUCCEEDS**

★★★ Everything above this line comes from *our* emulator. This file does not: it was recorded
**inside CPU-RM** on a real GA106, by a recorder patched into the open driver's own message
queue, during a boot that ends with a working `nvidia-smi`. `cap1` is a trace of a boot that
fails — it stops where the emulator stopped. This one carries the sequence *and the answers a
real GSP gave* past that point.

Built and captured by `scripts/rpctrace/`; full write-up in `docs/design/rpc_trace_capture.md`
§6. Format: `scripts/rpctrace/nv_rpctrace.h`. Decoder: `scripts/rpctrace/decode_rpctrace.py`,
which **refuses** a wrapped or truncated file rather than replaying a hole.

## Provenance

| | |
|---|---|
| part | NVIDIA GeForce RTX 3060 (GA106), `GPU-e28d7776-e4f9-704b-d392-d46f187343f8` |
| host | vast.ai instance `46494693`, kernel `6.8.0-59-generic` |
| driver | **NVIDIA open kernel modules 580.159.04**, rebuilt from source with the recorder |
| source | `research_clones/ogkm-580.159.04`, git tag `b81d58e` (`version.mk: 580.159.04`) |
| module | `nvidia.ko` sha256 `6e81064b5464b581d31fce99ac82ba8c974fc873d7cca9650a52e701568724ab` |
| date | 2026-08-03 |
| method | `capture.sh --tag boot1`: stock stack unloaded, instrumented module `insmod`ed **by path**, `nvidia-smi` + `nvidia-smi -q`, `/proc/driver/nvidia/rpctrace` drained, stock module restored **and the restore verified** |

The bench's stock module was never modified on disk. `traces/rpctrace_ga106_boot1_dmesg.log` is
the driver's own output for this capture, persisted deliberately — the serial/console log is
*not* where it lives, and a harness that writes an empty file reads as capture.

## The numbers

| | |
|---|---|
| file | 1 229 472 bytes, md5 `0fcc24c7074df68a585868b75326f329` |
| records | **1 076** (535 CPU→GSP, 541 GSP→CPU) |
| payload bytes | 1 176 776 |
| largest single element | **65 536** — `GSP_MSG_QUEUE_ELEMENT_SIZE_MAX` exactly |
| **wrapped?** | **no**; 1.17 MiB of a 64 MiB ring, `n_dropped = 0` |
| refused-empty / rx failures | 0 / 0 |
| sessions | **2** — one per `nvidia-smi`; RM tears the GPU down when the last client closes |
| retransmits within a session | 0 |
| distinct RPC functions | 14 |
| distinct `GSP_RM_CONTROL` commands | **104**, 310 request/reply pairs |

## ★★★ Why this file exists at all

`decode_rpctrace.py --controls` reports **replies declaring params with no bytes present: 0**.
The oracle it replaces — the C artifact's `mode2_initctrl_ga106.h` — has 11 rows of 56 that
declare a length and carry nothing, and every one checked against hardware is *contradicted*.
All six of those are in this file with bodies, and the headline row reproduces an independent
2026-08-01 measurement exactly: `0x20802a08` (`CE_GET_FAULT_METHOD_BUFFER_SIZE`) answers
**20480**, where the empty row decodes to 0 and RM DMAs into a buffer of exactly that size.

★ A genuinely zero-length reply is now distinguishable from an unmeasured one: `0x20800a70`
answers `paramsSize = 0` with `NV_OK`, and that is a *measurement*.

## The decoder is checked against a different instrument

`traces/real_ga106/rpc_transcript_real_ga106.txt` is an independent `NV_PRINTF` probe of the
same GPU from 2026-08-01, printing `cmd`/`psize`/`gspst` for 88 control calls. Decoding this
capture's `GSP_RM_CONTROL` replies and comparing: **88 of 88 agree on both `paramsSize` and GSP
status, 0 disagree, 0 absent.** A decoder that mis-located a field would produce a
self-consistently wrong table; this is the check that the offsets are right.

## ⊘ What it does not witness

One part, one kernel, one driver — GA10x only, **open** driver only. Two well-behaved
`nvidia-smi` boots: no CUDA context, no compute, no refusal, no reorder. And it does not
classify data-vs-act: `0x20800a6c` answers 17 on some calls and 49 on others, `0xa06f0103`
answers 3 bytes, and both look exactly like data here. Serving an act from a table fails late.

---

# `ga102_boot1.bin` — the second die (GA102, RTX 3090, 575.51.03)

Taken 2026-08-03 with the **same recorder**, re-anchored for 575 (`rpctrace-575.51.03.patch`;
`nv_rpctrace.{c,h}` byte-identical). **1 152 928 bytes**, md5
`6bc25a2e80858c2abaa7c7bbb50ca2c8`, **1 180 records**, **724** `GSP_RM_CONTROL` elements,
**122** distinct control commands, **ring did not wrap** (1.10 MiB of 64 MiB), dropped /
refused-empty / rx-failed = **0 / 0 / 0**, and **replies declaring params with no bytes = 0**.
The boot succeeds — `nvidia-smi` reports the RTX 3090 — and the stock (proprietary) module was
restored and verified afterwards.

⚠ **It is not a clean architecture comparison against `rpctrace_ga106_boot1.bin`**: that one is
580.159.04 and this one is 575.51.03, so the raw diff is arch ∧ driver-version. The three
groups are attributed separately in `docs/design/rpc_trace_capture.md` §7.2 — the 2 only-GA106
controls and all 11 reply-size differences are **version**; the 20 only-GA102 controls are a
**capability** difference, 17 of them NVLink.

★ The one row needing no cross-version argument, because both boards issue it:
`0x20800a87` (`INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO`) is answered `NV_ERR_NOT_SUPPORTED` on
the GA106 and `NV_OK` on the GA102, and the 17 NVLink controls follow only on the GA102. The
sequence branches on a **reply**, not on a part number (§7.3).

---

# `ad102_boot1.bin` — the second architecture (AD102, RTX 4090, 575.51.03)

Taken 2026-08-03 with the same recorder and **`rpctrace-575.51.03.patch` unchanged**.
**1 140 256 bytes**, md5 `751840ae979327bf63f8833036f56507`, **1 112 records**, **656**
`GSP_RM_CONTROL` elements, **108** distinct control commands, **ring did not wrap** (1.09 MiB of
64 MiB), dropped / refused-empty / rx-failed = **0 / 0 / 0**, **replies declaring params with no
bytes = 0**, 9 controls refused by a real GSP. Boot succeeds; stock (proprietary) module
restored and verified.

★★★ **This is the trace that makes a clean architecture comparison possible.** It runs the
**same driver and kernel as `ga102_boot1.bin`**, so AD102 ↔ GA102 varies only the architecture.
Result (`docs/design/rpc_trace_capture.md` §8): **105 common controls, and 0 of them differ in
reply size** — confirming that the 11 size differences in the GA106 ↔ GA102 comparison were
driver-version drift, not silicon. The whole observed architecture difference is two
capabilities: **NVLink** (17 controls, only the 3090) and **ECC** (6 controls, only the 4090),
each gated on a probe the GPU answers.

★ `0x20800a87` (`INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO`) answers `NV_ERR_NOT_SUPPORTED` on the
4090 — like the 3060, unlike the 3090. Three boards, two architectures, two driver versions, and
the only variable predicting the answer is whether the board has the connector.
