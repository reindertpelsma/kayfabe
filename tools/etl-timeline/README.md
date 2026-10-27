# etl-timeline

STATUS: LIVE, 2026-10-27. Read-only, stdlib-only reader for a guest DxgKrnl ETW trace (`dxg.etl`,
provider `{802EC45A-1E99-4B83-9920-87C98277BA9D}`) captured from the Windows guest. It answers one
question: **what was the Windows graphics scheduler waiting for when the engine timeout fired**.

It ships no manifest, no trace and no captured data. The event layouts are read at run time from the
`WEVT_TEMPLATE` resource of a `dxgkrnl.sys` you supply (the same manifest `tracerpt` and `wevtutil` use).
Treat every input file as untrusted data: nothing in it is executed or imported, and the script is run
with `python3 -I`.

```
python3 -I etl_timeline.py extract-wevt  dxgkrnl.sys  dxgkrnl.wevt
python3 -I etl_timeline.py summary       dxg.etl      dxgkrnl.wevt  [--top 25]
python3 -I etl_timeline.py stall         dxg.etl      dxgkrnl.wevt  [--ens3 ens3-dump-lines.txt]
```

* `summary`: span, buffer count, how long after the last event the session was stopped (last buffer
  flush), event-id histogram with field names.
* `stall`: finds `TdrPayloadEngineTimeout` (event 540: node, engine, last completed / last submitted
  fence), lists every DMA packet that was submitted (`DmaSubmit`) and not completed at that instant with
  its age and the time since its context last completed, then the TDR capture-dump events and the
  driver's `ExternalEvent` string (the text behind nvlddmkm event 153), which also anchors the trace
  clock to the guest wall clock.
* `--ens3`: with a kayfabe ENS3 dump, matches the guest's inter-completion intervals to the host sparse
  apply walks (`ENS3 APPLY ... wall_ns`) and reports how many are equal to < 3 ms and phase-locked.

## Clock anchoring

The trace clock is QPC at 10 MHz (`PerfFreq` in the header). Two independent anchors give the guest wall
clock: the last buffer flush equals the header `EndTime`, and the `ExternalEvent` "Resetting TDR occurred"
string is the same moment as the System-log nvlddmkm 153 record. On boot 7321 they agree to 2 ms.

## Limits

* The trace is a circular buffer: only the last ~6 minutes exist, and the per-node `NodeMetadata`
  (engine type) event has usually aged out.
* ETW shows what dxgkrnl/dxgmms2 did and was told. It cannot show what the GPU did, whether an interrupt
  was raised and not delivered, or what the miniport driver did inside the reset.
* Event labels are descriptor names from Microsoft's public symbols; field decoding is exact, labels
  are a convenience. Packet-type numbers are printed raw.
* Hardened captures 7370/7371/7380/7381 did **not** arm ETW (`Data Collector Set was not found`), so only
  boot 7321 has a trace.
