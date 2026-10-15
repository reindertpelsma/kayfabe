#!/bin/bash
# soak.sh <outdir> [minutes_in_process=10] [minutes_process_loop=10] : test (d), host side.
# Part 1: ONE Blender process rendering in a loop (alternating resolution/denoise) for N minutes (cycles_soak.py).
# Part 2: a fresh Blender process per render (context create/destroy cycles), alternating resolutions, for M minutes. Stops at the first failure.
O=$1; M1=${2:-10}; M2=${3:-10}; H=$(cd "$(dirname "$0")" && pwd); U=/data/claude-blender/up; mkdir -p "$O"
echo "SOAK_START $(date -Is) monitor: $($H/monitor.sh once)" | tee "$O/soak.log"
rm -f $U/d_inproc__*
$H/runwait.sh d_inproc $((M1*60+300)) Normal "-b --factory-startup -P C:\\kf\\apps\\blender\\cycles_soak.py -- --device OPTIX --minutes $M1 --out #d\\result.json" 400 > "$O/d_inproc.summary" 2>&1
grep -h "^SOAK" $U/d_inproc__stdout.txt > "$O/d_inproc.soak" 2>/dev/null
echo "PART1_END $(date -Is) $(tail -1 $O/d_inproc.soak) monitor: $($H/monitor.sh once)" | tee -a "$O/soak.log"
grep -q "status=ok" $O/d_inproc.soak || { echo "PART1 FAILED, stopping" | tee -a "$O/soak.log"; exit 1; }
end=$(( $(date +%s) + M2*60 )); i=0; sizes=("1920 1080" "1280 720" "2560 1440" "1024 576")
while [ $(date +%s) -lt $end ]; do
  s=${sizes[$((i % 4))]}; TAG=d_proc_$i; rm -f $U/${TAG}__*
  $H/runwait.sh $TAG 600 Normal "-b --factory-startup -P C:\\kf\\apps\\blender\\cycles_bench.py -- --device OPTIX --sub 9 --objs 3 --ntex 8 --texsize 4096 --res $s --samples 96 --denoise $((i % 2)) --warm 0 --out #d\\result.json --texdir C:\\kf\\apps\\blender\\tex" > "$O/$TAG.summary" 2>&1
  st=$(grep -h "BENCH status" $O/$TAG.summary); rs=$(grep -h "BENCH render_s" $O/$TAG.summary)
  echo "proc i=$i res=$s denoise=$((i%2)) $rs $st" | tee -a "$O/soak.log"
  [ "$st" != "BENCH status=ok" ] && { echo "PART2 FAILED at i=$i" | tee -a "$O/soak.log"; break; }
  i=$((i+1))
done
echo "SOAK_END $(date -Is) iterations_part2=$i monitor: $($H/monitor.sh once)" | tee -a "$O/soak.log"
