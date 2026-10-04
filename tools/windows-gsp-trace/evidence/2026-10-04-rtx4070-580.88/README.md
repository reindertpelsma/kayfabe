# First validated Windows GSP observation

The [complete command audit](command-audit/README.md) compares every retained
RPC/control/allocation ID with the actual `v3-windows` implementation, including
deferred wrapper contents and known request-shape mismatches. It separates
command names, specific handlers, authored host facts and generic allowlist rules.

This is a complete text export of a **partial passive capture**, taken on
2026-10-04 from Windows 11 build 26100 with an RTX 4070 assigned directly by
VFIO. NVIDIA driver 580.88 was running with GSP firmware **580.65.05** after
a controlled guest reboot. This is one GPU/driver configuration; it does not
establish behavior across dies or drivers.

The system-start observer attached to one queue table. Its buffered observations
were drained by a SYSTEM startup collector. All 4,535 exported messages pass
the decoder's wire framing, metadata and XOR-checksum validation; the complete
export reconstructs 26,509,784 source bytes with SHA256
`94745becce0c30227e3e7bb6134003a22a4963c05ecb6a8c04e561977a550f40`.
Only text data returned from the Windows guest. `gsp.jsonl.gz` is deterministic
gzip compression of that text, not an executable or a physical-memory dump.

There are 2,265 requests and 2,270 replies, including 1,999 RM_CONTROL messages
in each direction. No `GR_GFX_POOL_QUERY_SIZE` (`0x2080121f`) request or reply
was observed. **The earliest retained request/reply queue sequences are 2707
and 2710**, both marked with an unknown prefix. Therefore this recording cannot
show whether the query occurred earlier during initialization. One later reply
sequence is missing. The observer reports zero FIFO drops, zero read failures,
343 unstable snapshots and an empty FIFO after drain; those counters do not
recover the unknown prefix or prove complete coverage.

Every observed request has RPC status `0xffffffff` (pending), and every observed
reply has RPC status zero. Some individual control replies report `0x56` or
`0x57`; these are preserved in `histogram.json`, rather than reclassified as
successful controls. No target-query sizing values have been derived.

The earlier installation capture had no queue because GSP was not yet active:
the display-class firmware policy was written after NVIDIA setup had already
started the device. A controlled reboot consumed the existing policy and
`nvidia-smi` then reported the firmware version above. `firmware.json` records
the live PCI identity, PnP state, driver hash and firmware report.

Reproduce strict decoding from the repository root:

```sh
gzip -dc tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-580.88/gsp.jsonl.gz > /tmp/rtx4070-gsp.jsonl
python3 tools/windows-gsp-trace/decode.py /tmp/rtx4070-gsp.jsonl --require-query-pair
```

The expected exit code is **4**, with 4,535 validated records and no successful
query pair. `provenance.json` identifies the unsigned trusted MSVC driver,
collector, exporter, decoder and compressed/uncompressed text hashes.
`decoded.json`, `histogram.json`, and the initial/final stats retain the
analysis. The final collector exit was zero; the observer was then stopped,
restored to demand-start and its one-shot startup task disabled. No NVIDIA
queue pointers, memory or firmware were modified by the observer.
