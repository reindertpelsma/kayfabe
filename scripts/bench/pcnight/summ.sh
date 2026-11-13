#!/bin/bash
# summ.sh ARM REP (on PC): GPU util over the run + display fps lines summary
O=/workspace/bench/pcnight_20261113/arm_$1; R=$2
echo "--- dmon sm% (during run, from mark) "; S=$(awk '{print $3}' $O/mark_$R.start); E=$(( S + $(sed -n 's/.*dur_s=//p' $O/mark_$R.end) ))
awk -v s=$S -v e=$E 'BEGIN{n=0} /^ *[0-9]/ && NF>=15 { cmd="date -d \""$1"\" +%s"; cmd|getline t; close(cmd); if(t>=s && t<=e){n++; sm+=$4; if($4>mx)mx=$4; mem+=$5; pw+=$3; fb=$14} } END{ if(n) printf "n=%d sm_avg=%.1f sm_max=%d memutil_avg=%.1f pwr_avg=%.1fW\n", n, sm/n, mx, mem/n, pw/n }' $O/dmon_$R.txt
echo "--- display fps lines (first/mid/last)"; grep "display fps" $O/kf3_$R.log | sed -n '1p;$p' | cut -c1-330
grep "display fps" $O/kf3_$R.log | sed 's/.*kms=\([0-9.]*\).*copies=\([0-9.]*\).*/\1 \2/' | awk '{k+=$1;c+=$2;n++; if($1>km)km=$1; if($2>cm)cm=$2} END{printf "lines=%d kms_avg=%.1f kms_max=%.1f copies_avg=%.1f copies_max=%.1f\n",n,k/n,km,c/n,cm}'
grep -o "armed=[0-9.]* tick=[0-9.]*" $O/kf3_$R.log | sort | uniq -c | sort -rn | head -3
