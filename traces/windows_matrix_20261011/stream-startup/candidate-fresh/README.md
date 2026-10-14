# Fresh direct per-thread CUDA stream test, run1350

**STATUS: LIVE, 2026-10-11.** Product72741e2b, harness738b5385.

PASS94s, original deadline, actual direct CuPy per-thread stream multiplication
and full independent NumPy numerical readback. No guest153/4101, WER, GSPcycle
or reboot. Different-GPU stock580.88 reference also passes. This does not prove
unrelated CUDA or Vulkan workloads.

The outer batch kernel capture starts before this VM and includes the prior
native deferred probe's Xid32 at18:56:51.221583. The guest starts18:57:30;
corrected-journal/ restricts the interval to this session through its own stop
and reports0Xid. Raw original capture retained. host_xid_delta=-1 is a wrapped
ring-count difference, not a meaningful negative fault count.
