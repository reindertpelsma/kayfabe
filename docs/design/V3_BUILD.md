# V3_BUILD — how v3 is being built (branch `v3`)

**STATUS: LIVE, 2026-09-24 (w826).** Governs the build order and what is copied. The design is
`THE_ARCHITECTURE_v3.md` + `THE_V3_PLAN.md`; this file only records the execution decisions.

## Rules (owner, 2026-09-24)

- v3 is written **as designed**, in fresh `kf-*` crates. The old `kayfabe-*` crates are **frozen**:
  reference + grader (rm-ladder, fastguest harness, bare-metal suite, cup3/cup8/LLM hooks).
- Code may be **copied from the old tree whenever it fits v3**. The test is the code's shape, not its
  origin. Never copied: CPU reads of guest page tables, CPU-executed engine work, address
  tables/VA mirrors/joins, isolates/IPC plane, publication epochs/dirty gates/sweeps, per-page BAR
  fill traps, **and snapshots of the guest's tables** (the walk kernel's delta snapshot and
  `walkmirror` included — v3 §4.2 w825: *the snapshot is a shadow; the ledger replaces it*).
- The target is the v3 architecture; **LLM parity is what v3 must deliver**, never a shortcut.
  Temporary breakage is accepted.

## The crate map (from the w826 five-way inventory)

| v3 crate | built from (copy / adapt) | new |
|---|---|---|
| `kf-util` | copy `kayfabe-util` | — |
| `kf-arch` | copy `kayfabe-arch` (GSP seam; GMMU/USERD traits later) | — |
| `kf-abi` | copy `kayfabe-abi` (generated modules, wire/view/versions, table layouts); `oracle.rs` test-only | regenerate from ogkm later |
| `kf-gsp` | copy `kayfabe-gsp` {ring, element, ram, rpc, fault, seq}; **adapt** `boot.rs` (drop HeldReply / holds_for_refresh / deferred lane); `kayfabe-device/abi.rs`; drop `replay.rs` | — |
| `kf-chip` | `kayfabe-device/ga10x.rs` GspModel + boot regs, `cpuintr.rs`; `kayfabe-doorbell` classgen/swref | FB size from the host GPU, not a constant |
| `kf-trap` | `kayfabe-doorbell` {token, bitmap, wake, ring, trap, trappolicy, shadow, memmap}; the inline passthrough store | thin register adapter (~200 lines) replacing `RegPlane` |
| `kf-rm` | `kayfabe-device` {inittables, staticinfo, guestsysinfo, inert, unserviced, census, sticky}; `kayfabe-rmrpc` {translate, reasm, fault}; doorbell {rmgraph, accessmap, rpc} | object graph keyed `(hClient,hObject)`; `GPU_GET_NAME_STRING` from the host |
| `kf-host` | `RmConnection` from `isolate-host/rm.rs` (raw verbs public; no `HostRmBackend`, birth clients, exec VAS); `map_local_at_with_flags` + `invalidate_tlb`; `birth_channel`, `alloc_engine_object`, schedule; `kayfabe-linux-raw` chardev/ioctl/mapping/window/eventfd | deferred **unmap** flag; OS-event / NONSTALL registration |
| `kf-mem` | existing `kf-mem`; `kayfabe-cuda` + `cuda/walk` kernel (walk only, **no delta snapshot**); storemap pure planners + our-handle ledger + `reconcile_scoped` | our BAR1/BAR2 roots declared in static info; batched map with TLB-defer + one invalidate; scoped walks by named PDB |
| `kf-chan` | doorbell {channel, completion, plane, hostverb}; `kayfabe-rt/translated.rs`; birth rules from `kayfabe-fwd` (adoptability by GPGA arithmetic, not bindings); `kayfabe-completion` policy | **Translated execution** (windows, own ring, GP copy, split at MEM_OP); **host event → irqfd → MSI** |
| `kf-core` | doorbell `vmm.rs` seam | — |
| `kf-qemu` + `qemu/hw/misc/kf3/` | QOM glue + `KayfabeHostOps` from `nvkvm.c` / `shim_unsafe.rs`; `vmm-qemu` window verbs | device `kf3-gpu`, `kf3_` symbols, `KF_DEVICE` in the 3 harness sites |

**Dropped outright:** `kayfabe-device/plane.rs` (except GSP dispatch ideas), fbwin/gpgaview/ceresolve/
gvaspub/pubqueue/sweep/mmuinval/bar2/setpagedir, `kayfabe-core` gpu/project/gpa/reactor/promote-join,
`kayfabe-rt` device/ceutils/completion_watch, `kayfabe-fwd` CE half, isolate crates, barmirror,
deviceview, walkmirror, delta/ack.

## Order and gates (THE_V3_PLAN §2)

P2 GSP boot → `GSP_INIT_DONE` + first RPC · P3 objects/controls → `nvidia-smi` correct · P4 memory/VA/BAR
→ alias/map/invalidate arms · P5 channels → ring/doorbell/concurrency arms · P6 Translated → kernel scrub
`forwarded>0`, ce-client* · P7 compute → cup3, cup8 · then the LLM lane at ≥0.8× host tok/s (host
baseline **never recorded** — record it on the first box).
