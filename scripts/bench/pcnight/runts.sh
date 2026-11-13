#!/bin/bash
# runts.sh ARM REP : 3DMark Time Spy via GUI click on RUN; completion = new .3dmark-result file
A=$1; R=ts$2; D=/workspace/bench/pcnight_20261113; LO=$D/local_$A; mkdir -p $LO
ssh root@172.22.1.20 "bash $D/hostmon.sh start $A $R"
N0=$(/tmp/claude-0/g.sh '(Get-ChildItem C:\Users\kfimage\Documents\3DMark\*.3dmark-result | measure).Count' 2>&1 | grep -v Warn | tr -d '\r')
T0=$(date +%s)
/tmp/claude-0/ginp.sh 'm 461 498; c' >/dev/null
ok=0
for i in $(seq 160); do
  sleep 6
  N=$(/tmp/claude-0/g.sh '(Get-ChildItem C:\Users\kfimage\Documents\3DMark\*.3dmark-result | measure).Count' 2>&1 | grep -v Warn | tr -d '\r')
  [ "$N" -gt "$N0" ] && { ok=1; break; }
done
T1=$(date +%s)
ssh root@172.22.1.20 "bash $D/hostmon.sh stop $A $R" | tee $LO/hm_$R.txt
echo "n0=$N0 wall_s=$((T1-T0)) ok=$ok" | tee -a $LO/hm_$R.txt
/tmp/claude-0/g.sh 'Get-ChildItem C:\Users\kfimage\Documents\3DMark\*.3dmark-result | sort LastWriteTime | select -Last 1 | % Name' 2>&1 | grep -v Warn | tee $LO/tsfile_$R.txt
sleep 8
/tmp/claude-0/gshot.sh ${A}_${R}_result | tail -1
ssh root@172.22.1.20 "cd $D; bash summ.sh $A $R" > $LO/summ_$R.txt 2>&1
