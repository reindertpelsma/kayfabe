# Native Blackwell trace: remembered-region replay

**STATUS: LIVE, 2026-10-16. H-BC3 written before native capture; hardware PENDING.**

Own instance56165326, created2026-10-15T22:44:19.841586+00:00, quote$0.1288888889/h.
Timer2120908 armed175minutes, deadline01:39:19UTC / hardcap01:44:19UTC2026-10-16.
Same provider machine39241 as retired56141654, new lease/VM; old lease absence
verified before create. The old lease proved the board's unprivileged RM tuple
GB206(0x1b0,0x6); this fresh VM's tuple still needs its own probe before new die
facts are registered. No new product facts or T3rows planned in this wave.

Hypothesis H-BC3: retain an internally validated queue table after both queues
passed ring capacity, keep the capture module loaded, disable/re-enable the
NVIDIA device, and revalidate sequence-zero prefixes before the bulk-scan delay.
Module1d302de82, unsignedSHA62e89c6dd256ec56c4b951c1d40faa485d1e2919756b36bda0449ca1f539f530;
collectorSHAb7732d2edb0845f9d319d0dd2669474ff7cb7bec1adc54c10f261bd97c427acf.
Native driver must actually be32.0.15.8088 before/after, GSPfirmware580.65.05.

First late capture is only a seed: its missing prefix is an expected FAIL and
cannot establish complete reference traffic. Stop that collector, not the module;
disable owned NVIDIA display device; arm a new default scan-enabled capture;
enable the device, run nativeCUDAcopy/kernel/matmul+D3D11/12 oracles, and stop with
an explicit END. A complete capture must have method3, first sequence0 in BOTH
directions, one queue instance, nonempty boot/control/probe traffic, no loss,
no gap/torn/invalid/drop/refusal, oracle0 and balanced controls. Ordered metadata
uses the existing parser; no raw captures/binaries come back from the rental.

Falsifier: moved table, stale binding, nonzero first sequence, missed required
traffic, any loss/corruption or native oracle failure. RAM-scan fallback may yield
a useful partial census but still FAILS complete capture. One bounded replay;
if false, stop and rank next discovery paths, do not guess runtime replies.

No kayfabe GPU process in this native phase: kf3rev/KF3_* process env are N/A,
not a claim about a production guest. If a kayfabe run follows, print/assert its
actual process env, binary revision and both actual driver builds first.
