# V3 relay lifecycle log: the lines, their format, and the rules they obey

**STATUS: LIVE, 2026-10-24 (logging only, GPU-free tests; not run on hardware).** Branch `claude/relay-observability-20261024`.
This document is the stable format of the relay observability lines. It adds log lines and no behaviour: no decision of the
Windows USER-ring relay reads any of it. The lifecycle machine itself is `V3_USER_RING_LIFECYCLE_20261016.md`; this file only
names what it says aloud.

## 1. Why

The Q0 log census (`/root/host_q0_20261023.md`, 2026-10-24) read every qemu log with lifecycle counters and could not decide
whether OptiX / CUDA channels leave full scanning, and it found graphics channels that stayed in Scan for millions of entries
without ever binding a software object. The logs lacked the data: no revision line, no process / class / engine on the per-relay
line, no bind line, nothing for relays alive when QEMU stops (4,815 of 17,542 relay births printed no lifecycle line), and no
relay totals. Those are the five additions below.

## 2. The lines

Every line is one line of `key=value` words after a fixed prefix, printed with `eprintln!` (so it lands in `qemu.log`).

| line | when | how often |
|---|---|---|
| `kf3: BUILD rev=<hex sha\|unknown> dirty=<0\|1\|unknown> pkg=<version>` | device realize, first line | once per run |
| `kf3: WORKLOAD label=<label>` | device realize, only if `KF3_WORKLOAD_LABEL` is set and non-empty | once per run |
| `kf3: USER_RING_BOUND <key> to=<Pointer\|Window> scan_entries=<n> pid=.. classes=.. engine=..` | the scan saw the software object bound and the relay left Scan | at most once per relay |
| `kf3: USER_RING_SCAN_LONG <key> threshold=<n> scan_entries=<n> pid=.. classes=.. engine=..` | a relay still in Scan reached 100,000, 1,000,000 or 10,000,000 Scan entries | at most three per relay, once per threshold |
| `kf3: USER_RING_LIFECYCLE <key> mode=<Mode> Counts {..} pid=.. classes=.. engine=.. bound=<yes\|never> scan_entries=<n> at=<retire\|exit>` | the relay's final line: its release, or device stop if it is still alive | at most once per relay |
| `kf3: USER_RING_LIFECYCLE SUMMARY <old keys> relays_born=.. relays_bound=.. ...` | device stop, and the pre-existing "all relays gone" prints | once at stop (the interim prints keep their old bound) |

`<key>` is the pre-existing `{:x?}` of `(client, handle)`, for example `(c1d000bf, 5)`, the same text in the BOUND and
LIFECYCLE lines (and in the other per-relay lines of the pump), so the lines of one relay join on that string.

### 2.0 The in-run line (`USER_RING_SCAN_LONG`)

Added 2026-10-24 on the coordinator's request, because a QEMU that is killed prints no `at=exit` line and no SUMMARY (section 6).
`Lifecycle::forwarded` compares the Scan entry count with the next of `SCAN_LONG_THRESHOLDS` (`[100000, 1000000, 10000000]`)
and queues one `Notice::ScanLong` per threshold; the pump prints it from `report_lifecycle_notices`, the thread that prints the
other notices. A relay that binds before a threshold never says it for that threshold or later (it left Scan); one that binds
at 150,000 entries has already said `threshold=100000`. The lines carry the same `pid classes engine` fields as BOUND. They are
bounded by 3 x the number of relays, independent of the entry counts (test
`the_scan_long_flood_prints_an_exact_bounded_number_of_lines`: 2,000 relays at 20 and at 20,000 entries print none; 100 relays
at 1,000,000 print exactly 200; 50 at 1,000,001 print exactly 100). The hot-path cost is one compare per Scan entry.

### 2.1 Identity fields

`pid=0x<hex>|none`, `classes=<Kind>+<Kind>...|none`, `engine=0x<hex>|none`.

- `pid` and `engine` are the channel's declared `ProcessID` and `engineType`, the values `CHAN-FACTS` already prints (as
  `ProcessID=Some(..)` and `engine=Some(..)`). Nothing is captured anew; the value is stored at relay birth from the
  already-parsed channel allocation (`kf_rm::chanlink::ChannelAlloc`).
- `classes` is the set of engine-object kinds kept on the channel, as `kf_chip::classes::Kind` names (`Compute`, `ThreeD`,
  `DmaCopy`, `Usermode`, `TwoD`, `InlineToMemory`, `VideoEncoder`, `VideoDecoder`, `OpticalFlow`, `ChannelGpfifo`) in the order of
  `Kind::ALL`, joined by `+`. It is one bit per kind in an atomic, set when `keep_engine_object` keeps an object
  (wait-free, on the thread that already holds that lock). The software object is not an engine object and is not in the set.
  The set at the BOUND line is the set at the bind (the objects a SET_OBJECT can name exist by then); the set at the final line
  is the set at retire or exit.
- The process id is the guest's declaration, never trusted for a decision: it is printed only.

### 2.2 Bind fields

- `bound=yes` means the scan saw the 5080-class software object bound and the relay left Scan (`to_pointer` or
  `to_window_exact` nonzero). `bound=never` means the relay stayed in Scan for its whole life. Under
  `KF3_LIFECYCLE_MODE=always_scan` the machine never leaves Scan by construction, so every relay reads `bound=never`; read the
  `mode=` of `USER_RING_LIFECYCLE_MODE`.
- `scan_entries` is the number of entries forwarded in Scan: before the bind when `bound=yes`, over the whole life when
  `bound=never`. It is `Counts.entries_scan` at that point, repeated so the field is a plain integer next to the verdict.
- `to=` in the BOUND line is `Pointer` (no registration was live) or `Window` (a registration was live: the scan continues as an
  exact Window).

### 2.3 SUMMARY keys

The pre-existing keys come first and keep their order and names (`entries_scan=.. entries_pointer=.. entries_window=.. ...
set_object_5080_max_per_channel=.. window_entries_max=..`). Appended, in this order:

`relays_born` (pumps created; a birth that fails after its pump exists is still counted), `relays_bound`, `relays_never_bound` (`born - bound`, includes relays that forwarded nothing),
`relays_switching` (relays with at least one Pointer to Window or Window to Pointer flip, each once),
`relays_never_bound_with_entries` and `entries_scan_never_bound` (relays that forwarded entries and never bound, and their
Scan entries, accumulated when their final line is printed), `relays_exit_reported` (live relays that got an `at=exit` line),
`relays_exit_busy` (live relays whose record stayed busy for the 100 ms budget: no line, counted).

"Entries scanned by mode" is the pre-existing `entries_scan`, `entries_pointer`, `entries_window`.

## 3. Rules the lines obey

- **Once per event.** BOUND: the machine queues one `Notice::Bound` per channel (a `bound` field guards it); the pump drains it
  in the same turn. LIFECYCLE: `Pump::report_final` is guarded by a per-relay flag, so retire after an exit line prints nothing (an exit pass that finds a relay with no entries prints nothing and leaves the retire line free). BUILD,
  WORKLOAD: once at realize. Never per entry and never per guest-controlled count; the number of lines is a function of the
  number of relays (bounded by the channel quota), not of the entries they forward (test
  `a_flood_of_relays_and_entries_prints_a_bounded_number_of_lines`).
- **Threads.** BOUND is printed from the relay's turn (a worker thread, where `USER_RING_WINDOW_FRESH` and the other notices
  are printed). The `at=retire` line is printed from `finish_user_ring_free` (the thread that frees the channel), the same place
  as before. BUILD, WORKLOAD, the `at=exit` lines and the SUMMARY are printed from `Device::realize` and `Device::stop` (QEMU's
  main thread, where the SUMMARY was already printed). Nothing is printed on a vCPU thread.
- **No new lock on the vCPU path, nothing blocks.** The class set is an atomic. The exit pass takes the relay table and each
  record with `try_lock`, retrying for 100 ms in total at most; a busy record is counted, never waited for.
- **Format is stable.** New keys are appended after the last existing key of a line, never inserted. `at=` is the last word of
  the LIFECYCLE line. The prefix `kf3: USER_RING_LIFECYCLE <key> mode=<Mode> Counts {` is unchanged, so a parser that read the
  old line (join on `<key>`, read `Counts {..}`) reads the new one; a parser anchored on the closing brace at the end of the
  line must be loosened to `Counts \{[^}]*\}`.
- **Hostile guest.** Every value is host-owned or already host-parsed and bounded: `pid` and `engine` are `u32`, the class set
  is at most ten names, the label of `WORKLOAD` comes from the host's environment and is cut to 64 characters of
  `[A-Za-z0-9._:/+=@-]` (anything else becomes `_`), the revision is hex only.

## 4. The revision line

`scripts/bench/build_kf3.sh` exports `KF3_BUILD_REV` (the 8-hex revision `kf3-bins/<rev>` already uses) and
`KF3_BUILD_DIRTY` (`0` or `1`) for its `cargo build`; `crates/kf-qemu/src/buildinfo.rs` reads them with `option_env!`. There is
no build script. A build made any other way (a plain `cargo build`, a CI build) prints `rev=unknown dirty=unknown`.

## 5. The workload label

`KF3_WORKLOAD_LABEL=<name>` in QEMU's environment. Class M (labelling, never needed to boot; see `V3_FLAG_INVENTORY.md`). It
changes no behaviour: the value is only echoed in `kf3: WORKLOAD label=<name>`.

## 6. What the lines cannot do

- **The `at=exit` lines and the stop-time SUMMARY need `Device::stop` to run, and today it runs only when the PCI device is
  unrealized.** QEMU 10.2.4's `qemu_cleanup()` (`system/runstate.c`) ends with "TODO: unref root container, check all devices are
  ok": it does not unrealize devices, so `kf3_dev_exit` and `kf3_unrealize` are not reached by a QMP `quit`, by SIGTERM or by a guest
  poweroff (read from that source file; a log check is `grep -c bar12_reads= qemu.log`, which counts the line `kf3_dev_exit`
  prints). Only an exit notifier (`qemu_add_exit_notifier`, run through `atexit`) runs at process exit, and kf3 registers one only
  for the trace mode. Until such a notifier calls the stop path (a proposed six-line change to `kf3.c`, not applied, see the report
  of this branch), the `at=exit` lines and the stop SUMMARY appear only on a device hot-unplug. The interim `(all relays gone, #n)`
  SUMMARY and the `at=retire` lines are unaffected. SIGKILL runs no code at all, with or without the notifier.
- **`USER_RING_SCAN_LONG` is the line that survives every way QEMU can end**: it is printed during the run, so a killed QEMU, or
  one that never unrealizes, still names a relay that sat in Scan for 100,000 / 1,000,000 / 10,000,000 entries.
- A relay that forwarded no entry prints no final line, as before; it is in `relays_born` and `relays_never_bound`.
- `classes` lists engine objects only. The 5080 software object is visible through `bound=`, not through `classes`.

## 7. Reading it

- Which workload leaves Scan, and after how many entries: `USER_RING_BOUND`, filter on `pid=` and `classes=`.
- Which relays never leave Scan and how long they scan: `USER_RING_LIFECYCLE` with `bound=never`, sort on `scan_entries`; the
  `entries_scan_never_bound` of the SUMMARY is their total.
- Which relays were still alive at the end: `at=exit`.
- Which revision wrote the log: the `BUILD` line.
