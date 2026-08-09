# The ring-write path map — every route a guest byte can take into (or past) the framebuffer

**Pinned at `ebd63ec` + the working tree of 2026-08-09** (the tree was dirty while this was
written: `fbwin.rs`, `plane.rs`, `shim.rs`, `kayfabe_shim.h`, `nvkvm.c` carried the §16.13
residency-census edit. Line numbers were re-checked against the working tree immediately before
writing and the §16.13 diff adds **no** new base-address-register spelling, so nothing below
depends on it.)

Boot cited throughout: **`bar1_03a679f`**, `traces/guest_boots/run_bar1_03a679f_qemu.log`.

Claim tags: `[code]` = read at the cited `file:line` in this tree. `[boot]` = a number in a
committed boot log. `[ogkm]` = read in `research_clones/ogkm-580.159.04`. `[INFERRED]` = reasoned
from those. `[ASSUMED]` = neither.

---

## 0. What this document REFUTES — including its own brief

### R1. ⊘ The brief's leading candidate is dead, and it was already dead when the brief was written

The brief names the BAR1 **shadow memslot** (`nvkvm.c` "a plain RAM slot with no connection to the
framebuffer") as the leading candidate and asks whether it exists. `[code]` It cannot: the shadow
is installed at exactly one site, `nvkvm.c:1210` gated `if (s->window_size != 0)`, and
`window-size` defaults to `0` at `nvkvm.c:2324`; `scripts/bench/boot_nvkvm.sh` never sets it.
`[boot]` `run_bar1_03a679f_qemu.log:16` prints the precondition inline: *"NO shadow is installed
(window-size=0), so this IS a complete census of BAR1 traffic"* — **3 accesses, whole boot**.
`[code]` HEAD's own commit subject (`ebd63ec`) already says *"BAR1 innocent after all"*.

⇒ Do not spend another rung on the shadow. It is not merely absent from this boot; it is absent
from every boot the bench script can produce.

### R2. ⊘ The brief's correction to §16.5 is itself half wrong

The brief says the acquittal of BAR1 "was misread" because `reservation_touches` is a **miss**
counter. `[code]` The premise is right (`nvkvm.c:496`, `:507` are the only increment sites, inside
the *fallback* handlers). The conclusion does not follow: over a range with **no shadow**, "miss"
and "total" are the same set — every access reaches the fallback. §16.5's *reasoning* was unsound;
its *verdict* was correct, and §16.12's instrument is what makes that visible rather than lucky.

★ The transferable rule is narrower than the brief's: a conditional counter needs its condition
printed. It does not need its verdict retracted.

### R3. ⊘ "The publication declares only the top four levels" is not an anomaly to be explained

`[ogkm]` `0x90f10106` is `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES`
(`src/common/sdk/nvidia/inc/ctrl/ctrl90f1.h:268`), **every field `[in]`** (`:272-332`). Its sole
producer is `_gvaspacePopulatePDEentries`
(`src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:5219-5259`), which sets
`pdeInfo.pageSize = NVBIT64(GMMU_PD0_VADDR_BIT_LO)` — **2 MiB** — at `:5235`. `gvaspaceGetPageLevelInfo`
then walks down to *the level whose page shift is 21* and stops (`:3978`, `mmuFmtFindLevelWithPageShift`).

⇒ Four levels is **the control's specification**, not a truncation: `PD3, PD2, PD1, PD0`. The
leaf table is deliberately excluded. `[boot]` `pageSize 0x200000 levels 4` on all eleven rows,
exactly as specified.

### R4. ⊘⊘ The publication does not describe the ring's page tables at all — and it never claimed to

`[ogkm]` `gpu_vaspace.c:5236` — `pdeInfo.virtAddress = pGVAS->vaStartServerRMOwned`; `:5254-5256`
— `virtAddrLo = vaStartServerRMOwned`, `virtAddrHi = lo + SPLIT_VAS_SERVER_RM_MANAGED_VA_SIZE - 1`.
The two constants are `0x100000000` (4 GiB) and `0x20000000` (512 MiB)
(`src/nvidia/generated/g_gpu_vaspace_nvoc.h`). `[boot]` The log's row reads
`va [0x0000000100000000..0x000000011fffffff]` — **byte-identical**.

The ring is at `0x121010000`. `0x11fffffff + 1 = 0x120000000`, so the ring sits **0x1010000
(≈16.1 MiB) past the top of the published range**.

`[ogkm]` `gpu_vaspace.c:4122-4126` states the split-VAS contract verbatim: *"Any lower level
PDEs/PTEs allocated under these top level PDEs will be modified **exclusively by server RM.
Client RM won't touch those.**"*

⇒ `[INFERRED]` The publication's `levels[]` are the `PD3/PD2/PD1/PD0` instances **on the path to
VA `0x100000000`**, the *server-owned* half. The ring lives in the *client-owned* half. Only
`levels[0..2]` are shared with the ring's descent — and the boot's own walk trace proves the
divergence at `PD0`:

| | publication | walk to `0x121010000` |
|---|---|---|
| `PD3` (L0) | `0x4000` | `0x4000` ✔ shared |
| `PD2` (L1) | `0x5000` | `0x5000` ✔ shared |
| `PD1` (L2) | `0x6000` | `0x6000` ✔ shared (slot 8 vs slot 9 of one page) |
| `PD0` (L3) | `0x7000` | **`0x8000`** ✘ a *sibling*, never published |
| leaf (L4) | *(not published by design)* | `0xa000` → `0x20000/Vidmem/sz0x10000` |

★ This **strengthens** the walk's exoneration rather than weakening it: the walker correctly
ignored the published `0x7000` and followed the entry it actually read. But it retires a framing —
"the guest published this tree and we walked it" is false. The guest published *three quarters of
one branch of a different sub-tree*; `0x8000` and `0xa000` reached our store by an entirely
different route, and identifying that route is the same question as identifying the ring's.

### R5. ⊘ The aperture-confusion hypothesis (my own first candidate) is REFUTED at the decode site

`[code]` `crates/kayfabe-chips/src/ga10x.rs:614` `pde_aperture` (`0=INVALID→None, 1=Vidmem, 2=SysCoh,
3=SysNonCoh`) and `:629` `pte_aperture` (`0=Vidmem, 1=Peer, 2=SysCoh, 3=SysNonCoh`) are **two
separate functions**, and `:623-628` says why in as many words. This is exactly the trap recorded
in `two_encodings_agreeing_on_the_first_values`, and it is closed here. The ring's leaf really does
say Vidmem.

### R6. ⊘ "The ring is never written by the guest at all" is NOT yet supportable

The brief invites this answer. `[code]` It cannot be reached from any current instrument:
`SparseFb::read` returns **zero and `Ok`** for a page nobody wrote (`fbwin.rs:518`), so
`fbRING@0x20000 nz0/4096` cannot distinguish *never written* from *written with zeros*. §16.13's
`FbResidency` (in flight in the working tree) is precisely that discriminator. Until it lands,
"the guest never wrote it" is `[ASSUMED]`.

---

## 1. The stores — five byte-holders, and only two are on the address plane

| # | store | concrete type / backing | written by | **read by the address plane?** |
|---|---|---|---|---|
| **S1** | emulated framebuffer | `SparseFb` = `HashMap<u64, Box<[u8;4096]>>` on the shell heap, `fbwin.rs:426-434`; installed once at `shim.rs:2961` | `plane.rs:2271` (BAR0 window + BAR2), `cpu_ce.rs:119` (CPU-CE), `plane.rs:1951` (reset = erase) | **YES** — `FbStoreReader` (`plane.rs:1035-1042`) is the *entire* join to `walker::FbRead`; PTE fetches at `walker.rs:192`, page fetches at `walker.rs:550` |
| **S2** | guest RAM (sysmem) | QEMU's own `MemoryRegion` RAM, reached by `memcpy` against `memory_region_get_ram_ptr` (`nvkvm.c:988-1009` read, `:1011-1037` write) | the guest's own vCPU (native, no trap); `Vmm::gpa_write`; the emulated GSP via `MachineRam` (`shim.rs:~3064`) | **YES** — the `Aperture::Sysmem*` arm of `read_published_va` (`plane.rs:~1490`) and `ceutils.rs:394` |
| **S3** | BAR1 flat aperture | **no backing at all.** `nvkvm.c:500-508` `nvkvm_reservation_write` — the parameter is `(void)val;` | the guest, 3 times `[boot]` | **NO — the bytes cease to exist** |
| **S4** | BAR1 *shadow* memslot | anonymous `mmap` (`GuestWindow::create`), optionally `MAP_FIXED`-overlaid by a sealed `memfd` (`SharedRam`) — `kayfabe-vmm-qemu/src/lib.rs:1225`, `:1229-1250` | the guest natively, **if installed** | **NO** — and it is **not installed** (R1) |
| **S5** | isolate "fabricated aperture" | **does not exist.** `IsolateFb` (`kayfabe-fwd/src/ptdecode.rs:145`) is the production `FbRead`; its backend answers `NOT_ON_THIS_RUNG` (`kayfabe-isolate-host/src/rm.rs:2396`) | **nothing — there is no writer anywhere in the tree**, by design (`walker.rs:40`: `FbRead` has no write method) | read-only seam, currently refusing |

★ **S1 and S2 are the only two the address plane reads.** S3 destroys bytes; S4 would hold them
where nothing looks; S5 is the mirror image — a reader with no store behind it.
`[boot]` `isolates: 2 materialized … 2 no-plane` confirms S5 is inert this boot.

---

## 2. The paths — every entry point, in order of how many bytes it carries

### P1. BAR0 moving window (PRAMIN) → **S1** ✔ connected

- **Entry**: guest MMIO store into BAR0 at `0x700000..0x800000` → `nvkvm_trap_write`
  (`nvkvm.c:304`) → `kayfabe_shim_regs_write(…, KAYFABE_BUS_BAR_REGS, …)` (`nvkvm.c:311`)
  → `RegPlane::write` → `ChipProfile::fb_window` (`kayfabe-device/src/lib.rs:604-616`)
  → `FbWindow::Pramin`.
- **Address**: `window_phys` → `Bar0Window::fb_addr` (`plane.rs:2105`, `fbwin.rs:176`) =
  `(BASE<<16) + off`. Window register `NV_PBUS_BAR0_WINDOW` at `0x1700` (`ga10x.rs:198`).
- **Store**: `plane.rs:2271` → `SparseFb::write` (`fbwin.rs:526`, allocate-on-write at `:552`).
- **Read by the plane?** Yes — same `SparseFb`.
- **Would the ring VA route here?** Only indirectly. PRAMIN is addressed by *framebuffer physical
  address*, not by GPU VA, so RM would have to point the window at `0x20000` and store there.
  `[boot]` 337 854 window data writes / 38 window-register writes occurred; nothing records
  **which** addresses.
- **Tell from one boot**: a first-writer attribution per resident page, or simply
  `bar0_window` base-value histogram. Neither exists.

#### ★★★ P1 carries a live defect: `NV_PBUS_BAR0_WINDOW_TARGET` is decoded and never consulted

`[code]` `Bar0Window::target()` is defined at `fbwin.rs:155-157` and its own doc at `fbwin.rs:97`
gives the encoding (`0` vidmem, `2` sysmem coherent, `3` sysmem non-coherent). Its **only** caller
in the tree is a test assertion, `crates/kayfabe-device/tests/bar0_window.rs:286`. `window_phys`
(`plane.rs:2105`) ignores it.

`[ogkm]` The guest genuinely moves that field: `kbusVerifyBar2_GM107` sets
`testAddrSpace = kgmmuGetHwPteApertureFromMemdesc(…)` from the memdesc
(`src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:4073`) and writes it into
`_PBUS_BAR0_WINDOW_TARGET`; the same read-modify-write/restore pair appears twice in that file.
RM allocates BAR2 page tables in sysmem whenever `VASPACE_FLAGS_RETRY_PTE_ALLOC_IN_SYS` fires.

⇒ `[INFERRED]` Any PRAMIN access issued while `TARGET != VID_MEM` is filed into the **framebuffer**
at `(BASE<<16)+off`. It is a wrong-store write, and — because `fb_read` (`plane.rs:2202`) resolves
through the *same* one address function — the guest's read-back **agrees**. `kbusVerifyBar2`'s
read-after-write therefore passes over a mis-filed byte. ★ This is the "self-consistent wrong
store" shape: an instrument built to catch a lost write cannot catch a *misplaced* one when both
directions share the mistake.

⊘ This does not by itself explain an empty ring page — it produces *extra* bytes in S1, not
missing ones. It matters because it means **we do not know what the 337 854 window writes were
aimed at**, and one plausible consequence is precisely inverted: page-table pages the guest placed
in *sysmem* would appear in our framebuffer at plausible-looking low offsets. `[ASSUMED]` — a
target census would settle it in one boot and none exists.

---

### P2. BAR2 instance window (GMMU-translated) → **S1** ✔ connected

- **Entry**: guest MMIO store into BAR2 → `nvkvm_bar2_write` (`nvkvm.c:457`) →
  `kayfabe_shim_regs_write(…, KAYFABE_BUS_BAR_INST, …)` (`nvkvm.c:464`) → `FbWindow::InstanceWindow`.
- **Address**: `bar2_phys` (`plane.rs:2134-2199`) — a real page walk from the root the guest
  published over `UPDATE_BAR_PDE`, using `FbStoreReader` over the *same* `SparseFb` (`plane.rs:2177`;
  the "ONE STORE, TWO APERTURES" rationale is at `plane.rs:2118-2127`).
- **Store**: `plane.rs:2271`.
- **Read by the plane?** Yes.
- **Would the ring VA route here?** `[INFERRED]` **This is the expected route.** A kernel-RM
  channel's GPFIFO ring in vidmem is CPU-mapped through `kbusMapRmAperture_HAL`, i.e. BAR2 — not
  BAR1, which is the *user-mode* aperture. `[boot]` 286 352 BAR2 writes resolved, **0 refused by
  name**, 0 foreign-aperture refusals, 0 read-only refusals.
- **Three named refusals that would have been visible and were all zero** `[code]`:
  `BAR2_OUTSIDE_PUBLISHED_SLOT` (`plane.rs:2171`), `BAR2_FOREIGN_APERTURE` (`plane.rs:2186`),
  `BAR2_READ_ONLY` (`plane.rs:2192`) — all funnel to `bar2_faults` (`plane.rs:2251`), printed as
  *"0 REFUSED by name"*.
- **Tell from one boot**: ★ **the cheapest decisive rung available.** One counter —
  *"how many BAR2 writes resolved into `[ring_phys, ring_phys+64 KiB)`"* — or, generically, the
  set of distinct 4 KiB frames `plane.rs:2271` ever wrote, partitioned by `FbWindow`. That converts
  "the page is empty" into "**no BAR2 write was ever aimed at it**", which is a statement about the
  *guest* rather than about our store.

---

### P3. BAR1 flat framebuffer aperture → **nowhere** ✘

- **Entry**: guest MMIO store into BAR1 → `nvkvm_reservation_write` (`nvkvm.c:500-508`).
  The value parameter is discarded: `(void)val;`. Only `s->reservation_touches++` survives.
- **Store**: none. The bytes cease to exist.
- **Read by the plane?** No, in the strongest sense.
- **Would the ring VA route here?** `[INFERRED]` Only if the guest CPU-mapped the ring through the
  *user-mode* BAR1 aperture. `[boot]` It did not: 3 accesses, complete census.
- **Tell from one boot**: already told — `nvkvm.c:1462-1468`.

#### ★★★★ P3 has a SECOND, UNREACHABLE arm, and its counter is a structurally-zero number that the report describes wrongly

`[code]` `ChipProfile::fb_window` (`kayfabe-device/src/lib.rs:610`) maps
`bus_bar::FB` (= 1, `kayfabe-abi/src/pcibars.rs:139`) to `FbWindow::FbAperture`, and
`window_phys` answers it `Err(WindowRefusal::NoAddressModel)` (`plane.rs:2110`). That arm — and
**only** that arm — increments `fb_window_reads` (`plane.rs:2005`) and `fb_window_writes`
(`plane.rs:2242`).

`[code]` **`bar == 1` can never reach `RegPlane`.** `kayfabe_shim.h:948` and `:952` define exactly
two names — `KAYFABE_BUS_BAR_REGS 0u` and `KAYFABE_BUS_BAR_INST 2u`. There is **no
`KAYFABE_BUS_BAR_FB`**, and all four call sites in `nvkvm.c` (`:301`, `:311`, `:454`, `:464`) pass
one of those two. BAR1 is registered with `nvkvm_reservation_ops` (`nvkvm.c:532`), which never
crosses the seam at all.

⇒ **`fb_window_reads` / `fb_window_writes` cannot move.** `[boot]`
`run_bar1_03a679f_qemu.log:15` prints them as *"translated-window drops 0r/0w"*, and
`nvkvm.c:1414-1417` explains that number as *"counts the two GMMU-TRANSLATED windows … the other
says 'we are there and we lost bytes'"*. **Both halves are false**: it counts **one** window
(BAR2's refusals go to `bar2_faults`), and that one window is unreachable. Its zero is vacuous —
the `pgrep -x qemu-system-x86_64` shape, one plane over.

★ This is the brief's own discipline applied to a *different* counter than the one already burned:
`0r/0w` on the "translated-window drops" line is **not** evidence that no translated window lost
bytes. Either delete the `FbAperture` arm as dead, or give it a caller; leaving it costs a reader
a false acquittal every time they read that line.

---

### P4. Guest RAM, natively → **S2** ✔ connected

- **Entry**: the guest's own vCPU store to ordinary RAM. No trap, no counter, no record — QEMU's
  memslot serves it.
- **Store**: S2.
- **Read by the plane?** Yes: `read_published_va`'s sysmem arm (`plane.rs:~1490`) and
  `ceutils::read_va`'s `CpuPlane::GuestRam` arm (`ceutils.rs:394`), reached through
  `Aperture::SysmemCoherent|NonCoherent → CpuPlane::GuestRam` (`kayfabe-fwd/src/lib.rs:3193-3200`).
- **Would the ring VA route here?** Only if the leaf PTE said sysmem. It says Vidmem (R5), and the
  *sibling* CeUtils channel on this same boot proves the arm works: `[boot]`
  `sem fin va=0x42006c004 -> S:0x1e8e3004`, served, `cpu-ce: 1 gp, 9 methods, 1 launch`.
- ★ **This is the discriminator worth stating plainly**: on one boot, the channel whose ring is in
  **sysmem** was read, decoded and executed; the channel whose ring is in **vidmem** read as all
  zeros. The failure is aperture-correlated, and S1 is the only store on the failing side.

---

### P5. The archive writing guest RAM → **S2** ✔ connected

`nvkvm_op_write_region` (`nvkvm.c:1011-1037`) — a bounded `memcpy` into a QEMU `MemoryRegion`,
refusing ROM/read-only/ram-device regions, followed by `memory_region_set_dirty`. Serves
`Vmm::gpa_write`, which the emulated GSP uses for msgq/LibOS/fault packets. Lands in the same S2
the plane reads. Not a candidate for the ring (nothing here is addressed by GPU VA).

---

### P6. The shell's CPU copy-engine executor → **S1** and **S2** ✔ connected

`SharedDoorbell::ring` → `try_ce_submission` (`shim.rs:2466`) → `ceutils::run_submission` →
`cpu_ce::write_plane` (`cpu_ce.rs:111`), which dispatches `CpuPlane::Fb → fb.write` (`cpu_ce.rs:119`)
or `CpuPlane::GuestRam → vmm.gpa_write` (`cpu_ce.rs:120`).

⊘ **This is a writer, not a reader-gap** — and it is *downstream* of the ring. It cannot fill the
ring; it consumes it. `[boot]` 4 doorbells served locally, 1 refused (`RingBroughtNoEntry`).

---

### P7. Page-directory publication (`0x90f10106` / `0x20800a9f`) → **no store** ✔ correct

`[code]` The publication is *recorded* (`kayfabe-device/src/gvaspub.rs`) and answered `NV_OK`;
`nvkvm.c:1498-1501` states it outright — *"OBSERVED, never answered … today we do nothing with
it."* No byte enters any store on this path. `[boot]` 12 total, 10 accepted into `Vas::pdb`.

★ It is worth being explicit that this is **correct**, not a gap: the control's contract (R4) is
that *server RM* — us — owns the PDEs under `[0x100000000, 0x11fffffff]` and will write them.
We write none. That is a real unbuilt obligation, but it is a different one: the ring is outside
that range, in the half the guest's own client RM manages and demonstrably did populate (the walk
found live entries at the unpublished `0x8000` and `0xa000`).

---

### P8. Isolate / host-side writes → **S5**, which has no bytes ✘ (inert)

`export_backing` / `ExportRequest` have **zero production callers** outside
`kayfabe-isolate*`/mocks/loopback. `IsolateFb`'s backend refuses (`rm.rs:2396`). `[boot]` both
isolates are `no-plane`. Nothing can enter or leave here on this build.

---

### P9. Built-but-unwired memslot machinery → would be **S4-like** ✘

`ViewInstaller::drain_and_install` / `install_tiered_window` (`viewer_install.rs:813-819`) and
`Vmm::map_read_native` (`kayfabe-vmm-qemu/src/lib.rs:2325-2352`) can install slots over
`FbWindow::Pramin` / `FbAperture` / `InstanceWindow` apertures. `[code]` Neither has a
non-test caller. ★ Listed because if either is ever wired, it creates a *third* store with S4's
property — guest bytes in host RAM that the GMMU walker cannot see — and the `Tier::Observe` arm
(`slots.rs:349-355`, no slot) would silently route the remainder into P3's discard. That is the
one change that could re-open the refuted candidate, so it should not be wired without a
corresponding read path.

---

## 3. Where a write to the ring VA would actually go, and what decides

`[boot]` The leaf is `LEAF@0x121010000 -> 0x20000/Vidmem/sz0x10000` — a 64 KiB big page, decoded
by `L_PT_BIG` (`ga10x.rs:864`; 32 entries at `ga10x.rs:803`, consistent with the trace's
`ch0 lf2 sp0 inv30`).

The routing decision is made in **two** places and nowhere else:

1. **The guest's own choice of aperture**, when it CPU-maps the allocation. Vidmem ⇒ P1 or P2;
   sysmem ⇒ P4. `[boot]` It chose vidmem, so P4 is out.
2. **`ChipProfile::fb_window(bar, off)`** (`kayfabe-device/src/lib.rs:604-616`) — the only
   classifier — and then `window_phys` (`plane.rs:2097-2113`), the only address resolver.

`[INFERRED]` ⇒ **P2 (BAR2) is the route the ring's bytes should take**, and it is fully connected
in both directions to the store the walker reads. P1 is a possible alternate route and is equally
connected. P3 would destroy them but was touched 3 times. There is **no fourth possibility** that
reaches a store the plane does not read — S4 and S5 are both out by configuration, and P9 is
unwired.

⇒ **The wall is not a wrong-store problem.** Every reachable write path for a vidmem address lands
in `SparseFb`, and `SparseFb` is what the walker reads. `[INFERRED]` The remaining possibilities
are, in order of what the evidence supports:

| | hypothesis | what would confirm it | status |
|---|---|---|---|
| **H1** | No write was ever *aimed* at `0x20000` — the guest's CPU mapping of the ring was never established, or was established against something we never served | the §16.13 residency census showing frame `0x20` **absent**, plus a per-`FbWindow` frame set showing no P1/P2 write in `[0x20000,0x30000)` | **best supported**; `bar2_faults = 0`, `fb_refusals = 0`, `reservation_touches = 3` leave no lossy path with traffic on it |
| **H2** | Writes were aimed at it and landed at a *different* framebuffer address — P1's ignored `TARGET`, or a BAR2 walk resolving to the wrong leaf | the same frame set showing writes to frames adjacent to / unrelated to `0x20` in the same time window; a `TARGET` census on P1 | open; `[ASSUMED]` — no instrument distinguishes it today |
| **H3** | The ring page was written and then erased | `RegPlane::device_reset` → `s.fb.device_reset()` → `pages.clear()` (`plane.rs:1951`, `fbwin.rs:580`). A reset count beside the residency census | cheap to exclude, currently unexcluded |

---

## 4. Instrument findings, ranked (these are the reusable output)

1. ★★★★ **`fb_window_reads`/`fb_window_writes` — printed as "translated-window drops" — cannot
   move** (§2/P3). Its zero is vacuous and the comment describing it is wrong on two counts.
2. ★★★ **`Bar0Window::target()` is dead** (`fbwin.rs:155`, sole caller a test at
   `tests/bar0_window.rs:286`). Every PRAMIN access is filed as vidmem regardless of what the guest
   pointed the window at, and the read path shares the mistake, so a read-back cannot detect it.
3. ★★★ **`SparseFb::read` conflates "never written" with "written zero"** (`fbwin.rs:518`). Named
   here for completeness — §16.13 is closing it in the working tree.
4. ★★ **There is no per-address record on either connected write path.** 624 206 writes landed in
   `SparseFb` and the only spatial fact any report carries is a *total* (`resident 368640 bytes`)
   and, soon, a `lo/hi/pages` census. Neither can say *which window* wrote a frame. The single most
   informative addition is a **first-writer tag per resident frame** (`FbWindow` + a sequence
   number), which answers H1, H2 and H3 at once and costs one `u32` per page.
5. ★ **`docs/design/execution_plane_increments.md:7576` says `SparseFb` holds pages in a
   `BTreeMap`. It is a `HashMap`** (`fbwin.rs:433`). Harmless for correctness; the `lo/hi`
   residency census was designed against the wrong statement and is `O(n)`, not `O(1)`.

---

## 5. What NOT to do next

- ⊘ Do not widen or re-measure the BAR1 shadow (R1, R2).
- ⊘ Do not re-derive the aperture decode (R5) — it is correct and documented.
- ⊘ Do not treat the `0x90f10106` publication as describing the ring's page tables (R4); in
  particular, do not "fix" the walk to honour the published `levels[3] = 0x7000`, which is a
  sibling `PD0` on a different branch. The walker is right to ignore it.
- ⊘ Do not conclude "the guest never wrote the ring" from `nz0/4096` (R6).
