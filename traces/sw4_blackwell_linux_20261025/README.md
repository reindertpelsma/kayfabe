# SW4 independent Blackwell Linux cell

STATUS: REFUSED BEFORE NATIVE QUALIFICATION, 2026-10-24 UTC. One lease; no retry.

MEASURED (2026-10-24, lease57581047, [controller.log](controller.log)): the inherited preprovision check stopped with rc9 because `nvidia-smi` could not communicate with the NVIDIA driver. Actual die, native driver version and BAR1 size remain unverified. No driver-matrix row was added.

| Actual die | Native Linux | Guest Linux | Production source | Qualification n |
| --- | --- | --- | --- | --- |
| UNVERIFIED; marketed RTX5060Ti | Preprovision driver-readiness refusal; binding, gates and bare suite UNRUN | EXCLUDED/UNRUN | Planned714ec29d3; no remote checkout or build began | native0 / guest0 |

MEASURED (2026-10-24, lease57581047, [preregistration.json](preregistration.json), [teardown.json](teardown.json)): preferred offer57427095 disappeared from the final fresh snapshot. Authorized fallback50694293/machine45428/host70221 was verified, VM-capable, not VM-deverified, in CA, and absent from the deny lists. The one lease was created23:46:13UTC after watchdog3506194 armed23:46:12UTC; the fixed01:31:36UTC deadline was never extended. Settled console/SSH pin matched once; resource preflight found19 CPUs, KVM, and178524745728 free bytes.

CODE READ ([run_box.sh](../../scripts/bench/sw4/run_box.sh)): the initial `nvidia-smi` call precedes dependency installation, production checkout and pinned-driver provisioning. Its rc9 stopped the controller. No guard was reordered or skipped after this real refusal. This result does not establish a kayfabe failure, an actual GB206/GB207 identity, or a hardware defect.

CODE READ ([preregistration.json](preregistration.json), [operator source hashes](operator_source_hashes.json)): the intended native/guest pair was595.91.07 open; production source714ec29d3fdca0a013d9d478755346b0f1af6fa7; planned flags `KF_DEVICE=kf3 KF_FROM_HOST=1 KF3_FB_MB=8192`; fixed180s suite oracle. The intended8192MiB store preserves host headroom on the marketed16GB adapter; actual VRAM and BAR1 were not established. No QEMU, raw-client, kernel or initrd binary was built, so no binary acceptance receipt exists. Public die derivation was pinned separately to SW3 commit0c326f0a9; the production tree was never edited.

CODE READ ([native Xid guard](../../scripts/bench/native_xid_guard.py), [preregistration.json](preregistration.json)): the reused payload derives native and guest negative-control records from exact source markers and owned channels, VAs, fault types, engines and clients. Unknown, extra or mismatched records refuse. No global Xid exemption, Windows execution, BW8 authority, new RE or performance claim was used. Native/guest Xid and USER qualification censuses remain UNRUN.

MEASURED (2026-10-24, [rehearsal.json](rehearsal.json), [operator_tests.log](operator_tests.log), [payload_tests.log](payload_tests.log)): fresh SW4 suite134/134 and unchanged payload suite140/140 passed before create, with0 skipped. The executable adapter, provider, settled SSH pin, precreate watchdog, capture pull/hash and exact-ID teardown ran against fake transports. The first private payload copy had missing dependencies; those fixtures were restored from pinned public source before the complete green run. No BW8 rehearsal result was reused.

MEASURED (2026-10-24, [rehearsal.json](rehearsal.json), [SW4 mutants](sw4_mutants.json), [selector mutants](selector_mutants.json), [binding mutants](binding_mutants.json)): five tests-only EXPECTED RED commits preceded fix e0c03c908. All28 behavioral mutants have at least2 distinct killing tests:19 SW4 scope/budget/lifecycle/transfer,5 inherited selector,4 inherited binding. Formatting used Black25.1.0 with100-column code; no local Cargo build ran.

MEASURED (2026-10-24, lease57581047, [capture_receipt.json](capture_receipt.json), [teardown.json](teardown.json), [cleanup.json](cleanup.json)): the33003-byte private text capture was pulled and SHA256-verified23:55:09UTC before exact-ID API absence23:55:12UTC, then the starttime-pinned watchdog was cancelled. SHA256 is `011188e22b9124bd35ae6265fcb769e7b4810464d99f97ffca0f1217c3778f6e`. Original stop receipts and the full capture are retained privately.

MEASURED (2026-10-24, lease57581047, [cost.json](cost.json)): requested180GiB quote and observed rate0.320666667USD/h;538.461175s create-to-absence gives runtime estimate0.047962931USD. Retained transfer liability0.253906250USD and additional nonruntime margin0.500000000USD give conservative charge0.801869181USD, below stop1.50/cap2.00. With SW3's retained0.675822996USD, shared retained total is1.477692177USD versus cap3.50. Credit23.091378551USD exceeds reserve8USD. These are conservative estimates/reserves, not a settled invoice.

INFERRED (lease57581047, [result.json](result.json)): any future qualification needs separate authority and a fresh native baseline. This lane's one-lease authorization is exhausted. No actual die or driver cell can be accepted from this run.
