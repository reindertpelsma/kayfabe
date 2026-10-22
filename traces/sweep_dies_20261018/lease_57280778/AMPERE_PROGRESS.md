AMPERE RESUME RESULT — 2026-10-22

1. CODE READ (2026-10-22): S4 branch `codex/sweep-dies-20261022`, runtime source905a8bd75e0fc53f5494e45b6d2a0fee474d7757; requested Windows580.88 desktop/appmatrix/Unigine/600s hold after native gates.
2. MEASURED (2026-10-22, lease57280778, `native_gates/sweep_host_die.log`): RTX3090 is GA102, host595.91.07 OPEN; KVM present,8 CPUs,17949MiB actual RAM.
3. MEASURED (2026-10-22, lease57280778, `native_complete/sweep_host_preflight.log`, `native_complete/sweep_gates.log`): SOUND, gates9/9+gate10PASS, USER births12/12,0refused.
4. MEASURED (2026-10-22, lease57280778, `native_complete/sweep_bare595.log`): native raw30/30,0fail/0crash; CUDA4/4 and GRtierPASS; n1,spreadN/A.
5. MEASURED (2026-10-22, lease57280778, `native_complete/native_xid_baseline.json`, `native_controls/`): owner-approved fourCE0/clientCE0 source controls plus one GR/ESC control admitted exactly; channel/VA/type/count remain exact.
6. MEASURED (2026-10-22, lease57280778, `appdisk_receipt.json`, `parallel_transfer.log`): pinned app image6785255424 bytes and fullSHA verified; eight checked transport streams completed732.410s; NVRTC supplement also hash-verified on host.
7. MEASURED (2026-10-22, lease57280778, `bind_final/sweep_windows_install.log`): standard-VGA Windows installation/verificationPASS, WIN_CHECK_DONE fail0; base sealed23:28:57Z.
8. MEASURED (2026-10-22, lease57280778, `bind_final/bwr_bind.log`): display/GOP-on bind boot began23:28:59Z; readiness failed23:48:35Z with SSH banner timeout; controller exited1.
9. MEASURED (2026-10-22, lease57280778, `bind_final/arms/bw580_bind/summary.json`): RPC count0, no ordinal or control/class failure; GPU enable, Code0, Explorer/DWM acceptance, appmatrix, Unigine and600s hold NOTRUN.
10. MEASURED (2026-10-22, lease57280778, `BIND_BOOT_SNAPSHOT.json`, `BIND_BOOT_SNAPSHOT_NOTE.md`): QMP running, guest-ping timeout; only initial reset/start observed. Black/BootManager frames do not establish a reboot or TDR; exact cause unverified.
11. CODE READ (2026-10-22, `harness_recipe/`): normal19 production flags, pinned QEMU/source,8GiB guest RAM and framebuffer selected before boot; no timer, CPU, admission or runtime flag change to cure the failure.
12. MEASURED (2026-10-22, lease57280778, `bind_final/s4_final_native_check.log`, `bind_final/post_windows_kernel_snapshot.log`): post-bind journal still passes immutable native baseline; no new/unmatched Xid. No Windows event timestamps available before SSH.
13. MEASURED (2026-10-22, `../round3/all_resume_green.log`, `../round3/owner_family_mutants.json`, `../round3/chunks_mutants.json`):154/154 offline tests; six guard mutants killed3/2/5/5/5/3, five transport mutants each2; added Rust lint sites0, clippy not rerun.
14. INFERRED (2026-10-22): combined acceptance FAIL at pre-driver bind readiness, not evidence of an NVIDIA initialization defect; recommend a separate public-source boot/display investigation. Optional alloc-header exporter was absent; zero-RPC and channel-state metadata were exported without claiming allocation-body coverage.
15. CODE READ (2026-10-22): preserve this failed arm and evidence before exact-id teardown/API absence; Turing follows only after absence. No RE or secrets on the rental, no dev cargo target or shared-host GPU work.
