# V3 RAW CLIENT — the grader moves into the live tree; the refresh stress is its first new arm

**STATUS: DESIGN-ONLY, 2026-10-31.** Scoping and plan only. Nothing here is built: no crate, no arm, no CI edit.
Written against `origin/next` 23505064c with PR #214 (`kayfabe-device` to `archive/crates`) in flight. Owner direction
of 2026-10-31: (1) move the raw client `crates/kayfabe-rm-ladder` (the "grader", 30 arms) into the `kf-*` tree so the
frozen `crates/kayfabe-*` tree can be archived; (2) the new REFRESH STRESS test is an **arm of the raw client**, not a
one-off, runnable by the gpu-vast CI across drivers and dies; (3) no big bang: a strangler path, old and new verified
against each other arm by arm.

Evidence labels used throughout: **MEASURED** (a number I computed in this scoping, command given), **CODE READ**
(path:line), **INFERRED** (reasoned, not observed), **UNSURE** (I could not settle it). A claim without a label is
CODE READ.

---

## 0. Summary

- The grader is one 19,362-line `main.rs` (MEASURED: 13,799 code, 4,868 comment, 696 blank lines) plus the RM ioctl
  layer it borrows from `kayfabe-isolate-host` (`rm.rs`, 16,895 lines). It has **30 arms** (§2.1).
- Its declared closure is the 15 frozen crates still in `crates/` after #214 (175,047 lines incl. tests, from #214's
  table) plus `kayfabe-rm-ladder` itself. **Reachable machine code is far smaller** (§2.2): of the 15 crates, 7 add
  **0 bytes** to the release binary and 4 more add under 2 KB; 95% of the attributable kayfabe code (MEASURED, 786,868 of
  824,200 symbol bytes) is the ladder itself, `isolate-host`, `linux-raw` and `isolate`. That makes a port tractable.
- The v3 tree already has most of the substrate: `kf-host::HostRm` (open, `alloc_os_descriptor`, `map`, `unmap`,
  `invalidate_tlb`, `birth_channel`, `alloc_ce_object`), `kf_harness::CeRig` (a copy-engine rig with event-fd
  completion), `kf_harness::Ledger` (named `CHECK` lines), `kf_chip::{DieId, Family, MmuFormat}` and `kf_abi`
  per-driver tables. The thin-guest lane already runs a `kf-*` tool instead of the raw client (`KF_TOOL_BIN`,
  `kf-syncwait-stress`, build_fast_guest.sh:319-332), so a `kf-*` client inside the guest is **proven**, not new.
- What has **no** kf-* equivalent (§2.3): `probe_va` (page-level-info probe), the `/dev/nvidia-uvm` driver
  (`uvm_raw`, `mean`: 2,490 lines), the old backend's composed verbs (`alloc_probe_local`, `map_local_at`, ...), the
  fd-crossing/child-process helpers of `bar1-crossing`, the R33 ioctl census wrapper.
- The CI contract is narrow but has **sharp edges** (§3): exit code only for pass/fail, but also the process name
  (`kayfabe-rm-ladd`), the source-file pin in `native_xid_guard.py`, two arms' printed markers, and the literal
  number 30 in about nine places. A drop-in must keep all of them.
- Recommended shape (§5): new crate `kf-rawclient` with an arm registry; **its binary takes the name
  `kayfabe-rm-ladder`** and execs the old program (renamed `kayfabe-rm-ladder-legacy`) for any arm not yet ported.
  Every script, hook, the Xid guard's comm match and the release attestation keep working unchanged.
- Effort (INFERRED): framework + stress arm + 3 ported arms + CI wiring about 4,250 new lines and 10-11 working days (plus a
  1-2 week calibration wait); the remaining 27 arms about 9,400 new lines and 15-21 working days; retirement and
  archive 1-2 days. Overall about 14,000 lines and 27-34 working days, UNSURE by +-50%.

---

## 1. Why this fits and where it can fail

The stress arm needs exactly the verbs the host side already uses for the same job: `alloc_os_descriptor` over a
user buffer, a fixed `NV_ESC_RM_MAP_MEMORY_DMA`, one invalidate, unmap, free
(`kf-host/src/channel.rs`, `map_scattered_through`: "stitch -> ONE OS descriptor -> ONE fixed MAP_MEMORY_DMA").
The raw client in a guest issues these verbs to the guest's driver; the guest RM writes the PTEs; kayfabe's VA manager
sees them. The host stitch costs 20.4 us per piece on the one measured Enscape window (MEASURED upstream,
`traces/enscape_h_counts_20261029`, 98,304 attempts, 2,005.8 ms), so a faithful synthetic is dominated by exactly the
code we want to watch.

**The premise the whole arm rests on is UNSURE**: that a Linux guest RM `MAP_MEMORY_DMA` of an OS descriptor over
scattered user pages reaches the VA manager as the same row shape a Windows WDDM paging operation does (4,096
single-page runs per batch, stitched). Step S3 of the plan is therefore a one-box shape check with `KF3_MAPLOG=1`
(`kf-mem/src/maplog.rs:1`) **before anything is gated**; §4.9 lists it as falsifier F1.

---

## 2. Inventory

### 2.1 The 30 arms

Default list: `scripts/fastguest/fast_suite.sh:24-30` and `scripts/fastguest/bare_metal_suite.sh:25-31` (identical;
MEASURED count 30). One process per arm (`--gpu N --<arm>`). "bare" and "guest" are per-arm client wall time, range
over GA102 / GA106 / AD106 from `traces/sw5_die_sweep_20261025/*/{bare,fast}_scorecard.log` (MEASURED; Vast boxes are
nested KVM, so absolutes are per box). Handler = line of the `fn` in `crates/kayfabe-rm-ladder/src/main.rs`; "lines" =
MEASURED function span. Verbs = the `rm.<method>` calls of the handler (MEASURED by script) plus controls named in the
doc comment.

| # | arm | handler (lines) | what it proves | bare s | guest s | verbs / substrate |
|--:|---|---|---|---:|---:|---|
| 1 | `--timer` | `timer_probe` 5434 (436) | R19: an unprivileged process reads the host GPU ns counter, at which USERMODE page offset | 2.2-2.4 | 1.1-2.1 | alloc_timer_object, host_ptimer_via_usermode, map_object_uncached |
| 2 | `--engines` | `engines` 214 (89) | R13b: `engineType` routes to the right runlist (copy vs graphics) | 2.3-2.5 | 1.6-3.5 | alloc_vaspace, alloc_channel_on |
| 3 | `--doorbell-census` | `doorbell_census` 303 (289) | R13c: the `(runlist, chid)` in the work-submit token; **per-die discriminator** (Blackwell bit 30) | 2.3-2.6 | 2.7-6.2 | alloc_channel_on, control |
| 4 | `--gpu-info-sweep` | `gpu_info_sweep` 752 (112) | R21: every `GPU_GET_INFO_V2` index, one at a time | 2.2-2.4 | 1.2-1.8 | control |
| 5 | `--bus-info-sweep` | `bus_info_sweep` 864 (191) | R22: every `BUS_GET_INFO_V2` index; `PCIE_GEN_INFO` repeatedly | 5.4-5.6 | 4.3-5.1 | control |
| 6 | `--concurrency` | `concurrency` 66 (148) | R12: does the verb path buy wire concurrency | 5.8-6.8 | 9.0-26.6 | threads over one connection |
| 7 | `--defer-liveness` | `defer_liveness` 12370 (+`defer_arm`, `dl_*`: 2,885) | w289: does a PTE written with `NVOS46_FLAGS_DEFER_TLB_INVALIDATION` go live with no invalidate. **Provokes host Xid 31 by design** (admitted) | 14.3-14.5 | 13.4-14.6 | alloc_channel_*, map_local_at_with_flags, invalidate_tlb, pde/pte probe |
| 8 | `--blockage-coverage` | `blockage_coverage` 3541 (131) | T1: async-blockage coverage (emulated channels, RPC, TLB invalidate) | 2.6-2.7 | 1.5-2.6 | (inline) |
| 9 | `--alias-two-vas` | `alias_two_vas` 6387 (262) | w379 R1: two VAs alias one memory | 2.2-2.3 | 1.1-2.0 | alloc_vaspace, alloc_channel_at, map_local_at, unmap_local |
| 10 | `--alias-unmap-observe` | `alias_unmap_observe` 6649 (218) | R1'': unmapping one of two aliased VAs leaves the other intact and is observable | 2.2-2.3 | 1.1-1.9 | + probe_va |
| 11 | `--map-propagation` | `map_propagation` 6867 (172) | R2: a map at a dictated VA lands in the **host's** page tables (asked of the host driver) | 2.2-2.3 | 1.1-1.8 | alloc_probe_local, map_local_at, probe_va |
| 12 | `--late-map-race` | `late_map_race` 3339 (395) | W377: a map made after the doorbell reaches an already-running channel | 2.6-2.8 | 1.5-2.6 | channel_ring_va, submit_fenced_release, userd_cursors |
| 13 | `--missing-page-fault` | `missing_page_fault` 7039 (247) | w379 R3: a deliberate missing page is contained and named. **Xid 31 by design** (admitted) | 5.2-5.4 | 4.3-5.1 | alloc_sysmem, schedule, unmap_local |
| 14 | `--uvm-invalidate` | `mod uvm_raw` 16872 (781) | w392c: raw `/dev/nvidia-uvm` chain whose channel carries the MEMOPS `MMU_TLB_INVALIDATE` | 2.3-2.4 | 2.6-3.0 | UVM ioctls, `host_alloc_vaspace_externally_owned` |
| 15 | `--uvm-mean` | `mod mean` 17653 (1,709) | w392d: two clients, several threads, over the same UVM chain; `--mean-falsify` | 2.3-2.5 | 2.2-3.8 | UVM ioctls, threads |
| 16 | `--executor-vas` | `executor_vas_probe` 3672 (326) | R30: the isolate's completion semaphore is not nameable from a guest-bound VA space | 2.2-2.4 | 1.1-2.2 | ce_control_placement, probe_guest_reachability, prove_ce_copy |
| 17 | `--guest-ring-channel` | `guest_ring_channel_probe` 5214 (220) | R31: RM builds a channel on a command queue we did not allocate | 2.2-2.4 | 1.1-1.9 | alloc_channel_over_guest_ring |
| 18 | `--dictated-ring` | `dictated_ring_probe` 2859 (142) | R26: GPFIFO at a dictated address, engine fetches from it | 2.2-2.4 | 1.1-2.0 | alloc_channel_at, submit_semaphore_probe |
| 19 | `--dictated-ring-negative` | `dictated_ring_negative` 3001 (145) | R26 negative control (occupy the address first) | 2.2-2.4 | 1.4-1.6 | map_gpu_va |
| 20 | `--ce-client` | `ce_client` 4215 (+`ce_interrupt`: 906) | R33/R35: a CE copy and its interrupt end to end; R33 ioctl census | 2.2-2.4 | 1.3-2.0 | prove_ce_copy, prove_ce_interrupt, pde_info, osevent |
| 21 | `--ce-client-guest-ram` | same fn, guest-RAM operands | CE copy whose operands are guest RAM; the **slowest guest arm** | 4.8-5.5 | **51.9-111.9** | + `GuestWindow` |
| 22 | `--guest-ram-pin` | `guest_ram_pin_probe` 1287 (103) | R29: a sealed memfd described to RM and placed at a dictated VA | 2.2-2.4 | 1.3-1.9 | prove_guest_ram_pin |
| 23 | `--bar1-crossing` | `bar1_crossing_probe` 1635 (+child: 608) | W393: a vidmem object viewed through BAR1 across a process boundary, with an in-process mini KVM | 2.3-2.5 | 1.3-2.2 | alloc_vidmem, export_device_view, Kvm/Vcpu, fd passing |
| 24 | `--atomics-probe` | `atomics_probe` 1055 (161) | R23: `BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS`, caller vs instrument seed | 2.2-2.3 | 1.1-1.6 | control |
| 25 | `--pce-mask-probe` | `ce_pce_mask_probe` 1216 (71) | R24: `CE_GET_CE_PCE_MASK` per copy engine | 2.2-2.3 | 1.1-2.3 | control |
| 26 | `--gpga-reserve-probe` | inline 15763 (~70) | vidmem reservation plus a read sweep (`sweep_vidmem_reads`) | **45.6-56.7** | 18.8-27.3 | reserve, vidmem CPU view |
| 27 | `--map-stress` | `map_stress` 7286 (245) | w379 R5: 48 alloc/map/free cycles over 4 interleaved live mappings; the **small predecessor of the new arm** | 2.2-2.4 | 1.9-3.1 | alloc_channel_at, map_local_at, unmap_local, probe_va |
| 28 | `--concurrent-fuzz` | `concurrent_fuzz` 9951 (+worker: 806) | w385: threads x clients x jitter x placement fuzz with a seed | 2.5-2.9 | 4.8-9.7 | as map_stress, many threads |
| 29 | `--cross-client-leak` | `cross_client_leak` 8107 (579) | w381 R5b: two RM clients cannot see each other's memory | 2.2-2.4 | 1.1-2.2 | host_client, two connections |
| 30 | `--rpc-mixed-allocs` | `rpc_mixed_allocs` 7676 (431) | w381 R4: ordering and identity survive mixed RPC and normal allocs | 3.0-3.1 | 2.0-2.9 | alloc_sysmem, channel_ring_va |

Totals (MEASURED, from the same scorecards): all 30 arms take 136.0 / 152.7 / 145.6 s of client time on AD106 / GA102 /
GA106 bare, and 144.0 / 171.3 / 248.8 s in the thin guest. Three arms (`ce-client-guest-ram`, `concurrency`,
`gpga-reserve-probe`) are 58-63% of the guest time on each die. **Guest/bare per-arm ratios in the existing data
run 0.4x to 22x** (the fixed ~1 s that bare shows for light arms makes them look faster in the guest), so a ratio is
only meaningful per arm and per box: this is why §4.7 gates on a same-box native run of the same arm, with
thresholds calibrated, not on a table.

### 2.2 The closure, and what the binary really reaches

Edges: `cargo tree --offline -p kayfabe-rm-ladder -e normal,build --features cuda-window` (MEASURED): rm-ladder
declares abi, arch, chips, device, isolate, isolate-host, linux-raw, mmu, util. Transitively: doorbell, mocks, gsp
(via chips), trace, completion (via gsp), vmm (via isolate-host), cuda (only with the non-default `cuda-window`
feature), and `kf-abi`/`kf-arch`/`kf-util` (via isolate-host). No `kf-*` crate depends on a `kayfabe-*` crate (#214).

Direct references in `main.rs` (MEASURED, `grep -o 'kayfabe_[a-z_]*::'`): `isolate` 60 (all but a few are `HostHandle`),
`linux_raw` about 80, `isolate_host::rm` about 70, `abi::submit` 27, `arch::ids` 4, `util::trapwitness::OffTrap` 2, and **zero**
for `chips`, `device`, `mmu` (comments only).

Reachable code: release build of `kayfabe-rm-ladder` (rustc 1.99, no LTO, `CARGO_TARGET_DIR=/data/agent-target-scope`,
deleted afterwards), `nm -S -C`, symbol bytes grouped by crate path (MEASURED; text 3.75 MB, 11,084 symbols; 62% of
symbol bytes are std/rodata/closures that carry no crate path, so the table is a lower bound on attributable code and
**line counts below are INFERRED from it, not measured**):

| old crate | src lines (+tests) | symbol bytes in binary | what the ladder takes from it | kf-* equivalent |
|---|---:|---:|---|---|
| rm-ladder (the bin) | 19,362 | 359,631 | itself | `kf-rawclient` (new) |
| isolate-host | 33,125 (+9,010) | 312,746 (`rm` 220,175; `isolate` 38,108; `hostabi` 21,217; `listobj` 11,868; `proto` 11,012; `export` 3,877; `guestram` 3,048; `fdcross` 1,982) | `rm::HostRmBackend`/`RmConnection` and the probes it exports, `osevent`, `ClientAbi`, `ChildExports`, `listobj::run`, fd frames | `kf-host::HostRm` (7,117 lines), `kf-rm`, `kf-abi::hostabi` |
| linux-raw | 16,630 (+1,314) | 77,871 (`spawn_unsafe` 17.7K; `mapping_unsafe` 15.1K; `ioctltrace` 12.3K; `error` 11.2K; `census` 4.2K; `scm_unsafe` 3.6K; `chardev` 3.1K; `vcpu` 2.2K) | char devices, mappings, ioctl trace/census, Kvm/Vcpu, GuestWindow, child spawn | `kf-linux-raw` (15,370): has mapping, chardev, census, ioctltrace, kvm/vcpu, window, unixsock; **no spawn/sandbox/scm** |
| isolate | 5,805 (+108) | 36,620 | `HostHandle`, `IsolateId`, `RmBackend`, `RmError`, `VerbReply`, `VerbPlan`, `RingWorkingSet`, `IsolateFactory` | none, by design: v3 has no isolates (V3_BUILD.md rules); `kf-host::RmError` |
| util | 5,354 (+200) | 13,629 | `trapwitness::OffTrap`, `leafwitness` (inside `rm.rs`) | `kf-util` (same modules) |
| abi | 47,231 (+5,334) | 8,516 (+13,273 in `kf_abi`) | `submit`, `bringup`, `generated::{classes,nvos}`, `invariant_classes` | `kf-abi` (96,614 lines; `hostabi`, `generated::matrix`) |
| chips | 4,725 (+1,585) | 1,590 | nothing directly (transitive via isolate-host) | `kf-chip` |
| doorbell | 8,007 (+2,136) | 1,111 | transitive via chips | `kf-trap` |
| arch | 3,081 | 167 | `ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa}` | `kf-arch::ids` |
| vmm | 952 | 46 | transitive | none needed |
| mmu | 10,961 (+2,344) | 0 | nothing | `kf-mem`, `kf-chip` |
| gsp | 6,021 | 0 | nothing | `kf-gsp` |
| mocks | 4,801 | 0 | nothing | none needed |
| trace | 1,511 | 0 | nothing | `kf-trace` |
| completion | 788 | 0 | nothing | `kf-chan`/`kf-core` |
| cuda | 3,259 (+765) | 0 (feature off) | only `--cuda-window`, which is **not one of the 30 arms** | `kf-cuda` |
| device | 27,596 (+22,138) | 0 | dead declared dependency, two comments | #214 moves it |

Line counts are MEASURED (`wc -l` of `*.rs` under `src/`; tests = other `.rs`). **INFERRED reachable slab**: ladder
19.4k + `isolate-host` {rm 15.9k non-test, osevent 1.7k, hostabi 1.5k, listobj 0.8k, fdcross 0.4k, export 0.5k,
guestram 0.4k, `proto` + `isolate` up to 5.7k} + `isolate` 5.8k + `linux-raw` ~12k + `util` ~1.5k + `abi` ~10-15k + `arch`
ids ~1k = roughly 75-85k lines of the 194k the closure declares. The rest of the closure is compiled and linked out.

### 2.3 What has no kf-* equivalent (the port's real work)

1. **`probe_va`** (`VaProbe`, `ProbeAperture`, `last_dma_step`): asks host RM what is at a VA
   (`VASPACE_GET_PAGE_LEVEL_INFO`). `kf-abi` has the control definition (`generated/matrix.rs`,
   `capability.rs`), `kf-host` has no verb. Needed by arms 10, 11, 27, 28, 20, 7 and the new stress arm's unmap
   postcondition. About 150-250 new lines on top of `HostRm::raw_control`.
2. **The old backend's composed verbs** (`alloc_probe_local`, `map_local_at(_with_flags)`, `unmap_local`,
   `alloc_channel_at`, `alloc_sysmem`, `fill_words`, `read_words_independently`, `submit_fenced_release`,
   `userd_cursors`, `channel_ring_va`). `kf-host` has the primitives (`alloc_device_local`, `map` with `at`,
   `unmap_range`, `birth_channel` with `RingSpec`, `raw_alloc`) but not the composed forms. A `rawclient::verbs`
   module of ~400-600 lines.
3. **UVM driver** (`uvm_raw` 781 + `mean` 1,709): `kf-*` has no `/dev/nvidia-uvm` client (UVM appears only as
   constants and in `kf-chan` plumbing). Wave C.
4. **Child process, fd passing, mini KVM** (`bar1-crossing`; `spawn_unsafe`, `scm_unsafe`, `write_frame_with_fds`):
   `kf-linux-raw` has `kvm_unsafe`, `vcpu_unsafe`, `unixsock_unsafe`, `host_fd_unsafe`, no spawn/sandbox.
5. **The R33 ioctl census wrapper** (`print_ioctl_census`): `kf-linux-raw::census`/`ioctltrace` exist; the printing
   is client code.
6. **`GuestWindow`/`guestram`** for `ce-client-guest-ram`: `kf-linux-raw::window_unsafe` exists; the arm's usage is
   not ported.
7. For the **stress arm only**: anonymous region with `MADV_NOHUGEPAGE` (`kf-linux-raw` has `request_huge_pages`, the
   opposite call; a new wrapper in `mapping_unsafe.rs` is the only new unsafe), and a `/proc/self/pagemap` reader
   (safe `std::fs`, no unsafe).

---

## 3. How CI uses the raw client (the drop-in contract)

### 3.1 Who builds and runs it

| Consumer | How | Where |
|---|---|---|
| Box merge bar (gpu-vast) | `cargo build -q --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder`; `CLIENT`/`KF_LADDER` = `$CARGO_TARGET_DIR/release/kayfabe-rm-ladder`; `bare_metal_suite.sh <tag> 180`; thin guest `build_fast_guest.sh`; `fast_suite.sh <tag> 180` | `scripts/bench/box/merge_check.sh:68-77` |
| Box provisioning | builds the client; `LADDER_RC=0` is required evidence | `scripts/bench/box/provision_full.sh:55`; `vast_payload.py:60-64` |
| Bare suite | one process per arm: `timeout --kill-after=5 $BUDGET $BIN $arm ${KF_LADDER_ARGS:-}`; rc 0 PASS, rc 124/137 CRASH, other FAIL | `bare_metal_suite.sh:91-99` |
| Thin guest | binary copied to `/bin/rmladder`; init runs `/bin/rmladder --gpu $g $ARMS` (arms from kernel cmdline `KF_ARMS=<arm>,--probe-launch-dma`); prints `FASTGUEST: client rc=N`; the host reads that rc | `build_fast_guest.sh:54-59,317,524,546`; `run_fast_guest.sh:336-339` |
| Driver matrix | `cargo build --release --target x86_64-unknown-linux-musl -p kayfabe-rm-ladder --bin kayfabe-rm-ladder`; static musl in the guest | `scripts/drivermatrix/sweep.sh:390-393`; `guest_walk.sh:27` |
| Axis cell | builds and exports `KF_LADDER` | `scripts/fastguest/axis_cell.sh:64-68` |
| Release | `cargo build --locked --release --workspace`; **attests `target/release/kayfabe-rm-ladder`** (and `kayfabe-isolate-host`, line 42: UNSURE that path exists; the crate's bins are `kayfabe-isolate` and `kayfabe-sandbox-probe`) | `.github/workflows/release-build.yml:36,42-43` |
| CI `stable` | `cargo build --workspace` builds it; `scripts/ci/test_dependencies.py:13` names it | `.github/workflows/ci.yml` |
| Hooks | ~14 more bench scripts and 14 Windows host-tool scripts name the binary | MEASURED: 47 files outside docs match `kayfabe-rm-ladder` (35 outside `windows/host_tools`) |
| Docs | `docs/howto/building.md:90-94` documents the two build commands | must change with any rename |

### 3.2 The interface a replacement must keep

1. **Crate/bin name and build commands** as above, native and `x86_64-unknown-linux-musl` (static, no dlopen).
   `kayfabe-isolate-host/build.rs` embeds a nested static isolate today; a `kf-*` crate has no such build step.
2. **CLI**: `--gpu N` then exactly one arm flag; the guest appends `--probe-launch-dma`
   (`native_xid_guard.py:233-242` requires exactly two comma-separated tokens, first matching `--[a-z0-9-]+`, second
   `--probe-launch-dma`). `KF_LADDER_ARGS` is appended on bare metal. `vast_payload_hooks.sh:15-25` finds the raw client
   by an argument ending `*/kayfabe-rm-ladder` and takes the first `--*` argument as the arm.
3. **Exit code is the verdict**: 0 PASS; any other FAIL; the harness maps 124/137 to CRASH. No suite parses client
   stdout for pass/fail.
4. **Process name**: `native_xid_guard.py:76,80,96` match the kernel's Xid line `name=kayfabe-rm-ladd` (the 15-char
   comm of `kayfabe-rm-ladder`). Any binary whose basename starts with `kayfabe-rm-ladd` keeps it; any other name
   breaks the two admitted negative controls.
5. **Source pin**: `native_xid_guard.py:147-153` (`SUITE_FILES`) pins `crates/kayfabe-rm-ladder/src/main.rs` and
   the three `scripts/fastguest/*.sh` by git hash at the tested revision. Moving or splitting the file changes the
   pin list in the same PR.
6. **Printed markers read by the Xid guard** for exactly two arms: `FAULT_MARK=defer_liveness_prime arm=... va=...`,
   `DEFER_PRIME_FAULTS=3/3`, `DEFER_RIGCTL=PASS`, `DEFER_NEGCTL=PASS`, `RUNG_defer_liveness=PASS`, `info DL prime
   copy = Dead`, `info DL test copy = Live|Dead`, `DL rescue copy = Live`; `FAULT_MARK=victim_release va=...`,
   `RUNGCTL_missing_page=PASS`, `RUNG_missing_page=PASS`, `bystander before/after = Landed`, `info R3 victim = Lost`
   (`native_xid_guard.py:60-97`). Any port of arms 7 and 13 prints these byte for byte.
7. **Host Xid watch** around every arm (`vast_payload_hooks.sh`, `host_xid_watch.sh`): any new host Xid fails the run
   except the two admitted negative controls. This is also the free "no TDR-class stall" signal (§4.7).
8. **Counts**: the literal `30` in `vast_payload.py:103,108,112,117,118` (`BARE_SUITE_PASS=30 ... ARMS=30`,
   `FAST_SUITE_PASS=30 ... ARMS=30`, `BIRTH_CENSUS_OK arms=30`), `merge_check.sh:71,76`, `cloud_init_replay.py:191-203`
   and their tests/mutants (`test_vast_payload*.py`, `test_vast_merge_bar.py`, `payload_mutants.py`,
   `merge_bar_mutants.py`). **Adding a 31st arm to the default list means editing all of them in one PR.**
9. **Birth census** (`birth_census.sh`): every arm's QEMU log must show at least one USER channel birth and one CUDA
   thread posture line. UNSURE whether light arms satisfy this through the guest driver's own kernel channels or
   through their own; the stress arm births its own CE channel either way.
10. **Result row**: `kf-result.json` suites are `{name,pass,fail,skip}` (`VAST_PAYLOAD.md`); `raw_suite` and
    `thin_suite` are fixed names built by `merge_rows` (`vast_payload.py:97-120`).

### 3.3 How drivers and dies are selected, and where the architecture table is consumed

- **Dies (gpu-vast)**: `ci/vast-broker/defaults.json` `checkDies` = GA106, AD104, GA102, AD102 (per-PR check, one
  box per run); `sweepDies` = 21 classes (TU116 ... GB202, plus `CC_75/86/89/120`) for the optional sweep workflow
  (`gpu-vast-sweep.yml`, `--profile sweep`). The payload requires compute capability >= 750 and a host driver listed in
  `tools/drivermatrix/tags.txt` (`vast_payload.py`, `provision_grade`).
- **Drivers**: `tools/drivermatrix/tags.txt` lists the measured ogkm tags (36 lines, 29-30 tags per
  `V3_DRIVER_MATRIX.md`); `regen.sh` writes `kf-abi/src/generated/matrix.rs`. `scripts/drivermatrix/sweep.sh` swaps
  the host driver (`HOSTS`) and the guest driver (`GUESTS`) per row; `THIN_GUEST_RE=^580\.` limits the thin 30-arm
  suite to 580 guests.
- **The architecture table is not consumed by the client text**: the legacy client writes the *bench* struct layout
  and retargets per host driver through `kf_abi::hostabi` (`isolate-host/src/hostabi.rs:14-49`, CODE READ). Per-die
  facts are consumed by kf3 at realize (`kf_chip::DieId` from `MC_GET_ARCH_INFO`, `Family::from_arch`,
  `Family::host_classes(host_classlist)`) and by the sweep tooling (`SWEEP_ARCH ... src=pci.ids`, `matrix_table.py`).
  `kf_rm::consttab` says "No consumer is wired". **A new client is the first userspace consumer of `kf_chip` and
  `kf_abi` outside kf3**, which is the point of requirement (2).

---

## 4. The refresh-stress arm

Name `--refresh-stress` (`RUNG_refresh_stress=`). Goal: the measured Enscape refresh shape without Enscape. MEASURED
upstream (`traces/enscape_h_counts_20261029`, `enscape_g_costs_20261028`, SPECviewperf 15 Enscape 6, Windows guest,
RTX 4070): per refresh 12 or 24 map batches, 98,304 stitch pieces = 98,304 `MAP_FIXED` attempts in the larger window
(44,274 in the smaller), no unmaps and no fallbacks, whole-stitch 488.9 ms and 2,005.8 ms.

### 4.1 Shape

One refresh = 24 batches x 4,096 pages (4 KiB) = 98,304 pages = **384 MiB** of anonymous user memory whose guest
physical frames are scattered in mapping order; every batch is described to RM and mapped into one GPU VA space;
then **one** invalidate; then the unmap and free path. Several refreshes in a row, each on **fresh** backing, so a
stale translation from refresh r-1 points at pages that were returned to the kernel.

### 4.2 Fragmented memory, and the qualification that makes it count

1. `mmap(MAP_ANONYMOUS|MAP_PRIVATE|MAP_NORESERVE)` of 384 MiB, `madvise(MADV_NOHUGEPAGE)` (THP would give 2 MiB
   contiguous runs). New helper in `kf-linux-raw/src/mapping_unsafe.rs`.
2. **Pressure phase** (default on, bounded): map `X = min(0.45 x MemAvailable, 1 GiB)` anonymous, touch it
   sequentially, then `MADV_DONTNEED` a seeded random half of its pages, so the free lists hold holes spread over
   the guest's RAM. Then touch the workload's pages in a **seeded random permutation** (splitmix64 Fisher-Yates over
   the 98,304 indices; seed printed). Write each page's signature at the same touch. Release the pressure region
   only after qualification.
3. **Bounded**: peak anonymous memory `X + 384 MiB + 8 MiB staging` must be <= 0.8 x MemAvailable (read from
   `/proc/meminfo`); otherwise `NOTRUN` (rc 3), never an OOM kill. The thin guest has 2,048 MiB by default
   (`run_fast_guest.sh:201`, `NVKVM_RAM_MB`); the row sets 3,072 for this arm (host boxes have >= 34 GB,
   `defaults.json` `quality.ramMb`).
4. **Qualification (a workload that fails it decides nothing)**: read `/proc/self/pagemap` (8 bytes/page, 768 KB;
   PFN needs CAP_SYS_ADMIN, root in the guest init and in the box payload; rootless runs report `NOTRUN
   reason=pagemap-pfn-hidden`). For every batch, in **mapping order**: `runs = 1 + #{i : pfn[i+1] != pfn[i]+1}`;
   `run_ratio = runs/pages`. Required: all pages present, `AnonHugePages == 0` for the region (`smaps`), and
   `run_ratio >= 0.90` for every batch. The 0.90 floor is INFERRED from the smaller measured window
   (44,274 pieces / 12 x 4,096 pages = 0.9007; the page count of that row is itself assumed). Reported, not gated until
   calibrated: `max_run_len`, distinct 2 MiB GPA regions, GPA span.
   The printed line is `STRESS_QUALIFY pages=98304 runs=<n> run_ratio=<r> thp=off distinct_2m=<n> span_mib=<n>
   mem_avail_mib=<n> seed=<hex>`. Guest PFN is GPA >> 12, which is the unit the host stitch consumes; natively it is
   the host frame.
5. Unqualified, unsupported or too-little-memory returns **rc 3** with `RUNG_refresh_stress=NOTRUN reason=...`. The
   suites treat rc 3 as `NOTRUN` (§5.4), and the gpu-vast row fails if the stress row is `NOTRUN` after calibration:
   a gate that could not run is not green.

### 4.3 Pin and map (the same verbs `kf-host::HostRm` uses)

Per batch b of 24, over the 16 MiB sub-range of the region:
`HostRm::alloc_os_descriptor(region, offset = b x 16 MiB, len = 16 MiB)` (`NV_ESC_RM_ALLOC_MEMORY`, class
`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`, `PHYSICALITY_NONCONTIGUOUS`, `MAPPING_NO_MAP`; `kf-host/src/lib.rs:1987`), then
`HostRm::map(space, handle, MapBacking::SharedSlice, 0, 16 MiB, Some(va_b), defer = true)` (one
`NV_ESC_RM_MAP_MEMORY_DMA` with `NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE`, pinned to 4 KiB pages so FIXED is honoured
at every offset; `channel.rs:663`, `bringup.rs:719`). Batch VAs are consecutive 16 MiB windows in a band inside
`VaSpace` range (host-derived, not a constant). Variant `--stress-shape sliced`: 4,096 one-page `map` slices per batch
(98,304 `MAP_MEMORY_DMA` calls per refresh), the per-run shape, diagnostic only.
UNSURE: `MapBacking::Dedicated` versus `SharedSlice` for a fresh OS descriptor object; the host's own scatter path
(`map_scattered_through`) is the template and uses `SharedSlice`.

### 4.4 The one invalidate

After all 24 maps: **one** `HostRm::invalidate_tlb(space)` (`NV2080_CTRL_CMD_DMA_INVALIDATE_TLB`, `channel.rs:1253`),
the client-issued register-invalidate analog (on the guest side the BAR0 write of the MMU invalidate that kayfabe
traps). Variant `--stress-invalidate memop`: author a `MEM_OP_A..D` TLB invalidate in the oracle channel's pushbuffer
(`kf-abi/src/submit.rs:2647-2663`), the PDB-MEM_OP analog. Both are default-off knobs of the same arm; `ctl` is the
default. Nothing reads a mapped VA between the last map and the invalidate (a deferred PTE may not be live,
`--defer-liveness`; reading early would manufacture an Xid).

### 4.5 Correctness oracle: a map that "succeeds" but points at the wrong page must fail

- Every page holds its own signature: 512 identical 8-byte words `sig = splitmix64(seed ^ epoch<<48 ^ page_index)`.
- After the invalidate, a CE copy (a `kf_harness::CeRig` in the same VA space, completion by event fd, fence re-read
  only at a wake) copies a **sample** of whole pages into a staging buffer. Staging is a second small anonymous region
  (6 MiB for 1,536 pages) that is itself OS-described and mapped, so the CPU reads it with ordinary loads (no BAR1, no
  vidmem CPU view: Vast's BAR1 limit is a documented exception to avoid).
- Sample = per batch the first and last page plus 62 seeded random pages = 1,536 pages (1.56%) per refresh. The CPU
  checks all 512 words of each copied page against `sig(page)`.
- **The oracle must be able to fail**, in the arm itself: (i) `ORACLE_DECOY`: compare the staged data to a deliberately
  permuted expectation and require a mismatch; (ii) epoch: refresh r+1 reuses the same VA band on new backing, so a
  stale translation (invalidate not applied, a host stitch that kept r's pieces) returns r's signature or garbage;
  (iii) after the CPU rewrites K=32 sampled pages post-map, the GPU must see the new words (the PTE points at the live
  page).
- **Unmap postcondition**: after unmap + invalidate, a `probe_va` of sampled VAs must report Free (as `--map-stress`
  does today); the arm **never copies from an unmapped VA** (that is an Xid 31 and a red run for the wrong reason).
- **Bystander window** (`--stress-bystander`, default on): one more 16 MiB window in the same VA space, mapped once
  before refresh 1 and never touched by the refresh, read by a second `CeRig` thread at ~100 Hz during maps, invalidate
  and unmaps. Any wrong word or any fault fails the arm. This is the memory rule "unchanged VAs stay accessible during
  refresh" (`feedback_va_manager_unchanged_va_invariant`) made executable. INFERRED value; UNSURE that a second channel
  does not perturb the timing being measured, so the first calibration runs it both ways.

### 4.6 Parameters derived from the architecture table, never hardcoded per die

| parameter | source | refusal |
|---|---|---|
| GMMU small page (grain, 4 KiB) | `kf_chip::MmuFormat::small_page_bytes()` via `Family::from_arch(HostRm::arch_info())`; must equal the kernel page size (`HostPageSize::query()`) | unknown family: use the host-reported facts below and print `family=unknown`; a grain that is not 4 KiB: `NOTRUN` |
| CE class and engine | `HostRm::ce_class_id()` (derived from the host class list by `kf_chip::choose_host_classes`), `ENGINE_TYPE_COPY0`, `engine_type_copy(0)` | host lacks a CE class: `NOTRUN reason=no-ce` |
| channel class, usermode class | `HostRm::open(&dev, gpu, &kf_chip::choose_host_classes)` | `HostLacksKind` by name |
| VA window | `VaSpace` range from `alloc_vaspace` | none |
| wire layout per driver | `HostRm::host_abi()` (`kf_abi::hostabi`), `kf_abi::versions` | tag not in `tags.txt`: refused by name = `NOTRUN` |
| memory aperture and coherency | chosen inside `alloc_os_descriptor` (`PCI`, `COHERENCY_CACHED`) | none |

Every run prints `RAWCLIENT_CELL die=<DieId name or arch:impl> family=<Family|unknown> mmu=<Ver2|Ver3>
drv=<tag> kmod=<open|proprietary> mode=<bare|guest> rev=<sha>` (the cell identity `bare_metal_suite.sh` builds from
`nvidia-smi` today, here derived from the host's own answers). Thresholds are **not** per die (§4.7): the same arm and
the same ratio rules run on every die and driver; dies and drivers enter only through the cell line.

### 4.7 Metrics and gates

Printed (machine readable, one line each):

- `STRESS_REFRESH n=<i> pin_ms map_ms inval_ms oracle_ms unmap_ms free_ms critical_ms max_call_ms` where
  `critical_ms = map_ms + inval_ms` (the span a WDDM paging operation is blocked) and `max_call_ms` is the slowest
  single RM call or fence wait in that refresh.
- `STRESS_HIST op=map|unmap|pin|inval n=<n> p50_us p90_us p99_us max_us log2_buckets=...` (24 map calls per refresh
  x 3 refreshes = 72 samples whole-shape; 98,304 in the sliced diagnostic).
- `STRESS_RATIO` is computed by a pure subcommand `--refresh-compare native.log guest.log` (GPU-free, unit-tested),
  never by the client itself, because the client in one process cannot know the other run.
- Host side, existing and optional: with `KF3_MAPLOG=1` the QEMU log carries `APPLIED ... +n -m` and `CLEAR inval ...
  (arrive->clear N us)` (`kf-mem/src/vasmgr.rs:1088`); the compare step reads them as corroboration, not as a gate.

Hard caps for a paid gate: **target <= 15 s native and <= 60 s in the guest including setup**, memory as §4.2.3,
three refreshes (adaptive: stop starting new refreshes at `--stress-deadline-s`, default 50; fewer than 2 completed
refreshes is a FAIL `slow`), a **watchdog** (a thread with a deadline; on expiry print phase, refresh, batch and the
RM call in flight, then `exit(124)`, which every suite already scores CRASH) of 10 s per single phase. Cleanup is
process exit: no files, no sysfs, no `drop_caches`, no `swapon`, no mlock.

Gates (PASS requires all):

- **G0 qualified** (else rc 3).
- **G1 oracle**: zero mismatches in every refresh, `ORACLE_DECOY` detected, epoch and live-page checks, unmap
  postcondition Free, bystander clean.
- **G2 no stall**: no single RM call or fence wait over 10 s (hang class); **zero new host Xid** across the command
  (the existing `host_xid_watch`; the stress arm is not admitted any Xid); the guest `dmesg` is read by the harness
  for `NVRM: Xid` as for every arm. 2 s is WDDM's default TDR delay; the arm records `stall_over_2s=<n>` but does
  not gate on 2 s absolute, because a nested Vast box inflates guest time by an unknown factor.
- **G3 ratio** (**flagged: needs calibration, report-only until then**): `guest critical_ms median / native critical_ms
  median <= K1` and `guest map p99 / native map p99 <= K2`. Prior anchors, all different quantities: existing
  per-arm guest/bare ratios 0.4x-22x (§2.1); VM-exit cost 10-40x bare metal on Vast (AGENTS.md); the real-host
  Enscape target apply of 2.68 s for 98,304 pieces against a 2 s TDR. My starting guess is K1 = 30, K2 = 40, **with
  no data behind it**. Calibration (S8): at least 10 runs across at least 4 dies (the normal gate stream, §5), set
  `K = 1.5 x observed P95`, and require the coefficient of variation of the ratio under 0.30 (else the ratio cannot
  gate; falsifier F4).
- **A/B (report only, owner decides)**: the same guest arm under `KF3_NO_BATCHED_MAP=1` (per-run path;
  `kf-qemu/src/mem.rs:1260`, read by the QEMU process, so a second boot), batched, and the file-less RAM mode of
  #213. Output `STRESS_AB batched_ms perrun_ms nofd_ms`.

### 4.8 What it must NOT do

No allocation beyond §4.2.3; no unbounded loop (every loop has a count and a deadline); no `fork`; no persistent
state; no privileged channel (the CE rig births USER channels like every gate: `PRIVILEGED_CHANNEL=0`); no flag
required for a normal run (the knobs above only change diagnosis); no deliberate fault; no read of a VA it has
unmapped; no pressure outside its own anonymous memory; no timing-dependent verdict inside the oracle (waits are
bounded failure detection, results come from fences and data); no use of `KF3_*` host flags by the client.

### 4.9 Falsifiers and limits

- **F1 shape**: under `KF3_MAPLOG=1`, one refresh's `APPLIED` lines must show about 98,304 single-page rows per
  refresh through the batched path (not `batch_no_fd`). If a Linux guest RM coalesces, writes 64 KiB or 2 MiB PTEs
  for these pages, or the VA manager sees fewer larger rows, the arm does not reproduce the Windows input and is
  redesigned or dropped. This is S3 and runs before any gate.
- **F2 power**: against a kayfabe revision known to stall on Enscape, or with `KF3_NO_BATCHED_MAP=1`, the arm must
  be measurably worse (ratio difference beyond run-to-run noise). If batched and per-run are indistinguishable, the
  arm cannot see the stitch and says nothing about it.
- **F3 oracle can fail**: the decoy check must trip in every run, and a CPU-injected wrong signature must fail the
  arm in the GPU-free selftest.
- **F4 noise**: coefficient of variation of the guest/native ratio over 5 runs on one box above 0.30 means the ratio
  gate is not usable; fall back to the absolute caps and the Xid watch only.
- **F5 coverage**: if `NOTRUN` on the thin guest is common (memory, pagemap, THP), the row is not coverage.
- **What a synthetic cannot prove about Windows**: WDDM's paging-node scheduling and the 2 s TDR timer itself;
  dxgkrnl locks held while the guest polls the MMU invalidate trigger bit (the root cause recorded for the earlier
  hang, `project_windows_hang_root_cause_20261009`); VidMm's own PTE writing path (the Linux RM writes PTEs here,
  WDDM's kernel driver there, so the *mechanism* differs even if the host-visible rows match); Windows' free-page
  distribution and its VA layout (multi-GB windows, 64 KiB and 2 MiB pages); concurrent 3D, copy and paging nodes;
  DWM and the real application's allocation churn; the NVIDIA Windows driver's own invalidate sequence.
  **Enscape remains the final acceptance.** A green synthetic with a red Enscape is expected to be possible; a red
  synthetic is a real finding.

### 4.10 Size

About 1,100 lines of arm code (memory builder 150, pagemap and census 150, orchestration 300, oracle 200,
metrics 150, gates and compare 150) plus about 300 lines of GPU-free tests (pagemap parser, run census,
histogram, permutation determinism, gate logic, compare tool, decoy), plus the `probe_va` helper (200) and the
`kf-linux-raw` additions (about 60 lines of unsafe, 40 safe). INFERRED.

---

## 5. The strangler plan

### 5.1 Shape of the new crate

`crates/kf-rawclient` (adapter crate, not in the PURE set of `scripts/ci/dependencies.py`; may depend on `kf-host`,
`kf-arch`, `kf-chip`, `kf-abi`, `kf-linux-raw`, `kf-util`; **never** on a `kayfabe-*` crate). `src/lib.rs` is the
framework: `Arm { name, run(&Ctx) -> Outcome }`, `Outcome::{Pass, Fail(why), NotRun(reason)}`, a registry built from
`ARMS.tsv` (arm name, status `legacy|shadow|new`), a ledger in the style of `kf_harness::Ledger` (`CHECK name
PASS|FAIL why`) that also prints the legacy-compatible `RUNG_<name>=` / `RUNGCTL_<name>=` tokens, the cell line
(§4.6), the exit-code contract (0 / 1 / 3 NOTRUN / 124 watchdog), `--list-arms`, and a GPU-free unit-test layer.
`CeRig`, `ce_copy_push`, `host_fence_push` and `Ledger` move from `kf-harness` into `kf-rawclient::rig` (kf-harness
re-exports them), so the client does not pull `kf-mem`/`kf-chan`/`kf-cuda` into a static musl build. UNSURE whether
the move is worth it versus depending on `kf-harness` for the first steps; `kf-syncwait-stress` already ships to the
guest from `kf-harness` through `KF_TOOL_BIN`, so a dependency works for S4 at no cost.

**The facade.** The new binary is named `kayfabe-rm-ladder`. The old crate's `[[bin]]` is renamed
`kayfabe-rm-ladder-legacy` (the only edit to the frozen crate; `comm` stays `kayfabe-rm-ladd`, so the Xid guard's
`name=` match still holds for the admitted arms while they are legacy). For an arm whose status is `legacy` the new
binary `exec`s the legacy binary found beside `current_exe` with the same argv: same pid (so `timeout` and the Xid
hook see one process), same exit code. For `new` it runs in process. Alternative considered and not recommended:
leave both names and teach every script to route per arm (about 47 files).

### 5.2 Ordered steps

| step | what | new / changed lines | days | risk |
|---|---|---:|---:|---|
| **S1** | `kf-rawclient` skeleton: registry, `Outcome`, ledger, cell line, rc contract, `--list-arms`, facade `exec`, `ARMS.tsv`, a GPU-free test that `ARMS.tsv` equals the 30-name default list of `fast_suite.sh`. Not wired to any script. | ~900 | 1 | low |
| **S2** | `kf-linux-raw`: anon region + `MADV_NOHUGEPAGE` (unsafe, one `*_unsafe.rs` file; the unsafe-count ratchet `scripts/ci/test_unsafe_count.py` moves with its itemisation), pagemap reader (safe). `probe_va` helper. | ~350 | 1 | low |
| **S3** | **Stress arm** (§4) as a registry arm, run through the **existing `KF_TOOL_BIN` lane** in the thin guest and natively. No CI edit. First one-box run with `KF3_MAPLOG=1`: F1, F2 (A/B), F3. Evidence under `traces/`. | ~1,400 | 3 | **high** (premise F1; nested timing) |
| **S4** | Port `--timer`, `--engines`, `--doorbell-census` (old 436 + 89 + 289 lines; `kf-host` has `open_timer`, `usermode_view`, `gpu_time_ns`, `birth_channel`, `raw_control`, `kf-harness/bin/kf-timer-probe.rs` is already a 51-line native timer probe). `doorbell-census` is also the per-die discriminator, so it exercises `kf_chip`. Parity script (§6) and shadow mode. | ~600 | 2 | low-medium |
| **S5** | Wiring: rename legacy bin; build both in `merge_check.sh:68`, `provision_full.sh:55`, `axis_cell.sh:64`, `provision_bare_metal.sh:48`, `sweep.sh:393`; `build_fast_guest.sh` copies both (`/bin/rmladder`, `/bin/rmladder-legacy`); `release-build.yml:42-43` attests both; `native_xid_guard.py` `SUITE_FILES` gains the new sources; `docs/howto/building.md:90-94`; fixtures in `cloud_init_replay.py`; mutants. | ~250 changed | 1.5 | medium (CI fixtures and mutation harness) |
| **S6** | gpu-vast integration of the stress row, **advisory**: `merge_check.sh` stage after the thin suite; `merge_rows` gets a `refresh_stress` row (pass/fail/skip); `--refresh-compare`; rc 3 -> `NOTRUN` mapping in `bare_metal_suite.sh`/`run_fast_guest.sh`; `NVKVM_RAM_MB=3072` for that row; three variants for the A/B. The 30-arm count stays 30. | ~700 incl. tests | 2 | medium |
| **S7** | Calibration from the **normal gate stream** (no extra rentals): at least 10 runs, 4 dies, then set K1/K2 and flip the row to gating (a one-line change plus tests). Owner confirms. | ~50 | calendar 1-2 weeks | medium (CV of the ratio) |
| **S8** | **Wave A** (15 arms, old ~3,000 lines): gpu-info-sweep, bus-info-sweep, atomics-probe, pce-mask-probe, concurrency, blockage-coverage, alias-two-vas, alias-unmap-observe, map-propagation, late-map-race, dictated-ring, dictated-ring-negative, guest-ring-channel, executor-vas, guest-ram-pin. Needs `rawclient::verbs` (the composed verbs, §2.3.2). | ~2,400 | 5-7 | medium |
| **S9** | **Wave B** (8 arms, old ~6,100 lines): ce-client, ce-client-guest-ram, map-stress, cross-client-leak, rpc-mixed-allocs, concurrent-fuzz, and the two Xid-admitted arms missing-page-fault and defer-liveness (2,885 lines; prints byte-identical markers, §3.2.6; `comm`/pin updated in the same PR). | ~4,500 | 6-8 | **high** for the two admitted arms |
| **S10** | **Wave C** (4 arms, old ~3,200 lines): bar1-crossing (mini KVM, fd passing), uvm-invalidate and uvm-mean (a `/dev/nvidia-uvm` client), gpga-reserve-probe. | ~2,500 | 4-6 | medium-high (no substrate) |
| **S11** | Retire: delete the facade `exec` and `ARMS.tsv` status column, `git mv` `kayfabe-rm-ladder` and the 15 closure crates to `archive/crates/`, remove them from `Cargo.toml`, `ci.yml` (the `pure=`/`portable=` lists, lines 342-346 and 396-401, the nested-isolate target note), `release-build.yml:42-43`, the debt files (lines mentioning `kayfabe-`: `frozen-debt.json` 37, `clippy-debt.json` 179, `claims-debt.json` 839), `test_dependencies.py`, 47 scripts; rename nothing the guard matches without updating `native_xid_guard.py`. Same recipe as #214. | ~300 changed | 1-2 | low-medium (mechanical) |

Totals (INFERRED, +-50%): S1-S7 about 4,250 new lines and 11 working days (plus the 1-2 week calibration wait);
S8-S10 about 9,400 new lines and 15-21 days; S11 about 1-2 days. **Overall about 14,000 new lines and 27-34 working
days**, dominated by GPU-box verification, not typing. Wall-clock is bounded by the daily gate budget
(`defaults.json` `daily`, `dailyCount`) and the weekly usage rule.

Optional accelerator (decide with the owner, §8 Q3): before S8, vendor only the **reachable slab** (§2.2, ~80k lines)
into the one frozen crate and archive the other 14 at once. Cuts the archive horizon from S11 to about S5 for roughly
2 days of mechanical work. Conflicts with "move it into the live tree" only in timing; the strangler still ends in
`kf-rawclient`. UNSURE whether V3_BUILD.md's copy rules apply to frozen-to-frozen moves (they bind code entering
`kf-*`).

### 5.3 What is archived when

- **Now (#214)**: `kayfabe-device`. 15 crates and the grader remain.
- **S1-S10**: nothing, because the legacy binary still builds from all of them. Each step is independently
  revertible by flipping the arm's `ARMS.tsv` status back to `legacy`.
- **S11** (or the accelerator after S5): everything else. The frozen tree's last consumer is gone when the last arm
  is `new` and one full gpu-vast run is green without the legacy binary present.

### 5.4 CI integration points (the list S5-S6 edit)

`merge_check.sh` (build, the two `grep` of counts, a new stage); `provision_full.sh`; `axis_cell.sh`;
`provision_bare_metal.sh`; `provision_bench_tree.sh`; `sweep.sh`; `guest_walk.sh`; `failure_point.sh`;
`build_fast_guest.sh`; `bare_metal_suite.sh` (rc 3); `run_fast_guest.sh` (rc 3, `NVKVM_RAM_MB` per arm);
`vast_payload.py` (`merge_rows`, new row; **not** the 30s); `vast_payload_hooks.sh` (keeps matching
`*/kayfabe-rm-ladder`); `native_xid_guard.py` (`SUITE_FILES`); `release-build.yml`; `ci.yml` (S11);
`cloud_init_replay.py` and the payload tests/mutants; `docs/howto/building.md`; `AGENTS.md` Layout.
Everything that prints the literal `30` stays untouched until the owner decides whether the stress row is a 31st
suite arm (Q4).

---

## 6. Old-versus-new verification protocol

Applies to every ported arm (S4, S8-S10) and to nothing else.

1. **Same box, same boot, same revision**: both programs built from one commit; the legacy binary is
   `kayfabe-rm-ladder-legacy`; one host driver; one `kf3` build (`kf3-bins/<rev>/`). A bench claim carries its
   source revision (AGENTS.md).
2. **Runs**: per arm, per die, per mode: **N = 5 interleaved pairs** (L, N, L, N, ...) on bare metal, and N = 5 each
   in the thin guest (one arm per boot, as `fast_suite.sh` does, so the guest legs are 10 boots per arm per die;
   legacy first on odd pairs). Dies: at least GA106, AD104 and GA102 (`checkDies` minus AD102) plus TU116 when a
   Turing box is available; drivers: the host's current driver plus, for the guest, one older measured tag.
3. **Identical verdict** means all of: (a) equal exit code; (b) the same set of `CHECK`/`RUNG_`/`RUNGCTL_` tokens with
   the same PASS/FAIL (per-arm mapping file `parity/<arm>.map`, because the new ledger names checks); (c) the same
   host Xid delta (count and decoded fields) via `host_xid_watch.sh`; (d) the same birth-census counts;
   (e) for the two admitted arms, byte-identical marker lines (§3.2.6). Run time is recorded (ratio within 0.5x-2x),
   not gated.
4. **Detection parity**: "both pass" is not parity. Each ported arm names one way it must fail: its own existing
   negative control where it has one (`dictated-ring-negative`, `defer-liveness`, `missing-page-fault`), otherwise a
   GPU-free selftest with a planted wrong answer, plus, where the arm is a discriminator (`doorbell-census`,
   `pce-mask-probe`, `atomics-probe`), the same discriminating value on at least two families.
5. **Shadow mode, bare metal only**: `KF_RAWCLIENT_SHADOW=1` makes the facade run the legacy program, then the new
   one, print `PARITY arm=<a> legacy_rc=<n> new_rc=<n> identical=<0|1>` and return the legacy rc. This gives parity
   evidence on every normal bare-metal merge-check for free. It is **not** used in the guest: the thin-guest suite
   runs one arm per boot on purpose (`fast_suite.sh:3-9`, a killed arm poisons its successor).
6. **Promotion**: an arm's status becomes `new` only after the protocol above shows zero divergence for the
   whole matrix, and the commit that flips it carries the evidence path (`traces/rawclient_parity_<date>/`, one
   `PARITY ... n=5 identical=1` line per cell). The legacy implementation stays buildable until S11. A flip is
   reverted by editing one line.
7. **Falsifier of the protocol itself**: run the protocol once with a **planted divergence** in the new arm (a flag
   that makes it return PASS on a failing legacy case, test-only feature, never shipped) and require the parity
   script to report `identical=0`. A parity checker that cannot say "different" proves nothing.

---

## 7. Risks (top, with what would show them)

1. **Premise F1** (§4.9): Linux guest rows are not the Windows shape. Shown by S3 on one box in about half a day.
2. **Ratio noise on nested KVM** (F4): the gate degrades to caps plus Xid watch. Shown by the S3/S7 numbers.
3. **Two Xid-admitted arms depend on exact markers, comm and a source pin** (§3.2.4-6): a silent change turns a
   negative control into a red run or, worse, a green one. Mitigation: they are ported last, with the guard's own
   tests and mutants in the same PR.
4. **Literal 30 everywhere** (§3.2.8): a partial edit is the typical failure. Mitigation: keep 30 until Q4; a
   GPU-free test that every `30` in the list of files matches `ARMS.tsv`.
5. **Substrate that does not exist** (§2.3): `probe_va`, composed verbs, UVM client, child/fd. The estimate for S8-S10
   is the least certain number in this document.
6. **`kf-host::HostRm::open` has been run in a guest only through `kf-syncwait-stress`** (a graphics-channel tool). A
   guest-side CE/map workload through `HostRm` is new. UNSURE; S3 finds out.
7. **Unsafe ratchet and musl**: the one new unsafe wrapper moves `scripts/ci/test_unsafe_count.py`; a static musl
   `kf-rawclient` must not pull `kf-cuda`/`kf-mem` (§5.1).

---

## 8. Open questions for the owner (NEEDS-OWNER)

1. **Facade naming** (§5.1): new crate's binary is `kayfabe-rm-ladder` and the old one becomes
   `kayfabe-rm-ladder-legacy` (recommended; zero script churn, one edit to a frozen manifest), versus two names and a
   per-arm router in about 47 files.
2. **Crate name and placement**: `kf-rawclient` (recommended), or extend `kf-harness` (whose `kf-gate*` bins are
   gates, not arms, and whose dependencies pull `kf-mem`/`kf-chan`/`kf-cuda`).
3. **Accelerator** (§5.2): vendor the reachable slab into one frozen crate and archive 14 crates early, or wait for
   S11.
4. **Is the stress row a 31st arm or its own suite row?** Own row (recommended: the 30/30 contract and its ~9 hardcoded
   places stay stable, the row can start advisory and carries a NOTRUN state), or a 31st arm (one PR editing every `30`).
5. **When does the ratio gate?** Advisory until at least 10 runs on 4 dies show CV < 0.30 (recommended), or gate
   immediately on absolute caps only.
6. **Is an unqualified stress run (`NOTRUN`) a red gpu-vast result?** Recommended yes after calibration.
7. **Memory for the thin guest**: 3,072 MiB for this row only (`NVKVM_RAM_MB`), or raise the default for all arms.
8. **Windows**: should the same arm shape be offered as a Windows guest tool later (an Enscape-free reproducer for
   the WDDM path), or is the Linux synthetic the only target?

---

## 9. Sources

Code: `crates/kayfabe-rm-ladder/src/main.rs`, `crates/kayfabe-isolate-host/src/{rm,hostabi}.rs`,
`crates/kf-host/src/{lib,channel}.rs`, `crates/kf-harness/src/lib.rs`, `crates/kf-chip/src/{lib,die}.rs`,
`crates/kf-rm/src/consttab.rs`, `crates/kf-abi/src/bringup.rs`, `crates/kf-linux-raw/src/mapping_unsafe.rs`.
CI: `.github/workflows/{gpu-vast,gpu-vast-sweep,release-build,ci}.yml`, `scripts/bench/box/merge_check.sh`,
`scripts/fastguest/*`, `scripts/drivermatrix/sweep.sh`, `scripts/ci/vast_payload{,_hooks}.{py,sh}`,
`scripts/bench/native_xid_guard.py`, `ci/vast-broker/defaults.json`. Evidence: `traces/sw5_die_sweep_20261025/`,
`traces/enscape_h_counts_20261029` and `traces/enscape_g_costs_20261028` (on `origin/codex/enscape-tdr-run-20261028`, not yet on `next`). Related: `V3_BUILD.md`, `V3_BATCHED_MAP.md`,
`V3_DRIVER_MATRIX.md`, `V3_CONSTANT_STRATEGY.md`, PR #214.
