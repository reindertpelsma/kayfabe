# VM provider environment recovery

[CODE READ: Vast VM documentation, vast_report.py load_provider_env] Vast's VM interface
writes custom variables to /etc/environment; non-login startup must load that data.
[Primary documentation](https://github.com/vast-ai/docs/blob/main/linux-virtual-machines.mdx).
The reporter now fills only missing contract names from that bounded UTF-8 data file.
It parses quoted values without executing shell syntax, preserves explicit process values,
ignores unrelated/account/instance keys, and refuses duplicate or malformed missing fields.
The normal strict protocol/binding validator still decides acceptance. Worker code is unchanged.

[MEASURED: 2026-10-23, run v7-provider-env-tests-20261023; provider_env_tests.json] Thirteen
standalone probes failed before this support existed. Seventeen focused probes now pass;
the combined canonical/payload/provider suite passes 168/168 in 52.419 seconds. Black and the
100-column check pass. [MEASURED: run v7-provider-env-mutants-20261023;
provider_env_mutants.json] Eight new mutations each have at least two named killers.
[MEASURED: run v7-provider-env-tests-20261023; canonical_box_mutants.txt] All 60 existing
canonical mutations are killed under PR #104's separate two-mutants-per-behavior rule.

[MEASURED: 2026-10-23, run v7-kvm-20261023; startup_recovery.json] The single VM booted systemd/KVM
and checked out exact source 28084b96cebad2a853d72171f72f8d776946b8ba. The initial unattended
entry left no reporter or payload process and no /run/kf-ci/env. All fourteen contract names
were present in the VM provider file and login environment; the original validator accepts
that login environment. Initial startup output has not been recovered. Loss of the non-login
exported environment is therefore an INFERRED explanation, not a conclusive root-cause claim.
Its falsifier is initial startup output showing a different failure.

[MEASURED: 2026-10-23, run v7-kvm-20261023; startup_recovery.json, controlled recovery at 02:00:03 UTC] The unchanged approved
repo-side entrypoint was restarted via the VM's valid login environment on the SAME instance.
No code was edited on that VM, no payload was injected, and no second rental was created.
The Worker then accepted reports and the original payload entered provisioning. This is a
controlled recovery, not a claim of successful unattended startup. The new loader is locally
verified only; it has not run on the rented source SHA. Final suite/spend/absence evidence
will distinguish these scopes explicitly.
