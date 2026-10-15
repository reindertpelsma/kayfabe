# Windows run status propagation

**STATUS: ANSWERED, 2026-10-15.** Harness fix, no new Windows GPU acceptance.

Hypothesis before the check: die_run.sh can report rc0 after its QGA PowerShell
call fails, because it uses pipefail without errexit. Falsifier: the simulated
failed evidence/binding calls already return nonzero. Before: three regressions
fail, each measured rc0 after an injected rc77; positive runs succeed.

Enable errexit so evidence and setup pipeline failures abort the phase and reach
the existing exit marker. After: all16 harness tests pass; shell syntax passes.
These tests establish command-status propagation only, not the contents of a
successful evidence response or Windows Code0/DWM/D3D/TDR acceptance.

GA104 phase2's QGA timeouts remain a failed/unverified Windows cell, regardless
of its historical launcher rc0. This correction does not reinterpret the boot.
