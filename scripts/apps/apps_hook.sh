#!/usr/bin/env bash
# POST_CAPTURE_HOOK for boot_capture.sh (fat guest on kf3): run the apps in $APPS inside the
# guest, one process each, and for EACH app persist
#   $APPS_OUT/<app>.guest.log         the app's own output (from inside the guest)
#   $APPS_OUT/<app>.guest_dmesg.log   the guest dmesg lines this app added (NVRM / Xid)
#   $APPS_OUT/<app>.kf3.log           the kf3 device's stderr lines this app added
# and append its APPRES line (plus KF3/XID counters, the managed-memory verdict `loud=` of
# loud_verdict.sh and the boot's `rc[unarmed= none=]` gate counters) to $APPS_OUT/guest.res.
# Stops at the first app after which the guest no longer answers (verdict GUEST_DEAD);
# apps_matrix.sh reboots and continues with the rest.
# ★ 2026-10-03 (review of 5af7e644): (1) the wedge probe also runs after a PASS row whose guest dmesg
# gained an Xid — `vmm_probe`'s `ro_write` child faults ON PURPOSE and the row still PASSes, and a
# fault is what wedged nb1; (2) every row also carries `rc_silent_births=` (RC-UNARMED/RC-NONE birth
# lines in its own slice) — the per-row attribution of the boot gate apps_matrix.sh enforces
# (boot_gate.sh). `APPS_GSSH` replaces the guest ssh (test_hook.sh drives this hook offline with it).
# ⊘ CORRECTED 2026-10-03 (review of 9390f51c): the PASS-row probe was keyed on the GUEST Xid alone —
# a line that exists only if kf3's OS_ERROR_LOG path works, which no box has run yet (R3's host RCs
# reached the guest with NO Xid line at all). The probe now also follows a PASS row whose own kf3
# slice holds an RC or UNSERVICED-GPU-FAULT line (the host's direct fact that the row faulted).
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G=${APPS_GSSH:-$HERE/../bench/gssh_nv}
OUT=${APPS_OUT:?APPS_OUT}; mkdir -p "$OUT"
QLOG=${BENCH_DIR:-/workspace/bench}/run_${TAG}_qemu.log
RE_KF3_RC='kf3: (RC host twin 0x[0-9a-f]+ |RC_TRIGGERED posted: guest chid 0x|OS_ERROR_LOG posted: guest client 0x|UNSERVICED-GPU-FAULT guest client 0x)'
$G true >/dev/null 2>&1 || { echo "APPS_HOOK guest unreachable at start"; exit 0; }
$G 'sudo tee /opt/apps/bundle/run_apps.sh >/dev/null' < "$HERE/run_apps.sh"
# the tree's python drivers, so a harness fix does not need a re-provisioned image
# host-built probes added after the image was provisioned (same binaries the host lane ran)
# ★ 2026-10-03: plus the release rows' sample files (the image's samples/ predates them): only the ones
# the bundle has, so an older bundle still pushes bin/ cleanly.
extra=$(cd /workspace/apps/bundle 2>/dev/null && ls -d samples/vectorAddMMAP samples/vectorAdd_kernel64.fatbin 2>/dev/null | tr '\n' ' ')
tar -C /workspace/apps/bundle -cf - bin $extra | $G 'sudo tar -C /opt/apps/bundle -xf -'
$G 'test -d /opt/apps/bundle/cuda/include' || tar -C /workspace/apps/bundle -czf - cuda | $G 'sudo tar -C /opt/apps/bundle -xzf -'
for f in "$HERE"/src/*.py; do $G "sudo tee /opt/apps/bundle/share/$(basename "$f") >/dev/null" < "$f"; done
$G 'sudo rm -rf /opt/apps/out/guest'
echo "APPS_HOOK tag=$TAG apps=[$APPS] guest=$($G 'nvidia-smi --query-gpu=name,driver_version,persistence_mode --format=csv,noheader' 2>&1 | head -1)"
# ⊘ boot_capture.sh reloads ONLY `nvidia`; the host has nvidia_modeset + nvidia_drm loaded, and the
# EGL device platform / Vulkan ICD need them (measured r1: eglInitialize failed, "No vulkan device").
# Load them so both sides run the same experiment; APPS_GUEST_DRM=0 reproduces the bare state.
[ "${APPS_GUEST_DRM:-1}" = 1 ] && echo "APPS_GUEST_DRM $($G 'sudo modprobe nvidia_drm; echo rc=$?; lsmod | grep -c ^nvidia' 2>&1 | tr '\n' ' ')"
[ "${APPS_GUEST_PM:-0}" = 1 ] && echo "APPS_GUEST_PM=$($G 'sudo nvidia-smi -pm 1 2>&1 | tail -1')"
for app in $APPS; do
  q0=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
  d0=$($G 'sudo dmesg | wc -l' 2>/dev/null | tr -d '\r'); d0=${d0:-0}
  res=$(timeout "${APPS_APP_TMO:-2400}" "$G" "sudo bash /opt/apps/bundle/run_apps.sh guest $app" 2>&1 | tr -d '\r')
  src=$?
  alive=1; $G true >/dev/null 2>&1 || { sleep 20; $G true >/dev/null 2>&1 || alive=0; }
  line=$(grep -a '^APPRES ' <<<"$res" | tail -1)
  if [ -z "$line" ]; then
    v=HANG; [ $alive = 0 ] && v=GUEST_DEAD
    line="APPRES side=guest app=$app verdict=$v rc=ssh$src secs=- quiet=- note=no-result-line(guest_alive=$alive)"
  fi
  if [ $alive = 1 ]; then
    $G "sudo cat /opt/apps/out/guest/$app.log" > "$OUT/$app.guest.log" 2>&1
    $G "sudo dmesg | tail -n +$((d0+1))" > "$OUT/$app.guest_dmesg.log" 2>&1
  fi
  tail -n +"$((q0+1))" "$QLOG" 2>/dev/null | tail -3000 > "$OUT/$app.kf3.log"
  nx=$(grep -c 'Xid' "$OUT/$app.guest_dmesg.log" 2>/dev/null); nx=${nx:-0}
  # the host side's own record that this row faulted: kf3's RC / UNSERVICED line SHAPES (triage.py
  # RC_LINE) — never the bare words, which kf3's boot sentence also prints
  nrc=$(grep -acE "$RE_KF3_RC" "$OUT/$app.kf3.log" 2>/dev/null); nrc=${nrc:-0}
  nr=$(grep -v 'kf3: family=' "$OUT/$app.kf3.log" 2>/dev/null | grep -ciE 'refus'); nr=${nr:-0}
  nk=$(wc -l < "$OUT/$app.kf3.log")
  # ★ 2026-10-03 (release §I): the managed-memory verdict, and the boot's silent-twin gate — the last
  # `rc[...]` status line in this app's slice (cumulative for the boot; `-` = no status line yet).
  loud=$(bash "$HERE/loud_verdict.sh" "$app" "$OUT/$app.guest.log" "$OUT/$app.guest_dmesg.log" "$OUT/$app.kf3.log" "$line")
  st=$(grep -aoE 'rc\[armed=[0-9]+ unarmed=[0-9]+ none=[0-9]+' "$OUT/$app.kf3.log" 2>/dev/null | tail -1)
  ru=$(sed -n 's/.*unarmed=\([0-9]*\).*/\1/p' <<<"$st"); rn=$(sed -n 's/.*none=\([0-9]*\).*/\1/p' <<<"$st")
  # the birth LINE's shape, never the bare word (kf3's boot-report sentence names RC-UNARMED too)
  rb=$(grep -acE 'kf3: chan 0x[0-9a-f]+:0x[0-9a-f]+ RC-(UNARMED|NONE): ' "$OUT/$app.kf3.log" 2>/dev/null); rb=${rb:-0}
  echo "$line boot=$TAG guest_xid=$nx kf3_lines=$nk kf3_refusals=$nr rc_unarmed=${ru:--} rc_none=${rn:--} rc_silent_births=$rb loud=${loud#LOUD=}" | tee -a "$OUT/guest.res"
  grep -a '^APPDIG ' <<<"$res" | tee -a "$OUT/guest.dig"
  [ $alive = 0 ] && { echo "APPS_HOOK guest dead after $app — stopping this boot"; break; }
  # ⊘ measured nb1 (670bd310, vh): after UnifiedMemoryStreams' fault the boot was WEDGED — every
  # later app burned its whole timeout silently. After any non-PASS row, prove the boot still runs
  # CUDA (vectorAdd, 60 s); if not, record WEDGED and end the boot — apps_matrix.sh reboots and
  # continues with the remaining apps, so one wedge costs one app, not the rest of the list.
  # ★ 2026-10-03: and after ANY row that faulted, PASS or not — a GPU fault is what wedged nb1, and
  # vmm_probe faults on purpose yet PASSes (its rows after it would otherwise burn their timeouts and
  # score SILENT, blamed on the wrong row). "Faulted" = the guest dmesg gained an Xid OR (⊘ review of
  # 9390f51c) the row's kf3 slice holds an RC / UNSERVICED line: either fact alone triggers it.
  probe=0
  case "$line" in *verdict=PASS*) ;; *) probe=1 ;; esac
  [ "$nx" -gt 0 ] 2>/dev/null && probe=1
  [ "$nrc" -gt 0 ] 2>/dev/null && probe=1
  if [ $probe = 1 ] && [ "${APPS_WEDGE_PROBE:-1}" = 1 ]; then
    sane=$(timeout 90 "$G" 'sudo timeout -k 5 60 /opt/apps/bundle/samples/vectorAdd 2>&1 | grep -c "Test PASSED"' 2>/dev/null | tr -d '\r')
    echo "APPS_HOOK wedge probe after $app (guest_xid=$nx kf3_rc=$nrc): sanity_vectorAdd=${sane:-none}"
    if [ "${sane:-0}" != 1 ]; then
      echo "APPS_WEDGE boot=$TAG after=$app sanity_vectorAdd=${sane:-none}" | tee -a "$OUT/guest.res"
      echo "APPS_HOOK boot wedged after $app — stopping this boot"; break
    fi
  fi
done
echo "APPS_HOOK_DONE"
