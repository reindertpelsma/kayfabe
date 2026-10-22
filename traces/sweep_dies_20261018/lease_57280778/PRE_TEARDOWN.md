CODE READ (2026-10-22, lease57280778): native and generic Windows baseline pass; bind readiness failed before GPU enable. Full15-line result is `AMPERE_PROGRESS.md`; bounded evidence under `bind_final/`, pinned runtime source905a8bd75. Public export precedes exact-id teardown.

MEASURED (2026-10-22, lease57280778, `bind_final/s4_final_native_check.log`): post-bind journal matches the frozen five-record source baseline; no new/unmatched Xid. Only owned bind PID102996 is requested to exit using QMP quit after metadata export. API absence is not yet claimed; the170-minute auto-destroy remains armed until absence is verified.

MEASURED (2026-10-22, lease57280778, `cleanup_final/s4_owned_vm_stop.json`): owned QEMU PID102996 exited23:50:15Z after QMP quit, rc0. Post-cleanup journal check also matches the frozen baseline (`cleanup_final/s4_final_native_check.log`).
