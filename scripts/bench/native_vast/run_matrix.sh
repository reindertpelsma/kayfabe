#!/bin/bash
# run_matrix.sh OUTDIR : on the dev box. Runs warm-up + 3 interleaved D3D11/OpenGL Heaven benchmarks on the Windows box (ssh alias via W),
# fetching each run directory (text, PNG screenshots, dmon) into OUTDIR/<label>/ after the run. W defaults to /tmp/claude-0/w.sh (ssh wrapper).
# Start/exit markers go to OUTDIR/matrix.log. Order: warm(d3d11) d1 g1 d2 g2 d3 g3.
set -u
OUT=${1:?outdir}; W=${W:-/tmp/claude-0/w.sh}; SSHCFG=${SSHCFG:-/tmp/claude-0/ssh_win.cfg}; mkdir -p "$OUT"
log() { echo "$(date -u +%FT%TZ) $*" | tee -a "$OUT/matrix.log"; }
log "MATRIX_START"
for step in "warm direct3d11" "d1 direct3d11" "g1 opengl" "d2 direct3d11" "g2 opengl" "d3 direct3d11" "g3 opengl"; do
  set -- $step; lab=$1; api=$2
  log "RUN_START $lab $api"
  $W "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\bench\\launch_run.ps1 -Api $api -Label $lab" > "$OUT/$lab.launch.txt" 2>&1
  log "RUN_EXIT $lab rc=$? $(grep -a -E 'LAUNCH_RUN|HLOG.*FPS' $OUT/$lab.launch.txt | tr -d '\r' | head -2 | tr '\n' ' ' | cut -c1-200)"
  mkdir -p "$OUT/$lab"; scp -q -r -F "$SSHCFG" "nvw:C:/bench/runs/$lab/*" "$OUT/$lab/" 2>>"$OUT/matrix.log" || log "SCP_FAIL $lab"
done
log "MATRIX_END"
