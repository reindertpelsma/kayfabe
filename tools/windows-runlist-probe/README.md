STATUS: RESEARCH, 2026-10-05. Default-off allocation-only diagnostic; no production support claim.

# Software-runlist allocation probe

`KF3_SW_RUNLIST_PROBE=1` permits an **unscheduled guest metadata object** for the
one evidenced Windows 580.88 / public wire tag 580.65.06 registration cell. The
default capability table still denies the class. This isolates the known
StartDevice failure after a refused `0xb297` allocation; it does not implement
the native RunlistApi constructor or prove Windows will initialize successfully.

Evidence: the [pinned Windows allocation and successful-use audit](https://github.com/reindertpelsma/kayfabe/blob/ff820182/tools/windows-debug-capture/evidence/2026-10-05-nvcd/b297-usage.md)
and the [independent proprietary Linux/source comparison](https://github.com/reindertpelsma/kayfabe/blob/8223efc9/tools/windows-debug-capture/evidence/runlist-20261005/README.md).

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
