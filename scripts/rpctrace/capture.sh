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
# ★ Did we ever actually take the bench off its stock module? Set at the moment
# the stock stack is UNLOADED (step 1), which is the first instant the bench is
# not on stock — not at the insmod, which can fail after the unload. See on_exit().
SWAPPED=0
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
    #
    # ⚠⚠ MEASURED, 2026-08-03, on the GA102 box: this trap printed
    #   `‼‼‼ BENCH LEFT WITH A NON-STOCK OR NON-WORKING DRIVER — NEEDS A HUMAN`
    # about a bench that was on its stock module, with a working nvidia-smi, that
    # had never been touched. The script had died in the BASELINE check — step 0,
    # before the unload — and the trap fired purely on `RESTORED != 1`, which is
    # also true of every abort that happens before anything is swapped.
    #
    # ⊘ That is a false alarm of the worst kind on a shared bench: the loudest
    # possible message, saying a healthy machine is broken. It is the same defect
    # class as the empty-dmesg harness — a check whose output does not distinguish
    # "the thing is bad" from "I never looked".
    #
    # ⇒ The trap now branches on whether we ever SWAPPED. If we did not, there is
    # nothing to restore and it says so; the invariant is still asserted (we do
    # not just stay quiet) — it just asserts the true one.
    #
    if [ "$SWAPPED" != 1 ]; then
      echo "── exiting with rc=$rc; the instrumented module was never loaded, so there is"
      echo "   nothing to restore. Verifying the bench is on stock anyway:"
      if [ -e /proc/driver/nvidia/rpctrace ]; then
        echo "‼‼‼ /proc/driver/nvidia/rpctrace EXISTS but we never loaded — NEEDS A HUMAN"
      elif nvidia-smi -L >/dev/null 2>&1; then
        echo "   OK: stock module, nvidia-smi works. Bench untouched."
      else
        echo "‼‼‼ nvidia-smi does not work and we never swapped — the bench was ALREADY"
        echo "     broken when this script started. NEEDS A HUMAN."
      fi
    else
      echo "── exiting with rc=$rc before a verified restore; attempting one now"
      restore_stock || echo "‼‼‼ BENCH LEFT WITH A NON-STOCK OR NON-WORKING DRIVER — NEEDS A HUMAN"
    fi
  fi
  exit $rc
}
trap on_exit EXIT

# ── 0. Baseline: the bench must be healthy BEFORE we touch it ─────────────────
say "baseline check on the stock module"
nvidia-smi -L >"$OUTDIR/${TAG}_baseline_smi.txt" 2>&1 \
  || die "nvidia-smi already fails BEFORE we do anything — fix the bench first"
cat "$OUTDIR/${TAG}_baseline_smi.txt"

# ⚠ MEASURED, 2026-08-03, GA102 box: this fired on the script's OWN baseline
# `nvidia-smi -L` three lines above. nvidia-smi's exit is not instantaneous — it
# holds /dev/nvidia0, /dev/nvidiactl and /dev/nvidia-uvm open while it tears its
# context down, and on a box where nvidia_uvm is loaded that takes longer than the
# next statement. The check is right to exist (a real user of the GPU must stop
# us) but a single instantaneous sample cannot tell a departing process from a
# resident one. ⇒ Give it a few seconds to drain, and only then refuse.
for _ in 1 2 3 4 5 6 7 8 9 10; do
  fuser -s /dev/nvidia* 2>/dev/null || break
  sleep 1
done
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
# ★ From HERE the bench is no longer on its stock module — not from the insmod.
# An unload that succeeds and an insmod that then fails (which is exactly what
# happened on the GA102 box's first run, see step 2) leaves the box with NO nvidia
# module at all, and that needs the same restore as a successful swap.
SWAPPED=1

# ── 2. Load the instrumented one BY PATH ──────────────────────────────────────
#
# ⚠ MEASURED, 2026-08-03, on the GA102 box (RTX 3090, 575.51.03): the insmod
# below failed with `Unknown symbol ecc_make_pub_key / ecc_get_curve /
# ecc_gen_privkey`. ⊘ Nothing to do with the recorder — `insmod` loads exactly
# one file and resolves NO dependencies, and this box's instrumented module has
# `depends: ecc` where the GA106 bench's had none.
#
# The reason it differed is worth keeping: that box's STOCK module is the
# PROPRIETARY one (`license: NVIDIA`), which carries its own ECC inside
# `nv-kernel.o_binary`, while we necessarily build the OPEN module
# (`license: Dual MIT/GPL`), which links the kernel's `crypto/ecc.ko`. So the
# stock module's `modinfo -F depends` is EMPTY and ours is not — reading the
# dependency list off the *installed* module would have found nothing and this
# would still fail. It has to be read off the module we are about to load.
#
# We keep `insmod`-by-path (that is what guarantees the stock module on disk is
# never in play) and pre-load the dependencies with modprobe explicitly.
deps="$(modinfo -F depends "$KO" 2>/dev/null | tr ',' ' ')"
if [ -n "$deps" ]; then
  say "instrumented module depends on: $deps — pre-loading (insmod resolves nothing)"
  for d in $deps; do
    modprobe "$d" || die "could not modprobe dependency '$d' needed by $KO"
  done
fi

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
