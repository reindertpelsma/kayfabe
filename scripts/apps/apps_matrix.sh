#!/usr/bin/env bash
# apps_matrix.sh — drive the V3 app matrix on a bench box (strictly serial: one GPU job at a time).
#   apps_matrix.sh host  <run> <app|all>...   bare-metal baseline on this box (as root)
#   apps_matrix.sh guest <run> <app|all>...   the same apps inside the kf3 fat guest; batched per
#                                             boot, continuing in a fresh boot after a GUEST_DEAD,
#                                             then every non-PASS app is RE-RUN ALONE in its own
#                                             fresh boot (so a failure is not contamination from
#                                             an earlier app in the same boot).
#   apps_matrix.sh lane  <run>                the guest lane's boot-gate verdict over the results
#                                             already recorded (what `guest` exits on); exit 0/3
# env: KF3_BIN (kf3 qemu binary; default: the newest in $BENCH_DIR/kf3-bins),
#      NVKVM_RAM_MB (16384), KF_SMP (6), KF3_FB_MB, APPS_GUEST_PM (0/1),
#      APPS_RESULTS (/workspace/apps/results), BENCH_DIR (/workspace/bench, as boot_capture.sh)
# results: $APPS_RESULTS/<run>/{host.res,guest.res,guest_isolated.res,<app>.*.log}
# ★ 2026-10-03 (release §I, V3_APP_MATRIX.md §R5.3): every guest boot is GATED — after QEMU exits,
#   boot_gate.sh reads the boot's whole kf3 log and appends
#     APPS_BOOT_GATE boot=<tag> gate=PASS|FAIL|UNMEASURED unarmed= none= births= why=
#   to the boot's guest.res. A silent twin (RC-UNARMED / RC-NONE) under ANY row fails the gate, and
#   any boot that is not gate=PASS FAILS THE LANE: `guest` exits 3 after GUEST_DONE. summarize.py
#   prints the same verdict (BOOT_GATE … lane=).
# ⊘ CORRECTED 2026-10-03 (review of 9390f51c): (1) this script counted EVERY gate line in the
#   appended guest.res while summarize.py kept the LAST per boot, so a reused run name could exit 3
#   here and print lane=PASS there. The verdict is now boot_gates.py's — one rule, one
#   implementation, read by this script (`lane`, and the end of `guest`), summarize.py and
#   triage.py. (2) boot() removes the tag's old bench logs and result copies BEFORE booting:
#   boot_capture.sh can die before it truncates them, so the gate could read an EARLIER boot's kf3
#   log as this one's. (3) Which apps a boot completed (and which failed, for phase 2) is read from
#   the lines THIS invocation appended: an earlier attempt's row at the same tag no longer marks an
#   app done that this attempt never ran.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
SIDE=${1:?host|guest|lane}; RUN=${2:?run}; shift 2
R=${APPS_RESULTS:-/workspace/apps/results}/$RUN
BENCH=${BENCH_DIR:-/workspace/bench}
say(){ echo "[apps_matrix $(date -Is)] $*"; }
# the lane's boot-gate verdict: boot_gates.py, the ONE rule (summarize.py and triage.py call it too)
lane(){ python3 "$HERE/boot_gates.py" "$R"; }
if [ "$SIDE" = lane ]; then lane; exit $?; fi
mkdir -p "$R"
ALL=$(bash "$HERE/run_apps.sh" x list)
[ "${1:-}" = all ] && set -- $ALL
say "START side=$SIDE run=$RUN rev=$(git -C "$REPO" rev-parse --short=8 HEAD) apps=$#"
busy(){ pgrep -x qemu-system-x86 >/dev/null || pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; }
while busy; do say "waiting: a QEMU/cargo is running (serial bench)"; sleep 20; done

if [ "$SIDE" = host ]; then
  mkdir -p /opt/apps
  cp -f "$HERE"/src/*.py /opt/apps/bundle/share/
  exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9   # the host GPU is the bench GPU
  bash "$HERE/run_apps.sh" host "$@" | tee -a "$R/host.res"
  flock -u 9
  for f in /opt/apps/out/host/*.log; do cp -f "$f" "$R/$(basename "$f" .log).host.log"; done
  dmesg | grep -E 'Xid|NVRM' | tail -50 > "$R/host_dmesg_tail.log"
  say "HOST_DONE $(grep -c 'verdict=PASS' "$R/host.res") pass / $(wc -l < "$R/host.res") rows"
  exit 0
fi

QB=${KF3_BIN:-$(ls -td "$BENCH"/kf3-bins/*/qemu-system-x86_64 | head -1)}
[ -x "$QB" ] || { say "⊘ no kf3 binary"; exit 2; }
say "kf3 binary: $QB"
boot(){  # $1 tag, $2 apps, $3 outdir
  # ★ serialize each boot on the shared bench lock (held per boot, released between boots)
  exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
  # ⊘ every file read back below must be THIS boot's: drop the tag's old bench logs and result
  # copies first (a reused run name, or a boot_capture.sh that dies before it truncates them, would
  # otherwise hand the gate an earlier boot's kf3 log — and it would PASS on it)
  for x in qemu dmesg dmesg_after probe hostdmesg serial; do rm -f "$BENCH/run_$1_$x.log" "$3/boot_$1.$x.log"; done
  rm -f "$3/boot_$1.qemu.log.zst"
  env KF_DEVICE=kf3 QEMU_BIN="$QB" NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} KF_SMP=${KF_SMP:-6} GQ_TIMEOUT=300 \
      APPS="$2" APPS_OUT="$3" POST_CAPTURE_HOOK="$HERE/apps_hook.sh" \
      bash "$REPO/scripts/bench/boot_capture.sh" "$1" > "$3/boot_$1.driver.log" 2>&1
  local rc=$?
  flock -u 9; exec 9>&-
  for x in dmesg dmesg_after probe hostdmesg serial; do cp -f "$BENCH/run_$1_$x.log" "$3/boot_$1.$x.log" 2>/dev/null; done
  zstd -q -f "$BENCH/run_$1_qemu.log" -o "$3/boot_$1.qemu.log.zst" 2>/dev/null
  # ★ the boot's silent-twin gate, over the COMPLETE kf3 log (QEMU has exited)
  echo "APPS_BOOT_GATE boot=$1 $(bash "$HERE/boot_gate.sh" "$BENCH/run_$1_qemu.log" | sed 's/^GATE=/gate=/')" | tee -a "$3/guest.res"
  say "boot $1 rc=$rc apps=[$2] :: $(grep -a 'APPS_HOOK\|FAILED' "$3/boot_$1.driver.log" "$BENCH/run_$1_probe.log" 2>/dev/null | tail -2 | tr '\n' ' ' | cut -c1-200)"
  while busy; do sleep 3; done
}
# APPS_PER_BOOT (default 1): apps per fresh boot. ⊘ Measured r1 on va1 (2026-09-26): in ONE boot,
# the 3rd CUDA process wedged and every later app timed out silently (kf3: "slot 27 is full and
# nothing can be retired — raise WalkCfg::runs_per_pdb") — so a batched verdict is contaminated by
# the apps before it. One app per boot is the clean measurement; batching is a separate experiment.
PER=${APPS_PER_BOOT:-1}
# ⊘ (review of 9390f51c) guest.res is APPENDED to, and a reused run name repeats the boot tags: the
# bookkeeping of THIS invocation (which apps a boot completed, which failed) reads only the lines it
# appended. (The lane verdict reads every attempt, by boot_gates.py's last-line-per-boot rule.)
G0=0; [ -f "$R/guest.res" ] && G0=$(wc -l < "$R/guest.res")
mine(){ tail -n +"$((G0+1))" "$R/guest.res" 2>/dev/null; }
# ---- phase 1: batched --------------------------------------------------------------------------
todo="$*"; n=0
while [ -n "$(echo $todo)" ]; do
  n=$((n+1)); tag="ap_${RUN}_b$n"
  batch=$(echo $todo | cut -d' ' -f1-"$PER")
  boot "$tag" "$batch" "$R"
  done_apps=$(mine | grep -a "boot=$tag " | sed -n 's/.* app=\([^ ]*\) .*/\1/p')
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
  for a in $(mine | grep -a -v 'verdict=PASS' | sed -n 's/.* app=\([^ ]*\) .*/\1/p' | sort -u); do
    mkdir -p "$R/iso"
    boot "ap_${RUN}_i_$a" "$a" "$R/iso"
  done
  [ -f "$R/iso/guest.res" ] && cp "$R/iso/guest.res" "$R/guest_isolated.res"
fi
say "GUEST_DONE batched: $(mine | grep -c '^APPRES .*verdict=PASS') pass / $(mine | grep -c '^APPRES ');  isolated re-runs: $(grep -c '^APPRES .*verdict=PASS' "$R/guest_isolated.res" 2>/dev/null || echo 0) pass / $(grep -c '^APPRES ' "$R/guest_isolated.res" 2>/dev/null || echo 0)"
# ★ 2026-10-03: the lane fails on any boot whose silent-twin gate is not PASS (FAIL or UNMEASURED) —
# by boot_gates.py's rule (the last gate line per boot), the verdict summarize.py prints.
gv=$(lane); grc=$?
say "$(head -1 <<<"$gv")"; tail -n +2 <<<"$gv"
if [ "$grc" != 0 ]; then
  say "⊘ LANE FAIL: a boot not gate=PASS — a silent twin (RC-UNARMED/RC-NONE) or an unmeasured gate (rc=$grc)"
  exit 3
fi
