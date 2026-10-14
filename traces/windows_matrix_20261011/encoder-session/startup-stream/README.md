# Cold Python/CuPy startup diagnostic

**STATUS: RESEARCH, 2026-10-11.** Old producta61af1c3, fresh guests1262/1263.
Diagnostic180s deadline, then original canonical stream120s; no timeout increased.

Measured in run1262: Torch import82.542s, CUDA initialization83.287s, actual
Torch GPU readback84.590s, CuPy import178.082s, first CuPy arange178.581s.
The diagnostic TIMES OUT181s before complete CuPy readback. This does not prove
that CuPy GPU work hangs: only2.4s remained after arange started. The planned
warming falsifier is inconclusive because the preload did not finish.
The harness correctly starts a new guest for the remaining unchanged canonical
stream test. It TIMES OUT121s again. Both have zero guest driver/reset/WER/GSP
recovery records; bounded host journal has no additional Xid beyond the earlier
separate managed failure. Original cold failures remain failures.

A separate benchmark correction replaces unnecessary Torch/CuPy interoperability
with a direct CuPy per-thread CUDA stream and full NumPy CPU-reference readback.
It retains the same120s budget, real NVIDIA CUDA device and GPU arithmetic.
That corrected case is unmeasured here; no pass claim follows from import timings.
