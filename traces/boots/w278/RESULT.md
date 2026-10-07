# w278 / R33 — RESULT: the raw CE client REPRODUCES THE WALL IN 53 IOCTLS, WITH NO libcuda

**STATUS: LIVE — 2026-08-12.** Branch `w276-port-the-whole-vas-sweep`, boots at `4af8b7a`
(stamp gate PASS on both arms: the QEMU binary's `kayfabe-rev` equals HEAD exactly).
Arms `w278_guest` (route B off) / `w278b_guest` (route B on). Every number below was read
from an artefact opened in this session; none is carried.

Pre-registration: `PREREGISTRATION.md`, committed before either boot.

---

## ★★★★★ LEAD — FOUR THINGS CONTRADICT THE BRIEF, AND THE FIRST IS THE RUNG

### 1. ★★★★★ THE WALL IS **NOT** libcuda's CONTEXT PATH — it reproduces with libcuda ABSENT

`[measured, w278_guest]` The same static binary that moves 4096 bytes on bare metal
(md5 `d24a38cf…`, asserted equal on both sides) fails inside the guest:

```
FAIL R33 arm 1 COPY = dst[0] 0x3f0011cc -> 0x3f0011cc (want 0xc0ffee33),
                      semaphore 0x00000000 (want 0x00000001),
                      GP_GET 0 GP_PUT 1  — the entry was NEVER fetched
```

and **every one of its 53 ioctls was served** — `total=53 failed=0`, the *same* 53, with the
*same* per-`NV_ESC` histogram and the *same* phase split as the native arm.

⇒ The brief's largest arm (**G1** — *"works ⇒ the hang is libcuda-specific"*) **did not
fire**. **G3** did: *"`GP_GET 0 / GP_PUT 1` ⇒ the entry was never fetched ⇒ a ~53-ioctl
minimal repro of a wall we currently reproduce in 578 records."*

⊘ And this is **stronger** than G3 as written, because of what it excludes:

| excluded | by |
|---|---|
| libcuda, the CUDA runtime, UVM, `cuCtxCreate` | **not loaded by this process at all** |
| the RM control plane | **53 of 53 ioctls served, 0 failed** |
| RM's VA allocator under our emulated GSP | **arms 2 and 3 both green in the guest** |
| privilege (`euid 1000` vs `0`) | ★ the native arm at **`euid 65534`, `CapEff=0`, `NoNewPrivs=1`** is green with the same 53 ioctls (`traces/real_ga106/rmladder_r33_ce_client_real_ga106.txt`, arm B) |

### 2. ★★★★★ AND THE DEVICE **DECODED THE CLIENT'S PUSHBUFFER EXACTLY** — codec validated

`[measured, w278_guest, `run_w278_guest_qemu.log.gz`]` — one `DOORBELL-XLATE` in the whole
boot, and our chip codec read the client's own methods out of it:

```
SEMA-SOURCE-CE    token=0x3 engine=Ce → methods=5 launches=1 opaque=3
                  release_target(s)=1 [0x120022000]
OPERAND-SOURCE-CE token=0x3 engine=Ce → operand(s)=2 (1 write, 1 read)
                  [W@0x120010000+0x1000  R@0x120000000+0x1000]
```

The client independently printed `semaphore 0x0000000120022000`. **The decode is correct to
the byte, on a pushbuffer we wrote ourselves and can therefore check** — which no `cup2`
capture can offer, because nobody has ever decoded libcuda's `cuMemcpyHtoD` pushbuffer
(`native_dataplane_cup2_ga106.md` §8).

★ Attribution is by identity, not by token: `RING-ROSTER key=0xc1d00013:0xcafe000d
ring=0x120021000 entries=64` — the client's own `hClient` and its 64-entry GPFIFO. The
kernel's channels carry 4096.

### 3. ★★★★★ THE BLOCKER IS **NAMED**, AND ONE VARIABLE MOVED IT ONE STEP

| arm | `KAYFABE_RING_VIDMEM` | `PushbufferAperture` | the refusal |
|---|---|---|---|
| `w278_guest` | off | **1** | `FwdFault::PushbufferAperture` |
| `w278b_guest` | **on** | **0** | ★ `FwdFault::RingFbNeverWritten` |

Everything else is byte-identical — same build, same six carried arms, same binary, same
`total=53 failed=0`, same client output to the character.

⊘ This is **not a new hypothesis**: `w246`'s four-corner square already measured
`PushbufferAperture` = 8 with route B off and 0 with it on at `PT_WITNESS_EXEC=on`
(`shim.rs`, `RING_VIDMEM_ENV` docs). This rung is that square, reproduced on a workload of
our own, and it behaved exactly as recorded.

★★★ **The new fact is the refusal underneath it.** `RingFbNeverWritten` is
`kayfabe-fwd`'s *"a vidmem range resolved into our framebuffer, and **nothing ever wrote
that page**"* — the guard that exists because `FbBytes::read` would otherwise answer zeros,
and a zero-filled GPFIFO ring is indistinguishable from a legitimately quiet one.

⇒ **The client wrote its ring through `NV_ESC_RM_MAP_MEMORY` — a CPU mapping of a vidmem
object, served without error — and our emulated framebuffer has no record of those bytes.**
That is a byte-provenance gap with a name, a single reproducing doorbell, and ~20 lines of
client code behind it.

### 4. ⊘⊘ THE FIRST RUN OF ARM 4 MEASURED **THE INSTRUMENT'S OWN RING**

`[measured, vh, run `r33_ce_client_fault`]` Arm 4 asked hardware whether `0x7_0000_0000` was
mapped. The engine **retired the read**, moved `0x20018000`, `GP_GET 2` caught `GP_PUT 2`,
**no Xid** — and the arm printed `RESOLVED … arm 3 said otherwise`. Both halves were wrong:

- `0x7_0000_0000` **is** `probe_guest_reachability`'s private `PROBE_RING_AT`. The probe
  places its own channel ring there *by design*, for the reason its own doc comment gives
  verbatim — *"a probe that allocates from the same allocator, in the same space, at the
  same moment, is not an independent observer"* (its 2026-08-10 correction). The constant
  was **private**, so no caller could see it, and **the identical failure recurred one layer
  up**: the instrument read itself and the answer was indistinguishable from a real one.
- ⊘ **Arm 3 was never contradicted.** Arm 3 probes the *control* space; arm 4 builds a
  *third, fresh* space. Same number, different page-table trees. The rung printed a
  cross-VAS comparison as a contradiction — **the exact failure shape the brief warned
  about, committed by the instrument written to avoid it.**

**Fixed structurally, not with a better comment**: `rm::REACH_PROBE_WINDOW` is public, the
three private constants derive from it under a `const assert`, and the ladder's probe VA
(`0x9_0000_0000`) is checked against it **at compile time**. ★ The gate was run against its
own known positive: putting `0x7_0000_0000` back fails the build with `error[E0080]` naming
the window. Arm 4 then fired correctly —

```
★ arm 4 FAULTED = a CE pointed at 0x9_0000_0000 did NOT retire (sem 0, GP_GET 2 GP_PUT 2)
                  while its positive control on the SAME channel did
NVRM: Xid (PCI:0000:00:07): 31, … name=kayfabe-rm-ladd, channel 0x00000005,
      MMU Fault: ENGINE CE0 HUBCLIENT_CE1 faulted @ 0x9_00000000 FAULT_PDE VIRT_READ
```

— the negative control naming **the address we asked about**, in the host's own ring buffer.

---

## ⊘⊘ AND THE PREMISE OF THE LANE WAS ALREADY BUILT — say it first

`scripts/bench/gpu_wedge_probe.c` and `tests/mode2/nvdiff/nvd_prog.c` are **not** the seed:
both load `libcuda`. The seed is `crates/kayfabe-isolate-host/{rm.rs, bin/rmladder.rs}` —
the ladder's default run has been a raw CE client since R17, and `R26`
(`rmladder_r26_dictated_ring_real_ga106.txt`) already recorded `GP_GET` catching `GP_PUT` at
an address we dictated, at `CapEff=0`, with a negative control that fired. **Twenty
consecutive lanes now.**

What did not exist, and is this rung's contribution:

1. **`kayfabe-linux-raw/src/census.rs`** — nothing in this tree could say how many times a
   run entered the driver. Placed at `CharDevice::ioctl`, the one funnel, so a call site
   that forgot to register still counts; phase subtotals print **against** the grand total,
   so a shortfall is readable as *"ioctls outside any phase"* rather than being invisible.
2. **`--ce-client`** — one flag that RETURNS, with no isolate, no sandbox rung and no second
   channel. That is what makes it **pushable into a guest**; the full ladder spawns a
   sandboxed child and cannot be.

## ★★★ THE NUMBER, and it is the one the owner asked for

**53 ioctls, 0 failed**, identical on bare metal and in the guest.
★ **`strace -f -c -e trace=ioctl` independently counted 53** — the census is not verified by
its own reply.

```
R0-R6 bring-up 7 | R7 vaspace 2 | arm1 ce-copy 28 | arm2/3 va-probe 7 | teardown 9  ⇒ 53/53
RM_ALLOC x17  RM_FREE x12  RM_MAP_MEMORY_DMA x7  RM_MAP_MEMORY x6
RM_UNMAP_MEMORY_DMA x6  RM_CONTROL x3  CHECK_VERSION_STR x1  REGISTER_FD x1
```

⇒ **`cuCtxCreate` alone costs 479** in the nvdiff oracle. The whole raw round trip —
bring-up, channel, engine object, schedule, doorbell, completion, two VA probes, teardown —
costs **53**, of which **28** are the copy.

⚠ A matching total is **necessary, not sufficient**: a count cannot see a substitution. Every
arm above is graded by **identity** — payload value, `GP_GET`/`GP_PUT`, the read-back words —
never by the count.

---

## PRE-REGISTERED ARMS — how they fell

| arm | outcome |
|---|---|
| G1 works in the guest | ⊘ **did not fire** |
| G2 works, different ioctl count | ⊘ did not fire |
| **G3 `GP_GET 0 / GP_PUT 1`** | ★★★★★ **FIRED** — and with every ioctl served, which G3 did not assume |
| G4 `GP_GET == GP_PUT`, no semaphore | ⊘ did not fire |
| G5 dies before arm 1 | ⊘ did not fire — bring-up and channel alloc both clean |
| G6 arm 2 says `Free` (instrument broken) | ⊘ did not fire — arm 2 green in the guest |
| G7 hangs (`R33_RC=124`) | ⊘ did not fire — `R33_RC=1`, a verdict, not a timeout |
| G8 binary does not run | ⊘ did not fire — `GUEST_MD5` = native md5, `GUEST_EXECUTABLE=yes` |
| G9 works and the guest driver logs an `Xid` | ⊘ did not fire — 0 guest Xid, 0 host Xid |
| G10 no `/dev/nvidia*` | ⊘ did not fire — four nodes, `nvidia` loaded |
| ★ **unregistered**: the blocker is a **named refusal of ours** | ★★★★★ **FIRED** — `PushbufferAperture`, then `RingFbNeverWritten` under route B |
| ★ **unregistered**: arm 4 measures the instrument | ⊘⊘ **FIRED** — §4 above |

---

## ⊘⊘ WHAT THIS RUN CANNOT PROVE

- **It cannot say the `cup2` wall and this wall are the same.** They share a *shape*
  (`GP_GET` never moves), and this one has a name (`RingFbNeverWritten`). Nothing here shows
  `cup2` reaches the same refusal — `cup2` was **not run on either boot**, deliberately, so
  `CUP2_RC` is not a number this rung has.
- **It cannot say anything about the VA the GR engine faults on in `cup2`.** The client
  builds its **own** `FERMI_VASPACE_A` and probes **that** (ranges `0xcafe0005` /
  `0xcafe0009`, printed by the program). The faulting channel is the guest driver's own
  client with its own PDB. ⇒ The owner's *"poke whether each mapped address is mapped"* is
  now answerable **without gdb** — and only **within a VAS we own**. In `cup2`'s VAS it is
  still not answerable, and this rung does not pretend otherwise.
- **It cannot say the ring bytes are absent** — only that our framebuffer has **no record of
  a write** to those pages. *"The guest never wrote them"* and *"the write did not reach our
  model"* are different, and `RingFbNeverWritten` exists precisely because they are
  indistinguishable to a byte census.
- **Route B does not submit work.** `w246` recorded `CE-SUBMIT = 0` in all four corners; it
  enumerates a ring. `PushbufferAperture = 0` here is **not** the first forwarded work.
- **One workload, one chip (GA106), one driver (`580.159.04`), one boot per arm.**
- **The completion plane still has no oracle.** A green forward would not, on its own,
  distinguish a served completion from a forged one.

---

## ARTEFACTS

| what | where |
|---|---|
| pre-registration | `traces/boots/w278/PREREGISTRATION.md` |
| ★ the native reference (3 arms, incl. `euid 0` **and** `CapEff=0`, and the provoked Xid) | `traces/real_ga106/rmladder_r33_ce_client_real_ga106.txt` |
| both boots, whole | `traces/boots/w278/` (`w278_run.log`, `w278b_run.log`, `run_w278*_guest_*`) |
| the runner + grader | `scripts/bench/w278_run.sh` |
| the guest hook | `scripts/bench/r33_hook_ce_client.sh` |
| the census | `crates/kayfabe-linux-raw/src/census.rs` (+ `ioctl::nr_of` / `magic_of`) |
| the rung | `crates/kayfabe-isolate-host/src/bin/rmladder.rs` (`ce_client`, `print_ioctl_census`, `nv_esc_name`) |
| the window that made §4 unrepresentable | `crates/kayfabe-isolate-host/src/rm.rs` (`REACH_PROBE_WINDOW`) |

## ★ THE NEXT ONE FACT

`RingFbNeverWritten` says our framebuffer holds no write for the client's ring pages. The
client wrote them through `NV_ESC_RM_MAP_MEMORY`, which **succeeded**. ⇒ The one measurement
that would close this is **where those CPU stores went** — a BAR1/vidmem CPU-mapping write
path census, joined against the ring's own pages. It needs no guest driver change, no
libcuda, and the workload to reproduce it is already committed and takes 53 ioctls.
