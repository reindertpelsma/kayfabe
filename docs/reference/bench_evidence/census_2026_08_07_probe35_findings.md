# Boot census, 2026-08-07 — the notifier-35 probe MOVES the wall, and surfaces the aliasing bug

Source revision **`6c51da7`** (archive rev stamped in the binary; QEMU 10.2.4 + KVM, bench
`vh` = RTX 3060 GA106, guest NVIDIA open 580.159.04). Two boots, both through
`scripts/bench/boot_capture.sh`, dmesg + census committed beside this file.

## What was measured (not inferred)

| boot | probe set (from the boot's OWN census line) | dies at | census `0x20800301` |
|---|---|---|---|
| `census_stock` | `EMPTY (shipping configuration)` | `RmInitAdapter … Cannot load state` (`0x25:0xffff:1249`), scrubber `RegisterCallback` | event 194 served; **event 35 REFUSED 0x56** |
| `census_probe35` | `[35] — PROBE BOOT, reachability only` | `memmgrMemUtilsChannelSchedulingSetup … NV_ERR_GENERIC @ mem_utils_gm107.c:1027` | event 194 served; **event 35 served (obj 0x0b); event 35 on obj 0x0c REFUSED 0x56** |

★ The probe property works end-to-end on real hardware and **the boot's own report states
the set it ran with** — the exact anti-trap the property replaced an env var to buy: three
earlier boots ran probe-off while looking armed from the launching shell.

## Finding 1 — index 35 is a GATE, not a hang (the question the probe existed to answer)

Refusing the completion-notifier arming (`NV2080_NOTIFIERS_FIFO_EVENT_MTHD`, index 35) stops
the boot at `_memmgrMemUtilsScrubInitRegisterCallback`. Serving it (via the probe) advances
the guest **past** that site into `memmgrMemUtilsChannelSchedulingSetup`. So the guest does
**not** block on the callback during `RmInitAdapter`; it registers it and continues. This is
a reachability result, not a rung: index 35 is a completion notifier, and the honest fix is
to **deliver** the event, not to admit the arming — the probe is off in every shipping boot.

## Finding 2 — MEASURED: the device-global `notify_actions` aliases two subdevices (H1 confirmed)

This is the live hypothesis the census (`03a5c31`) was built to settle, now settled on
hardware. Under the probe, the census shows **three** armings:

```
arming event 194 action 2 client 0xc1e00002 object 0xcaf00001 result 0x0        (POWER_RESUME, served)
arming event  35 action 2 client 0xc1e00005 object 0x0000000b result 0x0        (scrubber ch.1, served)
arming event  35 action 2 client 0xc1e00006 object 0x0000000c result 0x56 REFUSED  (scrubber ch.2)
```

The GA106 scrubber sets up **two** channels (`fmb_real_ga106.txt`: `runQueues=2`). Each arms
completion notifier 35 on its **own** subdevice (`object 0x0b`, then `0x0c`). RM's
already-armed transition rule is **per-subdevice** (`ogkm-580:
subdevice_ctrl_event_kernel.c:126-131`), but `InitTablePolicy::notify_actions` is a single
**device-global** `[u8; MAXCOUNT]` array — so once index 35 is REPEAT-armed for `0x0b`, the
second REPEAT arming for `0x0c` is refused as an illegal REPEAT-over-REPEAT transition. That
`0x56` is what propagates up to `NV_ERR_GENERIC @ mem_utils_gm107.c:1027`.

⇒ **The next increment is a per-subdevice `notify_actions`.** The array must be keyed on
`(hObject, event)` — or scoped to the subdevice the arming arrived on — so two subdevices
arming the same index are two independent states, exactly as RM keeps them. The census rows
above are the regression fixture for it: two rows, same `event`, different `object`, and the
second must be **served** once the fix lands.

## Finding 3 — the real-GA106 differential (step 4), at this boot point

Against `traces/real_ga106/rpc_transcript_real_ga106.txt` (88 RPCs; bind `0xa06f0104` at
line 63; 25 entries after it). Under the probe, the census now serves through the **bind**
(`0xa06f0104`, the newly-served row vs the stock boot) and reaches the second arming above.

Of the coordinator's 15 distinct post-bind commands:

- **Served in our probe census**: `0xa06f0103`, `0xa06f0104`, `0x20800301`, `0x90f10106`,
  `0x20802a08`, `0x20800a6c`.
- **In our unserviced list** (refused by name, guest logs quietly): `0x20800a70`,
  `0x20800a38`, `0x20800afe`.
- **Never seen at all** (absent from served AND unserviced): the `0x0073…` display family
  (`0x00730107`, `0x0073028b`, `0x00730211`, `0x00730151`), `0x402c0101`, `0x2080012b`,
  `0x2080013f`. ⊘ These are **downstream of the current wall**, not a differential defect:
  our boot dies inside the second scrubber channel (`:1027`), which is *before* the guest
  reaches display/system-interface init. They will only appear once Finding 2's fix lets
  channel scheduling complete. This matches the coordinator's caveat that the 88-entry
  transcript runs past adapter-init while our wall is upstream of it.

⚠ `0x20802a08` (`CE_GET_FAULT_METHOD_BUFFER_SIZE`) is served in our census (result 0). It is
one of the C oracle's contradicted empty rows; real hardware answers **20480**
(`fmb_real_ga106.txt`). This boot did not reach the CE fault-buffer allocation, so whether
our served reply carries 20480 or 0 was not exercised here — a check for a later boot, not a
conclusion from this one.

## What remains the wall (beyond the RPC list)

The RPC census cannot see the two expensive items the dispatch named: the scrubber's actual
**CE copy** (execution, not a control) and **event delivery** for notifier 35 (an interrupt,
not a control). Finding 2 is a control-plane bug in front of both; clearing it is necessary
but not sufficient — the CE execution join is still owed.
