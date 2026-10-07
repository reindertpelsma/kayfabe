# w274 — RESULT: the window is an APERTURE, the MME is EXCLUDED by a native control, and the guest's context-init pushbuffer is BYTE-IDENTICAL to bare metal

**STATUS: LIVE — 2026-08-12.** Pre-registration: `PREREGISTRATION.md` beside this file, committed
at `ec78e31` **before** the capture was built or run.

**What was measured.** One new native capture on a real GA106 (`vh`, RTX 3060, open
`580.159.04`, **no QEMU, no emulated GPU, no kayfabe**) —
`nvidia-gpu-passthrough@61f8e8a2`, run `run_20260812T133554Z`, `NVDP_EXIT rc=0`. Plus a re-read
of the committed `w271` boot logs at build rev **`5feac90`**.

⊘ **No guest was booted.** `CUP2_RC` is **not** a w274 result; the standing value is w271's
`124` and this rung did not re-measure it. Arm E was pre-registered as untestable and stayed
untestable.

---

## ★★★★★ LEAD — FOUR OF THE BRIEF'S LOAD-BEARING CLAIMS ARE WRONG, AND THE RUNG INVERTS

### ⊘⊘⊘ 1. The fault has NOTHING TO DO with `SET_SHADER_SHARED_MEMORY_WINDOW`.

The brief's whole rung is *"is this window supposed to be backed, and by what?"* The two
addresses are **different objects in different reservations**, and the guest's own
`/proc/PID/maps` says so (`run_w271_pin_probe.log:166-174`):

| | value | containing map record |
|---|---|---|
| the **window** | `0x75b2_b9000000` | `75b2b6000000-75b2bc000000 ---p` (96 MiB) |
| the **fault** | `0x75b2_aee00000` | `75b2aea00000-75b2af000000 ---p` (6 MiB) |

They are **162 MiB apart with five mapped records between them**. Nothing joins them but a
shared ASLR slot. The window is not what faulted, and backing it would not have helped.

⊘ And note *why* the window was ever in the frame: it is the only 64-bit operand our
`GR-ADDRESS-CENSUS` fails to resolve, so it was the only candidate the log offered. **The one
address the census can see is not the one that faulted.**

### ★★★★★ 2. THE WINDOW IS AN APERTURE, MEASURED ON BARE METAL — arm B fires.

Native GA106, its own `cup2`, its own ASLR, `nvdp.log` ITEM 2c:

```
SET_SHADER_SHARED_MEMORY_WINDOW (aperture)   : 0x0000786879000000
    containing map      = 0000786876000000-000078687c000000 ---p [anon] +0x3000000
    cpu-readable        = 0
    ★ the record        = 0x786876000000-0x78687c000000 perm=---p (96 MiB), value at +0x3000000 = 50% of the way in
```

⇒ **Native reserves a 96 MiB PROT_NONE hole and puts the window base at the exact midpoint.**
The guest does **the identical thing** at `75b2b6000000-75b2bc000000`, window at
`0x75b2b9000000` — 96 MiB, midpoint, its own ASLR. **Native has no backing for it either.**
Arm A (*"native has a backing we lack"*) is **REFUTED**.

The class header says the same thing independently
(`ogkm-580.159.04:clc7c0.h`), and this is the part that generalises:

| method | field name | what it is |
|---|---|---|
| `SET_SHADER_LOCAL_MEMORY_A/B` (`0x0790`) | **`ADDRESS_UPPER`/`ADDRESS_LOWER`** | a real pointer |
| `SET_SHADER_LOCAL_MEMORY_WINDOW_A/B` (`0x07b0`) | **`BASE_ADDRESS_UPPER`/`BASE_ADDRESS`** | an aperture |
| `SET_SHADER_SHARED_MEMORY_WINDOW_A/B` (`0x02a0`) | **`BASE_ADDRESS_UPPER`/`BASE_ADDRESS`** | an aperture |
| **`SET_SHADER_SHARED_MEMORY_A/B`** | — | **DOES NOT EXIST** |

Local memory has *both* a pointer and a window. Shared memory has **only** a window — because
shared memory is on-chip SM SRAM and there is nothing in memory for a pointer to name. The
window base is where shared memory *appears* in the generic address space; the GMMU never
walks it.

> ⇒ **"Is the shared-memory window supposed to be backed?" is a CATEGORY ERROR. It can never
> be backed, on any hardware, in any run.** Our census's `unbound=1` on it is a **permanent
> false alarm** — it will report an unresolvable operand on a perfectly healthy boot forever.

### ⊘⊘ 3. THE MME IS EXCLUDED — by a native control, not by argument.

`w273` observed 39 dwords of MME microcode on all 8 compute channels in both arms and reasoned
that a method decoder structurally cannot see an MME-synthesised address. The brief asked to
tie it or set it aside.

**Native loads the same 39 dwords.** `nvdp.log`: `CTX-INIT FACTS: mme_dwords=39 i2m_launch=0
qmd=0 gr_report_sem=1`. Our `GR-ADDRESS-CENSUS`: `mme_dwords=39`, both arms.

⇒ **The MME is not a divergence. It is identical on the arm that works and the arm that
faults, so it cannot be the difference between them.** ⊘ This does not prove the MME never
synthesises an address; it proves the MME cannot explain a *guest-only* fault. Arm C is dead;
arm D fires.

★ And the absence of `0x75b2_aee00000` from our decode has **two fully-accounted non-MME
causes**, both checkable in the tree:
1. **We only ever dump ring index 0.** Exactly two `GR-PUSHBUFFER` lines exist in the pin arm,
   both `idx=0`. Native's copy sits at **`gpe[110]`**. The guest's `cuMemcpyHtoD` pushbuffer
   has never been decoded by anyone — which is the hole the native reference already named in
   its §8.
2. **The census cannot see the I2M destination even in principle.** `COMPUTE_ADDRESS_OPERANDS`
   is derived by the rule *"an `_A` method at offset `o` whose `_B` sits at `o+4`"*
   (`completion_watch.rs:200`). The I2M destination is
   `NVC7C0_OFFSET_OUT_UPPER` (`0x0188`) / `NVC7C0_OFFSET_OUT` (`0x018c`) — **not an `_A`/`_B`
   pair**. It is absent from the 17-row table.
   ⇒ **The one address `cup2`'s memcpy actually dereferences is the one address the census is
   structurally blind to.** A derivation rule is a filter, and this one filters out the target.

### ★★★★★ 4. THE GUEST'S CONTEXT-INIT PUSHBUFFER IS BYTE-IDENTICAL TO BARE METAL.

`gpe[0]` is 216 dwords at GPU VA `0x200400000` **in both**, and it is the one segment both
decoders dump. Reconstructing the guest's stream from `run_w271_pin_qemu.log` and diffing it
against native's `raw/pushbuffer_ctxinit.bin`:

```
native dwords: 216   guest dwords reconstructed: 216
dwords compared: 193   unknown (truncated in our log): 23
DIFFERENCES: 2
  dw[  5] native=0x7868       guest=0x75b2         SET_SHADER_SHARED_MEMORY_WINDOW_A
  dw[  7] native=0x79000000   guest=0xb9000000     SET_SHADER_SHARED_MEMORY_WINDOW_B
```

**193 of 193 comparable dwords match. The only two that differ are the two words of the
window, and they differ because ASLR differs.** The 23 uncompared dwords are the MME microcode,
truncated by our own logger (`..+7`, `..+16`) — a named instrument gap, not a divergence.

⇒ **The guest's CUDA driver, driving our emulated GPU, assembles the compute context exactly
as it does on real hardware.** The channel is the same one: native `ch[0]`,
`hObj=0x5c000019`, `gpFifoOffset=0x200200000` — and our walling channel is
`key=0xc1d0000c:0x5c000019 ring=0x200200000`. Same handle, same VA, same bytes.

---

## SO WHAT IS THE WALL? — a whole ADDRESS FAMILY with no publication path

The Xid, read carefully:

```
ENGINE GRAPHICS  HUBCLIENT_FE  faulted @ 0x75b2_aee00000  FAULT_PDE  ACCESS_TYPE_VIRT_WRITE
```

`HUBCLIENT_FE` is the graphics **front end**, and it is doing a **WRITE**. In `cup2` the front
end writes exactly one thing, and the native reference records it verbatim: the compute class's
**I2M** unit, `OFFSET_OUT_UPPER`/`OFFSET_OUT` + `LAUNCH_DMA(I2M)` + `LOAD_INLINE_DATA`, which
is how a 4-byte `cuMemcpyHtoD` is performed — **no copy engine at all**. Native's I2M
destination is `0x7f4f_66200000` / `0x7868_6a200000` (two runs): a CUDA unified VA with **no
CPU mapping**, in the same ASLR slot as the process.

And the guest is parked on precisely that operation. The spin probe
(`run_w271_pin_probe.log`) shows `cup2` polling two addresses in userspace:

| slot | kind | wanted | actual |
|---|---|---|---|
| `0x2_0440ff70` (`+0xf70`) | CE release | `0x45` | `5` |
| `0x2_0440fff0` (`+0xff0`) | **GR report semaphore** | `5` | `2` |

`+0xff0` is the **same slot, at the same page offset, that native's `cuMemcpyHtoD` releases**.

⇒ The chain is: the guest submits the I2M copy → the GR front end tries to write the
destination → **`FAULT_PDE`** → the report semaphore at `+0xff0` never reaches 5 → `cup2`
spins → `CUP2_RC=124`.

**And here is the measurable gap, in one line.** Every VA our device has ever resolved, bound,
joined or published in the pin arm, by family:

| family | count | what it is |
|---|---|---|
| `0x2_xxxxxxxx` | **1252** | sysmem / ring / pushbuffer / semaphores |
| `0x100_xxxxxxxx` | 64 | the tex pools |
| `0x1_xxxxxxxx` | 5 | — |
| **`0x75b2_xxxxxxxx`** | **32** | **all of them the window operand, all `Unresolved`** |

⇒ ★★★ **The CUDA unified-VA family has never been mapped by us — not once, in any arm.** The
GPU's `FAULT_PDE` (a failure at a *page-directory* level, not a leaf) says the same: there is
no directory coverage for that region at all.

⊘ **Is the CPU-side `---p` a defect? No — and the control proves it.** Native's `dp =
0x78686a200000`, the device pointer the GPU demonstrably writes with a landed semaphore, is
itself inside `786866000000-78686e400000 ---p`. **A working device pointer is CPU-PROT_NONE on
bare metal.** So "the fault address is in a PROT_NONE record" is *normal*, and nothing about the
guest's CPU mappings is wrong.

★ Stronger still, the two address spaces line up **record for record** once anchored on the
first `nvidiactl` chunk (native `0x78686e400000` ↔ guest `0x75b2ae400000`, Δ `0x2B5C0000000`):

| native | guest | |
|---|---|---|
| `78686e400000-78686e600000 rw-s nvidiactl` | `75b2ae400000-75b2ae600000 rw-s nvidiactl` | ✔ |
| `78686e600000-78686e800000 rw-s /dev/zero` | `75b2ae600000-75b2ae800000 rw-s /dev/zero` | ✔ |
| `78686e800000-78686ea00000 rw-s /dev/zero` | `75b2ae800000-75b2aea00000 rw-s /dev/zero` | ✔ |
| `78686ea00000-78686f000000 ---p` (6 MiB) | `75b2aea00000-75b2af000000 ---p` (6 MiB) | ✔ **the fault is in this record, in both** |
| `78686f000000-78686f200000 rw-s nvidiactl` | `75b2af000000-75b2af200000 rw-s nvidiactl` | ✔ |

The guest's faulting VA maps to native's `0x78686ee00000` — same 2 MiB slot, same 6 MiB
PROT_NONE record, same position within it. **The VA is where a device-memory allocation
belongs. Natively the GPU has a mapping there and here it does not.**

---

## THE ARMS, GRADED

| # | arm | prior | fired? |
|---|---|---|---|
| A | native has a backing for the window, we lack it | LOW | **NO — refuted, native's is unbacked too** |
| B | the window is an APERTURE, not an address (*inverts the rung*) | HIGH | ★★★ **YES** |
| C | MME **tied** | LOW | **NO — excluded by a native control** |
| D | MME **excluded** | HIGH | ★★★ **YES** |
| E | `CUP2_RC` moves off 124 | untestable | **N/A — no boot** |
| F | native `gpe[0]` emits the window at all | HIGH | ★ **YES**, at the same method index |
| G | native `gpe[0]` is 216 dw, byte-comparable | HIGH | ★★★★★ **YES — 193/193 match, 2 ASLR diffs** |
| H | the fault address is `dp` specifically | MED-HIGH | ⊘ **NOT PROVEN** — see below |

⊘⊘ **AND MY OWN INSTRUMENT FAILED, in the direction that would have flattered me.** I built
ITEM 2c to discriminate *aperture* from *pointer* by whether the value sits in a PROT_NONE
reservation. It does not discriminate: `SET_TEX_HEADER_POOL` and `SET_TEX_SAMPLER_POOL` are
**real pointers** and are *also* `---p`, `cpu-readable = 0`, natively. ⇒ **The maps census did
NOT prove "aperture"** — the class-header naming argument did, and the maps census proved
something else worth more (arm A refuted; the record-for-record alignment). Had I read the
census as my discriminator I would have reported a correct conclusion from an instrument that
cannot support it. `a_correct_citation_narrowed_by_the_reading`, in the mirror.

---

## SECONDARY — the 193 UNVECTORED completions: **not a second gap for `cup2`. Closing it.**

Measured, both arms: `off` = **193** UNVECTORED, `pin` = **217** (the brief's 193 is the `off`
arm's / w270's number). The emitter's own doc (`qemu/hw/misc/nvkvm/nvkvm.c:1777-1789`) defines
it as *"a copy this shell really performed and never announced"* — a **non-stall interrupt
vector** that was not raised.

**`cup2` cannot be blocked on it.** The spin probe measures `tid 1757 state=R`,
`/proc/1757/syscall: running`, RIP in the vDSO — it is **spinning on two memory reads in
userspace**. A non-stall vector does not unblock a memory poll; its absence cannot be that
wall. The counter that *does* cover waiters blocked in `poll()` is the os-event line, and it
reads **`0 UNVECTORED`** in both arms — that promise was kept.

⇒ **They are bookkeeping for work nobody is interrupt-blocked on. No channel `cup2` waits on is
affected.** ⚠ It stays a real *latent* gap: any future workload that blocks in `poll()` on a
completion event would hang on it. Worth a line in the ledger, not a rung.

---

## ⊘ WHAT THIS RUN CANNOT PROVE

1. **That `0x75b2_aee00000` is `dp`.** The shape, the client, the access type, the semaphore
   slot and the record alignment all fit, but `cup2` does not print `dp`, no boot was run, and
   under the record alignment native's *own* `dp` lands in a different (larger) reservation.
   It is **a device-memory VA in CUDA's unified pool** — which one is unmeasured.
2. **That the GR fault is the last wall.** One fault was observed. Removing the CE fault
   exposed this one; removing this one may expose another.
3. **Anything about the guest's memcpy pushbuffer.** It has still never been decoded. Both the
   ring index it lives at and its method stream are unknown on our side.
4. **The 23 MME dwords.** Our logger truncates them, so the byte diff covers 193 of 216 dwords.
   The *count* matches native; the *content* is uncompared.
5. **Ordering.** Post-hoc log reads and a native capture whose text lines carry emit-time, not
   event-time. The release-vs-`GP_GET` ordering hole named in the native doc §8 is untouched.
6. **Anything under another workload, chip or driver.** `cup2` (which launches **no kernel**),
   GA106, `580.159.04`. Two guest boots, two native runs.
7. **That publishing the CUDA-VA family would fix it.** That is the obvious next hypothesis and
   it is exactly the shape this campaign has adopted-as-cause and been wrong about before.

---

## NEXT RUNG — ranked

1. ★★★ **Decode the guest's memcpy submission.** Raise `GR_PUSHBUFFER_DUMPS_MAX`'s *index*
   coverage beyond ring entry 0, and add `OFFSET_OUT_UPPER`/`OFFSET_OUT` to
   `COMPUTE_ADDRESS_OPERANDS` as a hand-added row with a comment saying why the `_A`/`_B`
   derivation misses it. This turns the faulting address from an inference into a decode. It is
   the cheapest thing on this list and it blocks nothing.
2. ★★★ **Ask how a CUDA unified VA is supposed to reach our address table at all.** `1252`
   VAs in the `0x2_…` family and `0` in the `0x7xxx_…` family is not a bug in one binding —
   it is a missing source. The standing `nvdiff` finding (lockstep to
   `UVM_MAP_EXTERNAL_ALLOCATION`, then divergence) names the verb to look at first.
3. ★★ **Stop the census reporting the two `_WINDOW` methods as unresolved operands.** They can
   never resolve. Mark them `APERTURE` in `COMPUTE_ADDRESS_OPERANDS` and exclude them from
   `unbound`, or the census reports a defect on every healthy boot forever.
4. ★★ **Log the host `hwRunlistId`/`hwChannelId` at channel materialisation** (w273's item 1,
   still not done) so an Xid names itself instead of naming one of ten.
5. ⊘ **Do not "back" the shared-memory window.** It is an aperture. Mapping it would be
   fabricating a mapping no hardware has — the `cap2b` class, pointed inward.
