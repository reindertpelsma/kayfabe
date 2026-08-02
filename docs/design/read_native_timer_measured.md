# Read-native timer — what hardware said, and how it reshaped the task

> Companion to `register_plane_read_native.md`, which states the **ruling**. This states the
> **measurement**, taken 2026-08-02 on a real GA106.
>
> Status: **the blocking question is settled YES; the design is reshaped; the memslot caller
> is NOT built and needs an owner decision.** Task `#128`.

## 0. The epistemic frame

Everything below is one of three things and is labelled:

* **[measured]** — a run named here, output on disk under `docs/reference/bench_evidence/`.
* **[src]** — read out of `ogkm-580` or this tree. A source read is not a measurement.
* **[inferred]** — a deduction from the two above, with the premises named.

The run behind every **[measured]** claim:
`docs/reference/bench_evidence/timer-mappability-9087090.out`, revision
`9087090d281d0e25bceea79a5ed98d55a1f7d7db`, binary sha1 `c746957…`, RTX 3060 (GA106),
host driver 580.159.04 open, kernel 6.8.0-59-generic, 2026-08-02T01:13:54Z. Re-runnable:
`kayfabe-rm-ladder --gpu 0 --timer`.

★ It has **two arms and the second one is the control**: arm A runs under uid 65534 with no
capabilities and `no_new_privs`; arm B runs as root. That is not belt-and-braces. RM's
`RmValidateMmapRequest` returns `NV_PROTECT_READ_WRITE` immediately for
`osIsAdministrator()` and **never executes the range walk** [src: `ogkm-580:
src/nvidia/arch/nvalloc/unix/src/osapi.c:2023-2054`], so a root run measures nothing about
mappability. Arm A is the answer; arm B exists so *"the two agree"* is a fact rather than an
assumption.

## 1. The blocking question, and the answer

> **Can an unprivileged, capability-less isolate map the host GPU's timer registers at all?**

**[measured 2026-08-02, GA106, revision 9087090] Yes — by two independent routes, and root
and non-root get identical results.**

| what | arm A (uid 65534, no caps) | arm B (root) |
|---|---|---|
| `NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET` | `NV_OK`, `tmr_offset = 0x9000` | identical |
| `NV01_TIMER` alloc | `hObject 0xcafe0004` | identical |
| `NV01_TIMER` CPU map (ioctl `0x414` / mmap `0x1000`) | **ACCEPTED** | identical |
| PTIMER page counter over a 20 ms sleep | `+20 116 864 ns` | `+20 171 552 ns` |
| usermode-window mirror over a 20 ms sleep | `+20 100 288 ns` | `+20 207 488 ns` |
| the two mappings read one counter | **yes** — mirror `25.8 μs` after the second page read | yes |

Both counters advance by the sleep duration to within 1 %, and the mirror reading — taken
after the second PTIMER-page reading, in that order — is 25 792 ns later than it. Two
different BAR0 addresses, two different RM objects, agreeing to twenty-six microseconds in
strict temporal order: they are one counter.

**[src] Why RM permits it**, which is the part that survives a driver update:
`subdeviceCtrlCmdValidateMemMapRequest_IMPL` walks BAR0 range by range for a non-admin
caller, and the PTIMER range is the **first row it tries**, granted `NV_PROTECT_READABLE`
[`ogkm-580: src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c:2905-2917`]. The
usermode window is the second row, granted read-write — the row that already lets an
ordinary CUDA process ring its own doorbell.

★★ **The read-only-ness is the driver's policy, not ours.** An unprivileged holder of the
PTIMER mapping *cannot* write `NV_PTIMER_TIME_0`, so `tmrSetCurrentTime_GV100`'s register is
out of reach **by construction** rather than by a check we could forget. ⚠ That guarantee
evaporates for a root mapper, which is one more reason the isolate is the process that opens
the device (`guest_blast_radius.md` §3.1).

## 2. ★★★ The finding that reshaped the task

The ruling said *"make the timer page read-native onto the host GPU's own register page"*.
There are two candidate host pages, and **only one of them can be used** — for a reason that
has nothing to do with permission.

A KVM memslot maps a guest page onto a host page. **It cannot re-base within a page**: the
low 12 bits of the guest physical address are the low 12 bits of the host virtual address,
always. So passthrough is only expressible when the register sits at the *same offset within
its page* on both sides.

| side | register | BAR0 offset | page offset |
|---|---|---|---|
| **guest** (what the driver actually reads) | `NV_VIRTUAL_FUNCTION_TIME_0` | `0xBB0080` | **`0x080`** |
| host, **PTIMER page** — no doorbell in it | `NV_PTIMER_TIME_0` | `0x9400` | **`0x400`** |
| host, **usermode window** — the doorbell window | `NVC361_TIME_0` | window `+0x080` | **`0x080`** |

⇒ **[inferred; both premises measured 2026-08-02 on a GA106 at revision 9087090, §1]** The
clean, doorbell-free page is the one that does *not*
line up. The page that does line up carries `NVC361_NOTIFY_CHANNEL_PENDING` — **the doorbell
— sixteen bytes later**, in the same 4 KiB page [src: `ogkm-580:
src/common/sdk/nvidia/inc/class/clc361.h:29-33`].

★ The guest offset is not a choice we made: `tmrReadTimeLoReg_TU102` reads the counter
through the virtual-function aperture unconditionally, on a virtual function *and* on the
physical one [src: `ogkm-580: .../timer/arch/turing/timer_tu102.c:130-155`], which is why
`kayfabe_device::ga10x` serves `0xBB0080` and not `0x9400`.

**So there is no arrangement of pages in which the timer and the doorbell are different
pages.** The owner's ruling — *reads native, writes trapped, and the purpose of trapping the
writes is doorbell-token translation* — is therefore not two policies over two pages. It is
**one policy over one page**, and `KVM_MEM_READONLY` expresses exactly it: reads are served
by the hardware page with no exit, every write takes an `KVM_EXIT_MMIO` and reaches the
token translator. ⇒ The doorbell sharing the page is not an obstacle to the design; it is
the reason the design has the shape it has.

Held as a test rather than as prose:
`kayfabe-device/tests/chip_table.rs::the_guest_timer_offset_can_only_be_backed_by_the_host_usermode_page`.

## 3. ⚠ What reads natively that we did not ask for

The consequence of a page-granular mechanism: **every** register in the host's usermode
window becomes guest-readable, not only the two timer words. `NVC361` publicly defines three
offsets in 65 536 bytes — `TIME_0` (`0x80`), `TIME_1` (`0x84`), `NOTIFY_CHANNEL_PENDING`
(`0x90`) [src] — and the rest of the window is undocumented in the open tree.

**[unverified]** What those bytes contain on a live GA106 is **not measured**, and it is a
prerequisite for shipping §2, not a detail. ⊘ Do not assume they are zero: the C oracle's
empty rows taught this project that *an empty capture is evidence of nothing*
(`c_oracle_empty_rows_are_wrong`). The measurement is cheap — the mapping in §1 already
exists — and it belongs to whoever builds the memslot caller.

## 4. The write policy, decided

`tmrSetCurrentTime_GV100` writes `NV_PTIMER_TIME_0/_1` [src: `ogkm-580:
.../timer/arch/volta/timer_gv100.c:56-82`], so a guest write to the counter is a real event.

**Decided: refused by name.** `kayfabe_device::plane::PTIMER_WRITE_REFUSED`, counted in its
own `Counters::ptimer_writes_refused`.

Before this it fell through to `unclaimed_writes` and was dropped. The *effect* was already
right — nothing reached any counter — but the **decision had never been made**: it was
indistinguishable from the hundreds of offsets this port simply does not model, and it would
have stayed indistinguishable after the page went read-native. A separate counter is the
point: unclaimed means *"not modelled"*, this means *"modelled, and no"*.

⊘ It refuses rather than emulating a settable clock. A guest that could move its own counter
would destroy the one property §2 buys — that a guest timestamp and a host GPU timestamp are
in one timebase.

★★★ **How many mechanisms actually hold it — corrected, because the comfortable answer was
wrong.** The first draft of this section said "triply held", counting RM's
`NV_PROTECT_READABLE` grant among them. That grant applies to the **PTIMER page** [§1] — and
§2 is that the PTIMER page *cannot be the backing page*. The page that will back the guest
is the usermode window, and `subdeviceCtrlCmdValidateMemMapRequest_IMPL` returns from that
row with `protection` left at its default `NV_PROTECT_READ_WRITE` [src: `ogkm-580:
src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c:2903, 2919-2926`] — it has
to be writable, because it is the window an ordinary CUDA process rings its doorbell
through.

⇒ On the page that ships, the write is held **twice**, not three times:

| # | mechanism | layer | holds on the shipping page? |
|---|---|---|---|
| a | `PTIMER_WRITE_REFUSED` | this port | **yes** — and it is the only one that *states* the policy |
| b | RM's `NV_PROTECT_READABLE` grant | the host driver | ⊘ **no** — that is the PTIMER page, which §2 rules out |
| c | `KVM_MEM_READONLY` on the memslot | the kernel | **yes** — this is what makes the write trap at all |

⚠ A safety argument that counts a mechanism guarding a *different page* is not conservative,
it is wrong, and it is the more dangerous direction: (b) reads as a backstop nobody has to
maintain. There are two, they are (a) and (c), and (c) is not yet built.

## 5. The instrument was wrong five times, and each is recorded

★★★ **The first R19 run reported the driver refusing a mapping it had never been asked
about.** It printed `refused Other(19271)`; `19271` is `0x4B47` = `NOT_IN_THIS_OBJECT`, one
of `kayfabe-isolate-host`'s **own** local statuses. No ioctl was issued — `Mapping::anywhere`
requires a page-multiple length and `NV01_TIMER_MAP_SIZE` is `0x414`. A statement about our
length arithmetic was wearing the driver's clothes, and it was one edit away from being
reported as *"RM refuses the PTIMER page on GA106"*.

★★ **The second: no single length can work**, because the ioctl length and the `mmap` length
are different numbers. `gpuresMap_IMPL` refuses anything past the resource's own size with
`NV_ERR_INVALID_LIMIT` [src: `gpu_resource.c:126-143`] — `0x414` for `NV01_TIMER` — while
Linux requires a page multiple. RM reconciles them itself:
`nv_align_mmap_offset_length` rounds the *registered* range up to a page [src: `osapi.c:1976-1986`]
and `nvidia_mmap_helper` then compares the `mmap` length against that **rounded** size [src:
`nv-mmap.c:560-565`]. **[measured 2026-08-02, GA106, revision 9087090]** `ioctl 0x414 /
mmap 0x1000` is accepted; both
same-value pairs are refused, at two different layers.

⇒ The rung now sweeps `(ioctl, mmap)` **pairs** and classifies every refusal it prints as
*our own local status* / *an errno* / *an `NV_STATUS`*. A refusal that cannot say which layer
produced it is not evidence. (`suspect_the_instrument_first`, and it was the instrument
twice.)

★ **A third**, in a test rather than on hardware: the first draft of
`a_refused_counter_write_leaves_the_counter_readable_and_advancing` stepped the clock by
1 ns and asserted `0 != 0`. The low half's `NSEC` field is bits **31:5**, so the bottom five
bits read zero and a one-nanosecond step is invisible. The test was the defect.

★★ **A fourth: the identity check passed on slack.** The rung's first version asked only
that the mirror reading lie *"inside the interval"* of the two PTIMER-page readings, with a
**one second** tolerance. It passed — and it would have passed for two unrelated clocks that
merely happened to be near each other, which is not the claim being made. The bound is now
1 ms and the measured gap is 25 792 ns. ⚠ The first capture was taken with the loose
predicate, so it was **re-taken at `9087090`** rather than cited: a transcript that records a
weaker check than the code performs is the same defect as citing an empty row as
corroboration.

★★★ **A fifth, and it is the one that would have survived review: §4's safety argument
counted three mechanisms and one of them guards a different page.** See §4 — the error was in
the flattering direction, which is the direction that does not get questioned.

⇒ Five, on one small task, and none of them was found by the code failing. Four were found by
reading the output against what it was supposed to mean, and the fifth by writing the claim
down as a table and finding a row that could not be filled in.

## 6. What is NOT built, and what it needs

**[measured 2026-08-02, GA106, revision 9087090] Settled:** the mapping is obtainable,
unprivileged, and it is live.

**Not built:** the memslot caller. `Vmm::map_read_native` exists and is tested, `memslot_spans`
already rounds a write-trap sub-range outward to whole host pages, and `KVM_MEM_READONLY` is
plumbed to the syscall. What is missing is (a) any caller that installs a read-native overlay
over a BAR0 page, (b) a BAR0 row kind in `qemu/hw/misc/nvkvm/nvkvm.c` that permits a slot —
BAR0 is `NVKVM_KIND_TRAP` today and the archive is explicitly forbidden to install one over
it — and (c) any bridge from `kayfabe-qemu-raw`, the shim the live device runs through, to
the memslot plane.

★★★ **And one question that is not an implementation detail:** the host mapping is held by
the **isolate**, and a KVM memslot's `userspace_addr` must be in the **VMM's** address space.
So either the GPU descriptor crosses to the VMM (the `SCM_RIGHTS` path `#131` built, and the
one `guest_blast_radius.md` F14/§1.1 is about), or the VMM becomes an RM client in its own
right. Those are different trust boundaries, and choosing between them is an **owner
decision**, not a refactor. ⊘ Do not pick one by implementing it.

⚠ Whatever is built, **only a live boot is proof**. Nothing in this document claims the guest
has ever read a host counter — it claims the host counter is *reachable*, which is a
different and smaller statement.
