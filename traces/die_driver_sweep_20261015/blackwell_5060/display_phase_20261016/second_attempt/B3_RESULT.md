# GB206595 fixed-clock concurrency revalidation

**STATUS: LIVE, 2026-10-16 19:40UTC. Concurrency PASS; complete raw suite RUNNING.**

MEASURED own56286918/GB206/PCI2d04, stock host595.91.07 and exact guest595.91.07.
Private sourcef8485f6dbe2cdec62c3b1fa69133b5f266801dcc, QEMU SHA256
 e9744a23ab38ff45afbf788d9e426d71913d1f1e6954fa1ea0da1ea1920b25a6.
Actual KF3 environment{}, normal loglevel6, frozen180second budget.
Concurrency passes31seconds total/client14180ms; complete serial heartbeat
formula warning count0. Native gates9/9+gate10 and native CUDA4 pass first.
Earlier wrong-clock GB206 run timed out with15954warnings. Different rental
and source means revalidation, not a same-box controlled speedup. The earlier
same-binary normal/quiet ablation was on GB205, documented separately. No
logging suppression or grader change.

H-B3-GB206 falsifier: still exceeds180seconds or sustained nonzero-heartbeat
formula mismatch. MEASURED falsifier did not fire in this arm. Full30arm suite
is still running; startup warnings will be counted independently. No general
warning-free/startup-readiness fix claim. Windows on/off and tiling UNRUN.

Concurrency QEMU log is1.8MiB; only a screened excerpt and size/SHA receipt
return. Complete serial and native CUDA scoreboard are retained. An initial
collector correctly refused the oversized log before saving the concurrency
proof; the preceding7ca161749 commit saved only native caps equality despite
its broader subject. This follow-up supplies the actual concurrency evidence.
