# V3 sparse PTEs: the Enscape 07/08 write-after-unmap fault (H6)

**STATUS: DESIGN-ONLY, 2026-10-25.** Branch `claude/sparse-pte-20261025`, on top of the VAHIST logging
branch (`claude/enscape08-vahist-20261024`, candidate 6d `714ec29d3` plus logging-only commits).
Nothing here was run on hardware. Labels: MEASURED (a named log or run), CODE READ (source at the
named revision), PUBLIC (the public ogkm-595.84 sources or a public API contract), INFERRED
(reasoning; never evidence).

## 1. The claim under test (H6) and its falsifier

**H6.** Run 5227's Xid 31 (channel `0x6b`, CE0 `VIRT_WRITE`, VA `0x2_d75e5000`) is a write to a
64 KiB tile that the guest had just turned into a SPARSE page. On the real GPU a sparse PTE absorbs
the access without a fault. The host twin turns the same guest change into a plain unmap, which is an
invalid PTE, so the same write faults there.

**Falsified by any one of:**
1. The twin's unmap of a sparse-declared page is already sparse (the reservation carries SPARSE, or a
   sparse state is carried through the diff). CODE READ below says it is not.
2. The guest PTE at fault time was not the sparse encoding, for example plain `0`. MEASURED only at
   +0.6 s (see section 4), so this stays open.
3. Public documentation says a write to a sparse PTE faults on real hardware. PUBLIC below says it
   does not.
4. After a twin that keeps sparse state, the same write still faults (hardware validation).

## 2. What the sources say

PUBLIC, `ogkm-595.84`:
- VER2 PTE: `VALID` is bit 0, `VOL` is bit 3 (`gp100/dev_mmu.h:120-127`). The sparse PTE is VALID
  clear and VOL set: `kern_gmmu_gm200.c:41-46` ("GM20X supports sparse directly in HW by setting the
  volatile bit when the valid bit is clear"). The guest word `0x8` is exactly that.
- VER3 PTE: `PCF` is bits 7:3, `PCF_SPARSE = 1` (`gh100/dev_mmu.h:498-500`). `PCF_NO_VALID_4KB_PAGE =
  3` and `PCF_MAPPING_NOWHERE = 2` are different states. VER3 PDEs spell sparse as PCF 1 or 3
  (5:3), a known walker gap (the ratchet in `kf-mem/tests/walk_format_vs_ogkm.rs`).
- Hardware behaviour: `uvm_mmu.h:195-199` "Sparse PTEs will indicate to MMU to route all reads and
  writes to the debug pages. Therefore, accesses to sparse mappings do not generate faults."
  `nvos.h:1417-1421` (`NVOS32_ALLOC_FLAGS_SPARSE`): an unmapped range of a sparse virtual allocation
  "should not fault but simply return 0's". So a write is absorbed and does not fault.
- Not the same encodings: unmapped big PTE `VALID=0 VOL=0 PRIV=1` (`uvm_mmu.h:201-212`) and NV4K
  `VOL|PRIV` `0x28`. The walker already keeps them out of the sparse census.
- Host RM: a reservation allocated with `NVOS32_ALLOC_FLAGS_SPARSE` is sparsified at allocation
  (`gpu_vaspace.c:1399-1426`) and `gvaspaceUnmap` returns an unmapped range to SPARSE, not to invalid
  (`gpu_vaspace.c:2074-2080`). The state belongs to the whole reservation block, not to a range.

CODE READ, revision `b4c5e3023`:
- `cuda/walk/kf_walk.cu:359-372, 602, 689-701`: a sparse slot only increments `c.sparse`; no run is
  emitted. The diff therefore contains an UNMAP for the vanished placement and nothing says
  "sparse".
- `crates/kf-mem/src/apply.rs` (`apply_entry`, unmap rows) and `vasmgr.rs` (`on_walk_ready`): no
  sparse state exists anywhere on the way to the host.
- `crates/kf-host/src/channel.rs:111-112`: reservation flags `0x10 | 0x400 | 0x80000`; no
  `NVOS32_ALLOC_FLAGS_SPARSE` (`0x0400_0000`). An unmap inside the reservation is a plain invalid PTE.
- The VAHIST facts of run 5227 (MEASURED, `/root/night/enscape_vahist/vahist-report-5227.txt`): MAP
  of `0x2d75e0000+0x10000` at walk 306599, UNMAP of the same at walk 306600, verdict UNMAPPED,
  `walk_refusals=0`; the guest big PTE read `0x8`, printed as `bit0=0 vol=1`.

**Result: H6 stands against the code and the public sources.** No fact read contradicts it. It is
still INFERRED, not shown, because the PTE was read 0.6 s after the fault and the fault-to-unmap
interval is a loose bound.

## 3. Why the obvious fix (SPARSE on the guest reservation) is not the fix

The reservation flag is the only unprivileged host verb that produces a sparse PTE, and it is a
property of a whole reservation. Applied to the guest reservations (`GUEST_VA_RANGES`) it makes every
unmapped guest VA in the twin read as zero and absorb writes. The real GPU does that only where the
guest wrote the sparse encoding. Consequences:
- The owner-approved negative controls (2026-10-22: exact channel, exact VA, `FAULT_PTE`, CE client)
  write to a VA that is plain invalid. They would stop faulting, so the gate would pass for the wrong
  reason or fail by missing the expected Xid.
- The containment principle (2026-10-22: a single-channel guest-userspace fault is reported, not
  swallowed beyond what real hardware does) is violated for every plain-invalid access.
- Sparse accesses go to the GPU-global debug page (`uvm_mmu.h:195-199`). INFERRED: that page is
  shared by every VA space on the GPU, so a blanket flag widens a possible cross-process data channel
  through it; it must be measured before anything relies on it.

A selective twin needs two things, neither of which exists:
1. The walker has to report the sparse state. `kf_walk.cu` and the committed PTX must change; there is
   no `nvcc` on the development box, and `kf-gate9` holds the PTX to the Rust diff model on hardware.
2. The host needs a sparse state for exactly the declared ranges. The static reservation owns one heap
   block for its whole range, so a sparse micro reservation cannot be placed inside it
   (`sim.rs` header, `gpu_vaspace.c`). The remaining unprivileged routes are a reservation layout
   with sparse and non-sparse partitions, or UVM's `UVM_MAP_EXTERNAL_SPARSE` (a registered UVM VA
   space). Mapping a host scratch page at the VA is not allowed: passthrough spaces hold only
   guest-leaf mappings, and it would not read as zero after a write.

These are owner decisions (reservation layout; walker ABI), so this branch does not guess them.

## 4. What this branch delivers

- `kf_mem::sparse::classify_leaf`: the derived classification of a raw leaf word (`Valid`, `Sparse`,
  `Invalid`) for the VER2 and VER3 descriptors, checked against the die groups' `dev_mmu.h`. It is the
  single definition any later diff or probe uses.
- The VAHIST PTE probe names `sparse` explicitly, so the next captured lookup decides H6's second
  falsifier directly instead of by reading `vol=1`.
- A diagnostic, default OFF, measurement-only: `KF3_DIAG_SPARSE_RESV=1` adds `NVOS32_ALLOC_FLAGS_SPARSE`
  to the guest reservations (the "quick experiment"). It is never required to boot, and its use makes
  the negative controls invalid (section 3).
- A test that pins the default: guest reservations must not carry SPARSE unless the diagnostic is set.

## 5. Open decisions for the owner

1. D1: selective sparse in the twin. Which realization (reservation partitions, UVM external sparse)?
2. D2: walker ABI. Add a sparse run kind (rebuild PTX on a box with `nvcc`, rerun `kf-gate9`), or have
   the VA thread probe the guest PTE of an UNMAP run (a TOCTOU read of hostile bytes that can only
   choose between two authored actions)?
3. D3: VER3 PDE-level sparse (PCF 1 or 3 at 5:3) is not honoured by the walker (ratchet test).

## 6. Hardware validation plan (NOT RUN)

Pinning: every claim carries the source revision of the binary (`build_kf3.sh` installs it under
`kf3-bins/<rev>/`). Use the head of branch `claude/sparse-pte-20261025` for the experiment; the
control is `b4c5e3023` (the logging-only parent). Strictly serial, one box, bare metal first.

1. **Bare metal, no kayfabe (does real hardware absorb the write?).** New probe, not yet written:
   a Vulkan program (`sparseBinding`, `sparseResidencyBuffer`) that binds a 64 KiB tile, unbinds it
   with `vkQueueBindSparse`, then submits a copy-engine write into the unbound tile
   (`vkCmdFillBuffer` or `vkCmdCopyBuffer` on a transfer-only queue family, which uses a CE). Expect:
   no Xid, no `VK_ERROR_DEVICE_LOST`, and a read of the tile gives zeros. Also record whether a
   value written through the sparse tile is visible to a second process's sparse read (the shared
   debug page, section 3). This is the falsifier of H6's hardware half: an Xid 31 here means a
   real GPU faults on this write and H6 is wrong.
2. **Linux guest under kayfabe, same probe, product flags.** Gate `scripts/bench/gpu_fault_containment.sh`
   style harness (host Xid admission: exact channel, exact VA, `FAULT_PTE`, CE client). Expected on
   the PRODUCT build: an Xid 31 at the unbound tile (the bug, reproduced in a unit of one tile).
   This is the deterministic reproducer that Windows run 5227 was not (the Windows fault is about 1
   in 2 boots). With `KF3_DIAG_SPARSE_RESV=1` the Xid disappears (H6 supported) and the
   negative-control arm of `gpu_fault_containment.sh` stops faulting (expected; proves the flag is
   not a fix).
3. **Windows SPECviewperf, only after (2) reproduces.** Enscape 07 and 08 in the full nine-workload
   order, production flags (run 4501 set), VAHIST lookup mode on. Pass criterion for any fix: five
   consecutive full-order boots without Xid 31 at 07/08 (the base rate is about 50 percent per
   boot, so fewer boots prove nothing). A captured VAHIST PTE line now prints `leaf=sparse`.
4. **Regression set after any selective-sparse change:** `v3_gates.sh` 9/9, `fast_suite.sh` 30/30,
   `gpu_fault_containment.sh` (the plain-invalid negative control must still fault), the app matrix,
   and `broker_lane.sh` per the retest rule.

## 7. Side finding

CODE READ: `kf_slot_sparse` (`kf_walk.cu:359-364`) tests only VOLATILE on VER2, so NV4K (`0x28`)
would count in `sparse_slots` unless an earlier branch excludes it. `classify_leaf` additionally
requires PRIVILEGE clear. Nothing consumes the census for decisions today; check before a sparse run
kind is added to the kernel.

