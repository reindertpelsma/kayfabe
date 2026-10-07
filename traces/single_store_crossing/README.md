# The device-view crossing — decision (b), exercised end to end

**Measured 2026-09-14**, vast instance `51057666` (RTX 3060 / GA106, driver **580.159.04**),
binary **`kayfabe-rev:814c02c1e092ab7098e1978ef0a10470ed650e6a`** = the tree revision.

## The result

```
SCRATCHPAD-DEVICE-VIEW AT REALIZE: DEVICE_VIEW=OK mmap_len=0x1000
  sentinel_roundtrip=true released=true ⇒ the scratchpad isolate armed a view of the
  RESERVED OBJECT, the node crossed by SCM_RIGHTS, this process mapped it, CLOSED the
  descriptor, and the mapping survived — the owner's conditional ruling, exercised end to end
```

Same line at END OF RUN. Alongside it, unchanged:

```
SCRATCHPAD AT REALIZE: arm=on up=true proc=4294967295 gpu=0 pool=4
  reservation=HELD RESERVED_MB=4096 …
[client] W392D_GUEST_OUTCOME=(P)   THREADS 8 of 8 verified ✔   MEAN_FALSIFIER=PASS
```

## ★★★ The ruling's three conditions, and how each is discharged

`bar1_passthrough_device_local_host_visible.md` §4 item 1 grants decision (b)
**conditionally**. The conditions are the safety argument, not background:

| condition | where it lives | how it is discharged |
|---|---|---|
| 1. the VMM issues **no escape** — only `mmap` | `probe_device_view`'s body | by construction: the only thing done with the fd is `place_device_view`, and the binding it came from exposes no escape |
| 2. closed **the moment `mmap` returns** | the line after the mapping | `drop(fd)` is immediate — and the **sentinel round-trip proves it**: a mapping whose VMA had not survived the close would fault instead of answering |
| 3. the crossing is **`SCM_RIGHTS`** | `call_for_device_view` | one `sendmsg` carrying the body and the descriptor |

★ Condition 2 is the one usually left as an assertion. Here it is a **measurement**:
`sentinel_roundtrip=true` is a write and a read-back through a mapping whose descriptor is
already gone.

⊘ `released=true` — the view's BAR1 aperture was given back through
`NV_ESC_RM_UNMAP_MEMORY`. `[measured w722]` `munmap` + `close` returns **nothing**, silently.

## ⚠ What this is NOT

1. **Not the backing switch.** `page_backing` still returns memfd leaves; BAR1/BAR2 are still
   served from the aperture store. This proves the *crossing* the switch needs, which is what
   was blocked, and nothing downstream of it.
2. **No guest memslot is installed.** Placing one inside BAR1 without the mirror that decides
   which guest page maps where would be inventing a layout, and a failure could not be told
   apart from "we put it in the wrong place". The mapping uses
   `GuestWindow::place_device_view` — the exact verb `install_device_window` calls — so what is
   proven is the whole chain up to the memslot call, and that call is already exercised in
   production by the BAR0 counter page.
3. **One page.** The probe's job is the crossing, not capacity.

## The BAR1 sizing relation, still refusing

```
BAR1-BUDGET host_bar1=256 MiB at 0000:00:07.0 advertised_guest_bar1=256 MiB headroom=16 MiB
  ⇒ ⊘⊘ DOES NOT FIT
```

§w727's knob (`Bar1Choice`) exists and names **128 MiB** as the fix; wiring it to the chip row
is part of the switch, not of the crossing.

## Files

- `boot_swdv.log` — the harness output.
- `census_lines.txt` — the device-view, BAR1-budget and reservation lines.
- `run_swdv_dmesg.log` — the guest driver's own ring buffer.
