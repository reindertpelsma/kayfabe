# CMP1 E5: parked at the ZBC prerequisite

STATUS: RESEARCH / PARTIAL, 2026-10-26. Product base `6564dd597`. Instrument source `fc337998e`; tests-first instrument commit `c96e3f7a3`.

CODE READ [src] open595 source files identified by `source-hashes.json`: the proposed `PREFER_NO_ZBC` allocation attribute is not a demonstrated hardware prohibition. The source contradicts treating that preference alone as sufficient to enable compression while leaving kayfabe's private guest ZBC table unchanged. COMMON's stop-on-contradiction rule parks the lane here. No alternative ZBC policy or production allocation/mapping change was made.

CODE READ open595:

| Source | Finding |
|---|---|
| `mem_mgr_tu102.c:180-278` | Generic `COMPR_ANY` selects a compressible generic kind; this selection does not inspect the ZBC preference. |
| `mem_mgr_tu102_base.c:52-55` | `FB_IS_KIND_ZBC` uses the same compressible-kind predicate as `FB_IS_KIND_COMPRESSIBLE`. |
| `g_mem_mgr_nvoc.c:991-1014` | Ada uses these kind-selection and classification implementations. |
| `mem_mgr_gm107.c:517-549` | The comment requires UMD ZBC-slot masking for compression without ZBC; the normal resource path returns `PREFER_ZBC` for a ZBC-capable kind. |
| `virtual_mem.c:1320-1396` | The kind override is validated and physical compression capability checked; this code does not carry the allocation preference into a hardware ZBC prohibition. |

CODE READ product base `crates/kf-rm/src/zbc.rs:1-17`: guest ZBC values remain in a private per-VM software table; the source explicitly reopens that policy when real compression is introduced. INFERRED: blindly preserving guest compressible kinds would not establish correctness of guest clear indices against the host table. No observed ZBC use, data disclosure, allocation downgrade or compression failure is claimed: the GPU experiment did not run.

CODE READ base `crates/kf-host/src/channel.rs:1046-1065`: the T-space window map passes no kind override. That map would inherit a compressible allocation's kind. The CUDA import, guest BAR1, display and relay views also require qualification; this lane has not completed that audit or changed those consumers.

CODE READ design prerequisite: only leaves reach the twin. The original guest allocation attributes, boundaries and ZBC masks cannot be recovered from a kind byte. A possible join is the validated guest-store offset plus the derived leaf kind plus host allocation capability readback. That join alone says nothing about ZBC correctness or the other consumers of the same backing.

MEASURED 2026-10-26 `traces/compression_store_20261026/qualification.json`: the probe compiled on the host with strict compiler warnings; 27 GPU-free evaluator tests pass; all 12 mutants in `gate-mutations.json` have at least two killing tests. The EXPECTED RED is a missing new instrument API, not reproduction of a product defect. No Cargo build or product test claim.

MEASURED 2026-10-26 `traces/compression_store_20261026/qualification.json`: all three host-lock attempts returned busy; no lease was acquired and GPU run count is zero. The second attempt was 14 seconds early; the third was timestamp-guarded at 300 seconds. The foreign VM was untouched. The owned waiter was stopped when parking the lane.

E5(a,b) is prepared in `scripts/bench/compression_store/`; E5(c,d), GPU bandwidth, Task 2 and the surface intervention are NOT DONE. Allocation/PTE readback alone would not certify ROP/ZBC suppression or fresh compression state.

Owner options:

1. Keep compression off while a ZBC design is resolved (recommended).
2. Define a separately qualified host-owned ZBC policy through unprivileged verbs,
   including index consistency, finite-slot ownership and hostile-guest behavior;
   then resume the bare-metal producer/alias tests. No such policy is implemented.

No GPU state, GDM state, clocks, driver, VM, guest kernel or production default was changed. No owned GPU process or lock remains. Source and small text evidence are retained for a resumed lane; no build binary is committed.