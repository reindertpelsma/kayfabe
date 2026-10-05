# Repeated early native GSP boundary observations

**STATUS: RESEARCH, 2026-10-05.** Three healthy native Windows/VFIO recordings compared with
four Windows/Kayfabe L diagnostic runs. No product changes, new allowlist entries, copied native
capability values or claim of complete native coverage.

The native recordings now include the early pool-size query that the old observer missed.
Every recording contains one unambiguous successful `GR_GFX_POOL_QUERY_SIZE` request/reply pair
under the existing pinned observer decoder's matching rules. This validates an observed source
contract for this fixture; it supplies no constants to the implementation and establishes no
cross-driver or cross-GPU guarantee. Raw payloads remain private.

## Same-executable repetitions completed

Kayfabe6 and7 subsequently completed with the same immutable `a8845e69` QEMU
executable, disk, firmware, driver and flags as Kayfabe5 and native8/9/10.
All three Kayfabe runs report Code43 / nvidia-smi exit9 and have **identical
365-record normalized RPC order**, including every function, selector and result.
Host restoration and supervisor success are recorded for each. Their hashes and
fixture details are in [same-executable.json](same-executable.json). Thus this
cohort now has three boots per arm using the same executable; the earlier K2/3/4
binary distinction remains historical evidence. Native sampling limits and the
lack of parameter/object equivalence still apply. The table below retains the
original cohort; this addition does not silently regenerate its summary.

## Recorded results

| Run | NVIDIA PnP / nvidia-smi | Records | Attachments | Gaps / drops / unstable samples |
|---|---|---:|---|---|
| VFIO8 | 0 / 0 | 8226 | 1:2, 2:8224 | 0 / 0 / 15 |
| VFIO9 | 0 / 0 | 8195 | 1:2, 2:8193 | 0 / 0 / 8 |
| VFIO10 | 0 / 0 | 8208 | 1:2, 2:8206 | 0 / 0 / 7 |
| Kayfabe2 | 43 / 9 | 365 | n/a | logger boundary only |
| Kayfabe3 | 43 / 9 | 365 | n/a | logger boundary only |
| Kayfabe4 | 43 / 9 | 365 | n/a | logger boundary only |
| Kayfabe5 | 43 / 9 | 365 | n/a | logger boundary only |

In each native recording, attachment 1 contains functions 72/73 (`GSP_SET_SYSTEM_INFO` and
`SET_REGISTRY`). Attachment 2 includes that prefix again. Both are retained. These are observer
attachment epochs, not evidence of two separate boots. Sequence zero, no observed gaps and no
FIFO drops still do not prove completeness: sampling, unstable snapshots and unknown history
remain explicit. `invalid_elements` includes repeated inspection of empty/stale queue slots;
it is not a count of malformed submitted RPCs.

Source-defined unsolicited events account for 43/22/39 status-queue observations in VFIO8/9/10.
RPC status-queue observations account for 4089/4084/4082; attachment-2 requests are
4092/4087/4085. The first 2628 normalized attachment-2 requests agree across all three runs,
as do the first 2626 normalized RPC status-queue observations. This excludes payload/handle
identity and is not a proof of equivalent inputs. Later ordering/count variation is retained;
the 90-second native recording includes post-initialization display/userspace activity.

Kayfabe's four runs have identical function/selector/status counts. K3, K4 and K5 have identical
normalized order. K2 differs only in the four display-query positions documented in the
[earlier six-run baseline](../README.md). None of these logger sequences identifies a native
message by matching sequence numbers or handles.

## What the status differences do and do not establish

Kayfabe observes 112 distinct control IDs. Of these, 47 have at least one declined or nonzero
Kayfabe result. All 47 IDs appear on the native status queue in all three native recordings:

- 41 have native control success in every recording. Their inputs, target objects, timing and
  intended virtual behavior are not established equal by a numeric ID match.
- Six have only nonzero native statuses too: `20800a87` (NVLINK device info), `20800b05` (SM issue
  throttle), `20800173` (GPU function status), `2080012f` (ECC status), `00730285` (ACPI display-port
  attachment), and unresolved-in-this-source `2080a02f`. The first four and last use native
  `NOT_SUPPORTED`; the ACPI query has both `NOT_SUPPORTED` and `INVALID_ARGUMENT` observations.
- Among these 47 IDs, native status/count observations repeat exactly except
  `20802068` (`PERF_GET_CURRENT_PSTATE`), which succeeds 2/1/1 times. Every other native count and
  status for this subset repeats. This does not prove those calls are the cause of Code43.

[controls.md](controls.md) contains the named status table. [summary.json](summary.json) retains
all compared command/status counts, per-run values, hashes, sampling statistics and binary
identities. Native **inner control status** is distinct from outer RPC status. Kayfabe
`result=none` stays null; it is not silently converted to `0x56`. Native inner-control failure
totals are identical: 19 `NOT_SUPPORTED`, 12 `INVALID_ARGUMENT`, two `GENERIC` per recording.

Source associations help interpret, rather than automatically implement, the differences:

| Observation | Source-grounded implication / remaining limit |
|---|---|
| Connector, VRR, DisplayPort, hotplug and mux queries differ | These query a physical display topology. Native topology is not the authored virtual topology; successful native calls do not justify copying native connector/capability answers. The constructor-only L diagnostic intentionally refuses display-channel methods. |
| `20801303` FB info and `20801823` BUS info have a Kayfabe numeric refusal | [inittables.rs](../../../crates/kf-rm/src/inittables.rs) already handles source-derived subsets of these indexed queries and explicitly refuses unknown indices. A command-level match does not establish which requested index differs; this is not evidence that the whole control is unimplemented. |
| `2080012b` GPU context promotion succeeds natively | The [owner ruling](../../../docs/OWNER_RULINGS.md) permits guest-kernel promotion bookkeeping when the host twin owns the real context. This capture neither establishes the required object invariants nor authorizes forwarding the native privileged action. |
| `20801111` scheduling succeeds natively, declines in Kayfabe | The [separate scheduler audit](https://github.com/reindertpelsma/kayfabe/blob/4dfa6895/tools/windows-debug-capture/evidence/runlist-20261005/scheduling-control.md) establishes real scheduling effects. Kayfabe's first occurrence is at logger position 331, late in the short initialization/free sequence; this census alone cannot establish whether the refusal causes or follows failure. It remains refused. |
| 37 of the 47 declined/nonzero IDs have public 580.65.06 names | Ten remain unnamed by this compiler vocabulary. `20801110` has a name in a later public driver, but this report does not silently substitute that driver's definitions. Source names alone do not establish runtime user callability or safe forwarding. |

No UMD methods or passthrough-channel work are proposed for VMM implementation. The previously
identified per-window constructor failure remains the focused lead; these repeated controls do
not supersede its pinned static analysis.

## Fixture and provenance

All runs use the same immutable disk, Windows 580.88 / file version `32.0.15.8088`, RTX 4070 and
initial UEFI variable image. Exact shared hashes and per-input hashes are in `summary.json`.
NVIDIA driver SHA256 is `31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c`.
VFIO8/9/10 and K5 use QEMU executable SHA256
`62cd3c15cfc8f46936409b92e7d7a9c8f33557bf715a87b95dd028c2a3aba250`, artifact revision
`a8845e69847b969d01308a64f13e00394ec76e1d`, with the L product revision recorded as
`b431aeaf9fca5d78b451754c0de7b8dbbe9d7652`. K2/K3/K4 use the earlier QEMU executable hash
`1918611a188fccad0e8568633a18d5667243133706afc32a17483c11e0f18cab` under the same L label;
they are repetitions of behavior, not falsely declared identical executable fixtures.

K5 is L, not the later ILUT M experiment: `KF3_DISPLAY_ILUT_CONSTRUCTOR_PROBE` is explicitly
unset. L's pool/timer/runlist/memory-list/TMO constructor flags remain explicit in the report.
Native flags are unset. The native observer changes instrumentation, and native/virtual VRAM
sizes and topology intentionally differ. There is no claim that every stable difference is wrong.

The native capability-page hashes are all identical to the earlier native baseline. Kayfabe
capability snapshots are unavailable after Code43 disables BAR decoding; no synthetic zero page
or full capability table is published. Firmware code is recorded per run for V8/V9/V10/K5;
earlier K2/K3/K4 lack per-run code measurements, as the earlier baseline explains.

Source vocabulary is compiled from the clean public OGKM `580.65.06` commit
`307159f2623d3bf45feb9177bd2da52ffbc5ddf9` using
[source.spec](../../../tools/windows-boundary-compare/source.spec) and the existing
[compiler/DWARF tool](../../../tools/drivermatrix/dm.py). GCC is Ubuntu 15.2.0-16ubuntu1.
`values.tsv` has 2974 compiler values and SHA256
`033f12f15c8eb6edabc9979f85d2a84c1c43b68cb2417b6dcf054caaba95a9bc`; `missing.tsv` has 177
non-integer macro entries (including bit ranges), SHA256
`321f5218f91060b9cd8256c60023dfefd5ddc304cd81801ba13cb8f75d12f666`. These are vocabulary
measurements, not per-die tables. The observer decoder SHA256 is
`0365971eef425fb146278c1986e71b944d8a6fcd235c0f73edef3d7a833affd5` at source `0e53ab595c0c86867652b743259a1ef3e3ad2d55`.

Reproduction uses the [cohort utility instructions](../../../tools/windows-boundary-compare/README.md)
with private decoded inputs from runs `boundary-vfio-{8,9,10}` and saved Kayfabe logs
`boundary-kayfabe-{2,3,4,5}`. The report manifest records sanitized fixture/status metadata,
coverage notes, full input hashes, and the source vocabulary. Public output omits raw messages,
handles, addresses, timestamps/QPCs, payloads and capability words. The original private manifest
hash is retained, while private filesystem paths are omitted.
