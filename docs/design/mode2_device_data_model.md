# Mode-2 device data model + auto-generation from NVIDIA source

## Data-source PRIORITY (user, 2026-06-03 — canonical ordering)
For every value we must hand the guest driver, prefer sources in this order
(cheapest / most-trustworthy / most-portable first). This supersedes the
by-source A/B/C/D taxonomy below as the *decision procedure* — A/B/C/D still
describe the kinds of data; this says which source to reach for first.

1. **Static** — identical across all GPU families, or a handful of arch branches.
   Compile-time constants. (e.g. GMMU walk algorithm, GSP-RPC msg-queue protocol,
   the interrupt subtreeMap which is computed from arch constants.)
2. **Unprivileged host ioctl, collected at Mode-2 boot.** The host GPU is present
   (Mode-2 forwards compute to it), so query it through *unprivileged* NVML/CUDA/
   RM ioctls and fill the structs: GPU name, VRAM size, chip id / architecture /
   PCI ids, compute capability, #SMs, clocks, ECC. MOST trustworthy and MOST
   likely to work on a new card — anything observable from unprivileged guest
   userspace is, by construction, correct. nvidia-smi mostly surfaces exactly
   these (name + a few caps). KEEPS QEMU UNPRIVILEGED.
3. **NVIDIA kernel source, auto-generated.** Per-arch register offsets, GMMU/PTE
   formats, instblk field offsets, alloc-param/ctrl-struct sizes. Selected AFTER
   (2) tells us the target chip. Re-generate when NVIDIA ships a new card/driver.
   (tools/mode2_gen_regs.py.)
4. **Synthetic / minimal-to-pass.** Data that exists only to get the guest driver
   to boot and is irrelevant to operation: VBIOS image (placeholder with valid
   PCI-ROM/PCIR/BIT headers + version, no real code), fake-GSP-reported blobs.
   Build the minimal structure the kernel *parser/validator* accepts.
5. **Requires a real/host GPU AND root — AVOID.** Only if 1–4 cannot. If
   unavoidable, do it as a ONE-TIME root setup utility that collects the bytes,
   stores them, and then serves them to the *unprivileged* QEMU (which survives
   restarts). The current captured GA106 tables (devinfo/intr/gspstatic/initctrl)
   are effectively this tier (minus root) and are to be REPLACED by 1–4 over time.

Cross-cutting rule (applies to every tier): the value served must be
**self-consistent with what the emulator actually does** — interrupt vectors must
match the MSI-X vectors we raise, FB size must match the FB we back, engine list
must match the engines we expose. WE control the emulation, so when a value isn't
fixed by an upstream source, generate it and use the same value on both sides.

What tier (2) can and cannot reach: unprivileged host userspace gets the headline
identity + caps (name, VRAM, chipId/arch, PCI ids, CC, SM count). It does NOT
reach GSP-internal RM control responses (device-info engine fault IDs, interrupt
vectors, GspStaticConfigInfo internals) — those are tier 1 (static/arch) or 3
(source-gen), with capture (tier 5) as the stopgap we are migrating off.

Worked examples from the M3→M5 bring-up (2026-06-03):
- interrupt subtreeMap → tier 1 (computed: UVM_OWNED=subtree1, etc.).
- numDispChannels → moot once we report display fused-off (tier 1 decision).
- CE fault-method-buffer size → synthesized one page (tier 4); a cleaner home is
  tier 2/3 if an unprivileged host ioctl or source constant exposes it.
- GET_DEVICE_INFO_TABLE (engine topology + LCE fault IDs) → currently captured
  (tier 5); target is tier 3 (arch source) + tier 2 (which engines the host has).

## Goal (user, 2026-06-03)
What matters is **not** that we impersonate the exact host GPU — it's that **CUDA
compiles and runs in the guest** against whatever GPU we advertise. Implications:
- We advertise ONE real, supported NVIDIA GPU model, self-consistently.
- Because Mode-2 forwards real *compute* to a host GPU, the advertised **compute
  capability (arch) should match the host compute GPU** so generated SASS runs
  (or rely on PTX JIT). The *board/SKU* details need not match.
- So: pick a supported chip per host-arch; make its data a clean, swappable set.

## The data, classified by SOURCE (this is the crux of "auto-generate")

**A. SOURCE-DERIVABLE — auto-generatable from open-gpu-kernel-modules + SDK headers.**
These are constants/layouts NVIDIA publishes per-arch; a generator parses them so
they **auto-update when NVIDIA ships a new arch's headers**:
- Register offsets: NV_PRAMIN, NV_PBUS_BAR0_WINDOW/BAR2_BLOCK, NV_PMC_BOOT_0/42,
  NV_PTIMER, GSP/SEC falcon bases, WPR2 regs, GFW_BOOT, NV_USABLE_FB_SIZE_IN_MB…
  (swref/published/<arch>/<chip>/dev_*.h)
- GMMU page-table format + geometry (NV_MMU_VER2/VER3 PTE/PDE bit fields, per-level
  VA bit ranges) — dev_mmu.h + kern_gmmu_fmt_*.c
- Instance-block field offsets (NV_RAMIN_PAGE_DIR_BASE_*) — dev_ram.h
- Chip identity (PMC_BOOT_0 value, PCI device IDs) — partly source, partly public DB
- Per-class alloc-param sizes, control struct sizes — SDK ctrl/cl headers
=> Build `tools/mode2_gen_regs.py`: input = an OGKM checkout + target chip; output =
   `data/mode2/<arch>/regs.h` (offset/format tables). Adding a new card NVIDIA's
   open driver supports = run the generator against that driver tree. This directly
   satisfies "offsets auto-update for new cards."

**B. GENERATE-TO-SATISFY — device-info table, interrupt table, GSP static config.**
(REVISED 2026-06-03 per user: these are NOT "capture-required". The goal is to
SATISFY THE DRIVER, not to be truthful. The driver imposes *constraints* (from
source); we synthesize any values that pass them. We don't need the real
fuse/board values, and we don't want a card-ownership or proprietary-dump
dependency.) The constraints come in two flavors — both satisfiable by generation:
  - STRUCTURAL/VALIDATION (struct sizes, valid flags, count ranges, name string,
    checksums): trivially generated from source. nvidia-smi surfaces mostly the
    **GPU name** (gpuNameString in GSP static config) + a few caps — it does not
    deeply inspect engine/fault internals, so a sane synthesized set passes.
  - FUNCTIONAL/CONSISTENCY (interrupt vectors per engine, engine runlist IDs, FB
    region size): the value isn't arbitrary — it must be self-consistent with what
    our emulation actually DOES (the MSI-X vectors we raise, the engines we expose,
    the FB we back). But WE control that emulation, so we GENERATE the table and
    use the same values on both sides. Still generation, just self-consistent.
  => Generate per chip from the source constraints (engine list from the HAL/chip
     config, intr vectors we assign, FB size we choose, name we advertise). No card
     needed. `tools/mode2_gen_devtables.py`.
  => CAPTURE is now only a *validation reference*: a one-time real capture (the
     streamed-dmesg harness) to cross-check that our generated tables have the
     right SHAPE for a known card. Not a requirement, not shipped as the source of
     truth. The existing GA106 captures serve exactly this role.
  !! NOT in this class: things the driver *functionally exercises* rather than
     stores — e.g. kbusVerifyBar2 writes real VRAM through BAR2 and reads it back.
     No generated "value" satisfies that; it needs real emulation (the GMMU walk +
     FB backing). That is functionality (class D below), not data.

**C. CONTROL RESPONSES (the 56 init controls)** — mostly chip-static; captured today.
Many could be derived (caps/clock tables) but capture is the reliable baseline.

**D. REAL FUNCTIONALITY — not data, must be emulated/forwarded.**
Things the driver exercises for real: BAR0-PRAMIN/BAR2 memory round-trips (GMMU
walk + FB backing), DMA, channel/USERD doorbell, and the actual compute (forwarded
to a host GPU). These can never be "a value that passes" — they are the emulator's
real work. Independent of the data tables.

## Honest summary (REVISED per user 2026-06-03)
- "Auto-update offsets for new cards" → **YES, fully** (class A header generator).
- "Generate the whole per-device dataset from source" → **YES**, for all the DATA
  (classes A+B+C): the driver only *validates/stores* these, so we generate values
  that satisfy the source-derived constraints + are self-consistent with our
  emulation. No card ownership, no proprietary dumps. Capture is demoted to a
  one-time *validation reference* per card, not a requirement.
- The only things that aren't generatable are class D — REAL functionality (memory,
  DMA, compute) — but those were never "data" anyway; they're the emulator's job.
- Net: Mode-2 device support for a new GPU = run the generators against that chip's
  open-driver headers/HAL; no capture needed (capture optional, to validate shape).

## Repo layout (target)
```
data/mode2/
  <arch>/regs.h              # generated (class A): offsets + GMMU format
  <chip>/devinfo.bin         # captured (class B)
  <chip>/intrtable.bin
  <chip>/gspstaticinfo.bin
  <chip>/initctrl/*.bin      # captured (class C), per cmd
  <chip>/manifest.json       # chip id, arch, source driver ver, capture date
tools/
  mode2_gen_regs.py          # parse OGKM swref/sdk -> data/mode2/<arch>/regs.h
  mode2_capture_to_data.py   # parse streamed-dmesg capture -> data/mode2/<chip>/*.bin
```
The emulator loads the chip set selected by a device property (default: detected
host-arch's reference chip), instead of the current hardcoded GA106 #includes.

## Migration from today
Current: per-chip data is hand-generated C headers (mode2_{devinfo,intrtable,
initctrl,gspstaticinfo}_ga106.h) #included directly. Step 1 = move the captured
bytes to data/mode2/ga106/*.bin + a loader; Step 2 = write mode2_gen_regs.py and
replace the hardcoded register offsets/GMMU constants with generated tables; Step 3
= a runtime/property chip selector. The VBIOS placeholder (mode2_generality_and_vbios)
is the redistribution-safe analog for the PROM window.
```
