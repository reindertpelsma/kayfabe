# w278 / R33 milestone 2 — PRE-REGISTRATION: the raw CE client, inside the guest

**Written and committed BEFORE the boot.** Branch `w276-port-the-whole-vas-sweep`.
The native half (milestone 1) is already measured and committed — see `traces/real_ga106/
rmladder_r33_ce_client_real_ga106.txt` and commit `1620bc0`.

---

## ⊘⊘ WHAT IS BEING MEASURED, and what the DEVICE is doing differently

**Nothing.** The device is byte-identical to `w277_on`; the arming is `w271_pin`'s, carried
unchanged. The only new thing in the guest is **a userspace program**: a statically linked
`kayfabe-rm-ladder --ce-client`, pushed in over ssh and run against the stock NVIDIA driver
bound to our emulated GPU.

⇒ **`CUP2_RC` is not measured on this boot** and no claim about the `cup2` wall follows from
it either way. This rung asks a question `cup2` cannot: *does the ring / pushbuffer / USERD /
doorbell / semaphore surface work in the guest **with libcuda removed from the process**?*

## THE NATIVE REFERENCE THIS IS DIFFED AGAINST — measured, not carried

`[measured 2026-08-12, vh, RTX 3060 GA106 / 580.159.04, REV 1620bc0, the SAME musl binary]`

| fact | native |
|---|---|
| arm 1 COPY | 4096 bytes, `dst[0] 0x3f0011cc → 0xc0ffee33`, `dst[last] 0xc0fff232`, sem `0x1` = declared, **`GP_GET 1` caught `GP_PUT 1`** |
| arm 2 VA-OCCUPIED | RM refused a fresh object at the ring's own VA (`NoMemory`) |
| arm 3 VA-FREE | `0x9_0000_0000` unclaimed in the control range |
| arm 4 (opt-in) | CE pointed at `0x9_0000_0000` did **not** retire; host `Xid 31 … ENGINE CE0 HUBCLIENT_CE1 faulted @ 0x9_00000000 FAULT_PDE ACCESS_TYPE_VIRT_READ` |
| **ioctls** | **53, 0 failed** (`strace -c` independently: 53) |
| phases | bring-up 7 · vaspace 2 · ce-copy 28 · va-probe 7 · teardown 9 |

## PRE-REGISTERED ARMS — and the low ones are widened, because six of the last nine rungs had their least-weighted arm fire

| arm | outcome | what it would mean |
|---|---|---|
| **G1** | the client **works**, all three arms met, `total=53` | ★★★★★ the data plane is SOUND under our device model and the `cup2` wall is **specific to libcuda's context path**. The single largest result available on this rung |
| **G2** | works, but the **ioctl count differs** | the guest driver serves a different number of escapes for the same RM calls — an interesting fact about our GSP, and `by_request` says which |
| **G3** | **arm 1 fails at `GP_GET 0 / GP_PUT 1`** | the entry was never fetched ⇒ USERD, the doorbell token, or the schedule. **A ~53-ioctl minimal repro** of a wall we reproduce in 578 records |
| **G4** | **arm 1 fails at `GP_GET == GP_PUT`, no semaphore** | fetched, methods did nothing ⇒ `SET_OBJECT` class, subchannel, or an operand that does not resolve. Also a minimal repro, and a **different** one |
| **G5** | it dies **before** arm 1, in bring-up or `alloc_channel` | the census names the exact ioctl. Then the wall is not the data plane at all |
| **G6** | arm 2 says `Free` where the ring is | ⊘ **the INSTRUMENT is broken in the guest**, and arm 3 is worth nothing this run — RM's VA allocator inside our emulated GSP does not report occupancy |
| **G7** | the client **hangs** (`R33_RC=124`) | the semaphore poll never satisfied ⇒ same class as `cup2`, at 1/10th the surface |
| **G8** | the binary does not run at all | ⊘ NOT a GPU result. The hook asserts `file`-static and prints the guest md5 for exactly this |
| **G9** | it works and the **guest driver logs an `Xid`** anyway | a completion we forged rather than served — the `c_rust_trace_differential` gap, made visible |
| **G10** | the guest has no `/dev/nvidia*` / no module | ⊘ a BOOT failure, not a client failure. Preconditions are printed before the run |

## ⊘⊘ WHAT THIS BOOT CANNOT PROVE — stated before it runs

- **It cannot say anything about the VA the GR engine faults on in `cup2`.** The ladder builds
  **its own** `FERMI_VASPACE_A` and probes **that**. The faulting channel is the guest driver's
  own client with its own PDB. ⇒ *"is that VA mapped?"* is **not** answered here, and a probe
  in the wrong address space is this campaign's recorded failure shape. R33 prints its own
  range handles so the scope is on the artefact, not in a reader's head.
- **A green G1 does not fix `cup2`.** It relocates the question, which is worth a lot and is
  not the same thing.
- **It is one workload, one chip, one driver, one boot.**
- **It cannot separate "our device model served this correctly" from "our device model forged
  a completion".** The completion plane has no C oracle (`c_rust_trace_differential.md`), and
  this rung adds none. G9 is the only arm that would see it, and only if the guest driver
  complains.
- **`total=53` matching is necessary, not sufficient.** A count cannot see a substitution.
  The arms are graded by **identity** — payload value, `GP_GET`/`GP_PUT`, the read-back words
  — never by count alone.
