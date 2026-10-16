# Windows580 binding preparation

**STATUS: LIVE, 2026-10-16 20:02UTC. Bound parent prepared; proper baseline pending.**

MEASURED: first boot96946QGA answers after132seconds. Phase1autologon/service
GSP flag completes; drvbind exceeds180seconds and its outcome is uncertain.
No GSP RPC metadata was emitted. Owned shutdown completes19:49:37.
A fresh boot98033uses a direct cfgmgr32/registry census instead of the blocking
PnP/WMI path. It proves actual32.0.15.8088 installed, problem43, classGSPflag
absent. The pending installation is NOT replayed. Direct registry code sets
only this present8088adapter's EnableGpuFirmware DWORD1 and verifies it; DWM1/
Explorer1. Problem43before the required cold boot is not a production failure.
Owned stop completes19:59:05. The stopped disk/VARS are frozen as bound580,
and fresh siblings give displayon/off the same starting state.

First proper GSP-enabled display-on process98863started19:59:05, same source
f8485f6db/privateSHAe974, P19+2boundedtrace diagnostics. At20:01:24it has385RPC
metadata records ending in explicit UnloadingGuestDriver, reproducing the
known initialization sequence; actual configured census is stillpending.
ClassC67A/CA7A are cursor immediatePIO; C3700201 is GET_LOCKPINS_CAPS.
No guessed reply, native page injection, PCI driver reinstallation or host action
from guest bytes. This is an OS binding/harness correction; no product change.
