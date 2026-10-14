# Windows1570: synchronized CONTROL NOP refused during startup

STATUS: RESEARCH, 2026-10-12. ABORTED_STARTUP; no app/migration verdict.

MEASURED exact a38602015d0c295fdbb9a03e2e28fbabf78174ae, production setup
plus only default-OFF KF3_WIN_USER_RING_RELAY=1. Private USER births/native5080
children succeed. Multiple desktop workers refuse the same public descriptor:
USER_RING_GP_FORMAT Entry entry=0x8000020000000000 source_va=0x4000078
cursor=0xf put=0x24 entries=8192 (another source cursor0x1e/put0x21).
That is LENGTH0/OPCODE_NOP with SYNC_WAIT and LEVEL1. No GravityMark launch or
app verdict. Controller terminated matrix on the named falsifier; harness143.
Guest did not shut down after ACPI/QGA attempts: UNCLEAN SIGTERM at18:42:27UTC,
QEMU gone18:42:31UTC, IOMMUgroup11DMA-FQ. Host Xid count59 unchanged and bounded
journal has no Xid. Personal images untouched. Retained full gzip QEMU log,
command, markers and controller/session evidence; do not call this a clean stop.

Public NVIDIA open-gpu-doc commit9fdf5c4062007929d9f4e6cbad9c9771fe61b880,
manuals/ampere/ga100/dev_pbdma.ref.txt107-115 defines CONTROL NOP: no PB fetch
or operation, but SYNC respected;172-176 explicitly includes LENGTH0 entries.
The unused low-dword OPERAND is not a PB pointer. Existing decoder accepted
only literal zero; working correction admits NOP with optional SYNC/LEVEL,
rejects other upper bits/opcodes and preserves original descriptor unchanged
through actual native PUT/GP_GET. This creates no CPU completion.

Before native run: hypothesis is that the measured LEVEL1/SYNC_WAIT NOP retains
real HOST ordering and descriptor-fetch behavior. Falsifier: native matched held
prefix probe advances GET>=3/reference before release, fails real copy/reference/
GET=PUT4, or records hostXid. Compare storage-control-barrier-phased with
storage-control-barrier-level-phased; only the barrier LEVEL bit changes.
Then exact Linux9+10/fast30/desktop precedes Windows1580; one acceptance change.
