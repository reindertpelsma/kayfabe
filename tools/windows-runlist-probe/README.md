STATUS: RESEARCH, 2026-10-17. Default-off allocation-only diagnostic; Windows 581 hardware acceptance remains unrun for this change.

# Software-runlist allocation probe

`KF3_SW_RUNLIST_PROBE=1` permits an **unscheduled guest metadata object** for the
legacy Windows 580.88 / public wire tag 580.65.06 registration cell, and an
allocation-only Windows 581.51 / public wire tag 580.94.02 cell restricted to the
measured AD102 host die. The
default capability table still denies the class. This isolates the known
StartDevice failure after a refused `0xb297` allocation; it does not implement
the native RunlistApi constructor or prove Windows will initialize successfully.

Evidence: the [pinned Windows allocation and successful-use audit](https://github.com/reindertpelsma/kayfabe/blob/ff820182/tools/windows-debug-capture/evidence/2026-10-05-nvcd/b297-usage.md)
and the [independent proprietary Linux/source comparison](https://github.com/reindertpelsma/kayfabe/blob/8223efc9/tools/windows-debug-capture/evidence/runlist-20261005/README.md).

## Windows 581.51: native measurement and public layout

The original restriction to Windows 580.88 caused the first new allocation refusal
in the AD102 Windows 581.51 guest (`first_seq=216`, class `0xb297`). The same physical
AD102 running native Windows 581.51 passed CUDA, hardware D3D11/D3D12 and a 420.59 s
hold with zero System-4101 TDRs. Two independently checked native GSP metadata
streams each contain four allocations of this class with 12-byte declared parameters
and four matching successful replies. See the pinned [native reference](https://github.com/reindertpelsma/kayfabe/blob/8d2cb7cf9/traces/die_driver_sweep_20261015/ad102_second_builds_20261016/native_windows58151/RESULT.md).

`derive_native.py` compiles `NV_SWRUNLIST_ALLOCATION_PARAMS` from NVIDIA's public
580.94.02 `nvos.h` at commit `a86526f4e235ae3d9e2842a11a3bb11db77450d8`:
12 bytes, `engineId` at 0, `maxTSGs` at 4 and `qosIntrEnableMask` at 8.
It verifies both published metadata digests, declared sizes and unique successful
request/reply pairs. Its class binding is tagged
`measured:AD102,Windows581.51,2026-10-17`. The existing legacy generator and artifact
are unchanged; generating this new artifact never reads a closed executable.

Tier reasons: the public header defines the parameter type but contains no class
ID for it. The unprivileged Linux host's engine/class controls do not establish the
Windows allocation-class binding. The next permitted source is our privileged
native measurement. Only the class ID and declared size are measured here; their
association with the public software-runlist type remains the explicit diagnostic
hypothesis. Native parameter values, parent registration, constructor effects and
new-build scheduling semantics are **unverified**. The diagnostic retains the
existing conservative checks: zero parameter flags, zero capacity/QoS request,
engine membership in the host-advertised bitmap, and live same-client parents.
It does not represent a native constructor.

The measurement selects only host architecture `0x190`, implementation `2` (AD102),
exact public version `580.94.02`, and a matching Windows `581.51` fn1 identity.
Adjacent dies/builds refuse. The selection uses host `MC_GET_ARCH_INFO`, not the
untrusted guest's chip declaration. This extends neither normal class capabilities
nor the scheduling registration returned by `sw_runlist::cell`: the host-owned
policy still refuses `0x20801111` for the new build. Although native metadata shows
that control succeeds with 40 bytes, its reply body semantics are **NOT DERIVABLE**
from the retained metadata/public headers. No copied neighboring-build echo is added.

Hypothesis, before the CPU regression: admitting this exact bounded allocation
removes the first unsupported-cell refusal. CPU falsifier: a valid exact identity
and public layout still refuse, or admission also authorizes scheduling. The test
fails before the change with the measured unsupported-cell reason, then passes.
Hardware falsifier: the next cold guest boot succeeds at the allocation but still
aborts at the same point. **Hardware remains UNRUN for this change**; the prior
Windows 581.51 guest result remains **Code 43**, and no family completion is claimed.

Reproduce using the clean public tag and the two published TSVs from the reference:

```sh
python3 tools/windows-runlist-probe/derive_native.py \
  --ogkm /path/to/clean/ogkm-580.94.02 \
  --trace-dir /path/to/native_windows58151 \
  --out crates/kf-abi/src/sw_runlist_native_generated.rs
git diff --exit-code -- crates/kf-abi/src/sw_runlist_native_generated.rs
cargo test -p kf-abi -p kf-rm
```

Validation of this topic: `cargo test -p kf-abi -p kf-rm` passes **1,382 tests,
zero failures and zero ignored**. The three new tests cover allocation/default
refusal and absent submission support, other-die/adjacent-build refusals, identity
revocation, flags/length/capacity/QoS/engine/handle checks, physically delivered
parameter bounds and the 16-record log cap. Regeneration is byte-identical;
modified metadata, a size mismatch and the wrong public source tag reject.

The sections below describe the legacy Windows 580.88 experiment and its original
source/reproduction procedure. Its scheduling observations do not extend to 581.51.

## Explicit departures and missing behavior

The class number comes from a hash-pinned retail registration, not an OGKM
`#define`. That departs from v3's source-generated class-set goal and is confined
to this diagnostic cell. The public SDK supplies the parameter type and the C
compiler supplies all offsets. No captured GPU dimensions, addresses, register
tables, family limits, or adjacent driver-version guesses are used.

The native constructor is **not just bookkeeping**: proprietary Linux 610's
constructor converts an NV2080 engine type to its RM engine, resolves hardware
runlist properties, treats `maxTSGs=0` as hardware default capacity, allocates two
buffers and invokes the QoS-mask HAL. The probe deliberately omits these effects.
Its successful reply means only that the guest now owns an unscheduled diagnostic
name. It neither reserves native runlist capacity nor completes GPU work.
The context-switch-timeout control `0x20801110` (`FIFO_CONFIG_CTXSW_TIMEOUT`),
the software-runlist scheduling control `0x20801111`, hardware buffer allocation,
memory references, interrupt changes, and scheduler submissions remain outside
the probe. Later failure on those operations is a useful result, not permission
to make them succeed.

The allocation itself has NON_PRIVILEGED retail registration. Windows `RmAlloc`
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
  recorded only as the tested default-capacity request; no empty-runlist claim.
- Engine membership in the same host-derived NV2080 engine capability bitmap
  advertised to the guest, with no die-specific constants. The engine namespace
  is independently supported by the public SDK and proprietary Linux constructor
  conversion; this is not an assertion that every GPU/driver constructor works.
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
  semantics. The code and size are extracted from the pinned Windows caller's
  instructions at RVAs `0x19c45ec` and `0x19c45e4`; they are diagnostic evidence,
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

## Reproduce the generated cell

Use a trusted official Windows 580.88 package locally. No retail executable is
executed, copied from a rental, or committed here. The generator checks the exact
PE SHA256, external/internal registration IDs, Subdevice parent list, and retail
parameter size; it compiles the public OGKM 580.65.06 `nvos.h` layout independently.

```sh
python3 tools/windows-runlist-probe/derive.py \
  --ogkm /path/to/clean/ogkm-580.65.06 \
  --driver /path/to/580.88/Display.Driver/nvlddmkm.sys \
  --out crates/kf-abi/src/sw_runlist_generated.rs
git diff --exit-code -- crates/kf-abi/src/sw_runlist_generated.rs
```

The generated file contains the exact public source commit, retail digest and
registration-row digest. The data cell is intentionally separate from both the
normal OGKM class matrix and the broad capability allowlist. Additional cells
require their own audited registration and constructor evidence, not a range
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
channel action occurs. The generated cell reproduced byte for byte.
