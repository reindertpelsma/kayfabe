#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# run_windows_apps.sh — the Windows app matrix lane (docs/design/V3_WINDOWS_APP_MATRIX.md). Boots Windows guests on
# kf3 through windows_broker_prod2.sh (or windows_broker.sh), signs in by scripted keystrokes (bootstrap only), hot-plugs
# the read-only app disk, then runs the apps through the guest agent one after another with per-app timeouts, TDR/crash
# accounting and fresh-guest recovery, and writes win.res / win_isolated.res / apps/ / shots/ / summary.md.
#
#   run_windows_apps.sh --run-dir DIR --kf3-rev REV [--iso kfapps.iso] [--run-base N]
#                       [--apps all|id,id] [--tier 1] [--category c,c] [--exclude id,id] [--per-guest 8] [--max-tdr 3]
#                       [--no-isolate] [--resume] [--linux-ref DIR] [--list]
#   run_windows_apps.sh --dry-run          validate everything and run the whole driver against a mock guest (no VM, no GPU)
#
# env: KF3_REV            the kf3 binary under $WIN_DIR/kf3-bins/<rev>/ (same as windows_broker_prod2.sh); or --kf3-rev
#      KF_GUEST_PW        the throwaway password of the test guest's local account (never logged, never committed);
#                         unset = the guest must autologon
#      KF_GUEST_USER      that account (default vast)
#      WIN_DIR            default /var/lib/kf-windows-20261005
#      WIN_RUN_BASE       first boundary-kayfabe-N run number; each guest takes the next free one
#      KF_LOCK            the GPU lock (default /tmp/kayfabe-fastguest.lock, the lock of the Linux lanes and tdr-run.sh);
#                         KF_LOCKED=1 when the caller already holds it
#      WIN_PRE_GUEST_CMD / WIN_POST_GUEST_CMD  host-specific hooks around every guest, e.g. on the 1.20 host
#                         "bash $WIN_DIR/pti-iommu_nogdm.sh identity" / "... DMA-FQ" (what tdr-run.sh does per run)
#      KF3_*, WIN_FLAGS   passed through to the broker (production flags are the broker's own list; add measurement flags here)
# Exit: 0 done, 2 usage, 3 repeated guest boot failures, 4 refused (another QEMU runs / preflight), 5 dry-run failure.
# A run is: START marker in session.log ... EXIT line. No EXIT line = the run died, not finished.
set -uo pipefail
HERE=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)
W=${WIN_DIR:-/var/lib/kf-windows-20261005}
RUN=""; REV=${KF3_REV:-}; ISO=""; DRY=0; LINUXREF=""; PASS=()
while [ $# -gt 0 ]; do case "$1" in
  --run-dir) RUN=$2; shift 2;; --kf3-rev) REV=$2; shift 2;; --iso) ISO=$2; shift 2;; --dry-run) DRY=1; shift;;
  --linux-ref) LINUXREF=$2; shift 2;; -h|--help) sed -n '2,28p' "$0"; exit 0;;
  *) PASS+=("$1"); shift;; esac; done
say(){ echo "[run_windows_apps $(date -u +%FT%TZ)] $*"; }

if [ "$DRY" = 1 ]; then
  T=$(mktemp -d /tmp/kfwa-dry.XXXXXX); trap 'rm -rf "$T"' EXIT
  fail=0
  say "DRY-RUN 1/5 manifest + inventory"
  python3 -I "$HERE/appdisk.py" check --manifest "$HERE/manifest.json" --apps "$HERE/apps.json" --require-pinned || fail=1
  python3 -I "$HERE/gen_apps.py" --check || fail=1
  python3 -I "$HERE/gen_doc.py" --check || fail=1
  say "DRY-RUN 2/5 shell syntax"
  for f in "$HERE"/run_windows_apps.sh "$HERE"/build_appdisk.sh "$HERE"/build_tools.sh; do bash -n "$f" || fail=1; done
  say "DRY-RUN 3/5 unit tests (python -m unittest)"
  ( cd "$HERE" && python3 -I -m unittest discover -s tests 2>&1 | grep -E '^(Ran|OK|FAILED|FAIL:|ERROR:)' ) || fail=1
  say "DRY-RUN 4/5 the driver against a mock guest (fast): 3 guests, a TDR budget restart, a reboot, a wedge"
  cat > "$T/scen.json" <<'JSON'
{"vkpeak": "fail", "dxprobe_d3d12": {"kind": "tdr", "n": 2}, "gputest_fur": {"kind": "tdr", "n": 2}, "furmark_gl_bench": {"kind": "reboot", "once": true}, "scan_t": {"kind": "wedge", "once": true}}
JSON
  python3 -I "$HERE/winapps.py" --vm mock --fast --run-dir "$T/run" --apps nvidia_smi,deviceQuery_demo,dxprobe_d3d12,gputest_fur,furmark_gl_bench,scan_t,matmul_t,vkpeak \
      --per-guest 4 --max-tdr 3 --mock-scenario "$T/scen.json" > "$T/driver.out" 2>&1 || { tail -20 "$T/driver.out"; fail=1; }
  grep -c 'verdict=PASS' "$T/run/win.res" | sed 's/^/  PASS rows: /'
  say "DRY-RUN 5/5 summary"
  python3 -I "$HERE/summarize.py" "$T/run" | head -30 || fail=1
  [ -f "$T/run/win_isolated.res" ] || { echo "  no win_isolated.res: phase 2 did not run"; fail=1; }
  [ $fail = 0 ] && { say "DRY-RUN OK (nothing booted, no GPU touched)"; exit 0; } || { say "DRY-RUN FAILED"; exit 5; }
fi

[ -n "$RUN" ] || { sed -n '2,12p' "$0"; exit 2; }
if [ "${PASS[0]:-}" = "--list" ]; then python3 -I "$HERE/winapps.py" --run-dir "$RUN" --list "${PASS[@]:1}"; exit $?; fi
[ -n "$REV" ] || { say "REFUSED: --kf3-rev or KF3_REV names the kf3 binary"; exit 2; }
[ -x "$W/kf3-bins/$REV/qemu-system-x86_64" ] || { say "REFUSED: no kf3 binary at $W/kf3-bins/$REV/qemu-system-x86_64"; exit 4; }
ISO=${ISO:-$W/appmatrix/image/kfapps.iso}
[ -f "$ISO" ] || { say "REFUSED: no app disk at $ISO (build_appdisk.sh)"; exit 4; }
IMGMAN=$(dirname "$ISO")/manifest.json
if [ -f "$ISO.sha256" ] || [ -f "$(dirname "$ISO")/kfapps.iso.sha256" ]; then
  want=$(cut -d' ' -f1 "$(dirname "$ISO")/kfapps.iso.sha256"); got=$(sha256sum "$ISO" | cut -d' ' -f1)
  [ "$want" = "$got" ] || { say "REFUSED: $ISO sha256 $got != recorded $want"; exit 4; }
fi
if [ "$(pgrep -c qemu-system)" != 0 ]; then say "REFUSED: another QEMU is running (the GPU is serial)"; exit 4; fi
if [ -e "$RUN/session.log" ] && ! printf '%s\n' "${PASS[@]}" | grep -qx -- '--resume'; then say "REFUSED: $RUN has a session.log (use --resume or a new --run-dir)"; exit 4; fi
mkdir -p "$RUN"
if [ "${KF_LOCKED:-0}" != 1 ]; then      # KF_LOCKED=1: the caller already holds the lock (e.g. `flock -o LOCK bash run_windows_apps.sh ...`, as tdr-run.sh is launched)
  exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"
  if ! flock -n 9; then say "waiting for the GPU lock ${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9; fi
fi
XID0=$(dmesg 2>/dev/null | grep -c 'NVRM: Xid')
say "START run=$RUN kf3=$REV iso=$ISO xid_before=$XID0 gpu=$(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>/dev/null | head -1)"
export KF3_REV=$REV
python3 -I "$HERE/winapps.py" --run-dir "$RUN" --kf3-rev "$REV" --iso "$ISO" --image-manifest "$IMGMAN" --w-dir "$W" "${PASS[@]}"
rc=$?
XID1=$(dmesg 2>/dev/null | grep -c 'NVRM: Xid')
{ echo "HOST_XID before=$XID0 after=$XID1"; dmesg 2>/dev/null | grep 'NVRM: Xid' | tail -20; } > "$RUN/host_xid.txt"
python3 -I "$HERE/summarize.py" "$RUN" ${LINUXREF:+--linux-ref "$LINUXREF"} > "$RUN/summary.md"
say "EXIT rc=$rc host_xid_delta=$((XID1 - XID0)) summary=$RUN/summary.md"
exit $rc
