# Z1 Linux rootless — preregistration

Recorded before hardware execution, 2026-10-27T23:23Z. Product source is current `next`
`858ca1e8ad4a2b264656b8e499d820856bcc7310`; operator-only commits follow it.
Reuse the 2026-10-16 audit's clean-environment setpriv path, private inner lock,
readable copied assets and per-QEMU capability sampler. No stub cargo.

H2: a throwaway non-root user with only the kvm supplementary group, no sudo,
CapEff=0 and CapPrm=0 runs all 30 Linux fast-suite arms and native gates9+10.
FALSIFIER: an arm requires a root-only operation missed by the audit, wrong output,
hang at the original deadline, or non-approved host Xid. No VFIO rebind, host module,
driver, limits, sysctl or device-permission change is permitted.

Run exact-source native gates and bare suite first. Stage readable guest/kernel and
binary assets as a one-time administrator setup; record this separately from runtime.
Capture id, /proc/PID/status including CapEff/CapPrm/CapBnd, limits, permissions and
opened device nodes of each QEMU. Bound the capability set to zero with setpriv.
Observe host Xids from the root coordinator, while QEMU itself stays unprivileged.
Deliberate full-suite faults require the existing exact source-matched proof; no broad
Xid whitelist. Stop on real resource/identity/safety refusals. Remove only this task's
user/processes/files after capture, preserving evidence, GDM and the owner's assets.

The result covers Linux thin guests on the recorded host/driver/revision only.
No desktop, hardened node policy, building without root, Windows, multi-tenant
security or fault-isolation claim follows. Docs/howto text is proposed in the draft
PR body for integration by the docs agent; those pages are not changed here.

Capture correction before any hardware workload: the accumulated boot journal is
32.7 MB, beyond the existing 16 MiB reader bound. Z1 therefore pins the first of
two readable real kernel records and the boot ID before running. Every scoped
read includes that exact cursor and all later kernel entries, using the same
16 MiB byte bound; cursor loss, vacuum, boot change or malformed/non-kernel data
refuses. The existing source/PID/channel/VA/type/CE matching oracle is unchanged.
The unsupported nvidia-smi inventory field stopped setup before builds or GPU use;
that receipt is retained, and BAR1 is read using nvidia-smi -q -d MEMORY instead.
