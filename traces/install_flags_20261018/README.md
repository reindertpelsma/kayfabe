# D1 installation/default audit

**STATUS: RESEARCH, 2026-10-22.** Base: `e64b8e5f6d0f64c41c9c4d40775e74be08de7164`.
CODE READ: [design and pre-change falsifiers](../../docs/design/V3_FLAG_INVENTORY_20261022.md).
MEASURED: production launcher files retrieved read-only; hashes and a sanitized effective configuration are in the inventory. No VM or GPU work is authorized in D1.
Verification results will be recorded here as they complete. No hardware result is implied by the GPU-free checks.

## Launcher proof

MEASURED: test-only EXPECTED RED commit `5d0f53ad4` adds 8 GPU-free launcher tests:
`python3 scripts/bench/hitch/test_launch_h.py` (3 tests, 2 expected failures) and
`python3 scripts/bench/windows/test_launcher_defaults.py` (5 tests, 3 expected failures).
After removing only the default diagnostic/dead-name assignments: 8/8 pass; `bash -n` passes for both changed launchers.

MEASURED: restore each removed executable assignment separately, run the relevant suite,
require >=2 failed tests, restore the fixed file in `finally`. Before each Windows mutant's tests,
the shell-exported flag dictionary was checked to contain that mutant's `=1` value.

| Mutant | Failing tests | Result |
|---|---:|---|
| Restore `ENV=(KF3_RPC_TRACE=1)` | 2 | KILLED |
| Restore default `KF3_BAR0_TRACE` | 2 | KILLED |
| Restore default `KF3_MAPLOG` | 2 | KILLED |
| Restore default `KF3_DISPLAY_METHOD_TRACE` | 2 | KILLED |
| Restore dead `KF3_TIMER_MAP` | 2 | KILLED |
| Restore hitch's dead lifecycle extras | 2 | KILLED |

MEASURED instrumentation error: the first mutator matched the `WIN_FLAGS=` example in a comment,
so it had not restored an executable BAR0 default and the tests stayed green. That run stopped and
restored the file; it is not counted as a killed mutant. CODE READ: anchoring injection to the actual
`^FLAGS=` assignment and asserting the shell-exported value produced the six real mutants above.
No tests or failure thresholds were weakened.

## GPU-free baseline and broker

MEASURED: before default flips, `cargo test --release -j2 -p kf-qemu -p kf-chan -p kf-mem
-p kf-broker -p kf-disp -p kf-rm -p kf-host` completed: **2438 passed, 0 failed, 19 ignored**,
114 test-result groups (including doc tests). No flags or timing constants were changed for this run.
The first clippy attempt's 60-second shared Cargo lock wait expired while that baseline held the lock;
clippy did not start. It was attempted again only after the baseline process exited and released the lock.

MEASURED: `nice -n 19 make -C broker -j2 check` completed successfully with the compiled Wayland/X11
capabilities reported by make. Its built-in checks include 80,752 XOR and 566,237 tick assertions;
**29 XOR and 28 tick mutants** were each killed by >=2 distinct checks. This is local protocol/unit
coverage, not compositor import/presentation or hardware validation.
