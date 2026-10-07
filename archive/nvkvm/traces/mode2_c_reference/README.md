# C reference replay traces (task #90)

Five §6 replay traces recorded from the **C Mode-2 emulator** on real hardware, 2026-07-29.
They are the durable artefact: the C is a *perishable* oracle (it needs a booted guest, a
pinned kernel, a GA106 and a matching host driver, on a rented box), the traces are not.

Format: `src/qemu/nvkvm_m2_rec.h`. Recorder: `src/qemu/nvkvm_m2_rec.c` +
`m2rec/m2recfile/m2recmask` on `nvkvm-gpu-emul`. Decoder: `scripts/mode2_diag/rec_dump.py`.
Two-trace differ: `scripts/mode2_diag/rec_replydiff.py`. Capture driver:
`scripts/mode2_diag/cap1_capture_host.sh` + `cap1_coldboot_guest.sh`.
Consumer spec: the rewrite's `docs/design/mode2_gsp_port_plan.md` §6/§6.3 and
`docs/design/c_rust_trace_differential.md`.

```
zstd -dc cap1b_coldboot_hermetic_d6.rec.zst > /tmp/c.rec
python3 ../../scripts/mode2_diag/rec_dump.py /tmp/c.rec --head 40
```

`MD5SUMS` covers the **decompressed** `.rec` files.

## The captures

| file | records | raw / zst | props | what it is |
|---|---|---|---|---|
| `cap1_coldboot_hermetic.rec.zst` | 359 062 | 13.0 MB / 1.4 MB | `m2fwd=off m2exec=off m2romregs=off` — **hermetic** | Cold GSP bring-up: PCI enumerate → VBIOS stream → FWSEC/WPR2 → LibOS boot args → msgq handshake → `GSP_INIT_DONE` → `nvidia-smi -q` enumerates the emulated GA106. Superseded by `cap1b`; kept because the two together are the audit trail. |
| `cap1b_coldboot_hermetic_d6.rec.zst` | 360 725 | 14.1 MB / 1.5 MB | identical vector: `m2fwd=off m2exec=off m2romregs=off`, full mask, BAR0 trace on — **hermetic** | ★★ The same experiment as `cap1`, re-captured at `819282d` with the **GSP-D6 continuation elements witnessed**. **The only trace a replay can be closed over**, and the only one that can be closed *past the first multi-element command*. |
| `cap2_stalequeue_negative.rec.zst` | 886 999 | 34.2 MB / 3.1 MB | `m2fwd=on m2exec=on` — **NON-HERMETIC** | NEGATIVE. `cup2` (PASS) → `rmmod`/`insmod` twice in ONE QEMU lifetime. Life 2 dies on `msgqRxLink failed: -7 … NV_ERR_TIMEOUT`, life 3 on `unexpected WPR2 already up`. |
| `cap2b_stalequeue_nofn47.rec.zst` | 862 940 | 46.1 MB / 4.2 MB | `m2fwd=on m2exec=on` — **NON-HERMETIC** | ★ NEGATIVE, the sharp one. Driver restart **without** a prior CUDA process (so no `fn-47`). Contains **378 GSP command elements read out of arbitrary guest RAM and answered `NV_OK`**. |
| `cap3_matmul_forwarding.rec.zst` | 532 824 | 22.8 MB / 2.0 MB | `m2fwd=on m2exec=on` — **NON-HERMETIC** | `cuCtxCreate` → 2048² matmul (`cup8`, `bad=0 maxerr=0`, VERDICT PASS). Value is the **decision planes**, not replayability. |

Every header carries its own property vector, the declared filter, the guest/host driver
versions, the VBIOS md5 and an `nvidia-smi` summary — `rec_dump.py` prints it. All five are
`dense order: OK` with `n_errors=0`.

## ★★ `cap1b` — closing the closure limit (GSP-D6 made observable)

**A capture of a defective implementation cannot close a replay of a correct one.** The Rust
replay of `cap1` matched the C exactly *within the oracle's reach* and then stopped dead at
record **141 976** — `GSP_RM_CONTROL`, `rpc.length=8276`, `elemCount=3` — because it read
command ring **slot 7**, the first continuation element, and `cap1` holds no observation of
that slot while it was live. That is **GSP-D6**: the C acts on element 0 and advances its read
pointer past the continuations *without reading them*, so the recorder never saw them.

`cap1b` fixes the **artefact**, not the C. `nvkvm_m3_service_cmdq` now reads the continuation
slots through `nvkvm_dmar` — the recorder chokepoint — and **throws the bytes away**. GSP-D6 is
still a real row in the MUST-DIFFER ledger; the C still acts on element 0 alone and still emits
byte-identical replies. The bug is unchanged; it is merely *witnessed* now.

Measured on `cap1b` vs a control capture of the unmodified binary — 9 multi-element commands,
32 continuation elements:

| | `before` (unmodified) | `cap1b` (witnessed) |
|---|---|---|
| multi-element commands | 9 | 9 |
| continuation elements observed | **0 / 32** | **32 / 32** |
| `GuestRead` records | 629 | 661 (`+32`, exactly the continuations) |

The first multi-element command lands at record **141 976** in the control and **141 997** in
`cap1b` (`fn=76 elemCount=3 rpclen=8276`, identical) — i.e. the bring-up prefix is stable across
boots right up to the wall, and `cap1b` answers there. The `elemCount=9` message at the end of
the ring **wraps** to slots 0-7; its 8 continuations are witnessed too, contiguous in the record
stream, because the read is indexed `(cmd_readptr + i) % q_msgcount`.

### The reply stream is unchanged — the proof

The change may add `GuestRead` records and **nothing else**. Two captures of the *same* binary
are not identical (PTIMER/mailbox poll counts, a differently-placed queue, two wall-clock-bearing
replies), so a before/after diff means nothing without a control. Three captures, one command
each (`cap1_capture_host.sh`), diffed with `rec_replydiff.py`:

| projection | `before_A` vs `before_B` (control, same binary) | `before_B` vs `cap1b` (after) |
|---|---|---|
| `MmioWrite` (guest→dev) | 216 520 vs 216 520, **460 differ** | 216 520 vs 216 520, **460 differ** |
| `GuestWrite` CONTENT (the replies) | 859 vs 859, **2 differ** | 859 vs 859, **2 differ** |
| `IrqRaise` | **identical** | **identical** |
| `GuestRead` | 629 vs 629 | 629 vs **661** |

★ The 460 differing `MmioWrite` positions are the **identical index set** in all three pairings
(`A≠B`, `A≠C`, `B≠C` — set equality, zero symmetric difference): they are the writes carrying
guest physical addresses, which move every boot. The change adds **not one** new differing
position. The 2 differing replies are the same two in every pairing — queue element `seq=3
fn=228` (6 bytes: 3 payload bytes + the checksum over them) and `seq=165 fn=76 rpclen=848` —
both wall-clock-bearing. **The after-run is numerically indistinguishable from the control on
every channel except the one it was supposed to change.**

Reproduce: `cap1_capture_host.sh` (bench host) ×2 before and ×1 after, then
`rec_replydiff.py A.rec B.rec --bytes`.

### `cap1b` differs from `cap1` in one other way: it runs the FULL `nvidia-smi -q`

`cap1` was driven by hand; `cap1b` is driven by `cap1_coldboot_guest.sh`, which captures
`nvidia-smi -q` to a file instead of piping it (a closed pipe SIGPIPEs `nvidia-smi` part way
through its enumeration and silently truncates the RPC stream — and `$?` is then `head`'s).
`cap1b` therefore contains **more** RPC work than `cap1`: 859 `GuestWrite` vs 563. The property
vector, the guest, the driver and the bring-up prefix are identical; the extra work is a
superset, appended after `GSP_INIT_DONE`.

## Provenance

- Bench: vast.ai box, RTX 3060 = **GA106**, host driver **580.159.04 open**, host kernel
  6.8.0-59, QEMU 9.2.0.
- Guest: Ubuntu 24.04, kernel **6.8.0-117-generic** (the pin), **stock unpatched** open NVIDIA
  **580.159.04**, VBIOS `ga106_vbios.rom` md5 `48df40a04432aca6a35bee2785857eba`.
- Emulator source for `cap1`/`cap2`/`cap2b`/`cap3`: `src/qemu/nvkvm_gpu_emul.c` md5
  **`cced661c16f6856801d16dae151bc2f0`** (= `264caa2`, the commit that adds this directory),
  recorder `src/qemu/nvkvm_m2_rec.c` md5 **`d2ab3a95291396c0dce81e422a68e73a`**.
- Emulator source for **`cap1b`**: commit **`819282d`**, `nvkvm_gpu_emul.c` md5
  **`2132bbdbf98ab85449e9513c9c230bbf`**, recorder md5 unchanged. Property vector
  `trace=1 m2fwd=0 m2exec=0 m2hostsem=0 m2cefwd=0 m2cexec=0 m2opaque=0 m2trace=0 m2romregs=0`,
  mask `0xffffffffffffffff`, `hermetic=yes`. Installed binary md5
  `b21892d86716574acf29828663b31c68`.
  ★ `819282d` is the binding, not `HEAD` — later commits touch this file, so
  `md5sum src/qemu/nvkvm_gpu_emul.c` at HEAD is expected to differ. Check with
  `git show 819282d:src/qemu/nvkvm_gpu_emul.c | md5sum`.

★ **The first four headers have an EMPTY `emulator-src-commit` line** — the bench tree is not a
git checkout and `git rev-parse` yielded nothing, silently. A bench claim without a source
revision is worthless here; this bench served a binary built from `862c7c2` for weeks without
anyone noticing. `run_mode2_vm.sh` now falls back to a `.srcrev` file, then `$NVKVM_SRCREV`, and
prints `UNKNOWN` out loud rather than a blank. `cap1b` carries `emulator-src-commit: 819282d`.

★ **The captured emulator source is `consolidation` HEAD, *not* the previously-validated
`862c7c2`.** `862c7c2` was the last revision this bench had ever compiled — every revision from
`3710b8e` on carries a duplicate forward declaration that is a `-Werror=redundant-decls` build
failure under the bench's QEMU 9.2 configure, which this task had to fix first. So the
`#14 P0/P1` work is in these traces and had never run on hardware before. It was re-validated
in the act of capturing: `cup2` rc=0 (cap2) and `cup8` `bad=0 maxerr=0` (cap3), both with the
recorder on.

## How to use the negative trace

`cap2b` is the one whose **passing condition is that the Rust differs**. The C's defect is
visible from the artefact alone, no log required — decode `rpc.function` at offset 60 of every
4096-byte `GuestRead`:

```
cap2b : 546 elements — 168 with a sane function id, 378 GARBAGE (28 distinct)
cap1  : 178 elements — 178 sane, 0 garbage
```

Those 378 are arbitrary guest RAM parsed as GSP RPC while `q_ready` pointed at a dead queue,
each answered `NV_OK`. The Rust must emit exactly one `Refused(QueueNotBound)` and **zero**
`ElementPosted` there, with `cap1`'s positive replay as the non-vacuity arm.

## What these traces cannot witness

Recorded before the capture, so a green diff is never mistaken for coverage:

1. **The completion plane has no C oracle at all.** The C never observes a host completion
   source; it forges completions. Nothing here constrains it.
2. **Three of the five traces are non-hermetic by construction.** With `m2fwd`/`m2exec` on, the
   stub `MAP_FIXED`s guest RAM and the **host GPU DMAs into it directly** — guest-visible bytes
   that pass through neither `nvkvm_dmaw` nor `nvkvm_dmar` nor any QEMU path. `pci_dma_map`
   (`nvkvm_gpu_emul.c`, `nvkvm_m2_share_guest_ram`) is the hole. Only `cap1`/`cap1b` are closed.
3. **`cap1`/`cap1b` are hermetic AND slightly counterfactual**: `m2fwd=off` is a path the C was
   never shipped on (its own property comment calls host-GPU forwarding "the ONLY supported
   Mode-2 operating mode").
4. **One point on every axis**: one GPU, one host driver, one guest driver, one guest OS.
5. **`OverlaySnap` is absent from all five** (`m2romregs=off`). With the rom-device overlay on,
   the guest's reads of `IRQSTAT`/`MAILBOX0`/`CPUCTL`/`DMATRFCMD` do not trap at all and only
   the snapshot stands in for them.
6. **`cap1b` removes the *known* wall, not every possible wall.** The harness derives its
   closure limit as `first refusal, else first unanswerable read`. All 32 unanswerable reads are
   now answered; whether the replay then reaches the end of the trace or meets a **refusal** is
   a property of the replay, and only running it decides that. Nothing here is a claim that it
   closes — only that the reason it *could not* is gone.
