#!/usr/bin/env bash
# capture.sh — swap in the instrumented nvidia.ko, drive a SUCCESSFUL GSP boot,
# drain the RPC trace, and put the stock module back.
#
# Task #178, `replay-conformance`. Spec: docs/design/rpc_trace_capture.md.
#
# ────────────────────────────────────────────────────────────────────────────────
# ★★★ THE POINT OF THE CAPTURE IS THAT THE BOOT SUCCEEDS.
#
# `traces/mode2_c_reference/cap1_coldboot_hermetic` is a trace of a boot that
# FAILS — it ends where our emulator stopped. This one runs `nvidia-smi` against
# real GSP-RM on a real GA106 and only counts if `nvidia-smi` works, because the
# thing we do not have and cannot get any other way is the sequence PAST where we
# currently stop, together with the answers a real GSP gave.
#
# ⚠ THIS RUNS ON A SHARED BENCH. Two properties keep that safe, and both are
# asserted rather than assumed:
#   1. The stock module on disk is never modified. We `insmod` ours BY PATH; the
#      restore is a plain `modprobe nvidia`, which can only find the DKMS one.
#   2. The restore is verified by a POSITIVE test that discriminates the two
#      modules — `/proc/driver/nvidia/rpctrace` must be GONE — and by
#      `nvidia-smi` working afterwards. "modprobe returned 0" is not a restore.
# An EXIT trap re-attempts the restore on any failure path and shouts if it
# cannot. ⊘ If you see "COULD NOT RESTORE" in this output, the bench is broken
# and needs a human; do not read past it.
# ────────────────────────────────────────────────────────────────────────────────
#
#   sudo ~/rpctrace/capture.sh --tag boot1
#   sudo ~/rpctrace/capture.sh --tag overflow --kb 64     # force the guard to fire
#
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KO="${KO:-$HOME/rpctrace-out/nvidia.ko}"
OUTDIR="${OUTDIR:-$HOME/rpctrace-out}"
KB="${KB:-65536}"
TAG="boot1"

while [ $# -gt 0 ]; do
  case "$1" in
    --ko)  KO="$2";     shift 2;;
    --out) OUTDIR="$2"; shift 2;;
    --kb)  KB="$2";     shift 2;;
    --tag) TAG="$2";    shift 2;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done

die()  { echo "‼ $*" >&2; exit 1; }
say()  { echo "── $*"; }

[ "$(id -u)" = 0 ] || die "must run as root"
[ -f "$KO" ] || die "no instrumented module at $KO — run build_instrumented.sh first"
mkdir -p "$OUTDIR"

RESTORED=0
PERSISTENCED_WAS_ACTIVE=0
STOCK_KO="/lib/modules/$(uname -r)/updates/dkms/nvidia.ko"

# ⚠ MEASURED, 2026-08-03, first run of this script: `rmmod nvidia` in the restore
# path FAILED and the bench was left on the instrumented module. Cause:
# `nvidia-smi` shells out to `nvidia-modprobe`, which loads **nvidia_uvm** — so by
# drain time our module had a holder and a refcount of 1. The restore had only
# ever considered `nvidia` itself. Both the pre-load and the restore now go
# through the same unloader, which is the point: two hand-written unload
# sequences is one sequence and one bug waiting.
unload_all() {
  for m in nvidia_drm nvidia_modeset nvidia_uvm nvidia_peermem nvidia; do
    if lsmod | grep -q "^$m "; then rmmod "$m" || return 1; fi
  done
  lsmod | grep -q "^nvidia" && return 1
  return 0
}

restore_stock() {
  say "restoring the stock module"
  unload_all || { echo "‼‼‼ COULD NOT RESTORE: unload failed; holders: $(cat /sys/module/nvidia/holders/* 2>/dev/null; ls /sys/module/nvidia/holders 2>/dev/null)"; return 1; }
  modprobe nvidia || { echo "‼‼‼ COULD NOT RESTORE: modprobe nvidia FAILED"; return 1; }
  # ★ TWO POSITIVE discriminators, not an absence of errors.
  #   (a) only the instrumented module publishes this proc file;
  #   (b) srcversion is a hash of the sources the module was built from, so it
  #       tells the two apart even if (a) were ever to be removed.
  if [ -e /proc/driver/nvidia/rpctrace ]; then
    echo "‼‼‼ COULD NOT RESTORE: /proc/driver/nvidia/rpctrace still present — the"
    echo "     loaded module is STILL THE INSTRUMENTED ONE."
    return 1
  fi
  if [ -r "$STOCK_KO" ]; then
    want="$(modinfo -F srcversion "$STOCK_KO")"
    got="$(cat /sys/module/nvidia/srcversion 2>/dev/null)"
    if [ "$want" != "$got" ]; then
      echo "‼‼‼ COULD NOT RESTORE: loaded srcversion $got != stock $want"
      return 1
    fi
    say "srcversion matches the stock DKMS module ($got)"
  fi
  if ! nvidia-smi -L >"$OUTDIR/${TAG}_restore_smi.txt" 2>&1; then
    echo "‼‼‼ COULD NOT RESTORE: nvidia-smi fails on the stock module"
    cat "$OUTDIR/${TAG}_restore_smi.txt"
    return 1
  fi
  [ "$PERSISTENCED_WAS_ACTIVE" = 1 ] && systemctl start nvidia-persistenced 2>/dev/null
  say "stock module restored and verified: $(cat "$OUTDIR/${TAG}_restore_smi.txt")"
  RESTORED=1
  return 0
}

on_exit() {
  local rc=$?
  if [ "$RESTORED" != 1 ]; then
    echo
    echo "── exiting with rc=$rc before a verified restore; attempting one now"
    restore_stock || echo "‼‼‼ BENCH LEFT WITH A NON-STOCK OR NON-WORKING DRIVER — NEEDS A HUMAN"
  fi
  exit $rc
}
trap on_exit EXIT

# ── 0. Baseline: the bench must be healthy BEFORE we touch it ─────────────────
say "baseline check on the stock module"
nvidia-smi -L >"$OUTDIR/${TAG}_baseline_smi.txt" 2>&1 \
  || die "nvidia-smi already fails BEFORE we do anything — fix the bench first"
cat "$OUTDIR/${TAG}_baseline_smi.txt"

if fuser -s /dev/nvidia* 2>/dev/null; then
  die "something is using the GPU right now: $(fuser -v /dev/nvidia* 2>&1 | tail -n +2)"
fi

if systemctl is-active --quiet nvidia-persistenced 2>/dev/null; then
  PERSISTENCED_WAS_ACTIVE=1
  systemctl stop nvidia-persistenced
fi

# ── 1. Unload the stock stack ─────────────────────────────────────────────────
say "unloading the stock stack"
unload_all || die "could not unload the stock stack"

# ── 2. Load the instrumented one BY PATH ──────────────────────────────────────
say "loading $KO with NVreg_RpcTraceKB=$KB"
dmesg -C 2>/dev/null || true   # so the persisted dmesg below is THIS boot's
insmod "$KO" NVreg_RpcTraceKB="$KB" || die "insmod failed (dmesg: $(dmesg | tail -3))"

dmesg | grep -q "rpctrace: armed" \
  || die "recorder did not arm — dmesg says: $(dmesg | grep -i rpctrace | tail -3)"
[ -e /proc/driver/nvidia/rpctrace ] || die "/proc/driver/nvidia/rpctrace missing"

# Device nodes: we insmod'd by hand, so nothing created them.
if [ ! -e /dev/nvidia0 ]; then
  nvidia-modprobe -c 0 2>/dev/null || true
fi
if [ ! -e /dev/nvidia0 ]; then
  major="$(awk '/nvidia-frontend/ {print $1}' /proc/devices)"
  [ -n "$major" ] || die "no nvidia-frontend major in /proc/devices"
  mknod -m 0666 /dev/nvidiactl c "$major" 255
  mknod -m 0666 /dev/nvidia0   c "$major" 0
fi

# ── 3. THE BOOT. This is the run that has to SUCCEED ──────────────────────────
say "driving a GSP boot: nvidia-smi"
smi_rc=0
nvidia-smi          >"$OUTDIR/${TAG}_smi.txt"   2>&1 || smi_rc=$?
nvidia-smi -q       >"$OUTDIR/${TAG}_smi_q.txt" 2>&1 || smi_rc=$?
head -12 "$OUTDIR/${TAG}_smi.txt"

# ── 4. Drain BEFORE judging the boot ──────────────────────────────────────────
# ⊘ Deliberately ordered this way: a boot that failed still produced a trace, and
# that trace is the most interesting thing in the room. Judge afterwards.
say "draining /proc/driver/nvidia/rpctrace"
cat /proc/driver/nvidia/rpctrace >"$OUTDIR/$TAG.bin" || die "drain failed"
[ -s "$OUTDIR/$TAG.bin" ] || die "drained an EMPTY trace"

# ★★★ PERSIST dmesg AND ASSERT IT IS REAL. The project has been burned by a
# harness that wrote an empty log and exited 0 — the file's existence read as
# capture. An empty or NVRM-free dmesg here means the evidence is somewhere else,
# which is the same as not having it.
dmesg >"$OUTDIR/${TAG}_dmesg.log" 2>&1 || true
[ -s "$OUTDIR/${TAG}_dmesg.log" ] || die "persisted dmesg is EMPTY"
grep -qi NVRM "$OUTDIR/${TAG}_dmesg.log" \
  || die "persisted dmesg contains no NVRM lines — this is not the driver's output"

ls -l "$OUTDIR/$TAG.bin"

if [ "$smi_rc" != 0 ]; then
  echo "⚠ nvidia-smi FAILED (rc=$smi_rc). The trace was still captured, but it is a"
  echo "  trace of a boot that did not succeed — which is what we already had."
fi

# ── 5. Restore, and verify the restore ────────────────────────────────────────
restore_stock || die "restore failed"

say "capture complete: $OUTDIR/$TAG.bin"
say "decode with: scripts/rpctrace/decode_rpctrace.py $TAG.bin --summary"
[ "$smi_rc" = 0 ] || exit 1
exit 0
