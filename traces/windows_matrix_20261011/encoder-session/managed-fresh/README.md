# Fresh Windows managed-memory failure

**STATUS: RESEARCH, 2026-10-11.** Run1261, product `a61af1c3`, RTX4070,
host595.91.07, guest580.88; fresh isolated guest, canonical180s app budget.
Hypothesis: the old managed-memory failure persists after packaging corrections.
Falsifier: allocation and both visibility tests finish in the original budget.

Measured: TIMEOUT181s. The last marker is `MANAGED_STAGE allocating`; allocation
never returns. Zero guest reset/WER/GSP recovery records. Host Xid32 at16:15:17UTC,
QEMU82479, channel0x01000053, intr1=4, HCE_DBG0=0x300, HCE_DBG1=0x3185.
The matching stock Windows580.88 case finishes all stages and visibility checks.

The faulting host channel0x10053 is the passthrough twin of guest
`0xc1d000b2:0xcaf0000c`, token0x82c, engine0xb/COPY2. Birth explicitly records
`PRIVILEGE=KERNEL+UVM_OWNED`, but the process rule overrides it to user work.
All eight UVM channels in this separate client/VA space receive that route.
The owned engine object is public CE class0xc7b5.

Public clc7b5 defines method0x300 as LAUNCH_DMA. Its data0x3185 has both physical
operand bits set (SRC_TYPE12, DST_TYPE13). Public GA100 PBDMA documentation names
intr1 bit2 HCE_PRIV: a physical copy on a nonprivileged host channel is rejected.
Applying GA100's interrupt-name definition to AD104 is an inference; the raw Ada
fault, birth flags and public CE data fields are measured/source-backed separately.
The rejection is expected fail-closed host behavior, not a reason to request host
privilege. Candidate next step: preserve the authored Translated route for
kernel-stamped UVM-owned work at birth, with all host births still USER.
Unknown/USER privilege must not gain the privileged T-space from UVM_OWNED alone.

Original timeout and host fault remain failures. The guest's shutdown also stalls;
the broker has its existing bounded disposable-overlay teardown. No product timeout
or invented GPU completion is proposed. Logs/verdict/health are saved here;
`uvm-routing.txt` includes a source-log hash and bounded relevant records.
