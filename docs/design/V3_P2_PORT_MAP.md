# V3 P2/P3 PORT MAP — the old control plane, file by file

**STATUS: LIVE, 2026-09-24 (w826).** A read-only survey (agent, w826) of ~104k lines of the old
control plane against the v3 rules in `V3_BUILD.md`. It is the copy order for P2 (guest GSP boot)
and P3 (objects/controls → `nvidia-smi`). ⊘ Line numbers are as of `e5dbea85`.

⊘ **One recommendation below is OVERRULED, deliberately:** §2 step 1 says *strip `HeldReply` /
`holds_for_refresh`* from kf-gsp. The hold is **kept** — it is the RPC-map synchronization point
(owner, `the_three_synchronization_points`: a guest map RPC's reply is held until our host map is
published). What goes is **inline servicing on the vCPU** (`defer_commands` default off): deferred
servicing by the register drainer becomes the ONLY mode.

---


This was a read-only survey; I changed no files. About 104k lines are surveyed. Roughly **15k lines of Rust and 2k of C carry over; about 83k drop and 4k wait for P4/P5**. Most of what carries over is the RPC answers, the GSP boot model and the object graph. Most of what drops is the old device's CPU memory emulation and the isolate plumbing.

## 1. Old file verdicts

### kayfabe-device (27,580 lines)
| file | lines | purpose | verdict | real `use`s | lands in |
|---|---|---|---|---|---|
| abi.rs | 125 | builds the GSP ABI bundle for one driver version | COPY | abi, gsp | kf-rm |
| inittables.rs | 2610 | answers the ~25 boot `RM_CONTROL`s from a chip row | ADAPT: take a `HostFacts` value instead of `&'static ChipProfile` (`self.chip.*` at :1622-2502); drop the `crate::identity_for` call (:1651) | abi, gsp, `ChipProfile` | kf-rm |
| staticinfo.rs | 228 | `GET_GSP_STATIC_INFO` (fn 65) | ADAPT: framebuffer regions and GPU name from host facts | abi, gsp | kf-rm |
| guestsysinfo.rs | 131 | fn 1 / fn 64 version handshake | COPY | abi, gsp | kf-rm |
| inert.rs | 139 | "accepted, deliberately no effect" answers | COPY | gsp | kf-rm |
| unserviced.rs, census.rs | 327, 407 | ledgers of served, refused and never-seen commands | COPY | abi, gsp | kf-rm |
| sticky.rs | 654 | guards answers the guest caches forever | COPY | abi, gsp | kf-rm |
| **sweep.rs** | 944 | `SWEEP_TRIAGE` (:171): triage of controls the guest's engine sweep asks for | **COPY** (see §4, item 1) | inittables | kf-rm |
| faultbuffer.rs | 255 | records fault-buffer registration only | COPY | abi, gsp | kf-rm |
| osevent.rs | 732 | registry of guest `OS_EVENT` objects | ADAPT: pair each with a host event fd | abi, gsp | kf-rm/kf-core |
| cpuintr.rs | 484 | CPU interrupt register tree | ADAPT: hand-written offsets (:103-124) become generated; state moves to kf-trap shadow cells (write-1-to-clear, write-only ports) | — | kf-trap + kf-core |
| lib.rs | 1399 | `ChipProfile`, `identity_for`, `rom_for`, `served_chain` | ADAPT about 300 lines (the chain order at :1128-1215, identity, ROM); drop `ChipProfile` and `CHIPS` (:692); drop the GPU name read from an env var (:1310-1340) | — | kf-rm, kf-chip |
| ga10x.rs | 2290 | GA106 constants plus `Ga10xGspModel` | ADAPT about 480 lines: the family register model (:60-540) and fault codes (:1967). Every `GA106_*` row goes (§3) | abi, arch, gsp | kf-chip |
| bar2.rs, setpagedir.rs, gvaspub.rs | 332, 498, 427 | fn 70 BAR page-directory entry; `SET_PAGE_DIRECTORY`; the guest's reserved-PDE statement | defer to P4. Record the guest's statement as an attribute of the VA-space object | abi, gsp | kf-mem/kf-rm |
| mmuinval.rs | 1076 | guest TLB-invalidate register | P4 ADAPT: keep the decode (:241) and trigger semantics; drop `drain_dirty_pdbs` (:526, a dirty gate) | | kf-trap/kf-mem |
| nonstall.rs | 351 | completion interrupt vector | P5 | ga10x | kf-chan |
| plane.rs | 7654 | `RegPlane` | **DROP**. It reads guest page tables on the CPU (walker import :142, `window_leaves` :6031/:6079, `bar1_translate` :6164, `bar2_translate` :6244), serves read traps (`read` :5681, `read_inner` :5766), emulates BAR pages one access at a time (`fb_read`/`fb_write` :6311/:6346) and runs joins (:3443). Salvage only `publish_gsp_registers` (:3998) and the service-then-deliver order (:5261, :7240) | — | — |
| fbwin, ceresolve, gpgaview, pubqueue, pubmark, dropped, dbtable, doorbell, twoworlds | ~7.1k | | **DROP**. `ceresolve.rs:540` walks guest page tables on the CPU; `fbwin` uses the CPU copy engine and isolates; `pubqueue`/`pubmark` are publication lanes | | — |

### kayfabe-rmrpc (6,317 lines)
| file | lines | verdict | lands in |
|---|---|---|---|
| lib.rs | 1974 | ADAPT about 1,400 lines. Keep `translate`, `translate_alloc`, `_control`, `_dup`, `_free` and `translate_published_pdes`. Retarget them to kf-core events. Cut `Translation::CtxPromotion` (:274) and `translate_promote_ctx` (:1769), which are the promote join. | kf-rm |
| reasm.rs | 359 | COPY (continuation-record reassembly) | kf-rm |
| policy.rs | 3771 | ADAPT about 1,000 lines. Keep `GraphPolicy` dispatch and the refusal wire format. Drop `ObjectModel` (:616, built on `Gpu`/`Proc`), `PublicationObserver` (:3609), ring and promote censuses, and the `KAYFABE_SUBDEV_FWD` env gate (:3285, an on/off flag). | kf-rm |
| fault.rs | 213 | defer to P5.5 (`RC_TRIGGERED` emission) | kf-rm |

### kayfabe-core (15,583 lines)
- **rmgraph.rs (2797): ADAPT about 1,500 lines** into the kf-core object graph. It is correctly keyed per client (`ClientKey`/`NodeKey`/`ResourceKey`). Cut these, which together form a VA→memory address table: the `Mapping{pdb, mem_phys}` + `MapKey` table (:751-778), `backing_of` (:2677), `mappings` (:2659) and `pdb_of` (:2578).
- **DROP everything else:**
  - `gpu.rs` (6167): the `Proc` ownership spine.
  - `gpa.rs` (1428): per-process address arenas.
  - `project.rs` (1502): `by_pdb` projections.
  - `promote.rs` (1352): self-described as "the address-plane join" (:1-2).
  - `reactor.rs` (922): keyed on `ProcId`.
  - `channel_kind.rs` (709): imports isolates.
  - `fault.rs` (329): stays for P5.5, minus its `AddressFault::Miss` input.

### kayfabe-rt (14,850 lines)
**All DROP for P2/P3.**
- `cpu_ce.rs` is the CPU copy-engine executor.
- `ceutils.rs` runs copies on the CPU.
- `completion_watch.rs` is on the §8 delete list.
- `device.rs` is the `Proc` locking layer.
- `translated.rs` already lives on as `kf-chan/translated.rs`.

### kayfabe-qemu-raw (32,705 lines)
- **shim_unsafe.rs (1744): ADAPT about 600 lines.** Keep the FFI entry points: `realize` (:845), `regs_create`/`write`/`reset` (:1177/:1371/:1487), `bar0_shadow_fill`/`attach` (:1599/:1713), `chip_identity` (:1135, rebuilt on host facts). Shrink `KayfabeHostOps` (:316).
- **shim.rs (18788): ADAPT about 800 lines.** Keep `Status`/`classify` (:311-381), `ShimConfig`/`BarDesc`, and `MachineRam` (:826-897, the `GuestRam` implementation). Drop the rest:
  - `object_policy` pins `Ga10xArch` (:15720).
  - The environment arms (:15836-17567) select isolates, the CPU copy executor, joins, dirty gates and a VAS publish arm.
- **DROP:**
  - `barmirror` (3507): one memslot per BAR page, filled on first touch.
  - `walkmirror` and `walkshadow`: snapshots of the guest's tables.
  - `scratchpad`, `deviceview`, `storemap`: the isolate plane. The pure planners in `storemap` could move to kf-mem in P4.
  - `bar1budget`, `kftime`, `reclaimtick`, `armretry`, `testenv`.

### kayfabe-doorbell (3,259 lines, 16 files)
| file | verdict | lands in |
|---|---|---|
| rpc.rs 157 (18-function surface plus the control list) | COPY | kf-rm |
| accessmap.rs 235 | ADAPT: take BAR0 size from facts (:46-49, :73-74) | kf-rm |
| rmgraph.rs 207 | ADAPT, merge its class allowlist into the kf-core graph (§4, item 4) | kf-core |
| classgen.rs 184, swref.rs 298 | COPY | kf-chip generator |
| memmap.rs 287, trappolicy.rs 197 | COPY, not yet in kf-trap; switch to `kf_chip::Family` | kf-trap |
| vmm.rs 191 | COPY | kf-core |
| element.rs 175 | DROP: duplicates `kf-gsp::element` | — |
| plane, hostverb, channel, completion, caps, leaf, lifetime (~1.3k) | P5 | kf-chan / kf-core |

### qemu/hw/misc/nvkvm/nvkvm.c (3926 lines)
**ADAPT about 2,000 lines.**
- **Keep:** realize, class init and properties, config-space writes, the memory listener (:1938), and the BAR0 read-only shadow pieces with trapped writes (`rom_device`, :1161-1310).
- **Drop:**
  - `nvkvm_trap_read` (:583): a read trap.
  - The BAR2 read/write traps (:809/:818) and BAR1 read/write traps (:931/:1018).
  - `bar1_gp_put_live` (:948) and the `*_passthrough_miss` handlers (:787/:908).
  - The `NVKVM_KIND_TRAP` rows for BAR1/BAR2 (:1085, :1093).
- **Replace:** `nvkvm_deliver_vector` takes the global QEMU lock (`BQL_LOCK_GUARD`, :541). Use an irqfd instead.

### kf-gsp today
It is a copy of kayfabe-gsp minus replay. It is **not yet adapted** (§4, item 2).

## 2. Copy order

**P2 (reach `GSP_INIT_DONE` and the first RPC):**
1. **kf-gsp:** strip `HeldReply`, `holds_for_refresh` and `release_held`. Make deferred servicing the only mode, since the register drainer now services the queue.
2. **kf-chip:**
   - GSP models, one per family: `Ga10xGspModel` (ga10x.rs:60-540) plus the Ad10x/Gh100/Gb20x models from `kayfabe-chips`.
   - The framebuffer-size derivations (ga10x.rs:224-290).
   - Register offsets generated by `swref.rs`.
   - A minimal `HostFacts`: architecture info, PCI ids, BAR sizes.
3. **kf-trap:** `memmap.rs` + `trappolicy.rs`, plus a thin register adapter. A BAR0 write goes into `PrivRing`; the drainer calls `GspFsm::mmio_write`, then publishes the FSM's registers to the shadow (the `plane.rs:3998` shape). `cpuintr` becomes shadow cells.
4. **kf-rm (minimum):**
   - abi.rs, rpc.rs, guestsysinfo, staticinfo, inert, unserviced.
   - VBIOS synthesis (`kf-abi::vbios::build`) fed from host identity.
5. **kf-core:** vmm.rs (`VmmOps`/`GpuDevice`), with `GuestRam` implemented over `VmmOps::guest_read`/`guest_write`.
6. **kf-qemu:** the nvkvm.c and shim_unsafe subsets above.

**P3 adds:**
- inittables (backed by host facts), sweep, sticky, census, faultbuffer, osevent, accessmap, classgen.
- The kf-core graph built from `rmgraph.rs`.
- rmrpc `translate`, `reasm` and `GraphPolicy` dispatch.
- An allowlisted host control relay, and `GPU_GET_NAME_STRING` answered from the host.

⚠ **The P3 gate can't pass on P3 code alone.** `nvidia-smi` needs `RmInitAdapter` to finish. That needs the BAR2 check to pass (bar2.rs:5-12 records `kbusVerifyBar2` failing) and the kernel CeUtils scrub to complete. Those are P4 and P6 work. This matches V3_BUILD's "guest boot last" rule.

## 3. The interfaces between the device model and the host

| old | v3 replacement |
|---|---|
| `kayfabe_gsp::CommandPolicy` (kf-gsp boot.rs:356) | **Keep.** kf-rm's chain is Sticky(Census(objects, InitTables, StaticInfo, GuestSysInfo, BarPde, Inert)) |
| `kayfabe_rmrpc::ObjectModel` (policy.rs:616), implemented by `SharedObjectModel` (shim.rs:3393) over `Gpu`/`Proc`/isolates | A narrow kf-core `RmHost` trait over `kf_host::HostRm`, detailed below |
| `KayfabeHostOps` (shim_unsafe.rs:316: `ref`/`read`/`write_region`, `bar_is_unbacked…`) | `VmmOps` (vmm.rs:89): guest read/write, install/remove memslot, `raise_irq` (as irqfd), `signal_worker`, `signal_drainer` |
| `RegPlane`, plus the `DoorbellPort`, `FbMirrorPort` and `ReadShadowPort` ports (plane.rs:1145, :1213) | `kf_trap::TrapPath`, `PrivRing` and shadow `Cell`s |
| `kayfabe_device::GuestRam` via `MachineRam`/`QemuVmm` | the same trait, implemented over `VmmOps` |
| doorbell `HostOps` (plane.rs:56) | kf-chan in P5 |

The `RmHost` methods, all existing public `HostRm` verbs:
- **Twin objects:** `raw_alloc`, `raw_alloc_nested`, `free`.
- **Host-fact controls:** `raw_control(subdevice, cmd, payload)` for allowlisted NON_PRIVILEGED controls.
- **Events:** `alloc_os_event`, `open_event_fd`, `set_notification`.
- **P4:** `alloc_vaspace`, `map`, `unmap(defer)`, `invalidate_tlb`.
- **P5:** `birth_channel`, `alloc_ce_object`, `alloc_compute_object`, `schedule`. kf-chan's `HostRing`, `Completions` and `WorkerPlane` sit on top.

## 4. Old code that claims to be v3-native but still carries a forbidden shape

1. **V3_BUILD's "dropped outright" list names `sweep`, and that entry is wrong.** kayfabe-device/sweep.rs:83-171 is the control triage table, not a publication sweep. Dropping it brings back `t134a`'s silent engine amputation.
2. **kf-gsp was supposed to be adapted and wasn't.** `HeldReply` (boot.rs:348), `holds_for_refresh` (:389, :606), the held-reply push (:1776-1798) and the `defer_commands` on/off flag (default off, :641, :2016) are all still there. Servicing inline means blocking on the vCPU.
3. **v3 crates still carry per-die GA106 constants:**
   - `kf-trap/timer.rs:30-34` (`GA106_BAR0_BYTES`).
   - kf-abi: `grstatic.rs:486, :790`, `grinfo.rs:297`, `cepce.rs:174`, `fmbsize.rs:111`, `gpuinfo.rs:155`, `smcmode.rs:124`, `vbios.rs:451-459` (device `0x2504`).
4. **doorbell `rmgraph.rs` has three defects:**
   - Nodes are keyed on a bare `HandleId` (:125), not (hClient, hObject).
   - `alloc` calls the family-blind `class_policy` (:137) instead of `class_policy_on` (:90). That is the w823 A1 defect `classgen` says it fixes.
   - `dup` copies the source's reference count into the destination.
5. **Two `Family` enums exist:** `classgen.rs:60` (Turing…Blackwell, used by `memmap.rs:105`) and `kf-chip::Family`.
6. **Element layouts are defined twice:** `doorbell/element.rs:57-66` and `kf-gsp::element`.

## 5. Per-die constants and what replaces each group

| where | replace with |
|---|---|
| PCI identity (ga10x.rs:1859-1866), `chip-device-id` and BAR size properties (nvkvm.c:3828-3841), `chip_for` (shim.rs:2537) | host PCI info query plus sysfs BAR lengths; framebuffer size from the operator |
| `PMC_BOOT_0/42` (ga10x.rs:578-582, :702) | composed from `MC_GET_ARCH_INFO`, with field layout generated from ogkm |
| falcon, GSP queue, RISC-V, WPR2, PRAMIN and `BAR0_WINDOW` offsets (ga10x.rs:67-207) | generated per family by swref |
| WPR2/FRTS/`fb_length`/`bar1_pde_base` (ga10x.rs:224-290, :1170-1309) | the existing derivation functions, fed the operator's framebuffer size |
| engines, device info, FIFO channels (ga10x.rs:840, :1673, :1752) | `GET_ENGINES_V2` plus family rules; the channel count is ours to set |
| interrupt table and subtree map (ga10x.rs:1007, :1162) | `0x2080170e`, `0x2080170f`, `0x2080170d` |
| memory system, chip info, GPU info (ga10x.rs:1613, :1494; gpuinfo.rs:155) | `FB_GET_INFO_V2`, `MC_GET_ARCH_INFO`, `GPU_GET_INFO_V2` |
| GR static/info/context buffers, PCE masks, fault-method buffer size | `GR_GET_INFO_V2` plus GPC/TPC masks; `CE_GET_CAPS_V2`/PCE mask; `0x20802a08` |
| PCIe gen (ga10x.rs:600-615), confidential-compute and BIF static rows | host bus info query; fabricated so ogkm accepts them (CC off) |
| GMMU static (ga10x.rs:1767) | the family row |
| register access map (ga10x.rs:1537) | authored, via accessmap.rs |

The captured GA106 rows become the test oracle: on a GA106 host, derived must equal captured.

## 6. Line counts

| | lines |
|---|---|
| surveyed (the five old crates, the 16 doorbell files, nvkvm.c) | ≈104,300 |
| copy or adapt for P2+P3 | ≈15,200 Rust + ≈2,000 C |
| — kayfabe-device | ≈7,600 |
| — kayfabe-rmrpc | ≈3,000 |
| — kayfabe-core `rmgraph` | ≈1,500 |
| — kayfabe-doorbell | ≈1,750 |
| — kayfabe-qemu-raw | ≈1,400 |
| deferred to P4/P5/P5.5 | ≈4,100 |
| dropped | ≈83,000 |

About half of the lines kept are doc comments, so the code actually carried over is closer to 8k.