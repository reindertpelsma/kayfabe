# Install path review

STATUS: GPU-FREE VERIFICATION IN PROGRESS, 2026-10-20.

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

MEASURED: preflight omission reproduced in tests-only `762e54a87`: both OS
display checks failed before udmabuf validation. Display now requires read/write
access; headless does not. Removing this check kills two named tests. Doctor
remains fixture-tested only; no occupied-host GPU query was performed.

MEASURED real CPU layout check found dangling generated firmware/trace links
when only the QEMU executable target was built. Tests-only `744eb8afe` reproduced
both missing outputs with a synthetic real-Ninja fixture. The builder now
resolves missing bundle data via declared Ninja targets before publication;
unbuildable or external missing targets refuse publication. Two tests pass;
removing the builder call kills both. The real layout check will be rerun on
the corrected revision; no guest or GPU operation is part of that check.
