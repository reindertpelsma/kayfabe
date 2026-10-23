# One production-mode KVM run

[MEASURED: 2026-10-23, run v7-kvm-20261023; live_run.json, availability.json] Exactly one
vm:true, ssh_direct rental used the accepted Worker bundle and the production clone/checkout
bootstrap. GA106 had no qualifying offer; all five qualifying offers were AD102. The chosen
US RTX 4090 cost 0.753333333333 USD/h for compute/storage, with a full 0.046875 USD transfer
reserve, or 0.768958333333 USD/h effective over the three-hour policy window. No requirement
was relaxed. The independent reaper was armed before creation. The VM exposed 30 vCPUs and
103589964 KiB RAM. The local GitHub identity/JWKS service was used; this is local-run evidence.

[MEASURED: 2026-10-23, run v7-kvm-20261023; timings.json, startup_missing_contract.json] Vast
first reported actual_status=running 904.048 seconds after its create response. cur_state and
intended_status already said running during earlier unknown/loading/created observations.
Cloud-init cloned exact SHA 28084b96cebad2a853d72171f72f8d776946b8ba, then environment validation
refused every required KF_* field. The same unchanged entrypoint was restarted at 02:00:03 UTC
through the existing valid SSH login environment. First report acknowledgement followed the
create response by 1954.956 seconds. No source was edited on the VM and no second lease ran.
The new provider-file loader is fixture-tested; unattended VM startup with it is unverified.

[MEASURED: 2026-10-23, run v7-kvm-20261023; infra_timings_interim.json, public_log_final.txt] The
box setup took 54m55s; the host driver swap took 3m57s and reached open driver 580.159.04.
The selected APT mirror was azure.archive.ubuntu.com, chosen by a 1849679 B/s Release-file
probe. Provision entry through initial APT completion spans 53m52s, including preflight,
update, download and installation. This is an observation window, not exact apt command
runtime: APT history records only 57 seconds for package configuration. Bench-tree host
dependencies then took 15m04s. QEMU and cloud-image downloads returned zero. The QEMU track
reported A_RC=0 and KF3_BUILT for this revision; nested guest driver content was checked as
580.159.04. Those intermediate successes do not supply the missing full provision verdict.

[MEASURED: 2026-10-23, run v7-kvm-20261023; public_log_final.txt, live_run.json] The provision
stage ended after 4690.191 seconds with TREE_RC=1 and guest ladder MISSING, despite a successful
musl Cargo build. The adapter returned structured FAIL/provision and stopped before host CUDA,
merge_check and fat CUDA. The independent runner validated bindings, schema and the log chain;
its verified=1 flag denotes those checks. gpu-vast remains an untrusted tripwire.
[CODE READ: vast_payload.py, provision_bench_tree.sh] The adapter's global Cargo target pointed
at bench/kf-verify-target, while existing consumers resolve checkout/target. The adapter now
forces checkout/target. It also refreshes hardware before grading a failed provision: the old
wire FAIL carried the initial 575.51.03 probe even though the swap reached 580.159.04.
[MEASURED: 2026-10-23, run v7-all-vast-tests-20261023; target_contract_red.txt,
failure_hardware_red.txt, all_tests.json, mutants.json] Each defect had two standalone red
consumer/output probes before its fix. The final 198 tests pass; 27 payload mutations each
have at least two named killers. These fixes were not run on this immutable rented SHA.

[MEASURED: 2026-10-23, run v7-kvm-20261023; live_run.json, public_log_final.txt] A real direct
interactive SSH login exited zero, changed GRACE to MANUAL and preserved the recorded result.
The log records grace/heartbeat suspension with an active bounded hard timer. A separate
provider-proxy SSH attempt returned 255; the direct login supplies the hook evidence. The
controller stopped the box before the original grace deadline, so survival past that deadline
was not observed. There were 14 accepted callbacks and 109 real scheduled-handler completions;
this run did not exercise destruction at a cron deadline.

[MEASURED: 2026-10-23, run v7-kvm-20261023; live_run.json, cleanup.json] Owner stop destroyed
the exact owned ID/label, independently rechecked through the native Vast APIs. Conservative
compute/storage plus full transfer-reserve estimate: 1.442036 USD, below the 4 USD cap and
3 USD stop threshold; this is not an invoice. The reaper stopped only after API absence.
All owned listeners and controller children stopped, and the exact-box source-IP gate was
removed. The local rental path used the unchanged bundle
`2402798ce8ab53dcc4db5efca7891b565c0bca7e92723ff0b5050e5bb6e6760f`.

[CODE READ: V7 acceptance scope] Remaining work: a fresh authorized VM run of the provider-env,
Cargo-path and failure-hardware fixes; full host CUDA, merge bar and fat CUDA results; S4's
exact-channel guard integration; observed survival past the old grace deadline; hosted OIDC
and billed Cloudflare CPU. Deferred format pass 2, Worker README restructure and review lows
remain deferred. No Worker deployment or production renting enablement occurred.
