# V3 — the BAR0 trace mode: kf3 recorded by the VFIO reference's own tracer

**STATUS: LIVE, 2026-10-09 — DIAGNOSTIC-ONLY, default off (owner ruling `OWNER_RULINGS.md` §X).**
Code and GPU-free tests on branch `claude/kf3-read-trace-20261008`; fast-guest runs on the bench
4070 are in §8. Never a production or performance configuration.

## 1. What it is

The VFIO reference (`scripts/bench/windows/vfio_dvi_reference.sh` on
`claude/vfio-dvi-reference-20261008`) runs the real RTX 4070 under vfio-pci in a patched QEMU 10.2.4
and records the guest driver with two tools:

1. **QEMU's own trace events** — `vfio_region_read`, `vfio_region_write` (every BAR0 access, because
   `x-gsp-observer` keeps BAR0 un-mmapped), `vfio_msi_interrupt` and the other `vfio_*` events of
   `trace-events-vfio-reference.txt`, written by QEMU's `log` trace backend with `-msg timestamp=on`
   into `-trace events=FILE,file=LOG`;
2. **the GSP observer** (`tools/vfio-gsp-observer`, the `x-gsp-observer=PATH` property): samples the
   guest's GSP command and status queues in guest RAM at the mailbox, queue-head, interrupt-status
   and interrupt-injection boundaries and writes `kayfabe-gsp-text/1` JSONL for `decode.py`.

Normally kf3 cannot be compared with that: its BAR0 reads never exit (the shadow pieces are ROM
devices in ROMD mode), so nothing records what the guest driver READS. The trace mode makes kf3 feed
**the same two tools**, so a kayfabe boot and a VFIO boot produce the same records and one analyser
(`mmio_window.py`, `vfio_msi_vectors.py`, `decode.py` + `vfio_events.py`, …) reads both.

## 2. What is shared, and what is not

**Shared — the same code in the same binary:**

| piece | VFIO side | kf3 side |
|---|---|---|
| BAR0 read/write records | `trace_vfio_region_read/_write` in `hw/vfio/region.c` | the same generated functions (`trace/trace-hw_vfio.h`), called from `kf3_piece_read_trace`/`kf3_piece_write_trace` |
| MSI records | `trace_vfio_msi_interrupt` in `vfio_msi_interrupt` | the same function, from `kf3_msi_user` |
| trace backend, file, timestamps | QEMU `log` backend, `-trace`, `-msg timestamp=on` | identical (same options) |
| GSP observer | `gsp_observer_open/_mmio/_irq/_reset/_close`, `gsp_observer_is_trigger`, the writer, `record`, `guest_ram` (`hw/vfio/gsp-observer.c`), the core `vg_*` (`observer.c`, `queue.c`) — reached through the thin `vfio_gsp_*` wrappers | the same `gsp_observer_*` functions, from `kf3_obs_mmio` and `kf3_msi_user` |
| observer output | `x-gsp-observer=PATH`, `kayfabe-gsp-text/1`, same caps (32 MiB FIFO, 64 MiB, 100 000 records) | `x-gsp-observer=PATH` on `kf3-gpu`; same file, same decoder |

The observer was made device-agnostic for this (2026-10-09): the `VFIOPCIDevice`-typed body of
`gsp-observer.c` became `GspObserver` with a `PCIDevice *` (identity DMA address space, unplug
blocker, NVIDIA vendor check); vfio-pci's VFIO-only preconditions (`x-no-kvm-*`, migration off) stay
in `vfio_gsp_open`. The header line gains `"device":"vfio-pci"|"kf3-gpu"` (an added key; `decode.py`
accepts it). vfio-pci's behaviour is otherwise unchanged, and the observer's own tests (`test.sh`,
`test_writer.py` against the built tree, `test_apply.py`) pass on the refactor.

**Not shared, and why:**

- **The answer.** vfio-pci forwards an access to the real GPU; kf3 serves it from its model. That is
  what is being compared.
- **The hook point.** Each device's own MMIO dispatch calls the tracer: vfio's `vfio_region_read`
  (`region.c`), kf3's trace-mode `MemoryRegionOps` (`kf3_piece_ops_trace`). The order is vfio's:
  observer before the access is served, trace record after it.
- **The interrupt source.** vfio's MSI eventfd is written by the host kernel (with `x-no-kvm-msi*`
  the main loop handles it); kf3's is written by Rust. In trace mode kf3 also takes the main-loop
  path (`kf3_msi_user`: observer, record, `msix_notify`) instead of KVM irqfds.
- **The device name** in a record: vfio prints its host BDF; kf3 prints the BDF of the host GPU it
  runs on (`0000:01:00.0` on the bench), overridable with `KF3_TRACE_NAME`.
- **Coverage gaps on kf3** (reads that never exit even in trace mode, so they are not recorded):
  PRAMIN (`0x700000-0x7fffff`, plain RAM), the usermode page (`0xBB0000` on Ampere+, host
  passthrough) and the timer page (host passthrough). Writes to the usermode and timer pages are
  recorded. vfio-pci records all of them. Hopper+ BAR1 usermode doorbells are recorded by neither
  (vfio keeps BAR1 mmapped).
- **Reset.** kf3 has no device reset (a fresh QEMU per guest boot), so `gsp_observer_reset` is not
  wired on kf3.
- **The observer's capture window** is a property on kf3 (`x-gsp-observer-seconds`, 1-7200, default
  300); vfio-pci's stays fixed at 300 s.

## 3. Clocks, and a correction to the reference harness's comment

- Trace lines: `g_date_time_new_now_utc()` at the call (`util/log.c`), i.e. host **wall clock**, UTC,
  microseconds — taken before the log lock, on the calling (vCPU or main-loop) thread.
- Observer `qpc` / `started_qpc`: `qemu_clock_get_ns(QEMU_CLOCK_REALTIME)`, which is
  **CLOCK_MONOTONIC** ns, not the wall clock. Measured 2026-10-09 on the bench: kf3's
  `started_qpc=271085629395963` against `/proc/uptime` 271111 s a few seconds later.
- So both boots carry the same two clocks with the same relation; correlating a trace line with a
  `qpc` needs one anchor per boot (as the reference's `boot*-clock.txt` does). ⊘ The header comment
  of `vfio_dvi_reference.sh` (other branch) calls `qpc` "the same wall clock"; it is the monotonic
  clock.

## 4. What exits, and that the answer does not change

- `KF3_BAR0_READ_TRACE=1`: every **shadow piece** that overlaps a selected read range gets ROMD off
  for the whole run (`kf3_bar0_build`), so every read in that piece exits to `kf3_bar0_read`, which
  answers from the **same shadow word** ROMD would have shown (`Device::shadow_read`; live registers
  the model publishes into the shadow are served the same way). The unit is the piece, not the
  range: on Ada the display range lies in the piece `0x0-0x6fffff`, so all its reads exit; only the
  selected ones are recorded (`unselected_exits` in the report counts the rest).
- Writes already exit; in trace mode they are recorded too (`KF3_WRITE_TRACE_RANGES`, default all).
- MSI-X goes through the main loop (above). Rust raises it exactly as before.

## 5. Bounds (hostile guest)

- **kf3's records:** every record is admitted by Rust first (`readtrace::AccessTrace::admit`,
  lock-free): selection, then a record cap (`KF3_READ_TRACE_MAX_RECORDS`, default 8 000 000) and a
  byte cap (`KF3_READ_TRACE_MAX_BYTES`, default 512 MiB, at most 16 GiB) counted with the exact
  length of the line QEMU writes (timestamp at its longest; unit-tested against lines from the VFIO
  trace and across digit-count boundaries; measured equal to the file size in run `rt_smoke1`). The
  first refusal spends the trace: everything after is dropped and counted. The exit report states
  records, bytes, drops and whether the cap was hit.
- **The observer:** its own caps as on vfio-pci, plus the capture window.
- **Locks.** A record is written synchronously by QEMU's log backend under its log-file lock on the
  calling thread — exactly as vfio-pci does. That lock is a leaf (no holder waits on a vCPU), so it
  cannot deadlock; it can be slow, which the ruling accepts. ⊘ A queue to a writer thread was not
  used: the backend timestamps at the call, and the point is the same tracer. The observer's
  producer needs the BQL (as on vfio-pci, where every BAR0 access holds it); kf3's lockless vCPU
  takes it only at the observer's six trigger offsets, and the main-loop MSI path already holds it.
- With the mode off, none of this exists: see §6.

## 6. Default off — the proof

- Rust: `TraceConfig::from_env` is off unless `KF3_BAR0_READ_TRACE` is `1`/`on` (a typo refuses
  realize by name); off, `kf3_trace_mode` is 0, `kf3_trace_piece` and `kf3_trace_admit` are 0
  (`readtrace::tests::the_default_configuration_is_off_and_gates_every_path`).
- C (`crates/kf-qemu/tests/bar0_trace_gate.rs` reads `kf3.c`): `tr_on`/`diag` are set once, only from
  `kf3_trace_mode` and the unset-by-default `x-gsp-observer`; the trace ops are chosen in one place
  under `diag`; `kf3_piece_read`/`kf3_piece_write` (the default vCPU functions) are unchanged and
  contain no trace call; ROMD is turned off only under `tr_on && kf3_trace_piece`; MSI-X keeps the
  irqfd vector notifiers unless `diag`; every `trace_vfio_*` and `gsp_observer_*` call sits in a
  function reached only from those paths.
- Hardware: the 30-arm suite with the mode off (§8).

## 7. How to use it

Events file: `scripts/bench/trace-events-vfio-reference.txt` (the reference's boot3 list).

**Fast guest** (one arm; PMC_BOOT_0 and the GSP registers added to the display range):

```sh
KF3_BAR0_READ_TRACE=1 KF3_READ_TRACE_RANGES=0x0-0xfff,0x110000-0x110fff \
KF_TRACE_EVENTS=$PWD/scripts/bench/trace-events-vfio-reference.txt \
KF3_DEV_EXTRA=x-gsp-observer=/workspace/bench/fast_TAG_gsp.jsonl \
KF_ARMS=--timer bash scripts/fastguest/run_fast_guest.sh TAG 180
# -> /workspace/bench/fast_TAG_trace.log, fast_TAG_gsp.jsonl; report line in fast_TAG_qemu.log
```

**Windows display trace** (`win_vm.sh run`; the display range is always included):

```sh
export KF3_BAR0_READ_TRACE=1
export KF3_READ_TRACE_RANGES=0x110000-0x110fff,0xb81000-0xb81fff   # GSP queue/mailbox, interrupt leaves
export KF3_READ_TRACE_MAX_BYTES=2G                                 # default 512M
export WINVM_TRACE=1 WINVM_GSP_OBSERVER=1 WINVM_GSP_OBSERVER_SECONDS=3600
bash scripts/bench/windows/win_vm.sh run --tag TAG ...
# -> $VM/logs/TAG_b<n>_trace.log, TAG_b<n>_gsp.jsonl, report "kf3: BAR0-TRACE report" in TAG_b<n>_qemu.log
```

Without `win_vm.sh`: add `-msg timestamp=on -trace events=…/trace-events-vfio-reference.txt,file=LOG`
to QEMU and `x-gsp-observer=FRESH_PATH[,x-gsp-observer-seconds=N]` to `-device kf3-gpu`. The trace
mode refuses `doorbell-ioeventfd=on` (ioeventfd doorbells would bypass the record). The binary must
come from a QEMU tree with the observer installed (`python3 tools/vfio-gsp-observer/apply.py
<qemu-10.2.4>` on a pristine tree, then `scripts/bench/build_kf3.sh <tree> <build-dir>`); a tree
without it still builds kf3, and `x-gsp-observer` is then refused by name.

Decode and analyse exactly as for VFIO:
`decode.py GSP.jsonl --jsonl --all-records > decoded.json; vfio_events.py decoded.json`,
`mmio_window.py TRACE.log FIRST LAST`, `vfio_msi_vectors.py TRACE.log`.

## 8. Results on the bench

See the commit that adds this section's numbers (fast-guest runs on the bench 4070, revision named
there).
