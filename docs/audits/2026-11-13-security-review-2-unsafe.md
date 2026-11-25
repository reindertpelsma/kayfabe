# Security review 2 of 4: the unsafe Rust and the C device

STATUS: RESEARCH, 2026-11-13 (committed 2026-11-25; scrubbed copy). Point-in-time report, not maintained.

> Findings status in this report is as of 2026-11-13 at master `921c39444`. Current status of each finding is in the
> coverage table of the [stage-2 audit](2026-11-23-v3-stage2.md). Scrub: local paths, worktree names, host and account
> details and references to working notes or proof-of-concept files that are not in the repository were removed or
> replaced by "not retained"; the technical findings, severities, code locations and reasoning are unchanged.

Part status as of 2026-11-13: Security review 2 of 4: the UNSAFE code. Read-only review of origin/master `921c39444`. Nothing pushed, no PR, no hardware, no Vast, no product code changed. Notes, PoCs, logs: (not retained).

# Security review 2: the safe-to-unsafe boundary

## 0. Verdict

The hypothesis "every safe-to-unsafe boundary is validated, so a buggy or hostile safe caller can at worst panic" is **FALSE on master**. No guest breakout was demonstrated (nothing here is rated CRITICAL as a proven path), but about 54 unsafe sites rest on a precondition kept only by safe callers and about 14 have no check at all. The largest gap is `kf-cuda` (33 of 79 sites caller-validated; its real bounds live in the safe files `display.rs` and `walk.rs`). Five findings become CRITICAL under the owner's breakout rule if a stated precondition is met (section 3, "conditional CRITICAL"). (2026-11-13)

Labels: **MEASURED** = a PoC, test or tool was run here. **CODE READ** = read at the cited lines. **INFERRED** = reasoning, not observed. Nothing in this review ran on a GPU or in a guest.

## 1. Falsifier (written before the review)

(not retained): twelve conditions F1-F12, any one of which shows the hypothesis wrong (unchecked argument reaching an unsafe op, wrapping size arithmetic, unowned fd, reference over guest-writable memory, view outliving its mapping, unjustified Send/Sync, compiled-out precondition, ioctl length not tied to the buffer, FFI pointer without check/lifetime, classic C defects, invalid zeroed/transmute, signal/static misuse).

Result: F1 hit (D-01, D-02, D-03, B-01, P-02), F3 hit (C-03, D-07, D-15, C-09), F5 hit (A-01, C-02, D-04, D-05), F6 hit (A-02), F8 hit (B-02 = S1-40, B-03), F9 hit (C-01, C-04), F10 hit (C-05, C-07, F-04). F4 (reference over guest memory), F7 (debug_assert-guarded precondition), F11, F12: **not hit** (the one `debug_assert_eq!` in the perimeter, `mapping_unsafe.rs:621`, guards no unsafe precondition).

## 2. Method and what was measured (2026-11-13)

- Seven parallel passes, one per area, each producing a row per unsafe site (location, operation, SAFETY contract, safe callers and arguments, what validates each argument, verdict). The row tables are appended in full (Appendix A-G) and are also the files `A_...md` to `G_...md` in the notes directory.
- MEASURED: PoC A (5 arms: stale `HostSpan` compiles and runs, safe data race on `VolatileRegion`, SIGSEGV after `reprotect`, SIGBUS on short file x2); PoC B (`CharDevice::ioctl(FIONREAD, &mut buf[..1])` returns Ok and the kernel wrote 3 bytes outside the slice); PoC D (safe `view_bytes` exposes padding); `cargo test -p kf-cuda --lib` 53 pass; broker built with ASan+UBSan passes its 5 unit binaries, `selftest.sh` (91 checks) and 6 socket suites with no report; `gcc -fanalyzer -Wconversion` and `clang --analyze` on 12 broker files: 0 warnings; broker log flood (15000 WINDOW commands -> 15002 log lines in 0.83 s); clippy cast/arithmetic lints on the three crates (`clippy_*.log`, `clippy_unsafe_hits.txt`: 20 cast hits and 41 unchecked-arithmetic hits inside perimeter files, each read by the area pass; none yields a finding beyond those listed).
- NOT possible: Miri (not installed for toolchain 1.99.0; not installed over the network), Kani, `-fanalyzer` on `kf3.c` (needs QEMU 10.2 headers; box has 9.2.0/11.1.1), any NVIDIA-driver or KVM measurement.

### Sites by verdict

| area | VALIDATED | CALLER-ONLY | MISSING | PANIC-ONLY |
|---|---|---|---|---|
| A kf-linux-raw mapping/window/host_fd (48) | 44 | 2 | 2 | 2 paths |
| B kf-linux-raw ioctl/KVM/sockets/signals (57) | 49 | 3 | 3 | (2 of the 49) |
| C kf-qemu ffi/raw + kf3.c (~56 rows) | 49 | 5 | 2 | 0 |
| D kf-cuda driver_unsafe.rs (79) | 40 | 33 | 3 | 3 |
| E broker C + kf-broker peer (42 entry points) | 38 | 2 | 2 | 0 |
| F firmware/kf-gop + tools + patches (~49) | 36 | 9 | 2 | 1 |
| P kf-chan `tspace_unsafe.rs` (GPU-address perimeter, 11 exports, no unsafe block) | 8 | 3 | 0 | 0 |

## 3. Findings

Severity follows the owner addendum: a guest breakout is CRITICAL regardless of step count; unbounded DoS counts unless a planned limit is documented. "safe-side" = needs a bug in a non-unsafe kayfabe file (the owner's scope assumes such bugs exist).

### 3.1 Conditional CRITICAL (no guest path demonstrated today; CRITICAL when the stated precondition holds) (2026-11-13)

| ID | sev | where | scenario | minimal fix | label |
|---|---|---|---|---|---|
| C-02 | HIGH / CRITICAL on memory unplug | `kf-qemu/src/raw_unsafe.rs:5-17` (`RawRegion: Copy`, no lifetime), `mem.rs:82-91,176-206`, retained at `chan.rs:766` | Copies of a guest-RAM region leave the `RamMap` read guard. After `kf3_ram_del` (operator memory hot-unplug, or eject of a device whose BAR is plain RAM) QEMU frees the block; kayfabe threads keep reading/writing that host address: use-after-free read/write of VMM memory steered by guest ring contents. No guest-only trigger found in the default launch (INFERRED). | One accessor holding the read guard across the copy (the existing `MemoryListRam` pattern); make `RawRegion`/`RamBlock` non-`Copy`; narrow the C filter to backend RAM. = S1-07, still open. | CODE READ; QEMU free INFERRED |
| D-03 | HIGH / CRITICAL if `(w,h)` ever diverges from the staging allocation | `kf-cuda/src/display.rs:864-870, 909-946, 1494-1533` | `compose_layer`/`compose_cursor`/`compose_yuv` bound GPU writes by the caller's `(w,h)`, never by the staging allocation. A safe-side mismatch under a guest-set mode gives a GPU write up to ~1 GiB past staging: the store, host-compositor slots, pinned frames, and VMM memory where unified addressing/HMM applies. kf-qemu passes one pair today. | Four-line length check against the allocation, as `color_yuv` already has; test that can fail. | CODE READ (I re-read `compose_layer`) |
| D-02 | HIGH (= S1-04, open) | `kf-cuda/src/driver_unsafe.rs:685-853` | Safe `pub fn` `mem_free`, `memcpy_*`, `memset_d8*`, `launch_raw` take any `u64` device address. Under unified addressing a page-locked host frame address is a valid target: a wrong address from safe code writes console/broker frames, the walker's pinned report, or guest VRAM. | Merge the conversion on `origin/v3-sec-rawaddr` (owned handles with length); it is not on master. | CODE READ |
| X-01 | HIGH (experiment, default off) / CRITICAL if enabled for a product | `kf-rm/src/gssnative.rs:39-65`, `kf-host/src/lib.rs:1190-1228` (`raw_control_opaque`), `kf-qemu/src/chan.rs:96-102` | With `KF3_GSS_NATIVE=1`, up to 64 KiB of guest-supplied, layout-unknown control parameters are forwarded verbatim to host RM for GSS-legacy subdevice commands (20 000 per boot). This is "forwarded, not authored" (ruling A.2). If any such control body carries a pointer field the host kernel honours, the guest names an address in the VMM (INFERRED; the bodies are undocumented, which is the problem). Ties to B-03: the ioctl wrapper has no table refusing undeclared pointer fields. | Keep it out of any shipped profile; record it in the residual-risk list; if kept, a per-command allowlist with known layouts only. S1-41 (no host allowlist) has REGRESSED through this path. | CODE READ; breakout INFERRED |
| S1-20 | CRITICAL if it fails (stage-1 blocker, open) | `kf-host/src/channel.rs`; closed GSP | The host boundary (USER channels refuse physical-mode and privileged work) has never been tested in v3; T-PHYS-CE was never run. | Run the box test on every family, both privilege arms. | docs + grep (ledger) |

### 3.2 HIGH

| ID | where | scenario | minimal fix | label (2026-11-13) |
|---|---|---|---|---|
| D-01 | `kf-cuda/src/driver_unsafe.rs:733-795, 1265-1305` | `launch_raw`, `launch_args`, `graph_exec_kernel_set` check neither parameter count nor sizes; the SAFETY comment cites an assert that does not exist. A short `params` list from safe code makes libcuda read host heap past the pointer array and dereference it. In-tree launches match the PTX today (MEASURED by signature comparison). | Store the entry's parameter sizes in `Func`; assert count and sizes before `cuLaunchKernel`. | CODE READ (re-read by me) |
| B-02 | `kf-linux-raw/src/chardev_unsafe.rs:365-380, 689-735` | = S1-40, **not fixed**. `paramsSize`, NVOS02 `limit` and the nested list count are set by the safe caller; a value larger than the `Indirect` buffer makes host RM `copy_to_user` past a Rust allocation. kf-host's builders are consistent today. | The wrapper writes (or verifies) each size field from the real buffer length. | CODE READ |
| B-03 | `chardev_unsafe.rs:565-741` | Pointer fields not declared as `Indirect` go to the kernel unchanged. kf-host zeroes its own; the forwarded payload of X-01 is the one place guest bytes reach an RM payload. | A per-escape pointer-field table in the unsafe file that refuses a non-zero undeclared pointer. | CODE READ |

### 3.3 MEDIUM

| ID | where | scenario | fix | label (2026-11-13) |
|---|---|---|---|---|
| A-01 | `mapping_unsafe.rs:647,1022-1075,1196`; `window_unsafe.rs:189` | `HostSpan` is `Copy`, lifetime-free, minted by safe `host_span()`. Safe code can drop the mapping and hand the span to `kf_cuda host_register` or `kf3_bar_ram`/`kf3_timer_view`: CUDA writes or guest access land on whatever is mapped there next. Dormant only because windows are `'static` today. | `HostSpan<'a>` or an owning guard; trybuild row. | MEASURED (compiles, runs) |
| A-02 | `mapping_unsafe.rs:1103-1118` vs `:1260` | `unsafe impl Sync for VolatileRegion` is argued on "no bulk accessor", but `copy_out` is a plain memcpy: a safe data race against `store_u32` (callers `userstore.rs:429`, `chan.rs:1402`). | Word-wise atomic copy, or a separate `!Sync` view; correct the comment. | MEASURED |
| B-01 | `chardev_unsafe.rs:610-616, 735` | A request with `_IOC_SIZE` 0 has no bound: the kernel wrote 3 bytes outside a 1-byte slice. No product caller issues one today. | Refuse `declared == 0` unless a per-request size table covers it. | MEASURED |
| B-04 | `kvm_unsafe.rs:520-540` | `pub KvmVm::set_memslot` does not keep the window alive (test/harness reach only). | Private; `KvmMemslot::install` the only door. | CODE READ |
| C-01 | `ffi_unsafe.rs:116-120` + ~30 safe `pub extern "C" fn` | `dev(h)` dereferences any non-null pointer and mints an unbounded lifetime; safe Rust can call `kf3_bar0_write(dangling, ..)`. = S1-08, REGRESSED (24 entries, was 11). | Handle registry check in `dev()`, or `unsafe extern` + private module. | CODE READ |
| C-03 | `raw_unsafe.rs:100-121`; `mem.rs:170,705-716` | `BackendFd` is a `Copy` integer; `borrow()` fabricates a `BorrowedFd`. Backend closed between lookup and mmap -> a reused fd number is mapped `MAP_FIXED` into a guest-visible BAR window (operator-side trigger). | mmap under the guard, or `dup` into an `OwnedFd` at `kf3_ram_add`. | CODE READ |
| C-04 / D-06 | `ffi_unsafe.rs:685-698, 800-819`; `kf3.c:1054-1074`; `kf-cuda/display.rs:308` | = S1-03, **not fixed**. The console frame address crosses safe code as `usize`; `Kf3Frame` has a `pub data: *mut u8` and no length; the C check compares stride bytes with width pixels. A safe-side geometry bug makes QEMU show host memory on the console/VNC. | Carry a length minted in an unsafe file; check `(h-1)*stride + w*bpp <= len` on both sides. | CODE READ |
| D-04 | `driver_unsafe.rs:580,882,903,1335-1348`; `walk.rs:2148` | CUDA handles and `DeviceImage` are `Copy` with public destroy: double destroy / use after destroy inside libcuda from safe code. | Non-`Copy`, consuming destroy. | CODE READ |
| D-05 | `display.rs:300-343, 811-829, 1851` | A `Frame` can outlive its context; `release_frame` unpins without draining queued DMA. | Synchronize before release; liveness token. | CODE READ |
| D-12 | `cuda/walk/kf_walk.cu:813, 1010-1043, 1297-1305, 2290-2347` | The kernels trust host-authored `KfLayout` regions with no in-kernel pool bound; only safe `capacity.rs` guards them. | Pool sizes in `KfDev`; refuse in-kernel. | CODE READ |
| E-01 | `broker/nvkvm_broker.c:2972, 2915` (+ `:2590, :2530, :2824-2883`) | Log lines outside the 10/s reject budget, flushed on the thread holding the keyboard grab. Full rate needs a hostile VMM; an honest kayfabe gives one line per guest mode change. | Route through `nb_reject_log()`. | MEASURED |
| E-02 | `kf-broker/src/conn.rs:2175, 2188, 2204, 2223, 2518, 2600, 2659` | A hostile broker streams packets; kayfabe prints one unbounded `eprintln!` per packet on QEMU's main loop; a stalled stderr would block it (INFERRED). Violates "kayfabe validates everything from the broker" for emit, and the quiet-by-default rule. | Per-second budget in `log_line`. | CODE READ |

### 3.4 LOW / INFO (one line each; detail in the appendices)

- A-03 `reprotect(&self)` leaves stale prot: SIGSEGV instead of `NotWritable` (MEASURED). A-04 mapping length never checked against file size: SIGBUS (MEASURED). A-06 `request_huge_pages` non-atomic RMW on shared memory (no production caller). A-07 SAFETY comment cites a `ret == target` check that does not exist (`window_unsafe.rs:452-468`). A-08 `udmabuf_create`/`place_device_view` accept any fd. A-09 `memfd_create` without `MFD_CLOEXEC`. **A-10 (DoS, no planned limit found)** `stitch` has no piece-count cap; guest can drive the VMM to `max_map_count` (kills its own VM). (2026-11-13)
- B-05 `KvmVm::adopt` does not confirm the fd is a VM. B-06 `pub(crate) ioctl_arg` with raw fd. B-08 no const assert tying `size_of::<T>()` to the KVM request (sizes verified by hand). B-09 `install_break_handler` would clobber QEMU's SIGUSR1 (no caller).
- C-05 `kf3.c:674-675` overlay bounds `vf_rel + len`, `base + len` wrap (re-read by me; QEMU alias clipping should contain it, INFERRED). C-07 failed realize / exit: threads not joined, bottom halves not deleted (S1-80 residue). **C-08 (DoS, no planned limit found)** overlay request queue uncapped. C-06 comments say "volatile", code uses `read_unaligned`/`copy_nonoverlapping` on guest-shared memory (no reference is formed). C-09 raw `i32` fds in the broker watch verb.
- D-07 completion callback holds an fd number, not an owned fd. D-08 `view_bytes<T: Copy>` exposes padding (MEASURED; = S1-09). D-09 packed-YUV extent short by 2 bytes for odd widths (stays in guest VRAM). **D-10 (DoS, no limit)** one unbounded log line per walk at pool exhaustion. **D-11 (DoS)** staging frame up to 1 GiB host VRAM outside `fb-mb`; `import_slot` uncapped. D-13..15 INFO (posture memo, negative fd becomes stdin = S1-10, bring-up leaks).
- E-03 sparse sealed memfd makes compositor/broker allocate ~570 MB per buffer (hostile VMM). E-04 `--persist`: next VM's first COMMIT can present the previous VM's uncommitted buffer (`nb_session_wl.c:2117-2184`), a cross-VM frame leak on the host display; fix `w->pending = -1` on detach. E-05 two single-caller bounds.
- F-01 kf-gop `teardown` ignores uninstall statuses then frees the installed GOP (guest-firmware UAF, guest-only). F-02/F-03 INFO. F-04 diagnostic Windows capture driver: unserialised START/STOP (kernel UAF; admin-only tool, not shipped). F-06..F-10 INFO (observer cost under BQL, H5 patch mask held, EFS patch scan, 61 extra NVIDIA MIT headers vs README, test app).
- P-01 (LOW, CODE READ, mine) `kf-chan/src/tspace_unsafe.rs:592-643`: `fence_words`, `gate_acquire_words`, `ring_gp_entry` take a raw `u64` VA from safe code and check only alignment and `< 2^40`; "inside the ring region" is the caller's. A wrong value makes the GPU write the fence payload anywhere in the T-space (guest windows or kayfabe's ring; not host memory). Fix: a `RingAddr` type minted here from the ring region bounds. P-02 (LOW) `put_gr_address` `:499-513` takes the footprint `need` and the register `upper` from the caller (every other trigger recomputes its footprint from the emitted registers). Fix: derive `need` from a per-register table in this file.

### 3.5 False positives considered and rejected

`zeroed::<&T>()` (aborts by Rust's own check); `borrow_raw` on `/proc/self/fd` numbers (re-confirmed after dup); SCM_RIGHTS receive (no control buffer, no fd adopted); broker partial-command fd confusion, `tx[last_tick]` index, emit-triggered detach, dma-buf name injection, misaligned PutImage pointer, hostile broker passing fds to kayfabe; A-X1 (two memfd backends -> wrong page) left UNVERIFIED, not claimed. Each area's fuller list is in its appendix.

## 4. Owner rules and earlier-review items: checklist for the unsafe scope

| # | rule (source) | where it applies here | verdict | evidence |
|---|---|---|---|---|
| 1 | Every perimeter export validates all its own inputs; a caller precondition is forbidden (RULINGS §R, A.6) | all `*_unsafe.rs` | **VIOLATED** | 54 caller-only + 14 missing sites; D-01/02/03, B-01/02, A-01, C-01/02/03/04 |
| 2 | No raw or disguised host/device address outside the perimeter (§R; S1-03, S1-04) | kf-cuda API, console frame | **VIOLATED** | `driver_unsafe.rs:685-853`; `ffi_unsafe.rs:687` `pub data: *mut u8`; `v3-sec-rawaddr` unmerged |
| 3 | No VMM VA and no unvalidated offset+length from safe code; opaque RamBlockId + validated RamSpan (owner 2026-10-31) | guest-RAM access | PARTLY | `RamSpan` + `GuestRamAuthority::validate` exist (`kf-rm/src/memory_list.rs:14-50`, `mem.rs:332`); `HostBlock{hva,len}` is gone from master; but `RawRegion`/`RamBlock` copies still escape the guard (C-02) |
| 4 | Private fields on address-carrying handles; no `Copy`/`Clone` on handles that own memory (§R gates 4, 5) | handle types | **VIOLATED** | `Kf3Frame.data` pub; `RawRegion`, `HostSpan`, `BackendFd`, `CtxHandle`, `ModuleHandle`, `Func`, `GraphHandle`... are `Copy` |
| 5 | A reviewed table of every perimeter export with its checks and tests (§R gate 3) | all perimeter files | **VIOLATED** except T-space | only `V3_P1P2_TSPACE.md` §3.8 has one; no table for kf-linux-raw/kf-qemu/kf-cuda found (grep). This report's appendices are a first draft |
| 6 | Inside the perimeter, an unchecked precondition is `unsafe fn` (§R a) | perimeter | VIOLATED in places | F-03 (firmware private fns), D-01, C-01 (`dev()`) |
| 7 | `kf3.c` is perimeter and must compile in CI (§R d; S1-13) | kf3.c | **VIOLATED** | not compiled or analysed in any workflow (ledger); could not be analysed here either |
| 8 | Unsafe only in kf-linux-raw, kf-qemu, kf-cuda `*_unsafe.rs` (CLAUDE.md) | tree | HOLDS, doc stale | no `unsafe` block elsewhere in `kf-*` (grep); `firmware/kf-gop` (78 blocks) and `kf-chan/tspace_unsafe.rs` (0 blocks) are permitted by the gate but not named in CLAUDE.md |
| 9 | Compiled-out checks: no `debug_assert` guarding a precondition (plan §2.8) | perimeter | HOLDS | one `debug_assert_eq!` (`mapping_unsafe.rs:621`), not a precondition |
| 10 | Copy, then check, then use; no reference over live guest memory (CONSTRAINTS §39; S1-27) | guest memory readers | HOLDS for "no reference"; UNVERIFIED for double fetch | no `&[u8]`/`&T` over guest memory in A or C (only `&AtomicU32/U64`); plain memcpy is used, not volatile (A-05, C-06); no racing-writer test exists (S1-27 open) |
| 11 | Host verbs authored, never forwarded (A.2) | ioctl structs | HOLDS by default; **VIOLATED with `KF3_GSS_NATIVE=1`** | X-01 |
| 12 | Nothing blocks on a vCPU or under a lock a vCPU takes; MMIO-write allowlist (A.4, §41, §48.1) | kf3.c callbacks, Rust exports | HOLDS on the lockless path; VIOLATED (bounded) on two paths | `kf3.c:320-325`, `device.rs:1528-1780` clean; a vCPU PCI config write reaches `kf3_ram_del` -> `RwLock::write` (`mem.rs:162`); diagnostic flags (`x-gsp-observer`, `KF3_DOORBELL_CPL`) take the BQL / do ioctls on a vCPU; kf-trap/kf-chan callees UNVERIFIED |
| 13 | Only BAR0 writes trap (CLAUDE.md, §49.3/49.5) | kf3.c | HOLDS with the documented Hopper+ doorbell-overlay exception | `kf3.c:626-639`; `THE_CONSTRAINTS.md:3429` |
| 14 | Mapping never runs in a trap; BAR1/BAR2 touched only inside a refresh (2026-10-31) | mmap doors | first HOLDS; second UNVERIFIED | overlays queued to a main-loop BH (`kf3.c:720-736`); the refresh rule is decided in `kf-mem`/`mem.rs`, outside this scope |
| 15 | Unchanged VAs stay accessible; no unmap-then-remap (2026-10-10) | `GuestWindow` | HOLDS at this layer | single atomic `MAP_FIXED` replacements, no sub-range `munmap` (`window_unsafe.rs:420-469`) |
| 16 | Drainer/input threads never block; quiet by default (2026-10-09) | sockets, logs | HOLDS for sockets; VIOLATED for emit | `MSG_DONTWAIT` everywhere (`unixsock_unsafe.rs:110,277,313`); E-02, D-10 unbounded log lines |
| 17 | MMIO memcpy only for vidmem rings (2026-10-16) | `VolatileRegion::copy_out` | HOLDS as far as read | two callers, store/ring views (`userstore.rs:429`, `chan.rs:1402`) |
| 18 | Passthrough spaces hold only guest PT leaves; T-space only windows + ring (§Q, §AB) | address authoring | HOLDS in the perimeter; UNVERIFIED for space contents | `tspace_unsafe.rs` constructs addresses only inside the two windows; P-01/P-02 are the caller-kept parts; S1-21/23 PARTLY per ledger |
| 19 | Guest-reachable unbounded read/emit/alloc is a bug; DoS bounded per VM (plan §2.4) | all | **VIOLATED** (no planned limit found for these) | A-10, C-08, D-10, D-11, E-01, E-03; ledger: S1-42, S1-45, S1-82, S1-25 open |
| 20 | Broker sees only finished frames + cursor; kayfabe validates all it receives; broker owns cursors (§L, §AJ, §AL) | broker, kf-broker | HOLDS for memory and values; VIOLATED for emit (E-02); E-04 cross-VM stale buffer | sanitizer/analyzer clean (MEASURED) |
| 21 | No CPU executor, no forged completion (A.3) | kf-cuda | HOLDS for the walker; UNVERIFIED for display | walker completes on `cuEventQuery` only; `CompletionFd::signal` is `pub` (`driver_unsafe.rs:1505`) |
| 22 | CAP_SYS_ADMIN cleared at births and for CUDA threads (§P; S1-22) | capability, posture | HOLDS for libcuda half; other RM calls open | `capability_unsafe.rs:36,53`; ledger S1-22 PARTLY |
| 23 | Walker treats guest tables as hostile (plan §2.7, §39) | cuda/walk | HOLDS on spot check; S1-60 (VER3 big-PTE veto) and S1-61 open | `kf_walk.cu:409-414` |
| 24 | Stock install needs no host patch; RE boundary; no behaviour flag needed to boot | tools, kf3.c | HOLDS | `kf-host/src/tier.rs`; kf3.c env reads are diagnostic or default-off |
| 25 | Hot-unplug/exit lifecycle (S1-80) | kf3.c | PARTLY | `dc->hotpluggable = false` (`kf3.c:2001`); no join, no `qemu_bh_delete`; its test is run by no workflow |
| 26 | Channel fault containment; NVIDIA-granularity events; correctness on full perf config | - | NOT-APPLICABLE to the unsafe boundary | - |

## 5. Earlier audit findings against master

Full table: Appendix G (`G_audit_ledger.md`). No stage-2 findings document exists anywhere in the tree.

- Stage 1 (67 findings): **3 fixed** (S1-29, S1-43, S1-84 profile half), **8 partly** (S1-21, 22, 23, 25, 33, 80, 86, 89), **52 open**, **4 regressed** (open and grown: S1-08, S1-11, S1-16, S1-41). About 25 of the open rows rest on grep / no fixing commit rather than a re-read; each row says which.
- Memory-safety relevant and still open: S1-20 (blocker, untested), S1-40 (= B-02), S1-03 (= C-04/D-06), S1-04 (= D-02), S1-05, S1-07 (= C-02), S1-08 (= C-01, regressed), S1-09/10 (= D-08/D-15), S1-01/02/06 (gates lexical; ledger walks v2 crates), S1-13 (kf3.c not in CI), S1-27, S1-60/61/67, S1-81, S1-41 (regressed via `raw_control_opaque`), S1-42/45/82 (DoS).
- w301 (14 findings): all STALE-LOCATION (the audited crates are archived/frozen); v3 analogues were not audited.
- Stale doc statements: the plan's §4 "every row is open"; stage-1 entries for S1-21/23/43 still say "default OFF" although `KF3_TSPACE` was hardwired on 2026-10-10; CLAUDE.md's three-crate rule omits `firmware/kf-gop` and `kf-chan`.

## 6. Not reviewed

- Miri/Kani/loom runs; any dynamic test on a GPU, in a guest, or against the NVIDIA driver; `-fanalyzer` on `kf3.c`.
- The frozen `kayfabe-linux-raw` / `kayfabe-cuda` twins; `archive/`; patches under `archive/` and `traces/`.
- Safe-code logic beyond the call sites that feed the perimeter: `kf-mem` `vasmgr`/`batch` in depth, `kf-trap`/`kf-chan` callees on the vCPU path, `kf-host` beyond ioctl call sites, the rmrpc decoders (S1-24).
- `kf3.h` line by line; most of the Wayland/X11 backends outside attach/commit/cursor/clipboard; `nb_tick.c`; broker privilege drop and argv; `slots.rs` interleavings.
- The 12k-line committed PTX beyond entry signatures; the diff/commit kernels line by line; `walk-v2-frozen`.
- Lock-order and interleaving models for the VA-manager thread versus vCPU access (plan §2.3): only the mmap layer's atomic-replace property was checked.
- Whether a second memfd backend is refused (A-X1); closed-driver behaviour for a released CPU view whose VMA is still placed.

## 7. Suggested order of fixes (smallest first)

1. D-03 length check (4 lines) and D-01 parameter-size assert.
2. B-02 / B-01 in the ioctl funnel (size fields written by the wrapper; refuse size-less requests).
3. C-02 + C-03 + A-01: one pattern (guarded accessor, non-`Copy` handles, lifetimes).
4. Merge `v3-sec-rawaddr` (D-02, C-04/D-06).
5. Caps for A-10, C-08, D-10, D-11, E-01, E-02.
6. CI: compile `kf3.c`; point the unsafe ledger at the v3 crates; write the §R export table from the appendices.

---

# Appendices: per-site row tables and area detail



---

## Appendix A (file A_mapping_window_fd.md)

Part status as of 2026-11-13: Security review 2, PART A (mapping / window / fd unsafe). Tree origin/master 921c39444. Read-only review; no product code changed.

Scope read in full: `crates/kf-linux-raw/src/mapping_unsafe.rs` (non-test 1-1550), `window_unsafe.rs` (1-617), `host_fd_unsafe.rs` (1-587), `bounds.rs`, `view.rs`, `scratch.rs` (non-test), `page_size.rs`, `geometry.rs::require_aligned`, `pagemap.rs` (the `impl MappedRegion`), `udmabuf_gate.rs`. Safe callers sampled for reachability: `kf-qemu/src/mem.rs` (window verbs 575-720, guest-RAM view 2785-2815, `backing_fd`), `kf-qemu/src/broker.rs:165-200`, `kf-qemu/src/device.rs:1894-1905`, `kf-host/src/channel.rs:1150-1215` (stitch), `kf-host/src/timer.rs`, `kf-host/src/lib.rs:797-805`, `kf-mem/src/cpuwin.rs:575-600, 925-1000`, HostSpan consumers `kf-qemu/src/ffi_unsafe.rs:457-545`, `kf-cuda/src/driver_unsafe.rs:1003-1030`.

Labels: CODE READ unless marked MEASURED (a PoC test against `kf-linux-raw`, 5/5 passed, each hazard reproduced; PoC not retained) or INFERRED. (2026-11-13)

Severity note (owner addendum, 2026-11-13): any guest breakout is CRITICAL; unbounded DoS counts. No finding in this part is a demonstrated guest breakout; A-01 is the one whose failure mode WOULD be one if its caller-held precondition ever broke.

### 1. Unsafe sites

Verdict key: V = VALIDATED, CO = VALIDATED-BY-CALLER-ONLY, M = MISSING, P = PANIC-ONLY path.

### 1.1 mapping_unsafe.rs

| # | line | operation | SAFETY contract (documented?) | safe callers + arguments | validated at the boundary by | verdict |
|---|---|---|---|---|---|---|
| 1 | 293 | `mmap(NULL, len, prot, flags, fd, off)` in `Mapping::anywhere` | kernel-chosen address; yes | `MappedRegion::map`, `VolatileRegion::map{,_read_only}`, `Reservation::new`, `stitch` (len u64, Backing, HostPageSize) | len != 0, page-aligned, `usize::try_from`; file offset aligned + `off_t::try_from` (`decode_backing`); fd is a `BorrowedFd`; `HostPageSize` only constructible validated (`page_size.rs`); MAP_FAILED + null checked | V (file size not checked: A-04) |
| 2 | 326 | `munmap(base,len)` in `Drop for Mapping` | unique owner; yes | scope exit | private type, not Clone, `InsideReservation` returns early; `assert!(rc==0)` skipped while panicking | V; P on kernel refusal / lock held. Dangling `HostSpan`/`addr_at` consumers: A-01 |
| 3 | 441 | `unsafe impl Send for MappedRegion` | yes, and `Sync` deliberately withheld (ui test `region_is_not_sync`) | - | `RegionView` holds `&MappedRegion` so is `!Send` | V |
| 4 | 464 | `mprotect(base,len,prot)` in `pub fn reprotect(&self)` | "changes only permissions"; yes | any safe caller, any `HostProt` | range = own mapping. NOT validated: `self.prot` is not updated, so every later `prot` check lies | M (A-03) |
| 5 | 590 | `base.add(start)` (stitch) | yes (a)-(d) | `kf_host::HostRm::map_scattered_through` (guest-derived `(file_off,len)` pieces) | `checked_span(map.len, cursor, len)`; pieces non-zero, aligned, sum `checked_add` | V |
| 6 | 592 | `mmap(target, MAP_FIXED|MAP_SHARED)` (stitch) | yes | same | target inside own fresh PROT_NONE reservation; monotone cursor; ret checked for MAP_FAILED and `!= target` | V (file size: A-04; piece count: A-10) |
| 7 | 610 | `munmap(ret,len)` of a stray result | yes | - | only on `ret != target` | V |
| 8 | 693 | `madvise(MADV_HUGEPAGE)` in `request_huge_pages` | yes | no production caller (tests only) | own extent | V |
| 9 | 704 | `read_volatile` + `write_volatile` of one byte per 4 KiB | "changes nothing observable"; yes but wrong under a concurrent writer | same | `off < len`, prot has WRITE | V for bounds; A-06 |
| 10 | 740 | `copy_nonoverlapping(map -> dst)` `read_into` | yes; concurrent-writer residual documented on the type | everything that reads guest RAM views, `RegionView` | `checked_span(len, offset, dst.len())` | V (race residual A-05; SIGBUS A-04) |
| 11 | 769 | `copy_nonoverlapping(src -> map)` `write_from` | yes | same | `prot != ReadOnly`, `checked_span` | V (prot field can lie: A-03) |
| 12 | 833 | `madvise(NOHUGEPAGE / DONTNEED)` `advise` | yes | `kf-rawclient/src/hw.rs:102,259`, pagemap tests | len != 0, offset+len aligned, `checked_span`, closed enum, DontNeed needs ReadWrite | V |
| 13 | 985 | `_mm_sfence()` | yes | `release_fence()` | no operands | V |
| 14 | 993 | `asm!("dsb st")` (aarch64 only) | yes | same | no operands | V (not compiled here) |
| 15 | 1030, 1032 | `unsafe impl Send/Sync for HostSpan` | yes (address + length, no access) | - | only `as_ptr` (unsafe) opens it | V |
| 16 | 1049 | `NonNull::new_unchecked(base.add(off))` `HostSpan::within` | yes | `GuestWindow::host_span(off,len)` | `off.checked_add(len)`, `end <= total`, `len != 0` | V |
| 17 | 1072 | `pub unsafe fn HostSpan::as_ptr` | "no longer than the minting mapping lives"; documented | consumers: `kf_qemu::ffi_unsafe::kf3_bar_ram:472`, `kf3_usermode_view:516`, `kf3_timer_view:~540`, `kf_cuda::Driver::host_register:1021` | NOTHING ties the span to its mapping: `HostSpan` is `Copy`, has no lifetime, and is minted by SAFE `host_span()` (647, 1196, window 189) | CO (A-01) |
| 18 | 1101 | `unsafe impl Send for VolatileRegion` | yes | - | - | V |
| 19 | 1118 | `unsafe impl Sync for VolatileRegion` | justification says "There is no bulk accessor" - FALSE since `copy_out` (1260) | `kf-qemu/src/userstore.rs:429`, `chan.rs:1402` call `copy_out`; many threads share regions | nothing | M (A-02) |
| 20 | 1231 | `&*base.add(start).cast::<AtomicU32/U64>()` `word_at` | yes | `load_u32/u64`, `store_u32/u64` everywhere | align check, `checked_span`, sealed `AtomicWord`, ReadOnly refuses stores | V (INFO: a Rust reference to MMIO, A-05) |
| 21 | 1267 | `copy_nonoverlapping(map -> dst)` `copy_out` | yes for bounds | userstore, chan | `checked_span` | V for bounds; race is A-02 |
| 22 | 1487 | `base.add(start)` `Reservation::map_fixed_in` | yes | no caller outside this crate's tests | aligned, `checked_span`, overlap scan | V |
| 23 | 1491 | `mmap(MAP_FIXED)` in a reservation | yes | same | as 22; ret compared with target | V |
| 24 | 1513 | `munmap` stray | yes | - | - | V |
| - | 882 | `pub(crate) fn addr_at` (address as u64, no unsafe) | bounds-checked; yes | `chardev_unsafe.rs:351,655,723` (PART B), `pagemap.rs:134,153` (a NON-unsafe file handles a host address) | `checked_span` | V for bounds; lifetime of the pinned pages is PART B's |

### 1.2 window_unsafe.rs

| # | line | operation | contract | safe callers + arguments | validated by | verdict |
|---|---|---|---|---|---|---|
| 1 | 105 | `unsafe impl Send for GuestWindow` | yes | - | immutable fields | V |
| 2 | 117 | `unsafe impl Sync for GuestWindow` | yes, residual (racy memcpy) stated | - | `place`/`restore` are atomic kernel replacements, range never unmapped before Drop | V (residual A-05) |
| 3 | 154 | `mmap(NULL, RW, ANON|NORESERVE)` `create` | yes | `kf-qemu/src/mem.rs:2623` (PRAMIN/BAR1/BAR2 lengths from config) | len != 0, aligned, `usize::try_from`, MAP_FAILED/null | V |
| 4 | 404 | `madvise(DONTDUMP/DONTFORK/HUGEPAGE)` `advise` | yes | `scratch::advise_window`, mem.rs | aligned, `checked_span`, closed enum | V |
| 5 | 454 | `mmap(base+start, MAP_FIXED)` `fixed_map` | yes, but cites a result comparison that does not exist | `place` (mem.rs:711 guest-RAM fd + `file_off`; `ScratchTile::cover`), `place_device_view` (mem.rs:694 armed `/dev/nvidia` node), `restore` | len != 0, offset+len aligned, `checked_span(self.len)`; file offset aligned + `off_t`; `DeviceFile` refused in `place` | V for bounds (A-07 phantom check; A-04 file size; A-08 fd identity) |
| 6 | 493 | `copy_nonoverlapping(win -> dst)` `read_into` | yes | host-side window reads | `checked_span` | V (SIGBUS A-04) |
| 7 | 513 | `copy_nonoverlapping(src -> win)` `write_from` | yes | host-side window writes | `checked_span`. No writability check although `place_device_view(.., writable=false)` can make a range PROT_READ | V for bounds; SIGSEGV path in A-03 |
| 8 | 561 | `write_volatile::<u32>` `store_u32` | yes | (no production caller found for the `GuestWindow` one) | `checked_span(4)`, `start % 4 == 0` | V (non-atomic on a `Sync` type: A-05) |
| 9 | 606 | `munmap` in Drop | "no accessor yields the pointer" - stale since `host_span` / `userspace_addr_at` | scope exit | production windows are `&'static` (`Box::leak`, `kf-qemu/src/mem.rs:2701`) | V for own state; P on refusal; spans: A-01 |
| - | 582 | `pub(crate) userspace_addr_at` | bounds; yes | `kvm_unsafe.rs:494` (PART B) | aligned, `checked_span` | V for bounds |

### 1.3 host_fd_unsafe.rs

| # | line | operation | contract | callers | validated by | verdict (2026-11-13) |
|---|---|---|---|---|---|---|
| 1 | 34 | `OwnedFd::from_raw_fd` in `pub(crate) adopt_fd(raw)` | "value consumed on the expression that produced it"; yes | only `*_unsafe.rs` files of this crate (chardev, drm, epoll, kvm, vcpu, unixsock; grep-verified, 11 call sites) | `raw >= 0`; not reachable from a non-unsafe file | V (drm passes a kernel-filled out-field `arg.fd`: PART B) |
| 2 | 125 | `memfd_create(name, MFD_ALLOW_SEALING)` | yes | `SharedRam::create{,_named}` (kf-qemu broker frame, cursor, scratch tiles) | `&CStr`; len != 0, `off_t::try_from` | V (no `MFD_CLOEXEC`: A-09) |
| 3 | 131 | `ftruncate` | yes | same | owned fd | V |
| 4 | 140 | `fcntl(F_ADD_SEALS, SHRINK|GROW|SEAL)` | yes | same | owned fd; failure returned | V |
| 5 | 196 | `memfd_create` (cfg(test)) | yes | tests | - | V |
| 6 | 221 | `fcntl(F_DUPFD_CLOEXEC)` | yes | `dup_for_export` | `adopt_fd` | V |
| 7 | 289 | `ioctl(dev, UDMABUF_CREATE, &arg)` | struct size asserted 24; yes | `kf-qemu/src/broker.rs:193` (`dev` = any `BorrowedFd`) | struct layout, memfd from own `SharedRam`, size aligned. `dev` being `/dev/udmabuf` is the caller's word | CO (A-08) |
| 8 | 312 | `mem::zeroed::<libc::statfs>()` | yes (plain integers) | `fs_magic` | - | V |
| 9 | 315 | `fstatfs(fd, &mut st)` | yes | `fs_magic`, `fd_carrier` | rc checked before read | V |
| 10 | 369 | `fcntl(F_GET_SEALS)` | yes | `fd_carrier` | negative handled | V |
| 11 | 449 | `getrlimit` | yes | `descriptor_budget` | rc checked | V |
| 12 | 485 | `eventfd(0, NONBLOCK|CLOEXEC)` | yes | `Notifier::create` | `adopt_fd` | V |
| 13 | 531 | `write(fd, buf, 8)` | yes | `signal{,_under_lock}` | own array + len | V |
| 14 | 572 | `read(fd, buf, 8)` | yes | `drain` | own array + len | V |

Counts (non-test sites, Send/Sync of HostSpan counted as two): 48 sites. VALIDATED 44, VALIDATED-BY-CALLER-ONLY 2 (`HostSpan::as_ptr` and its three safe mints; `udmabuf_create`'s device fd), MISSING 2 (`reprotect` state, `VolatileRegion: Sync` justification), PANIC-ONLY paths 2 (the two `Drop` asserts, plus every `lockwitness::assert_lock_free`).

Safe siblings: `bounds::checked_span` is the single bounds primitive (zero length, `checked_add`, `end <= object_len`, `usize::try_from`): sound. `view.rs`: two-level check (`absolute` then the region's own): sound. `scratch.rs`: no unsafe, every placement goes through `GuestWindow::place`; `file_off + len <= tile` checked: sound. `page_size.rs`: `HostPageSize` only from `sysconf` (validated power of two, 4-64 KiB) or the test/feature `forced` (same validation): a safe caller cannot weaken alignment checks. No `debug_assert` guards an unsafe precondition in these three files (the one `debug_assert_eq!(cursor,total)` at mapping_unsafe.rs:621 restates a proved fact). (2026-11-13)

### 2. Findings

### A-01 MEDIUM (latent; would be CRITICAL if reached) - `HostSpan` has no lifetime: every `as_ptr` consumer is validated by the caller only (2026-11-13)
- Where: mint `mapping_unsafe.rs:647-652` (`MappedRegion::host_span`), `:1196-1201` (`VolatileRegion::host_span`), `window_unsafe.rs:189-191`; type `mapping_unsafe.rs:1022-1075` (`#[derive(Clone, Copy)]`, no `PhantomData<&'a _>`); `Drop` at `mapping_unsafe.rs:326`, `window_unsafe.rs:606` whose SAFETY text says "no accessor yields the pointer" (stale).
- Scenario (MEASURED for the safe half: test `a01_a_host_span_outlives_its_mapping_in_safe_code` compiles and passes): safe code does `let s = region.host_span(); drop(region);` and passes `s` on. (a) `kf_cuda::Driver::host_register(&s)` (`driver_unsafe.rs:1011-1030`, `pub(crate)`, reached from safe `display.rs:790`) page-locks `[p, p+len)` of whatever the allocator mapped there next, and the async scanout copy then writes a frame over it: host memory corruption. (b) `kf3_timer_view` / `kf3_usermode_view` / `kf3_bar_ram` hand the pointer to QEMU as a MemoryRegion backing; if the owner (`TimerWindow` drops its `VolatileRegion` in `Drop`, `kf-host/src/timer.rs:50-52`; `HostRm`'s usermode window; a `GuestWindow`) is dropped while the region is still installed, the guest reads (and for a BAR ram_device region writes) whatever is mapped at that address afterwards: guest access to VMM memory.
- Why it does not fire today (CODE READ): the three BAR windows are `&'static` (`kf-qemu/src/mem.rs:2653-2657, 2701`), the `Device` "lives for the process" (`ffi_unsafe.rs:1135-1141`, `kf3_unrealize` only stops threads), `display::Frame` owns map + registration together. All of that is discipline in non-unsafe files, which the owner's scope says must not be relied on.
- Minimal fix: make the span borrow its mapping (`HostSpan<'a>` with `PhantomData<&'a ()>`, minted by `fn host_span(&'a self)`), and give the FFI/driver consumers `HostSpan<'static>` only from `&'static` owners (which the windows already are); or return an owning guard (`Arc<Mapping>` inside the span) so the mapping cannot be unmapped while a span exists. Fix the two Drop SAFETY comments. Test: a trybuild row "span outlives region" must fail to compile.

### A-02 MEDIUM - `VolatileRegion: Sync` rests on a statement that is no longer true (`copy_out` is a plain memcpy) (2026-11-13)
- Where: `mapping_unsafe.rs:1103-1118` (argument: "every access goes through `word_at` ... There is no bulk accessor"), `:1260-1275` (`copy_out`, `copy_nonoverlapping`), type doc `:1077-1083` ("no bulk accessor").
- Scenario (MEASURED that safe code can do it: `a02_volatile_region_allows_a_plain_copy_racing_an_atomic_store`): thread 1 `store_u32(off)` (atomic), thread 2 `copy_out(off, buf)` (non-atomic read of the same bytes) through a shared `&VolatileRegion`: a data race between two Rust threads = UB by the language rules, independent of the hardware writer. Callers `kf-qemu/src/userstore.rs:429`, `chan.rs:1402` read hardware/guest-written rings this way. Practical effect today: torn bytes only (INFERRED); the defect is that the one `unsafe impl Sync` in the crate described as "the literal definition" is unsupported by its own text.
- Minimal fix: implement `copy_out` as a loop of `AtomicU64`/`AtomicU32` `Relaxed` loads over the aligned body (the doc already measured that width is the only lever), bytes at the edges via `AtomicU8`; or move `copy_out` to a separate `!Sync` read-only data-view type. Then correct the comment. Same remedy answers the documented "residual" on `MappedRegion`/`GuestWindow` copies if the owner wants it closed (A-05).

### A-03 LOW - `MappedRegion::reprotect` desynchronises the `prot` field; `GuestWindow::write_from` has no writability check: the refusal becomes a SIGSEGV (2026-11-13)
- Where: `mapping_unsafe.rs:459-475` (pub, `&self`, does not update `self.prot`), checks that then lie: `:754` (`write_from`), `:697` (`request_huge_pages`), `:809` (`advise`), `view.rs` `is_writable`; `window_unsafe.rs:503-517` after `place_device_view(.., writable=false)` (`:300-321`).
- Scenario (MEASURED, `a03_...`: child killed by signal 11): `r.reprotect(HostProt::ReadOnly)` then `r.write_from(ZERO, b"x")` -> SIGSEGV in the VMM instead of `RawError::NotWritable`. Same for a host `GuestWindow::write_from`/`store_u32` into a range holding a read-only device view. A crash, not corruption, but not a panic and not catchable.
- Minimal fix: make `reprotect` `pub(crate)`/test-only (its doc says the only caller asserts it FAILS), or take `&mut self` and update `self.prot` on success. For windows: either refuse host writes into a range placed read-only (track it) or document that host writes to windows are never made into device views (and remove `GuestWindow::store_u32`, which has no production caller).

### A-04 LOW - mapping length is never checked against the file: a short file is a SIGBUS inside `read_into`/`write_from` (2026-11-13)
- Where: `mapping_unsafe.rs:266-311` (`anywhere`), `:571-601` (`stitch`), `window_unsafe.rs:215-245, 420-469` (`place`/`fixed_map`).
- Scenario (MEASURED, `a04_...`: both children killed by signal 7): `MappedRegion::map(SharedFile{fd, 0}, 4 pages)` over a 1-page memfd, `read_into(2 pages)` -> SIGBUS; `GuestWindow::place(.., SharedFile{fd, offset: 16 pages})` then host `read_into` -> SIGBUS. Guest-side equivalent: a window range placed past EOF faults in KVM (`-EFAULT` from `KVM_RUN`), i.e. the guest kills its own VM. Production callers bound it today in safe code: `kf-qemu/src/mem.rs:2790-2797` uses the `fstat` length; frames/scratch use sealed `SharedRam` of the mapped length; `place_ram`'s `file_off` comes from the RAM-block table. `stitch`ed views are never touched by the VMM (only `pin_user_pages`, which returns `-EFAULT`).
- Minimal fix: in `decode_backing`/`place`, `fstat` the fd and refuse `offset + len > st_size` for `SharedFile` (one syscall per map; or accept a `SharedRam`-like sized handle instead of a bare fd so the bound is a field). Does not cover a peer truncating an unsealed file later: require `F_SEAL_SHRINK` for fds that leave the process (already true for `SharedRam`).

### A-05 INFO - the concurrent-writer residuals, for the owner's "data race = UB unless volatile/atomic" question
- `MappedRegion::read_into/write_from` (`:740, :769`), `GuestWindow::read_into/write_from` (`:493, :513`): plain `copy_nonoverlapping` on memory the guest writes concurrently. Documented on the types as a ruled residual (single copy, never re-read). No `&[u8]`/`&mut [u8]`/`&T` over guest-writable memory is ever created in these three files (F4 of the falsifier list: not falsified here) - the only reference formed is `&AtomicU32/U64` (`:1231`).
- `GuestWindow::store_u32` (`:561`) is `write_volatile` on a `Sync` type: two threads storing the same word race (undocumented, unlike the copies). No production caller found.
- `word_at` forms a Rust reference to what may be MMIO (BAR0 timer/usermode pages, USERD): a reference is `dereferenceable`, so the compiler may in principle insert a read; registers with read side effects must not be reached this way. Today's users are counters/USERD words (no read side effect known). Raw-pointer `AtomicU32::from_ptr`-style access without a long-lived reference would close it.

### A-06 LOW - `request_huge_pages` does a non-atomic read-modify-write of one byte per page
- Where: `mapping_unsafe.rs:697-711`. The SAFETY text says "changes nothing observable"; false when the guest (or the GPU, for a pinned frame) writes that byte between the read and the write: the guest's store is reverted (guest data corruption, not host). Also `off += 4096` assumes 4 KiB pages; on a `DeviceFile`-backed region it would rewrite device bytes; past EOF it is A-04's SIGBUS.
- Reachability: no production caller (tests only). Fix: restrict to `Backing::PrivateAnonymous`/sealed own memfd regions (record the backing kind in `MappedRegion`), or fault in with `madvise(MADV_POPULATE_WRITE)` which does not write data; or delete.

### A-07 LOW - `GuestWindow::fixed_map`'s SAFETY comment cites a check that is not there
- Where: `window_unsafe.rs:452-468`: "The result is compared against the requested address below" - only `MAP_FAILED` is tested. The identical phantom was found and fixed in `Reservation::map_fixed_in` on 2026-08-07 (`mapping_unsafe.rs:1478-1486, 1504-1520`) and in `stitch` (`:606-615`); this third copy was missed. Not reachable on a conforming kernel (MAP_FIXED returns the address or fails). Fix: add the same `ptr::eq(ret, target)` check + `munmap` of a stray result.

### A-08 LOW - two fds whose identity is the safe caller's word
- `udmabuf_create(dev, ..)` (`host_fd_unsafe.rs:267-297`): the ioctl number is only meaningful on `/dev/udmabuf`; on another character device the same number may be a different request with a different argument layout (a driver that treats the 24 bytes as containing a user pointer would write through it). Scenario needs a buggy caller passing e.g. a DRM or nvidia fd: INFERRED, no concrete colliding driver identified. Fix: own the open (`UdmabufDevice::open()` in this file, wrapping `udmabuf_gate::open_device`) or `fstat` + compare `st_rdev` with the misc device.
- `GuestWindow::place_device_view(fd)` (`window_unsafe.rs:300-321`) maps ANY fd `MAP_SHARED` at offset 0 into a guest-visible window (the doc says "not reachable from any production path until that ruling lands", but `kf-qemu/src/mem.rs:694` is the production BAR path). A wrong fd here is the guest seeing a host file/device = breakout class if a safe caller errs. Today the only caller passes the node returned by `arm_cpu_view`. Fix: accept only `&CharDevice` armed-view tokens (a type minted by `chardev_unsafe` when the map context was registered) instead of `BorrowedFd`; update the stale doc sentence.

### A-09 LOW - `SharedRam` memfds are created without `MFD_CLOEXEC`
- Where: `host_fd_unsafe.rs:125` (`MFD_ALLOW_SEALING` only); `dup_for_export` (`:221`) is CLOEXEC, the original is not. Display frames (`kf-qemu/src/broker.rs:174`), the cursor (`kf-broker/src/conn.rs:3078`) and the three scratch tiles are inherited by any child QEMU `exec`s (helpers, `exec:` migration, network scripts): guest display content readable/writable by a less trusted child; also an fd-number leak. Fix: add `MFD_CLOEXEC`.

### A-10 LOW (DoS, INFERRED) - `stitch` has no piece-count bound of its own
- Where: `mapping_unsafe.rs:538-623`; caller `kf-host/src/channel.rs:1185`. Each piece is one VMA until the view is reaped. The caller-side caps are extent-based (`kf-mem/src/batch.rs:825-857`: 64 GiB per batch, `MAX_LEAF_PIECES = 1 << 20`), the live-view count is bounded (`REAP_QUEUE + REAP_OVERFLOW`, channel.rs:2119-2146), but 1 << 20 pieces exceeds the default `vm.max_map_count` (65530). A guest kernel building maximally scattered batches drives the VMM to the process-wide VMA limit; `stitch` itself fails cleanly (ENOMEM -> whole range unmapped -> "rows go per run"), but while it is near the limit any other `mmap` in the process (glibc malloc arenas, thread stacks, `GuestWindow::place` splits) can fail, and QEMU aborts on allocation failure. Effect: the guest kills its own VM (allowed by the model), not other tenants. Planned limit: `V3_SECURITY_AUDIT_PLAN.md` §2.4 lists "mapping-count growth" as bounded for windows (`v3-scratch-bound`); I found no stated cap for transient stitched views: "planned, not in place" is the most that can be said. Fix: refuse `pieces.len()` above a named constant well under `max_map_count` (e.g. 16 384) in `stitch`. Not measured.

### Out of scope for this part (UNVERIFIED)
- A-X1: `kf-qemu/src/mem.rs:702-707` (`place_ram`) and `:170-172` (`backing_fd`, "The ONE backend fd every fd-backed block shares") take the FIRST fd-backed RAM block's fd for every `file_off`. If a VM has two fd-backed memory backends, a `file_off` computed for block B is mapped from block A's file: the BAR window (and the guest-RAM OS descriptor, `:2785-2815`) would show a different guest page than the page table named - an intra-guest isolation failure (rule A.9), not a host breakout. I did not find where a second distinct backend fd is refused; PART C (kf-qemu) should confirm.
- `kf3_bar_ram` etc. (`ffi_unsafe.rs`) and `host_register` (`kf-cuda`) are the `as_ptr` consumers of A-01: their rows belong to PARTs C and D.
- `adopt_fd(arg.fd)` in `drm_unsafe.rs:93,152` adopts a kernel-filled out-field: PART B.

### False positives considered and rejected
- `Reservation::map_fixed_in` overlap arithmetic `p.offset + p.len`, `offset + len` (`:1449`): cannot overflow, both were bounded by `checked_span` against a `usize` length first.
- `stitch` `cursor += len` (`:616`) unchecked: bounded by `total`, which was `checked_add`ed; `checked_span` re-proves each piece.
- A forged `HostPageSize` weakening alignment: not constructible outside `query()`/validated `forced`.
- `fixed_map` with an unaligned length rounding past the window: lengths are checked aligned to the real page size, and the window end is page-aligned.
- `GuestWindow::place`/`restore` racing a host `read_into` (use-after-unmap): MAP_FIXED replaces atomically; the range is never unmapped before `Drop`, and `Drop` needs `&mut` (production windows are `'static`).
- `fd_carrier`/`fs_magic`: `F_GET_SEALS` before `fstatfs`, bounded fdinfo read: fine. `mem::zeroed::<statfs>` is valid.
- `Mapping` `Drop` `assert!` in a destructor: guarded by `thread::panicking()`, so no double panic; PANIC-ONLY.
- `Notifier` write/read lengths come from the local array.
- `MappedRegion::advise(DontNeed)` on pinned or shared pages: content change only, no address becomes invalid.
- `stitch` partial failure leaking placed pieces: the owning `Mapping` unmaps the whole range.

### 3. Owner rules touched in this scope
- Nothing blocks on a vCPU / under a vCPU-taken lock: every syscall door here calls `lockwitness::assert_lock_free` (mapping 274, 569, 688, 802, 1436; window 138, 221, 307, 339, 380; host_fd 111, 216, 272, 439, 479) - HOLDS for "under a ranked lock" (panic if violated). Whether a vCPU thread calls them is the caller's (kf-mem/kf-qemu): UNVERIFIED here, except `arm_store_in_trap` + `place_view` in `kf-mem/src/cpuwin.rs:944-951`, which is the PRAMIN trap path doing one ioctl + one mmap on the trapping thread by design (PRAMIN is the stated exception).
- Unchanged VAs stay accessible during refresh / no unmap-then-remap: HOLDS at this layer - `GuestWindow` has no sub-range `munmap`; `place`/`restore`/`cover` are single atomic `MAP_FIXED` replacements (`window_unsafe.rs:420-469`); `ScratchTile::cover` retries a refused placement without unmapping first (`scratch.rs` cover loop).
- BAR1/BAR2 mappings touched only inside a refresh: UNVERIFIED (decided by the callers of `WindowOps::place_view/place_ram/sink`, `kf-qemu/src/mem.rs:683-720`, not by this layer).
- MMIO memcpy only for vidmem rings: `VolatileRegion::copy_out` (`mapping_unsafe.rs:1260`) is the one bulk MMIO read door; its two callers are `kf-qemu/src/userstore.rs:429` and `chan.rs:1402` (store/ring views). HOLDS as far as read here; no bulk MMIO WRITE door exists on `VolatileRegion` (word stores only).
- Guest-reachable unbounded read/emit: `stitch` piece count (A-10) is the one guest-steered unbounded quantity found at this boundary; `Reservation.placements` grows without bound but has no production caller.
- Host actions authored, never forwarded: no guest bytes are interpreted in these files; offsets/lengths arrive as integers from safe code. HOLDS at this layer.
- No behaviour flags needed to boot: none read here. HOLDS.

### 4. Not reviewed
- The `#[cfg(test)]` modules of the three unsafe files (mapping 1570-2519, window 619-1100, host_fd 589-951) beyond a grep for `unsafe` (none in mapping/host_fd tests except `unsealed_for_test`).
- `memtype.rs`, `cache.rs`, `census.rs`, `geometry.rs` beyond `require_aligned` (no unsafe, not on a memory path).
- `kf-mem/src/vasmgr.rs` and `batch.rs` in depth (only the caps and the cpuwin apply loop were read); `kf-chan`, `kf-rawclient` only as call-site lists. The thread-level question "does the VA manager apply while a vCPU holds X" was answered only at the syscall layer (atomic MAP_FIXED), not by a lock inventory.
- The frozen `crates/kayfabe-linux-raw` twin files.
- No Miri (not installed), no Kani; clippy cast lints and the debug_assert sweep are covered by other parts.
- NVIDIA driver behaviour for a released CPU view whose VMA is still placed in a window (reaper order in `kf-qemu/src/mem.rs:655-675`): closed driver side, not examined.


---

## Appendix B (file B_ioctl_kvm_sock.md)

Part status as of 2026-11-13: Security review 2, PART B (ioctl / KVM / sockets / signals / small syscalls in `crates/kf-linux-raw`). Tree: origin/master 921c39444. Falsifiers: the falsifier list (not retained). Labels: CODE READ / MEASURED / INFERRED.

### 0. Result against the falsifier

The hypothesis "every safe-to-unsafe boundary is validated" is **FALSE in this scope**: F8/F1 fire on `CharDevice::ioctl` (B-01 MEASURED, B-02, B-03 CODE READ), F5 fires on `KvmVm::set_memslot` (B-04 CODE READ), F3 on `KvmVm::adopt` (B-05, INFERRED scenario). No guest-reachable breakout was found in this scope: every failing boundary needs a wrong *safe caller*; today's callers (kf-host) pass consistent arguments (CODE READ of all 17 `.ioctl(` call sites outside kf-linux-raw: all build the request with `ioctl::readwrite(.., arg.len())`). All other sites (49) are VALIDATED inside the unsafe file. (2026-11-13)

### 1. Rows (one per unsafe site; identical ones grouped)

| location | operation | SAFETY contract (documented?) | safe callers + arguments | what validates at the boundary | verdict |
|---|---|---|---|---|---|
| chardev_unsafe.rs:142 `DevDir::open` | `open(path, O_PATH)` | NUL-terminated path (yes) | any; `&CStr` | type `&CStr`; `adopt_fd` checks sign, single owner | VALIDATED |
| chardev_unsafe.rs:195 `can_reach`, :471 `openat_mode` | `openat(dirfd, name)` | live dirfd, NUL name (yes) | kf-host connect, drm.rs; `&DevDir`, `&CStr` | `OwnedFd` in `DevDir`, `&CStr`, `adopt_fd` | VALIDATED |
| chardev_unsafe.rs:735 `CharDevice::ioctl` — (a) arg length vs `_IOC_SIZE` | `ioctl(fd, request, arg.as_mut_ptr())`; kernel copies `_IOC_SIZE(request)` both ways | "re-derived from request and compared" (yes) | kf-host (17 sites), kf-linux-raw drm.rs; `request: u64` arbitrary, `arg: &mut [u8]` | `declared > arg.len()` refused (:610-616), empty and > MAX refused | VALIDATED for `_IOC`-encoded requests |
| same — (b) requests with `_IOC_SIZE == 0` (legacy numbers: FIONREAD, TCGETS, UVM ioctls) | kernel writes its own fixed size | comment admits "the check can say nothing" (:606-609) | any safe caller; `CharDevice::adopt(any OwnedFd)` + any request + 1-byte arg | nothing | **MISSING** (B-01, MEASURED) |
| same — (c) length of an `Indirect` target vs the companion size field inside `arg` (`paramsSize`, NVOS02 `limit`, nested list count) | kernel `copy_from/to_user(ptr, sizeField)` | "the caller must set itself" (:369-373) | kf-host `raw_alloc_exact` :969, `raw_alloc_via` :1026, `raw_alloc_nested` :1088, `raw_control_status` :1212, `alloc_os_descriptor` :2011 (all use `params.len()` / `len-1`) | pointer field offset bounds-checked and non-overlapping; **size field not read or written by the wrapper** | **VALIDATED-BY-CALLER-ONLY** (B-02 = stage-1 S1-40, still open) |
| same — (d) pointer fields in `arg` / in a payload that are NOT declared as `Indirect` | kernel follows whatever u64 is there | none | any safe caller; any bytes | nothing (wrapper is driver-agnostic by design, :44-50) | **MISSING** (B-03) |
| chardev_unsafe.rs:700,713,723 address minting + scrub :770-781 | `as_mut_ptr() as u64` into arg, zeroed after | yes | — | exclusive borrows for the call; pre-pass makes patch loop infallible; `Region` re-checked with `addr_at` (mapping fork's function) | VALIDATED |
| kvm_unsafe.rs:168 `Kvm::open` | `open("/dev/kvm")` | yes | tests/harness only (kf-chan tests) | literal CStr, `adopt_fd`, API version check | VALIDATED |
| kvm_unsafe.rs:334 `discover_in_this_process` | `BorrowedFd::borrow_raw(n)` then dup | yes (TOCTOU named) | tests | dup then `confirm_is_a_vm` on the dup | VALIDATED (B-07 INFO) |
| kvm_unsafe.rs:561 `ioeventfd`, :600 `register_coalesced_mmio` | ioctl with 64 / 16-byte repr(C) struct | yes | kf-chan tests | struct built in-file; `BorrowedFd` | VALIDATED |
| kvm_unsafe.rs:765 `ioctl_ptr` (struct pointer) | `KVM_SET_USER_MEMORY_REGION` | yes | via set/clear_memslot | 32-byte repr(C) struct in-frame | VALIDATED |
| kvm_unsafe.rs:520-540 `KvmVm::set_memslot` (pub) — lifetime of `userspace_addr` | kernel retains window HVA | "Prefer KvmMemslot" (doc only) | any safe caller; `&GuestWindow` borrowed only for the call | range checked by `userspace_addr_at`; **nothing keeps the window mapped** | **VALIDATED-BY-CALLER-ONLY** (B-04) |
| kvm_unsafe.rs `KvmMemslot` install/Drop | clear slot before window release | yes | kf-chan tests | owns `Arc<GuestWindow>` + `Arc<KvmVm>`; Drop asserts the clear | PANIC-ONLY / VALIDATED (note: if already panicking and the clear fails, window is released with the slot live) |
| kvm_unsafe.rs `KvmVm::adopt(OwnedFd)` then :749 `ioctl_arg` results fed to `adopt_fd` | wraps an unverified fd; later `KVM_CREATE_VCPU` return value adopted as an fd | no (recorded as a known gap in tests/unsafe_naming.rs:121-127) | any safe caller | none (`confirm_is_a_vm` exists, not called) | **MISSING** (B-05) |
| kvm_unsafe.rs:749 `ioctl_arg` (`pub(crate)`, raw `c_int` fd, raw request, integer arg) | variadic ioctl | yes ("integer by value") | only kvm_unsafe.rs / vcpu_unsafe.rs today | none for a caller in a safe file of this crate (an `arg` is a pointer for most requests) | VALIDATED-BY-CALLER-ONLY (B-06, crate-internal) |
| vcpu_unsafe.rs:277 munmap, :327 mmap(kvm_run) | map/unmap run struct | yes | `KvmVcpu::create` | len from kernel, checked >= head+MMIO arm; NULL hint, no MAP_FIXED; not Clone; MAP_FAILED checked | VALIDATED |
| vcpu_unsafe.rs:441 KVM_RUN | `_IO` ioctl | yes | `&mut self` | owned fd | VALIDATED |
| vcpu_unsafe.rs:477 `complete_mmio_read`, :496 `read_exit` | 8-byte copy / `read_unaligned` inside mapping | yes | `&mut self`; `len: u8` | len 1..=8 refused otherwise; offsets constant and covered by the create-time size check; POD types | VALIDATED |
| vcpu_unsafe.rs:533 `unsafe impl Send for KvmVcpu` | move across threads | yes | — | no `Sync`; all access under `&mut self` | VALIDATED |
| vcpu_unsafe.rs:577/596 `ioctl_ref<T>` / `ioctl_ref_mut<T>` (private) | ioctl with `&T` | yes | in-file only (sizes 144/312 match KvmRegs/KvmSregs: computed) | in-file pairing; no const assert | VALIDATED (B-08 INFO) |
| epoll_unsafe.rs:163, :243, :276, :314 | epoll_create1 / epoll_pwait2 / epoll_wait / epoll_ctl | yes | kf-qemu, kf-chan; `BorrowedFd`, token u64, `&mut ReadyTokens` | fixed in-frame array, count from the same const, `n.min(MAX_READY_BATCH)` | VALIDATED |
| drm_unsafe.rs:81, :107, :142, :160 | PRIME_HANDLE_TO_FD / GEM_CLOSE / EXPORT_SYNC_FILE / poll | yes | kf-qemu gpucopy.rs:253,281; display.rs:1762 | const-asserted struct sizes, test ties numbers to sizes, fd initialised -1 and `adopt_fd` | VALIDATED |
| affinity_unsafe.rs:29,34,44,67,71,76 | zeroed cpu_set_t, CPU_SET/ISSET, sched_*affinity | yes | harness | index `< bits` checked before CPU_SET | VALIDATED |
| capability_unsafe.rs:36, :53 | capget/capset (v3, 8+24 bytes) | yes | capability.rs:181,184 (`pub(crate)`) | fixed arrays in-frame | VALIDATED |
| timerslack_unsafe.rs:24,34; sysconf_unsafe.rs:25 | prctl / sysconf by value | yes | harness / page_size.rs | no memory | VALIDATED |
| unixsock_unsafe.rs:107,130,161,196,204 | socket / connect / getsockopt / geteuid / zeroed msghdr | yes | kf-broker link.rs:27 | path < 108, no NUL, absolute; len == sizeof(ucred) | VALIDATED |
| unixsock_unsafe.rs:250 (cmsg build), :274 sendmsg | SCM_RIGHTS, one fd | yes | kf-broker link.rs:38; `&[u8]`, `Option<BorrowedFd>` | `[u64;3]` buffer, `CMSG_SPACE` asserted to fit, header non-null asserted; fd only borrowed (no ownership change) | PANIC-ONLY (asserts) / VALIDATED |
| unixsock_unsafe.rs:313 recvmsg | bounded read, NO control buffer | yes | kf-broker link.rs:51; `&mut [u8]` | iov = slice; `msg_controllen = 0` so the kernel discards any passed fd (no fd leak, no fd adoption); MSG_CTRUNC reported | VALIDATED |
| unixsock_unsafe.rs:393, :546 | test-only listen / recvmsg | — | tests | — | not product |
| signal_unsafe.rs:61,70,80 | zeroed sigaction, sigemptyset, sigaction(SIGUSR1) | yes | no caller in kf-* outside the crate | empty `extern "C"` handler (async-signal-safe), no statics | VALIDATED (B-09 INFO) |
| signal_unsafe.rs:101,130,135 | gettid, getpid, tgkill | yes | none in product | own tgid; `ThreadId` newtype only from `current_thread_id` | VALIDATED |

Counts: VALIDATED 49 (two of them guarded by asserts = PANIC-ONLY flavour), VALIDATED-BY-CALLER-ONLY 3 (ioctl size field, set_memslot lifetime, crate-internal `ioctl_arg`), MISSING 3 (size-less requests, undeclared pointer fields, `KvmVm::adopt`), test-only 2.

### 2. Findings

### B-01 MEDIUM — `CharDevice::ioctl` has no bound for requests whose `_IOC_SIZE` is 0 (MEASURED) (2026-11-13)
- `crates/kf-linux-raw/src/chardev_unsafe.rs:610-616` (the `declared > arg.len()` test passes for `declared == 0`), syscall at `:735`.
- Scenario (100% safe code): `CharDevice::adopt(OwnedFd::from(pipe_read_end)).ioctl(0x541B /*FIONREAD*/, &mut backing[..1], &mut [])`. The kernel writes a 4-byte `int` at the slice address. MEASURED with a PoC (not retained): `rc=Ok(0) backing=[00,00,00,00,aa,aa,aa,aa]` — 3 bytes outside the 1-byte slice were overwritten. With `TCGETS` on a tty the overrun is 35 bytes; with a UVM ioctl (not `_IOC`-encoded, sizes up to hundreds of bytes) on `/dev/nvidia-uvm` it is the whole params struct. On the heap or stack this is memory corruption from safe code.
- Reach: no product caller issues a size-less request today (all 17 use `ioctl::readwrite(..)`; CODE READ). Safe-side only.
- Minimal fix: refuse `declared == 0` unless `arg.len() >=` an in-file table entry for that exact request (FIONREAD -> 4) or unless the request has direction bits `_IOC_NONE` AND the caller passes through a separate `ioctl_no_payload()` door that sends `0` instead of a pointer. Add the PoC as a must-refuse test.

### B-02 HIGH (safe-side; = stage-1 S1-40, NOT fixed on master) — companion size fields are a safe-caller contract
- `chardev_unsafe.rs:365-380` (`Indirect::len` doc: "which the caller must set itself"), `:689-726`, `:735`.
- Scenario: a safe caller builds `NVOS54{params -> Indirect::new(16, &mut small /*32 bytes*/), paramsSize = 4096}` (or NVOS21 `paramsSize`, NVOS02 `limit`, or the page count of a `nested` list). RM does `copy_from_user`/`copy_to_user(params, paramsSize)` (`ogkm-580: rmapi param copy`), so the kernel writes up to `paramsSize - 32` bytes past `small` in the VMM heap/stack. CODE READ; not measured (no NVIDIA driver on this host). RM refuses a wrong size for most controls (size table), so the practical window is classes/controls with variable or unchecked sizes; that is RM behaviour, not a kayfabe guard (INFERRED).
- Today's callers are consistent: `kf-host/src/lib.rs:969,1026,1088,1212` use `params.len() as u32`; `:2011` uses `len - 1` with the same `len` passed to `Indirect::describing`.
- Minimal fix: give `Indirect` an optional `size_field: (offset, width, bias)`; the wrapper WRITES the field from the real target length (or refuses when the value already there exceeds it). For `nested`, same for the element count. Test: size field larger than the buffer is refused.

### B-03 HIGH (safe-side; guest reach UNVERIFIED) — pointer fields that are not declared as `Indirect` are passed to the kernel as they are
- `chardev_unsafe.rs:565-741`: `arg` and every `Indirect` buffer are opaque bytes; only declared offsets are patched/scrubbed.
- Scenario: safe code leaves a non-zero u64 in an undeclared pointer field (NVOS54 `params` at +16 with no patch, the `ptr` of `NV_ESC_IOCTL_XFER_CMD`, or a pointer embedded in a control payload such as a `pChannelHandleList`-style `NvP64`) — for instance because bytes derived from a guest RPC were copied into the payload. The host kernel then reads/writes VMM memory at that address for the size the struct names. If any guest-influenced payload with an embedded pointer field is ever forwarded this is a guest-to-host write primitive (would be CRITICAL under the owner addendum); whether such a path exists is outside this fork's scope (S1-41 "no host-side allowlist of RM controls" is the related open finding; kf-host's own builders set `p_alloc_parms/params/p_memory = 0`, CODE READ).
- Minimal fix: make the door NVIDIA-aware at the escape level (a small in-file table: escape nr -> pointer-field offsets + size-field offsets); refuse the call unless every listed pointer field is either zero or covered by an `Indirect`. Payload-internal pointers: the allowlist of S1-41 must mark controls with embedded pointers as never-forwardable.

### B-04 MEDIUM (test/harness reach only) — `KvmVm::set_memslot` is `pub` and does not keep the window alive
- `crates/kf-linux-raw/src/kvm_unsafe.rs:520-540`; the doc says "Prefer `KvmMemslot`".
- Scenario (safe code): `vm.set_memslot(0, gpa, &window, 0, len, false); drop(window); vcpu.run()`. The memslot still names the unmapped HVA range; the next `mmap` in the process (heap arena, another window) lands there and the guest reads and writes it: guest-to-host-heap access created by safe code. CODE READ; KVM types are used only by `kf-chan/tests/common` (kf3 runs inside QEMU, which owns the memslots).
- Minimal fix: make `set_memslot`/`clear_memslot` private (`KvmMemslot::install` is the only door), or `pub(crate)`.

### B-05 LOW — `KvmVm::adopt` wraps any descriptor; a non-KVM fd's ioctl return value is adopted as an fd
- `kvm_unsafe.rs` `KvmVm::adopt`; `vcpu_unsafe.rs:176-182` (`KVM_CREATE_VCPU` result -> `adopt_fd`). Already recorded as a ledger note in `tests/unsafe_naming.rs:121-127` ("EINVAL rather than corruption").
- Scenario (INFERRED): `KvmVm::adopt(fd of a driver whose ioctl 0xAE41 returns a positive integer N)`; `KvmVcpu::create` then owns "fd N", which may be a live descriptor of another object -> it is mmapped and later closed (I/O-safety violation, double close of someone else's fd). Needs an unusual driver; most answer ENOTTY.
- Minimal fix: call `confirm_is_a_vm` in `adopt` (make it fallible).

### B-06 LOW — `kvm_unsafe::ioctl_arg` and `Kvm::as_raw`/`KvmVm::as_raw` are `pub(crate)` with raw fd / raw request / integer argument
- `kvm_unsafe.rs:749`. A safe file of this crate could call `ioctl_arg(any_fd, any_request, any_integer_as_pointer)`. No such caller exists (grep: only kvm_unsafe.rs and vcpu_unsafe.rs). Fix: move the helper into a private module shared by the two unsafe files, or take `BorrowedFd` plus an enum of the four requests.

### B-07 INFO — `BorrowedFd::borrow_raw` on a number read from /proc/self/fd (2026-11-13)
- `kvm_unsafe.rs:334`. If the number is closed between the scan and the dup, the precondition "fd is open" is false for an instant; the only use is `F_DUPFD_CLOEXEC`, and the dup is re-confirmed. No scenario for corruption. False positive as a vulnerability.

### B-08 INFO — `ioctl_ref<T>`/`ioctl_ref_mut<T>` do not tie `size_of::<T>()` to the request (2026-11-13)
- `vcpu_unsafe.rs:570-600`. Private; sizes verified by hand (KvmRegs 144, KvmSregs 312, KvmRunHead 32, IoEventFd 64, CoalescedZone 16, UserspaceMemoryRegion 32). Fix: `const _: () = assert!(size_of::<KvmSregs>() == 312)` etc., as drm_unsafe.rs does.

### B-09 INFO — `install_break_handler` replaces the process-wide SIGUSR1 disposition
- `signal_unsafe.rs:57-92`. Handler is empty and async-signal-safe (no memory finding). INFERRED: QEMU uses SIGUSR1 as its vCPU kick (`SIG_IPI`); calling this from kf3 inside QEMU would replace QEMU's handler. No caller in kf-* today (grep).

### Considered and rejected (false positives)
- `request as libc::Ioctl` truncation: the kernel uses the low 32 bits; `declared_size` reads bits 16-29 of the same word. Consistent.
- Overlap test `q.at + POINTER_FIELD_WIDTH` (:634) overflow: `q.at` already passed `checked_add` in an earlier iteration.
- `recv_bounded` fd leak on MSG_CTRUNC: with no control buffer the kernel never installs the descriptor. `send_record` short send: returned to the caller, no memory effect.
- `sockaddr_un` not NUL-terminated: path length `< 108` enforced before the copy.
- `epoll_pwait2` timespec layout: 64-bit `timespec` == `__kernel_timespec` on x86_64/aarch64.
- kvm_run concurrent access: all reads/writes under `&mut self` after `KVM_RUN` returned; the vCPU fd is never exposed.
- `mem::zeroed` for `cpu_set_t`, `sigaction`, `msghdr`: all-integer/pointer C structs, zero is valid.
- `debug_assert` guarding an unsafe precondition: none in these files (grep).

### 3. Owner rules touched in this scope (addendum 2026-11-13)
- Host actions authored, ioctl structs built by kayfabe, no guest-supplied CPU pointers: mechanism HOLDS for declared pointer fields (`chardev_unsafe.rs:689-726`, scrub `:770-781`); kf-host builders zero their pointer fields (`kf-host/src/lib.rs:967,1210,2003`). UNVERIFIED at the wrapper for undeclared fields (B-03).
- Nothing blocks on a vCPU or under a lock a vCPU takes: every blocking wrapper asserts lock-free first (`chardev_unsafe.rs:571-572`, `kvm_unsafe.rs`, `epoll_unsafe.rs`, `unixsock_unsafe.rs`, `drm_unsafe.rs`) — HOLDS for ranked/leaf locks (a panic, not a block); whether a vCPU thread calls them is the callers' review.
- Drainer/input threads never block: broker socket is `SOCK_NONBLOCK`, `sendmsg`/`recvmsg` use `MSG_DONTWAIT` (`unixsock_unsafe.rs:110,277,313`), `dma_buf_idle` polls with timeout 0 (`drm_unsafe.rs:160`) — HOLDS. `CharDevice::ioctl` itself blocks in RM (by design; caller rule).
- CAP_SYS_ADMIN cleared around channel births: the primitive is per-thread capget/capset (`capability_unsafe.rs:36,53`), a failed restore is reported as `ClearedNotRestored` (capability.rs) — primitive HOLDS; the bracket placement is kf-host's (not reviewed here).
- Guest-reachable unbounded read/emit, DoS: nothing guest-steered allocates in these files; epoll batch is fixed (`MAX_READY_BATCH`); ioctl trace is off unless `KF_IOCTL_TRACE` is set (not reviewed for volume).

### 4. Not reviewed
- `mapping_unsafe.rs`, `window_unsafe.rs`, `host_fd_unsafe.rs` (`adopt_fd`, `MappedRegion::addr_at`, `GuestWindow::userspace_addr_at` are relied on here; another fork owns them).
- The test modules of each file beyond a skim; `census.rs`, `ioctltrace.rs` internals (safe code).
- kf-host / kf-rm / kf-qemu beyond the ioctl call sites: in particular whether any guest-derived bytes reach an RM control/alloc payload that has an embedded pointer field (B-03's guest reach), and the NVOS33/NVOS46 map replies (host addresses returned in `arg`).
- The frozen `crates/kayfabe-linux-raw` twins.
- Not run: miri (not installed), clippy cast lints (parent), the NVIDIA-driver side of B-02 (no driver on this host), a KVM PoC for B-04.

### 5. Evidence
- PoC source: not retained. Result of the run -> `rc=Ok(0) backing=[00, 00, 00, 00, aa, aa, aa, aa]`, assertion failed as predicted (the test asserts the bytes stay 0xAA).


---

## Appendix C (file C_kfqemu_kf3.md)

Part status as of 2026-11-13: Security review 2, PART C: `crates/kf-qemu/src/{ffi_unsafe,raw_unsafe}.rs` and `qemu/hw/misc/kf3/`.
Tree: origin/master 921c39444. Method: CODE READ of both unsafe files and kf3.c/kf3_guest_ram.h/kf3_gop.h/meson.build in full; safe callers read by range (device.rs, mem.rs, chan.rs, display.rs, kf-broker slots.rs, kf-chan dbfast.rs) only as far as needed. Nothing was built or run except one failed analyzer attempt (§5). Labels: CODE READ unless marked INFERRED. No MEASURED result in this part.
Severity rule applied (addendum, 2026-11-13): a guest breakout is CRITICAL; unbounded guest DoS is a finding.

### 1. Site table

Verdicts: V = VALIDATED, VC = VALIDATED-BY-CALLER-ONLY, M = MISSING, P = PANIC-ONLY.

### 1a. raw_unsafe.rs

| loc | operation | contract (documented?) | safe callers / arguments | validation at the boundary | verdict |
|---|---|---|---|---|---|
| :15,17 `unsafe impl Send/Sync for RawRegion` | share raw ptr across threads | yes: memory stays mapped until unregistered; only copy access | every thread (vCPU, drainer, workers, VA, display) | no reference is ever formed (true); LIFETIME is not enforced: type is `Copy`, copies outlive `ram_del` | VC (C-02) |
| :26 `RawRegion::adopt` (unsafe fn) | adopt ptr+len | yes | only ffi_unsafe.rs:391, :441 (C-supplied) | trusts C (`memory_region_get_ram_ptr`, section size) | V (unsafe fn, not safe-reachable) |
| :42-46 `span` + `ptr.add` | pointer offset | yes | all accessors; `off`, `n` arbitrary | `checked_add` + `end <= len` | V |
| :54 `store_u32` write_unaligned | 4-byte write to guest-shared memory | comment says "volatile"; code is NOT volatile | device.rs shadow_store, chan.rs UserdView::Ram | bounds V; concurrency: plain write racing a vCPU | V bounds / see C-06 |
| :63 `load_u32` read_unaligned | 4-byte read | same | same | same | V bounds / C-06 |
| :82 `write_from` copy_nonoverlapping | copy into region | yes | device.rs:1295,3986; mem.rs MemoryListRam; display.rs; chan.rs | `span(off, src.len())` | V bounds; lifetime VC (C-02) |
| :92 `read_into` copy_nonoverlapping | copy out | yes | device.rs:3973, chan.rs:1527/2072/2139/6484/6658/7641/7719, display.rs:256/2586/2659/2687/2744 | `span` | V bounds; lifetime VC (C-02) |
| :110 `BackendFd::adopt` (unsafe fn) | adopt raw fd | yes | ffi_unsafe.rs:442 | `fd >= 0` | V |
| :116-120 `BackendFd::borrow` → `BorrowedFd::borrow_raw` | forge a borrowed fd from an i32 | yes, but lifetime `'_` is of a `Copy` token | mem.rs:715, :1492, :2790 (mmap of guest RAM into BAR windows) | none: nothing ties the token to the fd being open | VC (C-03) |
| :148,150 Send/Sync OverlayHook; :169 `submit` | call C fn ptr with `opaque` | yes | mem.rs:2044 (VA thread): `seq, op, base, len, vf_rel` from the walked guest BAR1 PTE | C side `kf3_ov_apply` re-validates (with an overflow, C-05) | V (C clips; see C-05) |
| :191,193 Send/Sync ReadTrapHook; :209 `request` | call C fn | yes | bar0trace.rs (drainer) bool only | no argument to validate; `opaque` lifetime = process (non-hotpluggable) | V (realize-failure window: C-07) |
| :237,239 Send/Sync IoeventfdHook; :264 `set` | call C fn → KVM_IOEVENTFD | yes | kf-chan dbfast (act thread, main loop): gpa, value, BorrowedFd | fd is a live BorrowedFd; gpa arbitrary = only a KVM ioeventfd at a guest address (no host memory) | V |
| :303 Send BrokerHooks; :328 `watch`, :334 `timer` | call C fn with a RAW fd i32 | yes ("(0,0) before close") | kf-broker conn.rs:1034,1055,2787 (safe; fd is `i32`) | none in Rust; C passes it to `qemu_set_fd_handler` | VC (C-09) |
| :418 Send QemuSink; :428 `adopt` | copy 15 fn ptrs | yes | ffi_unsafe.rs:978 | every verb `?`-checked non-null | V |
| :473-493, :515-528 key/button/wheel/abs/rel/sync/select/missing/close/resize | call C fn, integers only | yes | kf-broker InputPolicy (values from the BROKER process) | C re-checks: key index (kf3.c:1250), button index (:1276), abs range (:1304), resize zero (:1385) | V |
| :500-507 `pointer_devices` | C writes ≤ cap entries into stack array | yes | kf-broker | `cap = min(16, out.len())`; C loop `n < cap` (kf3.c:1333); return value clamped `min(cap)` | V |
| :533-547 `define_cursor` | C reads `width*height` words | yes | kf-broker ConsoleCursor | `pixels.len() == w*h` checked in Rust (usize, 64-bit); C re-checks w,h in 1..=256 and hot spot (kf3.c:1001) | V |
| :551,:555,:559 hide/move/absolute | call C fn | yes | kf-broker | none needed | V |
| :566-577 test_owned_ram (cfg(test)) | leak a Vec as a region | yes | tests only | n/a | V (test only) |

### 1b. ffi_unsafe.rs (C → Rust exports)

Thread column: M = QEMU main loop (BQL), V = vCPU (lockless MMIO, no BQL), Vb = vCPU holding BQL (PCI config write / bus reset), R = realize.

| loc | export | thread | pointer / argument handling | verdict |
|---|---|---|---|---|
| :116-120 `dev(h)` | SAFE fn: `&*h.cast::<Device>()` for any non-null `h` | all | only a null check; reached from 30 SAFE `pub extern "C" fn`s in a `pub mod` | M for a safe Rust caller (C-01); V for the C caller (handle from kf3_realize, never freed) |
| :122-132 `write_err` | copy + NUL into `buf[len]` | R/M | null/zero check, `n = min(msg, len-1)` | V (trusts C's `len`) |
| :146 `kf3_realize` (unsafe) | CStr::from_ptr x4, `*out` | R | null checks; NUL-termination is C's contract (QOM strings) | V. Error path: thread-spawn failure returns -1 with the Device leaked and earlier threads running (leak only) |
| :288 `kf3_identity` (unsafe) | `*out = id` | R | null check, no alignment check | V (C passes a stack struct) |
| :315 `kf3_config_word` (unsafe) | `*off`, `*val` | R | `config_words.get(idx)`, null checks | V; C re-checks offset range and `pci->used` (kf3.c:1751) |
| :341 `kf3_memory_map` (unsafe) | `*out.add(i)` for `i < cap` | R | `take(cap)`, null check | V |
| :381 `kf3_shadow_attach` (unsafe) | adopt `[mem, mem+len)` | R | null check; `len as usize` (64-bit) | V; lifetime = ROM-device RAM, freed only if the object is finalized (C-07) |
| :397 `kf3_shadow_seal` | safe extern | R | `dev(h)` | C-01 |
| :405 `kf3_bar0_write` | safe extern; guest `off,val,width` | V | `u8::try_from(width)`; offsets resolved by `piece_for` + `span` (device.rs:1360-1366) | V for guest input; C-01 for the handle |
| :415 `kf3_bar0_read` | safe extern | V | same | V / C-01 |
| :426 `kf3_ram_add` (unsafe) | adopt guest RAM hva+len, fd | M, Vb | null check; contract "valid until kf3_ram_del" | V at adoption; the contract is NOT kept on the Rust side (C-02, C-03) |
| :457 `kf3_bar_ram` (unsafe) | `*ptr = span.as_ptr()` | R | `window_address`: `checked_sub`, `usize::try_from`, `win.host_span(rel,len)` (kf-linux-raw, other fork) | V here |
| :478 `kf3_ram_del` | safe extern | M, Vb | takes `RamMap.blocks.write()` (blocking; waits for bounded copies) | V memory-wise; rule note §4 |
| :489 `kf3_status`, :646 `kf3_trace_name`, :657 `kf3_trace_report` (unsafe) | `write_err` | M/R | as write_err | V |
| :501 `kf3_usermode_view`, :526 `kf3_timer_view` (unsafe) | `*ptr`, `*len` | R | null checks (timer: also alignment) | V; C checks `have >= len` (kf3.c:387, :747) |
| :548 `kf3_irq_fd` | returns Rust-owned eventfd number | R | `irq_lines.get(v)`; fd lives for the process (irq_lines is immutable, Device leaked) | V |
| :556, :618, :625, :633, :666, :675, :828, :838, :846, :862, :881, :918, :999, :1014, :1042, :1056, :1109, :1121, :1137 (safe externs: bar1_follows_guest, trace_mode, trace_piece, trace_admit, bar1_overlay_done, bar1_usermode_write, display_refresh, refresh_fd, refresh_drain, cursor_apply, doorbell_page_offset, doorbell_site, broker_frame_fd, broker_ready, broker_stop, display_ui_info, device_reset, device_reset_wait, unrealize) | integers only besides `h` | M / V / Vb | `dev(h)` | V for C; C-01 for a safe Rust caller |
| :568 `kf3_set_bar1_overlay`, :597 `kf3_set_read_trap`, :896 `kf3_set_ioeventfd` (unsafe) | adopt fn ptr + opaque | R | `Option<fn>` null-checked, once-only (`OnceLock`) | V |
| :800 `kf3_display_frame` (unsafe) | `data: f.addr as *mut u8` → C wraps it in a pixman surface | M | `out` null+aligned. `addr` is a `usize` produced by SAFE code (`FrameView.addr`, display.rs:885,1096,1119), no length crosses | M at the boundary; today held by safe code only (C-04 = S1-03, still open) |
| :944 `kf3_broker_start` (unsafe) | `*ops`, CStr path, adopt hooks | R | null + alignment on `ops`, null on path/watch/timer, every verb checked | V |
| :1077 `kf3_option_rom` (unsafe) | `*rom`, `*rom_len` | R | null checks; bytes owned by the leaked Device | V; C copies `len` into a region of `pow2ceil(len)` or `romsize >= len` (kf3.c:1498-1512) |

### 1c. kf3.c entry points

| loc | entry | thread | guest/peer-controlled inputs | bound | verdict |
|---|---|---|---|---|---|
| :274 kf3_piece_read, :320 kf3_piece_write (+ _trace :339/:349) | V | `addr` (< region size by QEMU), val, size 1..8 | forwarded as `p->base + addr`; Rust bounds it | V |
| :291 kf3_dbcpl | V | diag (env), 4096 lines max | `qatomic_fetch_dec` cap; stack structs filled by ioctl before use | V (log bounded) |
| :241 kf3_obs_mmio | V | diag only | takes the BQL on a lockless vCPU | V memory; rule note §4 |
| :379 kf3_host_rom_ops | R | `len` from Rust map | `have < len` refused | V |
| :411 kf3_bar0_build, :576 kf3_bar_build | R | Rust map (`n`, base, len, how) | `n` range checked (:417), piece counts (:431, :591), stack array of 128 | V |
| :494 kf3_read_trap_bh / :517 kf3_read_trap | M / drainer | bool | atomics + BH | V; `info_report` per flip (diag flag only) |
| :553/:560 kf3_bar12_read/write | V | none used | counters only (no piece is `how != 0` today) | V |
| :626 kf3_bar1_um_write | V | addr < 64 KiB (QEMU), val | forwarded | V |
| :648 kf3_ov_apply | M | `base, len, vf_rel` from Rust (walked guest BAR1 PTE) | `vf_rel + len > um_len`, `base + len > bar1_size` are UNCHECKED u64 additions | P-free but wrong (C-05); QEMU's flat-view clip is what actually bounds it |
| :693 kf3_ov_bh, :720 kf3_bar1_overlay | M / VA thread | queue of g_new0 requests | no cap on the queue (Rust waits per change) | VC (C-08) |
| :790 kf3_ioeventfd | act thread / M | gpa, datamatch, fd | kernel validates; no QEMU lock | V |
| :811 kf3_db_section | M / Vb | section offsets | u64 compare, `db_page_off + 4 > hi` | V |
| :835 kf3_region_add, :851 kf3_region_del | M / Vb | guest steers WHICH sections exist (BAR programming, device eject) | filter `kf3_is_guest_ram` (writable, not ram_device, has ram_block) | V here; Rust side keeps copies (C-02) |
| :862 kf3_vector_use, :901 kf3_vector_release | M / Vb | vector `v` | `v >= KF3_MAX_VECTORS` | V |
| :922 kf3_msi_user | M | diag | `x->nr < msix_vectors` by construction | V |
| :995 kf3_cur_define | M | w,h,hot from Rust (guest cursor image, kayfabe's copy) | 1..=256, hot inside, `cursor_alloc` NULL check, memcpy `w*h*4` as size_t | V |
| :1054 kf3_console_frame | M | `Kf3Frame` from Rust; width/height guest-steered (mode) | checks: fmt, non-null, non-zero, `stride >= width` (pixels vs bytes: wrong unit), `stride & 3`; NO length, no upper bound, `(int)` casts | M (C-04) |
| :1113 kf3_gfx_update, :1079 kf3_refresh_done, :1096/:1106 | M | none | timers/fd on the main AioContext | V |
| :1206 kf3_broker_watch, :1222 kf3_broker_timer | M | raw fd from Rust | none | VC (C-09) |
| :1245 kf3_in_key … :1380 kf3_in_resize_hint | M | broker-process input via Rust | key table length, button table length, abs INT_MAX/zero, `n < cap`, g_strlcpy | V |
| :1486 kf3_option_rom_build | R | Rust ROM bytes | `len > romsize` refused, else pow2ceil | V |
| :1519 kf3_dev_realize | R | operator properties | ABI check, discard-require first; `fail:` leaves `s->h` and its threads | V for the guest; C-07 |
| :1745 config words | R | Rust `off` | header/size/alignment/`used[]` | V |
| :1842 kf3_dev_exit | M | — | unreachable at run time: `dc->hotpluggable = false` (:2001), test `tests/test_hotplug_policy.py` | V (S1-80 residue: C-07) |
| :1908 kf3_reset_hold, :1926 kf3_reset_exit | M / Vb | guest can trigger resets at will | hold never waits; exit waits (bounded) on the main loop only | V |

Counts (rows above; a row may carry two aspects): VALIDATED 49, VALIDATED-BY-CALLER-ONLY 5 (RawRegion lifetime, BackendFd lifetime, overlay queue, broker fd x2 counted once), MISSING 2 (`dev(h)` for safe Rust callers; console frame length), PANIC-ONLY 0.

### 2. Findings

**C-01 · MEDIUM (soundness; not guest-reachable) · `crates/kf-qemu/src/ffi_unsafe.rs:116-120` and the 30 safe `pub extern "C" fn` entries (:397, :405, :415, :478, :548, :556, :618-633, :666, :675, :828-862, :881, :918, :999, :1014, :1042, :1056, :1109, :1121, :1137); `lib.rs:24` `pub mod ffi_unsafe`.**
`dev(h)` is a safe fn that dereferences any non-null pointer, and the entries are safe, public functions. Scenario (safe-side input): any crate that links kf-qemu calls `kf_qemu::ffi_unsafe::kf3_bar0_write(core::ptr::dangling_mut(), 0, 0, 4)` (no `*mut` type name, so the host-pointer CI gate does not see it) → `&Device` over address 1 → wild read, UB. This falsifies F1/F9 as written. Minimal fix: (a) keep the realized handle(s) in a static registry (`AtomicPtr<Device>` or a small set) and have `dev(h)` return `Some` only for a registered pointer; or (b) mark every entry `unsafe extern "C"` and make the module private (`mod ffi_unsafe`), keeping `#[unsafe(no_mangle)]`. (a) also turns a stale handle from C into a refusal.

**C-02 · HIGH; CRITICAL under the breakout rule where the precondition holds · guest-RAM regions outlive `kf3_ram_del` · `raw_unsafe.rs:5-17` (`RawRegion: Copy`), `mem.rs:82-91` (`RamBlock: Copy`), `mem.rs:176-206` (`block_for`, `at_file_offset` return copies outside the read guard), retained in `chan.rs:766` (`UserdView::Ram { mem: RawRegion }`, for the channel's life) and used after the guard at `device.rs:3973,3986`, `chan.rs:1527,2072,2139,6484,6658,7641,7719`, `display.rs:256,2586,2659,2687,2744`. INFERRED (QEMU frees a RAM block after `region_del` via RCU; Rust threads are not RCU readers).**
The FFI contract (`ffi_unsafe.rs:422-424`) says the hva is valid until `kf3_ram_del`. Only `MemoryListRam` (mem.rs:236-275) keeps the read guard across its copy; every other reader copies the `RawRegion` out and uses it later. Scenario: a writable RAM block that is registered (kf3_guest_ram.h accepts ANY writable non-ram_device RAM region: DIMMs, another device's RAM BAR such as VGA/bochs vram, ivshmem) is removed while a kayfabe thread still holds its region: (1) operator `device_del` of a pc-dimm + `object_del` of its backend, or (2) guest-only: the guest ACPI-ejects a hot-pluggable PCI device whose BAR is plain RAM while steering a kayfabe read/write at a GPA inside that BAR (GSP FSM `Ram::read/write`, display pushbuffer/instance-memory reads use `block_for`, which needs no fd). QEMU unrealizes the device and frees the block; the kayfabe thread's `read_into`/`write_from` then touches unmapped or REUSED host memory: host-memory read into guest-visible state or a host-memory write. In kayfabe's default launch (one memfd backend, no second RAM-BAR device) I found no guest-only trigger; that is an inference, not a test. Minimal fix: stop handing `RawRegion` out of the lock: one accessor `RamMap::with(gpa, len, |region, at| …)` that holds the read guard across the bounded copy (the MemoryListRam pattern), make `RawRegion`/`RamBlock` non-`Copy`, and replace `UserdView::Ram` with (gpa, generation) resolved per access. Narrow the C filter to the machine's main RAM (`sec->mr` owned by a memory backend) as defence in depth.

**C-03 · MEDIUM · `BackendFd` can outlive the descriptor · `raw_unsafe.rs:100-121`, `mem.rs:170-172,705-716,1488-1492,2787-2790`. INFERRED.**
`BackendFd` is a `Copy` integer; `borrow()` fabricates a `BorrowedFd` whose lifetime is that of the copy. `backing_fd()` returns it after dropping the lock, then the VA thread mmaps it `MAP_FIXED` into a guest-visible BAR window. Scenario: the backend is closed between the lookup and the mmap (same triggers as C-02, operator side: `object_del` of the memory backend) and the number is reused by another descriptor in QEMU (a disk image, kayfabe's own memfd): the guest gets a shared mapping of that file in BAR1/BAR2. Fix: do the `mmap` under the `RamMap` read guard (as C-02), or `dup` the fd once at `kf3_ram_add` into an `OwnedFd` kayfabe owns.

**C-04 · MEDIUM (stage-1 S1-03, still open on master) · `ffi_unsafe.rs:685-698,800-819`, `kf3.c:1054-1074`, `display.rs:883-899,1090-1104,1114-1139,5610-5611`.**
The console frame's host address is a `usize` in safe code (`FrameView.addr`, `AtomicUsize addrs[]`) and is cast to `*mut u8` at the boundary; `Kf3Frame` has no length. The C side wraps `height * stride` bytes at that address in a pixman surface that VNC/screendump/GTK read. Its only checks are non-zero and `stride >= width` (pixels compared with bytes; the right check is `stride >= width * 4` for the 32-bit formats, `* 2` for rgb565). What keeps it in bounds today is safe code alone: `need = w*h*4` vs `Frame::len()` (display.rs:5610), the `host` flag in `publish` (:1118) and `FrameRing::publish` offering the console only host-fresh slots (kf-broker slots.rs:709-723). Scenario (safe-side bug, the owner's assumption): any future path that describes a slot with a larger geometry than its host frame, or stores a wrong `addr`, makes QEMU read host memory past/at that address onto the console (host memory to the host display and to VNC clients). Width/height are guest-steered (the mode). Fix: carry `len` in `Kf3Frame`; in `kf3_display_frame` take the address and length from an object minted only in an `*_unsafe.rs` file (kf-cuda `Frame`), check `stride >= width*bpp` and `(height-1)*stride + width*bpp <= len` with checked arithmetic on both sides, and cap width/height (`kf_disp::scanout::MAX_PIXELS`).

**C-05 · LOW · unchecked u64 additions in the BAR1 overlay bound · `kf3.c:674-675`.**
`vf_rel + len > s->bar1_um_len || base + len > s->bar1_size` wraps. Input (from the Rust VA thread, values derived from a guest BAR1 PTE): `len = 0xFFFFFFFFFFFFF000, vf_rel = 0x1000, base = 0x1000` passes every test. Result: an alias of size ~2^64 at offset 0x1000 of the 64 KiB usermode ROM device, added to BAR1 at priority 1. QEMU's flat-view rendering clips an alias to its target and its container, so I expect no out-of-range host access (INFERRED from QEMU memory.c, not run); the effect is a write-trapped overlay larger than asked. Fix: `len > um_len || vf_rel > um_len - len || len > bar1_size || base > bar1_size - len`.

**C-06 · LOW/INFO · guest-shared memory is accessed with non-volatile primitives while the comments say volatile · `raw_unsafe.rs:13-14,48-63,82,92`.**
`write_unaligned`/`read_unaligned`/`copy_nonoverlapping` on memory a vCPU writes concurrently is formally a data race. No reference/slice over guest memory is created anywhere in the two files (checked: the only `from_raw_parts` are in `cfg(test)` verbs, :623, :649), so the practical exposure is the compiler assuming stability of a value it reads once; each accessor reads once. Scenario: none that corrupts memory; a double-fetch must come from a caller reading twice (S1-27, not in this part). Fix: `read_volatile`/`write_volatile` for the 4-byte paths (USERD cursors, shadow words) and correct the comment for the byte copies.

**C-07 · LOW · realize failure (and device exit) leaves Rust threads holding pointers into a device QEMU then frees (S1-80 residue) · `kf3.c:1828-1840` (`fail:`), `:1842-1897`, `ffi_unsafe.rs:209-269,1137-1141`, `device.rs:3588-3604` (`stop` signals, joins nothing).**
S1-80's hot-unplug half is closed: `dc->hotpluggable = false` (kf3.c:2001) with `tests/test_hotplug_policy.py`. The other half is unchanged: after `kf3_realize` succeeded, any later failure in `kf3_dev_realize` (BAR build, MSI-X, broker path, option ROM, config-word collision) goes to `fail:` with the drainer/worker/VA/display threads running, the shadow `RawRegion`s attached, and the `kf3_read_trap`/`kf3_bar1_overlay` hooks holding `opaque = s`. QEMU then finalizes the object (ROM-device RAM blocks, `Kf3State`). Scenario: `-device kf3-gpu,…,display-broker=relative/path` (refused by name after BAR0 was built): until QEMU's `exit(1)` the drainer can publish into freed shadow RAM or call `qemu_bh_schedule(s->read_trap_bh)` on freed state. Not guest-reachable (no guest runs yet), short window, process is dying. Fix: in `fail:` call `kf3_unrealize(s->h)` and have it JOIN the threads (keep the JoinHandles) before returning; delete the two bottom halves in exit.

**C-08 · LOW (DoS, safe-caller side) · unbounded overlay request queue · `kf3.c:720-736`.**
Each `kf3_bar1_overlay` call `g_new0`s a request with no cap; glib aborts on OOM. Rust submits one per view change and waits for `overlay_done`, and the pool is `bar1-overlays` slots, so a correct caller is bounded; a buggy caller (or a guest driving BAR1 PTE churn faster than the main loop drains, if the Rust side ever stops waiting) grows the queue without limit. No documented planned limit found. Fix: refuse with `-EBUSY` past e.g. `4 * bar1_overlays` queued.

**C-09 · LOW/INFO · raw fd numbers cross the broker verbs · `raw_unsafe.rs:326-329`, `kf3.c:1206-1219`, kf-broker `conn.rs:1034,1055,2787`.**
`watch(fd: i32, …)` is called by safe code with a plain integer, and the "(0,0) before close" order is a convention of the safe caller. Scenario: a caller that closes first leaves a QEMU fd handler on a number the process reuses; the handler then calls `kf3_broker_ready` for someone else's descriptor (spurious wakeups, a stuck handler), no memory corruption. Fix: pass `BorrowedFd` in the `Host` trait and unwatch in the connection's `Drop`.

False positives considered and rejected:
- `kf3_cur_define` memcpy size: `(size_t)width * height * 4` with both ≤ 256: no overflow; Rust also checks the slice length.
- `kf3_in_pointers` over-report: the count is clamped in Rust (`min(cap)`) and the C loop stops at `cap`.
- `kf3_option_rom_build` memcpy: region size is `pow2ceil(len)` or a `romsize >= len` (checked), never smaller.
- `kf3_memory_map` count: `n > capacity` refused in bar0_build; bar_build iterates `min(n, capacity)`.
- VRAM-only frame reusing a stale host address with a new geometry: `FrameRing::publish` offers the console only host-fresh slots (slots.rs:709-723) (still safe-side only, see C-04).
- `kf3_vector_use` index: checked against `KF3_MAX_VECTORS`.
- Format strings: every `error_setg`/`info_report`/`warn_report` uses a literal format; Rust text goes through `"%s"`.
- `msix-vectors` > 512 trips QEMU's own `msix_init` checks at start-up: operator input, not the guest.

### 3. Stage-1 items in this part

| ID | status on master 921c39444 |
|---|---|
| S1-03 (frame address as usize; no length) | OPEN, unchanged in substance (C-04). Line references moved: `ffi_unsafe.rs:800-819`, `display.rs:885`. |
| S1-80 (hot-unpluggable; exit joins nothing, bottom halves not deleted) | PARTLY FIXED: not hot-pluggable any more (kf3.c:2001, test SA-01). Join / BH deletion still absent (C-07), reachable only through a failed realize. |
| S1-27 (copy-then-check on guest memory) | not reviewed here beyond the accessors: each `RawRegion` accessor is a single copy; double fetches are the callers' (other parts). |

### 4. Owner rules touched in this scope

| rule | verdict | where |
|---|---|---|
| Only BAR0 writes trap; no BAR1/BAR2 traps | HOLDS with the documented exception: the Hopper+ BAR1 doorbell view is a write-trapped overlay (kf3.c:626-639, :682; THE_CONSTRAINTS.md:3429, OWNER_RULINGS.md:150). `kf3_bar12_ops` (kf3.c:553-572) would trap a `how != 0` BAR1/BAR2 piece; the map has none today (UNVERIFIED per family: kf-trap memmap not read). BAR0 READ exits exist for Hole pages (w828) and the default-off trace flags. |
| Nothing blocks on a vCPU or under a lock a vCPU takes | HOLDS lexically for the lockless MMIO path: kf3.c:320-325, :626-631 and `device.rs:1528-1780` contain no lock, allocation, log or wait (callees in kf-trap/kf-chan UNVERIFIED). Exceptions: (a) `kf3_obs_mmio` takes the BQL on a vCPU (kf3.c:245-252), diagnostic `x-gsp-observer` only; (b) `kf3_dbcpl` does two KVM ioctls + `fprintf` on the vCPU (kf3.c:310-317), `KF3_DOORBELL_CPL=1` only, 4096 lines; (c) a vCPU doing a PCI config write (BQL held) reaches `kf3_region_del → kf3_ram_del → RwLock::write` (mem.rs:162, waits for in-flight bounded copies) and, with `doorbell-ioeventfd=on` (default off), `kf3_doorbell_site → DbFast::reg.lock()` plus KVM_IOEVENTFD ioctls (kf-chan dbfast.rs:643-660): VIOLATED in those two config-write paths, bounded. |
| Mapping never runs in a trap | HOLDS in this scope: no mmap in ffi_unsafe/raw_unsafe; overlays are queued to a main-loop BH (kf3.c:720-736). |
| BAR1/BAR2 mappings touched only inside a refresh | UNVERIFIED here (mem.rs placement logic is another part's); the C side never re-points BAR memory itself. |
| Drainer/input threads never block | `kf3_read_trap` (drainer) only sets an atomic and schedules a BH: HOLDS (kf3.c:517-523). Rest UNVERIFIED. |
| Guest-reachable unbounded read/emit is a bug | C side: DBCPL capped at 4096; read-trap flip log is behind a diagnostic flag; no other per-guest-write log in kf3.c: HOLDS. Queue growth: C-08. |
| No behaviour flags needed to boot | HOLDS in kf3.c: every env/property read here is diagnostic or default-off (KF3_DOORBELL_CPL, trace, observer, doorbell-ioeventfd, dummy-bar). |

### 5. Tools

- `gcc -fanalyzer` on kf3.c: NOT POSSIBLE offline. The box has QEMU 11.1.1 (configured) and 9.2.0 trees only; kf3.c needs 10.2 headers (first fatal: `hw/qdev-properties.h: No such file`). Log: (not retained).
- No cargo build/test/miri (miri not installed; disk 4.7 GB).

### 6. Not reviewed

- kf3.h line by line (prototypes vs Rust: relied on `tests/wire_mirror.rs` existing; not run).
- The safe kf-qemu code beyond the call sites cited (mem.rs placement, chan.rs, userread/userstore/tspace/userdram, reset.rs, broker.rs, gop.rs); `Device::bar0_write_inner` callees in kf-trap/kf-chan; kf-broker's parser.
- `kf_linux_raw::HostSpan/GuestWindow::host_span` (other fork), kf-cuda `Frame` (other fork), tools/vfio-gsp-observer (other fork).
- QEMU core behaviour (alias clipping, RAM-block RCU free, ACPI eject) is cited from memory of QEMU 10.x, not re-read: every statement resting on it is marked INFERRED.
- No dynamic test of any finding.


---

## Appendix D (file D_kfcuda.md)

Part status as of 2026-11-13: Security review 2, PART D: kf-cuda unsafe surface + cuda/ launch contract.
Tree: origin/master 921c39444. Falsifiers: the falsifier list (not retained) (written first).
Labels: CODE READ / MEASURED / INFERRED. Severity rule (owner addendum,
2026-11-13): any guest breakout = CRITICAL; unbounded guest-steered DoS is a finding.

RESULT IN ONE PARAGRAPH. No guest-reachable host-memory corruption was found in this scope (CODE READ).
The hypothesis "every safe-to-unsafe boundary is validated" is FALSIFIED for kf-cuda: of 79 unsafe sites,
40 VALIDATED, 3 PANIC-ONLY, 33 VALIDATED-BY-CALLER-ONLY, 3 MISSING. The boundary that actually protects
the GPU today is the two SAFE files display.rs and walk.rs (which do bound store/slot/frame accesses);
`driver_unsafe.rs` itself exports raw device addresses, Copy handles and unchecked launch marshalling as
`pub` safe functions (stage-1 S1-04, still open; the fix sits unmerged on origin/v3-sec-rawaddr,
20a38e961, 2026-10-04). Every current in-tree launch matches its PTX signature (MEASURED, below).

### 0. What was run (MEASURED)
- `cargo test --offline -p kf-cuda --lib`: 53 passed .
- A PoC (not retained; a `#![forbid(unsafe_code)]` integration test, removed from the
  tree afterwards):
  - `zeroed::<&'static u64>()` -> process ABORT "attempted to zero-initialize type `&u64`, which is invalid"
    (Rust's own check). So `zeroed` is PANIC-ONLY, not UB. My initial suspicion was a false positive.
  - `view_bytes(&Padded{u8,u64})` compiles in safe code and returns 16 bytes including 7 padding bytes -> D-08.
- PTX signature extraction (scratch script ptxsig.py + awk over the clang-generated PTX): parameter count and
  byte size of every `.entry` compared with the `params` vectors built in walk.rs/display.rs:
  kf_walk.ptx 14 entries (KfArgs = 264 bytes; seed 4, expand 9, scan 4, compact 8, leaf 7, heads 6, bases 2,
  emit 8, join 7), kf_compose 18, kf_bl_pack 12, kf_sum 4, kf_color_validate/kf_tmo_validate 3,
  kf_color_compose 25, kf_color_output 7 (incl. a 48-byte by-value matrix), kf_compose_yuv/kf_color_yuv 34.
  ALL MATCH the host lists. (crates/kf-cuda/src/abi.rs:10 still says `[216]` in a comment; the PTX says 264.)
- No GPU: nothing here ran a kernel. libcuda behaviour statements are INFERRED (closed binary).

### 1. Site table: crates/kf-cuda/src/driver_unsafe.rs (79 unsafe sites; the only unsafe file of the crate)
Safe siblings abi.rs, walk.rs, display.rs, posture.rs, capacity.rs, completion.rs, selftest.rs, synth.rs,
refusal_samples.rs, diffmodel.rs contain no `unsafe` (grep; the word appears only in comments).

| line(s) | operation | SAFETY contract (documented?) | safe callers / arguments | validated at the boundary by | verdict (2026-11-13) |
|---|---|---|---|---|---|
| 43-53 | `extern "C"` dlopen/dlsym/dlerror/eventfd/read/write/poll | glibc prototypes (yes) | file-private | signatures match glibc; PollFd repr(C) | VALIDATED |
| 264, 266 | `unsafe impl Send/Sync for Cuda` | immutable fn pointers + AtomicU64 (yes) | - | struct has no interior mutability but the atomic; libcuda entry points are thread-safe | VALIDATED |
| 280, 360, 371 | `dlsym` | live handle, NUL-terminated name (yes) | `open_soname` only | CString; handle null-checked at 316 | VALIDATED |
| 312, 318 | `dlopen`, `dlerror` + `CStr::from_ptr` | (yes) | `Cuda::open[_soname](soname: &str)` | CString::new refuses interior NUL; null check | VALIDATED |
| 352, 365, 444 | `transmute(dlsym ptr -> fn pointer)` (sym!/opt!) | "the driver's ABI for this symbol" (yes) | open only | null check; versioned names (`_v2`); I compared all 48 field signatures with cuda.h from memory: consistent (INFERRED, no header in tree) | VALIDATED (trusts host libcuda) |
| 457, 459 | cuGetErrorName + CStr | (yes) | `check` | rc + null check | VALIDATED |
| 488-558 (6) | cuInit, cuDeviceGetCount/Get/GetByPCIBusId/GetName | out-pointers to locals (yes) | bring-up; `bdf: &str`, `ord`, `dev` ints | CString; 128-byte buffer with its length passed | VALIDATED |
| 572 | cuCtxCreate_v2 | (yes) | bring-up | out-pointer local; posture first | VALIDATED |
| 583 | cuCtxDestroy_v2 | "destroyed exactly once: WalkKernel::drop nulls its copy" (yes, names the CALLER) | `pub fn ctx_destroy(CtxHandle)`; handle is `Copy` | nothing | VALIDATED-BY-CALLER-ONLY (D-04) |
| 606 | cuCtxSetCurrent | handle from this binding (yes) | `make_current` (walk.rs:1949, display.rs:1739) | field is private (unforgeable) but may be stale after destroy | VALIDATED-BY-CALLER-ONLY (D-04) |
| 620 | cuCtxSynchronize | none needed | many | - | VALIDATED |
| 638 | cuModuleLoadData | NUL-terminated image (yes) | `module_load(&[u8])` | `assert!(last == 0)` at 629 | PANIC-ONLY |
| 653 | cuModuleGetFunction | (yes) | `module_function(ModuleHandle, &'static str)` | CString; private handle; modules are never unloaded | VALIDATED |
| 678, 680 | cuMemAlloc_v2 + cuMemsetD8 | (yes) | `mem_alloc_zeroed(bytes)` | the memset uses the same `bytes`; leak of `p` if the memset fails (D-15) | VALIDATED |
| 689 | cuMemFree_v2 | "came from mem_alloc_zeroed ... freed once" (yes, caller) | `pub fn mem_free(p: u64)`: any integer, any number of times | only `p != 0` | VALIDATED-BY-CALLER-ONLY (D-02, D-04) |
| 706 | cuMemcpyHtoD_v2 | host len = slice len (ok); "dst is a live allocation of at least that size, which every caller sizes" (yes, caller) | `pub fn memcpy_h2d(dst: u64, src: &[u8])`; in-tree: walk.rs:887, 1959, 2045, 2144; display.rs:898, 1182, 1805 + selftests | host side by the slice; DEVICE side by nothing here (walk.rs `store_at` :2023, display.rs `at` :1774 do it in safe files) | VALIDATED-BY-CALLER-ONLY (D-02) |
| 723 | cuMemcpyDtoH_v2 | as above | `pub fn memcpy_d2h(dst: &mut [u8], src: u64)`; walk.rs:2082, 2107, 2126; display.rs:1795 | host side by the slice; device side by callers | VALIDATED-BY-CALLER-ONLY (D-02) |
| 739 | cuLaunchKernel, ONE param | "the driver reads exactly the parameter size the module declares ... which `launch` asserts below" (documented, and the assert DOES NOT EXIST: `launch` at 1354-1363 only forwards) | `pub fn launch_raw/launch(Func, grid, block, &mut [u8])`; walk.rs:2164 | nothing: not the byte length, not the number of parameters | MISSING (D-01) |
| 779 | cuLaunchKernel, N params | "The array has exactly one entry per parameter" (yes, caller) | `pub fn launch_args(.., params: &mut [Vec<u8>])`; walk.rs:1734 (`kl`), display.rs:727, 1009, 1108, 1174, 1223, 1260, 1403, 1532 | nothing | MISSING (D-01) |
| 1302 | cuGraphExecKernelNodeSetParams | as launch_args (yes, caller) | `pub fn graph_exec_kernel_set`; walk.rs:1619 | nothing | MISSING (D-01) |
| 816, 834, 850 | cuMemcpyDtoDAsync, cuMemsetD8, cuMemsetD8Async | "live allocations of at least n bytes (callers size them)" (yes, caller) | pub; walk.rs:1486, 1651, 2062; display.rs:712, 856, 1056-1057, 1092, 1443, 1818 | nothing here; callers bound (store_at/at/slot length/fixed constants) | VALIDATED-BY-CALLER-ONLY (D-02) |
| 875, 896 | cuStreamCreate, cuEventCreate | (yes) | bring-up | out-pointer local | VALIDATED |
| 885, 906, 1338, 1346 | cuStreamDestroy, cuEventDestroy, cuGraphExecDestroy, cuGraphDestroy | "destroyed once by its owner" (yes, caller) | pub; Copy handles | only `!= 0` | VALIDATED-BY-CALLER-ONLY (D-04) |
| 916, 929, 943, 1164, 1179, 1203, 1240, 1253, 1315, 1329 | event record/query/elapsed, capture begin/end/info, graph instantiate/launch/upload, record-external | "handles came from this binding" (yes) | walk.rs, display.rs | provenance: private tuple fields (only `CtxHandle::null()`, `StreamHandle::legacy()` are mintable); LIFETIME: none (Copy, usable after destroy / after the context died) | VALIDATED-BY-CALLER-ONLY (D-04) |
| 1225 | `*deps` | n == 1 and non-NULL (yes) | `capture_leaf` | checked at 1214 | VALIDATED |
| 963 | cuLaunchHostFunc(stream, completion_hostfn, fd-number-as-pointer) | "the fd outlives every queued callback because WalkKernel::drop destroys the context ... before its CompletionFd drops" (yes, caller) | `pub fn launch_host_signal(StreamHandle, &CompletionFd)`; walk.rs:997, display.rs:1318 | nothing ties the fd's life to the queued callback | VALIDATED-BY-CALLER-ONLY (D-07) |
| 983 | cuMemAllocHost_v2 | (yes) | pub(crate) `pinned_alloc(len)` | len recorded in PinnedBuf | VALIDATED |
| 998 | cuMemFreeHost | "only once no queued copy can still target it (the caller observed ...)" (yes, caller) | pub(crate); display.rs:692 (after ctx_synchronize), 827 (`release_frame`, no sync) | consumes the PinnedBuf (no double free); pending-DMA state unchecked | VALIDATED-BY-CALLER-ONLY (D-05) |
| 1021, 1024 | `HostSpan::as_ptr`, cuMemHostRegister_v2 | "the caller keeps the mapping alive ... display::Frame owns both" (yes, caller = a safe file) | pub(crate) `host_register(&HostSpan)`; display.rs:790 | span length from kf-linux-raw; lifetime by `Frame` (display.rs:300-343) | VALIDATED-BY-CALLER-ONLY (D-05) |
| 1046 | cuMemHostUnregister | "only once no queued copy can still target the pages" (yes, caller) | display.rs:819 | consumes the PinnedBuf | VALIDATED-BY-CALLER-ONLY (D-05) |
| 1069 | cuMemHostGetDevicePointer_v2 | (yes) | walk.rs:898-925 | `assert!(off < buf.len)` | PANIC-ONLY |
| 1104, 1132 | cuMemcpyHtoDAsync / DtoHAsync on `buf[off..off+n]` | host range asserted (checked_add); "the caller does not rewrite/read the range until the stream passed" (yes, caller) | 1088 dead; `memcpy_d2h_async`: display.rs:728, 1266, 1347, 1568 | host range: assert (PANIC-ONLY, checked arithmetic); device `src`+n: callers; buffer still pinned when the DMA lands: callers | VALIDATED-BY-CALLER-ONLY (D-02, D-05) |
| 1442, 1459 | `copy_nonoverlapping` out of / into pinned memory (`PinnedBuf::read/write`) | range asserted; "DMA completed" (yes, caller) | walk.rs:896, 1179-1207, 1383-1390; display.rs:330 (`pub Frame::read`), 752, 1286 | range: assert with checked_add; raw-pointer copy, no reference formed over DMA/shared memory (good); ALLOCATION STILL ALIVE: nothing (D-05a) | VALIDATED-BY-CALLER-ONLY (D-05) |
| 1484, 1493, 1515, 1530, 1548 | eventfd, OwnedFd::from_raw_fd, read(8), poll(1), write(8) | (yes) | CompletionFd | fd < 0 checked; OwnedFd closes once; 8-byte locals | VALIDATED |
| 1633 | `slice::from_raw_parts(&T as *const u8, size_of::<T>())` (`view_bytes<T: Copy>`) | "T contains no padding ... discharged at every call site" (yes, caller) | `pub fn`, any `T: Copy`; walk.rs:701 (KfArgs, KfDev, KfLayout, KfAck), abi.rs:1317 | nothing (no trait bound, no const assert for KfFormat/KfArgs) | VALIDATED-BY-CALLER-ONLY (D-08) |
| 1663 | `mem::zeroed::<T>()` (`zeroed<T: Copy>`) | "all-zero is valid for T" (yes, caller) | `pub fn`; abi.rs:945, walk.rs:877, 1064 | Rust's zero-validity assertion aborts for references/NonZero/niche types (MEASURED) | PANIC-ONLY |
| 1759 | cuMemImportFromShareableHandle(fd as pointer) | (yes) | `pub fn import_and_map(device, fd: i32, bytes)`; walk.rs:1999, display.rs:1421, 1763 | fd is a bare i32; a negative fd becomes 0 = stdin (`unwrap_or(0)`, 1762); no ownership | VALIDATED-BY-CALLER-ONLY (D-14, no memory effect) |
| 1771, 1776, 1794 | cuMemAddressReserve / cuMemMap / cuMemSetAccess | (yes) | same | locals; the driver refuses a map larger than the handle (INFERRED); leaks on error (D-15) | VALIDATED |
| 1825, 1841, 1848, 1859 | cuMemGetAllocationGranularity / Create / Export / Release | (yes) | `export_device_allocation`: no caller in the tree | locals; repr(C) prop matches cuda.h (32 bytes) | VALIDATED (dead API; returns a raw fd and never releases the handle) |

Not unsafe but part of the boundary: `completion_hostfn` (1544, `extern "C"`, fd number in a pointer, never
dereferenced: VALIDATED); `CtxHandle/StreamHandle/EventHandle/ModuleHandle/Func/Graph*` (private `usize`
fields: unforgeable, but `Copy` and lifetime-free); `PinnedBuf` (pub(crate), private fields, no Drop).

### 2. cuda/ launch contract (host-side sizes vs kernel)
| kernel(s) | host buffer sizing (walk.rs / display.rs) | in-kernel bound | verdict |
|---|---|---|---|
| walk: table reads (`KF_GPGA_DEREF`, kf_walk.cu:168, 466, 476, 494, 1804) | win.base/len = the imported store (walk.rs:1065-1074, from `self.store` only) | overflow-free `off > len || n > len - off`; loops `i < KF_MAX_ENT && i < descriptor`; entry budget, frontier cap | VALIDATED in-kernel (stage-1 §2.7 not redone; spot check only) |
| walk: leaf targets | span = store length | kf_walk.cu:552, 1733 `gpga > span || len > span - gpga` | VALIDATED in-kernel |
| walk: `KfLayout` regions (walk_off/cap, slot_off/cap, prev_*) | `Capacity::plan` (capacity.rs:225-283, safe) keeps regions inside walk_pool/slot_pool; scratch 4x and iscratch 3x the walk pool (walk.rs:811, 851, 864) | NONE on region bounds: `KfDev` carries no pool size; only `slot_cap <= walk_cap` (kf_walk.cu:1026) and `s < max_slots` | VALIDATED-BY-CALLER-ONLY, device side (D-12) |
| walk: ack (`nrun`, `reset[]`, codes) | codes copied `min(len, run_capacity)` (walk.rs:1197-1200); nreset <= 64 | commit requires `nrun == h->run_count`, not TRUNCATED, `first_run + run_count <= run_count` (u64), `nreset` clamped, `s < max_slots` (kf_walk.cu:1283-1301) | VALIDATED in-kernel |
| walk: report written to pinned HOST memory (hdr/rpdb/rrun) | PinLayout::for_cfg (walk.rs:487-511) sized from run_capacity/pdb_capacity; decode clamps `pdb_count`/`run_count` to capacity (walk.rs:1387-1388) | kernel clamps to KfDev capacities (kf_walk.cu:1210-1239) | VALIDATED both sides |
| kf_compose (hand-written PTX) | reads: `ComposeLayer::check` + `at()`; writes: `check(w,h)` vs the frame the CALLER names; NOT vs the staging allocation (D-03) | clips to fw x fh (kf_scanout.ptx:82, 113, 116); no allocation bound | VALIDATED-BY-CALLER-ONLY (D-03) |
| kf_compose_yuv / kf_color_yuv | `YuvLayer::check`; color path checks the frame length (display.rs:960); the XRGB path does not (D-03); packed chroma under-counted (D-09) | `dx >= fw || dy >= fh` skip; source clamped to the rectangle | caller-only for w,h (D-03) |
| kf_color_compose / kf_color_output / validate kernels | frame, LUT slot (<= 64), entries (65..=1025 / 257,1025), pipeline program (`ColorPipeline::bytes`) all checked before launch (display.rs:1060-1270) | every LUT index clamped in-kernel (kf_color.cu `lookup`, `tone_lookup`, `inline_lookup` bounded by the validated program) | VALIDATED |
| kf_bl_pack | `BlPack::check` against the slot and staging lengths (display.rs:206-240, 1384) | `g >= gobs`, `y < h`, `px < w` | VALIDATED |
| kf_sum | `n <= staging len`, rows buffers sized `h*8` (display.rs:677-707) | one CTA per row | VALIDATED |

### 3. Findings (2026-11-13)
No finding below has a demonstrated guest path. "Safe-caller" = a bug or hostile code in a non-unsafe file.

### D-01 HIGH. Kernel-launch marshalling trusts the caller for parameter count and sizes (MISSING). CODE READ.
- crates/kf-cuda/src/driver_unsafe.rs:733-754 (`launch_raw`), 762-795 (`launch_args`), 1265-1305
  (`graph_exec_kernel_set`), 1354-1363 (`launch`). The SAFETY comment at 735-738 cites an assert in `launch`
  that is not there.
- libcuda takes the number and size of parameters from the kernel's signature, not from our arrays.
- Scenario (safe code, any crate, `Cuda` is `pub`): `cu.launch_args(s, f_compose, 1, 1, 0, &mut [vec![0u8; 1]], "x")`.
  libcuda reads 18 pointers from a 1-element `Vec<*mut c_void>` (17 heap words past the allocation) and then
  dereferences each as a parameter buffer: wild host reads, SIGSEGV or VMM heap bytes copied into GPU kernel
  arguments. With a short buffer for the 264-byte KfArgs: a 263-byte over-read of the host heap.
- Today: all 20 in-tree launches match the PTX (MEASURED §0). One edit to a `.cu` signature without the
  host list (or the reverse) turns into this silently; only kf_color has a source test (display.rs:1860).
- Minimal fix: `module_function` parses the `.entry` parameter list of the PTX it just loaded (text, in hand)
  and stores the sizes in `Func`; `launch_*`/`graph_exec_kernel_set` assert `params.len()` and every
  `params[i].len()` before the call (panic = acceptable). Add the missing assert the comment promises.

### D-02 HIGH (= stage-1 S1-04, OPEN on master). Raw device addresses in the public safe API. CODE READ; effect INFERRED.
- driver_unsafe.rs:685 (`mem_free`), 697 (`memcpy_h2d`), 715 (`memcpy_d2h`), 805 (`memcpy_d2d_async`), 825
  (`memset_d8`), 841 (`memset_d8_async`): `CUdeviceptr = u64`, any value, no length object.
- Scenario: under CUDA's unified addressing a page-locked HOST buffer's address is a valid `CUdeviceptr`
  (`cuMemAllocHost`, `cuMemHostRegister`). Safe code doing `cu.memcpy_h2d(frame.addr() as u64 + k, bytes)`
  (the address is handed out by `display::Frame::addr`, display.rs:308) or `cu.memset_d8(pin_addr, 0xff, n)`
  writes VMM host memory inside those regions: the console frames, the memfd frames shared with the broker,
  the walker's pinned report/ack/layout buffer (which the next walk then trusts). A wrong device address
  reaches the imported store (guest VRAM) or a display slot shared with the host compositor.
  Whether libcuda also rejects ranges outside tracked allocations is not documented (INFERRED: it does).
- OWNER_RULINGS.md:650-658 forbids exactly this ("A precondition left to callers is forbidden") and says the
  conversion is on `v3-sec-rawaddr`; `git merge-base --is-ancestor origin/v3-sec-rawaddr origin/master` =
  NOT MERGED (MEASURED); the branch is 2026-10-04 and predates the colour/YUV/graph code.
- Minimal fix: port the branch's shape: an owned `DevBuf { ptr, len }` minted only in driver_unsafe.rs; every
  copy/set takes `(buf, off, len)` and checks with `checked_add`; the raw `Cuda` methods become private.
  The store and slot imports return such a buffer, so `store_at`/`at` move into the perimeter.

### D-03 HIGH. The compose launches do not check the frame against the staging allocation. CODE READ.
- crates/kf-cuda/src/display.rs:1494-1533 `launch_compose` (`let Some((dst, _)) = self.staging`: the length
  is discarded), reached from `compose_layer` :864, `compose_cursor` :909; `compose_yuv` :938-946 (`(dst, _)`).
  `ComposeLayer::check(w, h)` / `YuvLayer::check(w, h)` bound the writes by the `w x h` the CALLER passes,
  and the kernels clip to the same pair. `color_yuv` :960, `color_pipeline_layer` :1152-1158, `color_output`
  :1243, `compose_checksum` :679 and `compose_to_host` :1344 do check the allocation; these three do not.
- Scenario (safe-caller): `compose_begin(64, 64)` (16 KiB staging) then
  `compose_layer(&ComposeLayer{ox:0, oy:16000, width:64, rows:1, ..}, 16384, 16384)`: the kernel writes at
  `staging + 16000*16384*4` (about +1 GiB): a GPU write to whatever the display context has mapped there:
  the imported store (all guest VRAM), an imported display slot (memory the HOST compositor shows), a
  page-locked console/broker frame (host RAM), or, on a host where HMM is active for the context
  (S1-05; master never queries `CU_DEVICE_ATTRIBUTE_PAGEABLE_MEMORY_ACCESS`), VMM memory. Pixel bytes are
  guest-controlled.
- Guest reachability: none found. kf-qemu/src/display.rs:5418-5470 passes the same `(w, h)` to
  `compose_begin`/`color_begin` and to every layer, and the staging frame only grows. If a later change let
  those differ under a guest-set mode, this becomes a CRITICAL (guest-steered write outside the VM's memory).
- Minimal fix (4 lines): in `launch_compose` and `compose_yuv`,
  `if u64::from(w) * u64::from(h) * 4 > len as u64 { return Err(refused(what, "the frame is larger than the staging frame")) }`.

### D-04 MEDIUM. Copy handles with public destroy functions: double destroy / use after destroy. CODE READ; effect INFERRED.
- driver_unsafe.rs:580-584 `ctx_destroy`, 882-887 `stream_destroy`, 903-908 `event_destroy`, 1335-1348
  graph destroys, 685 `mem_free`; handle types 1368-1398, 1576-1594 are `Clone + Copy`;
  crates/kf-cuda/src/walk.rs:2148 `release(DeviceImage)` with `DeviceImage: Copy` (2226).
- Scenario: `let s = cu.stream_create()?; cu.stream_destroy(s); cu.stream_destroy(s);` or any call with `s`
  afterwards: libcuda receives a freed `CUstream` (a host heap pointer). `kernel.release(img); kernel.release(img)`
  or `write_image(&img, ..)` after `release`: a double `cuMemFree` / a copy into freed device memory, which
  may by then belong to another allocation of the context. Whether libcuda detects stale handles is
  undocumented.
- In-tree owners (`WalkKernel::drop` walk.rs:2178-2217, `DisplayGpu::drop` display.rs:1822-1852) destroy each
  once (CODE READ).
- Minimal fix: handles not `Copy`; destroy consumes; `DeviceImage` not `Copy`; make the raw methods
  `pub(crate)`.

### D-05 MEDIUM. Page-locked frames have no lifetime tie to the context or to queued DMA. CODE READ; (b) INFERRED.
- (a) Use after free from safe code: `DisplayGpu::frame` (display.rs:764) returns a `Frame` backed by
  `cuMemAllocHost`; `DisplayGpu::drop` (1851) destroys the context, which reclaims that memory
  (driver_unsafe.rs:1404 says so), while `Frame::read` (330 -> driver_unsafe.rs:1433-1446) and `Frame::addr`
  (308) stay usable: `let f = gpu.frame(4096)?; drop(gpu); f.read(0, 4096)` copies from unmapped/reused host
  memory. The same address is what kf-qemu hands the C console (`FrameView.addr` -> `Kf3Frame.data`), so a
  context torn down while QEMU's console still holds a frame is a C-side read of freed memory (device
  teardown / hot-unplug, stage-1 S1-80).
- (b) `release_frame` (display.rs:811-829) unregisters and unmaps (or `cuMemFreeHost`s) without draining the
  stream; the contract is "the caller's frame state machine decides" (806). If a D2H copy queued by
  `compose_to_host` still targets the frame, the DMA lands after the pages were released: a write of frame
  bytes (guest-controlled pixels) into host pages that may have been reused. Not known whether libcuda
  drains first. On master it is called only on error paths before any copy (kf-qemu/src/broker.rs:209, 212),
  and the worker keeps frames for the device's life (`frames`/`retired`, kf-qemu/src/display.rs:4301-4302).
- Minimal fix: `release_frame` calls `ctx_synchronize` first (display worker, never a vCPU), as
  `compose_checksum` :689 and `compose_begin` :846 already do before freeing; `Frame` holds a liveness token
  of its context that `read` asserts; `DisplayGpu::drop` leaks the context if frames are outstanding.

### D-06 MEDIUM (= stage-1 S1-03, OPEN on master). The console frame's host address crosses safe code as `usize`, with no length. CODE READ.
- crates/kf-cuda/src/display.rs:308 (`pub fn addr`), driver_unsafe.rs:1420; crates/kf-qemu/src/display.rs:883-885
  (`pub struct FrameView { pub addr: usize, .. }`, built in safe code; tests build `frame(0x1000, 1)` at :7325);
  crates/kf-qemu/src/ffi_unsafe.rs:800-816 (`data: f.addr as *mut u8`; `Kf3Frame` has width/height/stride, no
  buffer length).
- Scenario (safe-caller): a `FrameView` with a wrong `addr`, or `width*height*4` larger than the frame, makes
  QEMU's console (C) read arbitrary VMM memory and show it on the host display/VNC, or fault.
- Fix: the perimeter mints the frame view (address + length checked against the `PinnedBuf`), as on
  `v3-sec-rawaddr`. Cross-reference for the kf-qemu part of this review.

### D-07 LOW. `launch_host_signal` passes an fd NUMBER whose lifetime is the caller's. CODE READ.
- driver_unsafe.rs:958-970, 1544-1549.
- Scenario: `cu.launch_host_signal(s, &fd)?; drop(fd);`: the driver thread later runs `write(n, &1u64, 8)` on a
  closed number, or on whatever file reused it (an eventfd gets a spurious wake; a socket or memfd gets 8
  stray bytes). No memory effect. In-tree: `DisplayGpu::drop` destroys the stream and the context before the
  field closes (display.rs:1822-1852), relying on `cuCtxDestroy` draining host callbacks (INFERRED).
- Fix: `dup` the fd into a leaked `Box<OwnedFd>` passed as the user datum and closed by the callback.

### D-08 LOW. `view_bytes<T: Copy>` is a generic safe function over padding; one real instance. MEASURED (compiles, 16 bytes) + CODE READ.
- driver_unsafe.rs:1627-1634. Any safe code can read padding bytes (uninitialised memory) through `&[u8]`.
- In-tree instance: `KfFormat` has implicit padding after `small_ps` and after `pcf_sparse`
  (driver_unsafe.rs:1644-1646 says so; abi.rs:187-215), `kf_format_ver2` zeroes it, but `KfFormat` is then
  moved by value (kf-qemu -> `bring_up_on` -> `self.fmt` -> `a.fmt = self.fmt`, walk.rs:1075) and a typed copy
  does not preserve padding. So the 264 bytes given to `cuLaunchKernel` on every walk formally contain
  uninitialised bytes (in practice the compiler memcpys). Information-only effect: stack bytes to our own
  GPU kernel, which reads named fields only.
- Fix: explicit `pad` fields in `KfFormat` (as `KfDev::sample_pad`), a const assertion that the fields tile
  the struct for every type passed (abi.rs:758 already does it for the report structs), and a sealed
  `unsafe trait` bound on `view_bytes`/`zeroed`.
- FALSE POSITIVE rejected: `zeroed::<&T>()` / `NonZero` is an abort, not UB (MEASURED §0).

### D-09 LOW. Packed YUV (YUYV/UYVY) extent under-counts the chroma read by up to 2 bytes. CODE READ.
- crates/kf-cuda/src/display.rs:536-541 (`need` uses 2 bytes per luma pixel, chroma 0); cuda/display/kf_yuv.cu
  `kf_chroma` cases 2/3 read byte `4*(x>>1) + 3`; for an odd `sx0+sw` that is `2*x_end + 1` on the last row,
  two bytes past `y_extent`. The planner has the same formula (crates/kf-disp/src/scanout.rs:851).
- Guest: the two bytes are inside the store (the planner's pitch is a multiple of 64 and offsets are
  256-aligned, so the extent cannot end at the store's last byte; parity argument, CODE READ): the guest reads
  its own VRAM just outside the surface's context DMA. Safe-caller with `y_pitch == 2*x_end` at the end of the
  store: a 2-byte GPU read past the store mapping (fault = the display context dies, or a neighbour mapping).
- Fix: for `layout >= 2` use `4 * x_end.div_ceil(2)` bytes per row in both checks.

### D-10 LOW (DoS, log flood). Capacity events are logged without a bound. CODE READ.
- crates/kf-cuda/src/walk.rs:1176, 1446, 1463, 1476, 1493, 1504 push one string per event;
  crates/kf-mem/src/vasmgr.rs:401-403 prints each (`kf3: walk ...`).
- Guest: 64 address spaces each mapping more than 16 384 non-contiguous 4 KiB runs (about 68 MiB of scattered
  guest RAM each) exhaust the 1 Mi-run walk pool; every later walk the guest triggers (one MMU invalidate)
  is refused with one or more "capacity ceiling: walk pool exhausted ..." lines. Rate = the guest's
  invalidate rate; no counter, no rate limit. V3_SECURITY_AUDIT_PLAN.md §2.4 names "unbounded log lines" as
  a known class; no limit for this line is in place (planned in general, not in place here).
- Fix: log the first N (as refusal_samples does: 32 lines per process) and count the rest.

### D-11 LOW (DoS, host VRAM/handles), UNVERIFIED bounds. CODE READ.
- display.rs:836-857 `compose_begin` allocates up to 16384 x 16384 x 4 = 1 GiB of device memory outside
  `fb-mb`; display.rs:1412-1429 `import_slot` has no count cap (each <= 64 MiB); driver_unsafe.rs:1766-1797
  leaks the imported handle and the reservation on each failed step. The effective bounds are the callers'
  (`kf_disp::scanout::MAX_PIXELS`, the slot ring), not checked in this part. Related: stage-1 S1-82, S1-25.

### D-12 MEDIUM (device side). The walk kernels trust the host-authored `KfLayout` regions. CODE READ.
- cuda/walk/kf_walk.cu:813-814, 1010-1043, 1258, 1297-1305, 2290-2347 use `lay->walk_off/cap`,
  `slot_off/cap`, `prev_off/cap` with no check against a pool size (`KfDev`, :174-208, carries none).
  cuda/walk/kf_walk.h says "the kernel refuses by name if broken"; only `slot_cap <= walk_cap` is checked.
- The only guard is `Capacity` (crates/kf-cuda/src/capacity.rs:225-283, safe code). A bug there makes four
  kernels write `KfMapRun`s (va/gpga/len decoded from guest PTEs) past `walk`/`com`/`scratch`/`iscratch`
  inside the walker context: the next allocations, the pinned report/ack buffer, the imported store (guest
  VRAM: the guest's own), or VMM memory where HMM is active (S1-05).
- Fix: add `walk_pool`/`slot_pool` to `KfDev` and refuse `off + cap > pool` by name in the kernels (ABI 7),
  and/or validate the `KfLayout` in the perimeter before `pin.write(pin_at.lay, ..)` (walk.rs:1179).

### D-13 INFO. Posture memo is never re-verified. CODE READ. (2026-11-13)
- crates/kf-cuda/src/posture.rs:53-63: after the first clear, `DONE` short-circuits; the permitted set keeps
  CAP_SYS_ADMIN, so safe code on a CUDA thread can raise it again and later libcuda calls would allocate
  ADMIN channels with no refusal. Only `open_soname`, `init`, `ctx_create`, `ctx_set_current` call
  `cuda_thread` (driver_unsafe.rs:303, 485, 568, 603). No in-tree code re-raises on those threads.

### D-14 INFO. Raw fd integers at the import API. driver_unsafe.rs:1745, 1762 (`unwrap_or(0)`: a negative fd
becomes stdin); walk.rs:1982; display.rs:1412, 1747. Take `BorrowedFd<'_>`.

### D-15 INFO. Error-path leaks at bring-up. walk.rs:782-928 and display.rs:578-624: a `?` after
`ctx_create` leaks the context and every allocation (no guard object yet); driver_unsafe.rs:678-681 leaks the
allocation when the zeroing fails. Realize failure only; no double free found.

### 4. Stage-1 findings in this scope: status on master (CODE READ)
| id | status | evidence |
|---|---|---|
| S1-03 console frame address as usize, no length | OPEN | D-06 |
| S1-04 kf-cuda safe API takes raw device addresses | OPEN (fix unmerged on origin/v3-sec-rawaddr) | D-02 |
| S1-05 unified addressing/HMM; compose kernel has no in-kernel bound | OPEN, narrowed by OWNER_RULINGS.md:600-605 (HMM not refused; kernel bounds are host-memory-safety obligations). No PAGEABLE_MEMORY_ACCESS query on master. kf_compose clips to fw x fh only; yuv/colour/pack have in-kernel guards | D-03, D-12 |
| S1-22 libcuda half (CUDA threads without CAP_SYS_ADMIN) | FIXED in code | posture.rs; driver_unsafe.rs:303, 485, 568, 603; kf-qemu `on_cuda_thread` device.rs:527, 560, 933 |
| S1-60 VER3: an unmapped big PTE does not veto stale 4 KiB PTEs | OPEN | cuda/walk/kf_walk.cu:409-414 (`kf_big_pte_unmapped` returns false unless VER2); walk.rs:760-763 states the gap |
| S1-61 walk resources shared per refresh | OPEN (pool is per VM: one space can exhaust it for all; D-10 is its log side) | capacity.rs:254-262 |

### 5. Owner rules touched in this scope
- No CPU executor for GPU work, no forged completion: HOLDS for the walker: completion is `cuEventQuery`
  only; a signalled fd cannot complete a walk (completion.rs `ready`, tests there; walk.rs:1362-1373).
  UNVERIFIED for the display: its completion is the host-function fd (display.rs:1316-1319) and
  `CompletionFd::signal` is `pub` (driver_unsafe.rs:1505), so safe code could signal early; kf-qemu also has
  `signal_state` (event query). No guest path.
- Nothing blocks on a vCPU: HOLDS by CODE READ of the callers: every libcuda call is made on
  `kf3-cuda-walk`/`kf3-cuda-store`/`kf3-cuda-disp` (kf-qemu/src/device.rs:527, 560, 933), the VA-manager
  thread (device.rs:1990; mem.rs:3344-3363, 3532; scrub.rs:255-262) or the display worker. kf-cuda itself
  cannot enforce it; blocking entries (`wait`, `refresh`, `sync_store`, `wait_readable`) are marked in docs only.
- Walker treats guest tables as hostile: HOLDS on spot check (one dereference expression with an
  overflow-free bound; loops bounded by KF_MAX_ENT, budget, frontier caps; one read per entry, volatile).
  Not re-audited in depth; S1-60 (VER3) open.
- CAP_SYS_ADMIN cleared for the life of every CUDA thread: HOLDS at the four entries; caveat D-13.
- Derive, never capture: not touched (KfFormat comes from kf-qemu; out of scope).
- Guest-reachable unbounded read/emit: VIOLATED for one log line (D-10); refusal samples are bounded (32).

### 6. Not reviewed
- The 12.8k-line kf_walk.ptx and the generated display PTX were not diffed against their `.cu` beyond entry
  signatures; kf_walk.cu's diff/commit/emit kernels were read only around the layout/ack uses, not line by line.
- cuda/walk-v2-frozen/, kf_tests.cu, kf_bench.cu, kf_corpus.cpp, kf_real_tables.py, Makefile, run_on_box.sh
  (harness, not shipped); kf_report_emit.c skimmed (fixed data, test only).
- abi.rs decoders, diffmodel.rs, synth.rs, selftest.rs, refusal_samples.rs: safe code, skimmed for callers only.
- kf-qemu/kf-mem/kf-disp callers were read only where they feed kf-cuda arguments (display.rs:5360-5680,
  device.rs:520-600, vasmgr.rs:395-545, mem.rs:3340-3365, 3525-3535); their own logic belongs to other parts.
- libcuda's real behaviour on stale handles, out-of-range device addresses and frees with pending DMA: not
  measurable here (no GPU, closed binary). Every such statement is INFERRED.
- crates/kayfabe-cuda (frozen pre-v3 twin of driver_unsafe.rs): not reviewed.
- No Miri (not installed), no hardware run.


---

## Appendix E (file E_broker.md)

Part status as of 2026-11-13: Security review 2, PART E: the C display broker (`broker/`) and its Rust peer (`crates/kf-broker`, `crates/kf-qemu/src/broker.rs`). Tree: origin/master 921c39444. Read-only review; nothing in the product tree was changed. Falsifiers: the falsifier list (not retained) (F10 is the one that applies to C; F3/F4/F9 to the fd and mapping paths).

Severity rule used (addendum of 2026-11-13): guest-steered corruption of the broker, or broker-steered corruption of the VMM, is CRITICAL; unbounded DoS counts as a finding unless a planned limit is documented.

### 0. Result in one paragraph

No memory-corruption finding. **MEASURED:** the broker built with `-fsanitize=address,undefined` passes its own suite with no sanitizer report (5 unit binaries: 66151 + 65431 + 80752 + 566237 checks and utf8; `selftest.sh` 91 checks, which includes the hostile-client harness; 6 python socket suites); `gcc 15 -fanalyzer -Wconversion` over 12 translation units: 0 warnings; `clang 21 --analyze`: 0 warnings (the gcc analyzer was sanity-checked on a planted use-after-free: it reports). **CODE READ:** every VMM-to-broker field that reaches a memory operation is bounded at the point of use or one call above it in the same file, with 64-bit size arithmetic. What I did find is DoS by logging in both directions (E-01 measured, E-02 code read) and three low/info items. The falsifier F10 was not met for corruption; it IS met for "unbounded loop/emit driven by peer values" (E-01, E-02). (2026-11-13)

### 1. Entry points where peer- or guest-influenced data reaches a memory operation

Who supplies: **V** = the VMM (kayfabe; in the broker's own model the VMM is hostile), **G** = the guest through an honest kayfabe (frame geometry, cursor image/size/hot spot/position, XOR mask), **C** = the compositor / X server (the user's session, trusted by the broker), **B** = the broker (untrusted by kayfabe), **L** = local user (argv).

| # | location | operation | precondition; documented? | supplied by | what validates it, where | verdict (2026-11-13) |
|---|---|---|---|---|---|---|
| 1 | `nvkvm_broker.c:3111-3131` `nb_sink_readable` | `recvmsg` into `s->rx + rxlen`, len `CMD_SIZE - rxlen` | rxlen < 40; yes (comment 2997-2999) | V | iov_len is the remainder; `rx[40]` (`nvkvm_broker.h` sink), rxlen reset at 40 (3198) | VALIDATED |
| 2 | `nvkvm_broker.c:3112-3187` | SCM_RIGHTS parse: `CMSG_DATA` memcpy of `nfd` ints | control buffer holds 4 fds; yes (HARDENING 5) | V | kernel bounds cmsg to `msg_controllen`; every fd seen is banked or dropped; >1 fd, a second fd on a partial command, or `MSG_CTRUNC` = violation; banked fd dropped in `nb_client_state_reset` (156-159) | VALIDATED |
| 3 | `nvkvm_broker.c:2573-2989` `nb_handle_cmd` | dispatch on `c->type` | no table: a `switch`, default = violation | V | switch; unknown flags/type/reserved1 = violation, fd dropped on every path (checked each case) | VALIDATED |
| 4 | `nvkvm_broker.c:1886-2191` `nb_validate_desc` | geometry vs the real fd: `need = stride*height + offset` | yes (HARDENING 1-3, S-1..S-4) | V, G (w/h/stride) | w,h in 1..8192; stride in [w*bpp, 2*w*bpp+4096]; `need` in u64, `<= INT32_MAX`, `<= lseek(SEEK_END)`; fd proved dma-buf (fdinfo `exp_name` + `DMA_BUF_MAGIC`) or, for F_SHM, tmpfs with `F_SEAL_SHRINK` | VALIDATED |
| 5 | `nb_session_wl.c:978-1013` `wl_attach` shm | `wl_shm_create_pool(fd, (int32_t)need)`, `create_buffer(offset,w,h,stride)` | "nb_validate_desc already enforced"; yes (`nvkvm_broker.h` nb_buf_desc comment) | V, G | only the core's check (row 4); no local re-check of `need <= size`/`INT32_MAX` (the X11 twin re-checks, row 7); the compositor re-validates pool/buffer extents | VALIDATED-BY-CALLER-ONLY (E-05) |
| 6 | `nb_session_wl.c:1074-1128` | dma-buf handed to `zwp_linux_buffer_params_v1` (probe, then `create_immed`) | pair advertised and geometry proven by an async probe; yes | V, G | row 4 + `proven[]`/`geoms` gate; a full table rejects (1034) | VALIDATED |
| 7 | `nb_session_x11.c:371-406` | `mmap(PROT_READ, need)` of an F_SHM memfd | size >= need; seal | V, G | re-checks `d->size < need` (375); seal proved in row 4 | VALIDATED |
| 8 | `nb_session_x11.c:727-758` `x11_commit` | row memcpy `packed` bytes from `map+offset+y*stride`; PutImage of w*h*4 | stride >= w*4; map_len = stride*h+offset | V, G | max read offset+(h-1)*stride+w*4 <= map_len by rows 4 and 7; malloc `packed*h` <= 256 MiB, checked for NULL | VALIDATED |
| 9 | `nb_session_x11.c:508-530` | DRI3 `pixmap_from_buffer(s)` casts to CARD16/CARD32 | no truncation | V, G | refuses stride > 0xffff or size > 4 GiB before the cast (508); w,h <= 8192 | VALIDATED |
| 10 | `nb_cursor.c:26-127` `nb_cursor_check` | classifies a CURSOR record | pure; yes | V, G | w,h 1..256; hot < w,h; stride in [w*4, 1024]; fourcc AR24; flags/reserved framing | VALIDATED |
| 11 | `nvkvm_broker.c:2261-2354` `nb_cursor_read` | `pread` of `span` bytes into `static staging[1024*256]` | span <= sizeof staging | V, G | bound restated at the write (2305); `offset+span <= st_size` in u64 (2317); short read = refusal; shmem proved before the read | VALIDATED |
| 12 | `nb_cursor.c:152-194` `nb_cursor_load` | writes `dst->px[y*w+x]`, reads `src[y*stride+x*4+3]` | w,h <= 256, stride >= w*4, src holds span; documented in the header comment of `struct nb_cursor` | V, G | nothing inside the function; its one caller (row 11) holds it | VALIDATED-BY-CALLER-ONLY (E-05) |
| 13 | `nb_cursor.c:320-357` `nb_cursor_set_xor`, `271-318` `_xor_approx` | memcpy px->mask `w*h*4`; in-place outline | w,h <= 256 | G (mask bits) | re-checks `defined`, w,h in 1..256 at entry (282, 328) | VALIDATED |
| 14 | `nb_cursor.c:359-399` `nb_cursor_copy` | memcpy px/mask `n*4` | n <= array | V, G | n clamped to the array size (391) | VALIDATED |
| 15 | `nb_cursor.c:723-788` `nb_cursor_crop` | row memcpy into `dst->px` | crop box inside src | G | w,h <= 256 and hot inside checked at entry; box clamped to the image | VALIDATED |
| 16 | `nb_cursor.c:196-209` `nb_cursor_move_clamp`; `nvkvm_broker.c:2430-2469` | MOVE position into int32 state | abs value <= 2^20 | V, G | refused past the limit, clamped to -256..8192 | VALIDATED |
| 17 | `nb_cursor.c:216-257` `nb_hot_move` | writes `c->hot_x/hot_y` | new hot inside the image | V, G | `0 <= dx < c->w`, `0 <= dy < c->h` (229) | VALIDATED |
| 18 | `nb_cursor.c:406-546` geometry (`geom64`, `surface_geom`) | scaled w,h for buffers | products < 2^63 | G (cursor size), C (window size, scale) | factors bounded (w <= 256, num < 2^32, scale <= 16x), result clamped to `max` | VALIDATED |
| 19 | `nb_session_wl.c:2559-2630` `gc_render` | `nb_cursor_scale` into shm slot of 1024x1024 px | b.w*b.h <= slot | G, C | restated at the write (2599) | VALIDATED |
| 20 | `nb_session_wl.c:2896-2992` `ov_refresh` (image) | same, overlay slot | same | G, C | restated (2945) | VALIDATED |
| 21 | `nb_session_wl.c:2809-2890` `ov_draw_xor` | `nb_px_scale_box` patch -> slot | pl.bw*pl.bh <= slot; src pw*ph <= 256^2 | G, C | restated (2835); `nb_xor_place` clamps pw,ph to 256 and gw,gh to 1024/k | VALIDATED |
| 22 | `nb_session_x11.c:1413-1487` `x11_gc_build` | scale into `static px[512*512]` | g.w*g.h <= array | G, C | restated (1450) | VALIDATED |
| 23 | `nb_frame.c:142-243` `nb_frame_open` | `mmap(PROT_READ, need)` of the VMM's frame | fd cannot shrink; extent fits | V, G | re-proves: fourcc, geometry vs size, `need <= INT32_MAX`; shm: tmpfs + `F_SEAL_SHRINK`; dma-buf: LINEAR/implicit + exporter allowlist (udmabuf, system heaps) + `DMA_BUF_MAGIC`; maps exactly `need` | VALIDATED (residual: E-03) |
| 24 | `nb_frame.c:295-335` `nb_frame_read_rect` | reads rw x rh pixels from the map into `out` | rect inside frame and map | G (cursor position/size) | rw,rh <= 256; rx+rw <= w, ry+rh <= h in u64; each row end <= `f->len` | VALIDATED |
| 25 | `nb_xor.c:184-273` `nb_xor_eval` | memset/compose into `ctx->patch`, `ctx->rect` (256^2 each) | bw,bh <= 256; top-left within 2^20 | G | bw,bh come only from `nb_cursor_set_xor` (bounded by w,h); rejects abs(tl) > 2^20; clip in i64; `nb_frame_check` before every read | VALIDATED |
| 26 | `nb_cursor.c:584-613` `nb_xor_patch_rect`, `557-582` `_xor_apply` | indexed reads of mask/frame, writes `out` | mw,mh <= 256, strides >= widths | G | checked at entry (592-595); every frame index passes `xor_on_frame` | VALIDATED |
| 27 | `nvkvm_broker.c:2727-2891` CMD_CLIPBOARD | memcpy `n` bytes into `clip_in[7168+1]` | n <= 27, len+n <= 7168 | V (kayfabe never sends it: no use of `CMD_CLIPBOARD` outside `wire.rs`) | n > 27 = violation; `n > MAX - len` drops the transfer; chunk count kept by the receiver; NUL written at `len <= 7168` | VALIDATED |
| 28 | `nvkvm_broker.c:1317-1358` `nb_utf8_ok` | reads `p[i+k]` | i+need < len | V | truncated sequence refused before the read (1336) | VALIDATED |
| 29 | `nvkvm_broker.c:258-344` tx ring coalesce, `354-381` `nb_emit`, `1241-1294` clipboard send | ring writes `tx[tx_head]`, static `out[512]` | used < 512 | C, L (input), V (answers to QUERY_FORMAT) | full ring = detach (368); clipboard reserves chunks + 6 slots before writing (1265) | VALIDATED |
| 30 | `nvkvm_broker.c:1617-1644`, `nb_frame.c:76-98` fdinfo read / `exp_name` parse | read into `buf[4097]`; copy into `out[n]` | bounded | V (chooses the fd; a dma-buf name is client-settable but is printed after `exp_name`) | read capped at sizeof-1; `j + 1 < n`; first match wins | VALIDATED |
| 31 | `nvkvm_broker.c:1700-1801` fd reaper | fd numbers through a pipe to a thread that closes them | each number written once | V | the main thread never closes an fd it queued; EOF ends the thread; pipe full = fd held | VALIDATED |
| 32 | `nb_session_wl.c:662-690`, `2468-2484`, `2661-2675` wl_buffer listeners | listener data points into `w->bufs[]` / slots | the proxy is destroyed before its slot is reused | C | `wl_buffer_destroy` precedes every slot reuse (686, 2591, 2848, 2948); slots are embedded in `w`, freed only at session close | VALIDATED |
| 33 | `nb_session_wl.c:839-904` deferred frame dup | `F_DUPFD_CLOEXEC`, at most one kept | one dup live | V | `wl_defer_forget` closes before each new dup; adopt closes on every path (889, 897, 900) | VALIDATED |
| 34 | `nb_session_wl.c:2072-2115` `wl_fetch_pump` | read host selection into `fetch_buf[7168+1]` | fetch_len <= sizeof | C (another host app's selection) | read length is the remainder; > 7168 ends the transfer | VALIDATED |
| 35 | `nb_session_x11.c:2527-2572` `x11_clip_receive` | property value handed to the sink | len <= 7168 | C (selection owner: any X client) | request capped at 1792 words; `len` re-checked; INCR refused | VALIDATED |
| 36 | `nb_session_wl.c:4970-4984` `fb_main_device` | memcpy `sizeof(dev_t)` from a `wl_array` | size == sizeof(dev_t) | C | checked (4980) | VALIDATED |
| 37 | `nb_placeholder.c:84-128`, title bar/dialog painters | glyph blits | x < w, y < h | L (`--title`), C (window size) | `put()` clips every pixel | VALIDATED |
| 38 | `nvkvm_broker.c` log sites on the command path: 2972 (WINDOW), 2915 (CAPS), 2590, 2530/2536, 2824, 2833, 2859, 2873, 2883 | `vfprintf` + `fflush` per command, on the thread that holds the keyboard grab | "the reject path gets its own budget" (1552-1565) | V; G only at guest mode-set rate (WINDOW) | nothing: these lines are outside `nb_reject_log` | MISSING (E-01) |
| 39 | `crates/kf-broker/src/conn.rs:2017-2079` `read_batch` (kayfabe) | reads <= 64 packets of 24 bytes, reassembles into `c.rx[24]` | safe Rust slices | B | budget `cap*24 <= buf`; no control buffer on `recvmsg` (`kf-linux-raw/src/unixsock_unsafe.rs:296-319`), `MSG_CTRUNC` = connection lost | VALIDATED (panic at worst) |
| 40 | `conn.rs:2081-2231` `handle` (kayfabe) | every broker field used | bounded | B | key/button code 0..=0x2ff; ABS clamped to range <= 2^20; SURFACE clamped 64..8192; REL saturating; unknown types skipped; tick operands checked (`wire.rs:164-185`); RELEASE matched against ids kayfabe sent; verdict table capped at 16 | VALIDATED |
| 41 | `conn.rs:2175, 2188, 2204, 2223, 2518, 2600, 2659` (kayfabe) | `eprintln!` per broker packet on QEMU's main loop | none stated | B | nothing (only RELEASE/format paths use `loud()`) | MISSING (E-02) |
| 42 | `wire.rs:512-540` `CursorCmd::set`, `conn.rs:1148-1160` `choose` (kayfabe -> broker) | what a guest-sized cursor/frame becomes on the wire | never send what the broker would refuse | G | w,h 1..=256, stride, hot, extent vs memfd in u64; frames over 8192 not sent | VALIDATED |

Counts: 42 rows; VALIDATED 38; VALIDATED-BY-CALLER-ONLY 2 (rows 5, 12); MISSING 2 (rows 38, 41: emit bounds, not memory); PANIC-ONLY 0.

### 2. Findings

### E-01 · MEDIUM · MEASURED · broker log lines outside the reject budget: a connected client drives ~15,000 flushed lines a second (2026-11-13)
- Where: `broker/nvkvm_broker.c:2972` (WINDOW after the first: "the guest changed resolution ... keeping the window"), `:2915` (CAPS: "the VM reports: clipboard agent ..."). Same shape, CODE READ only: `:2590` (ATTACH on a session without CAP_DMABUF, `nb_err`), `:2530`/`:2536` (`nb_note_tier`, one line per shm/dma-buf alternation), `:2824`, `:2833`, `:2859`, `:2873`, `:2883` (clipboard: one line per transfer or per over-rate chunk, up to 4000/s), and the attach/detach lines per reconnect under `--persist`.
- Scenario: a VMM (or anything running as an allowed uid) sends well-formed `CMD_WINDOW 800x600` records in a loop. Each is accepted, logged with `vfprintf` + `fflush` (`nb_vlog`, `:89-95`) on the single thread that also services the compositor and holds the keyboard grab, up to the command ceiling `NB_RX_MAX_PER_SEC` = 20,000/s. That is the exact failure the code's own note S-4 (`:1552-1565`) describes for the reject path and bounds at 10 lines/s; these accept-path lines were not put under it. Consequences: megabytes per second into the session journal (disk), and on a log pipe that stops draining the main loop blocks with the grab held.
- Measurement (a log-flood PoC, not retained; test backend, ASan build): 15000 WINDOW commands in 0.83 s -> 15002 log lines, 1.44 MB; 15000 CAPS in 0.81 s -> 15003 lines, 1.79 MB; control, 15000 QUERY_FORMAT (budgeted) -> 15 lines.
- Guest reach through an honest kayfabe: only WINDOW, once per guest mode change (`crates/kf-broker/src/conn.rs:1366-1385`, which also logs one kayfabe line per change, `:1380`); kayfabe never sends CLIPBOARD and sends CAPS once. So guest-steered rate = the guest's mode-set rate, not 20,000/s; the full rate needs a compromised VMM, which is the broker's stated threat model.
- Planned limit: none found for these lines (`docs/design/V3_DISPLAY.md:2375` documents a rate limit only for refused frames on the kayfabe side).
- Minimal fix: route these lines through `nb_reject_log()` (or a second per-second budget with the same "N more not logged" line); for WINDOW, log only when the size differs from the last one logged.

### E-02 · MEDIUM · CODE READ · kayfabe logs once per broker packet with no bound, on QEMU's main loop
- Where: `crates/kf-broker/src/conn.rs:2175` (EV_GRAB), `:2188` (EV_FOCUS), `:2223` (EV_BYE), `:2518` (unasked EV_FORMAT yes), `:2600` (EV_DEVICE naming an unresolved node: logged before the `c.device == d` de-duplication at `:2607`), `:2204` (EV_SURFACE alternating two sizes), `:2659` (EV_CLOSE); all through `log_line` (`:685`), an unconditional `eprintln!`.
- Scenario: a hostile or broken broker (it passed the peer-uid check: uid 0, QEMU's euid or `display-broker-uid`) writes a stream of 24-byte `EV_FOCUS` packets. The relay reads 64 per main-loop turn (`READ_BATCH`, `:100`) and prints one line each, with no per-second limit, for as long as the broker writes: unbounded log volume on the host, and each `eprintln!` is a blocking write on QEMU's main loop, so a stderr pipe that stops draining stalls the main loop (INFERRED: with the BQL held, which stalls vCPU MMIO exits). That is the one path I found by which the broker can hold a kayfabe thread, against "a slow or dead broker never blocks".
- Not corruption; the broker already holds console authority (it may power the guest off, `V3_SECURITY_MODEL.md` R4), so the new part is host disk/log volume and the main-loop stall, not guest availability.
- Minimal fix: a per-second budget inside `log_line` with a "suppressed N" line (the crate already has `loud()` for two paths), and move the EV_DEVICE line below the de-duplication.
- Regression test proposal (GPU-free, deterministic clock): feed 10,000 EV_FOCUS packets through the fake `Link` under `capture_log()` and assert the captured line count is under the budget; a mutant that removes the budget fails it.

### E-03 · LOW · INFERRED (kernel semantics, not measured) · a sparse sealed memfd as an F_SHM frame moves up to ~570 MB per buffer of page allocations onto the reader
- Where: `broker/nvkvm_broker.c:1954` (only `F_SEAL_SHRINK` is required), `nb_session_wl.c:988` (pool handed to the compositor), `nb_session_x11.c:380, 751-758` (the broker reads every row), `nb_frame.c:10-13` (the comment "a hole ... re-faults as zeros").
- Scenario: a hostile VMM creates an 8192x8192 F_SHM frame with the widest accepted stride (2*8192*4+4096 = 69,632 bytes; 8192 rows = 570 MB), sealed against shrinking but never written (all holes). The read faults in the compositor (wl_shm upload) or the broker (X11 PutImage path) allocate the pages, charged to the desktop session rather than to the VMM, and they stay allocated while the VMM keeps its fd; a new memfd per ATTACH repeats it (the X11 shm path remaps on every ATTACH, the Wayland cache is keyed by inode). Under a memory limit the fault can end in SIGBUS/OOM in the reader, which the seal does not prevent.
- Not reachable by a guest through an honest kayfabe: kayfabe's frames are its own memfds (`crates/kf-qemu/src/broker.rs:174`), page-locked and fully written. A compromised VMM could also simply allocate the memory itself; the delta is whose account it lands on and who takes the fault.
- Minimal fix: state it in the broker README as an accepted residual, or bound new F_SHM buffers per second / total mapped bytes per connection.

### E-04 · LOW · CODE READ · `--persist`: a buffer the previous VM attached but never committed can be presented by the next VM's first COMMIT (Wayland)
- Where: `broker/nb_session_wl.c:2117-2184` (`wl_client_detach` forgets proofs, the deferred frame and the read-only mappings, but leaves `w->pending` and `w->bufs[]`), `:3706` (`wl_show_idle` resets `current` only), `:1303-1310` (`wl_commit` presents `bufs[pending]`).
- Scenario: VM A sends ATTACH and disconnects before COMMIT. VM B connects to the persisting broker and sends COMMIT with no ATTACH: `pending` still indexes A's buffer, so A's never-shown frame is put on screen under B's session, and A's buffer id is later reported to B as EV_RELEASE. No memory unsafety (the wl_buffer is alive); it is the same class as the cursor and clipboard carry-over that `nb_client_state_reset` exists to prevent. I did not check whether the X11 backend resets `pending` on detach.
- Minimal fix: `w->pending = -1` in `wl_client_detach`.

### E-05 · INFO · CODE READ · two internal preconditions are held only by the caller
- `broker/nb_cursor.c:152` `nb_cursor_load` writes `dst->px` for w*h pixels with no check of its own; safe because its one caller (`nvkvm_broker.c:2347`) runs `nb_cursor_check` and restates the span bound. `broker/nb_session_wl.c:981-988` casts `need` to `int32_t` relying on `nb_validate_desc` (the X11 twin re-checks at `nb_session_x11.c:375`). Both are documented contracts inside one process, not a trust boundary; a two-line re-check at each site (w,h <= 256; need <= size and <= INT32_MAX) would make them independent of the caller, as the file's own convention ("restated where the write is") asks elsewhere.

### Considered and rejected (false positives)
- **fd confusion through a partial command** (`nvkvm_broker.c:3155-3187`): a second fd while one is banked is a violation and both are dropped; the banked fd is dropped on detach. No leak, no double close.
- **`nb_tx_coalesce` reading `tx[last_tick]` with `last_tick == ~0u`** (`:292-294`): only evaluated when the current packet is a TICK, in which case `last_tick` was set by the scan above.
- **Emit from inside a handler detaching the sink** (`nb_emit` -> `nb_sink_detach`): the sink is static for the process and every caller re-checks `client_fd`; the grab case was already fixed (S-5, `:616-636`).
- **dma-buf `name:` injection into the `exp_name` parse** (`nb_frame.c:76-98`): the kernel prints `exp_name` before the client-settable name and the parser takes the first match.
- **Wayland import-cache keyed by inode only** (`nb_session_wl.c:932-945`): a collision shows an earlier, validated buffer of the same geometry; no extent is trusted from the new record.
- **`(const uint32_t *)src` at `nb_session_x11.c:757`** with an odd `offset`: a misaligned pointer type that is only ever passed on as bytes to xcb; no dereference as `uint32_t`.
- **Signed/unsigned in `x11_put_image`** (`:1566-1581`): w <= 8192, `w*n*4` bounded by the maximum request length; no overflow.
- **Hostile broker sending fds to kayfabe**: `recv_bounded` has no control buffer, so the kernel discards them and sets `MSG_CTRUNC`, which the relay treats as a protocol violation (`conn.rs:2069`).

### 3. Owner rules touched in this scope

| rule | status | where |
|---|---|---|
| The broker sees only finished frames and the cursor image, never guest memory | HOLDS for descriptor provenance (frames are kayfabe-created sealed memfds / udmabufs over them, the cursor is a memfd kayfabe fills and closes); UNVERIFIED what the GPU copy writes into them (kf-cuda, another part) | `crates/kf-qemu/src/broker.rs:8-11, 174-193`; `crates/kf-broker/src/cursor.rs:28` |
| kayfabe validates everything it receives from the broker | HOLDS for memory and values (rows 39, 40); VIOLATED for emit bounds (E-02) | `crates/kf-broker/src/conn.rs:2017-2231`; `kf-linux-raw/src/unixsock_unsafe.rs:296-319` |
| The broker owns all cursors including true XOR; kayfabe composes none | HOLDS with a CAP_CURSOR_STREAM broker (`composes()` returns false whenever `stream`); the composing path still exists for a non-stream viewer or no broker (UNVERIFIED whether that is meant to remain) | `crates/kf-broker/src/cursor.rs:597-599`, `:71-76`; broker side `nb_xor.c`, `nb_cursor.c:320-357` |
| A slow or dead broker never blocks a vCPU, the drainer or input threads | HOLDS for the socket (non-blocking socket, `MSG_DONTWAIT`, `Sent::Full` owed and retried, relay on the main loop only, worker touches only a CAS and a non-blocking eventfd); one exception: blocking `eprintln!` per broker packet (E-02) | `kf-linux-raw/src/unixsock_unsafe.rs:110, 313`; `crates/kf-qemu/src/broker.rs:13-30`; `conn.rs:685` |
| A guest-reachable unbounded read or emit is a bug | Guest-reachable emit is bounded by the guest's mode-set rate (one broker line and one kayfabe line per change); VMM-reachable emit in the broker is unbounded up to 20,000/s (E-01) | `broker/nvkvm_broker.c:2972`; `conn.rs:1380` |

### 4. What I ran (MEASURED), logs not retained
- `build_asan.log`: broker, test client, relay and unit tests built with `-O1 -fsanitize=address,undefined` from a copy of `broker/` (Wayland + X11 + XRender + XI2 backends compiled in).
- `test-utf8.log`, `test-cursor.log`, `test-geom.log`, `test-xor.log`, `test-tick.log`, `selftest.log`, `test_adopted_socket.log`, `test_clipboard.log`, `test_lifecycle.log`, `test_cursor.log`, `test_xor.log`, `test_tick.log`: all exit 0, no `AddressSanitizer` or `runtime error` line in any log (leak detection off).
- `analyzer_*.log`: `gcc -O1 -fanalyzer -Wall -Wextra -Wformat=2 -Wconversion` per file: 0 warnings in 12 files (`-Wsign-conversion` alone gives 32 in `nb_session_wl.c`, not triaged). `clang_analyze.log`: 0 warnings.
- Log-flood PoC and its output (not retained): E-01.
- Not run: `mutate_*.py` (they rebuild), `check-probe`/`check-tick-probe` (need a compositor), any real Wayland/X11 session, any fuzzing beyond the suite's own random records.

### 5. Not reviewed, or only skimmed
- `nb_session_wl.c` lines 1552-1800 (title bar), 3123-3730 (letterbox, dialog, idle placeholder), 3740-4960 (keyboard/pointer/seat listeners, configure, cursors, borders, tick pump, presentation feedback), 5007-5830 (registry, poll, grab, open/close): grep for memory and fd operations plus the two analyzers only. Their inputs are the compositor and local input, not the VMM.
- `nb_session_x11.c` outside attach/commit/cursor/clipboard-receive: same treatment. X11 `client_detach` (E-04's twin) not checked.
- `nb_tick.c` (tick/statistics arithmetic), `nb_session_test.c` (test backend; it accepts unsealed memfds by design), `testclient.c`, `dmabuf_relay.c`, `dmabuf_source.c` (test tools, unprivileged): not read beyond grep.
- `nvkvm_broker.c` 641-1185 (key/grab/chord policy), 3448-3890 (privilege drop, argv parsing), `main` before the accept loop: skimmed. Privilege-drop correctness (`--drop-user`) was not audited.
- `crates/kf-broker`: `slots.rs` (FrameRing atomics: a §2.3 interleaving question, not memory safety here), `ticks.rs`, `gpucopy.rs`, `console.rs`, `input.rs` beyond the entry points; `crates/kf-qemu/src/broker.rs` beyond the frame-backing and seat description. The `kf-linux-raw` socket wrappers (`unixsock_unsafe.rs`) belong to another part; I read only the flags they pass.
- Lifetime of Wayland listeners across `wl_close_session` (free of `w`) was reasoned about (no dispatch after close), not tested.


---

## Appendix F (file F_firmware_tools.md)

Part status as of 2026-11-13: Security review 2, PART F (firmware/kf-gop, tools/, live-tree patches). Tree: origin/master 921c39444. Read-only review; nothing built for the firmware, nothing loaded, no hardware. Labels: CODE READ unless marked MEASURED / INFERRED.

Owner addendum applied (2026-11-13): any guest breakout = CRITICAL; unbounded DoS counts; diagnostic-only tools keep a lower severity and are named as such.

### 0. Result in one paragraph

No guest-to-host breakout and no host memory-corruption path was found in this scope. The shipped piece (the kf-gop UEFI driver in the option ROM, and its host-side packer) has one real lifetime defect that stays inside the guest's own firmware (F-01). The two host-kernel patches (H5 `nvidia.ko`, EFS `nvidia-uvm.ko`) were read line by line against stock 595.91.07 `nv.c`; their user-pointer, size and lifetime handling is sound as written; neither is issued by the product today (kayfabe only probes `NV_ESC_KF_QUERY`; there is no EFS client in `crates/`). The remaining items are diagnostic tools: one kernel-race in the Windows capture driver (F-04, admin-only, own machine) and bounded-cost notes.

### 1. What is product path and what is diagnostic

| piece | shipped / CI path? | privilege where it runs |
|---|---|---|
| `firmware/kf-gop` driver (`driver_unsafe.rs`, `efi_unsafe.rs`, `blt.rs`, `probe.rs`) | YES: built by `crates/kf-gop-image/build.rs`, served as kf3's option ROM with `gop=on` (default off) | guest firmware (boot services) |
| `firmware/kf-gop/src/bin/kf-gop-test`, `port_unsafe.rs` | NO (local stand-in test app; `debugcon` feature only) | guest firmware |
| `crates/kf-oprom`, `crates/kf-gop-image`, `crates/kf-qemu/src/gop.rs` | YES, host; safe Rust (`unsafe_code = forbid`), inputs are host config only | VMM |
| `tools/host_patches/h5_userd_dma` (H5) | OPTIONAL patched-host tier; build-only verified; product issues only `NV_ESC_KF_QUERY` (`crates/kf-host/src/tier.rs:13,24`) | host kernel |
| `tools/uvm_efs` (EFS / H3) | experimental, DESIGN-ONLY tier; no client in `crates/` (`docs/design/V3_HOST_PATCH_LIST.md:87`); `uvm_efs_enable=0` default | host kernel |
| `tools/vfio-gsp-observer` | diagnostic; compiled into kf3 only under `KF3_GSP_OBSERVER`, active only with `x-gsp-observer=` | VMM (parses hostile guest RAM) |
| `tools/windows-capture` | diagnostic, loaded on demand on an owned native Windows box; device ACL SYSTEM+Administrators | Windows kernel |
| `scripts/rpctrace/nv_rpctrace.c` + patches, `docs/kernel_patches/*.patch`, `scripts/bench/guest_ogkm_ringtrace.patch` | diagnostic instrumented driver builds | guest/host kernel (bench only) |
| `tools/cudart_probe/*.c`, `tools/probes/*.c`, `tools/windows-capture/linux-probe`, `tools/memory-list-probe`, `tools/windows-debug-capture/*.c`, `tools/windows-runlist-probe/*.c` | diagnostic userspace / compile-time layout probes | user / root on a bench box |
| `tools/api_boundaries`, `tools/drivermatrix`, `tools/legal`, `tools/derive_*`, `tools/windows-guest`, `tools/display-*-probe`, `tools/windows-ctrl-export` | scripts / docs / data only; no memory-unsafe code | n/a |

### 2. Rows

Verdict vocabulary: VALIDATED / VALIDATED-BY-CALLER-ONLY (VBCO) / MISSING / PANIC-ONLY. "VBCO[ABI]" = the precondition is the UEFI ABI contract of the firmware core that calls the entry point (not a safe-Rust caller in our tree); listed for completeness, not counted as a defect.

### 2.1 firmware/kf-gop (Rust, guest firmware)

| # | file:line | operation | precondition; documented? | who supplies the value | what validates it | verdict |
|---|---|---|---|---|---|---|
| R1 | driver_unsafe.rs:102 | `Bs::from_system_table(st)` deref | `st` is the firmware system table; yes | firmware (image entry) | nothing (cannot) | VBCO[ABI] |
| R2 | :126 | `d.write(Driver)` into pool | fresh pool of `size_of::<Driver>()`, align 8; yes | own `allocate_pool` | size from `size_of`, null/err checked in `Bs::allocate_pool` | VALIDATED |
| R3 | :130,137 | install binding/name interfaces | interface outlives install; yes | own allocation, never freed | never freed (no unload) | VALIDATED |
| R4 | :142-146 | `driver(this)`: `&*this.cast::<Driver>()` in a SAFE fn | `this` is our installed binding; yes | firmware | none; private safe fn with a raw-pointer parameter | VBCO (F-03) |
| R5 | :155,161 | read `RemainingDevicePath` header | well-formed node; yes | firmware | null check only | VBCO[ABI] |
| R6 | :168 | PCI config read into local | `pci` opened by us; yes | firmware PCI I/O | width 2, count 1, local `u32` | VALIDATED |
| R7 | :181-184 | `from_raw_parts(RomImage, RomSize)` | bus driver's ROM copy lives as long as PCI I/O; yes | firmware PCI bus driver | null/zero check; `probe::decide` copies out; all parsing in safe `kf_oprom` (checked indexing, checksum, `Geometry::check`) | VBCO[ABI] |
| R8 | :191 | `Attributes()` | local out; yes | — | local | VALIDATED |
| R9 | :199-214 | read 46 bytes of `GetBarAttributes` result after checking only byte 0 == 0x8A | descriptor is a full QWORD descriptor; yes | firmware | first tag only; `probe::bar_range` re-checks length field bytes | VBCO[ABI] |
| R10 | :330-344 | `device_path_bytes` walks nodes until END / len<4 | well-formed path; yes (`unsafe fn`) | firmware | `len < 4` stop; no upper bound | VBCO[ABI] |
| R11 | :372-384 | copy parent path + `_ADR` + end into pool | dest holds `parent_bytes+12`; yes | own alloc | allocated with the same expression | VALIDATED |
| R12 | :390-401 | error-path `free_pool`/`free_pages` | ours, unreferenced; yes | own | straight-line | VALIDATED |
| R13 | :409-461 | `ctx.write(Ctx{..})`, self-pointers | fresh pool, never moves; yes | own | `size_of::<Ctx>()` | VALIDATED |
| R14 | :477-504 | install 4 protocols on child | interfaces live until uninstalled; yes | own ctx | freed only in `teardown` | VALIDATED (but see R15) |
| R15 | :508-537 | `teardown`: uninstall ×4 (status ignored), then `free_pages`/`free_pool(ctx)` | "nothing references them once uninstalled"; yes | firmware decides whether uninstall succeeds | NOTHING: a failed `UninstallProtocolInterface` is not noticed | **MISSING (F-01)** |
| R16 | :558 | `*child.add(i)` for `i < children` | `Stop()` contract; yes | firmware | none | VBCO[ABI] |
| R17 | :582-589 | read ≤3 bytes of `Language`, write `*name` | NUL-terminated; yes | caller | null checks; short-circuit stops at NUL | VALIDATED |
| R18 | :604-613 | `ctx_of`: read `signature`, hand out `&'a mut Ctx` | `This` is our GOP; yes | any GOP caller | null + signature check (CR-macro idiom); lifetime unbounded, taken before `RaiseTPL` | VALIDATED (aliasing note F-02) |
| R19 | :633-643 | `base.add(i).write_volatile` to BAR | `i < words`, base aligned, range inside BAR; yes | Blt caller via plan | `i >= words` refused; `words = pitch*height/4`; `probe::place` (fits BAR, `checked_add`, 4-aligned); `Geometry::check` (`fb_size >= pitch*height`) | VALIDATED |
| R20 | :648-661 | `from_raw_parts_mut(shadow, n)` | `shadow_pages*4096 >= n`; yes | own alloc | same `surface.bytes()` used for both | VALIDATED |
| R21 | :689-699 | QueryMode out-params | non-null; yes | caller | null checks | VALIDATED |
| R22 | :713-769 | `from_raw_parts[_mut](BltBuffer, n)` | caller buffer covers the rectangle (UEFI); yes | Blt caller (sx,sy,dx,dy,w,h,delta) | `blt::plan`: `w,h != 0`, `checked_add/mul` everywhere (`inside`, `buffer_extent`), rectangle inside mode; `execute` re-checks every slice with `.get()`; null refused when `n > 0` | VALIDATED (extent of the caller's buffer itself is ABI) |
| R23 | efi_unsafe.rs:534-690 | safe `Bs` methods calling boot services | `Bs.0` is the live table; yes (type invariant, private field, only `unsafe` ctor) | — | invariant; out-params are locals; handles checked by firmware | VALIDATED |
| R24 | efi_unsafe.rs:155,527,562,581,632,669 | `unsafe fn` secure_boot / from_system_table / free_pool / free_pages / install / locate_device_path | each has a `# Safety` section | callers in the two `*_unsafe.rs` files | every call site read; all hold except via R15 | VALIDATED |
| R25 | port_unsafe.rs:19-28 | `out dx, al` | CPL 0, no memory; yes | debug text | constant port; `nomem,nostack` | VALIDATED (test builds only) |
| R26 | bin/kf-gop-test/check_unsafe.rs (23 sites) | GOP calls, `fb_pixel` volatile read, `Pixels` pool slices | "callers stay inside the mode"; yes | descriptor + mode block | none beyond the descriptor's own validation; `w*h`, `len*4` unchecked (F-10) | VBCO (test app, not shipped) |
| R27 | crates/kf-qemu/src/gop.rs:60-95; kf-oprom desc.rs:319-336,367-421; kf3.c:1486-1516 | host: plan the framebuffer, pack ROM, copy ROM into ROM BAR | fb fits BAR1; ROM fits romsize | host config only (monitor mode, bar1-size, romsize, `gop-efi=` file) | `fb.fits(bar1_bytes)`; `Descriptor::validate`; `read_bounded` (GOP_EFI_MAX); `len > romsize` refused, else `pow2ceil(len)`; ROM region is `memory_region_init_rom` (guest writes dropped) | VALIDATED (no guest input reaches it) |

### 2.2 Host kernel patches

| # | file:line | operation | who supplies the value | what validates it | verdict |
|---|---|---|---|---|---|
| C1 | h5 patch:211-224, 392-403 (`nv_kf_query`) | fill 24-byte reply | any opener of an nvidia node | `arg_size == sizeof` re-checked in the case (needed: the XFER path does not re-run `nv_validate_ioctls` on the inner cmd, stock nv.c:2508-2534); `memset` before fill, no padding leak | VALIDATED |
| C2 | h5 patch:257-304, 370-381 | `inner_size`/`inner_ptr` → kmalloc, `copy_from_user`, `rm_ioctl`, `copy_to_user` | caller | abi/flags/`0 < inner_size <= 512`/ptr != 0; `nv_validate_ioctls(_IOC(.., 0x27, inner_size))` pins the size to RM's table; single fetch (RM runs on the kernel copy); buffer fully initialised before copy-out; freed on every path | VALIDATED |
| C3 | h5 patch:307-367 | lower `dma_set_mask` for one call, restore | caller picks `dma_bits` | `nv_kf_dma_window_plan`: 40..63, only lowers, only behind a translating IOMMU domain, admin switch; mutex held from lowering to restore; restore failure logged | VALIDATED (notes F-07) |
| C4 | h5 patch:405-418 | dispatch | caller | `arg_size == sizeof`, `NV_ACTUAL_DEVICE_ONLY` | VALIDATED |
| C5 | uvm_efs.c:213-264, patch uvm.c hunk @@ -950 | alloc/attach/free of EFS state | `UVM_INITIALIZE` caller; `uvm_efs_max_records` admin | `1..65535`, kvmalloc_zero, attached before the file is published, freed on create failure; requires `uvm_efs_enable` and DISABLE_HMM/MPS flag | VALIDATED |
| C6 | uvm_efs.c:266-361 | park a fault packet | GPU fault buffer (guest-steered content: address, access type) | bounded table, `num_parked >= max_records` → stock cancel; slot search bounded; pointers in the copy cleared; `closing` checked under lock | VALIDATED |
| C7 | uvm_efs.c:738-784 | `UVM_EFS_WAIT` | owner tgid | owner check (`struct pid` ref held, no pid-reuse hole); `1 <= max <= UVM_EFS_MAX_WAIT_RECORDS`; each record `memset` then filled; only `n` filled records copied out | VALIDATED |
| C8 | uvm_efs.c:786-901 | `UVM_EFS_RESOLVE` ids → records | owner tgid | `count` bounded; `slot = id & 0xFFFF` checked `< max_records`; `in_use && rec->id == id` (seq<<16|slot, so a stale or duplicate id is "stale"); action enum checked; hardware action uses only the saved packet and the VA space's own PDB | VALIDATED |
| C9 | uvm_efs.c:365-460 | GPU retain, ISR lock → VA-space read lock, cancel/replay | record (kernel state) | GPU must be in `registered_gpu_va_spaces`; generation compare under the VA-space lock; retain/release paired | VALIDATED by reading; hardware behaviour UNVERIFIED (not run here) |
| C10 | uvm_efs.c:487-633; patch hooks in uvm_va_space.c / uvm_va_space_mm.c | timeout work, shutdown, free | kernel | `closing` set under lock before `cancel_delayed_work_sync`; work requeues only while `!closing`; shutdown called on mm-shutdown and destroy before channels stop; free after bottom halves flushed; NULL `saved` handled | VALIDATED |

### 2.3 tools/vfio-gsp-observer (QEMU side, parses guest RAM)

| # | file:line | operation | who supplies the value | what validates it | verdict |
|---|---|---|---|---|---|
| C11 | qemu/gsp-observer.c:67-94 (`guest_ram`) | memcpy from guest RAM by guest-named PA | guest (mailbox, LibOS args, page table) | `pa > UINT64_MAX - bytes`; identity DMA only; `address_space_translate` per chunk; `plain_ram` (no ROM, RAM-device, protected, guest_memfd); `offset/length` vs `memory_region_size`; re-validated on EVERY read (no cached host pointer) | VALIDATED |
| C12 | core/observer.c:20-49 (`bootstrap`) | page table → `ptes[512]`, queue offsets/sizes | guest | `pages` 5..512 (`table_copy[4096]` = 512×8); each PTE non-zero, aligned, readable, unique; `cmd == 4096`, `stat` aligned, sizes ≤ 1 MiB (`a[]`,`b[]` = 1 MiB) | VALIDATED |
| C13 | core/queue.c:9-83 | queue header, element walk, records | guest (private double-copied snapshot) | `size == n`, `count == size/4096-1 ≤ 255`, `write < count`; slot `< count`; `pages ≤ 16` and `== ceil((48+len)/4096)`, `len ≤ 65488`; wrap by `% count`; `slots[256]/seqs[256]` cannot fill (≤255 elements); fn-103 tail `params ≤ pages*4096-112` | VALIDATED |
| C14 | qemu/gsp-observer.c:96-250 | SPSC fifo, writer thread | observer's own records | `size > FIFO_BYTES - (at-read)` refused; RECORD_CAP 100000, SOURCE_CAP 64 MiB; writer `assert(payload_bytes <= 65536)` and available-bytes assert; hex buffer 2×65536+1 | PANIC-ONLY (asserts) / VALIDATED |
| C15 | core/observer.c:79-111, kf3.c:240-250 | work per trigger | guest (any write to 0x110c00-0x110c3f, read of 0x110004/8) | bounded per trigger (≤ 4 × 1 MiB copies + parse); bounded in time (`x-gsp-observer-seconds`, max 7200) and output | bounded; see F-06 |

### 2.4 tools/windows-capture (Windows kernel driver + engine)

| # | file:line | operation | who supplies the value | what validates it | verdict (2026-11-13) |
|---|---|---|---|---|---|
| C16 | driver/kfwc_drv.c:529-603 | IOCTL buffers (METHOD_BUFFERED) | admin caller | exact `in_len`, `out_len >= sizeof`; INFO/CAPS zeroed, STATS fully assigned (all u64, no padding); EVENTS writes `out_len/64` whole events; no ioctl takes an address | VALIDATED |
| C17 | :514-527 | READ into SystemBuffer | admin caller | `Length >= 70 KiB` (> max record 32+64 KiB), cap ≤ 4 MiB, single reader (`reader_busy`); `kfwc_ring_get` copies whole records only, `total > out_cap - done` stops | VALIDATED |
| C18 | :366-479, 504-506, 593-597 | `capture_start` / `capture_stop` state (`running`, `thread`, `ranges`, ring re-init) | two admin threads | NOTHING serialises START/STOP/CLEANUP against each other; `ranges` swap and `kfwc_ring_init` run outside `s->lock` | **MISSING (F-04)** |
| C19 | :253-301 | `MmCopyMemory` physical read, BAR0 volatile load | engine (addresses read from RAM / registers) | every read inside one `MmGetPhysicalMemoryRanges` range (overflow-checked); register offset whitelisted, aligned, inside the read-only mapping, BAR stable 20 ms | VALIDATED |
| C20 | lib/kfwc_discover.c, kfwc_engine.c, kfwc_elem.c, kfwc_ring.c | parse page table / queue headers / elements from RAM that any process on that machine can shape | hostile RAM content | `kfwc_region_read` bounds every read by `pages*4096` (so slot arithmetic cannot escape); `pages ≤ 512`; `ptes` unique, aligned, in RAM; queue size ≤ 1 MiB, `count`/`write_ptr` checked; `elems ≤ 16` → `run[16 pages]`; `head[128]` vs max offset 48+28; chain arrays 256 with `n < 256`; record encode checks `cap`; ring modulo `cap` | VALIDATED; `gcc -fanalyzer` clean (MEASURED, logs in `tools_analyzer/`) |

### 2.5 Other diagnostic C

| # | file | note | verdict |
|---|---|---|---|
| C21 | scripts/rpctrace/nv_rpctrace.c:118-196, 240-290 | append under irqsave lock, capacity test covers header+padded payload, `cap_len ≤ 64 KiB`; read exposes only header + `used` prefix (padding zeroed, header memset); proc entry removed before `vfree` (rpctrace.patch:69-70) | VALIDATED |
| C22 | tools/windows-capture/linux-probe/mailbox_probe.c:35-39 | `p_ram_contains` accepts any PA < 2^48 and relies on `/dev/mem` STRICT_DEVMEM; root-run one-shot probe, read-only | VBCO (diagnostic) |
| C23 | tools/cudart_probe/*.c, tools/probes/kvm_ioeventfd_readonly.c, tools/windows-capture/user/kfwc_collect.c, tools/memory-list-probe/*.c, tools/windows-debug-capture/*.c, tools/windows-runlist-probe/*.c, h5 tests | skimmed only; `gcc -fanalyzer` reports nothing but `_GNU_SOURCE redefined` (my flag); kfwc_collect.c and nvdiff_shim.c did not compile here (Windows headers / generated `uvm_sizes.h`) | not classified |
| C24 | docs/kernel_patches/0001,0002; scripts/bench/guest_ogkm_ringtrace.patch; scripts/rpctrace/rpctrace*.patch hooks | add `NV_PRINTF`/read-backs/recorder calls only; `%p` prints of kernel pointers in a bench build | VALIDATED (diagnostic) |

### 2.6 Counts

VALIDATED 36 · VALIDATED-BY-CALLER-ONLY 9 (7 are UEFI-ABI contracts R1,R5,R7,R9,R10,R16 + test app R26; R4 is F-03; C22 diagnostic) · MISSING 2 (R15 = F-01, C18 = F-04) · PANIC-ONLY 1 (C14) · bounded-cost note 1 (C15) · skimmed/unclassified 1 group (C23).

### 3. Findings

**F-01 — LOW (PRODUCT path, but confined to the guest's own firmware; not a host or cross-VM issue). `teardown` frees the context although uninstalling its protocols may have failed.**
`firmware/kf-gop/src/driver_unsafe.rs:508-537` (and `stop` at :539-570 always returns `SUCCESS`). The four `bs.uninstall(..)` results are discarded, then the shadow pages, the device path and the `Ctx` (which CONTAINS the installed `Gop`, `GopMode`, `ModeInfo`, both `Edid` blocks) are freed.
Scenario (CODE READ for our side, INFERRED for firmware behaviour): `DisconnectController` on the kf3 handle, or a failed `Start` after `install_child` got as far as GOP, while a consumer holds the GOP `BY_DRIVER` and refuses to stop (UEFI allows `UninstallProtocolInterface` to return `EFI_ACCESS_DENIED`). The GOP stays in the handle database pointing into freed pool; the consumer's next `Blt`/`QueryMode` calls a function pointer read from freed memory, or `ctx_of` reads a recycled `signature`. Result: firmware crash or wild call inside the guest before `ExitBootServices`. The guest can only hurt itself; severity LOW.
Minimal fix: uninstall in reverse order checking each status; on the first failure re-install what was already removed, leave the context alive and return `EFI_DEVICE_ERROR` from `Stop`; free only after all four succeeded. Test: host-side model of `teardown` with a fake `Bs` whose `uninstall` fails once (must not free).

**F-02 — INFO (product, guest firmware). `ctx_of` hands out `&'a mut Ctx` with an unbounded lifetime before `RaiseTPL`.**
`driver_unsafe.rs:604-613`, used at :683, :703, :725 before `raise_tpl` (:665, :760). A GOP call from a timer notification that interrupts another GOP call between `ctx_of` and `raise_tpl` creates two live `&mut Ctx` (aliasing UB by Rust's rules). No field is written in that window today (only `planes()` after the TPL raise, and `teardown`), so no concrete corruption scenario: INFO. Fix: raise TPL first, or return `*mut Ctx` / `&Ctx` and take `&mut` only inside the raised section.

**F-03 — INFO (product, guest firmware). Private SAFE functions dereference raw-pointer parameters.**
`driver_unsafe.rs:142 driver`, `:149 remaining_ok`, `:159 remaining_is_end`, `:164 config_dword`, `:173 examine`, `:188 attributes`, `:196 bar_range`; test app `check_unsafe.rs:54,154,172`. Under the owner's rule ("assume the safe caller is wrong") these are caller-validated only: any future safe call with a null/dangling pointer is UB without an `unsafe` at the call site. All are private to the `*_unsafe.rs` file and reached only from the extern entry points, so there is no scenario from outside: INFO. Fix: make them `unsafe fn` with a `# Safety` line (the crate already denies `unsafe_op_in_unsafe_fn`).

**F-04 — LOW (DIAGNOSTIC tool, not product/CI; Windows kernel; caller must be Administrator/SYSTEM on an owned box). `kfwc.sys` START/STOP/CLEANUP are not serialised.**
`tools/windows-capture/driver/kfwc_drv.c:366-460` (`capture_start`), `:462-479` (`capture_stop`), `:504-506`, `:593-597`.
Scenarios (CODE READ; the driver's own header says it has never been loaded):
(a) two threads issue `KFWC_IOCTL_START` together: both pass `if (s->running)`, two `Worker` threads are created, `s->thread` is overwritten (one reference leaked). `capture_stop` joins one; `KfwcUnload` then frees `STATE`, `engine`, `ring_mem` while the other worker still runs → kernel use-after-free (bugcheck).
(b) two threads run `capture_stop` (STOP ioctl + handle close CLEANUP): both see `running`, both `ObDereferenceObject(s->thread)` (double dereference) or the second waits on NULL.
(c) `capture_start` frees and replaces `s->ranges` and calls `kfwc_ring_init` (memset of head/tail) outside `s->lock` while a READ is inside `kfwc_ring_get`: no out-of-bounds (all ring indexing is modulo `cap`, `total` is checked against `out_cap`), but garbage records and, after the failed-`ObReferenceObjectByHandle` path (:452-455, worker alive with `running == FALSE`), a later START frees `ranges` under the live worker → UAF read in `plat_ram_contains`.
Minimal fix: one guarded mutex (or an interlocked `starting/stopping` state) around the whole of `capture_start` and `capture_stop`; do the `ranges` swap and ring re-init under `s->lock`; on the :452 failure path wait for the thread by handle before returning.

**F-06 — INFO (DIAGNOSTIC mode `x-gsp-observer`, kf3 and patched vfio-pci). Guest-steered host CPU while the observer is on; BQL taken on a vCPU.**
`tools/vfio-gsp-observer/core/observer.c:79-111`, `qemu/hw/misc/kf3/kf3.c:240-250`. Every guest write to BAR0 0x110c00-0x110c3f (and access to 0x110004/8) runs `vg_poll`: up to four 1 MiB guest-RAM copies, two page-table re-reads, `memcmp`, a ≤255-slot walk with ≤16-page checksums, all under the BQL, taken from the trapping vCPU thread for kf3. A guest that hammers the doorbell page burns host CPU and stalls the main loop. It is bounded: by the window (`x-gsp-observer-seconds`, default 300, max 7200, `gsp-observer.h:13`), by `RECORD_CAP`/`SOURCE_CAP` (64 MiB) for output, and it exists only when the operator turns the diagnostic on (README: RESEARCH, "changes neither product semantics nor a queue pointer"). It also relaxes the "nothing blocks on a vCPU" rule while on (kf3.c comment says so). No fix required for product; if the mode is ever used with an untrusted guest, rate-limit `vg_poll` (e.g. one poll per ms).

**F-07 — INFO (OPTIONAL patched-host tier H5; build-only; product does not issue the escape). Device-wide DMA mask is lowered for the caller's whole `RM_ALLOC_MEMORY`.**
`tools/host_patches/h5_userd_dma/patch/nvidia_h5_userd_dma_595.91.07.patch:300-367`. Any process that can open `/dev/nvidiaN` can (i) hold `nv_kf_dma_window_lock` across its own allocation (other windowed callers wait, interruptibly), (ii) make every concurrent DMA mapping of that GPU land below 2^40 during the window (documented in the patch as acceptable), (iii) if the restoring `dma_set_mask` fails, leave the mask lowered until the adapter re-initialises (logged once). No address is named, nothing is widened, no memory-safety issue; the guest cannot reach the escape directly, and kayfabe does not call it yet (`crates/kf-host/src/tier.rs:13`: "wired to a log line and a capability bit only"). When the product starts issuing it, count calls per VM (they are per USERD-in-sysmem channel, so already bounded by the channel budget). Note for the design doc: `NV_ACTUAL_DEVICE_ONLY` means the escape must be issued on the GPU node, not `nvidiactl`.

**F-08 — INFO (EXPERIMENTAL EFS patch; off by default; no product client). Costs and logs a fault storm can drive.**
`tools/uvm_efs/patch/uvm_efs.c`: (i) `uvm_efs_try_divert` (:297-313) scans all `max_records` slots under a spinlock per coalesced fault in the fault bottom half; with the admin maximum 65535 that is a large per-fault cost on a path shared by every tenant of the GPU (default 1024). (ii) `UVM_ERR_PRINT` at :423 and :434 is not rate-limited; a cancel that keeps failing would log once per record per tick. (iii) at shutdown a failed `uvm_kvmalloc` (:573) drops the records without the hardware cancel (stock teardown then stops the channels; safe, but the "cancelled as stock would" claim does not hold on that path). (iv) parked faults delay other tenants' overflow-dropped faults by up to one tick (≤ 1 s safety replay, :533-538): bounded by design. Bounds exist and are named (`uvm_efs_max_records`, `uvm_efs_timeout_ms`); nothing unbounded found. Suggested: cap the module parameter lower (e.g. 4096), use the `_RL` print variants.

**F-09 — INFO (docs). `tools/host_patches/h5_userd_dma/include/` holds 63 files, not "the two headers the patch adds".**
README.md table and `docs/design/V3_H5_USERD_DMA_PATCH.md:332` say two; `tests/run_tests.sh` checks only those two. The other 61 are stock NVIDIA headers with their MIT SPDX line (public OGKM, so not a provenance problem), apparently committed with the build tree in 8fd87dd7e. Either delete them or correct the README (doc-hygiene rule: an unmarked sentence is a claim of fact).

**F-10 — INFO (test app, never shipped).** `firmware/kf-gop/src/bin/kf-gop-test/check_unsafe.rs:191-194` `w * h` and `len * 4` are unchecked before `allocate_pool` + `from_raw_parts_mut`; inputs come from the validated descriptor (`MAX_DIM`), so no overflow is reachable. Use `checked_mul` for form.

### False positives considered and rejected

- **Blt size overflow** (`blt.rs:131-140`, `execute`): every sum/product that sizes the caller buffer is `checked_*`; `execute` uses `.get()` on every slice and `fits()` first; the unchecked `o + row` etc. are bounded by the validated `buffer_extent`. Not a finding.
- **Framebuffer write outside the BAR**: `probe::place` + `BootFramebuffer::fits` + `Geometry::check` + `i < words`. The descriptor is authored by the host and checksummed; the guest cannot alter the ROM BAR (ROM region). Not a finding.
- **H5: size bypass through `NV_ESC_IOCTL_XFER_CMD`** (stock nv.c:2508-2534 does not re-validate the inner cmd): both new cases compare `arg_size` with `sizeof` themselves. Not a finding.
- **H5: kernel stack/heap leak to user**: `nv_kf_query` memsets; the window struct's outputs are all assigned; `inner` is fully overwritten by `copy_from_user` before any copy-out. Not a finding.
- **EFS: stale or forged record id acts on another record / another VA space**: id = seq<<16|slot with `in_use && rec->id == id`; records are per-file; the ABI carries no pointer, PDB or instance. Not a finding.
- **EFS: owner check defeated by pid reuse**: `efs->owner` is a referenced `struct pid`, compared by pointer. Not a finding.
- **EFS: timeout work after free**: `closing` under lock → `cancel_delayed_work_sync` → free after bottom-half flush. Not a finding.
- **Observer: `slots[256]` overflow / element wrap read past the snapshot**: `count ≤ 255`, wrap is `% count` inside a snapshot of exactly `size` bytes. Not a finding.
- **kfwc ring: `total = 32 + payload` wrap**: can only shorten a copy; indexing is modulo `cap`; `total > out_cap - done` stops. Not a memory-safety finding (records come from the engine, not the caller).
- **rpctrace: reader races `vfree`**: `proc_remove(proc_nvidia)` runs before `nv_rpctrace_shutdown` (rpctrace.patch:69-70) and waits for in-flight reads; RPC traffic has stopped at module exit. Not a finding (the unlocked `rpctrace_ring == NULL` pre-check in `nv_rpctrace_record` is benign under that ordering).

### 4. Owner rules touched in this scope

| rule | verdict | where |
|---|---|---|
| Stock install needs no host patches; the patched host is an optional tier | HOLDS | `crates/kf-host/src/tier.rs:149-200` treats EINVAL/ENOTTY as stock; `HostTier::h5_dma_window()` is read by nothing that gates boot (`tier.rs:13`); `crates/kf-host/src/lib.rs:532` only probes and logs. Side effect on a stock host: one `NVRM:unknown NVRM ioctl command` kernel log line per device open (bounded: once per realize). |
| Passthrough / USERD rules where H5 bears on them | UNVERIFIED | H5 is build-only (`V3_H5_USERD_DMA_PATCH.md:3`), never loaded; the product does not issue `NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW` (`tier.rs:24`), so the "USERD sysmem fix absent" statement in earlier notes (not retained) still describes master. The patch itself adds no mapping and names no address (CODE READ). |
| RE boundary: nothing derived from closed binaries in the repo | HOLDS as far as read; not an exhaustive audit | `tools/windows-ctrl-export/README.md` and `tools/windows-runlist-probe/README.md:3-25` record the 2026-10-31 removal of the generators that read a retail driver; remaining data (`tools/api_boundaries/windows_controls.tsv`, `windows_ids.txt`, `tools/windows-runlist-probe/native-58151.json`) is trace-measured request metadata with source tags, not disassembly. `tools/windows-capture` reads RAM/registers of an owned machine (measurement). Text that still mentions a "private diagnosis" / "retail driver" as the origin of a HYPOTHESIS: `tools/display-ilut-constructor-probe/README.md:10-16`, `tools/windows-debug-capture/evidence/alloc-memory-20261005/README.md:6-14` (marked hypothesis, links removed). `tools/host_patches/h5_userd_dma/include/*` are public MIT OGKM headers (F-09). |
| A guest-reachable unbounded read or emit is a bug | HOLDS in this scope | Observer: reads bounded per trigger, output capped (RECORD_CAP, SOURCE_CAP, FIFO), time-boxed (F-06 is bounded cost, diagnostic). EFS: table, timeout and wait/resolve counts bounded (F-08: un-ratelimited error print on a failing cancel path, experimental patch). kf-gop: the release ROM contains no port I/O (`main.rs` cfg), so no guest-driven host log. |
| Unsafe only in `*_unsafe.rs` (firmware crate) | HOLDS | `firmware/kf-gop/src/lib.rs:15` `forbid(unsafe_code)`; unsafe only in `driver_unsafe.rs`, `efi_unsafe.rs`, `port_unsafe.rs`, `bin/kf-gop-test/check_unsafe.rs`; `Cargo.toml:77,82` deny `unsafe_op_in_unsafe_fn` and `undocumented_unsafe_blocks`. |

### 5. Not reviewed (or only skimmed)

- `archive/**` and `traces/**` patches (out of scope); `scripts/bench/windows/appmatrix/directx_vrs_gate.patch` (an app-source patch, not kernel/QEMU).
- `tools/uvm_efs/tests/*.cpp|*.cu`, `tools/uvm_efs/box/*.sh`, `tools/host_patches/h5_userd_dma/tests/hw/kf_h5_hwtest.c` and `box/*.sh`: test/bench programs, skimmed for the ioctl shapes only.
- `tools/cudart_probe/*.c` (LD_PRELOAD shims), `tools/probes/kvm_ioeventfd_readonly.c`, `tools/windows-capture/user/kfwc_collect.c`, `kfwc_meta.c`, `tools/windows-capture/tests/*`: skimmed + analyzer only (C23).
- `scripts/rpctrace/cuda_ioctl_trace.c`, `cuinit_probe.c`, `pcie_link_load.c`: not read.
- `crates/kf-oprom/src/{pe,rom,pack}.rs` and `crates/kf-gop-image/build.rs`: safe Rust; only the descriptor path (`desc.rs`) was read in full. A panic in these parsers inside the firmware is a guest boot hang on a host-authored ROM, not a breakout. The nested-cargo build (supply chain) is not a memory-safety topic and was not audited.
- All Python/shell under `tools/` (no memory-unsafe code).
- EFS and H5 interaction with the closed RM core and with real hardware: nothing was built, loaded or run; the EFS hook hunks in `uvm_gpu_replayable_faults.c` were read against the patch context only, not against the full 580.159.04 file.
- The kf3.c call sites of the observer (BQL/vCPU threading, unrealize ordering at kf3.c:1826-1863) belong to the kf3.c part; noted only.
- No `cargo` build/test of the firmware (disk at 3.4 GB free, below the 2.5 GB + build margin I was given); no Miri (not installed).

### 6. Tool runs (MEASURED) (2026-11-13)

`gcc 15.2.0 -fsyntax-only -Wall -Wextra -fanalyzer` over `tools/windows-capture/lib/*.c`, `tools/vfio-gsp-observer/core/*.c` (also `-Wconversion`), `tools/cudart_probe/*.c`, `tools/probes/*.c`, `tools/windows-capture/linux-probe/mailbox_probe.c`, `tools/memory-list-probe/*.c`, `tools/host_patches/h5_userd_dma/tests/test_dma_window.c`: zero analyzer diagnostics (only `_GNU_SOURCE redefined` from my own `-D`). Two files did not compile in this environment (`kfwc_collect.c`: Windows headers; `nvdiff_shim.c`: generated header). Logs: (not retained).


---

## Appendix G (file G_audit_ledger.md)

Part status as of 2026-11-13: Security review 2, PART G: ledger of prior audit findings re-checked against origin/master 921c39444. Read-only; nothing built or run.

### G. Prior-audit findings ledger (stage 1 S1-xx, w301) against master

Sources read: `docs/audits/2026-10-03-v3-stage1.md` (67 findings, audited at `12a526df`), `docs/audits/w301_cancellation_error_leaks.md` (14 ranked findings, audited at `91f8b34b`, PRE-v3 tree), `docs/design/V3_SECURITY_AUDIT_PLAN.md` §4, `docs/design/V3_SECURITY_MODEL.md` §7-§9.

**Stage-2 findings document: none exists.** Searched `docs/audits/` (two files only), `docs/design/V3_*SECUR*`, `docs/history/`, `traces/v3_security/` (three dirs: `libcuda_20261003`, `merge_bar_1d71f3db`, `nonpriv_20261003`), and `grep -rE 'S2-[0-9]{2}'` over docs/crates/scripts (0 hits). The plan's own STATUS line says the same (read 2026-11-07). MEASURED (grep).

Method and labels. Status on master = FIXED / PARTLY / OPEN / REGRESSED (worse than at the audit) / STALE-LOCATION (audited code is archived) / NOT-CHECKED. CODE READ = I read the current symbol; GREP = presence/absence by grep on master only; INFERRED = no code change found by `git log --grep` + grep, the body not re-read. Finding IDs appear in only a handful of commit messages (S1-21 ×7, S1-23 ×6, S1-43 ×3, a few ×1), so fixes were located by symbol.

Addendum of 2026-11-13: add the REGRESSED state, give a row to every "stage 2 inherits" item (§4 of the stage-1 doc) and each of the 33 bug classes (§3.4), and apply the severity rule (guest breakout = CRITICAL; unbounded DoS counts unless a limit is documented as planned). Folded in below (sections G.3, G.4, G.6).

### G.1 Counts (67 stage-1 findings)

| status on master | count | IDs |
|---|---|---|
| FIXED | 3 | S1-29 (doc), S1-43, S1-84 (profile half) |
| PARTLY | 8 | S1-21, S1-22, S1-23, S1-25, S1-33, S1-80, S1-89, S1-86 |
| OPEN | 52 | all others not listed in this table |
| REGRESSED (open and grown) | 4 | S1-08, S1-11, S1-16, S1-41 |
| NOT-CHECKED | 0 | (25 of the OPEN rows are GREP/INFERRED only; marked per row) |

w301 (14 findings): 2 FIXED in the pre-v3 tree (w303: #1 reap, #4 preempt), 12 STALE-LOCATION (the audited crates are under `archive/crates/` or frozen; the isolate does not exist in v3). v3 analogues were not audited here (NOT-CHECKED), except the notes in G.5.

### G.2 Stage-1 findings

Severity is as recorded in the stage-1 document. "Docs claim" is the Status line in the audit document (the plan §4 says "every row is open").

### §2.1 Host pointers / unsafe gates (S1-01..16)

| ID | sev | statement | cited location | docs claim | master | evidence | label |
|---|---|---|---|---|---|---|---|
| S1-01 | major | Lexical unsafe gates passable by rustfmt-clean code (gate A = two line greps; gate C drops lines starting with `*`) | ci.yml gate A / gate C | open | **OPEN** | `.github/workflows/ci.yml:1652-1665` gate A is still `grep -q '^\[lints\]' && grep -q '^workspace = true'`; `:631-638` still filters `(//|/\*|\*)`-led lines; no `-F unsafe_code` semantic gate, no tomllib (grep: 0 hits) | CODE READ |
| S1-02 | major | Host-pointer gate sees only `*mut`/`*const`/`NonNull`/`transmute` | ci.yml host-pointer gate | open | **OPEN** | `ci.yml:689-699` pattern unchanged; `clippy.toml` has one key (`large-error-threshold`), no `disallowed-methods/types`; no `&raw` in the pattern | CODE READ |
| S1-03 | major | Console frame host address crosses safe code as `usize`; `Kf3Frame` has no length; C stride check compares bytes with pixels | kf-cuda display.rs `addr()`, kf-qemu display.rs `FrameView.addr`, ffi_unsafe.rs, kf3.h, kf3.c | open | **OPEN** | `crates/kf-cuda/src/display.rs:308` `pub fn addr(&self) -> usize`; `crates/kf-qemu/src/display.rs:885` `pub addr: usize`; `ffi_unsafe.rs:809` `data: f.addr as *mut u8`; `kf3.h:94-98` Kf3Frame still {data,width,height,stride,format,serial}; `kf3.c:1063` still `f.stride < f.width` | CODE READ |
| S1-04 | major | kf-cuda public SAFE API takes raw device addresses (`mem_free`, `memcpy_*`, `launch_raw`, `memset_d8*`) | driver_unsafe.rs | open | **OPEN** | `crates/kf-cuda/src/driver_unsafe.rs:685,697,715,733,805,825,841` all still `pub fn`; no `pub(crate) unsafe fn`, no `DeviceSlice` (grep) | CODE READ |
| S1-05 | major | Under unified addressing/HMM in-process CUDA contexts can address the VMM; compose kernel has no in-kernel bound; no test ties `ComposeLayer::check` to the PTX address function | kf-cuda display.rs, kf_scanout.ptx | open, narrowed by owner §Q | **OPEN** | no `PAGEABLE` query anywhere in kf-cuda/kf-qemu (grep 0); `crates/kf-cuda/tests/` has no compose-address proptest (files: cuda_thread_posture, report_containment, synth_origin, walk_abi_matches_the_cu, walk_never_synchronizes) | GREP |
| S1-06 | major | v3 unsafe-soundness ledger walks the v2 crates | kf-linux-raw tests/unsafe_naming.rs | open | **OPEN** | `crates/kf-linux-raw/tests/unsafe_naming.rs:41` still `CRATES = ["kayfabe-linux-raw", "kayfabe-qemu-raw"]`; rows at `:97-130` name `crates/kayfabe-linux-raw/...`; `kayfabe-qemu-raw` does not exist. kf-qemu and kf-cuda have no ledger | CODE READ |
| S1-07 | minor | `RawRegion` is `Copy`, no lifetime; copies outlive `kf3_ram_del` | raw_unsafe.rs | open | **OPEN** | `crates/kf-qemu/src/raw_unsafe.rs:5-9` still `#[derive(Debug, Clone, Copy)] pub struct RawRegion { ptr: *mut u8, len: usize }`; no generation/keep-alive | CODE READ |
| S1-08 | minor | Safe `pub extern "C" fn` entries deref a raw handle through private `dev()` | ffi_unsafe.rs | open | **REGRESSED (grown)** | was 11 safe entries; now 25 `pub extern "C" fn` (24 take `h: *mut c_void`) vs 18 `pub unsafe extern "C" fn`; `fn dev<'a>(h) -> Option<&'a Device>` at `ffi_unsafe.rs:116-120` still a safe fn minting an unbounded lifetime | CODE READ |
| S1-09 | minor | Generic safe `view_bytes<T: Copy>` / `zeroed<T: Copy>` unsound | driver_unsafe.rs | open | **OPEN** | `driver_unsafe.rs:1627`, `:1658` unchanged signatures; no `PlainBytes` sealed trait | CODE READ |
| S1-10 | minor | Raw fd numbers as `i32`; `import_and_map` turns a negative fd into fd 0 | driver_unsafe.rs | open | **OPEN** | `driver_unsafe.rs:1745` `import_and_map(&self, device: i32, fd: i32, bytes: usize)`; `:1762` `usize::try_from(fd).unwrap_or(0) as *mut c_void`; `:1689` `pub fd: i32`. (An `OwnedFd` now exists at `:1474` for a different object) | CODE READ |
| S1-11 | minor | Unsafe ratchet undercounts attributes, extern blocks, macro expansions | ci.yml ratchet pattern | open | **REGRESSED (grown)** | pattern unchanged (`ci.yml:2009,2013`: `unsafe (\{|fn |impl |trait )|unsafe extern "C" fn [A-Za-z_]`); counted surface grew: kf-linux-raw 75→101, kf-cuda 75→78, kf-qemu 44→86 (`ci.yml:1652`), so the uncounted `#[unsafe(no_mangle)]`/extern-block share grew with it | CODE READ |
| S1-12 | minor | Pointer gates have no known-positive self-tests | ci.yml, scripts/ci | open | **OPEN** | no `test_pointer_gates` or fixture in `scripts/ci` (grep 0) | GREP |
| S1-13 | minor | `kf3.c` outside every gate, not compiled in CI; stride check mixed units | kf3.c | open | **OPEN** | no workflow compiles `qemu/hw/misc/kf3/kf3.c` (only a comment at `ci.yml:1604`); no `-fanalyzer`/`--analyze`; stride check unchanged (`kf3.c:1063`). File grew 927 → 2027 lines | GREP + CODE READ |
| S1-14 | minor | No gate pins kf3's external dependency set | dependencies.py | open | **OPEN** | no `cargo metadata`, `cargo deny/audit/vet` in `scripts/ci/dependencies.py` or workflows; no `deny.toml` | GREP |
| S1-15 | info | `pLinearAddress` (host physical) held as `u64` in safe code | kf-host lib.rs | open | **OPEN** | `crates/kf-host/src/lib.rs:75` `pub p_linear_address: u64`; no `RmMapCookie` | GREP |
| S1-16 | info | Dead unsafe surface (KVM/vCPU/affinity) linked into kf3; CUDA handles as `usize` | kf-linux-raw lib.rs, driver_unsafe.rs | open | **REGRESSED (partly moot)** | `kf-linux-raw/src/lib.rs:271,287,298` modules unconditional, re-exported `:317,:335`; no feature gate. `PinnedBuf { ptr: usize }` still at `driver_unsafe.rs:1407`. Note kf-qemu now uses `KvmVm::ioeventfd` (`ci.yml:1400-1410` comment), so part of kvm_unsafe is no longer dead but live surface | CODE READ |

### §2.5 Security model (S1-20..34)

| ID | sev | statement | docs claim | master | evidence | label |
|---|---|---|---|---|---|---|
| S1-20 | blocker | USER host channels refusing physical-mode/privileged work is the host boundary and is untested (T-PHYS-CE) | open (box test not run) | **OPEN** | `docs/design/V3_WINDOW_EXPOSURE_REVIEW.md:208-209,273`: "T-WINDOW-USER, T-RING-TRANSLATED and T-PHYS-CE have never run", probe tools "have not been written"; no trace dir for it under `traces/` | GREP (docs) |
| S1-21 | blocker | Identity windows user-accessible in every twin (A.9) | open, built behind flag, default OFF | **PARTLY** (code fix hardwired ON; hostile box test owed) | `KF3_TSPACE` deleted, always ON since 2026-10-10 (`crates/kf-qemu/src/tspace.rs:22`, `twin.rs:27`, `mem.rs:1684`, OWNER_RULINGS §AB); T-WINDOW-USER never run (same doc lines as S1-20). The audit doc still says "default OFF" (stale) | CODE READ (comments + flag grep) |
| S1-22 | major | Under a root VMM only births are de-privileged | open for other kf-host RM calls; libcuda half built | **PARTLY** | `kf_cuda::posture` on master; no in-process `GET_PRIVILEGED_STATUS` gate test found; `ioctltrace.rs:9` still says "complete by construction" (check 4 not done) | GREP |
| S1-23 | major | Guest kernel reaches Translated rings via forwarded virtual operands | open, built behind flag | **PARTLY** | T-mode rewriter is the only path (`chan.rs:1715`, `tspace_unsafe.rs`); T-RING-TRANSLATED never run, guest module not written | CODE READ (comments) |
| S1-24 | major | RPC/control decoders reachable from guest userspace; no fuzz | open | **OPEN** | no `fuzz/` dir, no cargo-fuzz/proptest job in ci.yml | GREP |
| S1-25 | major | Shared host-GPU resources not bounded per VM | open | **PARTLY** | display-SW caps and CPU-window view cap exist (`crates/kf-mem/tests/secaudit_cpu_window_views_are_capped.rs`); NVENC, twin VRAM, BAR1 across VMs, retire queue, timeslice, RM call rate still unbounded (rows S1-42/44/45/82) | GREP |
| S1-26 | major | THE_CONSTRAINTS §20 walker placement argument stale | open | **OPEN** (doc) | no dated correction naming the in-VMM walker found at §20; the file-level STATUS says isolates are history, which is not the fold-above-the-text fix | GREP |
| S1-27 | major | Copy-then-check only guard; no racing-writer test or type rule for Rust readers | open (stage 2) | **OPEN** | no racing-writer harness or compile-fail test found (grep `racing`, `trybuild` in kf-gsp/kf-chan: 0) | GREP |
| S1-28 | major | Plan omits packaging/install/supply chain | open | **OPEN** | plan §2 still ten areas | CODE READ (doc) |
| S1-29 | minor | Plan named `NV01_ROOT_NON_PRIV` | closed in that commit | **FIXED** | corrected text in plan §1/§4 | CODE READ (doc) |
| S1-30 | minor | Host usermode page readable by guest (clock disclosure) | open (owner decision) | **OPEN** | design unchanged; not re-read | INFERRED |
| S1-31 | minor | Residual VRAM relies on host scrub-on-free | open | **OPEN** | not re-read (`crates/kf-qemu/src/scrub.rs` exists; whether it scrubs at reservation was not checked) | NOT re-read, INFERRED |
| S1-32 | minor | Guest vIOMMU neither supported nor refused | open | **OPEN** | no `pci_device_iommu_address_space` in `kf3.c` (grep 0); the realize blocker at `kf3.c:1577-1594` is about RAM-pinning co-residents (VFIO/iommufd), not a vIOMMU | GREP |
| S1-33 | info | Broker holds console authority | open (stage 2) | **PARTLY** | both named tests exist (`crates/kf-broker/tests/broker_loopback.rs`, `relay_machine.rs`); message-rate bound not checked | GREP |
| S1-34 | info | §39(f) names a v2 chokepoint | open | **OPEN** | `THE_CONSTRAINTS.md:2494` still names `StoreMapPort::apply_ops`; no `apply_entry` mention | GREP |

### §2.6 RM calls (S1-40..50)

| ID | sev | statement | docs claim | master | evidence | label |
|---|---|---|---|---|---|---|
| S1-40 | major | RM companion size fields (`paramsSize`, NVOS02 `limit`, `pageCount`) are a safe-caller contract the funnel never checks | open | **OPEN** | `crates/kf-linux-raw/src/chardev_unsafe.rs:370` still "companion *size* field, which the caller must set itself"; `:897` "the length the caller must mirror into the size field"; no size-field descriptor on `Indirect` | CODE READ (doc comments; funnel body is PART B's) |
| S1-41 | major | No host-side allowlist of RM controls/classes | open | **REGRESSED (grown)** | `kf-host/src/lib.rs:889,994,1061,1130` still `pub fn raw_alloc/raw_alloc_via/raw_alloc_nested/raw_control`, plus a NEW `pub fn raw_control_opaque` (`:1190`); kf-qemu callers grew (`rmfacts.rs:63,77,86,164`, `chan.rs:5836,5878`); no allowlist table in kf-host | CODE READ |
| S1-42 | major | NVENC sessions GPU-wide, no per-VM cap | open | **OPEN** | `crates/kf-qemu/src/chan.rs:5813-5880` `enc_sessions` per-client map, no total cap constant | GREP |
| S1-43 | minor | Guest FB USERD/notifier offsets can name the carve-out | fixed behind flag, open by default | **FIXED** | strict is hardwired since 2026-10-10 (`tspace.rs:46-47`, `chan.rs:2199-2220` `in_usable_heap`, test `chan.rs:9319`; `kf-chip/tests/usable_heap.rs`). Test name still says "only_when_strict" | CODE READ |
| S1-44 | minor | Guest `SET_TIMESLICE` unbounded | open | **OPEN** | no clamp/max near timeslice in `kf-rm/src/chanlink.rs` (grep) | GREP |
| S1-45 | minor | PRAMIN view retire queue unbounded (fd + BAR1 per pending view) | open | **OPEN** | `crates/kf-qemu/src/mem.rs:641` still `std::sync::mpsc::channel::<StoreView>()` (unbounded); only the node channel is `sync_channel(TRAP_NODES)` | CODE READ |
| S1-46 | minor | Guest-visible usermode alias backed by a RW shared host mapping | open | **OPEN** | `kf-host/src/lib.rs` `open_usermode` → `map_cpu(.., WriteBack)` (RW path); `ViewAccess::ReadOnly` is used by timer.rs and userstore.rs only | GREP |
| S1-47 | info | std `Mutex` held across an RM ioctl | open | **OPEN** | `kf-host/src/event.rs:233-241` `armed` guard pattern unchanged | GREP |
| S1-48 | info | Translated rewriter forwards on a deny-list | open | **OPEN → largely superseded** | T-mode rewriter is an address author (S1-23 fix); per-family unclassified-method refusal status not checked | INFERRED |
| S1-49 | info | Handle minting wraps | open | **OPEN** | `kf-host/src/lib.rs:1112` `o.next = o.next.wrapping_add(1)` | CODE READ |
| S1-50 | info | Realize-time GSS replay sends 0xa5 fill | open | **OPEN** | not re-read | INFERRED |

### §2.7 Walker (S1-60..72)

| ID | sev | statement | docs claim | master | evidence | label |
|---|---|---|---|---|---|---|
| S1-60 | major | VER3 (Hopper/Blackwell): unmapped big PTE does not veto stale 4 KiB PTEs | open; blocker for Hopper/Blackwell release | **OPEN** | `cuda/walk/kf_walk.cu:409-414` `kf_big_pte_unmapped` still `if (F.table_version != KF_TBL_VER2) return false;`; `crates/kf-cuda/src/walk.rs:756-762` still "VER3 … is ACCEPTED … KNOWN GAP (w826)". Blackwell is now a worked family (memory/handoff), so the exposure is no longer hypothetical | CODE READ |
| S1-61 | major | Walk resources shared per refresh; one space fails every batched space | open | **OPEN** | no per-entry budget change found; not re-read in depth | INFERRED |
| S1-62 | minor | After truncation/abort the kernel's diff reads stale walk-table runs | open | **OPEN** | not re-read | INFERRED |
| S1-63 | minor | Entry budget is a flag, not a work bound; `FRONTIER_CAP` untested | open | **OPEN** | `kf_tests.cu`: no test names FRONTIER (grep 0) | GREP |
| S1-64 | minor | Race tests never assert leaf containment | open | **OPEN** | `kf_tests.cu:135` `cfg_default` still `gpga_span = KF_GPGA_SPAN_UNBOUNDED`; `cfg_span` used only by static cases (`:1375-1436`, `:1834`) | GREP |
| S1-65 | minor | No hostile/fuzz test reaches the production walk path or a CPU model | open | **OPEN** | no model/fuzz target found | GREP |
| S1-66 | minor | Commit kernel writes the acked-MAP list before checking fit | open | **OPEN** | `kf_walk.cu:1358` `B[nb + ex] = r;` precedes the check at `:1363` | CODE READ |
| S1-67 | minor | VER3 PDE sparse encoding (PCF=3) not honoured | open | **OPEN** | `kf_walk.cu:366-370` VER3 arm still a single equality against `pcf_sparse`, marked "UNTESTED" | CODE READ |
| S1-68 | minor | Walker security gates not in CI; PTX tied to `.cu` only via `KfArgs` size | open | **OPEN** | no `check-invariants`, `check_s39`, `make_ptx`, nvrtc in any workflow (only a comment at `ci.yml:1306`) | GREP |
| S1-69 | info | `Report::validate` weaker than the C validator; unknown op applied as MAP | open | **OPEN** | `crates/kf-mem/src/apply.rs:354` still `unmap: m.op == KFWR_OP_UNMAP` (else map) | CODE READ |
| S1-70 | info | Walk descriptor not validated at run time | open | **OPEN** | no `format_check` in kf-cuda src (grep 0) | GREP |
| S1-71 | info | Warp-collective shuffle after a per-lane abort read | open | **OPEN** | `kf_walk.cu:2130` still `if (kf_par_aborted(d)) {…return;}` per lane; no `__any_sync` (grep 0) | CODE READ |
| S1-72 | info | Turing decoded with GA10x geometry | open | **OPEN** | not re-read | INFERRED |

### §2.10 Bug classes (S1-80..91)

| ID | sev | statement | docs claim | master | evidence | label |
|---|---|---|---|---|---|---|
| S1-80 | major | kf3 hot-unpluggable; exit neither joins threads nor deletes its BH | open | **PARTLY** | FIXED half: `qemu/hw/misc/kf3/kf3.c:2001` `dc->hotpluggable = false` (commit `56e7932e9`), test `qemu/hw/misc/kf3/tests/test_hotplug_policy.py` (a C-shim model; NOT referenced by any workflow or `scripts/ci` — grep 0, so it is not a gate that runs). OPEN half: no `qemu_bh_delete` anywhere in kf3.c (`bar1_bh` created `:770`, scheduled `:734`); `kf3_dev_exit` (`:1842-1885`) calls `kf3_unrealize`, which only stops threads (no join). Reachable now only at process exit / failed realize paths (PART C owns the lifecycle review) | CODE READ |
| S1-81 | major | Late-invalidate tripwire never called | open | **OPEN** | `Trigger::is_overdue` (`crates/kf-trap/src/shadow.rs:207`) has callers only in `crates/kf-trap/src/tests.rs:409-418` | CODE READ |
| S1-82 | major | Twin RM-owned context buffers take host VRAM outside `fb-mb`, uncapped | open | **OPEN** | no twin/TSG cap found; not re-read | INFERRED |
| S1-83 | major | Hostile-input instruments retired (fuzz, tsan, mutants on product code, advguest) | open | **OPEN** | no `fuzz/`; ci.yml mutant jobs are CI-infra mutants (`scripts/ci/*_mutants.py`, `ci/vast-broker/*-mutants.mjs`), not product-decoder fuzz; no miri/loom/tsan job | GREP |
| S1-84 | minor | Release builds wrap on overflow | open | **FIXED (profile) / PARTLY (CI)** | `Cargo.toml:93-94` `[profile.release] overflow-checks = true` with a comment naming the audit reason; no `cargo test --release` of the guest-facing crates in ci.yml; no clippy arithmetic ratchet checked | CODE READ |
| S1-85 | minor | Translated rewriter silently drops out-of-range methods under a false comment | open | **OPEN** | `crates/kf-chan/src/translated.rs:937-945` `emit` unchanged, same comment, `None` still dropped; no `MethodOutOfRange` | CODE READ |
| S1-86 | minor | Guest-driven log lines uncapped on several paths | open | **PARTLY** (unverified per path) | some caps exist (as at audit); no single rate-limited logger found; per-path re-read not done | INFERRED |
| S1-87 | info | Typed address domains unused | open | **OPEN** | `kf-qemu/src/mem.rs` uses neither `Gpa` nor `StoreOffset` (grep count 0) | GREP |
| S1-88 | info | Doorbell-to-register fence vestigial | open | **OPEN** | no `fence_class` in kf-chip | GREP |
| S1-89 | info | Capture-derived control table served on every family | open | **PARTLY** | `kf_rm::consttab` per-die table now exists (`crates/kf-rm/src/consttab.rs`), `cudartinit` still referenced (`hostquery.rs:1704`); serve-only-on-captured-die rule not verified | INFERRED |
| S1-90 | info | No realize-time assert of `disable_reentrancy_guard` | open | **OPEN** | no such assert in kf3.c (grep 0) | GREP |
| S1-91 | info | Process-global statics shared across devices | open | **OPEN** | `kf-qemu/src/mem.rs:515` `static IDX: OnceLock<ViewIndex>`, `kf-host/src/channel.rs:2136` `static REAPER` | CODE READ |

### G.3 "What stage 2 inherits" (stage-1 doc §4), one row per item

| item | status on master | evidence |
|---|---|---|
| §2.2 inventory: S1-04, S1-08, S1-09, S1-10, S1-40, re-pointed ledger S1-06 | none done; S1-08 REGRESSED | rows above. No inventory of `*_unsafe.rs` public items with a refusing test per precondition exists; this review (parts A-F) is the first |
| §2.2 method: Miri on unsafe unit tests | NOT DONE | `cargo miri` not installed on this host; no miri job in ci.yml |
| §2.2 method: Kani on window placement / store slices / PRAMIN pool | NOT DONE | grep `kani` in crates/.github: 0 |
| §2.3: S1-27, S1-47, S1-07, S1-71, S1-80 | all open (S1-80 partly) | rows above |
| §2.3 method: loom/shuttle models (doorbell word, FrameRing, PRAMIN generations) | NOT DONE | no loom/shuttle dependency or job (grep 0 in ci.yml) |
| §2.4: model §7 table; S1-25, 42, 44, 45, 61, 82, 86 | open; the "one table with cap and cap's test" output does not exist | model §7 still lists 8 rows "open" (two corrected to bounded on 2026-10-04) |
| §2.8: S1-84, S1-49, S1-85 | S1-84 fixed (profile); 49, 85 open | rows above. Clippy `cast_possible_truncation`/`arithmetic_side_effects` ratchet on guest-facing crates: not found in `scripts/ci/clippy.sh` (not read in full; GREP) |
| Proposed area 11, packaging/install (S1-28) | no owner decision recorded in the plan | plan §2 unchanged |
| Re-run §2.1 graph + FFI inventory after `v3-gop` | NOT DONE as a document; surface moved a lot | kf3.c 927→2027 lines; FFI entries 24→43 (25 safe + 18 unsafe); kf-qemu unsafe count 44→86; `firmware/kf-gop` (78) added to AUDITED |
| Box tests: T-PHYS-CE, T-WINDOW-USER, T-RING-TRANSLATED | NEVER RUN; probes not written | `V3_WINDOW_EXPOSURE_REVIEW.md:208-209,273` |
| Box tests: scrub canary (S1-31), twin VRAM per family (S1-82), NVENC two-VM (S1-42), root-port unplug (S1-80), HMM attribute (S1-05) | no evidence found | no matching dir under `traces/`; root-port unplug is moot for `device_del` after `hotpluggable=false` |

### G.4 The 33 bug classes (stage-1 §3.4) re-checked

Status at audit → status on master. Only the classes whose v3 status the audit left exposed/partial are re-judged; "closed/keep" classes were not re-audited (NOT-CHECKED for regression unless noted).

| # | class | audit said | master | note |
|---|---|---|---|---|
| 1 | Guest-chosen size drives a host copy | `_IOC_SIZE` closed; companion size latent (S1-40) | unchanged, OPEN | S1-40 |
| 2 | Guest pointer / host address crosses boundary | structurally addressed; P2 (S1-23) | PARTLY (T-space hardwired; S1-03 `usize` frame address still crosses) | S1-03, S1-23 |
| 3 | Sanitizer keyed on guest discriminator | closed | NOT-CHECKED | fuzz of `rewrite()` still absent |
| 4 | Missing table row never denies | mostly closed; S1-41 | host side REGRESSED (more raw verbs) | S1-41 |
| 5 | Capture-derived table expires | exposed (S1-89) | PARTLY (`consttab`) | |
| 6 | Lifetime: object outlives owner | exposed (S1-80, S1-07) | PARTLY (hotplug off); S1-07 open | |
| 7 | Ownership keyed by non-unique id | mostly n/a | NOT-CHECKED | |
| 8 | CAS failure read as ownership; lost wakeup | closed | NOT-CHECKED; loom model still absent | |
| 9 | Unlock/relock, double fetch | no gate for Rust readers (S1-27) | OPEN | |
| 10 | Blocking on vCPU / write-drop | S1-90, S1-47 | OPEN (both) | |
| 11 | Refusal on a wait path forges a completion | closed for instances found | NOT-CHECKED | |
| 12 | Late invalidate leaves stale translations | exposed (S1-81) | OPEN | |
| 13 | Unbounded guest-steered host resources | exposed (S1-42, 82, 25) | OPEN (some caps added) | no documented planned limit found for NVENC/twin VRAM/retire queue beyond the audit's own "check" text |
| 14 | Unbounded logs | exposed (S1-86) | PARTLY/unverified | |
| 15 | Integer domain / release wrap | exposed (S1-84) | FIXED (overflow-checks = true) | |
| 16 | Bound on reads vs emits; validator on path | PTX gates not in CI (S1-68) | OPEN | |
| 17 | SAFETY comment describes safety the code lacks | exposed (S1-80, 85, 81, 64) | OPEN: S1-85 comment, S1-64 comment unchanged; `ffi_unsafe.rs:117-118` `dev()` SAFETY still asserts the C side's discipline | |
| 18 | Check exists but off the path / never run | S1-81, 87, 68, 83, 06 | OPEN (all five); NEW instance: `test_hotplug_policy.py` exists but no workflow runs it | |
| 19 | Struct ABI drift | addressed; S1-09 remains | OPEN (S1-09) | |
| 20-23 | status-in-struct; filled address field; lazy USERD; forwarded flags | closed | NOT-CHECKED | |
| 24 | Privileged host channel | in work | PARTLY (births + libcuda merged; other RM calls open, S1-22) | |
| 25 | In-guest boundary | P1 (S1-21) | PARTLY (code fixed, T-WINDOW-USER never run) | |
| 26 | Bench masks a defect | hides S1-80 | mitigated by hotplug off | |
| 27 | Cached transient "not yet" | closed | NOT-CHECKED | |
| 28 | Two `u64` address spaces | partial (S1-87) | OPEN | |
| 29 | Guest IOVA read as GPA | exposed (S1-32) | OPEN | |
| 30 | Stale evidence read as current | ledger exception (S1-06) | OPEN (S1-06) | |
| 31 | Severity by reachability | adopted | n/a | |
| 32 | Isolate/IPC classes | n/a in v3 | n/a | |
| 33 | Bench supply chain | partial (S1-28) | NOT-CHECKED | out of scope for the unsafe review |

### G.5 w301 (pre-v3 tree, audited at `91f8b34b`)

The document's STATUS is ANSWERED 2026-08-14 and says it is not being updated. The audited crates (`kayfabe-fwd`, `kayfabe-qemu-raw`, `kayfabe-isolate` shell paths, …) are under `archive/crates/` or frozen and do not ship in kf3. Rows are therefore STALE-LOCATION; the "v3 analogue" column is a GREP-level pointer only, not an audit.

| # | finding | docs claim | master | v3 analogue (GREP, not audited) |
|---|---|---|---|---|
| 1 | Reap never armed (host RM tree + isolate leak per dead guest process) | acted on, w303 (armed + composition-root test) | FIXED in pre-v3 tree; STALE-LOCATION | v3 has one RM client per device; release-on-client-free noted as handled in S1-42; not audited |
| 2 | Guest-RAM pins/joins/exports never released | NOT BUILT | STALE-LOCATION | v3 store/sysmem model differs; NOT-CHECKED |
| 3 | No GPU quiescence fence; refused doorbell commit frees the channel it rang | NOT BUILT / BUILT+REACHABLE | STALE-LOCATION | v3 rule "no forged completion for work that reached the GPU"; NOT-CHECKED (use-after-free-of-GPU-pages class, relevant to parts A/C/D) |
| 4 | `PREEMPT` answered NV_OK and performs nothing | acted on, w303 (decides; still does not preempt) | PARTLY in pre-v3; STALE-LOCATION | v3 `kf-rm/src/chanlink.rs:61-62` names `TSG_PREEMPT`; forwarding not audited |
| 5 | Host RM writes guest notifier slot 1 | mechanism verified, consequence unmeasured | STALE-LOCATION | NOT-CHECKED |
| 6 | Cancellation verbs unserved (`STOP_CHANNEL`, `RESET_CHANNEL`, `DISABLE_CHANNELS`, RC) | NOT BUILT | STALE-LOCATION | v3 references `DISABLE_CHANNELS` (`chanlink.rs:258`, `KF3_ASYNC_PREEMPT`) and STOP in `kf-host/src/channel.rs`; not audited |
| 7 | Descheduling never reaches the host | host plane NOT BUILT | STALE-LOCATION | NOT-CHECKED |
| 8 | `RC_TRIGGERED` emission orphaned | BUILT+ORPHANED | STALE-LOCATION | v3 has `kf-abi/src/rc.rs`, `eventnotify.rs`; reachability not audited |
| 9 | Isolate CE channel born with `hObjectError = 0` | NOT BUILT | STALE-LOCATION (no isolate) | n/a |
| 10 | Status flattening | BUILT+REACHABLE, lossy | STALE-LOCATION | NOT-CHECKED |
| 11 | `read_version` never reads in-struct reply | host-side | STALE-LOCATION | NOT-CHECKED |
| 12 | `device_reset` releases device half of FB joins early | BUILT+REACHABLE | STALE-LOCATION | v3 reset is new code (`kf-qemu/src/reset.rs`, quiesce test at `:848`); not audited |
| 13 | No `PR_SET_PDEATHSIG` for the isolate | NOT BUILT | STALE-LOCATION (no isolate) | n/a |
| 14 | `exec_engine_object` hard-codes `hObjectError = None` | differential hole | STALE-LOCATION | n/a |

### G.6 CI gate check requested by the directive

- **Unsafe-surface gate** exists: `ci.yml:615-651`. Excludes `archive/`, `third_party/`, `crates/kayfabe-doorbell/`, target dirs. Still comment-filter based (S1-01).
- **Host-pointer gate** exists: `ci.yml:689-715`, pattern unchanged (S1-02).
- **Containment gates A/B + ratchet** exist: `ci.yml:1087…1652-1700`, `AUDITED="kayfabe-linux-raw:127 kayfabe-cuda:43 kf-linux-raw:101 kf-cuda:78 kf-qemu:86 firmware/kf-gop:78 kf-chan:0"`; ratchet pattern at `:2009-2013`.
- **`unsafe_naming` test** exists but walks the v2 crates (S1-06 open).
- **`crates/kf-chan/src/tspace_unsafe.rs`**: gate B permits it because `kf-chan:0` is in AUDITED; the `:0` means (a) gate A does NOT exempt kf-chan, so it still inherits `unsafe_code = "forbid"` (`[ "${entry##*:}" = 0 ] || exempt=1`), and (b) the ratchet requires zero unsafe blocks. The file's own header (`:23`) says "No `unsafe` block … named `*_unsafe.rs` because §R makes it the [address perimeter]". Consistent; the word "unsafe" in it is prose. Consequence for the naming doctrine: "`ls *_unsafe.rs` enumerates the unsafe surface" now over-reports by one file, and CLAUDE.md's rule ("unsafe only in kf-linux-raw, kf-qemu and kf-cuda") omits both `kf-chan` (zero blocks) and `firmware/kf-gop` (78 counted blocks, a real fourth unsafe crate plus `port_unsafe.rs`/`check_unsafe.rs`).
- Also outside the three crates by gate exclusion: `crates/kayfabe-doorbell/` is excluded from the unsafe-surface and host-pointer gates and from gate A.

### G.7 Doc statements stale against master

1. `V3_SECURITY_AUDIT_PLAN.md` §4: "Every row is open" — false for S1-80 (hotplug half fixed, `56e7932e9`), S1-43 (strict hardwired), S1-84 (overflow-checks on), S1-21/S1-23 (flag deleted, always ON). The §4 rows for S1-80 ("open") and P1/P2 ("design in progress") are stale.
2. `2026-10-03-v3-stage1.md` S1-21, S1-23, S1-43: "default OFF", "fixed behind the flag; open on the default path" — `KF3_TSPACE`/`KF3_INCA_REFUSE` were deleted and hardwired on 2026-10-10 (OWNER_RULINGS §AB). No dated correction in the audit entries.
3. Same doc, S1-80 "Status: open" and §5 item 1; S1-84 "Status: open"; the header "Every finding is open unless its entry says otherwise".
4. `V3_SECURITY_MODEL.md` §9: "guest kernel … device lifetime on hotplug topologies (S1-80)" and "identity windows in every twin (S1-21)" as reasons rows do not hold; §8 "walker … S1-26", "identity windows currently do (S1-21)"; §7 rows are otherwise still accurate.
5. All stage-1 line citations are stale (kf3.c 927 → 2027 lines, ffi_unsafe.rs, chan.rs, mem.rs moved); the ratchet numbers in the §2.1 summary (75/75/44) are now 101/78/86.
6. S1-16 says kf3 "never calls the KVM … raw modules"; kf-qemu now uses `KvmVm::ioeventfd` (ci.yml ratchet note at `:1400-1410`).
7. `crates/kf-linux-raw/src/ioctltrace.rs:9` "complete by construction" (S1-22 check 4, never corrected).
8. CLAUDE.md "Unsafe code only in kf-linux-raw, kf-qemu and kf-cuda" vs AUDITED including `firmware/kf-gop:78` and `kf-chan:0`.
9. `chan.rs:9319` test name `the_heap_bound_refuses_only_when_strict` after strict became the only mode.

### G.8 Not reviewed in this part

- The unsafe code itself (parts A-F).
- Bodies of walker findings S1-61/62/63/65/72, S1-30/31/50/82/86 (status INFERRED from absence of a fixing commit or symbol).
- v3 analogues of the w301 findings (GREP pointers only).
- Whether `scripts/ci/clippy.sh` carries a cast/arithmetic ratchet (S1-84 check 3).
- OWNER_RULINGS and THE_CONSTRAINTS for accepted-risk rulings (not read in this part), so "OPEN" here does not mean "not owner-accepted".
