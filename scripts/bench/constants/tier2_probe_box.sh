#!/bin/sh
# Tier-2 constants probe on a rented box (lane C, 2026-10-14). Runs ON the box, as the box's normal user/root
# WITHOUT CAP_SYS_ADMIN (a default container rental is exactly kayfabe's unprivileged position; the probe refuses
# to run with CAP_SYS_ADMIN unless KF_PROBE_ALLOW_ADMIN=1, and then labels the run admin).
# Inputs next to this script: kf-ctl-probe (static musl build of crates/kf-harness/src/bin/kf-ctl-probe.rs),
# cudart_rows.tsv. Output: one text file on stdout; nothing else leaves the box.
# What it prints, and nothing more: GPU name, PCI device id and driver version (no UUID, serial, VBIOS, bus id),
# CapEff, the RM status of every kf_abi::gssreplay row kayfabe asks at realize, the built-in Windows-only rows
# (status + 16-byte prefix), and the cudart-gate replies (first 40 bytes: control words, no board data).
set -u
here=$(cd "$(dirname "$0")" && pwd)
echo "BOX_PROBE_START $(date -u +%FT%TZ)"
nvidia-smi --query-gpu=name,pci.device_id,driver_version --format=csv,noheader 2>&1 | head -4 | sed 's/^/GPU /'
grep -E '^(CapEff|Uid):' /proc/self/status | tr '\t' ' ' | sed 's/^/PROC /'
# A container sees the host's device node by the HOST's index (/dev/nvidia3 on a multi-GPU host): use the one visible.
KF_GATE_GPU=$(ls /dev/nvidia[0-9]* 2>/dev/null | head -1 | sed 's/[^0-9]//g'); export KF_GATE_GPU=${KF_GATE_GPU:-0}
echo "NODE nvidia$KF_GATE_GPU"
# run <pattern> <env...> -- <args...>: print the matching lines; if the probe printed none, print why (its first 4 lines,
# which are the tool's own diagnostics: errno / device-open failures; never RM data).
run() {
  pat=$1; shift
  out=$(env KF_PROBE_I_KNOW=1 "$@" 2>&1)
  m=$(printf '%s\n' "$out" | grep -E "$pat")
  if [ -n "$m" ]; then printf '%s\n' "$m"; else echo "PROBE_NO_OUTPUT first_lines:"; printf '%s\n' "$out" | head -4 | cut -c1-200; fi
}
echo "== gss rows (exactly kayfabe's realize-time requests)"
run '^(GSSROW|PROBE_SESSION|PROBE_DIE|PROBE_SUMMARY|PROBE_END|refusing)' KF_PROBE_GSS_ROWS=1 "$here/kf-ctl-probe"
echo "== cudart gate replies (measured request shapes)"
run '^(PROBE |PROBE_SUMMARY|PROBE_END)' KF_PROBE_EXTRA_ONLY=1 KF_PROBE_HEAD=40 "$here/kf-ctl-probe" "$here/cudart_rows.tsv"
echo "== built-in Windows-only rows (zero requests)"
run '^(PROBE |PROBE_SUMMARY|PROBE_END)' "$here/kf-ctl-probe"
echo "== 595-branch request sizes (Windows 596.51 / GSP 595.76 capture; zero requests)"
run '^(PROBE |PROBE_SUMMARY|PROBE_END)' KF_PROBE_EXTRA_ONLY=1 KF_PROBE_HEAD=16 "$here/kf-ctl-probe" "$here/rows_595_shapes.tsv"
echo "== ZBC clear table (idle rental: the occupied entries are the defaults physical RM installs)"
run '^(ZBC|PROBE_END)' KF_PROBE_ZBC=1 "$here/kf-ctl-probe"
# Diagnostics when the host RM session could not open (box facts only: device nodes and the process's confinement).
if ! KF_PROBE_I_KNOW=1 KF_PROBE_GSS_ROWS=1 "$here/kf-ctl-probe" 2>&1 | grep -q '^PROBE_SESSION'; then
  echo "DIAG nodes:"; ls -ln /dev/nvidia* 2>&1 | head -12
  echo "DIAG open nvidia$KF_GATE_GPU: $( (: < /dev/nvidia$KF_GATE_GPU) 2>&1 | head -1 )"
  grep -E '^(Seccomp|NoNewPrivs):' /proc/self/status | tr '\t' ' ' | sed 's/^/DIAG /'
  echo "DIAG cgroup devices: $(cat /sys/fs/cgroup/devices.list 2>/dev/null | head -3 | tr '\n' ';')"
fi
echo "BOX_PROBE_END rc=0 $(date -u +%FT%TZ)"
