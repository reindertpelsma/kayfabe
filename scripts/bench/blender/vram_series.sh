#!/bin/bash
# vram_series.sh — test (b): VRAM-pressure series on VM 2990 (host side). One Blender process per step (OPTIX), scene grows by NTEX
# distinct 4096^2 8-bit PNG textures (64 MiB each on the GPU) on top of OBJS x 20*4^(SUB-1) displaced triangles.
#   vram_series.sh <outdir> "<ntex list>" [sub=9] [objs=4] [samples=64] [device=OPTIX] [ram_floor_mib=3000]
# Stops at the first step that does not finish status=ok, or when the guest's free RAM fell below the floor (the owner is using the VM).
# Per step it prints one TSV row: step ntex tris status render_s base_mib peak_mib delta_mib min_free_ram_mib xid_delta resets_delta host_vram_used_before/after
O=$1; LIST=$2; SUB=${3:-9}; OBJS=${4:-4}; SPP=${5:-64}; DEV=${6:-OPTIX}; FLOOR=${7:-3000}
H=$(cd "$(dirname "$0")" && pwd); U=/data/claude-blender/up; mkdir -p "$O"; T=$O/series.tsv
MON=$H/monitor.sh; T0=$(date '+%F %T')
echo -e "step\tntex\ttris\tstatus\trender_s\tbase_mib\tpeak_mib\tdelta_mib\tmin_free_ram_mib\txid_new\tgsp_resets_new\thost_vram_used_mib_end\tnote" > "$T"
x0=$($MON once "$T0" | cut -d, -f2,4); i=0
for N in $LIST; do
  i=$((i+1)); TAG=b_${DEV,,}_n$N; rm -f $U/${TAG}__*
  $H/runwait.sh $TAG 1500 Normal "-b --factory-startup -P C:\\kf\\apps\\blender\\cycles_bench.py -- --device $DEV --sub $SUB --objs $OBJS --ntex $N --texsize 4096 --res 1920 1080 --samples $SPP --out #d\\result.json --texdir C:\\kf\\apps\\blender\\tex" > "$O/$TAG.summary" 2>&1
  R=$U/${TAG}__result.json; S=$U/${TAG}__smi.csv
  st=$(python3 -c "import json,sys;d=json.load(open('$R'));print(d.get('status'),d.get('render_s','-'),d.get('error','')[:150].replace(chr(9),' '))" 2>/dev/null || echo "noresult - -")
  tris=$(grep -h "BENCH tris=" $U/${TAG}__stdout.txt 2>/dev/null | head -1 | cut -d= -f2)
  base=$(awk -F, 'NR==2{print $2}' $S 2>/dev/null); peak=$(awk -F, 'NR>1&&$2+0>m{m=$2+0}END{print m}' $S 2>/dev/null)
  mfr=$(awk -F, 'NR>1&&(min==""||$4+0<min){min=$4+0}END{print min}' $S 2>/dev/null)
  cur=$($MON once "$T0"); xn=$(( $(echo $cur | cut -d, -f2) - ${x0%,*} )); rn=$(( $(echo $cur | cut -d, -f4) - ${x0#*,} ))
  set -- $st; stat=$1; rs=$2; shift 2
  echo -e "$i\t$N\t${tris:-?}\t$stat\t$rs\t$base\t$peak\t$((peak-base))\t$mfr\t$xn\t$rn\t$(echo $cur | cut -d, -f7)\t$*" | tee -a "$T"
  [ "$stat" != "ok" ] && { echo "STOP: step $i status=$stat" | tee -a "$T"; break; }
  [ "${mfr:-99999}" -lt "$FLOOR" ] && { echo "STOP: guest free RAM floor ${FLOOR} MiB reached (min ${mfr})" | tee -a "$T"; break; }
done
echo "SERIES_DONE $(date +%T)" >> "$T"
