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
