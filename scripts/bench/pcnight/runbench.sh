#!/bin/bash
# runbench.sh ARM REP : one Superposition run via the GUI (click RUN), no interactive tasks during the run
A=$1; R=$2; D=/workspace/bench/pcnight_20261113; LO=$D/arm_$A_local; LO=$D/local_$A; mkdir -p $LO
ssh root@172.22.1.20 "bash $D/hostmon.sh start $A $R"
GT=$(/tmp/claude-0/g.sh '(Get-Date).ToString("HH:mm:ss")' 2>&1 | grep -v Warn | tr -d '\r')
T0=$(date +%s)
if [ "$R" != 1 ]; then /tmp/claude-0/ginp.sh 'm 755 730; c; w 2500; m 1370 797; c' >/dev/null; else /tmp/claude-0/ginp.sh 'm 1370 797; c' >/dev/null; fi
sleep 5
ok=0
for i in $(seq 120); do
  L=$(/tmp/claude-0/g.sh 'Get-Content C:\Users\kfimage\Superposition\log.html | % { $_ -replace "<[^>]+>","" } | ? { $_ -match "Benchmark (running|stopped)|Unigine~# quit" } | out-string' 2>&1 | grep -v Warn | tr -d '\r')
  S=$(echo "$L" | grep "Benchmark stopped" | tail -1 | cut -c1-8)
  [ -n "$S" ] && [[ "$S" > "$GT" ]] && { ok=1; break; }
  sleep 5
done
T1=$(date +%s)
ssh root@172.22.1.20 "bash $D/hostmon.sh stop $A $R" | tee $LO/hm_$R.txt
echo "guest_click=$GT wall_s=$((T1-T0)) ok=$ok" | tee -a $LO/hm_$R.txt
echo "$L" > $LO/log_$R.txt
/tmp/claude-0/gshot.sh ${A}_r${R}_result | tail -1
/tmp/claude-0/g.sh 'Get-Content C:\Users\kfimage\Superposition\log.html | % { $_ -replace "<[^>]+>","" } | select -Last 25' 2>&1 | grep -v Warn > $LO/enginelog_tail_$R.txt
