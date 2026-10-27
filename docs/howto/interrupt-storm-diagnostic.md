# Re-notify completion interrupts for diagnosis

The interrupt storm diagnostic is disabled by default and is never required to boot. Configure the VMM environment before launch:

```sh
export KF3_DIAG_IRQ_STORM_MS=10
export KF3_DIAG_IRQ_STORM_MODE=all
```

The interval is an integer from 1 to 1000 milliseconds. Unset or 0 disables the diagnostic. Invalid intervals/modes disable it with a startup message. `all` is the default mode when an interval is enabled. `owed` is unavailable and is rejected; it never silently selects `all`.

The diagnostic re-notifies unmasked completion vectors using their existing shared MSI-X0 message. It does not set pending/status bits or write completion data. After the guest clears a pending bit, the message may therefore be spurious and may not run a completion DPC. This is a notification experiment, not a replacement completion path.

Read the regular heartbeat fields:

- `storm_raises`: successful diagnostic eventfd signal calls, not proof of guest ISR execution.
- `storm_skipped_masked`: distinct completion vectors skipped for a leaf/top mask at scheduled checks. Aliases count once per check.

One shared message covers all eligible logical vectors. Extra diagnostic messages are capped at 1000/s across the device; real completion interrupts keep their normal behavior. The timer can run late and does not catch up in a burst. Existing completion-probe output keeps its own cadence; enabling the storm alone does not enable channel probing or change worker polling.

For an A/B, keep the binary, guest state and other options identical, and record guest ISR/DPC activity plus pending state. A nonzero raise count alone does not show that the guest accepted a message. Stop the diagnostic by restarting with the interval unset or 0. Keep this optional diagnostic off for ordinary use.

The source boundary and interpretation are in [the diagnostic design](../design/V3_IRQ_STORM_DIAGNOSTIC.md).
