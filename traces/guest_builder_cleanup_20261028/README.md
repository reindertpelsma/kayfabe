# Guest-builder NBD cleanup

STATUS: RESEARCH, 2026-10-28. CPU fixtures only; no host, GPU, VM, mount or NBD operation.

MEASURED: fixture run `z1-builder-cleanup-20261028` starts from `26a7d3441de55e0c9e3bcc5c833a0beb1cd2646f`. EXPECTED RED commit `2ea2fb4dc` runs fifteen tests with nine failures: the builder disconnects without owning an attachment or disconnects after it has already released one. EXPECTED RED commit `5fe05181d` adds two signal-during-release cases; both fail against that commit's builder. The fixed builder passes all seventeen fixtures.

The tests execute the actual shell builder with inert executable replacements for every device, mount, module and process-probe command it can invoke. Successful extraction uses temporary kernel/module text fixtures. Those files are never booted or loaded. The tests cover host mode, skipped NBD, early failure, failed attach, failed mount, normal release, packing failure, failed-release retry, and INT/TERM during attach and release.

CODE READ: cleanup ownership starts only after successful attach and ends only after successful disconnect. INT/TERM are deferred across each external command's ownership transition so a pending signal cannot leak a newly acquired attachment or disconnect a released one again. Other builder flags and the existing staging path remain unchanged.

MEASURED 2026-10-28: CPU fixture run `z1-builder-cleanup-20261028` killed seven behavior mutants with two named tests each; evidence `traces/guest_builder_cleanup_20261028/mutations.json` ([record](mutations.json)). Reproduce without devices:

```bash
python3 -m unittest discover -s scripts/fastguest/tests -v
python3 scripts/fastguest/tests/mutate_build_cleanup.py
bash -n scripts/fastguest/build_fast_guest.sh
```

No hardware qualification follows from these fixtures. This change does not address the Z1 process-inspection refusal, admit its Xids or alter the parked rootless/two-VM results in PR187/188.
