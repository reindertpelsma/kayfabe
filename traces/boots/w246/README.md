# w246 — ROUTE B FIRES. The missing precondition was a flag nobody carried forward.

⊘⊘ **`CE-SUBMIT` is 0 in all four corners and nothing executed.** No line here may be read as
the first forwarded work. Revision **`acbb9a3`**, stamped inside both artifacts, all four boots.

## The square — four corners, two flags, `ce_executor=host` throughout

| corner | `PT_WITNESS_EXEC` | `RING_VIDMEM` | `RING-VA-UNBOUND` | `PushbufferAperture` | `RingFbNeverWritten` | **`CE-SUBMIT`** | doorbells |
|---|---|---|---|---|---|---|---|
| **A** `w245off_30f7900_rbo` | off | off | **8** | 0 | 0 | **0** | 183 srv / 8 ref |
| **B** `w245on_30f7900_rbn` | off | **on** | **8** | 0 | 0 | **0** | 183 srv / 8 ref |
| **C** `w246c_acbb9a3_witon_rboff` | **on** | off | 0 | **8** | 0 | **0** | 175 srv / **16 ref** |
| **D** `w246d_acbb9a3_witon_rbon` | **on** | **on** | 0 | **0** | **0** | **0** | 183 srv / 8 ref |

- **A vs B** — the vidmem flag alone changes **nothing** (w245's finding; it stands, scoped).
- **A vs C** — ★ **the witness is the variable that binds the ring.** `RING-VA-UNBOUND` 8→0,
  `rows=4 hit=NONE` → `rows=13348 hit=0x1024000/Vidmem/start0x200200000/len0x200000`, and the wall
  becomes `PushbufferAperture` **8**. **§16.86's premise, restored at the current revision.**
- **C vs D** — ★★★ **route B removes that refusal**: `PushbufferAperture` 8 → **0**.

⊘ **Neither flag alone gets there — which is why the square was needed.** The B/D pair alone
would have read as *"route B works"*; the A/C pair alone as *"the witness was all it needed"*.
Both wrong.

## ★★★ What route B actually did — all 8 doorbells, one line shape

```text
FWD-RING proc=2 chan=N key=K pdb=0x201000
    RING bytes=65536 cursor=0 live=1 spans=0
  → NOTHING FORWARDED (the ring decoded to no CE span; the doorbell still reports SERVED)
```

**64 KiB read out of our own emulated framebuffer.** One live GPFIFO entry. Zero CE spans. Its
pushbuffer, from the same boot's descent (`gp[0]@0x200224000 = 0x202c00000 + 0x20`, 8 dwords):

```text
pbm[8w of 32B]: [0] sub4 m0x0    n1 = 0xc7b5    SET_OBJECT = AMPERE_DMA_COPY_B
                [1] sub4 m0x240  n3 = 0x2       SET_SEMAPHORE_A/B/PAYLOAD
                [2] sub4 m0x300  n1 = 0x14      LAUNCH_DMA
```

★★ `0x14 & LAUNCH_TRANSFER_MASK(0x3) == 0 == LAUNCH_TRANSFER_NONE`, and this port's own ABI says
what that means, cited to the driver header (`kayfabe-abi/src/submit.rs:2042`,
`ogkm-580: clc7b5.h:86`):

> *"A launch with this moves **no bytes**; it exists to release a semaphore. Decoding one as a
> copy would report a transfer the engine never performs."*

⇒ **`spans=0` is the CORRECT decode.** These 8 doorbells carry a **semaphore-release-only**
`LAUNCH_DMA` — the CE channel's initialisation fence. There is **no copy in them to forward**,
and `CE-SUBMIT` 0 is the true content of this population rather than route B falling short.

## ⊘ Forbidden #2's residency gate — reachable for the FIRST time, and silent

- In corners A/B it was **unreachable** (downstream of the `RingVaUnbound` exit).
- In corner **D** the path is **live** (`fetch_ring_bytes` ran, 65536 bytes returned) and
  `RingFbNeverWritten` is **0**. ⇒ **reached, did not fire**, because the pages *had* been
  written. Correct outcome, first time on a live hardware path.
- ⊘ Stated in both directions, as asked: it did not fire; the path was live; the reason is
  residency being satisfied, not the gate being absent.

## ⊘ Boot health, all four corners

`no-blocking-under-lock` **0**; `RmInitAdapter failed` **0**; `SMI_RC=0`; `CUP2_RC=124` — the
standing `cuCtxCreate` wall, unmoved.

## ★ The next question

The first CE submission on these channels is a bare semaphore release. **A doorbell carrying a
real `LAUNCH_DMA` with a data-transfer type has not arrived**, because `cup2` walls at
`cuCtxCreate` before issuing one. ⇒ the next measurement needs a workload that reaches a copy, or
the `cuCtxCreate` wall cleared first. **Route B is no longer the blocker; it is instrumentation
waiting for traffic.**

⚠ §16.86.4's owner question is now **live**: enumerating the ring is agreed; these are user
`proc 2` doorbells, and what may happen *after* enumeration is the owner's call. This rung
enumerated and stopped — correctly, and because there was nothing in them to do.

Full record: `docs/design/execution_plane_increments.md` §16.98.
