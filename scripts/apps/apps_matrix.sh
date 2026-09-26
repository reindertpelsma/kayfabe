#!/usr/bin/env bash
# apps_matrix.sh — drive the V3 app matrix on a bench box (strictly serial: one GPU job at a time).
#   apps_matrix.sh host  <run> <app|all>...   bare-metal baseline on this box (as root)
#   apps_matrix.sh guest <run> <app|all>...   the same apps inside the kf3 fat guest; batched per
#                                             boot, continuing in a fresh boot after a GUEST_DEAD,
#                                             then every non-PASS app is RE-RUN ALONE in its own
#                                             fresh boot (so a failure is not contamination from
#                                             an earlier app in the same boot).
# env: KF3_BIN (kf3 qemu binary; default: the newest in /workspace/bench/kf3-bins),
#      NVKVM_RAM_MB (16384), KF_SMP (6), KF3_FB_MB, APPS_GUEST_PM (0/1)
# results: /workspace/apps/results/<run>/{host.res,guest.res,guest_isolated.res,<app>.*.log}
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
SIDE=${1:?host|guest}; RUN=${2:?run}; shift 2
R=/workspace/apps/results/$RUN; mkdir -p "$R"
say(){ echo "[apps_matrix $(date -Is)] $*"; }
ALL=$(bash "$HERE/run_apps.sh" x list)
[ "${1:-}" = all ] && set -- $ALL
say "START side=$SIDE run=$RUN rev=$(git -C "$REPO" rev-parse --short=8 HEAD) apps=$#"
busy(){ pgrep -x qemu-system-x86 >/dev/null || pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; }
while busy; do say "waiting: a QEMU/cargo is running (serial bench)"; sleep 20; done

if [ "$SIDE" = host ]; then
  mkdir -p /opt/apps
  cp -f "$HERE"/src/*.py /opt/apps/bundle/share/
  bash "$HERE/run_apps.sh" host "$@" | tee -a "$R/host.res"
  for f in /opt/apps/out/host/*.log; do cp -f "$f" "$R/$(basename "$f" .log).host.log"; done
  dmesg | grep -E 'Xid|NVRM' | tail -50 > "$R/host_dmesg_tail.log"
  say "HOST_DONE $(grep -c 'verdict=PASS' "$R/host.res") pass / $(wc -l < "$R/host.res") rows"
  exit 0
fi

QB=${KF3_BIN:-$(ls -td /workspace/bench/kf3-bins/*/qemu-system-x86_64 | head -1)}
[ -x "$QB" ] || { say "⊘ no kf3 binary"; exit 2; }
say "kf3 binary: $QB"
boot(){  # $1 tag, $2 apps, $3 outdir
  env KF_DEVICE=kf3 QEMU_BIN="$QB" NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} KF_SMP=${KF_SMP:-6} GQ_TIMEOUT=300 \
      APPS="$2" APPS_OUT="$3" POST_CAPTURE_HOOK="$HERE/apps_hook.sh" \
      bash "$REPO/scripts/bench/boot_capture.sh" "$1" > "$3/boot_$1.driver.log" 2>&1
  local rc=$?
  for x in dmesg dmesg_after probe hostdmesg serial; do cp -f "/workspace/bench/run_$1_$x.log" "$3/boot_$1.$x.log" 2>/dev/null; done
  zstd -q -f "/workspace/bench/run_$1_qemu.log" -o "$3/boot_$1.qemu.log.zst" 2>/dev/null
  say "boot $1 rc=$rc apps=[$2] :: $(grep -a 'APPS_HOOK\|FAILED' "$3/boot_$1.driver.log" "/workspace/bench/run_$1_probe.log" 2>/dev/null | tail -2 | tr '\n' ' ' | cut -c1-200)"
  while busy; do sleep 3; done
}
# APPS_PER_BOOT (default 1): apps per fresh boot. ⊘ Measured r1 on va1 (2026-09-26): in ONE boot,
# the 3rd CUDA process wedged and every later app timed out silently (kf3: "slot 27 is full and
# nothing can be retired — raise WalkCfg::runs_per_pdb") — so a batched verdict is contaminated by
# the apps before it. One app per boot is the clean measurement; batching is a separate experiment.
PER=${APPS_PER_BOOT:-1}
# ---- phase 1: batched --------------------------------------------------------------------------
todo="$*"; n=0
while [ -n "$(echo $todo)" ]; do
  n=$((n+1)); tag="ap_${RUN}_b$n"
  batch=$(echo $todo | cut -d' ' -f1-"$PER")
  boot "$tag" "$batch" "$R"
  done_apps=$(grep -a "boot=$tag " "$R/guest.res" 2>/dev/null | sed -n 's/.* app=\([^ ]*\) .*/\1/p')
  if [ -z "$done_apps" ]; then
    first=$(echo $todo | cut -d' ' -f1)
    echo "APPRES side=guest app=$first verdict=BOOT_FAIL rc=- secs=- quiet=- note=boot_capture-produced-no-app-result boot=$tag" | tee -a "$R/guest.res"
    done_apps=$first
  fi
  new=""; for a in $todo; do grep -qx "$a" <<<"$done_apps" || new="$new $a"; done; todo=$new
  [ $n -ge 200 ] && { say "⊘ 200 boots — stopping"; break; }
done
# ---- phase 2: every non-PASS app alone, in a fresh boot ---------------------------------------
if [ "$PER" -gt 1 ] && [ "${APPS_NO_ISOLATE:-0}" != 1 ]; then
  for a in $(grep -a -v 'verdict=PASS' "$R/guest.res" | sed -n 's/.* app=\([^ ]*\) .*/\1/p' | sort -u); do
    mkdir -p "$R/iso"
    boot "ap_${RUN}_i_$a" "$a" "$R/iso"
  done
  [ -f "$R/iso/guest.res" ] && cp "$R/iso/guest.res" "$R/guest_isolated.res"
fi
say "GUEST_DONE batched: $(grep -c 'verdict=PASS' "$R/guest.res") pass / $(wc -l < "$R/guest.res");  isolated re-runs: $(grep -c 'verdict=PASS' "$R/guest_isolated.res" 2>/dev/null || echo 0) pass / $(wc -l < "$R/guest_isolated.res" 2>/dev/null || echo 0)"
