# GPU identity health check

STATUS: LIVE, 2026-10-19. Tests-only EXPECTED RED042132e91 reproduces9runtime
assertion failures. Focused tests3before->13after; fullbench53before->63after,
0failed. Existing family admission assertions retained. Four distinct mutants
have2/2/2/3runtime killers; compilation errors not counted. bash-n passes.
No Rust source, driver protocol, QEMU device, grader or timer is changed.

MEASURED (2026-10-19) own56788212 initial proprietary575.51.03 cannot initialize PCI2b85:
NVIDIA says open modules required. Original host_preflight erroneously emits
SOUND because it accepts stdout error text while head hides query status. This
is retained as FALSE GREEN, never host health evidence. Approved standard stock
provisioning installs open595.91.07 before the strict health check and native
gates. An owned stage barrier holds the orchestration parent while stock install
finishes, so the stricter health check can run before native gates. If that health
check fails, retire the box; no repair or repeated failure loop.

Fix preserves the original query status, rejects absent/error GPU identity and
requires a numeric driver version, then retains first-GPU handling. No family or
marketing-name allowlist is introduced. A valid identity still does not prove
native workload health: gates and stock workloads remain required separately.
Product binary source stays24906bf33; strict preflight is a separately hashed
host-only acceptance tool from the new commit. No untested product tree is used.
