# V3 security audit, stage 2 (guest-reachable code, mapping, unsafe, resources, C device, privilege, supply chain)

**STATUS: LIVE, 2026-10-21.** Source audit of `origin/master` `e64b8e5f6` (merge of PR #64). It continues
`docs/audits/2026-10-03-v3-stage1.md` (stage 1, `12a526df`) and checks every stage-1 finding it could against
this tree (section 4). Read-only on product code; GPU-free; no hardware, no vast box, no trusted host. The only
repository changes are this document and five test files (section 7) that fail on `e64b8e5f6` on purpose, plus
one passing hostile-input harness. **No finding is critical.** Two are high.

Threat model (binding, from the audit brief): guest userspace and guest root are both hostile and control every
byte they can write (BAR0 writes, user rings, GSP queue messages, mailbox and shared memory, RM request payloads).
Host actions must be authored from unprivileged RM verbs, never forwarded from guest bytes; a guest-reachable
unbounded read, emit, loop or allocation is a security bug; `unsafe` lives only in `*_unsafe.rs` of `kf-linux-raw`,
`kf-qemu` and `kf-cuda`. A panic on a service thread aborts the VM (`crates/kf-qemu/src/failclosed.rs`), so a
guest-reachable panic is a stop of that VM, not of the host.

## 0. Method, evidence tags, severity

- **Method.** Five read-only lanes ran in parallel, each with its own question list (unsafe and sandbox; QEMU C
  device and display broker; privilege, flags and supply chain; `kf-rm`/`kf-gsp`/`kf-chip` decoders; `kf-mem`,
  `kf-trap`, the walker). The lead read the user-ring relay (`ptlock.rs`, `userstrand.rs`, `userrelay.rs`,
  `userread.rs`, `userpump.rs` arming code, `kf-chan` `handoff`, `worker`, `user_loop`, `user_scan`, `user_stream`,
  `user_ring`, `user_gate`, `ring.rs`), the vCPU entry paths in `device.rs` and `ffi_unsafe.rs`, `kf-disp`
  `pushbuf.rs`, ran clippy with the guest-parse lint set over every guest-facing crate (section 5), re-verified
  every high and the top medium findings against the code, and wrote the tests in section 7.
- **Tags.** CODE READ: the code was opened and the claim is a reading of it. MEASURED: a test or run in this lane
  produced it (the number is stated). INFERRED: reasoning from code plus a fact outside this tree.
- **Severity.** critical: guest-reachable memory corruption or code execution on the host, or cross-tenant memory
  reach. high: a stated security-model property fails for a guest actor with no host-side precondition beyond a
  documented, supported topology or configuration. medium: guest-reachable resource exhaustion, hang or privilege
  weakness with bounded blast radius (usually the guest's own VM), or a process gap that lets a defect through.
  low: latent, or reachable only by guest root against itself, or defence-in-depth. info: recorded so it is not
  rediscovered.
- **What was run.** `cargo clippy` with `arithmetic_side_effects`, `indexing_slicing`, `unwrap_used`,
  `expect_used`, `panic`, `unreachable` warned on `kf-chan`, `kf-disp`, `kf-trap`, `kf-rm`, `kf-gsp`, `kf-chip`,
  `kf-qemu`, `kf-host`, `kf-broker`, `kf-linux-raw`, `kf-mem`, `kf-core`, `kf-cuda`; the five EXPECTED RED tests
  and the hostile RPC harness of section 7. No product code was changed.

## 1. Summary

| severity | count | ids |
|---|---|---|
| critical | 0 | |
| high | 2 | SA-01, SA-02 |
| medium | 12 | SA-03 to SA-13, SA-15 |
| low | 13 | SA-14, SA-16 to SA-27 |
| info | 3 | SA-28 to SA-30 |

Top five, in the order to act on them:

1. **SA-01 (high)** Guest root can unplug the kf3 device on the topology the Windows production launcher uses
   (`-machine pc`, `bus=pci.0`); the exit path joins no thread and deletes no bottom half, so Rust threads keep
   using freed `Kf3State`. Stage-1 S1-80, still open. A one-line `dc->hotpluggable = false` closes the i440fx path.
2. **SA-02 (high)** The decision "this channel is the guest kernel's" rests partly on the client handle value being
   in `0xC1E00000..`, and the shipped open driver lets a guest userspace process choose its own client handle
   (the re-encode the code comment relies on is compiled out). A user channel so classed goes on the Translated
   route with the privileged T-space windows: guest userspace reaching guest-kernel and other-process memory
   through the GPU. The kayfabe code itself says that is the failure to avoid. Dynamic confirmation on a Linux
   guest is owed (UNCERTAIN U1).
3. **SA-03 (medium)** More than 64 rooted VA spaces (one per GPU-using guest process) makes an `ALL_PDB`
   invalidate unservable and leaves the trigger armed for ever: a guest-wide GPU hang started by unprivileged
   userspace. MEASURED: EXPECTED RED test.
4. **SA-05 (medium)** The worker loop must not sleep while any relay holds an obligation "no wake source
   announces"; a guest that keeps its ring full behind a stalled GPU keeps both workers at 100 percent CPU for
   as long as the stall lasts.
5. **SA-07 (medium)** Guest-driven `eprintln!` has no rate limit anywhere (S1-86), on at least 30 sites reachable
   from unprivileged guest userspace or guest root: host disk and stderr latency.

Hostile-input evidence worth stating up front: 3.2 million structured and mutated guest RPCs through the
production served chain (`kf_rm::served_policy` with object, memory, channel and display links) produced zero
panics in release mode with overflow checks on (3.2 million) and in debug builds (700 000) (section 7, MEASURED). The
guest-parse code in `kf-qemu` `chan.rs`/`mem.rs` and `kf-mem` is under a compiler-enforced no-panic, no-unchecked-
arithmetic deny set and shows none of the debt that the deny set would catch.

## 2. Findings

Format: id, severity, location, what the guest controls, impact, scenario, evidence, proposed fix, proposed test.

### SA-01 · high · kf3 device unplug frees state the Rust threads still use (stage-1 S1-80, open)

- **Where:** `qemu/hw/misc/kf3/kf3.c:1996-2011` (`kf3_class_init`: no `dc->hotpluggable = false`), `:1842-1897`
  (`kf3_dev_exit` calls `kf3_unrealize`, which only sets `stop` and signals eventfds), `:720-736`, `:770`, `:1661`
  (two `qemu_bh_new` bottom halves, no `qemu_bh_delete`, no mutex destroy, no free of `bar1_q`), `ffi_unsafe.rs:1135-1141`,
  `raw_unsafe.rs:145-147` ("the device is never freed"). Topology: `scripts/bench/windows/windows_broker.sh:249,254`
  puts `kf3-gpu` at `bus=pci.0,addr=0x6` of `-machine pc` (the other Windows launchers use `pc` as well).
- **Guest controls:** when the device is unplugged. On the i440fx `pc` machine the guest writes the ACPI PCI hotplug
  eject register (PCI_EJ, a port write only guest root can do from userspace, or the Windows "eject" path).
- **Impact:** QOM finalizes `Kf3State` while the drainer, worker and VA threads keep running and call back into the
  freed object (`qemu_mutex_lock(&s->bar1_q_lock); qemu_bh_schedule(s->bar1_bh)` on freed memory; ROM-device RAM
  stores). Host-process memory corruption at least; a crash of that guest's QEMU at best. R2.8 of the security model.
- **Scenario (no exploit):** guest root ejects slot 6; QEMU runs `kf3_dev_exit`; the VA thread, still alive, handles
  the next BAR1 overlay request and locks the freed mutex.
- **Evidence:** CODE READ for the C and for the launcher topology; INFERRED for QEMU's `acpi_pcihp` eject honouring
  `dc->hotpluggable` (QEMU source not in this tree). Whether a PCIe root-port slot power-off (q35) honours it is
  UNCERTAIN U2.
- **Fix:** set `dc->hotpluggable = false`; make exit safe anyway (unhook the Rust verbs, join the threads,
  `qemu_bh_delete` both bottom halves, drain `bar1_q`, NULL `s->h`, free); document `pcie-root-port,hotplug=off`;
  also call `kf3_unrealize` on the `fail:` path of realize (`kf3.c:1828-1840`, today the Rust half keeps running
  after a failed `device_add`).
- **Test:** qtest or manual run behind a root port that powers the slot off during an invalidate storm, under ASan;
  GPU-free source gate asserting `hotpluggable = false` appears in `kf3_class_init`.

### SA-02 · high · "kernel client" is decided from a client handle the guest chooses (feeds the privileged T-space)

- **Where:** `crates/kf-rm/src/chanlink.rs:2400-2402` (`is_rm_internal_client`: `h & 0xC1E00000 == 0xC1E00000`),
  `:2416-2420` (`kernel_channel`), `:1343`, `:1418`; consumers `crates/kf-qemu/src/tspace.rs:62-72`
  (`Privileged::of`, "the only key to the T-space's whole-RAM and whole-store windows"), `mem.rs:1722-1726`
  (`kernel_vas_for`), `twin.rs:60`, `chan.rs:716`.
- **Guest controls:** the client handle of its `NV01_ROOT` allocation. Guest userspace can do this with the ioctls
  every GPU process already uses.
- **Claim in the code:** the doc at `chanlink.rs:2394-2398` says a client that asks for a fixed handle "is
  RE-ENCODED onto the user base `0xC1D00000` (`ogkm-580: rs_server.c:3267-3271`), so no guest process can hold one in this
  range". Read at ogkm-580.159.04: that re-encode is inside `#if !(RS_COMPATABILITY_MODE)` (`ogkm-580: rs_server.c:3268`)
  and `ogkm-580: src/nvidia/Makefile:130` sets `CFLAGS += -DRS_COMPATABILITY_MODE=1`, so it is compiled out;
  `serverAllocClient` takes `hClient = pParams->hClient` (`ogkm-580: rs_server.c:612`; the reject guard at `:613` is out as
  well). The repository agrees with itself elsewhere: `rmgraph.rs:1963-1967` says "hRoot is caller-supplied in the
  shipped Linux driver". The mask is also wider than the range (`RS_CLIENT_HANDLE_MAX` is `0x100000`): it accepts
  `0xC1F00000` and `0xFFFFFFFF`.
- **Impact:** `chanlink.rs:1411-1418` explains the stake: "A user channel classed kernel would be Translated, and
  the Translated route REWRITES physical CE operands onto the store / guest-RAM windows (guest userspace reaching
  guest-kernel memory)". With the `Privileged` witness the T-space gives a Translated channel windows over all
  guest RAM and the store. Result: guest userspace reads and writes guest-kernel and other processes' memory by
  GPU copy. This is the in-guest isolation the value proposition rests on (owner rule A.9, R1.1). It is not a
  host breakout: the windows stop at guest RAM and the VM's store.
- **Scenario (no exploit):** a hostile guest process allocates its root client with handle `0xC1E00100`, builds a
  TSG and GPFIFO channel under it, and submits copy-engine methods with physical-mode operands. kayfabe classes the
  channel kernel, routes it Translated and rewrites those operands onto the whole-RAM window.
- **Why the stage-1 hardening does not stop it:** `kernel_channel` is `is_rm_internal_client(h) || stamp.is_kernel()`;
  the first term needs no stamp. The code already refuses to trust the pid sentinel because "the pid sentinel is
  reachable from guest userspace (`[measured p5bd]`)" (`chanlink.rs:1411-1414`); the handle range has the same
  property. The measured reason for the range test ("the PMA scrubber's client `0xc1e00006` was NOT marked kernel
  by the pid sentinel", `chanlink.rs:2394`) means removing it needs another discriminator for RM's own internal
  clients.
- **Evidence:** CODE READ of kayfabe and ogkm-580 (the lead re-verified `rs_server.c`, the Makefile and the
  consumers). The end-to-end behaviour on a Linux guest is INFERRED: UNCERTAIN U1 asks for the dynamic check.
  MEASURED: `is_rm_internal_client(0xC1F00000)` and `(0xFFFFFFFF)` return true (EXPECTED RED test).
- **Fix:** do not derive privilege from a handle range. Use only a fact the guest kernel stamps after clearing
  `internalFlags` (`ogkm-580: kernel_channel.c:220`, `:1058`, `:2805`), or identify RM's internal clients by the
  allocation path (they are allocated by the guest kernel RM before any user ioctl can run, and the first client
  with that handle is recorded as such); at minimum bound the predicate to the exact range and refuse a user root
  alloc whose handle falls in it.
- **Test:** `crates/kf-rm/tests/secaudit_internal_client_range.rs` (EXPECTED RED, range bound). The real test is
  a `ChannelPolicy` test: root alloc with `hClient = 0xC1E00100` and a user pid, then a channel alloc, asserting
  the emitted `ChannelAlloc.kernel_client == false`.

### SA-03 · medium · More than 64 rooted VA spaces leaves every ALL_PDB invalidate armed for ever (stage-1 S1-61)

- **Where:** `crates/kf-mem/src/vasmgr.rs:1323-1333` (`batch.walked.len() > MAX_SPACES_PER_WALK` → `refuse_batch`),
  `:1386-1396` (`refuse_batch` bumps `stats.unreconciled`, never completes the trigger); `MAX_SPACES_PER_WALK = 64`
  at `:68`.
- **Guest controls:** the number of VA spaces it roots: each guest process that opens the GPU roots one.
  The `ALL_PDB` invalidate itself is issued by the guest kernel RM.
- **Impact:** the guest RM polls the busy trigger while holding its RM locks (the run-223 hang class,
  named in `crates/kf-qemu/src/failclosed.rs:9`). Every GPU client in the guest hangs. Own VM only, but started by an
  unprivileged process, which violates R1.5.
- **Evidence:** MEASURED. `crates/kf-mem/tests/secaudit_all_pdb_over_64_spaces.rs` roots 70 spaces, sends the
  invalidate through the register port as a vCPU would, and fails: `submitted [], unreconciled=1, refusals=["70
  objects in one walk; the walk kernel carries 64"]`. How often a stock guest issues `ALL_PDB` is UNCERTAIN (S1-61
  asked for a census).
- **Fix:** chunk the batch into walks of at most 64 and clear the trigger after the last chunk (or walk the
  rooted set in rounds). **Test:** the file above, turned green.

### SA-04 · medium · Per-request work is linear or quadratic in total live RM state, on the register drainer

- **Where:** `crates/kf-rm/src/rmgraph.rs:1897-1923` (`free_subtree`: `while changed { for (&hkey,&href) in
  &self.handles ... }`, every non-root FREE scans all handles of all clients, at least twice), `:2004-2022`
  (`resolve_pending_dups`, on every Alloc and Dup), `:1486-1500` (`cache_targets`, a full resource scan on every
  Device alloc), `chanlink.rs:2267-2297` (`on_free`, full scans plus `vas_objects.retain(.. aliases.values().any
  ..)`), `barpde.rs:323-372` (`observe_free`, orphans times aliases). The drainer runs it:
  `kf-qemu/src/device.rs:4117-4119`.
- **Guest controls:** the number of live objects (cap `MAX_LIVE_HANDLES` = 2^18) and the order of its frees. The
  stock RM frees objects one at a time. Guest root can build a parent chain (the alloc arm does not require the
  parent to exist) so that one FREE needs about N passes over N entries.
- **Impact:** drainer stall of seconds to minutes: the drainer also serves BAR0 register effects, so the guest
  stalls itself, but the thread is shared with every other client in the VM. Violates "the drainer may never block".
- **Evidence:** CODE READ for all sites. MEASURED: `crates/kf-rm/tests/secaudit_graph_free_is_not_quadratic.rs`
  (debug build, one client, one device, N sibling VA spaces freed one by one): N = 3000 allocs 34 ms, frees 2.02 s;
  N = 6000 allocs 71 ms, frees 8.64 s (4.3 times for twice the objects). At the handle cap (2^18) this is hours.
- **Fix:** a parent-to-children index in `RmGraph`; indexed maps in `chanlink`/`barpde`; reverse indices for
  aliases and orphans. **Test:** that file, turned green.

### SA-05 · medium · The worker loop busy-spins at 100 percent on two threads while a guest keeps a relay's wait unannounced

- **Where:** `crates/kf-chan/src/worker.rs:237` (`user_loop::idle(relay.unannounced()) == Idle::Spin` → `continue`,
  no `epoll` timeout), `user_loop.rs:157-163` (`Obligation::Popped`: `source: None, can_miss: true`),
  `:247-249` (`idle`), `kf-qemu/src/userpump.rs:2893-2899` (`set_unannounced(Popped || guest_may_wait_for_pops)`),
  `:2931-2936` (`user_ring_unannounced` is true when any relay is, and when the `user_rings` map `try_lock` fails),
  `kf-chan/src/user_turn.rs:657-662`, `:896-975`. Two workers: `ffi_unsafe.rs:221-230`.
- **Guest controls:** the Windows USER-ring relay input: its GPFIFO fill level, an open packet at the end of a
  GP entry, and (by stalling its own GPU work with a never-released semaphore acquire or a long kernel) whether
  the PBDMA pops entries. A ring kept full while `fetched < issued` sets `guest_may_wait_for_pops`; a `Wait::Popped`
  (open packet, or no private slot left) is "carried": nothing announces it.
- **Impact:** both workers iterate at epoll timeout 0 until the state changes. A hung guest process costs two
  host cores, for the duration of the stall. A bare-metal stuck acquire costs no CPU. The owner removed every
  budget and timer on purpose ("design and test as if the timeout were infinite",
  `crates/kf-chan/tests/relay_fairness.rs`); the cost of that design is that the guest controls how long the spin
  lasts. Workers are the same threads that serve doorbells and completions, so a spinning worker also competes
  with them for its core.
- **Evidence:** CODE READ (lead). That a blocked GPU keeps `GP_GET` from advancing past the PBDMA prefetch depth is
  INFERRED (U3).
- **Fix:** keep correctness event-driven but add a CPU back-off that cannot change a decision: after K consecutive
  iterations with no state change and no ready fd, `epoll_wait` with a 1 ms timeout (a latency choice, not a
  correctness timer); or give `Popped` a wake source (the engine's GP_GET-advance interrupt does not exist, so a
  periodic marker at queue-full is the existing tool). Needs an owner ruling because it touches the "no timer"
  rule. **Test:** extend `relay_fairness.rs` with a relay whose `Popped` target is never satisfied and assert the
  worker reaches a blocking park within a bounded number of passes.

### SA-06 · medium · The number of live BAR1/BAR2 store views is uncapped (own-VM fd and VMA exhaustion)

- **Where:** `crates/kf-mem/src/cpuwin.rs:565-620` (`CpuWindow::map` has no placement cap); each placement arms a
  view (`kf-host/src/lib.rs:1562-1563`, one node fd per view), `scratch.rs:84-91` ("no cap below that is built").
- **Guest controls:** guest root (the guest kernel's BAR1/BAR2 page tables). A 256 MiB BAR1 allows 65 536
  alternating 4 KiB placements.
- **Impact:** about one fd and two VMAs per view against `RLIMIT_NOFILE` and `vm.max_map_count` (65 530): QEMU's
  own `mmap` and `open` start failing (VNC, QMP, eventfds). Other VMs are not reached; the kernel slab is charged
  to the cgroup.
- **Evidence:** CODE READ plus arithmetic; MEASURED: `crates/kf-mem/tests/secaudit_cpu_window_views_are_capped.rs`
  accepts 200 000 live views. Production rlimits are UNCERTAIN (U9).
- **Fix:** cap live views (for example 8192), refuse beyond by name, coalesce adjacent store offsets.
  **Test:** that file, turned green.

### SA-07 · medium · Guest-driven log lines are uncapped on most paths (stage-1 S1-86, open)

- **Where (re-read by the lead and the decoder lane):** `crates/kf-rm/src/inittables.rs:1572` (`refuse_named`),
  `:1899-1905` (`W343GSSCENSUS`: one line per bit-15 control with no row), `:1705`, `:1713`, `:1756`, `:1764`,
  `:2005`, `:2026`, `:2042`, `:2068`, `:2087`, `:2104` (`W349REFUSE`), `:2797`, `:2872` (up to 128 indices per
  line); `chanlink.rs:979`, `:1007`, `:1218-1252` (three lines per VASpace alloc); `kf-gsp/src/boot.rs:1874`,
  `:2210`, `:2409`, `:2518`; `barpde.rs:199`; `rmrpc/mod.rs:1847`; `guestsysinfo.rs:184-227`;
  `kf-qemu/src/device.rs:4108`, `:4123`; `kf-qemu/src/mem.rs:2453`, `:2494` (`GATE` lines per gated split), `chan.rs:7042`
  (one `BORN` line per channel), `:3117` (per act), `:7616`.
  Capped (so the pattern is known): `rmrpc/policy.rs:740-755` (24 lines), `kf-host` refusal paths (16, 32).
  No shared rate limiter exists in the workspace.
- **Guest controls:** the stock guest RM forwards non-privileged GSS-legacy controls for any client, so an
  unprivileged ioctl loop with distinct `cmd & 0x8000` ids produces one stderr line per call; guest root can do the
  same through the command queue.
- **Impact:** unbounded host log growth (disk, journald) and stderr-write latency on the drainer thread. Raised from
  stage 1's "minor" because it is reachable from unprivileged guest userspace and runs for the VM's whole life on a
  shared host.
- **Evidence:** CODE READ. **Fix:** one per-device, rate-limited logger that reports a swallowed count (the
  `log_fresh_refusals` pattern). **Test:** self-re-exec test counting stderr lines of 100 000 distinct unknown
  bit-15 controls through `InitTablePolicy::respond`; expect at most the cap plus a summary.

### SA-08 · medium · Chain-front links keep guest-keyed state without a cap (guest root; host memory growth)

- **Where:** `crates/kf-rm/src/chanlink.rs:1191-1192`, `:1201-1204`, `:1260-1261`, `:1267`, `:1005`, `:1527-1531`,
  `:2258` (`vas_objects`, `vas_parents`, `vas_under`, `tsgs`, `ctxshares`, `kernel_clients`, `vas_stated`,
  `vas_aliases`); `barpde.rs:298-299`, `:318`, `:347`. Only `devices` is capped (`chanlink.rs:1155`).
- **Guest controls:** `hObject`/`hVASpace` values in `GSP_RM_ALLOC` of class `0x90f1` or in
  `SET_PAGE_DIRECTORY`/`COPY_SERVER_RESERVED_PDES` controls with any `hClient`; no prior alloc is needed. These
  links sit in front of the object graph, so `MAX_LIVE_HANDLES` (2^18) does not bound them: an alloc the graph
  later refuses has already been recorded.
- **Impact:** about 50 to 100 bytes of host memory per ~100-byte RPC, for ever (plus the log lines of SA-07). Guest
  root against the host's memory, not only its own VM's budget.
- **Evidence:** CODE READ. **Fix:** cap each map at `MAX_LIVE_HANDLES` and refuse by name, or move the indices
  behind the graph's admission. **Test:** the decoder lane's `ChannelPolicy` test (65 543 page-directory
  statements for distinct `hVASpace`, assert `vas_objects.len() <= MAX_LIVE_HANDLES`), placed in `chanlink.rs`'s
  test module because the fields are private.

### SA-09 · medium · The PRAMIN view retire queue is unbounded, and its pool falls back to `openat` on a vCPU (stage-1 S1-45)

- **Where:** `crates/kf-qemu/src/mem.rs:641` (`mpsc::channel::<StoreView>()`), `:757-777`, `:779-788`, `:652-668`;
  pool `TRAP_NODES = 4` at `:624`, inline opens at `:763-770`.
- **Guest controls:** guest root's rate of BAR0 window register writes. Each write arms a store view in the trap
  (`arm_store_in_trap`: RM_MAP_MEMORY and an fd) and queues the old one for retirement.
- **Impact:** the backlog holds one `CharDevice` fd and host BAR1 aperture per entry (GPU-wide, outside
  `cardbudget`); when the 4-node pool is empty the vCPU itself calls `openat`.
- **Evidence:** CODE READ. **Fix:** `sync_channel(K)`; on full, release inline or refuse the move by name.
  **Test:** a fake `ViewOps` asserting queue depth never exceeds K; a cheap source gate also fails today.

### SA-10 · medium · Walker slots are retired for ever when more than 64 VA spaces are released between two walks

- **Where:** `crates/kf-mem/src/vasmgr.rs:1139-1148`, `kf-cuda/src/walk.rs:1189-1195`, `:1238-1245`,
  `cuda/walk/kf_walk.h:294` (`KF_MAX_RESET` = 64, drained only at `submit`).
- **Guest controls:** how many VA-space objects it frees before the next walk submit.
- **Impact:** the 65th and later releases are logged "the slot is retired" and dropped; `max_slots` is 128, so
  new spaces then get no slot and are refused ("no walker slot left"): the VM's GPU mappings die until device
  reset, for every guest process.
- **Evidence:** CODE READ; back-to-back retires without an intervening invalidate is INFERRED. **Fix:** keep
  refused slots in `table.released` and retry at the next `pump`/`submit`. **Test:** the decoder lane's fake-walker
  test (100 spaces released with a walker whose `reset` fails after 64 pending).

### SA-11 · medium · Diagnostic and negative-control flags can switch off isolation checks, with no common gate (policy)

- **Where:** `KF3_NEGCTL_CARVE=1` (`tspace.rs:84-97`, `device.rs:1085-1086`: carve-out refusal off, bound 0),
  `KF3_NEGCTL_TWIN_WINDOW=1` (`mem.rs:2915-2931`, re-opens the S1-21 window in every twin), `KF3_NEGCTL_NO_BAR1_DOORBELL`
  (`device.rs:1064-1070`), `KF3_NEGCTL_HEAP` (S1-43 bound count-only), `KF3_INJECT_*`, `KAYFABE_TRAP_*`,
  `KF_IOCTL_TRACE`, `KF3_NO_GUEST_VA_RESERVE` (`kf-host/channel.rs:677`). Only `KF3_DEBUG_FORCE_MIGRATABLE` requires
  `KF3_DEBUG=1` (`chanlink.rs:320-345`). `docs/design/V3_FLAG_INVENTORY.md` section 8 proposes `KF3_DIAGNOSTICS=1`;
  it is not implemented.
- **Guest controls:** nothing. The VMM environment is the deployer's. A guest cannot set any flag. What matters:
  a stray `export` or a copied bench launcher line in a shipped configuration weakens isolation silently (no
  banner), and the owner's ruling is to keep every measurement and control flag (2026-10-10).
- **Impact:** R1.1/R2.3 can be off without any visible sign. Local unprivileged users cannot reach the flags
  (no config file, no cwd read, no world-writable path read, no `set_var` outside tests).
- **Evidence:** CODE READ (the lead re-read `tspace.rs` and `mem.rs`). **Fix:** one flag registry read at realize
  that warns on any unknown `KF3_*`, prints a NON-PRODUCTION banner naming active NEGCTL/INJECT/DEBUG flags,
  and refuses those classes unless `KF3_DEBUG=1`; or compile them out of release builds with a cargo feature as
  `lifecycle-mutants` already does. **Test:** a realize test with a bogus `KF3_FOO` and a gate that every flag
  name in code is in the registry.

### SA-12 · medium · `KF3_GSS_NATIVE` forwards a guest-chosen control id and body verbatim to host RM (default off)

- **Where:** `crates/kf-rm/src/gssnative.rs:1-60`, `:356-427`; `kf-qemu/src/chan.rs:97-104`;
  `kf-host/src/lib.rs:1194-1201` (`raw_control_opaque`).
- **Guest controls:** `cmd` (any non-privileged GSS-legacy id on class `0x2080`, `(cmd & 0xC000) == 0x8000`) and up
  to 64 KiB of params; the host's reply bytes go back verbatim.
- **Filter:** privileged ids refused, a live guest subdevice required, 64 KiB cap, 20 000 forwards per boot
  (`new_boot` resets it). No host-side allowlist (S1-41).
- **Impact:** contradicts the rule that host actions are authored, not forwarded; a bad control id reaches GSP
  firmware on the shared GPU. `docs/design/V3_GSS_NATIVE.md` section 3.2 records that the "no CPU pointer in the
  params" claim is by construction in two sources and measured in none.
- **Evidence:** CODE READ. **Fix:** replace with a measured table {cmd → size, reply policy}, or keep default off
  and gate behind `KF3_DEBUG`; owner ruling before it can ship. **Test:** a sink asserting no forward for an id
  outside the table (the decoder lane's sketch).

### SA-13 · medium · Committed PTX runs inside the VMM's CUDA contexts and CI does not tie it to its source

- **Where:** `cuda/walk/kf_walk.ptx` (12 794 lines), `cuda/display/{kf_bl_pack,kf_color,kf_yuv}.ptx`
  (generated), `kf_scanout.ptx` and `kf_sum.ptx` (hand-written, no `.cu` source), `cuda/walk-v2-frozen/kf_walk.ptx`;
  `include_bytes!` at `crates/kf-cuda/src/display.rs:22`, `:42` and `walk.rs`.
- **Controls:** a code contributor, not the guest. The PTX is the walker and compose code whose bounds are the
  only barrier between guest page tables and memory the contexts can address (S1-05, S1-26).
- **Impact:** a PR that edits a `.ptx` without its `.cu` passes review as an opaque diff. Tests check only the
  `KfArgs` size and the Turing target; no CI step regenerates PTX. This also sits in tension with ruling §K ("no
  compiled binary in the repo"; the CI binary gate lists no `.ptx` suffix).
- **Evidence:** CODE READ plus `git ls-files | grep -c '\.ptx$'` = 7. **Fix:** a CI job installs the pinned NVRTC
  with hashes, regenerates and diffs; a manifest of source hash to PTX hash; the hand-written PTX gets the same
  interpreter-based bound test as the compose kernel (`crates/kf-disp/tests/compose_kernel.rs` already runs it
  host-side). **Test:** the regenerate-and-diff job.

### SA-14 · low · The armed MMU invalidate is rebuilt from the live latch cells (guest root against itself)

- **Where:** `crates/kf-trap/src/mmuinval.rs:236-241` (`armed_request`), used at `kf-qemu/src/device.rs:2180`.
- **Guest controls (guest root only):** a PDB_LO/HI latch write after arming the trigger changes which space the VA
  thread walks and clears over; a torn lo/hi mix is possible.
- **Impact:** stale translations for the armed space survive a completed invalidate. Guest userspace cannot write
  these registers, so this is in-guest only. The doc comment's soundness argument is a guest behaviour, not a host
  guarantee.
- **Evidence:** MEASURED: `crates/kf-trap/tests/secaudit_armed_request_is_a_snapshot.rs` fails (serves
  `0x2000000`, armed for `0x1000000`). **Fix:** snapshot `{seq, raw, lo, hi}` at arm time.

### SA-15 · medium (process) · CI and supply-chain hardening gaps

- **Where / what (CODE READ, `.github/workflows/ci.yml`, `scripts/ci/dependencies.py`, `.gitmodules`):**
  no `permissions:` block (default token scope); `dtolnay/rust-toolchain@master` (a branch) at lines 51, 2134,
  2194, 2330, `Swatinem/rust-cache@v2`, `actions/checkout@v4` not pinned to SHAs; `cargo build/test --workspace`
  without `--locked`; no `deny.toml`, `cargo-audit` or `cargo-vet` (`dependencies.py` is an architecture-isolation
  check, with two dev-only exceptions marked "owner decision pending": `kf-trap → loom`, `kf-disp → kf-broker,
  kf-linux-raw`); the four submodules (`ogkm`, `ogkm-580`, `linux`, `gvisor`) have no gitlink entries at HEAD
  (`git ls-files -s | grep -c '^160000'` = 0), so `git submodule update --init` pins nothing and the README's
  "a citation without a revision is not a citation" has no machine-checkable pin.
- **Positive:** no `pull_request_target`, no secrets, no self-hosted runners, no artifact upload; `Cargo.lock` has
  54 external entries, all crates.io with checksums, no git sources; the only product dependency is `libc`.
- **Fix:** `permissions: contents: read`, SHA pins with Dependabot, `--locked`, `cargo deny check`, restore
  gitlinks or drop `.gitmodules` and state revisions as text with a gate.

### SA-16 · low · The lexical unsafe gates are bypassable and the three unsafe-allowed crates have no compiler backstop

- **Where:** `ci.yml:770` (`grep -vE ':[0-9]+:[[:space:]]*(//|/\*|\*)'` drops any line starting `/*`), `:1771`
  (gate A accepts an empty `[lints]` plus `workspace = true` elsewhere), `:827` (host-pointer gate misses
  `ptr::from_ref(x) as usize`, `as_ptr() as usize`, `Box::into_raw`); `kf-qemu/src/lib.rs`, `kf-cuda/src/lib.rs`,
  `kf-linux-raw/src/lib.rs` have no `#[forbid(unsafe_code)]` on their safe modules (the lead's lane A probed two
  bypasses; the gate pipeline reports neither). The ratchet regex does not count `#[unsafe(no_mangle)]` (43) or
  `unsafe extern "C" {`.
- **Controls:** a code author. **Impact:** `ls *_unsafe.rs` may not enumerate the unsafe surface. **Fix:** a
  `tomllib` check of `[lints]`, per-module `#![forbid(unsafe_code)]` on every non-`_unsafe` module, a real lexer for
  comments, clippy `disallowed-methods` for pointer-producing calls. S1-01, S1-02, S1-11 remain open.

### SA-17 · low · A retired VA space with a live channel is leaked with its mappings

- **Where:** `crates/kf-qemu/src/mem.rs:3446-3458` (`retire_mirror`: "KEPT, never recycled"; the target is already
  removed from the walker, so it is never walked or unmapped again; no later sweep when `live` reaches 0).
- **Controls:** the order of the guest's frees (guest RM frees a subtree with one FREE). **Impact:** one host VA space,
  its batch objects (pinned guest-RAM descriptors) and live translations to guest pages the guest later reuses,
  per occurrence; the S1-25/S1-82 class. Reachability from a hostile guest is INFERRED (U4).
- **Fix:** record kept mirrors and run unmap, invalidate, `free_vaspace` when `live` hits 0, or at device reset.

### SA-18 · low · `RamMap::add/del` take a write lock on the BQL/vCPU path

- **Where:** `crates/kf-qemu/src/mem.rs:151-166`, called from `kf3_region_add/del` (`kf3.c:835-858`) under
  `memory_region_transaction_commit`. **Controls:** guest toggles of chipset RAM aliases (PAM/SMRAM) on its own vCPU.
  **Impact:** the vCPU waits for every reader's guard (readers hold it across the `admit_user_generation` closure,
  `userpump.rs:611`, and `MemoryListRam::try_access`); a nested read behind a queued writer deadlocks (U8). Breaks
  "nothing blocks on a vCPU". A poisoned lock silently ignores the add or delete. **Fix:** copy-on-write
  `Arc<Vec<_>>` swap.

### SA-19 · low · A full privileged-register ring poisons silently and permanently

- **Where:** `crates/kf-trap/src/trap.rs:214-216` (`Push::Dropped => Action::None`), `device.rs:1715`,
  `ring.rs:105-111`. **Controls:** guest root floods 4096 privileged writes while the drainer is behind.
  **Impact:** every later privileged write is dropped with no log, counter or guest-visible fault (the design says
  "by name" and "a guest-visible fault"); only device reset clears it. The vCPU never blocks. Own VM only.
  **Fix:** an `Action::Dropped`, a counter, one log line, the promised fault.

### SA-20 · low · Held GSP replies are unbounded, and a guest-declared element size amplifies host reads

- **Where:** `crates/kf-gsp/src/boot.rs:2523` (`held.push`, no cap; `release_held` does `held.remove(0)`, O(n),
  `:2783`; release stops at the first unresolved deferred act, `:2771-2776`); `ring.rs:431-488` accepts any
  `msg_size` up to the region size (about 16 MiB) while `element_size_max` is 64 KiB, and `drain_commands`
  (`boot.rs:2138-2140`, `:2255`) allocates and reads `vec![0u8; element_size]` per doorbell.
- **Controls:** guest root. **Impact:** host memory growth and roughly 1000 times read amplification on the drainer.
  **Fix:** cap `held`, use a `VecDeque`; refuse `msg_size > element_size_max` at `bind`.

### SA-21 · low · The no-panic deny set covers only part of the guest-facing code; one service thread is not fail-closed

- **Where:** `#![deny(clippy::arithmetic_side_effects, indexing_slicing, unwrap_used, expect_used, panic,
  unreachable)]` is on `kf-mem` (whole crate except `apply.rs:20`), `kf-qemu/src/chan.rs:42` (children
  `userpump`, `userquota`, `userrelay`, `usershadow`, `userstrand`) and `kf-qemu/src/mem.rs:45` (children
  `userread`, `userdram`). Not covered: `kf-rm`, `kf-chan`, `kf-gsp`, `kf-chip`, `kf-trap`, `kf-abi` decoders,
  `kf-core`, `kf-host`, `kf-disp`, `kf-broker`, and in `kf-qemu` `device.rs`, `display.rs`, `userva.rs`,
  `userstore.rs`, `userbirth.rs`, `reset.rs`, `tspace.rs`, `twin.rs`, `scrub.rs`, `defapi.rs`. `failclosed.rs:27-44`
  omits `kf3-scanline` (`ffi_unsafe.rs:254`).
- **Mitigation present:** `[profile.release] overflow-checks = true` (`Cargo.toml:104-105`), so an overflow panics
  (stage-1 S1-84 FIXED); service-thread panics abort the VM.
- **MEASURED (clippy, section 5):** the uncovered crates carry 100 to 500 sites each of unchecked arithmetic or
  indexing, but the production sites with `unwrap`/`expect`/`panic!`/`unreachable!` outside `kf-abi` and `kf-util`
  are about 35 and all assert construction invariants (for example `rmgraph.rs:1818`, `:2016` re-derive an id just
  returned by `resource_of`; `engine.rs:1046` unwraps a table allocated at construction for the Window kind).
  The clippy debt file (`scripts/ci/clippy-debt.json`, 532 entries) holds none of the guest-parse lints: they are
  style (`missing_docs` 158, `manual_is_multiple_of` 21, `dead_code` 20, `type_complexity`, `items_after_test_module`,
  `collapsible_if` 17 each) and 3 `missing_transmute_annotations` on `dlsym` casts. None is guest-reachable.
- **Fix:** extend the deny block to the decoder crates module by module; add `kf3-scanline` to `is_service_thread`.

### SA-22 · low · Per-VM bounds still missing for shared host-GPU resources (stage-1 S1-42, S1-44, S1-82)

- **NVENC sessions:** `kf-qemu/src/chan.rs:5775-5825` (`encoder_session`) acquires a GPU-wide host session per
  guest request with no per-VM cap (re-verified; open). **Timeslice:** `kf-host/src/channel.rs:1834` passes the
  guest's `u64` microseconds unbounded to the twin's host TSG (`chan.rs:3917-3976`) (open). **Context buffers:**
  the per-runlist `channel-budget` now caps twin births (`rmfacts.rs`, `chan.rs:2316`, `6949-6955`), partially
  closing S1-25/S1-82; RM-owned context-buffer VRAM is still outside `fb-mb`. **Impact:** other tenants on the
  same GPU. **Fix:** per-VM clamps by name. S1-44 is inside property P (a local process can do the same).

### SA-23 · low · Display broker and C device smaller items

- **Broker flood:** `kf-broker/src/conn.rs:2174-2195`, `:685-692` (unthrottled `say!` on `EV_GRAB`/`EV_FOCUS`) —
  a stalled stderr blocks QEMU's main loop; only `READ_BATCH` (64) bounds a wakeup. **Socket race:**
  `broker/nvkvm_broker.c:3301-3308`, `:3332`, `:3352` (`lstat`, `unlink`, `bind`, then `chmod`/`chown` follow
  symlinks if the directory is not sticky/private; the default mode 0600 and umask are set before `bind`).
  **`SUDO_UID`** parsed with an unchecked `strtoul` (`:3908-3911`). **Realize:** a failure after `kf3_realize`
  leaves the Rust half running (`kf3.c:1828-1840`, see SA-01). **Wrapping adds** in the overlay bounds check
  (`kf3.c:674-677`) are unreachable today (`bar1db.rs:110-123` rejects first). **Migration:** no `vmsd` and no
  blocker (`kf3.c:1996-2011`); host-triggered only. **Reset:** a guest reset makes the exit phase wait up to 2 s
  on the main loop (`kf3.c:1926-1934`, `reset.rs:247`), bounded.
- **Checked and sound:** the broker wire parser (fixed 40-byte read, one `SCM_RIGHTS` fd per command, ATTACH sizes
  validated in 64 bits against the real fd size, memfd with `F_SEAL_SHRINK`, cursor image `pread` into a static
  buffer, per-wakeup work budgets, command-rate cap, `SO_PEERCRED` default-deny with mode 0600, privilege drop,
  `PR_SET_DUMPABLE(0)`, `NO_NEW_PRIVS`); the VMM side checks the peer uid before reading a byte against uid 0, the
  euid at each connect and `display-broker-uid`, drops a received fd, and decodes the 24-byte event totally.
  EDID is authored, never parsed. All `MemoryRegionOps` use `valid`/`impl` 1 to 8 bytes with in-range offsets.

### SA-24 · low · Diagnostic file writes and a shell spawn

- `KF3_USER_RING_TRACE=<dir>` (`userpump.rs:150-165`, writes `:1605`, `:1636`, `:1847`) and `KF3_DEBUG_CURSOR=<dir>`
  (`display.rs:4196-4214`, `:4909-4913`) use `std::fs::write` with predictable names, default mode, no `O_EXCL`,
  following symlinks; guest-chosen content. `KAYFABE_TRAP_FATAL*` (`kf-util/src/trapwitness.rs:715-731`) runs
  `sh -c "head -c 1; kill -STOP <pid>"` through `PATH` from a possibly-root VMM; an `execve` regains capabilities.
  Deployer-set flags only; fix with `create_new`, `O_NOFOLLOW`, 0600 on a checked directory fd, and absolute paths.

### SA-25 · low · Kayfabe's CUDA contexts and the compose kernel (stage-1 S1-05, partial)

- Bring-up still does not query `CU_DEVICE_ATTRIBUTE_PAGEABLE_MEMORY_ACCESS` (no hit for `pageable` in
  `crates/kf-cuda/src`); the owner narrowed S1-05 by ruling §Q. The compose kernel's bound is now exercised by a
  host-run PTX interpreter (`kf-disp/tests/compose_kernel.rs`), which closes check (b). Display geometry is bounded
  in Rust (`FRAME_MAX`, `MAX_PIXELS`, cursor sizes allow-listed to 32/64/128/256, `decode_groups` 4 KiB with a
  jump budget). Still open: check (a), and `Kf3Frame` has no length with a byte-versus-pixel stride comparison in
  `kf3.c:1063-1064` (S1-03/S1-13; unreachable today, `kf3.c` is not compiled in CI).

### SA-26 · low · Safe-API unsoundness, stale justifications and small hygiene in the unsafe crates

- `kf-cuda/src/driver_unsafe.rs:1629`, `:1660` (`view_bytes<T: Copy>`, `zeroed<T: Copy>` safe and generic, S1-09);
  raw `CUdeviceptr` APIs public (`:707-709`, `:817-819`, `:836-837`, S1-04); `usize::try_from(fd).unwrap_or(0)` at
  `:1772` (S1-10); `chardev_unsafe.rs:610-616` accepts a request with declared size 0 (legacy ioctls write through
  the pointer from safe code), `Indirect::nested` does not tie count fields to buffer lengths (no caller);
  `RawRegion`/`BackendFd` are `Copy` with no lifetime (`raw_unsafe.rs:5`, `:100`; `place_ram` copies the fd out of
  a read guard, S1-07); 25 safe `pub extern "C" fn` take a raw handle (S1-08); `window_unsafe.rs:452-453` cites a
  compare that does not exist; `mapping_unsafe.rs:1022-1031` justifies `Sync` with "no bulk accessor" though
  `copy_out` exists; `kf-trap/src/memmap.rs:403` is a `debug_assert!` over a "refuse to install" invariant;
  `host_fd_unsafe.rs:125` creates the display-frame memfds without `MFD_CLOEXEC` (no `kf-*` code spawns children).
  No guest-reachable memory-safety hole was found in any `*_unsafe.rs`.

### SA-27 · low (owner decision) · Repository hygiene of a repo that is already public

- No API tokens, private keys or product keys in the tracked tree (grep over the tree). Items for the owner:
  the commit `f99caf2b` held a throwaway Windows guest password (scrubbed in tree, `V3_NOT_IN_GIT.md:38-42`; remains
  in history; rotate); the owner's personal mail address is in 3 in-tree patch files and 418 commit authors;
  about 10 public IPv4 addresses of rented boxes with SSH ports in docs and about 60 `traces/**` lines (ephemeral);
  172.22.1.20 appears in 58 lines of 40+ files (no 172.22.0.7); 192.168.77.x in 111; 130 distinct GPU UUIDs in 203
  files; `traces/20261021_131847.jpg` carries EXIF (phone model, image id; no GPS) and shows a room;
  `traces/` is 456 MB in 13.6k files (GSP bring-up recordings with driver strings and a GPU UUID, real page-table
  corpora, raw guest pushbuffers, 602 PNGs of Windows 11 Enterprise evaluation and third-party apps, compressed
  archives not inspected); four ELF executables are tracked under `archive/nvkvm` (the CI binary gate skips
  `archive/`); `.deadman/watch.sh` and `beat.log` are tracked though `.gitignore` lists them;
  `docs/design/V3_NOT_IN_GIT.md` names closed-driver RE tooling, which sits uneasily with the provenance policy.
  Licence: `LICENSE` exception 2 says the gVisor-derived `kf-abi/src/capability.rs` is Apache-only, so a QEMU build
  linking `kf-abi` is not GPL-distributable until re-derived.

### SA-28 · info · No sandbox ships with v3 (by ruling)

- `sandbox_unsafe.rs` exists only in the frozen v2 crate (`crates/kayfabe-linux-raw/src/sandbox_unsafe.rs`, 1261
  lines, namespaces plus `pivot_root` plus capability drop, "Not seccomp", no TSYNC, per-thread capset); no `kf-*`
  crate depends on a `kayfabe-*` crate (`dependencies.py`: `V3_ISOLATION crates=22 violations=0`). Confinement is
  the deployer's (ruling §P). The in-process measure is `kf-cuda/src/posture.rs`: it clears `CAP_SYS_ADMIN` from the
  effective set of every CUDA thread before the first CUDA call and fails closed; permitted and inheritable sets are
  untouched (`capability.rs`, `kf-host/src/birth.rs`), so it is a bracket, not a boundary. Under a root VMM the other
  kf-host RM calls still run with `CAP_SYS_ADMIN` (S1-22 remainder). Do not reuse the v2 sandbox as a boundary
  without a TSYNC seccomp allowlist. Provisioning scripts (`scripts/bench/*`, rented boxes only) download and run
  installers without checksums (`provision_host_driver.sh:40-47`, `curl | sh` for rustup in `provision_bare_metal.sh:32`,
  `provision_box.sh:201`), bind guest SSH forwards on all interfaces, and use `StrictHostKeyChecking=no`; there is
  no end-user install script yet.

### SA-29 · info · Decoder and transport observations with no defect found

- Every guest-bytes decoder read is bounded (`checked_add`, `.get()`, size gates, named refusals): `kf-gsp`
  `ram.rs`, `ring.rs`, `element.rs`, `large.rs`, `rpc.rs`; `kf-rm` `rmrpc/{policy,reasm}.rs`, `gssnative.rs`,
  `pmsr.rs`; `kf-chan` `user_scan.rs`, `user_stream.rs`, `user_ring.rs`, `ring.rs` (segment cap 256 KiB, copy
  once); `kf-disp` `pushbuf.rs` (4 KiB, jump budget, no overflow at `at + 4 + count * 4`). `kf-qemu` `userread.rs`
  bounds every span (8 MiB ring, 917 504-byte snapshot, piece caps, row/RAM guards re-checked at admission).
  Guest tag forgery in the user-gate (`MARK_TAG`) only lets a guest read its own late bytes early (ruling
  2026-10-13). `Handoff::register` is idempotent per relay and bounded by the relay count. No double fetch was
  found in `kf-gsp` (each reader copies once and decides on the copy) or in the `kf-qemu` user readers.
- `large::Assembler::step` clones the open message per continuation (bounded: 65 fragments, about 1 MiB, so 65 MiB
  per message) and does not check sequence continuity or duplicates (`large.rs:159`).
- Places that answer NV_OK without the work are documented: `pmsr.rs:132-150` (scheduling toggle, FBSR_INIT),
  default-off probes echo OK, `chanlink.rs:1244-1252` logs `VAS-DECL REFUSED` and proceeds.

### SA-30 · info · Mapping chain: no path out found

- Guest PTE → walker report → ledger → `apply_entry` → RM map call: the vidmem bound `at + len <= store` with
  `checked_add` (`ledger.rs:87-113`), peer and unknown apertures refused (`ledger.rs:112`, walker `kf_walk.cu:1731-1738`),
  sysmem leaves need one registered RAM block to cover the whole range (`RamMap::block_for`, `mem.rs:176-186`;
  registered RAM sections only, `kf3_guest_ram.h:9-12`), carve-out leaves refused and straddlers clipped
  (`apply.rs:1154-1197`, wired on, `device.rs:1105-1107`), `prepare_row` checked adds (`apply.rs:1141-1152`),
  stitched maps inside a kernel-chosen `PROT_NONE` reservation (`mapping_unsafe.rs:528-618`), no `PROT_EXEC` and
  `MAP_FIXED` only inside owned ranges. Page tables are read once by the walker (shared memory) and never
  re-read by the CPU. S1-21/S1-23/S1-43 are fixed in code (section 4). See SA-02 for how the privileged T-space
  key can still be obtained by a guest userspace process.

## 3. UNCERTAIN (areas where secure or not could not be decided; each question is self-contained)

| id | area | question |
|---|---|---|
| U1 | SA-02 | On a Linux guest, does `NV_ESC_RM_ALLOC` of `NV01_ROOT` with a user-chosen `hRoot` of `0xC1E00100` return that handle, and does a channel under it reach the Translated route with a `Privileged` witness (`ChannelAlloc.kernel_client == true`, `Privileged::of(true, false)`)? Is `NV0000_ALLOC_PARAMETERS.processID` (the sentinel that selects `ClientKind::Kernel`, `rmrpc/mod.rs:1362`) overwritten by the guest kernel or forgeable? What identifies RM's own internal clients if the range cannot (the PMA scrubber's `0xc1e00006` has no stamp)? |
| U2 | SA-01 | Does QEMU 10.2.4 honour `dc->hotpluggable` in `acpi_pcihp_eject_slot` (i440fx) and in `pcie_cap_slot_do_unplug` (q35 root-port slot power-off)? Is a Windows "safely remove" of the device possible on the production topology? |
| U3 | SA-05 | Does the PBDMA stop advancing `GP_GET` once its prefetch depth is full behind a blocked semaphore acquire or a long kernel (so a `Wait::Popped` is not satisfied for as long as the stall)? What is the depth? |
| U4 | SA-17 | Can a guest order its RPC frees so a VA space is retired while a channel of it is live (`live > 0`), and does anything free the kept mirror later? |
| U5 | walker | Are the VER3 PTE bit positions (PRIVILEGE, READ_ONLY, ATOMIC_DISABLE, `kf_walk.cu:1500-1510`) right on real Hopper/Blackwell? The sketch "has never run"; a wrong read-only decode maps read-only guest pages writable on the host GPU (S1-60 is the related open item). |
| U6 | `kf-mem` | Prove that no interleaving of UNMAP, MAP and kept intervals in `keep_only_what_stays_exact` (`apply.rs:576-700`) and the batch reservations leaves a host mapping live after the guest changed the page (not read). |
| U7 | `kf-mem` | Is there a hard ceiling on host RM mapping nodes per VM? Per-refresh budgets exist (`REFRESH_PLACEMENT_BUDGET = 1<<21`) but `OwnMaps.by_va` (`batch.rs:646`) has no global cap across refreshes; `runs_per_pdb = 1<<20` times 128 slots. |
| U8 | SA-18 | Does anything inside `admit_user_generation`'s closure or `MemoryListRam::try_access` call `blocks.read()` again (nested read behind a queued writer deadlocks)? Can QEMU free a RAM block's mapping while Rust holds a `Copy`'d `RawRegion` (memory hot-unplug, S1-07)? |
| U9 | SA-06 | What are production `RLIMIT_NOFILE`, `vm.max_map_count` and the cgroup limits, and does the launcher raise them? |
| U10 | `kf-qemu` | Bounds of guest values in `ChanStatement::{Timeslice, CtxswPreemption, Debugger, ZcullBind, PerfCudaLimit}` inside `chan.rs`, and of `UPDATE_BAR_PDE`'s `entry`/`level_shift` (`barpde.rs:84-104`, bounds in `kf-mem`) and of the `GuestRam::read` adapter for straddling or overflowing GPAs. |
| U11 | S1-20 | Do USER host channels refuse physical-mode and privileged work on every family and both privilege arms (T-PHYS-CE)? Needs hardware; not re-verified. Does the Windows Passthrough twin VA space hold only this VM's memory (`windows_user_work` classifies on guest-declared strings and `ProcessID`)? |
| U12 | broker | `nb_session_wl.c` (5796 lines) and `nb_session_x11.c` (2777) were read for bounds only: any reachable `abort()` or protocol error on VMM-chosen values in the Wayland dma-buf probe state machine or the X11 DRI3 import? |
| U13 | SA-13 | Is each committed `.ptx` equivalent to its `.cu` under the pinned NVRTC, and are the two hand-written PTX files (`kf_scanout.ptx`, `kf_sum.ptx`) bounded in-kernel? |
| U14 | S1-90 | In QEMU 10.2.4 does `memory_region_enable_lockless_io` set `disable_reentrancy_guard`? If not, a concurrent second-vCPU BAR0 write returns `MEMTX_ACCESS_ERROR` and is dropped. |
| U15 | privilege | Does every libcuda entry point that can create a channel pass through the four gated functions (`open_soname`, `init`, `ctx_create`, `ctx_set_current`)? `dlsym` symbols were not enumerated. Is a root VMM the supported release posture, and what is the minimal device and permission list? |
| U16 | hygiene | Contents of the compressed traces (`*.tgz`, `*.xz`, `*.zst`, `*.gz`), the 2 508 `application/octet-stream` files and the PNGs; whether VBIOS bytes sit in `.rec` MMIO records; whether the history sweep (password, personal data) is scheduled; whether the missing submodule gitlinks were dropped intentionally. |
| U17 | `kf-abi` | Hand-written decoders not read (`defapi::decode_registration`, `decode_bundle`, `eventnotify`, `faultbuffer`, `fmbpromote`, `regaccessmap`, `submit.rs`); `matrix::transcode` cost on large layouts (looks about O(F^2)); `ReselectAtFn1` (`kf-rm/src/lib.rs:329-374`) rebuilds the chain and drops the graph and `ChannelPolicy` maps at a guest fn-1 naming another version (a free reset of SA-08's maps, and are host twins keyed independently?). |
| U18 | panic | Does a panic across an `extern "C"` entry point abort on the pinned 1.99 toolchain as assumed (vCPU thread, `kf3_bar0_write`)? Not exercised. |

## 4. Stage-1 findings: fixed or open on this master

| id | status | evidence |
|---|---|---|
| S1-01 | open (a new variant: any line starting `/*` is dropped) | `ci.yml:770`; SA-16 |
| S1-02 | open | `ci.yml:827`; `clippy.toml` holds only `large-error-threshold` |
| S1-03 | open, latent | `Kf3Frame` has no length; `kf3.c:1063-1064` |
| S1-04 | open | raw `CUdeviceptr` safe APIs, SA-26 |
| S1-05 | partial | check (b) by interpreter test; no `PAGEABLE_MEMORY_ACCESS` query (SA-25) |
| S1-06 | open | `kf-linux-raw/tests/unsafe_naming.rs:41` still lists `kayfabe-*` |
| S1-07 | open | `RawRegion` `Copy` (`raw_unsafe.rs:5`) |
| S1-08, S1-09, S1-10, S1-11 | open | SA-26, SA-16 |
| S1-12 | not re-checked | |
| S1-13 | open | `kf3.c` not compiled in CI |
| S1-14 | open | `libc = "0.2"`, `build_kf3.sh:24` without `--locked` |
| S1-15 | open | `kf-host/lib.rs:1667` prints the cookie on a refused release |
| S1-16 | not re-checked | |
| S1-20 | not verifiable GPU-free | `birth_census.sh` exists; needs hardware (U11) |
| S1-21 | fixed in code, hardware gate not re-run | `mem.rs:3126-3150` `tmode_twin`; windows only via `negctl_twin_window()` (SA-11) |
| S1-22 | partial | libcuda bracket and births done; other kf-host RM calls on QEMU's threads still run with the VMM's capabilities (SA-28) |
| S1-23 | fixed in code, but see SA-02 | `chan.rs:6650` T-space hardwired; `Privileged` witness at `tspace.rs:62-72` |
| S1-24 | open; 3.9 M hostile RPCs found no panic | section 7; decoders are reachable from unprivileged guest userspace |
| S1-25 | partial | per-runlist `channel-budget` caps twin births; SA-06, SA-08, SA-09, SA-22 remain |
| S1-26 | doc, not re-checked | |
| S1-27 | open as a gate; no instance found | `kf-gsp` readers copy once; `kf-qemu` user readers snapshot once |
| S1-28 | partial | `dependencies.py` isolation only; SA-15 |
| S1-29 | corrected in docs | |
| S1-30, S1-46 | open | `kf3.c:379-404` backs the ROM device with the live host usermode window |
| S1-31 | not verifiable GPU-free | |
| S1-32 | open | no `pci_device_iommu_address_space` refusal in `kf3.c` |
| S1-33 | open (stage 2 broker) | peer-uid check present; no packet-rate cap on the VMM side (SA-23) |
| S1-34 | doc | |
| S1-40 | open | `chardev_unsafe.rs:368-372`; the one funnel sets `params_size` correctly, nothing enforces it |
| S1-41 | open, wider | `raw_alloc`, `raw_alloc_nested`, `raw_control` still `pub`; `raw_control_opaque` added (SA-12) |
| S1-42 | open | `chan.rs:5775-5825` |
| S1-43 | fixed | `tspace.rs:50` `INCA_STRICT = true`; `chan.rs:6683-6707`; `bar0.rs:434-441` |
| S1-44 | open | `kf-host/channel.rs:1834` |
| S1-45 | open | `mem.rs:641` (SA-09) |
| S1-47 to S1-50 | not re-checked | |
| S1-60 | open | `kf_walk.cu:411` (`table_version != VER2` returns false), `walk.rs:754-757` "KNOWN GAP" |
| S1-61 | open | SA-03; `kf_walk.cu:1952,2161` shared `entries_visited` |
| S1-62, S1-63 | open | `kf_walk.cu:2281-2296`; no `FRONTIER_CAP` test |
| S1-64 | not re-checked | |
| S1-65 | open | no CPU walk model or hostile-table fuzz in `kf-cuda`/`kf-mem` |
| S1-66 to S1-72 | not re-checked | |
| S1-80 | open | SA-01 |
| S1-81 | open | `is_overdue` (`shadow.rs:207`) still has no non-test caller |
| S1-82 | partial | `channel-budget` caps births; context-buffer VRAM unaccounted |
| S1-83 | open | no `cargo-fuzz` or proptest harness for any v3 decoder; the harness of section 7 is the first v3 hostile corpus |
| S1-84 | fixed | `Cargo.toml:104-105` `overflow-checks = true`; `failclosed.rs` |
| S1-85 | not re-checked | |
| S1-86 | open | SA-07 |
| S1-87 to S1-89 | not re-checked | |
| S1-90 | open | no reentrancy-guard assert in `kf3.c` (U14) |
| S1-91 | not re-checked | |

## 5. Coverage

Lines are of non-test hand-written source unless stated. "Fully" means read end to end; "ranges" lists what was
read; "grep" means only searched.

| area | size | depth |
|---|---|---|
| `kf-chan`: `handoff`, `worker`, `user_loop` (1-260), `user_scan`, `user_stream` (230-420), `user_ring`, `user_gate` (90-200), `user_storage` (1-140), `ring.rs` (1-120, 395-467), `user_turn` (640-670, 890-975) | 46.8k | ranges; `tmode`, `translated`, `host`, `dbfast`, `user_lifecycle`, `user_lazy`, `ptnsi` by clippy site list only |
| `kf-qemu`: `ptlock`, `userstrand`, `userrelay` (1-200), `userread` (1-260), `failclosed`, `ffi_unsafe` (all), `raw_unsafe` (non-test), `device.rs` (1343-1800, vCPU paths), `userpump.rs` (2040-2090, 2860-3100, 3240-3275), `userstore.rs` (215-370), `mem.rs` (3415-3490, 2400-2460), `chan.rs` (2015-2125, 5775-5825, 3917-3945) | 50.8k | ranges; `display.rs` (2560-2715, 4880-4980, 5280-5295) |
| `kf-rm`, `kf-gsp`, `kf-chip`, `kf-core`, `kf-arch`, `kf-abi` hand-written | about 100k | about 11k lines read closely, all grep-swept (decoder lane); `hostquery`, `hostfacts`, `authored`, `staticinfo`, `sweep`, `sw_runlist_*`, `falcon_gsp`, `hwref`, `classes`, `usermode`, about 99k generated `kf-abi` lines skipped |
| `kf-mem` (`apply`, `ledger`, `vasmgr`, `cpuwin`, `batch`), `kf-trap` (`trap`, `ring`, `pramin`, `shadow`, `mmuinval`, `memmap`, `token`), walker `kf_walk.cu` (about 800 of 2716), `kf-cuda/walk.rs` (about 600) | 19.3k, 8.2k, 2.7k | ranges; `kf_walk.ptx` and `sim` skipped |
| `kf-linux-raw` `*_unsafe.rs` (mapping, window, host_fd, chardev, unixsock, drm, capability, epoll, affinity, sysconf, signal, vcpu, kvm), `kf-cuda/driver_unsafe.rs` (about 1100 of 1881), `posture.rs` | 16.6k, 12.5k | mostly full; `kvm`/`vcpu` are dead for the product |
| `qemu/hw/misc/kf3/` (`kf3.c`, `kf3.h`, `kf3_gop.h`, `kf3_guest_ram.h`) | 2.4k | fully |
| `broker/` C: `nvkvm_broker.c` (about 75 percent), `nb_cursor.c` (fully), `nb_frame.c`, `nb_session_x11.c`, `nb_session_wl.c` (ranges), `nvkvm_broker_proto.h` (fully) | 17k | partial; test tools skipped |
| `kf-broker` (`lib`, `link`, `input`, `slots` fully; `wire` 520 of 628; `conn` about 2150 of 3246; `cursor` about 330 of 651; `console`, `gpucopy` not read) | 9.5k | partial |
| `kf-disp`: `pushbuf`, `ports` fully; `edid` (330), `scanout` (ranges), `engine`, `model` ranges; `caps`, `color`, `tick`, `lockstep`, `inst`, `layout`, `vramslot` skipped | 28k | partial |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `scripts/ci/dependencies.py`, `clippy-debt.json` (all 532 entries), `.gitmodules`, `LICENSE`, `.github/workflows/ci.yml` (2341 lines: grep plus about 300 read) | | fully or as stated |
| env reads in `crates/kf-*`, `kf3.c`, `broker/` (all `env::var`, `getenv`), `V3_FLAG_INVENTORY.md` (1-140) | about 100 names | full grep, about 40 opened |
| hygiene scans over the tracked tree; `traces/` by size, name and magic only | | grep; no history scan |
| not covered | | git history, `archive/` content, compressed traces, `docs/` beyond cited lines, `tools/windows-capture` beyond its ACL, `firmware/kf-gop` (guest-side), `kf-census`, `kf-crec`, `kf-harness`, `kf-oprom`, `kf-gop-image`, `kf-util` beyond the trap witness, `kayfabe-*` frozen tree |

Clippy census (MEASURED, `-W` the guest-parse lints, lib targets, distinct sites of unchecked arithmetic, indexing
or slicing): `kf-abi` 1005 (mostly generated tables), `kf-rm` 512, `kf-disp` 446, `kf-chan` 359, `kf-qemu` 306,
`kf-broker` 206, `kf-cuda` 213, `kf-gsp` 122, `kf-oprom` 122, `kf-trap` 94, `kf-host` 53, `kf-linux-raw` 49,
`kf-chip` 47, `kf-core` 30, `kf-util` 141; `kf-mem` zero, and `kf-qemu` `chan.rs`/`mem.rs` contribute zero, because
they are denied. In `kf-qemu` the 306 sites are `display.rs` 152, `device.rs` 65, `prof.rs` 20, `userstore.rs` 14,
`bar0trace.rs` 10, `readtrace.rs` 8, `scrub.rs` 8, `twin.rs` 4 and a few others (mostly counters and bounded
geometry; the reachable ones are checked by an earlier bound). With overflow checks on, each is a VM stop at worst,
never a wrap. `unwrap`/`expect`/`panic!`/`unreachable!` in non-test code outside `kf-abi`/`kf-util`: about 35, all
reviewed; none reachable from guest data.

## 6. Environment flags (condensed; the guest cannot set any of them)

Class: B behaviour the Windows profile needs, M measurement, D debug, X dangerous when set, N negative control,
I fault injection. Reads are lazy `OnceLock`s from the launcher's environment; there are no config files, no cwd
reads and no env-driven listen sockets.

| flags | where | class | note |
|---|---|---|---|
| `KF3_NEGCTL_CARVE`, `_TWIN_WINDOW`, `_NO_BAR1_DOORBELL`, `_HEAP`, `_TWIN`, `_STALE_BIND`, `_TSPACE_OVERSIZE`, `_NO_MICRO_RESERVE` | `tspace.rs`, `mem.rs`, `device.rs`, `chan.rs` | X/N | SA-11; `_SKIP_CAP_BRACKET` (`kf-host/channel.rs:497`) can only make births fail |
| `KF3_DEBUG` + `KF3_DEBUG_FORCE_MIGRATABLE` | `chanlink.rs:320-345` | D | the only properly double-gated flag |
| `KF3_GSS_NATIVE` | `gssnative.rs:40` | X | SA-12 |
| `KF3_INJECT_STALE_USERD`, `_FWSEC_FAIL`; `KAYFABE_TRAP_FATAL*`, `KAYFABE_VCPU_BLOCK_FATAL`; `KF_IOCTL_TRACE`, `KF_SELF_DEADLINE_MS` | `chan.rs`, `device.rs`, `kf-util` | I/D | SA-24 |
| `KF3_USER_RING_TRACE=<dir>`, `KF3_DEBUG_CURSOR=<dir>` | `userpump.rs:165`, `display.rs:4204` | D | file writes, SA-24 |
| `KF3_BAR0_TRACE` and ranges, `KF3_BAR0_READ_TRACE`, `KF3_MAPLOG`, `KF3_RPC_TRACE`, `KF3_PROF`, `KF3_DIAG_*`, `KF3_COMPLETION_PROBE`, `KF3_DISPLAY_*TRACE*` | various | M | stderr or QEMU `-trace`; bounded (512 MiB and 8M records default) |
| `KF3_WORKER_PARK_INFINITE`, `KF3_DIAG_UNMAP_DELAY_MS`, `KF3_NO_GUEST_VA_RESERVE` | `worker.rs:113`, `maplog.rs:50`, `kf-host/channel.rs:677` | D | operator-side DoS or dropped reservation |
| about 46 Windows-profile names (`KF3_KERNEL_GR_*`, `KF3_DEFERRED_API`, `KF3_WIN_USER_*`, `KF3_SW_RUNLIST_*`, `KF3_*_PROBE`, `KF3_DISPLAY_*`) | `chan.rs`, `kf-rm/*`, `display.rs` | B | change which RM calls are admitted or stubbed; captured-table flags conflict with "derive, never capture" |
| `KAYFABE_CUDART_GATE=replay` | `inittables.rs:1908` | D | replays a captured reply (frozen-tree heritage) |
| device properties (`gop-efi`, `x-gsp-observer` and others) | `kf3.c:1930-1993` | prod | `gop-efi` bounded and PE-checked; `x-gsp-observer` uses `O_EXCL | O_NOFOLLOW`, 0600 |

## 7. Tests committed by this audit

All are GPU-free. The five EXPECTED RED tests are intentionally red on `e64b8e5f6` and are evidence, not fixes;
they must not be wired into a required CI job until the findings are fixed. Each was committed alone.

| file | finding | state on `e64b8e5f6` |
|---|---|---|
| `crates/kf-mem/tests/secaudit_all_pdb_over_64_spaces.rs` | SA-03 | RED (MEASURED: `submitted [], unreconciled=1`) |
| `crates/kf-trap/tests/secaudit_armed_request_is_a_snapshot.rs` | SA-14 | RED (serves `0x2000000`, armed `0x1000000`) |
| `crates/kf-rm/tests/secaudit_internal_client_range.rs` | SA-02 | RED (`0xC1F00000` accepted as internal) |
| `crates/kf-mem/tests/secaudit_cpu_window_views_are_capped.rs` | SA-06 | RED (200 000 live views accepted) |
| `crates/kf-rm/tests/secaudit_graph_free_is_not_quadratic.rs` | SA-04 | RED (MEASURED: 3000 frees 2.02 s, 6000 frees 8.64 s, bound 5 s) |
| `crates/kf-rm/tests/secaudit_hostile_rpc.rs` | S1-24, S1-83 | GREEN: deterministic xorshift generator, structured and mutated `GSP_RM_CONTROL` (ids harvested from the `kf-rm`/`kf-abi` sources), `GSP_RM_ALLOC` (device, TSG, context share, channel, VA space and random classes with hostile parents and `paramsSize`), `FREE`, `DUP_OBJECT`, fn 1/47/65/71/72/73 and random functions, through `kf_rm::served_policy` with object, memory, channel and display links stubbed |

MEASURED, hostile RPC harness (`SECAUDIT_ITERS`, `SECAUDIT_SEED` override the defaults): 3 000 000 iterations,
release profile with `overflow-checks = true`, seed 424242, object link only: none 1 481 420, ok 212 688, refused
1 305 892, panics 0; 200 000 iterations release, default seed, same links: panics 0; 400 000 iterations, debug
profile (debug assertions on), seed 99, with the channel, memory and display seats stubbed in: none 195 712, ok
31 131, refused 173 157, panics 0; 300 000 iterations debug, seed 77, object link only: panics 0. The generator reaches about 7 percent of requests that the
chain answers OK; it does not model a multi-step driver session beyond a small handle set, does not reach the
channel plane or memory plane implementations (stubbed sinks), and does not fuzz the GSP queue FSM
(`kf-gsp` `GspFsm`, element decode) or the `kf-qemu` readers. It is a floor, not a proof.

Not written (not cheap GPU-free): SA-01 (needs QEMU), SA-05 (needs the worker plus a stalled relay model), SA-07
(stderr counting by re-exec), SA-08 and SA-10 (private fields or the walker fake; sketches are in the findings),
SA-09 (fake `ViewOps` in `mem.rs`), SA-12, SA-13, SA-15.
