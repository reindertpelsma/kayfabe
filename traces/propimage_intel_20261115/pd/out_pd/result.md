| step | verdict | evidence | class | detail |
|---|---|---|---|---|
| prerequisite | PASS | NONE | harness | swtpm >= 0.10 (libtpms state compatibility still required) |
| unpack | PASS | NONE | harness | SHA256 of image and state; derived seed verified |
| overlay | PASS | NONE | harness | throwaway clone of image.base.qcow2; base not booted |
| clock | PASS | NONE | harness | RTC base 2026-11-12T17:49:48Z (fresh overlay) |
| boot | PASS | NONE | harness | ci mode, restrict=on, disposable overlay |
| clock-save | WARN | NONE | harness | Guest clock not saved: Guest UTC unavailable before shutdown |
| stop | PASS | NONE | harness | own QEMU stopped; other own helpers cleaned by ExitStack |
| harness | HARNESS | NONE | harness | Guest SSH unavailable |
