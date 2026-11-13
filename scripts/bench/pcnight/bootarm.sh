#!/bin/bash
# bootarm.sh ARM ENV BACKEND [BARGS]  (env MAXFPS, BENV pass through)
A=$1; D=/workspace/bench/pcnight_20261113
ssh root@172.22.1.20 "cd $D; [ -d arm_$A ] && mv arm_$A arm_$A.old.$(date +%s); echo \"\$(date -u +%FT%TZ) pcnight arm $A boot\" >> /root/prov/PC_ACTIVE.log; MAXFPS='$MAXFPS' BENV='$BENV' nohup setsid bash runarm.sh $A $2 $3 '$4' > runarm_$A.out 2>&1 < /dev/null & sleep 15; cat $D/runarm_$A.out; cat $D/arm_$A/qemu.pid"
for i in $(seq 60); do /tmp/claude-0/g.sh '(Get-CimInstance Win32_ComputerSystem).UserName' 2>&1 | grep -q kfimage && break; sleep 10; done
sleep 30
/tmp/claude-0/g.sh 'mkdir C:\\kf\\t -Force | out-null' >/dev/null 2>&1; (cd /workspace/bench/pcnight_20261113/tools; ./put.sh runi.ps1 shot.ps1 inp.ps1 lau.ps1 l3d.ps1)
/tmp/claude-0/g.sh '(Get-Date).ToUniversalTime()' 2>&1 | grep -v Warn
/tmp/claude-0/gi.sh lau '& C:\\kf\\lau.ps1' | tail -1
sleep 12
/tmp/claude-0/gshot.sh boot_$A | tail -1
