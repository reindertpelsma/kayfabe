# The translated bus aperture — the first GMMU translation this port performs (`#149`)

> The rung after `#148`. `docs/design/boot_measured_2026_08_01.md` §27–§29 is the boot this
> answers; read it first.

## 1. The wall, in one sentence

`kbusVerifyBar2_GM107:4155-4230` writes sixteen bytes through the **BAR2 CPU mapping** — a
GMMU-*translated* aperture — and reads the same bytes back through the **untranslated** BAR0
moving window at their physical framebuffer address. Two apertures, one physical byte, in
both directions. Boot `l2evict1` (2026-08-01, rev `9551dd1`) read `0x0` where it had written
`0xabcdabcd`, and `RmInitAdapter` failed `NV_ERR_MEMORY_ERROR`.

⇒ there is no way past that statement without translating an address, and no way to
translate an address without a page-table format. Both were built.

## 2. Where the root comes from — a VALUE, not an address

★★★ The single most important fact about this rung, and the one that decides its shape.

On the firmware-offload model CPU-RM and the firmware own **different halves of one BAR2
address space**: CPU-RM owns everything under `PDE3[0]`, the firmware owns `PDE3[1]`, and
only the *firmware's* page directory is ever bound to hardware (`ogkm-580:
src/nvidia/src/kernel/gpu/bus/kern_bus.c:810-820`). So CPU-RM builds its own tree, reads
its own `PDE3[0]` **back out of the framebuffer through the BAR0 window** (`kern_bus.c:866-872`)
and sends that eight-byte value over as `UPDATE_BAR_PDE` (fn 70) for the firmware to install
in slot 0 of its own root (`kern_bus.c:880`).

In Mode 2 **we are the firmware.** There is therefore no page in any framebuffer holding our
root directory — the only fact this port ever receives about that tree is one *entry*.

- `kayfabe_mmu::walker::translate_from_entry` is the primitive that expresses it: a descent
  that starts from a decoded entry rather than from a table page. That is not a shortcut —
  it is exactly the state hardware would be in after the firmware wrote the entry.
- `kayfabe_device::bar2::BarPdeLog` latches it. `entryLevelShift` travels with the entry and
  the level is **derived** from it, never assumed to be zero: the eight bytes alone do not
  say which format row they belong to, and a dual entry and a single one do not share a
  layout.
- An access whose root-level index is not the published slot is refused **by name**
  (`BAR2_OUTSIDE_PUBLISHED_SLOT`). The other slots are the firmware's half and this port has
  published none of its own.

⊘ **Nothing reverse-derives a root.** The C artifact snoops framebuffer writes whose low
twelve bits are `0x200` and calls the containing page an instance block (`C:
src/qemu/nvkvm_gpu_emul.c:3757-3769`) — a heuristic over guest bytes. This port does not
need one: the guest *published* the root, which is `mode2_address_table.md`'s forward
direction.

⚠ `[inferred]` from the guest's source, not from a run: `NV_PBUS_BAR2_BLOCK` — the other way
a root could arrive — is written only when `bUsePhysicalBar2InitPagetable`, which is set in
exactly one place, `IS_VIRTUAL_WITH_SRIOV(pGpu)` (`ogkm-580: kern_bus.c:58-61`). This device
is not an SR-IOV virtual function, so a stock guest never writes it. If one ever does the
aperture stays unrooted and says so, rather than translating against a stale root.

## 3. ONE store, and the test a second one would fail

The page-table pages this walk reads come out of `PlaneState::fb` — **the same `FbStore` the
BAR0 moving window writes into** — because that is where the guest put them.
`kbusSetupBar2GpuVaSpace_GM107` builds the whole BAR2 tree through the BAR0 window *before*
it publishes the root, and the leaf the walk resolves is read back through that same window
by `kbusVerifyBar2`.

`RegPlane::FbStoreReader` is the whole of the join: a **borrow** of the one store wearing
`kayfabe_mmu::walker::FbRead`. A borrow cannot outlive the store, cannot copy it and cannot
become a second one.

★ `tests/tests/bar2_translation.rs::the_translated_aperture_and_the_window_share_one_framebuffer_store`
is the assertion, and its instrument is a page count: a device with a second store behind the
translated aperture would hold **more** pages than the window ever wrote.

★★ The whole page-table tree in that file is written **through the BAR0 moving window**,
dword by dword, exactly as the guest writes it. If the walk read from a second store, or if
the window resolved an address the walk did not, every assertion would still be satisfiable
and the guest's own test would still fail.

## 4. The two GA10x traps, encoded rather than commented

Both cost weeks once (`#13`; `resume_from_fault.md` §6 hole 7), and both are in
`kayfabe_chips::Ga10xGmmu`:

| trap | where | what a wrong answer does |
|---|---|---|
| **`PD0`'s entry is 16 bytes and names TWO sub-tables** (`ogkm-580: dev_mmu.h:112`, `kern_gmmu_fmt_gp10x.c:107-137`) | `entry_size(3) == 16`; `PteDecode::Pde::also` carries the second edge; the point query follows **both** | drops a whole sub-tree with no diagnostic |
| **`PD1` is itself a 512 MiB LEAF level on GA10x** — the whole generation delta is `pLevels[2].bPageTable = NV_TRUE` (`ogkm-580: kern_gmmu_fmt_ga10x.c:52`) | the `L_PD1` arm asks the valid bit first | a design keyed on *"leaves are PTEs"* silently drops every such mapping |

And the quieter third: **the PDE aperture table and the PTE aperture table are not one
table**. A PDE's `1` is video memory and a PTE's `0` is (`ogkm-580:
kern_gmmu_fmt_gm10x.c:165-201`). A decoder sharing one function between them puts every leaf
one aperture out, and an aperture is half of every physical-address key in this port.

★ 512 MiB is **enumerated** in `page_sizes()`. Leaving it out to dodge the whole-framebuffer
identity alias would turn a decodable leaf into `UnknownLeafSize` and rebuild `#13`'s drop at
the other end. The alias is declined by *policy*, at the binding site
(`walker::leaf_disposition`), and a translated aperture access is not a binding — so
`translate` deliberately does not consult it.

## 5. Why `translate` is not a second walker

It shares everything that can be wrong with `decode_page`: the same `GmmuFmt::decode_entry`,
the same `level_shift` geometry, the same `FbRead` source, the same `MISS = FAULT` rule and
the same `#13` leaf-size check. What differs is only the **question**:

- `decode_page` / `decode_subtree` ask *"what does this table contain"* — which is what a
  reachability shadow (`reach::ReachShadow`) is built from, and it reads whole pages;
- `translate` asks *"where does this address land"* — which is what a hardware TLB miss
  asks, and it reads one entry per level.

Neither can answer the other's question and neither re-implements the other's decode. ⊘ And
the compute path could not be served by the capture machinery even in principle here: the
guest writes BAR2 PTEs **through BAR2 itself** after bootstrap
(`kbusUpdateRmAperture_GM107`), so a design that needed the tree captured before it could
translate would have to translate first.

## 6. The refusals, and why none of them is a zero

| refusal | when |
|---|---|
| `NO_MMU_PORT` | the shell never called `set_mmu` — a **wiring** fact, not a guest one |
| `BAR2_UNROOTED` | no `UPDATE_BAR_PDE` has arrived |
| `BAR2_UNKNOWN_ROOT_LEVEL` | the published `entryLevelShift` names no level of this format |
| `BAR2_OUTSIDE_PUBLISHED_SLOT` | the address indexes a root slot the guest never published |
| `BAR2_FOREIGN_APERTURE` | the leaf names memory this port does not serve through the window |
| `BAR2_READ_ONLY` | a **write** to a mapping the guest itself marked read-only |
| `TranslateFault::{Unmapped, Sparse, TooDeep, Walk}` | the walk's own answers, kept distinct |

⊘ **Identity is the tempting wrong answer** and it is not taken. The C artifact falls back to
identity whenever its snooped `bar2_virtual` flag is clear (`C: nvkvm_gpu_emul.c:6588`); an
identity aperture would put `kbusVerifyBar2`'s write at framebuffer address `TEST_VA` — a
real, writable, completely wrong page — and the guest's read-back would find zero with no
other symptom.

⊘ **Sysmem leaves are refused, not served.** This port has a guest-RAM port that could serve
them, but nothing on this boot path maps sysmem through the bus window (every memdesc
`kbusVerifyBar2` and `kbusSetupCpuPointerForBusFlush` allocate is `ADDR_FBMEM` —
`ogkm-580: kern_bus_gm107.c:4050`, `kern_bus_gv100.c:66-72`), so serving it would be an
untested path answering a case nobody has seen. It is a named refusal with a counter, which
is what makes the day it *is* wanted a number rather than a mystery.

## 7. What is observable from outside the process

`nvkvm: BAR2 (translated): R reads / W writes resolved through the GMMU, F REFUSED by name;
roots published N (M bodies refused), BAR2 root entry 0x…`

★★ Three questions, three numbers, and they are not interchangeable. `roots published`
answers *did `UPDATE_BAR_PDE` ever arrive* — the guest **ignores that command's status**
(`kbusPatchBar2Pdb_GSPCLIENT` assigns it and returns `NV_OK`), so this is the only
observable. `bar2_faults` answers *was a translated access refused by name*. `reads/writes`
answer *did a page walk actually resolve one*. `kbusVerifyBar2`'s `NV_ERR_MEMORY_ERROR`
cannot tell those three apart; these can.

## 8. ⊘ What this rung does NOT do

- ⊘ **BAR1 is unchanged.** Its root is *recorded* (the guest publishes both through the same
  command) and nothing translates it. Recording a root is not serving an aperture.
- ⊘ **No compute, no execution plane.** `Ga10xArch`'s `userd()`, `pushbuffer()` and
  `decode_doorbell` are still unbuilt and still refuse. What exists is an address decoder.
- ⊘ **`ReachShadow` and the address table are untouched.** Nothing here binds a leaf into
  `AddressTable`; a translated aperture access is not a publication.
- ⊘ **No page-table page is written through BAR2 itself** in any test, which is what
  `kbusUpdateRmAperture_GM107` does after bootstrap. That path is reachable and unexercised.
- ⊘ **The tree in the tests is one shape.** A real guest's BAR2 tree is built by `mmuWalk`
  and carries reserved and sparse fills these fixtures do not construct.
