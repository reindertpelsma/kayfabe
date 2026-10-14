#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# die_run.sh phase1|phase2 — the two boots of the die sweep (docs/design/V3_DIE_SWEEP_20261014.md), ON the box.
#   phase1: fresh overlay; wait for QGA; census + EnableGpuFirmware on every NVIDIA display function +
#           autologon + Startup nvidia-smi (phase1.ps1); evidence; clean shutdown.
#   phase2: the same overlay in a NEW QEMU process; wait for QGA; let autologon reach the desktop;
#           evidence + screenshots every 30 s for PHASE2_SECONDS (default 300); the guest keeps running.
# Log: $RUN/<phase>.log with a start marker and an EXIT line (CLAUDE.md: a killed job looks like a running one).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
W=${WIN_DIR:-/root/win}; RUN=${WIN_RUN:-$W/run}; P=${1:?phase1|phase2}
mkdir -p -m 0700 "$RUN" "$W/secrets"
exec >> "$RUN/$P.log" 2>&1
echo "DIE_RUN_START $P $(date -Is)"
WD="$HERE/win_die.sh"
trap 'echo "DIE_RUN_EXIT $P rc=$? $(date -Is)"' EXIT
case "$P" in
phase1)
    [ -s "$W/secrets/pw" ] || { openssl rand -hex 9 > "$W/secrets/pw"; chmod 0600 "$W/secrets/pw"; }
    sed "s/@PW@/$(cat "$W/secrets/pw")/" "$HERE/phase1.ps1.in" > "$W/secrets/phase1.ps1"; chmod 0600 "$W/secrets/phase1.ps1"
    bash "$WD" start || exit 2
    ( for t in 30 60 90 120 180 240 300; do sleep 30; bash "$WD" shot "p1_t$(date +%H%M%S)" > /dev/null 2>&1; done ) &
    bash "$WD" wait-qga "${QGA_WAIT:-1200}" || { bash "$WD" shot p1_noqga; exit 3; }
    sleep 45
    bash "$WD" shot p1_qga
    bash "$WD" ps "$HERE/evidence.ps1" 240 | sed 's/^/P1 /'
    nvidia-smi -q -d MEMORY | sed -n '/BAR1 Memory Usage/,/Free/p' | tr -s ' ' | tr '\n' ' ' | sed 's/^/HOSTBAR1 p1 /'; echo
    bash "$WD" ps "$W/secrets/phase1.ps1" 240
    # D9: a new die's instance may still be driverless here (GB206: CM_PROB_REINSTALL) — bind + flag in the same boot
    bash "$WD" ps "$HERE/drvbind.ps1" 400
    bash "$WD" stop ;;
bind)
    # a new die whose device PnP left driverless (GB206: CM_PROB_REINSTALL): bind the image's own package, flag the new key
    bash "$WD" start || exit 2
    bash "$WD" wait-qga "${QGA_WAIT:-1200}" || { bash "$WD" shot bind_noqga; exit 3; }
    sleep 30
    bash "$WD" ps "$HERE/drvbind.ps1" 400
    bash "$WD" ps "$HERE/evidence.ps1" 240 | sed 's/^/PB /'
    bash "$WD" shot bind_end
    bash "$WD" stop ;;
probe)
    # one diagnostic boot of the same overlay (e.g. WIN_FLAGS="KF3_BAR0_TRACE KF3_BAR0_READ_TRACE KF3_RPC_TRACE"); then stop
    bash "$WD" start || exit 2
    bash "$WD" wait-qga "${QGA_WAIT:-1200}" || { bash "$WD" shot probe_noqga; exit 3; }
    sleep "${PROBE_SECONDS:-90}"
    bash "$WD" ps "$HERE/evidence.ps1" 240 | sed 's/^/PR /'
    [ -n "${PROBE_PS:-}" ] && bash "$WD" ps "$PROBE_PS" 240 | sed 's/^/PD /'
    bash "$WD" shot probe_end
    bash "$WD" stop ;;
phase2)
    bash "$WD" start || exit 2
    bash "$WD" wait-qga "${QGA_WAIT:-1200}" || { bash "$WD" shot p2_noqga; exit 3; }
    end=$(( $(date +%s) + ${PHASE2_SECONDS:-300} )); i=0
    while [ "$(date +%s)" -lt "$end" ]; do
        sleep 30; i=$((i + 30)); bash "$WD" shot "p2_t$(printf %04d $i)"
        [ $((i % 90)) = 0 ] && bash "$WD" ps "$HERE/evidence.ps1" 240 | sed "s/^/P2T$i /"
        # the HOST's BAR1 while the guest runs (the owner's question: does the non-resizable 256 MiB BAR1 matter?)
        nvidia-smi -q -d MEMORY | sed -n '/BAR1 Memory Usage/,/Free/p' | tr -s ' ' | tr '\n' ' ' | sed "s/^/HOSTBAR1 t=$i /"; echo
    done
    bash "$WD" ps "$HERE/evidence.ps1" 240 | sed 's/^/P2END /'
    bash "$WD" shot p2_final ;;
*) echo "usage: $0 phase1|phase2"; exit 2 ;;
esac
