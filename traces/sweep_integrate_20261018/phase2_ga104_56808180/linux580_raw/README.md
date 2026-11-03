# GA104 Linux580 raw suite

STATUS: LIVE, 2026-10-20. MEASURED FAIL29/30: CE guest-RAM R34 timed out at the
frozen180s limit. The harness crash counter1 denotes that timeout; no process
crash is established. Sourcee8553e6ce7ec557aef9ff96c3c876e3060a6b9f8, actual
host595.91.07 and guest580.159.04, raw suite00:36:10..00:49:37UTC.
Native gates9+10/raw30/CUDA4 passed first. Full30 serial logs have actual580
module banners and zero heartbeat warnings, guest Xid or flip timeouts.

The CE arm reached R34's real GPU source-in-guest-RAM operation behind13000 rows
and its first doorbell-store marker, then did not reach DONE within180s. The
other29 arms passed. The follow-on controller stopped before either CUDA run.
No oracle, timeout, driver image or fast-guest binary was changed.

Required matched candidate4 control is ACTIVE: same box, host driver, packed
580 guest, current launcher/grader, flags and frozen180s; only QEMU is changed.
Falsifier: candidate4 also times out, rejecting uniqueness to the imported stack.
That would not prove the root cause or eliminate every performance contribution.
No third attempt or R34 optimization is planned for this lease.
