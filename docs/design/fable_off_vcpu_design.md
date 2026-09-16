# Fable — the off-vCPU design for the w742 violation (constraints 4, 6, 7, 8)

**STATUS: LIVE — §2.1 (P1) and §2.2 (P2) BUILT 2026-09-16 by w752; §2.4 and §2.5 rows 3-5 remain DESIGN-ONLY. Originally DESIGN-ONLY, 2026-09-16 (w751).** Written from source at `8a842d90` (kf-master tip,
= `single-store` + the w749 leg-B ruling) and from the committed w742 evidence
(`traces/w742_publish_install/w742_evidence.tgz`, `kayfabe-rev d0a4b10b`). ⊘ No production code
was changed, no bench was booted. Every number below is either read out of that trace or is a
component sum that says so. Inferences are marked **INFERRED** with the probe that would settle
them (§8). This document makes no posture decision; §9 lists the ones it leaves to the owner.

Brief: `docs/design/THE_CONSTRAINTS.md` §"OPEN VIOLATION — 4, 6, 7, 8 on the `device` arm"
(`THE_CONSTRAINTS.md:647-690`) and constraint 25 (`:149-176`).

---

## 0. Headline — the census names one path, and it is not the one the brief assumed

```
device arm:  VCPU-BLOCKING total=197 doors=9
  [40 × receiving a descriptor across the isolate boundary]
  [20 × KVM_SET_USER_MEMORY_REGION (installing a memslot)]
  [20 × classifying a received descriptor]
  [20 × exporting a host device view to the VMM]
  [20 × mmap (creating a guest-physical window)]
  [20 × mmap MAP_FIXED (placing an armed device node)]
  [19 × KVM_SET_USER_MEMORY_REGION (dropping a memslot)]
  [19 × munmap (dropping a guest-physical window)]
  [19 × releasing a host device view]
```
`[w742, run_w742dev_qemu.log, the VCPU-BLOCKING line]`

★★★ **`20 × 7 + 19 × 3 = 197.` Every crossing is one PRAMIN window move on the device arm.**
Twenty arms of `1 export + 2 recv + 1 classify + 1 mmap + 1 MAP_FIXED + 1 memslot install`, and
nineteen retirements of `1 memslot drop + 1 munmap + 1 release`. The path is
`kayfabe_shim_regs_write` (`crates/kayfabe-qemu-raw/src/shim_unsafe.rs:1372`) → `Regs::write`
(`shim.rs:17666`) → `m.after_write(&out)` (`shim.rs:17774`) → `BarMirror::after_write`
(`crates/kayfabe-qemu-raw/src/barmirror.rs:2197`, `if out.claimed { … self.repoint_pramin() }`
`:2210`) → `repoint_pramin` (`:1630`) → device arm: `self.retire(vec![Retired{…}])` (`:1751`)
then `install_pramin_window` (`:1756` → `:1815`) → `port.with_node(base, span_len, true, |fd,
mmap_len| self.machine.install_device_page(gpa, mmap_len, fd, false))` (`:1837-1839`).

The same boot's PRAMIN census says what that costs the guest:

| | arena arm (control) | device arm |
|---|---|---|
| PRAMIN moves | 20 | 22 (end of run) |
| `move_ns` mean | **63 µs** | **7 248 µs** |
| `move_ns` worst | **533 µs** | **44 426 µs** |
| `worst_trap` | 22 311 µs **at `bar0+0x110c00`** (GSP queue head) | 44 440 µs **at `bar0+0x1700`** (BAR0 window latch) |
| `slow_traps(>1ms)` | 1 | 21 (18 in 1–10 ms, 3 in 10–100 ms) |

`[w742, both `PRAMIN-SLOT` lines and both `TRAPWITNESS` lines; the device arm's `move_ns` is the
END-OF-RUN line, the mid-boot line reads mean 7 365 µs / worst 44 426 µs]`

★★★★★ **And the 44 ms is CPU, not a wait.** `TRAP-CPU n=10456 worst_wall=44440us
cpu_of_that_trap=43390us slow_busy=21 slow_preempted=15 slow_blocked=21` `[w742 dev]` — the vCPU
thread was *running* for 97.6 % of its worst trap. ⇒ whatever the move is waiting on, it is not
mainly the isolate's socket (a `read()` would show as wall without CPU). §1.3 says what it most
plausibly is and how to tell.

### 0.1 The brief's framing, checked and refuted: `drain` is ALREADY off the vCPU

Constraint 25's text says *"Constraint 6 is exactly 'move `drain` to a worker'"*
(`THE_CONSTRAINTS.md:175-176`). Measured against the source, **`drain` does not run on a vCPU
today and contributes zero of the 197**:

- `DeviceFbBytePort::drain` **declines by name** before touching anything:
  `if on_vcpu_thread() || in_trap() { self.declined += 1; return DeviceFbDrained::declined(); }`
  (`crates/kayfabe-qemu-raw/src/deviceview.rs:915-922`). Its plane-side wrapper does the same,
  before any lock (`crates/kayfabe-device/src/plane.rs:3673-3682`, `FB_DEMAND_DECLINED_ON_VCPU`).
- The boot confirms the guard fires and the arms happen elsewhere: `FB-DEMAND drains=4691 armed=80
  … declined_on_vcpu=14` and `DEVICE-FB-PORT … declined_on_vcpu=0` `[w742 dev]`. The 4 691 drains
  ran on `kayfabe-doorbell-publish` (`shim.rs:5463`, spawned `:16670-16672`), reached through
  `barmirror.rs:1097`/`:2012` and `shim.rs:14315`/`:22540`.
- `BarMirror::fill` queues on the vCPU when `defer_reval` is on (`barmirror.rs:938-950`) and
  `enforce_device_store` **refuses the device arm at realize** if it is off
  (`deviceview.rs:1242-1275`, the refusal at `:1260-1272`), so `fill_now`'s device branch (`barmirror.rs:1262-1270`, the
  `R_ON_VCPU` refusal) is unreachable from a vCPU by construction.

⇒ The `want`/`drain` split is sound and is not the fix. The fix is aimed at `repoint_pramin`.

### 0.2 Why it was "sanctioned" and why the sanction does not reach it

The one sanctioned expensive trap is *"the BAR0 window latch / PRAMIN re-point. It is an
`mmap(MAP_FIXED)` over one slot — `[measured w653a]` 297 µs worst"*
(`docs/design/the_write_trap_contract.md:66-68`). Its own text carries the expiry: *"the
load-bearing half of that ruling is 'only happens at boot', not the number"* (`:70-72`), and the
mechanism it sanctions is *one syscall* (`:83`, and `QemuMachine::repoint_file_window`'s doc,
`crates/kayfabe-vmm-qemu/src/lib.rs:1324-1348`: *"One syscall. That is what makes it affordable
synchronously on a vCPU"*). The device arm replaced the one syscall with **nine doors** — the
ruling's *architecture* changed under it (`THE_CONSTRAINTS.md:644`: *"a ruling's date AND its
architecture are part of the citation"*). `barmirror.rs:1726-1731` even predicts the new cost as
*"~0.7 ms measured"*, citing `SINGLE_STORE_PLAN.md` §3 item 4 — that figure was
`arm_us + rel_us` of the *view* (`DEVICE_VIEW=OK … arm_us=328 rel_us=460` at realize `[w742
dev]`) and never included the memslot drop + install + munmap that the re-point actually does.

### 0.3 A second violator the census cannot see

The **control** arm's worst trap is `22 311 µs at bar0+0x110c00` with `cpu_of_that_trap=22181us`
and **`doors=3`** — all three doors are the arena PRAMIN path (`[20 × MAP_FIXED backing] [1 ×
memslot install] [1 × window mmap]`), whose worst move is 533 µs. ⇒ the 22 ms at
`NV_PGSP_QUEUE_HEAD(0)` is **CPU work inside the trap that passes through no `assert_lock_free`
door**, and the door census is structurally blind to it — its own `lock.rs` twin says so:
*"only sites that go through BlockingSection are visible here; cross-check
worst_trap/slow_traps, which measure the hold itself"* (`crates/kayfabe-util/src/lock.rs:995-996`).
`GspFsm::defer_commands` exists (`crates/kayfabe-gsp/src/boot.rs:638-643`, set at
`shim.rs:16686`) and defers only the *servicing*; what still costs 22 ms of CPU on the vCPU under
that arm is **not measured here** (w395's `KFTIME-SEG` instrument is the one that attributes it —
`git show 44c2cca9`). It is outside this brief's nine doors and is listed in §8 as its own probe,
because constraint 4 is violated on **both** arms and fixing the nine doors will not make
`worst_trap` sub-millisecond on the control.

---

## 1. The nine doors, one by one

Legend: **ELIMINATE** = the door stops existing on this path · **MOVE** = same work, on a worker ·
**STAYS** = structurally vCPU-bound, with the reason.

| # | door (census name) | site | per move | what it is in the PRAMIN move | verdict |
|---|---|---|---|---|---|
| 1 | `exporting a host device view to the VMM` | `crates/kayfabe-isolate/src/lib.rs:3688` (`Worker::export_device_view`) → `crates/kayfabe-isolate-host/src/isolate.rs:636` (`call_for_device_view`, write then blocking read) → child `crates/kayfabe-isolate-host/src/rm.rs:6841` (`arm_cpu_view`, i.e. `NV_ESC_RM_MAP_MEMORY`) | 1 | **the ARM**: RM maps 1 MiB of the reserved object at `base` and hands back a `/dev/nvidia<N>` node | **STAYS in the trap, then MOVE off it by pre-arming** (§2.3, §2.4) |
| 2 | `receiving a descriptor across the isolate boundary` | `crates/kayfabe-linux-raw/src/scm_unsafe.rs:350` (`recv_with_fds`), called twice per reply by `read_frame_with_fds` (`fdcross.rs:266-270`: the length word, then the body) | 2 | the reply half of door 1 | same as 1 |
| 3 | `classifying a received descriptor` | `scm_unsafe.rs:489` (`descriptor_kind`, an `fstat`) via `exports.adopt` (`isolate.rs:670-676`) | 1 | the `CharDevice` check on the node | same as 1 — ⊘ and it is a **local `fstat`**, microseconds; it is in the census only because it asserts lock-free |
| 4 | `mmap (creating a guest-physical window)` | `crates/kayfabe-linux-raw/src/window_unsafe.rs:126` (`GuestWindow::create`) from `install_window_inner` (`crates/kayfabe-vmm-qemu/src/lib.rs:1570`) | 1 | a **fresh** 1 MiB VMA for every move | **ELIMINATE** — the window already exists; re-place into it (§2.1) |
| 5 | `mmap MAP_FIXED (placing an armed device node)` | `window_unsafe.rs:286` (`place_device_view`) | 1 | the one syscall the sanction was written for | **STAYS** — it is the un-deferrable half (the guest reads through the aperture on its next instruction, `the_write_trap_contract.md:74-77`) |
| 6 | `KVM_SET_USER_MEMORY_REGION (installing a memslot)` | `crates/kayfabe-linux-raw/src/kvm_unsafe.rs:455` via `KvmMemslot::install` (`:511`) from `crates/kayfabe-vmm-qemu/src/slots.rs:233` | 1 | a **new** memslot at the same GPA, every move | **ELIMINATE** — the slot never needs to move (§2.1); the *first* install belongs at BAR-map time (constraint 23, `THE_CONSTRAINTS.md:131-135`) |
| 7 | `KVM_SET_USER_MEMORY_REGION (dropping a memslot)` | `kvm_unsafe.rs:556-571` (`KvmMemslot::drop`) from `QemuMachine::remove_window` (`lib.rs:2363`, `drop(live)` `:2412-2415`) | 1 | deleting the previous move's slot | **ELIMINATE** (§2.1) |
| 8 | `munmap (dropping a guest-physical window)` | `window_unsafe.rs:527` (`GuestWindow::drop`) via `reclaim_released_windows` → `collect_retired` (`lib.rs:1496`) | 1 | unmapping the previous move's VMA | **ELIMINATE** — `MAP_FIXED` replaces atomically (`lib.rs:1341-1344`), nothing to unmap |
| 9 | `releasing a host device view` | `kayfabe-isolate/src/lib.rs:3720` (`Worker::release_device_view`) from `DeviceViewPort::release_held` (`deviceview.rs:420-425`) from `BarMirror::drain_view_releases` (`barmirror.rs:1468-1500`), called **inline** by `retire` (`:1455`) | 1 | `NV_ESC_RM_UNMAP_MEMORY` of the previous view, giving back 1 MiB of host BAR1 aperture | **MOVE** — deferrable by construction; the parked list it needs already exists (§2.2) |

Two things the table makes visible that the brief's list did not:

- **Only door 5 is the sanctioned mmap.** Doors 4, 6, 7, 8 are memslot and VMA churn that the
  arena arm never does — `repoint_file_window` is *"one `mmap`, nothing else … there is no slot to
  update and no lock to take"* (`lib.rs:1325-1337`). `l1_os_shell.md` §6.7 rule 3 forbids exactly
  this shape: *"No slot delete/recreate to change a mapping. A DELETE is two grace periods plus a
  shadow zap. Re-`MAP_FIXED` the window's backing instead"* (`docs/design/l1_os_shell.md:1979-1985`),
  and constraint 16 restates it (`THE_CONSTRAINTS.md:102-107`).
- **Doors 1–3 are one IPC round trip** whose cost is RM's, not ours: measured `arm_us_total=544095`
  over 1 849 arms ⇒ **294 µs mean** `[w742 dev, end of run]`; 282 µs mean at the mid-boot line;
  328/380 µs at realize. Door 9 is another: `rel_us_total=85766` over 112 ⇒ **766 µs mean**.

### 1.3 Where the 43 ms of CPU goes — INFERRED, with the discriminator

Summing the measured components of a move gives `294 (arm) + 766 (release) + ~63 (mmap, the
arena's whole move) ≈ 1.1 ms`, and the census says the mean move is **7.2 ms and the worst 44 ms,
97 % of it CPU**. The unmeasured remainder is doors 4, 6, 7, 8. Of those, the memslot pair is the
only one with a known millisecond-class cost: `l1_os_shell.md:1931-1941` reads it out of
`virt/kvm/kvm_main.c` — a DELETE is *two* memslot-array swaps, each `synchronize_srcu_expedited`,
plus `kvm_arch_flush_shadow_memslot`'s zap with a remote TLB flush across all vCPUs, *"charged to
every vCPU … no matter which of our threads issues the ioctl"*.

**INFERRED:** on the bench kernel the x86 zap on memslot delete is a whole-VM
`kvm_mmu_zap_all_fast()` (the pre-`KVM_X86_QUIRK_SLOT_ZAP_ALL` behaviour), which would put
tens of milliseconds of *CPU* on the deleting thread and an EPT refault storm on every other vCPU
— matching `cpu_of_that_trap ≈ wall` and `slow_preempted=15`. ⚠ Not measured: the bench's kernel
version is not in the committed evidence (`grep "Linux version" run_w742dev_hostdmesg.log` → no
line). **Discriminator (§8 P-1):** time each door of one move separately; the sum must reproduce
`move_ns` within 5 %, and the door that carries the tens of milliseconds is the one to eliminate
first. If doors 4–8 sum to microseconds and the CPU is elsewhere, the elimination in §2.1 still
holds (it is `l1_os_shell` §6.7's rule regardless) but the 44 ms has another owner and this
document's cost prediction in §2.5 is wrong.

---

## 2. The design — four cuts, ordered by evidence

### 2.1 Cut P1 — re-point the PRAMIN window in place (eliminates doors 4, 6, 7, 8)

`barmirror.rs:1726-1731` says *"a device view cannot be re-pointed — each arming is its own fd at
offset 0"*. That is true of the **node** and false of the **window**. `nvidia_mmap_helper`
refuses `vm_pgoff != 0` (`window_unsafe.rs:251-253`), so *what* a node shows is fixed at arm
time — but `GuestWindow::place_device_view` (`window_unsafe.rs:279-300`) is `fixed_map` over a
`HostOffset` inside an **existing** window, exactly as `GuestWindow::place` is for a file, and
`repoint_file_window` (`lib.rs:1349-1367`) is nothing but `window.place(HostOffset::ZERO, len, …)`
on the region's live window. ⇒ The device analogue is:

```
repoint_device_window(region, new_node_fd, mmap_len)
  = installer-lock lookup of the region's GuestWindow            (as lib.rs:1356-1362)
  + window.place_device_view(HostOffset::ZERO, mmap_len, fd, true)   (window_unsafe.rs:279)
```

One `MAP_FIXED`. The hypervisor is not told, for the reason `lib.rs:1329-1337` gives: the GPA and
the HVA are unchanged, only what lies behind the HVA moved, and the MMU notifier retires the stale
EPT entries. The device page install already runs KVM over a `VM_IO | VM_PFNMAP` VMA
(`window_unsafe.rs:257-261`, and it booted: `PRAMIN-WINDOW installed … showing fb 0x0` `[w742
dev]`), so nothing new is asked of KVM — the same VMA kind is replaced under the same slot.

**What it deletes per move:** the fresh window (door 4), the memslot install (door 6), the memslot
drop (door 7), the munmap (door 8). What is left synchronous: doors 1–3 (the arm) and door 5.

⚠ **Two things this changes that must be re-asserted, not assumed (constraint 29):**

1. The old view's **mapping is gone the instant `MAP_FIXED` returns** — atomically, with no
   interval, which is *better* than today's `retire → install` (whose gap `barmirror.rs:1737-1746`
   names: *"between them PRAMIN has no slot. A sibling vCPU accessing PRAMIN in that interval
   traps"*). But the w735 release-ordering rule (`barmirror.rs:1427-1441`; `THE_CONSTRAINTS.md`
   §w735 *"AND ONE DEFECT THAT WOULD HAVE SHIPPED"*) keyed the release on
   `reclaim_released_windows` naming the region — i.e. on the *window* being collected. Under P1
   the window is never collected; the **old node's PTEs are gone because they were overwritten**,
   and any `Arc<GuestWindow>` holder now sees the *new* backing, which for PRAMIN is correct (an
   accessor of the moving aperture is supposed to see where it points now — this is precisely how
   the arena arm behaves today). ⇒ The barrier must be restated: *"release view V only after the
   `MAP_FIXED` that replaced V's mapping has returned"* — a happens-before that the re-point
   itself provides. **Replacement assert:** the parked release carries the `MAP_FIXED` sequence
   number that retired it, and the releaser refuses any entry whose sequence is not ≤ the last
   completed re-point. Fail-closed: a leak (§2.2), never an early release.
2. **The sentinel probe (§8 P-2)** is the discriminator between *"the re-point took effect"* and
   *"KVM kept a stale EPT entry"* — the difference reads as *the old framebuffer bytes*, which is
   exactly the failure the trap contract calls *"the one failure on this path that cannot be
   contained"* (`barmirror.rs:1799-1803`). It must be run before P1 is believed.

### 2.2 Cut P2 — park the release, drain it on the worker (moves door 9)

`retire` calls `drain_view_releases()` inline (`barmirror.rs:1455`), and that function is
documented *"Must be lock-free and off-trap … Called from `Self::retire` and from the reclaim
tick"* (`:1461-1466`) — the reclaim tick is the worker's. The move is one decline-by-name, in the
same shape `DeviceFbBytePort::drain` already uses (`deviceview.rs:918-922`):

```
drain_view_releases():  if on_vcpu_thread() || in_trap() { parked_declined += 1; return; }
```

with the parked entry pushed as today (`:1449-1453`) and the worker's tick releasing it. The
`parked` list and its census (`parked_releases=364` `[w742 dev]`) already exist; what changes is
*who* drains it. ⚠ **Count the decline**, or a worker that never ticks reads as *"nothing was
parked"* — the tree's `a_refusal_counter_read_as_absent_demand` class. And note the cost of the
safe direction: a view parked and never released holds 1 MiB of host BAR1 aperture; at 22 moves
that is 22 MiB of a ~254 MiB pool (`THE_CONSTRAINTS.md` §w722c), and the port's `outstanding=`
is the number that says whether the tick is keeping up.

### 2.3 What STAYS synchronous after P1+P2, and what it costs

| left on the vCPU per move | measured component | source |
|---|---|---|
| doors 1–3: the arm (one IPC round trip; RM `NV_ESC_RM_MAP_MEMORY` + SCM_RIGHTS) | **294 µs mean** (`544095 / 1849`); realize-time samples 328, 380 µs | `DEVICE-VIEW-PORT arm_us_total` `[w742 dev]`; `SCRATCHPAD-DEVICE-VIEW AT REALIZE` |
| door 5: one `MAP_FIXED` | ≈ the arena's whole move: **63 µs mean, 533 µs worst** | arena `PRAMIN-SLOT move_ns` `[w742 arena]` |
| predicted move | **≈ 0.36 ms mean**, tail ≈ 0.9 ms plus the arm's tail | component sum — **a prediction, not a measurement** |

⇒ P1+P2 bring the device arm's re-point back to the **order of the sanctioned 297 µs**, and the
census to `[20 × exporting] [40 × receiving] [20 × classifying] [20 × MAP_FIXED (armed node)]` =
100 crossings, 4 doors. That is not constraint 25's *empty*, and it is not guaranteed sub-ms:

⚠ **The arm's tail is not ours to bound.** `[w675]` RM holds a device-global client lock across
the GSP RPC (`memory: rm_s_client_lock_is_what_serialises_one_isolate.md`; `planreactor.rs:33-46`,
1.00× for every worker arrangement). A vCPU arming PRAMIN waits behind whatever RM verb *any*
isolate is in — the publication worker's own `fill_now` arms (1 849 of them in this boot), a CE
submission, a channel birth. The mean is 294 µs; the tail is *"the longest RM verb in flight"*.

⚠ **And there is a refusal on this path that is worse than a wait.** `with_node` arms through
`SharedIsolate::with_worker` (`deviceview.rs:281-283` → `crates/kayfabe-qemu-raw/src/scratchpad.rs:691-699`),
which is `g.checkout()?` — it **returns `None` rather than waiting** when every worker of the
`pool=4` scratchpad is checked out. `with_node` turns that into `ViewRefusal::NoWorker`
(`deviceview.rs:285`), and `repoint_pramin` then prints *"RE-ARM REFUSED … there is now NO
slot"* (`:1766-1777`) — after which every PRAMIN access traps and *"the single store refuses it
by name"*. `refused=0` in w742 says it did not fire *in that boot*; nothing bounds it. **INFERRED
reachable** under a publication burst (four fills in flight on four workers while the guest
moves the window); §8 P-4 is the known-positive.

### 2.4 Cut P3 — take the arm off the vCPU: retain, re-export without RM, and pre-arm

The arm exists because a node shows one fixed 1 MiB of the object and the guest chooses which.
Three ways to stop paying RM inside the trap, in increasing ambition:

**(a) Retain + re-export.** The child keeps every armed node in its export table until release
(`rm.rs:6856-6865`: `arm_cpu_view` then `exports.mint_armed_node(node, CpuViewRelease{…})`;
release is `take_cpu_view_release(token)` → `NV_ESC_RM_UNMAP_MEMORY`, `:6910`). ⇒ A view whose
base the guest revisits can be **re-sent over SCM_RIGHTS without an RM call**: a new wire verb
`ReexportDeviceView(release_token)` that answers with the retained node. Cost on the vCPU: the
socket round trip (w321: transport ≈ 29 µs against the ioctl's ≈ 132 µs, `planreactor.rs:30-32`)
+ 2 recv + fstat + `MAP_FIXED` — **no RM lock in the path, so the tail is socket-bound**. Aperture
cost: K retained MiB. The guest revisits bases — `moves=22 skipped=9481` `[w742 dev]` says the
latch is written thousands of times for 22 distinct moves — but whether the 22 moves are ≤ K
*distinct* bases is **not measured** (§8 P-3). ⚠ Condition 2 of the decision-(b) ruling (*"closed
the moment `mmap` returns"*, `deviceview.rs:302-306`) is about the **VMM's** copy of the fd and is
unchanged: the retained fd lives in the child, where it already lives today.

**(b) Pre-arm at realize.** If the base set is a deterministic function of the chip profile —
`showing=0xfff00000` `[w742]` is the top MiB of a 4 GiB reservation; ogkm's boot-time users of
the BAR0 window are RM writing BAR2's own page tables and `memUtilsMemSetNoBAR2`
(`memory: pramin_is_a_bringup_aperture_not_a_running_path.md`, verified in 580 `bar2_walk.c`) —
then the whole set can be armed on the worker **before the guest's first latch write**, and every
move becomes **one `MAP_FIXED` of a retained node**: zero IPC on the vCPU, census
`[22 × MAP_FIXED (placing an armed device node)]`, one door, the same door the arena arm has.
⚠ **INFERRED** and cheaply falsifiable (§8 P-3): three boots at two FB sizes; if the sequences
differ, (b) degrades to (a) with a warm cache and the first visit of each base pays §2.3's arm.

**(c) The first install moves to setup (constraint 23).** Today the window + memslot are created
on *"the guest's FIRST latch write"* (`barmirror.rs:1671-1712`), because w579 found the firmware
reprograms BAR0 after realize. Constraint 23's refinement answers exactly that: install in the BAR
*map* callback and re-install when the BAR moves (`THE_CONSTRAINTS.md:131-135`). That takes the
one-time `mmap + memslot install` (2 of the 197, and 7 of the device arm's crossings) off the trap
and onto setup, where a memslot ioctl belongs (constraint 16).

### 2.5 The census after each cut (predicted from the counts above)

| state | crossings | doors | vCPU time per move |
|---|---|---|---|
| w742 device arm (measured) | 197 | 9 | 7.2 ms mean / 44 ms worst |
| + P1 (in-place re-point) + P2 (parked release) | 100 | 4 | ≈ 0.36 ms mean (§2.3), RM-bound tail |
| + P3(a) retain/re-export, warm | 20 + 40 + 20 (socket-only) + 20 | 4 | ≈ 0.1 ms, socket-bound tail |
| + P3(b) pre-arm, if deterministic | 22 | **1** — `MAP_FIXED (armed device node)` | ≈ 63 µs mean, as the arena |
| + (c) first install at BAR map | 22 | 1 | — |

⊘ Row 4 is the only one that reaches *"one door, the sanctioned one"*, and it rests on an
INFERRED determinism. Row 2 is the one this document is confident of.

### ✔✔✔ w752 — MEASURED 2026-09-16. FOUR PREDICTIONS HELD, ONE REFUTED, AND THE REFUTED ONE IS THE INSTRUCTIVE ONE

`[vast 51210329, GA106, 580.159.04 OPEN, TREE_REV fcce2a11, three arms, one binary, control first]`

| | predicted | **measured** | |
|---|---|---|---|
| **1** census | 102 crossings / 6 doors, doors 4+6 at exactly 1, doors 7/8/9 absent | **`total=102 doors=6`**, `[1 × window mmap] [1 × memslot install]`, 7/8/9 **absent** | **HELD, to the crossing** |
| **2** cost | mean < 1 ms; worst not predicted sub-ms | `move_ns[worst=1115752 mean=496145]` = **1.12 ms / 0.50 ms** (w742: 44.4 ms / 7.25 ms) | **HELD** |
| **3** `worst_trap` does NOT go sub-ms | ~22 ms at `NV_PGSP_QUEUE_HEAD` remains | **`worst_trap=24999us at=bar0+0x110c00`**, `cpu_of_that_trap=23979us`; control **25 077 µs at the same address** | **HELD** |

> ### ⊘⊘⊘ **ROW 3's READING IS CORRECTED — 2026-09-16 (w754). The NUMBER held; the SENTENCE
> ### it was read as did not.** Row 3 predicted *"~22 ms at `NV_PGSP_QUEUE_HEAD` remains"* and
> the register name was taken as the mechanism. **`NV_PGSP_QUEUE_HEAD` has not serviced the GSP
> queue inline since w432**, and `defer_commands` has survived a guest reset since w472b. The
> cost was `adopt_pending_channel_rings`' guest page-table settlement, running on the vCPU from
> `Regs::write` on **whichever** register write noticed `pending_latch_epoch()` move —
> `0x110c00` merely wins that race most often during driver init.
> ⇒ `[measured w754]` settlement on the worker: that site goes **24 999 µs → 1 579 µs**, and
> `0xb81408/0410/1608/1610` leave `SLOW-SITES` with it. The worst trap was then a THIRD thing
> (a 4.19 M-iteration `OnceLock` sweep at `0x110118`, 100 % CPU), and after that a **lock wait**
> at `0xbb0090` (48 % CPU). Full record: `w754_gsp_queue_off_vcpu.md`.
> ⚠ The lesson for this table's own form: a prediction that names a **register** is confirmed by
> the register appearing, whatever is actually spending the time there. Predict the MECHANISM,
> or make the confirming row carry `cpu_of_that_trap` beside the site — which is what separated
> these three.
| **4** leak gauge | `early_release_refused=0`, `held` small, `declined_on_vcpu > 0` | `inplace=21 started=21 landed=21 released=21 held=0 early_release_refused=0` — but **`declined_on_vcpu=0`** | **REFUTED on its last clause** |
| **5** no regression | — | `TRAP_FILLS=0` / `misses=0` both BARs both arms; `named=311180`; `RmInitAdapter failed`=0; `SMI_RC=0`; control `(P)` `THREADS 8 of 8`; Xid=0 on the device arm | **HELD** |

⊘⊘⊘ **WHY PREDICTION 4's LAST CLAUSE WAS WRONG, AND IT IS THIS TREE'S OWN FAILURE CLASS.** I
wrote *"`declined_on_vcpu` must be > 0 with `inplace` > 0: a zero there would mean the decline is
not on the path, i.e. cut P2 is not doing anything."* Measured: **zero, and cut P2 works.** The
decline never fires because **cut P1 deleted its only vCPU-side caller** — `retire()` is not
reached at all on a successful re-point, so `drain_view_releases` is never entered from a trap.
⇒ The evidence that door 9 moved is **not** the decline counter; it is that
**`releasing a host device view` is ABSENT from the census while `released=21`** — 21 releases
happened, none of them on a vCPU.
★ The decline is still load-bearing, on the **refusal** path (`refuse_pramin_slot` → `retire` →
`drain_view_releases`), which did not fire this boot (`inplace_refused=0`). ⚠ Its census text says
the wrong thing and has been corrected in the source; **a counter's zero means what its callers
make it mean, and I predicted from the counter rather than from its call graph.**

⚠ **`port_outstanding=1729`** is the whole port's outstanding set (the BAR mirror's premap views),
not PRAMIN's; PRAMIN's own number is `held=0`.

⊘ **The arms count 20, not 22, while `moves=22`** — identical in shape to w742 (`20 × 7 + 19 × 3`
against the same `moves=22`). The census counts crossings **on a vCPU thread**; the exact identity
this boot is `5 × 19 + 7 = 102`.

### ★★★★★ w752 — CUTS P1 AND P2 ARE BUILT. PRE-REGISTERED PREDICTIONS, 2026-09-16

`[branch `w752-pramin-in-place`, off `e8ce6f4a`. **Committed before any box existed** — nothing
below was written after seeing a number.]`

Built: `QemuMachine::repoint_device_window` (P1), `BarMirror::repoint_pramin_in_place` (P1),
the decline-by-name in `BarMirror::drain_view_releases` plus the worker tick in
`BarMirror::revalidate_pending` (P2). **Not built: P3(a), P3(b), (c).**

**Prediction 1 — the census.** Row 2 above says `100 crossings / 4 doors`. ⚠ That row and §2.4(c)
disagree by exactly the one-time install, and §2.4(c) is the one that is right: the window and
its memslot are still created on *"the guest's FIRST latch write"*, which is a vCPU. So the
number this cut can reach is:

| | crossings | doors | which |
|---|---|---|---|
| Fable §2.5 row 2, literal | 100 | 4 | |
| **w752 pre-registered** | **102** | **6** | 19 moves x 5 + one install x 7; doors 4 and 6 survive with **count 1 each** |
| after §2.4(c), not built | 100 | 4 | the install moves to the BAR-map callback |

⇒ **Held** = `total` and `doors` fall to within `+2 / +2` of row 2, **with doors 4 and 6 at a
count of exactly 1** and doors 7, 8 and 9 **absent**. **Refuted** = anything else, including a
fall to 102 whose residual doors are not the one-time install.
⊘ The move count is not fixed at 20: `moves` is what the guest does. The identity to check is
`crossings == 5 * moves + 2` on the device arm, not the literal 102.

**Prediction 2 — the cost.** `PRAMIN-SLOT move_ns` on the device arm falls from
`mean 7 248 us / worst 44 426 us` to **the order of the arm plus one `MAP_FIXED`**: §2.3's
component sum is `294 us + 63 us ~= 0.36 ms` mean. Predicted **mean < 1 ms**; worst **not
predicted sub-ms**, because the arm's tail is RM's device-global client lock and is not ours to
bound.

**Prediction 3 — what does NOT move, stated so it cannot be claimed later.** `worst_trap` will
**not** go sub-millisecond. The control arm's `22 311 us at bar0+0x110c00` (`NV_PGSP_QUEUE_HEAD`)
is a second constraint-4 violator that passes through **no `assert_lock_free` door at all**, so
the door census cannot see it and this cut cannot touch it (§0.3). If the device arm's
`worst_trap` lands near the control's ~22 ms, that is this second violator becoming visible once
the 44 ms PRAMIN move is gone — **not** a failure of the cut, and **not** a success either.

**Prediction 4 — the leak gauge.** `PRAMIN-INPLACE … early_release_refused=0` and `held` small
(the tick is frequent). A non-zero `early_release_refused` means the restated w735 barrier fired
and a view is holding 1 MiB of host BAR1 aperture forever — the safe direction, and a bug.
`declined_on_vcpu` must be **> 0** with `inplace > 0`: a zero there would mean the decline is not
on the path, i.e. cut P2 is not doing anything.

**Prediction 5 — no regression.** BAR1/BAR2 `TRAP_FILLS=0` and `misses=0`, `named` unchanged,
`RmInitAdapter failed!`=0, `SMI_RC=0`, control arm `(P)` `THREADS 8 of 8`, the failing-test name
set identical to the 30-name baseline.


### 2.6 Constraint 25's gate, stated against the code that exists

Two censuses with two vocabularies, and the brief conflates them:

- `lock.rs`'s `BlockingSection` allowlist (`crates/kayfabe-util/src/lock.rs:1041-1090`,
  `enter_required_on_vcpu`) **already has zero production call sites** (`grep -rn
  "enter_required_on_vcpu(" crates/ --include=*.rs` outside `lock.rs` → none). It is empty and
  has been; it saw none of the 197.
- The 197 are `lockwitness::assert_lock_free` doors (`crates/kayfabe-util/src/lockwitness.rs:258`,
  `:286` → `assert_not_on_vcpu` `:309-343`), which have **no allowlist at all** — only
  `KAYFABE_VCPU_BLOCK_FATAL` (`:314-321`), off by default, report-only.

⇒ *"the allowlist goes to EMPTY and non-empty is a build failure"* has to be made of the
**lockwitness** door table, not `lock.rs`'s: a boot-report gate that fails the boot grade when
`vcpu_blocking_census()` is not the empty arm (`:222-226` prints it explicitly), plus — the
structural half — `OffVcpu` extended past publication. The tree already has that type:
`OffVcpu(())`, *"minted in exactly ONE place, `for_publication_worker`, whose call site is the
worker loop"* (`shim.rs:3374-3377`, `:3388`), and `DoorbellTable` is the worked example of taking
the reach away instead of policing it (`crates/kayfabe-device/src/dbtable.rs:1-32`). The
uncallable form of P1 is: `DeviceViewPort::with_node` takes an `&OffVcpu`; `repoint_pramin`
cannot mint one; therefore the vCPU cannot arm — it can only `MAP_FIXED` a node it was handed.
Under P3(b) that is the whole re-point; under P1 alone it forces the arm onto a worker the vCPU
must then *wait for*, which is a trap that blocks on a worker — forbidden by the write-trap
contract's shape 4 (`the_write_trap_contract.md:61-62`). ⇒ **P1+P2 without P3 cannot satisfy 25
as written; P3(b) can; P3(a) can only for revisited bases.** That is a cost, laid out, not a
decision.

⚠ One instrument gap in the census itself: `kayfabe_shim_regs_read` installs the `TrapGuard`
(`shim_unsafe.rs:1358`) but **does not `mark_vcpu_thread`** — only the write entry does (`:1385`,
`:1390`, twice). A door reached from a read on a thread that has not yet written is invisible to
the 197. Cheap to close; must be closed before the empty-arm gate is trusted.

---

## 3. Reads — the minimum a vCPU must do synchronously

**Measured surface on the device arm:** `BAR0-READS total=18 … reads_from_live_pages=18
reads_from_BACKED_pages=0 top[+0xbb0000=18]` `[w742 dev]` — eighteen reads in the whole boot,
all on the free-running counter page, all before `install_counter_page` placed the host's
usermode page over it from the worker (`shim.rs:5496-5516`, w640). `the_bar0_read_surface.md`
§6c measured the same shape on the arena arm: 134 reads, one page, the counter. PRAMIN, BAR1 and
BAR2 reads are memslot-resolved and never reach `Regs::read`.

**What `Regs::read` does today** (`shim.rs:17270-17331`): `plane.read` (an `&self` path —
`the_bar0_read_surface.md:26-43` audited eleven of twelve live pages as pure functions of state we
own), then two *fill* hooks that queue and wake (`m.fill` / `m.fill_after_refusal` →
`wake_for_mirror_fill`), and — **only on the no-worker arm** — `drain_mirror_revalidation()`
(`:17276`, guarded by `!doorbell_async.defers()`). Nothing on the shipping arm blocks in a read.

**The reduction.** Under constraint 23 every BAR0 read that can be answered from bytes *is*
answered from a read-only memslot, so a read that traps is one of:

1. the counter page before the usermode page is placed (18 in w742) — answered from the host's
   usermode page or, until it is placed, from `plane.read`'s `&self` path; **no queue, no wake**;
2. a page with a **read side effect** — `the_bar0_read_surface.md:26-43` found none except the
   counter, and constraint 23 says such a register *"cannot live in a read-only memslot"*
   (`THE_CONSTRAINTS.md:136-141`), so if one appears it is a shadow-page correctness bug, not a
   trap-cost one;
3. a trapping BAR1/BAR2 read — under 23 *"the error itself"*, to be **counted and refused**, never
   served with work (`Regs::read`'s `fill_after_refusal` arm at `:17300-17321` is the repair
   hook; the repair runs on the worker).

⇒ **The minimum synchronous read handler is: bounds-check, load from the shadow bytes, count.**
It can be reduced to *"answer from the shadow page"* **iff every write handler writes its O(1)
state through to the shadow before it queues** — which is the write-trap contract's own
correctness clause: *"arm the completion synchronously, or the next request reads the last
one's answer"* (`the_write_trap_contract.md:93-110`; the invalidate's wait bit is cleared in the
trap, and `BootStep::CommandDoorbell` stores the queue head *before* deferring the servicing,
`boot.rs:1047-1054`). A read never needs a worker; a **write** needs one store before it returns.

⊘ What this does *not* reduce: a read that lands while a deferred write's shadow says *pending*
is correct (the guest polls, as it would on hardware); a read that lands on a shadow nobody wrote
through is silently stale, which `the_bar0_read_surface.md` §5 answers with a teardown sweep
comparing shadow bytes against the classifier. That sweep is the read-side instrument and it is
already specified.

---

## 4. What breaks the semaphore-as-source-of-truth rule, and where an eventfd lies

`await_semaphore` (`crates/kayfabe-isolate-host/src/rm.rs:9313-9337`) polls the semaphore word
at 1 ms with a deadline and returns **three facts on every exit** — `semaphore`, `gp_get`,
`gp_put` — precisely so a timeout is legible (`:9316-9319`: *"a poll cannot mistake 'we were
never woken' for 'it never landed'"*). An eventfd can, in four places this tree already contains:

1. **The producer does not exist.** `Registrar::arm_counter` (`crates/kayfabe-shell/src/sources.rs:398`)
   hands back the eventfd *"the isolate's relay thread writes when one of its own nvidia
   descriptors fires"* (`:385-390`). There is no relay thread in `kayfabe-isolate-host`
   (`completion_wait_architecture.md` §4(c); re-confirmed for this document: no `relay`/`OsEvent`/
   `eventfd` site in `child.rs` or `rm.rs`). The one production `Reactor`
   (`kayfabe-completion-observer`, `shim.rs:16852-16908`) arms a counter (`:16871`) whose inbox
   receiver is **dropped on the next line** (`let (tx, _rx) = inbox()`, `:16872`), and its only
   `signal()` is from the vCPU's own `declare_gr_completion` (`shim.rs:8208` → `:8622`). ⇒ Today a
   worker that blocks on that eventfd waits on a signal nothing sends, and *"not woken"* reads
   exactly like *"not landed"*.
2. **Arm-directed events never fire for channels with no arm.** `the_interrupt_arming_model.md`:
   40 of 44 completions were never announced, all `no_engine` — the vector was derived from an
   engine bind the channels do not have. A relay conditioned on the same thing is silent for the
   same channels, and a blocked `epoll` then has no ceiling.
3. **Arm-after-landing.** A semaphore that flipped *before* its eventfd was registered signals
   nothing. The owner's rule already names the fix — *"armed after ⇒ send immediately"* — as an
   ordering: **register → re-read the semaphore**, never read → register.
4. **Coalescing is fine; draining to zero is not.** An eventfd counter folds N signals into one
   wake, which is harmless (the semaphores are re-read). `Reactor`'s `MAX_UNPRODUCTIVE_STREAK`
   gate (`completion_wait_architecture.md` §2.1) already makes a wake that drains nothing loud.

**The rule as a mechanism, not a note:**

- The blocking predicate is a function of the **semaphore set**, never of the eventfd set:
  `may_block ⇔ ∀ outstanding s : s.eventfd_registered ∧ s.reread_after_register ∧ s.not_landed`,
  and while any `s` fails a conjunct the loop polls with `epoll` timeout 0 (the owner's rule).
- **Every wake re-reads every outstanding semaphore**, and the loop keeps two counters that
  distinguish the two absences: `LANDED-WITHOUT-WAKE` (a semaphore found satisfied on a
  *timeout* wake with its eventfd never signalled — the instrument defect, a relay that did not
  fire) and `WOKEN-WITHOUT-LANDING` (an eventfd wake whose semaphores are unchanged — a spurious
  or coalesced signal, benign). ⊘ Without the first counter a broken relay is indistinguishable
  from a slow GPU; it is the same `dlen=0` shape this tree has paid for repeatedly.
- The blocked `epoll` has a **ceiling below the guest's own** — 4 s in graphics mode, 30 s in
  compute (`blocking_and_completion_model.md:49-69`) — so a lost wake costs a bounded stall and
  a counter, never a guest-visible timeout attributed to the wrong side.
- `await_semaphore`'s three-fact outcome stays the *only* thing that may mint a completion
  (`completion_wait_architecture.md` §7 R1: a `Completion` only an observation can construct).
  An eventfd is a **hint about when to look**, and nothing derived from it may be written to the
  guest.

⊘ Scope: this section is about constraint 10's worker loop. It does **not** touch PRAMIN, whose
arm is synchronous by nature and has no completion to wait for — an `epoll` loop buys PRAMIN
nothing (§7).

---

## 5. Where `nvkvm-pv` already solved this — and where it did not

Surveyed for this document (all citations `/workspace/nvkvm-pv`):

**What it did NOT solve.** Its TX handler runs **on the vCPU under the BQL**: the virtio-PCI
wrapper defines no `ioeventfd` property (`src/qemu/virtio_nvgpu_pci.c:136-151`, contrast upstream
`virtio-blk-pci.c:44`), so `virtio_queue_notify` calls `nvkvm_tx_handler` inline in the MMIO
exit. Only **2 of 24** request kinds are offloaded to QEMU's thread pool
(`src/qemu/virtio_nvgpu.c:1057-1059`, `:1115-1117`); `CREATE_ISOLATE`, `OPEN_NVIDIA_HANDLE`,
`MMAP_ON_ISOLATE` (with a full-length pre-fault loop, `nvkvm_isolate_handlers.c:4695-4699`),
`PRESENT`, `XISO_IMPORT` and a dozen more run inline with stub round trips up to 30 s
(`nvkvm_isolate.c:542`). Its own audits say so: *"dispatches inline on the TX handler with the
BQL held … every vCPU at its next BQL-taking exit"* (`docs/internal/audit-boundaries-2026-08-20.md:85-125`,
A-1 critical); *"holding QEMU's BQL across a 30-second round trip"*
(`docs/audit/2026-08-29-isolate.md:104-107`). ⇒ **Shipped is not compliant.** It is a
counter-example to the brief's *"if it has a working off-vCPU submission path"*: it has a partial
one, and the part that ships inline is the part this document is about.

**What it DOES prove, and it is the two halves this design rests on:**

1. **Memslots are setup; runtime is `MAP_FIXED` into one window with zero KVM ioctls.** One raw
   memslot over a 64–128 GiB sparse window, installed once (`src/qemu/nvkvm_mmap_host.c:639-645`);
   every GPU mapping is `mmap(MAP_SHARED|MAP_FIXED, h->fd, …)` inside it, tagged
   `NVKVM_IN_WINDOW_SLOT = -2` meaning *there is no memslot here*
   (`src/qemu/nvkvm_isolate_handlers.c:4600-4640`, `:4715`); an unmap restores anonymous backing
   (`:4851-4853`). The per-mmap-memslot path exists only inside an `#if 0` block
   (`virtio_nvgpu.c:310-658`). That is §2.1 exactly, at a scale of >1500 mmaps per `cuCtxCreate`,
   at 0.99–1.00× of host (`memory: the_doorbell_trap_is_the_whole_gap…`, the parity table).
2. **The guest waits on its own primitive; completion lands asynchronously.** For the two
   offloaded kinds the kick returns, the guest blocks in `wait_for_completion` in its own kernel
   (`src/guest/nvkvm_virtio.c:696`, `:713`), and the thread-pool completion writes the reply into
   guest memory and raises MSI-X (`virtio_nvgpu.c:732-746`: `virtqueue_push` + `virtio_notify`;
   4 vectors forced, `virtio_nvgpu_pci.c:96-99`). That is constraint 8's shape — completions land
   in VMM memory from a worker, the vCPU never waits — and it is the shape that measured parity.
   ⊘ It has no epoll loop anywhere (`grep -rn iothread\|epoll` → prose only); its reader thread
   per isolate blocks in `recvmsg` (`nvkvm_isolate.c:923-1270`, `:959`) and demuxes by `txn_id`
   onto per-caller condvars — the transport shape w675 found equivalent to our four sockets.

⇒ pv is strong evidence for **P1** (the window model) and for **the completion shape of §4**, and
no evidence at all that inline blocking is survivable — it survived it by being a control-plane
product whose steady state crosses no boundary, which Mode 2's PRAMIN and refresh planes do not
share.

---

## 6. What breaks if the work merely moves — the relocation trap, named twice already

Two measured instances in this tree of *"deferred, and the trap did not get faster"*:

- w395: `worst_trap` stayed 1.9 s across every async arm because the vCPU's
  `materialize_pending` — `state.write()` with nothing pending — waited on the **device write
  lock held by the off-vCPU worker** (`git show 2664967b`, `44c2cca9`: *"Deferring work off the
  vCPU RELOCATES the stall unless the worker also stops holding the lock the trap path takes"*).
- w735: the single store's wall was a **lock rank**, not a bandwidth
  (`THE_CONSTRAINTS.md` §w735) — the arm asserts lock-free and every store reader held
  `PlaneMem`.

Applied to §2: `repoint_pramin` takes `self.pramin` (an **unranked** `Mutex`, `barmirror.rs:440`,
`:1634`) for the whole move. Under P2 the worker's release tick must **never** hold it across the
release IPC, or a guest latch write on a vCPU waits on an IPC through a lock the witness cannot
see (`lockwitness.rs:11-13`: an unranked mutex is invisible to `assert_lock_free`). Under P3(a)/(b)
the retained-node table is written by the worker (arm) and read by the vCPU (`MAP_FIXED`): it
must be the trap contract's shape 1 or 2 — an atomic table keyed by base, or a leaf lock held for
the map lookup only (`the_write_trap_contract.md:49-58`) — never a lock spanning the arm. ⊘ These
are the two places where this design can become theatre: the census would go to one door and
`worst_trap` would not move.

---

## 7. The honest cost — latency versus throughput, and what is theatre

**Throughput: none of this buys any.** `[w675]` 1 worker, 1×4 workers, 4×1 isolates all measured
**1.00×** on 800 alloc+free pairs; RM's device-global lock is the serialiser
(`planreactor.rs:33-46`; `memory: rm_s_client_lock…`). PRAMIN's arm is one more RM verb behind
that lock; moving it between threads changes who waits, not how long. The reactor *"buys
liveness isolation, not throughput"* (`THE_CONSTRAINTS.md:48`).

**Latency: what the cuts buy, in the guest's terms.** Per boot the device arm spends
`22 × 7.2 ms ≈ 160 ms` of one vCPU frozen inside MMIO exits, with three exits over 10 ms and a
worst of 44 ms; each of the 19 memslot deletes is additionally *"charged to every vCPU"*
(`l1_os_shell.md:1941-1943`). P1+P2 are predicted to take a move to ≈ 0.36 ms mean (§2.3),
i.e. back to the sanctioned order; P3(b), if the base set is deterministic, to the arena's 63 µs.
⊘ 160 ms per boot is not a product problem by magnitude. The product argument is the *shape* —
*"a vCPU inside an MMIO exit is NOT preemptible … it freezes a core of the customer's VM"*
(`the_write_trap_contract.md:22-31`) — and the trap contract's whole point is that the same wall
time spent spinning on a shadow costs the guest nothing. PRAMIN is the one place that argument
does not apply: the guest *cannot* spin, it reads through the aperture on its next instruction.
So for PRAMIN the honest statement is: **the trap is un-deferrable, and the only levers are how
much work is in it (P1/P2) and whether the RM half can be paid earlier (P3).**

**Theatre, explicitly:**

1. An `epoll` worker for the PRAMIN path. There is no completion to wait on; a worker that arms
   and then *signals the vCPU* is a trap blocking on a worker, which is the shape the contract
   forbids. §4's loop is for constraint 10's channels, not for this.
2. Moving door 9 to a worker that holds `self.pramin` across the release (§6).
3. Declaring 25 met by counting: the `BlockingSection` allowlist is already empty and saw
   nothing (§2.6). A gate on the wrong census passes today.
4. Reading P1's `MAP_FIXED` as proven because the arena's is — the VMA kind differs
   (`VM_PFNMAP` from `nv-mmap.c:641`) and §8 P-2 is the only thing that turns *"should work"*
   into a measurement.
5. P3(b) sold as certain: it is one observed base (`0xfff00000`) and one ogkm reading; the
   determinism is a hypothesis with a cheap test.

**What is NOT theatre and is free:** P2's decline-by-name (one `if`, one counter) and the
read-path `mark_vcpu_thread` (§2.6) — both instrument corrections that make the next census
truthful before any mechanism moves.

---

## 8. Probes, each with the step that separates the right answer from a plausible wrong one

**P-1 — Where the 43 ms of CPU is.** On the w742 binary, time each of the nine doors of one
PRAMIN move (a per-door `Instant` pair inside `repoint_pramin`, printed once per move for the
first 25 moves). **Discriminator:** the nine phases must sum to that move's `move_ns` within
5 % — if they do not, the CPU is *outside* the doors (in `after_write`'s own path or the
plane lock) and §1.3's inference is wrong. **Negative control:** the arena arm's single phase
must reproduce its 63 µs mean. **Also record** `uname -r` on the bench and, on ≥ 6.12, whether
`KVM_X86_QUIRK_SLOT_ZAP_ALL` is disabled — that is the reading that would confirm or refute the
zap-all inference without touching our code.

**P-2 — The in-place re-point shows the NEW bytes (P1's correctness).** Through a worker-armed
host view, write sentinel `S1` at fb base `B1` and `S2` at `B2`. Guest: latch `B1`, read the
first dword through PRAMIN (expect `S1`); latch `B2`, read (expect `S2`); latch `B1` again, read
(expect `S1`). **Discriminator:** the third read. A stale EPT entry after the second re-point
would return `S2` at `B1`; a re-export that named the wrong node would return `S2` or zeros. Only
`S1` proves both that `MAP_FIXED` replaced the mapping under the live slot *and* that the
retained node still shows `B1`. **Negative control:** run the same sequence on the arena arm,
which is known to pass (`the_bar0_read_surface.md` §6c: *"the slot followed all 22 times"*).

**P-3 — Is the PRAMIN base set deterministic (P3(b))?** Print the 22 bases per boot; three boots
at 4096 MiB and three at 6144 MiB (`THE_CONSTRAINTS.md:711-725` — 6144 boots `(P)`).
**Discriminator:** byte-identical sequences *within* an FB size and a sequence *derivable* from
the profile across sizes (e.g. every base is `fb_end − k·MiB` or a WPR-relative offset). Identical
within a size but not derivable across sizes ⇒ (b) is a per-size table (constraint 12 forbids a
literal; it would have to be derived at start). Different within a size ⇒ (b) is dead and (a)'s
hit rate is the number to measure next.

**P-4 — The `NoWorker` refusal is reachable (§2.3's hazard).** Device arm, `pool=4`, drive a
publication burst that keeps four `fill_now` arms in flight (an LLM-scale premap does 1 849 arms
in this boot) while the guest is still moving the window. **Discriminator:**
`DEVICE-VIEW-PORT … first_refusal=[NoWorker …]` together with the *"RE-ARM REFUSED … NO
slot"* line — **and** a subsequent guest PRAMIN access counted as a trap (`pramin_reads`/
`pramin_writes` leaving zero). A `first_refusal` alone with the counters at zero means the
refusal fired on a path the guest did not then use; it must be paired with the trap.
**Negative control:** `pool=8` with the same burst must not refuse.

**P-5 — P2 releases the aperture, and never early.** With P1+P2, assert at teardown
`outstanding ≤ retained_K` on the port (`deviceview.rs` census `outstanding=`) and
`parked_declined > 0` (the vCPU declined at least once — the known-positive that the decline is
on the path). **Discriminator for "never early":** a release whose recorded re-point sequence is
greater than the last completed `MAP_FIXED` must be refused by the replacement assert of §2.1
item 1; inject one in a unit test and require red.

**P-6 — The control arm's 22 ms (§0.3).** `KAYFABE_KFTIME=census` on the arena arm; read
`KFTIME-SEG` for the `plane` and `materialize` segments at the `0x110c00` trap.
**Discriminator:** whichever segment's `max_us` matches `worst_trap` within ~100 µs owns it
(w395's method, `44c2cca9`). This is outside the nine doors and is listed so that fixing the nine
is not reported as *"constraint 4 met"* while the control still holds a vCPU for 22 ms.

---

## 9. What this document deliberately does not decide

1. **Whether the arm IPC (doors 1–3) is sanctioned inside the PRAMIN trap** once P1+P2 land — it
   is ≈ 300 µs mean with an RM-bound tail; the old sanction was 297 µs for *one syscall at boot*.
   Same order, different tail, different mechanism. Owner's call.
2. **Whether to spend K MiB of host BAR1 aperture on retained PRAMIN views** (P3(a)/(b)) against
   the headroom accounting in §w727, whose members are still *"not yet decided"*.
3. **Whether P3(b)'s determinism, if P-3 confirms it, may be encoded** as a derived boot-time
   table (constraint 12 permits *derived*, forbids *literal*).
4. **Whether constraint 25's gate is the boot grade or the build** — the lockwitness census can
   fail a boot today; making it a *build* failure needs the `OffVcpu`-typed reach of §2.6, which
   is a refactor of `DeviceViewPort`'s signature and every caller.
5. **Whether the control arm's 22 ms (§0.3) is in scope of "the w742 violation"** — it predates
   the single store and is CPU-bound, and no cut in §2 touches it.

⊘ Nothing above relaxes a constraint. P1 restores constraint 16's rule; P2 restores the
lock-free/off-trap placement `drain_view_releases` already documents for itself; P3 is the only
new mechanism, and it removes work from the trap rather than excusing it.
