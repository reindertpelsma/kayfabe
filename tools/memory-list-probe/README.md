# Checked guest SYSRAM registration experiment

**STATUS: RESEARCH, 2026-10-05.** Default off, isolated diagnostic branch. This
implements the bounded descriptor contract below; it does not implement software
runlist scheduling or establish Windows boot success.

**Extension, 2026-10-07:** the same opt-in registration path now additionally
supports source-audited contiguous pitch-linear FBMEM on guest contracts580.65.06
and580.159.04. The earlier SYSRAM-only statements below describe the original
subset; FBMEM uses a separate backing tag and immutable VM framebuffer layout,
not RAM topology/CPU-copy authority. [Implementation and validation](../../traces/windows_code43_walls_20261007/README.md).
Normal RM_ALLOC capabilities remain unchanged; no GPU work or completion is
introduced by memory-object registration.

`KF3_MEMORY_LIST_PROBE=1` independently enables function `ALLOC_MEMORY` only for
named public-source guest contracts. The request declares an existing guest RAM
range; Kayfabe creates no host GPU allocation, executes no GPU work, changes no
host memory mappings, and touches no contents at registration. Subsequent
scheduling `0x20801111` and timeout configuration `0x20801110` remain refused.
Normal `RM_ALLOC` does not gain a MemoryList allocation capability.

## Source and compatibility contract

The [source audit](../windows-debug-capture/evidence/alloc-memory-20261005/README.md)
contains the public caller/constructor analysis, compiler emitter, reproducible
command, tag commits, and measured ABI/flag/bitfield facts. The
[renderer](generate.py) consumes those compiler facts and explicitly selects the
behaviorally audited compressed-contiguous rows: 565.57.01, 570.86.15,
580.65.06, 580.126.09, 580.159.04, 580.173.02, 595.91.07, 610.43.02 and 615.71.09.
Normal driver admission is still required. Every other guest contract refuses;
there is no nearest-version fallback. Older rows sharing this wire layout use
different page-array semantics and are deliberately excluded.

This descriptor path is independent of host driver and GPU family: it invokes no
host RM operation and contains no captured die table or per-family geometry.
Family support remains subject to the normal realization/admission checks. The
same decoder/authority checks apply across Turing, Ampere, Ada, Hopper and
Blackwell. Source coverage is not a claim of hardware testing every combination.
The observation that led here used Windows580.88; no Windows identity gate or
Windows-specific values determine descriptor acceptance.

Accepted requests have the source-defined SYSTEM MemoryList class, one inline
PFN, no indirection, reserved descriptor bits zero, an exact complete wire body,
positive logical length, and a first-page adjustment below the source page size.
Alignment padding is ignored. Format must be the source-defined pitch-linear
hardware PTE kind; other tiling/compression kinds refuse. Length can exceed one
page: the single PFN represents the contiguous range, not a one-page limit.

Supported NVOS02 fields are contiguous physicality, PCI location, the six defined
cache values, registration-intent bit, and DEFAULT/NO_MAP/NEVER_MAP. Their original
values are retained for exact retry comparison. All other nonzero flag fields
refuse, including user/device read-only, special allocation, protection, peer and
syncpoint requests. Cache and user-mapping attributes are recorded; this operation
does not promise a new user mapping or CPU cache remap. NO_MAP does not remove the
native GSP's internal CPU-view requirement. Class81 is privileged in the native
resource table; this is not an unprivileged-host forwarding allowlist.

## Authority, ownership and lifetime

The complete page-rounded span from the supplied PFN must be one current writable
QEMU guest RAM section. PFN multiplication, adjustment, length, rounding and end
arithmetic are checked. The no-vIOMMU realization posture makes the address a guest
DMA/GPA, never a host PFN/HVA. The new authority follows the existing single-memfd
DMA policy, refuses holes, overlapping or multiple sections, missing/foreign fds,
ROM, RAM-device/MMIO and effective read-only aliases. Validation acquires no GPU
resources and performs no guest-sized allocation or read.

The shared C listener predicate uses `MemoryRegionSection.readonly`, which
includes inherited alias permissions. It must not use the live backing region's
`readonly` during removal: QEMU has already changed that region when it delivers
the old writable section's `region_del`. QEMU ae35f033 `system/memory.c` combines
inherited permissions at622, snapshots them into sections at242, compares them
in flat-range equality at254, mutates live permissions at2399 and passes the old
range to deletion at1009. This permission correction affects the shared listener
on this experimental branch. It is not a completed hostile-isolation hardware
validation claim.

The `GuestMemoryList` kind is distinct from generic GPU Memory. It requires a live
original Device or Subdevice→Device→Client chain in the declaring namespace;
alias parents and foreign client handles refuse. Facts retain the full declaration
and normal graph ownership/quota/refcount lifecycle. There is no auxiliary handle
map. An identical retry succeeds; changed facts or another resource at that handle
refuse without mutation.

A future CPU consumer must resolve a live graph handle and its original parent
and device lifetimes on each access. Reportable `ResourceKey` incarnation ordinals
can be reused after death, so they alone cannot prove lifetime. Private equality-only
`ResourceLifetime` tokens use the graph's existing monotonic resource IDs; exhaustion
now refuses before insertion. Tokens are owned by their graph and never transmitted
to the guest. A chain rebuild creates a fresh graph with no old descriptors.

RAM facts contain only guest scalars and a nonzero topology generation. Every
listener add/delete, including an unrelated change, permanently invalidates all
existing registration tokens; overflow is terminal. Retrying the same declaration after a
topology change conflicts rather than silently rebinding it. Free/re-register is
required. The bounded CPU-view seam checks logical byte bounds, live graph facts
and generation, then holds the RAM registration read guard across a copy of at most
4096 bytes. No cached `RawRegion`, HVA or memfd offset escapes that guard. Prefix
and suffix bytes admitted for page rounding are not accessible through logical
object accesses. No currently admitted RPC consumer invokes this seam.

## v3 scope and limits

The source constructor registers supplied pages and creates a CPU view without
zeroing contents. This implementation represents that existing storage using the
VMM's real guest-RAM authority. It returns no invented address/PFN. It neither
pretends a GPU submission completed nor implements a CPU executor. The source
GSP-side RPC dispatcher is not public in the audited tree; the producer plus
shared MemoryList constructor provide the documented oracle and limit.

This remains an opt-in causal experiment. It does not promote the earlier b297
metadata-only diagnostic into native scheduling support. Noncontiguous/indirect
lists, other memory classes, non-pitch kinds, unreviewed flags and driver rows,
hardware maps, scheduling, interrupts and completions remain unsupported. Any
future consumer needs its own address/permission/ownership audit and genuine GPU
execution path. No default or production merge is requested by this patch.

## GPU-free validation

Run focused checks under the repository's shared cargo lock, with two jobs:

```sh
python3 tools/memory-list-probe/generate.py
cc -std=c11 -Wall -Wextra -Werror tools/memory-list-probe/test-ram-section.c \
  -o /tmp/kf-memory-list-ram-section-test
/tmp/kf-memory-list-ram-section-test
flock /tmp/kayfabe-cargo.lock env CARGO_TARGET_DIR=/tmp/kayfabe-memory-list-target \
  CARGO_BUILD_JOBS=2 cargo test -p kf-abi -p kf-rm -p kf-qemu memory_list --lib
flock /tmp/kayfabe-cargo.lock env CARGO_TARGET_DIR=/tmp/kayfabe-memory-list-target \
  CARGO_BUILD_JOBS=2 cargo test -p kf-rm --test memory_list_chain
```

The C test compiles the actual production predicate against a minimal QEMU type
fixture. Rust tests cover exact driver cells and padding, malformed descriptor
and integer boundaries, cache/mapping enums, parent types and aliases, quota and
failure atomicity, free/dup/recycle, generation invalidation/exhaustion, logical
versus rounded bounds and actual bounded RAM copies. The full chain is tested
with the independent option on/off and a channel sink that panics on any action;
ordinary class81 allocation and both later controls still refuse.

Local validation for this change: 14 focused Rust tests (ABI4, QEMU RAM3, RM7),
126 graph/RPC-bridge/full-chain regression tests, and the C predicate test passed.
The generated Rust output reproduced byte-for-byte; local documentation links
and `git diff --check` passed. No hardware run is claimed here.
