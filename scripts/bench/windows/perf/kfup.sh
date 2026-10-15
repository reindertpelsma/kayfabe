#!/bin/bash
# kfup.sh TAG GLOB : upload C:\kfperf\out\TAG\GLOB from the guest to /data/claude-perf/up/TAG__name
python3 /data/kfagent_server.py run "Get-ChildItem C:\\kfperf\\out\\$1 | Where-Object { \$_.Name -like '$2' } | ForEach-Object { Invoke-RestMethod -Method Put -Uri ('http://10.0.2.2:8791/$1__' + \$_.Name) -InFile \$_.FullName -UseBasicParsing | Out-Null; 'up ' + \$_.Name }" ${3:-120}
