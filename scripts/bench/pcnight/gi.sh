#!/bin/bash
# gi.sh NAME 'ps snippet (no double quotes)'
N=$1; C=$2
/tmp/claude-0/g.sh "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\runi.ps1 -Name $N -Ps '$C'" 2>&1 | grep -v Warning
for i in $(seq 40); do /tmp/claude-0/g.sh "Test-Path C:\\kf\\t\\$N.done" 2>&1 | grep -q True && break; sleep 1; done
/tmp/claude-0/g.sh "type C:\\kf\\t\\$N.txt" 2>&1 | grep -v Warning
