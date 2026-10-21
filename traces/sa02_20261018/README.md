# SA-02 kernel-channel authorization

STATUS: RESEARCH, 2026-10-22. GPU-free fix of the security audit's client-handle
privilege bypass, based on origin/master e64b8e5f6.

INFERRED hypothesis, stated before inspecting the channel trace records: internal
kernel channels already carry CPU-RM's privilege stamp. FALSIFIER: a required
internal channel in the committed CHAN-FACTS records has a missing or non-KERNEL
stamp. This is a source/fixture check, not a new boot or exploit experiment.

MEASURED: a search of committed text/log CHAN-FACTS records whose client starts
with c1e found 281 records; every parsed privilege is level 2, uvm_owned false.
These are repeated historical records, not 281 independent boots. Examples:
`traces/windows_code43_walls_20261007/run65-chan-facts.txt`, and the Linux
`fast_codexMemoryAccess_dictated-ring_qemu.log` under
`traces/windows_matrix_20261011/gravity-vulkan/user-worker/authored-default/linux/bench/`.

CODE READ: `ChannelPrivilege` in kf-abi/notifier.rs documents the open driver's
clearing of caller internalFlags and recomputation from the call's security
context. USER and guest-root ADMIN calls cannot obtain its KERNEL level through
a chosen handle. kernel_channel now requires that stamp; a missing stamp refuses
kernel authorization. is_rm_internal_client is restricted to the documented 1 MiB
handle range and is not used to authorize kernel_channel.

MEASURED: tests-first commits d66fa3933 and 714631b96 contain tests only. Before
the fix, the six focused tests reported 1 passed / 5 failed. After the fix, the
whole kf-rm suite reported 756 passed / 0 failed / 0 ignored across 59 summaries,
including doc tests. The final six-test run after restoring mutants is queued under the shared build lock; its result is pending.

| Hand mutation | MEASURED killed tests |
|---|---:|
| Restore handle OR privilege authorization | 3 |
| Restore the broad internal-client mask | 2 |

MEASURED: Clippy --all-targets on the unchanged product base plus the new tests,
and on the fix, reported the same 57 warning fingerprints: added 0, removed 0.
The comparison uses scripts/ci/debt.py's source-bound diagnostic identity. No
lint baseline or deny set changed.

CODE READ: the old unit expectation granting privilege to internal-handle + None
was corrected to refuse None and retain a positive stamped-scrubber assertion.
No passing security check was weakened. The PID and Windows user-work routing
criteria otherwise retain their existing role; the handle label is not a new
proof of Windows-process identity.

INFERRED hardware obligations: boot supported Linux/Windows profiles, verify
scrubber/UVM channel births remain valid, then try a Linux userspace root alloc
with a chosen internal-range handle and USER/ADMIN channel construction. Observe
the route and T-space authorization. No such hardware result is claimed here.
