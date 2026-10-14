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
echo "== gss rows (exactly kayfabe's realize-time requests)"
KF_PROBE_I_KNOW=1 KF_PROBE_GSS_ROWS=1 "$here/kf-ctl-probe" 2>&1 | grep -E '^(GSSROW|PROBE_SESSION|PROBE_SUMMARY|PROBE_END|refusing)'
echo "== cudart gate replies (measured request shapes)"
KF_PROBE_I_KNOW=1 KF_PROBE_EXTRA_ONLY=1 KF_PROBE_HEAD=40 "$here/kf-ctl-probe" "$here/cudart_rows.tsv" 2>&1 | grep -E '^(PROBE |PROBE_SUMMARY|PROBE_END)'
echo "== built-in Windows-only rows (zero requests)"
KF_PROBE_I_KNOW=1 "$here/kf-ctl-probe" 2>&1 | grep -E '^(PROBE |PROBE_SUMMARY|PROBE_END)'
echo "BOX_PROBE_END rc=0 $(date -u +%FT%TZ)"
