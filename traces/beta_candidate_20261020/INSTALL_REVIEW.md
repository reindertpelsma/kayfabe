# Install path review

STATUS: GPU-FREE FUNCTIONAL CHECKS PASSED / HARDWARE WALK-THROUGH PENDING, 2026-10-20.

INFERRED falsifier registered before mutation: removing VRAM/BAR headroom,
changing the selected OS, accepting mixed component versions, launching/writing
state during --print, allowing uninstall traversal or replacing a published
revision must each fail at least two named tests. Mocked process/state hooks
keep dry-run mutants GPU-free; traversal/publication cases use only temporary
files created by the test. Syntax/import/harness failures do not count as kills.

CODE READ: launcher derivation follows V3_SWEEP_AND_INSTALL section 2.5 and
cardbudget, including the GOP overlap. Existing explicit device properties stay
unchanged; Linux QOM defaults remain the old values. Launcher-generated values
handle common small BARs without silently changing direct Linux device defaults.
Guest-driver remains explicit because the documented pre-fn1 ABI boundary
cannot be inferred safely from host version alone. The Windows property is
independent of that layout choice.

CODE READ: doctor reads KVM/device permissions, GPU query and sysfs BAR size,
version output, graphical-session variables, groups, limits, cgroup and disk.
It never installs drivers, creates a CUDA context, changes permissions or boots.
Commands it labels as fixes are printed only. C6 tests use offline fixtures.
No actual doctor/GPU query is run by C6 on the occupied host.

CODE READ: source-only QEMU defaults for display/GOP and memory apertures are
retained to meet the binding Linux bit-identical requirement. The launcher is
the host-derived minimal user interface; changing direct QOM unset defaults
would be a separate behavior change. No hardware-minimal-working claim is made
before Stage 5. Prefix installation is an unsigned local staging path, not a
published relocatable binary release with a certified glibc/library floor.


MEASURED: 20 launcher/doctor/prefix tests, 4 version/publication tests pass.
All seven installer mutants are killed by at least two named tests (3/2/2/2/2/2/2).
An initial BAR mutant exposed a headless test gap; the resource invariant now
covers both display and headless modes and the entire campaign was rerun.
The docs checker validates 5 local links, 7 shell blocks and 3 repository command
paths without executing a VM or install. Four thin shell wrappers and the build
script pass shellcheck. Full changed-tree Rust and broker checks are Stage 4.

MEASURED correction: the initial udmabuf preflight was too strict. CODE READ:
BrokerSeat explicitly falls back to shared memory when its open fails. Tests-only
0d0249092 reproduces two OS failures against the over-strict rule. Missing
udmabuf now WARNs without refusing that tier; headless needs no udmabuf.
Two mutants (hide warning / refuse valid fallback) each kill two named tests.
No occupied-host doctor/GPU query was performed.

MEASURED real CPU layout check found dangling generated firmware/trace links
when only the QEMU executable target was built. Tests-only `744eb8afe` reproduced
both missing outputs with a synthetic real-Ninja fixture. The builder now
resolves missing bundle data via declared Ninja targets before publication;
unbuildable or external missing targets refuse publication. Two tests pass;
removing the builder call kills both. The corrected real layout smoke passed at480a88d66 and896cfea8c:
174 manifest hashes, matching versions, private prefix installed/uninstalled.
No guest or GPU operation is part of that check.

MEASURED final script/docs revision7211e357a: full42-test Python suite and all
installer/doctor/bundle mutants pass; final CPU build and174-file private-prefix
smoke pass. Missing udmabuf WARNs and preserves the existing shared-memory tier.
The minimal *working* guest invocation and unfamiliar-person install remain
Stage5. Direct Linux defaults and explicit guest ABI are retained for correctness.
