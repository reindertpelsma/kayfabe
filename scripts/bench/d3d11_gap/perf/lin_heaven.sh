#!/bin/bash
# lin_heaven.sh LABEL CFG(K|V) [DEFS] : Unigine Heaven 4.0 for Linux (OpenGL), windowed 1920x1080, Ultra, in-app benchmark (F9 through xdotool), result read from the engine's own log.
LABEL=$1; CFG=$2; DEFS=${3:-RELEASE,QUALITY_ULTRA}
EV=/data/claude-gap/ev/linux/$LABEL; mkdir -p $EV; G=/data/claude-gap/gssh.sh; P=/data/claude-gap/perf
if [ $CFG = K ]; then QP=$(cat /data/claude-gap/linux/latest/qemu.pid); LOG=/data/claude-gap/linux/latest/qemu.log; OFF=$(stat -c %s $LOG)
  python3 $P/threadcpu.py snap $QP $EV/thr0.json; (timeout 420 nvidia-smi dmon -s pucm -d 1 -o T > $EV/host_dmon.txt 2>&1 &); fi
$G 'rm -f ~/.Heaven/log.html; pkill -x heaven_x64 2>/dev/null; true'
if [ $CFG = V ]; then ($G 'nvidia-smi --query-gpu=utilization.gpu,clocks.sm,clocks.mem,power.draw,pstate --format=csv,noheader -l 1' > $EV/guest_smi.csv 2>&1 &); fi
echo "START $(date +%T.%N)" > $EV/run.txt
$G "cd ~/Unigine_Heaven-4.0/bin && export LD_LIBRARY_PATH=./x64 DISPLAY=:0 && (nohup ./heaven_x64 -project_name Heaven -data_path ../ -engine_config ../data/heaven_4.0.cfg -system_script heaven/unigine.cpp -sound_app null -video_app opengl -video_multisample 0 -video_fullscreen 0 -video_mode -1 -video_width 1920 -video_height 1080 -extern_define $DEFS > /tmp/heaven_stdout.txt 2>&1 &) ; sleep 1; pgrep -x heaven_x64"  >> $EV/run.txt
sleep ${LOADWAIT:-28}
$G 'export DISPLAY=:0; xdotool search --onlyvisible --name "Unigine Heaven" | head -1 > /tmp/hwin; W=$(cat /tmp/hwin); echo win=$W; xdotool windowactivate --sync $W 2>&1 | tail -1; xdotool mousemove 960 540 click 1; sleep 1; xdotool key F9; xdotool getwindowgeometry $W 2>&1 | tr "\n" " "' >> $EV/run.txt 2>&1
echo "F9 $(date +%T.%N)" >> $EV/run.txt
for try in 1 2 3 4; do
  sleep 6
  $G 'grep -q "Benchmark running" ~/.Heaven/log.html' && break
  echo "F9 retry $try $(date +%T)" >> $EV/run.txt
  $G 'export DISPLAY=:0; W=$(xdotool search --onlyvisible --name "Unigine Heaven" | head -1); xdotool windowactivate --sync $W; xdotool mousemove 960 540 click 1; sleep 1; xdotool key F9' >> $EV/run.txt 2>&1
done
if [ -n "${HOOK:-}" ]; then ( sleep ${HDELAY:-60}; eval "$HOOK" > $EV/hook.out 2>&1 ) & HK=$!; fi
for i in $(seq 200); do
  $G 'grep -q "Benchmark results" ~/.Heaven/log.html 2>/dev/null' && break; sleep 3
done
$G 'sleep 2; grep -E "Benchmark|Time:|Frames:|FPS|Score|Direct3D|OpenGL|GL_RENDERER|renderer|Video memory|GPU:" ~/.Heaven/log.html | sed "s/<[^>]*>//g;s/&nbsp;/ /g" | head -30; cp ~/.Heaven/log.html /tmp/heaven_log.html; pkill -x heaven_x64; true' > $EV/hlog.txt 2>&1
$G 'cat /tmp/heaven_log.html' > $EV/heaven_log.html 2>/dev/null
[ -n "${HK:-}" ] && wait $HK
pkill -f "nvidia-smi dmon"; pkill -f "gssh.sh nvidia-smi" 2>/dev/null; $G 'pkill -f "nvidia-smi --query-gpu"; true'
if [ $CFG = K ]; then python3 $P/threadcpu.py snap $QP $EV/thr1.json; python3 $P/threadcpu.py diff $EV/thr0.json $EV/thr1.json 16 > $EV/threads_cpu.txt
  tail -c +$((OFF+1)) $LOG | grep -a -E "display fps|PROF|USER_RING_LOOP|REFUSED" | cut -c1-1500 > $EV/qemu_slice.txt; fi
cat $EV/hlog.txt | grep -E "Time:|FPS|Score|Benchmark" | cut -c1-150
