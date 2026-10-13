# Replacement-slot cursor discriminator

**STATUS: RESEARCH, 2026-10-13.** Native four-case discriminator PASS. No product
runtime change. The test plan below was committed before the hardware run.

## Measured result

Source `242647766f0b7b9cac643ffcae36e7c5539aec38`, RTX 4070 / stock 595.91.07,
2026-10-13 01:19:38–01:19:55 UTC. [Results](result/results.json) and raw process,
health, precise journal, source/SHA256 and build logs are retained alongside.

| Caller | Mode | Slot 0 marker | Slot 1 marker | Replacement GET | Old GET | Xids |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| UID 65534 / CapEff 0 | NOP | 0 | 0 | 0 | 1 | 0 |
| UID 65534 / CapEff 0 | Trigger | 10 | 2 | 2 | 1 | 0 |
| UID 0 | NOP | 0 | 0 | 0 | 1 | 0 |
| UID 0 | Trigger | 10 | 2 | 2 | 1 | 0 |

All four processes return 0 with successful owned-resource cleanup. The
H-RESET falsifier did not occur: replacement slot 0 was fetched despite old
GET 1. The start-at-slot-1 alternative is falsified for this fixture. This does
not distinguish an unconditional reset to zero from honoring the replacement
USERD's seeded zero, nor establish arbitrary seeded values or wrap behavior.
Compile-only `cargo test` passes with zero unit tests; hardware observations,
rather than that compile check, establish the result. The host was left idle,
no QEMU, GPU at 9 MiB, IOMMU DMA-FQ, shared lock released. No Vast instance used.

## Original test plan

Build only the bench binary after acquiring the shared GPU lock and confirming
no QEMU and DMA-FQ.

H-RESET: after relocation, replacement USERD seeded GET 0 causes replacement
slot 0 then slot 1 to be fetched. **Falsifier:** slot 0's marker stays 0 while
slot 1 writes 2 and replacement GET reaches 2. Conversely, observing slot 0's
marker 10 falsifies the start-at-slot-1 alternative for this fixture. Any
registration/cleanup/health failure, missing slot 1 effect or Xid leaves the
cursor hypothesis unresolved. Unknown inner RM status stays unknown.

The existing owned 64 KiB device-local fixture and 32-to-64 entry relocation
remain unchanged. Cursor modes replace only slot 0's duplicate PB with a
separate native WFI semaphore release to offset 0x8020, payload 10. Slot 1 keeps
the original PB/marker 2 at offset 0x8010. Every marker starts zero; CPU never
writes completion values. The replacement USERD GP_PUT is 2, GP_GET starts 0.
There is no wrap and no other reference to either marker. Final old USERD GET
is collected separately rather than assumed.

Run matched NOP/trigger pairs as UID 65534 (CapEff zero), then UID 0, each fresh
process. Identical 128 post-tail kicks and two-second backing hold; supervisor
observes another two seconds for delayed Xids, with microsecond journal windows.
Use `run.py <binary> <new-output> <source-revision> cursors`.

This tests the first fetched slot for this prepared fixture and host driver.
It does not establish arbitrary seeded cursors, ring wrap/capacity, in-flight
old work, cross-entry parser continuity, sysmem/private USERD behavior, Windows
relay cutover or any other driver. No product completion or lifetime is
released by a timeout. The earlier four-case positive reference is retained.
