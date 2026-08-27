# Mode-2 BAR0 trap reduction — stop trapping non-side-effect registers

**Status:** root-caused 2026-06-15; fix in progress. Owner-driven (user architectural direction).

## The finding (measured)

Mode-2 LLM generation runs at ~22 tok/s; **Mode-1 hit host parity (63 tok/s) on the *same*
nested-virt vast.ai box**, so the 2.5× gap is the Mode-2 *design*, not the environment.

Instrumenting KVM exits (`/sys/kernel/debug/kvm/<pid>-*/mmio_exits`) during a generation run:

- **320–423k MMIO vmexits per run ≈ 1–3k per token.**
- BAR0-read histogram, **both load and gen phases**:
  - `0x110094` read **~40k (load) + ~31k (gen-tail)** — **99% of all reads**, dominant in both.
  - everything else (`0x110118`, `0xb830b0`, `0x1103c0`, …) is < 1%.
- Gen-phase BAR0 **writes**: `0x110c00` (×1015), `0x110114/8/c` (×468) — see below.

`0x110094` = **`NV_PGSP + 0x94` = `NV_PFALCON_FALCON_DEBUGINFO`** (Ampere `dev_falcon_v4.h`,
RW-4R) — the **GSP falcon debug/scratch/status register**. Our `nvkvm_reg_read` has no case for
it → it returns the `default: 0`. The (closed) driver **busy-polls it ~constantly**, and under
nested virt every read is an L2→L1→L0 vmexit — the single most expensive op here. The per-token
GSP **writes** (`0x110c00` = `NV_PGSP_QUEUE_HEAD` cmd-queue doorbell; `0x110114/8/c` = GSP falcon
IRQ set/clear/mask) show the completion/sync path goes **through GSP**: the driver posts a GSP
command and **spin-polls `DEBUGINFO`/`IRQSTAT` for the response**, each poll a vmexit.

This is, finally, the gen bottleneck. It is also why earlier levers didn't move t/s:
- completion-sema/H2 (`event=0`), logging (`m578`), clocksource (`m575`), host-ioctl (`backmap=0`),
  O(n) overlay/va_seen scans (real but cheap in time), the opaque GPFIFO-walk skip (cuts per-doorbell
  *work*, not the *exit count*) — **none touch the `0x110094` poll**, which is a *register read*, not
  a doorbell or a walk.

## Classification rule (a BAR register should NOT trap unless…)

A read/write only needs to trap if it has a **genuine side effect**. Otherwise serve it without a
vmexit. Decision tree:

| Category | Example | No-trap mechanism |
|---|---|---|
| **Side-effect** (write triggers work) | doorbell `0x110c00`, CPUCTL STARTCPU, IRQSCLR (W1C) | **must trap** (write only) |
| **Passthrough timer** | PTIMER / USERMODE nano-clock | map host reg page → guest reads real HW |
| **Read-only constant** | GPU/boot info, HWCFG | serve from RAM (constant) |
| **Async-updated counter** | progress/notify counters | RAM page (memslot), updated out of band |
| **Atomically sim-updated status** | **FALCON_DEBUGINFO**, IRQSTAT, MAILBOX | RAM page, sim writes the value on state change |

`DEBUGINFO` is the last row: a status word the GSP writes and the driver reads. Reading it has **no
side effect** → it must not trap.

## The fix

Carve the GSP falcon page (`0x110000`, 4 KiB) out of the fully-trapping BAR0 MMIO region as a
**`memory_region_init_rom_device`** sub-region (higher priority than the BAR0 container):

- **Reads** are served from a backing **RAM buffer** → KVM maps the page → **zero vmexit**.
- **Writes** still invoke our callback (`nvkvm_bar0_write`) → side-effect registers (QUEUE_HEAD
  doorbell, CPUCTL, IRQSCLR W1C) keep working exactly as today.
- Our emulation **keeps the RAM buffer current**: whenever sim state changes (post a SWGEN0 →
  set `IRQSTAT` bit6 in the buffer; update `DEBUGINFO`/`MAILBOX0`), write the new value into the
  page. The existing read-time special-cases (`0x110008` IRQSTAT, `0x110018` IRQMASK, etc.) move
  to "write the RAM buffer when the source changes" instead of "compute on read".

Properties:
- **Even if the driver keeps spinning**, it now spins on RAM (no exits). That alone is most of the
  win. Knowing *what value* it waits for lets us also write that value so the spin *terminates*
  sooner — a second-order gain, not required for the exit-count win.
- Same mechanism generalizes to the **USERMODE doorbell / nano-clock page** (reads of the nano clock
  + status from RAM/host-passthrough; only the doorbell *write* traps) and is the prerequisite for
  full **doorbell passthrough** (guest rings host USERMODE directly, no trap — the Mode-1 model).

## Validation

- `mmio_exits` per token must **crater** (the 31k/gen-tail `0x110094` reads → ~0).
- Correctness gate unchanged: **cup8 2048² byte-exact (`bad=0`)**, LLM **coherent**, **Xid=0**.
- Watch for staleness bugs: any register the driver reads whose value our sim forgot to refresh in
  the RAM buffer will read a stale value — enumerate every read-time special-case and convert it to a
  write-on-change.

## Result (m582–m584, 2026-06-15)

Implemented (`m2romregs`, default off): GSP-falcon page → rom-device subregion of a BAR0 **container**
(reads from RAM via `nvkvm_gsp_falcon_sync`, writes via the thunk). Findings:

- **Correctness: clean.** LLM coherent, `rc=0`, **Xid=0**; GSP boot/compute unaffected. The page-split
  (status reads from RAM, side-effect writes still trap) is sound.
- **The rom-device read intercepts at the QEMU level** — `0x110094`/`0x110118` reads in the `bar0_read`
  trace dropped **104k → 0**, while the `0x110c00` doorbell write still traps (1015). So flatview +
  priority overlay work (a **container** is required; a plain leaf-with-subregion overlay did NOT
  render for the memory listener).
- **But `mmio_exits` did NOT drop** (≈318k either way). A **full RW RAM** memslot variant (definite
  memslot) *also* didn't drop exits. ⇒ **Under this nested-virt host, KVM does not serve no-exit reads
  for a BAR-subregion memslot** — the reads bypass the QEMU ops but still vmexit (nested EPT forces an
  exit on BAR-backed pages). This is consistent with Mode-1 reaching parity on the same box *by
  avoiding MMIO entirely* (virtio/ioctl), not via memslots. On **bare-metal** KVM this rom-device
  should give the intended no-exit win.
- **Scope correction:** the `0x110094` storm is **LOAD-dominated** — back-calculating across NGEN
  (~256k exits in load + ~640 exits/token in gen), the bulk is GSP-boot + model-setup RPC polling, not
  per-token generation. So a *working* trap-elimination mainly speeds **load / nvidia-smi /
  cudaMemGetInfo**, and only modestly helps gen t/s; gen is bound by per-token guest+host work + the
  ±40% nested-virt variance.

**Where this leaves it:** the rom-device is committed (gated, correct) as the right primitive and the
bare-metal/load win; the *gen* parity path is Mode-1's model — **don't trap the hot path at all**
(doorbell/poll passthrough via virtio-style submission), validated on a non-nested box. Re-measure on
bare-metal before investing further in the memslot path.

## Pointers

- Harnesses: `m580` (mmio_exit count + log-gate), `m581` (gen-vs-load trap histogram).
- Related: `docs/design/mode2_interrupt_delivery.md`, `mode2_doorbell_chid.md`,
  `mode2_dataplane_architecture.md` (USERMODE RO-memslot doorbell), `mode2_m3_gsp_rpc.md`.
- Memory: `mode2_execfwd_layer2.md` tail (committed `8a77d5c`: opaque fast-path + GPGA index +
  DIAG-log gate).
