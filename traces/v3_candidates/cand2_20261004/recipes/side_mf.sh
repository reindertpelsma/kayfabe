#!/bin/bash
# side_mf.sh <tag> <repo> — beside broker_lane.sh run (BRK_HOLD set): during the hook's hold, the
# v3-maxfps box stage's own steps (V3_DISPLAY.md §8.13/§8.16): the EDID the guest reads; D2 with the
# broker attached (idle, then an animated front buffer); the console cursor across grab->hover and
# broker reconnects under a timeline VNC client (vnc_watch.py); a real VNC viewer's cursor (TigerVNC);
# `info mice`; then D2 with NOBODY watching and a screendump. START/EXIT lines; every step a SIDE_* line.
TAG=$1; REPO=${2:-/root/wt-mf}
G=$REPO/scripts/bench/gssh_nv; XC=$REPO/scripts/bench/display/xcursor.py
P=/workspace/bench/run_${TAG}_probe.log; Q=/workspace/bench/run_${TAG}_qemu.log; MON=/workspace/bench/run_${TAG}.mon
EV=/root/mf/ev/$TAG; mkdir -p "${EV:?}"
exec > /root/mf/side_$TAG.log 2>&1
say(){ echo "SIDE_$* [$(date +%T.%N | cut -c1-12)]"; }
mark(){ echo "$(date +%s.%N | cut -c1-14) $*" >> "$EV/marks.txt"; say "MARK $*"; }
say "START $TAG repo=$(git -C $REPO rev-parse --short=8 HEAD) $(date -Is)"
for i in $(seq 1200); do grep -aq 'HOLD up to' $P 2>/dev/null && break; sleep 1; done
say "HOLD_SEEN after ${i}s"
gq(){ timeout ${2:-60} $G "$1" 2>&1 | tr -d '\r'; }
GX='sudo -u ubuntu env DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority'
p=$(pgrep -o -x plasmashell); sv(){ tr '\0' '\n' < /proc/$p/environ | sed -n "s/^$1=//p"; }
XD=$(sv DISPLAY); XA=$(sv XAUTHORITY); SU=$(stat -c %U /proc/$p)
HX(){ timeout ${HXT:-20} env DISPLAY="$XD" XAUTHORITY="$XA" "$@"; }
SOCK=/run/user/$(id -u $SU)/nvkvm/display.sock; B=/opt/nvkvm-broker/nvkvm-display-broker
fpsl(){ grep -a 'kf3: display fps\[' $Q | tail -1 | sed 's/^.*kf3: display //'; }
num(){ printf '%s\n' "$1" | grep -o " $2=[0-9.]*" | head -1 | cut -d= -f2; }
qmp(){ python3 - "$MON" "$1" <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(10)
time.sleep(0.2); s.recv(65536); s.sendall((sys.argv[2] + "\n").encode()); time.sleep(1.5)
print(s.recv(65536).decode(errors="replace").replace("\r", ""))
PY
}
shot(){ python3 /root/mf/qmp_shot.py "${MON%.mon}.qmp" "$1"; }
# A. the EDID the guest reads
gq 'for e in /sys/class/drm/card*-*/edid; do echo "EDID_FILE $e bytes=$(cat $e | wc -c) status=$(cat $(dirname $e)/status) sha256=$(sha256sum < $e | cut -c1-64)"; done' > "$EV/edid_list.txt"
sed 's/^/SIDE_/' "$EV/edid_list.txt"
for e in $(awk '$3 != "bytes=0" {print $2}' "$EV/edid_list.txt"); do
  n=$(basename $(dirname $e)); gq "xxd -p $e" > "$EV/edid_$n.hex"
done
gq 'sudo modetest -M nvidia-drm -c 2>&1 | head -60' > "$EV/modetest_c.txt"
gq "$GX xrandr 2>&1 | head -12" > "$EV/xrandr.txt"
say "XRANDR $(grep -E '\*|connected' "$EV/xrandr.txt" | tr -s ' ' | tr '\n' '|' | cut -c1-300)"
grep -a 'kf3: display: monitor\|EDID fnv\|period [0-9]* ns' $Q | head -8 | cut -c1-240 | sed 's/^/SIDE_QLOG /'
gq 'for t in xdotool xterm glxgears vkcube xsetroot import python3; do printf "%s=%s " $t $(command -v $t >/dev/null && echo y || echo n); done; echo' > "$EV/guest_tools.txt"
say "GUEST_TOOLS $(cat "$EV/guest_tools.txt")"
W=$(HX xdotool search --class '^nvkvm-display-broker$' 2>/dev/null | head -1)
eval "$(HX xdotool getwindowgeometry --shell "$W" 2>/dev/null)"
say "WINDOW $W ${WIDTH}x${HEIGHT}+${X}+${Y}"
gq "$GX xsetroot -cursor_name crosshair"
HX xdotool mousemove --window "$W" 70 60; sleep 1; HX xdotool mousemove --window "$W" 80 66; sleep 2
gp=$(gq "$GX python3 ~/display/xcursor.py pointer" | sed -n 's/^POINTER //p'); set -- $gp; gx=${1:-80}; gy=${2:-66}
fr=$(grep -ao 'guest resolution is now [0-9]*x[0-9]*' $Q | tail -1 | grep -o '[0-9]*x[0-9]*')
say "GUEST_POINTER $gx,$gy frame=$fr"
# B. D2 with the broker attached: idle, then an animated front buffer (glxgears, X11, no compositor flip)
sleep 6; f0=$(fpsl); t0=$(date +%s.%N); sleep 20; f1=$(fpsl); t1=$(date +%s.%N)
say "D2_IDLE_BROKER start=[$f0]"
say "D2_IDLE_BROKER end=[$f1] same_delta=$(( $(num "$f1" same) - $(num "$f0" same) )) over_s=$(echo "$t1 - $t0" | bc)"
mark animated_start
gq "$GX sh -c 'nohup xterm -geometry 60x12+300+260 -e sh -c \"while :; do date +%T.%N; done\" >/dev/null 2>&1 &'"
sleep 8; f2=$(fpsl); sleep 6; f2b=$(fpsl)
say "D2_ANIMATED_BROKER (an xterm printing the time as fast as it can) fps=[$f2] later=[$f2b]"
grep -a 'kf3: display fps\[' $Q | tail -6 | sed 's/^.*kf3: display /SIDE_D2_FPSLINE /'
gq "$GX pkill -x xterm"; sleep 3
if grep -q 'xterm=y' "$EV/guest_tools.txt" && grep -q 'xdotool=y' "$EV/guest_tools.txt"; then
  gq "$GX sh -c 'nohup xterm -geometry 80x24+300+200 >/dev/null 2>&1 &'"; sleep 3; f3=$(fpsl); sleep 4
  ( gq "$GX xdotool search --class xterm windowactivate --sync type --delay 40 'the quick brown fox jumps over the lazy dog 0123456789 the quick brown fox jumps over the lazy dog'" 60 ) &
  TP=$!; sleep 4; f4=$(fpsl); wait $TP; sleep 3; f5=$(fpsl)
  say "D2_TYPING before=[$f3] during=[$f4] after=[$f5]"
  gq "$GX pkill -x xterm"
fi
# C. the console cursor across grab->hover and broker reconnects, under a timeline VNC client
if [ -n "${BRK_VNC_PORT:-}" ]; then
  rx=$(( gx > 40 ? gx - 40 : 0 )); ry=$(( gy > 40 ? gy - 40 : 0 ))
  python3 /root/mf/vnc_watch.py 127.0.0.1:$BRK_VNC_PORT "$EV/vncw" $rx,$ry,96,96 900 &
  VW=$!; sleep 4; mark hover_ref
  HX xdotool windowactivate --sync "$W"; HX xdotool mousemove --window "$W" 80 66; sleep 5
  for k in 1 2 3; do
    mark grab_on_$k; HX xdotool key --clearmodifiers ctrl+alt+g; sleep 4
    mark grab_off_$k; HX xdotool key --clearmodifiers ctrl+alt+g; sleep 1; HX xdotool mousemove --window "$W" 80 66; sleep 5
  done
  for k in 1 2; do
    bp=$(pgrep -n -f "[n]vkvm-display-broker --socket $SOCK")
    mark broker_kill_$k pid=$bp; kill -9 $bp; sleep 6
    mark broker_start_$k
    runuser -u $SU -- env DISPLAY="$XD" XAUTHORITY="$XA" $B --socket $SOCK --backend x11 --persist --verbose >> /workspace/bench/brk/$TAG/broker.log 2>&1 &
    for _ in $(seq 40); do W2=$(HX xdotool search --class '^nvkvm-display-broker$' 2>/dev/null | head -1); [ -n "$W2" ] && break; sleep 0.5; done
    sleep 8; W=$W2; HX xdotool windowactivate --sync "$W"; HX xdotool mousemove --window "$W" 80 66; sleep 5
    mark broker_settled_$k win=$W
  done
  touch "$EV/vncw/stop"; sleep 2; kill $VW 2>/dev/null
  say "VNCW lines=$(wc -l < "$EV/vncw/timeline.txt") cursors=$(grep -c CURSOR "$EV/vncw/timeline.txt") rois=$(ls "$EV"/vncw/roi_*.ppm 2>/dev/null | wc -l)"
  grep -a 'kf3: broker: \(guest cursor\|grab\|connected\|the display broker closed\|re-sent\)' $Q | tail -30 | cut -c1-200 | sed 's/^/SIDE_RELAY /'
  # D. a real VNC viewer (TigerVNC): the cursor it draws, read back through XFixes on the host
  gq "$GX python3 ~/display/xcursor.py image /tmp/cur_guest_cross2.pam" | sed 's/^/SIDE_GUEST_CROSS /'
  $G 'cat /tmp/cur_guest_cross2.pam' > "$EV/cur_guest_cross2.pam"
  runuser -u $SU -- env DISPLAY="$XD" XAUTHORITY="$XA" xtigervncviewer -SecurityTypes None -Shared -geometry 1000x700+10+10 127.0.0.1:$BRK_VNC_PORT > "$EV/tigervnc.log" 2>&1 &
  for _ in $(seq 30); do TV=$(HX xdotool search --name 'TigerVNC|QEMU' 2>/dev/null | grep -v "^$W\$" | head -1); [ -n "$TV" ] && break; sleep 0.5; done
  sleep 3; HX xdotool windowactivate --sync "$TV"; HX xdotool mousemove --window "$TV" 200 150; sleep 2; HX xdotool mousemove --window "$TV" 210 160; sleep 3
  HX python3 $XC image "$EV/cur_tigervnc.pam" | sed 's/^/SIDE_TIGERVNC_CURSOR /'
  python3 $XC compare "$EV/cur_guest_cross2.pam" "$EV/cur_tigervnc.pam" | sed 's/^/SIDE_TIGERVNC_VS_GUEST_/'
  HX import -window "$TV" "$EV/tigervnc_window.png" 2>/dev/null
  say "TIGERVNC win=$TV log=[$(tail -3 "$EV/tigervnc.log" | tr '\n' ' ' | cut -c1-200)]"
  pkill -f '[x]tigervncviewer -SecurityTypes None' ; sleep 1
  HX xdotool windowactivate --sync "$W"; HX xdotool mousemove --window "$W" 80 66; sleep 2
fi
# E. info mice: hover (absolute) and grab (relative)
qmp 'info mice' > "$EV/info_mice_hover.txt"; say "MICE_HOVER $(grep -E '^\*|Mouse' "$EV/info_mice_hover.txt" | tr '\n' '|')"
HX xdotool key --clearmodifiers ctrl+alt+g; sleep 3
qmp 'info mice' > "$EV/info_mice_grab.txt"; say "MICE_GRAB $(grep -E '^\*|Mouse' "$EV/info_mice_grab.txt" | tr '\n' '|')"
HX xdotool key --clearmodifiers ctrl+alt+g; sleep 3
# F. D2 with NOBODY watching: brokers stopped, no VNC client, no screendump for 12 s
mark unwatched_start
pkill -9 -f "^runuser -u [a-z0-9_-]* -- env .*nvkvm-display-broker --socket $SOCK( |\$)"; pkill -f "^[^ ]*nvkvm-display-broker --socket $SOCK( |\$)"; sleep 1
gq "$GX sh -c 'nohup xterm -geometry 60x12+300+260 -e sh -c \"while :; do date +%T.%N; done\" >/dev/null 2>&1 &'"
sleep 13; f6=$(fpsl); gq "$GX pkill -x xterm"
say "D2_UNWATCHED (an xterm printing the time, no broker, no VNC client) fps=[$f6] brokers=$(pgrep -c -f "[n]vkvm-display-broker --socket")"
for c in 'ff0000:red' '0000ff:blue'; do
  gq "$GX pkill -x xterm; $GX sh -c 'nohup xterm -bg \"#${c%%:*}\" -geometry 200x70+0+0 >/dev/null 2>&1 &'"; sleep 5; fq=$(fpsl)
  mark screendump_${c#*:}
  r=$(shot "$EV/unwatched_${c#*:}.ppm")
  px=$(python3 -c "
d=open('$EV/unwatched_${c#*:}.ppm','rb').read().split(b'\n',3); w,h=map(int,d[1].split()); b=d[3]
def at(x,y): i=(y*w+x)*3; return '%02x%02x%02x'%(b[i],b[i+1],b[i+2])
print('size=%dx%d px(40,%d)=%s px(%d,%d)=%s'%(w,h,h-60,at(40,h-60),w-40,h-40,at(w-40,h-40)))
" 2>&1)
  say "D2_SCREENDUMP expect=${c%%:*} $r $px fps_before=[$fq]"
  convert "$EV/unwatched_${c#*:}.ppm" -resize 640x "$EV/unwatched_${c#*:}.png" 2>/dev/null && rm -f "$EV/unwatched_${c#*:}.ppm"
done
gq "$GX pkill -x xterm"
sleep 3; say "D2_AFTER_SCREENDUMPS fps=[$(fpsl)]"
grep -a 'kf3: display fps\[' $Q | tail -6 | sed 's/^.*kf3: display /SIDE_FPSLINE /'
touch /workspace/bench/brk/$TAG/release
say "EXIT $(date -Is)"
