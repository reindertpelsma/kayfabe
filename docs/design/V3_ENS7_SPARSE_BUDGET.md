# VRAM-scaled sparse budget and visible missing coverage

**STATUS: LIVE implementation, 2026-10-27; GPU-free validation.** The sparse twin stays default OFF.
ENS7 supersedes the fixed object limits in the ENS5/ENS6 design history. No ENS7 hardware run is claimed.

## Policy and evidence

CODE READ, `sparse/policy.rs`, `channel.rs` and `device.rs`: QEMU configures its immutable host-session policy from the configured guest framebuffer before any host-fact queries or selective-space creation. The common native reservation guard and each new guest tree use that same policy. Only the tree's native objects consume it; initial roots count, successful frees return slots, and failed frees retain their ownership accounting. Plain mappings keep the existing best-effort routing contract.

```text
per_vm    = clamp(fb_bytes / 65536, 8192, 131072)
per_space = min(65536, per_vm)
```

Integer division rounds down. No multiplication or guest-address-derived allocation is involved in this policy calculation. The QEMU configuration is host-owned; even an extreme value cannot exceed the hard ceiling. Callers without configured framebuffer size use the minimum policy. A used/configured policy cannot later change under existing trees.

| Guest framebuffer | Objects per space | Objects per VM/session |
|---|---:|---:|
| 128 MiB or less | 8192 | 8192 |
| 1 GiB | 16384 | 16384 |
| 2 GiB | 32768 | 32768 |
| 4 GiB | 65536 | 65536 |
| 8 GiB or more | 65536 | 131072 |

MEASURED in the coordinator's ENS6 native campaign, 2026-10-27, `traces/ens6_20261027/rm-bench-percentiles.txt` and `rm-bench-memory.json`: both reservation grains completed128000 objects without refusal. Allocation p50 was about10–20us; free p50 grew from about14–17us at1000 to70–73us at128000. Host memory delta at128000 was about310MB, with smaller samples obscured by host-memory drift. These are own-client bare-space results, not measurements of a live Windows twin.

INFERRED from the ENS6 native campaign 2026-10-27, `rm-bench-memory.json`: the131072 cap corresponds to roughly330MB of native RM object cost using2.5KB/object. This is a modest extrapolation beyond the128000 sample, not a measured refusal limit or a complete host-memory budget. Kayfabe's own tree, interval trackers and transaction copies also consume bounded memory. In particular, the inherited512-interval occupancy cap gives a conservative storage ceiling of about1.003GiB over131072 objects, plus tree/container overhead; a local undo copy can add up to about513MiB at the65536-space ceiling. Typical contiguous mappings need one interval. ENS7 deficit tracking adds at most256 intervals per space, plus bounded container overhead. The finite object cap also bounds the number of live spaces because every selective space must own its initial roots.

CODE READ, `sparse/slice.rs`: ENS6's100ms/4096-started-layout-verb allowance is unchanged. Increasing the tree size does not increase that allowance; unstarted work resumes in later refreshes. A started replacement and necessary repair may overshoot, and an individual native ioctl has no hard software wall-time guarantee. Diff map/unmap/invalidate and full space teardown remain outside that layout allowance. The native campaign's128000-object free totals were about8.8–9.3s, so larger trees have a real teardown cost; no claim that all teardown holds are100ms is made.

Hypothesis: reducing budget-driven invalid fallback removes the suspected missing-sparse trigger in Enscape. Falsifier for the coordinator's next Windows run: the same guest153/stall with zero current missing sparse coverage at onset. A green CPU model is not a Windows result and does not authorize default ON.

## Permanent exclusions

CODE READ, `channel.rs::GUEST_VA_RANGES`, `MirrorVaStart::low_range` and `Reservations::new_guest`: only the initial guest-owned reservation ranges can become sparse. Splitting or recycling never expands that authority. For a floor-started twin these are `[64KiB,1MiB)`, `[4.5GiB,HOST_HOLE_LO)`, and `[1TiB,2^47)`. `HOST_HOLE_LO` is64GiB below1TiB. Host-default twins omit the low root.

The complement includes the RM/split window `[1MiB,4.5GiB)` and the host-placement hole `[HOST_HOLE_LO,1TiB)`. The latter contains GR context placement and Kayfabe's ring region in its top4GiB. `channel.rs` records RM's refusal for the low window and the need to reserve the guest roots before host context birth. The boot address `0x19200000` falls in the low excluded window.

A sparse request intersecting an excluded window receives counted `Ok(false)` before cloning, splitting, recycling or native reservation. At most four permanent windows exist; one bit remembers whether each window was encountered, so `withheld_windows` increases once per window per logical space life. Per-attempt admission/decline counters still count calls. This decision does not expire with the transient RM-refusal cache or its back-off. A mixed request crossing an excluded window conservatively declines as a whole; already sparse coverage is preserved and the current-deficit calculation excludes it.

No zero-filled sparse result is fabricated. The fallback installs no reservation or mapping: an otherwise unmapped VA remains plain invalid and faults. Existing host-owned placements and backed mappings keep their existing ownership/permission checks. This is the documented closed negative control, not faithful sparse semantics in the withheld window.

## Current deficit and warnings

CODE READ, `sparse/deficit.rs` and `channel/sparse.rs`: `unhonoured_bytes` remains a cumulative per-admission event counter. The separate `current_unhonoured_bytes` is the union of accepted guest-sparse VA that lacks actual sparse reservation coverage. Repeated or overlapping declines do not double count. Partial progress is reconciled against the actual tree; a successful retry removes coverage from the deficit, retirement subtracts exactly the retired range, and successful clear/destruction removes that space's contribution. Failed clear preserves the gauge.

The interval tracker holds at most256 disjoint intervals. If hostile fragmentation exceeds this bound, it switches to the whole supported VA span as a conservative upper bound and sets `missing_capped=true`. The number is then explicitly an upper bound, not an exact byte count. Partial retirement cannot silently erase this uncertainty; a complete retirement or successful space reset clears it. A zero exact gauge must never be manufactured by cache eviction or tracker overflow.

The heartbeat prints `budget_space`, `budget_vm`, existing live/peak object gauges, `current_unhonoured_bytes`, `missing_capped` and `withheld_windows`. Native callbacks update session gauges; space destruction removes its contribution.

At1MiB or more of current missing coverage, a default-on `SPARSE_UNHONOURED` warning identifies the native space, byte count, cap flag and first four missing ranges. A capped tracker prints its enclosing span. The warning is a single bounded line, with a30s per-space cooldown and at most32 warnings over the host session. A recovery below threshold suppresses warnings. No diagnostic environment flag is required.

## Validation scope

The EXPECTED RED commit records13 assertion failures on the ENS6 behavior. The tests include a20000-tile demand: a large framebuffer eventually honours every tile across ENS6 slices; the small policy counts the decline without exceeding its limit. Additional tests cover union accounting, partial retirement, cap visibility, failed clear, warning truncation/cooldown and shared-session budget recovery. Source-composition checks supplement those runtime tests for QEMU initialization, common allocation guards, heartbeat fields and destruction wiring.

All existing ENS5/ENS6 model invariants remain required. `tsgmirror.rs` and the default-OFF switch are unchanged. Results, mutation killers and source seals are in `traces/ens7_20261027/`; no host or GPU was used for this implementation.
