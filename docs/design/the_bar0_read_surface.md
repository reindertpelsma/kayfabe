# THE BAR0 READ SURFACE — zero read traps, and what still exits on write

**STATUS: LIVE (2026-09-12, w561).** Owner's target, stated in as many words: *"get me a raw
client pass incl boot in mean test with 0 read traps in entirety of kayfabe"*. This document is
the decision that target is built from. Supersede it here, in place, if the shape changes.

## 1. The finding this rests on

`[measured w561, from the chip table — no boot required]` BAR0 is 4096 pages. **524 are live**,
and 512 of those are two windows:

| what claims it | pages | where |
|---|---|---|
| PRAMIN window | 256 | `0x700000`–`0x7ff000` |
| VBIOS / ROM | 256 | `0x300000`–`0x3ff000` |
| GSP registers | 5 | `0x110000`, `0x111000`, `0x118000`, `0x1fa000`, `0x840000` |
| boot registers | 3 | `0x0`, `0x9000`, `0x88000` |
| interrupt leaves | 1 | `0xb81000` |
| invalidate trigger | 1 | `0xb83000` |
| free-running counter | 1 | `0xbb0000` |
| window latch | 1 | `0x1000` |

`[measured w561]` the doorbell is at **`0xbb0090`** — the SAME page as the counter — and its own
offset reads `unclaimed`, because a doorbell is write-only.

## 2. Why every one of them can stop trapping on READ

⊘ Checked, not assumed. Of the twelve non-window pages, **eleven have no read side effect**:

- interrupt leaves — `CpuIntrState::read(&self)` is an array lookup.
- invalidate trigger — one atomic load.
- window latch — returns the guest's own last write.
- GSP registers — the whole read path is `&self`: `mmio_read_with`, `boot_context`, `observe`,
  `on_read`. Only `mmio_write` takes `&mut`.
- boot registers — a table.

⇒ Each is a **pure function of state we own**, so a page whose bytes we update when that state
changes answers the guest identically, with no exit.

The twelfth is the counter, and it is different in kind: it changes continuously, so no copy can
keep up. It is not shadowed — the host's own usermode page is mapped over it. ★ And because the
doorbell shares that page, one read-only mapping serves both correctly: counter reads resolve
natively, the doorbell write still exits.

## 3. The decision

**BAR0 becomes a container tiled by three kinds of piece, no overlaps.**

1. **SHADOW** — everything except the two exceptions below. A rom device: reads resolve out of
   its own memory with no exit; writes dispatch to this device. Its bytes are:
   - zero for the 3572 pages no register lives in (a fresh mapping already is),
   - the VBIOS image, written once at realize (static for the life of the boot),
   - every shadowable register's current value, written by its PRODUCER whenever it changes.
2. **PRAMIN** — one memory slot over `0x700000`, re-pointed by a single `mmap` when the guest
   moves the window. Owner: *"is just one mmap remap in vmm va, no bql lock, no kvm memslot
   update"*. Reads AND writes resolve natively: it is the framebuffer, not a register file.
3. **THE USERMODE PAGE** — `0xbb0000`, mapped read-only from the host so the counter is the
   host's own, and the doorbell write at `+0x90` still exits.

## 3a. THE TARGET, as the owner stated it

> *"get me a raw client pass incl boot in mean test with 0 read traps in entirety of kayfabe"*
> … *"and write traps only in bar0, so not in bar1/2, except pramin (and more optionally if you
> can get that)"*

⇒ Three numbers, and a boot that passes while they hold:

| surface | reads | writes |
|---|---|---|
| BAR0, except PRAMIN and the usermode page | **0 traps** | traps — the control plane |
| BAR0 PRAMIN | **0 traps** | **0 traps** |
| BAR1 / BAR2 | **0 traps** | **0 traps** |

⊘ BAR1/BAR2 are already demand-filled memory slots, so their steady state is trap-free; what is
NOT yet zero is the FIRST touch of each page, which is one exit per page by construction. Making
that zero means filling them before the guest arrives, not filling them faster — a different
change from the shadow below, and the one §7 lists last because it is the least understood.

## 3b. PRAMIN, concretely — one mapping, re-pointed by one `mmap`

Owner: *"is just one mmap remap in vmm va, no bql lock, no kvm memslot update"*, and
*"safe code may not touch raw vmm va pointers unchecked"*.

**The shape.** One memory slot over `[BAR0 + 0x700000, +1 MiB)`, installed once. A VA
reservation of the same size in this process. Moving the window is `mmap(MAP_FIXED, fd,
offset = window_base)` over that reservation — the MMU notifier makes KVM drop its own
entries, so there is no memslot ioctl, no big lock, and nothing for the hypervisor to arbitrate.
⊘ Synchronous on the vCPU, because RM writes the latch and then uses the window immediately;
there is no completion to defer behind. It is one syscall, which is what makes that affordable.

**What it requires, and this is the real work.** A single `mmap` can only place the window if
the framebuffer's backing is CONTIGUOUS BY FRAMEBUFFER ADDRESS in the fd. Today
`SharedPageArena::alloc` is a bump allocator with a free list (`arena_unsafe.rs:170-177`) —
fd offset is allocation ORDER, so a 1 MiB window is 256 unrelated offsets.

⇒ **Make the arena address-indexed: fd offset == framebuffer address.** This is a
SIMPLIFICATION, not an addition — the free list and the bump cursor both disappear, because the
address IS the offset. A sparse `memfd` the length of the framebuffer costs nothing until a
page is touched.

⊘ **And it must be the SAME arena, not a second one.** A separate address-indexed backing for
PRAMIN would give the same framebuffer byte two homes, with nothing keeping them equal — the
failure this file's §5 is about. The BAR1/BAR2 mirror keeps working unchanged: a per-page slot
at `fd_offset = frame` is still correct when the offset happens to equal the address.

⚠ `SharedPageArena::LEN` is 1 GiB today, chosen to match a residency ceiling. Address-indexing
makes the extent a property of the FRAMEBUFFER's length, not of a ceiling, and the two must
stop being conflated.

## 4. What still exits, and it is the whole point

After this, a guest read of BAR0 **never** leaves the vCPU. Writes exit only where they must:

- the **doorbell** (`0xbb0090`) — the work-submit token;
- the **GSP queue**, **boot registers**, **interrupt leaves**, **invalidate trigger** and the
  **window latch** — every one of which drives a state machine, raises an interrupt, moves a
  window or starts an invalidate;
- writes into the shadow's zero and VBIOS regions, which are rare and kept trapping because the
  run list was measured from the READ classifier and a piece that swallowed writes would be
  claiming something never measured.

⊘ PRAMIN writes stop exiting entirely. `[measured w542]` that alone is **69 730 of 89 322**
write traps in a boot — 78%.

## 5. The invariant a producer must keep, and how it is checked

★★★ **A shadowed register has TWO homes: the state it is computed from, and the shadow's bytes.
A producer that updates one and not the other makes the guest read a stale value with no fault
and no counter** — the failure this whole tree is written against.

⇒ The shadow is **not** written by hand at each producer. The plane owns one `write_through`
entry point; a register's value reaches the guest only through it. And a debug gate sweeps every
shadowable offset at teardown, comparing the shadow's bytes against the classifier's own answer
— a divergence is a named failure, not a mystery in a later boot.

## 6. What this does NOT do

⊘ It does not reduce write traps except through PRAMIN.
⊘ It does not make the counter's value ours — it is the host's, and the two timebases must come
from the same mapping or they disagree at 43 ppm (`native_dataplane_cup2_ga106.md` §4).
⊘ It says nothing about BAR1/BAR2, which are demand-filled memory slots under their own arms.

## 7. Status

- [x] w550 — the cut, and the 3572 dead pages backed. `[measured w553]` the raw client passed
      with it: `(P)`, `MEAN_FALSIFIER=PASS`, `THREADS 8 of 8`, `VCPU-BLOCKING none`.
- [x] w561 — the live-page census and the read-side-effect audit above.
- [ ] the shadow, its `write_through`, and its teardown gate.
- [ ] VBIOS bytes into the shadow at realize.
- [ ] PRAMIN as one re-pointed slot.
- [ ] the usermode page mapped from the host.
- [ ] graded: raw client `(P)` **and** a read-trap census of **zero**.
- [ ] BAR1/BAR2 first-touch: the last exits on those BARs, and the least understood item here.
