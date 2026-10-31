STATUS: RESEARCH, 2026-10-05; provenance corrected 2026-10-31. Default-off allocation-only diagnostic; no production support claim.

> **Correction, 2026-10-31 (provenance fix).** The shipped cell (`crates/kf-abi/src/sw_runlist.rs`)
> is now a hand-maintained table in which every item has a source tag
> (`docs/design/V3_PROVENANCE.md`; `kf_abi::provenance`): the class `0xb297` and the `0x20801111`
> size 40 are `measured:` from committed traces, the parameter and RPC layouts are `ogkm-580.65.06:`
> from the public headers. Everything below that says the class, the registration, the control's
> code or size, or any address came from the pinned retail driver is **withdrawn**: the generator
> that read that driver (`derive.py`) is deleted, no value is read from a closed binary, and the
> two evidence links that used to follow this paragraph (a binary audit and a proprietary-driver
> comparison) are removed from the tree's claims. What replaced them is the table's evidence
> column, checked by `crates/kf-abi/tests/windows_cell_provenance.rs`. One thing the traces do not
> show is that the class's parameters are *exactly* the 12-byte public struct; the table records
> that as `params_binding`, backed only by the accepted allocation in run53 (see "Verify the cell").

# Software-runlist allocation probe

`KF3_SW_RUNLIST_PROBE=1` permits an **unscheduled guest metadata object** for the
one evidenced Windows 580.88 / public wire tag 580.65.06 cell. The
default capability table still denies the class. This isolates the known
StartDevice failure after a refused `0xb297` allocation; it does not implement
the native RunlistApi constructor or prove Windows will initialize successfully.

Evidence: the table's own evidence column (`traces/windows_pool_20261005/probe-b/qemu.log:1122`,
`traces/windows_code43_walls_20261007/run53-requests.log:196-197`, `run42-requests.log:71`).

## Explicit departures and missing behavior

The class number is seen on the wire, not an OGKM
`#define`. That departs from v3's source-generated class-set goal and is confined
to this diagnostic cell. The public SDK supplies the parameter type and the C
compiler supplies all offsets. No captured GPU dimensions, addresses, register
tables, family limits, or adjacent driver-version guesses are used.

The native constructor is **not just bookkeeping**: it is expected to convert an NV2080 engine type to its RM engine, resolve hardware
runlist properties, treat `maxTSGs=0` as hardware default capacity, allocate two
buffers and invoke the QoS-mask HAL [inferred from the parameter documentation in `nvos.h:2905-2923`; not measured]. The probe deliberately omits these effects.
Its successful reply means only that the guest now owns an unscheduled diagnostic
name. It neither reserves native runlist capacity nor completes GPU work.
The context-switch-timeout control `0x20801110` (`FIFO_CONFIG_CTXSW_TIMEOUT`),
the software-runlist scheduling control `0x20801111`, hardware buffer allocation,
memory references, interrupt changes, and scheduler submissions remain outside
the probe. Later failure on those operations is a useful result, not permission
to make them succeed.

Windows `RmAlloc`
does not carry a trustworthy kernel-client stamp; the client handle is not used
as one, and the existing Linux PID classification is not claimed as Windows
proof. Fn1's source-matched Windows identity selects the evidence cell only: a
hostile guest can lie about it, and every accepted operation is still bounded
guest metadata with no host verb. This does not authorize forwarding the
kernel-client-only scheduling control.

## Checks and lifetime

- Explicit environment opt-in and a successfully decoded matching fn1 Windows
  twin identity, branch/changelist and RPC version. Missing, Linux, changed,
  unsupported or malformed identities refuse; a new bad identity revokes the gate.
- Exactly the public three-`NvU32`, 12-byte nonserialized layout. Unknown parameter
  flags, nonzero `maxTSGs` or QoS flags refuse as unimplemented. Zero `maxTSGs` is
  recorded only as the request the Windows guest was seen to send; no empty-runlist claim.
- Engine membership in the same host-derived NV2080 engine capability bitmap
  advertised to the guest, with no die-specific constants. The engine namespace
  is the public SDK's (`NV2080_ENGINE_TYPE_*`); no closed driver is a source here
  and this is not an assertion that every GPU/driver constructor works.
- Live allocated Subdevice, its live same-client Device, and its client root.
  Aliases and recycled origin values are refused as parents. Device target must
  resolve. Guest handle zero/root/parent reuse and conflicting allocations refuse.
- Typed `SoftwareRunlistProbe` graph object holding only engine and parent resource
  identities. It uses existing graph quota, idempotent retries, incarnation,
  duplicate-reference and subtree/client-free machinery. There is no second handle
  registry and no host object. No addresses or payload pointers are stored.
- At most 16 decoded scalar request records per policy construction are logged
  under the opt-in, including requests whose engine or features are refused.

## Follow-up observations after the allocation experiment

The same opt-in also enables a **logging-only** observer for two exact-cell
requests. They share a separate 16-record cap per chain construction, which a
repeated fn1 identity declaration cannot reset. Missing or revoked identity
disables both. Every observation still returns no reply: normal downstream
policy and the FSM's unsupported-command refusal remain unchanged.

- `0x20801111`: exactly 40 declared parameter bytes, zero parameter flags, and an
  exact complete declared control body are required. Only those 40 bytes are
  printed, as ten raw little-endian words. Labels do not assign unproved field
  semantics. The code and size are the ones the guest was measured to send
  (`traces/windows_code43_walls_20261007/run42-requests.log:71`); they are diagnostic evidence,
  not control authorization. Transport padding is never inspected.
- Function 4, publicly named `ALLOC_MEMORY`: the function number comes from the
  existing generated driver matrix. The fixed `rpc_alloc_memory_v13_01` size and
  nine scalar prefix fields are compiled from the exact public source. A payload
  shorter than the fixed struct is not observed. The log states payload byte
  length and `prefix_only=true`; the PTE descriptor and variable tail are neither
  printed nor validated, and no pointer/address is followed. This is not a claim
  that the complete memory request is valid or implemented.

The ordering matters: a refused `ALLOC_MEMORY` before `0x20801111` could make the
latter cleanup rather than the initial failure. The observer supplies evidence
to resolve that question and does not assume the scheduling call is causal.

This follow-up passed 12 focused tests and both served-chain tests. They cover
short/extended bodies, serialization flags, delivered-padding exclusion, exact
word output, omission of the PTE tail, identity revocation, the combined cap,
unchanged requests, and refusal/no channel action in both opt-in states.

## Verify the cell (2026-10-31: replaces "Reproduce the generated cell")

The cell is hand-maintained in `crates/kf-abi/src/sw_runlist.rs`; no generator remains and no
closed binary is read. Two checks keep it honest:

- `cargo test -p kf-abi` runs `crates/kf-abi/tests/windows_cell_provenance.rs`: every item has a
  source tag of kind `ogkm-<version>:<ref>`, `measured:<DIE>,<driver>,<date>` or
  `host-derived:<ref>` (anything naming a binary, `re` or a `blob` is refused), and every trace
  line a `measured:` item cites exists and shows the value.
- `python3 tools/windows-runlist-probe/check_layout.py --ogkm /path/to/ogkm-580.65.06` compiles
  `layout.c` and `rpc-layout.c` against the public OGKM 580.65.06 checkout and fails if the table's
  parameter size, field offsets and `ALLOC_MEMORY` prefix differ.

The data cell is intentionally separate from both the
normal OGKM class matrix and the broad capability allowlist. Additional cells
require a class seen on the wire and layout evidence, not a range
extension or a GPU-die special case. An unknown cell fails closed.

GPU-free verification covers exact identity and its revocation, unsupported cells,
short/long/truncated/serialized inputs, engine and feature refusals, parent type,
cross-client and recycled aliases, retries/conflicts, freeing an object/parent/
device/client, graph quota exhaustion, default refusal, and refusal by object
backends without this explicit graph-only seam. Hardware validation belongs to
the root's serial Windows experiment; it has not been claimed by this change.

Validation on the controller: 1,245 tests passed across `kf-abi`, `kf-arch`, `kf-rm`
and `kf-qemu`; zero failures or ignored tests. The additional real served-chain
test passed both isolated environments: default/off refuses the allocation,
opt-in accepts it, neither timeout nor scheduling control is answered successfully, and no
channel action occurs. (That paragraph is from the generated-table version; on 2026-10-31
the values were re-pinned byte for byte by unit tests, see the correction at the top.)
