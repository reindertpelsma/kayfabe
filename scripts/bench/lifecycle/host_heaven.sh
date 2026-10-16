#!/bin/bash
# host_heaven.sh LABEL [HOOKCMD] : Unigine Heaven 4.0 (Linux, OpenGL, 1920x1080 windowed, Ultra) on the host 4070 via PRIME render offload as user ubuntu.
# F9 (in-app benchmark) through XTest. HOOK is run (in background) 25 s after F9 with F9EPOCH exported. Result: /data/claude-perf2/ev/s2/LABEL.heaven.txt
LABEL=$1; HOOK=${2:-}; EV=/data/claude-perf2/ev/s2; H=/data/claude-perf2/heaven_host/x; LOGH=/home/ubuntu/.Heaven/log.html
RU="runuser -u ubuntu -- env XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 DISPLAY=:0 XAUTHORITY=$(ls /run/user/1000/.mutter-Xwaylandauth.* | head -1)"
rm -f $LOGH
echo "HH_START $LABEL $(date -Is)" > $EV/$LABEL.hh.marker
(timeout 420 nvidia-smi dmon -s pucm -d 1 -o T > $EV/$LABEL.dmon.txt 2>&1 &)
$RU bash -c "cd $H/bin && export LD_LIBRARY_PATH=./x64 __NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia && nohup ./heaven_x64 -project_name Heaven -data_path ../ -engine_config ../data/heaven_4.0.cfg -system_script heaven/unigine.cpp -sound_app null -video_app opengl -video_multisample 0 -video_fullscreen 0 -video_mode -1 -video_width 1920 -video_height 1080 -extern_define RELEASE,QUALITY_ULTRA > /tmp/claude-hh-$LABEL.stdout 2>&1 &"
sleep 30
GEO=$($RU /data/claude-perf2/xgeom); set -- $GEO; CX=$(($1 + $3 / 2)); CY=$(($2 + $4 / 2)); echo "geom $GEO center $CX $CY" >> $EV/$LABEL.hh.marker
/data/claude-perf2/uinput_in click $CX $CY $5 $6; sleep 1; /data/claude-perf2/uinput_in key F9
export F9EPOCH=$(date +%s.%N); echo $F9EPOCH > $EV/$LABEL.f9epoch
for try in 1 2 3; do sleep 8; grep -q "Benchmark running" $LOGH 2>/dev/null && break; /data/claude-perf2/uinput_in click $CX $CY $5 $6; sleep 1; /data/claude-perf2/uinput_in key F9; echo "retry $try" >> $EV/$LABEL.hh.marker; done
if [ -n "$HOOK" ]; then (sleep 25; eval "$HOOK" > $EV/$LABEL.hook.out 2>&1) & HK=$!; fi
for i in $(seq 140); do grep -q "Benchmark results" $LOGH 2>/dev/null && break; sleep 3; done
sleep 2
sed 's/<[^>]*>//g;s/&nbsp;/ /g' $LOGH 2>/dev/null | grep -E "Benchmark|Time:|Frames:|FPS|Score|GL_RENDERER|OpenGL|renderer" | head -30 > $EV/$LABEL.heaven.txt
pkill -x heaven_x64; [ -n "${HK:-}" ] && wait $HK
pkill -f "nvidia-smi dmon"
echo "HH_EXIT $LABEL $(date -Is)" >> $EV/$LABEL.hh.marker
grep -E "FPS|Score" $EV/$LABEL.heaven.txt | head -3
