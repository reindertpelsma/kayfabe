# The C reference replay traces — the standing oracle

`docs/design/c_rust_trace_differential.md` §4: **recorded traces are the durable artifact; a
bootable C on a rented box is not.** So the artifact is committed here, uncompressed, and the
harness that reads it (`crates/kayfabe-crec`) needs no decoder, no external binary and no
third-party crate.

| file | records | md5 | properties |
|---|---|---|---|
| `cap1_coldboot_hermetic.rec` | 359 062 | `87c310e1afcf0ee44054d8462b117158` | `m2fwd=off m2exec=off m2romregs=off`, full mask — **HERMETIC** |

## Provenance

Recorded 2026-07-29 from the C Mode-2 emulator (`nvkvm-gpu-emul`) on real hardware:
vast.ai box, RTX 3060 = **GA106**, host driver **580.159.04 open**, host kernel 6.8.0-59,
QEMU 9.2.0; guest Ubuntu 24.04, kernel 6.8.0-117-generic, **stock unpatched** open NVIDIA
580.159.04, VBIOS `ga106_vbios.rom` md5 `48df40a04432aca6a35bee2785857eba`. Emulator source
md5 `cced661c16f6856801d16dae151bc2f0`, recorder md5 `d2ab3a95291396c0dce81e422a68e73a`.
The whole provenance block is *inside the file* (`CHeader::provenance`) — an oracle whose
provenance is not in the artifact stops being an oracle the moment the bench dies.

Format: `nvkvm_m2_rec.h` in the C repo. Reference decoder: `scripts/mode2_diag/rec_dump.py`
there. The Rust decoder is `kayfabe_crec::format`, and it is **cross-validated against that
reference decoder** by `crates/kayfabe-crec/tests/decoder_matches_reference.rs`, which pins
the exact per-kind census `rec_dump.py` prints — the instrument is checked before a single
divergence is believed.

## Why only cap1

Three other captures exist in the C repo
(`traces/mode2_c_reference/`: `cap2_stalequeue_negative`, `cap2b_stalequeue_nofn47`,
`cap3_matmul_forwarding`). **They are non-hermetic by construction**: with `m2fwd=on` the stub
`MAP_FIXED`s guest RAM into itself and the host GPU DMAs into it directly, so guest-visible
bytes pass through no recorder at all. A replay cannot be *closed* over them. cap1 is the one
that can, and it is the one committed here.

## What cap1 cannot witness

Stated before any result, so a green diff is never mistaken for coverage — see
`docs/design/c_rust_trace_differential.md` §5a and `crates/kayfabe-crec/src/lib.rs`.
