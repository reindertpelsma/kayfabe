#!/usr/bin/env bash
# regress_matrix.sh — the Win11 app regression matrix on VM 2990, one command, one results table.
#
# Trusted host only (172.22.1.20). Launches VM 2990 on kf3 REV (production profile, no KF3_ measurement
# flag unless TRACE_DIR is set), runs the matrix through the kfagent channel, reboots the GUEST after each app
# so the USER-ring event loop prints its counters (`USER_RING_LOOP SUMMARY (all relays gone …)`, cumulative
# per QEMU process: the table shows each app's line), and writes a dated Markdown table.
#
#   REV=<kf3 rev> [APPS="heaven_d3d11_win …"] [TRACE_DIR=<dir>] [OUT=<file>] bash regress_matrix.sh
#
# Apps (default all, in this order): heaven_d3d11_win heaven_d3d11_fs heaven_gl_win heaven_gl_fs gm_d3d12 gm_vk
#   minecraft edge idle. Pass bar per app: 0 host Xid (this qemu pid), 0 GSP resets during the app, 0 new guest
#   nvlddmkm / dxgkrnl-4101 events, the app's own completion (benchmarks), the window evidence (Minecraft,
#   Edge: playback CONFIRMED by two screenshots differing in the video area).
# In-guest helpers: C:\kf\apps\{heaven,gm}.ps1 (scripts/bench/windows/kfagent_ogl/), C:\kf\ogl\runi.ps1;
# installers in C:\kf\apps (sha256 in traces/win11_userring_20261014/README.md).
# Traps: never pkill a pattern from this script's own command line; stop the VM gracefully (shutdown via
# kfagent, 60 s, then SIGTERM the exact qemu pid); our own pids only.
set -uo pipefail
REV=${REV:?REV=<kf3 rev under /var/lib/kf-windows-20261005/kf3-bins>}
W=/var/lib/kf-windows-20261005; R=$W/boundary-kayfabe-2990; STATE=$W/windows-broker-run2990.state
HERE=$(cd "$(dirname "$0")" && pwd)
LAUNCH=${LAUNCH:-/data/claude-urelay/launch-2990.sh}
KFAGENT=${KFAGENT:-/data/kfagent_server.py}
APPS=${APPS:-"heaven_d3d11_win heaven_d3d11_fs heaven_gl_win heaven_gl_fs gm_d3d12 gm_vk minecraft edge idle"}
EDGE_MIN=${EDGE_MIN:-20}; IDLE_MIN=${IDLE_MIN:-30}; MC_MIN=${MC_MIN:-6}
STAMP=$(date +%Y%m%d-%H%M)
WORK=${WORK:-/data/claude-urelay/matrix-$REV-$STAMP}; mkdir -p "$WORK"
OUT=${OUT:-$WORK/matrix-$REV-$STAMP.md}
LOG=$WORK/runner.log
say() { echo "$(date +%T) $*" | tee -a "$LOG"; }
ka() { timeout "${2:-40}" python3 "$KFAGENT" run "$1" $((${2:-40} - 5)) 2>/dev/null; }
qpid() { sed -n 's/^QPID=//p' "$STATE"; }
qmp() { # qmp.py-free QMP: one command per call
  python3 - "$R/qmp.sock" "$@" <<'PY'
import socket, json, sys, time
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile("rw"); f.readline()
def q(c, a=None):
    f.write(json.dumps({"execute": c, **({"arguments": a} if a else {})}) + "\n"); f.flush()
    while True:
        l = json.loads(f.readline())
        if "return" in l or "error" in l: return l
q("qmp_capabilities")
op, args = sys.argv[2], sys.argv[3:]
if op == "shot":
    q("screendump", {"filename": args[0] + ".ppm"})
    from PIL import Image
    Image.open(args[0] + ".ppm").convert("RGB").resize((960, 540)).save(args[0])
elif op == "click":
    x, y = int(float(args[0]) * 32767), int(float(args[1]) * 32767)
    q("input-send-event", {"events": [{"type": "abs", "data": {"axis": "x", "value": x}}, {"type": "abs", "data": {"axis": "y", "value": y}}]})
    time.sleep(0.3)
    for down in (True, False):
        q("input-send-event", {"events": [{"type": "btn", "data": {"button": "left", "down": down}}]}); time.sleep(0.15)
elif op == "keys":
    for i in range(0, len(args), 2):
        for down in (True, False):
            q("input-send-event", {"events": [{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": args[i]}}}]})
            if down: time.sleep(float(args[i + 1]))
        time.sleep(0.3)
PY
}
differ() { # differ a.png b.png -> "moving" when the video area changed
  python3 - "$1" "$2" <<'PY'
import sys
from PIL import Image, ImageChops
a = Image.open(sys.argv[1]).convert("L").crop((80, 100, 490, 330))
b = Image.open(sys.argv[2]).convert("L").crop((80, 100, 490, 330))
print("moving" if ImageChops.difference(a, b).getbbox() else "still")
PY
}
since=""   # journal start for this qemu
xid() { journalctl -k --since "$since" | grep -c "pid=$(qpid)" ; }
resets() { grep -c "GSP phase Running -> Suspending" "$R/qemu.log"; }
refused906f() { grep -c "REFUSED fn76/0x906f0102\|cmd=0x906f0102" "$R/qemu.log"; }
guest_events() { ka "\$b=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime; @(Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=\$b} -ErrorAction SilentlyContinue | Where-Object { \$_.ProviderName -eq 'nvlddmkm' -or (\$_.ProviderName -match 'Display' -and \$_.Id -eq 4101) }).Count" 60 | tr -d '\r' | tail -1; }
wait_agent() { timeout 400 bash -c "until timeout 20 python3 $KFAGENT run hostname 15 2>/dev/null | grep -q kayfabe; do sleep 10; done"; }
summaries() { grep -c "USER_RING_LOOP SUMMARY" "$R/qemu.log"; }

launch() {
  say "LAUNCH REV=$REV trace=${TRACE_DIR:-off}"
  local flags=""; [ -n "${TRACE_DIR:-}" ] && { mkdir -p "$TRACE_DIR"; flags="KF3_USER_RING_TRACE=$TRACE_DIR"; }
  REV=$REV OGL_FLAGS="$flags" nohup flock -w 60 -o /tmp/kayfabe-fastguest.lock bash "$LAUNCH" > "$WORK/launch.log" 2>&1 &
  sleep 15; since=$(date +%T); say "qemu pid $(qpid)"; sleep 30; wait_agent; sleep 40; say "guest up"
}
graceful_stop() {
  local p; p=$(qpid); ka "shutdown /s /t 0 /f" 20 >/dev/null
  timeout 70 bash -c "while kill -0 $p 2>/dev/null; do sleep 2; done"
  kill -0 "$p" 2>/dev/null && { say "SIGTERM $p"; kill -TERM "$p"; sleep 8; }
  cp "$R/qemu.log" "$WORK/qemu-final.log"
}
reboot_guest() { # the guest unloads its driver: every relay goes, the loop prints its counters
  local n0; n0=$(summaries); ka "shutdown /r /t 0 /f" 20 >/dev/null
  timeout 180 bash -c "until [ \$(grep -c 'USER_RING_LOOP SUMMARY' $R/qemu.log) -gt $n0 ]; do sleep 3; done"
  grep "USER_RING_LOOP SUMMARY" "$R/qemu.log" | tail -1 | cut -c1-900
  sleep 30; wait_agent; sleep 40
}

row() { echo "| $1 | $2 | $3 | $4 | $5 | $6 | $7 |" >> "$OUT"; }
bench() { # tag ps1 args…
  local tag=$1; shift; local r0 x0; r0=$(resets); x0=$(xid)
  say "APP $tag"
  ka "& C:\\kf\\ogl\\runi.ps1 -Name $tag -Cmd \"powershell -ExecutionPolicy Bypass -File $* -Tag $tag\"" 50 >/dev/null
  sleep 60; qmp shot "$WORK/$tag.png"
  timeout 900 bash -c "until timeout 20 python3 $KFAGENT run \"Get-Content C:\\kf\\apps\\out\\$tag.txt\" 15 | grep -q 'END\|TIMEOUT'; do sleep 20; done"
  local res; res=$(ka "Get-Content C:\\kf\\apps\\out\\$tag.stdout.txt | Select-String 'Phoronix finished|FPS:|Score:' | ForEach-Object { \$_.Line.Trim() -replace '^M:\\s+[0-9:.]+:\\s+','' }" 30 | tr -d '\r' | tr '\n' ' ')
  local done=FAIL; echo "$res" | grep -q "Phoronix finished\|Score:" && done=PASS
  local dr=$(( $(resets) - r0 )) dx=$(( $(xid) - x0 )) ev; ev=$(guest_events)
  [ "$dr" -ne 0 ] || [ "$dx" -ne 0 ] || [ "${ev:-0}" -ne 0 ] && done=FAIL
  local sum; sum=$(reboot_guest)
  row "$tag" "$done" "$res" "$dx" "$dr" "$ev" "\`${sum#kf3: }\`"
}
minecraft() {
  local r0 x0; r0=$(resets); x0=$(xid); say "APP minecraft"
  ka "& C:\\kf\\ogl\\runi.ps1 -Name mcl -Cmd \"start \"\"\"\" \"\"C:\\XboxGames\\Minecraft Launcher\\Content\\Minecraft.exe\"\"\"" 50 >/dev/null
  sleep 45; qmp click 0.552 0.698; sleep 55; qmp shot "$WORK/mc_title.png"
  qmp click 0.498 0.495; sleep 8; qmp click 0.45 0.40; sleep 1; qmp click 0.418 0.643; sleep 45
  qmp shot "$WORK/mc_world.png"
  local t=0; while [ $t -lt $((MC_MIN * 60)) ]; do qmp keys w 10 a 2 w 10 d 2 spc 1; sleep 30; t=$((t + 56)); done
  qmp shot "$WORK/mc_end.png"
  local a b; qmp shot "$WORK/mc_a.png"; sleep 3; qmp shot "$WORK/mc_b.png"
  ka "Get-Process javaw,Minecraft -ErrorAction SilentlyContinue | Stop-Process -Force" 30 >/dev/null
  local dr=$(( $(resets) - r0 )) dx=$(( $(xid) - x0 )) ev; ev=$(guest_events)
  local ok=PASS; [ "$dr" -ne 0 ] || [ "$dx" -ne 0 ] || [ "${ev:-0}" -ne 0 ] && ok=FAIL
  local sum; sum=$(reboot_guest)
  local mv; mv=$(differ "$WORK/mc_a.png" "$WORK/mc_b.png")
  row minecraft "$ok" "launcher → Play → Singleplayer → world, ${MC_MIN} min walking (mc_world.png, mc_end.png; frames $mv at the end)" "$dx" "$dr" "$ev" "\`${sum#kf3: }\`"
}
edge() {
  local r0 x0; r0=$(resets); x0=$(xid); say "APP edge"
  ka "& C:\\kf\\ogl\\runi.ps1 -Name edge -Cmd \"start msedge https://www.youtube.com/watch?v=aqz-KE-bpKQ\"" 50 >/dev/null
  sleep 30; qmp shot "$WORK/edge_a.png"; sleep 4; qmp shot "$WORK/edge_b.png"
  local st; st=$(differ "$WORK/edge_a.png" "$WORK/edge_b.png")
  if [ "$st" != moving ]; then qmp click 0.30 0.40; sleep 6; qmp shot "$WORK/edge_a.png"; sleep 4; qmp shot "$WORK/edge_b.png"; st=$(differ "$WORK/edge_a.png" "$WORK/edge_b.png"); fi
  local checks="start:$st" t=0
  while [ $t -lt $((EDGE_MIN * 60)) ]; do
    sleep 296; t=$((t + 300)); qmp shot "$WORK/edge_c.png"; sleep 4; qmp shot "$WORK/edge_d.png"
    checks="$checks $((t / 60))m:$(differ "$WORK/edge_c.png" "$WORK/edge_d.png")"
  done
  cp "$WORK/edge_d.png" "$WORK/edge_end.png"
  ka "Get-Process msedge -ErrorAction SilentlyContinue | Stop-Process -Force" 30 >/dev/null
  local dr=$(( $(resets) - r0 )) dx=$(( $(xid) - x0 )) ev; ev=$(guest_events)
  local ok=PASS; [ "$dr" -ne 0 ] || [ "$dx" -ne 0 ] || [ "${ev:-0}" -ne 0 ] && ok=FAIL
  echo "$checks" | grep -q "start:moving" || ok="FAIL(no playback)"
  local sum; sum=$(reboot_guest)
  row edge "$ok" "Big Buck Bunny 4K60 ${EDGE_MIN} min; video area per 5 min: $checks (the 10:34 film ends; 'still' after its end is the end screen)" "$dx" "$dr" "$ev" "\`${sum#kf3: }\`"
}
idle() {
  local r0 x0; r0=$(resets); x0=$(xid); say "APP idle"
  sleep $((IDLE_MIN * 60)); qmp shot "$WORK/idle_end.png"
  local dr=$(( $(resets) - r0 )) dx=$(( $(xid) - x0 )) ev; ev=$(guest_events)
  local ok=PASS; [ "$dr" -ne 0 ] || [ "$dx" -ne 0 ] || [ "${ev:-0}" -ne 0 ] && ok=FAIL
  ka hostname 20 | grep -q kayfabe || ok="FAIL(agent)"
  row idle "$ok" "${IDLE_MIN} min at the desktop" "$dx" "$dr" "$ev" "-"
}

{
  echo "# Win11 regression matrix — kf3 \`$REV\`, $(date -Is)"
  echo
  echo "VM 2990 (RTX 4070 AD104 host, host driver 595.91.07, guest 580.88), production profile$([ -n "${TRACE_DIR:-}" ] && echo ", KF3_USER_RING_TRACE=$TRACE_DIR")."
  echo "Pass = the app's own completion + 0 host Xid (this qemu) + 0 GSP resets during the app + 0 guest nvlddmkm/4101 events since the guest's boot."
  echo
  echo "| app | verdict | result (the app's own numbers) | host Xid | GSP resets | guest nvlddmkm | USER_RING_LOOP SUMMARY after the app (cumulative per QEMU) |"
  echo "|---|---|---|---|---|---|---|"
} > "$OUT"
if [ -n "$(qpid)" ] && kill -0 "$(qpid)" 2>/dev/null; then say "stopping the running VM $(qpid) gracefully"; graceful_stop; sleep 5; fi
launch
for a in $APPS; do
  case $a in
    heaven_d3d11_win) bench "$a" "C:\\kf\\apps\\heaven.ps1 -Api direct3d11 -Fs 0" ;;
    heaven_d3d11_fs) bench "$a" "C:\\kf\\apps\\heaven.ps1 -Api direct3d11 -Fs 1" ;;
    heaven_gl_win) bench "$a" "C:\\kf\\apps\\heaven.ps1 -Api opengl -Fs 0" ;;
    heaven_gl_fs) bench "$a" "C:\\kf\\apps\\heaven.ps1 -Api opengl -Fs 1" ;;
    gm_d3d12) bench "$a" "C:\\kf\\apps\\gm.ps1 -Api d3d12" ;;
    gm_vk) bench "$a" "C:\\kf\\apps\\gm.ps1 -Api vk" ;;
    minecraft) minecraft ;;
    edge) edge ;;
    idle) idle ;;
  esac
done
echo >> "$OUT"; echo "Runner: scripts/bench/windows/regress_matrix.sh; work dir $WORK; the VM stays up (production) at the desktop." >> "$OUT"
say "MATRIX_DONE $OUT"
