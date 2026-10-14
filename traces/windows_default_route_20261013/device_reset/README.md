# Device reset hardware results, 2026-10-13

**STATUS: EVIDENCE, 2026-10-13.** Branch under test: `claude/device-reset-20261013` at `c9ac4e77` (ABI 27) and `bf510bdf` (ABI 28, bounded wait + fn 47 twin free). Host: RTX 4070, driver 595.91.07, QEMU 10.2.4, binaries per revision. Nothing here ran with the owner's VM affected (a second kayfabe VM ran beside it).

**Measured, Linux (fast guest, QMP `system_reset` x5 mid-boot and x5 at 9-19.5 s into heavy arms, final boot completes all arms):**
- c9ac4e77: every reset freed the guest clients (2 early, 3 late including a CUDA client), channel plane EMPTY, 4 VA spaces retired, 79-87 BAR2 placements removed, the next boot passed (`FAST_VERDICT=PASS`), host Xid count unchanged (16 -> 16).
- bf510bdf (ABI 28): the main loop waited 1134773, 1190877, 649218 and 221910 us on the four resets that had live work (pristine resets 0 us); all under the 2 s bound.

**Measured, Windows LTSC baseline, run 2970, bf510bdf (record: `windows_run2970_bf510bdf_record.txt`):** 3x hard reset at the signed-in desktop: each a real reboot (new LastBootUpTime), guest agent back, NVIDIA adapter Code 0, kayfabe back to `Running`; 3x Device Manager disable/enable: Code 0 each, `fn 47 UNLOADING_GUEST_DRIVER bInPMTransition=false` each time; clean `Restart-Computer`: new boot, Code 0, `Running`, device reset logged at the platform reset (verified by hand: the driver's own W3 check hung because it still held the single guest-agent connection; fixed in the next driver).
First two Windows attempts were flawed harness runs (the driver held the guest-agent socket open and reset during boot); they are not evidence.

**Not measured:** reset while a GPU app is running on Windows; bugcheck reboot; Linux `reboot`/`reboot -f`/sysrq, rmmod cycles with `kill -9` (D4 on hardware), reset races (design note §8 L2-L6); the fast suite 30/30 and the gates on the reset build; whether Windows sends fn 47 without a PM transition during TDR recovery (the disable/enable path does).

## Validation of `bf510bdf` (candidate branch `integration/prod-candidate-2-20261013`), 2026-10-13
Measured: `v3_gates.sh` 9/9 and gate 10 PASS (`validation/gates_bf510bdf.log`); fast suite 30/30, 0 fail, 0 crash (`validation/fast_suite_bf510bdf_30of30.txt`). The first suite attempt deadlocked because the wrapper held the GPU lock that each arm takes itself (my script error, not a product result); an orphaned lock holder from that attempt was killed. Windows matrix, independent review and the exact-commit real-GPU pass required for master are still open.
Host scripts in `host_scripts_2/` are the final versions used for these runs (the Windows driver there still carries the single-guest-agent-connection bug in its W3 step, which was verified by hand).
