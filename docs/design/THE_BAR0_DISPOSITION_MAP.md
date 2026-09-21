# THE BAR0 DISPOSITION MAP — every page, every family, GSP and non-GSP

**STATUS: LIVE.** Created 2026-09-21 (w824). Owner-requested. Derived by `[fable w824]` from ogkm
(§50 level 2) and nouveau (§50 level 5); the timer analysis in §4 is mine and **corrects** one
step of fable's.

Read `THE_CONSTRAINTS.md` §52 (what has a read side effect) and §53 (the five dispositions, and
why a "read trap" is the absence of a memslot) first. This file is the table those two sections
point at.

## §0 — The dispositions, one line each

| | disposition | KVM mechanism |
|---|---|---|
| **A** | passthrough R/W, DRAM | ordinary r/w memslot |
| **B** | passthrough READ, trap WRITE | read-only memslot (`KVM_MEM_READONLY`) — **the default** |
| **C** | passthrough READ to a **live host** page | read-only memslot over the host's mapping |
| **D** | trap READ **and** WRITE | **no memslot — a hole** |
| **E** | refuse-by-name | read-only memslot + a named case in the write handler |

★ **Two refinements that shrink D more than anything else:**
1. **Writes to a data port are free.** `AINCW` costs nothing because every write traps under B and
   D alike — we see each one and advance our own cursor. ⇒ A port used **write-only** (firmware
   load into IMEM/DMEM) is plain **B**.
2. **A single read after a trapped cursor write is a computed shadow.** The write handler fills
   `DATA = MEM[cursor]`. ⇒ Only a **burst** (`for i: buf[i] = RD32(PORT)`) is un-servable from
   RAM. Every D row below is a burst.

## §1 — Disposition D: the complete hole list

| page | register | what it is | reachable on | cost of the hole |
|---|---|---|---|---|
| `0x8F2000` | `NV_PFSP_EMEMD` `0x8F2ac4` | FSP mailbox data port | **Hopper + discrete Blackwell, GSP** (`kfspReadPacket_GH100`); nouveau Hopper too | page also holds `QUEUE_HEAD/TAIL`, `MSGQ_HEAD/TAIL` — **boot + init only. Cheap.** |
| `0x840000` | `NV_PSEC_EMEMD` `0x840ac4` | SEC2 mailbox data port | **integrated Blackwell GB10B/GB20B only** (`ksec2SendBootCommands`). ⊘ Discrete Turing+ reads only `PSEC_FALCON_ENGINE` at reset ⇒ **B there** | boot only. **Cheap.** A family-internal split inside Blackwell. |
| `0x840000` | falcon `DMEMD` `0x8401c4` | SEC2 private-DMEM port | **nouveau non-GSP Turing/Ampere** (SEC2 msgq for ACR) | SEC2 ISR reads only during ACR bootstrap. **Cheap, boot-only.** |
| `0x087000` ⚠ inferred | falcon `DMEMD` | SEC2 private-DMEM port | **nouveau non-GSP Pascal GP102+, Volta** | boot-only. ⚠ **The base is inferred** — fable did not trace `gp102_sec2_new`'s default. **Verify before relying on it.** |
| `0x10a000` | `0x10a1c4` | PMU DMEM port | **nouveau non-GSP Maxwell-1 (GM107/GM108) and Kepler**. ⊘ GM200+ has no `.recv` ⇒ **B** — a family-internal split inside Maxwell | PMU init + memx reclock only. **Moderate, not runtime-hot.** |

★★★ **GSP Turing / Ampere / Ada: ZERO disposition-D pages.** That is the current product target and
the bench, and it derives from source what `THE_CONSTRAINTS.md:28` measured at w708–w710.

## §2 — Disposition C: the only live-host-page read

| page | registers | families |
|---|---|---|
| `0xbb0000` (VF usermode) | `VF_TIME_0/1` `0xbb0080/84`, doorbell `0xbb0090` | proprietary **Turing → Blackwell, GSP or not** — the HAL is chip-keyed, not GSP-keyed |

⊘ **No other C exists.** Fable ran a planted positive here: its search for hardware-advanced words
surfaced the fault-buffer `PUT` and the PMU msgq `PUT`, and both are values **we** author (we raise
the fault, we post the reply) ⇒ producer-updated **B**, not C. ★ That is the right shape for a
zero — the search demonstrated it could find candidates before concluding there were none.

## §3 — Disposition E: refuse-by-name

| registers | families |
|---|---|
| `NV_PTIMER_TIME_0/1` `0x9400/0x9410` write; PLM `0x9430` answers `LEVEL0=ENABLE` | Turing / Ampere / Ada, GSP |
| `NV_PGC6_SCI_SYS_TIMER_OFFSET_0/1` `0x118df4/f8` | Hopper / discrete Blackwell |
| none found | non-GSP nouveau — it writes no time register (only alarm/intr) |

## §4 — ⚠⚠⚠ THE ONE GENUINELY NEW PROBLEM: the non-GSP timer at `0x9000`

**nouveau reads `NV_PTIMER_TIME_0/1` (`0x9400`/`0x9410`) on EVERY family**, inside `nvkm_timer_read`
← **every `nvkm_msec` wait** (`timer/base.c:30,63`). That is millions of reads per boot.

⊘ It has **no read side effect** — §52 established the hi/lo/hi retry loop proves there is no
latch. The problem is different: **it is a live counter with no host mapping to alias.** The VF
usermode page can be C because the host's own driver maps it; BAR0 `0x9000` cannot.

### §4.0 — Which page, and can we map it from the host? ⊘ No.

**The whole PTIMER block is one page: `0x9000`.** `INTR_0 +0x100`, `INTR_EN_0 +0x140`,
`TIME_0 +0x400`, `TIME_1 +0x410`, `ALARM_0 +0x420`, PLM `+0x430` — all inside `0x9000–0x9FFF`.

⊘ **It is NOT mappable from host userspace.** The only BAR0 region RM hands to an unprivileged
client is the **usermode/VF window** — `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` = `0xB80000`
(`kern_gpu_tu102.c:100`), allocated as `VOLTA_USERMODE_A`/`HOPPER_USERMODE_A`. That is the page
holding the doorbell, which is *why* it is exposed. PTIMER at `0x9000` is ordinary privileged PRI
space and RM never maps it out — and §"no root required" closes the other routes.

⊘ **Aliasing the VF page onto it is also impossible**: the same counter appears there as
`VF_TIME_0/1` at `0xbb0080/84`, but a memslot maps page-to-page and the **offsets inside the page
differ** (`+0x400/+0x410` vs `+0x080/+0x084`), so no mapping can make one serve the other.

★ **But the VALUE is available, and that is what matters.** `VF_TIME_0/1` on the usermode page we
**do** map for the doorbell is the *same underlying PTIMER counter*. ⇒ The refresher should read
the host's mapped usermode page and store those two words into the guest's `0x9000` shadow —
**the same counter, so no drift, no rate conversion, no `clock_gettime` skew.**

⇒ **It cannot be C, and it must not be D**: a hole puts *three exits* through every wait-loop
iteration. The C artifact did exactly that (`nvkvm_gpu_emul.c:1517-1524`, whose own comment notes
*"RM timeout loops poll millions of times"*). **The answer is B with a VMM thread refreshing the
shadow from host time.**

### §4.1 — ⊘ CORRECTING FABLE'S JUSTIFICATION. The ordering matters, and not for the reason given.

Fable proposed storing **lo before hi**, reasoning that *"the guest's hi/lo/hi loop tolerates a
torn update if lo is stored before hi."* ⊘ **It does not tolerate it.** Worked through:

The reader accepts `(hi, lo)` whenever `hi` is unchanged across the window. Take a carry,
`(H, L_big) → (H+1, L_small)`:
- **lo first**: transient is `(H, L_small)`. A reader wholly inside the window sees `hi1 = hi2 = H`
  and **accepts** — a value ~4.295 s **BEHIND** true time.
- **hi first**: transient is `(H+1, L_big)`. Accepted likewise — ~4.295 s **AHEAD**.

⇒ **Neither order is correct.** With two independently-stored words and a reader that validates
only `hi`, a carry **cannot** be made atomic — the reader has no way to detect a change that
completed before it started. ⊘ And the usual escape is closed: `TIME_0` and `TIME_1` are
**16 bytes apart** (`0x9400`, `0x9410` — `regsnv04.h:6-7`), not adjacent, so a single atomic 8-byte
store cannot cover the pair.

★ **The race is nonetheless small and bounded, and it should be stated that way rather than
denied.** To be fooled, a reader must fit **all three** of its reads between our **two adjacent
stores** — a window of order 1 ns — and only around a carry, which happens **once per 2³² ns ≈
4.295 s**.

★★★ **So the ordering choice is real, but it is a FAILURE-MODE choice, not a correctness one:**
- **hi first** ⇒ the transient reads ~4.3 s **ahead** ⇒ an `nvkm_msec` deadline appears already
  passed ⇒ **a premature `-ETIMEDOUT` on a condition that would have been satisfied.** A spurious
  driver error.
- **lo first** ⇒ the transient reads ~4.3 s **behind** ⇒ the loop simply **waits longer**, and
  self-corrects on the next iteration once the shadow is consistent again.

⇒ **Store lo, then hi** — fable's recommendation, and it is the right one, but because the
residual failure is an over-wait rather than a spurious timeout, not because the reader tolerates
the tear. ⚠ **UNMEASURED.** The falsifier is a long-running non-GSP boot with a shadow refresher,
asserting monotonicity of observed guest time; until that runs this is reasoning, not a result.

⊘ GSP-path ogkm touches `0x9000` only at boot (`GR_TICK_FREQ`, `INTR_EN_0`, PLM `0x9430`) ⇒ plain
**B + E** there, and none of this applies. **Pre-Turing proprietary is UNKNOWN** — ogkm has no
pre-Turing timer HAL, so there is no source to read.

## §4A — ★ THE THREE EMEM PORTS ARE NOT THE SAME THING. Two are optional; one is the boot itself.

`[owner w824]` *"oh but if its only crash dump, which I don't care about as with kayfabe the guest
must not see the real GSP (and remain unprivileged), can we keep it read only (no read trap) and
just put 0x0 in it. then every read is 0."* — ★★★ **Yes. Confirmed, and for a reason stronger than
the gate.**

| port | what it carries | required? | disposition |
|---|---|---|---|
| **`NV_PGSP_EMEMD`** `0x110ac4` | **crash-dump readback** — one of four CrashCat apertures | **NO** | **B, serve 0** |
| **`NV_PFSP_EMEMD`** `0x8F2ac4` | **the GSP boot handshake** (Hopper, discrete Blackwell) | **YES** | **D** |
| **`NV_PSEC_EMEMD`** `0x840ac4` | **the GSP boot handshake** (integrated Blackwell) | **YES** | **D** |

### §4A.1 — GSP EMEM: serve zero, and the argument is two-deep

1. **The gate means it is never read.** `FALCON_DEBUGINFO = 0` ⇒ WFL0 fails
   `crashcatWayfinderL0Valid` (which requires bits 15:0 == `NV_CRASHCAT_SIGNATURE`,
   `nv-crashcat.h:310-311`) ⇒ `LoadWayfinder` returns `NV_WARN_NOTHING_TO_DO` ⇒
   `getNextCrashReport` returns `NULL` ⇒ `kgspReadEmem` is **never called**. See §53.5.
2. **And if it were read, zeros are safe.** A zero-filled buffer cannot form a valid crash report;
   CrashCat rejects it. ⇒ Belt and braces, not one thin gate.

★★★ **And the owner's framing is the load-bearing one, deeper than either:** *kayfabe **is** the
GSP.* There is no real GSP behind us to crash, so a crash dump is not a feature we are declining
to implement — **it is a feature that does not exist in our model**, and a guest asking for one is
asking about a processor it must never see. ⇒ Serving 0 is not a stub; it is the truthful answer.

### §4A.2 — FSP / SEC2 EMEM: the opposite — this IS how GSP gets booted

⊘ These are **not** debug channels. On Hopper+ the boot sequence changed: a **security processor
comes up first out of chip reset**, and RM asks *it* to bring GSP up, over a **packetised message
protocol carried in that processor's EMEM** — **MCTP** framing (`mctp_format.h`; SOM/EOM/TAG
validated at `kern_fsp_gh100.c:333-360`) wrapping **NVDM** messages (`nvdm_format.h`;
`NVDM_TYPE_INFOROM`, `NVDM_TYPE_HULK`, …).

The exchange: RM **writes** a command packet into EMEM through `EMEMC`/`EMEMD` with `AINCW`, rings
`QUEUE_HEAD`; the processor replies into EMEM and updates `MSGQ_HEAD`; RM **reads** the response
back out through `EMEMC`/`EMEMD` with `AINCR`. Commands include booting the **GSP-FMC** image,
WPR setup, and init-time knob queries (`kern_gpu_gh100.c:587`).

⇒ It is the Hopper+ **replacement for the Turing/Ampere ACR + GSP-falcon bootstrap**. Which
processor plays the role is a per-family assignment: **FSP** on Hopper and discrete Blackwell,
**SEC2** on integrated GB10B/GB20B (`ksec2SendBootCommands`, `ksec2SetupGspImages_GB20B`,
`ksec2GetGspBootArgs`, `ksec2WaitForSecureBoot_GB20B` — same MCTP/NVDM shape).

⇒ **Zeros are not an option here.** RM parses the MCTP header, validates SOM/EOM/TAG, dispatches
on the NVDM type, **and** asserts the cursor advanced by exactly `packetSize/4` (§52).

### §4A.3 — ⊘ THE 1-DWORD ESCAPE IS CLOSED. Recorded so nobody re-opens it.

§52 floated an avenue: *we* author the responses, so if every reply were a **single dword** the
read loop would run once (`N = 1`) and a pre-advanced `EMEMC` shadow would pass the assert ⇒ **B,
no hole**. **It does not work.**

`_kfspReadPacket_GH100:690` does permit `packetSize >= sizeof(NvU32)` — one dword is structurally
legal. But a packet the driver can *use* is rejected below that: `:406` requires
`size >= sizeof(MCTP_HEADER) + sizeof(NvU8)` — a 4-byte MCTP header **plus** at least the NVDM
type byte ⇒ **≥ 5 bytes ⇒ N ≥ 2 reads.** ⊘ And MCTP's multi-packet SOM/EOM framing does not help:
a 1-dword packet is header-only and carries no payload.

⇒ **Hopper and Blackwell genuinely need the hole.** ★ Which is affordable: both pages are
**boot-only**, they carry nothing polled at runtime, and **neither family is the current target** —
GSP Turing/Ampere/Ada has **zero** disposition-D pages.

## §4B — ✔ MEASURED w824: the OPEN module has NO non-GSP mode, and it lies about it

`[owner]` *"for non GSP we don't know ofc. thats worth a test with proprietary driver or if you can
satisfy from nouveau measurements."* ⇒ Ran the cheap half on the bench box (GA106,
`580.159.04`, vast `51894520`).

**Measured, 2026-09-21:**

| step | result |
|---|---|
| `modprobe nvidia NVreg_EnableGpuFirmware=0` | loads, **rc=0** |
| `/proc/driver/nvidia/params` | `EnableGpuFirmware: 0` ✔ *(accepted)* |
| `nvidia-smi` | `NVIDIA GeForce RTX 3060` — the GPU comes up |
| **`nvidia-smi -q` → GSP Firmware Version** | **`580.159.04`** ⊘ **GSP IS RUNNING** |

⇒ **The open kernel module accepts `EnableGpuFirmware=0`, echoes it back as `0`, and boots GSP
anyway.** It is GSP-only on Turing+ by construction, and it **silently overrides** the request
rather than refusing it. ★ So for the **open** driver the non-GSP axis **does not exist** on
Turing+ — no UNKNOWN, a measured absence. This is §51's premise confirmed rather than assumed.

### ⚠⚠⚠ TWO FALSE POSITIVES IN ONE EXPERIMENT — both of the tree's standing classes

1. **The parameter readback is what we ASKED FOR, not what the driver DID.** `params` reporting
   `EnableGpuFirmware: 0` is the *request*, echoed. Reading it as the outcome would have produced
   the confident, wrong headline *"GSP-off works on the open module"*. ⇒ **The observable had to be
   the thing itself** — `nvidia-smi -q`'s GSP Firmware Version — not the knob.
2. ⊘⊘ **And the obvious-looking direct observable is ALSO wrong.**
   `/proc/driver/nvidia/gpus/*/information` reports **`GPU Firmware: N/A`** on this very box **while
   GSP is running**. Two fields, same driver, opposite answers — and the one whose *name* matches
   the question is the misleading one. ⇒ Had I used procfs as the check, I would have "confirmed"
   GSP was off with a direct measurement. **Only `nvidia-smi -q` reports it correctly.**

⇒ Recorded as the instrument, not just the result: **when testing whether a knob took effect, the
observable must be downstream of the behaviour, never the knob's own readback — and check that the
field you picked actually tracks the behaviour, because a plausibly-named one may not.**

### ⊘ What is still UNKNOWN, and what would settle it

The **proprietary** module (`--kernel-module-type=proprietary`) is the only place a non-GSP
Turing/Ampere/Ada path could exist, and it is **not installed on the bench**. ⚠ Swapping the bench
box's driver would disturb the 30/30 bare-metal baseline that the whole guest lane indicts against,
so this needs **its own box or a deliberate window** — it was not done silently. ⇒ Until then the
three proprietary-GSP-off rows in §6 stay **UNKNOWN**, and the non-GSP map rests on **nouveau**,
which is source we can read.

## §5 — Everything else is B: the non-GSP block census (nouveau)

All rows: writes trap (W1C acks, enables, triggers); reads are plain or producer-updated.

| block | pages | read-side finding |
|---|---|---|
| PMC | `0x000000` | `PMC_INTR` producer-updated; `0x000600` reads are posted-write **flushes** — ordering only, no value needed |
| PBUS | `0x001000` | `0x1100` W1C; **`0x1700` is the PRAMIN window latch** — the trapped write that re-points the A-disposition mmap |
| PFIFO / PBDMA | `0x002000`, `0x040000` | `0x2100` W1C; pre-Pascal fault info `0x2800+` acked by a `0x259c` write |
| PTIMER | `0x009000` | **see §4** |
| PMGR / i2c / gpio, PTHERM | `0x00d000-0x00e000`, `0x020000` | plain |
| PFB, incl. Volta fault info | `0x100000`, `0x100e00` | `0x100e4c-5c` read then W1C `0x100e60`; replayable faults are a **memory buffer** + SW-written GET |
| PPWR / PMU | `0x10a000` | **D on GM107/GM108 + Kepler**, B on GM200+ |
| PRIVRING | `0x120000-0x128000` | read then mask-write ack |
| PROM (VBIOS) | `0x300000` | static image |
| PGRAPH incl. FECS | `0x400000-0x41ffff` | `0x400100/108` W1C; the 16 FECS mailbox polls at `0x409800` read values **we** author |
| SEC2 | `0x840000` / `0x087000` | **D where the msgq is used** (§1) |
| NVDEC / NVENC falcons | `0x084000`, `0x1c8000` | have `dmem_pio`, but `nvkm_falcon_pio_rd` has no caller outside msgq/fsp ⇒ **B** |
| GSP falcon | `0x110000` | ⊘ **B on every family** — see §53.5; unused by nouveau non-GSP |
| VFN | `0xb80000-0xbbffff` | `0xb81000` leaves W1C; `0xb83000` fault-buffer regs we author; **`0xbb0000` is C** |
| PRAMIN | `0x700000` | **A** — r/w memslot re-pointed inside the `0x1700` write trap (297 µs worst, ~22 moves/boot) |
| PDISP | `0x610000+` | out of scope (displayless) |

## §6 — The family matrix

| family | GSP path | non-GSP path |
|---|---|---|
| **Maxwell** | n/a | **GM107/GM108: D `0x10a000`**; GM200+: no D. `0x9000` refreshed-B |
| **Pascal** | n/a | GP102+: **D SEC2 `0x087000`** ⚠ base inferred; `0x9000` refreshed-B |
| **Volta** | n/a | D SEC2 ⚠ inferred via `gv100_acr`; `0x9000` refreshed-B |
| **Turing** | **no D**; C `0xbb0000`; E `0x9400/10` | nouveau: **D `0x840000`**; `0x9000` refreshed-B. ⊘ **OPEN module: no non-GSP mode at all — measured §4B.** ⚠ Proprietary GSP-off: **UNKNOWN**, needs its own box |
| **Ampere** | as Turing — **this is the bench, and it is measured** | as Turing non-GSP (`ga102_sec2`) |
| **Ada** | as Turing | ⚠ nouveau has **no non-GSP Ada**; proprietary GSP-off **UNKNOWN** |
| **Hopper** | **D `0x8F2000`** (FSP, boot); E `0x118df4/f8` | n/a (GSP mandatory) |
| **Blackwell** | discrete: **D `0x8F2000`**; integrated GB10B/GB20B: **D `0x840000`** instead | n/a |

⚠ **Three UNKNOWNs are recorded as UNKNOWN rather than assumed clean**, because this table is what
would back a support-matrix claim: proprietary-GSP-off SEC2 on Turing/Ampere, all of non-GSP Ada,
and pre-Turing proprietary timer. ⊘ And one row is **inferred, not read**: the Pascal/Volta SEC2
base `0x087000`.

## §7 — What was searched, so the zeros mean something

- **ogkm-580**: every `RD32`/`RegRead` line matching `EMEMD|DMEMD|IMEMD` — **4 hits, all listed**;
  every caller of `kgspReadEmem` / `kcrashcatEngineRead*` / `SyncBufferDescriptor`. `IMEMD` has
  **zero** GPU read sites.
- **nouveau**: every `nvkm_falcon_pio_rd` caller; every `rd32` of `0x1c4+|0x184+|0x10a1c4|0xac4+`;
  the block-by-block ISR review in §5; a corpus grep for read-clear/latch language (hits only in
  VBIOS opcode names).
- ★ **Positive control**: the grep surfaced the known `gt215.c:106-109` and `gm200.c:47` reads
  **before** anything was concluded. ⊘ This is the discipline my own first sweep skipped — see
  `a_sweep_that_reports_zero_must_first_report_one` — and it is why these zeros carry weight and
  that one did not.
- ⊘ **Access codes were not used**: both ports carry `RW-4A`, identical to any array register.
