STATUS: LIVE, 2026-10-22. S2 resumed by explicit owner instruction; guard implementation complete, hardware not yet run.

MEASURED: EXPECTED RED test-only commits c38cd6fde (46 absent-API errors),
446f28b46 / 48b9a7a17 (supervisor requirements), and fe9459464 (two native
four-fault baseline failures) precede the fix. The original 23 cases remain green.
MEASURED: final combined suite 97 passed / 0 failed / 0 skipped; shell syntax clean.
MEASURED: 22 hand-edited mutants each fail at least two distinct tests, with
zero test errors. Source is restored and the 97-case suite rerun green.

CODE READ: this optional bench supervisor pins the source/runner files to the
checkout revision and the QEMU path to its build revision. It records its actual
live QEMU child's PID, starttime and executable. Exact source FAULT_MARKs,
submission guest channels, matching physical host-twin RC records, PCI device,
address, fault/access/route, completion and recovery are required. Native approval
requires exactly five source-matched faults; the accepted history is immutable.
CODE READ: during an active negative arm, incomplete but compatible observations
remain pending, never accepted. Irreconcilable faults stop the owned child; full
proof is required after exit before history advances or another arm can start.
No wildcard Xid31 or Xid32 exception. Nonnegative arms require unchanged history.
CODE READ: the optional suite driver reads all 30 arms from the pinned original
suite and runs each through that suite, preserving the 180-second arm budget;
it stops on the first failed arm. No product or guest boot flags were changed.
MEASURED: missing-page parser fixtures are synthetic. The historical missing-page
channel gap was not promoted into evidence; fresh hardware proof remains required.

| Mutant | Distinct failing tests |
|---|---:|
| ignore_actual_qemu_pid | 2 |
| ignore_host_guest_channel_pair | 4 |
| ignore_fault_address | 3 |
| allow_non31_code | 3 |
| ignore_fault_access_type | 2 |
| skip_completed_recovery_proof | 5 |
| allow_native_history_replacement | 2 |
| allow_trailing_payload | 2 |
| allow_hidden_first_xid | 3 |
| skip_context_provenance | 3 |
| ignore_local_source_changes | 2 |
| skip_live_identity_recheck | 2 |
| allow_unowned_process_capture | 2 |
| ignore_route | 2 |
| ignore_pci | 2 |
| allow_reduced_native_baseline | 2 |
| ignore_final_twin_count | 2 |
| ignore_capture_executable | 2 |
| skip_binary_revision | 2 |
| skip_one_arm_primitive | 2 |
| leave_owned_child_running | 2 |
| continue_after_failed_arm | 2 |
