# P1 source findings before hardware

STATUS: RESEARCH, 2026-10-18. CODE READ unless otherwise labelled. Source: production `0452613b5`.

## W2: event provenance and interrupt cost

`kf-qemu/src/chan.rs::nsi_engine_edge` records the engine type, guest subscription slot and vector; `nsi_fifo_edge` selects a vector for an armed FIFO subscription. Both call the loss-free `kf-chan/src/ptnsi.rs::Relay`. Neither is a vCPU path. An engine edge is GPU-wide, not attributable to an individual channel or VM.

**Provenance limitation:** `kf-chan/src/completions.rs::open` subscribes FIFO_EVENT_MTHD, and `ChanPlane::realize` additionally subscribes the asynchronous host copy-engine event to that SAME fd with `Completions::also`. Its worker callback is named `nsi_fifo_edge`, but the fd alone cannot distinguish FIFO from copy-engine notification. `kf-host/src/event.rs` registers DATALESS events: the host driver's poll clears a coalesced readiness flag; individual original hardware edges and multiplicity are unavailable. A diagnostic must label this source **completion-fd (FIFO or the configured host CE)**, not claim it observed FIFO alone. Options for finer attribution: retain the ambiguity and correlate existing host IRQ/marker traces (no behavior change); or separately design/test diagnostic-only source subscriptions. P1 selects the first within the lane's observation work; no subscription or delivery design changes.

`user_gate::Marks` emits SET_REFERENCE plus NON_STALL_INTERRUPT, including batch markers (`user_turn::Relay::end_batch`). INFERRED hypothesis, not a new measurement: three independently completed private batches can produce three required host interrupts even for one D3D12 API submit. The prior report measured three distinct host interrupts and six irqfd writes; six writes alone do not establish six original completions. Current `CpuIntr::latch` sends when the leaf/subtree is enabled; enabling a pending leaf/top also sends. Exact attribution requires the new trace plus existing irqfd/MMIO/ETW capture.

`ptnsi::Pacer` defaults to interval zero. Nonzero intervals set an owed bit and flush a trailing raise after the interval. Existing tests include no-further-edge trailing delivery, bursts, independent vectors and two-worker accounting. The worker ticks every loop and uses a **1 ms** park while owed, so a 20/50 us configuration does not promise a 20/50 us upper delivery latency. P1 will measure actual latency, never change the park constant to make an experiment look better.

The relay's 50 ms backstop counters (`user_loop::Stats`, especially `backstop_found_work`) are unconditional and print in the USER_RING_LOOP summary; per-relay Pointer backstop counters also exist. In contrast, `worker::WorkerStats::timeouts_with_work` only updates when profiling enabled, because `after_timeout` is set inside `if let Some(tw) = tw`. An unchanged zero in that latter counter with profiling off is not evidence of event-complete operation. Use the unconditional relay counters and record the distinction.

## W3: synchronous mask/unmask/W1C writes

`device::bar0_write_inner` decodes four-byte CPU_INTR writes, applies `CpuIntr::write` atomics immediately, republishes the relevant read shadow, then sends the requested message. LEAF is W1C, enable aliases are bitwise set/clear, TOP derives from leaves; pending unmasked by an enable sends an IRQ. The next guest load bypasses the VMM through the read shadow, so deferred application can expose stale state. A writable page cannot implement W1C/set/clear or preserve the common alias view by itself.

The [KVM API, IOEVENTFD and IRQFD](https://docs.kernel.org/virt/kvm/api.html) documents ioeventfd as signalling an event on a matched guest write and irqfd as injecting a guest interrupt on an eventfd signal. Neither interface applies arbitrary device mask/W1C operations or updates kayfabe's shadow synchronously. DATAMATCH identifies a known write value but does not create a synchronous application barrier for a following shadow load. IRQFD is already used for MSI-X vector 0. INFERRED conclusion: no equivalent cheaper mask-write path is established with these existing APIs; leave behavior unchanged. A kernel device-state implementation would be a new design, outside a report-only shortcut.

## W4: lazy observe and guest GET write-back

The governing design is `V3_USER_RING_LIFECYCLE_20261016.md` section 0c/D6. `userpump::lazy_config` reads PUT-cache and lazy-observe settings once at relay creation; default observation interval is 16, low-credit/wait/open-segment/full-ring guards force fresh observation. A skipped observation also skips a new GET write-back; an already owed write-back is retried. The worker's fresh progress look may ring for `pop_writebacks` while a gate still drains, to release a guest blocked on its own full ring. Suppressing GET write-back must preserve that guard and avoid repeatedly waking for deliberately suppressed pops. It is a separate opt-in behavior experiment requiring red-first tests and the full correctness gates, not a logging change.

No experiment or gain is claimed by this source reading. Every hardware falsifier remains untested.
