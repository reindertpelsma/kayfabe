# Repeated native Windows and Kayfabe boundary observations

**STATUS: RESEARCH, 2026-10-05.** Six completed fresh-overlay runs from one immutable
Windows fixture. Native Windows is healthy in all three runs; Kayfabe's L construction-only
configuration reports Code43 in all three. No native RPC coverage or successful Windows GPU
workload under Kayfabe is established by this cohort.

| Run | NVIDIA PnP problem code | NVIDIA-SMI exit | Status uptime, seconds | Available boundary evidence |
|---|---:|---:|---:|---|
| VFIO5 | 0 | 0 | 101.162 | 4096-byte capability snapshot |
| Kayfabe2 | 43 | 9 | 127.710 | 365 `rpc-trace` records; caps unavailable |
| VFIO6 | 0 | 0 | 90.322 | 4096-byte capability snapshot |
| Kayfabe3 | 43 | 9 | 99.260 | 365 `rpc-trace` records; caps unavailable |
| VFIO7 | 0 | 0 | 91.828 | 4096-byte capability snapshot |
| Kayfabe4 | 43 | 9 | 98.841 | 365 `rpc-trace` records; caps unavailable |

All six supervisors report `Result=success`, `ExecMainStatus=0`; post-run host checks report
RTX4070 / Linux driver595.91.07. That is successful harness shutdown/restoration, not successful
Kayfabe guest-driver initialization. Both arms retain an auxiliary basic display adapter that
reports problem code10; the table above selects the NVIDIA device specifically.

## Matched fixture and deliberate differences

Guest OS10.0.26100, NVIDIA580.88 / file version32.0.15.8088, RTX4070. Every run independently
records the same baseline, QEMU binary, initial UEFI variables and configured guest-driver file hashes:

| Input | Recorded identity |
|---|---|
| QEMU/product source | `b431aeaf9fca5d78b451754c0de7b8dbbe9d7652` |
| Immutable Windows disk SHA256 | `9ff194d31f0a871757fd436f14ab0da2e1e8a6360dc36bb925c12af12a64f85c` |
| QEMU executable SHA256 | `1918611a188fccad0e8568633a18d5667243133706afc32a17483c11e0f18cab` |
| Initial UEFI variables SHA256 | `4a09148a02665b072071523206a1c9b9c16624413dafb85106e3f8d202db74da` |
| Configured `nvlddmkm.sys` SHA256 | `31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c` |

The historical `command.json` key `firmware_sha256` actually hashes initial **OVMF_VARS.fd**;
it is relabeled accurately here. The controller separately measured the3653632-byte
OVMF_CODE_4M.fd during Kayfabe4 as
`a5708766c49ee39db0f4e7e53d73376e2dbc0d45bf12501c0977c48412bf8902`.
This later observation is not a per-run firmware-code measurement. No task changed the code
image, but these files alone cannot independently prove its prior contents.

Kayfabe runs all use L's explicit pool, timer, Translated-space, software-runlist, memory-list,
and TMO-constructor flags. VFIO runs have those flags absent. `KF3_RPC_TRACE=1` observes the
Kayfabe RM boundary. L refuses every display method before execution: this configuration cannot
provide a working display. Virtual memory apertures, VRAM configuration and device topology
intentionally differ between arms. No captured native capabilities were installed into Kayfabe.

Caps were requested after two stable status observations, with final uptime at least90seconds,
before shutdown. Kayfabe2 was collected manually using the same helpers after handling for its
unavailable BAR was fixed; this exception is preserved in its completion record. Other intended
runs used the series controller. Failed pilots and the earlier Kayfabe1 are excluded explicitly,
not silently treated as successful repetitions.

## Repeated observations

Native VFIO5/6/7 snapshots are byte-identical: **all1024 words stable, zero variable words**.
Each page SHA256 is
`7cc7a2692d92d155e3c50a0f9554671feffdc63e043925084239ff1bee8f0c55`.
The offline annotation used C773 names from the compiler-derived580.65.06 class table, SHA256
`0f434f1b996054f1832fd15087e26881d8565df75c641806a7d98cbc39cb5449`.
The full capability pages and full annotated table remain private.

In Kayfabe runs, PCI BAR decoding is unavailable at the settled Code43 milestone, so the
fixed capability read is unavailable in all three runs. The comparison contains **no Kayfabe capability snapshot**.
No zero-filled page or source-authored page was substituted. Native RPC capture was not enabled
for this series, so there is **no cross-arm RPC comparison** or claim that native Windows omits
any particular call.

Kayfabe2/3/4 each have365 trace records. All136 distinct function/selector/status rows have
identical counts; the shared normalized multiset SHA256 is
`d710d887ea59bc374e484965c53fa32c94ab2f6407ea782ef2466968eef14289`.
Function/status totals in every Kayfabe run are:

| Function | Logger result | Count per run |
|---:|---|---:|
| 1 | `0x0` | 1 |
| 4 | `0x0` | 2 |
| 10 | `0x0` | 60 |
| 47 | `0x0` | 1 |
| 65 | `0x0` | 1 |
| 70 | `0x0` | 2 |
| 76 | `0x0` | 177 |
| 76 | `none` | 61 |
| 76 | `0x56` | 2 |
| 103 | `0x0` | 58 |

`none` is the logger's missing policy reply, preserved separately from numeric statuses.
The record count describes this logger boundary; it is not a complete census of every possible
GSP message, register access or driver operation.

Kayfabe3 and4 have identical normalized order. Kayfabe2 differs only at observed indices173–176:

| Index | Kayfabe2 | Kayfabe3 and4 |
|---:|---|---|
| 173 | `00730245` / `0x0` | `0073011d` / `none` |
| 174 | `0073011d` / `none` | `00730245` / `0x0` |
| 175 | `0073012c` / `none` | `0073010b` / `0x0` |
| 176 | `0073010b` / `0x0` | `0073012c` / `none` |

Public OGKM580.65.06 names these respectively `SPECIFIC_GET_EDID_V2`,
`SYSTEM_GET_CONNECTOR_TABLE`, `SYSTEM_VRR_DISPLAY_INFO`, and `SYSTEM_GET_HEAD_ROUTING_MAP`
in `ctrl0073specific.h` and `ctrl0073system.h`. Status and count per command are unchanged.
This is a reproducible description of observed ordering variation. It is not classified as
noise, a synchronization bug, or the cause of Code43 without further evidence.

## Evidence and reproduction

[summary.json](summary.json) contains the sanitized per-run hashes, exact configuration flags,
status/count summaries and the local order variation. It excludes command lines, PCI/runtime
addresses, handles, raw payloads, raw logs and the full capability table. Private source files
are preserved by the controller; their exact hashes are in that summary.

The [offline comparator and annotation helper](../../tools/windows-boundary-compare/README.md)
produced private full JSON/Markdown reports. The private manifest contains exact source paths and
identity metadata; its SHA256 is recorded in the summary. Kayfabe inputs are complete saved
`qemu.log` files with `rpc-trace` parsing; native inputs contain caps only. No message pairing uses
handles or all-zero sequence numbers. Schema/data validation verified six clean completions,
matching recorded identities, real4096-byte native pages, explicit unavailable Kayfabe pages,
identical multiset counts, and absence of unsupported cross-arm comparisons.

These repetitions establish fixture-local observations. They do not establish compatibility
across driver versions/GPU dies, justify copying native bits into the product, or satisfy the
production hardware/isolation merge bar.
