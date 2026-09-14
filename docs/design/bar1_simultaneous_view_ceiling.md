# BAR1: how much of one reserved vidmem object can be CPU-mapped at once

**STATUS: LIVE — measured 2026-09-14 (w722) on a real GA106.**
Bench: vast instance 51026490, RTX 3060 12 GiB, driver **580.159.03**, kernel 5.15.0-181,
CUDA container (not a VM — host-side RM only). Instrument:
`tests/bar1/bar1probe.c`, raw RM ioctls, **deliberately independent of the kayfabe stack**.

## The headline, in one line

**The BAR1 aperture is reclaimable — but ONLY via `NV_ESC_RM_UNMAP_MEMORY` (0x4F), which
this tree defines as a constant and NEVER CALLS.** `munmap` + `close(fd)` returns nothing.

## Q1 — Resizable BAR: NO. BAR1 is 256 MiB and that is the real budget.

From `/sys/bus/pci/devices/0000:01:00.0/resource` (what is actually programmed):

| region | base | size | kind |
|---|---|---|---|
| BAR0 | `0x52000000` | **16 MiB** | 32-bit non-prefetchable (registers) |
| BAR1 | `0x4000000000` | **256 MiB** | 64-bit prefetchable |
| BAR3 | `0x4010000000` | **32 MiB** | 64-bit prefetchable (NVIDIA "BAR2"/instance) |
| BAR5 | `0x4000` | 128 B | I/O |
| ROM | `0x53000000` | 512 KiB | disabled |

No `resource1_resize` sysfs attribute exists ⇒ the kernel sees no usable ReBAR capability
for BAR1. ⊘ **Caveat, stated because it is not closed:** the container lacks `CAP_SYS_ADMIN`,
so `lspci -vv` prints `Capabilities: <access denied>` and the PCIe *extended* config space
(where a ReBAR capability would live, ID 0x15) could not be read directly. The negative rests
on the kernel's own attribute, not on reading the capability. **It does not matter for the
design either way**: 256 MiB is what is programmed, so 256 MiB is the budget today.

## Q2 — the simultaneous ceiling: ~253 MiB, and it is *first-fit*, not fragmentation

Reservation is NOT the limit: **8192 MiB reserved fine** in the same process that could not
map 254 MiB. (The tree's old `6144` figure was already known to be a halving-search artefact —
`largest_reservable_mb`'s own comment. 8 GiB confirms it.)

Every refusal is `status=0x51 NV_ERR_NO_MEMORY` **inside the parameter struct**, with
`ioctl()` returning 0 and `errno == 0` — the ledger's `failed=0` trap, live.

| slice | views | total mapped | SMI free at ceiling |
|---|---|---|---|
| 1 MiB | 253 | **253 MiB** | 1 MiB |
| 2 MiB | 126 | 252 MiB | 2 MiB |
| 8 MiB | 31 | 248 MiB | 6 MiB |
| 32 MiB | 7 | 224 MiB | 30 MiB |
| 128 MiB | 1 | 128 MiB | 126 MiB |

⇒ usable pool ≈ **254 MiB** (256 total − 2 held by the idle host driver); you lose whatever
remainder is smaller than one slice. That is a plain first-fit allocator over a contiguous
range, **not** fragmentation — a single **192 MiB** mapping succeeds in a fresh process.

⚠ **An instrument failure worth keeping.** A bisect for "largest single mapping" reported
**128 MiB**, and it was wrong: each probe left its predecessor's aperture held (see Q3), so
later, larger attempts were measured against a shrinking pool. Direct singles at fixed sizes
(126/128/129/130/136/160/192 MiB all OK; 254 MiB refused) are the correct instrument.

## Q3 — reclaim: YES, but only on the explicit RM unmap. This is the finding.

Clean A/B, identical churn, 32 MiB slices, each round mapping **offsets never mapped before**
(so a success cannot be RM re-handing back a cached per-`(object, offset)` mapping):

```
A: munmap + NV_ESC_RM_UNMAP_MEMORY + close
   round 0..4 -> 7 views / 224 MiB EVERY round, offsets 0 -> 1120 MiB of the object
   unmap_failures=0 ; nvidia-smi returns to used=2 free=254
B: munmap + close ONLY                      (what this tree does today)
   round 0 -> 7 views / 224 MiB
   round 1..4 -> ZERO views, 0x51 NV_ERR_NO_MEMORY ; nvidia-smi stuck at used=226
```

⊘ **A first, weaker version of this test said "APERTURE IS FULLY RECLAIMED" and was a false
positive**: it remapped the *same* object offsets it had just released. Re-using an offset
succeeds even when the aperture was never returned. Only fresh offsets test reclamation.

### What this means for the single-store design

`HostRmBackend::export_device_view` arms a view with `NV_ESC_RM_MAP_MEMORY` and nothing in
the workspace ever issues `NV_ESC_RM_UNMAP_MEMORY`:

```
$ grep -rn NV_ESC_RM_UNMAP_MEMORY crates/ --include=*.rs | grep -v _DMA
crates/kayfabe-abi/src/submit.rs:70:pub const NV_ESC_RM_UNMAP_MEMORY: u8 = 0x4F;   # + 3 doc comments
```
— constant and prose only, **zero call sites**, while the *DMA* variant is wired in 8 files.
So a recycling BAR1 window built on today's primitives would leak the aperture after roughly
224 MiB and then refuse everything with `NoMemory` until the isolate process exits.
**Dropping a `DeviceView` is not a release.** That verb has to be built.

## Q4 — one global pool, shared with everything

- Idle host driver: **2 MiB** of 256.
- A live CUDA context (`cudaMalloc` 1 GiB + 64 MiB mapped host alloc): **5 MiB** used; our
  own 1 MiB-slice ceiling drops 253 -> **250 MiB** in lockstep.
- Process A holding 224 MiB ⇒ process B gets **0 views**. A exits ⇒ C gets 224 MiB back.
  **Process exit does release; per-view close does not.**

★★★ **And the constraint that bites hardest:** with 253 MiB held, `cudaMalloc` on the host
returns **`initialization error`** — CUDA cannot even create a context, because context
creation needs BAR1. It recovers the moment the hold is released. An all-resident BAR1 view
set therefore **makes the host GPU unusable for anything else, including our own isolate's
channel/USERD mappings.**

## Consequences

1. A 256 MiB guest BAR1 can be ~**98.8 %** resident (253/256) at 1 MiB granularity — but
   only by consuming the *entire* host aperture, which is not an option.
2. Recycling is therefore required, and it is **viable** — round-trip is clean and
   repeatable, 5/5 rounds at full size, provided 0x4F is issued.
3. The recycle verb costs one open `/dev/nvidiaN` fd per live view (RM gives each mmap
   context a one-shot node). 253 views = 253 fds; the default `RLIMIT_NOFILE` of 1024 is a
   ceiling nobody has budgeted for. The probe raises it to 65536 and reports it.
4. Budget the *working set*, not the aperture: leave headroom for the host driver (2 MiB
   idle) and every CUDA context on the box (~3 MiB each).
